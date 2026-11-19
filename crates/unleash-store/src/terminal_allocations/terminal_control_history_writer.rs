//! Append-only historical evidence, separate from the execution state machine.
use std::sync::atomic::Ordering;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, DaemonEpoch, DispatchId, Sequence, Sha256Digest, TimestampMs,
};
use unleash_protocol::{EventCursor, canonical_sha256, canonicalize_to_vec};

use super::validation::{self, Source, invalid, unavailable};
use super::{
    SealTerminalControlHistoryRequest, TERMINAL_CONTROL_HISTORY_EVENT,
    TerminalControlHistoryCommit, TerminalControlHistoryFact, TerminalControlHistoryProof,
};
use crate::commands::{DaemonStateRow, read_daemon_state_conn};
use crate::objects::{
    read_object_record_conn, read_prepared_evidence_bytes_bounded,
    read_verified_evidence_bytes_bounded, tx_publish_owned_reference, tx_release_pin,
};
use crate::{
    PublishObjectReferenceRequest, SnapshotProjection, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};

const MAX_PROOF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryPayload {
    schema_version: u32,
    dispatch_id: DispatchId,
    context_digest: Sha256Digest,
    proof_hash: Sha256Digest,
    object_generation: i64,
    producer_pin_id: String,
    native_ack_digest: Sha256Digest,
    inspected_by_epoch: DaemonEpoch,
    inspected_by: ActorId,
    observed_at_ms: TimestampMs,
}

struct Sealed {
    fact: TerminalControlHistoryFact,
    proof: TerminalControlHistoryProof,
    producer_pin_id: String,
}

struct WriteContext<'a> {
    config: &'a StoreConfig,
    daemon: &'a DaemonStateRow,
    now: TimestampMs,
    source: &'a Source,
}

pub(super) fn seal(
    store: &UnleashStore,
    request: SealTerminalControlHistoryRequest,
) -> Result<TerminalControlHistoryCommit, StoreError> {
    let config = store.config().clone();
    let committed = store.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
        crate::policies_secrets::require_reconciler(&request.caller, "ReconcileProcess")?;
        let source = validation::source(
            tx,
            &config,
            &request.caller.actor_id,
            request.source_action_id,
            request.dispatch_id,
        )?;
        if request.context_digest != source.record.context_digest {
            return Err(invalid("historical context request differs"));
        }
        let daemon = read_daemon_state_conn(tx)?;
        let publication = publication(&request);
        let context = WriteContext {
            config: &config,
            daemon: &daemon,
            now,
            source: &source,
        };
        if let Some(sealed) = read_sealed(tx, &config, &daemon, &source)? {
            return replay(tx, &context, &request, &publication, sealed);
        }
        if source.record.state == super::super::TerminalControlState::Disposed {
            return Err(unavailable(
                "disposed transfer has no retained historical fact",
            ));
        }
        let bytes = read_prepared_evidence_bytes_bounded(
            tx,
            &config,
            &request.prepared_proof,
            &publication,
            daemon.authority_generation,
            MAX_PROOF_BYTES,
        )?;
        let proof = decode_proof(&bytes)?;
        validation::proof(&source, &proof, now)?;
        reliable_clock(&config, now, proof.inspection.observed_at_ms)?;
        tx_publish_owned_reference(
            tx,
            config.id_generator.as_ref(),
            now,
            &publication,
            daemon.authority_generation,
        )?;
        let fact = append_fact(tx, &context, &proof, &publication)?;
        config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
        reliable_clock(&config, now, proof.inspection.observed_at_ms)?;
        // The writer lock held by this transaction prevents a concurrent source,
        // lineage or actor change. Re-read those bindings after proof I/O/barrier.
        validation::source(
            tx,
            &config,
            &request.caller.actor_id,
            request.source_action_id,
            request.dispatch_id,
        )?;
        Ok(TerminalControlHistoryCommit {
            fact,
            replayed: false,
        })
    })?;
    if !committed.replayed {
        store
            .config()
            .cutpoints
            .hit(StoreCutpoint::C2AfterCommandCommit)?;
    }
    Ok(committed)
}

fn publication(request: &SealTerminalControlHistoryRequest) -> PublishObjectReferenceRequest {
    PublishObjectReferenceRequest {
        pin_id: request.prepared_proof.pin_id.clone(),
        object_hash: request.prepared_proof.object_hash,
        object_generation: request.prepared_proof.object_generation,
        owner_kind: "action".into(),
        owner_id: request.source_action_id.to_string(),
        source_field: source_field(request.dispatch_id),
    }
}
fn source_field(dispatch: DispatchId) -> String {
    format!("terminal_control_history:{dispatch}")
}

fn decode_proof(bytes: &[u8]) -> Result<TerminalControlHistoryProof, StoreError> {
    let proof: TerminalControlHistoryProof = serde_json::from_slice(bytes)
        .map_err(|_| invalid("historical native proof is not its closed schema"))?;
    if canonicalize_to_vec(&proof)? != bytes {
        return Err(invalid("historical native proof is not canonical"));
    }
    Ok(proof)
}

fn reliable_clock(
    config: &StoreConfig,
    entered: TimestampMs,
    captured: TimestampMs,
) -> Result<(), StoreError> {
    let final_now = config.clock.now_ms();
    if config.clock_uncertain.load(Ordering::Acquire) || final_now < entered || captured > final_now
    {
        return Err(unavailable("historical inspection clock is unreliable"));
    }
    Ok(())
}

fn replay(
    tx: &Transaction<'_>,
    context: &WriteContext<'_>,
    request: &SealTerminalControlHistoryRequest,
    publication: &PublishObjectReferenceRequest,
    sealed: Sealed,
) -> Result<TerminalControlHistoryCommit, StoreError> {
    let config = context.config;
    let daemon = context.daemon;
    let now = context.now;
    let source = context.source;
    if request.prepared_proof.object_hash == sealed.fact.observation_object_hash {
        same_object_pin(tx, context, request, publication, &sealed.producer_pin_id)?;
    } else {
        let bytes = read_prepared_evidence_bytes_bounded(
            tx,
            config,
            &request.prepared_proof,
            publication,
            daemon.authority_generation,
            MAX_PROOF_BYTES,
        )?;
        let proof = decode_proof(&bytes)?;
        validation::proof(source, &proof, now)?;
        if proof.ack != sealed.proof.ack
            || proof.source_receipt_id != sealed.proof.source_receipt_id
            || proof.source_evaluation_id != sealed.proof.source_evaluation_id
            || proof.source_intent_digest != sealed.proof.source_intent_digest
        {
            return Err(invalid("historical native fact replay differs"));
        }
        reliable_clock(config, now, proof.inspection.observed_at_ms)?;
        // A later inspection does not publish another reference/event. Its
        // disposable preparation is released without changing the sealed fact.
        tx_release_pin(tx, now, &publication.pin_id)?;
    }
    Ok(TerminalControlHistoryCommit {
        fact: sealed.fact,
        replayed: true,
    })
}

fn same_object_pin(
    tx: &Transaction<'_>,
    context: &WriteContext<'_>,
    request: &SealTerminalControlHistoryRequest,
    publication: &PublishObjectReferenceRequest,
    original_pin: &str,
) -> Result<(), StoreError> {
    let record = read_object_record_conn(tx, request.prepared_proof.object_hash)?;
    if record.object_generation != request.prepared_proof.object_generation
        || record.relative_path != request.prepared_proof.relative_path
    {
        return Err(invalid(
            "historical proof handle differs from the sealed object",
        ));
    }
    if publication.pin_id == original_pin {
        let pin:Option<ReplayPin>=tx.query_row("SELECT object_hash,object_generation,owner_kind,owner_id,authority_generation,pin_kind,state FROM object_pins WHERE pin_id=?1",[original_pin],ReplayPin::read).optional()?;
        if let Some(pin) = pin {
            if pin.hash != publication.object_hash.to_string()
                || pin.generation != publication.object_generation
                || pin.owner_kind != "action"
                || pin.owner_id != request.source_action_id.to_string()
                || pin.authority != context.daemon.authority_generation.to_string()
                || pin.kind != "preparation"
                || pin.state != "released"
            {
                return Err(unavailable(
                    "original historical producer handle is not consumed and exact",
                ));
            }
        }
        // A consumed preparation can be collected. Its original identity is
        // still sealed in the event; the live proof was verified above.
    } else {
        read_prepared_evidence_bytes_bounded(
            tx,
            context.config,
            &request.prepared_proof,
            publication,
            context.daemon.authority_generation,
            MAX_PROOF_BYTES,
        )?;
        tx_release_pin(tx, context.now, &publication.pin_id)?;
    }
    Ok(())
}

struct ReplayPin {
    hash: String,
    generation: i64,
    owner_kind: String,
    owner_id: String,
    authority: String,
    kind: String,
    state: String,
}
impl ReplayPin {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            hash: row.get(0)?,
            generation: row.get(1)?,
            owner_kind: row.get(2)?,
            owner_id: row.get(3)?,
            authority: row.get(4)?,
            kind: row.get(5)?,
            state: row.get(6)?,
        })
    }
}

pub(super) fn inspect(
    store: &UnleashStore,
    actor: &ActorId,
    action: ActionId,
    dispatch: DispatchId,
) -> Result<SnapshotProjection<Option<TerminalControlHistoryFact>>, StoreError> {
    store.read_snapshot(|conn, _| {
        let source = validation::source(conn, store.config(), actor, action, dispatch)?;
        let daemon = read_daemon_state_conn(conn)?;
        let value = read_sealed(conn, store.config(), &daemon, &source)?.map(|sealed| sealed.fact);
        let high: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",
            [daemon.stream_id.to_string()],
            |row| row.get(0),
        )?;
        let sequence =
            u64::try_from(high).map_err(|_| unavailable("historical watermark is invalid"))?;
        Ok(SnapshotProjection {
            value,
            cursor: EventCursor::new(daemon.stream_id, Sequence::new(sequence)),
        })
    })
}

fn read_sealed(
    conn: &Connection,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    source: &Source,
) -> Result<Option<Sealed>, StoreError> {
    let action = source.record.context.action_id.to_string();
    let count:i64=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_type='action' AND aggregate_id=?1 AND event_type=?2",params![action,TERMINAL_CONTROL_HISTORY_EVENT],|row|row.get(0))?;
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        return Err(unavailable("historical terminal fact is not unique"));
    }
    let row:HistoryEvent=conn.query_row("SELECT sequence,stream_id,aggregate_revision,event_ordinal,schema_version,actor_id,payload_state,payload_json,payload_hash,recorded_at_ms FROM events WHERE aggregate_type='action' AND aggregate_id=?1 AND event_type=?2",params![action,TERMINAL_CONTROL_HISTORY_EVENT],HistoryEvent::read)?;
    let payload = row.payload(daemon, source)?;
    let reference:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='action' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live' AND o.object_generation=?4)",params![action,source_field(payload.dispatch_id),payload.proof_hash.to_string(),payload.object_generation],|r|r.get(0))?;
    if !reference {
        return Err(unavailable("historical proof reference is unavailable"));
    }
    let bytes =
        read_verified_evidence_bytes_bounded(conn, config, payload.proof_hash, MAX_PROOF_BYTES)?;
    let proof = decode_proof(&bytes)?;
    validation::proof(source, &proof, TimestampMs::new(row.recorded_at_ms))?;
    if proof.inspection.observed_at_ms != payload.observed_at_ms
        || canonical_sha256(&proof.ack)? != payload.native_ack_digest
    {
        return Err(unavailable(
            "historical proof differs from its sealed observation",
        ));
    }
    let fact = fact(source, &proof, &payload, row.cursor(daemon)?);
    Ok(Some(Sealed {
        fact,
        proof,
        producer_pin_id: payload.producer_pin_id,
    }))
}

struct HistoryEvent {
    sequence: i64,
    stream: String,
    revision: i64,
    ordinal: i64,
    schema: i64,
    actor: String,
    state: String,
    encoded: String,
    digest: String,
    recorded_at_ms: i64,
}
impl HistoryEvent {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            sequence: row.get(0)?,
            stream: row.get(1)?,
            revision: row.get(2)?,
            ordinal: row.get(3)?,
            schema: row.get(4)?,
            actor: row.get(5)?,
            state: row.get(6)?,
            encoded: row.get(7)?,
            digest: row.get(8)?,
            recorded_at_ms: row.get(9)?,
        })
    }
    fn payload(
        &self,
        daemon: &DaemonStateRow,
        source: &Source,
    ) -> Result<HistoryPayload, StoreError> {
        if self.state != "full"
            || self.schema != 1
            || self.stream != daemon.stream_id.to_string()
            || self.revision < 1
            || u64::try_from(self.revision)
                .ok()
                .is_none_or(|r| r > source.broker.revision.as_u64())
            || self.ordinal < 0
            || self.actor != source.record.context.actor_id.as_str()
        {
            return Err(unavailable("historical terminal event is unavailable"));
        }
        let payload: HistoryPayload = serde_json::from_str(&self.encoded)
            .map_err(|_| unavailable("historical event payload is invalid"))?;
        if payload.schema_version != 1
            || payload.producer_pin_id.is_empty()
            || canonical_sha256(&payload)?.to_string() != self.digest
            || payload.dispatch_id
                != source
                    .record
                    .dispatch_id
                    .ok_or_else(|| unavailable("historical dispatch is unavailable"))?
            || payload.context_digest != source.record.context_digest
            || payload.inspected_by != source.record.context.actor_id
            || payload.inspected_by_epoch < source.record.context.daemon_epoch
            || payload.inspected_by_epoch > daemon.daemon_epoch
            || payload.observed_at_ms.as_i64() > self.recorded_at_ms
        {
            return Err(unavailable(
                "historical event does not bind its original source",
            ));
        }
        Ok(payload)
    }
    fn cursor(&self, daemon: &DaemonStateRow) -> Result<EventCursor, StoreError> {
        let sequence = u64::try_from(self.sequence)
            .map_err(|_| unavailable("historical sequence is invalid"))?;
        Ok(EventCursor::new(daemon.stream_id, Sequence::new(sequence)))
    }
}

fn fact(
    source: &Source,
    proof: &TerminalControlHistoryProof,
    payload: &HistoryPayload,
    cursor: EventCursor,
) -> TerminalControlHistoryFact {
    let c = &source.record.context;
    TerminalControlHistoryFact {
        schema_version: 1,
        action_id: c.action_id,
        dispatch_id: proof.ack.dispatch_id,
        source_receipt_id: proof.source_receipt_id,
        source_evaluation_id: proof.source_evaluation_id,
        source_intent_digest: proof.source_intent_digest,
        context_digest: source.record.context_digest,
        native_ack_digest: payload.native_ack_digest,
        observation_object_hash: payload.proof_hash,
        rotation_epoch: c.daemon_epoch,
        rotation_fence: c.next_fence,
        inspected_by_epoch: payload.inspected_by_epoch,
        inspected_by: payload.inspected_by.clone(),
        observed_at_ms: payload.observed_at_ms,
        event_cursor: cursor,
    }
}

fn append_fact(
    tx: &Transaction<'_>,
    context: &WriteContext<'_>,
    proof: &TerminalControlHistoryProof,
    publication: &PublishObjectReferenceRequest,
) -> Result<TerminalControlHistoryFact, StoreError> {
    let config = context.config;
    let daemon = context.daemon;
    let now = context.now;
    let source = context.source;
    let c = &source.record.context;
    let payload = HistoryPayload {
        schema_version: 1,
        dispatch_id: proof.ack.dispatch_id,
        context_digest: source.record.context_digest,
        proof_hash: publication.object_hash,
        object_generation: publication.object_generation,
        producer_pin_id: publication.pin_id.clone(),
        native_ack_digest: canonical_sha256(&proof.ack)?,
        inspected_by_epoch: daemon.daemon_epoch,
        inspected_by: c.actor_id.clone(),
        observed_at_ms: proof.inspection.observed_at_ms,
    };
    let event = config.id_generator.new_uuid().to_string();
    let revision = i64::try_from(source.broker.revision.as_u64())
        .map_err(|_| invalid("historical action revision is invalid"))?;
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'action',?3,?4,(SELECT COALESCE(MAX(event_ordinal)+1,0) FROM events WHERE aggregate_type='action' AND aggregate_id=?3 AND aggregate_revision=?4),?5,1,?6,?7,?7,?8,?9,?10,'full',?11,'restricted','action',?3,NULL,?3)",params![event,daemon.stream_id.to_string(),c.action_id.to_string(),revision,TERMINAL_CONTROL_HISTORY_EVENT,c.project_id.to_string(),c.actor_id.as_str(),payload.observed_at_ms.as_i64(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.to_string()])?;
    let sequence = u64::try_from(tx.last_insert_rowid())
        .map_err(|_| unavailable("historical event sequence is invalid"))?;
    tx.execute("INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",params![event,now.as_i64(),daemon.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")])?;
    Ok(fact(
        source,
        proof,
        &payload,
        EventCursor::new(daemon.stream_id, Sequence::new(sequence)),
    ))
}
