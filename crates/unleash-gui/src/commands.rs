// SPDX-License-Identifier: Apache-2.0
//! Commandes Tauri typées et générées 1-à-1 depuis les 200 opérations exposées du catalogue.
//!
//! `Hello`, `RecordClaim`, `RecordObservation` et `RecordJudgment` ne disposent d'aucun handler.
//! Aucune commande proxy générique n'existe.

use serde::Deserialize;
use serde_json::{Map, Value};
use tauri::ipc::Channel;
use unleash_domain::{CommandId, ProjectId};
use unleash_protocol::{CommandReceipt, EventCursor, OperationKind, OperationName, QueryResponse};
use uuid::Uuid;

use crate::errors::GuiError;
use crate::markdown::{MarkdownDocumentAst, parse_markdown_ast};
use crate::registry::{ensure_operation_kind, exposed_gui_operations};
use crate::state::{GuiAppState, GuiStreamNotification};

/// Taille maximale autorisée pour une charge utile JSON reçue de la `WebView` (1 MiB).
pub const MAX_GUI_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Requête typée envoyée par la `WebView` vers une opération de lecture (`OperationKind::Query`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiQueryInput {
    /// Identifiant optionnel du projet ciblé.
    pub project_id: Option<ProjectId>,
    /// Paramètres JSON stricts (taille bornée, `actor_ref` interdit).
    pub payload: Map<String, Value>,
}

/// Commande typée envoyée par la `WebView` vers une opération mutante (`OperationKind::Command`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiCommandInput {
    /// Identifiant optionnel du projet ciblé.
    pub project_id: Option<ProjectId>,
    /// Identifiant UUID de l'agrégat ciblé.
    pub aggregate_id: Uuid,
    /// Révision attendue par contrôle de concurrence optimiste.
    pub expected_revision: u64,
    /// Clé d'idempotence durable conservée sur retry/reconnexion.
    pub command_id: CommandId,
    /// Charge utile métier JSON (`actor_ref` interdit).
    pub payload: Map<String, Value>,
}

/// Paramètres d'abonnement au flux d'événements via un `tauri::ipc::Channel` borné.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiSubscribeInput {
    /// Projet optionnel à filtrer.
    pub project_id: Option<ProjectId>,
    /// Agrégat optionnel à filtrer.
    pub aggregate_id: Option<Uuid>,
    /// Nombre maximal d'événements par lot (`1..=200`).
    pub batch_limit: Option<u32>,
}

/// Paramètres de rendu Markdown vers AST sûr côté Rust.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiMarkdownInput {
    /// Contenu Markdown brut non fiable à convertir en AST borné.
    pub markdown: String,
}

/// Valide les bornes de taille et l'absence de tentative d'usurpation `actor_ref` dans le payload.
///
/// # Errors
/// Retourne [`GuiError::InvalidInput`] si le payload dépasse [`MAX_GUI_PAYLOAD_BYTES`] ou contient `actor_ref`.
pub fn validate_webview_payload(payload: &Map<String, Value>) -> Result<(), GuiError> {
    if payload.contains_key("actor_ref") {
        return Err(GuiError::InvalidInput {
            detail: String::from("Le champ actor_ref est interdit depuis la WebView"),
        });
    }
    let serialized = serde_json::to_vec(payload).map_err(|e| GuiError::InvalidInput {
        detail: format!("Payload JSON invalide : {e}"),
    })?;
    if serialized.len() > MAX_GUI_PAYLOAD_BYTES {
        return Err(GuiError::InvalidInput {
            detail: format!(
                "Payload WebView trop volumineux ({} octets > plafond {})",
                serialized.len(),
                MAX_GUI_PAYLOAD_BYTES
            ),
        });
    }
    Ok(())
}

/// Retourne la liste exacte des 200 opérations exposées au renderer GUI.
#[tauri::command]
pub fn gui_list_exposed_operations() -> Vec<OperationName> {
    exposed_gui_operations()
}

/// Analyse côté Rust un document Markdown non fiable en AST borné sans HTML actif.
///
/// # Errors
/// Retourne [`GuiError::InvalidInput`] si l'entrée dépasse les bornes de sécurité.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "La macro #[tauri::command] exige une valeur possédée pour désérialiser GuiMarkdownInput"
)]
pub fn gui_render_markdown_ast(input: GuiMarkdownInput) -> Result<MarkdownDocumentAst, GuiError> {
    parse_markdown_ast(&input.markdown)
}

/// Démarre ou lit un abonnement borné `SubscribeEvents` vers un `tauri::ipc::Channel`.
///
/// # Errors
/// Propage les erreurs de validation ou du client IPC commun.
#[tauri::command]
pub async fn gui_subscribe_events(
    state: tauri::State<'_, GuiAppState>,
    input: GuiSubscribeInput,
    channel: Channel<GuiStreamNotification>,
) -> Result<EventCursor, GuiError> {
    state.subscribe_events(input, channel).await
}

/// Commande Tauri générée pour l'opération de flux `SubscribeEvents`.
///
/// # Errors
/// Propage [`gui_subscribe_events`].
#[tauri::command]
pub async fn op_subscribe_events(
    state: tauri::State<'_, GuiAppState>,
    input: GuiSubscribeInput,
    channel: Channel<GuiStreamNotification>,
) -> Result<EventCursor, GuiError> {
    ensure_operation_kind(OperationName::SubscribeEvents, OperationKind::Stream)?;
    gui_subscribe_events(state, input, channel).await
}

macro_rules! define_query_command {
    ($fn_name:ident, $variant:ident) => {
        /// Commande Tauri typée générée pour l'opération de lecture `OperationName::$variant`.
        ///
        /// # Errors
        /// Retourne [`GuiError`] si le payload est invalide ou si le daemon rejette la lecture.
        #[tauri::command]
        pub async fn $fn_name(
            state: tauri::State<'_, GuiAppState>,
            input: GuiQueryInput,
        ) -> Result<QueryResponse, GuiError> {
            state.execute_query(OperationName::$variant, input).await
        }
    };
}

macro_rules! define_mutation_command {
    ($fn_name:ident, $variant:ident) => {
        /// Commande Tauri typée générée pour la commande mutante `OperationName::$variant`.
        ///
        /// # Errors
        /// Retourne [`GuiError`] si le payload est invalide ou si le daemon rejette la commande.
        #[tauri::command]
        pub async fn $fn_name(
            state: tauri::State<'_, GuiAppState>,
            input: GuiCommandInput,
        ) -> Result<CommandReceipt, GuiError> {
            state.submit_command(OperationName::$variant, input).await
        }
    };
}

define_query_command!(op_browse_resources, BrowseResources);
define_query_command!(op_compare_contexts, CompareContexts);
define_query_command!(op_compare_reports, CompareReports);
define_query_command!(op_discover_terminals, DiscoverTerminals);
define_query_command!(op_get_account_connection, GetAccountConnection);
define_query_command!(op_get_activity_snapshot, GetActivitySnapshot);
define_query_command!(op_get_agent, GetAgent);
define_query_command!(op_get_automation_data_revision, GetAutomationDataRevision);
define_query_command!(op_get_automation_draft, GetAutomationDraft);
define_query_command!(op_get_automation_evaluation, GetAutomationEvaluation);
define_query_command!(op_get_automation_version, GetAutomationVersion);
define_query_command!(op_get_backup_manifest, GetBackupManifest);
define_query_command!(op_get_binding, GetBinding);
define_query_command!(op_get_client_capabilities, GetClientCapabilities);
define_query_command!(op_get_command_receipt, GetCommandReceipt);
define_query_command!(op_get_connector_account, GetConnectorAccount);
define_query_command!(op_get_context_manifest, GetContextManifest);
define_query_command!(op_get_conversation, GetConversation);
define_query_command!(op_get_daemon_status, GetDaemonStatus);
define_query_command!(op_get_decision, GetDecision);
define_query_command!(op_get_dependency, GetDependency);
define_query_command!(op_get_discovery_receipt, GetDiscoveryReceipt);
define_query_command!(op_get_evaluation_report, GetEvaluationReport);
define_query_command!(op_get_goal, GetGoal);
define_query_command!(op_get_guided_setup_draft, GetGuidedSetupDraft);
define_query_command!(op_get_guided_setup_validation, GetGuidedSetupValidation);
define_query_command!(op_get_health, GetHealth);
define_query_command!(op_get_integration, GetIntegration);
define_query_command!(op_get_invocation, GetInvocation);
define_query_command!(op_get_invocation_manifest, GetInvocationManifest);
define_query_command!(op_get_message, GetMessage);
define_query_command!(op_get_mission, GetMission);
define_query_command!(op_get_operation, GetOperation);
define_query_command!(op_get_overview, GetOverview);
define_query_command!(op_get_process, GetProcess);
define_query_command!(op_get_project, GetProject);
define_query_command!(op_get_provider_profile, GetProviderProfile);
define_query_command!(op_get_report_snapshot, GetReportSnapshot);
define_query_command!(op_get_resource, GetResource);
define_query_command!(op_get_role_draft, GetRoleDraft);
define_query_command!(op_get_role_version, GetRoleVersion);
define_query_command!(op_get_run, GetRun);
define_query_command!(op_get_secret_reference_status, GetSecretReferenceStatus);
define_query_command!(op_get_setup_preview, GetSetupPreview);
define_query_command!(op_get_skill_draft, GetSkillDraft);
define_query_command!(op_get_skill_version, GetSkillVersion);
define_query_command!(op_get_snapshot, GetSnapshot);
define_query_command!(op_get_state_snapshot, GetStateSnapshot);
define_query_command!(op_get_version_diff, GetVersionDiff);
define_query_command!(op_get_workflow_draft, GetWorkflowDraft);
define_query_command!(op_get_workflow_simulation, GetWorkflowSimulation);
define_query_command!(op_get_workflow_validation, GetWorkflowValidation);
define_query_command!(op_get_workflow_version, GetWorkflowVersion);
define_query_command!(op_get_workspace, GetWorkspace);
define_query_command!(op_get_workspace_diff, GetWorkspaceDiff);
define_query_command!(op_get_worktree, GetWorktree);
define_query_command!(op_inspect_recovery, InspectRecovery);
define_query_command!(op_list_agents, ListAgents);
define_query_command!(op_list_alerts, ListAlerts);
define_query_command!(op_list_assignments, ListAssignments);
define_query_command!(op_list_connector_accounts, ListConnectorAccounts);
define_query_command!(op_list_conversations, ListConversations);
define_query_command!(op_list_decisions, ListDecisions);
define_query_command!(op_list_dependencies, ListDependencies);
define_query_command!(op_list_goals, ListGoals);
define_query_command!(op_list_guided_templates, ListGuidedTemplates);
define_query_command!(op_list_invocations, ListInvocations);
define_query_command!(op_list_missions, ListMissions);
define_query_command!(op_list_processes, ListProcesses);
define_query_command!(op_list_projects, ListProjects);
define_query_command!(op_list_provider_profiles, ListProviderProfiles);
define_query_command!(op_list_reports, ListReports);
define_query_command!(op_list_runs, ListRuns);
define_query_command!(op_list_version_usages, ListVersionUsages);
define_query_command!(op_list_workflows, ListWorkflows);
define_query_command!(op_list_workspaces, ListWorkspaces);
define_query_command!(op_list_worktrees, ListWorktrees);
define_query_command!(op_preview_resource, PreviewResource);
define_query_command!(op_read_artifact, ReadArtifact);
define_query_command!(op_read_evidence, ReadEvidence);
define_query_command!(op_read_guard_health, ReadGuardHealth);
define_query_command!(op_read_policy_summary, ReadPolicySummary);
define_query_command!(op_read_settings, ReadSettings);
define_query_command!(op_read_terminal_log, ReadTerminalLog);
define_query_command!(op_search_automations, SearchAutomations);
define_query_command!(op_search_conversations, SearchConversations);
define_query_command!(op_search_events, SearchEvents);
define_query_command!(op_search_knowledge, SearchKnowledge);
define_query_command!(op_search_resources, SearchResources);
define_query_command!(op_search_roles, SearchRoles);
define_query_command!(op_search_skills, SearchSkills);

define_mutation_command!(op_accept_step, AcceptStep);
define_mutation_command!(op_acquire_terminal, AcquireTerminal);
define_mutation_command!(op_apply_guided_setup, ApplyGuidedSetup);
define_mutation_command!(op_assign_document_scope, AssignDocumentScope);
define_mutation_command!(op_assign_mission, AssignMission);
define_mutation_command!(op_assign_resource, AssignResource);
define_mutation_command!(op_assign_task, AssignTask);
define_mutation_command!(op_attach_discovery_receipt, AttachDiscoveryReceipt);
define_mutation_command!(op_begin_secret_import, BeginSecretImport);
define_mutation_command!(op_bind_role, BindRole);
define_mutation_command!(op_cancel_account_connection, CancelAccountConnection);
define_mutation_command!(op_cancel_mission, CancelMission);
define_mutation_command!(op_cancel_run, CancelRun);
define_mutation_command!(op_configure_guard, ConfigureGuard);
define_mutation_command!(op_configure_watch, ConfigureWatch);
define_mutation_command!(op_connect_account, ConnectAccount);
define_mutation_command!(op_create_agent, CreateAgent);
define_mutation_command!(op_create_automation_draft, CreateAutomationDraft);
define_mutation_command!(op_create_goal, CreateGoal);
define_mutation_command!(op_create_guided_setup_draft, CreateGuidedSetupDraft);
define_mutation_command!(op_create_project, CreateProject);
define_mutation_command!(op_create_report_snapshot, CreateReportSnapshot);
define_mutation_command!(op_create_role_draft, CreateRoleDraft);
define_mutation_command!(op_create_run, CreateRun);
define_mutation_command!(op_create_skill_draft, CreateSkillDraft);
define_mutation_command!(op_create_workflow_draft, CreateWorkflowDraft);
define_mutation_command!(op_declare_work_intent, DeclareWorkIntent);
define_mutation_command!(op_deprecate_automation_version, DeprecateAutomationVersion);
define_mutation_command!(op_deprecate_role_version, DeprecateRoleVersion);
define_mutation_command!(op_deprecate_skill_version, DeprecateSkillVersion);
define_mutation_command!(op_disconnect_account, DisconnectAccount);
define_mutation_command!(op_discover_resources, DiscoverResources);
define_mutation_command!(op_draft_workflow_from_intent, DraftWorkflowFromIntent);
define_mutation_command!(op_evaluate_automation_draft, EvaluateAutomationDraft);
define_mutation_command!(op_export_bundle, ExportBundle);
define_mutation_command!(op_export_report, ExportReport);
define_mutation_command!(op_import_automation_data, ImportAutomationData);
define_mutation_command!(op_learn_automation, LearnAutomation);
define_mutation_command!(op_normalize_workflow_draft, NormalizeWorkflowDraft);
define_mutation_command!(op_pause_mission, PauseMission);
define_mutation_command!(op_pause_run, PauseRun);
define_mutation_command!(op_prepare_integration, PrepareIntegration);
define_mutation_command!(op_publish_automation_version, PublishAutomationVersion);
define_mutation_command!(op_publish_integration, PublishIntegration);
define_mutation_command!(op_publish_role_version, PublishRoleVersion);
define_mutation_command!(op_publish_skill_version, PublishSkillVersion);
define_mutation_command!(op_publish_workflow_version, PublishWorkflowVersion);
define_mutation_command!(op_purge_history, PurgeHistory);
define_mutation_command!(op_rebuild_index, RebuildIndex);
define_mutation_command!(op_reconcile_action, ReconcileAction);
define_mutation_command!(op_reconcile_process, ReconcileProcess);
define_mutation_command!(
    op_record_declassification_decision,
    RecordDeclassificationDecision
);
define_mutation_command!(op_record_discovery_decision, RecordDiscoveryDecision);
define_mutation_command!(op_record_goal_decision, RecordGoalDecision);
define_mutation_command!(op_record_mission_decision, RecordMissionDecision);
define_mutation_command!(op_refresh_resource, RefreshResource);
define_mutation_command!(op_release_terminal, ReleaseTerminal);
define_mutation_command!(op_remove_assignment, RemoveAssignment);
define_mutation_command!(op_repair_automation, RepairAutomation);
define_mutation_command!(op_request_agent_reassignment, RequestAgentReassignment);
define_mutation_command!(op_request_backup, RequestBackup);
define_mutation_command!(op_request_context_compaction, RequestContextCompaction);
define_mutation_command!(op_request_correction, RequestCorrection);
define_mutation_command!(op_request_dependency, RequestDependency);
define_mutation_command!(op_request_interruption, RequestInterruption);
define_mutation_command!(op_request_skill_evaluation, RequestSkillEvaluation);
define_mutation_command!(op_resize_terminal, ResizeTerminal);
define_mutation_command!(op_resolve_action_approval, ResolveActionApproval);
define_mutation_command!(op_resolve_action_outcome, ResolveActionOutcome);
define_mutation_command!(op_restore_backup, RestoreBackup);
define_mutation_command!(op_resume_mission, ResumeMission);
define_mutation_command!(op_resume_recovered_invocation, ResumeRecoveredInvocation);
define_mutation_command!(op_resume_run, ResumeRun);
define_mutation_command!(op_resume_subject, ResumeSubject);
define_mutation_command!(op_retire_worktree, RetireWorktree);
define_mutation_command!(op_revise_goal, ReviseGoal);
define_mutation_command!(op_revise_mission, ReviseMission);
define_mutation_command!(op_revise_project, ReviseProject);
define_mutation_command!(op_revoke_automation_version, RevokeAutomationVersion);
define_mutation_command!(op_revoke_role_version, RevokeRoleVersion);
define_mutation_command!(op_revoke_secret_reference, RevokeSecretReference);
define_mutation_command!(op_revoke_skill_version, RevokeSkillVersion);
define_mutation_command!(op_revoke_subject, RevokeSubject);
define_mutation_command!(op_save_automation_draft, SaveAutomationDraft);
define_mutation_command!(op_save_guided_setup_draft, SaveGuidedSetupDraft);
define_mutation_command!(op_save_role_draft, SaveRoleDraft);
define_mutation_command!(op_save_skill_draft, SaveSkillDraft);
define_mutation_command!(op_save_workflow_draft, SaveWorkflowDraft);
define_mutation_command!(op_send_human_message, SendHumanMessage);
define_mutation_command!(op_send_terminal_input, SendTerminalInput);
define_mutation_command!(op_set_policy, SetPolicy);
define_mutation_command!(op_set_provider_profile, SetProviderProfile);
define_mutation_command!(op_set_secret_reference, SetSecretReference);
define_mutation_command!(op_set_settings, SetSettings);
define_mutation_command!(op_simulate_workflow, SimulateWorkflow);
define_mutation_command!(op_start_run, StartRun);
define_mutation_command!(op_stop_daemon, StopDaemon);
define_mutation_command!(op_stop_process, StopProcess);
define_mutation_command!(op_suspend_subject, SuspendSubject);
define_mutation_command!(op_test_connector_account, TestConnectorAccount);
define_mutation_command!(op_test_provider_profile, TestProviderProfile);
define_mutation_command!(op_validate_automation_data, ValidateAutomationData);
define_mutation_command!(op_validate_automation_draft, ValidateAutomationDraft);
define_mutation_command!(op_validate_backup, ValidateBackup);
define_mutation_command!(op_validate_guided_setup, ValidateGuidedSetup);
define_mutation_command!(op_validate_role, ValidateRole);
define_mutation_command!(op_validate_skill, ValidateSkill);
define_mutation_command!(op_validate_workflow_draft, ValidateWorkflowDraft);

/// Enregistre le routeur exhaustif des 203 commandes Tauri autorisées sur un `Builder`.
#[expect(
    clippy::too_many_lines,
    reason = "Table exhaustive 1-à-1 des 200 opérations exposées du catalogue plus 3 commandes hôte"
)]
pub(crate) fn install_invoke_handler<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        gui_list_exposed_operations,
        gui_render_markdown_ast,
        gui_subscribe_events,
        op_accept_step,
        op_acquire_terminal,
        op_apply_guided_setup,
        op_assign_document_scope,
        op_assign_mission,
        op_assign_resource,
        op_assign_task,
        op_attach_discovery_receipt,
        op_begin_secret_import,
        op_bind_role,
        op_browse_resources,
        op_cancel_account_connection,
        op_cancel_mission,
        op_cancel_run,
        op_compare_contexts,
        op_compare_reports,
        op_configure_guard,
        op_configure_watch,
        op_connect_account,
        op_create_agent,
        op_create_automation_draft,
        op_create_goal,
        op_create_guided_setup_draft,
        op_create_project,
        op_create_report_snapshot,
        op_create_role_draft,
        op_create_run,
        op_create_skill_draft,
        op_create_workflow_draft,
        op_declare_work_intent,
        op_deprecate_automation_version,
        op_deprecate_role_version,
        op_deprecate_skill_version,
        op_disconnect_account,
        op_discover_resources,
        op_discover_terminals,
        op_draft_workflow_from_intent,
        op_evaluate_automation_draft,
        op_export_bundle,
        op_export_report,
        op_get_account_connection,
        op_get_activity_snapshot,
        op_get_agent,
        op_get_automation_data_revision,
        op_get_automation_draft,
        op_get_automation_evaluation,
        op_get_automation_version,
        op_get_backup_manifest,
        op_get_binding,
        op_get_client_capabilities,
        op_get_command_receipt,
        op_get_connector_account,
        op_get_context_manifest,
        op_get_conversation,
        op_get_daemon_status,
        op_get_decision,
        op_get_dependency,
        op_get_discovery_receipt,
        op_get_evaluation_report,
        op_get_goal,
        op_get_guided_setup_draft,
        op_get_guided_setup_validation,
        op_get_health,
        op_get_integration,
        op_get_invocation,
        op_get_invocation_manifest,
        op_get_message,
        op_get_mission,
        op_get_operation,
        op_get_overview,
        op_get_process,
        op_get_project,
        op_get_provider_profile,
        op_get_report_snapshot,
        op_get_resource,
        op_get_role_draft,
        op_get_role_version,
        op_get_run,
        op_get_secret_reference_status,
        op_get_setup_preview,
        op_get_skill_draft,
        op_get_skill_version,
        op_get_snapshot,
        op_get_state_snapshot,
        op_get_version_diff,
        op_get_workflow_draft,
        op_get_workflow_simulation,
        op_get_workflow_validation,
        op_get_workflow_version,
        op_get_workspace,
        op_get_workspace_diff,
        op_get_worktree,
        op_import_automation_data,
        op_inspect_recovery,
        op_learn_automation,
        op_list_agents,
        op_list_alerts,
        op_list_assignments,
        op_list_connector_accounts,
        op_list_conversations,
        op_list_decisions,
        op_list_dependencies,
        op_list_goals,
        op_list_guided_templates,
        op_list_invocations,
        op_list_missions,
        op_list_processes,
        op_list_projects,
        op_list_provider_profiles,
        op_list_reports,
        op_list_runs,
        op_list_version_usages,
        op_list_workflows,
        op_list_workspaces,
        op_list_worktrees,
        op_normalize_workflow_draft,
        op_pause_mission,
        op_pause_run,
        op_prepare_integration,
        op_preview_resource,
        op_publish_automation_version,
        op_publish_integration,
        op_publish_role_version,
        op_publish_skill_version,
        op_publish_workflow_version,
        op_purge_history,
        op_read_artifact,
        op_read_evidence,
        op_read_guard_health,
        op_read_policy_summary,
        op_read_settings,
        op_read_terminal_log,
        op_rebuild_index,
        op_reconcile_action,
        op_reconcile_process,
        op_record_declassification_decision,
        op_record_discovery_decision,
        op_record_goal_decision,
        op_record_mission_decision,
        op_refresh_resource,
        op_release_terminal,
        op_remove_assignment,
        op_repair_automation,
        op_request_agent_reassignment,
        op_request_backup,
        op_request_context_compaction,
        op_request_correction,
        op_request_dependency,
        op_request_interruption,
        op_request_skill_evaluation,
        op_resize_terminal,
        op_resolve_action_approval,
        op_resolve_action_outcome,
        op_restore_backup,
        op_resume_mission,
        op_resume_recovered_invocation,
        op_resume_run,
        op_resume_subject,
        op_retire_worktree,
        op_revise_goal,
        op_revise_mission,
        op_revise_project,
        op_revoke_automation_version,
        op_revoke_role_version,
        op_revoke_secret_reference,
        op_revoke_skill_version,
        op_revoke_subject,
        op_save_automation_draft,
        op_save_guided_setup_draft,
        op_save_role_draft,
        op_save_skill_draft,
        op_save_workflow_draft,
        op_search_automations,
        op_search_conversations,
        op_search_events,
        op_search_knowledge,
        op_search_resources,
        op_search_roles,
        op_search_skills,
        op_send_human_message,
        op_send_terminal_input,
        op_set_policy,
        op_set_provider_profile,
        op_set_secret_reference,
        op_set_settings,
        op_simulate_workflow,
        op_start_run,
        op_stop_daemon,
        op_stop_process,
        op_subscribe_events,
        op_suspend_subject,
        op_test_connector_account,
        op_test_provider_profile,
        op_validate_automation_data,
        op_validate_automation_draft,
        op_validate_backup,
        op_validate_guided_setup,
        op_validate_role,
        op_validate_skill,
        op_validate_workflow_draft
    ])
}
