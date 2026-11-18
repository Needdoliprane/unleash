// SPDX-License-Identifier: Apache-2.0
//! # unleash-action-broker
//!
//! Responsabilité exclusive : Médiateur déterministe unique d'autorisation et de lancement d'effets
//! d'OVPT Unleash (`F017`, `ADR-10`, `REQ-SEC-001`..`REQ-SEC-011`).
//!
//! ## Responsabilités
//! - Calcul d'autorité effective ([`EffectiveAuthority`]) par intersection stricte sans auto-élévation.
//! - Validation de la liste fermée et versionnée des 11 classes d'effets ([`EffectsClass`]).
//! - Contrôle des scopes filesystem (anti-traversée, symlinks, `.claude/`, TOCTOU) et des
//!   exigences d'isolation ([`IsolationMode`], fail-closed en mode `strict`).
//! - Port de secrets [`SecretStore`] avec [`SecretRef`] sans fuite en `Debug`/logs/erreurs.
//! - Contrat interne versionné [`GuardPort`], profil initial `obligations=none_accepted` (`Q-04`),
//!   modes [`GuardMode`] (`BrokerOnly`, `GuardOptional`, `GuardRequired`), cache borné `<= 300 s`
//!   et invalidation instantanée sur suspension, révocation ou changement de politique.
//! - Gouvernance des sujets (`SuspendSubject`, `ResumeSubject`, `RevokeSubject`), approbations
//!   (`ResolveActionApproval`), rédaction de canaris et planification de rétention `unleash.retention.default.v1`.
//!
//! ## Limites du raccordement
//! Le daemon fournit un [`BrokerJournalPort`] durable à [`ActionBroker::new`]. Le constructeur
//! [`ActionBroker::new_for_test_memory`] et les doubles ne qualifient ni `SQLite`, ni IPC, ni le
//! confinement réel des processus. [`HostIsolationProbe`] refuse le mode strict sans adaptateur
//! qualifié. Les profils [`GuardMappingProfile::Qualified`] servent aux essais du contrat ;
//! leur activation réelle exige la qualification `EXT-GUARD` du mapping et de l'exécution.
//! Les helpers de rétention manipulent un modèle en mémoire ; la purge physique, WAL, index,
//! exports et sauvegardes relève du store et de ses consommateurs.
//!
//! ## Dépendances admises
//! - `unleash-domain` (types, identités, fences, transitions pures d'action).
//! - `unleash-protocol` (erreurs IPC, canonisation RFC 8785 `canonical_sha256`, `ClientKind`).
//! - `unleash-platform` (port coffre OS [`unleash_platform::KeyringPort`], garde `.claude/`).
//! - Bibliothèques runtime/sérialisation (`tokio`, `serde`, `serde_json`, `thiserror`, `secrecy`, `zeroize`, `tracing`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod authority;
pub mod broker;
pub mod cache;
pub mod effects;
pub mod error;
pub mod guard_port;
pub mod isolation;
pub mod journal;
pub mod retention;
pub mod secrets;

pub use authority::{
    AuthorityLayer, EffectiveAuthority, LimitingAuthorityTrace, ScopeKind,
    compute_effective_authority, validate_authority_mutation_provenance,
    validate_no_permission_expansion, validate_path_within_roots, verify_path_toctou,
};
pub use broker::{
    ActionBroker, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort, ActionReceipt,
    ApprovalDecision, AuthorizationDecision, AuthorizationOutcome, BrokerAdminCapability,
    BrokerAuditEvent, BrokerAuditRecord, BrokerConfig, CallerIdentity, ConfigureGuardCommand,
    FallbackPolicyPort, GuardHealthSnapshot, LocalAuthorityFallbackPolicy, PolicySummary,
    RemotePropagationStatus, ResolveActionApprovalCommand, ResumeSubjectCommand,
    RevokeSubjectCommand, SetPolicyCommand, SubjectRevocationReceipt, SubjectSuspensionReceipt,
    SuspendSubjectCommand,
};
pub use cache::{CachedDecision, DecisionCache, DecisionCacheKey, MAX_DECISION_CACHE_TTL};
pub use effects::{
    ActionIntent, ActionProvenance, ActionSubject, EffectsClass, ExternalSourceKind,
    NormalizedResourceRef, RoleVersionRef, SkillVersionRef, SubjectKind, TrustLevel,
    TypedSubjectRef, UnvalidatedActionIntent,
};
pub use error::{ActionBrokerError, GuardPortError, SecretStoreError};
pub use guard_port::{
    BoxFuture, DEFAULT_EVALUATE_TIMEOUT_MS, DEFAULT_HEALTH_CHECK_INTERVAL_MS, ExecutorKind,
    GUARD_PORT_PROTOCOL_VERSION, GuardAuditReference, GuardCapabilities, GuardConstraint,
    GuardDecisionRef, GuardEvaluateRequest, GuardEvaluationResponse, GuardExecuteRequest,
    GuardExecutionOutcome, GuardHealthReport, GuardHealthStatus, GuardMappingProfile, GuardMode,
    GuardPort, GuardRemotePolicySummary, GuardRevocationReceipt, GuardSubjectActionRequest,
    GuardSubjectSnapshot, GuardSuspensionReceipt, InternalObligation, MonotonicClockPort,
    NONE_ACCEPTED_PROFILE_ID, NegotiateCapabilitiesRequest, ObligationCapabilityPort, SleepPort,
    StrictNoneAcceptedObligationChecker, SystemClockAndTimer, map_and_validate_obligations,
    revalidate_obligations,
};
pub use isolation::{
    HostIsolationProbe, IsolationBackendStatus, IsolationCapability, IsolationMode,
    IsolationProbePort, IsolationRequirements, verify_isolation_requirements,
};
pub use journal::{BrokerJournalEntry, BrokerJournalError, BrokerJournalPort};
pub use retention::{
    ManagedBackupRef, PurgeLimitDisclosure, PurgeReport, PurgeTombstone, REDACTED_MARKER,
    RedactionOutcome, RetentionClass, RetentionItem, RetentionItemState, RetentionPolicy,
    plan_and_execute_purge, redact_canaries,
};
pub use secrets::{
    AuthorizedSecretAccess, KeyringStatus, PlatformKeyringSecretStore, ResolvedSecret, SecretStore,
};
pub use unleash_domain::{ActionState, SecretRef};
