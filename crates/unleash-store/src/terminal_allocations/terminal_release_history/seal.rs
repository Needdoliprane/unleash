//! Immutable native Release facts, with the old receipt as their cause.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{DaemonEpoch, ReceiptId, Sequence, Sha256Digest, TimestampMs};
use unleash_protocol::{EventCursor, canonical_sha256, canonicalize_to_vec};

use super::super::terminal_control_history::{
    MAX_PROOF_BYTES, reliable_clock, replay_historical_pin,
};
use super::super::terminal_release_observation::validate_native;
use super::super::terminal_release_types::{TerminalReleaseNativeAck, TerminalReleaseObservation};
use super::{
    TerminalReleaseHistorySource, TerminalReleaseHistoryTarget, invalid,
    qualify_terminal_release_history_source,
};
use crate::commands::read_daemon_state_conn;
use crate::objects::{
    read_prepared_evidence_bytes_bounded, read_verified_evidence_bytes_bounded,
    tx_publish_owned_reference, tx_release_pin,
};
use crate::{
    CallerAuthority, DaemonStateRow, PreparedObjectHandle, PublishObjectReferenceRequest,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};

/// Attests an inspected Release without disposing of the unknown intention.
pub const TERMINAL_RELEASE_HISTORY_EVENT: &str = "TerminalReleaseHistoryObserved";

/// Historical inspection proof prepared by the daemon, never by the client.
#[derive(Debug, Clone)]
pub struct SealTerminalReleaseHistoryRequest {
    /// Authenticated authority, resolved by the daemon.
    pub caller: CallerAuthority,
    /// Exact original action, dispatch and terminal generation.
    pub target: TerminalReleaseHistoryTarget,
    /// Object holding the canonical native inspection.
    pub prepared_proof: PreparedObjectHandle,
}

/// Causal observation, without a current credential or lease grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalReleaseHistoryFact {
    /// Original acceptance, preserved in its original state.
    pub source_receipt_id: ReceiptId,
    /// Complete native acknowledgement fingerprint.
    pub native_ack_digest: Sha256Digest,
    /// First published inspection object, retained even on replay.
    pub observation_object_hash: Sha256Digest,
    /// Epoch that sealed the first observation.
    pub inspected_by_epoch: DaemonEpoch,
    /// Timestamp of the retained first inspection.
    pub observed_at_ms: TimestampMs,
    /// Exact cursor of the attestation event.
    pub event_cursor: EventCursor,
}

/// Result of publishing or replaying a historical observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalReleaseHistoryCommit {
    /// The durable observation.
    pub fact: TerminalReleaseHistoryFact,
    /// Whether publication had already committed.
    pub replayed: bool,
}

impl UnleashStore {
    /// Publish a Release observation atomically with its reference and event.
    ///
    /// # Errors
    /// Refuses foreign, malformed, unavailable or divergent cached evidence.
    pub fn seal_terminal_release_history(
        &self,
        request: SealTerminalReleaseHistoryRequest,
    ) -> Result<TerminalReleaseHistoryCommit, StoreError> {
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            let source =
                qualify_terminal_release_history_source(tx, &request.caller, request.target)?;
            let daemon = read_daemon_state_conn(tx)?;
            let commit = seal_tx(
                tx,
                &config,
                &daemon,
                now,
                &source,
                Some(&request.prepared_proof),
            )?;
            if !commit.replayed {
                config.cutpoints.hit(StoreCutpoint::C1BeforeCommandCommit)?;
                reliable_clock(&config, now, commit.fact.observed_at_ms)?;
                qualify_terminal_release_history_source(tx, &request.caller, request.target)?;
            }
            Ok(commit)
        })?;
        if !committed.replayed {
            self.config()
                .cutpoints
                .hit(StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(committed)
    }
}

struct SealContext<'a> {
    config: &'a StoreConfig,
    daemon: &'a DaemonStateRow,
    now: TimestampMs,
    source: &'a TerminalReleaseHistorySource,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryPayload {
    schema_version: u32,
    dispatch_id: unleash_domain::DispatchId,
    context_digest: Sha256Digest,
    observation_object_hash: Sha256Digest,
    object_generation: i64,
    producer_pin_id: String,
    native_ack_digest: Sha256Digest,
    inspected_by_epoch: DaemonEpoch,
    captured_at_ms: TimestampMs,
}

impl HistoryPayload {
    fn fact(
        &self,
        source: &TerminalReleaseHistorySource,
        cursor: EventCursor,
    ) -> TerminalReleaseHistoryFact {
        TerminalReleaseHistoryFact {
            source_receipt_id: source.context.receipt_id,
            native_ack_digest: self.native_ack_digest,
            observation_object_hash: self.observation_object_hash,
            inspected_by_epoch: self.inspected_by_epoch,
            observed_at_ms: self.captured_at_ms,
            event_cursor: cursor,
        }
    }
}

struct Sealed {
    fact: TerminalReleaseHistoryFact,
    ack: TerminalReleaseNativeAck,
    producer_pin_id: String,
}

fn unavailable(detail: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(detail.into())
}

pub(super) fn seal_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    source: &TerminalReleaseHistorySource,
    prepared: Option<&PreparedObjectHandle>,
) -> Result<TerminalReleaseHistoryCommit, StoreError> {
    let context = SealContext {
        config,
        daemon,
        now,
        source,
    };
    let prior = read_sealed(tx, config, daemon, source)?;
    match (prior, prepared) {
        (Some(sealed), Some(handle)) => {
            replay(tx, &context, handle, &publication(source, handle), sealed)
        }
        (Some(sealed), None) => Ok(TerminalReleaseHistoryCommit {
            fact: sealed.fact,
            replayed: true,
        }),
        (None, Some(handle)) => {
            let publication = publication(source, handle);
            let observation = prepared_observation(tx, &context, handle, &publication)?;
            tx_publish_owned_reference(
                tx,
                config.id_generator.as_ref(),
                now,
                &publication,
                daemon.authority_generation,
            )?;
            let fact = append_fact(tx, &context, &observation, &publication)?;
            reliable_clock(config, now, observation.captured_at_ms)?;
            Ok(TerminalReleaseHistoryCommit {
                fact,
                replayed: false,
            })
        }
        (None, None) => Err(unavailable(
            "historical release inspection has not been sealed",
        )),
    }
}

fn source_field(source: &TerminalReleaseHistorySource) -> String {
    format!("terminal_release_history:{}", source.key.dispatch_id)
}

fn publication(
    source: &TerminalReleaseHistorySource,
    handle: &PreparedObjectHandle,
) -> PublishObjectReferenceRequest {
    PublishObjectReferenceRequest {
        pin_id: handle.pin_id.clone(),
        object_hash: handle.object_hash,
        object_generation: handle.object_generation,
        owner_kind: "action".into(),
        owner_id: source.context.action_id.to_string(),
        source_field: source_field(source),
    }
}

pub(super) fn inspect_sealed(
    conn: &Connection,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    source: &TerminalReleaseHistorySource,
) -> Result<Option<TerminalReleaseHistoryFact>, StoreError> {
    Ok(read_sealed(conn, config, daemon, source)?.map(|sealed| sealed.fact))
}

fn decode_observation(
    bytes: &[u8],
    source: &TerminalReleaseHistorySource,
    upper_bound: TimestampMs,
) -> Result<TerminalReleaseObservation, StoreError> {
    let observation: TerminalReleaseObservation = serde_json::from_slice(bytes)
        .map_err(|_| invalid("historical release proof is not its closed schema"))?;
    if canonicalize_to_vec(&observation)? != bytes
        || observation.ack.key != source.key
        || observation.captured_at_ms < source.dispatched_at_ms
        || observation.captured_at_ms > upper_bound
    {
        return Err(invalid(
            "historical release proof does not bind the original dispatch",
        ));
    }
    validate_native(
        &source.context,
        source.key.header.payload_hash,
        &observation,
    )?;
    Ok(observation)
}

fn prepared_observation(
    tx: &Transaction<'_>,
    context: &SealContext<'_>,
    handle: &PreparedObjectHandle,
    publication: &PublishObjectReferenceRequest,
) -> Result<TerminalReleaseObservation, StoreError> {
    let bytes = read_prepared_evidence_bytes_bounded(
        tx,
        context.config,
        handle,
        publication,
        context.daemon.authority_generation,
        MAX_PROOF_BYTES,
    )?;
    let observation = decode_observation(&bytes, context.source, context.now)?;
    reliable_clock(context.config, context.now, observation.captured_at_ms)?;
    Ok(observation)
}

fn replay(
    tx: &Transaction<'_>,
    context: &SealContext<'_>,
    handle: &PreparedObjectHandle,
    publication: &PublishObjectReferenceRequest,
    sealed: Sealed,
) -> Result<TerminalReleaseHistoryCommit, StoreError> {
    if handle.object_hash == sealed.fact.observation_object_hash {
        replay_historical_pin(
            tx,
            context.config,
            context.now,
            context.daemon.authority_generation,
            handle,
            publication,
            &sealed.producer_pin_id,
        )?;
    } else {
        let observation = prepared_observation(tx, context, handle, publication)?;
        if observation.ack != sealed.ack {
            return Err(invalid(
                "historical release acknowledgement differs on replay",
            ));
        }
        tx_release_pin(tx, context.now, &publication.pin_id)?;
    }
    Ok(TerminalReleaseHistoryCommit {
        fact: sealed.fact,
        replayed: true,
    })
}

fn read_sealed(
    conn: &Connection,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    source: &TerminalReleaseHistorySource,
) -> Result<Option<Sealed>, StoreError> {
    let c = &source.context;
    let action = c.action_id.to_string();
    let field = source_field(source);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE aggregate_type='action' AND aggregate_id=?1 AND event_type=?2",
        params![c.action_id.to_string(), TERMINAL_RELEASE_HISTORY_EVENT],
        |row| row.get(0)
    )?;
    let refs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM object_references WHERE owner_kind=?1 AND owner_id=?2 AND source_field=?3",
        params!["action", action, field],
        |row| row.get(0)
    )?;
    match (count, refs) {
        (0, 0) => return Ok(None),
        (1, 1) => {}
        _ => {
            return Err(unavailable(
                "historical release event and proof reference are not unique",
            ));
        }
    }
    let (sequence, occurred, recorded, encoded, digest): (i64, i64, i64, String, String) = conn
        .query_row(
            "SELECT sequence,occurred_at_ms,recorded_at_ms,payload_json,payload_hash FROM events
         WHERE aggregate_type='action' AND aggregate_id=?1 AND event_type=?2 AND stream_id=?3
           AND schema_version=1 AND aggregate_revision BETWEEN 1 AND ?4 AND payload_state='full'
           AND actor_id=?5 AND project_id=?6 AND causation_kind='command'
           AND causation_id=?7 AND causation_receipt_id=?7 AND correlation_id=?8",
            params![
                c.action_id.to_string(),
                TERMINAL_RELEASE_HISTORY_EVENT,
                daemon.stream_id.to_string(),
                super::super::integer(source.action_revision.as_u64())?,
                c.actor_id.as_str(),
                c.project_id.to_string(),
                c.receipt_id.to_string(),
                c.terminal_id.to_string()
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| unavailable("historical release event is unavailable"))?;
    let payload: HistoryPayload = serde_json::from_str(&encoded)
        .map_err(|_| unavailable("historical release event payload is invalid"))?;
    if payload.schema_version != 1
        || payload.dispatch_id != source.key.dispatch_id
        || payload.context_digest != source.key.header.payload_hash
        || payload.producer_pin_id.is_empty()
        || payload.inspected_by_epoch < c.daemon_epoch
        || payload.inspected_by_epoch > daemon.daemon_epoch
        || payload.captured_at_ms.as_i64() != occurred
        || canonical_sha256(&payload)?.as_prefixed_str() != digest
    {
        return Err(unavailable(
            "historical release event differs from its source",
        ));
    }
    let live: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash
         WHERE f.owner_kind=?1 AND f.owner_id=?2 AND f.source_field=?3 AND f.object_hash=?4
           AND f.state='live' AND o.state='live' AND o.object_generation=?5)",
        params!["action", action, field,
            payload.observation_object_hash.to_string(),payload.object_generation],|row|row.get(0),
    )?;
    if !live {
        return Err(unavailable(
            "historical release proof reference is unavailable",
        ));
    }
    let bytes = read_verified_evidence_bytes_bounded(
        conn,
        config,
        payload.observation_object_hash,
        MAX_PROOF_BYTES,
    )?;
    let observation = decode_observation(&bytes, source, TimestampMs::new(recorded))?;
    if observation.captured_at_ms != payload.captured_at_ms
        || canonical_sha256(&observation.ack)? != payload.native_ack_digest
    {
        return Err(unavailable(
            "historical release proof differs from its event",
        ));
    }
    let cursor = event_cursor(daemon, sequence)?;
    Ok(Some(Sealed {
        fact: payload.fact(source, cursor),
        ack: observation.ack,
        producer_pin_id: payload.producer_pin_id,
    }))
}

fn event_cursor(daemon: &DaemonStateRow, sequence: i64) -> Result<EventCursor, StoreError> {
    let sequence =
        u64::try_from(sequence).map_err(|_| unavailable("historical release cursor is invalid"))?;
    Ok(EventCursor::new(daemon.stream_id, Sequence::new(sequence)))
}

fn append_fact(
    tx: &Transaction<'_>,
    context: &SealContext<'_>,
    observation: &TerminalReleaseObservation,
    publication: &PublishObjectReferenceRequest,
) -> Result<TerminalReleaseHistoryFact, StoreError> {
    let payload = HistoryPayload {
        schema_version: 1,
        dispatch_id: context.source.key.dispatch_id,
        context_digest: context.source.key.header.payload_hash,
        observation_object_hash: publication.object_hash,
        object_generation: publication.object_generation,
        producer_pin_id: publication.pin_id.clone(),
        native_ack_digest: canonical_sha256(&observation.ack)?,
        inspected_by_epoch: context.daemon.daemon_epoch,
        captured_at_ms: observation.captured_at_ms,
    };
    let c = &context.source.context;
    let event = context.config.id_generator.new_uuid().to_string();
    let action_rev = super::super::integer(context.source.action_revision.as_u64())?;
    tx.execute(
        "INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,
          schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,
          payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id)
         VALUES(?1,?2,'action',?3,?4,(SELECT COALESCE(MAX(event_ordinal)+1,0) FROM events
           WHERE aggregate_type='action' AND aggregate_id=?3 AND aggregate_revision=?4),
           ?5,1,?6,?7,?7,?8,?9,?10,'full',?11,'restricted','command',?12,?12,?13)",
        params![event,context.daemon.stream_id.to_string(),c.action_id.to_string(),action_rev,
            TERMINAL_RELEASE_HISTORY_EVENT,c.project_id.to_string(),c.actor_id.as_str(),
            payload.captured_at_ms.as_i64(),context.now.as_i64(),serde_json::to_string(&payload)?,
            canonical_sha256(&payload)?.to_string(),c.receipt_id.to_string(),c.terminal_id.to_string()],
    )?;
    let cursor = event_cursor(context.daemon, tx.last_insert_rowid())?;
    tx.execute(
        "INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,
          consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",
        params![event,context.now.as_i64(),context.daemon.authority_generation.to_string(),
            context.config.consumer_version,format!("{event}:clients")],
    )?;
    Ok(payload.fact(context.source, cursor))
}
