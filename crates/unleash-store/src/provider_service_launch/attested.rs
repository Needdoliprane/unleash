//! Read the exact authenticated service observation for a committed dispatch.

use rusqlite::{Connection, OptionalExtension as _};
use serde::Deserialize;
use unleash_domain::{
    ActionId, ActionState, DaemonMode, DispatchId, Sequence, Sha256Digest, TimestampMs,
};
use unleash_protocol::{EventCursor, canonical_sha256};

use super::{ProviderServiceLaunchAttestation, ProviderServiceLaunchContext, read_context};
use crate::{StoreError, UnleashStore, commands::read_daemon_state_conn};

/// Historical launch observation tied to the exact dispatch in the current incarnation.
pub struct AttestedProviderServiceLaunch {
    /// Immutable preparation, including the selected local provider profile.
    pub context: ProviderServiceLaunchContext,
    /// Native identity observed by the daemon's owned host at launch.
    pub attestation: ProviderServiceLaunchAttestation,
    /// Canonical digest of the attestation.
    pub attestation_digest: Sha256Digest,
    /// Exact event which recorded it.
    pub event_cursor: EventCursor,
    /// Time the durable observation was recorded.
    pub recorded_at: TimestampMs,
}

#[derive(Deserialize)]
struct ObservationPayload {
    action_id: ActionId,
    dispatch_id: DispatchId,
    state: ActionState,
    output_digest: Sha256Digest,
    provider_service_launch: ProviderServiceLaunchAttestation,
}

fn unavailable() -> StoreError {
    StoreError::EvidenceUnavailable("attested provider service launch unavailable".into())
}

pub(crate) fn read_attested_launch(
    conn: &Connection,
    dispatch: DispatchId,
) -> Result<AttestedProviderServiceLaunch, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.mode != DaemonMode::Ready {
        return Err(unavailable());
    }
    let (launch_id, body, digest, at, sequence): (String, String, String, i64, i64) = conn
        .query_row(
            "SELECT l.launch_id,e.payload_json,e.payload_hash,e.recorded_at_ms,e.sequence
             FROM provider_service_launches l
             JOIN actions a ON a.action_id=l.action_id
             JOIN broker_dispatches b ON b.action_id=a.action_id
             JOIN action_dispatches d ON d.dispatch_id=b.dispatch_id AND d.action_id=b.action_id
             JOIN broker_phase_records r ON r.action_id=b.action_id AND r.evaluation_id=b.evaluation_id
             JOIN events e ON e.event_id=r.event_id
             WHERE b.dispatch_id=?1 AND r.phase_key='observation:1'
               AND e.event_type='ActionObserved' AND e.payload_state='full'
               AND e.stream_id=?2 AND e.aggregate_id=a.action_id
               AND a.state='observed_success' AND d.transport_state='observed_success'",
            rusqlite::params![dispatch.to_string(), daemon.stream_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?
        .ok_or_else(unavailable)?;
    let launch_id = launch_id.parse().map_err(|_| unavailable())?;
    let context = read_context(conn, launch_id)?.ok_or_else(unavailable)?;
    let document: serde_json::Value = serde_json::from_str(&body)?;
    if canonical_sha256(&document)?.as_prefixed_str() != digest {
        return Err(unavailable());
    }
    let event: ObservationPayload = serde_json::from_value(document)?;
    let proof = event.provider_service_launch;
    if context.lineage_fence.authority_generation != daemon.authority_generation
        || context.lineage_fence.epoch != daemon.daemon_epoch
        || event.action_id != context.action_id
        || event.dispatch_id != dispatch
        || event.state != ActionState::ObservedSuccess
        || canonical_sha256(&proof)? != event.output_digest
        || canonical_sha256(&context)? != proof.context_digest
        || proof.confinement_digest != context.plan.confinement_digest
        || proof.environment_digest != context.plan.environment_digest
        || proof.host.boot_identity != daemon.boot_identity
        || proof.child.boot_identity != daemon.boot_identity
    {
        return Err(unavailable());
    }
    Ok(AttestedProviderServiceLaunch {
        context,
        attestation: proof,
        attestation_digest: event.output_digest,
        event_cursor: EventCursor::new(
            daemon.stream_id,
            Sequence::new(u64::try_from(sequence).map_err(|_| unavailable())?),
        ),
        recorded_at: TimestampMs::new(at),
    })
}

impl UnleashStore {
    /// Read the exact historical record before seeking current local listener evidence.
    ///
    /// # Errors
    /// Refuses missing, unknown, inconsistent or stale-incarnation evidence.
    pub fn read_attested_provider_service_launch(
        &self,
        dispatch: DispatchId,
    ) -> Result<AttestedProviderServiceLaunch, StoreError> {
        self.read_snapshot(|conn, _| read_attested_launch(conn, dispatch))
    }
}
