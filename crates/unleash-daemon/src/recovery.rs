// SPDX-License-Identifier: Apache-2.0
//! Barrière de démarrage déterministe et classification de récupération (`starting -> recovering -> ready`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::TransactionBehavior;
use tokio::sync::Notify;
use unleash_domain::{DaemonEpoch, DaemonInstanceId, DaemonMode, Revision, Sequence, TimestampMs};
use unleash_platform::{ProcessVerification, inspect_process_identity, verify_process_identity};
use unleash_protocol::{
    EventCausation, EventCausationKind, EventEnvelope, Sensitivity, canonical_sha256,
};
use unleash_store::{
    DaemonStateRow, UnleashStore, WriteCriticality, ensure_global_execution_pool_tx,
};
use uuid::Uuid;

use crate::error::DaemonError;

/// Barrière déterministe injectable permettant aux tests d'observer le daemon en mode `recovering`
/// sans aucun `sleep` de synchronisation (`T-F001-006`).
#[derive(Debug, Default)]
pub struct RecoveryGate {
    entered: AtomicBool,
    released: AtomicBool,
    entered_notify: Notify,
    release_notify: Notify,
}

impl RecoveryGate {
    /// Crée une barrière fermée par défaut jusqu'à l'appel à [`Self::release`].
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Signale que le daemon est entré dans le mode `recovering` et attend la libération.
    pub async fn enter_and_wait(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_waiters();
        while !self.released.load(Ordering::Acquire) {
            self.release_notify.notified().await;
        }
    }

    /// Attend que le daemon ait atteint l'état `recovering`.
    pub async fn wait_until_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    /// Libère la barrière afin que la classification de récupération se termine vers `ready`.
    pub fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.release_notify.notify_waiters();
    }
}

/// Fait entrer atomiquement le daemon en mode `recovering`, incrémente `daemon_epoch` si
/// ce n'est pas le tout premier démarrage, crée un `daemon_instance_id` et journalise `DaemonRecoveryStarted`.
///
/// # Errors
/// Retourne [`DaemonError::Store`] si la transaction échoue.
pub fn enter_recovering_and_emit_event(
    store: &UnleashStore,
    boot_identity: &str,
    new_instance_id: DaemonInstanceId,
) -> Result<(DaemonStateRow, DaemonMode, EventEnvelope), DaemonError> {
    let boot_owned = boot_identity.to_owned();
    let id_gen = Arc::clone(&store.config().id_generator);
    let event_uuid = id_gen.new_uuid();

    let (row, prev_mode, env) =
        store.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now_ms| {
            let mut row = read_daemon_state_row_tx(tx)?;
            let prior_recovery_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM events WHERE event_type = 'DaemonRecoveryStarted'",
                [],
                |r| r.get(0),
            )?;

            let prev_mode = row.mode;
            let next_epoch = if prior_recovery_count > 0 {
                row.daemon_epoch.next()
            } else {
                row.daemon_epoch
            };
            let next_rev = row.revision.next();

            tx.execute(
                "UPDATE daemon_state
                 SET daemon_epoch = ?1,
                     daemon_instance_id = ?2,
                     boot_identity = ?3,
                     mode = 'recovering',
                     revision = ?4,
                     started_at_ms = ?5
                 WHERE singleton = 1",
                rusqlite::params![
                    i64::try_from(next_epoch.as_u64()).unwrap_or(1),
                    new_instance_id.to_string(),
                    boot_owned,
                    i64::try_from(next_rev.as_u64()).unwrap_or(1),
                    now_ms.as_i64(),
                ],
            )?;

            let mut payload = serde_json::Map::new();
            payload.insert(
                String::from("daemon_epoch"),
                serde_json::Value::from(next_epoch.as_u64()),
            );
            payload.insert(
                String::from("daemon_instance_id"),
                serde_json::Value::String(new_instance_id.to_string()),
            );
            payload.insert(
                String::from("previous_mode"),
                serde_json::Value::String(prev_mode.to_string()),
            );

            let env = insert_system_event_tx(
                tx,
                event_uuid,
                row.stream_id,
                "daemon",
                row.authority_generation.as_uuid(),
                next_rev,
                0,
                "DaemonRecoveryStarted",
                None,
                &format!("uid:{}", row.owner_uid),
                event_uuid,
                EventCausationKind::External,
                &new_instance_id.to_string(),
                payload,
                Sensitivity::Public,
                now_ms,
            )?;

            row.daemon_epoch = next_epoch;
            row.daemon_instance_id = new_instance_id;
            row.boot_identity = boot_owned;
            row.mode = DaemonMode::Recovering;
            row.revision = next_rev;
            row.started_at_ms = now_ms;

            Ok((row, prev_mode, env))
        })?;

    Ok((row, prev_mode, env))
}

/// Classifie les baux, actions en vol, invocations et processus après redémarrage puis
/// fait transiter le daemon de `recovering` vers `ready` (ou `read_only_recovery` si requis).
///
/// # Errors
/// Retourne [`DaemonError`] en cas d'échec de réconciliation ou de transition domaine.
#[expect(
    clippy::too_many_lines,
    reason = "Transaction atomique de classification des ressources et transition DaemonReady"
)]
pub fn classify_and_finish_recovery(
    store: &UnleashStore,
    previous_mode: DaemonMode,
    started_event_id: Uuid,
) -> Result<(DaemonStateRow, Vec<EventEnvelope>), DaemonError> {
    let config = store.config().clone();
    let id_gen = Arc::clone(&store.config().id_generator);
    let ready_event_id = id_gen.new_uuid();

    // Vérifier les processus vivants avant d'ouvrir la transaction writer SQLite (évite I/O dans tx).
    let process_verdicts = inspect_live_processes_read_only(store)?;

    let (row, emitted) = store.execute_in_writer_tx(
        WriteCriticality::Critical,
        move |tx, now_ms| {
            let mut row = read_daemon_state_row_tx(tx)?;
            let mut emitted_events = Vec::new();
            let auth_str = row.authority_generation.to_string();

            // 1. Marquer les baux actifs d'époques précédentes comme suspects
            tx.execute(
                "UPDATE resource_leases SET state = 'suspect'
                 WHERE authority_generation = ?1 AND state = 'held'",
                [&auth_str],
            )?;

            // 2. Classer les actions en vol / dispatch / cancel_requested en outcome_unknown
            let mut stmt = tx.prepare(
                "SELECT action_id, revision, run_id FROM actions
                 WHERE authority_generation = ?1
                   AND state IN ('dispatching', 'in_flight', 'cancel_requested')",
            )?;
            let rows = stmt.query_map([&auth_str], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?;
            let mut inflight_actions = Vec::new();
            for item in rows {
                inflight_actions.push(item?);
            }
            drop(stmt);

            for (act_id_str, act_rev, _run_id) in inflight_actions {
                let next_act_rev = act_rev.saturating_add(1);
                tx.execute(
                    "UPDATE actions SET state = 'outcome_unknown', revision = ?1 WHERE action_id = ?2",
                    rusqlite::params![next_act_rev, act_id_str],
                )?;
                tx.execute(
                    "UPDATE budget_reservations SET state = 'unknown'
                     WHERE action_id = ?1 AND state = 'held'",
                    [&act_id_str],
                )?;
                if let Ok(act_uuid) = act_id_str.parse::<Uuid>() {
                    let ev_id = id_gen.new_uuid();
                    let mut p = serde_json::Map::new();
                    p.insert(
                        String::from("action_id"),
                        serde_json::Value::String(act_id_str),
                    );
                    p.insert(
                        String::from("reason"),
                        serde_json::Value::String(String::from("daemon_recovery_classification")),
                    );
                    let env = insert_system_event_tx(
                        tx,
                        ev_id,
                        row.stream_id,
                        "action",
                        act_uuid,
                        Revision::new(u64::try_from(next_act_rev).unwrap_or(1)),
                        0,
                        "ActionOutcomeUnknown",
                        None,
                        &format!("uid:{}", row.owner_uid),
                        started_event_id,
                        EventCausationKind::Event,
                        &started_event_id.to_string(),
                        p,
                        Sensitivity::Project,
                        now_ms,
                    )?;
                    emitted_events.push(env);
                }
            }

            // 3. Invocations actives -> reconciling (sans nouvelle admission)
            tx.execute(
                "UPDATE invocations SET state = 'reconciling', revision = revision + 1
                 WHERE state IN ('admitted', 'running', 'checkpointing')",
                [],
            )?;

            // 4. Mettre à jour l'état des processus observés
            for (proc_id, new_state) in &process_verdicts {
                tx.execute(
                    "UPDATE processes SET state = ?1, last_observed_at_ms = ?2 WHERE process_id = ?3",
                    rusqlite::params![new_state, now_ms.as_i64(), proc_id],
                )?;
            }

            let target_mode = if previous_mode == DaemonMode::ReadOnlyRecovery {
                DaemonMode::ReadOnlyRecovery
            } else {
                DaemonMode::Ready
            };
            let next_mode = unleash_domain::transition_daemon_mode(row.mode, target_mode)?;
            let next_rev = row.revision.next();
            if next_mode == DaemonMode::Ready {
                emitted_events.extend(
                    unleash_store::terminal_allocations::recover_undispatched_terminal_releases_tx(
                        tx, &config, now_ms,
                    )?,
                );
                ensure_global_execution_pool_tx(tx, id_gen.as_ref())?;
            }

            tx.execute(
                "UPDATE daemon_state SET mode = ?1, revision = ?2 WHERE singleton = 1",
                rusqlite::params![
                    next_mode.to_string(),
                    i64::try_from(next_rev.as_u64()).unwrap_or(1)
                ],
            )?;

            let mut ready_payload = serde_json::Map::new();
            ready_payload.insert(
                String::from("mode"),
                serde_json::Value::String(next_mode.to_string()),
            );
            ready_payload.insert(
                String::from("daemon_epoch"),
                serde_json::Value::from(row.daemon_epoch.as_u64()),
            );
            ready_payload.insert(
                String::from("daemon_instance_id"),
                serde_json::Value::String(row.daemon_instance_id.to_string()),
            );

            let ready_ev = insert_system_event_tx(
                tx,
                ready_event_id,
                row.stream_id,
                "daemon",
                row.authority_generation.as_uuid(),
                next_rev,
                0,
                "DaemonReady",
                None,
                &format!("uid:{}", row.owner_uid),
                started_event_id,
                EventCausationKind::Event,
                &started_event_id.to_string(),
                ready_payload,
                Sensitivity::Public,
                now_ms,
            )?;
            emitted_events.push(ready_ev);

            row.mode = next_mode;
            row.revision = next_rev;
            Ok((row, emitted_events))
        },
    )?;

    // Réconcilier les pins et suppressions d'objets interrompus
    let _obj_report = store.reconcile_objects_and_deletions()?;

    Ok((row, emitted))
}

fn inspect_live_processes_read_only(
    store: &UnleashStore,
) -> Result<Vec<(String, &'static str)>, DaemonError> {
    let entries = store.read_snapshot(|conn, _ctx| {
        let mut stmt = conn.prepare(
            "SELECT process_id, pid, boot_identity, start_identity, owner_uid
             FROM processes WHERE state = 'live'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })?;

    let mut verdicts = Vec::with_capacity(entries.len());
    for (proc_id, pid_opt, boot_opt, start_opt, owner_uid_i64) in entries {
        let state = match (pid_opt, boot_opt, start_opt) {
            (Some(pid_i64), Some(boot), Some(start)) if pid_i64 > 0 => {
                let pid = u32::try_from(pid_i64).unwrap_or(0);
                match inspect_process_identity(pid) {
                    Ok(observed) => {
                        let expected_uid = u32::try_from(owner_uid_i64).unwrap_or(0);
                        let expected = unleash_platform::OsProcessIdentity {
                            pid,
                            pgid: observed.pgid,
                            owner_uid: expected_uid,
                            boot_identity: unleash_platform::BootIdentity::new(boot)
                                .unwrap_or_else(|_| observed.boot_identity.clone()),
                            start_identity: unleash_platform::StartIdentity::new(start)
                                .unwrap_or_else(|_| observed.start_identity.clone()),
                            backend: observed.backend,
                        };
                        match verify_process_identity(&expected) {
                            Ok(ProcessVerification::Exited | ProcessVerification::BootChanged) => {
                                "lost"
                            }
                            Ok(ProcessVerification::IdentityMismatch { .. }) => "identity_mismatch",
                            Ok(_) | Err(_) => "suspect",
                        }
                    }
                    Err(_) => "lost",
                }
            }
            _ => "suspect",
        };
        verdicts.push((proc_id, state));
    }
    Ok(verdicts)
}

pub(crate) fn read_daemon_state_row_tx(
    tx: &rusqlite::Transaction<'_>,
) -> Result<DaemonStateRow, unleash_store::StoreError> {
    tx.query_row(
        "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                stream_id, schema_version, mode, revision, started_at_ms
         FROM daemon_state WHERE singleton = 1",
        [],
        |r| {
            let owner_uid_i64: i64 = r.get(0)?;
            let auth_str: String = r.get(1)?;
            let epoch_i64: i64 = r.get(2)?;
            let inst_str: String = r.get(3)?;
            let boot_id: String = r.get(4)?;
            let stream_str: String = r.get(5)?;
            let schema_ver: i64 = r.get(6)?;
            let mode_str: String = r.get(7)?;
            let rev_i64: i64 = r.get(8)?;
            let started_i64: i64 = r.get(9)?;

            let auth = auth_str
                .parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let inst = inst_str
                .parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let stream_id = stream_str
                .parse()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let mode = match mode_str.as_str() {
                "starting" => DaemonMode::Starting,
                "recovering" => DaemonMode::Recovering,
                "ready" => DaemonMode::Ready,
                "degraded" => DaemonMode::Degraded,
                "read_only_recovery" => DaemonMode::ReadOnlyRecovery,
                "draining" => DaemonMode::Draining,
                "stopped" => DaemonMode::Stopped,
                _ => {
                    return Err(rusqlite::Error::InvalidColumnType(
                        7,
                        String::from("mode"),
                        rusqlite::types::Type::Text,
                    ));
                }
            };

            Ok(DaemonStateRow {
                owner_uid: u32::try_from(owner_uid_i64).unwrap_or(0),
                authority_generation: auth,
                daemon_epoch: DaemonEpoch::new(u64::try_from(epoch_i64).unwrap_or(1)),
                daemon_instance_id: inst,
                boot_identity: boot_id,
                stream_id,
                schema_version: schema_ver,
                mode,
                revision: Revision::new(u64::try_from(rev_i64).unwrap_or(0)),
                started_at_ms: TimestampMs::new(started_i64),
            })
        },
    )
    .map_err(unleash_store::StoreError::from_rusqlite)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Insertion directe dans events satisfaisant toutes les colonnes STRICT du schéma"
)]
pub(crate) fn insert_system_event_tx(
    tx: &rusqlite::Transaction<'_>,
    event_id: Uuid,
    stream_id: unleash_domain::StreamId,
    aggregate_type: &str,
    aggregate_id: Uuid,
    aggregate_revision: Revision,
    event_ordinal: i64,
    event_type: &str,
    project_id: Option<unleash_domain::ProjectId>,
    actor_ref: &str,
    correlation_id: Uuid,
    causation_kind: EventCausationKind,
    causation_id: &str,
    payload: serde_json::Map<String, serde_json::Value>,
    sensitivity: Sensitivity,
    now_ms: TimestampMs,
) -> Result<EventEnvelope, unleash_store::StoreError> {
    let payload_json = serde_json::to_string(&payload)?;
    let payload_hash = canonical_sha256(&payload)?;
    let caus_kind_str = match causation_kind {
        EventCausationKind::Event => "event",
        EventCausationKind::Action => "action",
        EventCausationKind::Timer => "timer",
        EventCausationKind::External | EventCausationKind::Command | _ => "external",
    };
    let sens_str = match sensitivity {
        Sensitivity::Public => "public",
        Sensitivity::Project => "project",
        Sensitivity::Confidential => "confidential",
        Sensitivity::Restricted | _ => "restricted",
    };

    tx.execute(
        "INSERT INTO events
         (event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal,
          event_type, schema_version, project_id, actor_ref, actor_id, occurred_at_ms, recorded_at_ms,
          payload_json, payload_state, payload_hash, sensitivity, command_actor_id, command_id,
          causation_kind, causation_id, causation_receipt_id, correlation_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?9, ?10, ?10, ?11, 'full', ?12, ?13, NULL, NULL, ?14, ?15, NULL, ?16)",
        rusqlite::params![
            event_id.to_string(),
            stream_id.to_string(),
            aggregate_type,
            aggregate_id.to_string(),
            i64::try_from(aggregate_revision.as_u64()).unwrap_or(1),
            event_ordinal,
            event_type,
            project_id.map(|p| p.to_string()),
            actor_ref,
            now_ms.as_i64(),
            payload_json,
            payload_hash.as_prefixed_str(),
            sens_str,
            caus_kind_str,
            causation_id,
            correlation_id.to_string(),
        ],
    )?;

    let seq = u64::try_from(tx.last_insert_rowid()).unwrap_or(1);
    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms.as_i64())
        .unwrap_or(chrono::DateTime::UNIX_EPOCH);

    Ok(EventEnvelope {
        event_id,
        stream_id,
        sequence: Sequence::new(seq),
        event_type: event_type.to_owned(),
        schema_version: 1,
        occurred_at: ts,
        recorded_at: ts,
        project_id,
        aggregate_id,
        aggregate_revision,
        actor_ref: actor_ref.to_owned(),
        correlation_id,
        causation: EventCausation {
            kind: causation_kind,
            id: causation_id.to_owned(),
        },
        command_id: None,
        payload_hash,
        payload,
        sensitivity,
    })
}

/// Vérifie s'il reste des actions en vol non résolues (distinctes de `outcome_unknown` déjà classifiées).
///
/// # Errors
/// Retourne [`DaemonError`] en cas d'erreur de lecture.
pub fn has_unresolved_inflight_actions(store: &UnleashStore) -> Result<Vec<String>, DaemonError> {
    let rows = store.read_snapshot(|conn, _ctx| {
        let mut stmt = conn.prepare(
            "SELECT action_id FROM actions WHERE state IN ('dispatching', 'in_flight', 'cancel_requested')",
        )?;
        let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for item in iter {
            out.push(item?);
        }
        Ok(out)
    })?;
    Ok(rows)
}

#[allow(
    dead_code,
    reason = "TransactionBehavior import kept for clarity if extended"
)]
const _: TransactionBehavior = TransactionBehavior::Immediate;
