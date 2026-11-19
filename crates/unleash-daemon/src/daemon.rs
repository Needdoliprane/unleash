// SPDX-License-Identifier: Apache-2.0
//! Point d'entrée de composition et cycle de vie complet du daemon Unleash (`unleashd`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore, mpsc, watch};
use unleash_browser::LocalInventorySearchPaths;
use unleash_domain::{
    AuthorityGeneration, DaemonEpoch, DaemonInstanceId, DaemonMode, Revision, StreamId,
    transition_daemon_mode,
};
use unleash_platform::{
    ADMIN_PROOF_FILENAME, AuthorityLock, PlatformError, PlatformPaths, assert_not_under_claude,
    bind_private_unix_listener, canonical_rendezvous_dir, current_boot_identity, effective_uid,
    read_admin_credential, validate_rendezvous_dir, validate_socket_file, verify_same_uid_peer,
    write_private_file_atomic,
};
use unleash_protocol::{
    AdminProof, ClientKind, EventCausationKind, EventEnvelope, HelloRequest, HelloResponse,
    MAX_FRAME_BYTES, ProtocolVersion, Sensitivity, from_json_str_strict,
};
use unleash_store::{StoreConfig, UnleashStore, WriteCriticality};

use crate::auth::AdministrativeAuthVerifier;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::executors::{DrainReport, ExecutorDrainPort};
use crate::ipc::{
    DaemonRuntimeState, StopRequest, publish_committed_events_ordered, spawn_accept_loop,
    write_json_frame_timeout,
};
use crate::recovery::{
    classify_and_finish_recovery, enter_recovering_and_emit_event, insert_system_event_tx,
    read_daemon_state_row_tx,
};
use crate::subscriptions::SubscriptionHub;

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn install_admin_proof(dir: &Path) -> Result<AdminProof, DaemonError> {
    install_admin_proof_with(dir, || Ok(()))
}

struct PendingAdminProof<'a> {
    dir: &'a Path,
    proof: AdminProof,
    armed: bool,
}

impl Drop for PendingAdminProof<'_> {
    fn drop(&mut self) {
        if self.armed {
            remove_owned_admin_proof(self.dir, &self.proof);
        }
    }
}

fn install_admin_proof_with(
    dir: &Path,
    after_publish: impl FnOnce() -> Result<(), DaemonError>,
) -> Result<AdminProof, DaemonError> {
    validate_rendezvous_dir(dir)?;
    let mut bytes = zeroize::Zeroizing::new([0_u8; 32]);
    getrandom::fill(&mut *bytes).map_err(|_| {
        DaemonError::InvalidConfiguration(String::from("Génération du credential impossible"))
    })?;
    let mut pending = PendingAdminProof {
        dir,
        proof: AdminProof::from_bytes(*bytes),
        armed: true,
    };
    write_private_file_atomic(
        &dir.join(ADMIN_PROOF_FILENAME),
        pending.proof.to_hex().as_bytes(),
    )?;
    after_publish()?;
    let verified = read_admin_credential(dir)?.and_then(|value| AdminProof::from_hex(&value));
    if !verified.is_some_and(|candidate| candidate.matches(&pending.proof)) {
        return Err(DaemonError::InvalidConfiguration(String::from(
            "Credential local invalide après création",
        )));
    }
    pending.armed = false;
    Ok(pending.proof.clone())
}

pub(crate) fn remove_owned_admin_proof(dir: &Path, expected: &AdminProof) {
    match read_admin_credential(dir) {
        Ok(None) => {}
        Ok(Some(value)) if AdminProof::from_hex(&value).is_some_and(|p| p.matches(expected)) => {
            if std::fs::remove_file(dir.join(ADMIN_PROOF_FILENAME)).is_err() {
                tracing::warn!("Suppression du credential local impossible à l'arrêt");
            }
        }
        _ => tracing::warn!("Credential local modifié ou inaccessible ; retrait non effectué"),
    }
}

/// Résultat d'une tentative de démarrage du daemon.
#[derive(Debug)]
#[non_exhaustive]
pub enum DaemonStartOutcome {
    /// Nouvelle instance démarrée et prête (ou en cours de reprise si `RecoveryGate` est actif).
    Started(DaemonHandle),
    /// Autorité diagnostique en lecture seule lorsque le store est corrompu.
    Diagnostic(crate::diagnostic::DiagnosticHandle),
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
    #[cfg(test)]
    test_state: std::sync::Weak<DaemonRuntimeState>,
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
    #[cfg(test)]
    pub(crate) fn test_runtime_state(&self) -> Arc<DaemonRuntimeState> {
        self.test_state.upgrade().expect("daemon test runtime")
    }
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
            drain_timeout_override: None,
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
            drain_timeout_override: None,
            ack: None,
        });
    }
}

fn resolve_paths_and_acquire_lock_blocking(
    explicit_data_root: Option<PathBuf>,
    explicit_rdv_dir: Option<PathBuf>,
) -> Result<
    (
        PlatformPaths,
        PathBuf,
        PathBuf,
        Result<AuthorityLock, PlatformError>,
    ),
    DaemonError,
> {
    let platform_paths = PlatformPaths::resolve()?;
    let data_root = explicit_data_root.unwrap_or_else(|| platform_paths.data_dir.clone());
    assert_not_under_claude(&data_root)?;

    let rdv_dir = match explicit_rdv_dir {
        Some(p) => p,
        None => canonical_rendezvous_dir()?,
    };
    assert_not_under_claude(&rdv_dir)?;

    let lock_res = AuthorityLock::acquire_in(&rdv_dir, &data_root);
    Ok((platform_paths, data_root, rdv_dir, lock_res))
}

pub(crate) fn prepare_socket_listener_blocking(
    lock: AuthorityLock,
) -> Result<(AuthorityLock, std::os::unix::net::UnixListener, PathBuf), DaemonError> {
    lock.remove_stale_socket()?;
    let socket_path = lock.socket_path().to_path_buf();
    let std_listener = bind_private_unix_listener(&socket_path)?;
    std_listener.set_nonblocking(true)?;
    Ok((lock, std_listener, socket_path))
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
        let cfg_data_root = config.data_root.clone();
        let cfg_rdv_dir = config.rendezvous_dir.clone();
        let (platform_paths, data_root, rdv_dir, lock_res) =
            tokio::task::spawn_blocking(move || {
                resolve_paths_and_acquire_lock_blocking(cfg_data_root, cfg_rdv_dir)
            })
            .await
            .map_err(|e| {
                DaemonError::InvalidConfiguration(format!("Join error lock acquire: {e}"))
            })??;

        // 1. Verrou OS exclusif d'autorité par UID effectif
        let lock = match lock_res {
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
        .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error hosts check: {e}")))?;
        let blocking = match blocking {
            Ok(b) => b,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || drop(lock)).await;
                return Err(err);
            }
        };

        if let Some((active_data_root, active_hosts)) = blocking {
            let _ = tokio::task::spawn_blocking(move || drop(lock)).await;
            return Err(DaemonError::AuthoritySwitchBlocked {
                active_data_root,
                requested_data_root: data_root,
                active_hosts,
            });
        }

        // 3. Ouvrir et qualifier UnleashStore AVANT de lier le socket IPC (`T-F001-006`)
        let mut store_cfg = StoreConfig::new(&data_root);
        store_cfg.consumer_version = config.consumer_version.clone();
        apply_store_overrides(&mut store_cfg, &config);

        let diagnostic_config = store_cfg.clone();
        let store_open_res = tokio::task::spawn_blocking(move || -> Result<_, DaemonError> {
            let boot_id = current_boot_identity()?.as_str().to_owned();
            let uid = effective_uid();
            store_cfg.owner_uid = uid;
            store_cfg.boot_identity.clone_from(&boot_id);
            let opened = UnleashStore::open_classified(store_cfg);
            Ok((opened, boot_id, uid))
        })
        .await
        .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error store open: {e}")))?;

        let (opened, boot_id, uid) = store_open_res?;
        let opened = match opened {
            Ok(opened) => opened,
            Err(unleash_store::StoreOpenFailure {
                stage: unleash_store::StoreOpenStage::ReadOnlyPreflight,
                error: unleash_store::StoreError::StorageCorrupt(_),
            }) => {
                let handle = crate::diagnostic::start(
                    lock,
                    rdv_dir,
                    diagnostic_config,
                    Arc::clone(&config.auth_verifier),
                )
                .await?;
                return Ok(DaemonStartOutcome::Diagnostic(handle));
            }
            Err(failure) => {
                let _ = tokio::task::spawn_blocking(move || drop(lock)).await;
                return Err(DaemonError::Store(failure.error));
            }
        };

        let store_init_res = tokio::task::spawn_blocking(move || -> Result<_, DaemonError> {
            let local_principal = format!("uid:{uid}");
            opened.register_principal(&local_principal, "user", &local_principal)?;
            let new_inst = DaemonInstanceId::from_uuid(opened.config().id_generator.new_uuid());
            let (row, prev_mode, started_ev) =
                enter_recovering_and_emit_event(&opened, &boot_id, new_inst)?;
            Ok((Arc::new(opened), row, prev_mode, started_ev))
        })
        .await
        .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error store init: {e}")))?;

        let (store, recovery_row, prev_mode, started_ev) = match store_init_res {
            Ok(tuple) => tuple,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || drop(lock)).await;
                return Err(err);
            }
        };

        // La preuve est renouvelée sous verrou et avant toute ouverture du socket.
        let proof_dir = rdv_dir.clone();
        let proof_result = tokio::task::spawn_blocking(move || install_admin_proof(&proof_dir))
            .await
            .map_err(|_| {
                DaemonError::InvalidConfiguration(String::from(
                    "Génération du credential interrompue",
                ))
            })
            .and_then(std::convert::identity);
        let admin_proof = match proof_result {
            Ok(proof) => proof,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || {
                    drop(store);
                    drop(lock);
                })
                .await;
                return Err(err);
            }
        };
        let auth_verifier: Arc<dyn crate::auth::AuthVerifierPort> =
            Arc::new(AdministrativeAuthVerifier {
                inner: Arc::clone(&config.auth_verifier),
                expected: admin_proof.clone(),
            });

        // 5. Nettoyage éventuel de socket orphelin et ouverture socket Unix privé (`0600`)
        let store_for_cleanup = Arc::clone(&store);
        let socket_prep = tokio::task::spawn_blocking(move || {
            let res = prepare_socket_listener_blocking(lock);
            if res.is_err() {
                drop(store_for_cleanup);
            }
            res
        })
        .await
        .map_err(|e| DaemonError::InvalidConfiguration(format!("Join error socket bind: {e}")))?;

        let (lock, std_listener, socket_path) = match socket_prep {
            Ok(ok) => ok,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || drop(store)).await;
                return Err(err);
            }
        };

        let tokio_listener = match tokio::net::UnixListener::from_std(std_listener) {
            Ok(l) => l,
            Err(err) => {
                let _ = tokio::task::spawn_blocking(move || {
                    drop(store);
                    drop(lock);
                })
                .await;
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
        let (drain_started_tx, _drain_started_rx) = watch::channel(false);
        let shared_mode = Arc::new(RwLock::new(DaemonMode::Recovering));

        let state = Arc::new(DaemonRuntimeState {
            rendezvous_dir: rdv_dir.clone(),
            terminal_children: Mutex::new(Vec::new()),
            broker_activity: crate::broker_activity::BrokerActivity::new(),
            store: Arc::clone(&store),
            authority_generation: recovery_row.authority_generation,
            daemon_epoch: recovery_row.daemon_epoch,
            daemon_instance_id: recovery_row.daemon_instance_id,
            stream_id: recovery_row.stream_id,
            mode: Arc::clone(&shared_mode),
            browser_search_paths,
            auth_verifier,
            post_commit_hook: config.post_commit_hook.clone(),
            subscriptions: Arc::clone(&subscriptions),
            last_published_seq: AsyncMutex::new(Some((
                recovery_row.stream_id,
                started_ev.sequence.as_u64(),
            ))),
            subscriber_write_timeout: config.subscriber_write_timeout,
            in_flight_mutations: Arc::new(Semaphore::new(config.max_in_flight_mutations.max(1))),
            conn_permits: Arc::new(Semaphore::new(config.max_connections.max(1))),
            accepting: AtomicBool::new(true),
            shutdown_notify: Arc::new(Notify::new()),
            drain_started_tx,
            stop_tx: stop_tx.clone(),
        });

        let active_conns = Arc::new(Mutex::new(Vec::new()));
        let accept_handle = spawn_accept_loop(
            tokio_listener,
            Arc::clone(&state),
            Arc::clone(&active_conns),
        );

        let started_event_id = started_ev.event_id;
        let has_recovery_gate = config.recovery_gate.is_some();
        let recovery_handle = config.recovery_gate.map(|gate| {
            let state_for_rec = Arc::clone(&state);
            let ready_tx_gate = ready_tx.clone();
            tokio::spawn(async move {
                let mut drain_rx = state_for_rec.subscribe_drain_started();
                let cancelled_early = tokio::select! {
                    biased;
                    _ = drain_rx.wait_for(|&started| started) => true,
                    () = gate.enter_and_wait() => false,
                };
                let cancelled = cancelled_early || *drain_rx.borrow();
                if cancelled || !matches!(state_for_rec.current_mode(), DaemonMode::Recovering) {
                    drop(state_for_rec);
                    let _ = ready_tx_gate
                        .send(Some(Err(String::from("arrêt demandé pendant recovering"))));
                    return;
                }
                let res = finish_recovery_step(&state_for_rec, prev_mode, started_event_id).await;
                drop(state_for_rec);
                let _ = ready_tx_gate.send(Some(res.map_err(|e| e.to_string())));
            })
        });

        // 6. Coordinateur d'arrêt propriétaire du Store, du Verrou OS et des tâches actives
        spawn_drain_coordinator(DrainCoordinatorParams {
            stop_rx,
            stopped_tx,
            state: Arc::clone(&state),
            store,
            lock,
            socket_path: socket_path.clone(),
            rdv_dir: rdv_dir.clone(),
            admin_proof,
            controller: Arc::clone(&config.executor_controller),
            drain_timeout: config.shutdown_drain_timeout,
            accept_handle,
            active_conns,
            recovery_handle,
        });

        let handle = DaemonHandle {
            #[cfg(test)]
            test_state: Arc::downgrade(&state),
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

        if has_recovery_gate {
            Ok(DaemonStartOutcome::Started(handle))
        } else {
            let res = finish_recovery_step(&state, prev_mode, started_event_id).await;
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
    if let Some(last_ev) = emitted.last() {
        publish_committed_events_ordered(
            state,
            last_ev.stream_id,
            last_ev.sequence.as_u64(),
            emitted.len(),
            &emitted,
        )
        .await;
    }
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
    admin_proof: AdminProof,
    controller: Arc<dyn ExecutorDrainPort>,
    drain_timeout: Duration,
    accept_handle: tokio::task::JoinHandle<()>,
    active_conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    recovery_handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug)]
enum ShutdownCleanupError {
    Pending(&'static str),
    Failed(DaemonError),
}

impl From<DaemonError> for ShutdownCleanupError {
    fn from(error: DaemonError) -> Self {
        Self::Failed(error)
    }
}

struct ShutdownResources {
    state: Option<Arc<DaemonRuntimeState>>,
    store: Option<Arc<UnleashStore>>,
    // The coordinator retains this owner while a blocking release borrows it.
    // An I/O error or panic cannot drop the only authority guard in that job.
    lock: Option<Arc<Mutex<AuthorityLock>>>,
    accept: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownResources {
    async fn release_authority_with<F>(&mut self, release: F) -> Result<(), ShutdownCleanupError>
    where
        F: FnOnce(&mut AuthorityLock) -> Result<(), PlatformError> + Send + 'static,
    {
        if let Some(lock) = self.lock.as_ref().map(Arc::clone) {
            tokio::task::spawn_blocking(move || {
                let mut authority = lock.lock().unwrap_or_else(PoisonError::into_inner);
                release(&mut authority)
            })
            .await
            .map_err(|error| {
                DaemonError::InvalidConfiguration(format!(
                    "Libération d'autorité interrompue : {error}",
                ))
            })?
            .map_err(DaemonError::Platform)?;
            self.lock.take();
        }
        Ok(())
    }

    async fn take_exclusive_store(
        &mut self,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<Option<UnleashStore>, ShutdownCleanupError> {
        loop {
            if let Some(state) = self.state.take() {
                match Arc::try_unwrap(state) {
                    Ok(state) => drop(state),
                    Err(state) => self.state = Some(state),
                }
            }
            if self.state.is_none() {
                let Some(store) = self.store.take() else {
                    return Ok(None);
                };
                match Arc::try_unwrap(store) {
                    Ok(store) => return Ok(Some(store)),
                    Err(store) => self.store = Some(store),
                }
            }
            if deadline.is_some_and(|end| tokio::time::Instant::now() >= end) {
                return Err(ShutdownCleanupError::Pending(
                    "Propriétaires du Store encore actifs ; verrou d'autorité conservé",
                ));
            }
            // The timeout only bounds waiting. Exclusive ownership, rather than
            // elapsed time or a cancelled async caller, proves the Store is idle.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn finish(
        &mut self,
        connections: &Mutex<Vec<tokio::task::JoinHandle<()>>>,
        socket_path: &Path,
        rendezvous_dir: &Path,
        proof: &AdminProof,
        timeout: Option<Duration>,
    ) -> Result<(), ShutdownCleanupError> {
        // One wait budget for this cleanup attempt, not a fresh window per stage.
        // This bounds waiting, never establishes completion of a blocking job.
        let deadline = timeout.map(|duration| tokio::time::Instant::now() + duration);
        if let Some(state) = &self.state {
            state.set_mode(DaemonMode::Stopped);
            state.subscriptions.close_all();
            state.stop_accepting();
        }
        if let Some(accept) = self.accept.as_mut() {
            let result = if let Some(deadline) = deadline {
                tokio::time::timeout_at(deadline, &mut *accept)
                    .await
                    .map_err(|_| {
                        ShutdownCleanupError::Pending(
                            "Acceptation IPC encore active ; verrou d'autorité conservé",
                        )
                    })?
            } else {
                accept.await
            };
            self.accept.take();
            result.map_err(|error| {
                ShutdownCleanupError::Failed(DaemonError::InvalidConfiguration(format!(
                    "Arrêt acceptation IPC : {error}"
                )))
            })?;
        }
        drain_connection_tasks(connections, deadline).await?;
        if let Some(store) = self.take_exclusive_store(deadline).await? {
            // Never detach Store::shutdown, even if the request's wait expired.
            tokio::task::spawn_blocking(move || store.shutdown())
                .await
                .map_err(|error| {
                    DaemonError::InvalidConfiguration(format!(
                        "Fermeture du Store interrompue : {error}",
                    ))
                })?
                .map_err(DaemonError::Store)?;
        }
        let socket = socket_path.to_path_buf();
        let directory = rendezvous_dir.to_path_buf();
        let expected = proof.clone();
        tokio::task::spawn_blocking(move || -> Result<(), DaemonError> {
            match std::fs::remove_file(socket) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(DaemonError::Io(error)),
            }
            if let Some(value) = read_admin_credential(&directory)? {
                if !AdminProof::from_hex(&value).is_some_and(|actual| actual.matches(&expected)) {
                    return Err(DaemonError::InvalidConfiguration(String::from(
                        "Credential local modifié ; verrou d'autorité conservé",
                    )));
                }
                std::fs::remove_file(directory.join(ADMIN_PROOF_FILENAME))?;
            }
            Ok(())
        })
        .await
        .map_err(|error| {
            DaemonError::InvalidConfiguration(format!("Nettoyage d'autorité interrompu : {error}",))
        })??;
        self.release_authority_with(AuthorityLock::release).await
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Drain, retries et libération ordonnée de l'autorité"
)]
fn spawn_drain_coordinator(params: DrainCoordinatorParams) {
    tokio::spawn(async move {
        let mut stop_rx = params.stop_rx;
        let mut resources = ShutdownResources {
            state: Some(params.state),
            store: Some(params.store),
            lock: Some(Arc::new(Mutex::new(params.lock))),
            accept: Some(params.accept_handle),
        };
        let mut recovery_opt = params.recovery_handle;
        let mut stopped_report = None;
        let mut cleanup_failure: Option<String> = None;

        while let Some(mut req) = stop_rx.recv().await {
            if let Some(message) = &cleanup_failure {
                if let Some(ack) = req.ack.take() {
                    let _ = ack.send(Err(DaemonError::InvalidConfiguration(message.clone())));
                }
                // A failed close/join must never become success on another call.
                // Keep any remaining authority until the process itself ends.
                continue;
            }
            if stopped_report.is_none() {
                let (Some(state), Some(store)) = (
                    resources.state.as_ref().map(Arc::clone),
                    resources.store.as_ref().map(Arc::clone),
                ) else {
                    break;
                };
                state.mark_draining();
                if let Some(recovery_handle) = recovery_opt.take()
                    && let Err(error) = recovery_handle.await
                {
                    let message = format!("Tâche de reprise interrompue à l'arrêt : {error}");
                    cleanup_failure = Some(message.clone());
                    if let Some(ack) = req.ack.take() {
                        let _ = ack.send(Err(DaemonError::InvalidConfiguration(message)));
                    }
                    continue;
                }
                let effective_drain_timeout =
                    req.drain_timeout_override.unwrap_or(params.drain_timeout);
                if tokio::time::timeout(
                    effective_drain_timeout,
                    state.broker_activity.close_and_wait(),
                )
                .await
                .is_err()
                {
                    if let Some(ack) = req.ack.take() {
                        let _ = ack.send(Err(DaemonError::InvalidConfiguration(String::from(
                            "Travaux broker encore actifs ; verrou d'autorité conservé",
                        ))));
                        continue;
                    }
                    // Sans demandeur vivant, poursuivre l'arrêt dès que le travail
                    // se termine tout en conservant le verrou entre-temps.
                    state.broker_activity.close_and_wait().await;
                }
                let result = execute_drain_and_stop(
                    &state,
                    &store,
                    &params.rdv_dir,
                    &params.controller,
                    effective_drain_timeout,
                    &req.reason,
                )
                .await;
                match result {
                    Ok(report) => stopped_report = Some(report),
                    Err(error) => {
                        if let Some(ack) = req.ack.take() {
                            let _ = ack.send(Err(error));
                        }
                        continue;
                    }
                }
            }
            let timeout = req
                .ack
                .as_ref()
                .map(|_| req.drain_timeout_override.unwrap_or(params.drain_timeout));
            match resources
                .finish(
                    &params.active_conns,
                    &params.socket_path,
                    &params.rdv_dir,
                    &params.admin_proof,
                    timeout,
                )
                .await
            {
                Ok(()) => {
                    let Some(report) = stopped_report.clone() else {
                        break;
                    };
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
                    break;
                }
                Err(ShutdownCleanupError::Pending(message)) => {
                    if let Some(ack) = req.ack {
                        let _ = ack.send(Err(DaemonError::InvalidConfiguration(message.into())));
                    }
                }
                Err(ShutdownCleanupError::Failed(error)) => {
                    cleanup_failure = Some(error.to_string());
                    tracing::error!(error = %error, "Arrêt incomplet ; autorité non acquittée");
                    if let Some(ack) = req.ack {
                        let _ = ack.send(Err(error));
                    }
                }
            }
        }
        if cleanup_failure.is_some() {
            // A close/join failure is not proof that a detached mutator ended.
            // Retain the remaining authority even after its requesters disappear.
            std::future::pending::<()>().await;
        }
        if let Some(recovery_handle) = recovery_opt.take() {
            recovery_handle.abort();
        }
        if let Some(state) = &resources.state {
            state.broker_activity.close_and_wait().await;
        }
        let state_drop = resources.state.take();
        let store_drop = resources.store.take();
        let lock_drop = resources.lock.take();
        if state_drop.is_some() || store_drop.is_some() || lock_drop.is_some() {
            let _ = tokio::task::spawn_blocking(move || {
                drop(state_drop);
                drop(store_drop);
                drop(lock_drop);
            })
            .await;
        }
    });
}

async fn drain_connection_tasks(
    active_conns: &Mutex<Vec<tokio::task::JoinHandle<()>>>,
    deadline: Option<tokio::time::Instant>,
) -> Result<(), ShutdownCleanupError> {
    let tasks = {
        let mut guard = active_conns.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    let mut pending = std::collections::VecDeque::from(tasks);
    while let Some(mut task) = pending.pop_front() {
        let result = if let Some(deadline) = deadline {
            if let Ok(result) = tokio::time::timeout_at(deadline, &mut task).await {
                result
            } else {
                pending.push_front(task);
                active_conns
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend(pending);
                return Err(ShutdownCleanupError::Pending(
                    "Connexions IPC encore actives ; verrou d'autorité conservé",
                ));
            }
        } else {
            task.await
        };
        if let Err(error) = result {
            active_conns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(pending);
            return Err(ShutdownCleanupError::Failed(
                DaemonError::InvalidConfiguration(format!(
                    "Terminaison d'une connexion IPC interrompue : {error}"
                )),
            ));
        }
    }
    Ok(())
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
        let end_seq = ev.sequence.as_u64();
        let sid = ev.stream_id;
        publish_committed_events_ordered(state, sid, end_seq, 1, &[ev]).await;
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
        let end_seq = ev.sequence.as_u64();
        let sid = ev.stream_id;
        publish_committed_events_ordered(state, sid, end_seq, 1, &[ev]).await;
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
    let sock_for_val = socket_path.to_path_buf();
    tokio::task::spawn_blocking(move || validate_socket_file(&sock_for_val))
        .await
        .map_err(|e| {
            DaemonError::InvalidConfiguration(format!("Join error validate socket: {e}"))
        })??;
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
        admin_proof: None,
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

#[cfg(test)]
mod proof_cleanup_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use unleash_testkit::local_test_dir;

    #[test]
    fn failed_after_proof_publication_cleans_owned_credential_before_unlock() {
        let data = local_test_dir("m1-proof-cleanup-data").unwrap();
        let rdv = local_test_dir("m1-proof-cleanup-rdv").unwrap();
        std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let lock = AuthorityLock::acquire_in(rdv.path(), data.path()).unwrap();
        let result = install_admin_proof_with(rdv.path(), || {
            Err(DaemonError::InvalidConfiguration("injected".into()))
        });
        assert!(result.is_err());
        assert!(!rdv.path().join(ADMIN_PROOF_FILENAME).exists());
        drop(lock);
        assert!(AuthorityLock::acquire_in(rdv.path(), data.path()).is_ok());
    }

    #[test]
    fn failed_install_preserves_credential_replaced_by_another_value() {
        let data = local_test_dir("m1-proof-replaced-data").unwrap();
        let rdv = local_test_dir("m1-proof-replaced-rdv").unwrap();
        std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let _lock = AuthorityLock::acquire_in(rdv.path(), data.path()).unwrap();
        let path = rdv.path().join(ADMIN_PROOF_FILENAME);
        let replacement = "1".repeat(64);
        let result = install_admin_proof_with(rdv.path(), || {
            write_private_file_atomic(&path, replacement.as_bytes())?;
            Err(DaemonError::InvalidConfiguration("injected".into()))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), replacement.as_bytes());
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod shutdown_ownership_tests {
    use super::*;
    use unleash_testkit::local_test_dir;

    async fn failed_release_keeps_coordinator_authority(panics: bool) {
        let directory = local_test_dir("shutdown-release-failure").unwrap();
        let rendezvous = directory.path().join("rdv");
        let data = directory.path().join("data");
        let mut resources = ShutdownResources {
            state: None,
            store: None,
            lock: Some(Arc::new(Mutex::new(
                AuthorityLock::acquire_in(&rendezvous, &data).unwrap(),
            ))),
            accept: None,
        };
        let result = resources
            .release_authority_with(move |_lock| {
                assert!(!panics, "injected blocking release panic");
                Err(PlatformError::Io(std::io::Error::other(
                    "injected blocking release failure",
                )))
            })
            .await;
        assert!(matches!(result, Err(ShutdownCleanupError::Failed(_))));
        assert!(
            matches!(
                AuthorityLock::acquire_in(&rendezvous, &data),
                Err(PlatformError::DaemonAlreadyRunning { .. }
                    | PlatformError::AuthorityAlreadyActive { .. })
            ),
            "release error abandoned the live coordinator's authority"
        );
        assert!(resources.lock.is_some());
        resources
            .release_authority_with(AuthorityLock::release)
            .await
            .unwrap();
        assert!(resources.lock.is_none());
        assert!(AuthorityLock::acquire_in(&rendezvous, &data).is_ok());
    }

    #[tokio::test]
    async fn release_error_keeps_coordinator_authority() {
        failed_release_keeps_coordinator_authority(false).await;
    }

    #[tokio::test]
    async fn release_join_error_keeps_coordinator_authority() {
        failed_release_keeps_coordinator_authority(true).await;
    }

    // REQ-CORE-001 / REQ-CORE-007: a connection timeout must keep ownership
    // of an admitted blocking writer until its real completion.
    #[tokio::test]
    async fn timed_out_connection_keeps_its_blocking_store_mutator_tracked() {
        let directory = local_test_dir("shutdown-blocking-owner").unwrap();
        let root = directory.path().to_path_buf();
        let store = Arc::new(
            tokio::task::spawn_blocking(move || UnleashStore::open(StoreConfig::new(root)))
                .await
                .unwrap()
                .unwrap(),
        );
        let writer = Arc::clone(&store);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
                writer
                    .register_principal("late-mutator", "system", "fixture:late-mutator")
                    .unwrap();
                drop(writer);
                let _ = done_tx.send(());
            })
            .await
            .unwrap();
        });
        let connections = Mutex::new(vec![task]);
        started_rx.await.unwrap();
        let _ = drain_connection_tasks(&connections, Some(tokio::time::Instant::now())).await;
        let retained = connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let owners = Arc::strong_count(&store);
        release_tx.send(()).unwrap();
        done_rx.await.unwrap();
        let _ = drain_connection_tasks(
            &connections,
            Some(tokio::time::Instant::now() + Duration::from_secs(5)),
        )
        .await;
        tokio::task::spawn_blocking(move || drop(store))
            .await
            .unwrap();
        assert!(owners > 1, "the blocking mutator must still own the Store");
        assert_eq!(retained, 1, "timeout detached an admitted connection");
    }

    #[tokio::test]
    async fn shutdown_keeps_authority_until_a_detached_store_owner_really_finishes() {
        let directory = local_test_dir("shutdown-retained-store").unwrap();
        let data = directory.path().join("data");
        let rendezvous = directory.path().join("rdv");
        let configuration = DaemonConfig {
            data_root: Some(data.clone()),
            rendezvous_dir: Some(rendezvous.clone()),
            shutdown_drain_timeout: Duration::from_millis(10),
            ..DaemonConfig::default()
        };
        let DaemonStartOutcome::Started(handle) =
            UnleashDaemon::start(configuration.clone()).await.unwrap()
        else {
            panic!("a new fixture authority is required");
        };
        let retained_store = Arc::clone(&handle.test_state.upgrade().unwrap().store);
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match handle.shutdown_graceful("retained Store owner").await {
                    Err(DaemonError::InvalidConfiguration(message))
                        if message.starts_with("Propriétaires du Store encore actifs") =>
                    {
                        break;
                    }
                    Err(DaemonError::InvalidConfiguration(_)) => tokio::task::yield_now().await,
                    result => panic!("shutdown cannot finish with a retained Store: {result:?}"),
                }
            }
        })
        .await;
        assert!(error.is_ok(), "shutdown never reported the retained owner");
        assert!(matches!(
            AuthorityLock::acquire_in(&rendezvous, &data),
            Err(PlatformError::DaemonAlreadyRunning { .. })
        ));
        tokio::task::spawn_blocking(move || drop(retained_store))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if handle.shutdown_graceful("owner finished").await.is_ok() {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let DaemonStartOutcome::Started(restarted) =
            UnleashDaemon::start(configuration).await.unwrap()
        else {
            panic!("authority must be immediately reusable after shutdown Ok");
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if restarted
                    .shutdown_graceful("fixture complete")
                    .await
                    .is_ok()
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
    }
}
