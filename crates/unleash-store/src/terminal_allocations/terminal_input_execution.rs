//! Consume an input dispatch exactly once before private host I/O. `SQLite` never
//! owns the raw bytes, a host capability or a PTY writer.

use super::terminal_input_types::{
    ResolveTerminalInputRequest, ResolvedTerminalInput, TerminalInputContext, TerminalInputState,
};
use super::terminal_input_writer::{read_input_conn, validate_live_context};
use super::{control_identity, integer, invalid, read_conn};
use crate::commands::read_daemon_state_conn;
use crate::{
    DaemonStateRow, StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};
use rusqlite::{Transaction, params};
use serde_json::json;
use unleash_domain::{DispatchId, TimestampMs};
use unleash_protocol::canonical_sha256;

impl UnleashStore {
    /// Consume the exact durable dispatch once, without any native I/O.
    ///
    /// # Errors
    /// Refuses an absent/wrong/consumed marker, altered context, controller or
    /// authority. Any error, including a lost commit ACK, grants no write permit.
    /// The caller must inspect/journal the outcome; it must never redispatch.
    pub fn resolve_terminal_input(
        &self,
        request: ResolveTerminalInputRequest,
    ) -> Result<ResolvedTerminalInput, StoreError> {
        let config = self.config().clone();
        let result = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            resolve_tx(tx, &config, now, &request)
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
        Ok(result)
    }
}

fn resolve_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &ResolveTerminalInputRequest,
) -> Result<ResolvedTerminalInput, StoreError> {
    let daemon = read_daemon_state_conn(tx)?;
    if !request.caller.role.is_human_decision_maker()
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "terminal input resolution requires authenticated control and reliable time",
        ));
    }
    let record = read_input_conn(tx, request.action_id)?
        .ok_or_else(|| invalid("terminal input preparation missing"))?;
    let c = &record.context;
    if request.authority_generation != c.authority_generation
        || request.caller.actor_id != c.actor_id
        || request.context_digest != record.context_digest
        || request.expected_revision != record.revision
        || record.state != TerminalInputState::Prepared
        || record.dispatch_id.is_some()
    {
        return Err(invalid("terminal input permit consumed or request differs"));
    }
    validate_live_context(tx, &daemon, now, &record)?;
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &daemon,
        now,
        c.action_id,
        request.dispatch_id,
    )?;
    let allocation =
        read_conn(tx, c.terminal_id)?.ok_or_else(|| invalid("terminal input launch missing"))?;
    let expected_os_identity = control_identity(tx, &daemon, &allocation)?;
    let changed = tx.execute(
        "UPDATE terminal_input_intents SET state='write_resolved',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state='prepared' AND revision=?3 AND dispatch_id IS NULL",
        params![c.action_id.to_string(),request.dispatch_id.to_string(),integer(record.revision.as_u64())?],
    )?;
    if changed != 1 {
        return Err(invalid("terminal input resolution CAS lost"));
    }
    append_resolution_event(tx, config, &daemon, now, c, request.dispatch_id)?;
    config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
    let input =
        read_input_conn(tx, c.action_id)?.ok_or_else(|| invalid("terminal input disappeared"))?;
    Ok(ResolvedTerminalInput {
        input,
        expected_os_identity,
    })
}

fn append_resolution_event(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalInputContext,
    dispatch: DispatchId,
) -> Result<(), StoreError> {
    let event = config.id_generator.new_uuid().to_string();
    let payload = json!({"terminal_id":c.terminal_id,"process_run_uuid":c.process_run_uuid,"action_id":c.action_id,"dispatch_id":dispatch,"input_seq":c.input_seq,"payload_digest":c.payload_digest,"payload_byte_length":c.payload_byte_length,"state":"write_resolved","native_receipt":null});
    tx.execute(
        "INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'terminal_input_intent',?3,2,0,'TerminalInputWritePrepared',1,?4,?5,?5,?6,?6,?7,'full',?8,'restricted','command',?9,?9,?10)",
        params![event,daemon.stream_id.to_string(),c.action_id.to_string(),c.project_id.to_string(),c.actor_id.as_str(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.to_string(),c.receipt_id.to_string(),c.terminal_id.to_string()],
    )?;
    tx.execute(
        "INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",
        params![event,now.as_i64(),daemon.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")],
    )?;
    Ok(())
}
