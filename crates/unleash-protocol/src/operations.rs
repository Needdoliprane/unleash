//! # Catalogue des opérations protocolaires
//!
//! Ce module contient l'énumération exhaustive des 204 opérations client
//! déclarées dans `.claude/catalog/operations.json` et `.claude/schemas/operation-name.schema.json`.
//!
//! Invariant : bijection exacte 1-à-1 avec le catalogue de conception.

#![allow(clippy::too_many_lines, clippy::match_same_arms)]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ts_rs::TS;

/// Famille ou nature d'une opération protocolaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationKind {
    /// Mutation d'état soumise à commande durable, autorité et idempotence.
    Command,
    /// Lecture sans effet secondaire, avec watermark transactionnel optionnel.
    Query,
    /// Flux continu ou abonnement d'événements/trames.
    Stream,
    /// Négociation de transport et de capacités à l'ouverture de session.
    Handshake,
}

/// Énumération exhaustive des 204 opérations client d'Unleash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[non_exhaustive]
pub enum OperationName {
    /// `AcceptStep` (command)
    AcceptStep,
    /// `AcquireTerminal` (command)
    AcquireTerminal,
    /// `ApplyGuidedSetup` (command)
    ApplyGuidedSetup,
    /// `AssignDocumentScope` (command)
    AssignDocumentScope,
    /// `AssignMission` (command)
    AssignMission,
    /// `AssignResource` (command)
    AssignResource,
    /// `AssignTask` (command)
    AssignTask,
    /// `AttachDiscoveryReceipt` (command)
    AttachDiscoveryReceipt,
    /// `BeginSecretImport` (command)
    BeginSecretImport,
    /// `BindRole` (command)
    BindRole,
    /// `BrowseResources` (query)
    BrowseResources,
    /// `CancelAccountConnection` (command)
    CancelAccountConnection,
    /// `CancelMission` (command)
    CancelMission,
    /// `CancelRun` (command)
    CancelRun,
    /// `CompareContexts` (query)
    CompareContexts,
    /// `CompareReports` (query)
    CompareReports,
    /// `ConfigureGuard` (command)
    ConfigureGuard,
    /// `ConfigureWatch` (command)
    ConfigureWatch,
    /// `ConnectAccount` (command)
    ConnectAccount,
    /// `CreateAgent` (command)
    CreateAgent,
    /// `CreateAutomationDraft` (command)
    CreateAutomationDraft,
    /// `CreateGoal` (command)
    CreateGoal,
    /// `CreateGuidedSetupDraft` (command)
    CreateGuidedSetupDraft,
    /// `CreateProject` (command)
    CreateProject,
    /// `CreateReportSnapshot` (command)
    CreateReportSnapshot,
    /// `CreateRoleDraft` (command)
    CreateRoleDraft,
    /// `CreateRun` (command)
    CreateRun,
    /// `CreateSkillDraft` (command)
    CreateSkillDraft,
    /// `CreateWorkflowDraft` (command)
    CreateWorkflowDraft,
    /// `DeclareWorkIntent` (command)
    DeclareWorkIntent,
    /// `DeprecateAutomationVersion` (command)
    DeprecateAutomationVersion,
    /// `DeprecateRoleVersion` (command)
    DeprecateRoleVersion,
    /// `DeprecateSkillVersion` (command)
    DeprecateSkillVersion,
    /// `DisconnectAccount` (command)
    DisconnectAccount,
    /// `DiscoverResources` (command)
    DiscoverResources,
    /// `DiscoverTerminals` (query)
    DiscoverTerminals,
    /// `DraftWorkflowFromIntent` (command)
    DraftWorkflowFromIntent,
    /// `EvaluateAutomationDraft` (command)
    EvaluateAutomationDraft,
    /// `ExportBundle` (command)
    ExportBundle,
    /// `ExportReport` (command)
    ExportReport,
    /// `GetAccountConnection` (query)
    GetAccountConnection,
    /// `GetActivitySnapshot` (query)
    GetActivitySnapshot,
    /// `GetAgent` (query)
    GetAgent,
    /// `GetAutomationDataRevision` (query)
    GetAutomationDataRevision,
    /// `GetAutomationDraft` (query)
    GetAutomationDraft,
    /// `GetAutomationEvaluation` (query)
    GetAutomationEvaluation,
    /// `GetAutomationVersion` (query)
    GetAutomationVersion,
    /// `GetBackupManifest` (query)
    GetBackupManifest,
    /// `GetBinding` (query)
    GetBinding,
    /// `GetClientCapabilities` (query)
    GetClientCapabilities,
    /// `GetCommandReceipt` (query)
    GetCommandReceipt,
    /// `GetConnectorAccount` (query)
    GetConnectorAccount,
    /// `GetContextManifest` (query)
    GetContextManifest,
    /// `GetConversation` (query)
    GetConversation,
    /// `GetDaemonStatus` (query)
    GetDaemonStatus,
    /// `GetDecision` (query)
    GetDecision,
    /// `GetDependency` (query)
    GetDependency,
    /// `GetDiscoveryReceipt` (query)
    GetDiscoveryReceipt,
    /// `GetEvaluationReport` (query)
    GetEvaluationReport,
    /// `GetGoal` (query)
    GetGoal,
    /// `GetGuidedSetupDraft` (query)
    GetGuidedSetupDraft,
    /// `GetGuidedSetupValidation` (query)
    GetGuidedSetupValidation,
    /// `GetHealth` (query)
    GetHealth,
    /// `GetIntegration` (query)
    GetIntegration,
    /// `GetInvocation` (query)
    GetInvocation,
    /// `GetInvocationManifest` (query)
    GetInvocationManifest,
    /// `GetMessage` (query)
    GetMessage,
    /// `GetMission` (query)
    GetMission,
    /// `GetOperation` (query)
    GetOperation,
    /// `GetOverview` (query)
    GetOverview,
    /// `GetProcess` (query)
    GetProcess,
    /// `GetProject` (query)
    GetProject,
    /// `GetProviderProfile` (query)
    GetProviderProfile,
    /// `GetReportSnapshot` (query)
    GetReportSnapshot,
    /// `GetResource` (query)
    GetResource,
    /// `GetRoleDraft` (query)
    GetRoleDraft,
    /// `GetRoleVersion` (query)
    GetRoleVersion,
    /// `GetRun` (query)
    GetRun,
    /// `GetSecretReferenceStatus` (query)
    GetSecretReferenceStatus,
    /// `GetSetupPreview` (query)
    GetSetupPreview,
    /// `GetSkillDraft` (query)
    GetSkillDraft,
    /// `GetSkillVersion` (query)
    GetSkillVersion,
    /// `GetSnapshot` (query)
    GetSnapshot,
    /// `GetStateSnapshot` (query)
    GetStateSnapshot,
    /// `GetVersionDiff` (query)
    GetVersionDiff,
    /// `GetWorkflowDraft` (query)
    GetWorkflowDraft,
    /// `GetWorkflowSimulation` (query)
    GetWorkflowSimulation,
    /// `GetWorkflowValidation` (query)
    GetWorkflowValidation,
    /// `GetWorkflowVersion` (query)
    GetWorkflowVersion,
    /// `GetWorkspace` (query)
    GetWorkspace,
    /// `GetWorkspaceDiff` (query)
    GetWorkspaceDiff,
    /// `GetWorktree` (query)
    GetWorktree,
    /// `Hello` (handshake)
    Hello,
    /// `ImportAutomationData` (command)
    ImportAutomationData,
    /// `InspectRecovery` (query)
    InspectRecovery,
    /// `LearnAutomation` (command)
    LearnAutomation,
    /// `ListAgents` (query)
    ListAgents,
    /// `ListAlerts` (query)
    ListAlerts,
    /// `ListAssignments` (query)
    ListAssignments,
    /// `ListConnectorAccounts` (query)
    ListConnectorAccounts,
    /// `ListConversations` (query)
    ListConversations,
    /// `ListDecisions` (query)
    ListDecisions,
    /// `ListDependencies` (query)
    ListDependencies,
    /// `ListGoals` (query)
    ListGoals,
    /// `ListGuidedTemplates` (query)
    ListGuidedTemplates,
    /// `ListInvocations` (query)
    ListInvocations,
    /// `ListMissions` (query)
    ListMissions,
    /// `ListProcesses` (query)
    ListProcesses,
    /// `ListProjects` (query)
    ListProjects,
    /// `ListProviderProfiles` (query)
    ListProviderProfiles,
    /// `ListReports` (query)
    ListReports,
    /// `ListRuns` (query)
    ListRuns,
    /// `ListVersionUsages` (query)
    ListVersionUsages,
    /// `ListWorkflows` (query)
    ListWorkflows,
    /// `ListWorkspaces` (query)
    ListWorkspaces,
    /// `ListWorktrees` (query)
    ListWorktrees,
    /// `NormalizeWorkflowDraft` (command)
    NormalizeWorkflowDraft,
    /// `PauseMission` (command)
    PauseMission,
    /// `PauseRun` (command)
    PauseRun,
    /// `PrepareIntegration` (command)
    PrepareIntegration,
    /// `PreviewResource` (query)
    PreviewResource,
    /// `PublishAutomationVersion` (command)
    PublishAutomationVersion,
    /// `PublishIntegration` (command)
    PublishIntegration,
    /// `PublishRoleVersion` (command)
    PublishRoleVersion,
    /// `PublishSkillVersion` (command)
    PublishSkillVersion,
    /// `PublishWorkflowVersion` (command)
    PublishWorkflowVersion,
    /// `PurgeHistory` (command)
    PurgeHistory,
    /// `ReadArtifact` (query)
    ReadArtifact,
    /// `ReadEvidence` (query)
    ReadEvidence,
    /// `ReadGuardHealth` (query)
    ReadGuardHealth,
    /// `ReadPolicySummary` (query)
    ReadPolicySummary,
    /// `ReadSettings` (query)
    ReadSettings,
    /// `ReadTerminalLog` (query)
    ReadTerminalLog,
    /// `RebuildIndex` (command)
    RebuildIndex,
    /// `ReconcileAction` (command)
    ReconcileAction,
    /// `ReconcileProcess` (command)
    ReconcileProcess,
    /// `RecordClaim` (command)
    RecordClaim,
    /// `RecordDeclassificationDecision` (command)
    RecordDeclassificationDecision,
    /// `RecordDiscoveryDecision` (command)
    RecordDiscoveryDecision,
    /// `RecordGoalDecision` (command)
    RecordGoalDecision,
    /// `RecordJudgment` (command)
    RecordJudgment,
    /// `RecordMissionDecision` (command)
    RecordMissionDecision,
    /// `RecordObservation` (command)
    RecordObservation,
    /// `RefreshResource` (command)
    RefreshResource,
    /// `ReleaseTerminal` (command)
    ReleaseTerminal,
    /// `RemoveAssignment` (command)
    RemoveAssignment,
    /// `RepairAutomation` (command)
    RepairAutomation,
    /// `RequestAgentReassignment` (command)
    RequestAgentReassignment,
    /// `RequestBackup` (command)
    RequestBackup,
    /// `RequestContextCompaction` (command)
    RequestContextCompaction,
    /// `RequestCorrection` (command)
    RequestCorrection,
    /// `RequestDependency` (command)
    RequestDependency,
    /// `RequestInterruption` (command)
    RequestInterruption,
    /// `RequestSkillEvaluation` (command)
    RequestSkillEvaluation,
    /// `ResizeTerminal` (command)
    ResizeTerminal,
    /// `ResolveActionApproval` (command)
    ResolveActionApproval,
    /// `ResolveActionOutcome` (command)
    ResolveActionOutcome,
    /// `RestoreBackup` (command)
    RestoreBackup,
    /// `ResumeMission` (command)
    ResumeMission,
    /// `ResumeRecoveredInvocation` (command)
    ResumeRecoveredInvocation,
    /// `ResumeRun` (command)
    ResumeRun,
    /// `ResumeSubject` (command)
    ResumeSubject,
    /// `RetireWorktree` (command)
    RetireWorktree,
    /// `ReviseGoal` (command)
    ReviseGoal,
    /// `ReviseMission` (command)
    ReviseMission,
    /// `ReviseProject` (command)
    ReviseProject,
    /// `RevokeAutomationVersion` (command)
    RevokeAutomationVersion,
    /// `RevokeRoleVersion` (command)
    RevokeRoleVersion,
    /// `RevokeSecretReference` (command)
    RevokeSecretReference,
    /// `RevokeSkillVersion` (command)
    RevokeSkillVersion,
    /// `RevokeSubject` (command)
    RevokeSubject,
    /// `SaveAutomationDraft` (command)
    SaveAutomationDraft,
    /// `SaveGuidedSetupDraft` (command)
    SaveGuidedSetupDraft,
    /// `SaveRoleDraft` (command)
    SaveRoleDraft,
    /// `SaveSkillDraft` (command)
    SaveSkillDraft,
    /// `SaveWorkflowDraft` (command)
    SaveWorkflowDraft,
    /// `SearchAutomations` (query)
    SearchAutomations,
    /// `SearchConversations` (query)
    SearchConversations,
    /// `SearchEvents` (query)
    SearchEvents,
    /// `SearchKnowledge` (query)
    SearchKnowledge,
    /// `SearchResources` (query)
    SearchResources,
    /// `SearchRoles` (query)
    SearchRoles,
    /// `SearchSkills` (query)
    SearchSkills,
    /// `SendHumanMessage` (command)
    SendHumanMessage,
    /// `SendTerminalInput` (command)
    SendTerminalInput,
    /// `SetPolicy` (command)
    SetPolicy,
    /// `SetProviderProfile` (command)
    SetProviderProfile,
    /// `SetSecretReference` (command)
    SetSecretReference,
    /// `SetSettings` (command)
    SetSettings,
    /// `SimulateWorkflow` (command)
    SimulateWorkflow,
    /// `StartRun` (command)
    StartRun,
    /// `StopDaemon` (command)
    StopDaemon,
    /// `StopProcess` (command)
    StopProcess,
    /// `SubscribeEvents` (stream)
    SubscribeEvents,
    /// `SuspendSubject` (command)
    SuspendSubject,
    /// `TestConnectorAccount` (command)
    TestConnectorAccount,
    /// `TestProviderProfile` (command)
    TestProviderProfile,
    /// `ValidateAutomationData` (command)
    ValidateAutomationData,
    /// `ValidateAutomationDraft` (command)
    ValidateAutomationDraft,
    /// `ValidateBackup` (command)
    ValidateBackup,
    /// `ValidateGuidedSetup` (command)
    ValidateGuidedSetup,
    /// `ValidateRole` (command)
    ValidateRole,
    /// `ValidateSkill` (command)
    ValidateSkill,
    /// `ValidateWorkflowDraft` (command)
    ValidateWorkflowDraft,
}

impl OperationName {
    /// Renvoie le nom textuel canonique de l'opération.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AcceptStep => "AcceptStep",
            Self::AcquireTerminal => "AcquireTerminal",
            Self::ApplyGuidedSetup => "ApplyGuidedSetup",
            Self::AssignDocumentScope => "AssignDocumentScope",
            Self::AssignMission => "AssignMission",
            Self::AssignResource => "AssignResource",
            Self::AssignTask => "AssignTask",
            Self::AttachDiscoveryReceipt => "AttachDiscoveryReceipt",
            Self::BeginSecretImport => "BeginSecretImport",
            Self::BindRole => "BindRole",
            Self::BrowseResources => "BrowseResources",
            Self::CancelAccountConnection => "CancelAccountConnection",
            Self::CancelMission => "CancelMission",
            Self::CancelRun => "CancelRun",
            Self::CompareContexts => "CompareContexts",
            Self::CompareReports => "CompareReports",
            Self::ConfigureGuard => "ConfigureGuard",
            Self::ConfigureWatch => "ConfigureWatch",
            Self::ConnectAccount => "ConnectAccount",
            Self::CreateAgent => "CreateAgent",
            Self::CreateAutomationDraft => "CreateAutomationDraft",
            Self::CreateGoal => "CreateGoal",
            Self::CreateGuidedSetupDraft => "CreateGuidedSetupDraft",
            Self::CreateProject => "CreateProject",
            Self::CreateReportSnapshot => "CreateReportSnapshot",
            Self::CreateRoleDraft => "CreateRoleDraft",
            Self::CreateRun => "CreateRun",
            Self::CreateSkillDraft => "CreateSkillDraft",
            Self::CreateWorkflowDraft => "CreateWorkflowDraft",
            Self::DeclareWorkIntent => "DeclareWorkIntent",
            Self::DeprecateAutomationVersion => "DeprecateAutomationVersion",
            Self::DeprecateRoleVersion => "DeprecateRoleVersion",
            Self::DeprecateSkillVersion => "DeprecateSkillVersion",
            Self::DisconnectAccount => "DisconnectAccount",
            Self::DiscoverResources => "DiscoverResources",
            Self::DiscoverTerminals => "DiscoverTerminals",
            Self::DraftWorkflowFromIntent => "DraftWorkflowFromIntent",
            Self::EvaluateAutomationDraft => "EvaluateAutomationDraft",
            Self::ExportBundle => "ExportBundle",
            Self::ExportReport => "ExportReport",
            Self::GetAccountConnection => "GetAccountConnection",
            Self::GetActivitySnapshot => "GetActivitySnapshot",
            Self::GetAgent => "GetAgent",
            Self::GetAutomationDataRevision => "GetAutomationDataRevision",
            Self::GetAutomationDraft => "GetAutomationDraft",
            Self::GetAutomationEvaluation => "GetAutomationEvaluation",
            Self::GetAutomationVersion => "GetAutomationVersion",
            Self::GetBackupManifest => "GetBackupManifest",
            Self::GetBinding => "GetBinding",
            Self::GetClientCapabilities => "GetClientCapabilities",
            Self::GetCommandReceipt => "GetCommandReceipt",
            Self::GetConnectorAccount => "GetConnectorAccount",
            Self::GetContextManifest => "GetContextManifest",
            Self::GetConversation => "GetConversation",
            Self::GetDaemonStatus => "GetDaemonStatus",
            Self::GetDecision => "GetDecision",
            Self::GetDependency => "GetDependency",
            Self::GetDiscoveryReceipt => "GetDiscoveryReceipt",
            Self::GetEvaluationReport => "GetEvaluationReport",
            Self::GetGoal => "GetGoal",
            Self::GetGuidedSetupDraft => "GetGuidedSetupDraft",
            Self::GetGuidedSetupValidation => "GetGuidedSetupValidation",
            Self::GetHealth => "GetHealth",
            Self::GetIntegration => "GetIntegration",
            Self::GetInvocation => "GetInvocation",
            Self::GetInvocationManifest => "GetInvocationManifest",
            Self::GetMessage => "GetMessage",
            Self::GetMission => "GetMission",
            Self::GetOperation => "GetOperation",
            Self::GetOverview => "GetOverview",
            Self::GetProcess => "GetProcess",
            Self::GetProject => "GetProject",
            Self::GetProviderProfile => "GetProviderProfile",
            Self::GetReportSnapshot => "GetReportSnapshot",
            Self::GetResource => "GetResource",
            Self::GetRoleDraft => "GetRoleDraft",
            Self::GetRoleVersion => "GetRoleVersion",
            Self::GetRun => "GetRun",
            Self::GetSecretReferenceStatus => "GetSecretReferenceStatus",
            Self::GetSetupPreview => "GetSetupPreview",
            Self::GetSkillDraft => "GetSkillDraft",
            Self::GetSkillVersion => "GetSkillVersion",
            Self::GetSnapshot => "GetSnapshot",
            Self::GetStateSnapshot => "GetStateSnapshot",
            Self::GetVersionDiff => "GetVersionDiff",
            Self::GetWorkflowDraft => "GetWorkflowDraft",
            Self::GetWorkflowSimulation => "GetWorkflowSimulation",
            Self::GetWorkflowValidation => "GetWorkflowValidation",
            Self::GetWorkflowVersion => "GetWorkflowVersion",
            Self::GetWorkspace => "GetWorkspace",
            Self::GetWorkspaceDiff => "GetWorkspaceDiff",
            Self::GetWorktree => "GetWorktree",
            Self::Hello => "Hello",
            Self::ImportAutomationData => "ImportAutomationData",
            Self::InspectRecovery => "InspectRecovery",
            Self::LearnAutomation => "LearnAutomation",
            Self::ListAgents => "ListAgents",
            Self::ListAlerts => "ListAlerts",
            Self::ListAssignments => "ListAssignments",
            Self::ListConnectorAccounts => "ListConnectorAccounts",
            Self::ListConversations => "ListConversations",
            Self::ListDecisions => "ListDecisions",
            Self::ListDependencies => "ListDependencies",
            Self::ListGoals => "ListGoals",
            Self::ListGuidedTemplates => "ListGuidedTemplates",
            Self::ListInvocations => "ListInvocations",
            Self::ListMissions => "ListMissions",
            Self::ListProcesses => "ListProcesses",
            Self::ListProjects => "ListProjects",
            Self::ListProviderProfiles => "ListProviderProfiles",
            Self::ListReports => "ListReports",
            Self::ListRuns => "ListRuns",
            Self::ListVersionUsages => "ListVersionUsages",
            Self::ListWorkflows => "ListWorkflows",
            Self::ListWorkspaces => "ListWorkspaces",
            Self::ListWorktrees => "ListWorktrees",
            Self::NormalizeWorkflowDraft => "NormalizeWorkflowDraft",
            Self::PauseMission => "PauseMission",
            Self::PauseRun => "PauseRun",
            Self::PrepareIntegration => "PrepareIntegration",
            Self::PreviewResource => "PreviewResource",
            Self::PublishAutomationVersion => "PublishAutomationVersion",
            Self::PublishIntegration => "PublishIntegration",
            Self::PublishRoleVersion => "PublishRoleVersion",
            Self::PublishSkillVersion => "PublishSkillVersion",
            Self::PublishWorkflowVersion => "PublishWorkflowVersion",
            Self::PurgeHistory => "PurgeHistory",
            Self::ReadArtifact => "ReadArtifact",
            Self::ReadEvidence => "ReadEvidence",
            Self::ReadGuardHealth => "ReadGuardHealth",
            Self::ReadPolicySummary => "ReadPolicySummary",
            Self::ReadSettings => "ReadSettings",
            Self::ReadTerminalLog => "ReadTerminalLog",
            Self::RebuildIndex => "RebuildIndex",
            Self::ReconcileAction => "ReconcileAction",
            Self::ReconcileProcess => "ReconcileProcess",
            Self::RecordClaim => "RecordClaim",
            Self::RecordDeclassificationDecision => "RecordDeclassificationDecision",
            Self::RecordDiscoveryDecision => "RecordDiscoveryDecision",
            Self::RecordGoalDecision => "RecordGoalDecision",
            Self::RecordJudgment => "RecordJudgment",
            Self::RecordMissionDecision => "RecordMissionDecision",
            Self::RecordObservation => "RecordObservation",
            Self::RefreshResource => "RefreshResource",
            Self::ReleaseTerminal => "ReleaseTerminal",
            Self::RemoveAssignment => "RemoveAssignment",
            Self::RepairAutomation => "RepairAutomation",
            Self::RequestAgentReassignment => "RequestAgentReassignment",
            Self::RequestBackup => "RequestBackup",
            Self::RequestContextCompaction => "RequestContextCompaction",
            Self::RequestCorrection => "RequestCorrection",
            Self::RequestDependency => "RequestDependency",
            Self::RequestInterruption => "RequestInterruption",
            Self::RequestSkillEvaluation => "RequestSkillEvaluation",
            Self::ResizeTerminal => "ResizeTerminal",
            Self::ResolveActionApproval => "ResolveActionApproval",
            Self::ResolveActionOutcome => "ResolveActionOutcome",
            Self::RestoreBackup => "RestoreBackup",
            Self::ResumeMission => "ResumeMission",
            Self::ResumeRecoveredInvocation => "ResumeRecoveredInvocation",
            Self::ResumeRun => "ResumeRun",
            Self::ResumeSubject => "ResumeSubject",
            Self::RetireWorktree => "RetireWorktree",
            Self::ReviseGoal => "ReviseGoal",
            Self::ReviseMission => "ReviseMission",
            Self::ReviseProject => "ReviseProject",
            Self::RevokeAutomationVersion => "RevokeAutomationVersion",
            Self::RevokeRoleVersion => "RevokeRoleVersion",
            Self::RevokeSecretReference => "RevokeSecretReference",
            Self::RevokeSkillVersion => "RevokeSkillVersion",
            Self::RevokeSubject => "RevokeSubject",
            Self::SaveAutomationDraft => "SaveAutomationDraft",
            Self::SaveGuidedSetupDraft => "SaveGuidedSetupDraft",
            Self::SaveRoleDraft => "SaveRoleDraft",
            Self::SaveSkillDraft => "SaveSkillDraft",
            Self::SaveWorkflowDraft => "SaveWorkflowDraft",
            Self::SearchAutomations => "SearchAutomations",
            Self::SearchConversations => "SearchConversations",
            Self::SearchEvents => "SearchEvents",
            Self::SearchKnowledge => "SearchKnowledge",
            Self::SearchResources => "SearchResources",
            Self::SearchRoles => "SearchRoles",
            Self::SearchSkills => "SearchSkills",
            Self::SendHumanMessage => "SendHumanMessage",
            Self::SendTerminalInput => "SendTerminalInput",
            Self::SetPolicy => "SetPolicy",
            Self::SetProviderProfile => "SetProviderProfile",
            Self::SetSecretReference => "SetSecretReference",
            Self::SetSettings => "SetSettings",
            Self::SimulateWorkflow => "SimulateWorkflow",
            Self::StartRun => "StartRun",
            Self::StopDaemon => "StopDaemon",
            Self::StopProcess => "StopProcess",
            Self::SubscribeEvents => "SubscribeEvents",
            Self::SuspendSubject => "SuspendSubject",
            Self::TestConnectorAccount => "TestConnectorAccount",
            Self::TestProviderProfile => "TestProviderProfile",
            Self::ValidateAutomationData => "ValidateAutomationData",
            Self::ValidateAutomationDraft => "ValidateAutomationDraft",
            Self::ValidateBackup => "ValidateBackup",
            Self::ValidateGuidedSetup => "ValidateGuidedSetup",
            Self::ValidateRole => "ValidateRole",
            Self::ValidateSkill => "ValidateSkill",
            Self::ValidateWorkflowDraft => "ValidateWorkflowDraft",
        }
    }

    /// Renvoie la nature (kind) de l'opération.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::AcceptStep => OperationKind::Command,
            Self::AcquireTerminal => OperationKind::Command,
            Self::ApplyGuidedSetup => OperationKind::Command,
            Self::AssignDocumentScope => OperationKind::Command,
            Self::AssignMission => OperationKind::Command,
            Self::AssignResource => OperationKind::Command,
            Self::AssignTask => OperationKind::Command,
            Self::AttachDiscoveryReceipt => OperationKind::Command,
            Self::BeginSecretImport => OperationKind::Command,
            Self::BindRole => OperationKind::Command,
            Self::BrowseResources => OperationKind::Query,
            Self::CancelAccountConnection => OperationKind::Command,
            Self::CancelMission => OperationKind::Command,
            Self::CancelRun => OperationKind::Command,
            Self::CompareContexts => OperationKind::Query,
            Self::CompareReports => OperationKind::Query,
            Self::ConfigureGuard => OperationKind::Command,
            Self::ConfigureWatch => OperationKind::Command,
            Self::ConnectAccount => OperationKind::Command,
            Self::CreateAgent => OperationKind::Command,
            Self::CreateAutomationDraft => OperationKind::Command,
            Self::CreateGoal => OperationKind::Command,
            Self::CreateGuidedSetupDraft => OperationKind::Command,
            Self::CreateProject => OperationKind::Command,
            Self::CreateReportSnapshot => OperationKind::Command,
            Self::CreateRoleDraft => OperationKind::Command,
            Self::CreateRun => OperationKind::Command,
            Self::CreateSkillDraft => OperationKind::Command,
            Self::CreateWorkflowDraft => OperationKind::Command,
            Self::DeclareWorkIntent => OperationKind::Command,
            Self::DeprecateAutomationVersion => OperationKind::Command,
            Self::DeprecateRoleVersion => OperationKind::Command,
            Self::DeprecateSkillVersion => OperationKind::Command,
            Self::DisconnectAccount => OperationKind::Command,
            Self::DiscoverResources => OperationKind::Command,
            Self::DiscoverTerminals => OperationKind::Query,
            Self::DraftWorkflowFromIntent => OperationKind::Command,
            Self::EvaluateAutomationDraft => OperationKind::Command,
            Self::ExportBundle => OperationKind::Command,
            Self::ExportReport => OperationKind::Command,
            Self::GetAccountConnection => OperationKind::Query,
            Self::GetActivitySnapshot => OperationKind::Query,
            Self::GetAgent => OperationKind::Query,
            Self::GetAutomationDataRevision => OperationKind::Query,
            Self::GetAutomationDraft => OperationKind::Query,
            Self::GetAutomationEvaluation => OperationKind::Query,
            Self::GetAutomationVersion => OperationKind::Query,
            Self::GetBackupManifest => OperationKind::Query,
            Self::GetBinding => OperationKind::Query,
            Self::GetClientCapabilities => OperationKind::Query,
            Self::GetCommandReceipt => OperationKind::Query,
            Self::GetConnectorAccount => OperationKind::Query,
            Self::GetContextManifest => OperationKind::Query,
            Self::GetConversation => OperationKind::Query,
            Self::GetDaemonStatus => OperationKind::Query,
            Self::GetDecision => OperationKind::Query,
            Self::GetDependency => OperationKind::Query,
            Self::GetDiscoveryReceipt => OperationKind::Query,
            Self::GetEvaluationReport => OperationKind::Query,
            Self::GetGoal => OperationKind::Query,
            Self::GetGuidedSetupDraft => OperationKind::Query,
            Self::GetGuidedSetupValidation => OperationKind::Query,
            Self::GetHealth => OperationKind::Query,
            Self::GetIntegration => OperationKind::Query,
            Self::GetInvocation => OperationKind::Query,
            Self::GetInvocationManifest => OperationKind::Query,
            Self::GetMessage => OperationKind::Query,
            Self::GetMission => OperationKind::Query,
            Self::GetOperation => OperationKind::Query,
            Self::GetOverview => OperationKind::Query,
            Self::GetProcess => OperationKind::Query,
            Self::GetProject => OperationKind::Query,
            Self::GetProviderProfile => OperationKind::Query,
            Self::GetReportSnapshot => OperationKind::Query,
            Self::GetResource => OperationKind::Query,
            Self::GetRoleDraft => OperationKind::Query,
            Self::GetRoleVersion => OperationKind::Query,
            Self::GetRun => OperationKind::Query,
            Self::GetSecretReferenceStatus => OperationKind::Query,
            Self::GetSetupPreview => OperationKind::Query,
            Self::GetSkillDraft => OperationKind::Query,
            Self::GetSkillVersion => OperationKind::Query,
            Self::GetSnapshot => OperationKind::Query,
            Self::GetStateSnapshot => OperationKind::Query,
            Self::GetVersionDiff => OperationKind::Query,
            Self::GetWorkflowDraft => OperationKind::Query,
            Self::GetWorkflowSimulation => OperationKind::Query,
            Self::GetWorkflowValidation => OperationKind::Query,
            Self::GetWorkflowVersion => OperationKind::Query,
            Self::GetWorkspace => OperationKind::Query,
            Self::GetWorkspaceDiff => OperationKind::Query,
            Self::GetWorktree => OperationKind::Query,
            Self::Hello => OperationKind::Handshake,
            Self::ImportAutomationData => OperationKind::Command,
            Self::InspectRecovery => OperationKind::Query,
            Self::LearnAutomation => OperationKind::Command,
            Self::ListAgents => OperationKind::Query,
            Self::ListAlerts => OperationKind::Query,
            Self::ListAssignments => OperationKind::Query,
            Self::ListConnectorAccounts => OperationKind::Query,
            Self::ListConversations => OperationKind::Query,
            Self::ListDecisions => OperationKind::Query,
            Self::ListDependencies => OperationKind::Query,
            Self::ListGoals => OperationKind::Query,
            Self::ListGuidedTemplates => OperationKind::Query,
            Self::ListInvocations => OperationKind::Query,
            Self::ListMissions => OperationKind::Query,
            Self::ListProcesses => OperationKind::Query,
            Self::ListProjects => OperationKind::Query,
            Self::ListProviderProfiles => OperationKind::Query,
            Self::ListReports => OperationKind::Query,
            Self::ListRuns => OperationKind::Query,
            Self::ListVersionUsages => OperationKind::Query,
            Self::ListWorkflows => OperationKind::Query,
            Self::ListWorkspaces => OperationKind::Query,
            Self::ListWorktrees => OperationKind::Query,
            Self::NormalizeWorkflowDraft => OperationKind::Command,
            Self::PauseMission => OperationKind::Command,
            Self::PauseRun => OperationKind::Command,
            Self::PrepareIntegration => OperationKind::Command,
            Self::PreviewResource => OperationKind::Query,
            Self::PublishAutomationVersion => OperationKind::Command,
            Self::PublishIntegration => OperationKind::Command,
            Self::PublishRoleVersion => OperationKind::Command,
            Self::PublishSkillVersion => OperationKind::Command,
            Self::PublishWorkflowVersion => OperationKind::Command,
            Self::PurgeHistory => OperationKind::Command,
            Self::ReadArtifact => OperationKind::Query,
            Self::ReadEvidence => OperationKind::Query,
            Self::ReadGuardHealth => OperationKind::Query,
            Self::ReadPolicySummary => OperationKind::Query,
            Self::ReadSettings => OperationKind::Query,
            Self::ReadTerminalLog => OperationKind::Query,
            Self::RebuildIndex => OperationKind::Command,
            Self::ReconcileAction => OperationKind::Command,
            Self::ReconcileProcess => OperationKind::Command,
            Self::RecordClaim => OperationKind::Command,
            Self::RecordDeclassificationDecision => OperationKind::Command,
            Self::RecordDiscoveryDecision => OperationKind::Command,
            Self::RecordGoalDecision => OperationKind::Command,
            Self::RecordJudgment => OperationKind::Command,
            Self::RecordMissionDecision => OperationKind::Command,
            Self::RecordObservation => OperationKind::Command,
            Self::RefreshResource => OperationKind::Command,
            Self::ReleaseTerminal => OperationKind::Command,
            Self::RemoveAssignment => OperationKind::Command,
            Self::RepairAutomation => OperationKind::Command,
            Self::RequestAgentReassignment => OperationKind::Command,
            Self::RequestBackup => OperationKind::Command,
            Self::RequestContextCompaction => OperationKind::Command,
            Self::RequestCorrection => OperationKind::Command,
            Self::RequestDependency => OperationKind::Command,
            Self::RequestInterruption => OperationKind::Command,
            Self::RequestSkillEvaluation => OperationKind::Command,
            Self::ResizeTerminal => OperationKind::Command,
            Self::ResolveActionApproval => OperationKind::Command,
            Self::ResolveActionOutcome => OperationKind::Command,
            Self::RestoreBackup => OperationKind::Command,
            Self::ResumeMission => OperationKind::Command,
            Self::ResumeRecoveredInvocation => OperationKind::Command,
            Self::ResumeRun => OperationKind::Command,
            Self::ResumeSubject => OperationKind::Command,
            Self::RetireWorktree => OperationKind::Command,
            Self::ReviseGoal => OperationKind::Command,
            Self::ReviseMission => OperationKind::Command,
            Self::ReviseProject => OperationKind::Command,
            Self::RevokeAutomationVersion => OperationKind::Command,
            Self::RevokeRoleVersion => OperationKind::Command,
            Self::RevokeSecretReference => OperationKind::Command,
            Self::RevokeSkillVersion => OperationKind::Command,
            Self::RevokeSubject => OperationKind::Command,
            Self::SaveAutomationDraft => OperationKind::Command,
            Self::SaveGuidedSetupDraft => OperationKind::Command,
            Self::SaveRoleDraft => OperationKind::Command,
            Self::SaveSkillDraft => OperationKind::Command,
            Self::SaveWorkflowDraft => OperationKind::Command,
            Self::SearchAutomations => OperationKind::Query,
            Self::SearchConversations => OperationKind::Query,
            Self::SearchEvents => OperationKind::Query,
            Self::SearchKnowledge => OperationKind::Query,
            Self::SearchResources => OperationKind::Query,
            Self::SearchRoles => OperationKind::Query,
            Self::SearchSkills => OperationKind::Query,
            Self::SendHumanMessage => OperationKind::Command,
            Self::SendTerminalInput => OperationKind::Command,
            Self::SetPolicy => OperationKind::Command,
            Self::SetProviderProfile => OperationKind::Command,
            Self::SetSecretReference => OperationKind::Command,
            Self::SetSettings => OperationKind::Command,
            Self::SimulateWorkflow => OperationKind::Command,
            Self::StartRun => OperationKind::Command,
            Self::StopDaemon => OperationKind::Command,
            Self::StopProcess => OperationKind::Command,
            Self::SubscribeEvents => OperationKind::Stream,
            Self::SuspendSubject => OperationKind::Command,
            Self::TestConnectorAccount => OperationKind::Command,
            Self::TestProviderProfile => OperationKind::Command,
            Self::ValidateAutomationData => OperationKind::Command,
            Self::ValidateAutomationDraft => OperationKind::Command,
            Self::ValidateBackup => OperationKind::Command,
            Self::ValidateGuidedSetup => OperationKind::Command,
            Self::ValidateRole => OperationKind::Command,
            Self::ValidateSkill => OperationKind::Command,
            Self::ValidateWorkflowDraft => OperationKind::Command,
        }
    }

    /// Renvoie les identifiants de groupes de parité GUI/TUI associés.
    #[must_use]
    pub const fn parity_group_ids(&self) -> &'static [&'static str] {
        match self {
            Self::AcceptStep => &["PAR-WORKFLOWS-05"],
            Self::AcquireTerminal => &["PAR-ACTIVITY-04", "PAR-SETTINGS-05"],
            Self::ApplyGuidedSetup => &["PAR-ONBOARDING-01"],
            Self::AssignDocumentScope => &["PAR-KNOWLEDGE-04"],
            Self::AssignMission => &["PAR-OVERVIEW-03"],
            Self::AssignResource => &["PAR-KNOWLEDGE-04"],
            Self::AssignTask => &["PAR-AGENTS-02"],
            Self::AttachDiscoveryReceipt => &["PAR-SKILLS-02", "PAR-KNOWLEDGE-05"],
            Self::BeginSecretImport => &["PAR-SETTINGS-01"],
            Self::BindRole => &["PAR-WORKFLOWS-05"],
            Self::BrowseResources => &["PAR-KNOWLEDGE-03"],
            Self::CancelAccountConnection => &["PAR-SETTINGS-02"],
            Self::CancelMission => &["PAR-OVERVIEW-04"],
            Self::CancelRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05"],
            Self::CompareContexts => &["PAR-AGENTS-03", "PAR-KNOWLEDGE-05"],
            Self::CompareReports => &["PAR-REPORTS-03"],
            Self::ConfigureGuard => &["PAR-SETTINGS-03"],
            Self::ConfigureWatch => &["PAR-KNOWLEDGE-04"],
            Self::ConnectAccount => &["PAR-SETTINGS-02"],
            Self::CreateAgent => &["PAR-AGENTS-01"],
            Self::CreateAutomationDraft => &["PAR-AUTOMATION-02"],
            Self::CreateGoal => &["PAR-OVERVIEW-02"],
            Self::CreateGuidedSetupDraft => &["PAR-ONBOARDING-01"],
            Self::CreateProject => &["PAR-OVERVIEW-01"],
            Self::CreateReportSnapshot => &["PAR-REPORTS-05", "PAR-AUTOMATION-06"],
            Self::CreateRoleDraft => &["PAR-SKILLS-03"],
            Self::CreateRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05", "PAR-AUTOMATION-06"],
            Self::CreateSkillDraft => &["PAR-SKILLS-02"],
            Self::CreateWorkflowDraft => &["PAR-WORKFLOWS-02"],
            Self::DeclareWorkIntent => &["PAR-KNOWLEDGE-02"],
            Self::DeprecateAutomationVersion => &["PAR-AUTOMATION-04"],
            Self::DeprecateRoleVersion => &["PAR-SKILLS-05"],
            Self::DeprecateSkillVersion => &["PAR-SKILLS-05"],
            Self::DisconnectAccount => &["PAR-SETTINGS-02"],
            Self::DiscoverResources => &["PAR-WORKSPACES-05", "PAR-SKILLS-02", "PAR-KNOWLEDGE-01"],
            Self::DiscoverTerminals => &["PAR-ACTIVITY-03"],
            Self::DraftWorkflowFromIntent => &["PAR-WORKFLOWS-02"],
            Self::EvaluateAutomationDraft => &["PAR-AUTOMATION-04"],
            Self::ExportBundle => &[
                "PAR-AGENTS-03",
                "PAR-SKILLS-05",
                "PAR-ACTIVITY-03",
                "PAR-REPORTS-05",
            ],
            Self::ExportReport => &["PAR-REPORTS-05", "PAR-AUTOMATION-06"],
            Self::GetAccountConnection => &["PAR-SETTINGS-02"],
            Self::GetActivitySnapshot => &["PAR-ACTIVITY-01"],
            Self::GetAgent => &["PAR-AGENTS-01", "PAR-AGENTS-02"],
            Self::GetAutomationDataRevision => &["PAR-AUTOMATION-03", "PAR-AUTOMATION-05"],
            Self::GetAutomationDraft => &["PAR-AUTOMATION-02"],
            Self::GetAutomationEvaluation => &["PAR-AUTOMATION-04", "PAR-AUTOMATION-06"],
            Self::GetAutomationVersion => &["PAR-AUTOMATION-01"],
            Self::GetBackupManifest => &["PAR-SETTINGS-05"],
            Self::GetBinding => &["PAR-WORKFLOWS-05"],
            Self::GetClientCapabilities => &["PAR-SETTINGS-04"],
            Self::GetCommandReceipt => &[
                "PAR-OVERVIEW-04",
                "PAR-OVERVIEW-05",
                "PAR-CONVERSATIONS-05",
                "PAR-ACTIVITY-02",
            ],
            Self::GetConnectorAccount => &["PAR-SETTINGS-02"],
            Self::GetContextManifest => &["PAR-AGENTS-03", "PAR-KNOWLEDGE-05"],
            Self::GetConversation => &["PAR-AGENTS-04", "PAR-CONVERSATIONS-01"],
            Self::GetDaemonStatus => &["PAR-SETTINGS-05"],
            Self::GetDecision => &[
                "PAR-OVERVIEW-05",
                "PAR-CONVERSATIONS-03",
                "PAR-CONVERSATIONS-04",
                "PAR-REPORTS-02",
                "PAR-REPORTS-03",
            ],
            Self::GetDependency => &["PAR-WORKSPACES-01", "PAR-WORKSPACES-05", "PAR-KNOWLEDGE-02"],
            Self::GetDiscoveryReceipt => &["PAR-KNOWLEDGE-05"],
            Self::GetEvaluationReport => &["PAR-SKILLS-04"],
            Self::GetGoal => &["PAR-OVERVIEW-01", "PAR-OVERVIEW-02", "PAR-REPORTS-01"],
            Self::GetGuidedSetupDraft => &["PAR-ONBOARDING-01"],
            Self::GetGuidedSetupValidation => &["PAR-ONBOARDING-01"],
            Self::GetHealth => &["PAR-SETTINGS-05"],
            Self::GetIntegration => &["PAR-WORKSPACES-04", "PAR-ACTIVITY-05"],
            Self::GetInvocation => &["PAR-AGENTS-02"],
            Self::GetInvocationManifest => &["PAR-AGENTS-03"],
            Self::GetMessage => &[
                "PAR-AGENTS-04",
                "PAR-CONVERSATIONS-02",
                "PAR-CONVERSATIONS-05",
            ],
            Self::GetMission => &["PAR-OVERVIEW-01", "PAR-OVERVIEW-03", "PAR-REPORTS-01"],
            Self::GetOperation => &["PAR-OVERVIEW-04", "PAR-OVERVIEW-05", "PAR-ACTIVITY-02"],
            Self::GetOverview => &["PAR-OVERVIEW-01", "PAR-ONBOARDING-01"],
            Self::GetProcess => &["PAR-AGENTS-05", "PAR-ACTIVITY-03", "PAR-ACTIVITY-05"],
            Self::GetProject => &["PAR-OVERVIEW-01"],
            Self::GetProviderProfile => &["PAR-SETTINGS-01", "PAR-LOCAL-01"],
            Self::GetReportSnapshot => &[
                "PAR-CONVERSATIONS-03",
                "PAR-REPORTS-01",
                "PAR-REPORTS-02",
                "PAR-AUTOMATION-06",
            ],
            Self::GetResource => &[
                "PAR-CONVERSATIONS-03",
                "PAR-KNOWLEDGE-03",
                "PAR-ACTIVITY-05",
                "PAR-REPORTS-04",
            ],
            Self::GetRoleDraft => &["PAR-SKILLS-03"],
            Self::GetRoleVersion => &["PAR-AGENTS-01", "PAR-SKILLS-01"],
            Self::GetRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05", "PAR-AUTOMATION-06"],
            Self::GetSecretReferenceStatus => &["PAR-SETTINGS-01"],
            Self::GetSetupPreview => &["PAR-ONBOARDING-01"],
            Self::GetSkillDraft => &["PAR-SKILLS-02"],
            Self::GetSkillVersion => &["PAR-AGENTS-01", "PAR-SKILLS-01"],
            Self::GetSnapshot => &[
                "PAR-WORKSPACES-02",
                "PAR-WORKSPACES-03",
                "PAR-CONVERSATIONS-03",
                "PAR-KNOWLEDGE-02",
                "PAR-ACTIVITY-05",
                "PAR-REPORTS-03",
                "PAR-REPORTS-04",
            ],
            Self::GetStateSnapshot => &["PAR-OVERVIEW-01", "PAR-ACTIVITY-02"],
            Self::GetVersionDiff => &[
                "PAR-WORKFLOWS-01",
                "PAR-AUTOMATION-01",
                "PAR-SKILLS-04",
                "PAR-REPORTS-03",
                "PAR-AUTOMATION-06",
            ],
            Self::GetWorkflowDraft => &["PAR-WORKFLOWS-02"],
            Self::GetWorkflowSimulation => &["PAR-WORKFLOWS-04"],
            Self::GetWorkflowValidation => &["PAR-WORKFLOWS-03"],
            Self::GetWorkflowVersion => &["PAR-WORKFLOWS-01"],
            Self::GetWorkspace => &["PAR-WORKSPACES-01", "PAR-WORKSPACES-02"],
            Self::GetWorkspaceDiff => &["PAR-WORKSPACES-03"],
            Self::GetWorktree => &["PAR-WORKSPACES-02"],
            Self::Hello => &[],
            Self::ImportAutomationData => &["PAR-AUTOMATION-03"],
            Self::InspectRecovery => &["PAR-SETTINGS-05"],
            Self::LearnAutomation => &["PAR-AUTOMATION-02"],
            Self::ListAgents => &["PAR-AGENTS-01"],
            Self::ListAlerts => &["PAR-OVERVIEW-05"],
            Self::ListAssignments => &["PAR-KNOWLEDGE-04"],
            Self::ListConnectorAccounts => &["PAR-SETTINGS-02"],
            Self::ListConversations => &["PAR-CONVERSATIONS-01"],
            Self::ListDecisions => &["PAR-OVERVIEW-05", "PAR-CONVERSATIONS-04"],
            Self::ListDependencies => &["PAR-OVERVIEW-03", "PAR-WORKSPACES-01", "PAR-KNOWLEDGE-02"],
            Self::ListGoals => &["PAR-OVERVIEW-01"],
            Self::ListGuidedTemplates => &["PAR-ONBOARDING-01"],
            Self::ListInvocations => &["PAR-AGENTS-02"],
            Self::ListMissions => &["PAR-OVERVIEW-01"],
            Self::ListProcesses => &["PAR-ACTIVITY-03"],
            Self::ListProjects => &["PAR-OVERVIEW-01"],
            Self::ListProviderProfiles => &["PAR-SETTINGS-01", "PAR-LOCAL-01"],
            Self::ListReports => &["PAR-REPORTS-01"],
            Self::ListRuns => &["PAR-WORKFLOWS-05"],
            Self::ListVersionUsages => &[
                "PAR-WORKFLOWS-01",
                "PAR-AUTOMATION-01",
                "PAR-SKILLS-01",
                "PAR-SKILLS-05",
            ],
            Self::ListWorkflows => &["PAR-WORKFLOWS-01"],
            Self::ListWorkspaces => &["PAR-WORKSPACES-01"],
            Self::ListWorktrees => &["PAR-WORKSPACES-02"],
            Self::NormalizeWorkflowDraft => &["PAR-WORKFLOWS-03"],
            Self::PauseMission => &["PAR-OVERVIEW-04", "PAR-AGENTS-05"],
            Self::PauseRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05"],
            Self::PrepareIntegration => &["PAR-WORKSPACES-04"],
            Self::PreviewResource => &["PAR-KNOWLEDGE-03"],
            Self::PublishAutomationVersion => &["PAR-AUTOMATION-04"],
            Self::PublishIntegration => &["PAR-WORKSPACES-04"],
            Self::PublishRoleVersion => &["PAR-SKILLS-05"],
            Self::PublishSkillVersion => &["PAR-SKILLS-05"],
            Self::PublishWorkflowVersion => &["PAR-WORKFLOWS-05"],
            Self::PurgeHistory => &["PAR-SETTINGS-05"],
            Self::ReadArtifact => &[
                "PAR-KNOWLEDGE-03",
                "PAR-REPORTS-04",
                "PAR-REPORTS-05",
                "PAR-AUTOMATION-06",
            ],
            Self::ReadEvidence => &[
                "PAR-CONVERSATIONS-03",
                "PAR-ACTIVITY-05",
                "PAR-REPORTS-04",
                "PAR-AUTOMATION-06",
            ],
            Self::ReadGuardHealth => &["PAR-SETTINGS-03"],
            Self::ReadPolicySummary => &["PAR-SETTINGS-03"],
            Self::ReadSettings => &["PAR-SETTINGS-04", "PAR-SETTINGS-05", "PAR-LOCAL-01"],
            Self::ReadTerminalLog => &["PAR-ACTIVITY-03", "PAR-ACTIVITY-04"],
            Self::RebuildIndex => &["PAR-SETTINGS-05"],
            Self::ReconcileAction => &["PAR-AUTOMATION-05", "PAR-AUTOMATION-06", "PAR-SETTINGS-05"],
            Self::ReconcileProcess => &["PAR-ACTIVITY-04"],
            Self::RecordClaim => &[],
            Self::RecordDeclassificationDecision => &["PAR-SETTINGS-03"],
            Self::RecordDiscoveryDecision => {
                &["PAR-WORKSPACES-05", "PAR-SKILLS-02", "PAR-KNOWLEDGE-05"]
            }
            Self::RecordGoalDecision => &["PAR-OVERVIEW-02", "PAR-CONVERSATIONS-04"],
            Self::RecordJudgment => &[],
            Self::RecordMissionDecision => &["PAR-OVERVIEW-03", "PAR-CONVERSATIONS-04"],
            Self::RecordObservation => &[],
            Self::RefreshResource => &["PAR-KNOWLEDGE-04"],
            Self::ReleaseTerminal => &["PAR-ACTIVITY-04"],
            Self::RemoveAssignment => &["PAR-KNOWLEDGE-04"],
            Self::RepairAutomation => &["PAR-AUTOMATION-06"],
            Self::RequestAgentReassignment => &["PAR-AGENTS-05"],
            Self::RequestBackup => &["PAR-SETTINGS-05"],
            Self::RequestContextCompaction => &["PAR-AGENTS-03"],
            Self::RequestCorrection => &["PAR-WORKFLOWS-05"],
            Self::RequestDependency => &["PAR-WORKSPACES-05", "PAR-KNOWLEDGE-05"],
            Self::RequestInterruption => &["PAR-AGENTS-05"],
            Self::RequestSkillEvaluation => &["PAR-SKILLS-04"],
            Self::ResizeTerminal => &["PAR-ACTIVITY-04"],
            Self::ResolveActionApproval => &["PAR-SETTINGS-03"],
            Self::ResolveActionOutcome => {
                &["PAR-AUTOMATION-05", "PAR-AUTOMATION-06", "PAR-SETTINGS-05"]
            }
            Self::RestoreBackup => &["PAR-SETTINGS-05"],
            Self::ResumeMission => &["PAR-OVERVIEW-04", "PAR-AGENTS-05"],
            Self::ResumeRecoveredInvocation => &["PAR-SETTINGS-05"],
            Self::ResumeRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05"],
            Self::ResumeSubject => &["PAR-SETTINGS-03"],
            Self::RetireWorktree => &["PAR-WORKSPACES-02"],
            Self::ReviseGoal => &["PAR-OVERVIEW-02"],
            Self::ReviseMission => &["PAR-OVERVIEW-03", "PAR-CONVERSATIONS-04"],
            Self::ReviseProject => &["PAR-OVERVIEW-01"],
            Self::RevokeAutomationVersion => &["PAR-AUTOMATION-04"],
            Self::RevokeRoleVersion => &["PAR-SKILLS-05"],
            Self::RevokeSecretReference => &["PAR-SETTINGS-01"],
            Self::RevokeSkillVersion => &["PAR-SKILLS-05"],
            Self::RevokeSubject => &["PAR-SETTINGS-03"],
            Self::SaveAutomationDraft => &["PAR-AUTOMATION-02"],
            Self::SaveGuidedSetupDraft => &["PAR-ONBOARDING-01"],
            Self::SaveRoleDraft => &["PAR-SKILLS-03"],
            Self::SaveSkillDraft => &["PAR-SKILLS-02"],
            Self::SaveWorkflowDraft => &["PAR-WORKFLOWS-02"],
            Self::SearchAutomations => &["PAR-AUTOMATION-01"],
            Self::SearchConversations => &["PAR-CONVERSATIONS-01"],
            Self::SearchEvents => &["PAR-ACTIVITY-01", "PAR-ACTIVITY-05"],
            Self::SearchKnowledge => &["PAR-KNOWLEDGE-01", "PAR-KNOWLEDGE-02"],
            Self::SearchResources => &["PAR-KNOWLEDGE-03"],
            Self::SearchRoles => &["PAR-SKILLS-01"],
            Self::SearchSkills => &["PAR-SKILLS-01"],
            Self::SendHumanMessage => &[
                "PAR-AGENTS-04",
                "PAR-CONVERSATIONS-02",
                "PAR-CONVERSATIONS-03",
                "PAR-CONVERSATIONS-05",
            ],
            Self::SendTerminalInput => &["PAR-ACTIVITY-04"],
            Self::SetPolicy => &["PAR-SETTINGS-03"],
            Self::SetProviderProfile => &["PAR-SETTINGS-01", "PAR-SETTINGS-04", "PAR-LOCAL-01"],
            Self::SetSecretReference => &["PAR-SETTINGS-01"],
            Self::SetSettings => &["PAR-SETTINGS-04", "PAR-SETTINGS-05"],
            Self::SimulateWorkflow => &["PAR-WORKFLOWS-04"],
            Self::StartRun => &["PAR-WORKFLOWS-05", "PAR-AUTOMATION-05"],
            Self::StopDaemon => &["PAR-SETTINGS-05"],
            Self::StopProcess => &["PAR-ACTIVITY-04"],
            Self::SubscribeEvents => &["PAR-ACTIVITY-02"],
            Self::SuspendSubject => &["PAR-SETTINGS-03"],
            Self::TestConnectorAccount => &["PAR-SETTINGS-02"],
            Self::TestProviderProfile => &["PAR-SETTINGS-01", "PAR-LOCAL-01"],
            Self::ValidateAutomationData => &["PAR-AUTOMATION-03"],
            Self::ValidateAutomationDraft => &["PAR-AUTOMATION-04"],
            Self::ValidateBackup => &["PAR-SETTINGS-05"],
            Self::ValidateGuidedSetup => &["PAR-ONBOARDING-01"],
            Self::ValidateRole => &["PAR-SKILLS-04"],
            Self::ValidateSkill => &["PAR-SKILLS-04"],
            Self::ValidateWorkflowDraft => &["PAR-WORKFLOWS-03"],
        }
    }

    /// Indique si cette opération est exposée directement dans l'interface graphique (GUI).
    /// Sont exclues : `Hello`, `RecordClaim`, `RecordObservation`, `RecordJudgment`.
    #[must_use]
    pub const fn is_exposed_to_gui(&self) -> bool {
        !matches!(
            self,
            Self::Hello | Self::RecordClaim | Self::RecordObservation | Self::RecordJudgment
        )
    }

    /// Renvoie la liste ordonnée des 204 opérations.
    #[must_use]
    pub const fn all_operations() -> &'static [Self] {
        &[
            Self::AcceptStep,
            Self::AcquireTerminal,
            Self::ApplyGuidedSetup,
            Self::AssignDocumentScope,
            Self::AssignMission,
            Self::AssignResource,
            Self::AssignTask,
            Self::AttachDiscoveryReceipt,
            Self::BeginSecretImport,
            Self::BindRole,
            Self::BrowseResources,
            Self::CancelAccountConnection,
            Self::CancelMission,
            Self::CancelRun,
            Self::CompareContexts,
            Self::CompareReports,
            Self::ConfigureGuard,
            Self::ConfigureWatch,
            Self::ConnectAccount,
            Self::CreateAgent,
            Self::CreateAutomationDraft,
            Self::CreateGoal,
            Self::CreateGuidedSetupDraft,
            Self::CreateProject,
            Self::CreateReportSnapshot,
            Self::CreateRoleDraft,
            Self::CreateRun,
            Self::CreateSkillDraft,
            Self::CreateWorkflowDraft,
            Self::DeclareWorkIntent,
            Self::DeprecateAutomationVersion,
            Self::DeprecateRoleVersion,
            Self::DeprecateSkillVersion,
            Self::DisconnectAccount,
            Self::DiscoverResources,
            Self::DiscoverTerminals,
            Self::DraftWorkflowFromIntent,
            Self::EvaluateAutomationDraft,
            Self::ExportBundle,
            Self::ExportReport,
            Self::GetAccountConnection,
            Self::GetActivitySnapshot,
            Self::GetAgent,
            Self::GetAutomationDataRevision,
            Self::GetAutomationDraft,
            Self::GetAutomationEvaluation,
            Self::GetAutomationVersion,
            Self::GetBackupManifest,
            Self::GetBinding,
            Self::GetClientCapabilities,
            Self::GetCommandReceipt,
            Self::GetConnectorAccount,
            Self::GetContextManifest,
            Self::GetConversation,
            Self::GetDaemonStatus,
            Self::GetDecision,
            Self::GetDependency,
            Self::GetDiscoveryReceipt,
            Self::GetEvaluationReport,
            Self::GetGoal,
            Self::GetGuidedSetupDraft,
            Self::GetGuidedSetupValidation,
            Self::GetHealth,
            Self::GetIntegration,
            Self::GetInvocation,
            Self::GetInvocationManifest,
            Self::GetMessage,
            Self::GetMission,
            Self::GetOperation,
            Self::GetOverview,
            Self::GetProcess,
            Self::GetProject,
            Self::GetProviderProfile,
            Self::GetReportSnapshot,
            Self::GetResource,
            Self::GetRoleDraft,
            Self::GetRoleVersion,
            Self::GetRun,
            Self::GetSecretReferenceStatus,
            Self::GetSetupPreview,
            Self::GetSkillDraft,
            Self::GetSkillVersion,
            Self::GetSnapshot,
            Self::GetStateSnapshot,
            Self::GetVersionDiff,
            Self::GetWorkflowDraft,
            Self::GetWorkflowSimulation,
            Self::GetWorkflowValidation,
            Self::GetWorkflowVersion,
            Self::GetWorkspace,
            Self::GetWorkspaceDiff,
            Self::GetWorktree,
            Self::Hello,
            Self::ImportAutomationData,
            Self::InspectRecovery,
            Self::LearnAutomation,
            Self::ListAgents,
            Self::ListAlerts,
            Self::ListAssignments,
            Self::ListConnectorAccounts,
            Self::ListConversations,
            Self::ListDecisions,
            Self::ListDependencies,
            Self::ListGoals,
            Self::ListGuidedTemplates,
            Self::ListInvocations,
            Self::ListMissions,
            Self::ListProcesses,
            Self::ListProjects,
            Self::ListProviderProfiles,
            Self::ListReports,
            Self::ListRuns,
            Self::ListVersionUsages,
            Self::ListWorkflows,
            Self::ListWorkspaces,
            Self::ListWorktrees,
            Self::NormalizeWorkflowDraft,
            Self::PauseMission,
            Self::PauseRun,
            Self::PrepareIntegration,
            Self::PreviewResource,
            Self::PublishAutomationVersion,
            Self::PublishIntegration,
            Self::PublishRoleVersion,
            Self::PublishSkillVersion,
            Self::PublishWorkflowVersion,
            Self::PurgeHistory,
            Self::ReadArtifact,
            Self::ReadEvidence,
            Self::ReadGuardHealth,
            Self::ReadPolicySummary,
            Self::ReadSettings,
            Self::ReadTerminalLog,
            Self::RebuildIndex,
            Self::ReconcileAction,
            Self::ReconcileProcess,
            Self::RecordClaim,
            Self::RecordDeclassificationDecision,
            Self::RecordDiscoveryDecision,
            Self::RecordGoalDecision,
            Self::RecordJudgment,
            Self::RecordMissionDecision,
            Self::RecordObservation,
            Self::RefreshResource,
            Self::ReleaseTerminal,
            Self::RemoveAssignment,
            Self::RepairAutomation,
            Self::RequestAgentReassignment,
            Self::RequestBackup,
            Self::RequestContextCompaction,
            Self::RequestCorrection,
            Self::RequestDependency,
            Self::RequestInterruption,
            Self::RequestSkillEvaluation,
            Self::ResizeTerminal,
            Self::ResolveActionApproval,
            Self::ResolveActionOutcome,
            Self::RestoreBackup,
            Self::ResumeMission,
            Self::ResumeRecoveredInvocation,
            Self::ResumeRun,
            Self::ResumeSubject,
            Self::RetireWorktree,
            Self::ReviseGoal,
            Self::ReviseMission,
            Self::ReviseProject,
            Self::RevokeAutomationVersion,
            Self::RevokeRoleVersion,
            Self::RevokeSecretReference,
            Self::RevokeSkillVersion,
            Self::RevokeSubject,
            Self::SaveAutomationDraft,
            Self::SaveGuidedSetupDraft,
            Self::SaveRoleDraft,
            Self::SaveSkillDraft,
            Self::SaveWorkflowDraft,
            Self::SearchAutomations,
            Self::SearchConversations,
            Self::SearchEvents,
            Self::SearchKnowledge,
            Self::SearchResources,
            Self::SearchRoles,
            Self::SearchSkills,
            Self::SendHumanMessage,
            Self::SendTerminalInput,
            Self::SetPolicy,
            Self::SetProviderProfile,
            Self::SetSecretReference,
            Self::SetSettings,
            Self::SimulateWorkflow,
            Self::StartRun,
            Self::StopDaemon,
            Self::StopProcess,
            Self::SubscribeEvents,
            Self::SuspendSubject,
            Self::TestConnectorAccount,
            Self::TestProviderProfile,
            Self::ValidateAutomationData,
            Self::ValidateAutomationDraft,
            Self::ValidateBackup,
            Self::ValidateGuidedSetup,
            Self::ValidateRole,
            Self::ValidateSkill,
            Self::ValidateWorkflowDraft,
        ]
    }
}

impl fmt::Display for OperationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur lors de la résolution d'un nom d'opération inconnu.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Opération inconnue : '{0}'")]
pub struct UnknownOperationError(pub String);

impl FromStr for OperationName {
    type Err = UnknownOperationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "AcceptStep" => Ok(Self::AcceptStep),
            "AcquireTerminal" => Ok(Self::AcquireTerminal),
            "ApplyGuidedSetup" => Ok(Self::ApplyGuidedSetup),
            "AssignDocumentScope" => Ok(Self::AssignDocumentScope),
            "AssignMission" => Ok(Self::AssignMission),
            "AssignResource" => Ok(Self::AssignResource),
            "AssignTask" => Ok(Self::AssignTask),
            "AttachDiscoveryReceipt" => Ok(Self::AttachDiscoveryReceipt),
            "BeginSecretImport" => Ok(Self::BeginSecretImport),
            "BindRole" => Ok(Self::BindRole),
            "BrowseResources" => Ok(Self::BrowseResources),
            "CancelAccountConnection" => Ok(Self::CancelAccountConnection),
            "CancelMission" => Ok(Self::CancelMission),
            "CancelRun" => Ok(Self::CancelRun),
            "CompareContexts" => Ok(Self::CompareContexts),
            "CompareReports" => Ok(Self::CompareReports),
            "ConfigureGuard" => Ok(Self::ConfigureGuard),
            "ConfigureWatch" => Ok(Self::ConfigureWatch),
            "ConnectAccount" => Ok(Self::ConnectAccount),
            "CreateAgent" => Ok(Self::CreateAgent),
            "CreateAutomationDraft" => Ok(Self::CreateAutomationDraft),
            "CreateGoal" => Ok(Self::CreateGoal),
            "CreateGuidedSetupDraft" => Ok(Self::CreateGuidedSetupDraft),
            "CreateProject" => Ok(Self::CreateProject),
            "CreateReportSnapshot" => Ok(Self::CreateReportSnapshot),
            "CreateRoleDraft" => Ok(Self::CreateRoleDraft),
            "CreateRun" => Ok(Self::CreateRun),
            "CreateSkillDraft" => Ok(Self::CreateSkillDraft),
            "CreateWorkflowDraft" => Ok(Self::CreateWorkflowDraft),
            "DeclareWorkIntent" => Ok(Self::DeclareWorkIntent),
            "DeprecateAutomationVersion" => Ok(Self::DeprecateAutomationVersion),
            "DeprecateRoleVersion" => Ok(Self::DeprecateRoleVersion),
            "DeprecateSkillVersion" => Ok(Self::DeprecateSkillVersion),
            "DisconnectAccount" => Ok(Self::DisconnectAccount),
            "DiscoverResources" => Ok(Self::DiscoverResources),
            "DiscoverTerminals" => Ok(Self::DiscoverTerminals),
            "DraftWorkflowFromIntent" => Ok(Self::DraftWorkflowFromIntent),
            "EvaluateAutomationDraft" => Ok(Self::EvaluateAutomationDraft),
            "ExportBundle" => Ok(Self::ExportBundle),
            "ExportReport" => Ok(Self::ExportReport),
            "GetAccountConnection" => Ok(Self::GetAccountConnection),
            "GetActivitySnapshot" => Ok(Self::GetActivitySnapshot),
            "GetAgent" => Ok(Self::GetAgent),
            "GetAutomationDataRevision" => Ok(Self::GetAutomationDataRevision),
            "GetAutomationDraft" => Ok(Self::GetAutomationDraft),
            "GetAutomationEvaluation" => Ok(Self::GetAutomationEvaluation),
            "GetAutomationVersion" => Ok(Self::GetAutomationVersion),
            "GetBackupManifest" => Ok(Self::GetBackupManifest),
            "GetBinding" => Ok(Self::GetBinding),
            "GetClientCapabilities" => Ok(Self::GetClientCapabilities),
            "GetCommandReceipt" => Ok(Self::GetCommandReceipt),
            "GetConnectorAccount" => Ok(Self::GetConnectorAccount),
            "GetContextManifest" => Ok(Self::GetContextManifest),
            "GetConversation" => Ok(Self::GetConversation),
            "GetDaemonStatus" => Ok(Self::GetDaemonStatus),
            "GetDecision" => Ok(Self::GetDecision),
            "GetDependency" => Ok(Self::GetDependency),
            "GetDiscoveryReceipt" => Ok(Self::GetDiscoveryReceipt),
            "GetEvaluationReport" => Ok(Self::GetEvaluationReport),
            "GetGoal" => Ok(Self::GetGoal),
            "GetGuidedSetupDraft" => Ok(Self::GetGuidedSetupDraft),
            "GetGuidedSetupValidation" => Ok(Self::GetGuidedSetupValidation),
            "GetHealth" => Ok(Self::GetHealth),
            "GetIntegration" => Ok(Self::GetIntegration),
            "GetInvocation" => Ok(Self::GetInvocation),
            "GetInvocationManifest" => Ok(Self::GetInvocationManifest),
            "GetMessage" => Ok(Self::GetMessage),
            "GetMission" => Ok(Self::GetMission),
            "GetOperation" => Ok(Self::GetOperation),
            "GetOverview" => Ok(Self::GetOverview),
            "GetProcess" => Ok(Self::GetProcess),
            "GetProject" => Ok(Self::GetProject),
            "GetProviderProfile" => Ok(Self::GetProviderProfile),
            "GetReportSnapshot" => Ok(Self::GetReportSnapshot),
            "GetResource" => Ok(Self::GetResource),
            "GetRoleDraft" => Ok(Self::GetRoleDraft),
            "GetRoleVersion" => Ok(Self::GetRoleVersion),
            "GetRun" => Ok(Self::GetRun),
            "GetSecretReferenceStatus" => Ok(Self::GetSecretReferenceStatus),
            "GetSetupPreview" => Ok(Self::GetSetupPreview),
            "GetSkillDraft" => Ok(Self::GetSkillDraft),
            "GetSkillVersion" => Ok(Self::GetSkillVersion),
            "GetSnapshot" => Ok(Self::GetSnapshot),
            "GetStateSnapshot" => Ok(Self::GetStateSnapshot),
            "GetVersionDiff" => Ok(Self::GetVersionDiff),
            "GetWorkflowDraft" => Ok(Self::GetWorkflowDraft),
            "GetWorkflowSimulation" => Ok(Self::GetWorkflowSimulation),
            "GetWorkflowValidation" => Ok(Self::GetWorkflowValidation),
            "GetWorkflowVersion" => Ok(Self::GetWorkflowVersion),
            "GetWorkspace" => Ok(Self::GetWorkspace),
            "GetWorkspaceDiff" => Ok(Self::GetWorkspaceDiff),
            "GetWorktree" => Ok(Self::GetWorktree),
            "Hello" => Ok(Self::Hello),
            "ImportAutomationData" => Ok(Self::ImportAutomationData),
            "InspectRecovery" => Ok(Self::InspectRecovery),
            "LearnAutomation" => Ok(Self::LearnAutomation),
            "ListAgents" => Ok(Self::ListAgents),
            "ListAlerts" => Ok(Self::ListAlerts),
            "ListAssignments" => Ok(Self::ListAssignments),
            "ListConnectorAccounts" => Ok(Self::ListConnectorAccounts),
            "ListConversations" => Ok(Self::ListConversations),
            "ListDecisions" => Ok(Self::ListDecisions),
            "ListDependencies" => Ok(Self::ListDependencies),
            "ListGoals" => Ok(Self::ListGoals),
            "ListGuidedTemplates" => Ok(Self::ListGuidedTemplates),
            "ListInvocations" => Ok(Self::ListInvocations),
            "ListMissions" => Ok(Self::ListMissions),
            "ListProcesses" => Ok(Self::ListProcesses),
            "ListProjects" => Ok(Self::ListProjects),
            "ListProviderProfiles" => Ok(Self::ListProviderProfiles),
            "ListReports" => Ok(Self::ListReports),
            "ListRuns" => Ok(Self::ListRuns),
            "ListVersionUsages" => Ok(Self::ListVersionUsages),
            "ListWorkflows" => Ok(Self::ListWorkflows),
            "ListWorkspaces" => Ok(Self::ListWorkspaces),
            "ListWorktrees" => Ok(Self::ListWorktrees),
            "NormalizeWorkflowDraft" => Ok(Self::NormalizeWorkflowDraft),
            "PauseMission" => Ok(Self::PauseMission),
            "PauseRun" => Ok(Self::PauseRun),
            "PrepareIntegration" => Ok(Self::PrepareIntegration),
            "PreviewResource" => Ok(Self::PreviewResource),
            "PublishAutomationVersion" => Ok(Self::PublishAutomationVersion),
            "PublishIntegration" => Ok(Self::PublishIntegration),
            "PublishRoleVersion" => Ok(Self::PublishRoleVersion),
            "PublishSkillVersion" => Ok(Self::PublishSkillVersion),
            "PublishWorkflowVersion" => Ok(Self::PublishWorkflowVersion),
            "PurgeHistory" => Ok(Self::PurgeHistory),
            "ReadArtifact" => Ok(Self::ReadArtifact),
            "ReadEvidence" => Ok(Self::ReadEvidence),
            "ReadGuardHealth" => Ok(Self::ReadGuardHealth),
            "ReadPolicySummary" => Ok(Self::ReadPolicySummary),
            "ReadSettings" => Ok(Self::ReadSettings),
            "ReadTerminalLog" => Ok(Self::ReadTerminalLog),
            "RebuildIndex" => Ok(Self::RebuildIndex),
            "ReconcileAction" => Ok(Self::ReconcileAction),
            "ReconcileProcess" => Ok(Self::ReconcileProcess),
            "RecordClaim" => Ok(Self::RecordClaim),
            "RecordDeclassificationDecision" => Ok(Self::RecordDeclassificationDecision),
            "RecordDiscoveryDecision" => Ok(Self::RecordDiscoveryDecision),
            "RecordGoalDecision" => Ok(Self::RecordGoalDecision),
            "RecordJudgment" => Ok(Self::RecordJudgment),
            "RecordMissionDecision" => Ok(Self::RecordMissionDecision),
            "RecordObservation" => Ok(Self::RecordObservation),
            "RefreshResource" => Ok(Self::RefreshResource),
            "ReleaseTerminal" => Ok(Self::ReleaseTerminal),
            "RemoveAssignment" => Ok(Self::RemoveAssignment),
            "RepairAutomation" => Ok(Self::RepairAutomation),
            "RequestAgentReassignment" => Ok(Self::RequestAgentReassignment),
            "RequestBackup" => Ok(Self::RequestBackup),
            "RequestContextCompaction" => Ok(Self::RequestContextCompaction),
            "RequestCorrection" => Ok(Self::RequestCorrection),
            "RequestDependency" => Ok(Self::RequestDependency),
            "RequestInterruption" => Ok(Self::RequestInterruption),
            "RequestSkillEvaluation" => Ok(Self::RequestSkillEvaluation),
            "ResizeTerminal" => Ok(Self::ResizeTerminal),
            "ResolveActionApproval" => Ok(Self::ResolveActionApproval),
            "ResolveActionOutcome" => Ok(Self::ResolveActionOutcome),
            "RestoreBackup" => Ok(Self::RestoreBackup),
            "ResumeMission" => Ok(Self::ResumeMission),
            "ResumeRecoveredInvocation" => Ok(Self::ResumeRecoveredInvocation),
            "ResumeRun" => Ok(Self::ResumeRun),
            "ResumeSubject" => Ok(Self::ResumeSubject),
            "RetireWorktree" => Ok(Self::RetireWorktree),
            "ReviseGoal" => Ok(Self::ReviseGoal),
            "ReviseMission" => Ok(Self::ReviseMission),
            "ReviseProject" => Ok(Self::ReviseProject),
            "RevokeAutomationVersion" => Ok(Self::RevokeAutomationVersion),
            "RevokeRoleVersion" => Ok(Self::RevokeRoleVersion),
            "RevokeSecretReference" => Ok(Self::RevokeSecretReference),
            "RevokeSkillVersion" => Ok(Self::RevokeSkillVersion),
            "RevokeSubject" => Ok(Self::RevokeSubject),
            "SaveAutomationDraft" => Ok(Self::SaveAutomationDraft),
            "SaveGuidedSetupDraft" => Ok(Self::SaveGuidedSetupDraft),
            "SaveRoleDraft" => Ok(Self::SaveRoleDraft),
            "SaveSkillDraft" => Ok(Self::SaveSkillDraft),
            "SaveWorkflowDraft" => Ok(Self::SaveWorkflowDraft),
            "SearchAutomations" => Ok(Self::SearchAutomations),
            "SearchConversations" => Ok(Self::SearchConversations),
            "SearchEvents" => Ok(Self::SearchEvents),
            "SearchKnowledge" => Ok(Self::SearchKnowledge),
            "SearchResources" => Ok(Self::SearchResources),
            "SearchRoles" => Ok(Self::SearchRoles),
            "SearchSkills" => Ok(Self::SearchSkills),
            "SendHumanMessage" => Ok(Self::SendHumanMessage),
            "SendTerminalInput" => Ok(Self::SendTerminalInput),
            "SetPolicy" => Ok(Self::SetPolicy),
            "SetProviderProfile" => Ok(Self::SetProviderProfile),
            "SetSecretReference" => Ok(Self::SetSecretReference),
            "SetSettings" => Ok(Self::SetSettings),
            "SimulateWorkflow" => Ok(Self::SimulateWorkflow),
            "StartRun" => Ok(Self::StartRun),
            "StopDaemon" => Ok(Self::StopDaemon),
            "StopProcess" => Ok(Self::StopProcess),
            "SubscribeEvents" => Ok(Self::SubscribeEvents),
            "SuspendSubject" => Ok(Self::SuspendSubject),
            "TestConnectorAccount" => Ok(Self::TestConnectorAccount),
            "TestProviderProfile" => Ok(Self::TestProviderProfile),
            "ValidateAutomationData" => Ok(Self::ValidateAutomationData),
            "ValidateAutomationDraft" => Ok(Self::ValidateAutomationDraft),
            "ValidateBackup" => Ok(Self::ValidateBackup),
            "ValidateGuidedSetup" => Ok(Self::ValidateGuidedSetup),
            "ValidateRole" => Ok(Self::ValidateRole),
            "ValidateSkill" => Ok(Self::ValidateSkill),
            "ValidateWorkflowDraft" => Ok(Self::ValidateWorkflowDraft),
            _ => Err(UnknownOperationError(s.to_owned())),
        }
    }
}

impl TryFrom<&str> for OperationName {
    type Error = UnknownOperationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}
