// SPDX-License-Identifier: Apache-2.0
//! Serveur propriétaire `ProcessHostServer`, client `HostClient` et réconciliation hors ligne.
//!
//! Orchestre dans un processus dédié (`unleash-process-host`) :
//! - la vérification `SIGCHLD` et l'isolation `strict` Q-05 avant spawn ;
//! - le spawn du PTY (`portable-pty`) ou service (`setsid()`) et la capture d'[`OwnedChildAnchor`] ;
//! - le drainage continu vers [`LogSpool`] même sans client connecté ;
//! - l'écoute Unix privée (`0700`/`0600` + `verify_same_uid_peer`) avec authentification et
//!   fencing strict ;
//! - la persistance atomique de [`HostManifest`] et la réconciliation après perte du host ou reboot.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use unleash_domain::{AuthorityGeneration, DaemonEpoch, Fence, ProcessRunUuid};
use unleash_platform::{
    PlatformError, ProcessVerification, assert_not_under_claude, bind_private_unix_listener,
    ensure_private_rendezvous_dir, validate_rendezvous_dir, verify_same_uid_peer,
};
use unleash_protocol::codec::{read_frame, write_frame};
use zeroize::Zeroize;

use crate::anchor::{
    OwnedChildAnchor, OwnedChildHandle, ProcessInspector, configure_setsid_pre_exec,
    ensure_sigchld_qualified,
};
use crate::error::ProcessHostError;
use crate::fencing::{FencingGate, HostCommandHeader};
use crate::isolation::{IsolationBackendProbe, prepare_launch_command};
use crate::manifest::{
    HOST_MANIFEST_FILENAME, HostBootstrapSpec, HostControlSecret, HostLaunchState, HostManifest,
    ResourceKind, load_canary_secrets, load_control_secret, store_control_secret,
};
use crate::protocol::{HostRequest, HostResponse};
use crate::pty_input::{
    HostCutpointObserver, HostInputCutpoint, NoopHostCutpointObserver, PtyInputTracker,
};
use crate::spool::{LogSpool, SpoolConfig, SpoolIndex};

/// Nom du socket Unix privé de contrôle dans `runtime_dir`.
pub const HOST_SOCKET_FILENAME: &str = "host.sock";

/// Préfixe de la ligne émise sur `stdout` une fois le host lié et l'enfant démarré.
pub const HOST_READY_PREFIX: &str = "UNLEASH_HOST_READY:";

#[cfg(test)]
#[path = "host_control_tests.rs"]
mod control_tests;

#[path = "host_release.rs"]
mod release;

#[path = "host_strict_service.rs"]
mod strict_service;

pub use strict_service::{
    StrictServiceLaunchDigests, StrictServiceLaunchObservation, attest_strict_service_launch,
    inspect_strict_service_launch_plan, probe_strict_service_confinement,
};

const IO_CHUNK_BYTES: usize = 8192;
const SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENT_CONNECTIONS: usize = 16;

/// Traitement d'erreur de réconciliation suite à perte du host (`SIGKILL` process-host) ou reboot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostHostReconciliation {
    /// État recalculé du lancement (`Lost` si boot différent, `Exited` ou `Running`/inconnu sinon).
    pub reconciled_state: HostLaunchState,
    /// Vérification d'identité OS du dernier PID connu (observation seule, sans signal).
    pub os_verification: String,
    /// Index durable des logs vérifié SHA-256 sur disque.
    pub durable_spool: SpoolIndex,
    /// Dernière séquence d'entrée confirmée par reçu.
    pub last_confirmed_input_seq: u64,
    /// Séquences d'entrée passées en `OutcomeUnknown` suite à perte avant reçu.
    pub unknown_input_seqs: Vec<u64>,
    /// Preuve que le contrôle par signal est bloqué (`IdentityUnverified`) après perte du host.
    pub signal_refusal_reason: String,
}

struct ServerShared {
    // Serializes authentication/fencing with actual PTY effects and rotation.
    // Socket reads/writes are outside this gate and cannot pin its owner.
    operation_gate: Mutex<()>,
    last_transfer: Mutex<Option<ControlTransferAttempt>>,
    resize_attempts: AtomicU64,
    #[cfg(test)]
    operation_contentions: AtomicU64,
    manifest_path: PathBuf,
    control_secret_path: PathBuf,
    socket_path: PathBuf,
    manifest: Mutex<HostManifest>,
    control_secret: Mutex<HostControlSecret>,
    auth_generation: AtomicU64,
    fencing: Mutex<FencingGate>,
    anchor: OwnedChildAnchor,
    pty_master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    spool: Arc<LogSpool>,
    inputs: PtyInputTracker,
    cutpoints: Arc<dyn HostCutpointObserver>,
    shutdown: AtomicBool,
    active_connections: AtomicU64,
}

struct ControlTransferAttempt {
    header: HostCommandHeader,
    dispatch_id: unleash_domain::DispatchId,
    initial_context_digest: unleash_domain::Sha256Digest,
    ack: Option<crate::ControlTransferAck>,
}

/// Instance active d'un `unleash-process-host` prêt à servir les requêtes du daemon.
pub struct ProcessHostServer {
    shared: Arc<ServerShared>,
    listener: Mutex<Option<UnixListener>>,
    drain_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for ProcessHostServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHostServer")
            .field("socket_path", &self.shared.socket_path)
            .field("manifest_path", &self.shared.manifest_path)
            .finish_non_exhaustive()
    }
}

impl ProcessHostServer {
    /// Démarre le host : qualifie `SIGCHLD`, applique `IsolationMode::Strict` (fail-closed),
    /// écrit le manifeste durable `Prepared`, ouvre le PTY/service (`setsid`), capture l'ancrage
    /// direct non récolté, lance les threads de drainage continu et lie le socket privé `0600`.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] (incluant `IsolationBackendMissing` /
    /// `IsolationObligationUnenforceable`) avant tout lancement enfant si les préconditions échouent.
    #[expect(
        clippy::too_many_lines,
        reason = "Séquence atomique de démarrage : SIGCHLD, isolation, manifeste, spawn PTY/service, ancrage, drainage et socket"
    )]
    pub fn start(
        spec: &HostBootstrapSpec,
        isolation_probe: &dyn IsolationBackendProbe,
        inspector: Box<dyn ProcessInspector>,
        cutpoint_observer: Option<Arc<dyn HostCutpointObserver>>,
    ) -> Result<Self, ProcessHostError> {
        assert_not_under_claude(&spec.runtime_dir)?;
        ensure_private_rendezvous_dir(&spec.runtime_dir)?;
        validate_rendezvous_dir(&spec.runtime_dir)?;

        let sigchld = ensure_sigchld_qualified()?;
        let initial_secret = load_control_secret(&spec.control_secret_path)?;
        let canaries = match &spec.canaries_secret_path {
            Some(p) => load_canary_secrets(p)?,
            None => Vec::new(),
        };

        let canon_cwd =
            std::fs::canonicalize(&spec.cwd).map_err(|e| ProcessHostError::InvalidArgument {
                reason: format!("cwd '{}' inaccessible : {e}", spec.cwd.display()),
            })?;
        assert_not_under_claude(&canon_cwd)?;

        // Fail-closed avant tout spawn : valide et sonde l'isolation OS si mode strict.
        let (prepared, effective_environment_digest) = match spec.environment_mode {
            None => (
                prepare_launch_command(
                    &spec.program,
                    &spec.arguments,
                    &spec.explicit_env,
                    spec.isolation_mode,
                    spec.strict_spec.as_ref(),
                    isolation_probe,
                )?,
                None,
            ),
            Some(crate::manifest::EnvironmentMode::ExplicitV1) => {
                let invalid = || ProcessHostError::InvalidArgument {
                    reason: "explicit terminal environment is not qualified".into(),
                };
                // Strict with an explicit managed TMPDIR requires its own confinement qualification.
                // Refuse before a probe or child instead of inheriting or weakening the request.
                if spec.isolation_mode != crate::isolation::IsolationMode::Standard
                    || !Path::new(&spec.program).is_absolute()
                    || spec.program.contains('\0')
                    || spec.arguments.iter().any(|v| v.contains('\0'))
                    || spec.allocation_context_digest.is_none()
                    || canon_cwd != spec.cwd
                {
                    return Err(invalid());
                }
                let runtime = std::fs::canonicalize(&spec.runtime_dir)?;
                if runtime != spec.runtime_dir {
                    return Err(invalid());
                }
                let child_tmp = runtime.join("child-tmp");
                let clean_env =
                    crate::manifest::explicit_environment_v1(&spec.explicit_env, &child_tmp)?;
                let digest = crate::manifest::effective_environment_digest_v1(&clean_env)?;
                if spec.expected_effective_environment_digest != Some(digest) {
                    return Err(invalid());
                }
                unleash_platform::ensure_private_rendezvous_dir(&child_tmp)?;
                unleash_platform::validate_rendezvous_dir(&child_tmp)?;
                (
                    crate::isolation::PreparedLaunchCommand {
                        executable: PathBuf::from(&spec.program),
                        args: spec.arguments.clone(),
                        clean_env,
                        backend_name: "none_standard".into(),
                        profile_digest: None,
                        profile_path: None,
                    },
                    Some(digest),
                )
            }
            Some(crate::manifest::EnvironmentMode::StrictServiceV1) => {
                let (prepared, digest) =
                    strict_service::prepare(spec, &canon_cwd, isolation_probe)?;
                (prepared, Some(digest))
            }
        };

        let command_digest = spec.compute_command_digest()?;
        let socket_path = spec.runtime_dir.join(HOST_SOCKET_FILENAME);
        let spool_dir = spec.runtime_dir.join("spool");
        let manifest_path = spec.runtime_dir.join(HOST_MANIFEST_FILENAME);
        let owner_uid = unleash_platform::effective_uid();
        let boot_id = inspector.current_boot()?;

        let mut manifest = HostManifest {
            schema_version: 1,
            host_id: spec.host_id,
            process_id: spec.process_id,
            terminal_id: spec.terminal_id,
            process_run_uuid: spec.process_run_uuid,
            authority_generation: spec.authority_generation,
            daemon_epoch: spec.daemon_epoch,
            highest_fence: spec.resource_fence,
            action_id: spec.action_id,
            mission_id: spec.mission_id,
            owner_agent_id: spec.owner_agent_id,
            cwd_canonical: canon_cwd.clone(),
            worktree_id: spec.worktree_id,
            command_digest,
            effective_environment_digest,
            control_state: Some(crate::HostControlState::Active {
                last_reacquisition: None,
            }),
            allocation_context_digest: if spec.environment_mode.is_some() {
                spec.allocation_context_digest
            } else {
                None
            },
            owner_uid,
            pid: None,
            pgid: None,
            boot_identity: Some(boot_id),
            start_identity: None,
            identity_backend: current_identity_backend(),
            isolation_mode: spec.isolation_mode,
            isolation_backend: Some(prepared.backend_name.clone()),
            isolation_profile_digest: prepared.profile_digest,
            socket_path: socket_path.clone(),
            spool_dir: spool_dir.clone(),
            control_secret_path: spec.control_secret_path.clone(),
            launch_state: HostLaunchState::Prepared,
            last_confirmed_input_seq: 0,
            in_flight_input_seq: None,
            durable_log_seq: 0,
        };
        manifest.save_atomic(&manifest_path)?;

        let spool_cfg = SpoolConfig {
            max_segment_bytes: spec.max_segment_bytes.max(64),
            max_total_bytes: spec.max_total_log_bytes.max(spec.max_segment_bytes.max(64)),
            max_volatile_bytes: spec.max_volatile_bytes.max(64),
        };
        let spool = Arc::new(LogSpool::open(&spool_dir, spool_cfg, &canaries)?);
        let cutpoints = cutpoint_observer.unwrap_or_else(|| Arc::new(NoopHostCutpointObserver));
        let inputs = PtyInputTracker::open(&spec.runtime_dir, Some(Arc::clone(&cutpoints)))?;

        let mut drain_handles = Vec::new();
        let (anchor, pty_master, writer): (
            OwnedChildAnchor,
            Option<Box<dyn MasterPty + Send>>,
            Option<Box<dyn Write + Send>>,
        ) = match spec.resource_kind {
            ResourceKind::Pty { rows, cols } => {
                if rows == 0 || cols == 0 {
                    return Err(ProcessHostError::InvalidArgument {
                        reason: String::from("dimensions PTY doivent être strictement positives"),
                    });
                }
                let pty_system = native_pty_system();
                let pair = pty_system
                    .openpty(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|e| ProcessHostError::TerminalUnavailable {
                        reason: format!("échec openpty : {e}"),
                    })?;

                let mut argv = Vec::with_capacity(prepared.args.len().saturating_add(1));
                argv.push(prepared.executable.as_os_str().to_os_string());
                for a in &prepared.args {
                    argv.push(std::ffi::OsString::from(a));
                }
                let mut builder = CommandBuilder::from_argv(argv);
                builder.env_clear();
                for (k, v) in &prepared.clean_env {
                    builder.env(k, v);
                }
                builder.cwd(&canon_cwd);

                let child = pair.slave.spawn_command(builder).map_err(|e| {
                    ProcessHostError::TerminalUnavailable {
                        reason: format!("échec spawn PTY : {e}"),
                    }
                })?;
                // Fermer immédiatement le côté esclave dans le host pour que EOF remonte à la fin de l'enfant.
                drop(pair.slave);

                let anchor = OwnedChildAnchor::capture_direct_child(
                    OwnedChildHandle::Pty(child),
                    sigchld,
                    inspector,
                )?;
                let reader = pair.master.try_clone_reader().map_err(|e| {
                    ProcessHostError::TerminalUnavailable {
                        reason: format!("échec ouverture lecteur PTY : {e}"),
                    }
                })?;
                let master_writer = pair.master.take_writer().map_err(|e| {
                    ProcessHostError::TerminalUnavailable {
                        reason: format!("échec ouverture écrivain PTY : {e}"),
                    }
                })?;

                let drain_spool = Arc::clone(&spool);
                let handle = thread::Builder::new()
                    .name(String::from("unleash-pty-drain"))
                    .spawn(move || drain_reader_to_spool(reader, &drain_spool, true))?;
                drain_handles.push(handle);

                (anchor, Some(pair.master), Some(master_writer))
            }
            ResourceKind::Service => {
                let mut cmd = Command::new(&prepared.executable);
                cmd.args(&prepared.args)
                    .env_clear()
                    .envs(&prepared.clean_env)
                    .current_dir(&canon_cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                configure_setsid_pre_exec(&mut cmd);

                let mut child = cmd.spawn()?;
                let child_stdin = child.stdin.take();
                let child_stdout = child.stdout.take();
                let child_stderr = child.stderr.take();

                let anchor = OwnedChildAnchor::capture_direct_child(
                    OwnedChildHandle::Service(child),
                    sigchld,
                    inspector,
                )?;

                if let Some(stdout) = child_stdout {
                    let s = Arc::clone(&spool);
                    let h = thread::Builder::new()
                        .name(String::from("unleash-svc-stdout"))
                        .spawn(move || drain_reader_to_spool(stdout, &s, true))?;
                    drain_handles.push(h);
                }
                if let Some(stderr) = child_stderr {
                    let s = Arc::clone(&spool);
                    let h = thread::Builder::new()
                        .name(String::from("unleash-svc-stderr"))
                        .spawn(move || drain_reader_to_spool(stderr, &s, true))?;
                    drain_handles.push(h);
                }

                let writer_box: Option<Box<dyn Write + Send>> =
                    child_stdin.map(|w| Box::new(w) as Box<dyn Write + Send>);
                (anchor, None, writer_box)
            }
        };

        let expected_id = anchor.expected_identity();
        manifest.pid = Some(expected_id.pid);
        manifest.pgid = Some(expected_id.pgid);
        manifest.boot_identity = Some(expected_id.boot_identity.clone());
        manifest.start_identity = Some(expected_id.start_identity.clone());
        manifest.identity_backend = expected_id.backend;
        manifest.launch_state = HostLaunchState::Running;
        manifest.save_atomic(&manifest_path)?;

        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = bind_private_unix_listener(&socket_path)?;

        let shared = Arc::new(ServerShared {
            operation_gate: Mutex::new(()),
            last_transfer: Mutex::new(None),
            resize_attempts: AtomicU64::new(0),
            #[cfg(test)]
            operation_contentions: AtomicU64::new(0),
            manifest_path,
            control_secret_path: spec.control_secret_path.clone(),
            socket_path,
            manifest: Mutex::new(manifest),
            control_secret: Mutex::new(initial_secret),
            auth_generation: AtomicU64::new(1),
            fencing: Mutex::new(FencingGate::new(
                spec.process_run_uuid,
                spec.authority_generation,
                spec.daemon_epoch,
                spec.resource_fence,
            )),
            anchor,
            pty_master: Mutex::new(pty_master),
            writer: Mutex::new(writer),
            spool,
            inputs,
            cutpoints,
            shutdown: AtomicBool::new(false),
            active_connections: AtomicU64::new(0),
        });

        Ok(Self {
            shared,
            listener: Mutex::new(Some(listener)),
            drain_handles: Mutex::new(drain_handles),
        })
    }

    /// Chemin du socket Unix privé d'écoute.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.shared.socket_path
    }

    /// Chemin du manifeste privé durable.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.shared.manifest_path
    }

    /// UUID de l'exécution physique gérée.
    #[must_use]
    pub fn process_run_uuid(&self) -> ProcessRunUuid {
        self.shared
            .manifest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_run_uuid
    }

    /// Retourne l'identité OS de l'enfant capturée au spawn.
    #[must_use]
    pub fn child_identity(&self) -> unleash_platform::OsProcessIdentity {
        self.shared.anchor.expected_identity()
    }

    /// Demande l'arrêt de la boucle d'écoute du serveur (sans tuer l'enfant si déjà terminé).
    pub fn request_shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.shared.socket_path);
    }

    /// Sert les requêtes Unix jusqu'à réception d'une commande `Shutdown` ou `request_shutdown()`,
    /// puis joint les threads de drainage et synchronise durablement les derniers octets de spool.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le listener a déjà été consommé ou en cas d'erreur fatale.
    pub fn serve_until_shutdown(&self) -> Result<(), ProcessHostError> {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| ProcessHostError::TerminalUnavailable {
                reason: String::from("listener Unix déjà consommé"),
            })?;

        let mut conn_handles: Vec<JoinHandle<()>> = Vec::new();

        while !self.shared.shutdown.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    if self.shared.shutdown.load(Ordering::SeqCst) {
                        drop(stream);
                        break;
                    }
                    let active = self.shared.active_connections.load(Ordering::SeqCst);
                    if active >= u64::try_from(MAX_CONCURRENT_CONNECTIONS).unwrap_or(16) {
                        tracing::warn!(
                            "saturation des connexions process-host : connexion refusée"
                        );
                        drop(stream);
                        continue;
                    }
                    let shared_for_conn = Arc::clone(&self.shared);
                    self.shared
                        .active_connections
                        .fetch_add(1, Ordering::SeqCst);
                    if let Ok(handle) = thread::Builder::new()
                        .name(String::from("unleash-host-conn"))
                        .spawn(move || {
                            handle_connection(stream, &shared_for_conn);
                            shared_for_conn
                                .active_connections
                                .fetch_sub(1, Ordering::SeqCst);
                        })
                    {
                        conn_handles.push(handle);
                    } else {
                        self.shared
                            .active_connections
                            .fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(err) => {
                    if self.shared.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    return Err(ProcessHostError::Io(err));
                }
            }
        }

        // Fermer le côté écriture uniquement à la fermeture du serveur.
        let _ = self
            .shared
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        for h in conn_handles {
            let _ = h.join();
        }

        let drains = std::mem::take(
            &mut *self
                .drain_handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for h in drains {
            let _ = h.join();
        }

        let durable_seq = self.shared.spool.flush_and_sync_durable()?;
        let polled_exit = self.shared.anchor.poll_exit(true).ok().flatten();
        {
            let mut mf = self
                .shared
                .manifest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            mf.durable_log_seq = durable_seq;
            if let Some(st) = polled_exit.or_else(|| self.shared.anchor.exit_status()) {
                mf.launch_state = HostLaunchState::Exited {
                    exit_code: st.exit_code,
                    exit_signal: st.exit_signal,
                };
            }
            let _ = mf.save_atomic(&self.shared.manifest_path);
        }

        Ok(())
    }
}

fn drain_reader_to_spool<R: Read>(mut reader: R, spool: &LogSpool, sync_on_eof: bool) {
    let mut buf = [0_u8; IO_CHUNK_BYTES];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Some(chunk) = buf.get(..n)
                    && let Err(err) = spool.append_raw_chunk(chunk)
                {
                    tracing::error!(error = %err, "échec écriture spool lors du drainage");
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => {
                // Sur macOS/Linux, EIO sur maître PTY survient quand le côté esclave ferme.
                if err.raw_os_error() == Some(libc::EIO) {
                    break;
                }
                tracing::warn!(error = %err, "fin de lecture flux processus");
                break;
            }
        }
    }
    if sync_on_eof {
        let _ = spool.flush_and_sync_durable();
    }
}

fn handle_connection(mut stream: UnixStream, shared: &Arc<ServerShared>) {
    let _ = stream.set_read_timeout(Some(SOCKET_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT));

    if let Err(err) = verify_same_uid_peer(&stream) {
        let ipc = ProcessHostError::Platform(err).to_ipc_error(None);
        let _ = send_response(&mut stream, &HostResponse::Error { error: ipc });
        return;
    }

    let mut bound_auth_gen = shared.auth_generation.load(Ordering::SeqCst);

    loop {
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let mut frame = match read_frame(&mut stream) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(err) => {
                if let unleash_protocol::ProtocolError::Io(io_err) = &err
                    && matches!(
                        io_err.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    )
                {
                    if shared.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    continue;
                }
                let ipc = ProcessHostError::Protocol(err).to_ipc_error(None);
                let _ = send_response(&mut stream, &HostResponse::Error { error: ipc });
                break;
            }
        };

        match handle_private_control(&mut stream, &mut frame, shared, &mut bound_auth_gen) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(_) => break,
        }

        let decoded = serde_json::from_slice::<HostRequest>(&frame);
        frame.zeroize();
        let req: HostRequest = match decoded {
            Ok(r) => r,
            Err(err) => {
                let ipc = ProcessHostError::Json(err).to_ipc_error(None);
                let _ = send_response(&mut stream, &HostResponse::Error { error: ipc });
                continue;
            }
        };

        let operation = lock_operation(shared);
        // Recheck channel generation under the same gate as fencing and I/O.
        // Si le secret a tourné après un Handshake de récupération sur une autre connexion,
        // invalider immédiatement toute ancienne connexion ouverte avant toute opération.
        let current_auth_gen = shared.auth_generation.load(Ordering::SeqCst);
        if current_auth_gen != bound_auth_gen && !matches!(req, HostRequest::Handshake { .. }) {
            let err = ProcessHostError::AuthenticationFailed {
                reason: String::from(
                    "canal obsolète invalidé par rotation de capacité lors d'une reprise",
                ),
            };
            drop(operation);
            let _ = send_response(
                &mut stream,
                &HostResponse::Error {
                    error: err.to_ipc_error(None),
                },
            );
            break;
        }

        let resize_before = shared.resize_attempts.load(Ordering::SeqCst);
        let resp = process_request(req, shared);
        let resize_after = shared.resize_attempts.load(Ordering::SeqCst);
        if matches!(
            resp,
            HostResponse::HandshakeAck {
                secret_rotated: true,
                ..
            }
        ) {
            bound_auth_gen = shared.auth_generation.load(Ordering::SeqCst);
        }
        let resize_attempt = (resize_before != resize_after).then_some(resize_after);
        drop(operation);
        let sent = send_host_reply(&mut stream, shared, &resp, resize_attempt);
        if sent.is_err() || matches!(resp, HostResponse::ShutdownAck) {
            break;
        }
    }
}

fn send_host_reply(
    stream: &mut UnixStream,
    shared: &ServerShared,
    response: &HostResponse,
    resize_attempt: Option<u64>,
) -> Result<(), ProcessHostError> {
    if let Some(attempt) = resize_attempt {
        shared.cutpoints.after_resize_attempt(attempt)?;
    }
    if matches!(response, HostResponse::HandshakeAck { .. }) {
        shared
            .cutpoints
            .hit(HostInputCutpoint::HandshakeAppliedBeforeAck)?;
    }
    send_response(stream, response)
}

fn lock_operation(shared: &ServerShared) -> std::sync::MutexGuard<'_, ()> {
    match shared.operation_gate.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            #[cfg(test)]
            shared.operation_contentions.fetch_add(1, Ordering::SeqCst);
            shared
                .operation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }
}

fn validate_new_transfer(
    request: &crate::ControlTransferRequest,
    shared: &ServerShared,
) -> Result<(), ProcessHostError> {
    if verify_secret(shared, &request.next_secret).is_ok() {
        return Err(ProcessHostError::InvalidArgument {
            reason: "control transfer requires a different capability".into(),
        });
    }
    let _validated_secret = HostControlSecret::new(request.next_secret.clone())?;
    let manifest = shared
        .manifest
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if manifest.allocation_context_digest != Some(request.initial_context_digest)
        || manifest.effective_environment_digest.is_none()
    {
        return Err(ProcessHostError::IdentityUnverified {
            reason: "control transfer launch binding differs".into(),
        });
    }
    drop(manifest);
    let anchor = shared.anchor.verification_report();
    if anchor.reaped || !anchor.sigchld_qualified || anchor.identity_status != "match" {
        return Err(ProcessHostError::IdentityUnverified {
            reason: "control transfer requires the same live anchored child".into(),
        });
    }
    let mut candidate = *shared
        .fencing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if request.header.resource_fence <= candidate.current().fence {
        return Err(ProcessHostError::StaleEpochOrFence {
            reason: "control transfer requires a higher resource fence".into(),
        });
    }
    candidate.check_recovery_handshake(&request.header, true)?;
    Ok(())
}

fn respond_to_control_transfer(
    stream: &mut UnixStream,
    request: &crate::ControlTransferRequest,
    shared: &ServerShared,
    bound_auth_gen: &mut u64,
) -> Result<(), ProcessHostError> {
    let response = {
        let _operation = lock_operation(shared);
        #[cfg(test)]
        let auth_before = shared.auth_generation.load(Ordering::SeqCst);
        let response = match process_control_transfer(request, shared) {
            Ok(ack) => crate::ControlTransferResponse::Ack { ack: Box::new(ack) },
            Err(error) => crate::ControlTransferResponse::Refused {
                error: error.to_ipc_error(Some(request.header.command_id)),
            },
        };
        if matches!(response, crate::ControlTransferResponse::Ack { .. }) {
            *bound_auth_gen = shared.auth_generation.load(Ordering::SeqCst);
        }
        #[cfg(test)]
        let response = {
            let transfer_applied = matches!(response, crate::ControlTransferResponse::Ack { .. })
                && shared.auth_generation.load(Ordering::SeqCst) != auth_before;
            (response, transfer_applied)
        };
        response
    };
    #[cfg(test)]
    let (response, transfer_applied) = response;
    let payload = serde_json::to_vec(&response)?;
    #[cfg(test)]
    if transfer_applied {
        shared
            .cutpoints
            .hit(HostInputCutpoint::TransferAppliedBeforeAck)?;
    }
    write_frame(stream, &payload)?;
    Ok(())
}

fn handle_private_control(
    stream: &mut UnixStream,
    frame: &mut Vec<u8>,
    shared: &ServerShared,
    bound_auth_gen: &mut u64,
) -> Result<bool, ProcessHostError> {
    if let Ok(crate::control_transfer::TransferEnvelope::TransferControlV1(request)) =
        serde_json::from_slice(frame)
    {
        frame.zeroize();
        respond_to_control_transfer(stream, &request, shared, bound_auth_gen)?;
        return Ok(true);
    }
    if let Ok(request) = serde_json::from_slice::<crate::control_release::ReleaseEnvelope>(frame) {
        frame.zeroize();
        release::respond(stream, &request, shared, bound_auth_gen)?;
        return Ok(true);
    }
    Ok(false)
}

fn process_control_transfer(
    request: &crate::ControlTransferRequest,
    shared: &ServerShared,
) -> Result<crate::ControlTransferAck, ProcessHostError> {
    verify_secret(shared, &request.current_secret)?;
    // A historical ACK is not an activation path after explicit retirement.
    release::require_input(shared)?;
    {
        let previous = shared
            .last_transfer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = previous.as_ref() {
            if previous.header == request.header
                && previous.dispatch_id == request.dispatch_id
                && previous.initial_context_digest == request.initial_context_digest
            {
                // After an ACK is lost only the currently active capability
                // can inspect the same transfer; no second rotation occurs.
                verify_secret(shared, &request.next_secret)?;
                return previous.ack.clone().ok_or_else(|| {
                    ProcessHostError::LaunchOutcomeUnknown {
                        reason: "control transfer requires inspection; no repeat permit".into(),
                    }
                });
            }
            if previous.ack.is_none() {
                return Err(ProcessHostError::LaunchOutcomeUnknown {
                    reason: "a control transfer is unresolved".into(),
                });
            }
        }
    }
    validate_new_transfer(request, shared)?;
    *shared
        .last_transfer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ControlTransferAttempt {
        header: request.header.clone(),
        dispatch_id: request.dispatch_id,
        initial_context_digest: request.initial_context_digest,
        ack: None,
    });
    let response = process_request(
        HostRequest::Handshake {
            header: request.header.clone(),
            current_secret: request.current_secret.clone(),
            rotate_to_secret: Some(request.next_secret.clone()),
            require_higher_epoch: true,
        },
        shared,
    );
    let HostResponse::HandshakeAck {
        os_identity,
        effective_environment_digest: Some(effective_environment_digest),
        allocation_context_digest: Some(initial_context_digest),
        durable_log_seq,
        last_confirmed_input_seq,
        unknown_input_seqs,
        secret_rotated: true,
        ..
    } = response
    else {
        return Err(ProcessHostError::LaunchOutcomeUnknown {
            reason: "control transfer has no complete acknowledgement".into(),
        });
    };
    let ack = crate::ControlTransferAck {
        header: request.header.clone(),
        dispatch_id: request.dispatch_id,
        initial_context_digest,
        effective_environment_digest,
        os_identity,
        durable_log_seq,
        last_confirmed_input_seq,
        unknown_input_seqs,
    };
    if let Some(attempt) = shared
        .last_transfer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()
    {
        attempt.ack = Some(ack.clone());
    }
    Ok(ack)
}

fn send_response(stream: &mut UnixStream, resp: &HostResponse) -> Result<(), ProcessHostError> {
    let payload = serde_json::to_vec(resp)?;
    write_frame(stream, &payload)?;
    Ok(())
}

fn verify_secret(shared: &ServerShared, candidate: &str) -> Result<(), ProcessHostError> {
    let guard = shared
        .control_secret
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.constant_time_eq(candidate) {
        Ok(())
    } else {
        Err(ProcessHostError::AuthenticationFailed {
            reason: String::from("secret de contrôle invalide ou périmé"),
        })
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Dispatch exhaustif des requêtes host avec auth, fencing et projection IpcError"
)]
fn process_request(req: HostRequest, shared: &ServerShared) -> HostResponse {
    let cmd_id = match &req {
        HostRequest::Handshake { header, .. }
        | HostRequest::SendInput { header, .. }
        | HostRequest::Resize { header, .. }
        | HostRequest::ReadLog { header, .. }
        | HostRequest::Stop { header, .. }
        | HostRequest::Reconcile { header, .. }
        | HostRequest::ModelCallAttempt { header, .. }
        | HostRequest::Shutdown { header, .. } => header.command_id,
    };

    let result: Result<HostResponse, ProcessHostError> = match req {
        HostRequest::ModelCallAttempt { prompt_summary, .. } => {
            Err(ProcessHostError::ModelCallsForbidden {
                reason: format!(
                    "tentative d'appel modèle rejetée par unleash-process-host ({prompt_summary})"
                ),
            })
        }
        HostRequest::Handshake {
            header,
            current_secret,
            rotate_to_secret,
            require_higher_epoch,
        } => verify_secret(shared, &current_secret).and_then(|()| {
            if rotate_to_secret.is_some() {
                release::require_input(shared)?;
            }
            let mut fencing = shared
                .fencing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            fencing.check_recovery_handshake(&header, require_higher_epoch)?;
            let current_fence = fencing.current();
            drop(fencing);

            let mut secret_rotated = false;
            if let Some(new_raw) = rotate_to_secret {
                let new_secret = HostControlSecret::new(new_raw)?;
                store_control_secret(&shared.control_secret_path, &new_secret)?;
                *shared
                    .control_secret
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = new_secret;
                shared.auth_generation.fetch_add(1, Ordering::SeqCst);
                secret_rotated = true;
                if let Some(crate::HostControlState::Active { last_reacquisition }) = shared
                    .manifest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .control_state
                    .as_mut()
                {
                    *last_reacquisition = None;
                }
            }

            shared.inputs.reconcile_in_flight()?;
            let (last_confirmed_input_seq, unknown_input_seqs) = shared.inputs.summary();
            let durable_log_seq = shared.spool.snapshot_index().durable_watermark_offset;

            let (effective_environment_digest, allocation_context_digest) = {
                let mut mf = shared
                    .manifest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                mf.daemon_epoch = current_fence.epoch;
                mf.highest_fence = current_fence.fence;
                mf.durable_log_seq = durable_log_seq;
                mf.last_confirmed_input_seq = last_confirmed_input_seq;
                mf.in_flight_input_seq = unknown_input_seqs.first().copied();
                mf.save_atomic(&shared.manifest_path)?;
                (
                    mf.effective_environment_digest,
                    mf.allocation_context_digest,
                )
            };

            Ok(HostResponse::HandshakeAck {
                effective_environment_digest,
                allocation_context_digest,
                process_run_uuid: header.process_run_uuid,
                authority_generation: current_fence.authority_generation,
                daemon_epoch: current_fence.epoch,
                highest_fence: current_fence.fence,
                os_identity: shared.anchor.expected_identity(),
                durable_log_seq,
                last_confirmed_input_seq,
                unknown_input_seqs,
                secret_rotated,
            })
        }),
        HostRequest::SendInput {
            header,
            secret,
            input_seq,
            payload_bytes,
            now_ms,
        } => verify_secret(shared, &secret).and_then(|()| {
            release::require_input(shared)?;
            {
                let mut fencing = shared
                    .fencing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                fencing.check_and_advance(&header)?;
            }
            let mut writer_guard = shared
                .writer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let writer =
                writer_guard
                    .as_mut()
                    .ok_or_else(|| ProcessHostError::TerminalUnavailable {
                        reason: String::from("flux d'écriture PTY/stdin fermé"),
                    })?;
            let receipt = shared.inputs.dispatch_input(
                writer.as_mut(),
                input_seq,
                header.command_id,
                header.action_id,
                &payload_bytes,
                now_ms,
            )?;
            {
                let mut mf = shared
                    .manifest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                mf.last_confirmed_input_seq = receipt.input_seq;
                mf.in_flight_input_seq = None;
                let _ = mf.save_atomic(&shared.manifest_path);
            }
            Ok(HostResponse::InputReceipt { receipt })
        }),
        HostRequest::Resize {
            header,
            secret,
            rows,
            cols,
        } => verify_secret(shared, &secret).and_then(|()| {
            release::require_input(shared)?;
            if rows == 0 || cols == 0 || rows > 500 || cols > 1000 {
                return Err(ProcessHostError::InvalidArgument {
                    reason: format!("dimensions PTY hors bornes ({rows}x{cols})"),
                });
            }
            {
                let mut fencing = shared
                    .fencing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                fencing.check_and_advance(&header)?;
            }
            let master_guard = shared
                .pty_master
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let master =
                master_guard
                    .as_ref()
                    .ok_or_else(|| ProcessHostError::InvalidArgument {
                        reason: String::from("Resize interdit sur ressource non-PTY"),
                    })?;
            shared
                .resize_attempts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                    count.checked_add(1)
                })
                .map_err(|_| ProcessHostError::TerminalUnavailable {
                    reason: String::from("native resize attempt counter exhausted"),
                })?;
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| ProcessHostError::TerminalUnavailable {
                    reason: format!("échec TIOCSWINSZ : {e}"),
                })?;
            let observed =
                master
                    .get_size()
                    .map_err(|e| ProcessHostError::TerminalUnavailable {
                        reason: format!("échec lecture taille PTY après resize : {e}"),
                    })?;
            Ok(HostResponse::ResizeAck {
                rows: observed.rows,
                cols: observed.cols,
            })
        }),
        HostRequest::ReadLog {
            header,
            secret,
            after_offset,
            max_bytes,
            strict_cursor,
        } => verify_secret(shared, &secret).and_then(|()| {
            {
                let mut fencing = shared
                    .fencing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                fencing.check_and_advance(&header)?;
            }
            let _ = shared.spool.sync_durable_now();
            let result = shared
                .spool
                .read_log(after_offset, max_bytes, strict_cursor)?;
            Ok(HostResponse::LogChunk { result })
        }),
        HostRequest::Stop {
            header,
            secret,
            escalate_to_sigkill,
            known_descendant_pids,
        } => verify_secret(shared, &secret).and_then(|()| {
            {
                let mut fencing = shared
                    .fencing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                fencing.check_and_advance(&header)?;
            }
            let observation = shared
                .anchor
                .stop_and_observe(escalate_to_sigkill, &known_descendant_pids)?;
            let durable_seq = shared.spool.flush_and_sync_durable()?;
            {
                let mut mf = shared
                    .manifest
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                mf.durable_log_seq = durable_seq;
                if let Some(st) = observation.direct_exit {
                    mf.launch_state = HostLaunchState::Exited {
                        exit_code: st.exit_code,
                        exit_signal: st.exit_signal,
                    };
                }
                let _ = mf.save_atomic(&shared.manifest_path);
            }
            Ok(HostResponse::StopObserved { observation })
        }),
        HostRequest::Reconcile { header, secret } => {
            verify_secret(shared, &secret).and_then(|()| {
                {
                    let mut fencing = shared
                        .fencing
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    fencing.check_and_advance(&header)?;
                }
                shared.inputs.reconcile_in_flight()?;
                let (last_confirmed_input_seq, unknown_input_seqs) = shared.inputs.summary();
                let durable_log_seq = shared.spool.sync_durable_now()?;
                let exit_status = shared.anchor.poll_exit(false)?;
                let anchor_report = shared.anchor.verification_report();
                let launch_state = {
                    let mut mf = shared
                        .manifest
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    mf.durable_log_seq = durable_log_seq;
                    mf.last_confirmed_input_seq = last_confirmed_input_seq;
                    mf.in_flight_input_seq = unknown_input_seqs.first().copied();
                    if let Some(st) = exit_status {
                        mf.launch_state = HostLaunchState::Exited {
                            exit_code: st.exit_code,
                            exit_signal: st.exit_signal,
                        };
                    }
                    let _ = mf.save_atomic(&shared.manifest_path);
                    mf.launch_state.clone()
                };
                Ok(HostResponse::ReconcileState {
                    launch_state,
                    os_verification: anchor_report.identity_status.clone(),
                    anchor: anchor_report,
                    durable_log_seq,
                    last_confirmed_input_seq,
                    unknown_input_seqs,
                    exit_status,
                })
            })
        }
        HostRequest::Shutdown { header, secret } => verify_secret(shared, &secret).and_then(|()| {
            {
                let mut fencing = shared
                    .fencing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                fencing.check_and_advance(&header)?;
            }
            shared.shutdown.store(true, Ordering::SeqCst);
            let _ = UnixStream::connect(&shared.socket_path);
            Ok(HostResponse::ShutdownAck)
        }),
    };

    match result {
        Ok(resp) => resp,
        Err(err) => HostResponse::Error {
            error: err.to_ipc_error(Some(cmd_id)),
        },
    }
}

/// Réconcilie un host disparu (`SIGKILL` process-host) ou obsolète après reboot uniquement
/// depuis son manifeste privé et son spool durable sur disque (`REQ-TERM-002`, `REQ-TERM-010`).
///
/// Ne tente jamais de signaler l'ancien PID observé : toute tentative de contrôle actif retourne
/// [`ProcessHostError::IdentityUnverified`].
///
/// # Errors
/// Retourne [`ProcessHostError`] si le manifeste ou le spool sont absents/corrompus.
pub fn reconcile_after_lost_host(
    manifest_path: &Path,
    inspector: &dyn ProcessInspector,
) -> Result<LostHostReconciliation, ProcessHostError> {
    let manifest = HostManifest::load_private(manifest_path)?;
    let durable_spool = LogSpool::verify_durable_segments(&manifest.spool_dir)?;
    let tracker_dir = manifest_path.parent().ok_or_else(|| {
        ProcessHostError::Platform(PlatformError::InvalidPath {
            path: manifest_path.display().to_string(),
            reason: String::from("manifeste sans répertoire parent"),
        })
    })?;
    let tracker = PtyInputTracker::open(tracker_dir, None)?;
    let unknown_input_seqs = tracker.reconcile_in_flight()?;
    let (last_confirmed_input_seq, all_unknowns) = tracker.summary();
    let combined_unknowns = if all_unknowns.is_empty() {
        unknown_input_seqs
    } else {
        all_unknowns
    };

    let current_boot = inspector.current_boot()?;
    let (reconciled_state, os_verification) = match &manifest.boot_identity {
        Some(expected_boot) if *expected_boot != current_boot => (
            HostLaunchState::Lost,
            String::from("boot_changed -> lost (processus réputé disparu)"),
        ),
        _ => match (
            manifest.pid,
            manifest.pgid,
            &manifest.boot_identity,
            &manifest.start_identity,
        ) {
            (Some(pid), Some(group_id), Some(boot), Some(start)) => {
                let expected = unleash_platform::OsProcessIdentity {
                    pid,
                    pgid: group_id,
                    owner_uid: manifest.owner_uid,
                    boot_identity: boot.clone(),
                    start_identity: start.clone(),
                    backend: manifest.identity_backend,
                };
                match inspector.verify_identity(&expected)? {
                    ProcessVerification::Match => (
                        HostLaunchState::Running,
                        format!("observed_alive_without_host_anchor(pid={pid})"),
                    ),
                    ProcessVerification::Exited => (
                        HostLaunchState::Exited {
                            exit_code: None,
                            exit_signal: None,
                        },
                        format!("observed_exited(pid={pid})"),
                    ),
                    ProcessVerification::IdentityMismatch { observed } => (
                        HostLaunchState::Lost,
                        format!(
                            "identity_mismatch(pid={pid}, expected={}, observed={})",
                            start, observed.start_identity
                        ),
                    ),
                    ProcessVerification::BootChanged => {
                        (HostLaunchState::Lost, String::from("boot_changed -> lost"))
                    }
                    _ => (
                        HostLaunchState::LaunchOutcomeUnknown,
                        String::from("unrecognized_os_verification"),
                    ),
                }
            }
            _ => (
                HostLaunchState::LaunchOutcomeUnknown,
                String::from("incomplete_manifest_identity"),
            ),
        },
    };

    let signal_refusal = OwnedChildAnchor::unanchored_signal_error(
        None,
        "process-host disparu : maître PTY perdu et aucun signal sans ancrage parent vivant",
    );

    Ok(LostHostReconciliation {
        reconciled_state,
        os_verification,
        durable_spool,
        last_confirmed_input_seq,
        unknown_input_seqs: combined_unknowns,
        signal_refusal_reason: signal_refusal.to_string(),
    })
}

/// Client synchrone du daemon pour dialoguer avec un `unleash-process-host` via son socket `0600`.
pub struct HostClient {
    stream: UnixStream,
    process_run_uuid: ProcessRunUuid,
    authority_generation: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    fence: Fence,
}

impl std::fmt::Debug for HostClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostClient")
            .field("process_run_uuid", &self.process_run_uuid)
            .field("authority_generation", &self.authority_generation)
            .field("daemon_epoch", &self.daemon_epoch)
            .field("fence", &self.fence)
            .finish_non_exhaustive()
    }
}

impl HostClient {
    /// Retire one exact terminal control. No legacy handshake fallback exists.
    /// # Errors
    /// Refuses client-coordinate mismatches, transport errors and missing ACKs.
    pub fn release_control(
        &mut self,
        request: &crate::ControlReleaseRequest,
    ) -> Result<crate::ControlReleaseResponse, ProcessHostError> {
        self.check_control_coordinates(&request.key.header)?;
        let response = self.control_release_call(
            &crate::control_release::ReleaseEnvelope::ReleaseControlV1(request.clone()),
        )?;
        Ok(serde_json::from_slice(&response)?)
    }

    /// Activate the same physical resource from an exact qualified retirement.
    /// # Errors
    /// Refuses coordinate mismatches, absent source facts and ambiguous responses.
    pub fn reacquire_released_control(
        &mut self,
        request: &crate::ControlReacquisitionRequest,
    ) -> Result<crate::ControlReacquisitionResponse, ProcessHostError> {
        self.check_control_coordinates(&request.transfer.header)?;
        let response = self.control_release_call(
            &crate::control_release::ReleaseEnvelope::AcquireReleasedControlV1(request.clone()),
        )?;
        Ok(serde_json::from_slice(&response)?)
    }

    fn check_control_coordinates(
        &self,
        header: &HostCommandHeader,
    ) -> Result<(), ProcessHostError> {
        if header.process_run_uuid != self.process_run_uuid
            || header.authority_generation != self.authority_generation
            || header.daemon_epoch != self.daemon_epoch
            || header.resource_fence != self.fence
        {
            return Err(ProcessHostError::InvalidArgument {
                reason: "native control request differs from client coordinates".into(),
            });
        }
        Ok(())
    }

    fn control_release_call(
        &mut self,
        request: &crate::control_release::ReleaseEnvelope,
    ) -> Result<Vec<u8>, ProcessHostError> {
        let mut bytes = serde_json::to_vec(request)?;
        let result = write_frame(&mut self.stream, &bytes);
        bytes.zeroize();
        result?;
        read_frame(&mut self.stream)?.ok_or_else(|| ProcessHostError::LaunchOutcomeUnknown {
            reason: "native control disposition response was lost".into(),
        })
    }

    /// Send a correlated control transfer to an existing host. No legacy fallback exists.
    /// # Errors
    /// Refuses mismatched client coordinates, transport failure or an unsupported response.
    pub fn transfer_control(
        &mut self,
        request: &crate::ControlTransferRequest,
    ) -> Result<crate::ControlTransferResponse, ProcessHostError> {
        if request.header.process_run_uuid != self.process_run_uuid
            || request.header.authority_generation != self.authority_generation
            || request.header.daemon_epoch != self.daemon_epoch
            || request.header.resource_fence != self.fence
        {
            return Err(ProcessHostError::InvalidArgument {
                reason: "control transfer differs from client coordinates".into(),
            });
        }
        let bytes = serde_json::to_vec(
            &crate::control_transfer::TransferEnvelope::TransferControlV1(request.clone()),
        )?;
        write_frame(&mut self.stream, &bytes)?;
        let response =
            read_frame(&mut self.stream)?.ok_or_else(|| ProcessHostError::TerminalUnavailable {
                reason: "control transfer connection closed".into(),
            })?;
        Ok(serde_json::from_slice(&response)?)
    }

    /// Ouvre une connexion Unix privée vers `socket_path` (vérifiant `0700`/`0600` et UID pair).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] si le socket est invalide ou inaccessible.
    pub fn connect(
        socket_path: &Path,
        process_run_uuid: ProcessRunUuid,
        authority_generation: AuthorityGeneration,
        daemon_epoch: DaemonEpoch,
        fence: Fence,
    ) -> Result<Self, ProcessHostError> {
        unleash_platform::validate_socket_file(socket_path)?;
        let stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(Some(SOCKET_IO_TIMEOUT))?;
        stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT))?;
        verify_same_uid_peer(&stream)?;
        Ok(Self {
            stream,
            process_run_uuid,
            authority_generation,
            daemon_epoch,
            fence,
        })
    }

    /// Confirm that the connected Unix peer is the supplied owned host process.
    ///
    /// # Errors
    /// Returns a closed identity error when the peer cannot be qualified.
    pub fn verify_owned_peer(
        &self,
        expected: &unleash_platform::OsProcessIdentity,
    ) -> Result<(), ProcessHostError> {
        let refused = || ProcessHostError::IdentityUnverified {
            reason: "connected Unix peer is not the owned host".into(),
        };
        let peer = verify_same_uid_peer(&self.stream)?;
        if peer.pid != Some(expected.pid)
            || peer.uid != expected.owner_uid
            || unleash_platform::inspect_process_identity(expected.pid).map_err(|_| refused())?
                != *expected
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Met à jour les coordonnées de lignée/époque/fence envoyées par ce client (utile pour tester le rejet).
    pub fn set_lineage(
        &mut self,
        authority_generation: AuthorityGeneration,
        daemon_epoch: DaemonEpoch,
        fence: Fence,
    ) {
        self.authority_generation = authority_generation;
        self.daemon_epoch = daemon_epoch;
        self.fence = fence;
    }

    /// Envoie une requête brute et décode la réponse du host.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError`] en cas d'erreur de transport ou de codec.
    pub fn call(&mut self, req: &HostRequest) -> Result<HostResponse, ProcessHostError> {
        let bytes = serde_json::to_vec(req)?;
        write_frame(&mut self.stream, &bytes)?;
        let raw_resp =
            read_frame(&mut self.stream)?.ok_or_else(|| ProcessHostError::TerminalUnavailable {
                reason: String::from("connexion fermée par unleash-process-host"),
            })?;
        let resp: HostResponse = serde_json::from_slice(&raw_resp)?;
        Ok(resp)
    }

    /// Construit un [`HostCommandHeader`] valide pour les coordonnées actuelles du client.
    #[must_use]
    pub fn make_header(
        &self,
        command_id: unleash_domain::CommandId,
        action_id: unleash_domain::ActionId,
        payload_hash: unleash_domain::Sha256Digest,
    ) -> HostCommandHeader {
        HostCommandHeader {
            protocol_version: FencingGate::PROTOCOL_VERSION,
            process_run_uuid: self.process_run_uuid,
            authority_generation: self.authority_generation,
            daemon_epoch: self.daemon_epoch,
            resource_fence: self.fence,
            command_id,
            action_id,
            payload_hash,
        }
    }
}

/// Observateur de points de coupure piloté par variable d'environnement pour tests de crash réels
/// du binaire `unleash-process-host` (sans aucun `sleep`, synchronisation par stdout + stdin pipe).
#[derive(Debug)]
pub struct StdioBarrierCutpointObserver {
    target: HostInputCutpoint,
    label: &'static str,
}

impl StdioBarrierCutpointObserver {
    /// Selects C3/C4/C5, `HandshakeAppliedBeforeAck` or `ResizeAttemptedBeforeReply`
    /// for an exclusively owned native crash fixture. Normal launch clears this variable.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let val = std::env::var("UNLEASH_HOST_CUTPOINT_TARGET").ok()?;
        match val.trim() {
            "C3" => Some(Self {
                target: HostInputCutpoint::C3BeforeWriteDispatched,
                label: "C3",
            }),
            "C4" => Some(Self {
                target: HostInputCutpoint::C4AfterDispatchedBeforePtyWrite,
                label: "C4",
            }),
            "C5" => Some(Self {
                target: HostInputCutpoint::C5AfterPtyWriteBeforeReceipt,
                label: "C5",
            }),
            "HandshakeAppliedBeforeAck" => Some(Self {
                target: HostInputCutpoint::HandshakeAppliedBeforeAck,
                label: "HandshakeAppliedBeforeAck",
            }),
            "ResizeAttemptedBeforeReply" => Some(Self {
                target: HostInputCutpoint::ResizeAttemptedBeforeReply,
                label: "ResizeAttemptedBeforeReply",
            }),
            "ReleaseAppliedBeforeAck" => Some(Self {
                target: HostInputCutpoint::ReleaseAppliedBeforeAck,
                label: "ReleaseAppliedBeforeAck",
            }),
            _ => None,
        }
    }
}

impl HostCutpointObserver for StdioBarrierCutpointObserver {
    fn hit(&self, cutpoint: HostInputCutpoint) -> Result<(), ProcessHostError> {
        if cutpoint == self.target {
            let line = format!("UNLEASH_CRASH_REACHED:{}\n", self.label);
            let mut out = std::io::stdout();
            out.write_all(line.as_bytes())?;
            out.flush()?;
            let mut buf = [0_u8; 1];
            std::io::stdin().read_exact(&mut buf)?;
        }
        Ok(())
    }
}

const fn current_identity_backend() -> unleash_platform::IdentityBackend {
    #[cfg(target_os = "macos")]
    {
        unleash_platform::IdentityBackend::MacosProcPidinfo
    }
    #[cfg(not(target_os = "macos"))]
    {
        unleash_platform::IdentityBackend::LinuxProcfs
    }
}
