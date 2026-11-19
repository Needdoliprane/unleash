//! Correlation of historical facts with immutable original writer records.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, ActorId, DispatchId, Fence, TimestampMs,
};
use unleash_protocol::{LineageStatus, ReceiptState, canonical_sha256};

use super::super::{
    TerminalControlRecord, TerminalControlState, TerminalIdentityBackend, control_read_conn,
    read_conn,
};
use super::{HistoricalTransferAck, TerminalControlHistoryProof};
use crate::commands::read_daemon_state_conn;
use crate::{
    BrokerActionSnapshot, BrokerAuthorizationDecision, BrokerCause, BrokerEvaluation, BrokerIntent,
    StoreError,
};

pub(super) struct Source {
    pub record: TerminalControlRecord,
    pub broker: BrokerActionSnapshot,
    pub intent: BrokerIntent,
    pub evaluation: BrokerEvaluation,
    pub started_at: TimestampMs,
    pub environment_digest: unleash_domain::Sha256Digest,
}

pub(super) fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.into(),
    }
}
pub(super) fn unavailable(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.into())
}
fn identity_error() -> StoreError {
    StoreError::IdentityUnverified("historical host generation or identity differs".into())
}
fn hidden() -> StoreError {
    StoreError::NotFound("historical terminal control is not visible".into())
}

pub(super) fn source(
    conn: &Connection,
    actor: &ActorId,
    action: ActionId,
    dispatch: DispatchId,
) -> Result<Source, StoreError> {
    let owner: Option<String> = conn
        .query_row(
            "SELECT actor_id FROM terminal_control_intents WHERE action_id=?1",
            [action.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if owner.as_deref() != Some(actor.as_str()) {
        return Err(hidden());
    }
    let record = control_read_conn(conn, action)?.ok_or_else(hidden)?;
    let c = &record.context;
    if c.actor_id != *actor {
        return Err(hidden());
    }
    if c.schema_version != 1 {
        return Err(invalid("historical control context schema differs"));
    }
    let daemon = read_daemon_state_conn(conn)?;
    if c.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: c.authority_generation,
            command_id: Some(c.command_id),
        });
    }
    if c.daemon_epoch > daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "historical rotation is from a future daemon epoch".into(),
        });
    }
    if c.daemon_boot_identity != daemon.boot_identity
        || c.expected_os_identity.owner_uid != daemon.owner_uid
    {
        return Err(identity_error());
    }
    let broker = crate::broker_writer::inspect_action_conn(conn, action)?.ok_or_else(hidden)?;
    if broker.state != ActionState::OutcomeUnknown
        || record.dispatch_id != Some(dispatch)
        || broker.dispatch_id != Some(dispatch)
        || !matches!(
            record.state,
            TerminalControlState::HandshakeResolved
                | TerminalControlState::OutcomeUnknown
                | TerminalControlState::Disposed
        )
    {
        return Err(invalid(
            "historical transfer has no consumed unknown dispatch",
        ));
    }
    validate_receipt(conn, &record)?;
    let environment_digest = validate_allocation(conn, &record)?;
    validate_control_coordinates(conn, &record)?;
    let encoded: String = conn.query_row(
        "SELECT intent_json FROM broker_intents WHERE action_id=?1",
        [action.to_string()],
        |row| row.get(0),
    )?;
    let intent: BrokerIntent = serde_json::from_str(&encoded)?;
    validate_intent(&record, &broker, &intent)?;
    let (evaluation, started_at) = validate_marker(conn, &record, &broker, &intent, dispatch)?;
    Ok(Source {
        record,
        broker,
        intent,
        evaluation,
        started_at,
        environment_digest,
    })
}

fn validate_receipt(conn: &Connection, record: &TerminalControlRecord) -> Result<(), StoreError> {
    let c = &record.context;
    let crate::ReceiptInspectionOutcome::Full(receipt) =
        crate::commands::read_command_receipt_inspection(conn, &c.actor_id, c.command_id)?
    else {
        return Err(unavailable("original terminal receipt is tombstoned"));
    };
    if receipt.lineage_status != LineageStatus::Current
        || receipt.receipt.receipt_id != c.receipt_id
        || receipt.receipt.command_id != c.command_id
        || receipt.receipt.aggregate_id != c.terminal_id.as_uuid()
        || receipt.receipt.state != ReceiptState::Accepted
        || receipt.receipt.result_ref.as_deref()
            != Some(format!("terminal-control:{}", c.action_id).as_str())
    {
        return Err(invalid(
            "original terminal receipt does not bind this control",
        ));
    }
    let exact:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3 AND command_type='AcquireTerminal' AND aggregate_id=?4 AND authority_generation=?5)",params![c.receipt_id.to_string(),c.actor_id.as_str(),c.command_id.to_string(),c.terminal_id.to_string(),c.authority_generation.to_string()],|row|row.get(0))?;
    if !exact {
        return Err(invalid("original terminal command binding differs"));
    }
    Ok(())
}

fn validate_allocation(
    conn: &Connection,
    record: &TerminalControlRecord,
) -> Result<unleash_domain::Sha256Digest, StoreError> {
    let c = &record.context;
    let allocation = read_conn(conn, c.terminal_id)?.ok_or_else(identity_error)?;
    let a = &allocation.context;
    let super::super::TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
        &a.preparation.scope;
    if a.action_id != c.initial_action_id
        || allocation.context_digest != c.initial_context_digest
        || a.process_id != c.process_id
        || a.process_run_uuid != c.process_run_uuid
        || a.host_id != c.host_id
        || a.actor_id != c.actor_id
        || a.principal_id != c.principal_id
        || a.authority_generation != c.authority_generation
        || a.daemon_boot_identity != c.daemon_boot_identity
        || *project_id != c.project_id
        || a.preparation.mission_id != c.mission_id
    {
        return Err(identity_error());
    }
    let i = &c.expected_os_identity;
    let backend = match i.backend {
        TerminalIdentityBackend::MacosProcPidinfo => "macos_proc_pidinfo",
        TerminalIdentityBackend::LinuxProcfs => "linux_procfs",
    };
    let verified:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM processes p JOIN actions a ON a.action_id=p.action_id JOIN terminal_sessions s ON s.process_id=p.process_id WHERE p.process_id=?1 AND p.action_id=?2 AND p.process_run_uuid=?3 AND p.host_id=?4 AND p.authority_generation=?5 AND p.pid=?6 AND p.pgid=?7 AND p.owner_uid=?8 AND p.boot_identity=?9 AND p.start_identity=?10 AND p.identity_backend=?11 AND s.terminal_id=?12 AND a.action_type='terminal.acquire' AND a.state='observed_success')",params![c.process_id.to_string(),c.initial_action_id.to_string(),c.process_run_uuid.to_string(),c.host_id.to_string(),c.authority_generation.to_string(),i.pid,i.pgid,i.owner_uid,i.boot_identity,i.start_identity,backend,c.terminal_id.to_string()],|row|row.get(0))?;
    if !verified {
        return Err(identity_error());
    }
    Ok(a.preparation.process_spec.environment.digest)
}

fn validate_intent(
    record: &TerminalControlRecord,
    broker: &BrokerActionSnapshot,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    let c = &record.context;
    if intent.action_id != c.action_id
        || intent.payload_digest != record.context_digest
        || intent.intent_digest != broker.intent_digest
        || canonical_sha256(&intent.manifest)? != intent.intent_digest
        || intent.principal_id != c.principal_id
        || intent.project_id != Some(c.project_id)
        || intent.scope_revision != c.scope_revision
        || intent.policy_revision != c.policy_revision
        || intent.lineage_fence != broker.fence
        || intent.lineage_fence.authority_generation != c.authority_generation
        || intent.lineage_fence.epoch != c.daemon_epoch
        || intent.lineage_fence.fence != Fence::new(0)
        || intent.action_deadline != c.expires_at_ms
        || intent.origin
            != (ActionOrigin::ControlPlane {
                control_principal_id: c.principal_id.clone(),
            })
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.manifest.get("verb") != Some(&json!(super::super::TERMINAL_CONTROL_VERB))
    {
        return Err(invalid("historical terminal broker intent differs"));
    }
    Ok(())
}

fn validate_control_coordinates(
    conn: &Connection,
    record: &TerminalControlRecord,
) -> Result<(), StoreError> {
    let c = &record.context;
    let epoch = i64::try_from(c.daemon_epoch.as_u64())
        .map_err(|_| invalid("historical epoch out of range"))?;
    let previous_epoch = i64::try_from(c.previous_epoch.as_u64())
        .map_err(|_| invalid("historical previous epoch out of range"))?;
    let fence = i64::try_from(c.next_fence.as_u64())
        .map_err(|_| invalid("historical fence out of range"))?;
    let previous_fence = i64::try_from(c.previous_fence.as_u64())
        .map_err(|_| invalid("historical previous fence out of range"))?;
    let exact:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM resource_leases old JOIN resource_leases next ON next.resource_kind=old.resource_kind AND next.resource_key=old.resource_key WHERE old.lease_id=?1 AND next.lease_id=?2 AND old.lease_id<>next.lease_id AND old.resource_kind='terminal' AND old.authority_generation=?3 AND next.authority_generation=?3 AND old.owner_type='principal' AND next.owner_type='principal' AND old.owner_id=?4 AND next.owner_id=?4 AND old.boot_identity=?5 AND next.boot_identity=?5 AND old.daemon_epoch=?6 AND next.daemon_epoch=?7 AND old.fence=?8 AND next.fence=?9)",params![c.previous_lease_id.to_string(),c.next_lease_id.to_string(),c.authority_generation.to_string(),c.principal_id.as_str(),c.daemon_boot_identity,previous_epoch,epoch,previous_fence,fence],|row|row.get(0))?;
    if !exact {
        return Err(invalid(
            "historical old and successor control coordinates differ",
        ));
    }
    Ok(())
}

fn validate_marker(
    conn: &Connection,
    record: &TerminalControlRecord,
    broker: &BrokerActionSnapshot,
    intent: &BrokerIntent,
    dispatch: DispatchId,
) -> Result<(BrokerEvaluation, TimestampMs), StoreError> {
    let c = &record.context;
    let (evaluation_id,encoded,authority,epoch,fence):(String,String,String,i64,i64)=conn.query_row("SELECT evaluation_id,decision_json,authority_generation,daemon_epoch,fence FROM broker_dispatches WHERE dispatch_id=?1 AND action_id=?2",params![dispatch.to_string(),c.action_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if evaluation_id != broker.evaluation_id.to_string()
        || authority != c.authority_generation.to_string()
        || epoch
            != i64::try_from(c.daemon_epoch.as_u64())
                .map_err(|_| invalid("historical epoch out of range"))?
        || fence != 0
    {
        return Err(invalid("historical broker marker coordinates differ"));
    }
    let previous:Option<String>=conn.query_row("SELECT previous_evaluation_id FROM broker_evaluations WHERE evaluation_id=?1 AND action_id=?2 AND state='authorized'",params![evaluation_id,c.action_id.to_string()],|row|row.get(0))?;
    let evaluation = BrokerEvaluation {
        evaluation_id: broker.evaluation_id,
        previous_evaluation_id: previous.map(|id| id.parse()).transpose()?,
    };
    let decision: BrokerAuthorizationDecision = serde_json::from_str(&encoded)?;
    if decision.action_id != c.action_id
        || decision.evaluation_id != evaluation.evaluation_id
        || decision.payload_digest != intent.payload_digest
        || decision.policy_revision != intent.policy_revision
        || decision.scope_revision != intent.scope_revision
    {
        return Err(invalid("historical broker decision differs"));
    }
    let auth = historical_phase(conn, c.action_id, &evaluation, "authorization")?;
    if auth.pointer("/phase/outcome/decision") != Some(&serde_json::to_value(&decision)?) {
        return Err(invalid("historical authorization differs from dispatch"));
    }
    phase_binding(&auth, c.action_id, intent.intent_digest)?;
    let marker = historical_phase(conn, c.action_id, &evaluation, "dispatch")?;
    phase_binding(&marker, c.action_id, intent.intent_digest)?;
    if marker.pointer("/phase/dispatch_id") != Some(&serde_json::to_value(dispatch)?)
        || marker.pointer("/phase/decision") != Some(&serde_json::to_value(&decision)?)
        || marker.pointer("/phase/fence") != Some(&serde_json::to_value(intent.lineage_fence)?)
    {
        return Err(invalid("historical dispatch phase differs from its marker"));
    }
    let normalized = json!({"schema_version":1,"action_id":c.action_id,"evaluation":evaluation,"intent_digest":intent.intent_digest,"payload_digest":intent.payload_digest,"decision":decision,"dispatch_id":dispatch,"fence":intent.lineage_fence});
    let (hash,ordinal,started,count):(String,i64,i64,i64)=conn.query_row("SELECT request_hash,ordinal,started_at_ms,(SELECT COUNT(*) FROM action_dispatches WHERE action_id=?2) FROM action_dispatches WHERE dispatch_id=?1 AND action_id=?2",params![dispatch.to_string(),c.action_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if hash != canonical_sha256(&normalized)?.to_string() || ordinal != 1 || count != 1 {
        return Err(invalid("historical normalized dispatch differs"));
    }
    Ok((evaluation, TimestampMs::new(started)))
}

fn historical_phase(
    conn: &Connection,
    action: ActionId,
    evaluation: &BrokerEvaluation,
    key: &str,
) -> Result<serde_json::Value, StoreError> {
    let (encoded,digest):(String,String)=conn.query_row("SELECT phase_json,phase_digest FROM broker_phase_records WHERE action_id=?1 AND evaluation_id=?2 AND phase_key=?3",params![action.to_string(),evaluation.evaluation_id.to_string(),key],|row|Ok((row.get(0)?,row.get(1)?)))?;
    let value: serde_json::Value = serde_json::from_str(&encoded)?;
    if canonical_sha256(&value)?.to_string() != digest
        || value.get("schema_version") != Some(&json!(1))
        || value.get("evaluation") != Some(&serde_json::to_value(evaluation)?)
    {
        return Err(invalid("historical broker phase integrity differs"));
    }
    Ok(value)
}

fn phase_binding(
    value: &serde_json::Value,
    action: ActionId,
    intent: unleash_domain::Sha256Digest,
) -> Result<(), StoreError> {
    if value.pointer("/phase/action_id") != Some(&serde_json::to_value(action)?)
        || value.pointer("/phase/intent_digest") != Some(&serde_json::to_value(intent)?)
    {
        return Err(invalid("historical broker phase source differs"));
    }
    Ok(())
}

pub(super) fn proof(
    source: &Source,
    proof: &TerminalControlHistoryProof,
    now: TimestampMs,
) -> Result<(), StoreError> {
    let c = &source.record.context;
    let ack = &proof.ack;
    if proof.schema_version != 1
        || proof.source_receipt_id != c.receipt_id
        || proof.source_evaluation_id != source.evaluation.evaluation_id
        || proof.source_intent_digest != source.intent.intent_digest
        || ack.header.protocol_version != 1
        || ack.header.command_id != c.command_id
        || ack.header.action_id != c.action_id
        || ack.header.payload_hash != source.record.context_digest
        || ack.dispatch_id
            != source
                .record
                .dispatch_id
                .ok_or_else(|| invalid("historical marker absent"))?
        || ack.initial_context_digest != c.initial_context_digest
    {
        return Err(invalid("historical native acknowledgement binding differs"));
    }
    if ack.header.authority_generation != c.authority_generation
        || ack.header.daemon_epoch != c.daemon_epoch
        || ack.header.resource_fence != c.next_fence
    {
        return Err(StoreError::StaleFence {
            message: "historical native rotation coordinates differ".into(),
        });
    }
    validate_native_identity(source, proof)?;
    let i = &proof.inspection;
    if i.action_id != c.action_id
        || i.dispatch_id != ack.dispatch_id
        || i.context_digest != source.record.context_digest
        || i.terminal_id != c.terminal_id
        || i.daemon_epoch != c.daemon_epoch
        || i.resource_fence != c.next_fence
        || i.transfer_ack_digest != canonical_sha256(ack)?
        || i.durable_log_seq != ack.durable_log_seq
        || i.last_confirmed_input_seq != ack.last_confirmed_input_seq
        || i.unknown_input_seqs != ack.unknown_input_seqs
        || i.observed_at_ms < source.started_at
        || i.observed_at_ms > now
        || ack.unknown_input_seqs.len() > 65536
        || ack.unknown_input_seqs.contains(&0)
    {
        return Err(invalid(
            "historical inspection is not the exact native acknowledgement",
        ));
    }
    Ok(())
}

fn validate_native_identity(
    source: &Source,
    proof: &TerminalControlHistoryProof,
) -> Result<(), StoreError> {
    let c = &source.record.context;
    let ack: &HistoricalTransferAck = &proof.ack;
    let i = &proof.inspection;
    let env = source.environment_digest;
    if ack.header.process_run_uuid != c.process_run_uuid
        || i.process_run_uuid != c.process_run_uuid
        || i.host_id != c.host_id
        || ack.os_identity != c.expected_os_identity
        || i.os_identity != c.expected_os_identity
        || ack.effective_environment_digest != env
        || i.effective_environment_digest != env
    {
        return Err(identity_error());
    }
    Ok(())
}
