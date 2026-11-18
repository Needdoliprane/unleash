// SPDX-License-Identifier: Apache-2.0
//! Port interne versionné [`GuardPort`], profils de mapping (`none_accepted` par défaut Q-04),
//! obligations typées et modes AI-Guard (`BrokerOnly`, `GuardOptional`, `GuardRequired`).

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AgentId, ApprovalScopeId, ClockPort, MissionId, ProjectId, Revision,
    Sha256Digest, TimestampMs,
};

use crate::effects::{ActionIntent, EffectsClass, RoleVersionRef, SubjectKind};
use crate::error::{ActionBrokerError, GuardPortError};
use crate::isolation::IsolationRequirements;

/// Future typée boîte et `Send` utilisée par le port objet-safe [`GuardPort`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Version courante du protocole interne [`GuardPort`].
pub const GUARD_PORT_PROTOCOL_VERSION: &str = "unleash.guard_port.v1";

/// Identifiant canonique du profil initial `obligations=none_accepted` (`Q-04`, `ADR-10`).
pub const NONE_ACCEPTED_PROFILE_ID: &str = "unleash.guard_mapping.none_accepted.v1";

/// Délai par défaut d'un appel `EvaluateAction` (2 000 ms, `ai-guard.md`).
pub const DEFAULT_EVALUATE_TIMEOUT_MS: u64 = 2_000;

/// Intervalle par défaut entre sondes `CheckHealth` (30 000 ms, `ai-guard.md`).
pub const DEFAULT_HEALTH_CHECK_INTERVAL_MS: u64 = 30_000;

/// Port d'horloge monotone injectable pour le calcul borné des TTL de cache (`<= 300 s`).
pub trait MonotonicClockPort: Send + Sync {
    /// Renvoie le temps monotone écoulé depuis un point de référence fixe.
    fn monotonic_now(&self) -> Duration;
}

/// Port d'attente asynchrone injectable pour déclencher des timeouts déterministes sans `sleep` réel.
pub trait SleepPort: Send + Sync {
    /// Attend de manière asynchrone pendant `duration`.
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()>;
}

/// Implémentation système combinant horloge UTC, horloge monotone et minuteur Tokio.
#[derive(Debug)]
pub struct SystemClockAndTimer {
    origin: Instant,
}

impl SystemClockAndTimer {
    /// Construit l'horloge et le minuteur système ancrés à `Instant::now()`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClockAndTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockPort for SystemClockAndTimer {
    fn now_ms(&self) -> TimestampMs {
        TimestampMs::new(chrono::Utc::now().timestamp_millis())
    }
}

impl MonotonicClockPort for SystemClockAndTimer {
    fn monotonic_now(&self) -> Duration {
        self.origin.elapsed()
    }
}

impl SleepPort for SystemClockAndTimer {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Référence opaque et non vide vers une décision produite par `GuardPort` ou le broker local.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct GuardDecisionRef(String);

impl GuardDecisionRef {
    /// Valide et construit une référence de décision.
    ///
    /// # Errors
    /// Retourne [`ActionBrokerError::InvalidArgument`] si la chaîne est vide.
    pub fn new(raw: impl Into<String>) -> Result<Self, ActionBrokerError> {
        let s = raw.into();
        if s.trim().is_empty() {
            return Err(ActionBrokerError::InvalidArgument {
                reason: String::from("GuardDecisionRef ne peut pas être vide"),
            });
        }
        Ok(Self(s))
    }

    /// Représentation textuelle sous-jacente.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GuardDecisionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Mode d'intégration AI-Guard côté Unleash (`Q-04`, `ADR-10`, `ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GuardMode {
    /// Broker Unleash seul, sans endpoint AI-Guard configuré (défaut sans endpoint).
    #[default]
    BrokerOnly,
    /// Garde consultée hors `local_read` ; repli uniquement sur indisponibilité/timeout/protocole
    /// incompatible vers une `fallback_policy_ref` préautorisée et versionnée.
    GuardOptional {
        /// Référence versionnée d'une politique locale de repli préautorisée.
        fallback_policy_ref: String,
    },
    /// Garde strictement requise (fail-closed ; défaut dès qu'un endpoint est enregistré).
    GuardRequired,
}

/// Profil de mapping des obligations d'un `Allow` AI-Guard (`Q-04`, `R6`, `ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "profile_kind", rename_all = "snake_case")]
pub enum GuardMappingProfile {
    /// Profil initial décidé par Q-04 : toute contrainte additionnelle sur `Allow` est refusée
    /// (`CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED`) jusqu'à qualification `EXT-GUARD`.
    #[default]
    NoneAccepted,
    /// Profil explicitement qualifié décrivant les types d'obligations supportées.
    Qualified {
        /// Identifiant de profil versionné.
        profile_id: String,
        /// Numéro de version du profil.
        version: u32,
        /// Noms exacts des types d'obligations admis par ce profil.
        supported_obligations: BTreeSet<String>,
        /// Indique si ce profil qualifie la remise de `RequireApproval` au protocole réel.
        supports_remote_approval: bool,
    },
}

impl GuardMappingProfile {
    /// Identifiant textuel versionné du profil actif.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        match self {
            Self::NoneAccepted => NONE_ACCEPTED_PROFILE_ID,
            Self::Qualified { profile_id, .. } => profile_id,
        }
    }

    /// Indique si le profil autorise la mise en attente d'approbation humaine (`RequireApproval`).
    #[must_use]
    pub const fn supports_remote_approval(&self) -> bool {
        match self {
            Self::NoneAccepted => false,
            Self::Qualified {
                supports_remote_approval,
                ..
            } => *supports_remote_approval,
        }
    }
}

/// Exécuteur choisi pour une action autorisée (`ADR-10` R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorKind {
    /// Exécuteur interne Unleash (défaut absolu en V1).
    #[default]
    Unleash,
    /// Délégation d'exécution à AI-Guard uniquement après négociation et qualification.
    Guard,
}

/// Contrainte brute portée par une réponse `Allow` externe avant mapping interne.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardConstraint {
    /// Identifiant de contrainte.
    pub constraint_id: String,
    /// Type de contrainte (ex. `"egress_loopback_only"`, `"strict_fs"`).
    pub kind: String,
    /// Paramètres structurés de la contrainte.
    pub parameters: BTreeMap<String, String>,
    /// Groupe d'exclusion mutuelle détectant les contradictions entre obligations simultanées.
    pub contradiction_group: Option<String>,
    /// Indique si la contrainte exige un contrôle continu pendant l'exécution.
    pub continuous: bool,
}

/// Obligation interne typée issue du mapping d'une [`GuardConstraint`] (`ai-guard.md` R6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalObligation {
    /// Identifiant corrélé à la contrainte source.
    pub obligation_id: String,
    /// Mécanisme technique responsable de l'application.
    pub mechanism: String,
    /// Composant Unleash propriétaire du contrôle.
    pub responsible_component: String,
    /// Frontière d'application exacte.
    pub enforcement_boundary: String,
    /// Durée optionnelle de validité continue en ms.
    pub duration_ms: Option<u64>,
    /// Condition d'échec en cas de rupture.
    pub failure_condition: String,
    /// Capacité technique requise devant rester disponible jusqu'après engagement.
    pub required_capability: String,
}

/// Port indépendant vérifiant si une capacité d'obligation est réellement imposable au runtime.
pub trait ObligationCapabilityPort: Send + Sync {
    /// Retourne `true` uniquement si l'obligation peut être effectivement imposée au moment T.
    fn is_obligation_enforceable(&self, obligation: &InternalObligation) -> bool;
}

/// Implémentation par défaut refusant toute obligation additionnelle (alignée sur `none_accepted`).
#[derive(Debug, Default, Clone, Copy)]
pub struct StrictNoneAcceptedObligationChecker;

impl ObligationCapabilityPort for StrictNoneAcceptedObligationChecker {
    fn is_obligation_enforceable(&self, _obligation: &InternalObligation) -> bool {
        false
    }
}

/// Convertit et valide toutes les contraintes d'un `Allow` selon le `GuardMappingProfile` actif
/// et les capacités réelles d'application (`ai-guard.md` § R6).
///
/// # Errors
/// Retourne [`ActionBrokerError::GuardObligationUnsatisfied`] si le profil est `NoneAccepted`,
/// si une contrainte est inconnue, contradictoire ou non imposable.
pub fn map_and_validate_obligations(
    constraints: &[GuardConstraint],
    profile: &GuardMappingProfile,
    capability_port: &dyn ObligationCapabilityPort,
) -> Result<Vec<InternalObligation>, ActionBrokerError> {
    if constraints.is_empty() {
        return Ok(Vec::new());
    }

    let supported_set = match profile {
        GuardMappingProfile::NoneAccepted => {
            return Err(ActionBrokerError::GuardObligationUnsatisfied {
                reason: format!(
                    "Profil actif '{}' ({}) refuse toute obligation additionnelle sur Allow ({} contrainte(s) reçue(s))",
                    profile.profile_id(),
                    "none_accepted",
                    constraints.len()
                ),
            });
        }
        GuardMappingProfile::Qualified {
            supported_obligations,
            ..
        } => supported_obligations,
    };

    let mut seen_groups: BTreeSet<&str> = BTreeSet::new();
    let mut mapped = Vec::with_capacity(constraints.len());

    for constraint in constraints {
        if !supported_set.contains(&constraint.kind) {
            return Err(ActionBrokerError::GuardObligationUnsatisfied {
                reason: format!(
                    "Contrainte '{}' (kind='{}') inconnue ou absente du profil '{}'",
                    constraint.constraint_id,
                    constraint.kind,
                    profile.profile_id()
                ),
            });
        }

        if let Some(ref group) = constraint.contradiction_group
            && !seen_groups.insert(group.as_str())
        {
            return Err(ActionBrokerError::GuardObligationUnsatisfied {
                reason: format!("Obligations contradictoires détectées dans le groupe '{group}'"),
            });
        }

        let obligation = InternalObligation {
            obligation_id: constraint.constraint_id.clone(),
            mechanism: format!("enforcer:{}", constraint.kind),
            responsible_component: String::from("unleash-action-broker"),
            enforcement_boundary: if constraint.continuous {
                String::from("continuous_pre_and_during_execution")
            } else {
                String::from("pre_dispatch")
            },
            duration_ms: constraint
                .parameters
                .get("duration_ms")
                .and_then(|v| v.parse::<u64>().ok()),
            failure_condition: String::from("deny_and_abort_on_capability_loss"),
            required_capability: constraint.kind.clone(),
        };

        if !capability_port.is_obligation_enforceable(&obligation) {
            return Err(ActionBrokerError::GuardObligationUnsatisfied {
                reason: format!(
                    "Obligation '{}' (capacité '{}') non imposable par l'environnement actif",
                    obligation.obligation_id, obligation.required_capability
                ),
            });
        }

        mapped.push(obligation);
    }

    Ok(mapped)
}

/// Revalide juste avant le dispatch que toutes les obligations engagées restent imposables.
///
/// # Errors
/// Retourne [`ActionBrokerError::GuardObligationUnsatisfied`] en cas de perte de capacité.
pub fn revalidate_obligations(
    obligations: &[InternalObligation],
    capability_port: &dyn ObligationCapabilityPort,
) -> Result<(), ActionBrokerError> {
    for obl in obligations {
        if !capability_port.is_obligation_enforceable(obl) {
            return Err(ActionBrokerError::GuardObligationUnsatisfied {
                reason: format!(
                    "Perte de capacité avant dispatch pour l'obligation '{}' ('{}')",
                    obl.obligation_id, obl.required_capability
                ),
            });
        }
    }
    Ok(())
}

/// Requête de négociation des capacités soumise à [`GuardPort::negotiate_capabilities`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateCapabilitiesRequest {
    /// Version de port demandée par Unleash (`unleash.guard_port.v1`).
    pub supported_protocol_versions: Vec<String>,
    /// Profil de mapping demandé côté Unleash.
    pub requested_mapping_profile: GuardMappingProfile,
    /// Exécuteur souhaité (`Unleash` par défaut).
    pub requested_executor: ExecutorKind,
}

/// Capacités négociées auprès d'une implémentation de [`GuardPort`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independently negotiated protocol capability flags do not encode mutually exclusive states"
)]
pub struct GuardCapabilities {
    /// Version du protocole négociée.
    pub protocol_version: String,
    /// Profil de mapping effectif.
    pub mapping_profile: GuardMappingProfile,
    /// Indique si AI-Guard accepte `SuspendSubject`/`ResumeSubject` distants.
    pub supports_suspend_resume: bool,
    /// Indique si AI-Guard accepte l'exécution déléguée (`ExecuteAuthorizedAction`).
    pub supports_execute_authorized_action: bool,
    /// Indique si AI-Guard et le profil supportent le flux `RequireApproval`.
    pub supports_remote_approval: bool,
    /// Indique si la modification de politique distante est supportée.
    pub supports_remote_policy_write: bool,
    /// Catalogue des obligations comprises par ce profil.
    pub supported_obligations: BTreeSet<String>,
}

/// État de santé observable de la connexion AI-Guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardHealthStatus {
    /// Connexion active et saine.
    Healthy,
    /// Connexion partiellement dégradée.
    Degraded,
    /// Service inaccessible ou en panne.
    Unavailable,
    /// Aucun endpoint configuré (`broker_only`).
    Disconnected,
}

/// Rapport de santé retourné par [`GuardPort::check_health`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardHealthReport {
    /// Statut de santé observé.
    pub status: GuardHealthStatus,
    /// Version du protocole annoncée.
    pub protocol_version: String,
    /// Dernière révision de politique connue côté garde.
    pub policy_revision: Option<Revision>,
    /// Horodatage UTC de la sonde.
    pub checked_at_ms: TimestampMs,
    /// Détail diagnostic sans secret.
    pub detail: Option<String>,
}

/// Projection typée du sujet envoyée dans [`GuardEvaluateRequest`] (sans identité agente factice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardSubjectSnapshot {
    /// Nature du sujet (`invocation`, `replay`, `control_plane`).
    pub kind: SubjectKind,
    /// Principal authentifié.
    pub principal_id: ActorId,
    /// Référence causale authentifiée.
    pub cause_ref: String,
    /// Identifiant d'agent (strictement `None` hors `Invocation`).
    pub agent_id: Option<AgentId>,
    /// Version de rôle éventuelle.
    pub role_version: Option<RoleVersionRef>,
    /// Mission associée éventuelle.
    pub mission_id: Option<MissionId>,
}

/// Requête complète d'évaluation d'une action transmise à [`GuardPort::evaluate_action`] (`ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardEvaluateRequest {
    /// Internal durable evaluation identity; no external idempotency contract is implied.
    pub evaluation_id: unleash_domain::AuthorizationEvaluationId,
    /// Identifiant unique de l'action.
    pub action_id: ActionId,
    /// Sujet typé et authentifié.
    pub subject: GuardSubjectSnapshot,
    /// Projet concerné si applicable.
    pub project_id: Option<ProjectId>,
    /// Révision du scope/mandat.
    pub scope_revision: Revision,
    /// Verbe de l'action.
    pub verb: String,
    /// Références de ressources expurgées (sans secrets).
    pub resource_refs: Vec<String>,
    /// Empreinte canonique RFC 8785 du payload (`actions.payload_hash`).
    pub payload_digest: Sha256Digest,
    /// Classe d'effet fermée (jamais `local_read` en principe).
    pub effects_class: EffectsClass,
    /// Révision de la politique de sécurité.
    pub policy_revision: Revision,
    /// Échéance limite de l'action.
    pub action_deadline: TimestampMs,
    /// Exigences d'isolation dérivées de l'autorité effective.
    pub isolation_requirements: IsolationRequirements,
}

impl GuardEvaluateRequest {
    /// Construit la requête expurgée depuis une [`ActionIntent`] validée.
    #[must_use]
    pub fn from_intent(
        intent: &ActionIntent,
        evaluation_id: unleash_domain::AuthorizationEvaluationId,
    ) -> Self {
        Self {
            evaluation_id,
            action_id: intent.action_id,
            subject: GuardSubjectSnapshot {
                kind: intent.subject.kind(),
                principal_id: intent.subject.principal_id().clone(),
                cause_ref: intent.subject.cause_ref().to_owned(),
                agent_id: intent.subject.agent_id(),
                role_version: intent.subject.role_version().cloned(),
                mission_id: intent.subject.mission_id(),
            },
            project_id: intent.project_id,
            scope_revision: intent.scope_revision,
            verb: intent.verb.clone(),
            resource_refs: intent
                .resource_refs
                .iter()
                .map(crate::effects::NormalizedResourceRef::as_sanitized_label)
                .collect(),
            payload_digest: intent.payload_digest,
            effects_class: intent.effects_class,
            policy_revision: intent.policy_revision,
            action_deadline: intent.action_deadline,
            isolation_requirements: intent.isolation_requirements.clone(),
        }
    }
}

/// Réponse normalisée retournée par [`GuardPort::evaluate_action`] (`ai-guard.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum GuardEvaluationResponse {
    /// Autorisation accordée sous contraintes optionnelles et borne `expiry` distincte de `action_deadline`.
    Allow {
        /// Référence opaque de la décision côté garde.
        decision_ref: GuardDecisionRef,
        /// Obligations/contraintes imposées par AI-Guard (validées via [`GuardMappingProfile`]).
        constraints: Vec<GuardConstraint>,
        /// Date d'expiration UTC de la décision (sans `expiry`, aucun cache autorisé).
        expiry: Option<TimestampMs>,
    },
    /// Refus explicite et définitif pour ce digest.
    Deny {
        /// Code de refus.
        reason_code: String,
        /// Détail lisible sans secret.
        reason_detail: String,
        /// Référence de décision côté garde.
        decision_ref: GuardDecisionRef,
    },
    /// Approbation humaine requise sur le périmètre exact.
    RequireApproval {
        /// Périmètre d'approbation corrélé.
        approval_scope: ApprovalScopeId,
        /// Référence de décision côté garde.
        decision_ref: GuardDecisionRef,
        /// Motif de la demande d'approbation.
        reason: String,
    },
    /// Service AI-Guard temporairement indisponible.
    Unavailable {
        /// Motif d'indisponibilité.
        reason: String,
    },
    /// Capacité requise ou version non supportée.
    UnsupportedCapability {
        /// Capacité manquante.
        capability: String,
        /// Détail technique.
        detail: String,
    },
}

/// Requête générique de suspension, reprise ou révocation transmise à `GuardPort`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardSubjectActionRequest {
    /// Clé canonique du sujet ciblé.
    pub subject_key: String,
    /// Projet éventuel.
    pub project_id: Option<ProjectId>,
    /// Motif administratif tracé.
    pub reason: String,
    /// Révision locale associée.
    pub revision: Revision,
}

/// Reçu distant de révocation d'un sujet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardRevocationReceipt {
    /// Référence distante d'audit de la révocation.
    pub remote_ref: String,
    /// Horodatage UTC de prise d'effet distante.
    pub revoked_at_ms: TimestampMs,
}

/// Reçu distant de suspension ou reprise d'un sujet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardSuspensionReceipt {
    /// Référence distante de la suspension/reprise.
    pub remote_ref: String,
    /// Horodatage UTC côté garde.
    pub recorded_at_ms: TimestampMs,
}

/// Résumé de politique lu auprès de `GuardPort::read_policy_summary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardRemotePolicySummary {
    /// Révision de politique distante.
    pub policy_revision: Revision,
    /// Indique si les mutations de politique à distance sont autorisées.
    pub remote_write_supported: bool,
    /// URL d'administration externe optionnelle.
    pub external_admin_url: Option<String>,
    /// Résumé lisible.
    pub summary: String,
}

/// Référence d'audit distante corrélée à une action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardAuditReference {
    /// Identifiant d'audit distant.
    pub audit_id: String,
    /// Statut observé côté garde.
    pub status: String,
    /// Horodatage UTC distant.
    pub observed_at_ms: TimestampMs,
}

/// Requête d'exécution déléguée vers AI-Guard lorsque `executor=guard` est négocié et qualifié.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardExecuteRequest {
    /// Identifiant de l'action.
    pub action_id: ActionId,
    /// Référence de décision préalable.
    pub decision_ref: GuardDecisionRef,
    /// Verbe de l'action.
    pub verb: String,
    /// Empreinte du payload.
    pub payload_digest: Sha256Digest,
}

/// Issue retournée par [`GuardPort::execute_authorized_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardExecutionOutcome {
    /// Référence distante d'exécution.
    pub execution_ref: String,
    /// Succès confirmé (`true`) ou échec confirmé sans effet (`false`).
    pub succeeded: bool,
    /// Empreinte du résultat observé.
    pub output_digest: Sha256Digest,
    /// Horodatage UTC de l'observation.
    pub observed_at_ms: TimestampMs,
}

/// Port interne abstrait et versionné d'intégration AI-Guard (`F017`, `ADR-10`, `ai-guard.md`).
///
/// L'implémentation réelle vit dans la crate séparée `unleash-guard-adapter` (après qualification
/// `EXT-GUARD`) ; le double déterministe vit dans `unleash-testkit`.
pub trait GuardPort: Send + Sync {
    /// Version de contrat `GuardPort` exposée par cet adaptateur.
    fn protocol_version(&self) -> &'static str;

    /// Négocie version, profil de mapping et capacités supportées (`NegotiateCapabilities`).
    fn negotiate_capabilities(
        &self,
        request: NegotiateCapabilitiesRequest,
    ) -> BoxFuture<'_, Result<GuardCapabilities, GuardPortError>>;

    /// Contrôle la santé et la disponibilité de la connexion AI-Guard (`CheckHealth`).
    fn check_health(&self) -> BoxFuture<'_, Result<GuardHealthReport, GuardPortError>>;

    /// Évalue une intention d'action expurgée (`EvaluateAction`).
    fn evaluate_action<'a>(
        &'a self,
        request: &'a GuardEvaluateRequest,
    ) -> BoxFuture<'a, Result<GuardEvaluationResponse, GuardPortError>>;

    /// Propage la révocation définitive d'un sujet vers AI-Guard (`RevokeSubject`).
    fn revoke_subject<'a>(
        &'a self,
        request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardRevocationReceipt, GuardPortError>>;

    /// Lit le résumé de politique distante (`ReadPolicySummary`).
    fn read_policy_summary(
        &self,
        project_id: Option<ProjectId>,
    ) -> BoxFuture<'_, Result<GuardRemotePolicySummary, GuardPortError>>;

    /// Lit une référence d'audit distante corrélée à l'action (`ReadAuditReference`).
    fn read_audit_reference<'a>(
        &'a self,
        action_id: ActionId,
        decision_ref: Option<&'a GuardDecisionRef>,
    ) -> BoxFuture<'a, Result<GuardAuditReference, GuardPortError>>;

    /// Méthode optionnelle de propagation de suspension locale (`SuspendSubject`).
    fn suspend_subject<'a>(
        &'a self,
        _request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardSuspensionReceipt, GuardPortError>> {
        Box::pin(async {
            Err(GuardPortError::UnsupportedCapability {
                capability: String::from("SuspendSubject"),
                protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
            })
        })
    }

    /// Méthode optionnelle de propagation de levée de suspension (`ResumeSubject`).
    fn resume_subject<'a>(
        &'a self,
        _request: &'a GuardSubjectActionRequest,
    ) -> BoxFuture<'a, Result<GuardSuspensionReceipt, GuardPortError>> {
        Box::pin(async {
            Err(GuardPortError::UnsupportedCapability {
                capability: String::from("ResumeSubject"),
                protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
            })
        })
    }

    /// Méthode optionnelle d'exécution déléguée (`ExecuteAuthorizedAction`, active seulement si
    /// négociée et qualifiée).
    fn execute_authorized_action<'a>(
        &'a self,
        _request: &'a GuardExecuteRequest,
    ) -> BoxFuture<'a, Result<GuardExecutionOutcome, GuardPortError>> {
        Box::pin(async {
            Err(GuardPortError::UnsupportedCapability {
                capability: String::from("ExecuteAuthorizedAction"),
                protocol_version: Some(String::from(GUARD_PORT_PROTOCOL_VERSION)),
            })
        })
    }
}
