//! One-shot native renewal observation and deadline activation under the writer.
use super::terminal_renewal::{read_renewal_conn, validate_live};
use super::terminal_renewal_types::{
    ResolveTerminalRenewalRequest, TerminalRenewalObservation, TerminalRenewalRecord,
    TerminalRenewalState,
};
use super::{TerminalObservationProof, integer, invalid};
use crate::commands::read_daemon_state_conn;
use crate::{
    BrokerPhase, BrokerPhaseCommit, BrokerPhaseRequest, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};
use rusqlite::{Connection, Transaction, params};
use unleash_domain::{ActionState, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::canonical_sha256;

impl UnleashStore {
    /// Consume one dispatch after identity preflight, without contacting a host.
    /// # Errors
    /// Any error, including an ambiguous commit ACK, grants no handshake permit.
    pub fn resolve_terminal_renewal(
        &self,
        request: ResolveTerminalRenewalRequest,
    ) -> Result<TerminalRenewalRecord, StoreError> {
        let config = self.config().clone();
        let result=self.execute_in_writer_tx(WriteCriticality::Normal,move |tx,now| {
            let d=read_daemon_state_conn(tx)?;
            let record=read_renewal_conn(tx,request.action_id)?.ok_or_else(||invalid("terminal renewal preparation missing"))?;
            if !request.caller.role.is_human_decision_maker() || request.caller.actor_id!=record.context.actor_id
                || request.context_digest!=record.context_digest || request.expected_revision!=record.revision
                || record.state!=TerminalRenewalState::Prepared || record.dispatch_id.is_some()
                || config.clock_uncertain.load(std::sync::atomic::Ordering::Acquire) {
                return Err(invalid("terminal renewal permit consumed or authority differs"));
            }
            validate_live(tx,&d,now,&record)?;
            crate::broker_writer::validate_execution_dispatch_tx(tx,&d,now,record.context.action_id,request.dispatch_id)?;
            if tx.execute("UPDATE terminal_renewal_intents SET state='handshake_resolved',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state='prepared' AND revision=?3 AND dispatch_id IS NULL",
                params![record.context.action_id.to_string(),request.dispatch_id.to_string(),integer(record.revision.as_u64())?])?!=1 {
                return Err(invalid("terminal renewal permit CAS lost"));
            }
            append_resolution(tx,&config,&d,now,&record,request.dispatch_id)?;
            config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
            read_renewal_conn(tx,record.context.action_id)?.ok_or_else(||invalid("terminal renewal disappeared"))
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(result)
    }

    /// Commit a native renewal proof and the existing lease deadline atomically.
    /// # Errors
    /// Refuses stale authority, expired leases, unqualified proof and unknown
    /// ledger state. Replay is exact and never invokes or permits another effect.
    pub fn commit_terminal_renewal_observation(
        &self,
        observation: TerminalRenewalObservation,
        phase: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            commit_tx(tx, &config, now, &observation, &phase)
        })?;
        if !committed.replayed {
            self.config()
                .cutpoints
                .hit(StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(committed)
    }
}

fn append_resolution(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &crate::DaemonStateRow,
    now: TimestampMs,
    record: &TerminalRenewalRecord,
    dispatch: DispatchId,
) -> Result<(), StoreError> {
    let c = &record.context;
    let payload = serde_json::json!({"terminal_id":c.terminal_id,"process_run_uuid":c.process_run_uuid,
        "action_id":c.action_id,"dispatch_id":dispatch,"state":"handshake_resolved","control_revision":c.control_revision});
    append_intent_event(
        tx,
        config,
        d,
        now,
        record,
        "TerminalRenewalHandshakePrepared",
        &payload,
    )?;
    Ok(())
}

pub(super) fn append_intent_event(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &crate::DaemonStateRow,
    now: TimestampMs,
    record: &TerminalRenewalRecord,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<(String, unleash_protocol::EventCursor), StoreError> {
    let c = &record.context;
    let event = config.id_generator.new_uuid().to_string();
    let revision = integer(
        record
            .revision
            .as_u64()
            .checked_add(1)
            .ok_or_else(|| invalid("terminal intention revision overflow"))?,
    )?;
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'terminal_control_intent',?3,?11,0,?12,1,?4,?5,?5,?6,?6,?7,'full',?8,'restricted','command',?9,?9,?10)",
        params![event,d.stream_id.to_string(),c.action_id.to_string(),c.project_id.to_string(),c.actor_id.as_str(),now.as_i64(),serde_json::to_string(payload)?,canonical_sha256(payload)?.to_string(),c.receipt_id.to_string(),c.terminal_id.to_string(),revision,event_type])?;
    let sequence = u64::try_from(tx.last_insert_rowid())
        .map_err(|_| invalid("terminal event sequence invalid"))?;
    tx.execute("INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",
        params![event,now.as_i64(),d.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")])?;
    Ok((
        event,
        unleash_protocol::EventCursor::new(d.stream_id, unleash_domain::Sequence::new(sequence)),
    ))
}

fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    observation: &TerminalRenewalObservation,
    phase: &BrokerPhaseRequest,
) -> Result<BrokerPhaseCommit, StoreError> {
    let record = read_renewal_conn(tx, observation.action_id)?
        .ok_or_else(|| invalid("renewal observation intention missing"))?;
    let digest = canonical_sha256(observation)?;
    validate_native(&record, observation)?;
    let BrokerPhase::Observation {
        action_id, receipt, ..
    } = &phase.phase
    else {
        return Err(invalid("renewal requires observation phase"));
    };
    if *action_id != record.context.action_id
        || receipt.action_id != *action_id
        || receipt.dispatch_id != observation.dispatch_id
        || receipt.output_digest != Some(digest)
        || receipt.state != ActionState::ObservedSuccess
        || record.dispatch_id != Some(observation.dispatch_id)
    {
        return Err(invalid("renewal broker receipt differs"));
    }
    if record.state == TerminalRenewalState::Observed {
        if record.observation_object_hash != Some(digest) {
            return Err(invalid("renewal proof replay differs"));
        }
        return crate::broker_writer::commit_phase_tx(tx, config, now, phase);
    }
    if record.state != TerminalRenewalState::HandshakeResolved {
        return Err(invalid("renewal requires one consumed pending permit"));
    }
    if read_published(tx, config, &record, observation.dispatch_id, digest)? != *observation {
        return Err(invalid("renewal published proof differs"));
    }
    let started: i64 = tx.query_row(
        "SELECT started_at_ms FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2",
        params![
            record.context.action_id.to_string(),
            observation.dispatch_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if observation.observed_at_ms.as_i64() < started
        || observation.observed_at_ms > receipt.observed_at_ms
    {
        return Err(invalid("renewal capture is outside dispatch and receipt"));
    }
    // Test-only Rust injection. No deadline or session update precedes this
    // barrier; all dynamic authority/time guards are repeated after release.
    config
        .cutpoints
        .hit(StoreCutpoint::C1BeforeTerminalRenewalObservationCommit)?;
    let final_now = config.clock.now_ms();
    if final_now < now {
        return Err(invalid("renewal clock moved backwards during observation"));
    }
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid("renewal clock became uncertain"));
    }
    let d = read_daemon_state_conn(tx)?;
    validate_live(tx, &d, final_now, &record)?;
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &d,
        final_now,
        record.context.action_id,
        observation.dispatch_id,
    )?;
    let c = &record.context;
    let input: i64 = tx.query_row(
        "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
        [c.terminal_id.to_string()],
        |row| row.get(0),
    )?;
    if u64::try_from(input).ok() != Some(observation.last_confirmed_input_seq) {
        return Err(invalid("renewal native input state differs"));
    }
    commit_observed_size(tx, &record, observation)?;
    if tx.execute("UPDATE resource_leases SET expires_at_ms=?2 WHERE lease_id=?1 AND state='held' AND authority_generation=?3 AND daemon_epoch=?4 AND fence=?5 AND boot_identity=?6 AND owner_type='principal' AND owner_id=?7 AND expires_at_ms=?8",
        params![c.lease_id.to_string(),c.proposed_expires_at_ms.as_i64(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(c.resource_fence.as_u64())?,c.daemon_boot_identity,c.principal_id.as_str(),c.previous_expires_at_ms.as_i64()])?!=1 {return Err(invalid("terminal renewal deadline CAS lost"));}
    if tx.execute("UPDATE terminal_renewal_intents SET state='observed',revision=revision+1,observation_object_hash=?2 WHERE action_id=?1 AND state='handshake_resolved' AND revision=?3 AND dispatch_id=?4",
        params![c.action_id.to_string(),digest.to_string(),integer(record.revision.as_u64())?,observation.dispatch_id.to_string()])?!=1 {return Err(invalid("terminal renewal observation CAS lost"));}
    tx.execute(
        "UPDATE processes SET last_observed_at_ms=?2 WHERE process_id=?1 AND process_run_uuid=?3",
        params![
            c.process_id.to_string(),
            observation.observed_at_ms.as_i64(),
            c.process_run_uuid.to_string()
        ],
    )?;
    let proof = TerminalObservationProof {
        phase_digest: canonical_sha256(&phase.phase)?,
        resize_result: resize_result(&record, observation),
    };
    crate::broker_writer::commit_terminal_observation_tx(tx, config, final_now, phase, &proof)
}

fn commit_observed_size(
    tx: &Transaction<'_>,
    record: &TerminalRenewalRecord,
    observation: &TerminalRenewalObservation,
) -> Result<(), StoreError> {
    let Some(ack) = &observation.resize_ack else {
        return Ok(());
    };
    let c = &record.context;
    let changed = tx.execute(
        "UPDATE terminal_sessions SET rows=?2,cols=?3 WHERE terminal_id=?1
          AND process_id=?4 AND control_revision=?5 AND lease_id=?6",
        params![
            c.terminal_id.to_string(),
            ack.dimensions.rows,
            ack.dimensions.cols,
            c.process_id.to_string(),
            integer(c.control_revision.as_u64())?,
            c.lease_id.to_string()
        ],
    )?;
    if changed != 1 {
        return Err(invalid("terminal resize session CAS lost"));
    }
    Ok(())
}

fn resize_result(
    record: &TerminalRenewalRecord,
    observation: &TerminalRenewalObservation,
) -> Option<super::terminal_renewal_types::TerminalResizeResult> {
    let c = &record.context;
    observation
        .resize_ack
        .as_ref()
        .map(|ack| super::terminal_renewal_types::TerminalResizeResult {
            command_id: c.command_id,
            receipt_id: c.receipt_id,
            terminal_id: c.terminal_id,
            process_run_uuid: c.process_run_uuid,
            dimensions: ack.dimensions,
            lease_expires_at: c.proposed_expires_at_ms,
        })
}

pub(super) fn validate_native(
    record: &TerminalRenewalRecord,
    observation: &TerminalRenewalObservation,
) -> Result<(), StoreError> {
    let c = &record.context;
    if observation.action_id != c.action_id
        || observation.context_digest != record.context_digest
        || observation.terminal_id != c.terminal_id
        || observation.host_id != c.host_id
        || observation.process_run_uuid != c.process_run_uuid
        || observation.os_identity != c.expected_os_identity
        || observation.daemon_epoch != c.daemon_epoch
        || observation.resource_fence != c.resource_fence
        || !observation.unknown_input_seqs.is_empty()
        || observation.last_confirmed_input_seq > 9_007_199_254_740_991
    {
        return Err(invalid("renewal native observation differs"));
    }
    match (
        c.resize,
        &observation.resize_ack,
        observation.handshake_response_digest,
    ) {
        (None, None, Some(_)) => {}
        (Some(size), Some(ack), None) if size == ack.dimensions => {
            let response =
                serde_json::json!({"kind":"resize_ack","rows":size.rows,"cols":size.cols});
            if canonical_sha256(&response)? != ack.response_digest {
                return Err(invalid("terminal resize acknowledgement digest differs"));
            }
        }
        _ => {
            return Err(invalid(
                "terminal acknowledgement does not match the prepared operation",
            ));
        }
    }
    Ok(())
}

pub(super) fn read_published(
    conn: &Connection,
    config: &StoreConfig,
    record: &TerminalRenewalRecord,
    dispatch: DispatchId,
    hash: Sha256Digest,
) -> Result<TerminalRenewalObservation, StoreError> {
    let published:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='terminal_renewal_observation' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live')",
        params![record.context.action_id.to_string(),dispatch.to_string(),hash.to_string()],|row|row.get(0))?;
    if !published {
        return Err(invalid("renewal observation object is not published"));
    }
    let bytes =
        crate::objects::read_verified_evidence_bytes_bounded(conn, config, hash, 2 * 1024 * 1024)?;
    let observation: TerminalRenewalObservation = serde_json::from_slice(&bytes)?;
    if canonical_sha256(&observation)? != hash || observation.dispatch_id != dispatch {
        return Err(invalid("renewal native proof integrity differs"));
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
    let Some(record) = read_renewal_conn(tx, *action_id)? else {
        return Ok(());
    };
    if record.state == TerminalRenewalState::Refused {
        return Err(invalid("terminal refusal forbids observation"));
    }
    if receipt.state == ActionState::OutcomeUnknown {
        if record.broker_dispatch_id != Some(receipt.dispatch_id) {
            return Err(invalid("unknown renewal dispatch differs"));
        }
        if matches!(
            record.state,
            TerminalRenewalState::Prepared | TerminalRenewalState::HandshakeResolved
        ) {
            tx.execute("UPDATE terminal_renewal_intents SET state='outcome_unknown',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state IN ('prepared','handshake_resolved')",
                params![action_id.to_string(),receipt.dispatch_id.to_string()])?;
        }
    } else if !proof
        .is_some_and(|p| canonical_sha256(phase).is_ok_and(|digest| p.phase_digest == digest))
    {
        return Err(invalid(
            "terminal renewal observation producer not qualified",
        ));
    }
    Ok(())
}
