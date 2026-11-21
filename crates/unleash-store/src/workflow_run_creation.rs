//! Commit a prepared workflow run and its effective plan together.

use rusqlite::{Connection, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};
use unleash_domain::{
    DaemonEpoch, DaemonMode, MissionId, ProjectId, Revision, RunId, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, EventCausationKind, ReceiptState, Sensitivity,
    canonicalize_to_vec, compute_command_digest_v2,
};

use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, PrepareObjectRequest, PublishObjectReferenceRequest,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
    commands::{
        NewWorkflowRun, classify_command_replay, ensure_scope_not_frozen_tx, insert_workflow_run,
        read_daemon_state_conn,
    },
    objects::{
        PreparedObjectHandle, tx_prepare_object, tx_publish_owned_reference, tx_release_pin,
        verify_object_file_before_publish,
    },
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_human_decider},
    workflow_run_plan::{self, PlanSource, RunPlan, invalid},
};

const COMMAND: &str = "CreateRun";
const OWNER_KIND: &str = "workflow_run";
const PLAN_FIELD: &str = "effective_bundle_hash";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    mission_id: MissionId,
    mission_revision: Revision,
    workflow_version_id: String,
    scope_manifest_hash: Sha256Digest,
    predecessor_run_id: Option<RunId>,
    selection_reason: String,
}

/// Verified publication captured by the Store before entering its writer.
#[derive(Debug, Clone)]
pub struct PreparedRunPublication {
    caller: CallerAuthority,
    daemon_epoch: DaemonEpoch,
    plan: RunPlan,
    reference: PublishObjectReferenceRequest,
}

impl UnleashStore {
    /// Prepare a run from an exact published workflow and a revisioned mission scope document.
    ///
    /// The Store resolves fixed slots from the visible published registry, retains
    /// their versions, and keeps late slots unbound. Starting the run is a
    /// separate transition.
    ///
    /// # Errors
    /// Refuses an unavailable publication, inconsistent scope, ambiguous registry
    /// version, conflicting identity, or stale mission or daemon state.
    pub fn create_prepared_workflow_run(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        let inputs = decode_inputs(&envelope, &caller)?;
        let digest = compute_command_digest_v2(&envelope)?;
        let replay = || self.read_snapshot(|conn, _| replay(conn, &envelope, &caller, digest));
        let (plan, epoch) = if let Some(receipt) = replay()? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        } else {
            match self.resolve_run_plan(&envelope, &caller, &inputs) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return replay()?.map_or(Err(error), |receipt| {
                        Ok(CommitOutcome::IdempotentReplay(receipt))
                    });
                }
            }
        };

        let bytes = canonicalize_to_vec(&plan)?;
        let hash = Sha256Digest::from_bytes(&bytes);
        let run = envelope.aggregate_id.to_string();
        let handle = self.reserve_plan_pin(
            &envelope,
            epoch,
            PrepareObjectRequest {
                object_hash: hash,
                byte_length: u64::try_from(bytes.len())
                    .map_err(|_| invalid("run plan exceeds storage range"))?,
                media_type: "application/json".to_owned(),
                owner_kind: OWNER_KIND.to_owned(),
                owner_id: run.clone(),
            },
        )?;
        let pin = handle.pin_id.clone();
        let authority = envelope.authority_generation;
        let result = (|| {
            self.write_prepared_object_bytes(&handle, &bytes)?;
            verify_object_file_before_publish(self.config(), &handle.relative_path, hash)?;
            self.commit_command(CommitCommandRequest {
                actor_id: caller.actor_id.clone(),
                envelope,
                target_aggregate: AggregateKind::WorkflowRun,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: Some(format!("workflow-run:{run}")),
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::CreatePreparedWorkflowRun(Box::new(
                    PreparedRunPublication {
                        caller,
                        daemon_epoch: epoch,
                        plan,
                        reference: PublishObjectReferenceRequest {
                            pin_id: handle.pin_id,
                            object_hash: hash,
                            object_generation: handle.object_generation,
                            owner_kind: OWNER_KIND.to_owned(),
                            owner_id: run,
                            source_field: PLAN_FIELD.to_owned(),
                        },
                    },
                )),
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
        })();
        if !matches!(result, Ok(CommitOutcome::Committed(_))) {
            let cleanup = self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
                if read_daemon_state_conn(tx)?.authority_generation == authority {
                    tx_release_pin(tx, now, &pin)?;
                }
                Ok(())
            });
            if let Err(error) = cleanup {
                tracing::warn!(%error, "prepared run pin release pending");
            }
        }
        result
    }

    fn reserve_plan_pin(
        &self,
        command: &CommandEnvelope,
        epoch: DaemonEpoch,
        request: PrepareObjectRequest,
    ) -> Result<PreparedObjectHandle, StoreError> {
        let cfg = self.config().clone();
        let authority = command.authority_generation;
        let command_id = command.command_id;
        let pin = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            if daemon.authority_generation != authority {
                return Err(StoreError::StaleAuthorityGeneration {
                    expected: daemon.authority_generation,
                    actual: authority,
                    command_id: Some(command_id),
                });
            }
            if daemon.daemon_epoch != epoch {
                return Err(StoreError::StaleFence {
                    message: "run plan staging crosses daemon epochs".to_owned(),
                });
            }
            tx_prepare_object(tx, &cfg, authority, now, &request)
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C10aAfterPrepPin)?;
        Ok(pin)
    }

    fn resolve_run_plan(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        inputs: &Inputs,
    ) -> Result<(RunPlan, DaemonEpoch), StoreError> {
        self.read_snapshot(|conn, _| {
            let daemon = read_daemon_state_conn(conn)?;
            let project = verify_target(conn, &daemon, envelope, caller, inputs)?;
            let plan = workflow_run_plan::resolve(
                conn,
                self.config(),
                &PlanSource {
                    project_id: project,
                    mission_id: inputs.mission_id,
                    mission_revision: inputs.mission_revision,
                    workflow_version_id: &inputs.workflow_version_id,
                    scope_manifest_hash: inputs.scope_manifest_hash,
                },
            )?;
            Ok((plan, daemon.daemon_epoch))
        })
    }
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

fn decode_inputs(
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Inputs, StoreError> {
    require_human_decider(caller, COMMAND)?;
    let inputs: Inputs = serde_json::from_value(Value::Object(envelope.payload.clone()))
        .map_err(|_| invalid("invalid workflow run preparation payload"))?;
    if envelope.command_type != COMMAND
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        || envelope.aggregate_id.is_nil()
        || envelope.project_id.is_none()
        || inputs.mission_id.as_uuid().is_nil()
        || inputs.workflow_version_id.trim().is_empty()
        || inputs.selection_reason.trim().is_empty()
    {
        return Err(invalid("inconsistent workflow run preparation"));
    }
    Ok(inputs)
}

pub(crate) fn validate_identity(req: &CommitCommandRequest) -> Result<(), StoreError> {
    let CommandMutation::CreatePreparedWorkflowRun(publication) = &req.mutation else {
        return if req.envelope.command_type == COMMAND {
            Err(invalid("run creation requires verified preparation"))
        } else {
            Ok(())
        };
    };
    let inputs = decode_inputs(&req.envelope, &publication.caller)?;
    let plan = &publication.plan;
    if req.actor_id != publication.caller.actor_id
        || req.target_aggregate != AggregateKind::WorkflowRun
        || req.receipt_state != ReceiptState::Completed
        || req.operation_id.is_some()
        || req.retention_days_override.is_some()
        || req.fence_assertion.is_some()
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
        || inputs.mission_id != plan.mission_id
        || inputs.mission_revision != plan.mission_revision
        || inputs.workflow_version_id != plan.workflow_version_id
        || inputs.scope_manifest_hash != plan.scope_manifest_hash
        || publication.reference.owner_kind != OWNER_KIND
        || publication.reference.owner_id != req.envelope.aggregate_id.to_string()
        || publication.reference.source_field != PLAN_FIELD
        || req.result_ref.as_deref()
            != Some(format!("workflow-run:{}", req.envelope.aggregate_id).as_str())
    {
        return Err(invalid("inconsistent prepared run command"));
    }
    Ok(())
}

fn verify_target(
    conn: &Connection,
    daemon: &DaemonStateRow,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    inputs: &Inputs,
) -> Result<ProjectId, StoreError> {
    if daemon.authority_generation != envelope.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "run preparation requires a ready daemon".to_owned(),
        ));
    }
    let project = envelope
        .project_id
        .ok_or_else(|| invalid("run preparation requires a project"))?;
    let revision = i64::try_from(inputs.mission_revision.as_u64())
        .map_err(|_| invalid("mission revision out of range"))?;
    let current: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM missions m
         JOIN projects p ON p.project_id=m.project_id
         JOIN principals u ON u.principal_id=m.owner_user_id
         WHERE m.mission_id=?1 AND m.revision=?2 AND m.project_id=?3
            AND m.state IN ('ready','active') AND m.owner_kind='user' AND p.state='active'
            AND u.actor_id=?4 AND u.principal_kind='user' AND u.state='active')",
        params![
            inputs.mission_id.to_string(),
            revision,
            project.to_string(),
            caller.actor_id.as_str()
        ],
        |row| row.get(0),
    )?;
    if !current {
        return Err(invalid("mission or mandate unavailable at that revision"));
    }
    Ok(project)
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    (config, daemon, now): (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &CommandEnvelope,
    publication: &PreparedRunPublication,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let inputs = decode_inputs(envelope, &publication.caller)?;
    let project = verify_target(tx, daemon, envelope, &publication.caller, &inputs)?;
    if publication.daemon_epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "prepared run crosses daemon epochs".to_owned(),
        });
    }
    ensure_scope_not_frozen_tx(tx, Some(project), None, None)?;
    ensure_subject_not_blocked_tx(
        tx,
        SubjectKind::Actor,
        publication.caller.actor_id.as_str(),
        Some(project),
    )?;
    publication.plan.revalidate(tx)?;
    let run = RunId::from_uuid(envelope.aggregate_id);
    let revision = insert_workflow_run(
        tx,
        NewWorkflowRun {
            run_id: run,
            mission_id: inputs.mission_id,
            workflow_version_id: &inputs.workflow_version_id,
            purpose: "operational",
            candidate_ref: None,
            predecessor_run_id: inputs.predecessor_run_id,
            selection_reason: &inputs.selection_reason,
            effective_bundle_hash: publication.reference.object_hash,
            expected_revision: envelope.expected_revision,
            command_id: envelope.command_id,
            now,
        },
    )?;
    tx_publish_owned_reference(
        tx,
        config.id_generator.as_ref(),
        now,
        &publication.reference,
        daemon.authority_generation,
    )?;
    retain_sources(tx, config, run, &publication.plan)?;
    events.push(EventAppendSpec {
        event_type: "RunCreated".to_owned(),
        aggregate_type: AggregateKind::WorkflowRun,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: revision,
        project_id: Some(project),
        correlation_id: envelope.request_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: serde_json::from_value(json!({
            "run_id": run,
            "mission_id": inputs.mission_id,
            "workflow_version_id": inputs.workflow_version_id,
            "definition_hash": publication.plan.definition_hash,
            "effective_bundle_hash": publication.reference.object_hash,
        }))?,
        sensitivity: Sensitivity::Project,
        outbox_destinations: Vec::new(),
    });
    Ok(revision)
}

fn retain_sources(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    run: RunId,
    plan: &RunPlan,
) -> Result<(), StoreError> {
    let version_refs = plan
        .registry_versions()
        .into_iter()
        .map(|(id, hash)| (format!("registry:{id}"), hash));
    let primary = [
        ("definition_hash".to_owned(), plan.definition_hash),
        ("scope_manifest_hash".to_owned(), plan.scope_manifest_hash),
    ];
    for (field, hash) in primary.into_iter().chain(version_refs) {
        tx.execute(
            "INSERT INTO object_references
                (reference_id,object_hash,owner_kind,owner_id,source_field,state)
             VALUES (?1,?2,?3,?4,?5,'live')",
            params![
                config.id_generator.new_uuid().to_string(),
                hash.to_string(),
                OWNER_KIND,
                run.to_string(),
                field
            ],
        )?;
    }
    Ok(())
}
