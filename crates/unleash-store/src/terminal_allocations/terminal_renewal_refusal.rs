//! Durable disposition of a held-control intention with no dispatch marker.
use super::terminal_renewal::read_renewal_conn;
use super::terminal_renewal_observation::append_intent_event;
use super::terminal_renewal_types::{TerminalRenewalRecord, TerminalRenewalState};
use super::{integer, invalid, policy};
use crate::{
    BrokerAuthorizationOutcome, BrokerPhase, DaemonStateRow, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use unleash_domain::{ActionId, ActionState, Sha256Digest, TimestampMs};
use unleash_protocol::{EventCursor, RetryClass, canonical_sha256};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RefusalReason {
    DaemonSuperseded,
    PolicyChanged,
    AuthorizationDenied,
}

fn can_refuse(tx: &Transaction<'_>, record: &TerminalRenewalRecord) -> Result<bool, StoreError> {
    if record.state != TerminalRenewalState::Prepared
        || record.dispatch_id.is_some()
        || record.broker_dispatch_id.is_some()
        || !matches!(
            record.broker_action_state,
            None | Some(
                ActionState::Prepared
                    | ActionState::Authorizing
                    | ActionState::Authorized
                    | ActionState::Denied
            )
        )
    {
        return Ok(false);
    }
    let c = &record.context;
    Ok(tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=?1)
         AND NOT EXISTS(SELECT 1 FROM broker_dispatches WHERE action_id=?1)
         AND NOT EXISTS(SELECT 1 FROM broker_phase_records WHERE action_id=?1
             AND (phase_key='dispatch' OR phase_key LIKE 'observation:%'))
         AND NOT EXISTS(SELECT 1 FROM actions a WHERE a.action_id=?1 AND
            (a.state NOT IN ('prepared','authorizing','authorized','denied')
             OR a.control_principal_id IS NOT ?2 OR a.authority_generation!=?3 OR a.daemon_epoch!=?4
             OR NOT EXISTS(SELECT 1 FROM broker_intents i WHERE i.action_id=a.action_id
                           AND json_extract(i.intent_json,'$.payload_digest')=?5)))",
        params![
            c.action_id.to_string(),
            c.principal_id.as_str(),
            c.authority_generation.to_string(),
            integer(c.daemon_epoch.as_u64())?,
            record.context_digest.to_string()
        ],
        |row| row.get(0),
    )?)
}

fn current_reason(
    tx: &Transaction<'_>,
    record: &TerminalRenewalRecord,
) -> Result<Option<RefusalReason>, StoreError> {
    let c = &record.context;
    if policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest) {
        return Ok(Some(RefusalReason::PolicyChanged));
    }
    if record.broker_action_state != Some(ActionState::Denied) {
        return Ok(None);
    }
    let persisted: Option<(String, String)> = tx.query_row(
        "SELECT p.phase_json,p.phase_digest FROM broker_phase_records p
         JOIN broker_evaluations e ON e.evaluation_id=p.evaluation_id AND e.action_id=p.action_id
         WHERE p.action_id=?1 AND p.phase_key='authorization' AND e.state='denied'
          AND NOT EXISTS(SELECT 1 FROM broker_evaluations n WHERE n.action_id=e.action_id AND n.ordinal>e.ordinal)",
        [c.action_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let Some((encoded, digest)) = persisted else {
        return Ok(None);
    };
    let phase: serde_json::Value = serde_json::from_str(&encoded)?;
    if canonical_sha256(&phase)?.to_string() != digest {
        return Err(invalid("terminal authorization proof digest differs"));
    }
    let Some(phase) = phase.get("phase") else {
        return Err(invalid("terminal authorization lacks a phase"));
    };
    let phase: BrokerPhase = serde_json::from_value(phase.clone())?;
    Ok(match phase {
        BrokerPhase::Authorization {
            action_id,
            outcome: BrokerAuthorizationOutcome::Failed { failure },
            ..
        } if action_id == c.action_id && failure.retry_class == RetryClass::Never => {
            Some(RefusalReason::AuthorizationDenied)
        }
        _ => None,
    })
}

pub(super) fn refuse_tx(
    tx: &Transaction<'_>,
    authority: (&StoreConfig, &DaemonStateRow),
    now: TimestampMs,
    record: &TerminalRenewalRecord,
    reason: RefusalReason,
) -> Result<EventCursor, StoreError> {
    let (config, daemon) = authority;
    if !can_refuse(tx, record)? {
        return Err(invalid(
            "terminal intention has a dispatch, observation or pending decision",
        ));
    }
    let c = &record.context;
    if matches!(record.broker_action_state, Some(state) if state != ActionState::Denied) {
        let changed = tx.execute(
            "UPDATE actions SET state='cancelled',revision=revision+1
             WHERE action_id=?1 AND state IN ('prepared','authorizing','authorized')
             AND authority_generation=?2 AND daemon_epoch=?3 AND control_principal_id=?4
             AND EXISTS(SELECT 1 FROM broker_intents i WHERE i.action_id=actions.action_id
                        AND json_extract(i.intent_json,'$.payload_digest')=?5)",
            params![
                c.action_id.to_string(),
                c.authority_generation.to_string(),
                integer(c.daemon_epoch.as_u64())?,
                c.principal_id.as_str(),
                record.context_digest.to_string()
            ],
        )?;
        if changed != 1 {
            return Err(invalid(
                "terminal refusal differs from its broker intention",
            ));
        }
    }
    let payload = serde_json::json!({
        "action_id": c.action_id, "command_id": c.command_id, "receipt_id": c.receipt_id,
        "terminal_id": c.terminal_id, "context_digest": record.context_digest, "reason": reason,
        "operation": if c.resize.is_some() { "resize" } else { "renewal" }
    });
    let (event, cursor) = append_intent_event(
        tx,
        config,
        daemon,
        now,
        record,
        "TerminalControlDispatchRefused",
        &payload,
    )?;
    let changed = tx.execute(
        "UPDATE terminal_renewal_intents SET refusal_event_id=?2,revision=revision+1
         WHERE action_id=?1 AND state='prepared' AND revision=?3
           AND refusal_event_id IS NULL AND dispatch_id IS NULL",
        params![
            c.action_id.to_string(),
            event,
            integer(record.revision.as_u64())?
        ],
    )?;
    if changed != 1 {
        return Err(invalid("terminal refusal lost its pending intention"));
    }
    Ok(cursor)
}

impl UnleashStore {
    /// Record a definite pre-dispatch refusal of the exact current intention.
    ///
    /// The transaction derives the reason from durable policy or authorization
    /// facts. A missing reason, a durable dispatch or a previous disposition
    /// returns no new event.
    ///
    /// # Errors
    /// A storage error or an inconsistency in the matching intention is reported.
    pub fn dispose_undispatched_terminal_renewal(
        &self,
        action: ActionId,
        digest: Sha256Digest,
    ) -> Result<Option<EventCursor>, StoreError> {
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let d = crate::commands::read_daemon_state_conn(tx)?;
            if d.schema_version < 17 {
                return Ok(None);
            }
            let Some(record) = read_renewal_conn(tx, action)? else {
                return Ok(None);
            };
            let c = &record.context;
            if record.context_digest != digest
                || c.authority_generation != d.authority_generation
                || c.daemon_epoch != d.daemon_epoch
                || c.daemon_boot_identity != d.boot_identity
                || !can_refuse(tx, &record)?
            {
                return Ok(None);
            }
            let Some(reason) = current_reason(tx, &record)? else {
                return Ok(None);
            };
            refuse_tx(tx, (&config, &d), now, &record, reason).map(Some)
        })
    }
}
