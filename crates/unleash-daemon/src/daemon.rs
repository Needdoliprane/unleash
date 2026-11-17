// SPDX-License-Identifier: Apache-2.0
//! Point d'entrée de composition et cycle de vie complet du daemon Unleash (`unleashd`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Notify, Semaphore, mpsc, watch};
use unleash_browser::LocalInventorySearchPaths;
use unleash_domain::{
    AuthorityGeneration, DaemonEpoch, DaemonInstanceId, DaemonMode, Revision, StreamId,
    transition_daemon_mode,
};
use unleash_platform::{
    AuthorityLock, PlatformError, PlatformPaths, assert_not_under_claude,
    bind_private_unix_listener, canonical_rendezvous_dir, current_boot_identity, effective_uid,
    validate_socket_file, verify_same_uid_peer,
};
use unleash_protocol::{
    ClientKind, EventCausationKind, EventEnvelope, HelloRequest, HelloResponse, MAX_FRAME_BYTES,
    ProtocolVersion, Sensitivity, from_json_str_strict,
};
use unleash_store::{StoreConfig, UnleashStore, WriteCriticality};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::executors::{DrainReport, ExecutorDrainPort};
use crate::ipc::{DaemonRuntimeState, StopRequest, spawn_accept_loop, write_json_frame_timeout};
use crate::recovery::{
    classify_and_finish_recovery, enter_recovering_and_emit_event, insert_system_event_tx,
    read_daemon_state_row_tx,
};
use crate::subscriptions::SubscriptionHub;

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Résultat d'une tentative de démarrage du daemon.
#[derive(Debug)]
#[non_exhaustive]
pub enum DaemonStartOutcome {
    /// Nouvelle instance démarrée et prête (ou en cours de reprise si `RecoveryGate` est actif).
    Started(DaemonHandle),
    /// Une instance active sur la même racine a répondu au handshake `Hello` (`attach_if_running=true`).
    AlreadyRunning {
        /// Réponse `HelloResponse` de l'instance rejointe.
        hello: HelloResponse,
        /// Chemin du socket Unix actif.
        socket_path: PathBuf,
    },
}

/// Handle de contrôle sur une instance [`UnleashDaemon`] active.
#[derive(Debug)]
pub struct DaemonHandle {
    socket_path: PathBuf,
    data_root: PathBuf,
    rendezvous_dir: PathBuf,
    authority_generation: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    daemon_instance_id: DaemonInstanceId,
    stream_id: StreamId,
    mode: Arc<RwLock<DaemonMode>>,
    ready_rx: watch::Receiver<Option<Result<(), String>>>,
    stopped_rx: watch::Receiver<Option<DrainReport>>,
    stop_tx: mpsc::Sender<StopRequest>,
}

impl DaemonHandle {
    /// Chemin du socket Unix servi par le daemon.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Racine de données gérée par cette autorité.
    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Répertoire de rendez-vous privé.
    #[must_use]
    pub fn rendezvous_dir(&self) -> &Path {
        &self.rendezvous_dir
    }

    /// Lignée d'autorité active.
    #[must_use]
    pub const fn authority_generation(&self) -> AuthorityGeneration {
        self.authority_generation
    }

    /// Époque d'incarnation du daemon.
    #[must_use]
    pub const fn daemon_epoch(&self) -> DaemonEpoch {
        self.daemon_epoch
    }

    /// Identifiant d'instance unique de ce daemon.
    #[must_use]
    pub const fn daemon_instance_id(&self) -> DaemonInstanceId {
        self.daemon_instance_id
    }

    /// Identifiant du flux d'événements.
    #[must_use]
    pub const fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Mode courant observé en mémoire.
    #[must_use]
    pub fn mode(&self) -> DaemonMode {
        *self.mode.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Attend la fin de la phase `recovering -> ready` (utile avec `RecoveryGate`).
    ///
    /// # Errors
    /// Retourne [`DaemonError`] si le démarrage ou la reprise a échoué.
    pub async fn wait_ready(&self) -> Result<(), DaemonError> {
        let mut rx = self.ready_rx.clone();
        loop {
            if let Some(status) = rx.borrow().clone() {
                return status.map_err(DaemonError::InvalidConfiguration);
            }
            if rx.changed().await.is_err() {
                return Err(DaemonError::InvalidConfiguration(String::from(
                    "Tâche de récupération terminée avant d'annoncer ready",
                )));
            }
        }
    }

    /// Déclenche un arrêt contrôlé et attend le rapport final de drainage.
    ///
    /// # Errors
    /// Retourne [`DaemonError`] en cas d'échec de drainage ou de persistance.
    pub async fn shutdown_graceful(&self, reason: &str) -> Result<DrainReport, DaemonError> {
        if let Some(report) = self.stopped_rx.borrow().clone() {
            return Ok(report);
        }
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let req = StopRequest {
            reason: reason.to_owned(),
            ack: Some(ack_tx),
        };
        if self.stop_tx.send(req).await.is_err() {
            return self.wait_stopped().await.ok_or_else(|| {
                DaemonError::InvalidConfiguration(String::from(
                    "Coordinateur d'arrêt déjà arrêté sans rapport disponible",
                ))
            });
        }
        match ack_rx.await {
            Ok(res) => res,
            Err(_) => self.wait_stopped().await.ok_or_else(|| {
                DaemonError::InvalidConfiguration(String::from(
                    "Canal de confirmation d'arrêt fermé prématurément",
                ))
            }),
        }
    }

    /// Attend que le daemon s'arrête (suite à `StopDaemon` IPC ou signal).
    pub async fn wait_stopped(&self) -> Option<DrainReport> {
        let mut rx = self.stopped_rx.clone();
        loop {
            if let Some(report) = rx.borrow().clone() {
                return Some(report);
            }
            if rx.changed().await.is_err() {
                return rx.borrow().clone();
            }
        }
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.try_send(StopRequest {
            reason: String::from("handle_dropped"),
            ack: None,
        });
    }
}

/// Orchestrateur principal du daemon utilisateur Unleash.
#[derive(Debug)]
pub struct UnleashDaemon;

impl UnleashDaemon {
    /// Démarre l'autorité daemon selon `config`.
    ///
    /// # Errors
    /// Retourne [`DaemonError`] si le verrou OS échoue, `data_root` touche `.claude/`, ou le store échoue.
    #[expect(
        clippy::too_many_lines,
        reason = "Séquence ordonnée de démarrage : chemins, verrou OS, store SQLite, recovery, socket IPC"
    )]
    pub async fn start(config: DaemonConfig) -> Result<DaemonStartOutcome, DaemonError> {
        let platform_paths = PlatformPaths::resolve()?;
        let data_root = match config.data_root.clone() {
            Some(p) => p,
            None => platform_paths.data_dir.clone(),
        };
        assert_not_under_claude(&data_root)?;

        let rdv_dir = match config.rendezvous_dir.clone() {
            Some(p) => p,
            None => canonical_rendezvous_dir()?,
        };
        assert_not_under_claude(&rdv_dir)?;

        // 1. Verrou OS exclusif d'autorité par UID effectif
        let lock = match AuthorityLock::acquire_in(&rdv_dir, &data_root) {
            Ok(l) => l,
            Err(PlatformError::AuthorityAlreadyActive {
                active_data_root,
                requested_data_root,
                socket_path,
            }) => {
                return Err(DaemonError::AuthorityAlreadyActive {
                    active_data_root,
                    requested_data_root,
                    socket_path,
                });
            }
            Err(PlatformError::DaemonAlreadyRunning {
                active_data_root,
                socket_path,
            }) => {
                let probe_res = probe_live_daemon_hello(&socket_path).await;
                if let Ok(hello) = probe_res
                    && config.attach_if_running
                {
                    return Ok(DaemonStartOutcome::AlreadyRunning { hello, socket_path });
                }
                return Err(DaemonError::DaemonAlreadyRunning {
                    active_data_root,
                    socket_path,
                });
            }
            Err(other) => return Err(DaemonError::Platform(other)),
        };

        // 2. Bloquer tout basculement de racine tant qu'un exécuteur/host de l'ancienne racine vit
        let controller = Arc::clone(&config.executor_controller);
        let rdv_check = rdv_dir.clone();
        let data_check = data_root.clone();
        let blocking = tokio::task::spawn_blocking(move || {
            controller.active_hosts_for_other_root(&rdv_check, &data_check)
        })
        .await
        .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error hosts check: {e}")))??;

        if let Some((active_data_root, active_hosts)) = blocking {
            return Err(DaemonError::AuthoritySwitchBlocked {
                active_data_root,
                requested_data_root: data_root,
                active_hosts,
            });
        }

        // 3. Ouvrir et qualifier UnleashStore AVANT de lier le socket IPC (`T-F001-006`)
        let boot_id = current_boot_identity()?.as_str().to_owned();
        let uid = effective_uid();
        let mut store_cfg = StoreConfig::new(&data_root);
        store_cfg.owner_uid = uid;
        store_cfg.boot_identity = boot_id.clone();
        store_cfg.consumer_version = config.consumer_version.clone();
        apply_store_overrides(&mut store_cfg, &config);

        let local_principal = format!("uid:{uid}");
        let boot_for_rec = boot_id.clone();
        let (store, recovery_row, prev_mode, started_ev) =
            tokio::task::spawn_blocking(move || -> Result<_, DaemonError> {
                let opened = UnleashStore::open(store_cfg)?;
                opened.register_principal(&local_principal, "user", &local_principal)?;
                let new_inst = DaemonInstanceId::from_uuid(opened.config().id_generator.new_uuid());
                let (row, prev_mode, started_ev) =
                    enter_recovering_and_emit_event(&opened, &boot_for_rec, new_inst)?;
                Ok((Arc::new(opened), row, prev_mode, started_ev))
            })
            .await
            .map_err(|e| {
                DaemonError::InvalidConfiguration(format!("Join error store init: {e}"))
            })??;

        // 5. Nettoyage éventuel de socket orphelin et ouverture socket Unix privé (`0600`)
        lock.remove_stale_socket()?;
        let socket_path = lock.socket_path().to_path_buf();
        let std_listener = match bind_private_unix_listener(&socket_path) {
            Ok(l) => l,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || drop(store)).await;
                return Err(DaemonError::Platform(err));
            }
        };
        if let Err(err) = std_listener.set_nonblocking(true) {
            let _ = tokio::task::spawn_blocking(move || drop(store)).await;
            return Err(DaemonError::Io(err));
        }
        let tokio_listener = match tokio::net::UnixListener::from_std(std_listener) {
            Ok(l) => l,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || drop(store)).await;
                return Err(DaemonError::Io(err));
            }
        };

        let browser_search_paths =
            config
                .browser_search_paths
                .clone()
                .unwrap_or_else(|| LocalInventorySearchPaths {
                    unleash_runtimes_dir: Some(platform_paths.browser_runtimes_dir),
                    ..LocalInventorySearchPaths::default()
                });

        let subscriptions = SubscriptionHub::new(config.subscriber_queue_capacity);
        let (stop_tx, stop_rx) = mpsc::channel::<StopRequest>(4);
        let (ready_tx, ready_rx) = watch::channel(None::<Result<(), String>>);
        let (stopped_tx, stopped_rx) = watch::channel(None::<DrainReport>);
        let shared_mode = Arc::new(RwLock::new(DaemonMode::Recovering));

        let state = Arc::new(DaemonRuntimeState {
            store: Arc::clone(&store),
            authority_generation: recovery_row.authority_generation,
            daemon_epoch: recovery_row.daemon_epoch,
            daemon_instance_id: recovery_row.daemon_instance_id,
            stream_id: recovery_row.stream_id,
            mode: Arc::clone(&shared_mode),
            browser_search_paths,
            auth_verifier: Arc::clone(&config.auth_verifier),
            post_commit_hook: config.post_commit_hook.clone(),
            subscriptions: Arc::clone(&subscriptions),
            subscriber_write_timeout: config.subscriber_write_timeout,
            in_flight_mutations: Arc::new(Semaphore::new(config.max_in_flight_mutations.max(1))),
            conn_permits: Arc::new(Semaphore::new(config.max_connections.max(1))),
            accepting: AtomicBool::new(true),
            shutdown_notify: Arc::new(Notify::new()),
            stop_tx: stop_tx.clone(),
        });

        let active_conns = Arc::new(Mutex::new(Vec::new()));
        let accept_handle = spawn_accept_loop(
            tokio_listener,
            Arc::clone(&state),
            Arc::clone(&active_conns),
        );

        // 6. Coordinateur d'arrêt propriétaire du Store, du Verrou OS et des tâches actives
        spawn_drain_coordinator(DrainCoordinatorParams {
            stop_rx,
            stopped_tx,
            state: Arc::clone(&state),
            store,
            lock,
            socket_path: socket_path.clone(),
            rdv_dir: rdv_dir.clone(),
            controller: Arc::clone(&config.executor_controller),
            drain_timeout: config.shutdown_drain_timeout,
            accept_handle,
            active_conns,
        });

        let state_for_rec = Arc::clone(&state);
        let started_event_id = started_ev.event_id;
        let handle = DaemonHandle {
            socket_path,
            data_root,
            rendezvous_dir: rdv_dir,
            authority_generation: recovery_row.authority_generation,
            daemon_epoch: recovery_row.daemon_epoch,
            daemon_instance_id: recovery_row.daemon_instance_id,
            stream_id: recovery_row.stream_id,
            mode: shared_mode,
            ready_rx,
            stopped_rx,
            stop_tx,
        };

        if let Some(gate) = config.recovery_gate {
            tokio::spawn(async move {
                gate.enter_and_wait().await;
                let res = finish_recovery_step(&state_for_rec, prev_mode, started_event_id).await;
                let _ = ready_tx.send(Some(res.map_err(|e| e.to_string())));
            });
            Ok(DaemonStartOutcome::Started(handle))
        } else {
            let res = finish_recovery_step(&state_for_rec, prev_mode, started_event_id).await;
            match res {
                Ok(()) => {
                    let _ = ready_tx.send(Some(Ok(())));
                    Ok(DaemonStartOutcome::Started(handle))
                }
                Err(err) => {
                    let _ = handle.shutdown_graceful("recovery_failed").await;
                    Err(err)
                }
            }
        }
    }
}

async fn finish_recovery_step(
    state: &Arc<DaemonRuntimeState>,
    prev_mode: DaemonMode,
    started_event_id: uuid::Uuid,
) -> Result<(), DaemonError> {
    let store = Arc::clone(&state.store);
    let (row, emitted) = tokio::task::spawn_blocking(move || {
        classify_and_finish_recovery(&store, prev_mode, started_event_id)
    })
    .await
    .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error recovery finish: {e}")))??;

    state.set_mode(row.mode);
    state.subscriptions.publish_events(&emitted);
    Ok(())
}

struct DrainCoordinatorParams {
    stop_rx: mpsc::Receiver<StopRequest>,
    stopped_tx: watch::Sender<Option<DrainReport>>,
    state: Arc<DaemonRuntimeState>,
    store: Arc<UnleashStore>,
    lock: AuthorityLock,
    socket_path: PathBuf,
    rdv_dir: PathBuf,
    controller: Arc<dyn ExecutorDrainPort>,
    drain_timeout: Duration,
    accept_handle: tokio::task::JoinHandle<()>,
    active_conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

fn spawn_drain_coordinator(params: DrainCoordinatorParams) {
    tokio::spawn(async move {
        let mut stop_rx = params.stop_rx;
        let mut state_opt = Some(params.state);
        let mut store_opt = Some(params.store);
        let mut lock_opt = Some(params.lock);
        let mut accept_opt = Some(params.accept_handle);

        if let Some(req) = stop_rx.recv().await
            && let (Some(state), Some(store)) = (
                state_opt.as_ref().map(Arc::clone),
                store_opt.as_ref().map(Arc::clone),
            )
        {
            state.set_mode(DaemonMode::Draining);
            let res = execute_drain_and_stop(
                &state,
                &store,
                &params.rdv_dir,
                &params.controller,
                params.drain_timeout,
                &req.reason,
            )
            .await;

            match res {
                Ok(report) => {
                    state.set_mode(DaemonMode::Stopped);
                    state.subscriptions.close_all();
                    state.stop_accepting();
                    drop(store);
                    drop(state);

                    if let Some(accept_handle) = accept_opt.take() {
                        let _ =
                            tokio::time::timeout(Duration::from_millis(250), accept_handle).await;
                    }

                    drain_active_connections(&params.active_conns, params.drain_timeout).await;

                    let _ = std::fs::remove_file(&params.socket_path);
                    // Libérer state + UnleashStore hors thread Tokio runtime (StoreWriterHandle::drop
                    // appelle blocking_recv()) AVANT le verrou OS `flock` et AVANT d'annoncer l'arrêt.
                    let state_drop = state_opt.take();
                    let store_drop = store_opt.take();
                    let _ = tokio::task::spawn_blocking(move || {
                        drop(state_drop);
                        drop(store_drop);
                    })
                    .await;
                    drop(lock_opt.take());

                    let _ = params.stopped_tx.send(Some(report.clone()));
                    if let Some(ack) = req.ack {
                        let _ = ack.send(Ok(report.clone()));
                    }
                    stop_rx.close();
                    while let Some(rem) = stop_rx.recv().await {
                        if let Some(ack) = rem.ack {
                            let _ = ack.send(Ok(report.clone()));
                        }
                    }
                }
                Err(err) => {
                    if let Some(ack) = req.ack {
                        let _ = ack.send(Err(err));
                    }
                }
            }
        }
        let state_drop = state_opt.take();
        let store_drop = store_opt.take();
        if state_drop.is_some() || store_drop.is_some() {
            let _ = tokio::task::spawn_blocking(move || {
                drop(state_drop);
                drop(store_drop);
            })
            .await;
        }
        drop(lock_opt);
    });
}

async fn drain_active_connections(
    active_conns: &Mutex<Vec<tokio::task::JoinHandle<()>>>,
    timeout: Duration,
) {
    let tasks: Vec<tokio::task::JoinHandle<()>> = {
        let mut guard = active_conns.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    let wait_all = async {
        for task in tasks {
            let _ = task.await;
        }
    };
    let _ = tokio::time::timeout(timeout.min(Duration::from_secs(2)), wait_all).await;
}

async fn execute_drain_and_stop(
    state: &Arc<DaemonRuntimeState>,
    store: &Arc<UnleashStore>,
    rdv_dir: &Path,
    controller: &Arc<dyn ExecutorDrainPort>,
    drain_timeout: Duration,
    reason: &str,
) -> Result<DrainReport, DaemonError> {
    let mut req_payload = serde_json::Map::new();
    req_payload.insert(
        String::from("reason"),
        serde_json::Value::String(reason.to_owned()),
    );
    if let Some(ev) = persist_daemon_mode_event(
        store,
        DaemonMode::Draining,
        "DaemonStopRequested",
        req_payload,
    )
    .await?
    {
        state.subscriptions.publish_events(&[ev]);
    }

    let ctrl_ref = Arc::clone(controller);
    let rdv_owned = rdv_dir.to_path_buf();
    let report =
        tokio::task::spawn_blocking(move || ctrl_ref.drain_on_stop(&rdv_owned, drain_timeout))
            .await
            .map_err(|e| {
                DaemonError::InvalidConfiguration(format!("Join error drain_on_stop: {e}"))
            })?;

    let mut stop_payload = serde_json::Map::new();
    stop_payload.insert(
        String::from("stop_outcome"),
        serde_json::Value::String(report.outcome.to_string()),
    );
    stop_payload.insert(
        String::from("retained_services"),
        serde_json::to_value(&report.retained_services)?,
    );
    stop_payload.insert(
        String::from("unknown_services"),
        serde_json::to_value(&report.unknown_services)?,
    );
    stop_payload.insert(
        String::from("reason"),
        serde_json::Value::String(reason.to_owned()),
    );
    if let Some(ev) =
        persist_daemon_mode_event(store, DaemonMode::Stopped, "DaemonStopped", stop_payload).await?
    {
        state.subscriptions.publish_events(&[ev]);
    }

    Ok(report)
}

async fn persist_daemon_mode_event(
    store: &Arc<UnleashStore>,
    target_mode: DaemonMode,
    event_type: &'static str,
    payload: serde_json::Map<String, serde_json::Value>,
) -> Result<Option<EventEnvelope>, DaemonError> {
    let store_ref = Arc::clone(store);
    tokio::task::spawn_blocking(move || -> Result<Option<EventEnvelope>, DaemonError> {
        let id_gen = Arc::clone(&store_ref.config().id_generator);
        let ev_id = id_gen.new_uuid();
        store_ref
            .execute_in_writer_tx(WriteCriticality::Critical, move |tx, now_ms| {
                let row = read_daemon_state_row_tx(tx)?;
                let skip = match target_mode {
                    DaemonMode::Draining => {
                        matches!(row.mode, DaemonMode::Draining | DaemonMode::Stopped)
                    }
                    DaemonMode::Stopped => row.mode == DaemonMode::Stopped,
                    _ => false,
                };
                if skip {
                    return Ok(None);
                }
                let next_mode = transition_daemon_mode(row.mode, target_mode)?;
                let next_rev: Revision = row.revision.next();
                tx.execute(
                    "UPDATE daemon_state SET mode = ?1, revision = ?2 WHERE singleton = 1",
                    rusqlite::params![
                        next_mode.to_string(),
                        i64::try_from(next_rev.as_u64()).unwrap_or(1)
                    ],
                )?;

                let ev = insert_system_event_tx(
                    tx,
                    ev_id,
                    row.stream_id,
                    "daemon",
                    row.authority_generation.as_uuid(),
                    next_rev,
                    0,
                    event_type,
                    None,
                    &format!("uid:{}", row.owner_uid),
                    ev_id,
                    EventCausationKind::External,
                    "daemon_shutdown",
                    payload,
                    Sensitivity::Public,
                    now_ms,
                )?;
                Ok(Some(ev))
            })
            .map_err(DaemonError::from)
    })
    .await
    .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error {event_type}: {e}")))?
}

fn apply_store_overrides(cfg: &mut StoreConfig, daemon_cfg: &DaemonConfig) {
    let ov = &daemon_cfg.store_overrides;
    if let Some(ref bp) = ov.build_policy {
        cfg.build_policy = bp.clone();
    }
    if let Some(cap) = ov.writer_queue_capacity {
        cfg.writer_queue_capacity = cap.max(1);
    }
    if let Some(ref clock) = ov.clock {
        cfg.clock = Arc::clone(clock);
    }
    if let Some(ref mono) = ov.monotonic_clock {
        cfg.monotonic_clock = Arc::clone(mono);
    }
    if let Some(ref id_gen) = ov.id_generator {
        cfg.id_generator = Arc::clone(id_gen);
    }
    if let Some(ref cutpoints) = ov.cutpoints {
        cfg.cutpoints = Arc::clone(cutpoints);
    }
    if let Some(ref fault) = ov.fault_injector {
        cfg.fault_injector = Arc::clone(fault);
    }
    if let Some(ref probe) = ov.disk_space_probe {
        cfg.disk_space_probe = Arc::clone(probe);
    }
}

async fn probe_live_daemon_hello(socket_path: &Path) -> Result<HelloResponse, DaemonError> {
    validate_socket_file(socket_path)?;
    let mut stream = tokio::time::timeout(PROBE_CONNECT_TIMEOUT, UnixStream::connect(socket_path))
        .await
        .map_err(|_| {
            DaemonError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "connect timeout",
            ))
        })??;
    verify_same_uid_peer(&stream)?;

    let req = HelloRequest {
        protocol_version: ProtocolVersion,
        client_kind: ClientKind::Probe,
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
        capabilities: Vec::new(),
    };
    write_json_frame_timeout(&mut stream, &req, PROBE_CONNECT_TIMEOUT).await?;

    let mut header = [0u8; 4];
    tokio::time::timeout(PROBE_CONNECT_TIMEOUT, stream.read_exact(&mut header))
        .await
        .map_err(|_| {
            DaemonError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "read header timeout",
            ))
        })??;
    let len = u32::from_be_bytes(header);
    let len_usize = usize::try_from(len).unwrap_or(usize::MAX);
    if len_usize > MAX_FRAME_BYTES {
        return Err(DaemonError::InvalidConfiguration(format!(
            "Réponse Hello trop volumineuse ({len} > {MAX_FRAME_BYTES})"
        )));
    }
    let mut body = vec![0u8; len_usize];
    tokio::time::timeout(PROBE_CONNECT_TIMEOUT, stream.read_exact(&mut body))
        .await
        .map_err(|_| {
            DaemonError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "read body timeout",
            ))
        })??;
    let text = std::str::from_utf8(&body).map_err(|e| {
        DaemonError::InvalidConfiguration(format!("HelloResponse UTF-8 invalide : {e}"))
    })?;
    let _ = stream.shutdown().await;
    from_json_str_strict::<HelloResponse>(text).map_err(DaemonError::from)
}
