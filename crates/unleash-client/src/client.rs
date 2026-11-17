// SPDX-License-Identifier: Apache-2.0
//! Façade unifiée `UnleashClient` utilisée par la CLI, la TUI et l'hôte GUI Tauri (`REQ-CORE-003`).

use serde_json::{Map, Value};
use unleash_domain::{CommandId, ProjectId, RequestId};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, CommandReceiptInspection, EventCursor, HelloResponse,
    ProtocolVersion, QueryEnvelope, QueryResponse, SubscribeEventsRequest, from_json_str_strict,
};

use crate::correlation::CommandCorrelationQueue;
use crate::errors::ClientError;
use crate::events::EventStreamReader;
use crate::transport::{ClientConfig, IpcConnection};
use crate::upload::{UploadSummary, build_upload_segments};

/// Client IPC commun vers le daemon Unleash (`unleash.ipc.v2`).
///
/// Aucune règle métier, transition ou accès `SQLite` n'existe côté client.
#[derive(Debug)]
pub struct UnleashClient {
    conn: IpcConnection,
}

impl UnleashClient {
    /// Établit une connexion authentifiée et négocie `Hello` (`unleash.ipc.v2`).
    ///
    /// # Sûreté à l'annulation
    /// Annuler cette future ferme simplement le descripteur local sans état persistant modifié.
    ///
    /// # Errors
    /// Propage les erreurs de sécurité socket/peer, de timeout ou de négociation [`IpcConnection::connect`].
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let conn = IpcConnection::connect(&config).await?;
        Ok(Self { conn })
    }

    /// Réponse `HelloResponse` négociée lors de la connexion active.
    #[must_use]
    pub const fn hello(&self) -> &HelloResponse {
        self.conn.hello()
    }

    /// Accès mutable à la connexion IPC sous-jacente (notamment pour [`crate::EventSynchronizer`]).
    pub const fn connection_mut(&mut self) -> &mut IpcConnection {
        &mut self.conn
    }

    /// Ferme la connexion courante et ouvre une nouvelle connexion `Hello` avec la même
    /// configuration, sans jamais modifier les enveloppes en attente dans [`CommandCorrelationQueue`].
    ///
    /// # Sûreté à l'annulation
    /// En cas d'annulation, l'ancienne connexion reste remplacée uniquement si la nouvelle aboutit
    /// (ou le client devra rappeler `reconnect`).
    ///
    /// # Errors
    /// Propage [`IpcConnection::connect`].
    pub async fn reconnect(&mut self) -> Result<&HelloResponse, ClientError> {
        let cfg = self.conn.config().clone();
        self.conn = IpcConnection::connect(&cfg).await?;
        Ok(self.conn.hello())
    }

    /// Transmet directement une [`CommandEnvelope`] sur la connexion active et lit le reçu ou l'erreur.
    ///
    /// # Sûreté à l'annulation
    /// Si annulée après écriture partielle/complète avant lecture du reçu, la commande doit être
    /// relancée via une nouvelle connexion en conservant `command_id` et `authority_generation`.
    ///
    /// # Errors
    /// Retourne [`ClientError::Remote`] si le daemon répond par une erreur IPC,
    /// [`ClientError::CorrelationMismatch`] si `receipt.command_id != envelope.command_id`,
    /// ou une erreur de transport/protocole.
    pub async fn send_command(
        &mut self,
        envelope: &CommandEnvelope,
    ) -> Result<CommandReceipt, ClientError> {
        self.conn
            .send_json_frame(envelope, "CommandEnvelope")
            .await?;
        let receipt: CommandReceipt = self.conn.read_json_response("CommandReceipt").await?;
        if receipt.command_id != envelope.command_id {
            return Err(ClientError::CorrelationMismatch {
                expected: envelope.command_id.to_string(),
                actual: receipt.command_id.to_string(),
            });
        }
        Ok(receipt)
    }

    /// Prépare (avec un `fresh_request_id` et sans jamais modifier `authority_generation` ni
    /// `command_id`), persiste en `Uncertain`, transmet puis enregistre le résultat dans `queue`.
    ///
    /// Si le daemon rejette avec `StaleFence / STALE_AUTHORITY_GENERATION`, `queue` verrouille
    /// l'entrée en [`crate::CorrelationState::StaleLineageRejected`] afin d'interdire toute
    /// réécriture automatique ultérieure.
    ///
    /// # Sûreté à l'annulation
    /// `queue` est persistée en `Uncertain` avant l'émission sur le socket : une annulation ou
    /// coupure laisse intactes l'identité `command_id` et `authority_generation` pour reprise.
    ///
    /// # Errors
    /// Propage les erreurs de `queue`, de transport, ou [`ClientError::Remote`].
    pub async fn submit_correlated(
        &mut self,
        queue: &mut CommandCorrelationQueue,
        command_id: CommandId,
        fresh_request_id: RequestId,
        now_ms: i64,
    ) -> Result<CommandReceipt, ClientError> {
        let envelope = queue.prepare_attempt(command_id, fresh_request_id, now_ms)?;
        match self.send_command(&envelope).await {
            Ok(receipt) => {
                queue.record_receipt(command_id, receipt.clone(), now_ms)?;
                Ok(receipt)
            }
            Err(ClientError::Remote(ipc_err)) => {
                let observed = Some(self.conn.authority_generation());
                queue.record_error(command_id, ipc_err.clone(), observed, now_ms)?;
                Err(ClientError::Remote(ipc_err))
            }
            Err(other) => Err(other),
        }
    }

    /// Découpe `bytes` en trames [`unleash_protocol::ClientUploadSegment`], les émet sur la même
    /// connexion authentifiée puis soumet l'enveloppe corrélée associée (ex. `ImportAutomationData`).
    ///
    /// # Sûreté à l'annulation
    /// Si annulée ou coupée pendant l'upload, les segments côté serveur sont éphémères et détruits
    /// à la fermeture ; un retry réémet les segments puis la même enveloppe conservée dans `queue`.
    ///
    /// # Errors
    /// - [`ClientError::UploadTooLarge`] avant toute I/O si `bytes.len() > 10 MiB`.
    /// - [`ClientError::Remote`] si le serveur rejette `CLIENT_UPLOAD_MISMATCH` ou toute autre erreur.
    pub async fn upload_and_submit(
        &mut self,
        queue: &mut CommandCorrelationQueue,
        command_id: CommandId,
        fresh_request_id: RequestId,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<(UploadSummary, CommandReceipt), ClientError> {
        let (segments, summary) = build_upload_segments(
            command_id,
            bytes,
            self.conn.hello().max_frame_bytes,
            self.conn.hello().suggested_segment_bytes,
        )?;
        let envelope = queue.prepare_attempt(command_id, fresh_request_id, now_ms)?;

        for seg in &segments {
            if let Err(err) = self.conn.send_json_frame(seg, "ClientUploadSegment").await {
                if matches!(err, ClientError::Disconnected { .. })
                    && let Some(ipc_err) = self.conn.try_read_trailing_ipc_error().await
                {
                    let observed = Some(self.conn.authority_generation());
                    let _ = queue.record_error(command_id, ipc_err.clone(), observed, now_ms);
                    return Err(ClientError::Remote(ipc_err));
                }
                return Err(err);
            }
        }

        match self.send_command(&envelope).await {
            Ok(receipt) => {
                queue.record_receipt(command_id, receipt.clone(), now_ms)?;
                Ok((summary, receipt))
            }
            Err(ClientError::Remote(ipc_err)) => {
                let observed = Some(self.conn.authority_generation());
                queue.record_error(command_id, ipc_err.clone(), observed, now_ms)?;
                Err(ClientError::Remote(ipc_err))
            }
            Err(other) => Err(other),
        }
    }

    /// Exécute une requête de lecture [`QueryEnvelope`] et vérifie `response.request_id`.
    ///
    /// # Sûreté à l'annulation
    /// Les requêtes sont sans effet durable ; en cas d'annulation, rouvrir ou reconnecter la
    /// connexion si la lecture de trame était entamée.
    ///
    /// # Errors
    /// Retourne [`ClientError::CorrelationMismatch`] si `request_id` diffère, ou erreur IPC/réseau.
    pub async fn query(&mut self, query: &QueryEnvelope) -> Result<QueryResponse, ClientError> {
        self.conn.send_json_frame(query, "QueryEnvelope").await?;
        let resp: QueryResponse = self.conn.read_json_response("QueryResponse").await?;
        if resp.request_id != query.request_id {
            return Err(ClientError::CorrelationMismatch {
                expected: query.request_id.to_string(),
                actual: resp.request_id.to_string(),
            });
        }
        Ok(resp)
    }

    /// Interroge `GetCommandReceipt` pour résoudre une commande incertaine ou inspecter un reçu
    /// historique (`current` vs `historical`).
    ///
    /// # Sûreté à l'annulation
    /// Lecture pure ; voir [`Self::query`].
    ///
    /// # Errors
    /// Propage [`Self::query`] ou erreur de désérialisation stricte de [`CommandReceiptInspection`].
    pub async fn get_command_receipt(
        &mut self,
        request_id: RequestId,
        command_id: CommandId,
    ) -> Result<CommandReceiptInspection, ClientError> {
        let mut payload = Map::new();
        payload.insert(
            String::from("command_id"),
            Value::String(command_id.to_string()),
        );
        let q = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id,
            query_type: String::from("GetCommandReceipt"),
            project_id: None,
            payload,
        };
        let resp = self.query(&q).await?;
        let raw =
            serde_json::to_string(&resp.result).map_err(unleash_protocol::ProtocolError::Json)?;
        from_json_str_strict(&raw).map_err(ClientError::Protocol)
    }

    /// Interroge `GetStateSnapshot` en exigeant la présence d'un watermark transactionnel cohérent.
    ///
    /// # Sûreté à l'annulation
    /// Lecture pure ; voir [`Self::query`].
    ///
    /// # Errors
    /// Retourne [`ClientError::MissingSnapshotWatermark`] si `watermark` est absent.
    pub async fn get_state_snapshot(
        &mut self,
        request_id: RequestId,
        project_id: Option<ProjectId>,
        payload: Map<String, Value>,
    ) -> Result<(Value, EventCursor), ClientError> {
        let q = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id,
            query_type: String::from("GetStateSnapshot"),
            project_id,
            payload,
        };
        let resp = self.query(&q).await?;
        let watermark = resp
            .watermark
            .ok_or(ClientError::MissingSnapshotWatermark)?;
        Ok((resp.result, watermark))
    }

    /// Ouvre un flux dédié d'abonnement `SubscribeEvents` sur une connexion séparée héritant de
    /// la même configuration client.
    ///
    /// # Sûreté à l'annulation
    /// Annuler ferme uniquement la nouvelle socket de souscription.
    ///
    /// # Errors
    /// Propage [`EventStreamReader::subscribe`].
    pub async fn subscribe_events(
        &self,
        request: &SubscribeEventsRequest,
    ) -> Result<EventStreamReader, ClientError> {
        EventStreamReader::subscribe(self.conn.config(), request).await
    }
}
