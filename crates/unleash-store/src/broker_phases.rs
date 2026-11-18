//! Frontière typée interne de journalisation du broker, sans dépendance vers le broker.
//! Les manifestes complets sont durables et adressés par leur digest ; ils ne vont jamais dans les logs.

use crate::StoreError;
use core::fmt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, ActorId, ApprovalScopeId, AuthorityGeneration,
    AuthorizationEvaluationId, DispatchId, LineageFence, ProjectId, ReceiptId, ReconcileStrategy,
    Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{ErrorCode, EventCursor, ReasonCode, RetryClass};
use uuid::Uuid;

/// Version du format interne de phase broker.
pub const BROKER_PHASE_SCHEMA_VERSION: u32 = 1;

/// Identité conservée pendant toute une évaluation et ses suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerEvaluation {
    /// Identité durable allouée avant Authorizing.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Prédécesseur explicite pour une nouvelle évaluation après résultat retryable et backoff.
    pub previous_evaluation_id: Option<AuthorizationEvaluationId>,
}

/// Cause durable du plan de contrôle, résolue dans le writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerCause {
    /// Reçu courant, avec cause_ref receipt:UUID.
    CommandReceipt {
        /// Reçu réel préalablement engagé.
        receipt_id: ReceiptId,
    },
    /// Événement du flux courant, avec cause_ref event:UUID.
    Event {
        /// Événement réel préalablement engagé.
        event_id: Uuid,
    },
    /// Timer durable ; producteur qualifié requis.
    Timer {
        /// Identité réelle du timer.
        timer_id: Uuid,
    },
}

/// Intention native exacte et champs d'autorité typés nécessaires au stockage.
/// Le manifeste est le JSON objet complet de ActionIntent v1, pas son payload métier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerIntent {
    /// Action durable.
    pub action_id: ActionId,
    /// Origine structurée ; aucun parent n'est inventé.
    pub origin: ActionOrigin,
    /// Clé du principal préenregistré.
    pub principal_id: ActorId,
    /// Cause prouvée par une ligne durable.
    pub cause: BrokerCause,
    /// Projet effectif éventuel.
    pub project_id: Option<ProjectId>,
    /// Empreinte exacte du payload autorisé.
    pub payload_digest: Sha256Digest,
    /// Révision de scope engagée.
    pub scope_revision: Revision,
    /// Révision de politique fournie au broker.
    pub policy_revision: Revision,
    /// Lignée/epoch/fence engagé.
    pub lineage_fence: LineageFence,
    /// Date limite de l'action.
    pub action_deadline: TimestampMs,
    /// Stratégie de réconciliation engagée.
    pub reconcile_strategy: ReconcileStrategy,
    /// Intention native complète, conservée sans transformation.
    pub manifest: Map<String, Value>,
    /// Digest RFC 8785 de cette intention complète.
    pub intent_digest: Sha256Digest,
}

/// Exécuteur reconnu par le profil v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerExecutor {
    /// Exécuteur Unleash qualifié.
    Unleash,
    /// Exécuteur Guard négocié et qualifié.
    Guard,
}

/// Décision complète liée à une évaluation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerAuthorizationDecision {
    /// Action autorisée.
    pub action_id: ActionId,
    /// Évaluation actuelle.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Évaluation source d'une décision mise en cache, si applicable.
    pub source_evaluation_id: Option<AuthorizationEvaluationId>,
    /// Référence originale de décision.
    pub decision_ref: String,
    /// Révision de politique exacte.
    pub policy_revision: Revision,
    /// Révision de scope exacte.
    pub scope_revision: Revision,
    /// Payload exact.
    pub payload_digest: Sha256Digest,
    /// Expiration d'un Allow, si présente.
    pub expiry: Option<TimestampMs>,
    /// Obligations complètes ; seules celles qualifiées sont admissibles.
    pub applied_obligations: Vec<Value>,
    /// Référence de repli réellement employée.
    pub fallback_policy_ref: Option<String>,
    /// Décision issue du cache.
    pub from_cache: bool,
    /// Exécuteur retenu.
    pub executor: BrokerExecutor,
}

/// Projection d'erreur fermée ; aucun Debug/Display d'erreur externe n'est conservé.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerAuthorizationFailure {
    /// Classe IPC canonique.
    pub code: ErrorCode,
    /// Raison canonique, si disponible.
    pub reason_code: Option<ReasonCode>,
    /// Politique de reprise déclarée.
    pub retry_class: RetryClass,
    /// Référence sûre vers les détails réellement disponibles.
    pub details_ref: Option<String>,
    /// Fin durable du backoff avant une nouvelle évaluation.
    pub retry_not_before_ms: Option<TimestampMs>,
}

/// Résultat complet d'évaluation, sans transformer une indisponibilité en refus terminal.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerAuthorizationOutcome {
    /// Autorisation complète.
    Authorized {
        /// Décision exacte.
        decision: BrokerAuthorizationDecision,
    },
    /// Attente d'une disposition humaine.
    ApprovalRequired {
        /// Action concernée.
        action_id: ActionId,
        /// Scope exact de l'approbation.
        approval_scope: ApprovalScopeId,
        /// Référence de décision.
        decision_ref: String,
        /// Révision fournie par le broker, distincte du CAS SQLite.
        revision: Revision,
    },
    /// Refus ou indisponibilité projetés sans message externe brut.
    Failed {
        /// Projection fermée du résultat.
        failure: BrokerAuthorizationFailure,
    },
}

/// Origine d'un résultat d'autorisation, sans réécrire son résultat initial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerAuthorizationCause {
    /// Résultat de l'évaluation de politique/Guard.
    Evaluation,
    /// Approbation déjà engagée par le service.
    HumanApproval {
        /// Acteur résolu.
        actor_id: ActorId,
        /// Reçu réel de la disposition.
        decision_receipt_id: ReceiptId,
        /// Digest canonique de cette commande.
        decision_command_digest: Sha256Digest,
        /// Scope exact approuvé.
        approval_scope: ApprovalScopeId,
    },
}

/// Reçu local d'observation complet ; output_digest n'est pas un reçu fournisseur.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerActionReceipt {
    /// Action observée.
    pub action_id: ActionId,
    /// Évaluation conservée.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Dispatch réel corrélé.
    pub dispatch_id: DispatchId,
    /// Décision corrélée.
    pub decision_ref: String,
    /// État rapporté, validé contre la phase durable.
    pub state: ActionState,
    /// Empreinte de sortie réellement fournie.
    pub output_digest: Option<Sha256Digest>,
    /// Références d'application des contrôles.
    pub enforcement_refs: Vec<String>,
    /// Référence d'audit distante réellement disponible.
    pub remote_audit_ref: Option<String>,
    /// Absence explicite d'audit distant.
    pub remote_audit_unavailable: bool,
    /// Date originale de l'observation.
    pub observed_at_ms: TimestampMs,
}

/// Provenance d'observation ; une référence fournie reste à vérifier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerObservationCause {
    /// Observation de l'exécuteur lié.
    Execution,
    /// Réconciliation avec preuve locale éventuelle.
    Reconciliation {
        /// Référence fournie, jamais inventée.
        evidence_ref: Option<String>,
    },
    /// Reçu externe avec preuve éventuelle.
    ExternalReceipt {
        /// Référence fournie, jamais inventée.
        evidence_ref: Option<String>,
    },
}

/// Phase fermée du journal.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerPhase {
    /// Intention exacte avant tout Guard.
    Authorizing {
        /// Intention engagée.
        intent: BrokerIntent,
    },
    /// Résultat d'évaluation ou approbation ultérieure distincte.
    Authorization {
        /// Action concernée.
        action_id: ActionId,
        /// Intention exacte.
        intent_digest: Sha256Digest,
        /// Résultat complet.
        outcome: BrokerAuthorizationOutcome,
        /// Cause attestée du résultat.
        cause: BrokerAuthorizationCause,
    },
    /// Marqueur durable, jamais rejouable vers un second effet.
    Dispatch {
        /// Action concernée.
        action_id: ActionId,
        /// Intention exacte.
        intent_digest: Sha256Digest,
        /// Décision exacte autorisée.
        decision: BrokerAuthorizationDecision,
        /// Identifiant stable de dispatch.
        dispatch_id: DispatchId,
        /// Fence exact du marqueur.
        fence: LineageFence,
    },
    /// Observation ou réconciliation attribuée.
    Observation {
        /// Action concernée.
        action_id: ActionId,
        /// Intention exacte.
        intent_digest: Sha256Digest,
        /// Reçu complet conservé.
        receipt: BrokerActionReceipt,
        /// Ordinal de la suite d'observations, indépendant de actions.revision.
        observation_revision: Revision,
        /// Provenance explicite.
        cause: BrokerObservationCause,
    },
}

/// Requête interne ; expected_revision est le CAS du writer, pas l'ordinal d'observation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerPhaseRequest {
    /// Lignée courante, vérifiée avant tout lookup.
    pub authority_generation: AuthorityGeneration,
    /// Révision d'action attendue pour une nouvelle phase.
    pub expected_revision: Revision,
    /// Identité durable de l'évaluation.
    pub evaluation: BrokerEvaluation,
    /// Phase exacte à engager.
    pub phase: BrokerPhase,
}

/// Résultat durable d'une phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerPhaseCommit {
    /// Action concernée.
    pub action_id: ActionId,
    /// Révision engagée.
    pub revision: Revision,
    /// Curseur du fait atomique.
    pub event_cursor: EventCursor,
    /// Rejeu exact sans nouvel événement, jamais vrai pour Dispatch.
    pub replayed: bool,
}

/// Vue minimale pour construire le CAS de l'adaptateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerActionSnapshot {
    /// Action concernée.
    pub action_id: ActionId,
    /// État d'autorité.
    pub state: ActionState,
    /// Révision CAS courante.
    pub revision: Revision,
    /// Intention engagée.
    pub intent_digest: Sha256Digest,
    /// Dernière évaluation connue.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Fence durable.
    pub fence: LineageFence,
    /// Dernier dispatch éventuel.
    pub dispatch_id: Option<DispatchId>,
}

/// Signature du port interne, implémenté par le Store ; aucune autorité client n'en découle.
pub trait BrokerPhaseStore {
    /// Engage état, contenu exact et événement dans le writer unique.
    ///
    /// # Errors
    /// Refuse les lignées/révisions périmées, divergences, parents non prouvés et tout Dispatch déjà engagé.
    fn commit_broker_phase(
        &self,
        request: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError>;
    /// Lit le CAS minimal sans réinterpréter une intention historique.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture du Store.
    fn inspect_broker_action(
        &self,
        action_id: ActionId,
    ) -> Result<Option<BrokerActionSnapshot>, StoreError>;
}

impl BrokerPhase {
    /// Action concernée par la phase.
    #[must_use]
    pub fn action_id(&self) -> ActionId {
        match self {
            Self::Authorizing { intent } => intent.action_id,
            Self::Authorization { action_id, .. }
            | Self::Dispatch { action_id, .. }
            | Self::Observation { action_id, .. } => *action_id,
        }
    }
    /// Nom structurel de la phase, sans données externes.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Authorizing { .. } => "authorizing",
            Self::Authorization { .. } => "authorization",
            Self::Dispatch { .. } => "dispatch",
            Self::Observation { .. } => "observation",
        }
    }
}

impl fmt::Debug for BrokerPhaseRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerPhaseRequest")
            .field("authority_generation", &self.authority_generation)
            .field("evaluation_id", &self.evaluation.evaluation_id)
            .field("phase", &self.phase.kind())
            .field("action_id", &self.phase.action_id())
            .field("expected_revision", &self.expected_revision)
            .finish_non_exhaustive()
    }
}
