//! Command and causal read for the historical Release attestation.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AuthorizationEvaluationId, CommandId, DaemonEpoch, DispatchId, Fence,
    ProcessRunUuid, ProjectId, ReceiptId, Revision, Sequence, Sha256Digest, TerminalId,
    TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, EventCausationKind, EventCursor, LineageStatus, ReceiptState,
    Sensitivity, canonical_sha256, compute_command_digest_v2,
};

use super::seal::{TerminalReleaseHistoryFact, inspect_sealed, seal_tx};
use super::{
    TerminalReleaseHistorySource, TerminalReleaseHistoryTarget, hidden, invalid,
    qualify_terminal_release_history_source,
};
use crate::commands::{
    classify_command_replay, read_command_receipt_inspection, read_daemon_state_conn,
};
use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    DaemonStateRow, EventAppendSpec, PreparedObjectHandle, ReceiptInspectionOutcome,
    SnapshotProjection, StoreConfig, StoreError, UnleashStore,
};

/// Prefix reserved for references that select a historical Release fact.
pub const TERMINAL_RELEASE_HISTORY_PREFIX: &str = "terminal-release-history:";
const REFERENCED_EVENT: &str = "TerminalReleaseHistoryReferenced";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptLink {
    result_ref: String,
    terminal_id: TerminalId,
    process_run_uuid: ProcessRunUuid,
    source_action_id: ActionId,
    source_dispatch_id: DispatchId,
    fact_cursor: EventCursor,
    observation_object_hash: Sha256Digest,
}

impl ReceiptLink {
    fn new(target: TerminalReleaseHistoryTarget, fact: &TerminalReleaseHistoryFact) -> Self {
        Self {
            result_ref: result_ref(target),
            terminal_id: target.terminal_id,
            process_run_uuid: target.process_run_uuid,
            source_action_id: target.action_id,
            source_dispatch_id: target.dispatch_id,
            fact_cursor: fact.event_cursor,
            observation_object_hash: fact.observation_object_hash,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
enum Request {
    AttestTerminalRelease {
        terminal_id: TerminalId,
        process_run_uuid: ProcessRunUuid,
        source_action_id: ActionId,
        source_dispatch_id: DispatchId,
    },
}

fn target(
    caller: &CallerAuthority,
    envelope: &CommandEnvelope,
) -> Result<TerminalReleaseHistoryTarget, StoreError> {
    crate::policies_secrets::require_reconciler(caller, "ReconcileProcess")?;
    let Request::AttestTerminalRelease {
        terminal_id,
        process_run_uuid,
        source_action_id,
        source_dispatch_id,
    } = serde_json::from_value(serde_json::Value::Object(envelope.payload.clone()))
        .map_err(|_| invalid("historical release command payload is invalid"))?;
    let project_id = envelope
        .project_id
        .ok_or_else(|| invalid("historical release requires its exact project"))?;
    if envelope.command_type != "ReconcileProcess"
        || terminal_id.as_uuid() != envelope.aggregate_id
        || [
            project_id.as_uuid(),
            terminal_id.as_uuid(),
            process_run_uuid.as_uuid(),
            source_action_id.as_uuid(),
            source_dispatch_id.as_uuid(),
        ]
        .iter()
        .any(uuid::Uuid::is_nil)
        || envelope
            .actor_ref
            .as_deref()
            .is_some_and(|actor| actor != caller.actor_id.as_str())
    {
        return Err(invalid("historical release command identity is invalid"));
    }
    Ok(TerminalReleaseHistoryTarget {
        project_id,
        terminal_id,
        process_run_uuid,
        action_id: source_action_id,
        dispatch_id: source_dispatch_id,
    })
}

fn result_ref(target: TerminalReleaseHistoryTarget) -> String {
    format!(
        "{TERMINAL_RELEASE_HISTORY_PREFIX}{}:{}",
        target.action_id, target.dispatch_id
    )
}

fn require_revision(
    source: &TerminalReleaseHistorySource,
    envelope: &CommandEnvelope,
) -> Result<(), StoreError> {
    let current_revision = source.context.control_revision;
    if envelope.expected_revision != current_revision {
        return Err(StoreError::RevisionConflict {
            current_revision,
            expected_revision: envelope.expected_revision,
            command_id: Some(envelope.command_id),
        });
    }
    Ok(())
}

/// Work required after the daemon has authenticated the request.
#[derive(Debug)]
pub enum TerminalReleaseReconcilePlan {
    /// This command is already durably received.
    Replay(CommandReceipt),
    /// The native fact is already retained.
    ReferenceSealed,
    /// The host must be inspected without granting control.
    Inspect(Box<TerminalReleaseHistorySource>),
}

/// Causal result of a historical Release command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseReconciliationResult {
    /// Closed result version.
    pub schema_version: u32,
    /// Current inspection command.
    pub command_id: CommandId,
    /// Durable receipt of the inspection command.
    pub receipt_id: ReceiptId,
    /// Exact selector from that receipt.
    pub result_ref: String,
    /// Terminal whose old controller was observed.
    pub terminal_id: TerminalId,
    /// Exact process generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Original retirement command.
    pub source_command_id: CommandId,
    /// Original retirement acceptance.
    pub source_receipt_id: ReceiptId,
    /// Original action, still Unknown.
    pub source_action_id: ActionId,
    /// Original committed dispatch.
    pub source_dispatch_id: DispatchId,
    /// Original authorization evaluation.
    pub source_evaluation_id: AuthorizationEvaluationId,
    /// Digest of the immutable intent.
    pub source_intent_digest: Sha256Digest,
    /// Digest of the accepted Release context.
    pub source_context_digest: Sha256Digest,
    /// Fingerprint of the native ACK.
    pub native_ack_digest: Sha256Digest,
    /// Fingerprint of the first retained evidence.
    pub observation_object_hash: Sha256Digest,
    /// Historical daemon epoch in the ACK.
    pub retirement_epoch: DaemonEpoch,
    /// Historical fence in the ACK, with no current authority.
    pub retirement_fence: Fence,
    /// Writer epoch that sealed the fact.
    pub inspected_by_epoch: DaemonEpoch,
    /// Verified actor of the inspection.
    pub inspected_by: ActorId,
    /// Timestamp of the first retained observation.
    pub observed_at_ms: TimestampMs,
    /// Exact cursor of the sealed historical fact.
    pub cursor: EventCursor,
}

impl UnleashStore {
    /// Check replay and the exact source before any host I/O.
    ///
    /// # Errors
    /// Refuses conflicting commands, wrong lineage, foreign targets or invalid evidence.
    pub fn plan_terminal_release_reconciliation(
        &self,
        caller: &CallerAuthority,
        envelope: &CommandEnvelope,
    ) -> Result<TerminalReleaseReconcilePlan, StoreError> {
        let target = target(caller, envelope)?;
        let digest = compute_command_digest_v2(envelope)?;
        self.read_snapshot(|conn, _| {
            let daemon = read_daemon_state_conn(conn)?;
            if let Some(receipt) =
                classify_command_replay(conn, &daemon, &caller.actor_id, envelope, digest)?
            {
                return Ok(TerminalReleaseReconcilePlan::Replay(receipt));
            }
            let source = qualify_terminal_release_history_source(conn, caller, target)?;
            require_revision(&source, envelope)?;
            if inspect_sealed(conn, self.config(), &daemon, &source)?.is_some() {
                Ok(TerminalReleaseReconcilePlan::ReferenceSealed)
            } else {
                Ok(TerminalReleaseReconcilePlan::Inspect(Box::new(source)))
            }
        })
    }

    /// Commit the factual observation and its causal command receipt in one transaction.
    ///
    /// # Errors
    /// Refuses invalid envelopes, stale context, unbound evidence or changed lineage.
    pub fn commit_terminal_release_reconciliation(
        &self,
        caller: CallerAuthority,
        envelope: CommandEnvelope,
        proof: Option<PreparedObjectHandle>,
    ) -> Result<CommitOutcome, StoreError> {
        let target = target(&caller, &envelope)?;
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: AggregateKind::TerminalControl,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: Some(result_ref(target)),
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::AttestHistoricalTerminalRelease { caller, proof },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
    }

    /// Read the observation reached from an exact, current command receipt.
    ///
    /// # Errors
    /// Refuses foreign or mismatched receipts, missing evidence and invalid lineage.
    pub fn inspect_terminal_release_reconciliation_result(
        &self,
        caller: &CallerAuthority,
        command: CommandId,
        reference: &str,
        project: ProjectId,
    ) -> Result<SnapshotProjection<TerminalReleaseReconciliationResult>, StoreError> {
        crate::policies_secrets::require_reconciler(caller, "ReconcileProcess")?;
        self.read_snapshot(|conn, _| {
            let receipt = qualified_receipt(conn, &caller.actor_id, command, reference)?;
            let target = target_from_reference(conn, caller, &receipt, reference, project)?;
            let source = qualify_terminal_release_history_source(conn, caller, target)?;
            if receipt.new_revision != source.context.control_revision {
                return Err(hidden());
            }
            let daemon = read_daemon_state_conn(conn)?;
            let fact = inspect_sealed(conn, self.config(), &daemon, &source)?.ok_or_else(|| {
                StoreError::EvidenceUnavailable("historical release result is not retained".into())
            })?;
            verify_receipt_link(conn, caller, target, &receipt, &fact)?;
            let high: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",
                [daemon.stream_id.to_string()],
                |row| row.get(0),
            )?;
            let cursor = EventCursor::new(
                daemon.stream_id,
                Sequence::new(
                    u64::try_from(high)
                        .map_err(|_| invalid("invalid historical release watermark"))?,
                ),
            );
            let c = source.context;
            Ok(SnapshotProjection {
                cursor,
                value: TerminalReleaseReconciliationResult {
                    schema_version: 1,
                    command_id: receipt.command_id,
                    receipt_id: receipt.receipt_id,
                    result_ref: reference.into(),
                    terminal_id: c.terminal_id,
                    process_run_uuid: c.process_run_uuid,
                    source_command_id: c.command_id,
                    source_receipt_id: c.receipt_id,
                    source_action_id: c.action_id,
                    source_dispatch_id: target.dispatch_id,
                    source_evaluation_id: source.evaluation_id,
                    source_intent_digest: source.intent_digest,
                    source_context_digest: source.key.header.payload_hash,
                    native_ack_digest: fact.native_ack_digest,
                    observation_object_hash: fact.observation_object_hash,
                    retirement_epoch: c.daemon_epoch,
                    retirement_fence: c.resource_fence,
                    inspected_by_epoch: fact.inspected_by_epoch,
                    inspected_by: c.actor_id,
                    observed_at_ms: fact.observed_at_ms,
                    cursor: fact.event_cursor,
                },
            })
        })
    }
}

fn verify_receipt_link(
    conn: &Connection,
    caller: &CallerAuthority,
    target: TerminalReleaseHistoryTarget,
    receipt: &CommandReceipt,
    fact: &TerminalReleaseHistoryFact,
) -> Result<(), StoreError> {
    let sequence = i64::try_from(receipt.event_cursor.sequence.as_u64()).map_err(|_| hidden())?;
    let (raw, digest): (String, String) = conn
        .query_row(
            "SELECT payload_json,payload_hash FROM events WHERE sequence=?1 AND stream_id=?2
           AND event_type=?3 AND aggregate_type='action' AND aggregate_id=?4 AND project_id=?5
           AND actor_id=?6 AND command_actor_id=?6 AND command_id=?7
           AND causation_kind='command' AND causation_receipt_id=?8 AND correlation_id=?9
           AND schema_version=1 AND payload_state='full'",
            params![
                sequence,
                receipt.event_cursor.stream_id.to_string(),
                REFERENCED_EVENT,
                target.action_id.to_string(),
                target.project_id.to_string(),
                caller.actor_id.as_str(),
                receipt.command_id.to_string(),
                receipt.receipt_id.to_string(),
                target.terminal_id.to_string()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(hidden)?;
    let link: ReceiptLink = serde_json::from_str(&raw)?;
    if link != ReceiptLink::new(target, fact)
        || canonical_sha256(&link)?.as_prefixed_str() != digest
    {
        return Err(hidden());
    }
    Ok(())
}

fn qualified_receipt(
    conn: &Connection,
    actor: &ActorId,
    command: CommandId,
    reference: &str,
) -> Result<CommandReceipt, StoreError> {
    let ReceiptInspectionOutcome::Full(inspected) =
        read_command_receipt_inspection(conn, actor, command)?
    else {
        return Err(StoreError::EvidenceUnavailable(
            "historical release receipt was purged".into(),
        ));
    };
    let receipt = inspected.receipt;
    let exact: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE actor_id=?1 AND command_id=?2
         AND receipt_id=?3 AND command_type='ReconcileProcess' AND state='completed')",
        params![
            actor.as_str(),
            command.to_string(),
            receipt.receipt_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if !exact
        || inspected.lineage_status != LineageStatus::Current
        || receipt.state != ReceiptState::Completed
        || receipt.result_ref.as_deref() != Some(reference)
    {
        return Err(hidden());
    }
    Ok(receipt)
}

fn target_from_reference(
    conn: &Connection,
    caller: &CallerAuthority,
    receipt: &CommandReceipt,
    reference: &str,
    project: ProjectId,
) -> Result<TerminalReleaseHistoryTarget, StoreError> {
    let (action, dispatch) = reference
        .strip_prefix(TERMINAL_RELEASE_HISTORY_PREFIX)
        .and_then(|suffix| suffix.split_once(':'))
        .ok_or_else(hidden)?;
    let action_id: ActionId = action.parse().map_err(|_| hidden())?;
    let dispatch_id: DispatchId = dispatch.parse().map_err(|_| hidden())?;
    let terminal_id = TerminalId::from_uuid(receipt.aggregate_id);
    let generation: String = conn.query_row(
        "SELECT json_extract(context_json,'$.process_run_uuid') FROM terminal_release_intents
         WHERE action_id=?1 AND actor_id=?2 AND terminal_id=?3 AND json_extract(context_json,'$.project_id')=?4",
        params![action_id.to_string(),caller.actor_id.as_str(),terminal_id.to_string(),project.to_string()],
        |row| row.get(0),
    ).optional()?.ok_or_else(hidden)?;
    Ok(TerminalReleaseHistoryTarget {
        project_id: project,
        terminal_id,
        action_id,
        dispatch_id,
        process_run_uuid: generation.parse()?,
    })
}

pub(crate) fn validate_command(
    req: &CommitCommandRequest,
    caller: &CallerAuthority,
) -> Result<(), StoreError> {
    let target = target(caller, &req.envelope)?;
    if caller.actor_id != req.actor_id
        || req.target_aggregate != AggregateKind::TerminalControl
        || req.receipt_state != ReceiptState::Completed
        || req.result_ref.as_deref() != Some(result_ref(target).as_str())
        || req.operation_id.is_some()
        || req.retention_days_override.is_some()
        || req.fence_assertion.is_some()
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
    {
        return Err(invalid(
            "historical release command carries unexpected effects",
        ));
    }
    Ok(())
}

pub(crate) struct HistoryCommand<'a> {
    pub(crate) caller: &'a CallerAuthority,
    pub(crate) envelope: &'a CommandEnvelope,
    pub(crate) proof: Option<&'a PreparedObjectHandle>,
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: &HistoryCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let t = target(command.caller, command.envelope)?;
    let source = qualify_terminal_release_history_source(tx, command.caller, t)?;
    require_revision(&source, command.envelope)?;
    let committed = seal_tx(tx, config, daemon, now, &source, command.proof)?;
    let payload = serde_json::to_value(ReceiptLink::new(t, &committed.fact))?
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("historical release receipt link is invalid"))?;
    events.push(EventAppendSpec {
        event_type: REFERENCED_EVENT.into(),
        aggregate_type: AggregateKind::Action,
        aggregate_id: t.action_id.as_uuid(),
        aggregate_revision: source.action_revision,
        project_id: Some(t.project_id),
        correlation_id: t.terminal_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Restricted,
        outbox_destinations: vec!["clients".into()],
    });
    Ok(source.context.control_revision)
}
