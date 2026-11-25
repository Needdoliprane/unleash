//! Durable persistence of a task activation selected by the workflow reducer.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    ActorId, DaemonEpoch, DaemonMode, IterationId, Revision, RunId, Sha256Digest, StepId, TaskId,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ReceiptState, Sensitivity, from_json_str_strict,
};

use crate::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, DaemonStateRow,
    EventAppendSpec, StoreError, UnleashStore,
    commands::ensure_scope_not_frozen_tx,
    workflow_run_plan::{definition_model_policy, require_agent_task_allowed},
};

pub(crate) const COMMAND: &str = "ActivateWorkflowTask";
const ACTOR: &str = "system:harness";

/// Exact plan produced by the harness for a ready task node on an existing run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTaskActivationPlan {
    /// Daemon epoch in which the reducer resolved the activation.
    pub daemon_epoch: DaemonEpoch,
    /// Digest of the immutable workflow definition pinned by the run.
    pub definition_hash: Sha256Digest,
    /// Digest of the run's effective plan.
    pub effective_bundle_hash: Sha256Digest,
    /// Identifier of the task node in the pinned definition.
    pub definition_node_id: String,
    /// Optional current iteration, whose vector is resolved by the writer.
    pub iteration_id: Option<IterationId>,
    /// Logical attempt of the step, distinct from any worker attempt.
    pub step_attempt: u32,
    /// Identity allocated for the step.
    pub step_id: StepId,
    /// Identity allocated for this task.
    pub task_id: TaskId,
    /// Exact contract version selected by the harness binding.
    pub contract_version_id: String,
    /// Scope resolved by the harness.
    pub scope_id: String,
    /// Revision of the resolved scope.
    pub scope_revision: Revision,
}

impl UnleashStore {
    /// Persist a ready task and its step from a reducer decision.
    ///
    /// The envelope targets the run and its expected revision. The reducer must
    /// have resolved readiness, the binding and the applicable budget before
    /// supplying this internal plan. This operation does not assign an agent
    /// or reserve execution capacity.
    ///
    /// # Errors
    /// Refuses invalid, stale or conflicting activation plans and missing
    /// definitions. Each task consumes one activation, including retries.
    /// If the run has no established budget, a new activation is refused.
    /// An agent task of a run whose engaged definition sets
    /// `model_policy=forbidden` is refused before any effect (REQ-MODEL-018).
    pub fn activate_workflow_task(
        &self,
        mut envelope: CommandEnvelope,
    ) -> Result<CommitOutcome, StoreError> {
        let plan = decode_plan(&envelope)?;
        envelope.actor_ref.get_or_insert_with(|| ACTOR.to_owned());
        self.commit_command(CommitCommandRequest {
            actor_id: ActorId::new(ACTOR)?,
            envelope,
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: Some(format!("task:{}", plan.task_id)),
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ActivateWorkflowTask,
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }
}

pub(crate) fn validate_identity(req: &CommitCommandRequest) -> Result<bool, StoreError> {
    let envelope = &req.envelope;
    let activation = matches!(req.mutation, CommandMutation::ActivateWorkflowTask);
    if activation != (envelope.command_type == COMMAND) {
        return Err(invalid("activation command and mutation must match"));
    }
    if !activation {
        return Ok(false);
    }
    let plan = decode_plan(envelope)?;
    if envelope.actor_ref.as_deref() != Some(ACTOR)
        || req.actor_id.as_str() != ACTOR
        || envelope.project_id.is_none()
        || envelope.aggregate_id.is_nil()
        || req.target_aggregate != AggregateKind::WorkflowRun
        || req.receipt_state != ReceiptState::Completed
        || req.result_ref.as_deref() != Some(format!("task:{}", plan.task_id).as_str())
        || req.operation_id.is_some()
        || req.retention_days_override.is_some()
        || req.fence_assertion.is_some()
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent workflow task activation command"));
    }
    Ok(true)
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn decode_plan(envelope: &CommandEnvelope) -> Result<WorkflowTaskActivationPlan, StoreError> {
    let plan: WorkflowTaskActivationPlan =
        serde_json::from_value(Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("invalid workflow task activation plan"))?;
    if plan.definition_node_id.trim().is_empty()
        || plan.contract_version_id.trim().is_empty()
        || plan.scope_id.trim().is_empty()
        || plan.step_attempt == 0
        || plan.step_id.as_uuid().is_nil()
        || plan.task_id.as_uuid().is_nil()
    {
        return Err(invalid(
            "empty or invalid workflow task activation coordinates",
        ));
    }
    Ok(plan)
}

struct RunBinding {
    mission_id: String,
    version_id: String,
    revision: i64,
    has_manifest: bool,
    activation_available: Option<bool>,
}

fn verify_run(
    conn: &Connection,
    envelope: &CommandEnvelope,
    plan: &WorkflowTaskActivationPlan,
) -> Result<RunBinding, StoreError> {
    let project = envelope.project_id.map(|project| project.to_string());
    let (mut run, definition_hash, bundle_hash): (RunBinding, String, String) = conn.query_row(
        "SELECT r.mission_id,r.workflow_version_id,r.revision,v.content_hash,r.effective_bundle_hash,
                r.step_activations_used < r.max_step_activations
           FROM workflow_runs r
           JOIN missions m ON m.mission_id=r.mission_id
           JOIN projects p ON p.project_id=m.project_id
           JOIN workflow_versions v ON v.workflow_version_id=r.workflow_version_id
          WHERE r.run_id=?1 AND m.project_id=?2 AND p.state='active'
            AND m.state IN ('ready','active') AND r.state='running'
            AND (v.visibility IN ('published','deprecated')
                 OR (r.purpose='evaluation' AND v.visibility='qualification_only'))",
        params![envelope.aggregate_id.to_string(), project],
        |row| Ok((
            RunBinding {
                mission_id: row.get(0)?,
                version_id: row.get(1)?,
                revision: row.get(2)?,
                has_manifest: false,
                activation_available: row.get(5)?,
            },
            row.get(3)?,
            row.get(4)?,
        )),
    ).optional()?.ok_or_else(|| StoreError::NotFound("running workflow unavailable in this project".to_owned()))?;
    let revision =
        Revision::new(u64::try_from(run.revision).map_err(|_| {
            StoreError::InvariantViolation("invalid workflow run revision".to_owned())
        })?);
    if revision != envelope.expected_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: envelope.expected_revision,
            command_id: Some(envelope.command_id),
        });
    }
    if definition_hash != plan.definition_hash.as_prefixed_str()
        || bundle_hash != plan.effective_bundle_hash.as_prefixed_str()
    {
        return Err(StoreError::EvidenceUnavailable(
            "workflow definition or effective plan changed".to_owned(),
        ));
    }
    let retained: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_references r JOIN object_records o USING(object_hash)
         WHERE r.owner_kind='workflow_version' AND r.owner_id=?1 AND r.source_field='content_hash'
           AND r.object_hash=?2 AND r.state='live' AND o.state='live')",
        params![run.version_id, definition_hash],
        |row| row.get(0),
    )?;
    if !retained {
        return Err(StoreError::EvidenceUnavailable(
            "run definition is not retained by the pinned workflow version".to_owned(),
        ));
    }
    run.has_manifest =
        crate::workflow_run_plan::has_committed_plan(conn, &envelope.aggregate_id.to_string())?;
    if run.has_manifest {
        crate::workflow_run_plan::require_run_manifest(
            conn,
            &envelope.aggregate_id.to_string(),
            plan.effective_bundle_hash,
        )?;
    }
    Ok(run)
}

/// Verify file contents outside the transaction. The writer checks the same
/// run revision, immutable hashes and live source reference again.
pub(crate) fn verify_definition(
    store: &UnleashStore,
    req: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let plan = decode_plan(&req.envelope)?;
    store.read_snapshot(|conn, _| {
        let run = verify_run(conn, &req.envelope, &plan)?;
        let bytes = crate::objects::read_verified_object_bytes_conn(
            conn,
            store.config(),
            plan.definition_hash,
        )?;
        let json = std::str::from_utf8(&bytes)
            .map_err(|_| invalid("workflow definition must be UTF-8"))?;
        let definition: Value = from_json_str_strict(json)?;
        let node = verify_task_node(&definition, &plan.definition_node_id, plan.step_attempt)?;
        // REQ-MODEL-018: the policy is read from the verified bytes of the engaged
        // definition (INV-018); the writer then checks that same definition hash.
        require_agent_task_allowed(
            definition_model_policy(&definition)?,
            &plan.definition_node_id,
        )?;
        if run.has_manifest {
            crate::workflow_run_plan::verify_activation(
                conn,
                store.config(),
                &run.mission_id,
                &run.version_id,
                &plan,
                node,
            )?;
        }
        Ok(())
    })
}

pub(crate) fn verify_task_node<'definition>(
    definition: &'definition Value,
    node_id: &str,
    step_attempt: u32,
) -> Result<&'definition Value, StoreError> {
    if !matches!(
        definition.get("schema_version").and_then(Value::as_str),
        Some("unleash.workflow.v1" | "unleash.workflow.v1.1" | "unleash.workflow.v1.2")
    ) {
        return Err(invalid("unsupported workflow definition"));
    }
    let mut pending = vec![
        definition
            .get("body")
            .ok_or_else(|| invalid("missing workflow body"))?,
    ];
    let mut matching = None;
    while let Some(node) = pending.pop() {
        let kind = node.get("kind").and_then(Value::as_str);
        if node.get("id").and_then(Value::as_str) == Some(node_id) {
            if matching.is_some() || kind != Some("task") {
                return Err(invalid(
                    "workflow activation must reference one exact task node",
                ));
            }
            let maximum = node
                .get("max_attempts")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("invalid task attempt limit"))?;
            if u64::from(step_attempt) > maximum {
                return Err(invalid("step attempt above the pinned definition limit"));
            }
            matching = Some(node);
        }
        push_workflow_children(&mut pending, node);
    }
    matching.ok_or_else(|| invalid("task missing from pinned definition"))
}

pub(crate) fn push_workflow_children<'a>(pending: &mut Vec<&'a Value>, node: &'a Value) {
    // Traverse DSL child slots only. Task inputs may themselves be JSON
    // objects but cannot be interpreted as nested execution nodes.
    let kind = node.get("kind").and_then(Value::as_str);
    match kind {
        Some("sequence") => pending.extend(
            node.get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        ),
        Some("parallel") => pending.extend(
            node.get("branches")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        ),
        Some("choice") => {
            pending.extend(
                node.get("cases")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|case| case.get("then")),
            );
            pending.extend(node.get("otherwise"));
        }
        Some("repeat" | "for_each") => {
            pending.extend(node.get("body"));
            if kind == Some("for_each") {
                pending.extend(node.get("after_batch"));
            }
        }
        _ => {}
    }
}

fn resolve_iteration(
    tx: &Transaction<'_>,
    run: RunId,
    iteration: Option<IterationId>,
) -> Result<String, StoreError> {
    let Some(iteration) = iteration else {
        return Ok("[]".to_owned());
    };
    tx.query_row(
        "SELECT iteration_vector FROM workflow_iterations WHERE iteration_id=?1 AND run_id=?2",
        params![iteration.to_string(), run.to_string()],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| invalid("iteration does not belong to the activated run"))
}

pub(crate) fn activate_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    envelope: &CommandEnvelope,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "task activation requires a ready daemon".to_owned(),
        ));
    }
    let plan = decode_plan(envelope)?;
    if plan.daemon_epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "workflow task was resolved in another daemon epoch".to_owned(),
        });
    }
    let run_id = RunId::from_uuid(envelope.aggregate_id);
    let run = verify_run(tx, envelope, &plan)?;
    ensure_scope_not_frozen_tx(tx, envelope.project_id, Some(&plan.scope_id), Some(run_id))?;
    let vector = resolve_iteration(tx, run_id, plan.iteration_id)?;
    let iterations: Vec<Value> = serde_json::from_str(&vector)?;
    let revision = run.revision.checked_add(1).ok_or_else(|| {
        StoreError::InvariantViolation("workflow run revision overflow".to_owned())
    })?;

    match run.activation_available {
        Some(true) => {}
        Some(false) => return Err(StoreError::WorkflowActivationBudgetExhausted),
        None => {
            return Err(StoreError::EvidenceUnavailable(
                "workflow activation budget is unavailable for this run".to_owned(),
            ));
        }
    }
    tx.execute(
        "UPDATE workflow_runs SET revision=?1,step_activations_used=step_activations_used+1 WHERE run_id=?2",
        params![revision, run_id.to_string()],
    )?;
    insert_ready_task(tx, &plan, run_id, &run.mission_id, &vector)?;
    let revision =
        Revision::new(u64::try_from(revision).map_err(|_| {
            StoreError::InvariantViolation("invalid workflow run revision".to_owned())
        })?);
    events.push(node_activated_event(
        envelope,
        &run.version_id,
        revision,
        &plan,
        &iterations,
    )?);
    Ok(revision)
}

pub(crate) fn node_activated_event(
    envelope: &CommandEnvelope,
    version_id: &str,
    revision: Revision,
    plan: &WorkflowTaskActivationPlan,
    iterations: &[Value],
) -> Result<EventAppendSpec, StoreError> {
    Ok(EventAppendSpec {
        event_type: "NodeActivated".to_owned(),
        aggregate_type: AggregateKind::WorkflowRun,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: revision,
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: serde_json::from_value(json!({
            "run_id": envelope.aggregate_id,
            "workflow_version_id": version_id,
            "definition_hash": plan.definition_hash,
            "definition_node_id": plan.definition_node_id,
            "step_id": plan.step_id,
            "task_id": plan.task_id,
            "iteration_vector": iterations,
            "step_attempt": plan.step_attempt,
            "state": "ready",
        }))?,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec!["clients".to_owned(), "harness".to_owned()],
    })
}

pub(crate) fn insert_ready_task(
    tx: &Transaction<'_>,
    plan: &WorkflowTaskActivationPlan,
    run_id: RunId,
    mission_id: &str,
    iteration_vector: &str,
) -> Result<(), StoreError> {
    let run = run_id.to_string();
    let step = plan.step_id.to_string();
    let task = plan.task_id.to_string();
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM step_instances
         WHERE step_id=?1 OR (run_id=?2 AND definition_node_id=?3 AND iteration_vector=?4 AND step_attempt=?5))
         OR EXISTS(SELECT 1 FROM tasks WHERE task_id=?6)",
        params![step, run, plan.definition_node_id, iteration_vector, plan.step_attempt, task],
        |row| row.get(0),
    )?;
    if exists {
        return Err(invalid(
            "logical activation or task identity already exists",
        ));
    }
    let scope_revision = i64::try_from(plan.scope_revision.as_u64())
        .map_err(|_| invalid("task scope revision outside storage range"))?;
    tx.execute(
        "INSERT INTO step_instances
         (step_id,run_id,iteration_id,definition_node_id,iteration_vector,step_attempt,state,revision)
         VALUES (?1,?2,?3,?4,?5,?6,'ready',1)",
        params![step, run, plan.iteration_id.map(|id| id.to_string()), plan.definition_node_id, iteration_vector, plan.step_attempt],
    )?;
    tx.execute(
        "INSERT INTO tasks (task_id,mission_id,run_id,step_id,contract_version_id,scope_id,scope_revision,state,revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'ready',1)",
        params![task, mission_id, run, step, plan.contract_version_id, plan.scope_id, scope_revision],
    )?;
    Ok(())
}
