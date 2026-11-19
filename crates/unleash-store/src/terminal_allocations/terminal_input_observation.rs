//! Commit a real native receipt as a fact independently of permission to renew
//! control. This path never sends input, resolves a permit or changes the broker
//! state machine. A previously Unknown ledger requires separate reconciliation.

use super::terminal_input_types::{
    TerminalInputContext, TerminalInputObservation, TerminalInputRecord, TerminalInputRenewal,
    TerminalInputState,
};
use super::terminal_input_writer::read_input_conn;
use super::{
    TerminalAllocationScope, TerminalObservationProof, control_holds, control_identity,
    control_lease, control_runtime, integer, invalid, key, policy, principal, read_conn,
    validate_preparation,
};
use crate::commands::read_daemon_state_conn;
use crate::{
    BrokerActionReceipt, BrokerObservationCause, BrokerPhase, BrokerPhaseCommit,
    BrokerPhaseRequest, DaemonStateRow, StoreConfig, StoreCutpoint, StoreError, UnleashStore,
    WriteCriticality,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::sync::atomic::Ordering;
use unleash_domain::{ActionState, Sha256Digest, TimestampMs};
use unleash_protocol::canonical_sha256;

impl UnleashStore {
    /// Record a published native receipt and its exact broker observation atomically.
    /// A true late receipt is retained even when its old lease cannot be renewed.
    ///
    /// # Errors
    /// Refuses missing/different proof, identity, dispatch, receipt or chronology.
    /// An already Unknown ledger is not reopened through this execution port.
    /// A lost commit ACK must replay the exact phase, never another native write.
    pub fn commit_terminal_input_observation(
        &self,
        observation: TerminalInputObservation,
        phase: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            commit_observation_tx(tx, &config, now, &observation, &phase)
        })?;
        if !committed.replayed {
            self.config()
                .cutpoints
                .hit(StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(committed)
    }
}

fn commit_observation_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    observation: &TerminalInputObservation,
    phase: &BrokerPhaseRequest,
) -> Result<BrokerPhaseCommit, StoreError> {
    let daemon = read_daemon_state_conn(tx)?;
    let input = read_input_conn(tx, observation.action_id)?
        .ok_or_else(|| invalid("terminal input observation has no intention"))?;
    let hash = canonical_sha256(observation)?;
    let receipt = validate_native_receipt(&input, observation, phase, hash)?;
    // Exact receipt replay precedes artifact/lease/policy checks. In particular
    // it cannot calculate a new deadline or repeat a native effect.
    if input.state == TerminalInputState::Observed {
        if input.observation_object_hash != Some(hash) {
            return Err(invalid("terminal input proof replay differs"));
        }
        return crate::broker_writer::commit_phase_tx(tx, config, now, phase);
    }
    if input.state != TerminalInputState::WriteResolved {
        return Err(invalid(
            "terminal input observation has no consumed pending permit",
        ));
    }
    let current = crate::broker_writer::inspect_action_conn(tx, input.context.action_id)?
        .ok_or_else(|| invalid("terminal input broker action missing"))?;
    if !matches!(
        current.state,
        ActionState::Dispatching | ActionState::InFlight | ActionState::CancelRequested
    ) || current.dispatch_id != input.dispatch_id
        || current.evaluation_id != phase.evaluation.evaluation_id
    {
        return Err(invalid(
            "terminal input ledger requires separate reconciliation",
        ));
    }
    super::validate_incarnation(tx, &daemon, input.context.action_id)?;
    let started: i64 = tx.query_row(
        "SELECT started_at_ms FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2",
        params![
            input.context.action_id.to_string(),
            observation.dispatch_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if observation.observed_at_ms.as_i64() < started
        || observation.observed_at_ms > receipt.observed_at_ms
    {
        return Err(invalid(
            "terminal input capture timestamp is outside dispatch and receipt",
        ));
    }
    verify_published_native_object(tx, config, observation, hash)?;
    let renewal_now = config.clock.now_ms();
    let (renewal, deadline) =
        renew_if_still_authorized(tx, config, &daemon, renewal_now, &input.context)?;
    record_native_sequence(tx, &input)?;
    let changed = tx.execute(
        "UPDATE terminal_input_intents SET state='observed',revision=revision+1,observation_object_hash=?2,renewal=?3,renewed_lease_expires_at=?4 WHERE action_id=?1 AND state='write_resolved' AND revision=?5 AND dispatch_id=?6",
        params![input.context.action_id.to_string(),hash.to_string(),renewal_name(renewal),deadline.map(|time| time.as_i64()),integer(input.revision.as_u64())?,observation.dispatch_id.to_string()],
    )?;
    if changed != 1 {
        return Err(invalid("terminal native observation CAS lost"));
    }
    let proof = TerminalObservationProof {
        phase_digest: canonical_sha256(&phase.phase)?,
    };
    crate::broker_writer::commit_terminal_observation_tx(tx, config, now, phase, &proof)
}

fn validate_native_receipt<'a>(
    input: &TerminalInputRecord,
    observation: &TerminalInputObservation,
    phase: &'a BrokerPhaseRequest,
    hash: Sha256Digest,
) -> Result<&'a BrokerActionReceipt, StoreError> {
    let c = &input.context;
    let BrokerPhase::Observation {
        action_id,
        receipt,
        cause,
        ..
    } = &phase.phase
    else {
        return Err(invalid("terminal input requires an observation phase"));
    };
    validate_native_observation_context(input, observation)?;
    if *action_id != c.action_id
        || receipt.action_id != c.action_id
        || phase.authority_generation != c.authority_generation
        || !matches!(cause, BrokerObservationCause::Execution)
        || receipt.state != ActionState::ObservedSuccess
        || receipt.output_digest != Some(hash)
        || receipt.dispatch_id != observation.dispatch_id
    {
        return Err(invalid(
            "terminal native input receipt differs from consumed request",
        ));
    }
    Ok(receipt)
}

pub(super) fn validate_native_observation_context(
    input: &TerminalInputRecord,
    observation: &TerminalInputObservation,
) -> Result<(), StoreError> {
    let c = &input.context;
    let native = &observation.native_receipt;
    if observation.action_id != c.action_id
        || input.dispatch_id != Some(observation.dispatch_id)
        || observation.context_digest != input.context_digest
        || observation.terminal_id != c.terminal_id
        || observation.host_id != c.host_id
        || observation.process_run_uuid != c.process_run_uuid
        || observation.daemon_epoch != c.daemon_epoch
        || observation.resource_fence != c.resource_fence
        || native.action_id != c.action_id
        || native.command_id != c.command_id
        || native.input_seq != c.input_seq
        || native.payload_hash != c.payload_digest
        || native.receipt_id.as_uuid().is_nil()
    {
        return Err(invalid(
            "terminal native input receipt differs from consumed request",
        ));
    }
    Ok(())
}

fn verify_published_native_object(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    observation: &TerminalInputObservation,
    hash: Sha256Digest,
) -> Result<(), StoreError> {
    let stored = read_published_native_observation(
        tx,
        config,
        observation.action_id,
        observation.dispatch_id,
        hash,
    )?;
    if stored != *observation {
        return Err(StoreError::EvidenceUnavailable(
            "native input observation bytes differ".into(),
        ));
    }
    Ok(())
}

pub(super) fn read_published_native_observation(
    conn: &Connection,
    config: &StoreConfig,
    action: unleash_domain::ActionId,
    dispatch: unleash_domain::DispatchId,
    hash: Sha256Digest,
) -> Result<TerminalInputObservation, StoreError> {
    let published: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_references WHERE owner_kind='terminal_input_observation' AND owner_id=?1 AND source_field=?2 AND object_hash=?3 AND state='live')",
        params![action.to_string(),dispatch.to_string(),hash.to_string()], |row|row.get(0),
    )?;
    if !published {
        return Err(StoreError::EvidenceUnavailable(
            "native input observation is not published".into(),
        ));
    }
    let bytes =
        crate::objects::read_verified_evidence_bytes_bounded(conn, config, hash, 2 * 1024 * 1024)?;
    let stored: TerminalInputObservation = serde_json::from_slice(&bytes)?;
    if canonical_sha256(&stored)? != hash {
        return Err(StoreError::EvidenceUnavailable(
            "native input observation bytes differ".into(),
        ));
    }
    Ok(stored)
}

fn record_native_sequence(
    tx: &Transaction<'_>,
    input: &TerminalInputRecord,
) -> Result<(), StoreError> {
    let c = &input.context;
    let previous = c
        .input_seq
        .checked_sub(1)
        .ok_or_else(|| invalid("native input sequence invalid"))?;
    let changed = tx.execute(
        "UPDATE terminal_sessions SET input_seq=?2 WHERE terminal_id=?1 AND process_id=?3 AND input_seq=?4 AND EXISTS(SELECT 1 FROM processes WHERE process_id=?3 AND process_run_uuid=?5 AND action_id=?6 AND host_id=?7 AND authority_generation=?8)",
        params![c.terminal_id.to_string(),integer(c.input_seq)?,c.process_id.to_string(),integer(previous)?,c.process_run_uuid.to_string(),c.initial_action_id.to_string(),c.host_id.to_string(),c.authority_generation.to_string()],
    )?;
    if changed != 1 {
        return Err(invalid("native terminal sequence CAS lost"));
    }
    Ok(())
}

fn renew_if_still_authorized(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalInputContext,
) -> Result<(TerminalInputRenewal, Option<TimestampMs>), StoreError> {
    use TerminalInputRenewal as Renewal;
    if daemon.mode != unleash_domain::DaemonMode::Ready {
        return Ok((Renewal::AuthorityBlocked, None));
    }
    if config.clock_uncertain.load(Ordering::Acquire) {
        return Ok((Renewal::ClockUncertain, None));
    }
    if c.authority_generation != daemon.authority_generation
        || c.daemon_epoch != daemon.daemon_epoch
        || c.daemon_boot_identity != daemon.boot_identity
    {
        return Ok((Renewal::AuthorityChanged, None));
    }
    let current: Option<(String,i64)> = tx.query_row("SELECT lease_id,control_revision FROM terminal_sessions WHERE terminal_id=?1 AND process_id=?2", params![c.terminal_id.to_string(),c.process_id.to_string()], |row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if current
        != Some((
            c.lease_id.to_string(),
            integer(c.control_revision.as_u64())?,
        ))
    {
        return Ok((Renewal::ControllerChanged, None));
    }
    let lease = control_lease(tx, c.lease_id)?;
    let expires: i64 = tx.query_row(
        "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
        [c.lease_id.to_string()],
        |row| row.get(0),
    )?;
    if lease.state != "held"
        || lease.authority_generation != c.authority_generation
        || lease.daemon_epoch != c.daemon_epoch
        || lease.fence != c.resource_fence
        || lease.boot_identity != c.daemon_boot_identity
        || lease.owner_type != "principal"
        || lease.owner_id != c.principal_id.as_str()
    {
        return Ok((Renewal::ControllerChanged, None));
    }
    if expires <= now.as_i64() {
        return Ok((Renewal::LeaseExpired, None));
    }
    if renewal_authority(tx, daemon, now, c).is_err() {
        return Ok((Renewal::AuthorityBlocked, None));
    }
    let latest = config.clock.now_ms();
    if config.clock_uncertain.load(Ordering::Acquire) || latest < now {
        return Ok((Renewal::ClockUncertain, None));
    }
    if expires <= latest.as_i64() {
        return Ok((Renewal::LeaseExpired, None));
    }
    let deadline = TimestampMs::new(expires.max(c.proposed_lease_expires_at.as_i64()));
    let changed = tx.execute(
        "UPDATE resource_leases SET expires_at_ms=?2 WHERE lease_id=?1 AND state='held' AND expires_at_ms=?3 AND expires_at_ms>?4 AND authority_generation=?5 AND daemon_epoch=?6 AND fence=?7 AND boot_identity=?8",
        params![c.lease_id.to_string(),deadline.as_i64(),expires,latest.as_i64(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(c.resource_fence.as_u64())?,c.daemon_boot_identity],
    )?;
    if changed != 1 {
        return Ok((Renewal::ControllerChanged, None));
    }
    Ok((Renewal::Renewed, Some(deadline)))
}

fn renewal_authority(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalInputContext,
) -> Result<(), StoreError> {
    let live: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN processes p ON p.process_id=s.process_id WHERE s.terminal_id=?1 AND s.process_id=?2 AND s.state IN ('ready','busy','detached') AND p.process_run_uuid=?3 AND p.state='live')",
        params![c.terminal_id.to_string(),c.process_id.to_string(),c.process_run_uuid.to_string()], |row|row.get(0),
    )?;
    if !live {
        return Err(invalid("terminal renewal requires a current live session"));
    }
    let allocation =
        read_conn(tx, c.terminal_id)?.ok_or_else(|| invalid("terminal native source missing"))?;
    let runtime = control_runtime(tx, now, &allocation)?;
    if principal(tx, &c.actor_id)? != c.principal_id
        || canonical_sha256(&runtime)? != c.runtime_settings_digest
        || policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest)
    {
        return Err(invalid("terminal renewal authority changed"));
    }
    let lease = control_lease(tx, c.lease_id)?;
    if lease.resource_kind != "terminal"
        || lease.resource_key != key(&allocation.context.preparation)?
    {
        return Err(invalid("terminal renewal resource changed"));
    }
    let mut scoped = allocation.context.preparation.clone();
    scoped.scope = TerminalAllocationScope::ProjectConfiguration {
        project_id: c.project_id,
        revision: c.scope_revision,
    };
    scoped.mission_revision = c.mission_revision;
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, &c.actor_id, &c.principal_id)?;
    control_holds(tx, daemon, &allocation)?;
    control_identity(tx, daemon, &allocation)?;
    Ok(())
}

fn renewal_name(renewal: TerminalInputRenewal) -> &'static str {
    match renewal {
        TerminalInputRenewal::Renewed => "renewed",
        TerminalInputRenewal::LeaseExpired => "lease_expired",
        TerminalInputRenewal::ControllerChanged => "controller_changed",
        TerminalInputRenewal::AuthorityChanged => "authority_changed",
        TerminalInputRenewal::ClockUncertain => "clock_uncertain",
        TerminalInputRenewal::AuthorityBlocked => "authority_blocked",
    }
}
