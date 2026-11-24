//! Persistance transactionnelle des politiques de sécurité, configuration AI-Guard,
//! suspensions/révocations de sujets, références secrètes opaques (zéro octet secret) et alertes.

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActorId, GuardMode, IdGeneratorPort, ProjectId, Revision, SecretRef, Sha256Digest, TimestampMs,
};
use unleash_protocol::{ReasonCode, canonical_sha256};

use crate::error::StoreError;

/// Rôle effectif de l'appelant résolu côté serveur pour le contrôle d'autorité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerRole {
    /// Opérateur humain authentifié.
    HumanOperator,
    /// Administrateur de politique habilité.
    PolicyAdmin,
    /// Adaptateur / coordinateur habilité pour la réconciliation factuelle.
    ReconcileCoordinator,
    /// Service interne de maintenance du daemon.
    SystemMaintenance,
    /// Agent d'exécution (jamais autorisé à décider d'une approbation ou disposition humaine).
    Agent,
    /// Modèle de langage (jamais décideur d'autorité).
    Model,
}

impl CallerRole {
    /// Représentation textuelle canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::HumanOperator => "human_operator",
            Self::PolicyAdmin => "policy_admin",
            Self::ReconcileCoordinator => "reconcile_coordinator",
            Self::SystemMaintenance => "system_maintenance",
            Self::Agent => "agent",
            Self::Model => "model",
        }
    }

    /// Indique si ce rôle peut rendre une décision humaine ou administrative (`ResolveActionApproval`, `ResolveActionOutcome`).
    #[must_use]
    pub const fn is_human_decision_maker(&self) -> bool {
        matches!(self, Self::HumanOperator | Self::PolicyAdmin)
    }

    /// Indique si ce rôle peut exécuter une réconciliation factuelle (`ReconcileAction`).
    #[must_use]
    pub const fn can_reconcile(&self) -> bool {
        matches!(
            self,
            Self::HumanOperator | Self::PolicyAdmin | Self::ReconcileCoordinator
        )
    }

    /// Indique si ce rôle peut modifier politiques, gardes, secrets, sujets ou paramètres globaux.
    #[must_use]
    pub const fn can_administer_security(&self) -> bool {
        matches!(self, Self::HumanOperator | Self::PolicyAdmin)
    }
}

/// Autorité authentifiée de l'appelant pour les mutations sensibles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerAuthority {
    /// Identifiant d'acteur authentifié résolu serveur.
    pub actor_id: ActorId,
    /// Rôle d'autorité associé.
    pub role: CallerRole,
}

pub(crate) fn require_human_decider(
    caller: &CallerAuthority,
    operation: &str,
) -> Result<(), StoreError> {
    if caller.role.is_human_decision_maker() {
        return Ok(());
    }
    Err(StoreError::Unauthorized(format!(
        "rôle '{}' non habilité pour '{operation}' (décideur humain requis)",
        caller.role.as_str()
    )))
}

pub(crate) fn require_reconciler(
    caller: &CallerAuthority,
    operation: &str,
) -> Result<(), StoreError> {
    if caller.role.can_reconcile() {
        return Ok(());
    }
    Err(StoreError::Unauthorized(format!(
        "rôle '{}' non habilité pour '{operation}'",
        caller.role.as_str()
    )))
}

pub(crate) fn require_security_admin(
    caller: &CallerAuthority,
    operation: &str,
) -> Result<(), StoreError> {
    if caller.role.can_administer_security() {
        return Ok(());
    }
    Err(StoreError::Unauthorized(format!(
        "rôle '{}' non habilité à administrer '{operation}'",
        caller.role.as_str()
    )))
}

/// Portée d'une politique de sécurité (`global` ou `project`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyScopeKind {
    /// Politique globale par défaut.
    Global,
    /// Politique spécifique à un projet.
    Project,
}

impl PolicyScopeKind {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// Mode d'isolation d'exécution (Q-05 : `standard` ou `strict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyIsolationMode {
    /// Mode standard par défaut.
    Standard,
    /// Mode strict tracé.
    Strict,
}

impl PolicyIsolationMode {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Strict => "strict",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "standard" => Ok(Self::Standard),
            "strict" => Ok(Self::Strict),
            other => Err(StoreError::InvariantViolation(format!(
                "isolation_mode inconnu : '{other}'"
            ))),
        }
    }
}

/// Politique de sécurité persistée dans `security_policies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityPolicyRecord {
    /// Portée (`global` ou `project`).
    pub scope_kind: PolicyScopeKind,
    /// Identifiant de portée (`"global"` ou `project_id`).
    pub scope_id: String,
    /// Mode d'isolation effectif.
    pub isolation_mode: PolicyIsolationMode,
    /// Version logique de politique.
    pub policy_version: String,
    /// Empreinte canonique RFC 8785 / SHA-256.
    pub policy_digest: Sha256Digest,
    /// Motif tracé de la modification.
    pub reason: String,
    /// Acteur ayant engagé la révision.
    pub updated_by_actor: String,
    /// Révision optimiste.
    pub revision: Revision,
    /// Date de création.
    pub created_at_ms: TimestampMs,
    /// Date de dernière mise à jour.
    pub updated_at_ms: TimestampMs,
}

/// Mode de gouvernance AI-Guard côté configuration (`broker_only`, `guard_optional`, `guard_required`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardPolicyMode {
    /// Seul le broker local évalue sans appel externe (défaut sans endpoint).
    BrokerOnly,
    /// Garde optionnelle : repli vers `fallback_policy_ref` si indisponible.
    GuardOptional,
    /// Garde strictement obligatoire : fail-closed (défaut dès qu'un endpoint est configuré).
    GuardRequired,
}

impl GuardPolicyMode {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BrokerOnly => "broker_only",
            Self::GuardOptional => "guard_optional",
            Self::GuardRequired => "guard_required",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "broker_only" => Ok(Self::BrokerOnly),
            "guard_optional" => Ok(Self::GuardOptional),
            "guard_required" => Ok(Self::GuardRequired),
            other => Err(StoreError::InvariantViolation(format!(
                "guard_mode inconnu : '{other}'"
            ))),
        }
    }
}

impl From<GuardMode> for GuardPolicyMode {
    fn from(value: GuardMode) -> Self {
        match value {
            GuardMode::GuardRequired => Self::GuardRequired,
            GuardMode::GuardOptional => Self::GuardOptional,
        }
    }
}

/// Transport supporté vers AI-Guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardTransport {
    /// Aucun transport configuré (mode `broker_only`).
    None,
    /// Socket Unix local avec vérification UID pair.
    UnixSocket,
    /// HTTPS avec TLS vérifié.
    HttpsTls,
    /// HTTPS mTLS avec identité client.
    HttpsMtls,
}

impl GuardTransport {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::UnixSocket => "unix_socket",
            Self::HttpsTls => "https_tls",
            Self::HttpsMtls => "https_mtls",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "none" => Ok(Self::None),
            "unix_socket" => Ok(Self::UnixSocket),
            "https_tls" => Ok(Self::HttpsTls),
            "https_mtls" => Ok(Self::HttpsMtls),
            other => Err(StoreError::InvariantViolation(format!(
                "transport AI-Guard inconnu : '{other}'"
            ))),
        }
    }
}

/// État de santé observé du service AI-Guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardHealthStatus {
    /// Aucune sonde vérifiée pour le moment.
    Unknown,
    /// Service sain et qualifié.
    Healthy,
    /// Dégradé (latence/mapping partiel).
    Degraded,
    /// Indisponible.
    Unavailable,
}

impl GuardHealthStatus {
    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "unknown" => Ok(Self::Unknown),
            "healthy" => Ok(Self::Healthy),
            "degraded" => Ok(Self::Degraded),
            "unavailable" => Ok(Self::Unavailable),
            other => Err(StoreError::InvariantViolation(format!(
                "health_status inconnu : '{other}'"
            ))),
        }
    }
}

/// Entrée de configuration pour `ConfigureGuard`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardConfigInput {
    /// Identifiant de configuration (ex. `"default"`).
    pub config_id: String,
    /// Mode de garde (`broker_only`, `guard_optional`, `guard_required`).
    pub guard_mode: GuardPolicyMode,
    /// Endpoint optionnel (`unix://...` ou `https://...`).
    pub endpoint: Option<String>,
    /// Transport associé.
    pub transport: GuardTransport,
    /// Référence secrète opaque éventuelle (jamais d'octet secret en clair).
    pub secret_ref_id: Option<String>,
    /// Délai maximal d'évaluation en ms (100..=60000, défaut 2000).
    pub evaluate_timeout_ms: u32,
    /// Intervalle de sonde de santé en secondes (5..=3600, défaut 30).
    pub health_check_interval_s: u32,
    /// Durée maximale de réemploi d'une décision en secondes (0..=3600, défaut 300).
    pub decision_cache_max_age_s: u32,
    /// Politique locale de repli obligatoire pour `guard_optional`.
    pub fallback_policy_ref: Option<String>,
    /// Identifiant de profil de mapping vérifié (`EXT-GUARD`).
    pub guard_mapping_profile: String,
    /// Motif du changement de configuration.
    pub reason: String,
}

/// Configuration persistée d'AI-Guard dans `guard_configs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardConfigRecord {
    /// Identifiant de configuration.
    pub config_id: String,
    /// Mode actif.
    pub guard_mode: GuardPolicyMode,
    /// Endpoint éventuel.
    pub endpoint: Option<String>,
    /// Transport configuré.
    pub transport: GuardTransport,
    /// Référence secrète opaque éventuelle.
    pub secret_ref_id: Option<String>,
    /// Timeout d'évaluation en ms.
    pub evaluate_timeout_ms: u32,
    /// Période de vérification santé en s.
    pub health_check_interval_s: u32,
    /// Âge maximal de réemploi de décision en s.
    pub decision_cache_max_age_s: u32,
    /// Politique de repli éventuelle.
    pub fallback_policy_ref: Option<String>,
    /// Profil de mapping actif.
    pub guard_mapping_profile: String,
    /// État de santé observé.
    pub health_status: GuardHealthStatus,
    /// Version négociée.
    pub negotiated_version: Option<String>,
    /// Capacités annoncées.
    pub capabilities: Vec<String>,
    /// Horodatage de dernière vérification.
    pub last_health_check_ms: Option<TimestampMs>,
    /// Motif de dernière modification.
    pub reason: String,
    /// Acteur modificateur.
    pub updated_by_actor: String,
    /// Révision optimiste.
    pub revision: Revision,
    /// Horodatage de mise à jour.
    pub updated_at_ms: TimestampMs,
}

/// Type de sujet gouvernable par `SuspendSubject` / `ResumeSubject` / `RevokeSubject`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// Agent logique ou worker.
    Agent,
    /// Acteur client / principal.
    Actor,
    /// Compte fournisseur de modèles.
    ProviderAccount,
    /// Compte connecteur documentaire.
    ConnectorAccount,
    /// Skill ou version publiée.
    Skill,
    /// Automatisation réutilisable.
    Automation,
    /// Client IPC enregistré.
    Client,
    /// Définition ou run de workflow.
    Workflow,
}

impl SubjectKind {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Actor => "actor",
            Self::ProviderAccount => "provider_account",
            Self::ConnectorAccount => "connector_account",
            Self::Skill => "skill",
            Self::Automation => "automation",
            Self::Client => "client",
            Self::Workflow => "workflow",
        }
    }
}

/// État courant d'un sujet gouverné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectStatus {
    /// Sujet actif et autorisé.
    Active,
    /// Suspension locale temporaire et réversible par `ResumeSubject`.
    Suspended,
    /// Révocation définitive (distincte d'une suspension, non réactivable).
    Revoked,
}

impl SubjectStatus {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Revoked => "revoked",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "revoked" => Ok(Self::Revoked),
            other => Err(StoreError::InvariantViolation(format!(
                "subject status inconnu : '{other}'"
            ))),
        }
    }
}

/// État de propagation d'une suspension/révocation vers AI-Guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardPropagationState {
    /// Appliquée localement uniquement (aucun garde externe actif).
    LocalOnly,
    /// Propagée avec succès via `GuardPort`.
    Propagated,
    /// Limitation locale uniquement car le garde distant ne l'a pas négociée.
    UnnegotiatedWarning,
}

impl GuardPropagationState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Propagated => "propagated",
            Self::UnnegotiatedWarning => "unnegotiated_warning",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "local_only" => Ok(Self::LocalOnly),
            "propagated" => Ok(Self::Propagated),
            "unnegotiated_warning" => Ok(Self::UnnegotiatedWarning),
            other => Err(StoreError::InvariantViolation(format!(
                "guard_propagation_state inconnu : '{other}'"
            ))),
        }
    }
}

/// Enregistrement d'état de suspension ou révocation d'un sujet (`subject_suspensions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectSuspensionRecord {
    /// Identifiant technique de l'entrée.
    pub suspension_id: String,
    /// Nature typée du sujet.
    pub subject_kind: SubjectKind,
    /// Identifiant du sujet.
    pub subject_id: String,
    /// Portée (`global`, `project`, etc.).
    pub scope_kind: String,
    /// Identifiant de portée.
    pub scope_id: String,
    /// État courant (`active`, `suspended`, `revoked`).
    pub status: SubjectStatus,
    /// Motif de la dernière transition.
    pub reason: String,
    /// Visibilité de propagation AI-Guard.
    pub guard_propagation_state: GuardPropagationState,
    /// Acteur ayant effectué la transition.
    pub acted_by_actor: String,
    /// Révision optimiste.
    pub revision: Revision,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
    /// Horodatage de mise à jour.
    pub updated_at_ms: TimestampMs,
}

/// Synthèse agrégée de politique et garde lue par `ReadPolicySummary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySummary {
    /// Politique globale éventuelle.
    pub global_policy: Option<SecurityPolicyRecord>,
    /// Politique du projet ciblé éventuelle.
    pub project_policy: Option<SecurityPolicyRecord>,
    /// Mode d'isolation effectif résultant (`strict` si global ou projet l'exige).
    pub effective_isolation_mode: PolicyIsolationMode,
    /// Configuration AI-Guard par défaut éventuelle.
    pub guard_config: Option<GuardConfigRecord>,
    /// Nombre de sujets actuellement suspendus localement.
    pub active_suspensions_count: u64,
    /// Nombre de sujets révoqués.
    pub active_revocations_count: u64,
    /// Mandats fournisseur visibles dans les mêmes portées que les politiques.
    #[serde(default)]
    pub provider_probe_mandates: Vec<crate::ProviderProbeMandateView>,
}

/// État courant d'une référence secrète opaque (`secret_references.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretRefState {
    /// Import en cours sur canal protégé.
    PendingImport,
    /// Référence active dans le coffre OS.
    Active,
    /// Coffre verrouillé / inaccessible.
    Locked,
    /// Révoquée localement (sans effacement implicite distant).
    RevokedLocal,
    /// Révoquée localement et confirmée côté distant.
    RevokedRemote,
}

impl SecretRefState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PendingImport => "pending_import",
            Self::Active => "active",
            Self::Locked => "locked",
            Self::RevokedLocal => "revoked_local",
            Self::RevokedRemote => "revoked_remote",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "pending_import" => Ok(Self::PendingImport),
            "active" => Ok(Self::Active),
            "locked" => Ok(Self::Locked),
            "revoked_local" => Ok(Self::RevokedLocal),
            "revoked_remote" => Ok(Self::RevokedRemote),
            other => Err(StoreError::InvariantViolation(format!(
                "secret status inconnu : '{other}'"
            ))),
        }
    }
}

/// État de révocation distante distinct de la révocation locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRevocationState {
    /// Non applicable avant révocation.
    NotApplicable,
    /// Le fournisseur/coffre distant ne prend pas en charge la révocation distante.
    Unsupported,
    /// Révocation distante en attente de confirmation.
    Pending,
    /// Révocation distante confirmée.
    Confirmed,
    /// Révocation distante en échec.
    Failed,
}

impl RemoteRevocationState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::Unsupported => "unsupported",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "not_applicable" => Ok(Self::NotApplicable),
            "unsupported" => Ok(Self::Unsupported),
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "failed" => Ok(Self::Failed),
            other => Err(StoreError::InvariantViolation(format!(
                "remote_revocation_state inconnu : '{other}'"
            ))),
        }
    }
}

/// Entrée de liaison d'une référence opaque vers un secret dans le coffre OS (`SetSecretReference`).
///
/// Ne contient JAMAIS d'octets secrets bruts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReferenceInput {
    /// Identifiant logique de la liaison (`aggregate_id` / PK `secret_references`).
    pub secret_ref_id: String,
    /// Référence opaque validée vers le coffre OS.
    pub secret_ref: SecretRef,
    /// Espace de noms (`provider`, `connector`, `guard`, `automation`).
    pub namespace: String,
    /// Finalité (`api_key`, `oauth_refresh_token`, `mtls_client_token`, ...).
    pub purpose: String,
    /// Type fonctionnel (`bearer_token`, `oauth2_refresh`, ...).
    pub secret_kind: String,
    /// Type de cible liée (`provider_profile`, `connector_account`, `guard_config`, ...).
    pub target_kind: String,
    /// Identifiant de la cible.
    pub target_id: String,
    /// Portée (`user`, `project`).
    pub scope_kind: String,
    /// Identifiant de portée.
    pub scope_id: String,
    /// Coffre OS cible (`os_keychain`, `secret_service`, ...).
    pub backend_kind: String,
}

/// Métadonnées de statut retournées par `GetSecretReferenceStatus` (aucun octet secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReferenceStatus {
    /// Identifiant logique de la liaison.
    pub secret_ref_id: String,
    /// Référence opaque (non résolue).
    pub opaque_secret_ref: SecretRef,
    /// Espace de noms.
    pub namespace: String,
    /// Finalité.
    pub purpose: String,
    /// Type de secret.
    pub secret_kind: String,
    /// Type de cible liée.
    pub target_kind: String,
    /// Identifiant de cible.
    pub target_id: String,
    /// Portée.
    pub scope_kind: String,
    /// Identifiant de portée.
    pub scope_id: String,
    /// Backend de coffre OS.
    pub backend_kind: String,
    /// État de disponibilité/révocation.
    pub status: SecretRefState,
    /// Horodatage de révocation locale éventuelle.
    pub local_revoked_at_ms: Option<TimestampMs>,
    /// État explicite de révocation distante.
    pub remote_revocation_state: RemoteRevocationState,
    /// Révision optimiste.
    pub revision: Revision,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
    /// Horodatage de dernière modification.
    pub updated_at_ms: TimestampMs,
}

/// Sévérité d'une alerte persistée (`alerts.severity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// Informationnel.
    Info,
    /// Avertissement (ex. préavis 24h rétention, limite locale suspension).
    Warning,
    /// Erreur opérationnelle.
    Error,
    /// Blocage nécessitant une action ou décision explicite.
    Blocking,
}

impl AlertSeverity {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Blocking => "blocking",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            "blocking" => Ok(Self::Blocking),
            other => Err(StoreError::InvariantViolation(format!(
                "alert severity inconnue : '{other}'"
            ))),
        }
    }
}

/// État du cycle de vie d'une alerte (`alerts.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    /// Active et à afficher.
    Active,
    /// Acquittée par l'utilisateur.
    Acknowledged,
    /// Résolue.
    Resolved,
    /// Remplacée par une alerte plus récente.
    Superseded,
}

impl AlertState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Acknowledged => "acknowledged",
            Self::Resolved => "resolved",
            Self::Superseded => "superseded",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "active" => Ok(Self::Active),
            "acknowledged" => Ok(Self::Acknowledged),
            "resolved" => Ok(Self::Resolved),
            "superseded" => Ok(Self::Superseded),
            other => Err(StoreError::InvariantViolation(format!(
                "alert state inconnu : '{other}'"
            ))),
        }
    }
}

/// Alerte durable persistée dans la table `alerts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRecord {
    /// Identifiant unique de l'alerte.
    pub alert_id: String,
    /// Catégorie (`recovery_blocked`, `retention_warning`, `storage_corrupt`, ...).
    pub kind: String,
    /// Sévérité.
    pub severity: AlertSeverity,
    /// État courant.
    pub state: AlertState,
    /// Indique si l'alerte bloque la progression ou doit remonter hors filtre (`PAR-OVERVIEW-05`).
    pub blocking: bool,
    /// Projet associé éventuel.
    pub project_id: Option<ProjectId>,
    /// Type de sujet visé.
    pub subject_kind: String,
    /// Identifiant du sujet visé.
    pub subject_id: String,
    /// Titre synthétique.
    pub title: String,
    /// Détail sans secret.
    pub summary: String,
    /// Code de raison éventuel.
    pub reason_code: Option<String>,
    /// Référence de preuve éventuelle.
    pub evidence_ref: Option<String>,
    /// Opérations de remédiation autorisées.
    pub allowed_operations: Vec<String>,
    /// Révision optimiste.
    pub revision: Revision,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
    /// Horodatage de mise à jour.
    pub updated_at_ms: TimestampMs,
    /// Horodatage de résolution éventuel.
    pub resolved_at_ms: Option<TimestampMs>,
}

/// Spécification de création ou mise à jour d'une alerte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAlertSpec {
    /// Identifiant de l'alerte.
    pub alert_id: String,
    /// Catégorie fonctionnelle.
    pub kind: String,
    /// Sévérité.
    pub severity: AlertSeverity,
    /// Alerte bloquante remontée même hors filtre.
    pub blocking: bool,
    /// Projet associé optionnel.
    pub project_id: Option<ProjectId>,
    /// Type d'objet concerné.
    pub subject_kind: String,
    /// Identifiant de l'objet concerné.
    pub subject_id: String,
    /// Titre court.
    pub title: String,
    /// Résumé opérationnel (aucun secret).
    pub summary: String,
    /// Code de raison canonique éventuel.
    pub reason_code: Option<String>,
    /// Référence de preuve éventuelle.
    pub evidence_ref: Option<String>,
    /// Opérations de résolution proposées.
    pub allowed_operations: Vec<String>,
}

/// Filtre de requête pour `ListAlerts`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAlertsFilter {
    /// Filtrer par projet éventuel.
    pub project_id: Option<ProjectId>,
    /// Filtrer par état éventuel.
    pub state: Option<AlertState>,
    /// Filtrer par sévérité éventuelle.
    pub severity: Option<AlertSeverity>,
    /// Conserver les alertes bloquantes actives même si le filtre projet/état les masque (`PAR-OVERVIEW-05`).
    pub include_blocking_regardless_of_filter: bool,
}

fn require_non_empty(value: &str, field: &str) -> Result<String, StoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("champ obligatoire '{field}' vide"),
        });
    }
    Ok(trimmed.to_owned())
}

pub(crate) fn compute_policy_digest(
    scope_kind: PolicyScopeKind,
    scope_id: &str,
    isolation_mode: PolicyIsolationMode,
    policy_version: &str,
) -> Result<Sha256Digest, StoreError> {
    let payload = serde_json::json!({
        "scope_kind": scope_kind.as_str(),
        "scope_id": scope_id,
        "isolation_mode": isolation_mode.as_str(),
        "policy_version": policy_version,
    });
    canonical_sha256(&payload).map_err(StoreError::from)
}

/// Empreinte canonique de la politique engagée pour une portée, calculée depuis un résumé
/// lu dans une seule transaction lectrice. Forme unique, partagée par les écritures du
/// magasin et par les courtiers réservés du démon.
///
/// # Errors
/// Retourne les erreurs de sérialisation canonique.
pub fn engaged_policy_digest(summary: &PolicySummary) -> Result<Sha256Digest, StoreError> {
    canonical_sha256(
        &serde_json::json!({
            "schema_version": 1,
            "global_policy": summary.global_policy,
            "project_policy": summary.project_policy,
            "guard_config": summary.guard_config,
        }),
    )
    .map_err(StoreError::from)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Mutation transactionnelle atomique SetPolicy avec contrôle d'autorité et digest canonique"
)]
pub(crate) fn tx_set_policy(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    scope_kind: PolicyScopeKind,
    scope_id: &str,
    isolation_mode: PolicyIsolationMode,
    policy_version: &str,
    reason: &str,
    caller: &CallerAuthority,
) -> Result<Revision, StoreError> {
    require_security_admin(caller, "SetPolicy")?;
    let scope_id = require_non_empty(scope_id, "scope_id")?;
    if scope_kind == PolicyScopeKind::Project {
        let project_id = scope_id.parse::<ProjectId>()?;
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if exists != 1 {
            return Err(StoreError::NotFound(format!("project {project_id}")));
        }
    }
    let policy_version = require_non_empty(policy_version, "policy_version")?;
    let reason = require_non_empty(reason, "reason")?;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT revision, created_at_ms FROM security_policies
             WHERE scope_kind = ?1 AND scope_id = ?2",
            rusqlite::params![scope_kind.as_str(), scope_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let (cur_rev, created_at) = existing.unwrap_or((0, now_ms.as_i64()));
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let next_rev = cur_rev.saturating_add(1);
    let digest = compute_policy_digest(scope_kind, &scope_id, isolation_mode, &policy_version)?;

    tx.execute(
        "INSERT INTO security_policies
         (scope_kind, scope_id, isolation_mode, policy_version, policy_digest, reason,
          updated_by_actor, revision, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(scope_kind, scope_id) DO UPDATE SET
           isolation_mode = excluded.isolation_mode,
           policy_version = excluded.policy_version,
           policy_digest = excluded.policy_digest,
           reason = excluded.reason,
           updated_by_actor = excluded.updated_by_actor,
           revision = excluded.revision,
           updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            scope_kind.as_str(),
            scope_id,
            isolation_mode.as_str(),
            policy_version,
            digest.as_prefixed_str(),
            reason,
            caller.actor_id.as_str(),
            next_rev,
            created_at,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

pub(crate) fn read_policy_row(
    conn: &Connection,
    scope_kind: PolicyScopeKind,
    scope_id: &str,
) -> Result<Option<SecurityPolicyRecord>, StoreError> {
    let row: Option<(String, String, String, String, String, i64, i64, i64)> = conn
        .query_row(
            "SELECT isolation_mode, policy_version, policy_digest, reason, updated_by_actor,
                    revision, created_at_ms, updated_at_ms
             FROM security_policies WHERE scope_kind = ?1 AND scope_id = ?2",
            rusqlite::params![scope_kind.as_str(), scope_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((mode_s, ver, dig_s, reason, actor, rev, cr, up)) = row else {
        return Ok(None);
    };
    Ok(Some(SecurityPolicyRecord {
        scope_kind,
        scope_id: scope_id.to_owned(),
        isolation_mode: PolicyIsolationMode::parse(&mode_s)?,
        policy_version: ver,
        policy_digest: dig_s.parse::<Sha256Digest>()?,
        reason,
        updated_by_actor: actor,
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        created_at_ms: TimestampMs::new(cr),
        updated_at_ms: TimestampMs::new(up),
    }))
}

pub(crate) fn read_guard_config_conn(
    conn: &Connection,
    config_id: &str,
) -> Result<Option<GuardConfigRecord>, StoreError> {
    #[expect(
        clippy::type_complexity,
        reason = "Tuple SQLite temporaire projeté aussitôt vers GuardConfigRecord"
    )]
    let row: Option<(
        String,
        Option<String>,
        String,
        Option<String>,
        u32,
        u32,
        u32,
        Option<String>,
        String,
        String,
        Option<String>,
        String,
        Option<i64>,
        String,
        String,
        i64,
        i64,
    )> = conn
        .query_row(
            "SELECT guard_mode, endpoint, transport, secret_ref_id, evaluate_timeout_ms,
                    health_check_interval_s, decision_cache_max_age_s, fallback_policy_ref,
                    guard_mapping_profile, health_status, negotiated_version, capabilities_json,
                    last_health_check_ms, reason, updated_by_actor, revision, updated_at_ms
             FROM guard_configs WHERE config_id = ?1",
            [config_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                    r.get(13)?,
                    r.get(14)?,
                    r.get(15)?,
                    r.get(16)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((
        mode_s,
        endpoint,
        transport_s,
        secret_ref_id,
        evaluate_timeout_ms,
        health_check_interval_s,
        decision_cache_max_age_s,
        fallback_policy_ref,
        guard_mapping_profile,
        health_s,
        negotiated_version,
        caps_json,
        last_hc,
        reason,
        updated_by_actor,
        rev,
        updated_at_ms,
    )) = row
    else {
        return Ok(None);
    };
    let capabilities: Vec<String> = serde_json::from_str(&caps_json)?;
    Ok(Some(GuardConfigRecord {
        config_id: config_id.to_owned(),
        guard_mode: GuardPolicyMode::parse(&mode_s)?,
        endpoint,
        transport: GuardTransport::parse(&transport_s)?,
        secret_ref_id,
        evaluate_timeout_ms,
        health_check_interval_s,
        decision_cache_max_age_s,
        fallback_policy_ref,
        guard_mapping_profile,
        health_status: GuardHealthStatus::parse(&health_s)?,
        negotiated_version,
        capabilities,
        last_health_check_ms: last_hc.map(TimestampMs::new),
        reason,
        updated_by_actor,
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        updated_at_ms: TimestampMs::new(updated_at_ms),
    }))
}

pub(crate) fn read_policy_summary_conn(
    conn: &Connection,
    project_id: Option<ProjectId>,
) -> Result<PolicySummary, StoreError> {
    let global_policy = read_policy_row(conn, PolicyScopeKind::Global, "global")?;
    let project_policy = if let Some(pid) = project_id {
        read_policy_row(conn, PolicyScopeKind::Project, &pid.to_string())?
    } else {
        None
    };
    let effective_isolation_mode = match (
        global_policy.as_ref().map(|p| p.isolation_mode),
        project_policy.as_ref().map(|p| p.isolation_mode),
    ) {
        (Some(PolicyIsolationMode::Strict), _) | (_, Some(PolicyIsolationMode::Strict)) => {
            PolicyIsolationMode::Strict
        }
        _ => PolicyIsolationMode::Standard,
    };
    let guard_config = read_guard_config_conn(conn, "default")?;
    let susp_cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subject_suspensions WHERE status = 'suspended'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let rev_cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subject_suspensions WHERE status = 'revoked'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    Ok(PolicySummary {
        global_policy,
        project_policy,
        effective_isolation_mode,
        guard_config,
        active_suspensions_count: u64::try_from(susp_cnt).unwrap_or(0),
        active_revocations_count: u64::try_from(rev_cnt).unwrap_or(0),
        provider_probe_mandates: Vec::new(),
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "Validation Q-04 complète de la configuration AI-Guard et émission d'alerte éventuelle"
)]
pub(crate) fn tx_configure_guard(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    input: &GuardConfigInput,
    caller: &CallerAuthority,
) -> Result<Revision, StoreError> {
    require_security_admin(caller, "ConfigureGuard")?;
    let config_id = require_non_empty(&input.config_id, "config_id")?;
    let reason = require_non_empty(&input.reason, "reason")?;

    if !(100..=60_000).contains(&input.evaluate_timeout_ms) {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!(
                "evaluate_timeout_ms {} hors [100..=60000]",
                input.evaluate_timeout_ms
            ),
        });
    }
    if !(5..=3600).contains(&input.health_check_interval_s) || input.decision_cache_max_age_s > 3600
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "paramètres temporels AI-Guard hors bornes".to_owned(),
        });
    }

    let endpoint_clean = input
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let fallback_clean = input
        .fallback_policy_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    match input.guard_mode {
        GuardPolicyMode::BrokerOnly => {}
        GuardPolicyMode::GuardRequired => {
            if endpoint_clean.is_none() || input.transport == GuardTransport::None {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: "guard_required exige endpoint et transport explicites".to_owned(),
                });
            }
        }
        GuardPolicyMode::GuardOptional => {
            if endpoint_clean.is_none() || input.transport == GuardTransport::None {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: "guard_optional exige endpoint et transport explicites".to_owned(),
                });
            }
            if fallback_clean.is_none() {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: "guard_optional exige fallback_policy_ref non vide".to_owned(),
                });
            }
        }
    }

    if let Some(ref sref_id) = input.secret_ref_id {
        let sec_status: Option<String> = tx
            .query_row(
                "SELECT status FROM secret_references WHERE secret_ref_id = ?1",
                [sref_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        match sec_status.as_deref() {
            None => {
                return Err(StoreError::NotFound(format!(
                    "secret_reference '{sref_id}' introuvable"
                )));
            }
            Some("revoked_local" | "revoked_remote") => {
                return Err(StoreError::CapabilityDenied {
                    reason: ReasonCode::SecretAccessDenied,
                    message: format!("secret_reference '{sref_id}' est révoquée"),
                });
            }
            Some(_) => {}
        }
    }

    let cur_rev: i64 = tx
        .query_row(
            "SELECT revision FROM guard_configs WHERE config_id = ?1",
            [&config_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?
        .unwrap_or(0);
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    let next_rev = cur_rev.saturating_add(1);

    tx.execute(
        "INSERT INTO guard_configs
         (config_id, guard_mode, endpoint, transport, secret_ref_id, evaluate_timeout_ms,
          health_check_interval_s, decision_cache_max_age_s, fallback_policy_ref,
          guard_mapping_profile, reason, updated_by_actor, revision, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(config_id) DO UPDATE SET
           guard_mode = excluded.guard_mode,
           endpoint = excluded.endpoint,
           transport = excluded.transport,
           secret_ref_id = excluded.secret_ref_id,
           evaluate_timeout_ms = excluded.evaluate_timeout_ms,
           health_check_interval_s = excluded.health_check_interval_s,
           decision_cache_max_age_s = excluded.decision_cache_max_age_s,
           fallback_policy_ref = excluded.fallback_policy_ref,
           guard_mapping_profile = excluded.guard_mapping_profile,
           reason = excluded.reason,
           updated_by_actor = excluded.updated_by_actor,
           revision = excluded.revision,
           updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            config_id,
            input.guard_mode.as_str(),
            endpoint_clean,
            input.transport.as_str(),
            input.secret_ref_id,
            i64::from(input.evaluate_timeout_ms),
            i64::from(input.health_check_interval_s),
            i64::from(input.decision_cache_max_age_s),
            fallback_clean,
            input.guard_mapping_profile,
            reason,
            caller.actor_id.as_str(),
            next_rev,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Transition de statut sujet atomique avec contrôle révision et visibilité Guard"
)]
pub(crate) fn tx_transition_subject(
    tx: &Transaction<'_>,
    id_gen: &dyn IdGeneratorPort,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    subject_kind: SubjectKind,
    subject_id: &str,
    scope_kind: &str,
    scope_id: &str,
    target_status: SubjectStatus,
    reason: &str,
    guard_propagated: bool,
    caller: &CallerAuthority,
) -> Result<Revision, StoreError> {
    let op_name = match target_status {
        SubjectStatus::Suspended => "SuspendSubject",
        SubjectStatus::Active => "ResumeSubject",
        SubjectStatus::Revoked => "RevokeSubject",
    };
    require_security_admin(caller, op_name)?;
    let subject_id = require_non_empty(subject_id, "subject_id")?;
    let scope_kind = require_non_empty(scope_kind, "scope_kind")?;
    let scope_id = require_non_empty(scope_id, "scope_id")?;
    let reason = require_non_empty(reason, "reason")?;

    let existing: Option<(String, String, i64, i64)> = tx
        .query_row(
            "SELECT suspension_id, status, revision, created_at_ms
             FROM subject_suspensions
             WHERE subject_kind = ?1 AND subject_id = ?2 AND scope_kind = ?3 AND scope_id = ?4",
            rusqlite::params![subject_kind.as_str(), subject_id, scope_kind, scope_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let (suspension_id, cur_status, cur_rev, created_at) = match existing {
        Some((sid, st_s, rev, cr)) => (sid, SubjectStatus::parse(&st_s)?, rev, cr),
        None => (
            id_gen.new_uuid().to_string(),
            SubjectStatus::Active,
            0_i64,
            now_ms.as_i64(),
        ),
    };

    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    match (cur_status, target_status) {
        (SubjectStatus::Active, SubjectStatus::Suspended | SubjectStatus::Revoked)
        | (SubjectStatus::Suspended, SubjectStatus::Active | SubjectStatus::Revoked) => {}
        (SubjectStatus::Revoked, SubjectStatus::Active) => {
            return Err(StoreError::ContractViolation {
                reason: Some(ReasonCode::SubjectRevoked),
                message: format!(
                    "le sujet '{}:{subject_id}' est définitivement révoqué et ne peut pas être réactivé par ResumeSubject",
                    subject_kind.as_str()
                ),
            });
        }
        (from, to) => {
            return Err(StoreError::ContractViolation {
                reason: None,
                message: format!(
                    "transition sujet invalide de '{}' vers '{}'",
                    from.as_str(),
                    to.as_str()
                ),
            });
        }
    }

    let prop_state = if guard_propagated {
        GuardPropagationState::Propagated
    } else {
        let guard_mode_opt: Option<String> = tx
            .query_row(
                "SELECT guard_mode FROM guard_configs WHERE config_id = 'default'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        match guard_mode_opt.as_deref() {
            Some("guard_optional" | "guard_required") => GuardPropagationState::UnnegotiatedWarning,
            _ => GuardPropagationState::LocalOnly,
        }
    };

    let next_rev = cur_rev.saturating_add(1);
    tx.execute(
        "INSERT INTO subject_suspensions
         (suspension_id, subject_kind, subject_id, scope_kind, scope_id, status, reason,
          guard_propagation_state, acted_by_actor, revision, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(subject_kind, subject_id, scope_kind, scope_id) DO UPDATE SET
           status = excluded.status,
           reason = excluded.reason,
           guard_propagation_state = excluded.guard_propagation_state,
           acted_by_actor = excluded.acted_by_actor,
           revision = excluded.revision,
           updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            suspension_id,
            subject_kind.as_str(),
            subject_id,
            scope_kind,
            scope_id,
            target_status.as_str(),
            reason,
            prop_state.as_str(),
            caller.actor_id.as_str(),
            next_rev,
            created_at,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

pub(crate) fn read_subject_suspension_conn(
    conn: &Connection,
    kind: SubjectKind,
    subject_id: &str,
    scope_kind: &str,
    scope_id: &str,
) -> Result<Option<SubjectSuspensionRecord>, StoreError> {
    let row: Option<(String, String, String, String, String, i64, i64, i64)> = conn
        .query_row(
            "SELECT suspension_id, status, reason, guard_propagation_state, acted_by_actor,
                    revision, created_at_ms, updated_at_ms
             FROM subject_suspensions
             WHERE subject_kind = ?1 AND subject_id = ?2 AND scope_kind = ?3 AND scope_id = ?4",
            rusqlite::params![kind.as_str(), subject_id, scope_kind, scope_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((suspension_id, status_s, reason, prop_s, acted_by_actor, rev, cr, up)) = row else {
        return Ok(None);
    };
    Ok(Some(SubjectSuspensionRecord {
        suspension_id,
        subject_kind: kind,
        subject_id: subject_id.to_owned(),
        scope_kind: scope_kind.to_owned(),
        scope_id: scope_id.to_owned(),
        status: SubjectStatus::parse(&status_s)?,
        reason,
        guard_propagation_state: GuardPropagationState::parse(&prop_s)?,
        acted_by_actor,
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        created_at_ms: TimestampMs::new(cr),
        updated_at_ms: TimestampMs::new(up),
    }))
}

pub(crate) fn ensure_subject_not_blocked_tx(
    tx: &Transaction<'_>,
    kind: SubjectKind,
    subject_id: &str,
    project_id: Option<ProjectId>,
) -> Result<(), StoreError> {
    let proj_str = project_id.map_or_else(String::new, |p| p.to_string());
    let blocked: Option<(String, String)> = tx
        .query_row(
            "SELECT status, reason FROM subject_suspensions
             WHERE subject_kind = ?1 AND subject_id = ?2
               AND status IN ('suspended', 'revoked')
               AND (scope_kind = 'global' OR (scope_kind = 'project' AND scope_id = ?3))
             ORDER BY CASE status WHEN 'revoked' THEN 0 ELSE 1 END
             LIMIT 1",
            rusqlite::params![kind.as_str(), subject_id, proj_str],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some((status_s, reason)) = blocked {
        let rc = if status_s == "revoked" {
            ReasonCode::SubjectRevoked
        } else {
            ReasonCode::SubjectSuspended
        };
        return Err(StoreError::CapabilityDenied {
            reason: rc,
            message: format!(
                "sujet '{}:{subject_id}' bloqué ({status_s}) : {reason}",
                kind.as_str()
            ),
        });
    }
    Ok(())
}

fn validate_opaque_secret_ref(secret_ref: &SecretRef) -> Result<(), StoreError> {
    let raw = secret_ref.as_opaque_str();
    let trimmed = raw.trim();
    if trimmed != raw || trimmed.contains(char::is_whitespace) {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "SecretRef opaque invalide (espaces interdits)".to_owned(),
        });
    }
    // Défense en profondeur : refuser les jetons bruts manifestement non encapsulés en référence coffre.
    if trimmed.starts_with("sk-")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("xoxb-")
        || trimmed.starts_with("AIza")
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message:
                "aucun octet ou jeton secret brut ne peut être enregistré dans secret_references"
                    .to_owned(),
        });
    }
    let _ = SecretRef::new(trimmed.to_owned())?;
    Ok(())
}

pub(crate) fn tx_set_secret_reference(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    input: &SecretReferenceInput,
    caller: &CallerAuthority,
) -> Result<Revision, StoreError> {
    require_security_admin(caller, "SetSecretReference")?;
    let secret_ref_id = require_non_empty(&input.secret_ref_id, "secret_ref_id")?;
    let namespace = require_non_empty(&input.namespace, "namespace")?;
    let purpose = require_non_empty(&input.purpose, "purpose")?;
    let secret_kind = require_non_empty(&input.secret_kind, "secret_kind")?;
    let target_kind = require_non_empty(&input.target_kind, "target_kind")?;
    let target_id = require_non_empty(&input.target_id, "target_id")?;
    let scope_kind = require_non_empty(&input.scope_kind, "scope_kind")?;
    let scope_id = require_non_empty(&input.scope_id, "scope_id")?;
    let backend_kind = require_non_empty(&input.backend_kind, "backend_kind")?;
    validate_opaque_secret_ref(&input.secret_ref)?;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT revision, created_at_ms FROM secret_references WHERE secret_ref_id = ?1",
            [&secret_ref_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let (cur_rev, created_at) = existing.unwrap_or((0, now_ms.as_i64()));
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    let next_rev = cur_rev.saturating_add(1);

    tx.execute(
        "INSERT INTO secret_references
         (secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind, target_kind,
          target_id, scope_kind, scope_id, backend_kind, status, local_revoked_at_ms,
          remote_revocation_state, revision, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', NULL, 'not_applicable', ?11, ?12, ?13)
         ON CONFLICT(secret_ref_id) DO UPDATE SET
           opaque_secret_ref = excluded.opaque_secret_ref,
           namespace = excluded.namespace,
           purpose = excluded.purpose,
           secret_kind = excluded.secret_kind,
           target_kind = excluded.target_kind,
           target_id = excluded.target_id,
           scope_kind = excluded.scope_kind,
           scope_id = excluded.scope_id,
           backend_kind = excluded.backend_kind,
           status = 'active',
           local_revoked_at_ms = NULL,
           remote_revocation_state = 'not_applicable',
           revision = excluded.revision,
           updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            secret_ref_id,
            input.secret_ref.as_opaque_str(),
            namespace,
            purpose,
            secret_kind,
            target_kind,
            target_id,
            scope_kind,
            scope_id,
            backend_kind,
            next_rev,
            created_at,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Révocation atomique locale distincte de la révocation distante éventuelle"
)]
pub(crate) fn tx_revoke_secret_reference(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    secret_ref_id: &str,
    reason: &str,
    caller: &CallerAuthority,
    remote_revocation_supported: bool,
    remote_revocation_confirmed: bool,
) -> Result<Revision, StoreError> {
    require_security_admin(caller, "RevokeSecretReference")?;
    let _ = require_non_empty(reason, "reason")?;
    let cur_rev: Option<i64> = tx
        .query_row(
            "SELECT revision FROM secret_references WHERE secret_ref_id = ?1",
            [secret_ref_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some(rev) = cur_rev else {
        return Err(StoreError::NotFound(format!(
            "secret_reference '{secret_ref_id}' introuvable"
        )));
    };
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let (status, remote_state) = if remote_revocation_supported && remote_revocation_confirmed {
        (
            SecretRefState::RevokedRemote,
            RemoteRevocationState::Confirmed,
        )
    } else if remote_revocation_supported {
        (SecretRefState::RevokedLocal, RemoteRevocationState::Pending)
    } else {
        (
            SecretRefState::RevokedLocal,
            RemoteRevocationState::Unsupported,
        )
    };

    let next_rev = rev.saturating_add(1);
    tx.execute(
        "UPDATE secret_references
         SET status = ?1,
             local_revoked_at_ms = COALESCE(local_revoked_at_ms, ?2),
             remote_revocation_state = ?3,
             revision = ?4,
             updated_at_ms = ?2
         WHERE secret_ref_id = ?5",
        rusqlite::params![
            status.as_str(),
            now_ms.as_i64(),
            remote_state.as_str(),
            next_rev,
            secret_ref_id
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

struct SecretRefRow {
    secret_ref_id: String,
    opaque_raw: String,
    namespace: String,
    purpose: String,
    secret_kind: String,
    target_kind: String,
    target_id: String,
    scope_kind: String,
    scope_id: String,
    backend_kind: String,
    status_s: String,
    local_revoked_at_ms: Option<i64>,
    remote_s: String,
    rev: i64,
    cr: i64,
    up: i64,
}

fn map_secret_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SecretRefRow> {
    Ok(SecretRefRow {
        secret_ref_id: row.get(0)?,
        opaque_raw: row.get(1)?,
        namespace: row.get(2)?,
        purpose: row.get(3)?,
        secret_kind: row.get(4)?,
        target_kind: row.get(5)?,
        target_id: row.get(6)?,
        scope_kind: row.get(7)?,
        scope_id: row.get(8)?,
        backend_kind: row.get(9)?,
        status_s: row.get(10)?,
        local_revoked_at_ms: row.get(11)?,
        remote_s: row.get(12)?,
        rev: row.get(13)?,
        cr: row.get(14)?,
        up: row.get(15)?,
    })
}

fn build_secret_status(row: SecretRefRow) -> Result<SecretReferenceStatus, StoreError> {
    let SecretRefRow {
        secret_ref_id,
        opaque_raw,
        namespace,
        purpose,
        secret_kind,
        target_kind,
        target_id,
        scope_kind,
        scope_id,
        backend_kind,
        status_s,
        local_revoked_at_ms,
        remote_s,
        rev,
        cr,
        up,
    } = row;
    Ok(SecretReferenceStatus {
        secret_ref_id,
        opaque_secret_ref: SecretRef::new(opaque_raw)?,
        namespace,
        purpose,
        secret_kind,
        target_kind,
        target_id,
        scope_kind,
        scope_id,
        backend_kind,
        status: SecretRefState::parse(&status_s)?,
        local_revoked_at_ms: local_revoked_at_ms.map(TimestampMs::new),
        remote_revocation_state: RemoteRevocationState::parse(&remote_s)?,
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        created_at_ms: TimestampMs::new(cr),
        updated_at_ms: TimestampMs::new(up),
    })
}

pub(crate) fn get_secret_reference_status_conn(
    conn: &Connection,
    secret_ref_id: &str,
) -> Result<SecretReferenceStatus, StoreError> {
    let row = conn
        .query_row(
            "SELECT secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind,
                    target_kind, target_id, scope_kind, scope_id, backend_kind, status,
                    local_revoked_at_ms, remote_revocation_state, revision, created_at_ms, updated_at_ms
             FROM secret_references WHERE secret_ref_id = ?1",
            [secret_ref_id],
            map_secret_row,
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some(t) = row else {
        return Err(StoreError::NotFound(format!(
            "secret_reference '{secret_ref_id}' introuvable"
        )));
    };
    build_secret_status(t)
}

pub(crate) fn list_secret_references_conn(
    conn: &Connection,
    namespace: Option<&str>,
) -> Result<Vec<SecretReferenceStatus>, StoreError> {
    let mut out = Vec::new();
    if let Some(ns) = namespace {
        let mut stmt = conn
            .prepare(
                "SELECT secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind,
                        target_kind, target_id, scope_kind, scope_id, backend_kind, status,
                        local_revoked_at_ms, remote_revocation_state, revision, created_at_ms, updated_at_ms
                 FROM secret_references WHERE namespace = ?1 ORDER BY secret_ref_id ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([ns], map_secret_row)
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            out.push(build_secret_status(r.map_err(StoreError::from_rusqlite)?)?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind,
                        target_kind, target_id, scope_kind, scope_id, backend_kind, status,
                        local_revoked_at_ms, remote_revocation_state, revision, created_at_ms, updated_at_ms
                 FROM secret_references ORDER BY secret_ref_id ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], map_secret_row)
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            out.push(build_secret_status(r.map_err(StoreError::from_rusqlite)?)?);
        }
    }
    Ok(out)
}

pub(crate) fn record_alert_tx(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    spec: &NewAlertSpec,
) -> Result<AlertRecord, StoreError> {
    let alert_id = require_non_empty(&spec.alert_id, "alert_id")?;
    let kind = require_non_empty(&spec.kind, "kind")?;
    let title = require_non_empty(&spec.title, "title")?;
    let summary = require_non_empty(&spec.summary, "summary")?;
    let ops_json = serde_json::to_string(&spec.allowed_operations)?;
    let blocking_i64 = i64::from(spec.blocking);

    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT revision, created_at_ms FROM alerts WHERE alert_id = ?1",
            [&alert_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let (cur_rev, created_at) = existing.unwrap_or((0, now_ms.as_i64()));
    let next_rev = cur_rev.saturating_add(1);

    tx.execute(
        "INSERT INTO alerts
         (alert_id, kind, severity, state, blocking, project_id, subject_kind, subject_id,
          title, summary, reason_code, evidence_ref, allowed_operations_json, revision,
          created_at_ms, updated_at_ms, resolved_at_ms)
         VALUES (?1, ?2, ?3, 'active', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL)
         ON CONFLICT(alert_id) DO UPDATE SET
           kind = excluded.kind,
           severity = excluded.severity,
           state = 'active',
           blocking = excluded.blocking,
           project_id = excluded.project_id,
           subject_kind = excluded.subject_kind,
           subject_id = excluded.subject_id,
           title = excluded.title,
           summary = excluded.summary,
           reason_code = excluded.reason_code,
           evidence_ref = excluded.evidence_ref,
           allowed_operations_json = excluded.allowed_operations_json,
           revision = excluded.revision,
           updated_at_ms = excluded.updated_at_ms,
           resolved_at_ms = NULL",
        rusqlite::params![
            alert_id,
            kind,
            spec.severity.as_str(),
            blocking_i64,
            spec.project_id.map(|p| p.to_string()),
            spec.subject_kind,
            spec.subject_id,
            title,
            summary,
            spec.reason_code,
            spec.evidence_ref,
            ops_json,
            next_rev,
            created_at,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(AlertRecord {
        alert_id,
        kind,
        severity: spec.severity,
        state: AlertState::Active,
        blocking: spec.blocking,
        project_id: spec.project_id,
        subject_kind: spec.subject_kind.clone(),
        subject_id: spec.subject_id.clone(),
        title,
        summary,
        reason_code: spec.reason_code.clone(),
        evidence_ref: spec.evidence_ref.clone(),
        allowed_operations: spec.allowed_operations.clone(),
        revision: Revision::new(u64::try_from(next_rev).unwrap_or(1)),
        created_at_ms: TimestampMs::new(created_at),
        updated_at_ms: now_ms,
        resolved_at_ms: None,
    })
}

pub(crate) fn tx_transition_alert(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: unleash_domain::CommandId,
    alert_id: &str,
    target_state: AlertState,
) -> Result<Revision, StoreError> {
    let row: Option<i64> = tx
        .query_row(
            "SELECT revision FROM alerts WHERE alert_id = ?1",
            [alert_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some(cur_rev) = row else {
        return Err(StoreError::NotFound(format!(
            "alerte '{alert_id}' introuvable"
        )));
    };
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if cur_rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(cur_rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    let next_rev = cur_rev.saturating_add(1);
    let resolved_at = (target_state == AlertState::Resolved).then_some(now_ms.as_i64());
    tx.execute(
        "UPDATE alerts
         SET state = ?1, revision = ?2, updated_at_ms = ?3,
             resolved_at_ms = COALESCE(?4, resolved_at_ms)
         WHERE alert_id = ?5",
        rusqlite::params![
            target_state.as_str(),
            next_rev,
            now_ms.as_i64(),
            resolved_at,
            alert_id
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

pub(crate) fn list_alerts_conn(
    conn: &Connection,
    filter: &ListAlertsFilter,
) -> Result<Vec<AlertRecord>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT alert_id, kind, severity, state, blocking, project_id, subject_kind,
                    subject_id, title, summary, reason_code, evidence_ref,
                    allowed_operations_json, revision, created_at_ms, updated_at_ms, resolved_at_ms
             FROM alerts
             ORDER BY blocking DESC, created_at_ms DESC, alert_id ASC",
        )
        .map_err(StoreError::from_rusqlite)?;

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, String>(12)?,
                r.get::<_, i64>(13)?,
                r.get::<_, i64>(14)?,
                r.get::<_, i64>(15)?,
                r.get::<_, Option<i64>>(16)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut out = Vec::new();
    for item in rows {
        let (
            alert_id,
            kind,
            sev_s,
            st_s,
            blk_i,
            proj_s,
            subject_kind,
            subject_id,
            title,
            summary,
            reason_code,
            evidence_ref,
            ops_json,
            rev,
            cr,
            up,
            res_ms,
        ) = item.map_err(StoreError::from_rusqlite)?;

        let severity = AlertSeverity::parse(&sev_s)?;
        let state = AlertState::parse(&st_s)?;
        let blocking = blk_i != 0;
        let project_id = proj_s.as_deref().map(str::parse::<ProjectId>).transpose()?;

        let is_unfiltered_blocking =
            filter.include_blocking_regardless_of_filter && blocking && state == AlertState::Active;
        if !is_unfiltered_blocking {
            if let Some(pid) = filter.project_id
                && project_id != Some(pid)
            {
                continue;
            }
            if let Some(st) = filter.state
                && state != st
            {
                continue;
            }
            if let Some(sev) = filter.severity
                && severity != sev
            {
                continue;
            }
        }

        let allowed_operations: Vec<String> = serde_json::from_str(&ops_json)?;
        out.push(AlertRecord {
            alert_id,
            kind,
            severity,
            state,
            blocking,
            project_id,
            subject_kind,
            subject_id,
            title,
            summary,
            reason_code,
            evidence_ref,
            allowed_operations,
            revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            created_at_ms: TimestampMs::new(cr),
            updated_at_ms: TimestampMs::new(up),
            resolved_at_ms: res_ms.map(TimestampMs::new),
        });
    }
    Ok(out)
}
