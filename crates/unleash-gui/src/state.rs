// SPDX-License-Identifier: Apache-2.0
//! État de présentation et adaptateur IPC de l'hôte Tauri `unleash-gui`.
//!
//! `GuiAppState` ne contient aucune règle métier ni connexion `SQLite` : il adapte uniquement
//! les commandes `WebView` validées vers `unleash_client::UnleashClient`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::ipc::Channel;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use ts_rs::TS;
use unleash_client::{
    ClientConfig, ClientError, CommandCorrelationQueue, EventFollowerConfig, EventSyncUpdate,
    EventSynchronizer, IpcConnection, UnleashClient,
};
use unleash_domain::{RequestId, Revision};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ErrorCode, EventCursor, EventEnvelope, OperationKind,
    OperationName, ProtocolVersion, QueryEnvelope, QueryResponse,
};

use crate::commands::{
    GuiCommandInput, GuiQueryInput, GuiSubscribeInput, validate_webview_payload,
};
use crate::errors::GuiError;
use crate::registry::{ensure_operation_kind, validate_exposed_operation};

/// Lot borné poussé vers la `WebView` via [`tauri::ipc::Channel`].
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuiStreamNotification {
    /// Snapshot initial ou consécutif à `ResyncRequired`.
    SnapshotLoaded {
        /// Watermark transactionnel du snapshot.
        watermark: EventCursor,
        /// Projection JSON retournée par le daemon.
        #[ts(type = "unknown")]
        snapshot: Value,
        /// Motif optionnel de resynchronisation.
        resync_reason: Option<String>,
    },
    /// Lot ordonné et dédupliqué d'événements.
    Events {
        /// Événements du lot.
        events: Vec<EventEnvelope>,
        /// Curseur atteint.
        cursor: EventCursor,
    },
    /// Fermeture ou perte de flux vers le daemon (sans arrêter l'application ni les brouillons).
    Disconnected {
        /// Motif borné de déconnexion du flux.
        reason: String,
    },
}

fn allocate_request_id(seq_counter: &AtomicU64) -> RequestId {
    let seq = seq_counter.fetch_add(1, Ordering::Relaxed);
    RequestId::from_u128(0x6100_0000_0000_0000_0000_0000_0000_0000_u128 | u128::from(seq))
}

fn map_sync_update(update: EventSyncUpdate) -> Result<GuiStreamNotification, GuiError> {
    match update {
        EventSyncUpdate::SnapshotLoaded {
            watermark,
            snapshot,
            resync_reason,
        } => Ok(GuiStreamNotification::SnapshotLoaded {
            watermark,
            snapshot,
            resync_reason,
        }),
        EventSyncUpdate::Events { events, cursor } => {
            Ok(GuiStreamNotification::Events { events, cursor })
        }
        _ => Err(GuiError::Tauri(String::from(
            "Variante EventSyncUpdate inattendue",
        ))),
    }
}

/// État partagé de l'hôte Tauri `unleash-gui` (présentation et adaptateur IPC uniquement).
#[derive(Debug)]
pub struct GuiAppState {
    client_config: RwLock<Option<ClientConfig>>,
    client: Mutex<Option<UnleashClient>>,
    correlation_queue: Mutex<CommandCorrelationQueue>,
    stream_task: std::sync::Mutex<Option<JoinHandle<()>>>,
    next_seq: Arc<AtomicU64>,
}

impl Drop for GuiAppState {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.stream_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }
}

impl GuiAppState {
    /// Crée un état de présentation vierge avec ou sans configuration IPC daemon initiale.
    #[must_use]
    pub fn new(client_config: Option<ClientConfig>) -> Self {
        Self::with_correlation_queue(client_config, CommandCorrelationQueue::in_memory())
    }

    /// Crée un état de présentation avec une file de corrélation persistable injectée.
    #[must_use]
    pub fn with_correlation_queue(
        client_config: Option<ClientConfig>,
        correlation_queue: CommandCorrelationQueue,
    ) -> Self {
        let seed = current_unix_ms_nonneg();
        Self {
            client_config: RwLock::new(client_config),
            client: Mutex::new(None),
            correlation_queue: Mutex::new(correlation_queue),
            stream_task: std::sync::Mutex::new(None),
            next_seq: Arc::new(AtomicU64::new(seed.saturating_mul(1024).saturating_add(1))),
        }
    }

    fn stop_active_stream(&self) {
        if let Ok(mut guard) = self.stream_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }

    /// Configure (ou remplace) la configuration IPC du daemon cible.
    pub async fn set_client_config(&self, config: ClientConfig) {
        self.stop_active_stream();
        {
            let mut cfg_guard = self.client_config.write().await;
            *cfg_guard = Some(config);
        }
        let mut client_guard = self.client.lock().await;
        *client_guard = None;
    }

    fn next_request_id(&self) -> RequestId {
        allocate_request_id(&self.next_seq)
    }

    async fn ensure_connected_client<'a>(
        &'a self,
        client_guard: &'a mut Option<UnleashClient>,
    ) -> Result<&'a mut UnleashClient, GuiError> {
        if client_guard.is_none() {
            let cfg = {
                let guard = self.client_config.read().await;
                guard.clone().ok_or(GuiError::ClientNotConnected)?
            };
            let connected = UnleashClient::connect(cfg).await?;
            *client_guard = Some(connected);
        }
        client_guard.as_mut().ok_or(GuiError::ClientNotConnected)
    }

    /// Exécute une opération de lecture du catalogue exposée à la GUI via `unleash-client`.
    ///
    /// # Sûreté à l'annulation
    /// Lecture pure sans effet durable côté daemon.
    ///
    /// # Errors
    /// Retourne [`GuiError`] si l'opération est interdite, invalide ou si le client IPC échoue.
    pub async fn execute_query(
        &self,
        op: OperationName,
        input: GuiQueryInput,
    ) -> Result<QueryResponse, GuiError> {
        let verified = validate_exposed_operation(op.as_str())?;
        ensure_operation_kind(verified, OperationKind::Query)?;
        validate_webview_payload(&input.payload)?;

        let envelope = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: self.next_request_id(),
            query_type: verified.as_str().to_owned(),
            project_id: input.project_id,
            payload: input.payload,
        };

        let mut client_guard = self.client.lock().await;
        let client = self.ensure_connected_client(&mut client_guard).await?;
        let res = client.query(&envelope).await;
        if matches!(res, Err(unleash_client::ClientError::Disconnected { .. })) {
            tracing::warn!(
                operation = verified.as_str(),
                "Connexion IPC GUI perdue lors de la lecture"
            );
            *client_guard = None;
        } else {
            tracing::debug!(operation = verified.as_str(), "Lecture GUI exécutée");
        }
        res.map_err(GuiError::from)
    }

    /// Soumet une commande mutante exposée via la file de corrélation de `unleash-client`.
    ///
    /// L'identité `actor_ref` est toujours laissée à `None` afin que le daemon l'extraie du pair OS.
    ///
    /// # Sûreté à l'annulation
    /// En cas d'interruption après enregistrement dans `correlation_queue`, `command_id` et
    /// `authority_generation` restent épinglés pour réconciliation.
    ///
    /// # Errors
    /// Retourne [`GuiError`] en cas de rejet de frontière, de lignée obsolète ou d'erreur IPC.
    pub async fn submit_command(
        &self,
        op: OperationName,
        input: GuiCommandInput,
    ) -> Result<CommandReceipt, GuiError> {
        let verified = validate_exposed_operation(op.as_str())?;
        ensure_operation_kind(verified, OperationKind::Command)?;
        validate_webview_payload(&input.payload)?;

        let now_ms = i64::try_from(current_unix_ms_nonneg()).unwrap_or(i64::MAX);
        let fresh_request_id = self.next_request_id();

        let mut client_guard = self.client.lock().await;
        let client = self.ensure_connected_client(&mut client_guard).await?;
        let authority_generation = client.hello().authority_generation;

        let envelope = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: fresh_request_id,
            command_id: input.command_id,
            authority_generation,
            actor_ref: None,
            project_id: input.project_id,
            aggregate_id: input.aggregate_id,
            expected_revision: Revision::new(input.expected_revision),
            command_type: verified.as_str().to_owned(),
            payload: input.payload,
        };

        let mut queue_guard = self.correlation_queue.lock().await;
        queue_guard.enqueue(envelope, now_ms)?;
        let res = client
            .submit_correlated(&mut queue_guard, input.command_id, fresh_request_id, now_ms)
            .await;
        drop(queue_guard);

        if matches!(res, Err(unleash_client::ClientError::Disconnected { .. })) {
            tracing::warn!(
                operation = verified.as_str(),
                command_id = %input.command_id,
                "Connexion IPC GUI perdue pendant soumission"
            );
            *client_guard = None;
        } else {
            tracing::debug!(
                operation = verified.as_str(),
                command_id = %input.command_id,
                "Commande GUI transmise via file de corrélation"
            );
        }
        res.map_err(GuiError::from)
    }

    /// Synchronise un snapshot initial + watermark puis lance une tâche continue consommant
    /// le flux `SubscribeEvents` (y compris resynchronisations `ResyncRequired`) vers le
    /// [`tauri::ipc::Channel`] jusqu'à fermeture du canal ou déconnexion.
    ///
    /// # Sûreté à l'annulation
    /// Annule tout abonnement précédent avant d'en démarrer un nouveau, et la tâche spawnée est
    /// annulée automatiquement lors du [`Drop`] de [`GuiAppState`].
    ///
    /// # Errors
    /// Retourne [`GuiError`] si la connexion IPC ou l'envoi initial sur le canal Tauri échoue.
    pub async fn subscribe_events(
        &self,
        input: GuiSubscribeInput,
        channel: Channel<GuiStreamNotification>,
    ) -> Result<EventCursor, GuiError> {
        self.stop_active_stream();

        let cfg = {
            let guard = self.client_config.read().await;
            guard.clone().ok_or(GuiError::ClientNotConnected)?
        };

        let batch_limit = input.batch_limit.unwrap_or(128).clamp(1, 200);
        let follower = EventFollowerConfig {
            project_id: input.project_id,
            aggregate_id: input.aggregate_id,
            batch_limit,
            snapshot_query_payload: Map::new(),
        };

        let mut sync = EventSynchronizer::new(cfg.clone(), follower);
        let req_id = self.next_request_id();
        let (initial, mut reader, cursor) = {
            let mut client_guard = self.client.lock().await;
            let client = self.ensure_connected_client(&mut client_guard).await?;
            let (initial, reader) = sync.sync_initial(client.connection_mut(), req_id).await?;
            let cursor = sync.current_cursor().ok_or(GuiError::ClientNotConnected)?;
            (initial, reader, cursor)
        };

        channel
            .send(map_sync_update(initial)?)
            .map_err(|e| GuiError::Tauri(e.to_string()))?;

        tracing::info!(
            stream_id = %cursor.stream_id,
            sequence = cursor.sequence.as_u64(),
            "Abonnement continu SubscribeEvents GUI initialisé"
        );

        let seq_counter = Arc::clone(&self.next_seq);
        let handle = tokio::spawn(async move {
            let mut resync_conn: Option<IpcConnection> = None;
            loop {
                if resync_conn.is_none() {
                    match IpcConnection::connect(&cfg).await {
                        Ok(conn) => resync_conn = Some(conn),
                        Err(err) => {
                            tracing::warn!(error = %err, "Échec reconnexion canal resync GUI");
                            let _ = channel.send(GuiStreamNotification::Disconnected {
                                reason: err.to_string(),
                            });
                            break;
                        }
                    }
                }
                let Some(q_conn) = resync_conn.as_mut() else {
                    break;
                };
                let step = sync
                    .poll_next(&mut reader, q_conn, || allocate_request_id(&seq_counter))
                    .await;
                match step {
                    Ok((update, maybe_new_reader)) => {
                        if let Some(new_reader) = maybe_new_reader {
                            reader = new_reader;
                        }
                        match map_sync_update(update) {
                            Ok(notif) => {
                                if channel.send(notif).is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                tracing::warn!(error = %err, "Erreur mapping lot SubscribeEvents GUI");
                                let _ = channel.send(GuiStreamNotification::Disconnected {
                                    reason: err.to_string(),
                                });
                                break;
                            }
                        }
                    }
                    Err(ClientError::Remote(ipc)) if ipc.code == ErrorCode::ResyncRequired => {
                        tracing::info!("ResyncRequired reçu sur le flux GUI, réouverture canal");
                        resync_conn = None;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "Flux SubscribeEvents GUI interrompu");
                        let _ = channel.send(GuiStreamNotification::Disconnected {
                            reason: err.to_string(),
                        });
                        break;
                    }
                }
            }
        });

        if let Ok(mut guard) = self.stream_task.lock() {
            *guard = Some(handle);
        } else {
            handle.abort();
        }

        Ok(cursor)
    }
}

fn current_unix_ms_nonneg() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    u64::try_from(millis).unwrap_or(u64::MAX)
}
