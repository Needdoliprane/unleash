//! Atomic persistence of an assignment of a ready workflow task (REQ-AGENT-002).

use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};
use serde_json::{Map, Value, json};
use unleash_domain::{
    AgentId, DaemonEpoch, DaemonMode, InvocationId, Revision, RunId, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    AssignTaskPayload, CommandEnvelope, EventCausationKind, ReceiptState, Sensitivity,
    canonical_sha256, from_json_str_strict,
};
use uuid::Uuid;

use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, StoreConfig, StoreError, UnleashStore,
    commands::{ensure_scope_not_frozen_tx, read_daemon_state_conn},
    objects::read_verified_object_bytes_conn,
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_human_decider},
};

pub(crate) const COMMAND: &str = "AssignTask";

/// Exact preparation resolved by the harness for a task assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTaskAssignment {
    /// Daemon epoch that resolved the assignment.
    pub daemon_epoch: DaemonEpoch,
    /// Existing active mission assignment.
    pub assignment_id: Uuid,
    /// Epoch of the selected mission assignment.
    pub assignment_epoch: u64,
    /// Revision of the selected mission assignment.
    pub assignment_revision: Revision,
    /// The durable activation event for the task.
    pub trigger_event_id: Uuid,
    /// Exact immutable bundle already persisted by the resolver.
    pub effective_bundle_hash: Sha256Digest,
    /// Identity allocated by the harness for the invocation.
    pub invocation_id: InvocationId,
}

impl UnleashStore {
    /// Queue a ready workflow task for its authenticated human owner.
    ///
    /// Only the agent ID comes from the command payload. The trusted resolver
    /// supplies the existing mission assignment and published bundle. Cancellation
    /// of the caller does not retract a committed queue entry. The same command
    /// can retrieve its receipt after an interruption.
    ///
    /// # Errors
    /// Refuses an unavailable or stale preparation, a different owner, a busy
    /// agent, or a task that is no longer ready.
    pub fn assign_task(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
        preparation: PreparedTaskAssignment,
    ) -> Result<CommitOutcome, StoreError> {
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: AggregateKind::Task,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: Some(format!("invocation:{}", preparation.invocation_id)),
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::AssignTask {
                caller,
                preparation,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }
}

pub(crate) struct AssignmentCommand<'a> {
    pub envelope: &'a CommandEnvelope,
    pub caller: &'a CallerAuthority,
    pub preparation: &'a PreparedTaskAssignment,
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn agent_id(envelope: &CommandEnvelope) -> Result<AgentId, StoreError> {
    let payload: AssignTaskPayload =
        serde_json::from_value(Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("task assignment requires exactly one agent ID"))?;
    if payload.agent_id.as_uuid().is_nil() {
        return Err(invalid("task assignment requires a non-nil agent ID"));
    }
    Ok(payload.agent_id)
}

pub(crate) fn validate_assignment_envelope(
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<AgentId, StoreError> {
    require_human_decider(caller, COMMAND)?;
    if envelope.command_type != COMMAND
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        || envelope.project_id.is_none()
        || envelope.aggregate_id.is_nil()
    {
        return Err(invalid("inconsistent task assignment envelope"));
    }
    agent_id(envelope)
}

pub(crate) fn validate_identity(request: &CommitCommandRequest) -> Result<bool, StoreError> {
    let envelope = &request.envelope;
    let CommandMutation::AssignTask {
        caller,
        preparation,
    } = &request.mutation
    else {
        if envelope.command_type == COMMAND {
            return Err(invalid(
                "task assignment command requires a prepared assignment",
            ));
        }
        return Ok(false);
    };
    validate_assignment_envelope(envelope, caller)?;
    if request.actor_id != caller.actor_id
        || preparation.assignment_id.is_nil()
        || preparation.trigger_event_id.is_nil()
        || preparation.invocation_id.as_uuid().is_nil()
        || request.target_aggregate != AggregateKind::Task
        || request.receipt_state != ReceiptState::Accepted
        || request.result_ref.as_deref()
            != Some(format!("invocation:{}", preparation.invocation_id).as_str())
        || request.operation_id.is_some()
        || request.retention_days_override.is_some()
        || request.fence_assertion.is_some()
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent task assignment command"));
    }
    Ok(true)
}

struct Binding {
    mission: String,
    run: String,
    step: String,
    contract_version: String,
    scope: String,
    scope_revision: i64,
    task_state: String,
    task_revision: i64,
    step_state: String,
    step_revision: i64,
    assignment_epoch: i64,
    assignment_revision: i64,
    session: String,
    role_version: String,
    workflow_version: String,
    profile_version: String,
    workflow_entry: bool,
    configured_profile_digest: Option<Sha256Digest>,
    policy_hash: String,
    bundle_schema: i64,
    context: String,
    context_hash: String,
    context_schema: i64,
    context_revision: i64,
}

impl Binding {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            mission: row.get("mission_id")?,
            run: row.get("run_id")?,
            step: row.get("step_id")?,
            contract_version: row.get("contract_version_id")?,
            scope: row.get("scope_id")?,
            scope_revision: row.get("scope_revision")?,
            task_state: row.get("task_state")?,
            task_revision: row.get("task_revision")?,
            step_state: row.get("step_state")?,
            step_revision: row.get("step_revision")?,
            assignment_epoch: row.get("assignment_epoch")?,
            assignment_revision: row.get("assignment_revision")?,
            session: row.get("session_id")?,
            role_version: row.get("role_version_id")?,
            workflow_version: row.get("workflow_version_id")?,
            profile_version: row.get("requested_model_profile_version_id")?,
            workflow_entry: row.get("workflow_entry")?,
            configured_profile_digest: None,
            policy_hash: row.get("allowed_model_policy_hash")?,
            bundle_schema: row.get("bundle_schema")?,
            context: row.get("manifest_id")?,
            context_hash: row.get("context_hash")?,
            context_schema: row.get("context_schema")?,
            context_revision: row.get("base_session_revision")?,
        })
    }
}

const READY_BINDING_SQL: &str = "SELECT t.mission_id,t.run_id,t.step_id,t.contract_version_id,t.scope_id,t.scope_revision,
                t.state AS task_state,t.revision AS task_revision,s.state AS step_state,
                s.revision AS step_revision,a.assignment_epoch,a.revision AS assignment_revision,
                sess.session_id,a.role_version_id,r.workflow_version_id,b.requested_model_profile_version_id,
                b.allowed_model_policy_hash,b.schema_version AS bundle_schema,c.manifest_id,
                c.content_hash AS context_hash,c.schema_version AS context_schema,c.base_session_revision,
                cr.command_type='StartRun' AS workflow_entry
         FROM tasks t
         JOIN workflow_runs r ON r.run_id=t.run_id AND r.mission_id=t.mission_id
         JOIN step_instances s ON s.step_id=t.step_id AND s.run_id=t.run_id
         JOIN missions m ON m.mission_id=t.mission_id
         JOIN projects p ON p.project_id=m.project_id
         JOIN principals owner ON owner.principal_id=m.owner_user_id
            AND owner.actor_id=:actor AND owner.principal_kind='user' AND owner.state='active'
         JOIN workflow_versions w ON w.workflow_version_id=r.workflow_version_id
         JOIN mission_assignments a ON a.mission_id=m.mission_id AND a.assignment_id=:assignment
            AND a.agent_id=:agent AND a.state='active' AND a.scope_id=t.scope_id
            AND a.scope_revision=t.scope_revision
         JOIN agents g ON g.agent_id=a.agent_id AND g.project_id=m.project_id
            AND g.state IN ('eligible','active')
         JOIN sessions sess ON sess.agent_id=g.agent_id AND sess.state='open'
         JOIN effective_bundles b ON b.bundle_hash=:bundle AND b.content_object_hash=b.bundle_hash
            AND b.role_version_id=a.role_version_id AND b.workflow_version_id=r.workflow_version_id
            AND b.scope_id=t.scope_id AND b.scope_revision=t.scope_revision
         JOIN context_manifests c ON c.manifest_id=b.initial_context_manifest_id
            AND c.session_id=sess.session_id AND c.base_session_revision=sess.revision
         JOIN object_records bo ON bo.object_hash=b.content_object_hash AND bo.state='live'
         JOIN object_records co ON co.object_hash=c.content_hash AND co.state='live'
         JOIN events e ON e.event_id=:trigger AND e.stream_id=:stream AND e.project_id=m.project_id
            AND e.aggregate_type='workflow_run' AND e.aggregate_id=r.run_id
            AND e.event_type='NodeActivated' AND e.payload_state='full'
            AND json_extract(e.payload_json,'$.task_id')=t.task_id
            AND json_extract(e.payload_json,'$.run_id')=t.run_id
            AND json_extract(e.payload_json,'$.step_id')=t.step_id
            AND json_extract(e.payload_json,'$.workflow_version_id')=r.workflow_version_id
            AND json_extract(e.payload_json,'$.definition_hash')=w.content_hash
         JOIN command_receipts cr ON cr.receipt_id=e.causation_receipt_id
            AND cr.aggregate_id=r.run_id AND cr.state='completed' AND cr.authority_generation=:authority
            AND (cr.command_type='ActivateWorkflowTask' OR (
                cr.command_type='StartRun' AND EXISTS (
                   SELECT 1 FROM events started
                   WHERE started.causation_receipt_id=cr.receipt_id
                     AND started.event_type='RunStarted' AND started.payload_state='full'
                     AND started.stream_id=e.stream_id AND started.project_id=e.project_id
                     AND started.aggregate_type='workflow_run' AND started.aggregate_id=r.run_id
                     AND started.aggregate_revision=e.aggregate_revision AND started.sequence<e.sequence
                     AND json_extract(started.payload_json,'$.run_id')=r.run_id
                     AND json_extract(started.payload_json,'$.workflow_version_id')=r.workflow_version_id
                     AND json_extract(started.payload_json,'$.definition_hash')=w.content_hash
                     AND json_extract(started.payload_json,'$.effective_bundle_hash')=r.effective_bundle_hash
                )
            ))
         WHERE t.task_id=:task AND p.project_id=:project AND p.state='active'
            AND m.owner_kind='user' AND m.state IN ('ready','active') AND r.state='running'
            AND (w.visibility IN ('published','deprecated')
                 OR (r.purpose='evaluation' AND w.visibility='qualification_only'))
            AND EXISTS (SELECT 1 FROM object_references ref
                WHERE ref.object_hash=bo.object_hash AND ref.owner_kind='effective_bundle'
                  AND ref.owner_id=b.bundle_hash AND ref.source_field='content_object_hash'
                  AND ref.state='live')
            AND EXISTS (SELECT 1 FROM object_references ref
                WHERE ref.object_hash=co.object_hash AND ref.owner_kind='context_manifest'
                  AND ref.owner_id=c.manifest_id AND ref.source_field='content_hash'
                  AND ref.state='live')";

fn read_binding(
    conn: &Connection,
    daemon: &DaemonStateRow,
    command: &AssignmentCommand<'_>,
    assignee: AgentId,
) -> Result<Binding, StoreError> {
    let envelope = command.envelope;
    let prep = command.preparation;
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "task assignment requires a ready daemon".to_owned(),
        ));
    }
    if prep.daemon_epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "task assignment was resolved in another daemon epoch".to_owned(),
        });
    }
    let mut row = conn
        .query_row(
            READY_BINDING_SQL,
            named_params! {
                ":actor": command.caller.actor_id.as_str(),
                ":assignment": prep.assignment_id.to_string(),
                ":agent": assignee.to_string(),
                ":bundle": prep.effective_bundle_hash.as_prefixed_str(),
                ":trigger": prep.trigger_event_id.to_string(),
                ":stream": daemon.stream_id.to_string(),
                ":authority": daemon.authority_generation.to_string(),
                ":task": envelope.aggregate_id.to_string(),
                ":project": envelope.project_id.map(|id| id.to_string()),
            },
            Binding::read,
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(
                "ready task, mission assignment or pinned bundle unavailable for this caller"
                    .to_owned(),
            )
        })?;
    let current = revision(row.task_revision)?;
    if current != envelope.expected_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: current,
            expected_revision: envelope.expected_revision,
            command_id: Some(envelope.command_id),
        });
    }
    if u64::try_from(row.assignment_epoch) != Ok(prep.assignment_epoch)
        || revision(row.assignment_revision)? != prep.assignment_revision
    {
        return Err(StoreError::StaleFence {
            message: "mission assignment epoch or revision changed".to_owned(),
        });
    }
    if row.task_state != "ready" || row.step_state != "ready" {
        return Err(invalid("task and workflow step must both be ready"));
    }
    if row.workflow_entry {
        row.configured_profile_digest = Some(configured_profile_digest(
            conn,
            daemon,
            &row.profile_version,
            envelope.project_id,
        )?);
    }
    Ok(row)
}

fn configured_profile_digest(
    conn: &Connection,
    daemon: &DaemonStateRow,
    profile_ref: &str,
    project: Option<unleash_domain::ProjectId>,
) -> Result<Sha256Digest, StoreError> {
    let (id, version): (String, i64) = conn
        .query_row(
            "SELECT p.profile_id,p.profile_version
               FROM provider_profiles p
               JOIN command_receipts c ON c.receipt_id=p.receipt_id AND c.actor_id=p.actor_id
                 AND c.aggregate_id=p.profile_id AND c.authority_generation=?2
                 AND c.command_type='SetProviderProfile' AND c.state IN ('accepted','completed')
               JOIN object_references ref ON ref.owner_kind='provider_profile'
                 AND ref.owner_id=p.profile_version_ref AND ref.source_field='configuration'
                 AND ref.object_hash=p.config_digest AND ref.state='live'
               JOIN object_records o ON o.object_hash=ref.object_hash AND o.state='live'
              WHERE p.profile_version_ref=?1
                AND ((p.scope_kind='user' AND p.scope_owner_key='default')
                  OR (p.scope_kind='project' AND p.scope_owner_key=?3))",
            params![
                profile_ref,
                daemon.authority_generation.to_string(),
                project.map(|project| project.to_string())
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::EvidenceUnavailable(
                "configured task profile unavailable in this project and authority".to_owned(),
            )
        })?;
    let profile = crate::provider_profiles::resolve(
        conn,
        id.parse().map_err(|_| {
            StoreError::InvariantViolation("invalid configured task profile ID".to_owned())
        })?,
        revision(version)?.as_u64(),
    )?
    .ok_or_else(|| {
        StoreError::EvidenceUnavailable("configured task profile version missing".to_owned())
    })?;
    if profile.profile_version_ref != profile_ref {
        return Err(StoreError::EvidenceUnavailable(
            "configured task profile reference changed".to_owned(),
        ));
    }
    Ok(profile.configuration_digest)
}

fn revision(value: i64) -> Result<Revision, StoreError> {
    u64::try_from(value).map(Revision::new).map_err(|_| {
        StoreError::InvariantViolation("invalid task or assignment revision".to_owned())
    })
}

fn next_revision(value: i64) -> Result<i64, StoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| StoreError::InvariantViolation("task or step revision overflow".to_owned()))
}

/// Verify stored bytes before submitting the command to the single writer.
/// The writer resolves the same association again and retains both objects.
pub(crate) fn verify_published(
    store: &UnleashStore,
    request: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let CommandMutation::AssignTask {
        caller,
        preparation,
    } = &request.mutation
    else {
        return Err(invalid("missing task assignment preparation"));
    };
    let command = AssignmentCommand {
        envelope: &request.envelope,
        caller,
        preparation,
    };
    let assignee = agent_id(command.envelope)?;
    store.read_snapshot(|conn, _| {
        let daemon = read_daemon_state_conn(conn)?;
        let binding = read_binding(conn, &daemon, &command, assignee)?;
        crate::task_bundle::require_selected_bundle(conn, &command, assignee)?;
        let bundle = read_document(conn, store.config(), preparation.effective_bundle_hash)?;
        verify_bundle_document(&command, assignee, &binding, &bundle)?;
        if let Some(digest) = binding.configured_profile_digest {
            read_verified_object_bytes_conn(conn, store.config(), digest)?;
        }
        let context_hash = binding.context_hash.parse().map_err(|_| {
            StoreError::InvariantViolation("invalid context manifest digest".to_owned())
        })?;
        let context = read_document(conn, store.config(), context_hash)?;
        if !includes_fields(
            &context,
            &json!({
                "schema_version": binding.context_schema,
                "session_id": binding.session,
                "base_session_revision": binding.context_revision,
            }),
        ) {
            return Err(StoreError::EvidenceUnavailable(
                "context manifest does not identify the prepared session revision".to_owned(),
            ));
        }
        Ok(())
    })
}

fn verify_bundle_document(
    command: &AssignmentCommand<'_>,
    assignee: AgentId,
    binding: &Binding,
    bundle: &Value,
) -> Result<(), StoreError> {
    let preparation = command.preparation;
    if binding
        .configured_profile_digest
        .is_some_and(|digest| bundle.get("requested_model_profile_digest") != Some(&json!(digest)))
    {
        return Err(StoreError::EvidenceUnavailable(
            "configured task profile digest differs from its selected version".to_owned(),
        ));
    }
    let bundle_identity = json!({
        "schema_version": binding.bundle_schema,
        "agent_id": assignee,
        "session_id": binding.session,
        "trigger_event_id": preparation.trigger_event_id,
        "mission_id": binding.mission,
        "run_id": binding.run,
        "step_id": binding.step,
        "task_id": command.envelope.aggregate_id,
        "role_version_id": binding.role_version,
        "workflow_version_id": binding.workflow_version,
        "requested_model_profile_version_id": binding.profile_version,
        "allowed_model_policy_hash": binding.policy_hash,
        "scope_id": binding.scope,
        "scope_revision": binding.scope_revision,
        "initial_context_manifest_id": binding.context,
    });
    if canonical_sha256(bundle)? != preparation.effective_bundle_hash
        || !includes_fields(bundle, &bundle_identity)
        || !bundle
            .get("contract_version_ids")
            .and_then(Value::as_array)
            .is_some_and(|versions| {
                versions.iter().all(Value::is_string)
                    && versions
                        .iter()
                        .any(|version| version.as_str() == Some(&binding.contract_version))
            })
    {
        return Err(StoreError::EvidenceUnavailable(
            "bundle does not identify the prepared task, assignment and context".to_owned(),
        ));
    }

    Ok(())
}

pub(crate) fn verify_staged_bundle(
    conn: &Connection,
    daemon: &DaemonStateRow,
    command: &AssignmentCommand<'_>,
    assignee: AgentId,
    document: &Value,
) -> Result<(), StoreError> {
    let binding = read_binding(conn, daemon, command, assignee)?;
    verify_bundle_document(command, assignee, &binding, document)
}

pub(crate) fn read_document(
    conn: &Connection,
    config: &StoreConfig,
    hash: Sha256Digest,
) -> Result<Value, StoreError> {
    let bytes = read_verified_object_bytes_conn(conn, config, hash)?;
    let json = std::str::from_utf8(&bytes).map_err(|_| {
        StoreError::EvidenceUnavailable("assignment document is not UTF-8".to_owned())
    })?;
    Ok(from_json_str_strict(json)?)
}

fn includes_fields(document: &Value, identity: &Value) -> bool {
    identity.as_object().is_some_and(|fields| {
        fields
            .iter()
            .all(|(key, value)| document.get(key) == Some(value))
    })
}

pub(crate) fn queue_tx(
    tx: &Transaction<'_>,
    (config, daemon, now): (&StoreConfig, &DaemonStateRow, TimestampMs),
    command: &AssignmentCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let envelope = command.envelope;
    let prep = command.preparation;
    let assignee = agent_id(envelope)?;
    let binding = read_binding(tx, daemon, command, assignee)?;
    crate::task_bundle::require_selected_bundle(tx, command, assignee)?;
    let run = binding.run.parse::<RunId>().map_err(|_| {
        StoreError::InvariantViolation("invalid run identity for ready task".to_owned())
    })?;
    ensure_scope_not_frozen_tx(tx, envelope.project_id, Some(&binding.scope), Some(run))?;
    for (kind, id) in [
        (SubjectKind::Actor, command.caller.actor_id.as_str()),
        (SubjectKind::Agent, &assignee.to_string()),
        (SubjectKind::Workflow, &binding.run),
    ] {
        ensure_subject_not_blocked_tx(tx, kind, id, envelope.project_id)?;
    }
    let has_invocation: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM invocations WHERE agent_id=?1
            AND state NOT IN ('completed','failed','cancelled'))",
        [assignee.to_string()],
        |row| row.get(0),
    )?;
    if has_invocation {
        return Err(StoreError::CapacityBlocked(
            "agent already has an active invocation".to_owned(),
        ));
    }

    let task_revision = next_revision(binding.task_revision)?;
    let step_revision = next_revision(binding.step_revision)?;
    let task = envelope.aggregate_id.to_string();
    let invocation = prep.invocation_id.to_string();
    tx.execute(
        "INSERT INTO invocations
         (invocation_id,agent_id,session_id,trigger_event_id,task_id,run_id,step_id,
          effective_bundle_hash,context_manifest_id,state,admission_class,revision,created_at_ms,queued_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued','work',1,?10,?10)",
        params![
            invocation, assignee.to_string(), binding.session, prep.trigger_event_id.to_string(),
            task, binding.run, binding.step, prep.effective_bundle_hash.as_prefixed_str(),
            binding.context, now.as_i64(),
        ],
    )?;
    tx.execute(
        "UPDATE tasks SET state='queued',revision=?1 WHERE task_id=?2",
        params![task_revision, task],
    )?;
    tx.execute(
        "UPDATE step_instances SET state='queued',revision=?1 WHERE step_id=?2",
        params![step_revision, binding.step],
    )?;
    let references = [
        (
            "effective_bundle_hash",
            prep.effective_bundle_hash.as_prefixed_str(),
        ),
        ("context_manifest_id", binding.context_hash.clone()),
    ]
    .into_iter()
    .chain(
        binding
            .configured_profile_digest
            .map(|digest| ("requested_model_profile_version_id", digest.to_string())),
    );
    for (field, hash) in references {
        tx.execute(
            "INSERT INTO object_references (reference_id,object_hash,owner_kind,owner_id,source_field,state)
             VALUES (?1,?2,'invocation',?3,?4,'live')",
            params![config.id_generator.new_uuid().to_string(), hash, invocation, field],
        )?;
    }
    let task_revision = revision(task_revision)?;
    append_events(events, command, &binding, assignee, task_revision)?;
    Ok(task_revision)
}

fn append_events(
    events: &mut Vec<EventAppendSpec>,
    command: &AssignmentCommand<'_>,
    binding: &Binding,
    assignee: AgentId,
    task_revision: Revision,
) -> Result<(), StoreError> {
    let envelope = command.envelope;
    let prep = command.preparation;
    let mut payload: Map<String, Value> = serde_json::from_value(json!({
        "task_id": envelope.aggregate_id,
        "run_id": binding.run,
        "step_id": binding.step,
        "agent_id": assignee,
        "invocation_id": prep.invocation_id,
        "assignment_id": prep.assignment_id,
        "assignment_epoch": prep.assignment_epoch,
        "assignment_revision": prep.assignment_revision,
        "state": "queued",
    }))?;
    events.push(EventAppendSpec {
        event_type: "TaskAssigned".to_owned(),
        aggregate_type: AggregateKind::Task,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: task_revision,
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: payload.clone(),
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec!["clients".to_owned(), "harness".to_owned()],
    });
    payload.extend(serde_json::from_value::<Map<String, Value>>(json!({
        "session_id": binding.session,
        "trigger_event_id": prep.trigger_event_id,
        "effective_bundle_hash": prep.effective_bundle_hash,
        "context_manifest_id": binding.context,
        "admission_class": "work",
    }))?);
    events.push(EventAppendSpec {
        event_type: "InvocationQueued".to_owned(),
        aggregate_type: AggregateKind::Invocation,
        aggregate_id: prep.invocation_id.as_uuid(),
        aggregate_revision: Revision::new(1),
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Restricted,
        outbox_destinations: vec!["harness".to_owned()],
    });
    Ok(())
}
