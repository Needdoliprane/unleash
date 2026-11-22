//! Algorithme transactionnel de commande (`CommitCommand`), reçus, tombstones et purge (`REQ-PERSIST-001`, `REQ-PERSIST-004`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unleash_domain::{
    ActionId, ActionState, ActorId, AgentId, AgentLevel, AgentState, AttemptId,
    AuthorityGeneration, BackupId, CommandId, CompletionPolicy, DaemonEpoch, DaemonMode, Fence,
    GoalId, GoalState, InvocationId, InvocationState, MissionId, MissionState, OperationId,
    ProjectId, ReceiptId, ReconcileOutcome, ReplayWorkerId, Revision, RunId, Sha256Digest, StepId,
    StreamId, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, CommandReceiptInspection, EventCausationKind, ReceiptState,
    Sensitivity,
};
use uuid::Uuid;

use crate::budgets::{BudgetReservationRequest, CapacityReservationRequest};
use crate::events::{RetentionPurgeRequest, tx_execute_retention_purge};
use crate::leases::FenceAssertion;
use crate::policies_secrets::{
    AlertState, CallerAuthority, GuardConfigInput, PolicyIsolationMode, PolicyScopeKind,
    SecretReferenceInput, SubjectKind, SubjectStatus, ensure_subject_not_blocked_tx,
    tx_configure_guard, tx_revoke_secret_reference, tx_set_policy, tx_set_secret_reference,
    tx_transition_alert, tx_transition_subject,
};
use crate::projects_settings::{
    ProjectRepositorySpec, ProjectSourceSpec, SettingsPatch, SettingsScopeKind, tx_revise_project,
    tx_set_settings,
};
use crate::recovery_ops::{
    ActionOutcomeDisposition, ApprovalDecision, tx_reconcile_action, tx_resolve_action_approval,
    tx_resolve_action_outcome,
};

/// Nature d'agrégat vérifié lors d'une commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateKind {
    /// État global du daemon (`daemon_state`).
    Daemon,
    /// Projet (`projects`).
    Project,
    /// Objectif (`goals`).
    Goal,
    /// Mission (`missions`).
    Mission,
    /// Exécution de workflow (`workflow_runs`).
    WorkflowRun,
    /// Tâche durable (`tasks`).
    Task,
    /// Agent logique (`agents`).
    Agent,
    /// Invocation durable (`invocations`).
    Invocation,
    /// Action broker (`actions`).
    Action,
    /// Entité générique persistée dans une table clé/révision dédiée ou `workflow_runs`/`missions`.
    GenericRun,
    /// Agrégat de paramètres (`settings`).
    Settings,
    /// Politique de sécurité (`security_policies`).
    SecurityPolicy,
    /// Configuration AI-Guard (`guard_configs`).
    GuardConfig,
    /// Suspension ou révocation d'un sujet (`subject_suspensions`).
    SubjectSuspension,
    /// Référence secrète opaque (`secret_references`).
    SecretReference,
    /// Alerte opérationnelle (`alerts`).
    Alert,
    /// Décision humaine d'approbation (`approval_resolutions`).
    ApprovalResolution,
    /// Audit de purge Q-12 (`retention_purges`).
    RetentionPurge,
    /// Demande et job de sauvegarde (`backup_requests`, `backup_jobs`).
    BackupJob,
    /// Préparation de probe de génération (`provider_probes`).
    ProviderProbe,
    /// Version immuable d'un profil fournisseur configuré.
    ProviderProfile,
    /// Contexte de préparation explicite d'un service fournisseur.
    ProviderServiceLaunch,
    /// Préparation d'allocation terminal humaine.
    TerminalAllocation,
    /// Contrôle courant d'un terminal enregistré, distinct de son lancement.
    TerminalControl,
    /// Intention de transfert, distincte de la révision effective du terminal.
    TerminalControlIntent,
    /// Durable input intention, independent of the controller revision.
    TerminalInputIntent,
}

impl AggregateKind {
    /// Nom canonique stocké dans `events.aggregate_type`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::Project => "project",
            Self::Goal => "goal",
            Self::Mission => "mission",
            Self::WorkflowRun | Self::GenericRun => "workflow_run",
            Self::Task => "task",
            Self::Agent => "agent",
            Self::Invocation => "invocation",
            Self::Action => "action",
            Self::Settings => "settings",
            Self::SecurityPolicy => "security_policy",
            Self::GuardConfig => "guard_config",
            Self::SubjectSuspension => "subject_suspension",
            Self::SecretReference => "secret_reference",
            Self::Alert => "alert",
            Self::ApprovalResolution => "approval_resolution",
            Self::RetentionPurge => "retention_purge",
            Self::BackupJob => "backup_job",
            Self::ProviderProbe => "provider_probe",
            Self::ProviderProfile => "provider_profile",
            Self::ProviderServiceLaunch => "provider_service_launch",
            Self::TerminalAllocation => "terminal_allocation",
            Self::TerminalControl => "terminal_control",
            Self::TerminalControlIntent => "terminal_control_intent",
            Self::TerminalInputIntent => "terminal_input_intent",
        }
    }
}

/// Mutation pure décidée par le domaine et exécutée atomiquement par [`CommitCommandRequest`].
#[derive(Debug, Clone)]
pub enum CommandMutation {
    /// Seal or reference historical Release evidence without changing the original action.
    AttestHistoricalTerminalRelease {
        /// Reconciliation authority resolved by the authenticated daemon.
        caller: CallerAuthority,
        /// Optional prepared proof from an exact native inspection.
        proof: Option<crate::PreparedObjectHandle>,
    },
    /// Prepare explicit native control retirement without stopping the resource.
    PrepareTerminalRelease {
        /// Transport-authenticated human authority.
        caller: CallerAuthority,
        /// Exact registered controller target, with no client-supplied evidence.
        request: crate::terminal_allocations::terminal_release_types::TerminalReleaseRequest,
    },
    /// Accept a resize for the currently held terminal lease without contacting its host.
    PrepareTerminalResize {
        /// Actor and role of the authenticated human controller.
        caller: CallerAuthority,
        /// Requested dimensions and exact held control.
        request: crate::terminal_allocations::terminal_renewal_types::TerminalResizeRequest,
    },
    /// Accept an exact input intention without retaining its raw bytes or performing I/O.
    PrepareTerminalInput {
        /// Identity and role resolved by the authenticated daemon connection.
        caller: CallerAuthority,
        /// Exact closed payload, distinct from any observed native receipt.
        request: crate::terminal_allocations::terminal_input_types::TerminalInputRequest,
    },
    /// Prépare un transfert de contrôle sans contact host ni nouvelle lease held.
    PrepareTerminalControl {
        /// Autorité humaine dérivée de la session.
        caller: CallerAuthority,
        /// Les quatre champs fermés de la cible enregistrée.
        request: crate::terminal_allocations::RegisteredTerminalControl,
    },
    /// Enregistre une version de configuration sans qualifier le fournisseur.
    SetProviderProfile {
        /// Autorité humaine dérivée de la session authentifiée.
        caller: CallerAuthority,
        /// Configuration et provenance déjà validées côté service.
        preparation: Box<crate::ProviderProfilePreparation>,
    },
    /// Engage une observation de configuration locale, sans admission ni effet fournisseur.
    ValidateProviderProfile {
        /// Autorité administrative dérivée de la session authentifiée.
        caller: CallerAuthority,
        /// Observation réelle et objet publié avant la transaction.
        preparation: Box<crate::ProviderValidationPreparation>,
    },
    /// Prépare un contexte réseau authentifié sans action, `ModelCall` ou réservation.
    PrepareProviderProbeConnect {
        /// Autorité issue de la session, sans délégation d'un payload client.
        caller: CallerAuthority,
        /// Résolution exacte, relue dans la transaction du nouveau reçu.
        preparation: Box<crate::ProviderProbeConnectPreparation>,
    },
    /// Prépare l'inventaire passif authentifié, sans action, réseau ou réservation.
    PrepareProviderProbeInventory {
        /// Autorité de la session réelle.
        caller: CallerAuthority,
        /// Résolution exacte de la phase inventory, relue par le writer.
        preparation: Box<crate::ProviderProbeInventoryPreparation>,
    },
    /// Prepare the effect context for an explicit managed service launch.
    PrepareProviderServiceLaunch {
        /// Human security authority resolved by the daemon.
        caller: CallerAuthority,
    },
    /// Prépare un terminal sans action OS ni action broker préinsérée.
    PrepareTerminalAllocation {
        /// Identité égale à l'agrégat de commande.
        terminal_id: unleash_domain::TerminalId,
        /// Autorité authentifiée, pas un champ du payload.
        caller: CallerAuthority,
        /// Résolution serveur exacte.
        preparation: crate::TerminalAllocationPreparation,
    },
    /// Prépare un probe authentifié, sans action, réseau ou réservation de capacité.
    PrepareProviderProbe {
        /// Identité stable égale à l'agrégat de commande.
        probe_id: unleash_domain::ProviderProbeId,
        /// Autorité dérivée de la connexion, jamais du payload client.
        caller: CallerAuthority,
        /// Références et bornes résolues côté service.
        preparation: crate::ProviderProbePreparation,
    },
    /// Commits a configured generation probe after resolving its active mandate.
    PrepareAuthenticatedProviderGeneration {
        /// Authority derived from the authenticated session.
        caller: CallerAuthority,
        /// Resolved, immutable, verified context and its exact artifact references.
        preparation: Box<crate::AuthenticatedGenerationPreparation>,
    },
    /// Réserve une sauvegarde manuelle gérée, sans I/O filesystem.
    RequestBackup {
        /// Identité client stable égale à l'agrégat de l'enveloppe.
        backup_id: BackupId,
    },
    /// Crée un nouveau projet.
    CreateProject {
        /// Identifiant du projet.
        project_id: ProjectId,
        /// Nom du projet.
        name: String,
        /// Poids d'équité (1..=10).
        weight: i64,
    },
    /// Crée un nouvel agent logique et sa session primaire.
    CreateAgent {
        /// Identifiant de l'agent.
        agent_id: AgentId,
        /// Niveau hiérarchique (`N`, `N+1`, `N+2`).
        level: AgentLevel,
        /// Parent organisationnel optionnel.
        organization_parent_id: Option<AgentId>,
        /// Créateur immuable optionnel.
        created_by_agent_id: Option<AgentId>,
        /// Projet rattaché optionnel.
        project_id: Option<ProjectId>,
        /// État initial.
        state: AgentState,
    },
    /// Crée l'agent, sa session primaire et l'affectation résolue par le harness.
    CreateAssignedAgent {
        /// Autorité humaine du propriétaire de la mission.
        caller: CallerAuthority,
    },
    /// Create an assigned agent from a sealed workflow task binding.
    CreateAgentFromWorkflowTask(Box<crate::AgentWorkflowPreparation>),
    /// Crée un objectif.
    CreateGoal {
        /// Identifiant de l'objectif.
        goal_id: GoalId,
        /// Projet propriétaire.
        project_id: ProjectId,
        /// Propriétaire humain (`principal_id`) ou agent (`agent_id`).
        owner_principal_id: Option<String>,
        /// Agent propriétaire N+2.
        owner_agent_id: Option<AgentId>,
        /// Politique de complétion.
        completion_policy: CompletionPolicy,
        /// Référence de définition.
        definition_ref: String,
    },
    /// Crée une mission.
    CreateMission {
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Objectif parent optionnel.
        goal_id: Option<GoalId>,
        /// Projet propriétaire.
        project_id: ProjectId,
        /// Propriétaire utilisateur optionnel.
        owner_principal_id: Option<String>,
        /// Agent propriétaire optionnel.
        owner_agent_id: Option<AgentId>,
        /// Politique de complétion.
        completion_policy: CompletionPolicy,
        /// État initial (`proposed` ou `ready`).
        state: MissionState,
    },
    /// Crée une mission et retient le manifeste de sa portée initiale.
    CreateScopedMission(Box<crate::PreparedMissionScope>),
    /// Met à jour l'état d'une mission (ex: `PauseMission`, `ResumeMission`, `CancelMission`).
    TransitionMission {
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Nouvel état cible.
        target_state: MissionState,
    },
    /// Crée un `WorkflowRun` (avec contrôle `predecessor_run_id` terminal et unicité de successeur).
    CreateWorkflowRun {
        /// Identifiant du run.
        run_id: RunId,
        /// Identifiant de mission.
        mission_id: MissionId,
        /// Version de workflow épinglée.
        workflow_version_id: String,
        /// Finalité (`operational` ou `evaluation`).
        purpose: String,
        /// Candidat d'évaluation éventuel.
        candidate_ref: Option<String>,
        /// Prédécesseur optionnel.
        predecessor_run_id: Option<RunId>,
        /// Motif de sélection.
        selection_reason: String,
        /// Hash du bundle effectif.
        effective_bundle_hash: Sha256Digest,
    },
    /// Prépare le run et publie son plan effectif dans la transaction du reçu.
    CreatePreparedWorkflowRun(Box<crate::PreparedRunPublication>),
    /// Start a prepared workflow and commit its first activation and deadlines.
    StartPreparedWorkflowRun(Box<crate::PreparedRunStart>),
    /// Fait transiter un `WorkflowRun` (`StartRun`, `PauseRun`, `ResumeRun`, `CancelRun`, etc.).
    TransitionWorkflowRun {
        /// Identifiant du run.
        run_id: RunId,
        /// Nouvel état cible (après validation pure).
        target_state: unleash_domain::RunState,
        /// Horodatage de fin si terminal.
        ended_at_ms: Option<TimestampMs>,
    },
    /// Matérialise une étape et sa tâche depuis la décision du reducer.
    ActivateWorkflowTask,
    /// Affecte une tâche prête avec son plan interne et la met en file.
    AssignTask {
        /// Autorité authentifiée par le serveur.
        caller: crate::CallerAuthority,
        /// Affectation et bundle résolus par le harness.
        preparation: crate::PreparedTaskAssignment,
    },
    /// Enregistre une nouvelle action durable préparée.
    CreateAction {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Origine (`invocation`, `replay` ou `control_plane`).
        origin_kind: String,
        /// Principal de contrôle si `control_plane`.
        control_principal_id: Option<String>,
        /// Run éventuel.
        run_id: Option<RunId>,
        /// Step éventuel.
        step_id: Option<StepId>,
        /// Invocation éventuelle.
        invocation_id: Option<InvocationId>,
        /// Tentative éventuelle.
        attempt_id: Option<AttemptId>,
        /// Worker de replay éventuel.
        replay_worker_id: Option<ReplayWorkerId>,
        /// Type d'action.
        action_type: String,
        /// Hash immuable du payload.
        payload_hash: Sha256Digest,
        /// Stratégie de réconciliation.
        reconcile_strategy: String,
        /// Révision de politique.
        policy_revision: i64,
        /// Lease optionnelle associée.
        resource_lease_id: Option<String>,
    },
    /// Met à jour le mode du daemon (`StopDaemon`, etc.).
    TransitionDaemonMode {
        /// Mode cible.
        target_mode: DaemonMode,
    },
    /// Met à jour l'état d'un objectif.
    TransitionGoal {
        /// Identifiant d'objectif.
        goal_id: GoalId,
        /// État cible.
        target_state: GoalState,
    },
    /// Met à jour l'état d'une invocation.
    TransitionInvocation {
        /// Identifiant d'invocation.
        invocation_id: InvocationId,
        /// État cible.
        target_state: InvocationState,
        /// Raison d'attente optionnelle.
        wait_reason: Option<unleash_domain::WaitReason>,
    },
    /// Mutation neutre sur `daemon_state` (pour tests de bout en bout / charge WAL / receipts).
    NoopDaemonBump,
    /// Révise un projet existant (`ReviseProject` : nom, description, poids, racines/dépôts validés).
    ReviseProject {
        /// Identifiant du projet ciblé.
        project_id: ProjectId,
        /// Nouveau nom.
        name: String,
        /// Description mise à jour.
        description: String,
        /// Nouveau poids d'ordonnancement (1..=10).
        weight: i64,
        /// Nouvelle racine locale optionnelle (validée).
        primary_root_path: Option<String>,
        /// Liste mise à jour des sources.
        sources: Vec<ProjectSourceSpec>,
        /// Liste mise à jour des dépôts.
        repositories: Vec<ProjectRepositorySpec>,
        /// Paramètres JSON typés du projet.
        settings_json: Value,
    },
    /// Met à jour les paramètres typés (`SetSettings` : Q-15 pool d'exécution + rétention Q-12).
    SetSettings {
        /// Portée des paramètres modifiés.
        scope_kind: SettingsScopeKind,
        /// Identifiant de portée.
        scope_id: String,
        /// Autorité authentifiée.
        caller: CallerAuthority,
        /// Patch typé des paramètres.
        patch: SettingsPatch,
    },
    /// Met à jour les réglages et une préférence versionnée dans le même commit.
    SetSettingsWithModelSelectionBinding {
        /// Portée des paramètres modifiés.
        scope_kind: SettingsScopeKind,
        /// Identifiant de portée.
        scope_id: String,
        /// Autorité authentifiée.
        caller: CallerAuthority,
        /// Autres réglages de la même modification.
        patch: SettingsPatch,
        /// Changement de préférence pour la référence exacte du skill.
        binding: Box<crate::ModelSelectionBindingChange>,
    },
    /// Met à jour la politique d'isolation (`SetPolicy` : `standard` ou `strict`).
    SetPolicy {
        /// Portée de politique.
        scope_kind: PolicyScopeKind,
        /// Identifiant de portée (`"global"` ou `project_id`).
        scope_id: String,
        /// Mode d'isolation.
        isolation_mode: PolicyIsolationMode,
        /// Version de politique.
        policy_version: String,
        /// Motif tracé.
        reason: String,
        /// Autorité appelante.
        caller: CallerAuthority,
    },
    /// Voie `SetPolicy` avec émission ou révocation de permission fournisseur.
    SetPolicyWithProviderProbeMandate {
        /// Portée de politique authentifiée.
        scope_kind: PolicyScopeKind,
        /// Identité de portée canonique.
        scope_id: String,
        /// Mode d'isolation demandé.
        isolation_mode: PolicyIsolationMode,
        /// Version logique de politique.
        policy_version: String,
        /// Motif de la commande administrative.
        reason: String,
        /// Autorité issue de la session.
        caller: CallerAuthority,
        /// Permission fermée, sans exécution ni observation implicite.
        mandate: crate::ProviderProbeMandateChange,
    },
    /// Configure l'intégration AI-Guard (`ConfigureGuard`).
    ConfigureGuard {
        /// Configuration typée AI-Guard.
        config: GuardConfigInput,
        /// Autorité appelante.
        caller: CallerAuthority,
    },
    /// Suspend localement un sujet typé (`SuspendSubject`).
    SuspendSubject {
        /// Type de sujet.
        subject_kind: SubjectKind,
        /// Identifiant de sujet.
        subject_id: String,
        /// Portée (`global`, `project`, ...).
        scope_kind: String,
        /// Identifiant de portée.
        scope_id: String,
        /// Motif explicite.
        reason: String,
        /// Autorité appelante.
        caller: CallerAuthority,
        /// Indique si `GuardPort` a négocié et propagé la suspension.
        guard_propagated: bool,
    },
    /// Rétablit localement un sujet suspendu (`ResumeSubject`, refusé si révoqué).
    ResumeSubject {
        /// Type de sujet.
        subject_kind: SubjectKind,
        /// Identifiant de sujet.
        subject_id: String,
        /// Portée.
        scope_kind: String,
        /// Identifiant de portée.
        scope_id: String,
        /// Motif explicite.
        reason: String,
        /// Autorité appelante.
        caller: CallerAuthority,
    },
    /// Révoque définitivement un sujet typé (`RevokeSubject`).
    RevokeSubject {
        /// Type de sujet.
        subject_kind: SubjectKind,
        /// Identifiant de sujet.
        subject_id: String,
        /// Portée.
        scope_kind: String,
        /// Identifiant de portée.
        scope_id: String,
        /// Motif explicite.
        reason: String,
        /// Autorité appelante.
        caller: CallerAuthority,
        /// Indique si `GuardPort` a propagé la révocation.
        guard_propagated: bool,
    },
    /// Lie ou met à jour une référence secrète opaque (`SetSecretReference`, zéro octet secret).
    SetSecretReference {
        /// Métadonnées et `SecretRef` opaque.
        input: SecretReferenceInput,
        /// Autorité appelante.
        caller: CallerAuthority,
    },
    /// Révoque une référence secrète (`RevokeSecretReference`, local vs distant).
    RevokeSecretReference {
        /// Identifiant de liaison secrète.
        secret_ref_id: String,
        /// Motif explicite.
        reason: String,
        /// Autorité appelante.
        caller: CallerAuthority,
        /// Le fournisseur supporte la révocation distante.
        remote_revocation_supported: bool,
        /// La révocation distante est déjà confirmée.
        remote_revocation_confirmed: bool,
    },
    /// Crée une action durable gouvernée avec scope, approbation et contrôle de sujet.
    CreateGovernedAction {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Origine (`invocation`, `replay` ou `control_plane`).
        origin_kind: String,
        /// Principal de contrôle si `control_plane`.
        control_principal_id: Option<String>,
        /// Projet associé optionnel.
        project_id: Option<ProjectId>,
        /// Clé de scope concernée.
        scope_key: Option<String>,
        /// Run éventuel.
        run_id: Option<RunId>,
        /// Step éventuel.
        step_id: Option<StepId>,
        /// Invocation éventuelle.
        invocation_id: Option<InvocationId>,
        /// Tentative éventuelle.
        attempt_id: Option<AttemptId>,
        /// Worker de replay éventuel.
        replay_worker_id: Option<ReplayWorkerId>,
        /// Type d'action.
        action_type: String,
        /// Hash immuable du payload.
        payload_hash: Sha256Digest,
        /// Stratégie de réconciliation.
        reconcile_strategy: String,
        /// Révision de politique.
        policy_revision: i64,
        /// Lease optionnelle associée.
        resource_lease_id: Option<String>,
        /// Scope d'approbation éventuel pour `approval_required`.
        approval_scope: Option<String>,
        /// État initial durable.
        initial_state: ActionState,
        /// Sujet éventuel contrôlé contre `subject_suspensions`.
        subject: Option<(SubjectKind, String)>,
    },
    /// Applique une observation vérifiée sur une action (`ReconcileAction`, F018).
    ReconcileAction {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Autorité appelante habilitée.
        caller: CallerAuthority,
        /// Issue observée.
        outcome: ReconcileOutcome,
        /// Digest de preuve éventuelle (obligatoire si `ConfirmedSuccess` sans `receipt_hash`).
        evidence_digest: Option<Sha256Digest>,
        /// Empreinte de reçu transport éventuelle.
        receipt_hash: Option<Sha256Digest>,
        /// Motif explicite.
        reason: String,
        /// Époque source attendue pour rejeter un callback tardif (`StaleFence`).
        expected_source_epoch: Option<DaemonEpoch>,
        /// Fence source attendu pour rejeter un callback tardif (`StaleFence`).
        expected_source_fence: Option<Fence>,
    },
    /// Enregistre une disposition humaine sur une action `outcome_unknown` (`ResolveActionOutcome`).
    ResolveActionOutcome {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Décideur humain/policy habilité.
        caller: CallerAuthority,
        /// Disposition choisie (`actions.state` reste `outcome_unknown`).
        disposition: ActionOutcomeDisposition,
        /// Motif et limites explicites.
        reason: String,
        /// Référence de preuve éventuelle.
        evidence_ref: Option<String>,
    },
    /// Enregistre la décision humaine d'approbation sur `approval_scope` et `payload_digest` exacts.
    ResolveActionApproval {
        /// Identifiant d'action.
        action_id: ActionId,
        /// Décideur humain/policy habilité.
        caller: CallerAuthority,
        /// Scope d'approbation exact attendu.
        approval_scope: String,
        /// Digest exact attendu.
        payload_digest: Sha256Digest,
        /// Décision (`approve` / `deny`).
        decision: ApprovalDecision,
        /// Motif explicite.
        reason: String,
    },
    /// Fait transiter l'état d'une alerte (`acknowledged` / `resolved`).
    AcknowledgeAlert {
        /// Identifiant d'alerte.
        alert_id: String,
        /// Nouvel état cible.
        target_state: AlertState,
    },
    /// Exécute une purge sous politique de rétention Q-12 (`PurgeHistory`).
    PurgeHistory {
        /// Paramètres et portée de la purge.
        request: RetentionPurgeRequest,
    },
}

/// Événement à journaliser dans la même transaction que la mutation.
#[derive(Debug, Clone)]
pub struct EventAppendSpec {
    /// Type d'événement `PascalCase`.
    pub event_type: String,
    /// Agrégat modifié.
    pub aggregate_type: AggregateKind,
    /// Identifiant de l'agrégat modifié.
    pub aggregate_id: Uuid,
    /// Révision résultante (>= 1).
    pub aggregate_revision: Revision,
    /// Projet associé.
    pub project_id: Option<ProjectId>,
    /// Identifiant de corrélation.
    pub correlation_id: Uuid,
    /// Cause (par défaut `Command`, ce qui lie `causation_receipt_id` au reçu créé).
    pub causation_kind: EventCausationKind,
    /// ID de cause si différent de `Command`.
    pub causation_id: Option<String>,
    /// Payload JSON borné.
    pub payload: serde_json::Map<String, Value>,
    /// Sensibilité.
    pub sensitivity: Sensitivity,
    /// Consommateurs internes à notifier via `outbox`.
    pub outbox_destinations: Vec<String>,
}

/// Requête complète soumise à `CommitCommand`.
#[derive(Debug, Clone)]
pub struct CommitCommandRequest {
    /// Acteur authentifié côté serveur (clé d'idempotence avec `command_id`).
    pub actor_id: ActorId,
    /// Enveloppe v2 validée.
    pub envelope: CommandEnvelope,
    /// Agrégat ciblé pour vérification d'`expected_revision`.
    pub target_aggregate: AggregateKind,
    /// Opération asynchrone longue éventuelle à lier au reçu.
    pub operation_id: Option<OperationId>,
    /// État final du reçu (par défaut `Accepted`).
    pub receipt_state: ReceiptState,
    /// Référence opaque de résultat.
    pub result_ref: Option<String>,
    /// Fenêtre de conservation complète (1..=90 jours, `None` = défaut config 7 jours).
    pub retention_days_override: Option<u32>,
    /// Vérification optionnelle de fence/lease dans la lignée active.
    pub fence_assertion: Option<FenceAssertion>,
    /// Mutation de domaine.
    pub mutation: CommandMutation,
    /// Événements à écrire atomiquement (le dernier donne son curseur au reçu).
    pub events: Vec<EventAppendSpec>,
    /// Réservations de capacité atomiques associées.
    pub capacity_reservations: Vec<CapacityReservationRequest>,
    /// Réservations budgétaires atomiques associées.
    pub budget_reservations: Vec<BudgetReservationRequest>,
}

/// Issue d'un appel à `CommitCommand`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Nouvelle décision engagée durablement pour la première fois.
    Committed(CommandReceipt),
    /// Rejeu idempotent exact dans la même lignée et fenêtre active.
    IdempotentReplay(CommandReceipt),
}

impl CommitOutcome {
    /// Renvoie le reçu quel que soit le chemin (`Committed` ou `IdempotentReplay`).
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        match self {
            Self::Committed(r) | Self::IdempotentReplay(r) => r,
        }
    }
}

/// Résultat d'inspection d'un reçu (`GetCommandReceipt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptInspectionOutcome {
    /// Reçu complet toujours dans sa fenêtre de conservation.
    Full(CommandReceiptInspection),
    /// Réponse purgée en tombstone minimal ; clé et causalité conservées.
    Tombstone {
        /// Identifiant serveur unique du reçu.
        receipt_id: ReceiptId,
        /// Identifiant de commande client.
        command_id: CommandId,
        /// Lignée d'origine.
        authority_generation: AuthorityGeneration,
        /// Statut de lignée (`Current` ou `Historical`).
        lineage_status: unleash_protocol::LineageStatus,
        /// Digest canonique enregistré.
        command_digest: Sha256Digest,
        /// État historique du reçu.
        state: ReceiptState,
        /// Horodatage d'engagement.
        committed_at_ms: TimestampMs,
    },
}

/// Rapport de purge des réponses de commandes expirées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptPurgeReport {
    /// Purge exécutée avec nombre de réponses converties en tombstones.
    Purged {
        /// Nombre de reçus purgés en tombstones.
        tombstoned_count: usize,
    },
    /// Purge suspendue car l'horloge est déclarée incertaine.
    SuspendedClockUncertain,
}

/// État courant d'un `daemon_state` lu en base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStateRow {
    /// UID propriétaire.
    pub owner_uid: u32,
    /// Lignée d'autorité active.
    pub authority_generation: AuthorityGeneration,
    /// Époque dans la lignée.
    pub daemon_epoch: DaemonEpoch,
    /// Identifiant d'instance.
    pub daemon_instance_id: unleash_domain::DaemonInstanceId,
    /// Identité de boot OS.
    pub boot_identity: String,
    /// Identifiant du flux d'événements.
    pub stream_id: StreamId,
    /// Version de schéma active.
    pub schema_version: i64,
    /// Mode opérationnel courant.
    pub mode: DaemonMode,
    /// Révision du singleton daemon.
    pub revision: Revision,
    /// Horodatage de démarrage.
    pub started_at_ms: TimestampMs,
}

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};
use unleash_domain::RunState;
use unleash_protocol::{
    EventCursor, LineageStatus, canonical_sha256, compute_command_digest_v2, from_json_str_strict,
};

use crate::budgets::{tx_reserve_budgets, tx_reserve_capacity};
use crate::config::{CutpointObserver, StoreConfig, StoreCutpoint};
use crate::error::StoreError;
use crate::leases::tx_validate_lease_fence;

pub(crate) fn decode_command_envelope_json(raw_json: &str) -> Result<CommandEnvelope, StoreError> {
    match from_json_str_strict::<CommandEnvelope>(raw_json) {
        Ok(env) => Ok(env),
        Err(unleash_protocol::ProtocolError::ProtocolIncompatible { version }) => {
            Err(StoreError::ProtocolIncompatible { version })
        }
        Err(unleash_protocol::ProtocolError::Json(err)) => {
            let msg = err.to_string();
            if msg.contains("[protocol_incompatible:") || msg.contains("unleash.ipc.v1") {
                Err(StoreError::ProtocolIncompatible { version: msg })
            } else {
                Err(StoreError::Protocol(unleash_protocol::ProtocolError::Json(
                    err,
                )))
            }
        }
        Err(other) => Err(StoreError::Protocol(other)),
    }
}

pub(crate) fn read_daemon_state_conn(conn: &Connection) -> Result<DaemonStateRow, StoreError> {
    conn.query_row(
        "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                stream_id, schema_version, mode, revision, started_at_ms
         FROM daemon_state WHERE singleton = 1",
        [],
        map_daemon_state_row,
    )
    .map_err(StoreError::from_rusqlite)
}

pub(crate) fn read_daemon_state_tx(tx: &Transaction<'_>) -> Result<DaemonStateRow, StoreError> {
    tx.query_row(
        "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                stream_id, schema_version, mode, revision, started_at_ms
         FROM daemon_state WHERE singleton = 1",
        [],
        map_daemon_state_row,
    )
    .map_err(StoreError::from_rusqlite)
}

fn map_daemon_state_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DaemonStateRow> {
    let owner_uid_i64: i64 = r.get(0)?;
    let auth_str: String = r.get(1)?;
    let epoch_i64: i64 = r.get(2)?;
    let inst_str: String = r.get(3)?;
    let boot_id: String = r.get(4)?;
    let stream_str: String = r.get(5)?;
    let schema_ver: i64 = r.get(6)?;
    let mode_str: String = r.get(7)?;
    let rev_i64: i64 = r.get(8)?;
    let started_i64: i64 = r.get(9)?;

    let auth = auth_str
        .parse::<AuthorityGeneration>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let inst = inst_str
        .parse::<unleash_domain::DaemonInstanceId>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let stream_id = stream_str
        .parse::<StreamId>()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let mode = parse_daemon_mode(&mode_str).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(7, "mode".to_owned(), rusqlite::types::Type::Text)
    })?;

    Ok(DaemonStateRow {
        owner_uid: u32::try_from(owner_uid_i64).unwrap_or(0),
        authority_generation: auth,
        daemon_epoch: DaemonEpoch::new(u64::try_from(epoch_i64).unwrap_or(0)),
        daemon_instance_id: inst,
        boot_identity: boot_id,
        stream_id,
        schema_version: schema_ver,
        mode,
        revision: Revision::new(u64::try_from(rev_i64).unwrap_or(0)),
        started_at_ms: TimestampMs::new(started_i64),
    })
}

fn parse_daemon_mode(s: &str) -> Option<DaemonMode> {
    match s {
        "starting" => Some(DaemonMode::Starting),
        "recovering" => Some(DaemonMode::Recovering),
        "ready" => Some(DaemonMode::Ready),
        "degraded" => Some(DaemonMode::Degraded),
        "read_only_recovery" => Some(DaemonMode::ReadOnlyRecovery),
        "draining" => Some(DaemonMode::Draining),
        "stopped" => Some(DaemonMode::Stopped),
        _ => None,
    }
}

pub(crate) fn compute_validated_command_params(
    config: &StoreConfig,
    req: &CommitCommandRequest,
) -> Result<(Sha256Digest, u32), StoreError> {
    let retention_days = req
        .retention_days_override
        .unwrap_or(config.default_receipt_retention_days);
    if !(1..=90).contains(&retention_days) {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("fenêtre de rétention '{retention_days}' hors [1..=90] jours"),
        });
    }
    let digest = compute_command_digest_v2(&req.envelope)?;
    Ok((digest, retention_days))
}

struct CommandReplayRow {
    receipt_id: String,
    digest: String,
    state: String,
    response: Option<String>,
    authority_generation: String,
}

impl CommandReplayRow {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            receipt_id: row.get(0)?,
            digest: row.get(1)?,
            state: row.get(2)?,
            response: row.get(3)?,
            authority_generation: row.get(4)?,
        })
    }
}

/// Classificateur unique de rejeu : contrôle d'autorité avant lookup et digest wire.
/// Une clé F012 historique reste occupée et produit un conflit, jamais un miss ou
/// un reçu réactivé. Les autres ports gardent leurs préconditions métier existantes.
pub(crate) fn classify_command_replay(
    conn: &Connection,
    daemon: &DaemonStateRow,
    actor: &ActorId,
    envelope: &CommandEnvelope,
    digest: Sha256Digest,
) -> Result<Option<CommandReceipt>, StoreError> {
    if envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    let existing = conn.query_row(
        "SELECT receipt_id,command_digest,response_state,response_json,authority_generation FROM command_receipts WHERE actor_id=?1 AND command_id=?2",
        rusqlite::params![actor.as_str(),envelope.command_id.to_string()],
        CommandReplayRow::read,
    ).optional()?;
    let Some(row) = existing else {
        return Ok(None);
    };
    let lineage_bound = matches!(
        envelope.command_type.as_str(),
        "SetProviderProfile"
            | "TestProviderProfile"
            | "ResizeTerminal"
            | "ReconcileProcess"
            | crate::workflow_activation::COMMAND
            | crate::workflow_run_start::COMMAND
            | crate::task_assignment::COMMAND
    ) || (envelope.command_type == "AcquireTerminal"
        && envelope.payload.contains_key("terminal_id"))
        || (envelope.command_type == crate::agent_creation::COMMAND
            && envelope.payload.contains_key("task_id"));
    let historical_command =
        lineage_bound && row.authority_generation != daemon.authority_generation.to_string();
    if historical_command || row.digest != digest.as_prefixed_str() {
        return Err(StoreError::IdempotencyConflict {
            command_id: envelope.command_id,
        });
    }
    if row.state == "tombstone" {
        return Err(StoreError::IdempotencyWindowExpired {
            command_id: envelope.command_id,
            receipt_id: row.receipt_id.parse()?,
        });
    }
    let receipt: CommandReceipt = serde_json::from_str(&row.response.ok_or_else(|| {
        StoreError::InvariantViolation("reçu full sans response_json".to_owned())
    })?)?;
    Ok(Some(receipt))
}

/// Exécute l'algorithme transactionnel complet `CommitCommand` dans `BEGIN IMMEDIATE` (REQ-PERSIST-001/004).
#[expect(
    clippy::too_many_lines,
    reason = "Algorithme atomique unifié : contrôle lignée, lookup idempotence, mutation, événements, outbox, réserves et receipt"
)]
pub(crate) fn tx_commit_command(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now_ms: TimestampMs,
    req: &CommitCommandRequest,
    command_digest: Sha256Digest,
    retention_days: u32,
    cutpoints: &dyn CutpointObserver,
) -> Result<CommitOutcome, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;

    // Étape 1 : Contrôler `authority_generation` AVANT tout lookup de reçu (REQ-PERSIST-004)
    if req.envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: req.envelope.authority_generation,
            command_id: Some(req.envelope.command_id),
        });
    }
    crate::backup_requests::validate_request_backup_envelope(req)?;
    let local_validation = crate::provider_profile_validation::validate_identity(req)?;
    let connect = crate::provider_probe_connect::validate_identity(req)?;
    if !local_validation && !connect {
        crate::provider_probes::validate_command(tx, req)?;
    }
    crate::provider_profiles::validate_identity(req)?;
    crate::provider_service_launch::validate_command(req)?;
    crate::workflow_activation::validate_identity(req)?;
    crate::task_assignment::validate_identity(req)?;
    crate::agent_creation::validate_identity(req)?;
    crate::agent_from_task::validate_identity(req)?;
    crate::workflow_run_creation::validate_identity(req)?;
    crate::workflow_run_start::validate_identity(req)?;
    crate::mission_scope::validate_identity(req)?;
    crate::terminal_allocations::validate_command(tx, req)?;

    // Étape 2 : Rechercher `(actor_id, command_id)` dans `command_receipts`
    let actor_str = req.actor_id.as_str();
    let cmd_str = req.envelope.command_id.to_string();
    if let Some(receipt) =
        classify_command_replay(tx, &daemon, &req.actor_id, &req.envelope, command_digest)?
    {
        return Ok(CommitOutcome::IdempotentReplay(receipt));
    }
    crate::provider_profiles::validate_new_command(req)?;
    crate::provider_profile_validation::validate_new_command(req)?;
    crate::provider_probe_connect::validate_new_command(req)?;

    // Mode `read_only_recovery` refuse les nouvelles écritures métier ordinaires tout en
    // autorisant les opérations d'urgence / réconciliation / arrêt / purge.
    let allowed_in_ro_recovery = matches!(
        req.mutation,
        CommandMutation::TransitionDaemonMode { .. }
            | CommandMutation::ReconcileAction { .. }
            | CommandMutation::ResolveActionOutcome { .. }
            | CommandMutation::ResolveActionApproval { .. }
            | CommandMutation::AcknowledgeAlert { .. }
            | CommandMutation::RevokeSecretReference { .. }
            | CommandMutation::SuspendSubject { .. }
            | CommandMutation::RevokeSubject { .. }
            | CommandMutation::PurgeHistory { .. }
    );
    if daemon.mode == DaemonMode::ReadOnlyRecovery && !allowed_in_ro_recovery {
        return Err(StoreError::RecoveryInProgress(
            "daemon en mode read_only_recovery".to_owned(),
        ));
    }

    // Étape 3 : Vérifier fence, expected_revision et appliquer la décision pure
    if let Some(ref assertion) = req.fence_assertion {
        tx_validate_lease_fence(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            assertion,
        )?;
    }

    let receipt_id = ReceiptId::from_uuid(config.id_generator.new_uuid());
    let receipt_id_str = receipt_id.to_string();
    let expected_rev = req.envelope.expected_revision;
    let mut generated_events = Vec::new();
    let new_rev = apply_command_mutation(
        tx,
        config,
        &daemon,
        now_ms,
        expected_rev,
        &req.mutation,
        req.envelope.command_id,
        receipt_id,
        &req.envelope,
        &req.actor_id,
        &mut generated_events,
    )?;

    // Réservations atomiques de capacité et de budgets
    for cap_req in &req.capacity_reservations {
        tx_reserve_capacity(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            now_ms,
            cap_req,
        )?;
    }
    for budget_req in &req.budget_reservations {
        tx_reserve_budgets(tx, config.id_generator.as_ref(), budget_req)?;
    }

    // Étape 4 : Écrire events + outbox + command_receipts dans la MÊME transaction
    let mut first_seq: Option<i64> = None;
    let mut last_seq: Option<i64> = None;
    let mut ordinal_counters: HashMap<(String, String, i64), i64> = HashMap::new();

    for ev in req.events.iter().chain(generated_events.iter()) {
        let event_id = config.id_generator.new_uuid().to_string();
        let payload_json = serde_json::to_string(&ev.payload)?;
        let payload_hash = canonical_sha256(&ev.payload)?;
        let (causation_kind_str, causation_id_str, causation_rcpt_opt) = match ev.causation_kind {
            EventCausationKind::Command => (
                "command",
                receipt_id_str.clone(),
                Some(receipt_id_str.clone()),
            ),
            EventCausationKind::Event => (
                "event",
                ev.causation_id.clone().unwrap_or_else(|| event_id.clone()),
                None,
            ),
            EventCausationKind::Action => {
                ("action", ev.causation_id.clone().unwrap_or_default(), None)
            }
            EventCausationKind::Timer => {
                ("timer", ev.causation_id.clone().unwrap_or_default(), None)
            }
            EventCausationKind::External => (
                "external",
                ev.causation_id.clone().unwrap_or_default(),
                None,
            ),
            other => {
                return Err(StoreError::InvariantViolation(format!(
                    "type de causalité non supporté : {other:?}"
                )));
            }
        };
        let sensitivity_str = match ev.sensitivity {
            Sensitivity::Public => "public",
            Sensitivity::Project => "project",
            Sensitivity::Confidential => "confidential",
            Sensitivity::Restricted => "restricted",
            other => {
                return Err(StoreError::InvariantViolation(format!(
                    "niveau de sensibilité non supporté : {other:?}"
                )));
            }
        };
        let agg_rev_i64 = i64::try_from(ev.aggregate_revision.as_u64()).unwrap_or(1);
        let key = (
            ev.aggregate_type.as_str().to_owned(),
            ev.aggregate_id.to_string(),
            agg_rev_i64,
        );
        let ord_i64 = if let Some(next) = ordinal_counters.get(&key).copied() {
            next
        } else {
            tx.query_row(
                "SELECT COALESCE(MAX(event_ordinal) + 1, 0) FROM events
                 WHERE aggregate_type = ?1 AND aggregate_id = ?2 AND aggregate_revision = ?3",
                rusqlite::params![&key.0, &key.1, key.2],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?
        };
        ordinal_counters.insert(key, ord_i64.saturating_add(1));

        tx.execute(
            "INSERT INTO events
             (event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal,
              event_type, schema_version, project_id, actor_ref, actor_id, occurred_at_ms, recorded_at_ms,
              payload_json, payload_state, payload_hash, sensitivity, command_actor_id, command_id,
              causation_kind, causation_id, causation_receipt_id, correlation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11, ?11, ?12, 'full', ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                event_id,
                daemon.stream_id.to_string(),
                ev.aggregate_type.as_str(),
                ev.aggregate_id.to_string(),
                agg_rev_i64,
                ord_i64,
                ev.event_type,
                ev.project_id.map(|p| p.to_string()),
                actor_str,
                actor_str,
                now_ms.as_i64(),
                payload_json,
                payload_hash.as_prefixed_str(),
                sensitivity_str,
                actor_str,
                cmd_str,
                causation_kind_str,
                causation_id_str,
                causation_rcpt_opt,
                ev.correlation_id.to_string()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;

        let seq = tx.last_insert_rowid();
        if first_seq.is_none() {
            first_seq = Some(seq);
        }
        last_seq = Some(seq);

        for dest in &ev.outbox_destinations {
            let delivery_key = format!("{event_id}:{dest}");
            tx.execute(
                "INSERT INTO outbox
                 (event_id, destination, state, delivery_attempt, next_attempt_at_ms,
                  authority_generation, lease_epoch, consumer_version, delivery_key, acked_at_ms, last_error, rearm_count)
                 VALUES (?1, ?2, 'pending', 0, ?3, ?4, NULL, ?5, ?6, NULL, NULL, 0)",
                rusqlite::params![
                    event_id,
                    dest,
                    now_ms.as_i64(),
                    daemon.authority_generation.to_string(),
                    config.consumer_version,
                    delivery_key
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    let cursor_seq_u64 = if let Some(s) = last_seq {
        u64::try_from(s).unwrap_or(0)
    } else {
        let max_s: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
                [daemon.stream_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        u64::try_from(max_s).unwrap_or(0)
    };

    let accepted_at =
        DateTime::<Utc>::from_timestamp_millis(now_ms.as_i64()).unwrap_or(DateTime::UNIX_EPOCH);
    let purge_id: Option<String> = if matches!(req.mutation, CommandMutation::PurgeHistory { .. }) {
        tx.query_row(
            "SELECT purge_id FROM retention_purges WHERE command_receipt_id = ?1",
            [receipt_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?
    } else {
        None
    };
    let reserved_backup = if let CommandMutation::RequestBackup { backup_id } = &req.mutation {
        Some(*backup_id)
    } else {
        None
    };
    let prepared_probe = match &req.mutation {
        CommandMutation::PrepareProviderProbe { probe_id, .. } => Some(*probe_id),
        CommandMutation::PrepareProviderProbeConnect { .. }
        | CommandMutation::PrepareProviderProbeInventory { .. }
        | CommandMutation::PrepareAuthenticatedProviderGeneration { .. } => Some(
            unleash_domain::ProviderProbeId::from_uuid(req.envelope.aggregate_id),
        ),
        _ => None,
    };
    let effective_receipt_state = if purge_id.is_some()
        || reserved_backup.is_some()
        || prepared_probe.is_some()
        || matches!(req.mutation, CommandMutation::SetProviderProfile { .. })
        || matches!(
            req.mutation,
            CommandMutation::PrepareTerminalAllocation { .. }
                | CommandMutation::PrepareTerminalControl { .. }
                | CommandMutation::PrepareTerminalInput { .. }
                | CommandMutation::PrepareTerminalResize { .. }
                | CommandMutation::PrepareTerminalRelease { .. }
        ) {
        ReceiptState::Accepted
    } else {
        req.receipt_state
    };
    let control_result = if matches!(req.mutation, CommandMutation::PrepareTerminalControl { .. }) {
        let sql = if daemon.schema_version >= 16 {
            "SELECT action_id FROM terminal_control_intents WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3 UNION ALL SELECT action_id FROM terminal_renewal_intents WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3"
        } else {
            "SELECT action_id FROM terminal_control_intents WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3"
        };
        let mut query = tx.prepare(sql)?;
        let actions = query
            .query_map(
                rusqlite::params![receipt_id.to_string(), actor_str, cmd_str],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let [action] = actions.as_slice() else {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "registered acquisition must have one exact intention".into(),
            });
        };
        Some(format!("terminal-control:{action}"))
    } else {
        None
    };
    let input_result = if matches!(req.mutation, CommandMutation::PrepareTerminalInput { .. }) {
        let action: String = tx.query_row(
            "SELECT action_id FROM terminal_input_intents WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3",
            rusqlite::params![receipt_id.to_string(), actor_str, cmd_str],
            |row| row.get(0),
        )?;
        Some(format!("terminal-input:{action}"))
    } else {
        None
    };
    let release_result = if matches!(req.mutation, CommandMutation::PrepareTerminalRelease { .. }) {
        let action: String = tx.query_row(
            "SELECT action_id FROM terminal_release_intents WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3",
            rusqlite::params![receipt_id.to_string(), actor_str, cmd_str], |row| row.get(0),
        )?;
        Some(format!("terminal-release:{action}"))
    } else {
        None
    };
    let service_launch_result = matches!(
        req.mutation,
        CommandMutation::PrepareProviderServiceLaunch { .. }
    )
    .then(|| format!("provider-service-launch:{}", req.envelope.aggregate_id));
    let model_selection_result = if let CommandMutation::SetSettingsWithModelSelectionBinding {
        scope_kind,
        scope_id,
        ..
    } = &req.mutation
    {
        Some(crate::model_selection_bindings::result_ref_tx(
            tx,
            *scope_kind,
            scope_id,
            receipt_id,
        )?)
    } else {
        None
    };
    let receipt = CommandReceipt {
        receipt_id,
        command_id: req.envelope.command_id,
        accepted_at,
        aggregate_id: req.envelope.aggregate_id,
        new_revision: new_rev,
        operation_id: req.operation_id,
        state: effective_receipt_state,
        event_cursor: EventCursor::new(
            daemon.stream_id,
            unleash_domain::Sequence::new(cursor_seq_u64),
        ),
        result_ref: purge_id
            .map(|id| format!("retention-purge:{id}"))
            .or_else(|| reserved_backup.map(|id| format!("backup:{id}")))
            .or_else(|| prepared_probe.map(|id| format!("provider-probe:{id}")))
            .or(service_launch_result)
            .or(model_selection_result)
            .or(release_result)
            .or_else(|| {
                if matches!(req.mutation, CommandMutation::SetProviderProfile { .. }) {
                    Some(format!(
                        "provider-profile:{}:{}",
                        req.envelope.aggregate_id,
                        new_rev.as_u64()
                    ))
                } else {
                    None
                }
            })
            .or_else(|| {
                if let CommandMutation::PrepareTerminalAllocation { terminal_id, .. } =
                    &req.mutation
                {
                    Some(format!("terminal-allocation:{terminal_id}"))
                } else {
                    None
                }
            })
            .or(control_result)
            .or(input_result)
            .or_else(|| req.result_ref.clone()),
    };
    let receipt_json = serde_json::to_string(&receipt)?;
    let state_str = match effective_receipt_state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::Completed => "completed",
        ReceiptState::Blocked => "blocked",
        ReceiptState::Rejected => "rejected",
        other => {
            return Err(StoreError::InvariantViolation(format!(
                "état de reçu non supporté : {other:?}"
            )));
        }
    };
    let retained_until_ms = now_ms
        .as_i64()
        .saturating_add(i64::from(retention_days).saturating_mul(86_400_000));

    tx.execute(
        "INSERT INTO command_receipts
         (actor_id, command_id, authority_generation, receipt_id, request_id, command_type,
          command_digest, aggregate_id, state, response_state, response_json,
          response_retained_until_ms, first_event_seq, last_event_seq, committed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'full', ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            actor_str,
            cmd_str,
            daemon.authority_generation.to_string(),
            receipt_id_str,
            req.envelope.request_id.to_string(),
            req.envelope.command_type,
            command_digest.as_prefixed_str(),
            req.envelope.aggregate_id.to_string(),
            state_str,
            receipt_json,
            retained_until_ms,
            first_seq,
            last_seq,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    if matches!(req.mutation, CommandMutation::PurgeHistory { .. }) {
        tx.execute(
            "UPDATE object_deletions SET decision_receipt_id = ?1
             WHERE decision_receipt_id IS NULL AND reason IN (
                 SELECT 'retention_q12:' || purge_id FROM retention_purges WHERE command_receipt_id = ?1
             )",
            [&receipt_id_str],
        ).map_err(StoreError::from_rusqlite)?;
    }

    cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;

    Ok(CommitOutcome::Committed(receipt))
}

pub(crate) struct NewAgent {
    pub agent_id: AgentId,
    pub level: AgentLevel,
    pub organization_parent_id: Option<AgentId>,
    pub created_by_agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub state: AgentState,
}

pub(crate) fn insert_agent_and_session(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    identity: &NewAgent,
) -> Result<unleash_domain::SessionId, StoreError> {
    let agent = identity.agent_id.to_string();
    tx.execute(
        "INSERT INTO agents (agent_id, level, organization_parent_id, created_by_agent_id, project_id, state, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        rusqlite::params![
            agent,
            identity.level.to_string(),
            identity.organization_parent_id.map(|a| a.to_string()),
            identity.created_by_agent_id.map(|a| a.to_string()),
            identity.project_id.map(|p| p.to_string()),
            identity.state.to_string(),
        ],
    )?;
    let session = config.id_generator.new_session_id();
    tx.execute(
        "INSERT INTO sessions (session_id, agent_id, revision, state, created_at_ms)
         VALUES (?1, ?2, 0, 'open', ?3)",
        rusqlite::params![session.to_string(), agent, now.as_i64()],
    )?;
    Ok(session)
}

pub(crate) fn ensure_scope_not_frozen_tx(
    tx: &Transaction<'_>,
    project_id: Option<ProjectId>,
    scope_key: Option<&str>,
    run_id: Option<RunId>,
) -> Result<(), StoreError> {
    let mut candidates = Vec::new();
    if let Some(pid) = project_id {
        candidates.push(pid.to_string());
    }
    if let Some(sk) = scope_key {
        candidates.push(sk.to_owned());
    }
    if let Some(rid) = run_id {
        candidates.push(rid.to_string());
        if let Some(run_proj) = tx
            .query_row(
                "SELECT m.project_id
                 FROM workflow_runs wr
                 JOIN missions m ON m.mission_id = wr.mission_id
                 WHERE wr.run_id = ?1",
                [rid.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?
        {
            candidates.push(run_proj);
        }
    }

    let global_cnt: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM recovery_scope_freezes
             WHERE state = 'frozen' AND global_freeze = 1",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if global_cnt > 0 {
        return Err(StoreError::RecoveryInProgress(
            "gel global post-restauration actif tant que la récupération n'est pas disposée"
                .to_owned(),
        ));
    }

    for cand in candidates {
        let cnt: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM recovery_scope_freezes
                 WHERE state = 'frozen' AND scope_id = ?1",
                [&cand],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if cnt > 0 {
            return Err(StoreError::RecoveryInProgress(format!(
                "scope '{cand}' gelé post-restauration en attente de réconciliation/disposition"
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct NewMission<'a> {
    pub mission_id: MissionId,
    pub goal_id: Option<GoalId>,
    pub project_id: ProjectId,
    pub owner_principal_id: Option<&'a str>,
    pub owner_agent_id: Option<AgentId>,
    pub completion_policy: CompletionPolicy,
    pub state: MissionState,
    pub expected_revision: Revision,
    pub command_id: CommandId,
}

pub(crate) fn insert_mission(
    tx: &Transaction<'_>,
    input: NewMission<'_>,
) -> Result<Revision, StoreError> {
    let NewMission {
        mission_id,
        goal_id,
        project_id,
        owner_principal_id,
        owner_agent_id,
        completion_policy,
        state,
        expected_revision: expected_rev,
        command_id: cmd_id,
    } = input;
    ensure_creation_rev(expected_rev, cmd_id)?;
    let exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM missions WHERE mission_id = ?1",
            [mission_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if exists > 0 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    let owner_kind = if owner_principal_id.is_some() {
        "user"
    } else {
        "agent"
    };
    tx.execute(
        "INSERT INTO missions (mission_id, goal_id, project_id, owner_kind, owner_user_id, owner_agent_id, completion_policy, leader_agent_id, worktree_id, state, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, 1)",
        rusqlite::params![
            mission_id.to_string(),
            goal_id.map(|g| g.to_string()),
            project_id.to_string(),
            owner_kind,
            owner_principal_id,
            owner_agent_id.map(|a| a.to_string()),
            completion_policy.to_string(),
            state.to_string()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(Revision::new(1))
}

#[derive(Clone, Copy)]
pub(crate) struct NewWorkflowRun<'a> {
    pub run_id: RunId,
    pub mission_id: MissionId,
    pub workflow_version_id: &'a str,
    pub purpose: &'a str,
    pub candidate_ref: Option<&'a str>,
    pub predecessor_run_id: Option<RunId>,
    pub selection_reason: &'a str,
    pub effective_bundle_hash: Sha256Digest,
    pub expected_revision: Revision,
    pub command_id: CommandId,
    pub now: TimestampMs,
}

pub(crate) fn insert_workflow_run(
    tx: &Transaction<'_>,
    input: NewWorkflowRun<'_>,
) -> Result<Revision, StoreError> {
    let NewWorkflowRun {
        run_id,
        mission_id,
        workflow_version_id,
        purpose,
        candidate_ref,
        predecessor_run_id,
        selection_reason,
        effective_bundle_hash,
        expected_revision: expected_rev,
        command_id: cmd_id,
        now: now_ms,
    } = input;
    ensure_creation_rev(expected_rev, cmd_id)?;
    if let Some(pred) = predecessor_run_id {
        let pred_state_str: Option<String> = tx
            .query_row(
                "SELECT state FROM workflow_runs WHERE run_id = ?1",
                [pred.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        let Some(ps) = pred_state_str else {
            return Err(StoreError::NotFound(format!("run prédécesseur {pred}")));
        };
        let pred_state = parse_run_state(&ps)?;
        if !pred_state.is_terminal() {
            return Err(StoreError::PredecessorNotTerminal { state: ps });
        }
        let succ_cnt: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_runs WHERE predecessor_run_id = ?1",
                [pred.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if succ_cnt > 0 {
            return Err(StoreError::SuccessorExists);
        }
    }
    tx.execute(
        "INSERT INTO workflow_runs
         (run_id, mission_id, workflow_version_id, purpose, candidate_ref, predecessor_run_id,
          selection_reason, selected_via_alias, effective_bundle_hash, state, revision, started_at_ms, ended_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 'prepared', 1, ?9, NULL)",
        rusqlite::params![
            run_id.to_string(),
            mission_id.to_string(),
            workflow_version_id,
            purpose,
            candidate_ref,
            predecessor_run_id.map(|r| r.to_string()),
            selection_reason,
            effective_bundle_hash.as_prefixed_str(),
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(Revision::new(1))
}

#[expect(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "Dispatch exhaustif des variantes CommandMutation dans la transaction atomique"
)]
fn apply_command_mutation(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now_ms: TimestampMs,
    expected_rev: Revision,
    mutation: &CommandMutation,
    cmd_id: CommandId,
    receipt_id: ReceiptId,
    envelope: &CommandEnvelope,
    actor_id: &ActorId,
    generated_events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(0);
    match mutation {
        CommandMutation::AttestHistoricalTerminalRelease { caller, proof } => {
            use crate::terminal_allocations::terminal_release_history::route::{
                self, HistoryCommand,
            };
            route::commit_tx(
                tx,
                config,
                daemon,
                now_ms,
                &HistoryCommand {
                    caller,
                    envelope,
                    proof: proof.as_ref(),
                },
                generated_events,
            )
        }
        CommandMutation::PrepareTerminalRelease { request, .. } => {
            crate::terminal_allocations::terminal_release::prepare_tx(
                tx,
                config,
                daemon,
                now_ms,
                &crate::terminal_allocations::terminal_release::ReleaseCommand {
                    actor: actor_id,
                    envelope,
                    request,
                    receipt: receipt_id,
                },
                generated_events,
            )
        }
        CommandMutation::PrepareTerminalResize { caller, request } => {
            crate::terminal_allocations::terminal_renewal::prepare_resize_tx(
                tx,
                config,
                daemon,
                now_ms,
                &crate::terminal_allocations::terminal_renewal::ResizeCommand {
                    envelope,
                    caller,
                    request,
                    receipt: receipt_id,
                },
                generated_events,
            )
        }
        CommandMutation::PrepareTerminalInput { request, .. } => {
            crate::terminal_allocations::terminal_input_writer::prepare_tx(
                tx,
                config,
                daemon,
                now_ms,
                &crate::terminal_allocations::terminal_input_writer::InputCommand {
                    actor_id,
                    envelope,
                    request,
                    receipt_id,
                },
                generated_events,
            )
        }
        CommandMutation::PrepareTerminalControl { request, caller } => {
            crate::terminal_allocations::terminal_renewal::prepare_registered_tx(
                tx,
                config,
                daemon,
                now_ms,
                &crate::terminal_allocations::terminal_renewal::RegisteredCommand {
                    request,
                    receipt: receipt_id,
                    command: cmd_id,
                    actor: actor_id,
                    caller,
                    project: envelope.project_id,
                    expected: expected_rev,
                },
                generated_events,
            )
        }
        CommandMutation::SetProviderProfile { preparation, .. } => {
            crate::provider_profiles::set_tx(
                tx,
                config,
                daemon,
                now_ms,
                crate::provider_profiles::ProfileCommand {
                    envelope,
                    actor_id,
                    preparation,
                },
                receipt_id,
                generated_events,
            )
        }
        CommandMutation::ValidateProviderProfile {
            caller,
            preparation,
        } => crate::provider_profile_validation::commit_tx(
            tx,
            config,
            daemon,
            now_ms,
            crate::provider_profile_validation::ValidationCommand {
                envelope,
                caller,
                preparation,
            },
            receipt_id,
            generated_events,
        ),
        CommandMutation::PrepareProviderProbeConnect {
            caller,
            preparation,
        } => crate::provider_probe_connect::commit_tx(
            tx,
            config,
            daemon,
            now_ms,
            &crate::provider_probe_connect::ConnectCommand {
                envelope,
                caller,
                preparation,
                phase: crate::provider_probe_connect::TransportPhase::Connect,
            },
            receipt_id,
            generated_events,
        ),
        CommandMutation::PrepareProviderProbeInventory {
            caller,
            preparation,
        } => crate::provider_probe_connect::commit_tx(
            tx,
            config,
            daemon,
            now_ms,
            &crate::provider_probe_connect::ConnectCommand {
                envelope,
                caller,
                preparation: &preparation.inner,
                phase: crate::provider_probe_connect::TransportPhase::Inventory,
            },
            receipt_id,
            generated_events,
        ),
        CommandMutation::PrepareProviderServiceLaunch { caller } => {
            crate::provider_service_launch::commit_tx(
                tx,
                config,
                daemon,
                now_ms,
                envelope,
                caller,
                receipt_id,
                generated_events,
            )
        }
        CommandMutation::PrepareTerminalAllocation {
            terminal_id,
            preparation,
            ..
        } => crate::terminal_allocations::prepare_tx(
            tx,
            config,
            daemon,
            now_ms,
            *terminal_id,
            receipt_id,
            actor_id,
            envelope.project_id,
            preparation,
            generated_events,
        ),
        CommandMutation::PrepareProviderProbe {
            probe_id,
            preparation,
            ..
        } => crate::provider_probes::prepare_tx(
            tx,
            config,
            daemon,
            now_ms,
            *probe_id,
            receipt_id,
            actor_id,
            envelope.project_id,
            preparation,
            None,
            generated_events,
        ),
        CommandMutation::PrepareAuthenticatedProviderGeneration {
            caller,
            preparation,
        } => crate::provider_probe_generation::commit_tx(
            tx,
            config,
            daemon,
            now_ms,
            &crate::provider_probe_generation::GenerationCommit {
                command: envelope,
                caller,
                receipt: receipt_id,
                preparation,
            },
            generated_events,
        ),
        CommandMutation::RequestBackup { backup_id } => crate::backup_requests::reserve_backup_tx(
            tx,
            config,
            daemon,
            now_ms,
            *backup_id,
            receipt_id,
            actor_id,
            generated_events,
        ),
        CommandMutation::CreateProject {
            project_id,
            name,
            weight,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
                    [project_id.to_string()],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if exists > 0 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(1),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            tx.execute(
                "INSERT INTO projects (project_id, name, weight, state, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 'active', 1, ?4, ?4)",
                rusqlite::params![project_id.to_string(), name, weight, now_ms.as_i64()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateAgent {
            agent_id,
            level,
            organization_parent_id,
            created_by_agent_id,
            project_id,
            state,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            insert_agent_and_session(
                tx,
                config,
                now_ms,
                &NewAgent {
                    agent_id: *agent_id,
                    level: *level,
                    organization_parent_id: *organization_parent_id,
                    created_by_agent_id: *created_by_agent_id,
                    project_id: *project_id,
                    state: *state,
                },
            )?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateAssignedAgent { caller } => crate::agent_creation::commit_tx(
            tx,
            (config, daemon, now_ms),
            envelope,
            caller,
            generated_events,
        ),
        CommandMutation::CreateAgentFromWorkflowTask(prepared) => {
            crate::agent_from_task::commit_tx(
                tx,
                (config, daemon, now_ms),
                envelope,
                prepared,
                generated_events,
            )
        }
        CommandMutation::CreateGoal {
            goal_id,
            project_id,
            owner_principal_id,
            owner_agent_id,
            completion_policy,
            definition_ref,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let owner_kind = if owner_principal_id.is_some() {
                "user"
            } else {
                "agent"
            };
            tx.execute(
                "INSERT INTO goals (goal_id, project_id, owner_kind, owner_user_id, owner_agent_id, owner_epoch, completion_policy, state, definition_ref, revision, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 'draft', ?7, 1, ?8)",
                rusqlite::params![
                    goal_id.to_string(),
                    project_id.to_string(),
                    owner_kind,
                    owner_principal_id,
                    owner_agent_id.map(|a| a.to_string()),
                    completion_policy.to_string(),
                    definition_ref,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::CreateMission {
            mission_id,
            goal_id,
            project_id,
            owner_principal_id,
            owner_agent_id,
            completion_policy,
            state,
        } => insert_mission(
            tx,
            NewMission {
                mission_id: *mission_id,
                goal_id: *goal_id,
                project_id: *project_id,
                owner_principal_id: owner_principal_id.as_deref(),
                owner_agent_id: *owner_agent_id,
                completion_policy: *completion_policy,
                state: *state,
                expected_revision: expected_rev,
                command_id: cmd_id,
            },
        ),
        CommandMutation::CreateScopedMission(publication) => crate::mission_scope::commit_tx(
            tx,
            (config, daemon, now_ms),
            envelope,
            actor_id,
            publication,
        ),
        CommandMutation::TransitionMission {
            mission_id,
            target_state,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM missions WHERE mission_id = ?1",
                    [mission_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("mission {mission_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE missions SET state = ?1, revision = ?2 WHERE mission_id = ?3",
                rusqlite::params![target_state.to_string(), next_rev, mission_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::CreateWorkflowRun {
            run_id,
            mission_id,
            workflow_version_id,
            purpose,
            candidate_ref,
            predecessor_run_id,
            selection_reason,
            effective_bundle_hash,
        } => insert_workflow_run(
            tx,
            NewWorkflowRun {
                run_id: *run_id,
                mission_id: *mission_id,
                workflow_version_id,
                purpose,
                candidate_ref: candidate_ref.as_deref(),
                predecessor_run_id: *predecessor_run_id,
                selection_reason,
                effective_bundle_hash: *effective_bundle_hash,
                expected_revision: expected_rev,
                command_id: cmd_id,
                now: now_ms,
            },
        ),
        CommandMutation::CreatePreparedWorkflowRun(publication) => {
            crate::workflow_run_creation::commit_tx(
                tx,
                (config, daemon, now_ms),
                envelope,
                publication,
                generated_events,
            )
        }
        CommandMutation::StartPreparedWorkflowRun(start) => crate::workflow_run_start::commit_tx(
            tx,
            (config, daemon, now_ms),
            envelope,
            start,
            generated_events,
        ),
        CommandMutation::TransitionWorkflowRun {
            run_id,
            target_state,
            ended_at_ms,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM workflow_runs WHERE run_id = ?1",
                    [run_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("run {run_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE workflow_runs SET state = ?1, revision = ?2, ended_at_ms = COALESCE(?3, ended_at_ms)
                 WHERE run_id = ?4",
                rusqlite::params![
                    target_state.to_string(),
                    next_rev,
                    ended_at_ms.map(|t| t.as_i64()),
                    run_id.to_string()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::ActivateWorkflowTask => {
            crate::workflow_activation::activate_tx(tx, daemon, envelope, generated_events)
        }
        CommandMutation::AssignTask {
            caller,
            preparation,
        } => crate::task_assignment::queue_tx(
            tx,
            (config, daemon, now_ms),
            &crate::task_assignment::AssignmentCommand {
                envelope,
                caller,
                preparation,
            },
            generated_events,
        ),
        CommandMutation::CreateAction {
            action_id,
            origin_kind,
            control_principal_id,
            run_id,
            step_id,
            invocation_id,
            attempt_id,
            replay_worker_id,
            action_type,
            payload_hash,
            reconcile_strategy,
            policy_revision,
            resource_lease_id,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            ensure_scope_not_frozen_tx(tx, None, None, *run_id)?;
            let epoch_i64 = i64::try_from(daemon.daemon_epoch.as_u64()).unwrap_or(1);
            tx.execute(
                "INSERT INTO actions
                 (action_id, origin_kind, control_principal_id, run_id, step_id, invocation_id, attempt_id,
                  replay_worker_id, action_type, payload_hash, authorization_ref, policy_revision, resource_lease_id,
                  authority_generation, daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, ?13, ?14, 0, 'prepared', ?15, NULL, 1, ?16, ?16)",
                rusqlite::params![
                    action_id.to_string(),
                    origin_kind,
                    control_principal_id,
                    run_id.map(|r| r.to_string()),
                    step_id.map(|s| s.to_string()),
                    invocation_id.map(|i| i.to_string()),
                    attempt_id.map(|a| a.to_string()),
                    replay_worker_id.map(|w| w.to_string()),
                    action_type,
                    payload_hash.as_prefixed_str(),
                    policy_revision,
                    resource_lease_id,
                    daemon.authority_generation.to_string(),
                    epoch_i64,
                    reconcile_strategy,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::TransitionDaemonMode { target_mode } => {
            let cur_rev = i64::try_from(daemon.revision.as_u64()).unwrap_or(0);
            if cur_rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: daemon.revision,
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_mode = unleash_domain::transition_daemon_mode(daemon.mode, *target_mode)?;
            let next_rev = cur_rev.saturating_add(1);
            tx.execute(
                "UPDATE daemon_state SET mode = ?1, revision = ?2 WHERE singleton = 1",
                rusqlite::params![next_mode.to_string(), next_rev],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::TransitionGoal {
            goal_id,
            target_state,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM goals WHERE goal_id = ?1",
                    [goal_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("goal {goal_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            tx.execute(
                "UPDATE goals SET state = ?1, revision = ?2 WHERE goal_id = ?3",
                rusqlite::params![target_state.to_string(), next_rev, goal_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::TransitionInvocation {
            invocation_id,
            target_state,
            wait_reason,
        } => {
            let cur_rev: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM invocations WHERE invocation_id = ?1",
                    [invocation_id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StoreError::from_rusqlite)?;
            let Some(rev) = cur_rev else {
                return Err(StoreError::NotFound(format!("invocation {invocation_id}")));
            };
            if rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = rev.saturating_add(1);
            let wr = wait_reason.map(|w| w.to_string());
            tx.execute(
                "UPDATE invocations SET state = ?1, wait_reason = ?2, revision = ?3 WHERE invocation_id = ?4",
                rusqlite::params![target_state.to_string(), wr, next_rev, invocation_id.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::NoopDaemonBump => {
            let cur_rev = i64::try_from(daemon.revision.as_u64()).unwrap_or(0);
            if cur_rev != exp_i64 {
                return Err(StoreError::RevisionConflict {
                    current_revision: daemon.revision,
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let next_rev = cur_rev.saturating_add(1);
            tx.execute(
                "UPDATE daemon_state SET revision = ?1 WHERE singleton = 1",
                [next_rev],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
        }
        CommandMutation::ReviseProject {
            project_id,
            name,
            description,
            weight,
            primary_root_path,
            sources,
            repositories,
            settings_json,
        } => {
            let new_rev = tx_revise_project(
                tx,
                now_ms,
                expected_rev,
                cmd_id,
                *project_id,
                name,
                description,
                *weight,
                primary_root_path.as_deref(),
                sources,
                repositories,
                settings_json,
            )?;
            let mut payload = Map::new();
            payload.insert("name".to_owned(), Value::String(name.clone()));
            payload.insert("weight".to_owned(), Value::Number((*weight).into()));
            generated_events.push(EventAppendSpec {
                event_type: "ProjectRevised".to_owned(),
                aggregate_type: AggregateKind::Project,
                aggregate_id: project_id.as_uuid(),
                aggregate_revision: new_rev,
                project_id: Some(*project_id),
                correlation_id: project_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::SetSettings {
            scope_kind,
            scope_id,
            caller,
            patch,
        }
        | CommandMutation::SetSettingsWithModelSelectionBinding {
            scope_kind,
            scope_id,
            caller,
            patch,
            ..
        } => {
            let project_id = if *scope_kind == SettingsScopeKind::Project {
                Some(scope_id.parse::<ProjectId>()?)
            } else {
                None
            };
            if project_id != envelope.project_id {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: "settings project scope does not match command envelope".to_owned(),
                });
            }
            let new_rev = tx_set_settings(
                tx,
                now_ms,
                expected_rev,
                cmd_id,
                *scope_kind,
                scope_id,
                caller,
                patch,
                config.id_generator.as_ref(),
            )?;
            if let CommandMutation::SetSettingsWithModelSelectionBinding { binding, .. } = mutation
            {
                if &caller.actor_id != actor_id || envelope.command_type != "SetSettings" {
                    return Err(StoreError::InvariantViolation(
                        "model preference command identity diverges".to_owned(),
                    ));
                }
                crate::model_selection_bindings::append_tx(
                    tx,
                    (*scope_kind, scope_id),
                    crate::model_selection_bindings::BindingEdit {
                        command: envelope,
                        actor: actor_id,
                        receipt_id,
                    },
                    binding,
                    config.id_generator.as_ref(),
                )?;
            }
            let mut payload = Map::new();
            payload.insert(
                "scope_kind".to_owned(),
                Value::String(scope_kind.as_str().to_owned()),
            );
            payload.insert("scope_id".to_owned(), Value::String(scope_id.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SettingsUpdated".to_owned(),
                aggregate_type: AggregateKind::Settings,
                aggregate_id: envelope.aggregate_id,
                aggregate_revision: new_rev,
                project_id,
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::SetPolicy {
            scope_kind,
            scope_id,
            isolation_mode,
            policy_version,
            reason,
            caller,
        } => {
            let new_rev = tx_set_policy(
                tx,
                now_ms,
                expected_rev,
                cmd_id,
                *scope_kind,
                scope_id,
                *isolation_mode,
                policy_version,
                reason,
                caller,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "isolation_mode".to_owned(),
                Value::String(isolation_mode.as_str().to_owned()),
            );
            payload.insert("reason".to_owned(), Value::String(reason.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SecurityPolicyUpdated".to_owned(),
                aggregate_type: AggregateKind::SecurityPolicy,
                aggregate_id: envelope.aggregate_id,
                aggregate_revision: new_rev,
                project_id: scope_id.parse::<ProjectId>().ok(),
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::SetPolicyWithProviderProbeMandate {
            scope_kind,
            scope_id,
            isolation_mode,
            policy_version,
            reason,
            caller,
            mandate,
        } => {
            let (new_rev, audit) = crate::provider_probe_mandates::apply_policy_change_tx(
                tx,
                &crate::provider_probe_mandates::PolicyMandateCommand {
                    daemon,
                    envelope,
                    actor_id,
                    caller,
                    receipt_id,
                    now: now_ms,
                    scope_kind: *scope_kind,
                    scope_id,
                    isolation_mode: *isolation_mode,
                    policy_version,
                    reason,
                },
                mandate,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "isolation_mode".to_owned(),
                Value::String(isolation_mode.as_str().to_owned()),
            );
            payload.insert("reason".to_owned(), Value::String(reason.clone()));
            payload.insert(
                "provider_probe_mandate".to_owned(),
                serde_json::to_value(audit)?,
            );
            generated_events.push(EventAppendSpec {
                event_type: "SecurityPolicyUpdated".to_owned(),
                aggregate_type: AggregateKind::SecurityPolicy,
                aggregate_id: envelope.aggregate_id,
                aggregate_revision: new_rev,
                project_id: envelope.project_id,
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec!["clients".to_owned()],
            });
            Ok(new_rev)
        }
        CommandMutation::ConfigureGuard {
            config: guard_cfg,
            caller,
        } => {
            let new_rev = tx_configure_guard(tx, now_ms, expected_rev, cmd_id, guard_cfg, caller)?;
            let mut payload = Map::new();
            payload.insert(
                "guard_mode".to_owned(),
                Value::String(guard_cfg.guard_mode.as_str().to_owned()),
            );
            payload.insert("reason".to_owned(), Value::String(guard_cfg.reason.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "GuardConfigured".to_owned(),
                aggregate_type: AggregateKind::GuardConfig,
                aggregate_id: envelope.aggregate_id,
                aggregate_revision: new_rev,
                project_id: None,
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::SuspendSubject {
            subject_kind,
            subject_id,
            scope_kind,
            scope_id,
            reason,
            caller,
            guard_propagated,
        } => {
            let new_rev = tx_transition_subject(
                tx,
                config.id_generator.as_ref(),
                now_ms,
                expected_rev,
                cmd_id,
                *subject_kind,
                subject_id,
                scope_kind,
                scope_id,
                SubjectStatus::Suspended,
                reason,
                *guard_propagated,
                caller,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "subject_kind".to_owned(),
                Value::String(subject_kind.as_str().to_owned()),
            );
            payload.insert("subject_id".to_owned(), Value::String(subject_id.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SubjectSuspended".to_owned(),
                aggregate_type: AggregateKind::SubjectSuspension,
                aggregate_id: config.id_generator.new_uuid(),
                aggregate_revision: new_rev,
                project_id: scope_id.parse::<ProjectId>().ok(),
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::ResumeSubject {
            subject_kind,
            subject_id,
            scope_kind,
            scope_id,
            reason,
            caller,
        } => {
            let new_rev = tx_transition_subject(
                tx,
                config.id_generator.as_ref(),
                now_ms,
                expected_rev,
                cmd_id,
                *subject_kind,
                subject_id,
                scope_kind,
                scope_id,
                SubjectStatus::Active,
                reason,
                false,
                caller,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "subject_kind".to_owned(),
                Value::String(subject_kind.as_str().to_owned()),
            );
            payload.insert("subject_id".to_owned(), Value::String(subject_id.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SubjectResumed".to_owned(),
                aggregate_type: AggregateKind::SubjectSuspension,
                aggregate_id: config.id_generator.new_uuid(),
                aggregate_revision: new_rev,
                project_id: scope_id.parse::<ProjectId>().ok(),
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::RevokeSubject {
            subject_kind,
            subject_id,
            scope_kind,
            scope_id,
            reason,
            caller,
            guard_propagated,
        } => {
            let new_rev = tx_transition_subject(
                tx,
                config.id_generator.as_ref(),
                now_ms,
                expected_rev,
                cmd_id,
                *subject_kind,
                subject_id,
                scope_kind,
                scope_id,
                SubjectStatus::Revoked,
                reason,
                *guard_propagated,
                caller,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "subject_kind".to_owned(),
                Value::String(subject_kind.as_str().to_owned()),
            );
            payload.insert("subject_id".to_owned(), Value::String(subject_id.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SubjectRevoked".to_owned(),
                aggregate_type: AggregateKind::SubjectSuspension,
                aggregate_id: config.id_generator.new_uuid(),
                aggregate_revision: new_rev,
                project_id: scope_id.parse::<ProjectId>().ok(),
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::SetSecretReference { input, caller } => {
            let new_rev = tx_set_secret_reference(tx, now_ms, expected_rev, cmd_id, input, caller)?;
            let mut payload = Map::new();
            payload.insert(
                "secret_ref_id".to_owned(),
                Value::String(input.secret_ref_id.clone()),
            );
            payload.insert(
                "namespace".to_owned(),
                Value::String(input.namespace.clone()),
            );
            payload.insert("purpose".to_owned(), Value::String(input.purpose.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SecretReferenceUpdated".to_owned(),
                aggregate_type: AggregateKind::SecretReference,
                aggregate_id: config.id_generator.new_uuid(),
                aggregate_revision: new_rev,
                project_id: input.scope_id.parse::<ProjectId>().ok(),
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Confidential,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::RevokeSecretReference {
            secret_ref_id,
            reason,
            caller,
            remote_revocation_supported,
            remote_revocation_confirmed,
        } => {
            let new_rev = tx_revoke_secret_reference(
                tx,
                now_ms,
                expected_rev,
                cmd_id,
                secret_ref_id,
                reason,
                caller,
                *remote_revocation_supported,
                *remote_revocation_confirmed,
            )?;
            let mut payload = Map::new();
            payload.insert(
                "secret_ref_id".to_owned(),
                Value::String(secret_ref_id.clone()),
            );
            payload.insert("reason".to_owned(), Value::String(reason.clone()));
            generated_events.push(EventAppendSpec {
                event_type: "SecretReferenceRevoked".to_owned(),
                aggregate_type: AggregateKind::SecretReference,
                aggregate_id: config.id_generator.new_uuid(),
                aggregate_revision: new_rev,
                project_id: None,
                correlation_id: cmd_id.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload,
                sensitivity: Sensitivity::Confidential,
                outbox_destinations: vec![],
            });
            Ok(new_rev)
        }
        CommandMutation::CreateGovernedAction {
            action_id,
            origin_kind,
            control_principal_id,
            project_id,
            scope_key,
            run_id,
            step_id,
            invocation_id,
            attempt_id,
            replay_worker_id,
            action_type,
            payload_hash,
            reconcile_strategy,
            policy_revision,
            resource_lease_id,
            approval_scope,
            initial_state,
            subject,
        } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            ensure_scope_not_frozen_tx(tx, *project_id, scope_key.as_deref(), *run_id)?;
            if let Some((skind, sid)) = subject.as_ref() {
                ensure_subject_not_blocked_tx(tx, *skind, sid, *project_id)?;
            }
            let epoch_i64 = i64::try_from(daemon.daemon_epoch.as_u64()).unwrap_or(1);
            let auth_ref: Option<String> = if matches!(
                initial_state,
                ActionState::Authorized
                    | ActionState::Dispatching
                    | ActionState::InFlight
                    | ActionState::ObservedSuccess
                    | ActionState::OutcomeUnknown
            ) {
                Some(format!("auth-{action_id}"))
            } else {
                None
            };
            tx.execute(
                "INSERT INTO actions
                 (action_id, origin_kind, control_principal_id, project_id, scope_key, run_id, step_id,
                  invocation_id, attempt_id, replay_worker_id, action_type, payload_hash,
                  authorization_ref, policy_revision, resource_lease_id, approval_scope,
                  subject_kind, subject_id, authority_generation, daemon_epoch, fence,
                  state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20, 0, ?21, ?22, NULL, 1, ?23, ?23)",
                rusqlite::params![
                    action_id.to_string(),
                    origin_kind,
                    control_principal_id,
                    project_id.map(|p| p.to_string()),
                    scope_key,
                    run_id.map(|r| r.to_string()),
                    step_id.map(|s| s.to_string()),
                    invocation_id.map(|i| i.to_string()),
                    attempt_id.map(|a| a.to_string()),
                    replay_worker_id.map(|w| w.to_string()),
                    action_type,
                    payload_hash.as_prefixed_str(),
                    auth_ref,
                    policy_revision,
                    resource_lease_id,
                    approval_scope,
                    subject.as_ref().map(|(k, _)| k.as_str()),
                    subject.as_ref().map(|(_, s)| s.as_str()),
                    daemon.authority_generation.to_string(),
                    epoch_i64,
                    initial_state.to_string(),
                    reconcile_strategy,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(Revision::new(1))
        }
        CommandMutation::ReconcileAction {
            action_id,
            caller,
            outcome,
            evidence_digest,
            receipt_hash,
            reason,
            expected_source_epoch,
            expected_source_fence,
        } => tx_reconcile_action(
            tx,
            now_ms,
            daemon.daemon_epoch,
            expected_rev,
            cmd_id,
            *action_id,
            caller,
            *outcome,
            *evidence_digest,
            *receipt_hash,
            reason,
            *expected_source_epoch,
            *expected_source_fence,
            generated_events,
        ),
        CommandMutation::ResolveActionOutcome {
            action_id,
            caller,
            disposition,
            reason,
            evidence_ref,
        } => tx_resolve_action_outcome(
            tx,
            now_ms,
            expected_rev,
            cmd_id,
            receipt_id,
            *action_id,
            caller,
            *disposition,
            reason,
            evidence_ref.as_deref(),
            generated_events,
        ),
        CommandMutation::ResolveActionApproval {
            action_id,
            caller,
            approval_scope,
            payload_digest,
            decision,
            reason,
        } => tx_resolve_action_approval(
            tx,
            config.id_generator.as_ref(),
            now_ms,
            expected_rev,
            cmd_id,
            receipt_id,
            *action_id,
            caller,
            approval_scope,
            *payload_digest,
            *decision,
            reason,
            generated_events,
        ),
        CommandMutation::AcknowledgeAlert {
            alert_id,
            target_state,
        } => tx_transition_alert(tx, now_ms, expected_rev, cmd_id, alert_id, *target_state),
        CommandMutation::PurgeHistory { request } => {
            ensure_creation_rev(expected_rev, cmd_id)?;
            let existing: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM retention_purges WHERE purge_id = ?1",
                    [envelope.aggregate_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if existing != 0 {
                return Err(StoreError::RevisionConflict {
                    current_revision: Revision::new(1),
                    expected_revision: expected_rev,
                    command_id: Some(cmd_id),
                });
            }
            let rep = tx_execute_retention_purge(
                tx,
                config,
                now_ms,
                Some(receipt_id),
                Some(envelope.aggregate_id),
                request,
            )?;
            if rep.suspended_clock_uncertain {
                return Err(StoreError::RecoveryInProgress(
                    "automatic retention suspended: clock uncertainty".to_owned(),
                ));
            }
            if !rep.suspended_clock_uncertain {
                let mut payload = Map::new();
                payload.insert("purge_id".to_owned(), Value::String(rep.purge_id));
                payload.insert(
                    "events_tombstoned_count".to_owned(),
                    Value::Number((rep.events_tombstoned_count as u64).into()),
                );
                payload.insert(
                    "phase".to_owned(),
                    Value::String("logical_purge_committed".to_owned()),
                );
                payload.insert("objects_purged_count".to_owned(), Value::Number(0.into()));
                payload.insert(
                    "objects_deletion_prepared_count".to_owned(),
                    Value::Number((rep.objects_purged_count as u64).into()),
                );
                payload.insert(
                    "physical_deletion_state".to_owned(),
                    Value::String("pending".to_owned()),
                );
                payload.insert("wal_checkpoint_truncated".to_owned(), Value::Bool(false));
                generated_events.push(EventAppendSpec {
                    event_type: "HistoryPurged".to_owned(),
                    aggregate_type: AggregateKind::RetentionPurge,
                    aggregate_id: envelope.aggregate_id,
                    aggregate_revision: Revision::new(1),
                    project_id: None,
                    correlation_id: cmd_id.as_uuid(),
                    causation_kind: EventCausationKind::Command,
                    causation_id: None,
                    payload,
                    sensitivity: Sensitivity::Project,
                    outbox_destinations: vec![],
                });
            }
            Ok(Revision::new(1))
        }
    }
}

fn ensure_creation_rev(expected_rev: Revision, cmd_id: CommandId) -> Result<(), StoreError> {
    if expected_rev != Revision::INITIAL {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::INITIAL,
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }
    Ok(())
}

pub(crate) fn parse_run_state(s: &str) -> Result<RunState, StoreError> {
    match s {
        "prepared" => Ok(RunState::Prepared),
        "running" => Ok(RunState::Running),
        "paused" => Ok(RunState::Paused),
        "blocked" => Ok(RunState::Blocked),
        "cancelling" => Ok(RunState::Cancelling),
        "succeeded" => Ok(RunState::Succeeded),
        "failed" => Ok(RunState::Failed),
        "cancelled" => Ok(RunState::Cancelled),
        other => Err(StoreError::InvariantViolation(format!(
            "état de run inconnu '{other}'"
        ))),
    }
}

pub(crate) fn read_command_receipt_inspection(
    conn: &Connection,
    actor_id: &ActorId,
    command_id: CommandId,
) -> Result<ReceiptInspectionOutcome, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let row: Option<(String, String, String, String, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT authority_generation, receipt_id, command_digest, state, response_state, response_json, committed_at_ms
             FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![actor_id.as_str(), command_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((auth_str, rcpt_id_str, digest_str, state_str, resp_state, resp_json, committed_ms)) =
        row
    else {
        return Err(StoreError::NotFound(format!(
            "reçu absent pour ({actor_id}, {command_id})"
        )));
    };

    let authority_generation = auth_str.parse::<AuthorityGeneration>()?;
    let lineage_status = if authority_generation == daemon.authority_generation {
        LineageStatus::Current
    } else {
        LineageStatus::Historical
    };

    if resp_state == "tombstone" {
        let state = match state_str.as_str() {
            "accepted" => ReceiptState::Accepted,
            "completed" => ReceiptState::Completed,
            "blocked" => ReceiptState::Blocked,
            _ => ReceiptState::Rejected,
        };
        return Ok(ReceiptInspectionOutcome::Tombstone {
            receipt_id: rcpt_id_str.parse::<ReceiptId>()?,
            command_id,
            authority_generation,
            lineage_status,
            command_digest: digest_str.parse::<Sha256Digest>()?,
            state,
            committed_at_ms: TimestampMs::new(committed_ms),
        });
    }

    let json_str = resp_json
        .ok_or_else(|| StoreError::InvariantViolation("reçu full sans response_json".to_owned()))?;
    let receipt: CommandReceipt = serde_json::from_str(&json_str)?;
    Ok(ReceiptInspectionOutcome::Full(CommandReceiptInspection {
        receipt,
        authority_generation,
        lineage_status,
        provenance_ref: Some(format!("sqlite:command_receipts:{rcpt_id_str}")),
    }))
}

pub(crate) fn tx_purge_expired_command_receipts(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    clock_uncertain: bool,
) -> Result<ReceiptPurgeReport, StoreError> {
    if clock_uncertain {
        return Ok(ReceiptPurgeReport::SuspendedClockUncertain);
    }

    let count = tx
        .execute(
            "UPDATE command_receipts
             SET response_state = 'tombstone', response_json = NULL
             WHERE response_state = 'full' AND ?1 >= response_retained_until_ms",
            [now_ms.as_i64()],
        )
        .map_err(StoreError::from_rusqlite)?;

    Ok(ReceiptPurgeReport::Purged {
        tombstoned_count: count,
    })
}
