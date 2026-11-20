//! One-shot claim of the exact admitted worker launch, before native startup.

mod continuation;
pub(crate) use continuation::record_worker_continuation_lost;

use rusqlite::{OptionalExtension, Transaction, named_params, params};
use serde::Serialize;
use serde_json::json;
use unleash_domain::{
    AgentId, AttemptId, InvocationId, ProjectId, Revision, SessionId, Sha256Digest, TimestampMs,
};
use unleash_protocol::EventCursor;
use uuid::Uuid;

use crate::commands::{DaemonStateRow, read_daemon_state_tx};
use crate::leases::{FenceAssertion, tx_validate_lease_fence};
use crate::{StoreConfig, StoreError};

use super::{
    AdmissionCommit, AdmissionUnit, append_event, check_lineage, integer, invalid,
    validate_scope_and_subjects,
};

const MAX_WIRE_INTEGER: i64 = 9_007_199_254_740_991;

/// Admitted identity and manifest reference, committed once by the current writer.
#[derive(Debug, Clone)]
pub struct WorkerLaunchPreparation {
    admission: AdmissionCommit,
    /// Invocation associated with the lease and reservation.
    pub invocation_id: InvocationId,
    /// Exact active invocation attempt.
    pub attempt_id: AttemptId,
    /// Logical agent.
    pub agent_id: AgentId,
    /// Open session associated with the attempt's context.
    pub session_id: SessionId,
    /// Frozen scope identifier.
    pub scope_id: String,
    /// Frozen scope revision.
    pub scope_revision: Revision,
    /// Context manifest used by this attempt.
    pub context_manifest_id: Uuid,
    /// Digest of the published object containing the manifest.
    pub context_manifest_digest: Sha256Digest,
    /// Invocation revision after the claim, still in `admitted`.
    pub prepared_revision: Revision,
    /// Durable launch claim event, not a native start observation.
    pub event_cursor: EventCursor,
}

/// Native identity observed by the daemon on its owned child after Ready.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerProcessObservation {
    /// PID of the owned child.
    pub pid: u32,
    /// UID reported by the OS for the child.
    pub owner_uid: u32,
    /// Native boot identity.
    pub boot_identity: String,
    /// Native process creation identity, used in addition to the PID.
    pub start_identity: String,
}

#[derive(Clone, Copy)]
enum LaunchAnchor {
    Admitted(EventCursor),
    Prepared(EventCursor),
}

impl LaunchAnchor {
    const fn cursor(self) -> EventCursor {
        match self {
            Self::Admitted(cursor) | Self::Prepared(cursor) => cursor,
        }
    }

    const fn event_type(self) -> &'static str {
        match self {
            Self::Admitted(_) => "InvocationAdmitted",
            Self::Prepared(_) => "InvocationWorkerLaunchPrepared",
        }
    }
}

#[derive(Debug)]
struct LaunchContext {
    agent: AgentId,
    session: SessionId,
    project: Option<ProjectId>,
    manifest: Uuid,
    digest: Sha256Digest,
    scope: String,
    scope_revision: Revision,
}

pub(crate) fn claim_worker_launch(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    admission: &AdmissionCommit,
) -> Result<WorkerLaunchPreparation, StoreError> {
    let (AdmissionUnit::Invocation(invocation), Some(attempt)) =
        (admission.unit, admission.attempt_id)
    else {
        return Err(invalid("l'admission n'est pas une tentative agente"));
    };
    let anchor = LaunchAnchor::Admitted(admission.event_cursor);
    let daemon = validate_launch_authority(tx, config, admission, invocation, anchor)?;
    require_admitted_revision(tx, invocation, admission.revision)?;
    let context = read_launch_context(tx, &daemon, now, admission, (invocation, attempt), anchor)?;
    validate_scope_and_subjects(tx, admission.unit, context.project)?;
    persist_claim(
        tx,
        config,
        &daemon,
        now,
        admission,
        (invocation, attempt),
        context,
    )
}

fn validate_launch_authority(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    admission: &AdmissionCommit,
    invocation: InvocationId,
    anchor: LaunchAnchor,
) -> Result<DaemonStateRow, StoreError> {
    let daemon = read_daemon_state_tx(tx)?;
    check_lineage(
        &daemon,
        admission.lease.authority_generation,
        admission.lease.daemon_epoch,
    )?;
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid("horloge inconnue pendant la préparation du worker"));
    }
    if anchor.cursor().stream_id != daemon.stream_id {
        return Err(invalid("preuve de lancement d’un autre flux"));
    }
    validate_launch_sequence(anchor.cursor())?;
    if !(1..=MAX_WIRE_INTEGER).contains(&integer(daemon.daemon_epoch.as_u64())?)
        || !(1..=MAX_WIRE_INTEGER).contains(&integer(admission.lease.fence.as_u64())?)
    {
        return Err(invalid("coordonnées de démarrage hors plage"));
    }
    tx_validate_lease_fence(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        &FenceAssertion {
            lease_id: admission.lease.lease_id,
            authority_generation: admission.lease.authority_generation,
            daemon_epoch: admission.lease.daemon_epoch,
            fence: admission.lease.fence,
            owner_id: invocation.to_string(),
        },
    )?;
    Ok(daemon)
}

fn validate_launch_sequence(cursor: EventCursor) -> Result<(), StoreError> {
    let in_range = i64::try_from(cursor.sequence.as_u64())
        .is_ok_and(|sequence| (1..=MAX_WIRE_INTEGER).contains(&sequence));
    if !in_range {
        return Err(StoreError::InvariantViolation(
            "séquence de lancement hors plage".to_owned(),
        ));
    }
    Ok(())
}

fn require_admitted_revision(
    tx: &Transaction<'_>,
    invocation: InvocationId,
    expected: Revision,
) -> Result<(), StoreError> {
    let revision: i64 = tx
        .query_row(
            "SELECT revision FROM invocations WHERE invocation_id=?1 AND state='admitted'",
            [invocation.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("invocation non admise"))?;
    let current = Revision::new(
        u64::try_from(revision).map_err(|_| invalid("révision invocation invalide"))?,
    );
    if current != expected {
        return Err(StoreError::RevisionConflict {
            current_revision: current,
            expected_revision: expected,
            command_id: None,
        });
    }
    Ok(())
}

fn read_launch_context(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    admission: &AdmissionCommit,
    (invocation, attempt): (InvocationId, AttemptId),
    anchor: LaunchAnchor,
) -> Result<LaunchContext, StoreError> {
    let row = tx
        .query_row(
            "SELECT i.agent_id,i.session_id,a.project_id,m.manifest_id,m.content_hash,
                    b.scope_id,b.scope_revision
             FROM invocations i
             JOIN agents a ON a.agent_id=i.agent_id AND a.state IN ('eligible','active')
             JOIN sessions s ON s.session_id=i.session_id AND s.agent_id=i.agent_id AND s.state='open'
             JOIN context_manifests m ON m.manifest_id=i.context_manifest_id AND m.session_id=s.session_id
             JOIN object_records obj ON obj.object_hash=m.content_hash AND obj.state='live'
             JOIN effective_bundles b ON b.bundle_hash=i.effective_bundle_hash
             JOIN context_manifests original ON original.manifest_id=b.initial_context_manifest_id
                  AND original.session_id=s.session_id
             JOIN invocation_attempts t ON t.invocation_id=i.invocation_id AND t.attempt_id=:attempt
             JOIN resource_leases l ON l.lease_id=:lease AND l.owner_id=i.invocation_id
             JOIN capacity_reservations c ON c.reservation_id=:reservation AND c.owner_id=i.invocation_id
             JOIN execution_reservation_owners o ON o.reservation_id=c.reservation_id
                  AND o.invocation_id=i.invocation_id AND o.attempt_id=t.attempt_id
             JOIN capacity_pools p ON p.pool_id=c.pool_id
             JOIN events e ON e.sequence=:sequence AND e.stream_id=:stream
                  AND e.aggregate_type='invocation' AND e.aggregate_id=i.invocation_id
                  AND e.aggregate_revision=i.revision AND e.event_type=:event_type
                  AND e.project_id IS a.project_id
                  AND json_extract(e.payload_json, '$.reservation_id')=:reservation
                  AND json_extract(e.payload_json, '$.attempt_id')=:attempt
                  AND json_extract(e.payload_json, '$.lease_id')=:lease
                  AND json_extract(e.payload_json, '$.fence')=:fence
                  AND (:event_type='InvocationAdmitted' OR (
                       json_extract(e.payload_json, '$.authority_generation')=:authority
                       AND json_extract(e.payload_json, '$.daemon_epoch')=:epoch
                       AND json_extract(e.payload_json, '$.context_manifest_id')=m.manifest_id
                       AND json_extract(e.payload_json, '$.context_manifest_digest')=m.content_hash
                       AND json_extract(e.payload_json, '$.scope_id')=b.scope_id
                       AND json_extract(e.payload_json, '$.scope_revision')=b.scope_revision))
             WHERE i.invocation_id=:invocation AND i.state='admitted'
                  AND t.state='admitted' AND t.process_id IS NULL
                  AND t.authority_generation=:authority AND t.daemon_epoch=:epoch AND t.fence=:fence
                  AND l.state='held' AND l.owner_type='agent_invocation' AND l.resource_kind='execution'
                  AND l.resource_key=:resource AND l.authority_generation=:authority
                  AND l.daemon_epoch=:epoch AND l.fence=:fence AND l.boot_identity=:boot
                  AND l.expires_at_ms>:now
                  AND c.state='held' AND c.owner_type='agent_invocation' AND c.amount=1
                  AND c.authority_generation=:authority AND c.daemon_epoch=:epoch
                  AND c.admission_class=i.admission_class
                  AND p.pool_kind='execution' AND p.scope_key='global' AND p.unit='slot'
                  AND (a.project_id IS NULL OR EXISTS(
                        SELECT 1 FROM projects WHERE project_id=a.project_id AND state='active'))",
            named_params! {
                ":invocation": invocation.to_string(),
                ":attempt": attempt.to_string(),
                ":lease": admission.lease.lease_id.to_string(),
                ":reservation": admission.reservation_id.to_string(),
                ":sequence": integer(anchor.cursor().sequence.as_u64())?,
                ":event_type": anchor.event_type(),
                ":stream": daemon.stream_id.to_string(),
                ":authority": daemon.authority_generation.to_string(),
                ":epoch": integer(daemon.daemon_epoch.as_u64())?,
                ":fence": integer(admission.lease.fence.as_u64())?,
                ":resource": format!("agent_invocation:{invocation}"),
                ":boot": daemon.boot_identity,
                ":now": now.as_i64(),
            },
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::StaleFence {
            message: "tentative, manifeste ou réservation non attribuable au lancement".to_owned(),
        })?;
    if row.5.trim().is_empty() || !(0..=MAX_WIRE_INTEGER).contains(&row.6) {
        return Err(invalid("scope de démarrage invalide"));
    }
    Ok(LaunchContext {
        agent: row.0.parse()?,
        session: row.1.parse()?,
        project: row.2.map(|project| project.parse()).transpose()?,
        manifest: row
            .3
            .parse()
            .map_err(|_| invalid("manifeste sans identité valide"))?,
        digest: row.4.parse()?,
        scope: row.5,
        scope_revision: Revision::new(
            u64::try_from(row.6).map_err(|_| invalid("révision scope invalide"))?,
        ),
    })
}

fn persist_claim(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    admission: &AdmissionCommit,
    (invocation, attempt): (InvocationId, AttemptId),
    context: LaunchContext,
) -> Result<WorkerLaunchPreparation, StoreError> {
    let current = integer(admission.revision.as_u64())?;
    let next = current
        .checked_add(1)
        .filter(|next| *next <= MAX_WIRE_INTEGER)
        .ok_or_else(|| invalid("révision worker épuisée"))?;
    let updated = tx.execute(
        "UPDATE invocations SET revision=?1
         WHERE invocation_id=?2 AND revision=?3 AND state='admitted'",
        params![next, invocation.to_string(), current],
    )?;
    if updated != 1 {
        return Err(invalid("préparation worker concurrente"));
    }
    let prepared_revision =
        Revision::new(u64::try_from(next).map_err(|_| invalid("révision worker invalide"))?);
    let event_cursor = append_event(
        tx,
        config,
        daemon,
        now,
        admission.unit,
        prepared_revision,
        context.project,
        "InvocationWorkerLaunchPrepared",
        &json!({
            "attempt_id": attempt,
            "reservation_id": admission.reservation_id,
            "lease_id": admission.lease.lease_id,
            "authority_generation": daemon.authority_generation,
            "daemon_epoch": daemon.daemon_epoch,
            "fence": admission.lease.fence,
            "context_manifest_id": context.manifest,
            "context_manifest_digest": context.digest,
            "scope_id": context.scope,
            "scope_revision": context.scope_revision
        }),
    )?;
    validate_launch_sequence(event_cursor)?;
    Ok(WorkerLaunchPreparation {
        admission: admission.clone(),
        invocation_id: invocation,
        attempt_id: attempt,
        agent_id: context.agent,
        session_id: context.session,
        scope_id: context.scope,
        scope_revision: context.scope_revision,
        context_manifest_id: context.manifest,
        context_manifest_digest: context.digest,
        prepared_revision,
        event_cursor,
    })
}

pub(crate) fn commit_worker_running(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    preparation: &WorkerLaunchPreparation,
    identity: &WorkerProcessObservation,
) -> Result<EventCursor, StoreError> {
    let anchor = LaunchAnchor::Prepared(preparation.event_cursor);
    let admission = &preparation.admission;
    let invocation = preparation.invocation_id;
    let attempt = preparation.attempt_id;
    let daemon = validate_launch_authority(tx, config, admission, invocation, anchor)?;
    require_admitted_revision(tx, invocation, preparation.prepared_revision)?;
    if identity.pid == 0
        || identity.owner_uid != daemon.owner_uid
        || identity.boot_identity != daemon.boot_identity
        || identity.start_identity.trim().is_empty()
    {
        return Err(invalid(
            "identité native du worker non attribuable au daemon",
        ));
    }
    let context = read_launch_context(tx, &daemon, now, admission, (invocation, attempt), anchor)?;
    if context.agent != preparation.agent_id
        || context.session != preparation.session_id
        || context.manifest != preparation.context_manifest_id
        || context.digest != preparation.context_manifest_digest
        || context.scope != preparation.scope_id
        || context.scope_revision != preparation.scope_revision
    {
        return Err(invalid("contexte du worker modifié depuis sa préparation"));
    }
    validate_scope_and_subjects(tx, admission.unit, context.project)?;
    let revision = mark_invocation_running(tx, preparation)?;
    let cursor = append_event(
        tx,
        config,
        &daemon,
        now,
        admission.unit,
        revision,
        context.project,
        "InvocationStarted",
        &started_payload(preparation, identity),
    )?;
    validate_launch_sequence(cursor)?;
    Ok(cursor)
}

fn started_payload(
    preparation: &WorkerLaunchPreparation,
    identity: &WorkerProcessObservation,
) -> serde_json::Value {
    let admission = &preparation.admission;
    json!({
        "attempt_id": preparation.attempt_id,
        "reservation_id": admission.reservation_id,
        "lease_id": admission.lease.lease_id,
        "authority_generation": admission.lease.authority_generation,
        "daemon_epoch": admission.lease.daemon_epoch,
        "fence": admission.lease.fence,
        "prepared_sequence": preparation.event_cursor.sequence,
        "context_manifest_id": preparation.context_manifest_id,
        "context_manifest_digest": preparation.context_manifest_digest,
        "scope_id": preparation.scope_id,
        "scope_revision": preparation.scope_revision,
        "process_identity": identity
    })
}

fn mark_invocation_running(
    tx: &Transaction<'_>,
    preparation: &WorkerLaunchPreparation,
) -> Result<Revision, StoreError> {
    let current = integer(preparation.prepared_revision.as_u64())?;
    let next = current
        .checked_add(1)
        .filter(|next| *next <= MAX_WIRE_INTEGER)
        .ok_or_else(|| invalid("révision worker épuisée"))?;
    let invocation = preparation.invocation_id.to_string();
    let changed = tx.execute(
        "UPDATE invocations SET state='running',revision=?1
         WHERE invocation_id=?2 AND revision=?3 AND state='admitted'",
        params![next, invocation, current],
    )?;
    let attempt_changed = tx.execute(
        "UPDATE invocation_attempts SET state='running'
         WHERE attempt_id=?1 AND invocation_id=?2 AND state='admitted'",
        params![preparation.attempt_id.to_string(), invocation],
    )?;
    if changed != 1 || attempt_changed != 1 {
        return Err(invalid("lancement worker concurrent"));
    }
    Ok(Revision::new(
        u64::try_from(next).map_err(|_| invalid("révision worker invalide"))?,
    ))
}
