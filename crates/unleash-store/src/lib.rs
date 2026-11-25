//! # unleash-store
//!
//! Responsabilité exclusive : Persistance locale `SQLite` en mode WAL qualifié,
//! connexion writer unique sur thread dédié et file bornée, migrations versionnées,
//! journal transactionnel d'événements, outbox/inbox, objets sous pins, sauvegarde et restauration.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod admission;
mod agent_creation;
mod agent_from_task;
mod agent_projection;
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
mod initial_context;
mod invocation_projection;
mod invocation_queue;
pub mod leases;
pub mod migrations;
mod mission_scope;
mod model_selection_bindings;
pub mod objects;
pub mod policies_secrets;
pub mod projects_settings;
pub mod provider_probe_connect;
mod provider_probe_connect_disposition;
mod provider_probe_connect_observation;
mod provider_probe_generation;
pub mod provider_probe_inventory;
mod provider_probe_inventory_observation;
mod provider_probe_mandates;
pub mod provider_probes;
mod provider_profile_validation;
pub mod provider_profiles;
mod provider_service_launch;
pub mod qualification;
pub mod recovery_ops;
pub mod restore;
pub mod store;
mod task_assignment;
mod task_bundle;
pub mod terminal_allocations;
mod workflow_activation;
mod workflow_run_creation;
mod workflow_run_plan;
mod workflow_run_projection;
mod workflow_run_start;
pub mod writer;

pub use admission::{
    ActiveAdmission, AdmissionCandidate, AdmissionCommit, AdmissionDecision,
    AdmissionEligibilityObservation, AdmissionOutcome, AdmissionSnapshotToken, AdmissionUnit,
    AdmissionUrgency, ExecutionAdmissionSnapshot, ExecutionCheckpointObservation,
    ExecutionCheckpointRequestRecord, ExecutionContinuation, WorkerLaunchPreparation,
    WorkerProcessObservation,
};
pub use agent_creation::PreparedAgentCreation;
pub use agent_from_task::AgentWorkflowPreparation;
pub use agent_projection::AgentProjection;
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
    ExecutionOwner, ExecutionPoolParams, PricingCheckStatus, ensure_global_execution_pool_tx,
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
pub use initial_context::InitialContextPlan;
pub use invocation_projection::{InvocationAttemptProjection, InvocationProjection};
pub use invocation_queue::InvocationQueuePlan;
pub use leases::{AcquireLeaseRequest, FenceAssertion, LeaseState, ResourceLeaseRecord};
pub use migrations::{CURRENT_SCHEMA_VERSION, MigrationRecord, MigrationReport, MigrationState};
pub use mission_scope::PreparedMissionScope;
pub use model_selection_bindings::{ModelSelectionBinding, ModelSelectionBindingChange};
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
    ClaimProviderProbeConnectRequest, ProviderConnectPolicySnapshot, ProviderProbeConnectClaim,
    ProviderProbeConnectClaimOutcome, ProviderProbeConnectClaimRecord, ProviderProbeConnectContext,
    ProviderProbeConnectPreparation, ProviderProbeConnectRecord,
};
pub use provider_probe_connect_disposition::ProviderConnectDispositionCommit;
pub use provider_probe_connect_observation::{
    AdoptProviderConnectObservation, AdoptedProviderConnectObservation, CONNECT_OBSERVER_PRINCIPAL,
    CommitProviderConnectObservation, ConnectCertificateValidation, ProviderConnectObservation,
};
pub use provider_probe_generation::AuthenticatedGenerationPreparation;
pub use provider_probe_generation::dispatch::{
    ManagedGenerationDispatchPermit, ManagedGenerationObservation, ManagedOllamaTransport,
};
pub use provider_probe_generation::placement::{
    AdmittedOllamaChatInputs, ManagedGenerationAdmission, ManagedProviderListener,
    PLACEMENT_OBSERVER_PRINCIPAL,
};
pub use provider_probe_inventory::{
    ClaimProviderProbeInventoryRequest, InventoryTransportLimits, ProviderProbeInventoryClaim,
    ProviderProbeInventoryClaimOutcome, ProviderProbeInventoryPreparation,
    ProviderProbeInventoryRecord,
};
pub use provider_probe_inventory_observation::{
    AdoptProviderInventoryObservation, AdoptedProviderInventoryObservation,
    CommitProviderInventoryObservation, INVENTORY_OBSERVER_PRINCIPAL, INVENTORY_PRODUCER_KIND,
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
pub use provider_service_launch::{
    AttestedProviderServiceLaunch, ProviderServiceBackend, ProviderServiceLaunchAttestation,
    ProviderServiceLaunchContext, ProviderServiceLaunchFingerprint, ProviderServiceLaunchOutcome,
    ProviderServiceLaunchPermit, ProviderServiceLaunchPlan, ProviderServiceLaunchRefusal,
    ProviderServiceLaunchRefusalRecord, ProviderServiceProcessIdentity,
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

pub use workflow_activation::WorkflowTaskActivationPlan;
pub use workflow_run_creation::PreparedRunPublication;
pub use workflow_run_projection::WorkflowRunProjection;
pub use workflow_run_start::PreparedRunStart;

pub use task_assignment::PreparedTaskAssignment;
