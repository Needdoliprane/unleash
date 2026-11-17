// SPDX-License-Identifier: Apache-2.0
//! Boucle serveur Unix IPC (`unleash.ipc.v2`), handshake `Hello`, barrière de récupération,
//! dispatch commandes/requêtes/abonnements et rétro-pression bornée.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore, mpsc, oneshot, watch};
use unleash_browser::LocalInventorySearchPaths;
use unleash_domain::{
    AuthorityGeneration, CommandId, CompletionPolicy, DaemonEpoch, DaemonInstanceId, DaemonMode,
    MissionId, MissionState, OperationId, ProjectId, Revision, Sequence, StreamId, TimestampMs,
    transition_mission_state,
};
use unleash_platform::verify_same_uid_peer;
use unleash_protocol::{
    CommandEnvelope, ErrorCode, EventCausationKind, EventCursor, EventEnvelope, HelloRequest,
    HelloResponse, IpcError, MAX_FRAME_BYTES, PROTOCOL_VERSION_V1, ProtocolError, ProtocolVersion,
    QueryEnvelope, QueryResponse, ReasonCode, ReceiptState, RetryClass, Sensitivity,
    SubscribeEventsRequest, from_json_str_strict, parse_json_value_strict,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, EventAppendSpec,
    ReadEventsOutcome, ReceiptInspectionOutcome, StoreError, UnleashStore,
};
use uuid::Uuid;

use crate::auth::{
    AuthVerifierPort, AuthenticatedSession, Capability, authorize_command, authorize_query,
    authorize_subscription,
};
use crate::config::PostCommitAction;
use crate::config::PostCommitHook;
use crate::error::{
    DaemonError, domain_error_to_ipc_error, platform_error_to_ipc_error, store_error_to_ipc_error,
};
use crate::executors::DrainReport;
use crate::subscriptions::{SubscriptionHub, run_subscription_stream};

const SUGGESTED_SEGMENT_BYTES: u32 = 1_048_576;
const IDEMPOTENCY_RETENTION_SECONDS: u64 = 7 * 86_400;
const HANDSHAKE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Requête asynchrone adressée au coordinateur de drainage et d'arrêt.
#[derive(Debug)]
pub(crate) struct StopRequest {
    pub(crate) reason: String,
    pub(crate) drain_timeout_override: Option<Duration>,
    pub(crate) ack: Option<oneshot::Sender<Result<DrainReport, DaemonError>>>,
}

/// État d'exécution partagé entre la boucle d'acceptation IPC et les connexions clientes.
pub(crate) struct DaemonRuntimeState {
    pub(crate) store: Arc<UnleashStore>,
    pub(crate) authority_generation: AuthorityGeneration,
    pub(crate) daemon_epoch: DaemonEpoch,
    pub(crate) daemon_instance_id: DaemonInstanceId,
    pub(crate) stream_id: StreamId,
    pub(crate) mode: Arc<RwLock<DaemonMode>>,
    pub(crate) browser_search_paths: LocalInventorySearchPaths,
    pub(crate) auth_verifier: Arc<dyn AuthVerifierPort>,
    pub(crate) post_commit_hook: Option<Arc<dyn PostCommitHook>>,
    pub(crate) subscriptions: Arc<SubscriptionHub>,
    pub(crate) last_published_seq: AsyncMutex<Option<(StreamId, u64)>>,
    pub(crate) subscriber_write_timeout: Duration,
    pub(crate) in_flight_mutations: Arc<Semaphore>,
    pub(crate) conn_permits: Arc<Semaphore>,
    pub(crate) accepting: AtomicBool,
    pub(crate) shutdown_notify: Arc<Notify>,
    pub(crate) drain_started_tx: watch::Sender<bool>,
    pub(crate) stop_tx: mpsc::Sender<StopRequest>,
}

impl DaemonRuntimeState {
    pub(crate) fn current_mode(&self) -> DaemonMode {
        *self.mode.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set_mode(&self, next: DaemonMode) {
        let mut guard = self.mode.write().unwrap_or_else(PoisonError::into_inner);
        *guard = next;
    }

    pub(crate) fn mark_draining(&self) {
        self.set_mode(DaemonMode::Draining);
        let _ = self.drain_started_tx.send(true);
    }

    pub(crate) fn subscribe_drain_started(&self) -> watch::Receiver<bool> {
        self.drain_started_tx.subscribe()
    }

    pub(crate) fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    pub(crate) fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
        self.shutdown_notify.notify_waiters();
    }
}

/// Lance la boucle d'acceptation sur `listener` jusqu'au signal d'arrêt et retourne les handles des tâches.
pub(crate) fn spawn_accept_loop(
    listener: UnixListener,
    state: Arc<DaemonRuntimeState>,
    active_conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if !state.is_accepting() {
                break;
            }
            tokio::select! {
                biased;
                () = state.shutdown_notify.notified() => {
                    if !state.is_accepting() {
                        break;
                    }
                }
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((stream, _addr)) => {
                            if !state.is_accepting() {
                                drop(stream);
                                break;
                            }
                            let Ok(permit) = Arc::clone(&state.conn_permits).try_acquire_owned() else {
                                let mut s = stream;
                                let busy = IpcError::from_reason(
                                    ReasonCode::Backpressure,
                                    "Nombre maximal de connexions IPC simultanées atteint",
                                );
                                let _ = write_json_frame_timeout(&mut s, &busy, RESPONSE_WRITE_TIMEOUT).await;
                                continue;
                            };
                            let st = Arc::clone(&state);
                            let task = tokio::spawn(async move {
                                let _permit = permit;
                                handle_client_connection(s_from(stream), st).await;
                            });
                            prune_and_push_handle(&active_conns, task);
                        }
                        Err(err) => {
                            if !state.is_accepting() {
                                break;
                            }
                            tracing::warn!(error = %err, "Erreur accept() IPC Unix");
                        }
                    }
                }
            }
        }
    })
}

const fn s_from(s: UnixStream) -> UnixStream {
    s
}

fn prune_and_push_handle(
    handles: &Mutex<Vec<tokio::task::JoinHandle<()>>>,
    task: tokio::task::JoinHandle<()>,
) {
    let mut guard = handles.lock().unwrap_or_else(PoisonError::into_inner);
    guard.retain(|h| !h.is_finished());
    guard.push(task);
}

async fn handle_client_connection(mut stream: UnixStream, state: Arc<DaemonRuntimeState>) {
    let Some(session) = perform_hello_handshake(&mut stream, &state).await else {
        return;
    };

    loop {
        let frame = tokio::select! {
            biased;
            () = state.shutdown_notify.notified(), if !state.is_accepting() => {
                break;
            }
            read_res = read_raw_frame_no_timeout(&mut stream, MAX_FRAME_BYTES) => {
                match read_res {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) => break,
                    Err(ipc_err) => {
                        let _ = write_json_frame_timeout(&mut stream, &ipc_err, RESPONSE_WRITE_TIMEOUT).await;
                        break;
                    }
                }
            }
        };

        if !dispatch_incoming_frame(&mut stream, &state, &session, &frame).await {
            return;
        }
    }
}

async fn perform_hello_handshake(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
) -> Option<AuthenticatedSession> {
    let peer_creds = match verify_same_uid_peer(&*stream) {
        Ok(creds) => creds,
        Err(err) => {
            let ipc = platform_error_to_ipc_error(&err);
            let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
            return None;
        }
    };

    let hello_raw =
        match read_raw_frame_timeout(stream, MAX_FRAME_BYTES, HANDSHAKE_IO_TIMEOUT).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(ipc_err) => {
                let _ = write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT).await;
                return None;
            }
        };

    let hello_req = match decode_strict_with_v1_check::<HelloRequest>(&hello_raw) {
        Ok(req) => req,
        Err(ipc_err) => {
            let _ = write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT).await;
            return None;
        }
    };

    let session = match state
        .auth_verifier
        .authenticate_peer(peer_creds, &hello_req)
    {
        Ok(s) => s,
        Err(ipc_err) => {
            let _ = write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT).await;
            return None;
        }
    };

    let resp = HelloResponse {
        protocol_version: ProtocolVersion,
        authority_generation: state.authority_generation,
        daemon_epoch: state.daemon_epoch,
        daemon_instance_id: state.daemon_instance_id,
        stream_id: state.stream_id,
        max_frame_bytes: u32::try_from(MAX_FRAME_BYTES).unwrap_or(u32::MAX),
        suggested_segment_bytes: SUGGESTED_SEGMENT_BYTES,
        idempotency_retention_seconds: IDEMPOTENCY_RETENTION_SECONDS,
        capabilities: session_capability_labels(&session),
    };
    if write_json_frame_timeout(stream, &resp, HANDSHAKE_IO_TIMEOUT)
        .await
        .is_err()
    {
        return None;
    }

    Some(session)
}

/// Retourne `true` si la connexion reste ouverte après traitement de la trame.
async fn dispatch_incoming_frame(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    frame: &[u8],
) -> bool {
    let text = match std::str::from_utf8(frame) {
        Ok(s) => s,
        Err(err) => {
            let ipc = IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("Trame UTF-8 invalide : {err}"),
                RetryClass::Never,
            );
            return write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT)
                .await
                .is_ok();
        }
    };

    if let Some(v1_err) = detect_v1_protocol_error(text) {
        return write_json_frame_timeout(stream, &v1_err, RESPONSE_WRITE_TIMEOUT)
            .await
            .is_ok();
    }

    let value = match parse_json_value_strict(text) {
        Ok(v) => v,
        Err(err) => {
            let ipc = protocol_error_to_ipc_error(&err);
            return write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT)
                .await
                .is_ok();
        }
    };

    let Some(obj) = value.as_object() else {
        let ipc = IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Chaque trame IPC doit être un objet JSON",
            RetryClass::Never,
        );
        return write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT)
            .await
            .is_ok();
    };

    if obj.contains_key("command_type") {
        return match decode_strict_with_v1_check::<CommandEnvelope>(frame) {
            Ok(cmd) => !handle_command_frame(stream, state, session, cmd).await,
            Err(ipc_err) => write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT)
                .await
                .is_ok(),
        };
    }

    if obj.contains_key("query_type") {
        return match decode_strict_with_v1_check::<QueryEnvelope>(frame) {
            Ok(query) => handle_query_frame(stream, state, session, query)
                .await
                .is_ok(),
            Err(ipc_err) => write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT)
                .await
                .is_ok(),
        };
    }

    if obj.contains_key("after_sequence") && obj.contains_key("stream_id") {
        handle_subscribe_frame(stream, state, session, text).await;
        return false;
    }

    let ipc = IpcError::new(
        ErrorCode::InvalidArgument,
        None,
        "Enveloppe IPC non reconnue (attendu command_type, query_type ou SubscribeEventsRequest)",
        RetryClass::Never,
    );
    write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT)
        .await
        .is_ok()
}

async fn handle_subscribe_frame(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    text: &str,
) {
    let sub_req = match from_json_str_strict::<SubscribeEventsRequest>(text) {
        Ok(req) => req,
        Err(err) => {
            let ipc = protocol_error_to_ipc_error(&err);
            let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
            return;
        }
    };

    if let Err(ipc) = authorize_subscription(session, &sub_req) {
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return;
    }

    let mode = state.current_mode();
    if matches!(mode, DaemonMode::Starting | DaemonMode::Recovering) {
        let ipc = IpcError::from_reason(
            ReasonCode::RecoveryInProgress,
            "Abonnement SubscribeEvents refusé tant que la récupération est en cours",
        );
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return;
    }

    run_subscription_stream(
        stream,
        sub_req,
        session.allowed_projects.clone(),
        Arc::clone(&state.store),
        Arc::clone(&state.subscriptions),
        state.subscriber_write_timeout,
    )
    .await;
}

fn session_capability_labels(session: &AuthenticatedSession) -> Vec<String> {
    let mut labels = Vec::new();
    for cap in &session.capabilities {
        let name = match cap {
            Capability::QueryDaemonDiagnostic => "query_daemon_diagnostic",
            Capability::QueryState => "query_state",
            Capability::ManageProjects => "manage_projects",
            Capability::ManageMissions => "manage_missions",
            Capability::StopDaemon => "stop_daemon",
            Capability::WorkerReport => "worker_report",
        };
        labels.push(String::from(name));
    }
    labels.sort();
    labels
}

async fn handle_query_frame(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<(), std::io::Error> {
    if let Err(ipc) = authorize_query(session, &query) {
        return write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
    }

    let mode = state.current_mode();
    if matches!(mode, DaemonMode::Starting | DaemonMode::Recovering)
        && query.query_type != "GetDaemonStatus"
    {
        let ipc = IpcError::from_reason(
            ReasonCode::RecoveryInProgress,
            format!(
                "Requête '{}' refusée en mode '{mode}' : seul GetDaemonStatus est autorisé avant ready",
                query.query_type
            ),
        );
        return write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
    }

    match dispatch_query(state, session, query).await {
        Ok(resp) => write_json_frame_timeout(stream, &resp, RESPONSE_WRITE_TIMEOUT).await,
        Err(ipc) => write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await,
    }
}

async fn dispatch_query(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    match query.query_type.as_str() {
        "GetDaemonStatus" => query_daemon_status(state, query.request_id).await,
        "GetHealth" => query_health(state, query.request_id).await,
        "GetStateSnapshot" => query_state_snapshot(state, session, query).await,
        "GetCommandReceipt" => query_command_receipt(state, session, query).await,
        other => Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Type de requête non supporté : '{other}'"),
            RetryClass::Never,
        )),
    }
}

async fn query_daemon_status(
    state: &Arc<DaemonRuntimeState>,
    request_id: unleash_domain::RequestId,
) -> Result<QueryResponse, IpcError> {
    let store = Arc::clone(&state.store);
    let mode_mem = state.current_mode();
    let (daemon_row, watermark, dead_letters) = tokio::task::spawn_blocking(move || {
        let dl = store.list_dead_letters()?;
        let snap = store.read_snapshot(|conn, _ctx| {
            let row = read_daemon_row_conn(conn)?;
            let wm = read_watermark_conn(conn, row.stream_id)?;
            Ok((row, wm))
        })?;
        Ok::<_, StoreError>((snap.0, snap.1, dl))
    })
    .await
    .map_err(|e| internal_ipc_error(&format!("Join error GetDaemonStatus: {e}"), None))?
    .map_err(|e| store_error_to_ipc_error(&e, None))?;

    let result = json!({
        "mode": mode_mem.to_string(),
        "persisted_mode": daemon_row.mode.to_string(),
        "authority_generation": daemon_row.authority_generation,
        "daemon_epoch": daemon_row.daemon_epoch.as_u64(),
        "daemon_instance_id": daemon_row.daemon_instance_id,
        "stream_id": daemon_row.stream_id,
        "revision": daemon_row.revision.as_u64(),
        "owner_uid": daemon_row.owner_uid,
        "started_at_ms": daemon_row.started_at_ms.as_i64(),
        "dead_letter": dead_letters,
        "dead_letter_count": dead_letters.len(),
        "watermark": watermark,
    });
    Ok(QueryResponse {
        request_id,
        watermark: Some(watermark),
        result,
    })
}

async fn query_health(
    state: &Arc<DaemonRuntimeState>,
    request_id: unleash_domain::RequestId,
) -> Result<QueryResponse, IpcError> {
    let store = Arc::clone(&state.store);
    let search_paths = state.browser_search_paths.clone();
    let mode_mem = state.current_mode();
    let (daemon_row, watermark, dead_letters, wal, sqlite_manifest, browser_runtimes) =
        tokio::task::spawn_blocking(move || {
            let dl = store.list_dead_letters()?;
            let wal = store.wal_telemetry();
            let manifest = store.runtime_manifest().clone();
            let snap = store.read_snapshot(|conn, _ctx| {
                let row = read_daemon_row_conn(conn)?;
                let wm = read_watermark_conn(conn, row.stream_id)?;
                Ok((row, wm))
            })?;
            let inventory = unleash_browser::inventory_local_runtimes(&search_paths);
            Ok::<_, StoreError>((snap.0, snap.1, dl, wal, manifest, inventory))
        })
        .await
        .map_err(|e| internal_ipc_error(&format!("Join error GetHealth: {e}"), None))?
        .map_err(|e| store_error_to_ipc_error(&e, None))?;

    let health_status = if matches!(
        mode_mem,
        DaemonMode::Degraded | DaemonMode::ReadOnlyRecovery
    ) || !dead_letters.is_empty()
    {
        "degraded"
    } else {
        "healthy"
    };
    let result = json!({
        "status": health_status,
        "mode": mode_mem.to_string(),
        "authority_generation": daemon_row.authority_generation,
        "daemon_epoch": daemon_row.daemon_epoch.as_u64(),
        "revision": daemon_row.revision.as_u64(),
        "dead_letter": dead_letters,
        "dead_letter_count": dead_letters.len(),
        "browser_runtimes": browser_runtimes,
        "sqlite_runtime": sqlite_manifest,
        "wal_telemetry": {
            "queue_depth": wal.queue_depth,
            "max_queue_depth_observed": wal.max_queue_depth_observed,
            "total_submitted": wal.total_submitted,
            "total_rejected_backpressure": wal.total_rejected_backpressure,
            "total_rejected_wal_stop": wal.total_rejected_wal_stop,
            "wal_bytes": wal.wal_bytes,
            "max_wal_bytes_observed": wal.max_wal_bytes_observed,
            "passive_checkpoints": wal.passive_checkpoints,
            "truncate_checkpoints": wal.truncate_checkpoints,
            "last_checkpoint_frames": wal.last_checkpoint_frames,
            "active_readers": wal.active_readers,
            "oldest_reader_age_ms": wal.oldest_reader_age_ms,
            "expired_readers_terminated": wal.expired_readers_terminated,
            "reader_neutralization_blocked": wal.reader_neutralization_blocked,
        },
        "watermark": watermark,
    });
    Ok(QueryResponse {
        request_id,
        watermark: Some(watermark),
        result,
    })
}

async fn query_state_snapshot(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    let store = Arc::clone(&state.store);
    let proj_filter = query.project_id;
    let allowed_projects = session.allowed_projects.clone();
    let (snapshot_val, watermark) = tokio::task::spawn_blocking(move || {
        store.read_snapshot(|conn, _ctx| {
            let daemon_row = read_daemon_row_conn(conn)?;
            let wm = read_watermark_conn(conn, daemon_row.stream_id)?;
            let projects = read_projects_conn(conn, proj_filter, allowed_projects.as_ref())?;
            let missions = read_missions_conn(conn, proj_filter, allowed_projects.as_ref())?;
            let val = json!({
                "daemon": {
                    "authority_generation": daemon_row.authority_generation,
                    "daemon_epoch": daemon_row.daemon_epoch.as_u64(),
                    "daemon_instance_id": daemon_row.daemon_instance_id,
                    "stream_id": daemon_row.stream_id,
                    "mode": daemon_row.mode.to_string(),
                    "revision": daemon_row.revision.as_u64(),
                },
                "projects": projects,
                "missions": missions,
            });
            Ok((val, wm))
        })
    })
    .await
    .map_err(|e| internal_ipc_error(&format!("Join error GetStateSnapshot: {e}"), None))?
    .map_err(|e| store_error_to_ipc_error(&e, None))?;

    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(watermark),
        result: snapshot_val,
    })
}

async fn query_command_receipt(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    let cmd_id_str = query
        .payload
        .get("command_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "GetCommandReceipt requiert payload.command_id (UUID)",
                RetryClass::Never,
            )
        })?;
    let uuid = Uuid::parse_str(cmd_id_str).map_err(|err| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("command_id invalide dans GetCommandReceipt : {err}"),
            RetryClass::Never,
        )
    })?;
    let cmd_id = CommandId::from_uuid(uuid);
    let store = Arc::clone(&state.store);
    let actor_id = session.actor_id.clone();

    let outcome =
        tokio::task::spawn_blocking(move || store.inspect_command_receipt(&actor_id, cmd_id))
            .await
            .map_err(|e| {
                internal_ipc_error(&format!("Join error GetCommandReceipt: {e}"), Some(cmd_id))
            })?
            .map_err(|e| store_error_to_ipc_error(&e, Some(cmd_id)))?;

    match outcome {
        ReceiptInspectionOutcome::Full(inspection) => {
            let result = serde_json::to_value(inspection).map_err(|e| {
                internal_ipc_error(
                    &format!("Sérialisation CommandReceiptInspection échouée : {e}"),
                    Some(cmd_id),
                )
            })?;
            Ok(QueryResponse {
                request_id: query.request_id,
                watermark: None,
                result,
            })
        }
        ReceiptInspectionOutcome::Tombstone {
            receipt_id,
            command_id,
            ..
        } => Err(IpcError::from_reason(
            ReasonCode::IdempotencyWindowExpired,
            format!("Réponse complète expirée et purgée en tombstone (reçu {receipt_id})"),
        )
        .with_command_id(command_id)),
    }
}

fn validate_command_mode(mode: DaemonMode, cmd: &CommandEnvelope) -> Result<(), IpcError> {
    if mode == DaemonMode::Starting
        || (mode == DaemonMode::Recovering && cmd.command_type != "StopDaemon")
    {
        return Err(IpcError::from_reason(
            ReasonCode::RecoveryInProgress,
            format!(
                "Commande '{}' refusée tant que le daemon est en mode '{mode}'",
                cmd.command_type
            ),
        )
        .with_command_id(cmd.command_id));
    }
    if matches!(mode, DaemonMode::Draining | DaemonMode::Stopped) {
        return Err(IpcError::new(
            ErrorCode::ResourceBusy,
            None,
            format!(
                "Commande '{}' refusée : arrêt du daemon en cours ({mode})",
                cmd.command_type
            ),
            RetryClass::Never,
        )
        .with_command_id(cmd.command_id));
    }
    Ok(())
}

/// Retourne `true` si le hook `PostCommitAction::DropConnection` demande de couper la connexion sans réponse.
async fn handle_command_frame(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> bool {
    if let Err(ipc) = authorize_command(session, &cmd) {
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return false;
    }

    if let Err(ipc) = validate_command_mode(state.current_mode(), &cmd) {
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return false;
    }

    // Backpressure immédiate côté serveur IPC avant attente
    let Ok(_in_flight_permit) = Arc::clone(&state.in_flight_mutations).try_acquire_owned() else {
        let ipc = IpcError::from_reason(
            ReasonCode::Backpressure,
            "Nombre maximal de mutations IPC en vol atteint",
        )
        .with_command_id(cmd.command_id);
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return false;
    };

    let mut prepared = match prepare_command_request(state, session, cmd.clone()).await {
        Ok(p) => p,
        Err(ipc) => {
            let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
            return false;
        }
    };

    if matches!(prepared.req.mutation, CommandMutation::CreateMission { .. })
        && let Err(ipc) = ensure_principal_registered(&state.store, session).await
    {
        let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
        return false;
    }

    let num_events = prepared.req.events.len();
    let store_ref = Arc::clone(&state.store);
    let req_to_commit = prepared.req;
    let outcome =
        match tokio::task::spawn_blocking(move || store_ref.commit_command(req_to_commit)).await {
            Ok(Ok(o)) => o,
            Ok(Err(err)) => {
                let ipc = store_error_to_ipc_error(&err, Some(cmd.command_id));
                let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
                return false;
            }
            Err(join_err) => {
                let ipc = internal_ipc_error(
                    &format!("Join error commit_command: {join_err}"),
                    Some(cmd.command_id),
                );
                let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
                return false;
            }
        };

    let (receipt, newly_committed) = match outcome {
        CommitOutcome::Committed(r) => (r, true),
        CommitOutcome::IdempotentReplay(r) => (r, false),
    };

    if newly_committed && num_events > 0 {
        publish_committed_events_for_command(state, &receipt, num_events).await;
    }

    if newly_committed && let Some(stop_reason) = prepared.stop_reason.take() {
        state.mark_draining();
        let _ = state.stop_tx.try_send(StopRequest {
            reason: stop_reason,
            drain_timeout_override: prepared.stop_drain_timeout.take(),
            ack: None,
        });
    }

    if let Some(ref hook) = state.post_commit_hook
        && hook.after_commit(&cmd) == PostCommitAction::DropConnection
    {
        return true;
    }

    let _ = write_json_frame_timeout(stream, &receipt, RESPONSE_WRITE_TIMEOUT).await;
    false
}

struct PreparedCommand {
    req: CommitCommandRequest,
    stop_reason: Option<String>,
    stop_drain_timeout: Option<Duration>,
}

async fn ensure_principal_registered(
    store: &Arc<UnleashStore>,
    session: &AuthenticatedSession,
) -> Result<(), IpcError> {
    let store_ref = Arc::clone(store);
    let actor_ref = session.actor_ref.clone();
    let actor_id_str = session.actor_id.as_str().to_owned();
    tokio::task::spawn_blocking(move || {
        let already = store_ref.read_snapshot(|conn, _ctx| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM principals WHERE principal_id = ?1",
                    [&actor_ref],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            Ok(count > 0)
        })?;
        if !already {
            store_ref.register_principal(&actor_ref, "user", &actor_id_str)?;
        }
        Ok::<_, StoreError>(())
    })
    .await
    .map_err(|e| internal_ipc_error(&format!("Join error principal registration: {e}"), None))?
    .map_err(|e| store_error_to_ipc_error(&e, None))?;
    Ok(())
}

async fn prepare_command_request(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    match cmd.command_type.as_str() {
        "CreateProject" => prepare_create_project(session, cmd),
        "AssignMission" => prepare_assign_mission(session, cmd),
        "PauseMission" | "ResumeMission" | "CancelMission" => {
            prepare_mission_transition_command(state, session, cmd).await
        }
        "StopDaemon" => prepare_stop_daemon(state, session, cmd),
        other => Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Commande non supportée dans ce jalon : '{other}'"),
            RetryClass::Never,
        )
        .with_command_id(cid)),
    }
}

fn prepare_create_project(
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let correlation_id = cmd.request_id.as_uuid();
    let project_id = match cmd.project_id {
        Some(pid) => {
            if pid.as_uuid() != cmd.aggregate_id {
                return Err(IpcError::new(
                    ErrorCode::InvalidArgument,
                    None,
                    "CreateProject : aggregate_id doit correspondre à project_id",
                    RetryClass::Never,
                )
                .with_command_id(cid));
            }
            pid
        }
        None => ProjectId::from_uuid(cmd.aggregate_id),
    };
    let name = cmd
        .payload
        .get("name")
        .and_then(Value::as_str)
        .map_or_else(|| format!("project-{project_id}"), ToOwned::to_owned);
    if name.trim().is_empty() {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "CreateProject : name ne peut pas être vide",
            RetryClass::Never,
        )
        .with_command_id(cid));
    }
    let weight = cmd
        .payload
        .get("weight")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    if !(1..=10).contains(&weight) {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "CreateProject : weight doit être compris entre 1 et 10",
            RetryClass::Never,
        )
        .with_command_id(cid));
    }

    let mut ev_payload = Map::new();
    ev_payload.insert(String::from("name"), Value::String(name.clone()));
    ev_payload.insert(String::from("weight"), Value::from(weight));

    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id,
                name,
                weight,
            },
            events: vec![EventAppendSpec {
                event_type: String::from("ProjectCreated"),
                aggregate_type: AggregateKind::Project,
                aggregate_id: project_id.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(project_id),
                correlation_id,
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: ev_payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: Vec::new(),
            }],
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

#[derive(Debug)]
struct AssignMissionFields {
    project_id: ProjectId,
    mission_id: MissionId,
    goal_id: Option<unleash_domain::GoalId>,
    completion_policy: CompletionPolicy,
    initial_state: MissionState,
}

fn parse_assign_mission_fields(cmd: &CommandEnvelope) -> Result<AssignMissionFields, IpcError> {
    let cid = cmd.command_id;
    let project_id = cmd.project_id.ok_or_else(|| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "AssignMission requiert project_id",
            RetryClass::Never,
        )
        .with_command_id(cid)
    })?;
    let completion_policy = match cmd.payload.get("completion_policy").and_then(Value::as_str) {
        None | Some("mechanical") => CompletionPolicy::Mechanical,
        Some("human") => CompletionPolicy::Human,
        Some("owner_agent") => CompletionPolicy::OwnerAgent,
        Some(other) => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("completion_policy inconnue : '{other}'"),
                RetryClass::Never,
            )
            .with_command_id(cid));
        }
    };
    let initial_state = match cmd.payload.get("state").and_then(Value::as_str) {
        None | Some("active") => MissionState::Active,
        Some("proposed") => MissionState::Proposed,
        Some("ready") => MissionState::Ready,
        Some(other) => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("État initial de mission invalide pour AssignMission : '{other}'"),
                RetryClass::Never,
            )
            .with_command_id(cid));
        }
    };
    let goal_id = cmd
        .payload
        .get("goal_id")
        .and_then(Value::as_str)
        .map(|s| Uuid::parse_str(s).map(unleash_domain::GoalId::from_uuid))
        .transpose()
        .map_err(|err| {
            IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("goal_id invalide : {err}"),
                RetryClass::Never,
            )
            .with_command_id(cid)
        })?;
    Ok(AssignMissionFields {
        project_id,
        mission_id: MissionId::from_uuid(cmd.aggregate_id),
        goal_id,
        completion_policy,
        initial_state,
    })
}

fn prepare_assign_mission(
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let correlation_id = cmd.request_id.as_uuid();
    let fields = parse_assign_mission_fields(&cmd)?;
    let project_id = fields.project_id;
    let mission_id = fields.mission_id;
    let goal_id = fields.goal_id;
    let completion_policy = fields.completion_policy;
    let initial_state = fields.initial_state;

    let mut ev_payload = Map::new();
    ev_payload.insert(
        String::from("completion_policy"),
        Value::String(completion_policy.to_string()),
    );
    ev_payload.insert(
        String::from("state"),
        Value::String(initial_state.to_string()),
    );

    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateMission {
                mission_id,
                goal_id,
                project_id,
                owner_principal_id: Some(session.actor_ref.clone()),
                owner_agent_id: None,
                completion_policy,
                state: initial_state,
            },
            events: vec![EventAppendSpec {
                event_type: String::from("MissionAssigned"),
                aggregate_type: AggregateKind::Mission,
                aggregate_id: mission_id.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(project_id),
                correlation_id,
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: ev_payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: Vec::new(),
            }],
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

pub(crate) fn validate_stop_drain_timeout_ms(
    payload: &Map<String, Value>,
    cid: CommandId,
) -> Result<Option<Duration>, IpcError> {
    let Some(val) = payload.get("drain_timeout_ms") else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Some(ms) = val.as_u64() else {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "StopDaemon : drain_timeout_ms doit être un entier positif ou nul en millisecondes",
            RetryClass::Never,
        )
        .with_command_id(cid));
    };
    if ms > 3_600_000 {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "StopDaemon : drain_timeout_ms dépasse la borne maximale de 3600000 ms",
            RetryClass::Never,
        )
        .with_command_id(cid));
    }
    Ok(Some(Duration::from_millis(ms)))
}

fn prepare_stop_daemon(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let correlation_id = cmd.request_id.as_uuid();
    if cmd.aggregate_id != state.authority_generation.as_uuid() {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "StopDaemon requiert aggregate_id = authority_generation",
            RetryClass::Never,
        )
        .with_command_id(cid));
    }
    let drain_timeout_override = validate_stop_drain_timeout_ms(&cmd.payload, cid)?;
    let reason = cmd
        .payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("arrêt demandé via IPC")
        .to_owned();
    let op_id = OperationId::from_uuid(state.store.config().id_generator.new_uuid());
    let next_rev = cmd.expected_revision.next();
    let mut ev_payload = Map::new();
    ev_payload.insert(String::from("reason"), Value::String(reason.clone()));
    ev_payload.insert(
        String::from("operation_id"),
        Value::String(op_id.to_string()),
    );
    if let Some(dur) = drain_timeout_override {
        let ms = u64::try_from(dur.as_millis()).unwrap_or(u64::MAX);
        ev_payload.insert(String::from("drain_timeout_ms"), Value::from(ms));
    }

    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::Daemon,
            operation_id: Some(op_id),
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::TransitionDaemonMode {
                target_mode: DaemonMode::Draining,
            },
            events: vec![EventAppendSpec {
                event_type: String::from("DaemonStopRequested"),
                aggregate_type: AggregateKind::Daemon,
                aggregate_id: state.authority_generation.as_uuid(),
                aggregate_revision: next_rev,
                project_id: None,
                correlation_id,
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: ev_payload,
                sensitivity: Sensitivity::Public,
                outbox_destinations: Vec::new(),
            }],
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: Some(reason),
        stop_drain_timeout: drain_timeout_override,
    })
}

async fn prepare_mission_transition_command(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let mission_id = MissionId::from_uuid(cmd.aggregate_id);
    let correlation_id = cmd.request_id.as_uuid();
    let (target_state, event_type) = match cmd.command_type.as_str() {
        "PauseMission" => (MissionState::Paused, "MissionPaused"),
        "ResumeMission" => (MissionState::Active, "MissionResumed"),
        "CancelMission" => (MissionState::Cancelled, "MissionCancelled"),
        other => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("Transition mission inconnue : '{other}'"),
                RetryClass::Never,
            )
            .with_command_id(cid));
        }
    };

    let event_project_id =
        check_mission_transition_preconditions(state, session, &cmd, mission_id, target_state)
            .await?;

    let next_rev = cmd.expected_revision.next();
    let mut ev_payload = Map::new();
    ev_payload.insert(
        String::from("target_state"),
        Value::String(target_state.to_string()),
    );
    if let Some(reason) = cmd.payload.get("reason") {
        ev_payload.insert(String::from("reason"), reason.clone());
    }

    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::TransitionMission {
                mission_id,
                target_state,
            },
            events: vec![EventAppendSpec {
                event_type: String::from(event_type),
                aggregate_type: AggregateKind::Mission,
                aggregate_id: mission_id.as_uuid(),
                aggregate_revision: next_rev,
                project_id: event_project_id,
                correlation_id,
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: ev_payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: Vec::new(),
            }],
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

async fn check_mission_transition_preconditions(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: &CommandEnvelope,
    mission_id: MissionId,
    target_state: MissionState,
) -> Result<Option<ProjectId>, IpcError> {
    // Important (`T-F001-005`) : la vérification d'authority_generation et la détection d'idempotence
    // (ou de conflit d'idempotence) doivent passer AVANT toute pré-vérification d'état mission
    // qui rejetterait un retry valide post-coupure.
    let cid = cmd.command_id;
    let store_ref = Arc::clone(&state.store);
    let actor_id = session.actor_id.clone();
    let env_auth = cmd.authority_generation;
    let env_proj = cmd.project_id;
    let expected_rev = cmd.expected_revision;

    tokio::task::spawn_blocking(move || {
        let daemon = store_ref
            .daemon_state()
            .map_err(|e| store_error_to_ipc_error(&e, Some(cid)))?;
        if env_auth != daemon.authority_generation {
            return Ok(env_proj);
        }
        if store_ref.inspect_command_receipt(&actor_id, cid).is_ok() {
            return Ok(env_proj);
        }

        let found: Option<(String, String, i64)> = store_ref
            .read_snapshot(|conn, _ctx| {
                rusqlite::OptionalExtension::optional(conn.query_row(
                    "SELECT state, project_id, revision FROM missions WHERE mission_id = ?1",
                    [mission_id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                ))
                .map_err(StoreError::from_rusqlite)
            })
            .map_err(|e| store_error_to_ipc_error(&e, Some(cid)))?;

        let Some((m_state_str, m_proj_str, m_rev)) = found else {
            return Err(store_error_to_ipc_error(
                &StoreError::NotFound(format!("mission {mission_id}")),
                Some(cid),
            ));
        };

        let cur_rev = Revision::new(u64::try_from(m_rev).unwrap_or(0));
        if cur_rev != expected_rev {
            return Err(store_error_to_ipc_error(
                &StoreError::RevisionConflict {
                    current_revision: cur_rev,
                    expected_revision: expected_rev,
                    command_id: Some(cid),
                },
                Some(cid),
            ));
        }

        let m_proj_uuid = Uuid::parse_str(&m_proj_str).map_err(|e| {
            internal_ipc_error(
                &format!("project_id mission invalide '{m_proj_str}': {e}"),
                Some(cid),
            )
        })?;
        let mission_project_id = ProjectId::from_uuid(m_proj_uuid);
        if let Some(pid) = env_proj
            && pid != mission_project_id
        {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("project_id {pid} incompatible avec la mission {mission_id}"),
                RetryClass::Never,
            )
            .with_command_id(cid));
        }

        let cur_state = parse_mission_state(&m_state_str)
            .map_err(|e| store_error_to_ipc_error(&e, Some(cid)))?;
        let _ = transition_mission_state(cur_state, target_state)
            .map_err(|e| domain_error_to_ipc_error(&e, Some(cid)))?;

        Ok(Some(mission_project_id))
    })
    .await
    .map_err(|e| internal_ipc_error(&format!("Join error mission precheck: {e}"), Some(cid)))?
}

async fn publish_committed_events_for_command(
    state: &Arc<DaemonRuntimeState>,
    receipt: &unleash_protocol::CommandReceipt,
    num_events: usize,
) {
    publish_committed_events_ordered(
        state,
        receipt.event_cursor.stream_id,
        receipt.event_cursor.sequence.as_u64(),
        num_events,
        &[],
    )
    .await;
}

fn is_contiguous_direct_batch(
    batch: &[EventEnvelope],
    prev_opt: Option<u64>,
    target_end_seq: u64,
) -> bool {
    let Some(prev) = prev_opt else {
        return false;
    };
    let mut iter = batch.iter();
    let Some(first) = iter.next() else {
        return false;
    };
    let mut expected = prev.saturating_add(1);
    if first.sequence.as_u64() != expected {
        return false;
    }
    for ev in iter {
        expected = expected.saturating_add(1);
        if ev.sequence.as_u64() != expected {
            return false;
        }
    }
    expected >= target_end_seq
}

/// Publie les événements engagés jusqu'à `target_end_seq` dans l'ordre strict de `sequence`
/// afin qu'aucun commit concurrent ne soit perdu par avance prématurée de curseur.
pub(crate) async fn publish_committed_events_ordered(
    state: &DaemonRuntimeState,
    stream_id: StreamId,
    target_end_seq: u64,
    num_events_hint: usize,
    direct_fallback: &[EventEnvelope],
) {
    let mut guard = state.last_published_seq.lock().await;
    let prev_opt = match *guard {
        Some((sid, s)) if sid == stream_id => Some(s),
        _ => None,
    };
    if prev_opt.is_some_and(|p| target_end_seq <= p) {
        return;
    }

    if is_contiguous_direct_batch(direct_fallback, prev_opt, target_end_seq) {
        let max_seq = direct_fallback
            .last()
            .map_or(target_end_seq, |ev| ev.sequence.as_u64());
        state.subscriptions.publish_events(direct_fallback);
        *guard = Some((stream_id, max_seq));
        return;
    }

    let after_seq = prev_opt.unwrap_or_else(|| {
        target_end_seq.saturating_sub(u64::try_from(num_events_hint.max(1)).unwrap_or(1))
    });
    let store = Arc::clone(&state.store);
    let read_res =
        tokio::task::spawn_blocking(move || -> Result<Option<Vec<EventEnvelope>>, StoreError> {
            let mut cursor_seq = after_seq;
            let mut collected = Vec::new();
            while cursor_seq < target_end_seq {
                match store.read_events(stream_id, Sequence::new(cursor_seq), 256, None)? {
                    ReadEventsOutcome::ResyncRequired { .. } => return Ok(None),
                    ReadEventsOutcome::Events { events, .. } => {
                        if events.is_empty() {
                            break;
                        }
                        let mut advanced = false;
                        for ev in events {
                            let s = ev.sequence.as_u64();
                            if s > cursor_seq {
                                cursor_seq = s;
                                advanced = true;
                                collected.push(ev);
                            }
                        }
                        if !advanced {
                            break;
                        }
                    }
                }
            }
            Ok(Some(collected))
        })
        .await;

    match read_res {
        Ok(Ok(Some(mut collected))) if !collected.is_empty() => {
            collected.sort_by_key(|ev| ev.sequence.as_u64());
            let max_seq = collected
                .last()
                .map_or(target_end_seq, |ev| ev.sequence.as_u64());
            state.subscriptions.publish_events(&collected);
            *guard = Some((stream_id, max_seq));
        }
        _ => {
            let mut fallback: Vec<EventEnvelope> = direct_fallback
                .iter()
                .filter(|ev| prev_opt.is_none_or(|p| ev.sequence.as_u64() > p))
                .cloned()
                .collect();
            fallback.sort_by_key(|ev| ev.sequence.as_u64());
            if !fallback.is_empty() {
                let max_seq = fallback
                    .last()
                    .map_or(target_end_seq, |ev| ev.sequence.as_u64());
                state.subscriptions.publish_events(&fallback);
                *guard = Some((stream_id, max_seq));
            }
        }
    }
}

fn parse_mission_state(s: &str) -> Result<MissionState, StoreError> {
    match s {
        "proposed" => Ok(MissionState::Proposed),
        "ready" => Ok(MissionState::Ready),
        "active" => Ok(MissionState::Active),
        "paused" => Ok(MissionState::Paused),
        "blocked" => Ok(MissionState::Blocked),
        "succeeded" => Ok(MissionState::Succeeded),
        "failed" => Ok(MissionState::Failed),
        "cancelled" => Ok(MissionState::Cancelled),
        other => Err(StoreError::InvariantViolation(format!(
            "état de mission inconnu en base : '{other}'"
        ))),
    }
}

fn read_daemon_row_conn(conn: &Connection) -> Result<unleash_store::DaemonStateRow, StoreError> {
    use unleash_domain::{DaemonMode as Dm, Revision as Rev};
    let row = conn
        .query_row(
            "SELECT owner_uid, authority_generation, daemon_epoch, daemon_instance_id, boot_identity,
                    stream_id, schema_version, mode, revision, started_at_ms
             FROM daemon_state WHERE singleton = 1",
            [],
            |r| {
                let owner_uid_i64: i64 = r.get(0)?;
                let auth_str: String = r.get(1)?;
                let epoch_i64: i64 = r.get(2)?;
                let inst_str: String = r.get(3)?;
                let boot_identity: String = r.get(4)?;
                let stream_str: String = r.get(5)?;
                let schema_version: i64 = r.get(6)?;
                let mode_str: String = r.get(7)?;
                let rev_i64: i64 = r.get(8)?;
                let started_i64: i64 = r.get(9)?;
                Ok((
                    owner_uid_i64,
                    auth_str,
                    epoch_i64,
                    inst_str,
                    boot_identity,
                    stream_str,
                    schema_version,
                    mode_str,
                    rev_i64,
                    started_i64,
                ))
            },
        )
        .map_err(StoreError::from_rusqlite)?;

    let owner_uid = u32::try_from(row.0)
        .map_err(|_| StoreError::InvariantViolation(String::from("owner_uid négatif")))?;
    let authority_generation = row.1.parse::<AuthorityGeneration>().map_err(|e| {
        StoreError::InvariantViolation(format!("authority_generation invalide : {e}"))
    })?;
    let daemon_epoch = DaemonEpoch::new(
        u64::try_from(row.2)
            .map_err(|_| StoreError::InvariantViolation(String::from("daemon_epoch invalide")))?,
    );
    let daemon_instance_id = row.3.parse::<DaemonInstanceId>().map_err(|e| {
        StoreError::InvariantViolation(format!("daemon_instance_id invalide : {e}"))
    })?;
    let stream_id = row
        .5
        .parse::<StreamId>()
        .map_err(|e| StoreError::InvariantViolation(format!("stream_id invalide : {e}")))?;
    let mode = match row.7.as_str() {
        "starting" => Dm::Starting,
        "recovering" => Dm::Recovering,
        "ready" => Dm::Ready,
        "degraded" => Dm::Degraded,
        "read_only_recovery" => Dm::ReadOnlyRecovery,
        "draining" => Dm::Draining,
        "stopped" => Dm::Stopped,
        other => {
            return Err(StoreError::InvariantViolation(format!(
                "mode daemon inconnu : '{other}'"
            )));
        }
    };
    let revision = Rev::new(u64::try_from(row.8).unwrap_or(0));
    let started_at_ms = TimestampMs::new(row.9);

    Ok(unleash_store::DaemonStateRow {
        owner_uid,
        authority_generation,
        daemon_epoch,
        daemon_instance_id,
        boot_identity: row.4,
        stream_id,
        schema_version: row.6,
        mode,
        revision,
        started_at_ms,
    })
}

fn read_watermark_conn(conn: &Connection, stream_id: StreamId) -> Result<EventCursor, StoreError> {
    let max_seq_i64: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
            [stream_id.to_string()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(EventCursor::new(
        stream_id,
        Sequence::new(u64::try_from(max_seq_i64).unwrap_or(0)),
    ))
}

fn read_projects_conn(
    conn: &Connection,
    project_filter: Option<ProjectId>,
    allowed_projects: Option<&std::collections::HashSet<ProjectId>>,
) -> Result<Vec<Value>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT project_id, name, weight, state, revision, created_at_ms
             FROM projects
             ORDER BY created_at_ms ASC, project_id ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;
    let mut out = Vec::new();
    for item in rows {
        let (pid_str, name, weight, state, rev, created_at_ms) =
            item.map_err(StoreError::from_rusqlite)?;
        let pid_opt = Uuid::parse_str(&pid_str).ok().map(ProjectId::from_uuid);
        if let Some(filter) = project_filter
            && pid_opt != Some(filter)
        {
            continue;
        }
        if let Some(allowed) = allowed_projects
            && pid_opt.is_none_or(|p| !allowed.contains(&p))
        {
            continue;
        }
        out.push(json!({
            "project_id": pid_str,
            "name": name,
            "weight": weight,
            "state": state,
            "revision": rev,
            "created_at_ms": created_at_ms,
        }));
    }
    Ok(out)
}

fn read_missions_conn(
    conn: &Connection,
    project_filter: Option<ProjectId>,
    allowed_projects: Option<&std::collections::HashSet<ProjectId>>,
) -> Result<Vec<Value>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT mission_id, goal_id, project_id, owner_kind, completion_policy, state, revision
             FROM missions
             ORDER BY project_id ASC, mission_id ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;
    let mut out = Vec::new();
    for item in rows {
        let (mid_str, goal_id, pid_str, owner_kind, policy, state, rev) =
            item.map_err(StoreError::from_rusqlite)?;
        let pid_opt = Uuid::parse_str(&pid_str).ok().map(ProjectId::from_uuid);
        if let Some(filter) = project_filter
            && pid_opt != Some(filter)
        {
            continue;
        }
        if let Some(allowed) = allowed_projects
            && pid_opt.is_none_or(|p| !allowed.contains(&p))
        {
            continue;
        }
        out.push(json!({
            "mission_id": mid_str,
            "goal_id": goal_id,
            "project_id": pid_str,
            "owner_kind": owner_kind,
            "completion_policy": policy,
            "state": state,
            "revision": rev,
        }));
    }
    Ok(out)
}

fn detect_v1_protocol_error(text: &str) -> Option<IpcError> {
    if text.contains(&format!("\"{PROTOCOL_VERSION_V1}\""))
        && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text)
        && map.get("protocol_version").and_then(Value::as_str) == Some(PROTOCOL_VERSION_V1)
    {
        return Some(IpcError::new(
            ErrorCode::ProtocolIncompatible,
            None,
            "Version de protocole 'unleash.ipc.v1' refusée : mise à niveau vers unleash.ipc.v2 requise",
            RetryClass::Never,
        ));
    }
    None
}

fn decode_strict_with_v1_check<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, IpcError> {
    let text = std::str::from_utf8(bytes).map_err(|err| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Trame JSON UTF-8 invalide : {err}"),
            RetryClass::Never,
        )
    })?;
    if let Some(v1_err) = detect_v1_protocol_error(text) {
        return Err(v1_err);
    }
    from_json_str_strict::<T>(text).map_err(|err| protocol_error_to_ipc_error(&err))
}

fn protocol_error_to_ipc_error(err: &ProtocolError) -> IpcError {
    match err {
        ProtocolError::ProtocolIncompatible { version } => IpcError::new(
            ErrorCode::ProtocolIncompatible,
            None,
            format!("Version de protocole incompatible : '{version}'"),
            RetryClass::Never,
        ),
        ProtocolError::Json(serde_err) => {
            let msg = serde_err.to_string();
            if msg.contains("[protocol_incompatible:") || msg.contains(PROTOCOL_VERSION_V1) {
                IpcError::new(
                    ErrorCode::ProtocolIncompatible,
                    None,
                    msg,
                    RetryClass::Never,
                )
            } else {
                IpcError::new(ErrorCode::InvalidArgument, None, msg, RetryClass::Never)
            }
        }
        other => IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            other.to_string(),
            RetryClass::Never,
        ),
    }
}

fn internal_ipc_error(msg: &str, command_id: Option<CommandId>) -> IpcError {
    let mut ipc = IpcError::new(ErrorCode::InternalFailure, None, msg, RetryClass::Never);
    if let Some(cid) = command_id {
        ipc = ipc.with_command_id(cid);
    }
    ipc
}

/// Écrit une trame JSON préfixée par sa longueur `u32` big-endian avec un délai maximal.
pub(crate) async fn write_json_frame_timeout<T: Serialize + Sync>(
    stream: &mut UnixStream,
    value: &T,
    timeout: Duration,
) -> Result<(), std::io::Error> {
    let encoded = serde_json::to_vec(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Trame de réponse trop grande ({} > {MAX_FRAME_BYTES})",
                encoded.len()
            ),
        ));
    }
    let len_u32 = u32::try_from(encoded.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Taille trame u32 dépassée",
        )
    })?;
    let header = len_u32.to_be_bytes();

    match tokio::time::timeout(timeout, async {
        stream.write_all(&header).await?;
        stream.write_all(&encoded).await?;
        stream.flush().await
    })
    .await
    {
        Ok(res) => res,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Délai dépassé lors de l'écriture sur socket Unix",
        )),
    }
}

async fn read_raw_frame_timeout(
    stream: &mut UnixStream,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, IpcError> {
    match tokio::time::timeout(timeout, read_raw_frame_no_timeout(stream, max_bytes)).await {
        Ok(res) => res,
        Err(_) => Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Délai expiré lors de la lecture d'une trame IPC",
            RetryClass::AfterBackoff,
        )),
    }
}

async fn read_raw_frame_no_timeout(
    stream: &mut UnixStream,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, IpcError> {
    let mut header = [0u8; 4];
    let mut header_read = 0_usize;
    while let Some(buf) = header.get_mut(header_read..) {
        if buf.is_empty() {
            break;
        }
        match stream.read(buf).await {
            Ok(0) => {
                if header_read == 0 {
                    return Ok(None);
                }
                return Err(IpcError::new(
                    ErrorCode::InvalidArgument,
                    None,
                    "EOF inattendu dans l'en-tête de trame IPC",
                    RetryClass::Never,
                ));
            }
            Ok(n) => header_read += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => {
                return Err(IpcError::new(
                    ErrorCode::InternalFailure,
                    None,
                    format!("Erreur de lecture socket IPC : {err}"),
                    RetryClass::Never,
                ));
            }
        }
    }

    let length = u32::from_be_bytes(header);
    let length_usize = usize::try_from(length).unwrap_or(usize::MAX);
    if length_usize > max_bytes || length_usize > MAX_FRAME_BYTES {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Trame IPC trop volumineuse : {length} > {MAX_FRAME_BYTES} octets"),
            RetryClass::Never,
        ));
    }

    let mut payload = vec![0u8; length_usize];
    if let Err(err) = stream.read_exact(&mut payload).await {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Corps de trame IPC tronqué : {err}"),
            RetryClass::Never,
        ));
    }
    Ok(Some(payload))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use unleash_domain::RequestId;

    use super::*;

    #[test]
    fn detect_v1_protocol_error_rejects_v1_and_allows_v2() {
        let v1 = r#"{"protocol_version":"unleash.ipc.v1","query_type":"GetDaemonStatus"}"#;
        let err = detect_v1_protocol_error(v1).expect("v1 doit être rejeté");
        assert_eq!(err.code, ErrorCode::ProtocolIncompatible);

        let v2 = r#"{"protocol_version":"unleash.ipc.v2","query_type":"GetDaemonStatus"}"#;
        assert!(detect_v1_protocol_error(v2).is_none());
    }

    #[test]
    fn validate_stop_drain_timeout_ms_enforces_types_and_bounds() {
        let cid = CommandId::from_u128(0x501);
        let mut p = Map::new();
        assert_eq!(validate_stop_drain_timeout_ms(&p, cid).unwrap(), None);

        p.insert(String::from("drain_timeout_ms"), Value::Null);
        assert_eq!(validate_stop_drain_timeout_ms(&p, cid).unwrap(), None);

        p.insert(String::from("drain_timeout_ms"), Value::from(450));
        assert_eq!(
            validate_stop_drain_timeout_ms(&p, cid).unwrap(),
            Some(Duration::from_millis(450))
        );

        p.insert(String::from("drain_timeout_ms"), Value::from(-5));
        assert_eq!(
            validate_stop_drain_timeout_ms(&p, cid).unwrap_err().code,
            ErrorCode::InvalidArgument
        );

        p.insert(String::from("drain_timeout_ms"), Value::from(4_000_000));
        assert_eq!(
            validate_stop_drain_timeout_ms(&p, cid).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn parse_assign_mission_fields_validates_payload() {
        let pid = ProjectId::from_u128(0x11);
        let mut payload = Map::new();
        payload.insert(String::from("completion_policy"), Value::from("human"));
        payload.insert(String::from("state"), Value::from("ready"));
        let cmd = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: CommandId::from_u128(2),
            authority_generation: AuthorityGeneration::from_u128(3),
            actor_ref: None,
            project_id: Some(pid),
            aggregate_id: Uuid::from_u128(4),
            expected_revision: Revision::INITIAL,
            command_type: String::from("AssignMission"),
            payload,
        };
        let fields = parse_assign_mission_fields(&cmd).unwrap();
        assert_eq!(fields.completion_policy, CompletionPolicy::Human);
        assert_eq!(fields.initial_state, MissionState::Ready);

        let mut bad = cmd;
        bad.payload
            .insert(String::from("state"), Value::from("unknown_state"));
        assert_eq!(
            parse_assign_mission_fields(&bad).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
