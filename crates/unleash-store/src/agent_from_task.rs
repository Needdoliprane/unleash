//! Resolve a public `CreateAgent` request against a ready workflow task.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::Value;
use unleash_domain::{
    DaemonMode, MissionId, ProjectId, Revision, RunId, Sha256Digest, TaskId, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ReceiptState, compute_command_digest_v2};

use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, PreparedAgentCreation, StoreConfig, StoreError, UnleashStore,
    agent_creation::{COMMAND, commit_agent_plan, result_ref},
    commands::{classify_command_replay, read_daemon_state_conn},
    policies_secrets::require_human_decider,
    workflow_run_plan::{self, ExecutionScope, RunManifestTarget, RunPlan, invalid, parse_hash},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTarget {
    mission_id: MissionId,
    mission_revision: Revision,
    task_id: TaskId,
    task_revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyTask {
    run: RunId,
    version: String,
    definition: Sha256Digest,
    bundle: Sha256Digest,
    mission_revision: Revision,
    project_revision: Revision,
    task_revision: Revision,
    scope: ExecutionScope,
    node_id: String,
    step_attempt: u32,
    contract: String,
}

/// Store-resolved assignment and workflow inputs for public agent creation.
#[derive(Debug, Clone)]
pub struct AgentWorkflowPreparation {
    caller: CallerAuthority,
    command_digest: Sha256Digest,
    task: ReadyTask,
    run_plan: RunPlan,
    agent: PreparedAgentCreation,
}

impl UnleashStore {
    /// Register an agent and its first assignment using a task's bound role.
    ///
    /// The client supplies the task and mission revisions. Role, scope and
    /// assignment identity come from the retained plan and the Store.
    /// # Errors
    /// Refuses a stale task or mission, missing workflow inputs, conflicting
    /// identities and commands outside the authenticated owner's scope.
    pub fn create_agent_from_task(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        let target = decode_target(&envelope, &caller)?;
        let digest = compute_command_digest_v2(&envelope)?;
        let replay = || {
            self.read_snapshot(|conn, _| {
                let daemon = read_daemon_state_conn(conn)?;
                classify_command_replay(conn, &daemon, &caller.actor_id, &envelope, digest)
            })
        };
        if let Some(receipt) = replay()? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        let prepared = match self.read_snapshot(|conn, _| {
            prepare_agent(conn, self.config(), &envelope, &caller, &target, digest)
        }) {
            Ok(prepared) => prepared,
            Err(error) => {
                return replay()?.map_or(Err(error), |receipt| {
                    Ok(CommitOutcome::IdempotentReplay(receipt))
                });
            }
        };
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id,
            envelope,
            target_aggregate: AggregateKind::Agent,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: Some(result_ref(prepared.agent.assignment_id)),
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateAgentFromWorkflowTask(Box::new(prepared)),
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }
}

fn decode_target(
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<AgentTarget, StoreError> {
    require_human_decider(caller, COMMAND)?;
    let target: AgentTarget = serde_json::from_value(Value::Object(envelope.payload.clone()))
        .map_err(|_| invalid("invalid CreateAgent task payload"))?;
    if envelope.command_type != COMMAND
        || envelope.aggregate_id.is_nil()
        || envelope.project_id.is_none()
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        || target.mission_id.as_uuid().is_nil()
        || target.task_id.as_uuid().is_nil()
    {
        return Err(invalid("invalid assigned agent creation coordinates"));
    }
    Ok(target)
}

pub(crate) fn validate_identity(req: &CommitCommandRequest) -> Result<(), StoreError> {
    let CommandMutation::CreateAgentFromWorkflowTask(prepared) = &req.mutation else {
        return if req.envelope.command_type == COMMAND
            && req.envelope.payload.contains_key("task_id")
            && !matches!(req.mutation, CommandMutation::CreateAssignedAgent { .. })
        {
            Err(invalid(
                "task-based agent creation requires its retained binding",
            ))
        } else {
            Ok(())
        };
    };
    let target = decode_target(&req.envelope, &prepared.caller)?;
    if req.actor_id != prepared.caller.actor_id
        || compute_command_digest_v2(&req.envelope)? != prepared.command_digest
        || target.mission_id != prepared.agent.mission_id
        || target.mission_revision != prepared.agent.mission_revision
        || target.task_id != prepared.agent.task_id
        || target.task_revision != prepared.agent.task_revision
        || req.target_aggregate != AggregateKind::Agent
        || req.receipt_state != ReceiptState::Completed
        || req.result_ref.as_deref() != Some(result_ref(prepared.agent.assignment_id).as_str())
        || req.operation_id.is_some()
        || req.retention_days_override.is_some()
        || req.fence_assertion.is_some()
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent workflow agent creation"));
    }
    Ok(())
}

fn prepare_agent(
    conn: &Connection,
    config: &StoreConfig,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    selection: &AgentTarget,
    command_digest: Sha256Digest,
) -> Result<AgentWorkflowPreparation, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.authority_generation != envelope.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "agent creation requires a ready daemon".to_owned(),
        ));
    }
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("agent project required"))?;
    let task = read_ready_task(conn, envelope, project, caller, selection)?;
    let mission = selection.mission_id.to_string();
    let plan = workflow_run_plan::load_run_manifest(
        conn,
        config,
        &RunManifestTarget {
            run: task.run,
            mission: &mission,
            version: &task.version,
            definition_hash: task.definition,
            bundle_hash: task.bundle,
        },
    )?;
    verify_mission_and_scopes(&plan, &task, envelope, project)?;
    plan.revalidate_start(conn)?;
    let (role_version_id, role_content_hash) = plan.resolve_task_role(
        (conn, config),
        &task.node_id,
        task.step_attempt,
        &task.scope,
        &task.contract,
    )?;
    let agent = PreparedAgentCreation {
        daemon_epoch: daemon.daemon_epoch,
        assignment_id: config.id_generator.new_uuid(),
        mission_id: selection.mission_id,
        mission_revision: task.mission_revision,
        task_id: selection.task_id,
        task_revision: task.task_revision,
        scope_id: task.scope.scope_id.clone(),
        scope_revision: task.scope.scope_revision,
        role_version_id,
        role_content_hash,
    };
    Ok(AgentWorkflowPreparation {
        caller: caller.clone(),
        command_digest,
        task,
        run_plan: plan,
        agent,
    })
}

fn read_ready_task(
    conn: &Connection,
    envelope: &CommandEnvelope,
    project: ProjectId,
    caller: &CallerAuthority,
    selection: &AgentTarget,
) -> Result<ReadyTask, StoreError> {
    let (run, version, definition, bundle, mission_rev, project_rev, task_rev, scope, scope_rev,
        node, step_attempt, contract) = conn.query_row(
        "SELECT r.run_id,r.workflow_version_id,v.content_hash,r.effective_bundle_hash,
                m.revision,p.revision,t.revision,t.scope_id,t.scope_revision,
                s.definition_node_id,s.step_attempt,t.contract_version_id
           FROM tasks t
           JOIN step_instances s ON s.step_id=t.step_id AND s.run_id=t.run_id
           JOIN workflow_runs r ON r.run_id=t.run_id AND r.mission_id=t.mission_id
           JOIN workflow_versions v ON v.workflow_version_id=r.workflow_version_id
           JOIN missions m ON m.mission_id=t.mission_id
           JOIN projects p ON p.project_id=m.project_id
           JOIN principals u ON u.principal_id=m.owner_user_id
          WHERE t.task_id=?1 AND m.mission_id=?2 AND m.project_id=?3 AND u.actor_id=?4
            AND p.state='active' AND m.owner_kind='user' AND m.state IN ('ready','active')
            AND u.principal_kind='user' AND u.state='active'
            AND r.state='running' AND r.purpose='operational' AND s.state='ready' AND t.state='ready'",
        params![selection.task_id.to_string(), selection.mission_id.to_string(),
                project.to_string(), caller.actor_id.as_str()],
        |row| Ok((
            row.get::<_, String>(0)?, row.get(1)?, row.get::<_, String>(2)?,
            row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?, row.get(7)?, row.get::<_, i64>(8)?,
            row.get(9)?, row.get(10)?, row.get(11)?,
        )),
    ).optional()?.ok_or_else(|| StoreError::NotFound(
        "ready task unavailable to this project and mission owner".to_owned(),
    ))?;
    let task_rev = stored_revision(task_rev)?;
    let mission_rev = stored_revision(mission_rev)?;
    require_revision(envelope, mission_rev, selection.mission_revision)?;
    require_revision(envelope, task_rev, selection.task_revision)?;
    Ok(ReadyTask {
        run: run.parse()?,
        version,
        definition: parse_hash(&definition)?,
        bundle: parse_hash(&bundle)?,
        mission_revision: mission_rev,
        project_revision: stored_revision(project_rev)?,
        task_revision: task_rev,
        scope: ExecutionScope {
            scope_id: scope,
            scope_revision: stored_revision(scope_rev)?,
        },
        node_id: node,
        step_attempt,
        contract,
    })
}

fn stored_revision(value: i64) -> Result<Revision, StoreError> {
    u64::try_from(value).map(Revision::new).map_err(|_| {
        StoreError::InvariantViolation("invalid stored task or project revision".to_owned())
    })
}

fn require_revision(
    envelope: &CommandEnvelope,
    current: Revision,
    expected: Revision,
) -> Result<(), StoreError> {
    if current != expected {
        return Err(StoreError::RevisionConflict {
            current_revision: current,
            expected_revision: expected,
            command_id: Some(envelope.command_id),
        });
    }
    Ok(())
}

fn verify_mission_and_scopes(
    plan: &RunPlan,
    task: &ReadyTask,
    envelope: &CommandEnvelope,
    project: ProjectId,
) -> Result<(), StoreError> {
    require_revision(envelope, task.mission_revision, plan.mission_revision)?;
    plan.require_current_project_scopes(project, task.project_revision, envelope.command_id)
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    context: (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &CommandEnvelope,
    prepared: &AgentWorkflowPreparation,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let selection = decode_target(envelope, &prepared.caller)?;
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("agent project required"))?;
    let task = read_ready_task(tx, envelope, project, &prepared.caller, &selection)?;
    verify_mission_and_scopes(&prepared.run_plan, &task, envelope, project)?;
    if task != prepared.task {
        return Err(StoreError::EvidenceUnavailable(
            "ready task binding changed before creation".to_owned(),
        ));
    }
    workflow_run_plan::require_run_manifest(tx, &task.run.to_string(), task.bundle)?;
    prepared.run_plan.revalidate_start(tx)?;
    commit_agent_plan(
        tx,
        context,
        envelope,
        (&prepared.caller, &prepared.agent),
        true,
        events,
    )
}
