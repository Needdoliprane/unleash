//! Disposition of an earlier incarnation's unstarted release intentions.
use super::terminal_release::read_conn_release;
use super::terminal_release_types::{TerminalReleaseRecord, TerminalReleaseState};
use super::terminal_renewal_observation::{
    TerminalIntentEventTarget, append_terminal_intent_event,
};
use super::terminal_renewal_refusal::{RefusalReason, UndispatchedTerminalAction};
use super::{integer, invalid};
use crate::commands::read_daemon_state_conn;
use crate::{DaemonStateRow, StoreConfig, StoreError};
use rusqlite::{Transaction, params};
use unleash_domain::{ActionId, DaemonMode, TimestampMs};
use unleash_protocol::EventEnvelope;

/// Close unstarted terminal releases during the recovery transaction.
///
/// Each refusal keeps the exact original command and terminal control. A
/// dispatch or observation marker leaves the intention for reconciliation.
///
/// # Errors
/// Returns any storage or integrity failure so the caller can roll back recovery.
pub fn recover_undispatched_terminal_releases_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
) -> Result<Vec<EventEnvelope>, StoreError> {
    let daemon = read_daemon_state_conn(tx)?;
    if daemon.schema_version < 18 || daemon.mode != DaemonMode::Recovering {
        return Ok(Vec::new());
    }
    let mut statement = tx.prepare(
        "SELECT action_id FROM terminal_release_intents
         WHERE state='prepared' AND authority_generation=?1 AND daemon_epoch<?2
         ORDER BY daemon_epoch,action_id",
    )?;
    let pending = statement
        .query_map(
            params![
                daemon.authority_generation.to_string(),
                integer(daemon.daemon_epoch.as_u64())?
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    let mut events = Vec::new();
    for action in pending {
        let action_id: ActionId = action.parse()?;
        let record = read_conn_release(tx, action_id)?
            .ok_or_else(|| invalid("terminal release disappeared during recovery"))?;
        if let Some(event) = refuse_release(tx, config, &daemon, now, &record)? {
            events.push(event);
        }
    }
    Ok(events)
}

fn refuse_release(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &TerminalReleaseRecord,
) -> Result<Option<EventEnvelope>, StoreError> {
    let c = &record.context;
    let authority = UndispatchedTerminalAction {
        action_id: c.action_id,
        principal: &c.principal_id,
        authority: c.authority_generation,
        epoch: c.daemon_epoch,
        context_digest: record.context_digest,
    };
    if record.state != TerminalReleaseState::Prepared
        || record.dispatch_id.is_some()
        || record.broker_dispatch_id.is_some()
        || c.authority_generation != daemon.authority_generation
        || c.daemon_epoch >= daemon.daemon_epoch
        || !authority.has_no_effect_markers(tx)?
    {
        return Ok(None);
    }
    let receipt_matches = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts
         WHERE receipt_id=?1 AND command_id=?2 AND actor_id=?3 AND aggregate_id=?4
           AND authority_generation=?5 AND command_type='ReleaseTerminal' AND state='accepted')",
        params![
            c.receipt_id.to_string(),
            c.command_id.to_string(),
            c.actor_id.as_str(),
            c.terminal_id.to_string(),
            c.authority_generation.to_string(),
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !receipt_matches {
        return Err(invalid("terminal release refusal receipt differs"));
    }
    authority.cancel_pending(tx, record.broker_action_state)?;
    let event = append_terminal_intent_event(
        tx,
        config,
        daemon,
        now,
        TerminalIntentEventTarget {
            action: c.action_id,
            project: c.project_id,
            actor: &c.actor_id,
            receipt: c.receipt_id,
            terminal: c.terminal_id,
            previous_revision: record.revision,
        },
        "TerminalControlDispatchRefused",
        &serde_json::json!({
            "action_id": c.action_id, "command_id": c.command_id, "receipt_id": c.receipt_id,
            "terminal_id": c.terminal_id, "context_digest": record.context_digest,
            "reason": RefusalReason::DaemonSuperseded, "operation": "release"
        }),
    )?;
    let changed = tx.execute(
        "UPDATE terminal_release_intents SET state='not_dispatched',refusal_event_id=?2,revision=revision+1
         WHERE action_id=?1 AND state='prepared' AND revision=?3 AND dispatch_id IS NULL",
        params![c.action_id.to_string(), event.event_id.to_string(), integer(record.revision.as_u64())?],
    )?;
    if changed != 1 {
        return Err(invalid(
            "terminal release refusal lost its pending intention",
        ));
    }
    Ok(Some(event))
}
