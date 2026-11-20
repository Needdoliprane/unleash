//! Journal the loss of the exact worker continuation without disposing its resources.

use rusqlite::{OptionalExtension, Transaction, named_params, params};
use serde_json::{Value, json};
use unleash_domain::{DaemonMode, ProjectId, Revision, Sequence, TimestampMs};
use unleash_protocol::{EventCursor, canonical_sha256};

use crate::commands::{DaemonStateRow, read_daemon_state_tx};
use crate::leases::{FenceAssertion, tx_validate_lease_fence};
use crate::{StoreConfig, StoreError};

use super::super::{append_event, check_incarnation, integer, invalid};
use super::{
    MAX_WIRE_INTEGER, WorkerLaunchPreparation, WorkerProcessObservation, started_payload,
    validate_launch_sequence,
};

const RECONCILING: &str = "InvocationReconciling";

pub(crate) fn record_worker_continuation_lost(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    preparation: &WorkerLaunchPreparation,
    start: EventCursor,
    identity: &WorkerProcessObservation,
    process_exit_observed: bool,
) -> Result<EventCursor, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    let admission = &preparation.admission;
    check_incarnation(
        &daemon,
        admission.lease.authority_generation,
        admission.lease.daemon_epoch,
    )?;
    if !matches!(
        daemon.mode,
        DaemonMode::Ready | DaemonMode::Draining | DaemonMode::Degraded
    ) {
        return Err(invalid(
            "mode daemon incompatible avec la perte de continuation",
        ));
    }
    if start.stream_id != daemon.stream_id
        || start.sequence <= preparation.event_cursor.sequence
        || identity.boot_identity != daemon.boot_identity
        || identity.owner_uid != daemon.owner_uid
    {
        return Err(stale());
    }
    validate_launch_sequence(start)?;
    let (running_revision, reconciliation_revision) = revisions(preparation.prepared_revision)?;
    let project = start_project(tx, preparation, start, identity, running_revision)?;
    let payload = json!({
        "reason": "worker_continuation_unconfirmed",
        "start_sequence": start.sequence,
        "attempt_id": preparation.attempt_id,
        "reservation_id": admission.reservation_id,
        "lease_id": admission.lease.lease_id,
        "authority_generation": daemon.authority_generation,
        "daemon_epoch": daemon.daemon_epoch,
        "fence": admission.lease.fence,
        "process_identity": identity,
        "process_exit_observed": process_exit_observed,
    });
    if let Some(cursor) = replay(tx, preparation, &daemon, reconciliation_revision, &payload)? {
        return Ok(cursor);
    }
    require_owned_attempt(tx, &daemon, preparation)?;
    move_to_reconciling(tx, preparation, running_revision, reconciliation_revision)?;
    let revision = u64::try_from(reconciliation_revision).map_err(|_| stale())?;
    let cursor = append_event(
        tx,
        config,
        &daemon,
        now,
        admission.unit,
        Revision::new(revision),
        project,
        RECONCILING,
        &payload,
    )?;
    validate_launch_sequence(cursor)?;
    Ok(cursor)
}

fn stale() -> StoreError {
    StoreError::StaleFence {
        message: "démarrage ou propriétaire de tentative non attribuable".to_owned(),
    }
}

fn revisions(prepared: Revision) -> Result<(i64, i64), StoreError> {
    let running = integer(prepared.as_u64())?
        .checked_add(1)
        .filter(|running| *running < MAX_WIRE_INTEGER)
        .ok_or_else(|| invalid("révision de réconciliation hors plage"))?;
    Ok((running, running + 1))
}

fn start_project(
    tx: &Transaction<'_>,
    preparation: &WorkerLaunchPreparation,
    start: EventCursor,
    identity: &WorkerProcessObservation,
    revision: i64,
) -> Result<Option<ProjectId>, StoreError> {
    let expected = started_payload(preparation, identity);
    let row: Option<(Option<String>, String)> = tx
        .query_row(
            "SELECT project_id,payload_json FROM events WHERE sequence=?1 AND stream_id=?2
           AND aggregate_type='invocation' AND aggregate_id=?3 AND aggregate_revision=?4
           AND event_type='InvocationStarted' AND schema_version=1 AND payload_state='full'
           AND payload_hash=?5",
            params![
                integer(start.sequence.as_u64())?,
                start.stream_id.to_string(),
                preparation.invocation_id.to_string(),
                revision,
                canonical_sha256(&expected)?.to_string()
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (project, raw) = row.ok_or_else(stale)?;
    if serde_json::from_str::<Value>(&raw)? != expected {
        return Err(stale());
    }
    project
        .map(|project| project.parse().map_err(StoreError::from))
        .transpose()
}

fn replay(
    tx: &Transaction<'_>,
    preparation: &WorkerLaunchPreparation,
    daemon: &DaemonStateRow,
    revision: i64,
    payload: &Value,
) -> Result<Option<EventCursor>, StoreError> {
    let prior: Option<(i64, Option<String>)> = tx
        .query_row(
            "SELECT sequence,payload_json FROM events WHERE stream_id=?1
           AND aggregate_type='invocation' AND aggregate_id=?2 AND aggregate_revision=?3
           AND event_type=?4 AND schema_version=1",
            params![
                daemon.stream_id.to_string(),
                preparation.invocation_id.to_string(),
                revision,
                RECONCILING
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((sequence, raw)) = prior else {
        return Ok(None);
    };
    let raw = raw.ok_or_else(stale)?;
    if serde_json::from_str::<Value>(&raw)? != *payload {
        return Err(stale());
    }
    let sequence = Sequence::new(u64::try_from(sequence).map_err(|_| stale())?);
    let cursor = EventCursor::new(daemon.stream_id, sequence);
    validate_launch_sequence(cursor)?;
    Ok(Some(cursor))
}

fn require_owned_attempt(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    preparation: &WorkerLaunchPreparation,
) -> Result<(), StoreError> {
    let admission = &preparation.admission;
    let invocation = preparation.invocation_id.to_string();
    tx_validate_lease_fence(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        &FenceAssertion {
            lease_id: admission.lease.lease_id,
            authority_generation: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            fence: admission.lease.fence,
            owner_id: invocation.clone(),
        },
    )?;
    let owned: bool = tx.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM execution_reservation_owners o
           JOIN capacity_reservations c USING(reservation_id)
           JOIN capacity_pools p USING(pool_id)
           JOIN invocation_attempts t ON t.attempt_id=o.attempt_id AND t.invocation_id=o.invocation_id
           JOIN resource_leases l ON l.lease_id=:lease AND l.owner_id=o.invocation_id
           WHERE o.reservation_id=:reservation AND o.invocation_id=:invocation AND o.attempt_id=:attempt
             AND c.owner_id=:invocation AND c.owner_type='agent_invocation' AND c.amount=1 AND c.state='held'
             AND c.authority_generation=:authority AND c.daemon_epoch=:epoch
             AND p.pool_kind='execution' AND p.scope_key='global' AND p.unit='slot'
             AND t.state='running' AND t.authority_generation=:authority AND t.daemon_epoch=:epoch AND t.fence=:fence
             AND l.state='held' AND l.owner_type='agent_invocation' AND l.resource_kind='execution'
             AND l.resource_key=:resource AND l.authority_generation=:authority AND l.boot_identity=:boot
         )",
        named_params! {
            ":lease": admission.lease.lease_id.to_string(),
            ":reservation": admission.reservation_id.to_string(),
            ":invocation": invocation,
            ":attempt": preparation.attempt_id.to_string(),
            ":authority": daemon.authority_generation.to_string(),
            ":epoch": integer(daemon.daemon_epoch.as_u64())?,
            ":fence": integer(admission.lease.fence.as_u64())?,
            ":resource": format!("agent_invocation:{invocation}"),
            ":boot": daemon.boot_identity,
        },
        |r| r.get(0),
    )?;
    // Expiry forbids new effects; it does not erase a retained lease's loss observation.
    if !owned {
        return Err(stale());
    }
    Ok(())
}

fn move_to_reconciling(
    tx: &Transaction<'_>,
    preparation: &WorkerLaunchPreparation,
    running: i64,
    reconciling: i64,
) -> Result<(), StoreError> {
    let invocation = preparation.invocation_id.to_string();
    let changed = tx.execute(
        "UPDATE invocations SET state='reconciling',revision=?1
         WHERE invocation_id=?2 AND state='running' AND revision=?3",
        params![reconciling, invocation, running],
    )?;
    let attempt = tx.execute(
        "UPDATE invocation_attempts SET state='reconciling'
         WHERE attempt_id=?1 AND invocation_id=?2 AND state='running'",
        params![preparation.attempt_id.to_string(), invocation],
    )?;
    if changed != 1 || attempt != 1 {
        return Err(stale());
    }
    Ok(())
}
