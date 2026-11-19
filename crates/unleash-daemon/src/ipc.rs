// SPDX-License-Identifier: Apache-2.0
//! Boucle serveur Unix IPC (`unleash.ipc.v2`), handshake `Hello`, barrière de récupération,
//! dispatch commandes/requêtes/abonnements et rétro-pression bornée.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
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
    AggregateKind, AlertSeverity, CommandMutation, CommitCommandRequest, CommitOutcome,
    EventAppendSpec, NewAlertSpec, ReadEventsOutcome, ReceiptInspectionOutcome, StoreError,
    UnleashStore,
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
    pub(crate) rendezvous_dir: std::path::PathBuf,
    pub(crate) terminal_children: Mutex<Vec<std::process::Child>>,
    pub(crate) broker_activity: Arc<crate::broker_activity::BrokerActivity>,
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
                () = wait_for_shutdown(&state.accepting, &state.shutdown_notify) => {
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
        let frame =
            match read_frame_until_shutdown(&mut stream, &state.accepting, &state.shutdown_notify)
                .await
            {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(ipc_err) => {
                    let _ = write_json_frame_timeout(&mut stream, &ipc_err, RESPONSE_WRITE_TIMEOUT)
                        .await;
                    break;
                }
            };

        if !dispatch_incoming_frame(&mut stream, &state, &session, &frame).await {
            return;
        }
    }
}

async fn read_frame_until_shutdown(
    stream: &mut UnixStream,
    accepting: &AtomicBool,
    shutdown: &Notify,
) -> Result<Option<Vec<u8>>, IpcError> {
    tokio::select! {
        biased;
        () = wait_for_shutdown(accepting, shutdown) => Ok(None),
        result = read_raw_frame_no_timeout(stream, MAX_FRAME_BYTES) => result,
    }
}

async fn wait_for_shutdown(accepting: &AtomicBool, shutdown: &Notify) {
    loop {
        let notified = shutdown.notified();
        tokio::pin!(notified);
        // Register before checking: notify_waiters does not store a permit for
        // a future waiter. Both a stop before this check and one after it wake us.
        notified.as_mut().enable();
        if !accepting.load(Ordering::Acquire) {
            return;
        }
        notified.await;
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

    let hello_raw = zeroize::Zeroizing::new(
        match read_raw_frame_timeout(stream, MAX_FRAME_BYTES, HANDSHAKE_IO_TIMEOUT).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(ipc_err) => {
                let _ = write_json_frame_timeout(stream, &ipc_err, RESPONSE_WRITE_TIMEOUT).await;
                return None;
            }
        },
    );

    let hello_req = match decode_strict_with_v1_check::<HelloRequest>(&hello_raw) {
        Ok(req) => req,
        Err(mut ipc_err) => {
            ipc_err.message = String::from("Handshake Hello invalide");
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
            Capability::Administer => "administer",
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
    if !query.payload.is_empty()
        && matches!(
            query.query_type.as_str(),
            "GetDaemonStatus" | "GetHealth" | "GetStateSnapshot"
        )
    {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Payload de requête invalide",
            RetryClass::Never,
        ));
    }
    match query.query_type.as_str() {
        "GetDaemonStatus" => query_daemon_status(state, session, query.request_id).await,
        "GetHealth" => query_health(state, query.request_id).await,
        "GetStateSnapshot" => query_state_snapshot(state, session, query).await,
        "GetCommandReceipt" => query_command_receipt(state, session, query).await,
        "GetBackupManifest" => query_backup_manifest(state, session, query).await,
        "ReadTerminalLog" => crate::terminal_service::read_log(state, session, query).await,
        "GetProviderProfile" => {
            crate::provider_profile_validation_service::query(
                Arc::clone(&state.store),
                session.clone(),
                query,
            )
            .await
        }
        "ListProjects" | "GetProject" | "ReadSettings" | "ReadPolicySummary"
        | "ReadGuardHealth" | "InspectRecovery" => query_m1_projection(state, session, query).await,
        other => Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            format!("Type de requête non supporté : '{other}'"),
            RetryClass::Never,
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupManifestQuery {
    backup_id: unleash_domain::BackupId,
}

async fn query_backup_manifest(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    if session.allowed_projects.is_some() {
        return Err(IpcError::from_reason(
            ReasonCode::AccessDenied,
            "Projection globale hors du périmètre autorisé",
        ));
    }
    let input: BackupManifestQuery =
        serde_json::from_value(Value::Object(query.payload)).map_err(|_| invalid_projection())?;
    if query.project_id.is_some() || input.backup_id.as_uuid().is_nil() {
        return Err(invalid_projection());
    }
    let store = Arc::clone(&state.store);
    let snapshot =
        tokio::task::spawn_blocking(move || store.inspect_backup_request_snapshot(input.backup_id))
            .await
            .map_err(|_| internal_ipc_error("Lecture de sauvegarde interrompue", None))?
            .map_err(|error| store_error_to_ipc_error(&error, None))?;
    let record = snapshot.value.ok_or_else(|| {
        IpcError::new(
            ErrorCode::NotFoundOrHidden,
            None,
            "Sauvegarde introuvable ou masquée",
            RetryClass::Never,
        )
    })?;
    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(snapshot.cursor),
        result: json!({
            "backup_id": record.backup_id,
            "authority_generation": record.authority_generation,
            "lineage_status": if record.authority_generation == state.authority_generation { "current" } else { "historical" },
            "state": record.state,
            "action_state": record.action_state,
            "blocked_reason": record.block_reason,
            "manifest": record.manifest,
            "manifest_digest": record.manifest_digest,
        }),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsQueryPayload {
    scope_kind: unleash_store::SettingsScopeKind,
    scope_id: String,
}

fn projection_response<T: Serialize>(
    projection: unleash_store::SnapshotProjection<T>,
) -> Result<(Value, EventCursor), IpcError> {
    let value = serde_json::to_value(projection.value)
        .map_err(|_| internal_ipc_error("Sérialisation de projection impossible", None))?;
    Ok((value, projection.cursor))
}

fn invalid_projection() -> IpcError {
    IpcError::new(
        ErrorCode::InvalidArgument,
        None,
        "Paramètres de projection invalides",
        RetryClass::Never,
    )
}

fn retain_unverified_guard_health(guard: &mut unleash_store::GuardConfigRecord) {
    guard.health_status = unleash_store::GuardHealthStatus::Unknown;
    guard.negotiated_version = None;
    guard.capabilities.clear();
    guard.last_health_check_ms = None;
    if guard.guard_mapping_profile != "none_accepted" {
        guard.guard_mapping_profile = String::from("unverified");
    }
}

fn read_guard_health(store: &UnleashStore) -> Result<(Value, EventCursor), IpcError> {
    let snapshot = store
        .read_policy_summary_snapshot(None)
        .map_err(|e| store_error_to_ipc_error(&e, None))?;
    let value = snapshot.value.guard_config.map(|mut guard| {
        retain_unverified_guard_health(&mut guard);
        json!({
            "guard_mode": guard.guard_mode, "guard_mapping_profile": guard.guard_mapping_profile,
            "health_status": "unknown", "negotiated_version": null,
            "capabilities": [], "last_health_check_ms": null,
        })
    });
    projection_response(unleash_store::SnapshotProjection {
        value,
        cursor: snapshot.cursor,
    })
}

fn parse_settings_query(
    query: &QueryEnvelope,
    restricted: bool,
) -> Result<SettingsQueryPayload, IpcError> {
    let parsed: SettingsQueryPayload = serde_json::from_value(Value::Object(query.payload.clone()))
        .map_err(|_| invalid_projection())?;
    match parsed.scope_kind {
        unleash_store::SettingsScopeKind::Project
            if query
                .project_id
                .is_some_and(|p| p.to_string() == parsed.scope_id) => {}
        unleash_store::SettingsScopeKind::User
            if query.project_id.is_none() && parsed.scope_id == "default" =>
        {
            if restricted {
                return Err(IpcError::from_reason(
                    ReasonCode::AccessDenied,
                    "Projection hors du périmètre autorisé",
                ));
            }
        }
        _ => return Err(invalid_projection()),
    }
    Ok(parsed)
}

async fn query_m1_projection(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    let invalid = invalid_projection;
    let denied = || {
        IpcError::from_reason(
            ReasonCode::AccessDenied,
            "Projection hors du périmètre autorisé",
        )
    };
    let name = query.query_type.clone();
    let project_id = query.project_id;
    let allowed = session.allowed_projects.clone();
    let settings = if name == "ReadSettings" {
        Some(parse_settings_query(&query, allowed.is_some())?)
    } else {
        if !query.payload.is_empty() {
            return Err(invalid());
        }
        if name == "GetProject" && project_id.is_none() {
            return Err(invalid());
        }
        if matches!(name.as_str(), "InspectRecovery" | "ReadGuardHealth") && project_id.is_some() {
            return Err(invalid());
        }
        // Ces rapports contiennent des données globales non filtrables par projet.
        if allowed.is_some()
            && matches!(
                name.as_str(),
                "ReadPolicySummary" | "ReadGuardHealth" | "InspectRecovery"
            )
        {
            return Err(denied());
        }
        None
    };
    let store = Arc::clone(&state.store);
    let (value, cursor) =
        tokio::task::spawn_blocking(move || -> Result<(Value, EventCursor), IpcError> {
            let map_error = |e: StoreError| store_error_to_ipc_error(&e, None);
            match name.as_str() {
                "ListProjects" => {
                    let mut snapshot = store.list_projects_snapshot().map_err(map_error)?;
                    snapshot.value.retain(|p| {
                        project_id.is_none_or(|id| id == p.project_id)
                            && allowed
                                .as_ref()
                                .is_none_or(|ids| ids.contains(&p.project_id))
                    });
                    projection_response(snapshot)
                }
                "GetProject" => projection_response(
                    store
                        .get_project_snapshot(project_id.ok_or_else(invalid)?)
                        .map_err(map_error)?,
                ),
                "ReadSettings" => {
                    let settings = settings.ok_or_else(invalid)?;
                    projection_response(
                        store
                            .read_settings_snapshot(settings.scope_kind, &settings.scope_id)
                            .map_err(map_error)?,
                    )
                }
                "ReadPolicySummary" => {
                    let mut snapshot = store
                        .read_policy_summary_snapshot(project_id)
                        .map_err(map_error)?;
                    if let Some(guard) = &mut snapshot.value.guard_config {
                        retain_unverified_guard_health(guard);
                    }
                    projection_response(snapshot)
                }
                "ReadGuardHealth" => read_guard_health(&store),
                "InspectRecovery" => {
                    projection_response(store.inspect_recovery_snapshot().map_err(map_error)?)
                }
                _ => Err(invalid()),
            }
        })
        .await
        .map_err(|_| internal_ipc_error("Lecture de projection interrompue", None))??;
    Ok(QueryResponse {
        request_id: query.request_id,
        watermark: Some(cursor),
        result: value,
    })
}

async fn query_daemon_status(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    request_id: unleash_domain::RequestId,
) -> Result<QueryResponse, IpcError> {
    if !session.capabilities.contains(&Capability::Administer) {
        return Ok(QueryResponse {
            request_id,
            watermark: None,
            result: json!({"mode": state.current_mode().to_string()}),
        });
    }
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
    let terminal_actor = session.actor_id.clone();
    let (snapshot_val, watermark) = tokio::task::spawn_blocking(move || {
        store.read_snapshot(|conn, _ctx| {
            let daemon_row = read_daemon_row_conn(conn)?;
            let wm = read_watermark_conn(conn, daemon_row.stream_id)?;
            let projects = read_projects_conn(conn, proj_filter, allowed_projects.as_ref())?;
            let missions = read_missions_conn(conn, proj_filter, allowed_projects.as_ref())?;
            let terminals = crate::terminal_service::read_terminal_metadata(
                conn,
                &terminal_actor,
                proj_filter,
                allowed_projects.as_ref(),
            )?;
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
                "terminals": terminals,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptQueryPayload {
    command_id: CommandId,
    #[serde(default, deserialize_with = "deserialize_acquisition_reference")]
    result_ref: Option<String>,
}

fn deserialize_acquisition_reference<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let reference = String::deserialize(deserializer)?;
    if reference.is_empty() || reference.len() > 128 || reference.chars().any(char::is_control) {
        return Err(serde::de::Error::custom(
            "invalid acquisition result reference",
        ));
    }
    Ok(Some(reference))
}

async fn query_terminal_acquisition_result(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    request_id: unleash_domain::RequestId,
    project_id: Option<unleash_domain::ProjectId>,
    command: CommandId,
    reference: String,
) -> Result<QueryResponse, IpcError> {
    let project = project_id.ok_or_else(|| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Acquisition result requires its exact project",
            RetryClass::Never,
        )
    })?;
    let store = Arc::clone(&state.store);
    let actor = session.actor_id.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        store.inspect_terminal_acquisition_result(&actor, command, &reference, project)
    })
    .await
    .map_err(|_| internal_ipc_error("Acquisition result inspection task failed", Some(command)))?
    .map_err(|error| store_error_to_ipc_error(&error, Some(command)))?;
    let result = serde_json::to_value(snapshot.value).map_err(|_| {
        internal_ipc_error("Acquisition result serialization failed", Some(command))
    })?;
    Ok(QueryResponse {
        request_id,
        watermark: Some(snapshot.cursor),
        result,
    })
}

async fn query_command_receipt(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    query: QueryEnvelope,
) -> Result<QueryResponse, IpcError> {
    // Le project_id de la requête est déclaré par le client, pas la provenance du reçu.
    // Sans provenance vérifiée par le Store, refuser aussi les reçus historiques.
    if session.allowed_projects.is_some() {
        return Err(IpcError::from_reason(
            ReasonCode::AccessDenied,
            "Provenance du reçu non vérifiée pour cette portée",
        ));
    }
    let payload: ReceiptQueryPayload = serde_json::from_value(Value::Object(query.payload))
        .map_err(|_| {
            IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Payload GetCommandReceipt invalide",
                RetryClass::Never,
            )
        })?;
    let cmd_id = payload.command_id;
    if let Some(reference) = payload.result_ref {
        return query_terminal_acquisition_result(
            state,
            session,
            query.request_id,
            query.project_id,
            cmd_id,
            reference,
        )
        .await;
    }
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

fn is_provider_profile_command(command: &CommandEnvelope) -> bool {
    matches!(
        command.command_type.as_str(),
        "SetProviderProfile" | "TestProviderProfile"
    )
}

/// Retourne `true` si le hook `PostCommitAction::DropConnection` demande de couper la connexion sans réponse.
#[expect(
    clippy::too_many_lines,
    reason = "Admission, commit, réconciliation et réponse IPC ordonnés"
)]
async fn handle_command_frame(
    stream: &mut UnixStream,
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> bool {
    if let Err(mut ipc) = authorize_command(session, &cmd) {
        if !audit_security_denial(state, session, &cmd, &ipc).await {
            ipc.message = String::from("Commande refusée ; audit indisponible ou indéterminé");
            tracing::warn!("Échec ou délai dépassé lors de l'audit du refus de commande");
        }
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

    let (outcome, num_events, mut stop_reason, mut stop_drain_timeout) =
        if is_provider_profile_command(&cmd) {
            let committed = if cmd.command_type == "SetProviderProfile" {
                crate::provider_profile_service::commit(
                    Arc::clone(&state.store),
                    session.clone(),
                    cmd.clone(),
                )
                .await
            } else {
                crate::provider_profile_validation_service::commit(
                    Arc::clone(&state.store),
                    session.clone(),
                    cmd.clone(),
                )
                .await
            };
            let outcome = match committed {
                Ok(outcome) => outcome,
                Err(error) => {
                    let _ = write_json_frame_timeout(stream, &error, RESPONSE_WRITE_TIMEOUT).await;
                    return false;
                }
            };
            (outcome, 1, None, None)
        } else {
            let mut prepared = match prepare_command_request(state, session, cmd.clone()).await {
                Ok(p) => p,
                Err(ipc) => {
                    let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
                    return false;
                }
            };

            if matches!(
                prepared.req.mutation,
                CommandMutation::CreateMission { .. }
                    | CommandMutation::RequestBackup { .. }
                    | CommandMutation::PrepareTerminalAllocation { .. }
                    | CommandMutation::PrepareTerminalControl { .. }
            ) && let Err(ipc) = ensure_principal_registered(&state.store, session).await
            {
                let _ = write_json_frame_timeout(stream, &ipc, RESPONSE_WRITE_TIMEOUT).await;
                return false;
            }

            let num_events = prepared.req.events.len();
            let store_ref = Arc::clone(&state.store);
            let req_to_commit = prepared.req;
            let outcome = match tokio::task::spawn_blocking(move || {
                store_ref.commit_command(req_to_commit)
            })
            .await
            {
                Ok(Ok(o)) => o,
                Ok(Err(err)) => {
                    let mut ipc = store_error_to_ipc_error(&err, Some(cmd.command_id));
                    if matches!(cmd.command_type.as_str(), "PurgeHistory" | "RequestBackup")
                        && matches!(
                            err,
                            StoreError::StorageBusy(_)
                                | StoreError::StorageFull(_)
                                | StoreError::StorageUnavailable(_)
                                | StoreError::StorageCorrupt(_)
                                | StoreError::Io(_)
                                | StoreError::CutpointInjected { .. }
                        )
                    {
                        // Le Store peut avoir committé avant une erreur de finalisation physique.
                        // La recherche ci-dessous ne change pas la réponse en succès.
                        publish_existing_receipt_cursor(state, session, cmd.command_id).await;
                        if cmd.command_type == "RequestBackup" {
                            reconcile_accepted_backup(state, session, &cmd).await;
                        }
                        ipc.retry_class = RetryClass::AfterReconciliation;
                        ipc.message = String::from(
                            "Issue de commande indéterminée ; inspecter le reçu et la récupération",
                        );
                    }
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
            (
                outcome,
                num_events,
                prepared.stop_reason.take(),
                prepared.stop_drain_timeout.take(),
            )
        };

    let (receipt, newly_committed) = match outcome {
        CommitOutcome::Committed(r) => (r, true),
        CommitOutcome::IdempotentReplay(r) => (r, false),
    };

    if newly_committed || is_provider_profile_command(&cmd) {
        publish_committed_events_for_command(state, &receipt, num_events).await;
    }

    if cmd.command_type == "RequestBackup" {
        crate::backup_service::start(state, unleash_domain::BackupId::from_uuid(cmd.aggregate_id));
    }
    if cmd.command_type == "AcquireTerminal" {
        if cmd.payload.contains_key("terminal_id") {
            if newly_committed {
                crate::terminal_service::start_registered(
                    state,
                    session.actor_id.clone(),
                    cmd.command_id,
                );
            }
        } else {
            crate::terminal_service::start(
                state,
                unleash_domain::TerminalId::from_uuid(cmd.aggregate_id),
            );
        }
    }

    if newly_committed && let Some(stop_reason) = stop_reason.take() {
        state.mark_draining();
        let _ = state.stop_tx.try_send(StopRequest {
            reason: stop_reason,
            drain_timeout_override: stop_drain_timeout.take(),
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

async fn reconcile_accepted_backup(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: &CommandEnvelope,
) {
    let store = Arc::clone(&state.store);
    let actor = session.actor_id.clone();
    let command_id = cmd.command_id;
    let backup_id = unleash_domain::BackupId::from_uuid(cmd.aggregate_id);
    let generation = state.authority_generation;
    let attested = tokio::task::spawn_blocking(move || {
        let ReceiptInspectionOutcome::Full(receipt) =
            store.inspect_command_receipt(&actor, command_id).ok()?
        else {
            return None;
        };
        let record = store
            .inspect_backup_request_snapshot(backup_id)
            .ok()?
            .value?;
        Some(
            receipt.lineage_status == unleash_protocol::LineageStatus::Current
                && receipt.authority_generation == generation
                && receipt.receipt.command_id == command_id
                && receipt.receipt.aggregate_id == backup_id.as_uuid()
                && receipt.receipt.state == ReceiptState::Accepted
                && record.receipt_id == receipt.receipt.receipt_id
                && record.actor_id == actor
                && record.authority_generation == generation
                && record.state == unleash_store::BackupRequestState::Reserved,
        )
    })
    .await;
    if matches!(attested, Ok(Some(true))) {
        crate::backup_service::start(state, backup_id);
    }
}

async fn publish_existing_receipt_cursor(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    command_id: CommandId,
) {
    let store = Arc::clone(&state.store);
    let actor = session.actor_id.clone();
    if let Ok(Ok(ReceiptInspectionOutcome::Full(inspection))) =
        tokio::task::spawn_blocking(move || store.inspect_command_receipt(&actor, command_id)).await
        && inspection.lineage_status == unleash_protocol::LineageStatus::Current
        && inspection.receipt.event_cursor.stream_id == state.stream_id
    {
        publish_committed_events_for_command(state, &inspection.receipt, 1).await;
    }
}

async fn audit_security_denial(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    cmd: &CommandEnvelope,
    error: &IpcError,
) -> bool {
    let reason = match error.code {
        ErrorCode::AuthenticationFailed => "AUTHENTICATION_FAILED",
        ErrorCode::CapabilityDenied => "OPERATION_NOT_AUTHORIZED",
        ErrorCode::ScopeDenied => "ACCESS_DENIED",
        _ => return true,
    };
    let Ok(operation) = cmd.command_type.parse::<unleash_protocol::OperationName>() else {
        return true;
    };
    if operation.kind() != unleash_protocol::OperationKind::Command {
        return true;
    }
    let Ok(digest) = unleash_protocol::canonical_sha256(&json!({
        "actor": session.actor_id.as_str(), "operation": operation.as_str(),
        "command_id": cmd.command_id, "reason": reason,
    })) else {
        return false;
    };
    let digest = digest.to_string();
    let spec = NewAlertSpec {
        alert_id: format!("security-denial:{digest}"),
        kind: String::from("security_denial"),
        severity: AlertSeverity::Warning,
        blocking: false,
        project_id: None,
        subject_kind: operation.as_str().to_owned(),
        subject_id: digest,
        title: String::from("Commande refusée"),
        summary: String::from("Une commande a été refusée par le contrôle d'autorité local."),
        reason_code: Some(reason.to_owned()),
        evidence_ref: None,
        allowed_operations: Vec::new(),
    };
    let store = Arc::clone(&state.store);
    matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || store.record_alert(spec))
        )
        .await,
        Ok(Ok(Ok(_)))
    )
}

struct PreparedCommand {
    req: CommitCommandRequest,
    stop_reason: Option<String>,
    stop_drain_timeout: Option<Duration>,
}

pub(crate) async fn ensure_principal_registered(
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
    validate_known_command_payload(&cmd)?;
    match cmd.command_type.as_str() {
        "CreateProject" => prepare_create_project(session, cmd),
        "ReviseProject" => prepare_revise_project(session, cmd),
        "PurgeHistory" => prepare_purge_history(session, cmd),
        "RequestBackup" => prepare_request_backup(session, cmd),
        "AcquireTerminal" => Ok(PreparedCommand {
            req: crate::terminal_service::prepare_acquisition(state, session, cmd).await?,
            stop_reason: None,
            stop_drain_timeout: None,
        }),
        "SetSettings" | "SetPolicy" | "ConfigureGuard" => {
            let mut prepared = prepare_admin_command(state, session, cmd.clone())?;
            if let Some(pins) = crate::terminal_service::prepare_configuration_objects(
                state,
                &session.actor_id,
                &cmd,
            )
            .await?
                && let CommandMutation::SetSettings { patch, .. } = &mut prepared.req.mutation
            {
                patch.extra_typed.get_or_insert_with(Map::new).insert(
                    "__terminal_prepared".into(),
                    serde_json::to_value(pins).map_err(|_| {
                        internal_ipc_error("Terminal preparation serialization", Some(cid))
                    })?,
                );
            }
            Ok(prepared)
        }
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

fn prepare_request_backup(
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    validate_request_backup_command(&cmd)?;
    let backup_id = unleash_domain::BackupId::from_uuid(cmd.aggregate_id);
    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::BackupJob,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::RequestBackup { backup_id },
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

fn validate_request_backup_command(cmd: &CommandEnvelope) -> Result<(), IpcError> {
    if !cmd.payload.is_empty() || cmd.project_id.is_some() || cmd.aggregate_id.is_nil() {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Paramètres de sauvegarde invalides",
            RetryClass::Never,
        )
        .with_command_id(cmd.command_id));
    }
    if cmd.expected_revision != Revision::INITIAL {
        return Err(store_error_to_ipc_error(
            &StoreError::RevisionConflict {
                current_revision: Revision::INITIAL,
                expected_revision: cmd.expected_revision,
                command_id: Some(cmd.command_id),
            },
            Some(cmd.command_id),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignMissionPayload {
    #[serde(rename = "completion_policy")]
    _completion_policy: Option<String>,
    #[serde(rename = "state")]
    _state: Option<String>,
    #[serde(rename = "goal_id")]
    _goal_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MissionTransitionPayload {
    #[serde(rename = "reason")]
    _reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopDaemonPayload {
    #[serde(rename = "reason")]
    _reason: Option<String>,
    #[serde(rename = "drain_timeout_ms")]
    _drain_timeout_ms: Option<u64>,
}

fn validate_known_command_payload(cmd: &CommandEnvelope) -> Result<(), IpcError> {
    let payload = Value::Object(cmd.payload.clone());
    let valid = match cmd.command_type.as_str() {
        "AssignMission" => serde_json::from_value::<AssignMissionPayload>(payload).is_ok(),
        "PauseMission" | "ResumeMission" | "CancelMission" => {
            serde_json::from_value::<MissionTransitionPayload>(payload).is_ok()
        }
        "StopDaemon" => serde_json::from_value::<StopDaemonPayload>(payload).is_ok(),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Payload de commande invalide",
            RetryClass::Never,
        )
        .with_command_id(cmd.command_id))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PurgeHistoryPayload {
    scope_kind: String,
    scope_id: String,
    reason: String,
    older_than_override_ms: Option<TimestampMs>,
}

fn prepare_purge_history(
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Périmètre ou paramètres de purge invalides",
            RetryClass::Never,
        )
        .with_command_id(cid)
    };
    let payload: PurgeHistoryPayload =
        serde_json::from_value(Value::Object(cmd.payload.clone())).map_err(|_| invalid())?;
    // Le Store courant sélectionne globalement ; ne pas présenter cette opération comme limitée à un projet.
    if payload.scope_kind != "user"
        || payload.scope_id != "default"
        || cmd.project_id.is_some()
        || cmd.aggregate_id.is_nil()
        || payload.reason.trim().is_empty()
        || payload.reason.len() > 1024
    {
        return Err(invalid());
    }
    if cmd.expected_revision != Revision::INITIAL {
        return Err(store_error_to_ipc_error(
            &StoreError::RevisionConflict {
                current_revision: Revision::INITIAL,
                expected_revision: cmd.expected_revision,
                command_id: Some(cid),
            },
            Some(cid),
        ));
    }
    let request = unleash_store::RetentionPurgeRequest {
        scope_kind: payload.scope_kind,
        scope_id: payload.scope_id,
        requested_by_actor: session.actor_id.as_str().to_owned(),
        reason: payload.reason,
        older_than_override_ms: payload.older_than_override_ms,
        is_automatic_sweep: false,
    };
    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: AggregateKind::RetentionPurge,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::PurgeHistory { request },
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSettingsPayload {
    scope_kind: unleash_store::SettingsScopeKind,
    scope_id: String,
    patch: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPolicyPayload {
    scope_kind: unleash_store::PolicyScopeKind,
    scope_id: String,
    isolation_mode: unleash_store::PolicyIsolationMode,
    policy_version: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureGuardPayload {
    config: Value,
}

fn has_only_keys(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| keys.contains(&key.as_str())))
}

fn strict_settings_patch(mut value: Value) -> Option<unleash_store::SettingsPatch> {
    if !has_only_keys(
        &value,
        &[
            "execution_pool",
            "retention_policy",
            "confirm_retention_shortening",
            "analyzer_paths",
            "terminal_runtime",
        ],
    ) {
        return None;
    }
    let object = value.as_object()?;
    if let Some(pool) = object.get("execution_pool").filter(|v| !v.is_null())
        && !has_only_keys(
            pool,
            &[
                "max_active_executions",
                "control_reserve_mode",
                "control_reserved_units",
                "control_burst_limit",
                "checkpoint_escalation_s",
                "project_weight",
            ],
        )
    {
        return None;
    }
    if let Some(policy) = object.get("retention_policy").filter(|v| !v.is_null())
        && !has_only_keys(
            policy,
            &[
                "policy_version",
                "command_receipts_min_days",
                "events_min_days",
                "model_payloads_days",
                "tool_terminal_logs_days",
                "browser_artifacts_days",
                "dirty_captures_days",
                "automation_data_days",
                "document_copies_days",
                "export_artifacts_days",
                "logs_telemetry_days",
                "local_backups_daily",
                "local_backups_weekly",
                "max_backup_rotation_days",
            ],
        )
    {
        return None;
    }
    // Les chemins d'analyseurs ne sont pas encore validés jusqu'au binaire/source effectif.
    if let Some(paths) = object.get("analyzer_paths").filter(|v| !v.is_null())
        && !paths.as_array().is_some_and(Vec::is_empty)
    {
        return None;
    }
    let terminal = value.as_object_mut()?.remove("terminal_runtime");
    if terminal.is_some() {
        value
            .as_object_mut()?
            .entry("confirm_retention_shortening")
            .or_insert(Value::Bool(false));
    }
    let mut patch: unleash_store::SettingsPatch = serde_json::from_value(value).ok()?;
    if let Some(raw) = terminal {
        let typed: unleash_store::terminal_allocations::TerminalRuntimeSettings =
            serde_json::from_value(raw).ok()?;
        typed.validate().ok()?;
        patch.extra_typed = Some(Map::from_iter([(
            "terminal_runtime".into(),
            serde_json::to_value(typed).ok()?,
        )]));
    }
    Some(patch)
}

fn prepare_guard_mutation(
    state: &DaemonRuntimeState,
    cmd: &CommandEnvelope,
    raw: Value,
    caller: unleash_store::CallerAuthority,
) -> Result<(AggregateKind, CommandMutation), IpcError> {
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Commande administrative invalide ou non prise en charge",
            RetryClass::Never,
        )
        .with_command_id(cmd.command_id)
    };
    let payload: ConfigureGuardPayload = serde_json::from_value(raw).map_err(|_| invalid())?;
    if cmd.project_id.is_some()
        || cmd.aggregate_id != state.authority_generation.as_uuid()
        || !has_only_keys(
            &payload.config,
            &[
                "config_id",
                "guard_mode",
                "endpoint",
                "transport",
                "secret_ref_id",
                "evaluate_timeout_ms",
                "health_check_interval_s",
                "decision_cache_max_age_s",
                "fallback_policy_ref",
                "guard_mapping_profile",
                "reason",
            ],
        )
    {
        return Err(invalid());
    }
    let config: unleash_store::GuardConfigInput =
        serde_json::from_value(payload.config).map_err(|_| invalid())?;
    if config.config_id != "default"
        || config.guard_mode != unleash_store::GuardPolicyMode::BrokerOnly
        || config.endpoint.is_some()
        || config.transport != unleash_store::GuardTransport::None
        || config.secret_ref_id.is_some()
        || config.fallback_policy_ref.is_some()
        || config.guard_mapping_profile != "none_accepted"
        || config.reason.len() > 1024
    {
        return Err(invalid());
    }
    Ok((
        AggregateKind::GuardConfig,
        CommandMutation::ConfigureGuard { config, caller },
    ))
}

fn prepare_admin_command(
    state: &DaemonRuntimeState,
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Commande administrative invalide ou non prise en charge",
            RetryClass::Never,
        )
        .with_command_id(cid)
    };
    let caller = unleash_store::CallerAuthority {
        actor_id: session.actor_id.clone(),
        role: unleash_store::CallerRole::PolicyAdmin,
    };
    let raw = Value::Object(cmd.payload.clone());
    let (kind, mutation) = match cmd.command_type.as_str() {
        "SetSettings" => {
            let payload: SetSettingsPayload = serde_json::from_value(raw).map_err(|_| invalid())?;
            match payload.scope_kind {
                unleash_store::SettingsScopeKind::User
                    if payload.scope_id == "default"
                        && cmd.project_id.is_none()
                        && cmd.aggregate_id == state.authority_generation.as_uuid() => {}
                unleash_store::SettingsScopeKind::Project
                    if cmd.project_id.is_some_and(|id| {
                        id.to_string() == payload.scope_id && id.as_uuid() == cmd.aggregate_id
                    }) => {}
                _ => return Err(invalid()),
            }
            let patch = strict_settings_patch(payload.patch).ok_or_else(invalid)?;
            (
                AggregateKind::Settings,
                CommandMutation::SetSettings {
                    scope_kind: payload.scope_kind,
                    scope_id: payload.scope_id,
                    caller,
                    patch,
                },
            )
        }
        "SetPolicy" => {
            let payload: SetPolicyPayload = serde_json::from_value(raw).map_err(|_| invalid())?;
            match payload.scope_kind {
                unleash_store::PolicyScopeKind::Global
                    if payload.scope_id == "global"
                        && cmd.project_id.is_none()
                        && cmd.aggregate_id == state.authority_generation.as_uuid() => {}
                unleash_store::PolicyScopeKind::Project
                    if cmd.project_id.is_some_and(|id| {
                        id.to_string() == payload.scope_id && id.as_uuid() == cmd.aggregate_id
                    }) => {}
                _ => return Err(invalid()),
            }
            if payload.reason.len() > 1024 || payload.policy_version.len() > 128 {
                return Err(invalid());
            }
            (
                AggregateKind::SecurityPolicy,
                CommandMutation::SetPolicy {
                    scope_kind: payload.scope_kind,
                    scope_id: payload.scope_id,
                    isolation_mode: payload.isolation_mode,
                    policy_version: payload.policy_version,
                    reason: payload.reason,
                    caller,
                },
            )
        }
        "ConfigureGuard" => prepare_guard_mutation(state, &cmd, raw, caller)?,
        _ => return Err(invalid()),
    };
    Ok(PreparedCommand {
        req: CommitCommandRequest {
            actor_id: session.actor_id.clone(),
            envelope: cmd,
            target_aggregate: kind,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation,
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviseProjectPayload {
    name: String,
    description: String,
    weight: i64,
    primary_root_path: Option<String>,
    sources: Vec<StrictProjectSource>,
    repositories: Vec<StrictProjectRepository>,
    settings_json: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictProjectSource {
    kind: String,
    path_or_uri: String,
    label: String,
    read_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictProjectRepository {
    repo_id: String,
    root_path: String,
    default_branch: Option<String>,
}

fn prepare_revise_project(
    session: &AuthenticatedSession,
    cmd: CommandEnvelope,
) -> Result<PreparedCommand, IpcError> {
    let cid = cmd.command_id;
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "ReviseProject : payload ou cible invalide",
            RetryClass::Never,
        )
        .with_command_id(cid)
    };
    let project_id = cmd.project_id.ok_or_else(invalid)?;
    if project_id.as_uuid() != cmd.aggregate_id {
        return Err(invalid());
    }
    let payload: ReviseProjectPayload =
        serde_json::from_value(Value::Object(cmd.payload.clone())).map_err(|_| invalid())?;
    // Sans schéma de paramètres versionné ni validation des URI distantes, refuser ces entrées.
    if !payload.settings_json.is_empty()
        || payload
            .sources
            .iter()
            .any(|s| !matches!(s.kind.as_str(), "directory" | "git" | "local"))
    {
        return Err(invalid());
    }
    let sources = payload
        .sources
        .into_iter()
        .map(|s| unleash_store::ProjectSourceSpec {
            kind: s.kind,
            path_or_uri: s.path_or_uri,
            label: s.label,
            read_only: s.read_only,
        })
        .collect();
    let repositories = payload
        .repositories
        .into_iter()
        .map(|r| unleash_store::ProjectRepositorySpec {
            repo_id: r.repo_id,
            root_path: r.root_path,
            default_branch: r.default_branch,
        })
        .collect();
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
            mutation: CommandMutation::ReviseProject {
                project_id,
                name: payload.name,
                description: payload.description,
                weight: payload.weight,
                primary_root_path: payload.primary_root_path,
                sources,
                repositories,
                settings_json: Value::Object(payload.settings_json),
            },
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        },
        stop_reason: None,
        stop_drain_timeout: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProjectPayload {
    name: Option<String>,
    weight: Option<i64>,
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
    let payload: CreateProjectPayload = serde_json::from_value(Value::Object(cmd.payload.clone()))
        .map_err(|_| {
            IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Payload CreateProject invalide",
                RetryClass::Never,
            )
            .with_command_id(cid)
        })?;
    let name = payload
        .name
        .unwrap_or_else(|| format!("project-{project_id}"));
    if name.trim().is_empty() {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "CreateProject : name ne peut pas être vide",
            RetryClass::Never,
        )
        .with_command_id(cid));
    }
    let weight = payload.weight.unwrap_or(1);
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
        Some(_) => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "completion_policy inconnue",
                RetryClass::Never,
            )
            .with_command_id(cid));
        }
    };
    let initial_state = match cmd.payload.get("state").and_then(Value::as_str) {
        None | Some("active") => MissionState::Active,
        Some("proposed") => MissionState::Proposed,
        Some("ready") => MissionState::Ready,
        Some(_) => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "État initial de mission invalide pour AssignMission",
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

pub(crate) async fn read_raw_frame_timeout(
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

    // REQ-CORE-007 / T-F001-007: establish an already-pending real UDS read
    // before stopping; neither a frozen select guard nor a lost wake may hang it.
    #[tokio::test]
    async fn already_idle_connection_closes_when_shutdown_is_notified() {
        let (_client, mut server) = UnixStream::pair().unwrap();
        let accepting = AtomicBool::new(true);
        let shutdown = Notify::new();
        {
            let read = read_frame_until_shutdown(&mut server, &accepting, &shutdown);
            tokio::pin!(read);
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(read.as_mut(), &mut context).is_pending());
            accepting.store(false, Ordering::Release);
            shutdown.notify_waiters();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), read).await,
                Ok(Ok(None))
            ));
        }
        // A notification before the future is armed must also be harmless.
        shutdown.notify_waiters();
        assert!(matches!(
            tokio::time::timeout(
                Duration::from_secs(1),
                read_frame_until_shutdown(&mut server, &accepting, &shutdown)
            )
            .await,
            Ok(Ok(None))
        ));
    }

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
        bad.payload
            .insert(String::from("state"), Value::from("DO_NOT_ECHO"));
        assert!(
            !parse_assign_mission_fields(&bad)
                .unwrap_err()
                .message
                .contains("DO_NOT_ECHO")
        );
        for kind in [
            "AssignMission",
            "PauseMission",
            "ResumeMission",
            "CancelMission",
            "StopDaemon",
        ] {
            bad.command_type = kind.into();
            bad.payload = serde_json::from_value(json!({"unexpected":"DO_NOT_ECHO"})).unwrap();
            let err = validate_known_command_payload(&bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
            assert!(!err.message.contains("DO_NOT_ECHO"));
        }
        bad.command_type = "AssignMission".into();
        bad.payload = serde_json::from_value(json!({"state":123})).unwrap();
        assert_eq!(
            validate_known_command_payload(&bad).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
