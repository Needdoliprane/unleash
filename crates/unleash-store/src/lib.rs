//! # unleash-store
//!
//! Responsabilité exclusive : Persistance locale `SQLite` en mode WAL qualifié,
//! connexion writer unique sur thread dédié et file bornée, migrations versionnées,
//! journal transactionnel d'événements, outbox/inbox, objets sous pins, sauvegarde et restauration.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod admission;
pub mod backup;
pub mod backup_requests;
pub mod broker_phases;
mod broker_writer;
pub mod budgets;
pub mod commands;
pub mod config;
pub mod error;
pub mod events;
pub mod fs_sync;
pub mod leases;
pub mod migrations;
pub mod objects;
pub mod policies_secrets;
pub mod projects_settings;
pub mod provider_probe_connect;
mod provider_probe_mandates;
pub mod provider_probes;
mod provider_profile_validation;
pub mod provider_profiles;
pub mod qualification;
pub mod recovery_ops;
pub mod restore;
pub mod store;
pub mod terminal_allocations;
pub mod writer;

pub use admission::{
    ActiveAdmission, AdmissionCandidate, AdmissionCommit, AdmissionDecision,
    AdmissionEligibilityObservation, AdmissionOutcome, AdmissionSnapshotToken, AdmissionUnit,
    AdmissionUrgency, ExecutionAdmissionSnapshot, ExecutionCheckpointObservation,
    ExecutionCheckpointRequestRecord, ExecutionContinuation,
};
pub use backup::{
    BackupJobState, BackupManifest, BackupObjectEntry, BackupRetentionTier, BoundedExportOutcome,
    BoundedExportRequest, ExportCancellation, PrepareBackupRequest, ValidateBackupReport,
};
pub use backup_requests::{
    BackupBlockReason, BackupRequestRecord, BackupRequestState, ExecuteReservedBackupRequest,
    ExecutedBackup, REQUEST_BACKUP_VERB,
};
pub use budgets::{
    BudgetAccountCharge, BudgetAccountSnapshot, BudgetReservationRecord, BudgetReservationRequest,
    BudgetScopeType, CapacityPoolState, CapacityReservationRequest, ControlReserveMode,
    ExecutionOwner, ExecutionPoolParams, PricingCheckStatus,
};
pub use commands::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, DaemonStateRow,
    EventAppendSpec, ReceiptInspectionOutcome, ReceiptPurgeReport,
};
pub use config::{
    CutpointObserver, DEFAULT_EXPORT_CHUNK_BYTES, DEFAULT_EXPORT_MAX_BYTES,
    DEFAULT_MAX_TX_WAL_GROWTH_BYTES, DEFAULT_MIN_FREE_SPACE_BYTES, DEFAULT_READ_TIMEOUT,
    DEFAULT_RECEIPT_RETENTION_DAYS, DEFAULT_WAL_IDLE_CHECKPOINT_INTERVAL,
    DEFAULT_WAL_PRESSURE_BYTES, DEFAULT_WAL_STOP_BYTES, DEFAULT_WRITER_QUEUE_CAPACITY,
    DfDiskSpaceProbe, DiskSpaceProbe, EVENT_AND_INBOX_MIN_RETENTION_MS, FaultTarget,
    FixedDiskSpaceProbe, InjectedStorageFault, MonotonicClockPort, NoopCutpointObserver,
    NoopFaultInjector, SequentialUuidGenerator, StorageFaultInjector, StoreConfig, StoreCutpoint,
    SystemMonotonicClock, SystemUtcClock, WriteCriticality,
};
pub use error::StoreError;
pub use events::{
    AffectedBackupEntry, DeadLetterItem, InboxApplyOutcome, MAX_OUTBOX_AGGREGATE_PARALLELISM,
    MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS, OutboxDispatchItem, OutboxState, PurgeStats,
    ReadEventsOutcome, RetentionPurgeReport, RetentionPurgeRequest, RetentionPurgeStatusRecord,
};
pub use leases::{AcquireLeaseRequest, FenceAssertion, LeaseState, ResourceLeaseRecord};
pub use migrations::{CURRENT_SCHEMA_VERSION, MigrationRecord, MigrationReport, MigrationState};
pub use objects::{
    DeletionIntent, ObjectReconciliationReport, ObjectRecordView, ObjectState, PinKind, PinState,
    PrepareObjectDeletionRequest, PrepareObjectRequest, PreparedObjectHandle,
    PublishObjectReferenceRequest,
};
pub use policies_secrets::{
    AlertRecord, AlertSeverity, AlertState, CallerAuthority, CallerRole, GuardConfigInput,
    GuardConfigRecord, GuardHealthStatus, GuardPolicyMode, GuardPropagationState, GuardTransport,
    ListAlertsFilter, NewAlertSpec, PolicyIsolationMode, PolicyScopeKind, PolicySummary,
    RemoteRevocationState, SecretRefState, SecretReferenceInput, SecretReferenceStatus,
    SecurityPolicyRecord, SubjectKind, SubjectStatus, SubjectSuspensionRecord,
};
pub use projects_settings::{
    AnalyzerPathConfig, ExecutionPoolSettings, ProjectRecord, ProjectRepositorySpec,
    ProjectSourceSpec, RETENTION_SHORTENING_NOTICE_MS, RetentionClass, RetentionPolicySettings,
    RetentionShorteningNotice, SettingsPatch, SettingsRecord, SettingsScopeKind,
};
pub use provider_probe_connect::{
    ClaimProviderProbeConnectRequest, ProviderProbeConnectClaim, ProviderProbeConnectClaimOutcome,
    ProviderProbeConnectClaimRecord, ProviderProbeConnectContext, ProviderProbeConnectPreparation,
    ProviderProbeConnectRecord,
};
pub use provider_probe_mandates::{
    ProviderProbeMandateChange, ProviderProbeMandateIssue, ProviderProbeMandateStatus,
    ProviderProbeMandateView,
};
pub use provider_probes::{
    ModelContextRef, ModelOrigin, PROVIDER_PROBE_VERB, PreparedProviderProbe,
    ProbeGenerationLimits, ProbeProfileSnapshot, ProviderProbeContext, ProviderProbePreparation,
};
pub use provider_profile_validation::{
    LocalProviderValidation, LocalValidationStatus, ProviderProfileView,
    ProviderValidationPreparation, ProviderValidationSummary,
};
pub use provider_profiles::{
    ConfiguredProviderProfile, ProviderCatalogBinding, ProviderCatalogSource,
    ProviderProfileConfiguration, ProviderProfilePreparation, ProviderProfileQualification,
    ProviderSecretBinding,
};
pub use qualification::{
    APPROVED_BACKPORT_VERSIONS, EffectivePragmaProfile, MIN_QUALIFIED_SQLITE_VERSION,
    SqliteBuildPolicy, SqliteRuntimeManifest, WITHDRAWN_SQLITE_VERSIONS, compiled_sqlite_source_id,
    compiled_sqlite_version, validate_build_metadata,
};
pub use recovery_ops::{
    ActionDispatchInput, ActionOutcomeDisposition, ApprovalDecision, ApprovalResolutionRecord,
    RecoveryActionProvenance, RecoveryItem, RecoveryReport, RecoveryScopeFreezeRecord,
    RestoreImportRecord, RestoreRequestRecord, ScopeFreezeDisposition,
};
pub use restore::{
    CapsuleActionDelta, CapsuleBudgetDelta, CapsuleCoverage, CapsuleDispatchDelta,
    CapsuleReceiptDelta, CapsuleReceiptResponseState, CapsuleReceiptRetention,
    CapsuleTombstoneDelta, CapsuleTombstoneKind, EmptyExecutorNeutralizer, ExecutorInventoryItem,
    ExecutorNeutralizer, MaintenanceManifestEntry, MaintenanceRestoreManifest,
    RestoreActivationResult, RestoreBackupRequest, RestoreCapsule, RestoreRequestState,
};
pub use store::{SnapshotProjection, StoreOpenFailure, StoreOpenStage, UnleashStore};
pub use terminal_allocations::{
    NormalizedTerminalProcessSpec, ResolveTerminalLaunchRequest, ResolvedTerminalLaunch,
    TERMINAL_ALLOCATION_VERB, TerminalAllocationContext, TerminalAllocationPreparation,
    TerminalAllocationRecord, TerminalAllocationScope, TerminalAllocationState,
    TerminalAuxiliaryPools, TerminalEnvironmentRef,
};
pub use unleash_domain::ReconcileOutcome;
pub use writer::{HeldReaderSession, ReaderContext, ReaderControl, WalTelemetry};

pub use broker_phases::{
    BROKER_PHASE_SCHEMA_VERSION, BrokerActionReceipt, BrokerActionSnapshot,
    BrokerAuthorizationCause, BrokerAuthorizationDecision, BrokerAuthorizationFailure,
    BrokerAuthorizationOutcome, BrokerCause, BrokerEvaluation, BrokerExecutor, BrokerIntent,
    BrokerObservationCause, BrokerPhase, BrokerPhaseCommit, BrokerPhaseRequest, BrokerPhaseStore,
};
