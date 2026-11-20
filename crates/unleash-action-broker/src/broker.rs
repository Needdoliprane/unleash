// SPDX-License-Identifier: Apache-2.0
//! Médiateur central [`ActionBroker`] : autorisation, gouvernance des politiques et sujets,
//! revalidation de fence/mandat avant dispatch, intégration [`GuardPort`], gestion des secrets
//! et réconciliation des effets (`REQ-SEC-001`..`REQ-SEC-011`, `failure-recovery.md`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionEvent, ActionId, ActionState, ActorId, ApprovalScopeId, AuthorizationEvaluationId,
    ClockPort, DispatchId, GuardMode as DomainGuardMode, IdGeneratorPort, LineageFence, ProjectId,
    ReceiptId, ReconcileOutcome, Revision, Sha256Digest, TimestampMs, transition_action,
};
use unleash_protocol::ClientKind;

use crate::authority::{
    EffectiveAuthority, validate_authority_mutation_provenance, validate_path_within_roots,
    verify_path_toctou,
};
use crate::cache::{DecisionCache, DecisionCacheKey, MAX_DECISION_CACHE_TTL};
use crate::effects::{
    ActionIntent, ActionProvenance, EffectsClass, TypedSubjectRef, UnvalidatedActionIntent,
};
use crate::error::{ActionBrokerError, GuardPortError};
use crate::guard_port::{
    BoxFuture, DEFAULT_EVALUATE_TIMEOUT_MS, DEFAULT_HEALTH_CHECK_INTERVAL_MS, ExecutorKind,
    GuardCapabilities, GuardDecisionRef, GuardEvaluateRequest, GuardEvaluationResponse,
    GuardExecuteRequest, GuardHealthStatus, GuardMappingProfile, GuardMode, GuardPort,
    GuardSubjectActionRequest, InternalObligation, MonotonicClockPort,
    NegotiateCapabilitiesRequest, ObligationCapabilityPort, SleepPort,
    map_and_validate_obligations, revalidate_obligations,
};
use crate::isolation::{IsolationMode, IsolationProbePort, verify_isolation_requirements};
use crate::journal::{
    AuthorizationCause, AuthorizationEvaluation, BrokerJournalEntry, BrokerJournalPort,
    ObservationCause, TestMemoryJournal,
};
use crate::retention::redact_canaries;
use crate::secrets::{AuthorizedSecretAccess, ResolvedSecret, SecretStore};

const AUTHORIZATION_RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// Capacité administrative vérifiée pour les opérations de gouvernance F017 (`REQ-SEC-004`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerAdminCapability {
    /// Modification de la politique de sécurité (`SetPolicy`).
    ManagePolicy,
    /// Configuration du port AI-Guard (`ConfigureGuard`).
    ConfigureGuard,
    /// Suspension et reprise locale d'un sujet (`SuspendSubject`, `ResumeSubject`).
    SuspendResumeSubject,
    /// Révocation définitive d'un sujet (`RevokeSubject`).
    RevokeSubject,
    /// Décision humaine d'approbation d'action (`ResolveActionApproval`).
    ResolveActionApproval,
}

/// Identité et capacités vérifiées de l'appelant soumettant une opération au broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    /// Identifiant authentifié de l'acteur.
    pub actor_id: ActorId,
    /// Type de client IPC (`Cli`, `Tui`, `Gui`, `Worker`, `Probe`).
    pub client_kind: ClientKind,
    /// UID noyau du pair si applicable.
    pub peer_uid: Option<u32>,
    /// Capacités administratives accordées au pair.
    pub capabilities: BTreeSet<BrokerAdminCapability>,
}

impl CallerIdentity {
    /// Construit un appelant administratif complet (CLI/TUI/GUI authentifié).
    #[must_use]
    pub fn admin(actor_id: ActorId) -> Self {
        Self {
            actor_id,
            client_kind: ClientKind::Cli,
            peer_uid: None,
            capabilities: BTreeSet::from([
                BrokerAdminCapability::ManagePolicy,
                BrokerAdminCapability::ConfigureGuard,
                BrokerAdminCapability::SuspendResumeSubject,
                BrokerAdminCapability::RevokeSubject,
                BrokerAdminCapability::ResolveActionApproval,
            ]),
        }
    }

    /// Construit un appelant worker restreint sans aucune capacité administrative (`REQ-SEC-004`).
    #[must_use]
    pub fn worker(actor_id: ActorId) -> Self {
        Self {
            actor_id,
            client_kind: ClientKind::Worker,
            peer_uid: None,
            capabilities: BTreeSet::new(),
        }
    }

    fn require_admin(
        &self,
        cap: BrokerAdminCapability,
        op_name: &str,
    ) -> Result<(), ActionBrokerError> {
        if matches!(self.client_kind, ClientKind::Worker | ClientKind::Probe)
            || !self.capabilities.contains(&cap)
        {
            return Err(ActionBrokerError::OperationNotAuthorized {
                operation: op_name.to_owned(),
                actor: self.actor_id.to_string(),
            });
        }
        Ok(())
    }
}

/// Résultat d'une exécution d'effet accomplie par l'adaptateur physique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionExecutionResult {
    /// Identifiant du dispatch physique.
    pub dispatch_id: DispatchId,
    /// Identifiant corrélé chez un fournisseur distant éventuel.
    pub provider_request_id: Option<String>,
    /// Empreinte du résultat observé.
    pub output_digest: Sha256Digest,
    /// Horodatage UTC d'observation.
    pub observed_at_ms: TimestampMs,
}

/// Échec ou ambiguïté retournée par l'adaptateur physique d'exécution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionExecutionFailure {
    /// Échec établi sans effet résiduel.
    ConfirmedFailureNoEffect {
        /// Identifiant de dispatch tenté.
        dispatch_id: DispatchId,
        /// Motif d'échec sans secret.
        reason: String,
    },
    /// Départ ou effet impossible à exclure (C4/C5) : doit mener à `OutcomeUnknown`.
    OutcomeUnknown {
        /// Identifiant de dispatch ambigu.
        dispatch_id: DispatchId,
        /// Motif d'incertitude.
        reason: String,
    },
}

/// Port d'exécution physique appelé exclusivement par [`ActionBroker::execute_authorized`].
pub trait ActionExecutorPort: Send + Sync {
    /// Exécute l'effet autorisé avec les secrets temporaires déjà validés pour le transport.
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        resolved_secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>>;
}

/// Port évaluant la politique de repli préautorisée sous `GuardMode::GuardOptional`.
pub trait FallbackPolicyPort: Send + Sync {
    /// Évalue la politique locale `fallback_policy_ref`. Retourne `Ok(())` si autorisé, `Err(motif)` sinon.
    ///
    /// # Errors
    /// Retourne une description textuelle du refus de repli.
    fn evaluate_fallback(
        &self,
        fallback_policy_ref: &str,
        intent: &ActionIntent,
        authority: &EffectiveAuthority,
    ) -> Result<(), String>;
}

/// Évaluateur de politique de repli vérifiant que la référence appartient à la liste préautorisée.
#[derive(Debug, Default, Clone)]
pub struct LocalAuthorityFallbackPolicy {
    authorized_refs: BTreeSet<String>,
}

impl LocalAuthorityFallbackPolicy {
    /// Construit un évaluateur autorisant explicitement les références listées.
    #[must_use]
    pub fn with_authorized_refs<'a>(refs: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            authorized_refs: refs.into_iter().map(str::to_owned).collect(),
        }
    }
}

impl FallbackPolicyPort for LocalAuthorityFallbackPolicy {
    fn evaluate_fallback(
        &self,
        fallback_policy_ref: &str,
        _intent: &ActionIntent,
        _authority: &EffectiveAuthority,
    ) -> Result<(), String> {
        if self.authorized_refs.contains(fallback_policy_ref) {
            Ok(())
        } else {
            Err(format!(
                "Politique de repli '{fallback_policy_ref}' non préautorisée"
            ))
        }
    }
}

/// Décision humaine d'approbation (`ResolveActionApproval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Autorise l'exécution de l'action sur le digest et le périmètre exacts.
    Approve,
    /// Refuse définitivement l'action.
    Deny,
}

/// Statut de propagation distante d'une suspension ou révocation vers AI-Guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RemotePropagationStatus {
    /// Aucun endpoint AI-Guard configuré (`broker_only`).
    NoGuardConfigured,
    /// Capacité non négociée par l'adaptateur AI-Guard actif (limitation affichée).
    UnsupportedByGuard,
    /// Propagé avec succès vers AI-Guard.
    Propagated,
    /// Échec ou indisponibilité lors de la propagation distante (mesure locale maintenue).
    PropagationFailed {
        /// Détail d'erreur.
        reason: String,
    },
}

/// Commande typée `SetPolicy` (`REQ-SEC-005`, `Q-05`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPolicyCommand {
    /// Projet ciblé (`None` = défaut global).
    pub project_id: Option<ProjectId>,
    /// Révision optimiste attendue.
    pub expected_revision: Revision,
    /// Nouveau mode d'isolation (`Standard` ou `Strict`).
    pub isolation_mode: IsolationMode,
    /// Motif tracé du changement.
    pub reason: String,
}

/// Commande typée `ConfigureGuard` (`Q-04`, `ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigureGuardCommand {
    /// Endpoint UDS ou HTTPS (`None` = efface l'endpoint -> `BrokerOnly`).
    pub endpoint: Option<String>,
    /// Mode explicite (`None` -> `BrokerOnly` sans endpoint, `GuardRequired` avec endpoint).
    pub guard_mode: Option<GuardMode>,
    /// Profil de mapping demandé (`None` -> conserve ou `NoneAccepted` par défaut).
    pub mapping_profile: Option<GuardMappingProfile>,
    /// Exécuteur demandé (`Unleash` par défaut).
    pub executor_kind: Option<ExecutorKind>,
    /// Délai maximal d'évaluation en ms (`2000` par défaut).
    pub evaluate_timeout_ms: Option<u64>,
    /// TTL du cache borné en secondes (`<= 300`).
    pub cache_ttl_secs: Option<u64>,
    /// Référence de secret optionnelle pour mTLS/token transport AI-Guard.
    pub auth_secret_ref: Option<unleash_domain::SecretRef>,
}

/// Commande typée `SuspendSubject` (`F017` R7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspendSubjectCommand {
    /// Sujet typé à suspendre localement.
    pub subject: TypedSubjectRef,
    /// Portée projet optionnelle.
    pub project_id: Option<ProjectId>,
    /// Révision attendue du sujet.
    pub expected_revision: Revision,
    /// Motif de la suspension.
    pub reason: String,
}

/// Commande typée `ResumeSubject` (`F017` R7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeSubjectCommand {
    /// Sujet typé à rétablir.
    pub subject: TypedSubjectRef,
    /// Portée projet optionnelle.
    pub project_id: Option<ProjectId>,
    /// Révision attendue du sujet.
    pub expected_revision: Revision,
    /// Motif de la reprise.
    pub reason: String,
}

/// Commande typée `RevokeSubject` (`F017`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeSubjectCommand {
    /// Sujet typé à révoquer définitivement.
    pub subject: TypedSubjectRef,
    /// Portée projet optionnelle.
    pub project_id: Option<ProjectId>,
    /// Révision attendue du sujet.
    pub expected_revision: Revision,
    /// Motif de la révocation.
    pub reason: String,
}

/// Commande typée `ResolveActionApproval` (`F017`, `ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveActionApprovalCommand {
    /// Existing durable human decision receipt, resolved by the service.
    pub decision_receipt_id: ReceiptId,
    /// Canonical command digest bound to the decision receipt.
    pub decision_command_digest: Sha256Digest,
    /// Action en attente d'approbation (`approval_required`).
    pub action_id: ActionId,
    /// Périmètre d'approbation exact attendu.
    pub approval_scope: ApprovalScopeId,
    /// Digest exact de l'action approuvée.
    pub payload_digest: Sha256Digest,
    /// Décision humaine (`Approve` ou `Deny`).
    pub decision: ApprovalDecision,
    /// Révision attendue de l'action.
    pub expected_revision: Revision,
    /// Motif explicite de la décision.
    pub reason: String,
}

/// Résumé de politique retourné par `ReadPolicySummary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySummary {
    /// Projet concerné (`None` = globale).
    pub project_id: Option<ProjectId>,
    /// Révision courante de la politique.
    pub policy_revision: Revision,
    /// Mode d'isolation effectif (`Standard` ou `Strict`).
    pub isolation_mode: IsolationMode,
    /// Libellé exact du confinement (`"non confiné"` en `Standard`).
    pub confinement_label: String,
    /// Mode AI-Guard actif.
    pub guard_mode: GuardMode,
    /// Profil de mapping actif.
    pub mapping_profile_id: String,
    /// Sujets actuellement suspendus localement.
    pub suspended_subjects: Vec<String>,
    /// Sujets définitivement révoqués.
    pub revoked_subjects: Vec<String>,
    /// Acteur ayant opéré la dernière mise à jour.
    pub updated_by: Option<String>,
    /// Motif de la dernière mise à jour.
    pub last_reason: Option<String>,
}

/// Instantané de santé et capacités retourné par `ReadGuardHealth`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardHealthSnapshot {
    /// Mode configuré côté Unleash.
    pub mode: GuardMode,
    /// Vrai uniquement si un endpoint réel/double est connecté.
    pub connected: bool,
    /// Endpoint configuré (sans credential).
    pub endpoint: Option<String>,
    /// Version de protocole négociée.
    pub negotiated_protocol_version: Option<String>,
    /// Profil de mapping actif.
    pub mapping_profile_id: String,
    /// Capacités négociées si connecté.
    pub capabilities: Option<GuardCapabilities>,
    /// Santé observée du garde.
    pub health_status: GuardHealthStatus,
    /// Exécuteur actif (`Unleash` ou `Guard`).
    pub active_executor: ExecutorKind,
    /// Limitations visibles (ex: `SuspendSubject` distant non négocié).
    pub remote_limitations: Vec<String>,
}

/// Reçu de suspension ou reprise d'un sujet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectSuspensionReceipt {
    /// Sujet concerné (identité inchangée après reprise).
    pub subject: TypedSubjectRef,
    /// État résultant (`true` si suspendu, `false` si rétabli).
    pub suspended: bool,
    /// Nouvelle révision du sujet.
    pub revision: Revision,
    /// Actions en vol détectées devant être réconciliées pendant la suspension.
    pub in_flight_actions_to_reconcile: Vec<ActionId>,
    /// Statut de propagation distante vers AI-Guard.
    pub remote_propagation: RemotePropagationStatus,
}

/// Reçu de révocation définitive d'un sujet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectRevocationReceipt {
    /// Sujet définitivement révoqué.
    pub subject: TypedSubjectRef,
    /// Nouvelle révision du sujet.
    pub revision: Revision,
    /// Statut de propagation distante vers AI-Guard.
    pub remote_propagation: RemotePropagationStatus,
    /// Référence d'audit distante si retournée par `GuardPort`.
    pub remote_ref: Option<String>,
}

/// Décision d'autorisation délivrée par `ActionBroker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationDecision {
    /// Durable evaluation that produced this authorization.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Source evaluation when a cached decision was reused.
    pub source_evaluation_id: Option<AuthorizationEvaluationId>,
    /// Action associée.
    pub action_id: ActionId,
    /// Référence corrélée de décision.
    pub decision_ref: GuardDecisionRef,
    /// Révision de politique validée.
    pub policy_revision: Revision,
    /// Révision de scope validée.
    pub scope_revision: Revision,
    /// Empreinte exacte du payload autorisé.
    pub payload_digest: Sha256Digest,
    /// Expiration éventuelle de la décision `Allow`.
    pub expiry: Option<TimestampMs>,
    /// Obligations internes validées devant être maintenues jusqu'après dispatch.
    pub applied_obligations: Vec<InternalObligation>,
    /// Référence de politique de repli utilisée si `GuardOptional` a basculé.
    pub fallback_policy_ref: Option<String>,
    /// Indique si la décision provient d'un hit du cache borné.
    pub from_cache: bool,
    /// Exécuteur retenu pour cette action.
    pub executor: ExecutorKind,
}

/// Résultat d'une soumission d'autorisation auprès du broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AuthorizationOutcome {
    /// Action passée à `authorized` et prête pour `execute_authorized`.
    Authorized(AuthorizationDecision),
    /// Action passée à `approval_required` en attente de `ResolveActionApproval`.
    ApprovalRequired {
        /// Durable evaluation awaiting approval.
        evaluation_id: AuthorizationEvaluationId,
        /// Action en attente.
        action_id: ActionId,
        /// Périmètre d'approbation exact.
        approval_scope: ApprovalScopeId,
        /// Référence de décision du garde.
        decision_ref: GuardDecisionRef,
        /// Révision courante de l'action pour contrôle optimiste.
        revision: Revision,
    },
}

/// Reçu durable d'exécution ou de réconciliation d'une action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionReceipt {
    /// Evaluation used by the committed dispatch.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Identifiant de l'action.
    pub action_id: ActionId,
    /// Identifiant du dispatch physique corrélé.
    pub dispatch_id: DispatchId,
    /// Référence de décision corrélée.
    pub decision_ref: GuardDecisionRef,
    /// État résultant dans la machine d'état d'action.
    pub state: ActionState,
    /// Empreinte de sortie si observée.
    pub output_digest: Option<Sha256Digest>,
    /// Références d'application des obligations/contrôles effectifs.
    pub enforcement_refs: Vec<String>,
    /// Référence d'audit distante si disponible.
    pub remote_audit_ref: Option<String>,
    /// Indique explicitement si le journal d'audit distant était indisponible.
    pub remote_audit_unavailable: bool,
    /// Horodatage UTC d'observation.
    pub observed_at_ms: TimestampMs,
}

/// Dossier complet d'audit corrélé conservé par le broker (`REQ-SEC-010`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerAuditRecord {
    /// Identifiant de l'action.
    pub action_id: ActionId,
    /// Empreinte canonique du payload.
    pub payload_digest: Sha256Digest,
    /// Référence de décision finale ou dernière connue.
    pub decision_ref: Option<GuardDecisionRef>,
    /// Identifiant du profil de mapping actif lors de la décision.
    pub mapping_profile_id: String,
    /// Révision de politique.
    pub policy_revision: Revision,
    /// Obligations imposées.
    pub obligations: Vec<InternalObligation>,
    /// Références effectives d'application.
    pub enforcement_refs: Vec<String>,
    /// Instant d'engagement durable (dispatch).
    pub committed_at_ms: Option<TimestampMs>,
    /// Motif de refus ou d'incertitude éventuel.
    pub denial_or_unknown_reason: Option<String>,
}

/// Événements d'audit typés émis par le broker (`F017/interfaces.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "PascalCase")]
pub enum BrokerAuditEvent {
    /// Demande d'autorisation enregistrée.
    ActionAuthorizationRequested {
        /// Action concernée.
        action_id: ActionId,
        /// Classe d'effet.
        effects_class: EffectsClass,
        /// Sujet canonique.
        subject: String,
    },
    /// Approbation humaine requise.
    ActionApprovalRequired {
        /// Action concernée.
        action_id: ActionId,
        /// Périmètre d'approbation.
        approval_scope: ApprovalScopeId,
    },
    /// Action autorisée.
    ActionAuthorized {
        /// Action concernée.
        action_id: ActionId,
        /// Référence de décision.
        decision_ref: GuardDecisionRef,
        /// Repli éventuel utilisé.
        fallback_policy_ref: Option<String>,
    },
    /// Action refusée.
    ActionDenied {
        /// Action concernée.
        action_id: ActionId,
        /// Motif expurgé.
        reason: String,
    },
    /// Marqueur durable de dispatch engagé avant I/O.
    ActionDispatchCommitted {
        /// Action concernée.
        action_id: ActionId,
        /// Dispatch engagé.
        dispatch_id: DispatchId,
        /// Référence de décision.
        decision_ref: GuardDecisionRef,
    },
    /// Observation finale ou `OutcomeUnknown` enregistrée.
    ActionObserved {
        /// Action concernée.
        action_id: ActionId,
        /// Dispatch concerné.
        dispatch_id: DispatchId,
        /// Nouvel état de l'action.
        state: ActionState,
    },
    /// Sujet suspendu.
    SubjectSuspended {
        /// Clé du sujet.
        subject: String,
        /// Motif expurgé.
        reason: String,
    },
    /// Suspension levée.
    SubjectResumed {
        /// Clé du sujet.
        subject: String,
        /// Motif expurgé.
        reason: String,
    },
    /// Sujet révoqué définitivement.
    SubjectRevoked {
        /// Clé du sujet.
        subject: String,
        /// Motif expurgé.
        reason: String,
    },
    /// Changement de mode, santé ou repli AI-Guard.
    GuardHealthChanged {
        /// Nouveau statut observé.
        status: GuardHealthStatus,
        /// Détail expurgé.
        detail: String,
    },
    /// Incident de sécurité ouvert et tracé.
    IncidentOpened {
        /// Catégorie d'incident.
        category: String,
        /// Détail expurgé.
        detail: String,
    },
    /// Politique de sécurité mise à jour.
    PolicyUpdated {
        /// Projet concerné.
        project_id: Option<ProjectId>,
        /// Nouvelle révision.
        policy_revision: Revision,
        /// Mode d'isolation.
        isolation_mode: IsolationMode,
        /// Acteur ayant appliqué la modification.
        actor: String,
    },
}

/// Effective policy loaded by a trusted durable service for one exact scope.
/// This is internal initialization data, never a client codec or an authority grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialEffectivePolicy {
    /// Exact project scope, or the global user scope.
    pub project_id: Option<ProjectId>,
    /// Effective max of applicable policy revisions, without fabricating an update.
    pub policy_revision: Revision,
    /// Actual scope revision retained by the original producer.
    pub scope_revision: Revision,
    /// Digest of the complete global/project/Guard snapshot; revalidated by the journal writer.
    pub policy_digest: Sha256Digest,
    /// Strict if any applicable policy requires it.
    pub isolation_mode: IsolationMode,
    /// Actual configured mode; unsupported Guard integration must remain closed.
    pub guard_mode: GuardMode,
    /// Exact authority incarnation of the durable snapshot.
    pub lineage_fence: LineageFence,
}

/// Configuration d'initialisation de [`ActionBroker`].
#[derive(Clone)]
pub struct BrokerConfig {
    /// Barrière d'autorité courante (`AuthorityGeneration`, `DaemonEpoch`, `Fence`).
    pub current_fence: LineageFence,
    /// Délai maximal d'évaluation `GuardPort` (défaut `2 000 ms`).
    pub evaluate_timeout: Duration,
    /// Intervalle de santé `CheckHealth` (défaut `30 000 ms`).
    pub health_check_interval: Duration,
    /// Capacité maximale du cache de décisions.
    pub cache_max_entries: usize,
    /// TTL du cache borné (plafonné à 300 s).
    pub cache_ttl: Duration,
    /// Canaris connus à rédiger avant toute persistance d'audit ou log.
    pub canaries_to_redact: Vec<SecretString>,
}

impl BrokerConfig {
    /// Construit une configuration de test déterministe avec les défauts canoniques (2 000 ms, 300 s).
    #[must_use]
    pub fn for_test(current_fence: LineageFence) -> Self {
        Self {
            current_fence,
            evaluate_timeout: Duration::from_millis(DEFAULT_EVALUATE_TIMEOUT_MS),
            health_check_interval: Duration::from_millis(DEFAULT_HEALTH_CHECK_INTERVAL_MS),
            cache_max_entries: 1024,
            cache_ttl: MAX_DECISION_CACHE_TTL,
            canaries_to_redact: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct PolicyEntry {
    revision: Revision,
    isolation_mode: IsolationMode,
    updated_by: Option<String>,
    last_reason: Option<String>,
}

impl Default for PolicyEntry {
    fn default() -> Self {
        Self {
            revision: Revision::INITIAL,
            isolation_mode: IsolationMode::Standard,
            updated_by: None,
            last_reason: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SubjectGovernance {
    suspended: bool,
    revoked: bool,
    revision: Revision,
}

impl Default for SubjectGovernance {
    fn default() -> Self {
        Self {
            suspended: false,
            revoked: false,
            revision: Revision::INITIAL,
        }
    }
}

#[derive(Debug, Clone)]
struct ActionRecord {
    evaluation: AuthorizationEvaluation,
    intent: ActionIntent,
    state: ActionState,
    revision: Revision,
    decision: Option<AuthorizationDecision>,
    pending_approval_scope: Option<ApprovalScopeId>,
    verified_paths: Vec<(PathBuf, PathBuf)>,
    dispatches: Vec<(DispatchId, LineageFence)>,
    enforcement_refs: Vec<String>,
    committed_at_ms: Option<TimestampMs>,
    denial_or_unknown_reason: Option<String>,
    mapping_profile_id: String,
    guard_revision: Revision,
    cache_generation: Revision,
    journal_authorization_pending: bool,
    observation_in_progress: bool,
    observation_revision: Revision,
    pending_observation: Option<BrokerJournalEntry>,
    pending_approval: Option<PendingApproval>,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    entry: BrokerJournalEntry,
    decision: ApprovalDecision,
    expected_revision: Revision,
}

#[derive(Debug, Clone)]
struct EvaluationAttempt {
    context: AuthorizationEvaluation,
    intent: ActionIntent,
    intent_committed: bool,
    evaluating: bool,
    result: Option<Result<AuthorizationOutcome, ActionBrokerError>>,
    result_entry: Option<BrokerJournalEntry>,
    result_committed: bool,
    result_observed_at: Option<(TimestampMs, Duration)>,
    retry_after: Option<Duration>,
}

struct BrokerState {
    current_fence: LineageFence,
    initial_policy: Option<InitialEffectivePolicy>,
    global_policy: PolicyEntry,
    project_policies: BTreeMap<ProjectId, PolicyEntry>,
    scope_revisions: BTreeMap<Option<ProjectId>, Revision>,
    guard_mode: GuardMode,
    guard_endpoint: Option<String>,
    guard_port: Option<Arc<dyn GuardPort>>,
    guard_capabilities: Option<GuardCapabilities>,
    guard_health: GuardHealthStatus,
    mapping_profile: GuardMappingProfile,
    active_executor: ExecutorKind,
    subjects: BTreeMap<String, SubjectGovernance>,
    actions: BTreeMap<ActionId, ActionRecord>,
    cache: DecisionCache,
    events: Vec<BrokerAuditEvent>,
    guard_revision: Revision,
    cache_generation: Revision,
    authorizations_in_progress: BTreeSet<ActionId>,
    evaluations: BTreeMap<ActionId, EvaluationAttempt>,
}

/// Médiateur unique et déterministe des effets d'actions d'OVPT Unleash (`F017`, `ADR-10`).
pub struct ActionBroker {
    clock: Arc<dyn ClockPort>,
    monotonic_clock: Arc<dyn MonotonicClockPort>,
    sleep_port: Arc<dyn SleepPort>,
    ids: Arc<dyn IdGeneratorPort>,
    isolation_probe: Arc<dyn IsolationProbePort>,
    obligation_checker: Arc<dyn ObligationCapabilityPort>,
    fallback_policy: Arc<dyn FallbackPolicyPort>,
    evaluate_timeout_ms: AtomicU64,
    canaries: Vec<SecretString>,
    state: Mutex<BrokerState>,
    journal: Arc<dyn BrokerJournalPort>,
}

enum PendingJournalPhase {
    Authorization,
    Observation,
}

struct PendingJournal<'a> {
    broker: &'a ActionBroker,
    action_id: ActionId,
    phase: PendingJournalPhase,
}

impl Drop for PendingJournal<'_> {
    fn drop(&mut self) {
        let mut st = self.broker.lock_state();
        match self.phase {
            PendingJournalPhase::Authorization => {
                st.authorizations_in_progress.remove(&self.action_id);
                let cancelled = if let Some(attempt) = st.evaluations.get_mut(&self.action_id) {
                    if attempt.evaluating && attempt.result.is_none() {
                        let result = Err(ActionBrokerError::GuardUnavailable {
                            reason: "Evaluation cancelled without a usable decision".into(),
                        });
                        attempt.result_entry = Some(BrokerJournalEntry::Authorization {
                            evaluation: attempt.context.clone(),
                            cause: AuthorizationCause::Evaluation,
                            intent: attempt.intent.clone(),
                            outcome: result.clone(),
                            retry_not_before_ms: self
                                .broker
                                .authorization_retry_not_before_ms(&result),
                        });
                        attempt.result = Some(result);
                        attempt.evaluating = false;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if cancelled && let Some(rec) = st.actions.get_mut(&self.action_id) {
                    rec.state = ActionState::Authorizing;
                    rec.decision = None;
                    rec.journal_authorization_pending = true;
                }
            }
            PendingJournalPhase::Observation => {
                if let Some(rec) = st.actions.get_mut(&self.action_id)
                    && rec.observation_in_progress
                {
                    rec.observation_in_progress = false;
                    rec.state = ActionState::OutcomeUnknown;
                    rec.denial_or_unknown_reason =
                        Some("Observation commit cancelled; reconcile the ledger".into());
                }
            }
        }
    }
}

impl ActionBroker {
    /// Initialise un nouveau [`ActionBroker`] avec ses ports injectables sans état global caché.
    #[expect(
        clippy::too_many_arguments,
        reason = "Injection explicite de tous les ports requis pour un déterminisme strict sans I/O implicite"
    )]
    #[must_use]
    pub fn new(
        clock: Arc<dyn ClockPort>,
        monotonic_clock: Arc<dyn MonotonicClockPort>,
        sleep_port: Arc<dyn SleepPort>,
        ids: Arc<dyn IdGeneratorPort>,
        isolation_probe: Arc<dyn IsolationProbePort>,
        obligation_checker: Arc<dyn ObligationCapabilityPort>,
        fallback_policy: Arc<dyn FallbackPolicyPort>,
        config: BrokerConfig,
        journal: Arc<dyn BrokerJournalPort>,
    ) -> Self {
        let cache = DecisionCache::new(config.cache_max_entries, config.cache_ttl);
        Self {
            clock,
            monotonic_clock,
            sleep_port,
            ids,
            isolation_probe,
            obligation_checker,
            fallback_policy,
            evaluate_timeout_ms: AtomicU64::new(
                u64::try_from(config.evaluate_timeout.as_millis())
                    .unwrap_or(u64::MAX)
                    .max(1),
            ),
            canaries: config.canaries_to_redact,
            journal,
            state: Mutex::new(BrokerState {
                current_fence: config.current_fence,
                initial_policy: None,
                global_policy: PolicyEntry::default(),
                project_policies: BTreeMap::new(),
                scope_revisions: BTreeMap::new(),
                guard_mode: GuardMode::BrokerOnly,
                guard_endpoint: None,
                guard_port: None,
                guard_capabilities: None,
                guard_health: GuardHealthStatus::Disconnected,
                mapping_profile: GuardMappingProfile::NoneAccepted,
                active_executor: ExecutorKind::Unleash,
                subjects: BTreeMap::new(),
                actions: BTreeMap::new(),
                cache,
                events: Vec::new(),
                guard_revision: Revision::INITIAL,
                cache_generation: Revision::INITIAL,
                authorizations_in_progress: BTreeSet::new(),
                evaluations: BTreeMap::new(),
            }),
        }
    }

    /// Constructs a new broker bound to one durable effective policy snapshot.
    /// No admin mutation or revision increment is synthesized.
    /// # Errors
    /// Rejects an unqualified snapshot; no broker or journal acknowledgement is returned.
    #[expect(
        clippy::too_many_arguments,
        reason = "Explicit constructor injection of existing ports plus the durable policy snapshot"
    )]
    pub fn new_with_policy_snapshot(
        clock: Arc<dyn ClockPort>,
        monotonic_clock: Arc<dyn MonotonicClockPort>,
        sleep_port: Arc<dyn SleepPort>,
        ids: Arc<dyn IdGeneratorPort>,
        isolation_probe: Arc<dyn IsolationProbePort>,
        obligation_checker: Arc<dyn ObligationCapabilityPort>,
        fallback_policy: Arc<dyn FallbackPolicyPort>,
        config: BrokerConfig,
        journal: Arc<dyn BrokerJournalPort>,
        snapshot: InitialEffectivePolicy,
    ) -> Result<Self, ActionBrokerError> {
        if config.current_fence != snapshot.lineage_fence {
            return Err(ActionBrokerError::StaleFence {
                reason: "Initial policy belongs to a different authority incarnation".into(),
                lineage_mismatch: config.current_fence.authority_generation
                    != snapshot.lineage_fence.authority_generation,
            });
        }
        // This constructor has no qualified GuardPort. Never silently replace
        // Required/Optional by BrokerOnly, nor authorize their fallback here.
        if !matches!(&snapshot.guard_mode, GuardMode::BrokerOnly) {
            return Err(ActionBrokerError::GuardUnavailable {
                reason: "Initial Guard policy requires a qualified port".into(),
            });
        }
        let broker = Self::new(
            clock,
            monotonic_clock,
            sleep_port,
            ids,
            isolation_probe,
            obligation_checker,
            fallback_policy,
            config,
            journal,
        );
        {
            // The new instance has not escaped this constructor: no caller,
            // evaluation, cache entry or live action can be overwritten.
            let mut state = broker.lock_state();
            let effective = PolicyEntry {
                revision: snapshot.policy_revision,
                isolation_mode: snapshot.isolation_mode,
                ..PolicyEntry::default()
            };
            if let Some(project) = snapshot.project_id {
                state.project_policies.insert(project, effective);
            } else {
                state.global_policy = effective;
            }
            state
                .scope_revisions
                .insert(snapshot.project_id, snapshot.scope_revision);
            state.guard_mode = snapshot.guard_mode.clone();
            state.initial_policy = Some(snapshot);
        }
        Ok(broker)
    }

    /// Creates an explicitly non-durable broker for local tests and examples only.
    /// A daemon must use a durable constructor with its real store journal.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "Explicit test port injection mirrors the durable constructor"
    )]
    pub fn new_for_test_memory(
        clock: Arc<dyn ClockPort>,
        monotonic_clock: Arc<dyn MonotonicClockPort>,
        sleep_port: Arc<dyn SleepPort>,
        ids: Arc<dyn IdGeneratorPort>,
        isolation_probe: Arc<dyn IsolationProbePort>,
        obligation_checker: Arc<dyn ObligationCapabilityPort>,
        fallback_policy: Arc<dyn FallbackPolicyPort>,
        config: BrokerConfig,
    ) -> Self {
        Self::new(
            clock,
            monotonic_clock,
            sleep_port,
            ids,
            isolation_probe,
            obligation_checker,
            fallback_policy,
            config,
            Arc::new(TestMemoryJournal),
        )
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, BrokerState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn authorization_retry_not_before_ms(
        &self,
        outcome: &Result<AuthorizationOutcome, ActionBrokerError>,
    ) -> Option<TimestampMs> {
        matches!(outcome, Err(ActionBrokerError::GuardUnavailable { .. })).then(|| {
            let backoff_ms =
                i64::try_from(AUTHORIZATION_RETRY_BACKOFF.as_millis()).unwrap_or(i64::MAX);
            TimestampMs::new(self.clock.now_ms().as_i64().saturating_add(backoff_ms))
        })
    }

    fn evaluation_for_action(
        &self,
        action_id: ActionId,
    ) -> Result<AuthorizationEvaluation, ActionBrokerError> {
        self.lock_state()
            .actions
            .get(&action_id)
            .map(|record| record.evaluation.clone())
            .ok_or_else(|| ActionBrokerError::NotFound {
                resource: action_id.to_string(),
            })
    }

    fn sanitize(&self, text: &str) -> String {
        if self.canaries.is_empty() {
            return text.to_owned();
        }
        let refs: Vec<&SecretString> = self.canaries.iter().collect();
        redact_canaries(text, &refs).redacted_text
    }

    fn contains_sensitive_value(&self, value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::String(text) => self.sanitize(text) != *text,
            serde_json::Value::Array(values) => values
                .iter()
                .any(|value| self.contains_sensitive_value(value)),
            serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
                self.sanitize(key) != *key || self.contains_sensitive_value(value)
            }),
            _ => false,
        }
    }

    async fn call_guard<T>(
        &self,
        future: BoxFuture<'_, Result<T, GuardPortError>>,
    ) -> Result<T, GuardPortError> {
        let timeout_ms = self.evaluate_timeout_ms.load(Ordering::Acquire);
        let timer = self.sleep_port.sleep(Duration::from_millis(timeout_ms));
        tokio::select! { result = future => result, () = timer => Err(GuardPortError::Timeout { timeout_ms }) }
    }

    /// Met à jour le fence courant (utile lors de réélection/epoch ou tests de fencing).
    pub fn set_current_fence(&self, fence: LineageFence) {
        let mut st = self.lock_state();
        st.current_fence = fence;
        st.cache_generation = st.cache_generation.next();
    }

    /// Met à jour la révision de scope et invalide les décisions en cache concernées.
    pub fn bump_scope_revision(&self, project_id: Option<ProjectId>, new_revision: Revision) {
        let mut st = self.lock_state();
        st.scope_revisions.insert(project_id, new_revision);
        st.cache.invalidate_project(project_id);
        st.cache_generation = st.cache_generation.next();
    }

    /// Rejette toute tentative de mutation d'autorité provenant d'une source externe non fiable
    /// (`REQ-SEC-006`) et ouvre un incident auditable.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::PermissionExpansionDenied`] ou [`ActionBrokerError::OperationNotAuthorized`].
    pub fn validate_untrusted_command_proposal(
        &self,
        caller: &CallerIdentity,
        provenance: &ActionProvenance,
        reason: &str,
    ) -> Result<(), ActionBrokerError> {
        if let Err(err) = validate_authority_mutation_provenance(provenance) {
            let mut st = self.lock_state();
            let detail = self.sanitize(&format!(
                "Tentative de mutation d'autorité depuis source externe refusée ({reason}): {err}"
            ));
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("untrusted_authority_mutation"),
                detail,
            });
            return Err(err);
        }
        caller.require_admin(
            BrokerAdminCapability::ManagePolicy,
            "validate_untrusted_command_proposal",
        )
    }

    /// Met à jour la politique de sécurité (`SetPolicy`, `isolation_mode = Standard | Strict`)
    /// avec contrôle optimiste de révision et invalidation instantanée du cache (`REQ-SEC-005`, `REQ-SEC-007`).
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `RevisionConflict` ou `InvalidArgument`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Owned administrative command matches the other mutation boundaries"
    )]
    pub fn set_policy(
        &self,
        caller: &CallerIdentity,
        cmd: SetPolicyCommand,
    ) -> Result<PolicySummary, ActionBrokerError> {
        if let Err(err) = caller.require_admin(BrokerAdminCapability::ManagePolicy, "SetPolicy") {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_admin_command"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        if cmd.reason.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("SetPolicy exige un motif non vide"),
            });
        }

        let sanitized_reason = self.sanitize(&cmd.reason);
        let mut st = self.lock_state();
        let next_rev = {
            let entry = match cmd.project_id {
                Some(pid) => st.project_policies.entry(pid).or_default(),
                None => &mut st.global_policy,
            };
            if entry.revision != cmd.expected_revision {
                return Err(ActionBrokerError::RevisionConflict {
                    current_revision: entry.revision,
                    expected_revision: cmd.expected_revision,
                });
            }
            entry.revision = entry.revision.next();
            entry.isolation_mode = cmd.isolation_mode;
            entry.updated_by = Some(caller.actor_id.to_string());
            entry.last_reason = Some(sanitized_reason);
            entry.revision
        };

        st.cache.invalidate_project(cmd.project_id);
        st.cache_generation = st.cache_generation.next();
        st.events.push(BrokerAuditEvent::PolicyUpdated {
            project_id: cmd.project_id,
            policy_revision: next_rev,
            isolation_mode: cmd.isolation_mode,
            actor: caller.actor_id.to_string(),
        });

        Ok(build_policy_summary(&st, cmd.project_id))
    }

    /// Lit le résumé courant de politique sans effet de bord (`ReadPolicySummary`).
    #[must_use]
    pub fn read_policy_summary(&self, project_id: Option<ProjectId>) -> PolicySummary {
        let st = self.lock_state();
        build_policy_summary(&st, project_id)
    }

    /// Configure le endpoint et le mode AI-Guard (`ConfigureGuard`, `Q-04`).
    ///
    /// - Sans endpoint : le mode effectif vaut `BrokerOnly`.
    /// - Dès qu'un endpoint est configuré : le mode par défaut devient `GuardRequired` (fail-closed).
    /// - `GuardOptional` exige une `fallback_policy_ref` non vide.
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `InvalidArgument`, ou `GuardProtocolUnsupported`.
    #[expect(
        clippy::too_many_lines,
        reason = "Configuration validates mode, negotiates capabilities and atomically replaces the complete guard state"
    )]
    pub async fn configure_guard(
        &self,
        caller: &CallerIdentity,
        cmd: ConfigureGuardCommand,
        guard_port: Option<Arc<dyn GuardPort>>,
    ) -> Result<GuardHealthSnapshot, ActionBrokerError> {
        if let Err(err) =
            caller.require_admin(BrokerAdminCapability::ConfigureGuard, "ConfigureGuard")
        {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_admin_command"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        let target_mode = match (&cmd.endpoint, cmd.guard_mode) {
            (None, None | Some(GuardMode::BrokerOnly)) => GuardMode::BrokerOnly,
            (None, Some(_)) => {
                return Err(ActionBrokerError::InvalidArgument {
                    reason: String::from(
                        "Impossible d'activer guard_required ou guard_optional sans endpoint configuré",
                    ),
                });
            }
            (Some(ep), mode_opt) => {
                validate_guard_endpoint(ep)?;
                match mode_opt {
                    None => GuardMode::GuardRequired,
                    Some(GuardMode::GuardOptional {
                        ref fallback_policy_ref,
                    }) if fallback_policy_ref.trim().is_empty() => {
                        return Err(ActionBrokerError::InvalidArgument {
                            reason: String::from(
                                "guard_optional exige une fallback_policy_ref préautorisée non vide",
                            ),
                        });
                    }
                    Some(m) => m,
                }
            }
        };

        let requested_profile = cmd
            .mapping_profile
            .unwrap_or(GuardMappingProfile::NoneAccepted);
        let requested_executor = cmd.executor_kind.unwrap_or(ExecutorKind::Unleash);
        if cmd.evaluate_timeout_ms == Some(0) {
            return Err(ActionBrokerError::InvalidArgument {
                reason: "Guard timeout must be positive".into(),
            });
        }

        let negotiated_caps = if let Some(ref port) = guard_port {
            let neg_req = NegotiateCapabilitiesRequest {
                supported_protocol_versions: vec![String::from(
                    crate::guard_port::GUARD_PORT_PROTOCOL_VERSION,
                )],
                requested_mapping_profile: requested_profile.clone(),
                requested_executor,
            };
            let caps = self.call_guard(port.negotiate_capabilities(neg_req)).await.map_err(|e| match e {
                GuardPortError::IncompatibleProtocol { expected, observed } => {
                    ActionBrokerError::GuardProtocolUnsupported {
                        reason: format!(
                            "Version incompatible : attendue '{expected}', observée '{observed}'"
                        ),
                    }
                }
                GuardPortError::UnsupportedCapability { capability, .. } => {
                    ActionBrokerError::GuardProtocolUnsupported {
                        reason: format!("Capacité non prise en charge : '{capability}'"),
                    }
                }
                other => ActionBrokerError::GuardUnavailable {
                    reason: other.to_string(),
                },
            })?;
            if caps.protocol_version != crate::guard_port::GUARD_PORT_PROTOCOL_VERSION
                || caps.mapping_profile != requested_profile
            {
                return Err(ActionBrokerError::GuardProtocolUnsupported {
                    reason: "Le profil ou la version négociée diffère de la demande".into(),
                });
            }
            Some(caps)
        } else {
            None
        };

        let health_status = if let Some(ref port) = guard_port {
            match self.call_guard(port.check_health()).await {
                Ok(rep) => rep.status,
                Err(_) => GuardHealthStatus::Unavailable,
            }
        } else {
            GuardHealthStatus::Disconnected
        };

        let mut st = self.lock_state();
        st.guard_revision = st.guard_revision.next();
        if let Some(timeout_ms) = cmd.evaluate_timeout_ms {
            self.evaluate_timeout_ms
                .store(timeout_ms, Ordering::Release);
        }
        st.guard_endpoint = cmd.endpoint;
        st.guard_mode = target_mode;
        st.guard_port = guard_port;
        st.guard_health = health_status;

        if let Some(ref caps) = negotiated_caps {
            st.mapping_profile = caps.mapping_profile.clone();
            st.active_executor = if requested_executor == ExecutorKind::Guard
                && caps.supports_execute_authorized_action
            {
                ExecutorKind::Guard
            } else {
                ExecutorKind::Unleash
            };
        } else {
            st.mapping_profile = requested_profile;
            st.active_executor = ExecutorKind::Unleash;
        }
        st.guard_capabilities = negotiated_caps;

        if let Some(ttl_secs) = cmd.cache_ttl_secs {
            st.cache.set_configured_ttl(Duration::from_secs(ttl_secs));
        }
        st.cache.clear();
        st.cache_generation = st.cache_generation.next();
        let health_detail = format!("guard configured in mode {:?}", st.guard_mode);
        st.events.push(BrokerAuditEvent::GuardHealthChanged {
            status: health_status,
            detail: health_detail,
        });

        Ok(build_guard_health_snapshot(&st))
    }

    /// Lit l'état de santé, les capacités négociées et les limites visibles de AI-Guard (`ReadGuardHealth`).
    ///
    /// # Errors
    /// Retourne une erreur interne uniquement si le verrou est corrompu (jamais ici).
    pub async fn read_guard_health(&self) -> Result<GuardHealthSnapshot, ActionBrokerError> {
        let port = {
            let st = self.lock_state();
            st.guard_port.clone()
        };

        if let Some(p) = port {
            let live_status = match self.call_guard(p.check_health()).await {
                Ok(rep) => rep.status,
                Err(_) => GuardHealthStatus::Unavailable,
            };
            let mut st = self.lock_state();
            st.guard_health = live_status;
            return Ok(build_guard_health_snapshot(&st));
        }

        let st = self.lock_state();
        Ok(build_guard_health_snapshot(&st))
    }

    /// Suspend localement et immédiatement un sujet typé (`SuspendSubject`), invalide son cache
    /// et propage la suspension à `GuardPort` uniquement si négocié (`F017` R7).
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `RevisionConflict`, `SubjectRevoked` ou `InvalidArgument`.
    pub async fn suspend_subject(
        &self,
        caller: &CallerIdentity,
        cmd: SuspendSubjectCommand,
    ) -> Result<SubjectSuspensionReceipt, ActionBrokerError> {
        if let Err(err) = caller.require_admin(
            BrokerAdminCapability::SuspendResumeSubject,
            "SuspendSubject",
        ) {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_admin_command"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        if cmd.reason.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("SuspendSubject exige un motif non vide"),
            });
        }

        let key = cmd.subject.canonical_key();
        let sanitized_reason = self.sanitize(&cmd.reason);
        let (next_rev, in_flight, guard_opt, supports_remote) = {
            let mut st = self.lock_state();
            let gov = st.subjects.entry(key.clone()).or_default();
            if gov.revoked {
                return Err(ActionBrokerError::SubjectRevoked {
                    subject: key,
                    reason: String::from("Sujet déjà définitivement révoqué"),
                });
            }
            if gov.revision != cmd.expected_revision {
                return Err(ActionBrokerError::RevisionConflict {
                    current_revision: gov.revision,
                    expected_revision: cmd.expected_revision,
                });
            }
            gov.suspended = true;
            gov.revision = gov.revision.next();
            let next_rev = gov.revision;

            st.cache.invalidate_subject(&key);
            st.cache_generation = st.cache_generation.next();
            let in_flight: Vec<ActionId> = st
                .actions
                .iter()
                .filter(|(_, rec)| {
                    matches!(rec.state, ActionState::Dispatching | ActionState::InFlight)
                        && rec
                            .intent
                            .subject
                            .matching_subject_refs()
                            .iter()
                            .any(|r| r.canonical_key() == key)
                })
                .map(|(id, _)| *id)
                .collect();

            st.events.push(BrokerAuditEvent::SubjectSuspended {
                subject: key.clone(),
                reason: sanitized_reason.clone(),
            });

            let supports_remote = st
                .guard_capabilities
                .as_ref()
                .is_some_and(|c| c.supports_suspend_resume);
            (next_rev, in_flight, st.guard_port.clone(), supports_remote)
        };

        let remote_propagation = match guard_opt {
            None => RemotePropagationStatus::NoGuardConfigured,
            Some(_) if !supports_remote => RemotePropagationStatus::UnsupportedByGuard,
            Some(port) => {
                let req = GuardSubjectActionRequest {
                    subject_key: key,
                    project_id: cmd.project_id,
                    reason: sanitized_reason,
                    revision: next_rev,
                };
                match self.call_guard(port.suspend_subject(&req)).await {
                    Ok(_) => RemotePropagationStatus::Propagated,
                    Err(e) => RemotePropagationStatus::PropagationFailed {
                        reason: e.to_string(),
                    },
                }
            }
        };

        Ok(SubjectSuspensionReceipt {
            subject: cmd.subject,
            suspended: true,
            revision: next_rev,
            in_flight_actions_to_reconcile: in_flight,
            remote_propagation,
        })
    }

    /// Lève la suspension locale d'un sujet sans changer son identité (`ResumeSubject`, `F017` R7).
    /// Refuse si le sujet a été définitivement révoqué (`SUBJECT_REVOKED`).
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `RevisionConflict`, `SubjectRevoked` ou `InvalidArgument`.
    pub async fn resume_subject(
        &self,
        caller: &CallerIdentity,
        cmd: ResumeSubjectCommand,
    ) -> Result<SubjectSuspensionReceipt, ActionBrokerError> {
        if let Err(err) =
            caller.require_admin(BrokerAdminCapability::SuspendResumeSubject, "ResumeSubject")
        {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_admin_command"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        if cmd.reason.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("ResumeSubject exige un motif non vide"),
            });
        }

        let key = cmd.subject.canonical_key();
        let sanitized_reason = self.sanitize(&cmd.reason);
        let (next_rev, guard_opt, supports_remote) = {
            let mut st = self.lock_state();
            let gov = st.subjects.entry(key.clone()).or_default();
            if gov.revoked {
                return Err(ActionBrokerError::SubjectRevoked {
                    subject: key,
                    reason: String::from(
                        "Impossible de lever la suspension : le sujet est définitivement révoqué",
                    ),
                });
            }
            if gov.revision != cmd.expected_revision {
                return Err(ActionBrokerError::RevisionConflict {
                    current_revision: gov.revision,
                    expected_revision: cmd.expected_revision,
                });
            }
            gov.suspended = false;
            gov.revision = gov.revision.next();
            let next_rev = gov.revision;

            st.cache.invalidate_subject(&key);
            st.cache_generation = st.cache_generation.next();
            st.events.push(BrokerAuditEvent::SubjectResumed {
                subject: key.clone(),
                reason: sanitized_reason.clone(),
            });
            let supports_remote = st
                .guard_capabilities
                .as_ref()
                .is_some_and(|c| c.supports_suspend_resume);
            (next_rev, st.guard_port.clone(), supports_remote)
        };

        let remote_propagation = match guard_opt {
            None => RemotePropagationStatus::NoGuardConfigured,
            Some(_) if !supports_remote => RemotePropagationStatus::UnsupportedByGuard,
            Some(port) => {
                let req = GuardSubjectActionRequest {
                    subject_key: key,
                    project_id: cmd.project_id,
                    reason: sanitized_reason,
                    revision: next_rev,
                };
                match self.call_guard(port.resume_subject(&req)).await {
                    Ok(_) => RemotePropagationStatus::Propagated,
                    Err(e) => RemotePropagationStatus::PropagationFailed {
                        reason: e.to_string(),
                    },
                }
            }
        };

        Ok(SubjectSuspensionReceipt {
            subject: cmd.subject,
            suspended: false,
            revision: next_rev,
            in_flight_actions_to_reconcile: Vec::new(),
            remote_propagation,
        })
    }

    /// Révoque définitivement un sujet (`RevokeSubject`), invalide son cache immédiatement
    /// et propage la révocation au `GuardPort` en journalisant tout échec distant comme incident.
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `RevisionConflict` ou `InvalidArgument`.
    pub async fn revoke_subject(
        &self,
        caller: &CallerIdentity,
        cmd: RevokeSubjectCommand,
    ) -> Result<SubjectRevocationReceipt, ActionBrokerError> {
        if let Err(err) =
            caller.require_admin(BrokerAdminCapability::RevokeSubject, "RevokeSubject")
        {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_admin_command"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        if cmd.reason.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("RevokeSubject exige un motif non vide"),
            });
        }

        let key = cmd.subject.canonical_key();
        let sanitized_reason = self.sanitize(&cmd.reason);
        let (next_rev, guard_opt) = {
            let mut st = self.lock_state();
            let gov = st.subjects.entry(key.clone()).or_default();
            if gov.revision != cmd.expected_revision {
                return Err(ActionBrokerError::RevisionConflict {
                    current_revision: gov.revision,
                    expected_revision: cmd.expected_revision,
                });
            }
            gov.revoked = true;
            gov.suspended = false;
            gov.revision = gov.revision.next();
            let next_rev = gov.revision;

            st.cache.invalidate_subject(&key);
            st.cache_generation = st.cache_generation.next();
            st.events.push(BrokerAuditEvent::SubjectRevoked {
                subject: key.clone(),
                reason: sanitized_reason.clone(),
            });
            (next_rev, st.guard_port.clone())
        };

        let mut remote_ref = None;
        let remote_propagation = match guard_opt {
            None => RemotePropagationStatus::NoGuardConfigured,
            Some(port) => {
                let req = GuardSubjectActionRequest {
                    subject_key: key.clone(),
                    project_id: cmd.project_id,
                    reason: sanitized_reason,
                    revision: next_rev,
                };
                match self.call_guard(port.revoke_subject(&req)).await {
                    Ok(receipt) => {
                        remote_ref = Some(receipt.remote_ref);
                        RemotePropagationStatus::Propagated
                    }
                    Err(e) => {
                        let mut st = self.lock_state();
                        st.events.push(BrokerAuditEvent::IncidentOpened {
                            category: String::from("guard_revocation_propagation_failed"),
                            detail: self.sanitize(&format!(
                                "Révocation locale appliquée pour '{key}', propagation AI-Guard échouée : {e}"
                            )),
                        });
                        RemotePropagationStatus::PropagationFailed {
                            reason: e.to_string(),
                        }
                    }
                }
            }
        };

        Ok(SubjectRevocationReceipt {
            subject: cmd.subject,
            revision: next_rev,
            remote_propagation,
            remote_ref,
        })
    }

    /// Enregistre la décision humaine d'approbation (`ResolveActionApproval`) sur le périmètre
    /// et le `payload_digest` exacts d'une action `approval_required`.
    ///
    /// # Errors
    /// Retourne `OperationNotAuthorized`, `NotFound`, `RevisionConflict`, `ContractViolation`,
    /// `SubjectSuspended`, `SubjectRevoked` ou `DomainTransitionError` projetée.
    #[expect(
        clippy::too_many_lines,
        reason = "Exact pending human-decision replay with receipt-bound provenance"
    )]
    pub async fn resolve_action_approval(
        &self,
        caller: &CallerIdentity,
        cmd: ResolveActionApprovalCommand,
    ) -> Result<AuthorizationDecision, ActionBrokerError> {
        let id = cmd.action_id;
        if let Err(error) = caller.require_admin(
            BrokerAdminCapability::ResolveActionApproval,
            "ResolveActionApproval",
        ) {
            self.lock_state()
                .events
                .push(BrokerAuditEvent::IncidentOpened {
                    category: "unauthorized_approval_attempt".into(),
                    detail: self.sanitize(&error.to_string()),
                });
            return Err(error);
        }
        let cause = AuthorizationCause::HumanApproval {
            actor_id: caller.actor_id.clone(),
            decision_receipt_id: cmd.decision_receipt_id,
            decision_command_digest: cmd.decision_command_digest,
            approval_scope: cmd.approval_scope,
        };
        let pending = {
            let mut st = self.lock_state();
            if st.authorizations_in_progress.contains(&id) {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "Authorization commit in progress".into(),
                    terminal_immutable: false,
                });
            }
            let rec = st
                .actions
                .get(&id)
                .ok_or_else(|| ActionBrokerError::NotFound {
                    resource: id.to_string(),
                })?;
            let pending = rec.pending_approval.clone();
            if let Some(pending) = &pending {
                let matches = matches!(&pending.entry, BrokerJournalEntry::Authorization { cause: recorded, intent, .. }
                    if recorded == &cause && intent.payload_digest == cmd.payload_digest);
                if !matches
                    || pending.decision != cmd.decision
                    || pending.expected_revision != cmd.expected_revision
                {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: "Human decision differs from uncertain commit".into(),
                        terminal_immutable: false,
                    });
                }
            }
            st.authorizations_in_progress.insert(id);
            pending
        };
        let _pending = PendingJournal {
            broker: self,
            action_id: id,
            phase: PendingJournalPhase::Authorization,
        };
        let entry = if let Some(pending) = pending {
            pending.entry
        } else {
            let decision = self.resolve_action_approval_inner(caller, &cmd)?;
            let outcome = if cmd.decision == ApprovalDecision::Deny {
                Err(ActionBrokerError::GuardDenied {
                    reason_code: "HUMAN_DENIED".into(),
                    detail: "Human approval denied".into(),
                    decision_ref: Some(decision.decision_ref.to_string()),
                })
            } else {
                Ok(AuthorizationOutcome::Authorized(decision))
            };
            let mut st = self.lock_state();
            let rec = st
                .actions
                .get_mut(&id)
                .ok_or_else(|| ActionBrokerError::NotFound {
                    resource: id.to_string(),
                })?;
            let entry = BrokerJournalEntry::Authorization {
                evaluation: rec.evaluation.clone(),
                cause,
                intent: rec.intent.clone(),
                outcome,
                retry_not_before_ms: None,
            };
            rec.pending_approval = Some(PendingApproval {
                entry: entry.clone(),
                decision: cmd.decision,
                expected_revision: cmd.expected_revision,
            });
            entry
        };
        self.journal
            .append(&entry)
            .await
            .map_err(|_| ActionBrokerError::ContractViolation {
                reason: "Approval journal commit uncertain; retain receipt-bound decision".into(),
                terminal_immutable: false,
            })?;
        let BrokerJournalEntry::Authorization { outcome, .. } = entry else {
            return Err(ActionBrokerError::ContractViolation {
                reason: "Expected retained approval".into(),
                terminal_immutable: false,
            });
        };
        let mut st = self.lock_state();
        if let Some(rec) = st.actions.get_mut(&id) {
            rec.pending_approval = None;
            rec.journal_authorization_pending = false;
        }
        let AuthorizationOutcome::Authorized(decision) = outcome? else {
            return Err(ActionBrokerError::ContractViolation {
                reason: "Human resolution did not authorize".into(),
                terminal_immutable: false,
            });
        };
        st.events.push(BrokerAuditEvent::ActionAuthorized {
            action_id: id,
            decision_ref: decision.decision_ref.clone(),
            fallback_policy_ref: decision.fallback_policy_ref.clone(),
        });
        Ok(decision)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Approval validates exact scope, digest and revision before its single state transition"
    )]
    fn resolve_action_approval_inner(
        &self,
        caller: &CallerIdentity,
        cmd: &ResolveActionApprovalCommand,
    ) -> Result<AuthorizationDecision, ActionBrokerError> {
        if let Err(err) = caller.require_admin(
            BrokerAdminCapability::ResolveActionApproval,
            "ResolveActionApproval",
        ) {
            let mut st = self.lock_state();
            st.events.push(BrokerAuditEvent::IncidentOpened {
                category: String::from("unauthorized_approval_attempt"),
                detail: self.sanitize(&err.to_string()),
            });
            return Err(err);
        }

        let mut st = self.lock_state();
        let evaluation_id = st
            .actions
            .get(&cmd.action_id)
            .map(|r| r.evaluation.evaluation_id)
            .ok_or_else(|| ActionBrokerError::NotFound {
                resource: cmd.action_id.to_string(),
            })?;
        let active_profile = st.mapping_profile.profile_id().to_owned();
        let active_executor = st.active_executor;

        let subject_check = {
            let rec =
                st.actions
                    .get(&cmd.action_id)
                    .ok_or_else(|| ActionBrokerError::NotFound {
                        resource: format!("action:{}", cmd.action_id),
                    })?;
            rec.intent.subject.matching_subject_refs()
        };
        check_subject_governance_locked(&st.subjects, &subject_check)?;

        let (action_id, policy_revision, scope_revision, payload_digest) = {
            let rec =
                st.actions
                    .get_mut(&cmd.action_id)
                    .ok_or_else(|| ActionBrokerError::NotFound {
                        resource: format!("action:{}", cmd.action_id),
                    })?;

            if rec.revision != cmd.expected_revision {
                return Err(ActionBrokerError::RevisionConflict {
                    current_revision: rec.revision,
                    expected_revision: cmd.expected_revision,
                });
            }

            if rec.state != ActionState::ApprovalRequired {
                return Err(ActionBrokerError::ContractViolation {
                    reason: format!(
                        "Action {} n'est pas en attente d'approbation (état={})",
                        cmd.action_id, rec.state
                    ),
                    terminal_immutable: rec.state.is_terminal(),
                });
            }
            if rec.journal_authorization_pending {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "Authorization journal commit pending".into(),
                    terminal_immutable: false,
                });
            }

            if rec.pending_approval_scope != Some(cmd.approval_scope) {
                return Err(ActionBrokerError::ContractViolation {
                    reason: format!(
                        "Incohérence d'approval_scope pour {} (attendu {:?}, reçu {})",
                        cmd.action_id, rec.pending_approval_scope, cmd.approval_scope
                    ),
                    terminal_immutable: false,
                });
            }

            if rec.intent.payload_digest != cmd.payload_digest {
                return Err(ActionBrokerError::ContractViolation {
                    reason: format!(
                        "Incohérence de payload_digest pour l'approbation de {}",
                        cmd.action_id
                    ),
                    terminal_immutable: false,
                });
            }

            let approved = matches!(cmd.decision, ApprovalDecision::Approve);
            let next_state =
                transition_action(rec.state, ActionEvent::HumanApprovalDecision { approved })?;
            rec.state = next_state;
            rec.revision = rec.revision.next();
            rec.pending_approval_scope = None;
            rec.journal_authorization_pending = true;
            (
                rec.intent.action_id,
                rec.intent.policy_revision,
                rec.intent.scope_revision,
                rec.intent.payload_digest,
            )
        };

        let approved = matches!(cmd.decision, ApprovalDecision::Approve);
        if approved {
            let dref = GuardDecisionRef::new(format!("approval:{}", self.ids.new_uuid()))?;
            let decision = AuthorizationDecision {
                evaluation_id,
                source_evaluation_id: None,
                action_id,
                decision_ref: dref.clone(),
                policy_revision,
                scope_revision,
                payload_digest,
                expiry: None,
                applied_obligations: Vec::new(),
                fallback_policy_ref: None,
                from_cache: false,
                executor: active_executor,
            };
            if let Some(rec) = st.actions.get_mut(&cmd.action_id) {
                rec.decision = Some(decision.clone());
                rec.mapping_profile_id = active_profile;
            }
            Ok(decision)
        } else {
            let reason = self.sanitize(&cmd.reason);
            if let Some(rec) = st.actions.get_mut(&cmd.action_id) {
                rec.denial_or_unknown_reason = Some(reason.clone());
            }
            st.events.push(BrokerAuditEvent::ActionDenied {
                action_id: cmd.action_id,
                reason,
            });
            Ok(AuthorizationDecision {
                evaluation_id,
                source_evaluation_id: None,
                action_id: cmd.action_id,
                decision_ref: GuardDecisionRef::new("denied_by_human")?,
                policy_revision,
                scope_revision,
                payload_digest,
                expiry: None,
                applied_obligations: Vec::new(),
                fallback_policy_ref: None,
                from_cache: false,
                executor: active_executor,
            })
        }
    }

    /// Soumet une intention brute et valide d'abord sa classe d'effet avant tout appel [`GuardPort`].
    ///
    /// # Sûreté à l'annulation
    /// Aucune écriture ni effet physique n'est engagé avant `Authorizing`. Une annulation laisse
    /// l'action non autorisée et sans dispatch.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::EffectsClassUnknown`] ou toute erreur d'autorisation.
    pub async fn authorize_raw_intent(
        &self,
        raw: UnvalidatedActionIntent,
        authority: &EffectiveAuthority,
    ) -> Result<AuthorizationOutcome, ActionBrokerError> {
        let intent = raw.validate()?;
        self.authorize_action(intent, authority).await
    }

    /// Évalue une [`ActionIntent`] contre l'autorité effective, le statut sujet, l'isolation et
    /// [`GuardPort`] (si configuré et `effects_class != LocalRead`).
    ///
    /// Aucune transaction de stockage ou verrou n'est maintenu pendant l'appel réseau [`GuardPort`].
    ///
    /// # Errors
    /// Retourne l'erreur F017 exacte (`PolicyDenied`, `ScopeDenied`, `SubjectSuspended`,
    /// `SubjectRevoked`, `IsolationBackendMissing`, `GuardDenied`, `GuardObligationUnsatisfied`,
    /// `GuardUnavailable`, `GuardProtocolUnsupported`, etc.).
    #[expect(
        clippy::too_many_lines,
        reason = "Serialized evaluation identity, exact journal retry and durable-result backoff"
    )]
    pub async fn authorize_action(
        &self,
        intent: ActionIntent,
        authority: &EffectiveAuthority,
    ) -> Result<AuthorizationOutcome, ActionBrokerError> {
        let action_id = intent.action_id;
        let evaluation = {
            let mut st = self.lock_state();
            if let Some(snapshot) = &st.initial_policy {
                validate_initial_policy_intent(snapshot, &intent)?;
            }
            if st.authorizations_in_progress.contains(&action_id) {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "Action authorization already in progress".into(),
                    terminal_immutable: false,
                });
            }
            let previous_id = if let Some(attempt) = st.evaluations.get(&action_id) {
                if attempt.intent != intent {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: "Evaluation intent cannot be replaced".into(),
                        terminal_immutable: false,
                    });
                }
                if attempt.result_committed {
                    let retryable = matches!(
                        attempt.result,
                        Some(Err(ActionBrokerError::GuardUnavailable { .. }))
                    );
                    if !retryable {
                        return attempt.result.clone().ok_or_else(|| {
                            ActionBrokerError::ContractViolation {
                                reason: "Durable evaluation has no result".into(),
                                terminal_immutable: false,
                            }
                        })?;
                    }
                    if attempt
                        .retry_after
                        .is_none_or(|deadline| self.monotonic_clock.monotonic_now() < deadline)
                    {
                        return Err(ActionBrokerError::GuardUnavailable {
                            reason: "Authorization retry requires durable result and backoff"
                                .into(),
                        });
                    }
                    Some(attempt.context.evaluation_id)
                } else {
                    None
                }
            } else {
                None
            };
            let resume = st
                .evaluations
                .get(&action_id)
                .is_some_and(|attempt| !attempt.result_committed);
            if !resume {
                let context = AuthorizationEvaluation {
                    evaluation_id: AuthorizationEvaluationId::from_uuid(self.ids.new_uuid()),
                    previous_evaluation_id: previous_id,
                };
                st.evaluations.insert(
                    action_id,
                    EvaluationAttempt {
                        context,
                        intent: intent.clone(),
                        intent_committed: false,
                        evaluating: false,
                        result: None,
                        result_entry: None,
                        result_committed: false,
                        result_observed_at: None,
                        retry_after: None,
                    },
                );
            }
            let context = st
                .evaluations
                .get(&action_id)
                .map(|attempt| attempt.context.clone())
                .ok_or_else(|| ActionBrokerError::NotFound {
                    resource: action_id.to_string(),
                })?;
            st.authorizations_in_progress.insert(action_id);
            context
        };
        let _pending = PendingJournal {
            broker: self,
            action_id,
            phase: PendingJournalPhase::Authorization,
        };
        let intent_committed = self
            .lock_state()
            .evaluations
            .get(&action_id)
            .is_some_and(|attempt| attempt.intent_committed);
        if !intent_committed {
            let entry = BrokerJournalEntry::Authorizing {
                evaluation: evaluation.clone(),
                intent: intent.clone(),
            };
            self.journal.append(&entry).await.map_err(|_| {
                ActionBrokerError::ContractViolation {
                    reason: "Authorizing journal commit uncertain; retain evaluation identity"
                        .into(),
                    terminal_immutable: false,
                }
            })?;
            if let Some(attempt) = self.lock_state().evaluations.get_mut(&action_id) {
                attempt.intent_committed = true;
            }
        }
        let known = self
            .lock_state()
            .evaluations
            .get(&action_id)
            .and_then(|attempt| attempt.result_entry.clone());
        let entry = if let Some(entry) = known {
            entry
        } else {
            if let Some(attempt) = self.lock_state().evaluations.get_mut(&action_id) {
                attempt.evaluating = true;
            }
            let outcome = self
                .authorize_action_inner(intent.clone(), authority, &evaluation)
                .await;
            let entry = BrokerJournalEntry::Authorization {
                evaluation: evaluation.clone(),
                cause: AuthorizationCause::Evaluation,
                intent: intent.clone(),
                outcome: outcome.clone(),
                retry_not_before_ms: self.authorization_retry_not_before_ms(&outcome),
            };
            if let Some(attempt) = self.lock_state().evaluations.get_mut(&action_id) {
                attempt.evaluating = false;
                attempt.result_observed_at =
                    Some((self.clock.now_ms(), self.monotonic_clock.monotonic_now()));
                attempt.result = Some(outcome);
                attempt.result_entry = Some(entry.clone());
            }
            entry
        };
        self.journal
            .append(&entry)
            .await
            .map_err(|_| ActionBrokerError::ContractViolation {
                reason: "Authorization journal commit uncertain; replay the exact recorded result"
                    .into(),
                terminal_immutable: false,
            })?;
        let BrokerJournalEntry::Authorization { outcome, .. } = entry else {
            return Err(ActionBrokerError::ContractViolation {
                reason: "Expected retained authorization result".into(),
                terminal_immutable: false,
            });
        };
        let mut st = self.lock_state();
        if let Some(attempt) = st.evaluations.get_mut(&action_id) {
            attempt.result_committed = true;
            if matches!(outcome, Err(ActionBrokerError::GuardUnavailable { .. })) {
                attempt.retry_after = Some(
                    self.monotonic_clock
                        .monotonic_now()
                        .saturating_add(AUTHORIZATION_RETRY_BACKOFF),
                );
            }
        }
        if let Some(rec) = st.actions.get_mut(&action_id) {
            rec.journal_authorization_pending = false;
        }
        let cache_current = st
            .actions
            .get(&action_id)
            .is_some_and(|rec| rec.cache_generation == st.cache_generation);
        if let Ok(AuthorizationOutcome::Authorized(decision)) = &outcome
            && !decision.from_cache
            && decision.expiry.is_some()
            && cache_current
        {
            let observed = st
                .evaluations
                .get(&action_id)
                .and_then(|attempt| attempt.result_observed_at);
            if let Some((observed_utc, observed_mono)) = observed {
                let mapping_profile_id = st
                    .actions
                    .get(&action_id)
                    .map(|rec| rec.mapping_profile_id.clone())
                    .unwrap_or_default();
                let key = DecisionCacheKey {
                    project_id: intent.project_id,
                    subject_key: cache_binding(&intent)?,
                    verb: intent.verb.clone(),
                    effects_class: intent.effects_class,
                    payload_digest: intent.payload_digest,
                    policy_revision: intent.policy_revision,
                    scope_revision: intent.scope_revision,
                    mapping_profile_id,
                };
                let subjects = intent
                    .subject
                    .matching_subject_refs()
                    .into_iter()
                    .map(|subject| subject.canonical_key())
                    .collect();
                st.cache.insert(
                    key,
                    decision.evaluation_id,
                    decision.decision_ref.clone(),
                    decision.applied_obligations.clone(),
                    decision.expiry,
                    observed_utc,
                    observed_mono,
                    subjects,
                );
            }
        }
        if let Ok(AuthorizationOutcome::Authorized(decision)) = &outcome {
            st.events.push(BrokerAuditEvent::ActionAuthorized {
                action_id,
                decision_ref: decision.decision_ref.clone(),
                fallback_policy_ref: decision.fallback_policy_ref.clone(),
            });
        }
        outcome
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Closed authorization state and GuardPort response mapping"
    )]
    async fn authorize_action_inner(
        &self,
        intent: ActionIntent,
        authority: &EffectiveAuthority,
        evaluation: &AuthorizationEvaluation,
    ) -> Result<AuthorizationOutcome, ActionBrokerError> {
        let now_utc = self.clock.now_ms();
        let now_mono = self.monotonic_clock.monotonic_now();
        let action_id = intent.action_id;
        let evaluation_id = evaluation.evaluation_id;

        // 1. Transition Prepared -> Authorizing et contrôles locaux déterministes.
        let initial_state = transition_action(ActionState::Prepared, ActionEvent::SubmitToBroker)?;

        let (guard_mode, guard_port_opt, mapping_profile, active_executor, cached_hit, eval_req) = {
            let mut st = self.lock_state();
            let subject_refs = intent.subject.matching_subject_refs();
            let profile_id = st.mapping_profile.profile_id().to_owned();
            let guard_revision = st.guard_revision;
            let cache_generation = st.cache_generation;
            let revision = st
                .actions
                .get(&action_id)
                .map_or(Revision::INITIAL.next(), |record| record.revision.next());
            if st
                .actions
                .get(&action_id)
                .is_some_and(|record| !record.dispatches.is_empty())
            {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "An existing dispatch forbids automatic reevaluation".into(),
                    terminal_immutable: false,
                });
            }
            st.actions.insert(
                action_id,
                ActionRecord {
                    evaluation: evaluation.clone(),
                    intent: intent.clone(),
                    state: initial_state,
                    revision,
                    decision: None,
                    pending_approval_scope: None,
                    verified_paths: Vec::new(),
                    dispatches: Vec::new(),
                    enforcement_refs: Vec::new(),
                    committed_at_ms: None,
                    denial_or_unknown_reason: None,
                    mapping_profile_id: profile_id.clone(),
                    guard_revision,
                    cache_generation,
                    journal_authorization_pending: true,
                    observation_in_progress: false,
                    observation_revision: Revision::INITIAL,
                    pending_observation: None,
                    pending_approval: None,
                },
            );
            st.events
                .push(BrokerAuditEvent::ActionAuthorizationRequested {
                    action_id,
                    effects_class: intent.effects_class,
                    subject: intent.subject.primary_identity_key(),
                });

            let policy = intent
                .project_id
                .and_then(|pid| st.project_policies.get(&pid))
                .unwrap_or(&st.global_policy);
            if intent.lineage_fence != st.current_fence
                || intent.policy_revision != policy.revision
                || (policy.isolation_mode == IsolationMode::Strict
                    && intent.isolation_requirements.mode != IsolationMode::Strict)
            {
                let err = ActionBrokerError::PolicyDenied {
                    reason: "Intent lineage/policy/isolation is stale or weaker than broker policy"
                        .into(),
                    limiting_authorities: vec!["broker_policy".into()],
                };
                mark_action_denied_locked(&mut st, action_id, &err.to_string());
                return Err(err);
            }

            if let Err(err) = check_subject_governance_locked(&st.subjects, &subject_refs) {
                mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
                return Err(err);
            }

            let verified_paths = match authority.check_intent(&intent, now_utc) {
                Ok(paths) => paths,
                Err(err) => {
                    mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
                    return Err(err);
                }
            };

            if let Err(err) = verify_isolation_requirements(
                &intent.isolation_requirements,
                self.isolation_probe.as_ref(),
            ) {
                mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
                return Err(err);
            }

            if let Some(rec) = st.actions.get_mut(&action_id) {
                rec.verified_paths = verified_paths;
            }

            if matches!(st.guard_mode, GuardMode::BrokerOnly)
                || !intent.effects_class.requires_guard_evaluation()
            {
                let dref = GuardDecisionRef::new(format!("local:{}", self.ids.new_uuid()))?;
                let decision = AuthorizationDecision {
                    evaluation_id,
                    source_evaluation_id: None,
                    action_id,
                    decision_ref: dref,
                    policy_revision: intent.policy_revision,
                    scope_revision: intent.scope_revision,
                    payload_digest: intent.payload_digest,
                    expiry: None,
                    applied_obligations: Vec::new(),
                    fallback_policy_ref: None,
                    from_cache: false,
                    executor: ExecutorKind::Unleash,
                };
                mark_action_authorized_locked(&mut st, action_id, decision.clone())?;
                return Ok(AuthorizationOutcome::Authorized(decision));
            }

            let cache_key = DecisionCacheKey {
                project_id: intent.project_id,
                subject_key: cache_binding(&intent)?,
                verb: intent.verb.clone(),
                effects_class: intent.effects_class,
                payload_digest: intent.payload_digest,
                policy_revision: intent.policy_revision,
                scope_revision: intent.scope_revision,
                mapping_profile_id: profile_id,
            };
            let cached_hit = st.cache.get(&cache_key, now_utc, now_mono);
            let eval_req = GuardEvaluateRequest::from_intent(&intent, evaluation_id);
            (
                st.guard_mode.clone(),
                st.guard_port.clone(),
                st.mapping_profile.clone(),
                st.active_executor,
                cached_hit,
                eval_req,
            )
        };

        // 2. Cache hit : vérifier que toutes les obligations en cache restent imposables.
        if let Some(cached) = cached_hit {
            if let Err(err) =
                revalidate_obligations(&cached.obligations, self.obligation_checker.as_ref())
            {
                let mut st = self.lock_state();
                mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
                return Err(err);
            }
            let decision = AuthorizationDecision {
                evaluation_id,
                source_evaluation_id: Some(cached.source_evaluation_id),
                action_id,
                decision_ref: cached.decision_ref,
                policy_revision: intent.policy_revision,
                scope_revision: intent.scope_revision,
                payload_digest: intent.payload_digest,
                expiry: Some(cached.expires_at_utc),
                applied_obligations: cached.obligations,
                fallback_policy_ref: None,
                from_cache: true,
                executor: active_executor,
            };
            let mut st = self.lock_state();
            mark_action_authorized_locked(&mut st, action_id, decision.clone())?;
            return Ok(AuthorizationOutcome::Authorized(decision));
        }

        // 3. Appel externe GuardPort HORS VERROU, borné par evaluate_timeout sans retry immédiat.
        let guard_result: Result<GuardEvaluationResponse, GuardPortError> = match guard_port_opt {
            None => Err(GuardPortError::Unavailable {
                reason: String::from("Aucun adaptateur GuardPort connecté"),
            }),
            Some(port) => self.call_guard(port.evaluate_action(&eval_req)).await,
        };

        // Sensitive decision identifiers/constraints cannot be rewritten and reused safely.
        // Refuse them locally; never turn a Deny into an optional-mode fallback.
        let guard_result = match guard_result {
            Ok(
                response @ (GuardEvaluationResponse::Allow { .. }
                | GuardEvaluationResponse::Deny { .. }
                | GuardEvaluationResponse::RequireApproval { .. }),
            ) => {
                let wire = serde_json::to_value(&response).map_err(|_| {
                    ActionBrokerError::GuardUnavailable {
                        reason: "Invalid guard response".into(),
                    }
                })?;
                if self.contains_sensitive_value(&wire) {
                    Ok(GuardEvaluationResponse::Deny {
                        reason_code: "SENSITIVE_GUARD_RESPONSE".into(),
                        reason_detail: "Sensitive guard metadata rejected".into(),
                        decision_ref: GuardDecisionRef::new("sensitive-response-refused")?,
                    })
                } else {
                    Ok(response)
                }
            }
            other => other,
        };

        // 4. Interprétation stricte et atomique de la réponse du garde.
        let post_now_utc = self.clock.now_ms();
        let mut st = self.lock_state();

        if st
            .actions
            .get(&action_id)
            .is_none_or(|r| r.guard_revision != st.guard_revision)
        {
            return Err(ActionBrokerError::GuardUnavailable {
                reason: "Guard configuration changed during evaluation".into(),
            });
        }

        // Revalider qu'aucune révocation/suspension n'est survenue pendant l'appel réseau.
        if let Err(err) =
            check_subject_governance_locked(&st.subjects, &intent.subject.matching_subject_refs())
        {
            mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
            return Err(err);
        }

        match guard_result {
            Ok(GuardEvaluationResponse::Allow {
                decision_ref,
                constraints,
                expiry,
            }) => {
                if let Some(exp) = expiry
                    && exp <= post_now_utc
                {
                    let err = ActionBrokerError::GuardDenied {
                        reason_code: String::from("DECISION_EXPIRED"),
                        detail: String::from("Décision Allow déjà expirée à réception"),
                        decision_ref: Some(decision_ref.to_string()),
                    };
                    mark_action_denied_locked(&mut st, action_id, &err.to_string());
                    return Err(err);
                }

                let obligations = match map_and_validate_obligations(
                    &constraints,
                    &mapping_profile,
                    self.obligation_checker.as_ref(),
                ) {
                    Ok(obls) => obls,
                    Err(err) => {
                        mark_action_denied_locked(
                            &mut st,
                            action_id,
                            &self.sanitize(&err.to_string()),
                        );
                        return Err(err);
                    }
                };

                let decision = AuthorizationDecision {
                    evaluation_id,
                    source_evaluation_id: None,
                    action_id,
                    decision_ref,
                    policy_revision: intent.policy_revision,
                    scope_revision: intent.scope_revision,
                    payload_digest: intent.payload_digest,
                    expiry,
                    applied_obligations: obligations,
                    fallback_policy_ref: None,
                    from_cache: false,
                    executor: active_executor,
                };
                mark_action_authorized_locked(&mut st, action_id, decision.clone())?;
                Ok(AuthorizationOutcome::Authorized(decision))
            }

            Ok(GuardEvaluationResponse::Deny {
                reason_code,
                reason_detail,
                decision_ref,
            }) => {
                // Deny ne déclenche JAMAIS de repli, même en GuardOptional.
                let clean_detail = self.sanitize(&reason_detail);
                let err = ActionBrokerError::GuardDenied {
                    reason_code,
                    detail: clean_detail.clone(),
                    decision_ref: Some(decision_ref.to_string()),
                };
                mark_action_denied_locked(&mut st, action_id, &clean_detail);
                Err(err)
            }

            Ok(GuardEvaluationResponse::RequireApproval {
                approval_scope,
                decision_ref,
                reason,
            }) => {
                // RequireApproval ne déclenche JAMAIS de repli, même en GuardOptional.
                if !mapping_profile.supports_remote_approval() {
                    let err = ActionBrokerError::GuardApprovalUnsupported {
                        reason: format!(
                            "Le profil '{}' ne qualifie pas RequireApproval",
                            mapping_profile.profile_id()
                        ),
                    };
                    mark_action_denied_locked(&mut st, action_id, &err.to_string());
                    return Err(err);
                }

                let next =
                    transition_action(ActionState::Authorizing, ActionEvent::GuardRequireApproval)?;
                let rev = {
                    let rec = st.actions.get_mut(&action_id).ok_or_else(|| {
                        ActionBrokerError::NotFound {
                            resource: format!("action:{action_id}"),
                        }
                    })?;
                    rec.state = next;
                    rec.revision = rec.revision.next();
                    rec.pending_approval_scope = Some(approval_scope);
                    rec.denial_or_unknown_reason = Some(self.sanitize(&reason));
                    rec.revision
                };
                st.events.push(BrokerAuditEvent::ActionApprovalRequired {
                    action_id,
                    approval_scope,
                });
                Ok(AuthorizationOutcome::ApprovalRequired {
                    evaluation_id,
                    action_id,
                    approval_scope,
                    decision_ref,
                    revision: rev,
                })
            }

            Ok(GuardEvaluationResponse::Unavailable { reason })
            | Err(
                GuardPortError::Unavailable { reason }
                | GuardPortError::Transport { detail: reason }
                | GuardPortError::Unauthorized { reason },
            ) => handle_guard_unusable_decision(
                &mut st,
                action_id,
                &guard_mode,
                &intent,
                authority,
                self.fallback_policy.as_ref(),
                &self.ids,
                UnusableGuardKind::Unavailable(self.sanitize(&reason)),
            ),

            Ok(GuardEvaluationResponse::UnsupportedCapability { capability, detail }) => {
                handle_guard_unusable_decision(
                    &mut st,
                    action_id,
                    &guard_mode,
                    &intent,
                    authority,
                    self.fallback_policy.as_ref(),
                    &self.ids,
                    UnusableGuardKind::ProtocolUnsupported(
                        self.sanitize(&format!("{capability}: {detail}")),
                    ),
                )
            }

            Err(GuardPortError::Timeout { timeout_ms }) => handle_guard_unusable_decision(
                &mut st,
                action_id,
                &guard_mode,
                &intent,
                authority,
                self.fallback_policy.as_ref(),
                &self.ids,
                UnusableGuardKind::Unavailable(format!(
                    "EvaluateAction expiré après {timeout_ms} ms"
                )),
            ),

            Err(GuardPortError::MalformedResponse { detail }) => handle_guard_unusable_decision(
                &mut st,
                action_id,
                &guard_mode,
                &intent,
                authority,
                self.fallback_policy.as_ref(),
                &self.ids,
                UnusableGuardKind::Unavailable(
                    self.sanitize(&format!("Réponse malformée : {detail}")),
                ),
            ),

            Err(GuardPortError::IncompatibleProtocol { expected, observed }) => {
                handle_guard_unusable_decision(
                    &mut st,
                    action_id,
                    &guard_mode,
                    &intent,
                    authority,
                    self.fallback_policy.as_ref(),
                    &self.ids,
                    UnusableGuardKind::ProtocolUnsupported(self.sanitize(&format!(
                        "protocole incompatible (attendu '{expected}', observé '{observed}')"
                    ))),
                )
            }

            Err(GuardPortError::UnsupportedCapability { capability, .. }) => {
                handle_guard_unusable_decision(
                    &mut st,
                    action_id,
                    &guard_mode,
                    &intent,
                    authority,
                    self.fallback_policy.as_ref(),
                    &self.ids,
                    UnusableGuardKind::ProtocolUnsupported(
                        self.sanitize(&format!("capacité non supportée '{capability}'")),
                    ),
                )
            }
        }
    }

    /// Revalide juste avant engagement le fence, le mandat, les révisions, le statut sujet,
    /// les chemins TOCTOU, l'isolation et toutes les obligations, committe le marqueur durable
    /// `Dispatching`, résout les secrets uniquement au transport puis exécute et observe l'effet
    /// (`REQ-SEC-002`, `REQ-SEC-003`, `REQ-SEC-007`, `REQ-SEC-010`).
    ///
    /// # Sûreté à l'annulation
    /// Une fois le marqueur `ActionEvent::CommitDispatchMarker` engagé (`Dispatching`), une
    /// annulation du future laisse l'action en état `Dispatching` ou `OutcomeUnknown` afin qu'aucune
    /// réexécution aveugle n'ait lieu sans réconciliation explicite.
    ///
    /// # Errors
    /// Retourne l'erreur F017 appropriée sans jamais résoudre de secret ni lancer d'I/O si une
    /// précondition échoue.
    #[expect(
        clippy::too_many_lines,
        reason = "Séquence critique atomique : revalidation pré-dispatch, commit marqueur, secrets, exécution et observation"
    )]
    pub async fn execute_authorized(
        &self,
        action_id: ActionId,
        decision_ref: &GuardDecisionRef,
        fence: LineageFence,
        authority: &EffectiveAuthority,
        executor: &dyn ActionExecutorPort,
        secret_store: &dyn SecretStore,
    ) -> Result<ActionReceipt, ActionBrokerError> {
        let now_utc = self.clock.now_ms();

        // Phase 1 : Revalidation atomique stricte sous verrou avant tout marqueur ou secret.
        let (intent, decision, dispatch_id, guard_port_opt) = {
            let mut st = self.lock_state();
            let current_fence = st.current_fence;
            let project_policy_rev = {
                let rec =
                    st.actions
                        .get(&action_id)
                        .ok_or_else(|| ActionBrokerError::NotFound {
                            resource: format!("action:{action_id}"),
                        })?;
                rec.intent
                    .project_id
                    .and_then(|pid| st.project_policies.get(&pid).map(|p| p.revision))
                    .unwrap_or(st.global_policy.revision)
            };

            // Contrôle de lignée et fence.
            if let Err(fe) = current_fence.check_monotone_successor(&fence) {
                return Err(ActionBrokerError::from(fe));
            }
            if fence != current_fence {
                return Err(ActionBrokerError::StaleFence {
                    reason: format!(
                        "Fence {fence:?} obsolète par rapport au fence actif {current_fence:?}"
                    ),
                    lineage_mismatch: false,
                });
            }

            let rec_snapshot = {
                let rec =
                    st.actions
                        .get(&action_id)
                        .ok_or_else(|| ActionBrokerError::NotFound {
                            resource: format!("action:{action_id}"),
                        })?;

                if rec.state != ActionState::Authorized {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: format!(
                            "Action {action_id} ne peut pas être dispatchée depuis l'état '{}'",
                            rec.state
                        ),
                        terminal_immutable: rec.state.is_terminal(),
                    });
                }

                if rec.journal_authorization_pending
                    || rec.guard_revision != st.guard_revision
                    || rec.intent.lineage_fence != current_fence
                {
                    return Err(ActionBrokerError::PolicyDenied { reason: "Authorization pending durable commit or invalidated by guard/lineage change".into(), limiting_authorities: vec!["broker".into()] });
                }

                let Some(ref stored_dec) = rec.decision else {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: format!("Décision absente pour l'action {action_id}"),
                        terminal_immutable: false,
                    });
                };

                if &stored_dec.decision_ref != decision_ref {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: String::from("DecisionRef périmé ou incohérent (non-résurrection)"),
                        terminal_immutable: false,
                    });
                }
                (
                    rec.intent.clone(),
                    stored_dec.clone(),
                    rec.verified_paths.clone(),
                )
            };

            let (intent, decision, verified_paths) = rec_snapshot;

            // Vérifier révocation / suspension survenue après Allow.
            if let Err(err) = check_subject_governance_locked(
                &st.subjects,
                &intent.subject.matching_subject_refs(),
            ) {
                let revoked = matches!(err, ActionBrokerError::SubjectRevoked { .. });
                if let Some(rec) = st.actions.get_mut(&action_id) {
                    if let Ok(next) =
                        transition_action(rec.state, ActionEvent::ReevaluateAuthorized { revoked })
                    {
                        rec.state = next;
                        rec.decision = None;
                        rec.revision = rec.revision.next();
                    }
                    rec.denial_or_unknown_reason = Some(self.sanitize(&err.to_string()));
                }
                if revoked {
                    st.events.push(BrokerAuditEvent::ActionDenied {
                        action_id,
                        reason: self.sanitize(&err.to_string()),
                    });
                }
                return Err(err);
            }

            // Vérifier révisions de politique et de scope + expiration décision + deadline.
            let current_scope_rev = st
                .scope_revisions
                .get(&intent.project_id)
                .copied()
                .unwrap_or(intent.scope_revision);
            let decision_expired = decision.expiry.is_some_and(|exp| now_utc >= exp);
            if intent.policy_revision != project_policy_rev
                || intent.scope_revision != current_scope_rev
                || decision_expired
                || now_utc > intent.action_deadline
            {
                if let Some(rec) = st.actions.get_mut(&action_id)
                    && let Ok(next) = transition_action(
                        rec.state,
                        ActionEvent::ReevaluateAuthorized { revoked: false },
                    )
                {
                    rec.state = next;
                    rec.decision = None;
                    rec.revision = rec.revision.next();
                }
                return Err(ActionBrokerError::PolicyDenied {
                    reason: String::from(
                        "Mandat, révision ou expiration invalidé avant engagement : réévaluation requise",
                    ),
                    limiting_authorities: authority.limiting_layer_names(),
                });
            }

            // Revalider autorité effective, TOCTOU des chemins, isolation et obligations.
            authority.check_intent(&intent, now_utc)?;
            for (orig_path, canon_path) in &verified_paths {
                verify_path_toctou(canon_path, orig_path, &authority.allowed_roots)?;
            }
            verify_isolation_requirements(
                &intent.isolation_requirements,
                self.isolation_probe.as_ref(),
            )?;
            if let Err(err) = revalidate_obligations(
                &decision.applied_obligations,
                self.obligation_checker.as_ref(),
            ) {
                mark_action_denied_locked(&mut st, action_id, &self.sanitize(&err.to_string()));
                return Err(err);
            }

            // Commit durable du marqueur Dispatching AVANT toute résolution de secret ou I/O.
            let dispatch_id = DispatchId::from_uuid(self.ids.new_uuid());
            let next_state =
                transition_action(ActionState::Authorized, ActionEvent::CommitDispatchMarker)?;
            let enforcement_refs: Vec<String> = decision
                .applied_obligations
                .iter()
                .map(|o| format!("{}:{}", o.mechanism, o.obligation_id))
                .collect();

            if let Some(rec) = st.actions.get_mut(&action_id) {
                rec.state = next_state;
                rec.revision = rec.revision.next();
                rec.dispatches.push((dispatch_id, fence));
                rec.enforcement_refs = enforcement_refs;
            }

            (intent, decision, dispatch_id, st.guard_port.clone())
        };

        if self
            .journal
            .append(&BrokerJournalEntry::Dispatch {
                evaluation: self.evaluation_for_action(action_id)?,
                intent: intent.clone(),
                decision: decision.clone(),
                dispatch_id,
                fence,
            })
            .await
            .is_err()
        {
            if let Some(rec) = self.lock_state().actions.get_mut(&action_id) {
                rec.state = ActionState::OutcomeUnknown;
                rec.denial_or_unknown_reason = Some(
                    "Dispatch journal commit unavailable; no external effect attempted".into(),
                );
            }
            return Err(ActionBrokerError::OutcomeUnknown {
                action_id,
                reason: "Dispatch journal commit unavailable; reconcile before retry".into(),
            });
        }
        {
            let mut st = self.lock_state();
            if let Some(rec) = st.actions.get_mut(&action_id) {
                rec.committed_at_ms = Some(self.clock.now_ms());
            }
            st.events.push(BrokerAuditEvent::ActionDispatchCommitted {
                action_id,
                dispatch_id,
                decision_ref: decision.decision_ref.clone(),
            });
        }

        // Phase 2 : Résolution des secrets uniquement au transport autorisé.
        let grant = AuthorizedSecretAccess::new(
            action_id,
            decision.decision_ref.clone(),
            intent.secret_refs.clone(),
        );
        let mut resolved_secrets = Vec::with_capacity(intent.secret_refs.len());
        for sref in &intent.secret_refs {
            match secret_store.resolve_for_authorized_transport(sref, &grant) {
                Ok(sec) => resolved_secrets.push(sec),
                Err(sec_err) => {
                    let broker_err = ActionBrokerError::from(sec_err);
                    let receipt = ActionReceipt {
                        evaluation_id: decision.evaluation_id,
                        action_id,
                        dispatch_id,
                        decision_ref: decision.decision_ref.clone(),
                        state: ActionState::ObservedFailure,
                        output_digest: None,
                        enforcement_refs: Vec::new(),
                        remote_audit_ref: None,
                        remote_audit_unavailable: false,
                        observed_at_ms: self.clock.now_ms(),
                    };
                    if let Err(err @ ActionBrokerError::OutcomeUnknown { .. }) = self
                        .commit_observation(
                            &intent,
                            receipt,
                            Some(self.sanitize(&broker_err.to_string())),
                            ObservationCause::Execution,
                        )
                        .await
                    {
                        return Err(err);
                    }
                    return Err(broker_err);
                }
            }
        }

        // Phase 3 : Exécution physique via l'exécuteur retenu (Unleash par défaut, Guard si qualifié).
        let exec_outcome = if decision.executor == ExecutorKind::Guard
            && let Some(port) = guard_port_opt
        {
            let greq = GuardExecuteRequest {
                action_id,
                decision_ref: decision.decision_ref.clone(),
                verb: intent.verb.clone(),
                payload_digest: intent.payload_digest,
            };
            match self.call_guard(port.execute_authorized_action(&greq)).await {
                Ok(out) if out.succeeded => Ok(ActionExecutionResult {
                    dispatch_id,
                    provider_request_id: Some(out.execution_ref),
                    output_digest: out.output_digest,
                    observed_at_ms: out.observed_at_ms,
                }),
                Ok(out) => Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                    dispatch_id,
                    reason: format!("Exécution distante échouée ({})", out.execution_ref),
                }),
                Err(gerr) => Err(ActionExecutionFailure::OutcomeUnknown {
                    dispatch_id,
                    reason: format!("Erreur AI-Guard après envoi : {gerr}"),
                }),
            }
        } else {
            executor
                .execute_action(dispatch_id, &intent, &decision, &resolved_secrets)
                .await
        };

        drop(resolved_secrets);

        // Phase 4: commit the observation before acknowledging it. The committed dispatch ID
        // is authoritative; an executor response carrying another ID is ambiguous.
        let (state, output_digest, remote_audit_ref, reason) = match exec_outcome {
            Ok(res) if res.dispatch_id == dispatch_id => (
                ActionState::ObservedSuccess,
                Some(res.output_digest),
                res.provider_request_id,
                None,
            ),
            Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                dispatch_id: observed,
                reason,
            }) if observed == dispatch_id => (
                ActionState::ObservedFailure,
                None,
                None,
                Some(self.sanitize(&reason)),
            ),
            Err(ActionExecutionFailure::OutcomeUnknown { reason, .. }) => (
                ActionState::OutcomeUnknown,
                None,
                None,
                Some(self.sanitize(&reason)),
            ),
            _ => (
                ActionState::OutcomeUnknown,
                None,
                None,
                Some(String::from("Executor dispatch identity mismatch")),
            ),
        };
        let receipt = ActionReceipt {
            evaluation_id: decision.evaluation_id,
            action_id,
            dispatch_id,
            decision_ref: decision.decision_ref,
            state,
            output_digest,
            enforcement_refs: self
                .lock_state()
                .actions
                .get(&action_id)
                .map(|r| r.enforcement_refs.clone())
                .unwrap_or_default(),
            remote_audit_ref,
            remote_audit_unavailable: false,
            observed_at_ms: self.clock.now_ms(),
        };
        self.commit_observation(&intent, receipt, reason, ObservationCause::Execution)
            .await
    }

    async fn commit_observation(
        &self,
        intent: &ActionIntent,
        mut receipt: ActionReceipt,
        reason: Option<String>,
        cause: ObservationCause,
    ) -> Result<ActionReceipt, ActionBrokerError> {
        if receipt
            .remote_audit_ref
            .as_ref()
            .is_some_and(|reference| self.sanitize(reference) != *reference)
        {
            receipt.remote_audit_ref = None;
            receipt.remote_audit_unavailable = true;
        }
        let entry = {
            let st = self.lock_state();
            let rec =
                st.actions
                    .get(&intent.action_id)
                    .ok_or_else(|| ActionBrokerError::NotFound {
                        resource: intent.action_id.to_string(),
                    })?;
            BrokerJournalEntry::Observation {
                evaluation: rec.evaluation.clone(),
                observation_revision: rec.observation_revision.next(),
                cause,
                intent: intent.clone(),
                receipt,
                reason: reason.clone(),
            }
        };
        let receipt = self.persist_observation_entry(entry).await?;
        if receipt.state == ActionState::OutcomeUnknown && reason.is_some() {
            return Err(ActionBrokerError::OutcomeUnknown {
                action_id: intent.action_id,
                reason: reason.unwrap_or_default(),
            });
        }
        if receipt.state == ActionState::ObservedFailure && reason.is_some() {
            return Err(ActionBrokerError::ContractViolation {
                reason: reason.unwrap_or_default(),
                terminal_immutable: false,
            });
        }
        Ok(receipt)
    }

    async fn persist_observation_entry(
        &self,
        entry: BrokerJournalEntry,
    ) -> Result<ActionReceipt, ActionBrokerError> {
        let BrokerJournalEntry::Observation {
            intent,
            receipt,
            observation_revision,
            ..
        } = &entry
        else {
            return Err(ActionBrokerError::ContractViolation {
                reason: "Expected retained observation".into(),
                terminal_immutable: false,
            });
        };
        let action_id = intent.action_id;
        {
            let mut st = self.lock_state();
            let rec =
                st.actions
                    .get_mut(&action_id)
                    .ok_or_else(|| ActionBrokerError::NotFound {
                        resource: action_id.to_string(),
                    })?;
            if rec.observation_in_progress || rec.state.is_terminal() {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "Observation pending or action already terminal".into(),
                    terminal_immutable: rec.state.is_terminal(),
                });
            }
            if receipt.evaluation_id != rec.evaluation.evaluation_id
                || receipt.action_id != action_id
            {
                return Err(ActionBrokerError::ContractViolation {
                    reason: "Observation evaluation/action mismatch".into(),
                    terminal_immutable: false,
                });
            }
            if let Some(pending) = &rec.pending_observation {
                if pending != &entry {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: "Resolve the exact pending observation first".into(),
                        terminal_immutable: false,
                    });
                }
            } else {
                if *observation_revision != rec.observation_revision.next() {
                    return Err(ActionBrokerError::ContractViolation {
                        reason: "Observation revision changed before commit".into(),
                        terminal_immutable: false,
                    });
                }
                rec.pending_observation = Some(entry.clone());
            }
            rec.observation_in_progress = true;
        }
        let _pending = PendingJournal {
            broker: self,
            action_id,
            phase: PendingJournalPhase::Observation,
        };
        let committed = self.journal.append(&entry).await;
        let mut st = self.lock_state();
        let rec = st
            .actions
            .get_mut(&action_id)
            .ok_or_else(|| ActionBrokerError::NotFound {
                resource: action_id.to_string(),
            })?;
        rec.observation_in_progress = false;
        if committed.is_err() {
            rec.state = ActionState::OutcomeUnknown;
            rec.denial_or_unknown_reason = Some("Observation journal commit uncertain".into());
            return Err(ActionBrokerError::OutcomeUnknown {
                action_id,
                reason: "Observation journal commit uncertain; retain exact entry".into(),
            });
        }
        rec.pending_observation = None;
        rec.state = receipt.state;
        rec.observation_revision = *observation_revision;
        rec.revision = rec.revision.next();
        rec.enforcement_refs.clone_from(&receipt.enforcement_refs);
        if let BrokerJournalEntry::Observation { reason, .. } = &entry {
            rec.denial_or_unknown_reason.clone_from(reason);
        }
        st.events.push(BrokerAuditEvent::ActionObserved {
            action_id,
            dispatch_id: receipt.dispatch_id,
            state: receipt.state,
        });
        Ok(receipt.clone())
    }

    async fn resume_pending_observation(
        &self,
        action_id: ActionId,
    ) -> Result<Option<ActionReceipt>, ActionBrokerError> {
        let pending = self
            .lock_state()
            .actions
            .get(&action_id)
            .and_then(|rec| rec.pending_observation.clone());
        let Some(entry) = pending else {
            return Ok(None);
        };
        let receipt = self.persist_observation_entry(entry).await?;
        // The old unknown record is durable history; an explicit fresh reconciliation may follow it.
        if matches!(
            receipt.state,
            ActionState::OutcomeUnknown | ActionState::InFlight | ActionState::CancelRequested
        ) {
            Ok(None)
        } else {
            Ok(Some(receipt))
        }
    }

    /// Réconcilie une action en `Dispatching`/`InFlight`/`OutcomeUnknown` en consultant la référence
    /// d'audit distante si `GuardPort` est configuré (`REQ-SEC-010`).
    ///
    /// # Errors
    /// Retourne `NotFound`, `ContractViolation` ou l'erreur de transition associée.
    pub async fn reconcile_action(
        &self,
        action_id: ActionId,
        outcome: ReconcileOutcome,
        evidence_ref: Option<String>,
    ) -> Result<ActionReceipt, ActionBrokerError> {
        if let Some(receipt) = self.resume_pending_observation(action_id).await? {
            return Ok(receipt);
        }
        let cause = ObservationCause::Reconciliation {
            evidence_ref: evidence_ref.clone(),
        };
        let (decision_ref_opt, guard_opt) = {
            let st = self.lock_state();
            let rec = st
                .actions
                .get(&action_id)
                .ok_or_else(|| ActionBrokerError::NotFound {
                    resource: format!("action:{action_id}"),
                })?;
            (
                rec.decision.as_ref().map(|d| d.decision_ref.clone()),
                st.guard_port.clone(),
            )
        };

        let (remote_audit_ref, remote_audit_unavailable) = if let Some(port) = guard_opt {
            match self
                .call_guard(port.read_audit_reference(action_id, decision_ref_opt.as_ref()))
                .await
            {
                Ok(r) => (Some(r.audit_id), false),
                Err(_) => (None, true),
            }
        } else {
            (None, false)
        };

        let (intent, receipt) = self.prepare_reconciliation(
            action_id,
            None,
            outcome,
            evidence_ref,
            remote_audit_ref,
            remote_audit_unavailable,
        )?;
        self.commit_observation(&intent, receipt, None, cause).await
    }

    /// Applies a receipt only to its committed action/dispatch/fence and journals the observation.
    /// An uncertain journal commit leaves the action unknown, never acknowledged as success.
    ///
    /// # Errors
    /// Returns stale fence, foreign dispatch, terminal-state or journal errors.
    pub async fn apply_external_receipt(
        &self,
        action_id: ActionId,
        dispatch_id: DispatchId,
        fence: LineageFence,
        outcome: ReconcileOutcome,
        evidence_ref: Option<String>,
    ) -> Result<ActionReceipt, ActionBrokerError> {
        // Validate the incoming identity before resuming any prior observation.
        self.prepare_reconciliation(
            action_id,
            Some((dispatch_id, fence)),
            outcome,
            evidence_ref.clone(),
            None,
            false,
        )?;
        if let Some(receipt) = self.resume_pending_observation(action_id).await? {
            return Ok(receipt);
        }
        let cause = ObservationCause::ExternalReceipt {
            evidence_ref: evidence_ref.clone(),
        };
        let (intent, receipt) = self.prepare_reconciliation(
            action_id,
            Some((dispatch_id, fence)),
            outcome,
            evidence_ref,
            None,
            false,
        )?;
        self.commit_observation(&intent, receipt, None, cause).await
    }

    fn prepare_reconciliation(
        &self,
        action_id: ActionId,
        external: Option<(DispatchId, LineageFence)>,
        outcome: ReconcileOutcome,
        evidence_ref: Option<String>,
        remote_audit_ref: Option<String>,
        remote_audit_unavailable: bool,
    ) -> Result<(ActionIntent, ActionReceipt), ActionBrokerError> {
        let st = self.lock_state();
        if let Some((_, fence)) = external
            && fence != st.current_fence
        {
            return Err(ActionBrokerError::StaleFence {
                reason: "Receipt lineage/fence is stale".into(),
                lineage_mismatch: fence.authority_generation
                    != st.current_fence.authority_generation,
            });
        }
        let rec = st
            .actions
            .get(&action_id)
            .ok_or_else(|| ActionBrokerError::NotFound {
                resource: action_id.to_string(),
            })?;
        let (dispatch_id, _) =
            rec.dispatches
                .last()
                .copied()
                .ok_or_else(|| ActionBrokerError::ContractViolation {
                    reason: "No committed dispatch to reconcile".into(),
                    terminal_immutable: rec.state.is_terminal(),
                })?;
        if let Some(expected) = external
            && !rec.dispatches.contains(&expected)
        {
            return Err(ActionBrokerError::ContractViolation {
                reason: "Receipt does not match a committed action/dispatch/fence".into(),
                terminal_immutable: rec.state.is_terminal(),
            });
        }
        let state = transition_action(rec.state, ActionEvent::ApplyReconciliation(outcome))?;
        let decision_ref = rec
            .decision
            .as_ref()
            .map(|d| d.decision_ref.clone())
            .ok_or_else(|| ActionBrokerError::ContractViolation {
                reason: "Dispatch has no authorization decision".into(),
                terminal_immutable: false,
            })?;
        let mut enforcement_refs = rec.enforcement_refs.clone();
        if let Some(reference) = evidence_ref {
            if self.sanitize(&reference) != reference {
                return Err(ActionBrokerError::InvalidArgument {
                    reason: "Sensitive evidence reference refused".into(),
                });
            }
            enforcement_refs.push(reference);
        }
        Ok((
            rec.intent.clone(),
            ActionReceipt {
                evaluation_id: rec.evaluation.evaluation_id,
                action_id,
                dispatch_id,
                decision_ref,
                state,
                output_digest: None,
                enforcement_refs,
                remote_audit_ref,
                remote_audit_unavailable,
                observed_at_ms: self.clock.now_ms(),
            },
        ))
    }

    /// Retourne l'état courant d'une action connue du broker.
    #[must_use]
    pub fn action_state(&self, action_id: ActionId) -> Option<ActionState> {
        self.lock_state().actions.get(&action_id).map(|r| r.state)
    }

    /// Reports whether the immutable intention contains any secret reference.
    /// Callers that forbid secret access must reject an unknown action as well.
    /// This read grants no execution authority and replaces no dispatch checks.
    #[must_use]
    pub fn action_has_secret_refs(&self, action_id: ActionId) -> Option<bool> {
        self.lock_state()
            .actions
            .get(&action_id)
            .map(|record| !record.intent.secret_refs.is_empty())
    }

    /// Retourne une copie des événements d'audit émis (déjà expurgés des canaris configurés).
    #[must_use]
    pub fn audit_events(&self) -> Vec<BrokerAuditEvent> {
        self.lock_state().events.clone()
    }

    /// Retourne l'enregistrement d'audit corrélé d'une action (`REQ-SEC-010`).
    #[must_use]
    pub fn action_audit(&self, action_id: ActionId) -> Option<BrokerAuditRecord> {
        self.lock_state()
            .actions
            .get(&action_id)
            .map(|rec| BrokerAuditRecord {
                action_id,
                payload_digest: rec.intent.payload_digest,
                decision_ref: rec.decision.as_ref().map(|d| d.decision_ref.clone()),
                mapping_profile_id: rec.mapping_profile_id.clone(),
                policy_revision: rec.intent.policy_revision,
                obligations: rec
                    .decision
                    .as_ref()
                    .map(|d| d.applied_obligations.clone())
                    .unwrap_or_default(),
                enforcement_refs: rec.enforcement_refs.clone(),
                committed_at_ms: rec.committed_at_ms,
                denial_or_unknown_reason: rec.denial_or_unknown_reason.clone(),
            })
    }

    /// Utilitaire exposant la validation de scope fichier côté broker.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::ScopeDenied`] si le chemin sort du scope ou traverse un symlink.
    pub fn check_path_scope(
        path: &std::path::Path,
        allowed_roots: &[PathBuf],
    ) -> Result<PathBuf, ActionBrokerError> {
        validate_path_within_roots(path, allowed_roots)
    }
}

fn cache_binding(intent: &ActionIntent) -> Result<String, ActionBrokerError> {
    let value = serde_json::to_value((
        &intent.subject,
        &intent.resource_refs,
        &intent.isolation_requirements,
        &intent.secret_refs,
        intent.lineage_fence,
    ))
    .map_err(|_| ActionBrokerError::InvalidArgument {
        reason: String::from("Invalid action cache binding"),
    })?;
    Ok(ActionIntent::compute_payload_digest(&value)?.to_string())
}

enum UnusableGuardKind {
    Unavailable(String),
    ProtocolUnsupported(String),
}

#[expect(
    clippy::too_many_arguments,
    reason = "Traitement unifié des pannes GuardRequired (fail-closed) et GuardOptional (repli préautorisé)"
)]
fn handle_guard_unusable_decision(
    st: &mut BrokerState,
    action_id: ActionId,
    guard_mode: &GuardMode,
    intent: &ActionIntent,
    authority: &EffectiveAuthority,
    fallback_policy: &dyn FallbackPolicyPort,
    ids: &Arc<dyn IdGeneratorPort>,
    kind: UnusableGuardKind,
) -> Result<AuthorizationOutcome, ActionBrokerError> {
    let evaluation_id = st
        .actions
        .get(&action_id)
        .map(|r| r.evaluation.evaluation_id)
        .ok_or_else(|| ActionBrokerError::NotFound {
            resource: action_id.to_string(),
        })?;
    match guard_mode {
        GuardMode::BrokerOnly => Err(ActionBrokerError::InvalidArgument {
            reason: String::from("État incohérent : GuardPort appelé en mode broker_only"),
        }),
        GuardMode::GuardRequired => match kind {
            UnusableGuardKind::Unavailable(reason) => {
                // Rester Authorizing, fail-closed (`GUARD_UNAVAILABLE`, `after_backoff`).
                let _ = transition_action(
                    ActionState::Authorizing,
                    ActionEvent::GuardUnavailable {
                        mode: DomainGuardMode::GuardRequired,
                        fallback_decision: None,
                    },
                );
                if let Some(rec) = st.actions.get_mut(&action_id) {
                    rec.denial_or_unknown_reason = Some(reason.clone());
                }
                st.events.push(BrokerAuditEvent::GuardHealthChanged {
                    status: GuardHealthStatus::Unavailable,
                    detail: reason.clone(),
                });
                Err(ActionBrokerError::GuardUnavailable { reason })
            }
            UnusableGuardKind::ProtocolUnsupported(reason) => {
                mark_action_denied_locked(st, action_id, &reason);
                Err(ActionBrokerError::GuardProtocolUnsupported { reason })
            }
        },
        GuardMode::GuardOptional {
            fallback_policy_ref,
        } => {
            let reason_label = match &kind {
                UnusableGuardKind::Unavailable(r) | UnusableGuardKind::ProtocolUnsupported(r) => {
                    r.clone()
                }
            };
            st.events.push(BrokerAuditEvent::GuardHealthChanged {
                status: GuardHealthStatus::Degraded,
                detail: format!(
                    "Repli guard_optional vers '{fallback_policy_ref}' suite à : {reason_label}"
                ),
            });

            let fb_res = fallback_policy.evaluate_fallback(fallback_policy_ref, intent, authority);
            let allowed = fb_res.is_ok();
            let _next = transition_action(
                ActionState::Authorizing,
                ActionEvent::GuardUnavailable {
                    mode: DomainGuardMode::GuardOptional,
                    fallback_decision: Some(allowed),
                },
            )?;

            match fb_res {
                Ok(()) => {
                    let dref = GuardDecisionRef::new(format!("fallback:{}", ids.new_uuid()))?;
                    let decision = AuthorizationDecision {
                        evaluation_id,
                        source_evaluation_id: None,
                        action_id,
                        decision_ref: dref,
                        policy_revision: intent.policy_revision,
                        scope_revision: intent.scope_revision,
                        payload_digest: intent.payload_digest,
                        expiry: None,
                        applied_obligations: Vec::new(),
                        fallback_policy_ref: Some(fallback_policy_ref.clone()),
                        from_cache: false,
                        executor: ExecutorKind::Unleash,
                    };
                    mark_action_authorized_locked(st, action_id, decision.clone())?;
                    Ok(AuthorizationOutcome::Authorized(decision))
                }
                Err(deny_reason) => {
                    mark_action_denied_locked(st, action_id, &deny_reason);
                    Err(ActionBrokerError::PolicyDenied {
                        reason: deny_reason,
                        limiting_authorities: vec![fallback_policy_ref.clone()],
                    })
                }
            }
        }
    }
}

fn check_subject_governance_locked(
    subjects: &BTreeMap<String, SubjectGovernance>,
    refs: &[TypedSubjectRef],
) -> Result<(), ActionBrokerError> {
    for r in refs {
        let key = r.canonical_key();
        if let Some(gov) = subjects.get(&key) {
            if gov.revoked {
                return Err(ActionBrokerError::SubjectRevoked {
                    subject: key,
                    reason: String::from("Sujet définitivement révoqué"),
                });
            }
            if gov.suspended {
                return Err(ActionBrokerError::SubjectSuspended {
                    subject: key,
                    reason: String::from("Sujet temporairement suspendu"),
                });
            }
        }
    }
    Ok(())
}

fn mark_action_authorized_locked(
    st: &mut BrokerState,
    action_id: ActionId,
    decision: AuthorizationDecision,
) -> Result<(), ActionBrokerError> {
    let next = transition_action(ActionState::Authorizing, ActionEvent::GuardAllow)?;
    if let Some(rec) = st.actions.get_mut(&action_id) {
        rec.state = next;
        rec.revision = rec.revision.next();
        rec.decision = Some(decision);
    }
    Ok(())
}

fn mark_action_denied_locked(st: &mut BrokerState, action_id: ActionId, reason: &str) {
    if let Some(rec) = st.actions.get_mut(&action_id) {
        if let Ok(next) = transition_action(
            rec.state,
            ActionEvent::GuardDeny {
                reason: reason.to_owned(),
            },
        ) {
            rec.state = next;
            rec.revision = rec.revision.next();
            rec.decision = None;
        }
        rec.denial_or_unknown_reason = Some(reason.to_owned());
    }
    st.events.push(BrokerAuditEvent::ActionDenied {
        action_id,
        reason: reason.to_owned(),
    });
}

fn validate_guard_endpoint(ep: &str) -> Result<(), ActionBrokerError> {
    let trimmed = ep.trim();
    if trimmed.is_empty() {
        return Err(ActionBrokerError::InvalidArgument {
            reason: String::from("Endpoint AI-Guard vide"),
        });
    }
    if !(trimmed.starts_with("unix://") || trimmed.starts_with("https://")) {
        return Err(ActionBrokerError::InvalidArgument {
            reason: format!(
                "Transport AI-Guard non autorisé pour '{trimmed}' (seuls unix:// et https:// sont admis)"
            ),
        });
    }
    Ok(())
}

fn validate_initial_policy_intent(
    snapshot: &InitialEffectivePolicy,
    intent: &ActionIntent,
) -> Result<(), ActionBrokerError> {
    if intent.lineage_fence != snapshot.lineage_fence {
        return Err(ActionBrokerError::StaleFence {
            reason: "Action differs from the initial policy incarnation".into(),
            lineage_mismatch: intent.lineage_fence.authority_generation
                != snapshot.lineage_fence.authority_generation,
        });
    }
    if intent.project_id != snapshot.project_id {
        return Err(ActionBrokerError::ScopeDenied {
            reason: "Action differs from the initial policy scope".into(),
        });
    }
    if intent.policy_revision != snapshot.policy_revision
        || intent.scope_revision != snapshot.scope_revision
    {
        return Err(ActionBrokerError::PolicyDenied {
            reason: "Action differs from the loaded effective policy or scope revision".into(),
            limiting_authorities: vec!["initial_effective_policy".into()],
        });
    }
    Ok(())
}

fn build_policy_summary(st: &BrokerState, project_id: Option<ProjectId>) -> PolicySummary {
    let entry = project_id
        .and_then(|pid| st.project_policies.get(&pid))
        .unwrap_or(&st.global_policy);
    let suspended_subjects = st
        .subjects
        .iter()
        .filter(|(_, g)| g.suspended && !g.revoked)
        .map(|(k, _)| k.clone())
        .collect();
    let revoked_subjects = st
        .subjects
        .iter()
        .filter(|(_, g)| g.revoked)
        .map(|(k, _)| k.clone())
        .collect();

    PolicySummary {
        project_id,
        policy_revision: entry.revision,
        isolation_mode: entry.isolation_mode,
        confinement_label: entry.isolation_mode.confinement_label().to_owned(),
        guard_mode: st.guard_mode.clone(),
        mapping_profile_id: st.mapping_profile.profile_id().to_owned(),
        suspended_subjects,
        revoked_subjects,
        updated_by: entry.updated_by.clone(),
        last_reason: entry.last_reason.clone(),
    }
}

fn build_guard_health_snapshot(st: &BrokerState) -> GuardHealthSnapshot {
    let mut remote_limitations = Vec::new();
    if let Some(ref caps) = st.guard_capabilities {
        if !caps.supports_suspend_resume {
            remote_limitations.push(String::from(
                "SuspendSubject/ResumeSubject non négociés par GuardPort : application locale uniquement",
            ));
        }
        if !caps.supports_execute_authorized_action {
            remote_limitations.push(String::from(
                "ExecuteAuthorizedAction non négocié : exécuteur Unleash actif par défaut",
            ));
        }
        if !caps.supports_remote_policy_write {
            remote_limitations.push(String::from(
                "Modification distante de politique AI-Guard non supportée (consultation seule)",
            ));
        }
    } else if matches!(st.guard_mode, GuardMode::BrokerOnly) {
        remote_limitations.push(String::from(
            "Mode broker_only actif sans endpoint AI-Guard configuré",
        ));
    }

    GuardHealthSnapshot {
        mode: st.guard_mode.clone(),
        connected: st.guard_port.is_some()
            && !matches!(st.guard_health, GuardHealthStatus::Disconnected),
        endpoint: st.guard_endpoint.clone(),
        negotiated_protocol_version: st
            .guard_capabilities
            .as_ref()
            .map(|c| c.protocol_version.clone()),
        mapping_profile_id: st.mapping_profile.profile_id().to_owned(),
        capabilities: st.guard_capabilities.clone(),
        health_status: st.guard_health,
        active_executor: st.active_executor,
        remote_limitations,
    }
}
