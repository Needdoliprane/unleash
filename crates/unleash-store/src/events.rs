//! Journal d'événements, outbox durable, inbox transactionnelle, `dead_letter` et `SubscribeEvents` (`REQ-PERSIST-003`).

use serde::{Deserialize, Serialize};
use unleash_domain::{
    AuthorityGeneration, DaemonEpoch, Sequence, Sha256Digest, StreamId, TimestampMs,
};
use unleash_protocol::{EventCursor, EventEnvelope};
use uuid::Uuid;

/// Nombre maximal de tentatives non transitoires avant passage en `dead_letter` (ADR-02).
pub const MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS: i64 = 8;

/// Parallélisme maximal entre agrégats dans le dispatcher outbox (ADR-02).
pub const MAX_OUTBOX_AGGREGATE_PARALLELISM: usize = 32;

/// État d'une entrée `outbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    /// En attente de livraison.
    Pending,
    /// Louée par le dispatcher dans l'époque courante.
    Leased,
    /// Acquittée après consommation réussie.
    Acked,
    /// En échec définitif après 8 échecs non transitoires.
    DeadLetter,
}

/// Entrée `outbox` prête ou louée pour livraison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxDispatchItem {
    /// Identifiant de l'événement.
    pub event_id: Uuid,
    /// Séquence de l'événement dans le flux.
    pub sequence: Sequence,
    /// Agrégat concerné (pour ordre par `(destination, aggregate_id)`).
    pub aggregate_type: String,
    /// Identifiant d'agrégat.
    pub aggregate_id: Uuid,
    /// Destination (`inbox.consumer_id`).
    pub destination: String,
    /// Clé d'idempotence de livraison.
    pub delivery_key: String,
    /// Nombre de tentatives déjà effectuées.
    pub delivery_attempt: i64,
    /// Époque de lease si louée.
    pub lease_epoch: Option<DaemonEpoch>,
    /// Enveloppe complète de l'événement à livrer.
    pub envelope: EventEnvelope,
}

/// Entrée `dead_letter` exposée à `GetHealth` et `GetDaemonStatus` (ADR-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterItem {
    /// Consommateur cible (`outbox.destination`).
    pub consumer_id: String,
    /// Identifiant de l'événement en échec.
    pub event_id: Uuid,
    /// Dernière erreur enregistrée.
    pub last_error: String,
    /// Nombre total de tentatives.
    pub delivery_attempt: i64,
    /// Version du consommateur ayant échoué.
    pub consumer_version: Option<String>,
    /// Lignée associée.
    pub authority_generation: Option<AuthorityGeneration>,
}

/// Résultat d'application dédupliquée d'un événement dans `inbox`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxApplyOutcome {
    /// Première application : effet métier et ligne `inbox` committés atomiquement.
    Applied {
        /// Empreinte du résultat déterministe.
        result_hash: Sha256Digest,
    },
    /// Doublon détecté dans `inbox` : aucune seconde mutation exécutée.
    AlreadyProcessed {
        /// Empreinte conservée du premier traitement.
        result_hash: Sha256Digest,
    },
}

/// Résultat de lecture incrémentale d'événements (`ReadEvents` / `SubscribeEvents`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadEventsOutcome {
    /// Lot ordonné d'événements et curseur résultant.
    Events {
        /// Événements lus.
        events: Vec<EventEnvelope>,
        /// Curseur atteint.
        cursor: EventCursor,
    },
    /// Curseur expiré (> 7 j), tombstoné ou stream changé (après restore).
    ResyncRequired {
        /// Identifiant du flux actif.
        stream_id: StreamId,
        /// Watermark courant à recharger via snapshot.
        current_watermark: EventCursor,
        /// Motif du resync.
        reason: String,
    },
}

/// Rapport de purge de `inbox` ou de `events` (`PurgeHistory`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeStats {
    /// Nombre de lignes purgées (supprimées pour `inbox`, tombstonées pour `events`).
    pub affected_rows: usize,
    /// Indique si un `PRAGMA wal_checkpoint(TRUNCATE)` final a été exécuté.
    pub wal_truncated: bool,
    /// Timestamp de fin.
    pub completed_at_ms: TimestampMs,
}

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};
use unleash_domain::Revision;
use unleash_protocol::{EventCausation, EventCausationKind, Sensitivity};

use crate::commands::read_daemon_state_conn;
use crate::config::{EVENT_AND_INBOX_MIN_RETENTION_MS, StoreConfig};
use crate::error::StoreError;

pub(crate) fn read_events_with_window(
    conn: &Connection,
    now_ms: TimestampMs,
    requested_stream_id: StreamId,
    after_sequence: Sequence,
    limit: u32,
    cursor_recorded_at_ms: Option<TimestampMs>,
) -> Result<ReadEventsOutcome, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    let max_seq_i64: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
            [daemon.stream_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let watermark = EventCursor::new(
        daemon.stream_id,
        Sequence::new(u64::try_from(max_seq_i64).unwrap_or(0)),
    );

    if requested_stream_id != daemon.stream_id {
        return Ok(ReadEventsOutcome::ResyncRequired {
            stream_id: daemon.stream_id,
            current_watermark: watermark,
            reason: "stream_id changé (après restauration)".to_owned(),
        });
    }

    if let Some(cursor_ts) = cursor_recorded_at_ms
        && now_ms.as_i64().saturating_sub(cursor_ts.as_i64()) > EVENT_AND_INBOX_MIN_RETENTION_MS
    {
        return Ok(ReadEventsOutcome::ResyncRequired {
            stream_id: daemon.stream_id,
            current_watermark: watermark,
            reason: "curseur antérieur à la fenêtre garantie de 7 jours".to_owned(),
        });
    }

    let after_seq_i64 = i64::try_from(after_sequence.as_u64()).unwrap_or(i64::MAX);
    if after_seq_i64 > 0 {
        let cursor_rec: Option<(i64, String)> = conn
            .query_row(
                "SELECT recorded_at_ms, payload_state FROM events WHERE stream_id = ?1 AND sequence = ?2",
                rusqlite::params![daemon.stream_id.to_string(), after_seq_i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        if let Some((rec_ms, pstate)) = cursor_rec
            && (pstate == "tombstone"
                || now_ms.as_i64().saturating_sub(rec_ms) > EVENT_AND_INBOX_MIN_RETENTION_MS)
        {
            return Ok(ReadEventsOutcome::ResyncRequired {
                stream_id: daemon.stream_id,
                current_watermark: watermark,
                reason: "curseur dépassé ou historique tombstoné".to_owned(),
            });
        }
    }

    let has_tombstone_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events
             WHERE stream_id = ?1 AND sequence > ?2 AND payload_state = 'tombstone'",
            rusqlite::params![daemon.stream_id.to_string(), after_seq_i64],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if has_tombstone_after > 0 {
        return Ok(ReadEventsOutcome::ResyncRequired {
            stream_id: daemon.stream_id,
            current_watermark: watermark,
            reason: "événements non consommés déjà tombstonés".to_owned(),
        });
    }

    let lim_i64 = i64::from(limit.max(1));
    let mut stmt = conn
        .prepare(
            "SELECT event_id, stream_id, sequence, event_type, schema_version, occurred_at_ms,
                    recorded_at_ms, project_id, aggregate_id, aggregate_revision, actor_ref,
                    correlation_id, causation_kind, causation_id, command_id, payload_hash,
                    payload_json, sensitivity
             FROM events
             WHERE stream_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )
        .map_err(StoreError::from_rusqlite)?;

    let rows = stmt
        .query_map(
            rusqlite::params![daemon.stream_id.to_string(), after_seq_i64, lim_i64],
            row_to_event_envelope,
        )
        .map_err(StoreError::from_rusqlite)?;

    let mut events = Vec::new();
    for item in rows {
        events.push(item.map_err(StoreError::from_rusqlite)?);
    }

    let end_cursor = events.last().map_or(watermark, |e| {
        EventCursor::new(daemon.stream_id, e.sequence)
    });

    Ok(ReadEventsOutcome::Events {
        events,
        cursor: end_cursor,
    })
}

fn row_to_event_envelope(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventEnvelope> {
    let event_id_str: String = r.get(0)?;
    let stream_str: String = r.get(1)?;
    let seq_i64: i64 = r.get(2)?;
    let event_type: String = r.get(3)?;
    let schema_ver: u32 = r.get(4)?;
    let occ_ms: i64 = r.get(5)?;
    let rec_ms: i64 = r.get(6)?;
    let proj_opt: Option<String> = r.get(7)?;
    let agg_id_str: String = r.get(8)?;
    let agg_rev_i64: i64 = r.get(9)?;
    let actor_ref: String = r.get(10)?;
    let corr_str: String = r.get(11)?;
    let caus_kind_str: String = r.get(12)?;
    let caus_id: String = r.get(13)?;
    let cmd_id_opt: Option<String> = r.get(14)?;
    let payload_hash_str: String = r.get(15)?;
    let payload_json_str: String = r.get(16)?;
    let sens_str: String = r.get(17)?;

    let event_id: Uuid = event_id_str
        .parse()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let stream_id: StreamId = stream_str
        .parse()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let project_id = match proj_opt {
        Some(p) => Some(
            p.parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        ),
        None => None,
    };
    let aggregate_id: Uuid = agg_id_str
        .parse()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let correlation_id: Uuid = corr_str
        .parse()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let command_id = match cmd_id_opt {
        Some(c) => Some(
            c.parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        ),
        None => None,
    };
    let payload_hash: Sha256Digest = payload_hash_str
        .parse()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let payload: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&payload_json_str)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let caus_kind = match caus_kind_str.as_str() {
        "command" => EventCausationKind::Command,
        "event" => EventCausationKind::Event,
        "action" => EventCausationKind::Action,
        "timer" => EventCausationKind::Timer,
        _ => EventCausationKind::External,
    };
    let sensitivity = match sens_str.as_str() {
        "public" => Sensitivity::Public,
        "project" => Sensitivity::Project,
        "confidential" => Sensitivity::Confidential,
        _ => Sensitivity::Restricted,
    };

    Ok(EventEnvelope {
        event_id,
        stream_id,
        sequence: Sequence::new(u64::try_from(seq_i64).unwrap_or(1)),
        event_type,
        schema_version: schema_ver,
        occurred_at: DateTime::<Utc>::from_timestamp_millis(occ_ms).unwrap_or(DateTime::UNIX_EPOCH),
        recorded_at: DateTime::<Utc>::from_timestamp_millis(rec_ms).unwrap_or(DateTime::UNIX_EPOCH),
        project_id,
        aggregate_id,
        aggregate_revision: Revision::new(u64::try_from(agg_rev_i64).unwrap_or(1)),
        actor_ref,
        correlation_id,
        causation: EventCausation {
            kind: caus_kind,
            id: caus_id,
        },
        command_id,
        payload_hash,
        payload,
        sensitivity,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "Requête SQL de location bornée par agrégat et mise à jour atomique outbox"
)]
pub(crate) fn tx_lease_outbox_batch(
    tx: &Transaction<'_>,
    authority: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    now_ms: TimestampMs,
    destination: &str,
    limit: usize,
) -> Result<Vec<OutboxDispatchItem>, StoreError> {
    let global_frozen: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM recovery_scope_freezes WHERE global_freeze = 1 AND state = 'frozen'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if global_frozen > 0 {
        return Ok(Vec::new());
    }

    let current_leased: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE state = 'leased'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let avail = MAX_OUTBOX_AGGREGATE_PARALLELISM
        .saturating_sub(usize::try_from(current_leased).unwrap_or(0));
    let cap = limit.min(avail);
    if cap == 0 {
        return Ok(Vec::new());
    }

    let mut stmt = tx
        .prepare(
            "SELECT e.event_id, e.stream_id, e.sequence, e.event_type, e.schema_version, e.occurred_at_ms,
                    e.recorded_at_ms, e.project_id, e.aggregate_id, e.aggregate_revision, e.actor_ref,
                    e.correlation_id, e.causation_kind, e.causation_id, e.command_id, e.payload_hash,
                    e.payload_json, e.sensitivity, e.aggregate_type, o.delivery_key, o.delivery_attempt
             FROM outbox o
             JOIN events e ON e.event_id = o.event_id
             WHERE o.destination = ?1
               AND o.state = 'pending'
               AND o.next_attempt_at_ms <= ?2
               AND NOT EXISTS (
                   SELECT 1 FROM recovery_scope_freezes f
                   WHERE f.state = 'frozen'
                     AND (f.scope_id = e.project_id OR f.scope_id = e.aggregate_id)
               )
               AND NOT EXISTS (
                   SELECT 1 FROM outbox o2
                   JOIN events e2 ON e2.event_id = o2.event_id
                   WHERE o2.destination = o.destination
                     AND e2.aggregate_type = e.aggregate_type
                     AND e2.aggregate_id = e.aggregate_id
                     AND (o2.state = 'leased' OR (o2.state IN ('pending', 'dead_letter') AND e2.sequence < e.sequence))
               )
             ORDER BY e.sequence ASC
             LIMIT ?3",
        )
        .map_err(StoreError::from_rusqlite)?;

    let mut candidates = Vec::new();
    let rows = stmt
        .query_map(
            rusqlite::params![
                destination,
                now_ms.as_i64(),
                i64::try_from(cap).unwrap_or(32)
            ],
            |r| {
                let env = row_to_event_envelope(r)?;
                let agg_type: String = r.get(18)?;
                let del_key: String = r.get(19)?;
                let del_att: i64 = r.get(20)?;
                Ok((env, agg_type, del_key, del_att))
            },
        )
        .map_err(StoreError::from_rusqlite)?;

    for item in rows {
        candidates.push(item.map_err(StoreError::from_rusqlite)?);
    }
    drop(stmt);

    let mut out = Vec::new();
    let mut seen_aggs = std::collections::HashSet::new();
    let epoch_i64 = i64::try_from(daemon_epoch.as_u64()).unwrap_or(1);

    for (env, agg_type, del_key, del_att) in candidates {
        if !seen_aggs.insert((agg_type.clone(), env.aggregate_id)) {
            continue;
        }
        tx.execute(
            "UPDATE outbox
             SET state = 'leased', authority_generation = ?1, lease_epoch = ?2
             WHERE event_id = ?3 AND destination = ?4",
            rusqlite::params![
                authority.to_string(),
                epoch_i64,
                env.event_id.to_string(),
                destination
            ],
        )
        .map_err(StoreError::from_rusqlite)?;

        out.push(OutboxDispatchItem {
            event_id: env.event_id,
            sequence: env.sequence,
            aggregate_type: agg_type,
            aggregate_id: env.aggregate_id,
            destination: destination.to_owned(),
            delivery_key: del_key,
            delivery_attempt: del_att,
            lease_epoch: Some(daemon_epoch),
            envelope: env,
        });
    }

    Ok(out)
}

pub(crate) fn tx_apply_inbox_event<F>(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    consumer_id: &str,
    event_id: Uuid,
    result_hash: Sha256Digest,
    mutation_fn: F,
) -> Result<InboxApplyOutcome, StoreError>
where
    F: FnOnce(&Transaction<'_>) -> Result<(), StoreError>,
{
    let ev_str = event_id.to_string();
    let existing_hash: Option<String> = tx
        .query_row(
            "SELECT result_hash FROM inbox WHERE consumer_id = ?1 AND event_id = ?2",
            rusqlite::params![consumer_id, ev_str],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some(prev) = existing_hash {
        let parsed = prev.parse::<Sha256Digest>()?;
        return Ok(InboxApplyOutcome::AlreadyProcessed {
            result_hash: parsed,
        });
    }

    mutation_fn(tx)?;

    tx.execute(
        "INSERT INTO inbox (consumer_id, event_id, processed_at_ms, result_hash)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            consumer_id,
            ev_str,
            now_ms.as_i64(),
            result_hash.as_prefixed_str()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(InboxApplyOutcome::Applied { result_hash })
}

pub(crate) fn tx_ack_outbox(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    event_id: Uuid,
    destination: &str,
) -> Result<(), StoreError> {
    let updated = tx
        .execute(
            "UPDATE outbox SET state = 'acked', acked_at_ms = ?1, lease_epoch = NULL
             WHERE event_id = ?2 AND destination = ?3",
            rusqlite::params![now_ms.as_i64(), event_id.to_string(), destination],
        )
        .map_err(StoreError::from_rusqlite)?;
    if updated == 0 {
        return Err(StoreError::NotFound(format!(
            "entrée outbox introuvable : ({event_id}, {destination})"
        )));
    }
    Ok(())
}

pub(crate) fn tx_record_transient_failure(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    event_id: Uuid,
    destination: &str,
    error_message: &str,
    jitter_bps: i16,
) -> Result<TimestampMs, StoreError> {
    let attempt: i64 = tx
        .query_row(
            "SELECT delivery_attempt FROM outbox WHERE event_id = ?1 AND destination = ?2",
            rusqlite::params![event_id.to_string(), destination],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let next_attempt = attempt.saturating_add(1);
    let shift = u32::try_from(attempt.clamp(0, 10)).unwrap_or(10);
    let base_ms = 200_i64.saturating_mul(1_i64 << shift).min(60_000);
    let clamped_jitter = i64::from(jitter_bps.clamp(-2000, 2000));
    let jittered = base_ms.saturating_add(base_ms.saturating_mul(clamped_jitter) / 10_000);
    let next_at = now_ms.as_i64().saturating_add(jittered);

    tx.execute(
        "UPDATE outbox
         SET state = 'pending', lease_epoch = NULL, delivery_attempt = ?1, next_attempt_at_ms = ?2, last_error = ?3
         WHERE event_id = ?4 AND destination = ?5",
        rusqlite::params![
            next_attempt,
            next_at,
            error_message,
            event_id.to_string(),
            destination
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(TimestampMs::new(next_at))
}

pub(crate) fn tx_record_non_transient_failure(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    event_id: Uuid,
    destination: &str,
    error_message: &str,
    aggregate_blocking_target: Option<(&str, Uuid)>,
) -> Result<bool, StoreError> {
    let cur_attempts: i64 = tx
        .query_row(
            "SELECT delivery_attempt FROM outbox WHERE event_id = ?1 AND destination = ?2",
            rusqlite::params![event_id.to_string(), destination],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    let next_attempts = cur_attempts.saturating_add(1);
    let is_dead = next_attempts >= MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS;
    let new_state = if is_dead { "dead_letter" } else { "pending" };

    tx.execute(
        "UPDATE outbox
         SET state = ?1, lease_epoch = NULL, delivery_attempt = ?2, last_error = ?3, consumer_version = ?4
         WHERE event_id = ?5 AND destination = ?6",
        rusqlite::params![
            new_state,
            next_attempts,
            error_message,
            config.consumer_version,
            event_id.to_string(),
            destination
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    if is_dead && let Some((agg_kind, agg_id)) = aggregate_blocking_target {
        let id_str = agg_id.to_string();
        match agg_kind {
            "mission" => {
                tx.execute(
                    "UPDATE missions SET state = 'blocked', revision = revision + 1 WHERE mission_id = ?1",
                    [&id_str],
                )
                .map_err(StoreError::from_rusqlite)?;
            }
            "workflow_run" => {
                tx.execute(
                    "UPDATE workflow_runs SET state = 'blocked', revision = revision + 1 WHERE run_id = ?1",
                    [&id_str],
                )
                .map_err(StoreError::from_rusqlite)?;
            }
            "goal" => {
                tx.execute(
                    "UPDATE goals SET state = 'blocked', revision = revision + 1 WHERE goal_id = ?1",
                    [&id_str],
                )
                .map_err(StoreError::from_rusqlite)?;
            }
            "agent" => {
                tx.execute(
                    "UPDATE agents SET state = 'blocked', revision = revision + 1 WHERE agent_id = ?1",
                    [&id_str],
                )
                .map_err(StoreError::from_rusqlite)?;
            }
            _ => {}
        }
    }

    Ok(is_dead)
}

pub(crate) fn read_dead_letters_conn(conn: &Connection) -> Result<Vec<DeadLetterItem>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT destination, event_id, COALESCE(last_error, ''), delivery_attempt, consumer_version, authority_generation
             FROM outbox WHERE state = 'dead_letter' ORDER BY event_id ASC, destination ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            let ev_str: String = r.get(1)?;
            let ev_id: Uuid = ev_str
                .parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let auth_opt: Option<String> = r.get(5)?;
            let auth = match auth_opt {
                Some(s) => Some(
                    s.parse::<AuthorityGeneration>()
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                ),
                None => None,
            };
            Ok(DeadLetterItem {
                consumer_id: r.get(0)?,
                event_id: ev_id,
                last_error: r.get(2)?,
                delivery_attempt: r.get(3)?,
                consumer_version: r.get(4)?,
                authority_generation: auth,
            })
        })
        .map_err(StoreError::from_rusqlite)?;
    let mut items = Vec::new();
    for item in rows {
        items.push(item.map_err(StoreError::from_rusqlite)?);
    }
    Ok(items)
}

pub(crate) fn tx_purge_expired_inbox(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    clock_uncertain: bool,
) -> Result<usize, StoreError> {
    if clock_uncertain {
        return Ok(0);
    }
    let cutoff = now_ms
        .as_i64()
        .saturating_sub(EVENT_AND_INBOX_MIN_RETENTION_MS);
    let deleted = tx
        .execute(
            "DELETE FROM inbox
             WHERE EXISTS (
                 SELECT 1 FROM outbox o
                 WHERE o.event_id = inbox.event_id
                   AND o.destination = inbox.consumer_id
                   AND o.state = 'acked'
                   AND o.acked_at_ms IS NOT NULL
                   AND o.acked_at_ms <= ?1
             )",
            [cutoff],
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(deleted)
}

pub(crate) fn tx_purge_history(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    older_than_ms: TimestampMs,
    clock_uncertain: bool,
) -> Result<usize, StoreError> {
    if clock_uncertain {
        return Ok(0);
    }
    let min_cutoff = now_ms
        .as_i64()
        .saturating_sub(EVENT_AND_INBOX_MIN_RETENTION_MS);
    let effective_cutoff = older_than_ms.as_i64().min(min_cutoff);
    let updated = tx
        .execute(
            "UPDATE events
             SET payload_state = 'tombstone',
                 payload_json = '{\"tombstoned\":true}'
             WHERE payload_state = 'full'
               AND recorded_at_ms <= ?1
               AND NOT EXISTS (
                   SELECT 1 FROM outbox o
                   WHERE o.event_id = events.event_id AND o.state != 'acked'
               )",
            [effective_cutoff],
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(updated)
}
