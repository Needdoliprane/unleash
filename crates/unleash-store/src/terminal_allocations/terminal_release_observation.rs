//! A real native retirement qualifies disposition, never physical resource release.
use super::terminal_release::{read_conn_release, validate_live};
use super::terminal_release_types::{
    ResolveTerminalReleaseRequest, TerminalReleaseDisposition, TerminalReleaseObservation,
    TerminalReleaseRecord, TerminalReleaseResult, TerminalReleaseState,
};
use super::{TerminalObservationProof, integer, invalid};
use crate::commands::read_daemon_state_conn;
use crate::{
    BrokerPhase, BrokerPhaseCommit, BrokerPhaseRequest, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};
use rusqlite::{Connection, Transaction, params};
use unleash_domain::{ActionState, Sha256Digest, TimestampMs};
use unleash_protocol::canonical_sha256;

impl UnleashStore {
    /// Consume the exact committed retirement dispatch after native identity preflight.
    /// # Errors
    /// A failed or ambiguous commit never grants a native effect permit.
    pub fn resolve_terminal_release(
        &self,
        request: ResolveTerminalReleaseRequest,
    ) -> Result<TerminalReleaseRecord, StoreError> {
        let config = self.config().clone();
        let result=self.execute_in_writer_tx(WriteCriticality::Normal,move |tx,now| {
            let d=read_daemon_state_conn(tx)?;
            let record=read_conn_release(tx,request.action_id)?.ok_or_else(||invalid("terminal release preparation missing"))?;
            if !request.caller.role.is_human_decision_maker() || request.caller.actor_id!=record.context.actor_id || request.context_digest!=record.context_digest || request.expected_revision!=record.revision || record.state!=TerminalReleaseState::Prepared || record.dispatch_id.is_some() {return Err(invalid("terminal release permit consumed or authority differs"));}
            validate_live(tx,&config,&d,now,&record)?;
            crate::broker_writer::validate_execution_dispatch_tx(tx,&d,now,record.context.action_id,request.dispatch_id)?;
            let c=&record.context;
            if tx.execute("UPDATE resource_leases SET state='reconciling' WHERE lease_id=?1 AND state='held' AND authority_generation=?2 AND daemon_epoch=?3 AND fence=?4 AND expires_at_ms=?5",params![c.lease_id.to_string(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(c.resource_fence.as_u64())?,c.source_expires_at_ms.as_i64()])?!=1 {return Err(invalid("terminal release lease CAS lost"));}
            if tx.execute("UPDATE terminal_release_intents SET state='retire_resolved',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state='prepared' AND revision=?3 AND dispatch_id IS NULL",params![c.action_id.to_string(),request.dispatch_id.to_string(),integer(record.revision.as_u64())?])?!=1 {return Err(invalid("terminal release permit CAS lost"));}
            config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
            read_conn_release(tx,c.action_id)?.ok_or_else(||invalid("terminal release disappeared"))
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(result)
    }

    /// Atomically commit native retirement, the retired lease and causal broker event.
    /// # Errors
    /// Requires the exact published ACK and live authority; exact replay creates no effect.
    pub fn commit_terminal_release_observation(
        &self,
        observation: TerminalReleaseObservation,
        phase: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        let result = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            commit_tx(tx, &config, now, &observation, &phase)
        })?;
        if !result.replayed {
            self.config()
                .cutpoints
                .hit(StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(result)
    }
}

fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    observation: &TerminalReleaseObservation,
    phase: &BrokerPhaseRequest,
) -> Result<BrokerPhaseCommit, StoreError> {
    let action = observation.ack.key.header.action_id;
    let record = read_conn_release(tx, action)?
        .ok_or_else(|| invalid("terminal release intention missing"))?;
    let digest = canonical_sha256(observation)?;
    validate_native(&record, observation)?;
    let BrokerPhase::Observation {
        action_id, receipt, ..
    } = &phase.phase
    else {
        return Err(invalid("terminal release requires observation phase"));
    };
    if *action_id != action
        || receipt.action_id != action
        || receipt.dispatch_id != observation.ack.key.dispatch_id
        || receipt.output_digest != Some(digest)
        || receipt.state != ActionState::ObservedSuccess
        || record.dispatch_id != Some(receipt.dispatch_id)
    {
        return Err(invalid("terminal release broker receipt differs"));
    }
    // Exact replay must not require a still-held lease or perform another disposition.
    if record.state == TerminalReleaseState::Observed {
        if record.observation_object_hash != Some(digest) {
            return Err(invalid("terminal release proof replay differs"));
        }
        return crate::broker_writer::commit_phase_tx(tx, config, now, phase);
    }
    if record.state != TerminalReleaseState::RetireResolved {
        return Err(invalid(
            "terminal release requires one consumed pending permit",
        ));
    }
    if read_published(tx, config, &record, digest)? != *observation {
        return Err(invalid("terminal release published proof differs"));
    }
    let started: i64 = tx.query_row(
        "SELECT started_at_ms FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2",
        params![action.to_string(), receipt.dispatch_id.to_string()],
        |row| row.get(0),
    )?;
    if observation.captured_at_ms.as_i64() < started
        || observation.captured_at_ms > receipt.observed_at_ms
    {
        return Err(invalid(
            "terminal release capture outside dispatch and receipt",
        ));
    }
    let final_now = config.clock.now_ms();
    if final_now < now {
        return Err(invalid(
            "terminal release clock moved backwards during observation",
        ));
    }
    let d = read_daemon_state_conn(tx)?;
    validate_live(tx, config, &d, final_now, &record)?;
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &d,
        final_now,
        action,
        receipt.dispatch_id,
    )?;
    let c = &record.context;
    let input: i64 = tx.query_row(
        "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
        [c.terminal_id.to_string()],
        |row| row.get(0),
    )?;
    if u64::try_from(input).ok() != Some(observation.ack.last_confirmed_input_seq) {
        return Err(invalid("terminal release native input differs"));
    }
    if tx.execute("UPDATE resource_leases SET state='released' WHERE lease_id=?1 AND state='reconciling' AND authority_generation=?2 AND daemon_epoch=?3 AND fence=?4 AND expires_at_ms=?5",params![c.lease_id.to_string(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(c.resource_fence.as_u64())?,c.source_expires_at_ms.as_i64()])?!=1 {return Err(invalid("terminal release lease disposition CAS lost"));}
    if tx.execute("UPDATE terminal_sessions SET state='detached',control_revision=control_revision+1 WHERE terminal_id=?1 AND process_id=?2 AND lease_id=?3 AND control_revision=?4",params![c.terminal_id.to_string(),c.process_id.to_string(),c.lease_id.to_string(),integer(c.control_revision.as_u64())?])?!=1 {return Err(invalid("terminal release control revision CAS lost"));}
    if tx.execute("UPDATE terminal_release_intents SET state='observed',revision=revision+1,observation_object_hash=?2 WHERE action_id=?1 AND state='retire_resolved' AND revision=?3 AND dispatch_id=?4",params![action.to_string(),digest.to_string(),integer(record.revision.as_u64())?,receipt.dispatch_id.to_string()])?!=1 {return Err(invalid("terminal release observation CAS lost"));}
    let proof = TerminalObservationProof {
        phase_digest: canonical_sha256(&phase.phase)?,
        resize_result: None,
    };
    crate::broker_writer::commit_terminal_observation_tx(tx, config, final_now, phase, &proof)
}

fn validate_native(
    record: &TerminalReleaseRecord,
    observation: &TerminalReleaseObservation,
) -> Result<(), StoreError> {
    let c = &record.context;
    let ack = &observation.ack;
    let h = &ack.key.header;
    if observation.schema_version != 1
        || h.protocol_version != 1
        || h.action_id != c.action_id
        || h.command_id != c.command_id
        || h.payload_hash != record.context_digest
        || h.authority_generation != c.authority_generation
        || h.daemon_epoch != c.daemon_epoch
        || h.resource_fence != c.resource_fence
        || h.process_run_uuid != c.process_run_uuid
        || ack.key.initial_context_digest != c.initial_context_digest
        || ack.key.lease_id != c.lease_id
        || ack.key.control_revision != c.control_revision
        || ack.host_id != c.host_id
        || ack.process_id != c.process_id
        || ack.terminal_id != c.terminal_id
        || ack.os_identity != c.expected_os_identity
        || !ack.unknown_input_seqs.is_empty()
        || ack.last_confirmed_input_seq > 9_007_199_254_740_991
    {
        return Err(invalid("terminal release native ACK differs"));
    }
    Ok(())
}

pub(super) fn read_published(
    conn: &Connection,
    config: &StoreConfig,
    record: &TerminalReleaseRecord,
    hash: Sha256Digest,
) -> Result<TerminalReleaseObservation, StoreError> {
    let dispatch = record
        .dispatch_id
        .ok_or_else(|| invalid("terminal release dispatch missing"))?;
    let published:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='terminal_release_observation' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live')",params![record.context.action_id.to_string(),dispatch.to_string(),hash.to_string()],|row|row.get(0))?;
    if !published {
        return Err(StoreError::EvidenceUnavailable(
            "terminal release proof is not published".into(),
        ));
    }
    let bytes =
        crate::objects::read_verified_evidence_bytes_bounded(conn, config, hash, 2 * 1024 * 1024)?;
    let observation: TerminalReleaseObservation = serde_json::from_slice(&bytes)?;
    if canonical_sha256(&observation)? != hash || observation.ack.key.dispatch_id != dispatch {
        return Err(invalid("terminal release proof integrity differs"));
    }
    validate_native(record, &observation)?;
    Ok(observation)
}

pub(super) fn guard_observation(
    tx: &Transaction<'_>,
    phase: &BrokerPhase,
    proof: Option<&TerminalObservationProof>,
) -> Result<(), StoreError> {
    let BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase
    else {
        return Ok(());
    };
    let Some(record) = read_conn_release(tx, *action_id)? else {
        return Ok(());
    };
    if receipt.state == ActionState::OutcomeUnknown {
        if record.broker_dispatch_id != Some(receipt.dispatch_id) {
            return Err(invalid("terminal release unknown dispatch differs"));
        }
        if matches!(
            record.state,
            TerminalReleaseState::Prepared | TerminalReleaseState::RetireResolved
        ) {
            tx.execute("UPDATE terminal_release_intents SET state='outcome_unknown',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state IN ('prepared','retire_resolved')",params![action_id.to_string(),receipt.dispatch_id.to_string()])?;
        }
    } else if !proof
        .is_some_and(|p| canonical_sha256(phase).is_ok_and(|digest| p.phase_digest == digest))
    {
        return Err(invalid(
            "terminal release observation producer not qualified",
        ));
    }
    Ok(())
}

pub(crate) fn observed_result(
    conn: &Connection,
    action: unleash_domain::ActionId,
) -> Result<Option<TerminalReleaseResult>, StoreError> {
    let Some(record) = read_conn_release(conn, action)? else {
        return Ok(None);
    };
    if record.state != TerminalReleaseState::Observed {
        return Ok(None);
    }
    let c = record.context;
    Ok(Some(TerminalReleaseResult {
        command_id: c.command_id,
        receipt_id: c.receipt_id,
        terminal_id: c.terminal_id,
        process_run_uuid: c.process_run_uuid,
        lease_id: c.lease_id,
        fence: c.resource_fence,
        control_revision: unleash_domain::Revision::new(
            c.control_revision
                .as_u64()
                .checked_add(1)
                .ok_or_else(|| invalid("terminal release revision exhausted"))?,
        ),
        disposition: TerminalReleaseDisposition::Released,
    }))
}
