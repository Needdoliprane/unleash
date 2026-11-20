//! Reconcile prior-epoch invocation attempts while retaining their resources.

use rusqlite::{Transaction, named_params, params};
use serde_json::json;
use unleash_domain::{Revision, TimestampMs};
use unleash_protocol::{EventCausationKind, EventEnvelope, Sensitivity};
use unleash_store::{DaemonStateRow, StoreConfig, StoreError};
use uuid::Uuid;

use super::insert_system_event_tx;

struct PreviousAttempt {
    invocation: String,
    attempt: String,
    invocation_revision: i64,
    project: Option<String>,
    epoch: i64,
    fence: i64,
    reservation: Option<String>,
    lease: Option<String>,
    start_sequence: Option<i64>,
}

pub(super) fn reconcile_invocations(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    recovery_started: Uuid,
    now: TimestampMs,
) -> Result<Vec<EventEnvelope>, StoreError> {
    let epoch = i64::try_from(daemon.daemon_epoch.as_u64())
        .map_err(|_| invalid("époque de récupération hors plage"))?;
    let revision = i64::try_from(daemon.revision.as_u64())
        .map_err(|_| invalid("révision daemon hors plage"))?;
    let sequence: i64 = tx.query_row(
        "SELECT sequence FROM events WHERE event_id=?1 AND stream_id=?2
           AND aggregate_type='daemon' AND aggregate_id=?3 AND aggregate_revision=?4
           AND event_type='DaemonRecoveryStarted' AND schema_version=1 AND payload_state='full'
           AND json_extract(payload_json,'$.daemon_epoch')=?5
           AND json_extract(payload_json,'$.daemon_instance_id')=?6",
        params![
            recovery_started.to_string(),
            daemon.stream_id.to_string(),
            daemon.authority_generation.to_string(),
            revision,
            epoch,
            daemon.daemon_instance_id.to_string()
        ],
        |r| r.get(0),
    )?;
    let attempts = prior_attempts(tx, daemon, epoch, sequence)?;
    let mut events = Vec::with_capacity(attempts.len());
    for attempt in attempts {
        events.push(record_recovery(
            tx,
            config,
            daemon,
            now,
            recovery_started,
            &attempt,
        )?);
    }
    // Quarantine remaining active invocations, including legacy rows and
    // attempts whose authority or epoch we cannot attribute to this recovery.
    // Only the invocation changes when the binding is unavailable.
    tx.execute(
        "UPDATE invocations SET state='reconciling',revision=revision+1
         WHERE state IN ('admitted','running','checkpointing')",
        [],
    )?;
    Ok(events)
}

fn prior_attempts(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    epoch: i64,
    recovery_sequence: i64,
) -> Result<Vec<PreviousAttempt>, StoreError> {
    let mut stmt = tx.prepare(
        "WITH retained_slots AS (
           SELECT c.reservation_id, c.authority_generation, c.daemon_epoch, o.invocation_id, o.attempt_id
           FROM capacity_reservations c JOIN execution_reservation_owners o USING(reservation_id)
           JOIN capacity_pools p USING(pool_id)
           WHERE c.owner_type='agent_invocation' AND c.owner_id=o.invocation_id
             AND c.state IN ('held','reconciling') AND c.amount=1
             AND p.pool_kind='execution' AND p.scope_key='global' AND p.unit='slot'
         )
         SELECT i.invocation_id,t.attempt_id,i.revision,a.project_id,t.daemon_epoch,t.fence,
                c.reservation_id,l.lease_id,
                (SELECT CASE WHEN COUNT(*)=1 THEN MIN(e.sequence) END
                 FROM events e WHERE e.stream_id=:stream AND e.sequence<:recovery
                   AND e.aggregate_type='invocation' AND e.aggregate_id=i.invocation_id
                   AND e.event_type='InvocationStarted' AND e.schema_version=1 AND e.payload_state='full'
                   AND e.aggregate_revision<=i.revision AND e.project_id IS a.project_id
                   AND json_extract(e.payload_json,'$.attempt_id')=t.attempt_id
                   AND json_extract(e.payload_json,'$.authority_generation')=t.authority_generation
                   AND json_extract(e.payload_json,'$.daemon_epoch')=t.daemon_epoch
                   AND json_extract(e.payload_json,'$.fence')=t.fence
                   AND json_extract(e.payload_json,'$.reservation_id')=c.reservation_id
                   AND json_extract(e.payload_json,'$.lease_id')=l.lease_id)
         FROM invocations i
         JOIN agents a USING(agent_id)
         JOIN invocation_attempts t USING(invocation_id)
         LEFT JOIN retained_slots c ON c.invocation_id=i.invocation_id AND c.attempt_id=t.attempt_id
              AND c.authority_generation=t.authority_generation AND c.daemon_epoch=t.daemon_epoch
         LEFT JOIN resource_leases l ON l.owner_type='agent_invocation' AND l.owner_id=i.invocation_id
              AND l.resource_kind='execution' AND l.resource_key='agent_invocation:' || i.invocation_id
              AND l.authority_generation=t.authority_generation AND l.daemon_epoch=t.daemon_epoch
              AND l.fence=t.fence AND l.state IN ('suspect','reconciling')
         WHERE i.state IN ('admitted','running','checkpointing','reconciling')
           AND t.state IN ('admitted','running','checkpointing','reconciling')
           AND (i.state!='reconciling' OR t.state!='reconciling')
           AND t.authority_generation=:authority AND t.daemon_epoch<:epoch
         ORDER BY i.invocation_id",
    )?;
    let attempts = stmt
        .query_map(
            named_params! {":stream":daemon.stream_id.to_string(), ":recovery":recovery_sequence,
            ":authority":daemon.authority_generation.to_string(), ":epoch":epoch},
            |r| {
                Ok(PreviousAttempt {
                    invocation: r.get(0)?,
                    attempt: r.get(1)?,
                    invocation_revision: r.get(2)?,
                    project: r.get(3)?,
                    epoch: r.get(4)?,
                    fence: r.get(5)?,
                    reservation: r.get(6)?,
                    lease: r.get(7)?,
                    start_sequence: r.get(8)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(attempts)
}

fn record_recovery(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    recovery_started: Uuid,
    attempt: &PreviousAttempt,
) -> Result<EventEnvelope, StoreError> {
    let next = attempt
        .invocation_revision
        .checked_add(1)
        .filter(|revision| *revision <= 9_007_199_254_740_991)
        .ok_or_else(|| invalid("révision de réconciliation hors plage"))?;
    let invocation = attempt
        .invocation
        .parse::<Uuid>()
        .map_err(|_| invalid("invocation historique illisible"))?;
    let project = attempt.project.as_deref().map(str::parse).transpose()?;
    let invocations = tx.execute(
        "UPDATE invocations SET state='reconciling',revision=?2
         WHERE invocation_id=?1 AND revision=?3 AND state IN ('admitted','running','checkpointing','reconciling')",
        params![attempt.invocation, next, attempt.invocation_revision],
    )?;
    let attempts = tx.execute(
        "UPDATE invocation_attempts SET state='reconciling' WHERE attempt_id=?1
           AND invocation_id=?2 AND authority_generation=?3 AND daemon_epoch=?4 AND fence=?5
           AND state IN ('admitted','running','checkpointing','reconciling')",
        params![
            attempt.attempt,
            attempt.invocation,
            daemon.authority_generation.to_string(),
            attempt.epoch,
            attempt.fence
        ],
    )?;
    if invocations != 1 || attempts != 1 {
        return Err(invalid("tentative modifiée pendant la récupération"));
    }
    let payload = [
        ("reason", json!("daemon_recovery_classification")),
        ("attempt_id", json!(attempt.attempt)),
        ("authority_generation", json!(daemon.authority_generation)),
        ("daemon_epoch", json!(attempt.epoch)),
        ("recovery_epoch", json!(daemon.daemon_epoch)),
        ("fence", json!(attempt.fence)),
        ("reservation_id", json!(attempt.reservation)),
        ("lease_id", json!(attempt.lease)),
        ("start_sequence", json!(attempt.start_sequence)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    let event = insert_system_event_tx(
        tx,
        config.id_generator.new_uuid(),
        daemon.stream_id,
        "invocation",
        invocation,
        Revision::new(
            u64::try_from(next).map_err(|_| invalid("révision de réconciliation négative"))?,
        ),
        0,
        "InvocationReconciling",
        project,
        &format!("uid:{}", daemon.owner_uid),
        recovery_started,
        EventCausationKind::Event,
        &recovery_started.to_string(),
        payload,
        Sensitivity::Restricted,
        now,
    )?;
    for destination in ["clients", "harness"] {
        tx.execute(
            "INSERT INTO outbox(event_id,destination,state,next_attempt_at_ms,authority_generation,consumer_version,delivery_key)
             VALUES(?1,?2,'pending',?3,?4,?5,?6)",
            params![event.event_id.to_string(), destination, now.as_i64(), daemon.authority_generation.to_string(),
                config.consumer_version, format!("{}:{destination}", event.event_id)],
        )?;
    }
    Ok(event)
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvariantViolation(message.to_owned())
}
