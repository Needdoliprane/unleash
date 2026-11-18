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
use unleash_domain::{BackupId, ReceiptId, Revision};
use unleash_protocol::{EventCausation, EventCausationKind, Sensitivity};

use crate::backup::BackupRetentionTier;
use crate::commands::read_daemon_state_conn;
use crate::config::{EVENT_AND_INBOX_MIN_RETENTION_MS, StoreConfig};
use crate::error::StoreError;
use crate::objects::{PrepareObjectDeletionRequest, tx_prepare_object_deletion};
use crate::projects_settings::{RetentionClass, resolve_effective_retention_tx};

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

/// Requête d'exécution de purge sous politique Q-12 (`PurgeHistory` / `unleash.retention.default.v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPurgeRequest {
    /// Portée des paramètres de rétention appliqués (`"user"`, `"project"`, ...).
    pub scope_kind: String,
    /// Identifiant de portée (`"default"` ou `project_id`).
    pub scope_id: String,
    /// Acteur ayant demandé ou déclenché la purge.
    pub requested_by_actor: String,
    /// Motif auditée de purge.
    pub reason: String,
    /// Borne supérieure explicite optionnelle pour le tombstoning événements (toujours >= 7 j).
    pub older_than_override_ms: Option<TimestampMs>,
    /// Vrai si déclenchée automatiquement par le balayage périodique (suspendu si `clock_uncertain`).
    pub is_automatic_sweep: bool,
}

/// État durable d'une purge logique et de ses suppressions physiques/checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPurgeStatusRecord {
    /// Identifiant durable, également porté par `result_ref`.
    pub purge_id: String,
    /// Reçu causal lorsqu'il s'agit d'une commande.
    pub command_receipt_id: Option<ReceiptId>,
    /// Nombre d'intentions de suppression préparées au commit.
    pub objects_deletion_prepared_count: usize,
    /// Intentions dont l'unlink/finalisation reste en attente.
    pub pending_deletions: usize,
    /// Intentions arrêtées sur un objet étranger ou corrompu préservé.
    pub quarantined_deletions: usize,
    /// Checkpoint de fin de purge effectivement réussi.
    pub wal_checkpoint_truncated: bool,
    /// `pending`, `quarantined`, `checkpoint_pending` ou `completed`.
    pub state: String,
}

pub(crate) fn read_retention_purge_statuses_conn(
    conn: &Connection,
) -> Result<Vec<RetentionPurgeStatusRecord>, StoreError> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'retention_purges'",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if table_exists == 0 {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "SELECT p.purge_id, p.command_receipt_id, p.objects_purged_count, p.wal_checkpoint_truncated,
            (SELECT COUNT(*) FROM object_deletions d WHERE d.reason = 'retention_q12:' || p.purge_id AND d.state != 'completed'),
            (SELECT COUNT(*) FROM object_deletions d JOIN object_records o ON o.object_hash = d.object_hash
                AND o.object_generation = d.object_generation
                WHERE d.reason = 'retention_q12:' || p.purge_id AND d.state = 'completed' AND o.state != 'tombstone')
         FROM retention_purges p ORDER BY p.executed_at_ms, p.purge_id"
    ).map_err(StoreError::from_rusqlite)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;
    let mut records = Vec::new();
    for row in rows {
        let (purge_id, receipt, count, checkpoint, pending, quarantined) =
            row.map_err(StoreError::from_rusqlite)?;
        records.push(RetentionPurgeStatusRecord {
            purge_id,
            command_receipt_id: receipt.as_deref().map(str::parse).transpose()?,
            objects_deletion_prepared_count: usize::try_from(count).unwrap_or(0),
            pending_deletions: usize::try_from(pending).unwrap_or(0),
            quarantined_deletions: usize::try_from(quarantined).unwrap_or(0),
            wal_checkpoint_truncated: checkpoint,
            state: if quarantined > 0 {
                "quarantined"
            } else if pending > 0 {
                "pending"
            } else if !checkpoint {
                "checkpoint_pending"
            } else {
                "completed"
            }
            .to_owned(),
        });
    }
    Ok(records)
}
/// Sauvegarde gérée impactée contenant encore des octets antérieurs à la purge (rotation <= 35 j).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedBackupEntry {
    /// Identifiant de la sauvegarde gérée.
    pub backup_id: BackupId,
    /// Niveau de rotation (`daily`, `weekly`, `manual`).
    pub retention_tier: BackupRetentionTier,
    /// Chemin de destination de la sauvegarde.
    pub destination_path: Option<String>,
    /// Date de création de la sauvegarde.
    pub created_at_ms: TimestampMs,
    /// Durée maximale de rétention résiduelle dans la rotation (au plus 35 j).
    pub max_residual_days: u32,
}

/// Rapport complet d'audit retourné par `execute_retention_purge` (`PurgeHistory` Q-12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPurgeReport {
    /// Identifiant unique d'audit dans `retention_purges`.
    pub purge_id: String,
    /// Identifiant de politique appliqué (`unleash.retention.default.v1`).
    pub policy_version: String,
    /// Vrai si le balayage a été suspendu par horloge incertaine (`clock_uncertain`).
    pub suspended_clock_uncertain: bool,
    /// Événements passés à `payload_state = 'tombstone'` (lignes préservées).
    pub events_tombstoned_count: usize,
    /// Reçus de commande passés à `response_state = 'tombstone'`.
    pub receipts_tombstoned_count: usize,
    /// Objets expirés dont la suppression durable est engagée (finalisés avant retour réussi de la façade).
    pub objects_purged_count: usize,
    /// Objets/événements âgés épargnés car requis par un run/action/preuve/version/pin actif.
    pub protected_skipped_count: usize,
    /// Sauvegardes locales gérées impactées pouvant encore contenir ces octets (<= 35 j).
    pub affected_backups: Vec<AffectedBackupEntry>,
    /// Limites explicites affichées (`APFS`/Time Machine, exports remis, copies distantes, manuelles).
    pub limitations: Vec<String>,
    /// Vrai après exécution effective de `PRAGMA wal_checkpoint(TRUNCATE)`.
    pub wal_checkpoint_truncated: bool,
    /// Taille observée du fichier `-wal` après troncature.
    pub wal_bytes_after: u64,
    /// Horodatage d'exécution.
    pub executed_at_ms: TimestampMs,
}

fn retention_limitations() -> Vec<String> {
    vec![
        "Les sauvegardes gérées listées ci-contre peuvent conserver les octets purgés au plus 35 jours selon leur rotation (7 quotidiennes + 4 hebdomadaires), sauf suppression explicite.".to_owned(),
        "Unleash n'efface ni les sauvegardes manuelles ou externes, ni les snapshots APFS / Time Machine, ni les exports déjà remis, ni les copies conservées chez les fournisseurs distants.".to_owned(),
    ]
}

fn is_event_protected_tx(
    tx: &Transaction<'_>,
    aggregate_type: &str,
    aggregate_id: &str,
) -> Result<bool, StoreError> {
    let protected: i64 = match aggregate_type {
        "action" => tx.query_row(
            "SELECT COUNT(*) FROM actions WHERE action_id = ?1 AND state IN ('outcome_unknown', 'prepared', 'authorizing', 'approval_required', 'authorized', 'dispatching', 'in_flight', 'cancel_requested')",
            [aggregate_id],
            |r| r.get(0),
        ),
        "workflow_run" => tx.query_row(
            "SELECT COUNT(*) FROM workflow_runs WHERE run_id = ?1 AND state NOT IN ('succeeded', 'failed', 'cancelled')",
            [aggregate_id],
            |r| r.get(0),
        ),
        "mission" => tx.query_row(
            "SELECT COUNT(*) FROM missions WHERE mission_id = ?1 AND state NOT IN ('succeeded', 'failed', 'cancelled')",
            [aggregate_id],
            |r| r.get(0),
        ),
        "invocation" => tx.query_row(
            "SELECT COUNT(*) FROM invocations WHERE invocation_id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
            [aggregate_id],
            |r| r.get(0),
        ),
        _ => Ok(0),
    }
    .map_err(StoreError::from_rusqlite)?;
    Ok(protected > 0)
}

fn is_object_protected_tx(
    tx: &Transaction<'_>,
    object_hash: &str,
    keep_by_policy: bool,
) -> Result<bool, StoreError> {
    const PROTECTION_QUERIES: &[&str] = &[
        "SELECT COUNT(*) FROM object_pins
         WHERE state IN ('held', 'reconciling')
           AND (pin_kind = 'backup_barrier' OR object_hash = ?1)",
        "SELECT COUNT(*) FROM evidence
         WHERE object_hash = ?1 AND availability = 'available'",
        "SELECT COUNT(*) FROM workflow_versions
         WHERE content_hash = ?1 AND visibility = 'published'",
        "SELECT COUNT(*) FROM registry_versions
         WHERE content_hash = ?1 AND publication_state = 'published'",
        "SELECT COUNT(*) FROM automation_versions av
         JOIN registry_versions rv ON rv.registry_version_id = av.registry_version_id
         WHERE rv.publication_state = 'published'
           AND (av.content_hash = ?1 OR av.parameter_schema_hash = ?1)",
    ];
    if keep_by_policy {
        return Ok(true);
    }
    for sql in PROTECTION_QUERIES {
        let hits: i64 = tx
            .query_row(sql, [object_hash], |r| r.get(0))
            .map_err(StoreError::from_rusqlite)?;
        if hits > 0 {
            return Ok(true);
        }
    }

    let mut stmt = tx
        .prepare(
            "SELECT owner_kind, owner_id FROM object_references
             WHERE object_hash = ?1 AND state = 'live'",
        )
        .map_err(StoreError::from_rusqlite)?;
    let refs = stmt
        .query_map([object_hash], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(StoreError::from_rusqlite)?;
    let mut owners = Vec::new();
    for item in refs {
        owners.push(item.map_err(StoreError::from_rusqlite)?);
    }
    drop(stmt);

    for (owner_kind, owner_id) in owners {
        let locked: i64 = match owner_kind.as_str() {
            "workflow_run" | "run" => tx.query_row(
                "SELECT COUNT(*) FROM workflow_runs
                 WHERE run_id = ?1 AND state NOT IN ('succeeded', 'failed', 'cancelled')",
                [&owner_id],
                |r| r.get(0),
            ),
            "mission" => tx.query_row(
                "SELECT COUNT(*) FROM missions
                 WHERE mission_id = ?1 AND state NOT IN ('succeeded', 'failed', 'cancelled')",
                [&owner_id],
                |r| r.get(0),
            ),
            "invocation" => tx.query_row(
                "SELECT COUNT(*) FROM invocations
                 WHERE invocation_id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
                [&owner_id],
                |r| r.get(0),
            ),
            "action" => tx.query_row(
                "SELECT COUNT(*) FROM actions
                 WHERE action_id = ?1 AND state IN ('outcome_unknown', 'prepared', 'authorizing', 'approval_required', 'authorized', 'dispatching', 'in_flight', 'cancel_requested')",
                [&owner_id],
                |r| r.get(0),
            ),
            "evidence" => tx.query_row(
                "SELECT COUNT(*) FROM evidence
                 WHERE evidence_id = ?1 AND availability = 'available'",
                [&owner_id],
                |r| r.get(0),
            ),
            "workflow_version" | "skill_version" | "role_version" | "automation_version"
            | "pin" | "keep_by_policy" | "contract_proof" | "restore_import" => Ok(1),
            _ => Ok(0),
        }
        .map_err(StoreError::from_rusqlite)?;
        if locked > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

#[expect(
    clippy::too_many_lines,
    reason = "Exécution transactionnelle complète de purge Q-12 avec préavis, protections, sauvegardes affectées et piste retention_purges"
)]
pub(crate) fn tx_execute_retention_purge(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now_ms: TimestampMs,
    receipt_id: Option<ReceiptId>,
    req: &RetentionPurgeRequest,
) -> Result<RetentionPurgeReport, StoreError> {
    let purge_id = config.id_generator.new_uuid().to_string();
    let policy = resolve_effective_retention_tx(tx, now_ms, &req.scope_kind, &req.scope_id)?;
    let clock_uncertain = config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire);
    if clock_uncertain && req.is_automatic_sweep {
        return Ok(RetentionPurgeReport {
            purge_id,
            policy_version: policy.policy_version,
            suspended_clock_uncertain: true,
            events_tombstoned_count: 0,
            receipts_tombstoned_count: 0,
            objects_purged_count: 0,
            protected_skipped_count: 0,
            affected_backups: vec![],
            limitations: retention_limitations(),
            wal_checkpoint_truncated: false,
            wal_bytes_after: 0,
            executed_at_ms: now_ms,
        });
    }

    let mut protected_skipped_count = 0usize;
    let mut events_tombstoned_count = 0usize;

    let min_ev_days = i64::from(policy.events_min_days.max(7));
    let policy_ev_cutoff = now_ms
        .as_i64()
        .saturating_sub(min_ev_days.saturating_mul(86_400_000));
    let hard_min_ev_cutoff = now_ms
        .as_i64()
        .saturating_sub(EVENT_AND_INBOX_MIN_RETENTION_MS);
    let effective_ev_cutoff = match req.older_than_override_ms {
        Some(ov) => ov.as_i64().min(hard_min_ev_cutoff).min(policy_ev_cutoff),
        None => policy_ev_cutoff.min(hard_min_ev_cutoff),
    };

    // 1. Événements éligibles (>= max(7j, policy)) dont l'outbox est intégralement acked
    {
        let mut ev_stmt = tx
            .prepare(
                "SELECT event_id, aggregate_type, aggregate_id
                 FROM events
                 WHERE payload_state = 'full'
                   AND recorded_at_ms <= ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM outbox o
                       WHERE o.event_id = events.event_id AND o.state != 'acked'
                   )",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = ev_stmt
            .query_map([effective_ev_cutoff], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        let mut candidates = Vec::new();
        for item in rows {
            candidates.push(item.map_err(StoreError::from_rusqlite)?);
        }
        drop(ev_stmt);

        for (ev_id, agg_type, agg_id) in candidates {
            if is_event_protected_tx(tx, &agg_type, &agg_id)? {
                protected_skipped_count += 1;
                continue;
            }
            let updated = tx
                .execute(
                    "UPDATE events
                     SET payload_state = 'tombstone',
                         payload_json = '{\"tombstoned\":true}'
                     WHERE event_id = ?1 AND payload_state = 'full'",
                    [ev_id],
                )
                .map_err(StoreError::from_rusqlite)?;
            events_tombstoned_count += updated;
        }
    }

    // 2. Reçus de commandes expirés (sans jamais raccourcir `response_retained_until_ms` déjà promis !)
    let min_rcpt_cutoff = now_ms.as_i64().saturating_sub(
        i64::from(policy.command_receipts_min_days.max(7)).saturating_mul(86_400_000),
    );
    let receipts_tombstoned_count = tx
        .execute(
            "UPDATE command_receipts
             SET response_state = 'tombstone', response_json = NULL
             WHERE response_state = 'full'
               AND ?1 >= response_retained_until_ms
               AND committed_at_ms <= ?2",
            rusqlite::params![now_ms.as_i64(), min_rcpt_cutoff],
        )
        .map_err(StoreError::from_rusqlite)?;

    // 3. Artefacts / objets par classe Q-12 avec protections strictes
    let mut objects_purged_count = 0usize;
    {
        let mut obj_stmt = tx
            .prepare(
                "SELECT object_hash, object_generation, retention_class, keep_by_policy, created_at_ms
                 FROM object_records
                 WHERE state = 'live'",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = obj_stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        let mut obj_candidates = Vec::new();
        for item in rows {
            obj_candidates.push(item.map_err(StoreError::from_rusqlite)?);
        }
        drop(obj_stmt);

        for (obj_hash, generation, ret_class_s, keep_i64, created_at_ms) in obj_candidates {
            let class = RetentionClass::parse(&ret_class_s);
            let days = i64::from(policy.days_for_class(class));
            let class_cutoff = now_ms
                .as_i64()
                .saturating_sub(days.saturating_mul(86_400_000));
            if created_at_ms > class_cutoff {
                continue;
            }
            if is_object_protected_tx(tx, &obj_hash, keep_i64 != 0)? {
                protected_skipped_count += 1;
                continue;
            }
            tx.execute(
                "UPDATE object_references
                 SET state = 'tombstone', tombstone_reason = 'retention_q12'
                 WHERE object_hash = ?1 AND state = 'live'",
                [&obj_hash],
            )
            .map_err(StoreError::from_rusqlite)?;
            tx_prepare_object_deletion(
                tx,
                config.id_generator.as_ref(),
                now_ms,
                &PrepareObjectDeletionRequest {
                    object_hash: obj_hash.parse()?,
                    expected_generation: generation,
                    reason: format!("retention_q12:{purge_id}"),
                    decision_receipt_id: None,
                },
            )?;
            objects_purged_count += 1;
        }
    }

    // 4. Sauvegardes gérées contenant encore potentiellement ces octets (<= 35 j rotation)
    let mut affected_backups = Vec::new();
    {
        let mut bkp_stmt = tx
            .prepare(
                "SELECT backup_id, retention_tier, destination_path, created_at_ms
                 FROM backup_jobs
                 WHERE state = 'complete'
                 ORDER BY created_at_ms DESC, backup_id ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = bkp_stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        let residual = policy.max_backup_rotation_days.min(35);
        for item in rows {
            let (bid_s, tier_s, dst, cr) = item.map_err(StoreError::from_rusqlite)?;
            let tier = match tier_s.as_str() {
                "daily" => BackupRetentionTier::Daily,
                "weekly" => BackupRetentionTier::Weekly,
                _ => BackupRetentionTier::Manual,
            };
            affected_backups.push(AffectedBackupEntry {
                backup_id: bid_s.parse::<BackupId>()?,
                retention_tier: tier,
                destination_path: dst,
                created_at_ms: TimestampMs::new(cr),
                max_residual_days: residual,
            });
        }
    }

    let limitations = retention_limitations();
    let effective_cutoffs_json = serde_json::json!({
        "events_cutoff_ms": effective_ev_cutoff,
        "command_receipts_min_days": policy.command_receipts_min_days,
        "tool_terminal_logs_days": policy.tool_terminal_logs_days,
        "model_payloads_days": policy.model_payloads_days,
        "browser_artifacts_days": policy.browser_artifacts_days,
        "dirty_captures_days": policy.dirty_captures_days,
        "automation_data_days": policy.automation_data_days,
        "document_copies_days": policy.document_copies_days,
        "export_artifacts_days": policy.export_artifacts_days,
        "logs_telemetry_days": policy.logs_telemetry_days,
    })
    .to_string();
    let backups_json = serde_json::to_string(&affected_backups)?;
    let limits_json = serde_json::to_string(&limitations)?;

    tx.execute(
        "INSERT INTO retention_purges
         (purge_id, policy_version, scope_kind, scope_id, reason, requested_by_actor,
          command_receipt_id, requested_older_than_ms, effective_cutoffs_json,
          events_tombstoned_count, receipts_tombstoned_count, objects_purged_count,
          protected_skipped_count, affected_backups_json, limitations_json,
          wal_checkpoint_truncated, executed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0, ?16)",
        rusqlite::params![
            purge_id,
            policy.policy_version,
            req.scope_kind,
            req.scope_id,
            req.reason,
            req.requested_by_actor,
            receipt_id.map(|r| r.to_string()),
            req.older_than_override_ms.map(|t| t.as_i64()),
            effective_cutoffs_json,
            i64::try_from(events_tombstoned_count).unwrap_or(i64::MAX),
            i64::try_from(receipts_tombstoned_count).unwrap_or(i64::MAX),
            i64::try_from(objects_purged_count).unwrap_or(i64::MAX),
            i64::try_from(protected_skipped_count).unwrap_or(i64::MAX),
            backups_json,
            limits_json,
            now_ms.as_i64(),
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(RetentionPurgeReport {
        purge_id,
        policy_version: policy.policy_version,
        suspended_clock_uncertain: false,
        events_tombstoned_count,
        receipts_tombstoned_count,
        objects_purged_count,
        protected_skipped_count,
        affected_backups,
        limitations,
        wal_checkpoint_truncated: false,
        wal_bytes_after: 0,
        executed_at_ms: now_ms,
    })
}
