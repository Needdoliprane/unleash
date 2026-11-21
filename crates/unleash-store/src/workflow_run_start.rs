//! Start an operational run from its retained plan and materialize its first task.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use unleash_domain::{
    DaemonEpoch, DaemonMode, Revision, RunEvent, RunId, Sha256Digest, StepId, TaskId, TimestampMs,
    transition_run,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, EventCausationKind, ReceiptState, Sensitivity,
    compute_command_digest_v2,
};

use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, StoreConfig, StoreError, UnleashStore,
    WorkflowTaskActivationPlan,
    commands::{
        classify_command_replay, ensure_scope_not_frozen_tx, parse_run_state,
        read_daemon_state_conn,
    },
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_human_decider},
    workflow_activation::{insert_ready_task, node_activated_event},
    workflow_run_plan::{self, EntryTask, RunManifestTarget, RunPlan, invalid},
};

pub(crate) const COMMAND: &str = "StartRun";

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredRun {
    mission: String,
    version: String,
    definition: Sha256Digest,
    bundle: Sha256Digest,
    revision: Revision,
    mission_revision: Revision,
    project_revision: Revision,
    state: unleash_domain::RunState,
    prepared_at_ms: i64,
}

/// Captured workflow inputs for a revision-checked start transaction.
#[derive(Debug, Clone)]
pub struct PreparedRunStart {
    caller: CallerAuthority,
    epoch: DaemonEpoch,
    digest: Sha256Digest,
    run: StoredRun,
    plan: RunPlan,
    entry: EntryTask,
}

impl UnleashStore {
    /// Start a prepared operational workflow and persist the first ready task.
    ///
    /// The client supplies the run coordinate; the writer uses the exact
    /// retained definition and selects the initial task from its entry node.
    /// # Errors
    /// Refuses invalid or stale commands, missing evidence, unavailable scopes
    /// and workflows whose entry needs a different reducer decision or exceeds
    /// the activation budget.
    pub fn start_prepared_workflow_run(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        require_human_decider(&caller, COMMAND)?;
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        check_envelope(&envelope, &caller)?;
        let digest = compute_command_digest_v2(&envelope)?;
        let replay = || self.read_snapshot(|conn, _| replay(conn, &envelope, &caller, digest));
        if let Some(receipt) = replay()? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        let prepared = self.read_snapshot(|conn, _| {
            prepare_start(conn, self.config(), &envelope, &caller, digest)
        });
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                return replay()?.map_or(Err(error), |receipt| {
                    Ok(CommitOutcome::IdempotentReplay(receipt))
                });
            }
        };
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id,
            result_ref: Some(result_ref(&envelope)),
            envelope,
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::StartPreparedWorkflowRun(Box::new(prepared)),
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }
}

fn result_ref(envelope: &CommandEnvelope) -> String {
    format!("workflow-run:{}", envelope.aggregate_id)
}

fn replay(
    conn: &Connection,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    digest: Sha256Digest,
) -> Result<Option<CommandReceipt>, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    classify_command_replay(conn, &daemon, &caller.actor_id, envelope, digest)
}

fn check_envelope(envelope: &CommandEnvelope, caller: &CallerAuthority) -> Result<(), StoreError> {
    if envelope.command_type != COMMAND
        || envelope.aggregate_id.is_nil()
        || envelope.project_id.is_none()
        || !envelope.payload.is_empty()
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
    {
        return Err(invalid("invalid operational workflow start envelope"));
    }
    Ok(())
}

pub(crate) fn validate_identity(req: &CommitCommandRequest) -> Result<(), StoreError> {
    let CommandMutation::StartPreparedWorkflowRun(start) = &req.mutation else {
        return if req.envelope.command_type == COMMAND {
            Err(invalid("run start requires its verified workflow plan"))
        } else {
            Ok(())
        };
    };
    check_envelope(&req.envelope, &start.caller)?;
    if req.actor_id != start.caller.actor_id
        || compute_command_digest_v2(&req.envelope)? != start.digest
        || req.target_aggregate != AggregateKind::WorkflowRun
        || req.receipt_state != ReceiptState::Completed
        || req.result_ref.as_deref() != Some(result_ref(&req.envelope).as_str())
        || req.operation_id.is_some()
        || req.retention_days_override.is_some()
        || req.fence_assertion.is_some()
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent prepared run start"));
    }
    Ok(())
}

fn read_run(
    conn: &Connection,
    daemon: &DaemonStateRow,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<StoredRun, StoreError> {
    if daemon.authority_generation != envelope.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "run start requires a ready daemon".to_owned(),
        ));
    }
    let (
        mission,
        version,
        definition,
        bundle,
        run_rev,
        mission_rev,
        project_rev,
        state,
        epoch_start,
    ) = conn
        .query_row(
            "SELECT m.mission_id,r.workflow_version_id,v.content_hash,r.effective_bundle_hash,
                    r.revision,m.revision,p.revision,r.state,r.started_at_ms
               FROM workflow_runs r JOIN missions m ON m.mission_id=r.mission_id
               JOIN projects p ON p.project_id=m.project_id
               JOIN principals u ON u.principal_id=m.owner_user_id
               JOIN workflow_versions v ON v.workflow_version_id=r.workflow_version_id
              WHERE r.run_id=?1 AND m.project_id=?2 AND p.state='active'
                AND m.state IN ('ready','active') AND m.owner_kind='user'
                AND u.actor_id=?3 AND u.principal_kind='user' AND u.state='active'
                AND r.purpose='operational'",
            params![
                envelope.aggregate_id.to_string(),
                envelope.project_id.map(|id| id.to_string()),
                caller.actor_id.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(
                "operational workflow unavailable for this project and owner".to_owned(),
            )
        })?;
    let revision = decode_revision(run_rev)?;
    require_revision(envelope, revision, envelope.expected_revision)?;
    let state = parse_run_state(&state)?;
    transition_run(state, RunEvent::StartRun)?;
    Ok(StoredRun {
        mission,
        version,
        definition: workflow_run_plan::parse_hash(&definition)?,
        bundle: workflow_run_plan::parse_hash(&bundle)?,
        revision,
        mission_revision: decode_revision(mission_rev)?,
        project_revision: decode_revision(project_rev)?,
        state,
        prepared_at_ms: epoch_start,
    })
}

fn decode_revision(value: i64) -> Result<Revision, StoreError> {
    u64::try_from(value)
        .map(Revision::new)
        .map_err(|_| StoreError::InvariantViolation("negative workflow start revision".to_owned()))
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

fn prepare_start(
    conn: &Connection,
    config: &StoreConfig,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    digest: Sha256Digest,
) -> Result<PreparedRunStart, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let run = read_run(conn, &daemon, envelope, caller)?;
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("run project required"))?;
    let plan = workflow_run_plan::load_run_manifest(
        conn,
        config,
        &RunManifestTarget {
            run: RunId::from_uuid(envelope.aggregate_id),
            mission: &run.mission,
            version: &run.version,
            definition_hash: run.definition,
            bundle_hash: run.bundle,
        },
    )?;
    require_revision(envelope, run.mission_revision, plan.mission_revision)?;
    plan.require_current_project_scopes(project, run.project_revision, envelope.command_id)?;
    let entry = plan.entry_task(conn, config)?;
    Ok(PreparedRunStart {
        caller: caller.clone(),
        epoch: daemon.daemon_epoch,
        digest,
        run,
        plan,
        entry,
    })
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    (config, daemon, now): (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &CommandEnvelope,
    prepared: &PreparedRunStart,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if prepared.epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "run start crosses daemon epochs".to_owned(),
        });
    }
    let run = read_run(tx, daemon, envelope, &prepared.caller)?;
    let id = RunId::from_uuid(envelope.aggregate_id);
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("run project required"))?;
    require_revision(
        envelope,
        run.mission_revision,
        prepared.plan.mission_revision,
    )?;
    prepared.plan.require_current_project_scopes(
        project,
        run.project_revision,
        envelope.command_id,
    )?;
    if run != prepared.run {
        return Err(StoreError::EvidenceUnavailable(
            "run coordinates changed before start".to_owned(),
        ));
    }
    ensure_scope_not_frozen_tx(
        tx,
        Some(project),
        Some(&prepared.entry.scope.scope_id),
        Some(id),
    )?;
    ensure_subject_not_blocked_tx(
        tx,
        SubjectKind::Actor,
        prepared.caller.actor_id.as_str(),
        Some(project),
    )?;
    workflow_run_plan::require_run_manifest(tx, &id.to_string(), run.bundle)?;
    prepared.plan.revalidate_start(tx)?;
    record_start(tx, config, now, envelope, prepared, events)
}

fn record_start(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    start: &PreparedRunStart,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let run = &start.run;
    let id = RunId::from_uuid(envelope.aggregate_id);
    if now.as_i64() < run.prepared_at_ms {
        return Err(StoreError::EvidenceUnavailable(
            "run start clock precedes preparation".to_owned(),
        ));
    }
    let next = transition_run(run.state, RunEvent::StartRun)?;
    let revision_i64 = i64::try_from(run.revision.as_u64())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            StoreError::InvariantViolation("workflow run revision overflow".to_owned())
        })?;
    let revision = decode_revision(revision_i64)?;
    let entry = &start.entry;
    if entry.activations > entry.max_step_activations {
        return Err(StoreError::WorkflowActivationBudgetExhausted);
    }
    let task = WorkflowTaskActivationPlan {
        daemon_epoch: start.epoch,
        definition_hash: run.definition,
        effective_bundle_hash: run.bundle,
        definition_node_id: entry.node_id.clone(),
        iteration_id: None,
        step_attempt: 1,
        step_id: StepId::from_uuid(config.id_generator.new_uuid()),
        task_id: TaskId::from_uuid(config.id_generator.new_uuid()),
        contract_version_id: entry.contract_version_id.clone(),
        scope_id: entry.scope.scope_id.clone(),
        scope_revision: entry.scope.scope_revision,
    };
    tx.execute(
        "UPDATE workflow_runs
            SET state=?1,revision=?2,started_at_ms=?3,max_step_activations=?4,step_activations_used=?5
          WHERE run_id=?6",
        params![
            next.to_string(),
            revision_i64,
            now.as_i64(),
            entry.max_step_activations,
            entry.activations,
            id.to_string(),
        ],
    )?;
    insert_ready_task(tx, &task, id, &run.mission, "[]")?;
    let due = deadline(now, entry.max_duration_seconds)?;
    let step_due = deadline(now, entry.max_duration_seconds.min(entry.timeout_seconds))?;
    arm_timer(tx, config, id, None, "run_deadline", due)?;
    arm_timer(tx, config, id, Some(task.step_id), "step_timeout", step_due)?;
    events.push(EventAppendSpec {
        event_type: "RunStarted".to_owned(),
        aggregate_type: AggregateKind::WorkflowRun,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: revision,
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: serde_json::from_value(json!({
            "run_id": id, "workflow_version_id": run.version,
            "definition_hash": run.definition, "effective_bundle_hash": run.bundle,
            "started_at_ms": now, "deadline_at_ms": due, "max_step_activations": entry.max_step_activations,
        }))?,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec!["clients".to_owned(), "harness".to_owned()],
    });
    events.push(node_activated_event(
        envelope,
        &run.version,
        revision,
        &task,
        &[],
    )?);
    Ok(revision)
}

fn deadline(now: TimestampMs, seconds: u32) -> Result<i64, StoreError> {
    now.as_i64()
        .checked_add(i64::from(seconds) * 1000)
        .ok_or_else(|| invalid("workflow deadline outside the storage range"))
}

fn arm_timer(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    run: RunId,
    step: Option<StepId>,
    kind: &str,
    due_at_ms: i64,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO workflow_timers (timer_id,run_id,step_id,timer_kind,due_at_ms,state,revision)
         VALUES (?1,?2,?3,?4,?5,'armed',1)",
        params![
            config.id_generator.new_uuid().to_string(),
            run.to_string(),
            step.map(|id| id.to_string()),
            kind,
            due_at_ms
        ],
    )?;
    Ok(())
}
