//! Create a durable agent and its first mission assignment from a prepared task.

use rusqlite::{OptionalExtension, Transaction, named_params, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    AgentId, AgentLevel, AgentState, DaemonEpoch, DaemonMode, MissionId, ProjectId, Revision,
    RunId, Sha256Digest, TaskId, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, EventCausationKind, ReceiptState, Sensitivity};
use uuid::Uuid;

use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, StoreConfig, StoreError, UnleashStore,
    commands::{NewAgent, ensure_scope_not_frozen_tx, insert_agent_and_session},
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_human_decider},
};

pub(crate) const COMMAND: &str = "CreateAgent";

/// Internal preparation for a worker identity in a human-owned mission.
///
/// The resolver selects the role version and the scope from the mission and
/// the activated task. The writer checks these exact coordinates before
/// creating the identity. This plan belongs to the internal Store interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedAgentCreation {
    /// Daemon epoch in which the plan was resolved.
    pub daemon_epoch: DaemonEpoch,
    /// New stable assignment identity.
    pub assignment_id: Uuid,
    /// Mission that owns the assignment.
    pub mission_id: MissionId,
    /// Observed revision of that mission.
    pub mission_revision: Revision,
    /// Activated task that establishes the assignment's scope.
    pub task_id: TaskId,
    /// Observed revision of the task.
    pub task_revision: Revision,
    /// Exact execution scope of the task.
    pub scope_id: String,
    /// Exact revision of the execution scope.
    pub scope_revision: Revision,
    /// Published role version selected by the resolver.
    pub role_version_id: String,
    /// Digest of the selected role's published document.
    pub role_content_hash: Sha256Digest,
}

impl UnleashStore {
    /// Create an eligible N agent, its primary session and its initial assignment.
    ///
    /// The internal envelope carries the exact prepared plan. An identical
    /// redelivery returns the retained receipt. `AssignTask` and the scheduler
    /// handle invocation queueing and execution capacity.
    ///
    /// # Errors
    /// Refuses an invalid or stale plan, an unavailable role, a mission outside
    /// the current owner's authority, or a duplicate identity.
    pub fn create_assigned_agent(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        let plan = decode_plan(&envelope)?;
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: AggregateKind::Agent,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: Some(result_ref(plan.assignment_id)),
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateAssignedAgent { caller },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }
}

fn decode_plan(envelope: &CommandEnvelope) -> Result<PreparedAgentCreation, StoreError> {
    let plan: PreparedAgentCreation =
        serde_json::from_value(Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("invalid prepared agent creation"))?;
    if plan.assignment_id.is_nil()
        || plan.mission_id.as_uuid().is_nil()
        || plan.task_id.as_uuid().is_nil()
        || plan.scope_id.trim().is_empty()
        || plan.role_version_id.trim().is_empty()
    {
        return Err(invalid(
            "agent creation requires exact mission, task, scope and role",
        ));
    }
    Ok(plan)
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

pub(crate) fn result_ref(assignment: Uuid) -> String {
    format!("mission-assignment:{assignment}")
}

pub(crate) fn validate_identity(request: &CommitCommandRequest) -> Result<(), StoreError> {
    let CommandMutation::CreateAssignedAgent { caller } = &request.mutation else {
        if request.envelope.command_type == COMMAND
            && request.envelope.payload.contains_key("assignment_id")
        {
            return Err(invalid(
                "agent creation with assignment requires the prepared port",
            ));
        }
        return Ok(());
    };
    let envelope = &request.envelope;
    require_human_decider(caller, COMMAND)?;
    let plan = decode_plan(envelope)?;
    if envelope.command_type != COMMAND
        || envelope.aggregate_id.is_nil()
        || envelope.project_id.is_none()
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        || request.actor_id != caller.actor_id
        || request.target_aggregate != AggregateKind::Agent
        || request.receipt_state != ReceiptState::Completed
        || request.result_ref.as_deref() != Some(result_ref(plan.assignment_id).as_str())
        || request.operation_id.is_some()
        || request.retention_days_override.is_some()
        || request.fence_assertion.is_some()
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent prepared agent creation"));
    }
    Ok(())
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    context: (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let plan = decode_plan(envelope)?;
    commit_agent_plan(tx, context, envelope, (caller, &plan), false, events)
}

pub(crate) fn commit_agent_plan(
    tx: &Transaction<'_>,
    (config, daemon, now): (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &CommandEnvelope,
    (caller, plan): (&CallerAuthority, &PreparedAgentCreation),
    allow_deprecated: bool,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "assigned agent creation requires a ready daemon".to_owned(),
        ));
    }
    if plan.daemon_epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "agent creation was prepared in another daemon epoch".to_owned(),
        });
    }
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("assigned agent creation requires a project"))?;
    let agent = AgentId::from_uuid(envelope.aggregate_id);
    ensure_new_agent(tx, envelope, agent)?;
    let run = resolve_task(tx, project, caller, plan)?;
    ensure_scope_not_frozen_tx(tx, Some(project), Some(&plan.scope_id), Some(run))?;
    for (kind, id) in [
        (SubjectKind::Actor, caller.actor_id.as_str()),
        (SubjectKind::Workflow, &run.to_string()),
    ] {
        ensure_subject_not_blocked_tx(tx, kind, id, Some(project))?;
    }
    validate_role(tx, project, plan, allow_deprecated)?;

    let session = insert_agent_and_session(
        tx,
        config,
        now,
        &NewAgent {
            agent_id: agent,
            level: AgentLevel::N,
            organization_parent_id: None,
            created_by_agent_id: None,
            project_id: Some(project),
            state: AgentState::Eligible,
        },
    )?;
    tx.execute(
        "INSERT INTO mission_assignments
            (assignment_id, mission_id, agent_id, role_version_id, scope_id,
             scope_revision, assignment_epoch, state, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 'active', 1)",
        params![
            plan.assignment_id.to_string(),
            plan.mission_id.to_string(),
            agent.to_string(),
            plan.role_version_id,
            plan.scope_id,
            sql_revision(plan.scope_revision)?,
        ],
    )?;
    tx.execute(
        "INSERT INTO object_references
            (reference_id, object_hash, owner_kind, owner_id, source_field, state)
         VALUES (?1, ?2, 'mission_assignment', ?3, 'role_version_id', 'live')",
        params![
            config.id_generator.new_uuid().to_string(),
            plan.role_content_hash.as_prefixed_str(),
            plan.assignment_id.to_string(),
        ],
    )?;
    events.push(EventAppendSpec {
        event_type: "AgentCreated".to_owned(),
        aggregate_type: AggregateKind::Agent,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: Revision::new(1),
        project_id: Some(project),
        correlation_id: envelope.request_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: serde_json::from_value(json!({
            "agent_id": agent,
            "session_id": session,
            "assignment_id": plan.assignment_id,
            "assignment_epoch": 1,
            "mission_id": plan.mission_id,
            "task_id": plan.task_id,
            "role_version_id": plan.role_version_id,
            "scope_id": plan.scope_id,
            "scope_revision": plan.scope_revision,
        }))?,
        sensitivity: Sensitivity::Project,
        outbox_destinations: Vec::new(),
    });
    Ok(Revision::new(1))
}

fn ensure_new_agent(
    tx: &Transaction<'_>,
    envelope: &CommandEnvelope,
    agent: AgentId,
) -> Result<(), StoreError> {
    let revision = tx
        .query_row(
            "SELECT revision FROM agents WHERE agent_id=?1",
            [agent.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if revision.is_some() || envelope.expected_revision != Revision::INITIAL {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(
                revision
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| {
                        StoreError::InvariantViolation("invalid stored agent revision".to_owned())
                    })?
                    .unwrap_or(0),
            ),
            expected_revision: envelope.expected_revision,
            command_id: Some(envelope.command_id),
        });
    }
    Ok(())
}

fn resolve_task(
    tx: &Transaction<'_>,
    project: ProjectId,
    caller: &CallerAuthority,
    plan: &PreparedAgentCreation,
) -> Result<RunId, StoreError> {
    let run = tx
        .query_row(
            "SELECT t.run_id
         FROM tasks t
         JOIN step_instances s ON s.step_id=t.step_id AND s.run_id=t.run_id
         JOIN workflow_runs r ON r.run_id=t.run_id AND r.mission_id=t.mission_id
         JOIN missions m ON m.mission_id=t.mission_id
         JOIN projects p ON p.project_id=m.project_id
         JOIN principals u ON u.principal_id=m.owner_user_id
         WHERE t.task_id=:task AND t.revision=:task_revision AND t.state='ready'
            AND t.scope_id=:scope AND t.scope_revision=:scope_revision AND s.state='ready'
            AND r.state='running' AND m.mission_id=:mission AND m.revision=:mission_revision
            AND m.owner_kind='user' AND m.state IN ('ready','active')
            AND p.project_id=:project AND p.state='active'
            AND u.actor_id=:actor AND u.principal_kind='user' AND u.state='active'",
            named_params! {
                ":task": plan.task_id.to_string(),
                ":task_revision": sql_revision(plan.task_revision)?,
                ":scope": plan.scope_id,
                ":scope_revision": sql_revision(plan.scope_revision)?,
                ":mission": plan.mission_id.to_string(),
                ":mission_revision": sql_revision(plan.mission_revision)?,
                ":project": project.to_string(),
                ":actor": caller.actor_id.as_str(),
            },
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| invalid("mission or task preparation unavailable or changed"))?;
    run.parse().map_err(|_| {
        StoreError::InvariantViolation("invalid run identity in assigned task".to_owned())
    })
}

fn validate_role(
    tx: &Transaction<'_>,
    project: ProjectId,
    plan: &PreparedAgentCreation,
    allow_deprecated: bool,
) -> Result<(), StoreError> {
    let found = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM registry_versions v
            JOIN object_records o ON o.object_hash=v.content_hash AND o.state='live'
            JOIN object_references rr ON rr.object_hash=o.object_hash AND rr.state='live'
                AND rr.owner_kind='registry_version' AND rr.owner_id=v.registry_version_id
                AND rr.source_field='content_hash'
            WHERE v.registry_version_id=?1 AND v.content_hash=?2 AND v.entry_kind='role'
                AND (v.publication_state='published' OR (?4 AND v.publication_state='deprecated'))
                AND (v.scope_kind='global'
                     OR (v.scope_kind='project' AND v.scope_owner_key=?3))
        )",
        params![
            plan.role_version_id,
            plan.role_content_hash.as_prefixed_str(),
            project.to_string(),
            allow_deprecated,
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !found {
        return Err(invalid(
            "published role version unavailable for this project",
        ));
    }
    Ok(())
}

fn sql_revision(revision: Revision) -> Result<i64, StoreError> {
    i64::try_from(revision.as_u64()).map_err(|_| invalid("revision out of range"))
}
