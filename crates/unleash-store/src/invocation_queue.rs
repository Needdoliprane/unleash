//! Entry to the invocation queue from an already resolved harness decision.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use unleash_domain::{
    ActorId, AgentId, DaemonEpoch, DaemonMode, Revision, SessionId, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, EventCausationKind, ReceiptState, Sensitivity};
use uuid::Uuid;

use crate::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, DaemonStateRow,
    EventAppendSpec, StoreConfig, StoreError, UnleashStore,
    commands::ensure_scope_not_frozen_tx,
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx},
};

const COMMAND: &str = "CreateInvocation";
const ACTOR: &str = "system:harness";

/// Exact references resolved by the harness before this internal queueing operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationQueuePlan {
    /// Daemon incarnation that resolved the inputs.
    pub daemon_epoch: DaemonEpoch,
    /// Logical agent to wake.
    pub agent_id: AgentId,
    /// Revision of that agent at resolution.
    pub agent_revision: Revision,
    /// Exact primary session bound to the initial manifest.
    pub session_id: SessionId,
    /// Session revision covered by the initial manifest.
    pub session_revision: Revision,
    /// Event from the current project stream that caused the invocation.
    pub trigger_event_id: Uuid,
    /// Already materialized immutable bundle.
    pub effective_bundle_hash: Sha256Digest,
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        message: message.to_owned(),
        reason: None,
    }
}

pub(crate) fn validate_command(request: &CommitCommandRequest) -> Result<(), StoreError> {
    let envelope = &request.envelope;
    if !matches!(request.mutation, CommandMutation::CreateInvocation) {
        return if envelope.command_type == COMMAND {
            Err(invalid(
                "invocation command requires a resolved queue mutation",
            ))
        } else {
            Ok(())
        };
    }
    if envelope.command_type != COMMAND
        || envelope.aggregate_id.is_nil()
        || envelope.expected_revision != Revision::INITIAL
        || envelope.actor_ref.as_deref() != Some(ACTOR)
        || request.actor_id.as_str() != ACTOR
        || request.target_aggregate != AggregateKind::Invocation
        || request.receipt_state != ReceiptState::Accepted
        || request.operation_id.is_some()
        || request.result_ref.is_some()
        || request.retention_days_override.is_some()
        || request.fence_assertion.is_some()
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent internal invocation command"));
    }
    Ok(())
}

impl UnleashStore {
    /// Queue a resolved work invocation with its receipt and event.
    ///
    /// The harness supplies immutable inputs from a trusted resolver.
    /// Admission and native execution are separate operations.
    ///
    /// # Errors
    /// Returns an error on invalid or stale inputs, a conflict, or a writer failure.
    pub fn queue_invocation(&self, envelope: CommandEnvelope) -> Result<CommitOutcome, StoreError> {
        self.commit_command(CommitCommandRequest {
            actor_id: ActorId::new(ACTOR)?,
            envelope,
            target_aggregate: AggregateKind::Invocation,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateInvocation,
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        })
    }
}

pub(crate) fn queue_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != DaemonMode::Ready || daemon.schema_version < 8 {
        return Err(StoreError::RecoveryInProgress(
            "invocation queue requires a ready daemon and admission schema".to_owned(),
        ));
    }
    let plan: InvocationQueuePlan = serde_json::from_value(Value::Object(envelope.payload.clone()))
        .map_err(|_| invalid("invalid resolved invocation plan"))?;
    if daemon.daemon_epoch != plan.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "invocation was resolved in another daemon epoch".to_owned(),
        });
    }
    let project = envelope.project_id.map(|id| id.to_string());
    let session_revision = validate_agent_and_session(tx, envelope, &plan, project.as_deref())?;
    let trigger = plan.trigger_event_id.to_string();
    let current_event: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE event_id=?1 AND stream_id=?2
                       AND project_id IS ?3 AND payload_state='full')",
        params![trigger, daemon.stream_id.to_string(), project],
        |row| row.get(0),
    )?;
    if !current_event {
        return Err(StoreError::EvidenceUnavailable(
            "invocation trigger unavailable in the current project stream".to_owned(),
        ));
    }
    let bundle = plan.effective_bundle_hash.to_string();
    let (manifest, context, scope): (String, String, String) = tx
        .query_row(
            "SELECT m.manifest_id,m.content_hash,b.scope_id
             FROM effective_bundles b
             JOIN context_manifests m ON m.manifest_id=b.initial_context_manifest_id
             JOIN object_records bo ON bo.object_hash=b.content_object_hash AND bo.state='live'
             JOIN object_records co ON co.object_hash=m.content_hash AND co.state='live'
             WHERE b.bundle_hash=?1 AND b.content_object_hash=b.bundle_hash
               AND m.session_id=?2 AND m.base_session_revision=?3",
            params![bundle, plan.session_id.to_string(), session_revision],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::EvidenceUnavailable(
                "pinned invocation bundle or session manifest is unavailable".to_owned(),
            )
        })?;
    ensure_scope_not_frozen_tx(tx, envelope.project_id, Some(&scope), None)?;
    ensure_subject_not_blocked_tx(
        tx,
        SubjectKind::Agent,
        &plan.agent_id.to_string(),
        envelope.project_id,
    )?;
    insert_invocation(tx, config, now, envelope, &plan, &manifest, &context)?;
    let revision = Revision::new(1);
    let payload: Map<String, Value> = serde_json::from_value(json!({
        "invocation_id": envelope.aggregate_id,
        "agent_id": plan.agent_id,
        "session_id": plan.session_id,
        "effective_bundle_hash": plan.effective_bundle_hash,
        "context_manifest_id": manifest,
        "trigger_event_id": plan.trigger_event_id,
        "state": "queued",
    }))?;
    events.push(EventAppendSpec {
        event_type: "InvocationQueued".to_owned(),
        aggregate_type: AggregateKind::Invocation,
        aggregate_id: envelope.aggregate_id,
        aggregate_revision: revision,
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Event,
        causation_id: Some(trigger),
        payload,
        sensitivity: Sensitivity::Restricted,
        // REQ-AGENT-002: same audience as the `InvocationQueued` of `AssignTask`
        // (`task_assignment`). A restricted event reaches the harness only; widening
        // its audience to clients requires a decision of the event model.
        outbox_destinations: vec!["harness".to_owned()],
    });
    Ok(revision)
}

fn validate_agent_and_session(
    tx: &Transaction<'_>,
    envelope: &CommandEnvelope,
    plan: &InvocationQueuePlan,
    project: Option<&str>,
) -> Result<i64, StoreError> {
    let (agent, session): (i64, i64) = tx
        .query_row(
            "SELECT a.revision,s.revision FROM agents a
             JOIN sessions s ON s.agent_id=a.agent_id AND s.state='open'
             WHERE a.agent_id=?1 AND s.session_id=?2 AND a.project_id IS ?3
               AND a.state IN ('eligible','active')
               AND (?3 IS NULL OR EXISTS(SELECT 1 FROM projects p
                                         WHERE p.project_id=?3 AND p.state='active'))",
            params![
                plan.agent_id.to_string(),
                plan.session_id.to_string(),
                project
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound(
                "eligible agent or open session unavailable in this project".to_owned(),
            )
        })?;
    for (revision, expected) in [
        (agent, plan.agent_revision),
        (session, plan.session_revision),
    ] {
        let revision = Revision::new(u64::try_from(revision).map_err(|_| {
            StoreError::InvariantViolation("negative agent or session revision".to_owned())
        })?);
        if revision != expected {
            return Err(StoreError::RevisionConflict {
                current_revision: revision,
                expected_revision: expected,
                command_id: Some(envelope.command_id),
            });
        }
    }
    Ok(session)
}

fn insert_invocation(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    plan: &InvocationQueuePlan,
    manifest: &str,
    context: &str,
) -> Result<(), StoreError> {
    let invocation = envelope.aggregate_id.to_string();
    if let Some(revision) = tx
        .query_row(
            "SELECT revision FROM invocations WHERE invocation_id=?1",
            [&invocation],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(revision).map_err(|_| {
                StoreError::InvariantViolation("negative invocation revision".to_owned())
            })?),
            expected_revision: Revision::INITIAL,
            command_id: Some(envelope.command_id),
        });
    }
    let active: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM invocations
         WHERE agent_id=?1 AND state NOT IN ('completed','failed','cancelled'))",
        [plan.agent_id.to_string()],
        |row| row.get(0),
    )?;
    if active {
        return Err(StoreError::CapacityBlocked(
            "agent already has an active invocation".to_owned(),
        ));
    }
    let bundle = plan.effective_bundle_hash.to_string();
    tx.execute(
        "INSERT INTO invocations(invocation_id,agent_id,session_id,trigger_event_id,
                                 effective_bundle_hash,context_manifest_id,state,admission_class,
                                 revision,created_at_ms,queued_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,'queued','work',1,?7,?7)",
        params![
            invocation,
            plan.agent_id.to_string(),
            plan.session_id.to_string(),
            plan.trigger_event_id.to_string(),
            bundle,
            manifest,
            now.as_i64()
        ],
    )?;
    for (field, hash) in [
        ("effective_bundle", bundle.as_str()),
        ("context_manifest", context),
    ] {
        tx.execute(
            "INSERT INTO object_references(reference_id,object_hash,owner_kind,owner_id,source_field,state)
             VALUES(?1,?2,'invocation',?3,?4,'live')",
            params![config.id_generator.new_uuid().to_string(), hash, invocation, field],
        )?;
    }
    Ok(())
}
