//! # unleash-store
//!
//! Responsabilité exclusive : Persistance locale `SQLite` en mode WAL qualifié,
//! connexion writer unique sur thread dédié et file bornée, migrations versionnées,
//! journal transactionnel d'événements, outbox/inbox, objets sous pins, sauvegarde et restauration.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backup;
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
pub mod qualification;
pub mod recovery_ops;
pub mod restore;
pub mod store;
pub mod writer;

pub use backup::{
    BackupJobState, BackupManifest, BackupObjectEntry, BackupRetentionTier, BoundedExportOutcome,
    BoundedExportRequest, ExportCancellation, PrepareBackupRequest, ValidateBackupReport,
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
    CapsuleReceiptDelta, CapsuleTombstoneDelta, CapsuleTombstoneKind, EmptyExecutorNeutralizer,
    ExecutorInventoryItem, ExecutorNeutralizer, MaintenanceManifestEntry,
    MaintenanceRestoreManifest, RestoreActivationResult, RestoreBackupRequest, RestoreCapsule,
    RestoreRequestState,
};
pub use store::{SnapshotProjection, UnleashStore};
pub use unleash_domain::ReconcileOutcome;
pub use writer::{HeldReaderSession, ReaderContext, ReaderControl, WalTelemetry};
