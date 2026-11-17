// SPDX-License-Identifier: Apache-2.0
//! Abonnement aux événements `SubscribeEvents` avec reprise `(stream_id, sequence)`,
//! déduplication at-least-once bornée et rechargement atomique snapshot+watermark sur `ResyncRequired`
//! (`REQ-CORE-010`, `AC-F001-010`, `ipc-and-apis.md`).

use std::collections::{HashSet, VecDeque};

use serde_json::{Map, Value};
use unleash_domain::{ProjectId, RequestId, StreamId};
use unleash_protocol::{
    ErrorCode, EventCursor, EventEnvelope, ProtocolVersion, QueryEnvelope, QueryResponse,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use uuid::Uuid;

use crate::errors::ClientError;
use crate::transport::{ClientConfig, IpcConnection};

const DEFAULT_BATCH_LIMIT: u32 = 128;
const MAX_DEDUP_WINDOW: usize = 4096;

/// Lecteur dédié d'un flux d'abonnement `SubscribeEvents`.
#[derive(Debug)]
pub struct EventStreamReader {
    conn: IpcConnection,
}

impl EventStreamReader {
    /// Ouvre une nouvelle connexion authentifiée `Hello` puis émet `SubscribeEventsRequest`.
    ///
    /// # Sûreté à l'annulation
    /// Annuler cette future ferme la socket de souscription sans impacter d'autres connexions.
    ///
    /// # Errors
    /// Retourne [`ClientError`] si la connexion, le handshake ou l'envoi de la requête échoue.
    pub async fn subscribe(
        config: &ClientConfig,
        request: &SubscribeEventsRequest,
    ) -> Result<Self, ClientError> {
        let mut conn = IpcConnection::connect(config).await?;
        conn.send_json_frame(request, "SubscribeEventsRequest")
            .await?;
        Ok(Self { conn })
    }

    /// Lit la trame suivante du flux d'événements (`Events` ou `ResyncRequired`).
    ///
    /// Retourne `Ok(None)` lorsque le serveur ferme proprement le flux.
    ///
    /// # Sûreté à l'annulation
    /// Si annulée pendant la réception partielle d'une trame, la connexion sous-jacente doit
    /// être recréée via une nouvelle souscription depuis le dernier curseur durable.
    ///
    /// # Errors
    /// Retourne [`ClientError::Remote`] si le serveur émet un [`unleash_protocol::IpcError`]
    /// (par exemple `ResyncRequired`), ou [`ClientError::Protocol`] sur JSON invalide/trame > 8 MiB.
    pub async fn next_frame(&mut self) -> Result<Option<SubscribeEventsFrame>, ClientError> {
        let Some(raw) = self
            .conn
            .read_raw_frame_unbounded_wait("SubscribeEventsFrame")
            .await?
        else {
            return Ok(None);
        };
        crate::transport::decode_response_or_ipc_error(&raw, "SubscribeEventsFrame").map(Some)
    }
}

/// Filtre et paramètres de suivi d'un [`EventSynchronizer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFollowerConfig {
    /// Filtre projet optionnel.
    pub project_id: Option<ProjectId>,
    /// Filtre agrégat optionnel.
    pub aggregate_id: Option<Uuid>,
    /// Taille maximale d'un lot d'événements demandé au daemon (`1..=10000`).
    pub batch_limit: u32,
    /// Payload typé transmis à `GetStateSnapshot` lors de l'initialisation ou de la resynchronisation.
    pub snapshot_query_payload: Map<String, Value>,
}

impl Default for EventFollowerConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            aggregate_id: None,
            batch_limit: DEFAULT_BATCH_LIMIT,
            snapshot_query_payload: Map::new(),
        }
    }
}

/// Événement produit par [`EventSynchronizer`] après déduplication et gestion atomique des
/// bascules `ResyncRequired`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventSyncUpdate {
    /// Snapshot complet et son watermark transactionnel chargés atomiquement.
    SnapshotLoaded {
        /// Watermark transactionnel exact du snapshot.
        watermark: EventCursor,
        /// Projection d'état retournée par `GetStateSnapshot`.
        snapshot: Value,
        /// Motif fourni par le daemon si déclenché par `ResyncRequired`.
        resync_reason: Option<String>,
    },
    /// Lot ordonné d'événements dédupliqués strictement postérieurs au watermark courant.
    Events {
        /// Événements filtrés sans doublon.
        events: Vec<EventEnvelope>,
        /// Nouveau curseur atteint dans le flux actif.
        cursor: EventCursor,
    },
}

/// Synchroniseur d'état client garantissant reprise `(stream_id, sequence)`, déduplication
/// at-least-once bornée et rechargement atomique snapshot+watermark sur `ResyncRequired`.
#[derive(Debug)]
pub struct EventSynchronizer {
    config: ClientConfig,
    follower: EventFollowerConfig,
    cursor: Option<EventCursor>,
    seen_ids: HashSet<Uuid>,
    seen_order: VecDeque<Uuid>,
}

impl EventSynchronizer {
    /// Construit un nouveau synchroniseur pour la configuration IPC et le filtre donnés.
    #[must_use]
    pub fn new(config: ClientConfig, follower: EventFollowerConfig) -> Self {
        Self {
            config,
            follower,
            cursor: None,
            seen_ids: HashSet::new(),
            seen_order: VecDeque::new(),
        }
    }

    /// Curseur courant connu du client (watermark initial ou dernier lot reçu).
    #[must_use]
    pub const fn current_cursor(&self) -> Option<EventCursor> {
        self.cursor
    }

    /// Charge un `GetStateSnapshot` initial avec son watermark transactionnel puis ouvre le
    /// flux `SubscribeEvents` à partir de `(watermark.stream_id, watermark.sequence)`.
    ///
    /// # Sûreté à l'annulation
    /// N'avance le curseur interne qu'après réception réussie du snapshot et ouverture du flux.
    ///
    /// # Errors
    /// Retourne [`ClientError::MissingSnapshotWatermark`] si le serveur ne renvoie pas de
    /// watermark, ou toute erreur de transport/protocole.
    pub async fn sync_initial(
        &mut self,
        query_conn: &mut IpcConnection,
        request_id: RequestId,
    ) -> Result<(EventSyncUpdate, EventStreamReader), ClientError> {
        let (snapshot, watermark) = self
            .fetch_snapshot_with_watermark(query_conn, request_id)
            .await?;
        self.reset_cursor_atomically(watermark);
        let reader = self.open_stream_at(watermark).await?;
        Ok((
            EventSyncUpdate::SnapshotLoaded {
                watermark,
                snapshot,
                resync_reason: None,
            },
            reader,
        ))
    }

    /// Consomme le prochain lot du flux d'abonnement `stream` ou exécute une resynchronisation
    /// atomique `GetStateSnapshot` + réabonnement dès qu'un `ResyncRequired` survient.
    ///
    /// Lorsque une resynchronisation crée un nouveau [`EventStreamReader`], celui-ci est retourné
    /// en `Some(new_stream)` et doit remplacer `stream` pour les appels suivants.
    ///
    /// # Sûreté à l'annulation
    /// Le curseur local n'est mis à jour qu'une fois les données valides décodées (ou après le
    /// nouveau `GetStateSnapshot` complet lors d'un `ResyncRequired`).
    ///
    /// # Errors
    /// Retourne [`ClientError::Disconnected`] si le flux se ferme sans `ResyncRequired`, ou
    /// [`ClientError::MissingSnapshotWatermark`] / erreurs réseau et protocole.
    pub async fn poll_next<F>(
        &mut self,
        stream: &mut EventStreamReader,
        query_conn: &mut IpcConnection,
        mut fresh_request_id: F,
    ) -> Result<(EventSyncUpdate, Option<EventStreamReader>), ClientError>
    where
        F: FnMut() -> RequestId,
    {
        match stream.next_frame().await {
            Ok(Some(SubscribeEventsFrame::Events { events, cursor })) => {
                let filtered = self.filter_and_advance(events, cursor);
                Ok((
                    EventSyncUpdate::Events {
                        events: filtered,
                        cursor,
                    },
                    None,
                ))
            }
            Ok(Some(SubscribeEventsFrame::ResyncRequired { reason, .. })) => {
                self.perform_atomic_resync(query_conn, fresh_request_id(), reason)
                    .await
            }
            Ok(Some(_)) => Err(ClientError::UnexpectedResponse {
                expected: "SubscribeEventsFrame",
                detail: String::from("Variante de trame SubscribeEventsFrame inconnue"),
            }),
            Ok(None) => Err(ClientError::Disconnected {
                operation: "SubscribeEvents",
            }),
            Err(ClientError::Remote(ipc_err)) if ipc_err.code == ErrorCode::ResyncRequired => {
                self.perform_atomic_resync(query_conn, fresh_request_id(), ipc_err.message)
                    .await
            }
            Err(other) => Err(other),
        }
    }

    async fn perform_atomic_resync(
        &mut self,
        query_conn: &mut IpcConnection,
        request_id: RequestId,
        reason: String,
    ) -> Result<(EventSyncUpdate, Option<EventStreamReader>), ClientError> {
        let (snapshot, new_watermark) = self
            .fetch_snapshot_with_watermark(query_conn, request_id)
            .await?;
        self.reset_cursor_atomically(new_watermark);
        let new_reader = self.open_stream_at(new_watermark).await?;
        Ok((
            EventSyncUpdate::SnapshotLoaded {
                watermark: new_watermark,
                snapshot,
                resync_reason: Some(reason),
            },
            Some(new_reader),
        ))
    }

    async fn fetch_snapshot_with_watermark(
        &self,
        query_conn: &mut IpcConnection,
        request_id: RequestId,
    ) -> Result<(Value, EventCursor), ClientError> {
        let query = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id,
            query_type: String::from("GetStateSnapshot"),
            project_id: self.follower.project_id,
            payload: self.follower.snapshot_query_payload.clone(),
        };
        query_conn
            .send_json_frame(&query, "GetStateSnapshot")
            .await?;
        let resp: QueryResponse = query_conn.read_json_response("GetStateSnapshot").await?;
        if resp.request_id != request_id {
            return Err(ClientError::CorrelationMismatch {
                expected: request_id.to_string(),
                actual: resp.request_id.to_string(),
            });
        }
        let watermark = resp
            .watermark
            .ok_or(ClientError::MissingSnapshotWatermark)?;
        Ok((resp.result, watermark))
    }

    async fn open_stream_at(&self, cursor: EventCursor) -> Result<EventStreamReader, ClientError> {
        let req = SubscribeEventsRequest {
            stream_id: cursor.stream_id,
            after_sequence: cursor.sequence,
            project_id: self.follower.project_id,
            aggregate_id: self.follower.aggregate_id,
            batch_limit: self.follower.batch_limit.clamp(1, 10_000),
        };
        EventStreamReader::subscribe(&self.config, &req).await
    }

    fn reset_cursor_atomically(&mut self, watermark: EventCursor) {
        self.cursor = Some(watermark);
        self.seen_ids.clear();
        self.seen_order.clear();
    }

    fn filter_and_advance(
        &mut self,
        events: Vec<EventEnvelope>,
        batch_cursor: EventCursor,
    ) -> Vec<EventEnvelope> {
        let mut accepted = Vec::with_capacity(events.len());
        let active_stream: Option<StreamId> = self.cursor.map(|c| c.stream_id);
        let mut highest_seq = self.cursor.map_or(0, |c| c.sequence.as_u64());

        for ev in events {
            if let Some(stream) = active_stream
                && ev.stream_id != stream
            {
                continue;
            }
            if ev.sequence.as_u64() <= highest_seq {
                continue;
            }
            if !self.record_seen_event_id(ev.event_id) {
                continue;
            }
            highest_seq = ev.sequence.as_u64();
            accepted.push(ev);
        }

        let new_seq = batch_cursor.sequence.as_u64().max(highest_seq);
        self.cursor = Some(EventCursor::new(
            batch_cursor.stream_id,
            unleash_domain::Sequence::new(new_seq),
        ));
        accepted
    }

    fn record_seen_event_id(&mut self, event_id: Uuid) -> bool {
        if !self.seen_ids.insert(event_id) {
            return false;
        }
        self.seen_order.push_back(event_id);
        while self.seen_order.len() > MAX_DEDUP_WINDOW {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen_ids.remove(&oldest);
            }
        }
        true
    }
}
