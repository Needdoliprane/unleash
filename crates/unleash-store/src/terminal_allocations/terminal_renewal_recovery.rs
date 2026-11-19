//! A separate observed successor rotation lifts an old renewal quarantine.
//! Neither preparing nor disposing that intent repeats its handshake or TTL.
use super::terminal_renewal::read_renewal_conn;
use super::terminal_renewal_types::{TerminalRenewalRecord, TerminalRenewalState};
use super::{
    TerminalControlContext, TerminalControlObservation, TerminalControlRecord, integer, invalid,
};
use crate::{CallerAuthority, DaemonStateRow, EventAppendSpec, StoreError};
use rusqlite::{OptionalExtension, Transaction, params};
use unleash_domain::{ActionState, ReconcileOutcome, TimestampMs};
use unleash_protocol::canonical_sha256;

fn pending(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    c: &TerminalControlContext,
) -> Result<Option<TerminalRenewalRecord>, StoreError> {
    if d.schema_version < 16 {
        return Ok(None);
    }
    let id:Option<String>=tx.query_row("SELECT action_id FROM terminal_renewal_intents WHERE terminal_id=?1 AND state IN ('prepared','handshake_resolved','outcome_unknown')",
        [c.terminal_id.to_string()],|row|row.get(0)).optional()?;
    id.map(|id| read_renewal_conn(tx, id.parse()?))
        .transpose()
        .map(Option::flatten)
}

fn same_source(old: &TerminalRenewalRecord, c: &TerminalControlContext) -> Result<(), StoreError> {
    let r = &old.context;
    if c.terminal_id != r.terminal_id
        || c.initial_action_id != r.initial_action_id
        || c.initial_context_digest != r.initial_context_digest
        || c.host_id != r.host_id
        || c.process_id != r.process_id
        || c.process_run_uuid != r.process_run_uuid
        || c.actor_id != r.actor_id
        || c.principal_id != r.principal_id
        || c.project_id != r.project_id
        || c.mission_id != r.mission_id
        || c.authority_generation != r.authority_generation
        || c.daemon_boot_identity != r.daemon_boot_identity
        || c.daemon_epoch <= r.daemon_epoch
        || c.previous_epoch != r.daemon_epoch
        || c.previous_fence != r.resource_fence
        || c.previous_lease_id != r.lease_id
        || c.previous_control_revision != r.control_revision
        || c.next_fence <= r.resource_fence
        || c.expected_os_identity != r.expected_os_identity
    {
        return Err(invalid(
            "renewal quarantine requires an exact successor transfer",
        ));
    }
    Ok(())
}

pub(super) fn link_transfer_tx(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalControlContext,
    caller: &CallerAuthority,
    events: &mut Vec<EventAppendSpec>,
) -> Result<(), StoreError> {
    let Some(record) = pending(tx, d, c)? else {
        return Ok(());
    };
    same_source(&record, c)?;
    if caller.actor_id != c.actor_id || !caller.role.is_human_decision_maker() {
        return Err(invalid("renewal quarantine transfer caller differs"));
    }
    let dispatch = record
        .broker_dispatch_id
        .ok_or_else(|| invalid("prepared renewal without dispatch is not a transfer exception"))?;
    if record.dispatch_id.is_some_and(|id| id != dispatch)
        || !matches!(
            record.broker_action_state,
            Some(
                ActionState::Dispatching
                    | ActionState::InFlight
                    | ActionState::CancelRequested
                    | ActionState::OutcomeUnknown
            )
        )
    {
        return Err(invalid(
            "renewal quarantine has no exact unresolved broker marker",
        ));
    }
    if record.broker_action_state != Some(ActionState::OutcomeUnknown) {
        let current = crate::broker_writer::inspect_action_conn(tx, record.context.action_id)?
            .ok_or_else(|| invalid("renewal action disappeared"))?;
        crate::recovery_ops::tx_reconcile_action(
            tx,
            now,
            d.daemon_epoch,
            current.revision,
            c.command_id,
            record.context.action_id,
            caller,
            ReconcileOutcome::Unknown,
            None,
            None,
            "successor observes an unresolved exact renewal dispatch",
            Some(d.daemon_epoch),
            None,
            events,
        )?;
    }
    if matches!(
        record.state,
        TerminalRenewalState::Prepared | TerminalRenewalState::HandshakeResolved
    ) {
        if tx.execute("UPDATE terminal_renewal_intents SET state='outcome_unknown',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND revision=?3 AND state IN ('prepared','handshake_resolved')",
            params![record.context.action_id.to_string(),dispatch.to_string(),integer(record.revision.as_u64())?])?!=1 {
            return Err(invalid("renewal quarantine CAS lost"));
        }
    } else if record.state != TerminalRenewalState::OutcomeUnknown {
        return Err(invalid("renewal quarantine state differs"));
    }
    tx.execute("INSERT INTO terminal_renewal_transfer_links(renewal_action_id,renewal_dispatch_id,renewal_context_digest,transfer_action_id,transfer_receipt_id) VALUES(?1,?2,?3,?4,?5)",
        params![record.context.action_id.to_string(),dispatch.to_string(),record.context_digest.to_string(),c.action_id.to_string(),c.receipt_id.to_string()])?;
    Ok(())
}

pub(super) fn dispose_linked_transfer_tx(
    tx: &Transaction<'_>,
    control: &TerminalControlRecord,
    observation: &TerminalControlObservation,
) -> Result<(), StoreError> {
    let d = crate::commands::read_daemon_state_conn(tx)?;
    if d.schema_version < 16 {
        return Ok(());
    }
    let linked:Option<(String,String,String,String)>=tx.query_row(
        "SELECT renewal_action_id,renewal_dispatch_id,renewal_context_digest,transfer_receipt_id FROM terminal_renewal_transfer_links WHERE transfer_action_id=?1",
        [control.context.action_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    let Some((action, dispatch, digest, receipt)) = linked else {
        return Ok(());
    };
    let old =
        read_renewal_conn(tx, action.parse()?)?.ok_or_else(|| invalid("linked renewal missing"))?;
    same_source(&old, &control.context)?;
    let c = &control.context;
    if old.state != TerminalRenewalState::OutcomeUnknown
        || old.broker_action_state != Some(ActionState::OutcomeUnknown)
        || old.dispatch_id.map(|id| id.to_string()).as_deref() != Some(dispatch.as_str())
        || old.context_digest.to_string() != digest
        || c.receipt_id.to_string() != receipt
        || control.dispatch_id != Some(observation.dispatch_id)
        || observation.action_id != c.action_id
        || observation.context_digest != control.context_digest
        || observation.resource_fence != c.next_fence
        || observation.daemon_epoch != c.daemon_epoch
        || observation.os_identity != c.expected_os_identity
    {
        return Err(invalid(
            "linked transfer does not dispose the exact renewal",
        ));
    }
    let old_expiry: i64 = tx.query_row(
        "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
        [old.context.lease_id.to_string()],
        |row| row.get(0),
    )?;
    if old_expiry != old.context.previous_expires_at_ms.as_i64() {
        return Err(invalid(
            "unknown renewal deadline changed before disposition",
        ));
    }
    if tx.execute("UPDATE terminal_renewal_intents SET state='disposed',revision=revision+1,disposition_object_hash=?2,disposition_receipt_id=?3 WHERE action_id=?1 AND state='outcome_unknown' AND revision=?4 AND dispatch_id=?5",
        params![old.context.action_id.to_string(),canonical_sha256(observation)?.to_string(),c.receipt_id.to_string(),integer(old.revision.as_u64())?,dispatch])?!=1 {
        return Err(invalid("renewal disposition CAS lost"));
    }
    Ok(())
}
