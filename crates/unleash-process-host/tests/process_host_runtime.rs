// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration réels Spike 3 Runtime (`T-F011-002..010`, `T-SEC-03/04`, `Q-05`).
//!
//! Zéro `sleep` de synchronisation : toutes les coordinations entre test, daemon simulé,
//! `unleash-process-host` et processus enfants/petits-enfants utilisent exclusivement des tubes,
//! sockets Unix de test, FIFOs et marqueurs `stdout`.

#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, AuthorityGeneration, CommandId, DaemonEpoch, Fence, HostId, MissionId, ProcessId,
    ProcessRunUuid, Sha256Digest, TerminalId, TimestampMs,
};
use unleash_platform::{
    BootIdentity, OsProcessIdentity, ProcessVerification, ensure_private_rendezvous_dir,
    inspect_process_identity, write_private_file_atomic,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, HOST_MANIFEST_FILENAME, HOST_READY_PREFIX, HostBootstrapSpec,
    HostClient, HostCommandHeader, HostControlSecret, HostLaunchState, HostManifest, HostRequest,
    HostResponse, IsolationBackendProbe, IsolationMode, IsolationProfileKind, LogSpool,
    OsIsolationBackendProbe, OsProcessInspector, ProcessHostError, ProcessHostServer,
    ProcessInspector, ResourceKind, SandboxProbeOutcome, StrictIsolationSpec,
    reconcile_after_lost_host, store_canary_secrets, store_control_secret,
};
use unleash_protocol::{ErrorCode, ReasonCode, RetryClass};
use unleash_testkit::{ExternalEffectOracle, local_test_dir};

const ENV_ROLE: &str = "UNLEASH_SPIKE_RUNTIME_ROLE";
const ENV_SYNC_SOCK: &str = "UNLEASH_SPIKE_SYNC_SOCK";
const ENV_ORACLE_DIR: &str = "UNLEASH_SPIKE_ORACLE_DIR";
const ENV_DAEMON_CFG: &str = "UNLEASH_SPIKE_DAEMON_CFG";
const ENV_PROBE_CFG: &str = "UNLEASH_SPIKE_PROBE_CFG";

const IO_TIMEOUT: Duration = Duration::from_secs(15);

fn host_bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_unleash-process-host"))
}

fn current_exe_fixture_args(test_fn: &str) -> Vec<String> {
    vec![
        test_fn.to_owned(),
        String::from("--exact"),
        String::from("--nocapture"),
    ]
}

fn mk_header(
    run_uuid: ProcessRunUuid,
    gen_id: AuthorityGeneration,
    epoch: u64,
    fence: u64,
    cmd_seed: u128,
) -> HostCommandHeader {
    HostCommandHeader {
        protocol_version: 1,
        process_run_uuid: run_uuid,
        authority_generation: gen_id,
        daemon_epoch: DaemonEpoch::new(epoch),
        resource_fence: Fence::new(fence),
        command_id: CommandId::from_u128(cmd_seed),
        action_id: ActionId::from_u128(cmd_seed + 1000),
        payload_hash: Sha256Digest::from_bytes(&cmd_seed.to_le_bytes()),
    }
}

fn write_bootstrap(path: &Path, spec: &HostBootstrapSpec) {
    let bytes = serde_json::to_vec_pretty(spec).unwrap();
    write_private_file_atomic(path, &bytes).unwrap();
}

fn spawn_real_host_binary(
    bootstrap_path: &Path,
    extra_env: &[(&str, &str)],
    with_stdin_pipe: bool,
) -> (Child, String, PathBuf, mpsc::Receiver<String>) {
    let mut cmd = Command::new(host_bin_path());
    cmd.arg("--bootstrap")
        .arg(bootstrap_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if with_stdin_pipe {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let (line_tx, line_rx) = mpsc::channel::<String>();
    let line_tx_err = line_tx.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line) {
            if n == 0 {
                break;
            }
            let _ = line_tx.send(line.trim_end_matches(['\r', '\n']).to_owned());
            line.clear();
        }
    });
    std::thread::spawn(move || {
        let mut s = String::new();
        let mut r = BufReader::new(stderr);
        let _ = r.read_to_string(&mut s);
        if !s.is_empty() {
            let _ = line_tx_err.send(format!("STDERR:{s}"));
        }
    });

    let first = line_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("timeout en attendant UNLEASH_HOST_READY");
    let rest = first
        .strip_prefix(HOST_READY_PREFIX)
        .unwrap_or_else(|| panic!("ligne ready inattendue : {first}"));
    let (uuid_str, sock_str) = rest.split_once(':').unwrap();
    (child, uuid_str.to_owned(), PathBuf::from(sock_str), line_rx)
}

fn accept_sync_client(listener: &UnixListener) -> (UnixStream, BufReader<UnixStream>) {
    let (stream, _) = listener.accept().unwrap();
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let reader = BufReader::new(stream.try_clone().unwrap());
    (stream, reader)
}

fn read_line_timeout(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    let n = reader.read_line(&mut line).unwrap();
    assert!(n > 0, "EOF inattendu sur sync_sock");
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn read_all_spool_text(client: &mut HostClient, hdr: HostCommandHeader, secret: &str) -> String {
    let resp = client
        .call(&HostRequest::ReadLog {
            header: hdr,
            secret: secret.to_owned(),
            after_offset: 0,
            max_bytes: 1024 * 1024,
            strict_cursor: false,
        })
        .unwrap();
    let HostResponse::LogChunk { result } = resp else {
        panic!("réponse ReadLog inattendue : {resp:?}");
    };
    let mut combined = Vec::new();
    for seg in &result.segments {
        combined.extend_from_slice(&seg.bytes);
    }
    String::from_utf8_lossy(&combined).into_owned()
}

#[derive(Debug, Serialize, Deserialize)]
struct DaemonFixtureConfig {
    bootstrap_path: PathBuf,
    runtime_dir: PathBuf,
    secret_1: String,
    process_run_uuid: ProcessRunUuid,
    authority_generation: AuthorityGeneration,
    sync_sock: PathBuf,
}

/// Fixture processus daemon simulé pour `T-F011-003`.
#[test]
fn fixture_simulated_daemon_entrypoint() {
    if std::env::var(ENV_ROLE).as_deref() != Ok("simulated_daemon") {
        return;
    }
    let cfg_path = PathBuf::from(std::env::var_os(ENV_DAEMON_CFG).unwrap());
    let cfg: DaemonFixtureConfig = serde_json::from_slice(&fs::read(&cfg_path).unwrap()).unwrap();

    // Lance le vrai binaire unleash-process-host en session indépendante
    let (mut host_child, _uuid, sock_path, _rx) =
        spawn_real_host_binary(&cfg.bootstrap_path, &[], false);

    // Connecte et fait le handshake epoch 1
    let mut client = HostClient::connect(
        &sock_path,
        cfg.process_run_uuid,
        cfg.authority_generation,
        DaemonEpoch::new(1),
        Fence::new(1),
    )
    .unwrap();
    let ack = client
        .call(&HostRequest::Handshake {
            header: mk_header(cfg.process_run_uuid, cfg.authority_generation, 1, 1, 1),
            current_secret: cfg.secret_1.clone(),
            rotate_to_secret: None,
            require_higher_epoch: false,
        })
        .unwrap();
    assert!(matches!(ack, HostResponse::HandshakeAck { .. }));

    // Annonce au test parent l'identifiant du host et bloque sur stdin jusqu'au SIGKILL
    let host_pid = host_child.id();
    let ready = format!("DAEMON_READY:{host_pid}:{}\n", sock_path.display());
    std::io::stdout().write_all(ready.as_bytes()).unwrap();
    std::io::stdout().flush().unwrap();

    let mut hold = [0_u8; 1];
    let _ = std::io::stdin().read(&mut hold);
    let _ = host_child.wait();
}

/// Fixture worker PTY interactif avec oracle optionnel pour `T-F011-003`, `T-F011-005` et `T-F011-008`.
#[test]
fn fixture_pty_worker_entrypoint() {
    let Ok(role) = std::env::var(ENV_ROLE) else {
        return;
    };
    if role != "pty_worker" {
        return;
    }

    let sync_sock = PathBuf::from(std::env::var_os(ENV_SYNC_SOCK).unwrap());
    let mut conn = UnixStream::connect(&sync_sock).unwrap();
    conn.set_read_timeout(Some(IO_TIMEOUT)).unwrap();

    // Émet immédiatement des octets pré-kill sur stdout PTY
    println!("PTY_PRE_KILL_DATA");
    std::io::stdout().flush().unwrap();

    // Signale au test que le worker PTY est actif
    conn.write_all(b"PTY_READY\n").unwrap();
    conn.flush().unwrap();

    // Thread lecteur de contrôle (test -> PTY worker) pour émettre post-kill ou quitter
    let mut ctrl_conn = conn.try_clone().unwrap();
    let mut ctrl_reader = BufReader::new(conn.try_clone().unwrap());
    let ctrl_handle = std::thread::spawn(move || {
        let mut line = String::new();
        while let Ok(n) = ctrl_reader.read_line(&mut line) {
            if n == 0 {
                break;
            }
            let cmd = line.trim();
            if cmd == "EMIT_POST_KILL" {
                println!("PTY_POST_KILL_DATA");
                let _ = std::io::stdout().flush();
                let _ = ctrl_conn.write_all(b"POST_KILL_EMITTED\n");
                let _ = ctrl_conn.flush();
            } else if cmd == "EMIT_FLOOD_AND_CANARIES" {
                // Canaris coupés en morceaux + multi-lignes + séquences ANSI OSC
                print!("part1=CANARY_");
                let _ = std::io::stdout().flush();
                print!("SECRET_ALPHA ");
                let _ = std::io::stdout().flush();
                println!("multi=CANARY_MULTI");
                let _ = std::io::stdout().flush();
                println!("LINE_BETA end \x1b]8;;https://evil.example\x07click\x1b]8;;\x07");
                for i in 0..40 {
                    println!("flood-line-{i:03}-0123456789abcdef0123456789abcdef");
                }
                let _ = std::io::stdout().flush();
                let _ = ctrl_conn.write_all(b"FLOOD_DONE\n");
                let _ = ctrl_conn.flush();
            } else if cmd == "EXIT" {
                std::process::exit(0);
            }
            line.clear();
        }
    });

    // Boucle principale lisant les entrées PTY depuis stdin (SendTerminalInput)
    let oracle_dir = std::env::var_os(ENV_ORACLE_DIR).map(PathBuf::from);
    let mut stdin_reader = BufReader::new(std::io::stdin());
    let mut in_line = String::new();
    while let Ok(n) = stdin_reader.read_line(&mut in_line) {
        if n == 0 {
            break;
        }
        let trimmed = in_line.trim();
        if trimmed == "APPLY_ORACLE_1" {
            if let Some(ref dir) = oracle_dir {
                let oracle = ExternalEffectOracle::open(dir).unwrap();
                oracle
                    .record_effect("pty-input-seq-1", "terminal_input", 1_000)
                    .unwrap();
                conn.write_all(b"EFFECT_COMMITTED\n").unwrap();
                conn.flush().unwrap();
            }
            println!("ORACLE_APPLIED");
            let _ = std::io::stdout().flush();
        } else if !trimmed.is_empty() {
            println!("ECHO:{trimmed}");
            let _ = std::io::stdout().flush();
            let _ = writeln!(conn, "ECHOED:{trimmed}");
            let _ = conn.flush();
        }
        in_line.clear();
    }

    let _ = ctrl_handle.join();
}

/// Fixture arbre enfant + petit-enfant même PGID + petit-enfant détaché (`setsid`) pour `T-F011-006`.
#[test]
fn fixture_descendant_tree_entrypoint() {
    let role = std::env::var(ENV_ROLE).unwrap_or_default();
    if role == "descendant_tree" {
        let sync_sock = PathBuf::from(std::env::var_os(ENV_SYNC_SOCK).unwrap());
        let exe = std::env::current_exe().unwrap();

        // 1. Petit-enfant ordinaire (même PGID)
        let mut same = Command::new(&exe);
        same.args(current_exe_fixture_args(
            "fixture_descendant_tree_entrypoint",
        ))
        .env(ENV_ROLE, "descendant_leaf_same")
        .env(ENV_SYNC_SOCK, &sync_sock)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let mut same_child = same.spawn().unwrap();

        // 2. Petit-enfant détaché (appelle setsid())
        let mut detached = Command::new(&exe);
        detached
            .args(current_exe_fixture_args(
                "fixture_descendant_tree_entrypoint",
            ))
            .env(ENV_ROLE, "descendant_leaf_detached")
            .env(ENV_SYNC_SOCK, &sync_sock)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unleash_process_host::configure_setsid_pre_exec(&mut detached);
        let mut det_child = detached.spawn().unwrap();

        let mut conn = UnixStream::connect(&sync_sock).unwrap();
        writeln!(conn, "LEADER_READY:{}", std::process::id()).unwrap();
        conn.flush().unwrap();

        // Bloque en attente de SIGTERM/SIGKILL
        let mut buf = [0_u8; 1];
        let _ = std::io::stdin().read(&mut buf);
        let _ = same_child.wait();
        let _ = det_child.wait();
    } else if role == "descendant_leaf_same" || role == "descendant_leaf_detached" {
        let sync_sock = PathBuf::from(std::env::var_os(ENV_SYNC_SOCK).unwrap());
        let mut conn = UnixStream::connect(&sync_sock).unwrap();
        let tag = if role == "descendant_leaf_same" {
            "SAME"
        } else {
            "DETACHED"
        };
        writeln!(conn, "{tag}:{}", std::process::id()).unwrap();
        conn.flush().unwrap();
        let mut buf = [0_u8; 1];
        let _ = conn.read(&mut buf);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StrictProbeConfig {
    allowed_work_dir: PathBuf,
    outside_forbidden_path: PathBuf,
    data_secret_path: PathBuf,
    socket_secret_path: PathBuf,
    cred_secret_path: PathBuf,
    keychain_probe_path: PathBuf,
    allowed_tcp_port: u16,
    blocked_tcp_port: u16,
    done_fifo_path: PathBuf,
    report_path: PathBuf,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize, Deserialize)]
struct StrictProbeReport {
    write_inside_ok: bool,
    write_outside_denied: bool,
    read_data_root_denied: bool,
    read_socket_dir_denied: bool,
    read_cred_denied: bool,
    read_keychain_denied: bool,
    allowed_tcp_ok: bool,
    blocked_tcp_denied: bool,
    inherited_secret_env_absent: bool,
}

/// Fixture enfant exécutée SOUS Seatbelt `/usr/bin/sandbox-exec` pour `T-SEC-03/04`.
#[test]
fn fixture_strict_sandbox_probe_entrypoint() {
    if std::env::var(ENV_ROLE).as_deref() != Ok("strict_sandbox_probe") {
        return;
    }
    let cfg_path = PathBuf::from(std::env::var_os(ENV_PROBE_CFG).unwrap());
    let cfg: StrictProbeConfig = serde_json::from_slice(&fs::read(&cfg_path).unwrap()).unwrap();

    let write_inside_ok = fs::write(cfg.allowed_work_dir.join("probe_ok.txt"), b"ok").is_ok();
    let write_outside_denied = fs::write(&cfg.outside_forbidden_path, b"escape").is_err();
    let read_data_root_denied = fs::read(&cfg.data_secret_path).is_err();
    let read_socket_dir_denied = fs::read(&cfg.socket_secret_path).is_err();
    let read_cred_denied = fs::read(&cfg.cred_secret_path).is_err();
    let read_keychain_denied = fs::read_dir(&cfg.keychain_probe_path).is_err();

    let allowed_tcp_ok = std::net::TcpStream::connect(("127.0.0.1", cfg.allowed_tcp_port)).is_ok();
    let blocked_tcp_denied =
        std::net::TcpStream::connect(("127.0.0.1", cfg.blocked_tcp_port)).is_err();
    let inherited_secret_env_absent = std::env::var_os("UNLEASH_FAKE_API_KEY").is_none();

    let report = StrictProbeReport {
        write_inside_ok,
        write_outside_denied,
        read_data_root_denied,
        read_socket_dir_denied,
        read_cred_denied,
        read_keychain_denied,
        allowed_tcp_ok,
        blocked_tcp_denied,
        inherited_secret_env_absent,
    };
    fs::write(
        &cfg.report_path,
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();

    // Signale la fin au parent via FIFO dans allowed_work_dir (sans sleep)
    let mut fifo = OpenOptions::new()
        .write(true)
        .open(&cfg.done_fifo_path)
        .unwrap();
    fifo.write_all(b"1").unwrap();
    fifo.flush().unwrap();
}

// ================================================================================================
// TESTS D'INTÉGRATION PRINCIPAUX (SPIKE 3 RUNTIME)
// ================================================================================================

// T-F011-003 / AC-F011-003 / REQ-TERM-003
#[test]
fn t_f011_003_sigkill_daemon_during_active_pty_survives_and_reattaches_same_lineage_only() {
    let dir = local_test_dir("sp-dmn").unwrap();
    let runtime_dir = dir.path().join("host");
    let work_dir = dir.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    ensure_private_rendezvous_dir(&runtime_dir).unwrap();

    let sync_sock_path = dir.path().join("sync.sock");
    let sync_listener = UnixListener::bind(&sync_sock_path).unwrap();

    let secret_1 = "ctrl-secret-epoch-1-xyz";
    let secret_path = runtime_dir.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &HostControlSecret::new(secret_1).unwrap()).unwrap();

    let run_uuid = ProcessRunUuid::from_u128(0x1111);
    let gen_a = AuthorityGeneration::from_u128(0xAAAA);

    let mut explicit_env = BTreeMap::new();
    explicit_env.insert(ENV_ROLE.to_owned(), "pty_worker".to_owned());
    explicit_env.insert(
        ENV_SYNC_SOCK.to_owned(),
        sync_sock_path.display().to_string(),
    );

    let spec = HostBootstrapSpec {
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: Some(TerminalId::from_u128(3)),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(4),
        mission_id: Some(MissionId::from_u128(5)),
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir,
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_pty_worker_entrypoint"),
        explicit_env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: secret_path,
        canaries_secret_path: None,
        max_segment_bytes: 256,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 256,
    };
    let bootstrap_path = runtime_dir.join("bootstrap.json");
    write_bootstrap(&bootstrap_path, &spec);

    let dmn_cfg = DaemonFixtureConfig {
        bootstrap_path,
        runtime_dir: runtime_dir.clone(),
        secret_1: secret_1.to_owned(),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        sync_sock: sync_sock_path.clone(),
    };
    let dmn_cfg_path = dir.path().join("dmn.json");
    fs::write(&dmn_cfg_path, serde_json::to_vec(&dmn_cfg).unwrap()).unwrap();

    // 1. Lancer le processus daemon simulé
    let mut daemon = Command::new(std::env::current_exe().unwrap())
        .args(current_exe_fixture_args(
            "fixture_simulated_daemon_entrypoint",
        ))
        .env(ENV_ROLE, "simulated_daemon")
        .env(ENV_DAEMON_CFG, &dmn_cfg_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let daemon_stdout = daemon.stdout.take().unwrap();
    let mut daemon_reader = BufReader::new(daemon_stdout);
    let rest = loop {
        let mut ready_line = String::new();
        let n = daemon_reader.read_line(&mut ready_line).unwrap();
        assert!(n > 0, "EOF avant DAEMON_READY sur stdout du daemon simulé");
        if let Some(found) = ready_line.trim().strip_prefix("DAEMON_READY:") {
            break found.to_owned();
        }
    };
    let (host_pid_str, host_sock_str) = rest.split_once(':').unwrap();
    let host_pid: u32 = host_pid_str.parse().unwrap();
    let host_sock = PathBuf::from(host_sock_str);

    // Attendre que le worker PTY ait émis PTY_PRE_KILL_DATA
    let (mut pty_sock, mut pty_reader) = accept_sync_client(&sync_listener);
    assert_eq!(read_line_timeout(&mut pty_reader), "PTY_READY");

    // Vérifier qu'aucun secret ne figure dans argv de unleash-process-host
    let ps_out = Command::new("ps")
        .args(["-p", &host_pid.to_string(), "-o", "args="])
        .output()
        .unwrap();
    let cmdline = String::from_utf8_lossy(&ps_out.stdout);
    assert!(
        !cmdline.contains(secret_1),
        "secret exposé dans argv : {cmdline}"
    );

    // 2. SIGKILL du daemon pendant que le PTY est actif !
    daemon.kill().unwrap();
    let daemon_status = daemon.wait().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(daemon_status.signal(), Some(libc::SIGKILL));
    }

    // Le process-host doit toujours être vivant malgré la mort SIGKILL du daemon
    let host_os_id = inspect_process_identity(host_pid).unwrap();
    assert_eq!(host_os_id.pid, host_pid);

    // 3. Le worker PTY continue à produire de la sortie pendant l'absence du daemon
    pty_sock.write_all(b"EMIT_POST_KILL\n").unwrap();
    pty_sock.flush().unwrap();
    assert_eq!(read_line_timeout(&mut pty_reader), "POST_KILL_EMITTED");

    // 4. Reprise par nouveau daemon : même lignée, daemon_epoch=2, rotation de secret
    let secret_2 = "ctrl-secret-epoch-2-rotated";
    let mut pre_rotation_channel = HostClient::connect(
        &host_sock,
        run_uuid,
        gen_a,
        DaemonEpoch::new(1),
        Fence::new(1),
    )
    .unwrap();
    let mut client = HostClient::connect(
        &host_sock,
        run_uuid,
        gen_a,
        DaemonEpoch::new(2),
        Fence::new(1),
    )
    .unwrap();
    let ack = client
        .call(&HostRequest::Handshake {
            header: mk_header(run_uuid, gen_a, 2, 1, 10),
            current_secret: secret_1.to_owned(),
            rotate_to_secret: Some(secret_2.to_owned()),
            require_higher_epoch: true,
        })
        .unwrap();
    assert!(matches!(
        ack,
        HostResponse::HandshakeAck {
            secret_rotated: true,
            ..
        }
    ));

    // Tout ancien canal connecté avant rotation est invalidé immédiatement
    let stale_chan_resp = pre_rotation_channel
        .call(&HostRequest::ReadLog {
            header: mk_header(run_uuid, gen_a, 2, 1, 9),
            secret: secret_2.to_owned(),
            after_offset: 0,
            max_bytes: 1024,
            strict_cursor: false,
        })
        .unwrap();
    assert!(matches!(
        stale_chan_resp,
        HostResponse::Error { error } if error.code == ErrorCode::AuthenticationFailed
    ));

    // L'ancien secret est refusé (AuthenticationFailed)
    let old_secret_resp = client
        .call(&HostRequest::ReadLog {
            header: mk_header(run_uuid, gen_a, 2, 2, 11),
            secret: secret_1.to_owned(),
            after_offset: 0,
            max_bytes: 1024,
            strict_cursor: false,
        })
        .unwrap();
    assert!(matches!(
        old_secret_resp,
        HostResponse::Error { error } if error.code == ErrorCode::AuthenticationFailed
    ));

    // Une autre authority_generation (restauration) est strictement refusée même avec epoch=9999
    let gen_foreign = AuthorityGeneration::from_u128(0xBBBB);
    let foreign_resp = client
        .call(&HostRequest::Handshake {
            header: mk_header(run_uuid, gen_foreign, 9999, 1, 12),
            current_secret: secret_2.to_owned(),
            rotate_to_secret: None,
            require_higher_epoch: true,
        })
        .unwrap();
    assert!(matches!(
        foreign_resp,
        HostResponse::Error { error }
            if error.code == ErrorCode::StaleFence
                && error.reason_code == Some(ReasonCode::StaleAuthorityGeneration)
                && error.retry_class == RetryClass::Never
    ));

    // Un ancien epoch dans la même lignée est refusé (StaleEpoch)
    let stale_epoch_resp = client
        .call(&HostRequest::ReadLog {
            header: mk_header(run_uuid, gen_a, 1, 99, 13),
            secret: secret_2.to_owned(),
            after_offset: 0,
            max_bytes: 1024,
            strict_cursor: false,
        })
        .unwrap();
    assert!(matches!(
        stale_epoch_resp,
        HostResponse::Error { error }
            if error.code == ErrorCode::StaleFence && error.reason_code == Some(ReasonCode::StaleEpoch)
    ));

    // Entrée PTY + Resize + ReadLog confirment conservation pré- et post-crash
    let input_resp = client
        .call(&HostRequest::SendInput {
            header: mk_header(run_uuid, gen_a, 2, 2, 14),
            secret: secret_2.to_owned(),
            input_seq: 1,
            payload_bytes: b"PING_AFTER_REATTACH\n".to_vec(),
            now_ms: TimestampMs::new(5000),
        })
        .unwrap();
    assert!(matches!(input_resp, HostResponse::InputReceipt { .. }));
    assert_eq!(
        read_line_timeout(&mut pty_reader),
        "ECHOED:PING_AFTER_REATTACH"
    );

    let resize_resp = client
        .call(&HostRequest::Resize {
            header: mk_header(run_uuid, gen_a, 2, 3, 15),
            secret: secret_2.to_owned(),
            rows: 40,
            cols: 120,
        })
        .unwrap();
    assert!(matches!(
        resize_resp,
        HostResponse::ResizeAck {
            rows: 40,
            cols: 120
        }
    ));

    // Arrêter le worker PTY vivant via StopProcess (ancrage direct + groupe) puis Shutdown
    let stop_resp = client
        .call(&HostRequest::Stop {
            header: mk_header(run_uuid, gen_a, 2, 4, 16),
            secret: secret_2.to_owned(),
            escalate_to_sigkill: true,
            known_descendant_pids: vec![],
        })
        .unwrap();
    let _ = pty_sock.write_all(b"EXIT\n");
    assert!(
        matches!(stop_resp, HostResponse::StopObserved { .. }),
        "stop_resp inattendu : {stop_resp:?}"
    );

    let logs = read_all_spool_text(&mut client, mk_header(run_uuid, gen_a, 2, 5, 17), secret_2);
    assert!(logs.contains("PTY_PRE_KILL_DATA"), "logs={logs}");
    assert!(logs.contains("PTY_POST_KILL_DATA"), "logs={logs}");
    assert!(logs.contains("ECHO:PING_AFTER_REATTACH"), "logs={logs}");

    let sd_resp = client
        .call(&HostRequest::Shutdown {
            header: mk_header(run_uuid, gen_a, 2, 6, 18),
            secret: secret_2.to_owned(),
        })
        .unwrap();
    assert!(matches!(sd_resp, HostResponse::ShutdownAck));
    LogSpool::verify_durable_segments(runtime_dir.join("spool")).unwrap();
}

// T-F011-005 / AC-F011-005 / REQ-TERM-005
#[test]
fn t_f011_005_sigkill_host_after_pty_write_before_receipt_outcome_unknown_zero_repeat() {
    let dir = local_test_dir("sp-c5").unwrap();
    let runtime_dir = dir.path().join("host");
    let work_dir = dir.path().join("work");
    let oracle_dir = dir.path().join("oracle");
    fs::create_dir_all(&work_dir).unwrap();
    ensure_private_rendezvous_dir(&runtime_dir).unwrap();
    let oracle = ExternalEffectOracle::open(&oracle_dir).unwrap();

    let sync_sock_path = dir.path().join("sync.sock");
    let sync_listener = UnixListener::bind(&sync_sock_path).unwrap();

    let secret = "ctrl-secret-c5";
    let secret_path = runtime_dir.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &HostControlSecret::new(secret).unwrap()).unwrap();

    let run_uuid = ProcessRunUuid::from_u128(0x2222);
    let gen_a = AuthorityGeneration::from_u128(0xAAAA);

    let mut explicit_env = BTreeMap::new();
    explicit_env.insert(ENV_ROLE.to_owned(), "pty_worker".to_owned());
    explicit_env.insert(
        ENV_SYNC_SOCK.to_owned(),
        sync_sock_path.display().to_string(),
    );
    explicit_env.insert(ENV_ORACLE_DIR.to_owned(), oracle_dir.display().to_string());

    let spec = HostBootstrapSpec {
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(10),
        process_id: ProcessId::from_u128(20),
        terminal_id: Some(TerminalId::from_u128(30)),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(40),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir,
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_pty_worker_entrypoint"),
        explicit_env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: secret_path,
        canaries_secret_path: None,
        max_segment_bytes: 256,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 256,
    };
    let bootstrap_path = runtime_dir.join("bootstrap.json");
    write_bootstrap(&bootstrap_path, &spec);

    // Armer C5 dans le binaire host réel (bloque après write PTY avant reçu durable)
    let (mut host_child, _uuid, host_sock, host_rx) = spawn_real_host_binary(
        &bootstrap_path,
        &[("UNLEASH_HOST_CUTPOINT_TARGET", "C5")],
        true,
    );

    let (mut pty_sock, mut pty_reader) = accept_sync_client(&sync_listener);
    assert_eq!(read_line_timeout(&mut pty_reader), "PTY_READY");

    // Envoyer SendInput dans un thread client (il bloquera sur réponse à cause de C5)
    let sock_clone = host_sock.clone();
    let client_thread = std::thread::spawn(move || {
        let mut client = HostClient::connect(
            &sock_clone,
            run_uuid,
            gen_a,
            DaemonEpoch::new(1),
            Fence::new(1),
        )
        .unwrap();
        let _ = client.call(&HostRequest::SendInput {
            header: mk_header(run_uuid, gen_a, 1, 1, 100),
            secret: secret.to_owned(),
            input_seq: 1,
            payload_bytes: b"APPLY_ORACLE_1\n".to_vec(),
            now_ms: TimestampMs::new(1234),
        });
    });

    // Attendre que : (1) le worker PTY ait committé l'effet dans l'oracle, ET (2) le host soit à C5
    assert_eq!(read_line_timeout(&mut pty_reader), "EFFECT_COMMITTED");

    loop {
        let line = host_rx.recv_timeout(IO_TIMEOUT).unwrap();
        if line == "UNLEASH_CRASH_REACHED:C5" {
            break;
        }
    }

    // SIGKILL réel du process-host juste après write PTY et avant persistance du reçu !
    host_child.kill().unwrap();
    let status = host_child.wait().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }
    let _ = client_thread.join();

    // Vérifier oracle = 1 effet accompli
    assert_eq!(oracle.count().unwrap(), 1);

    // Réconciliation hors ligne -> entrée seq=1 classée OutcomeUnknown et aucune répétition permise
    let manifest_path = runtime_dir.join(HOST_MANIFEST_FILENAME);
    let recon = reconcile_after_lost_host(&manifest_path, &OsProcessInspector).unwrap();
    assert_eq!(recon.unknown_input_seqs, vec![1]);
    assert!(recon.signal_refusal_reason.contains("IDENTITY_UNVERIFIED"));

    // Tenter de rejouer seq=1 sur PtyInputTracker rouvert -> TerminalInputOutcomeUnknown, 0 écriture !
    let tracker = unleash_process_host::PtyInputTracker::open(&runtime_dir, None).unwrap();
    let mut sink = Vec::new();
    let err = tracker
        .dispatch_input(
            &mut sink,
            1,
            CommandId::from_u128(100),
            ActionId::from_u128(1100),
            b"APPLY_ORACLE_1\n",
            TimestampMs::new(2000),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        ProcessHostError::TerminalInputOutcomeUnknown { input_seq: 1, .. }
    ));
    assert!(sink.is_empty());
    assert_eq!(oracle.count().unwrap(), 1);

    // Nettoyer le worker PTY s'il n'a pas déjà quitté sur EOF/SIGHUP du maître PTY
    let _ = pty_sock.write_all(b"EXIT\n");
    let _ = pty_sock.flush();
}

// T-F011-006 / T-F011-010 / REQ-TERM-006 / REQ-TERM-010
#[test]
fn t_f011_006_t_f011_010_sigkill_host_preserves_logs_and_stop_reports_detached_descendant() {
    // 1. SIGKILL process-host -> conservation logs durables, perte PTY exposée, reboot -> lost
    let dir = local_test_dir("sp-hk").unwrap();
    let runtime_dir = dir.path().join("host");
    let work_dir = dir.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    ensure_private_rendezvous_dir(&runtime_dir).unwrap();

    let sync_sock_path = dir.path().join("sync.sock");
    let sync_listener = UnixListener::bind(&sync_sock_path).unwrap();

    let secret = "ctrl-secret-hk";
    let secret_path = runtime_dir.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &HostControlSecret::new(secret).unwrap()).unwrap();

    let run_uuid = ProcessRunUuid::from_u128(0x3333);
    let gen_a = AuthorityGeneration::from_u128(0xAAAA);
    let mut explicit_env = BTreeMap::new();
    explicit_env.insert(ENV_ROLE.to_owned(), "pty_worker".to_owned());
    explicit_env.insert(
        ENV_SYNC_SOCK.to_owned(),
        sync_sock_path.display().to_string(),
    );

    let spec = HostBootstrapSpec {
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: Some(TerminalId::from_u128(3)),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(4),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir.clone(),
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_pty_worker_entrypoint"),
        explicit_env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: secret_path,
        canaries_secret_path: None,
        max_segment_bytes: 256,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 256,
    };
    let bootstrap_path = runtime_dir.join("bootstrap.json");
    write_bootstrap(&bootstrap_path, &spec);

    let (mut host_child, _uuid, host_sock, _rx) =
        spawn_real_host_binary(&bootstrap_path, &[], false);
    let (mut pty_sock, mut pty_reader) = accept_sync_client(&sync_listener);
    assert_eq!(read_line_timeout(&mut pty_reader), "PTY_READY");

    // Synchroniser les logs durables avant SIGKILL du host
    let mut client = HostClient::connect(
        &host_sock,
        run_uuid,
        gen_a,
        DaemonEpoch::new(1),
        Fence::new(1),
    )
    .unwrap();
    let logs = read_all_spool_text(&mut client, mk_header(run_uuid, gen_a, 1, 1, 1), secret);
    assert!(logs.contains("PTY_PRE_KILL_DATA"));
    drop(client);

    host_child.kill().unwrap();
    let _ = host_child.wait().unwrap();

    let manifest_path = runtime_dir.join(HOST_MANIFEST_FILENAME);
    let recon = reconcile_after_lost_host(&manifest_path, &OsProcessInspector).unwrap();
    // Pour un PTY, la perte du maître PTY peut entraîner SIGHUP/EOF sur l'enfant (Exited)
    // ou le laisser vivant (Running) s'il ignore SIGHUP ; dans tous les cas, les logs durables
    // sont conservés et tout contrôle par signal sans ancrage est refusé en IdentityUnverified.
    assert!(matches!(
        recon.reconciled_state,
        HostLaunchState::Exited { .. } | HostLaunchState::Running
    ));
    assert!(!recon.durable_spool.segments.is_empty());
    assert!(recon.signal_refusal_reason.contains("IDENTITY_UNVERIFIED"));

    // Cas Service survivant (sans PTY SIGHUP) : l'enfant survit au SIGKILL du host -> Running,
    // puis contrôle sans ancrage refusé IdentityUnverified.
    let dir_svc = local_test_dir("sp-svchk").unwrap();
    let rt_svc = dir_svc.path().join("host");
    ensure_private_rendezvous_dir(&rt_svc).unwrap();
    let sync_svc_path = dir_svc.path().join("sync.sock");
    let sync_svc_listener = UnixListener::bind(&sync_svc_path).unwrap();
    let svc_ctrl_secret = "ctrl-secret-svchk";
    let svc_ctrl_secret_path = rt_svc.join(CONTROL_SECRET_FILENAME);
    store_control_secret(
        &svc_ctrl_secret_path,
        &HostControlSecret::new(svc_ctrl_secret).unwrap(),
    )
    .unwrap();
    let run_svc = ProcessRunUuid::from_u128(0x3399);
    let mut env_svc = BTreeMap::new();
    env_svc.insert(ENV_ROLE.to_owned(), "pty_worker".to_owned());
    env_svc.insert(
        ENV_SYNC_SOCK.to_owned(),
        sync_svc_path.display().to_string(),
    );
    let svc_bootstrap = HostBootstrapSpec {
        runtime_dir: rt_svc.clone(),
        host_id: HostId::from_u128(11),
        process_id: ProcessId::from_u128(22),
        terminal_id: None,
        process_run_uuid: run_svc,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(44),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir.clone(),
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_pty_worker_entrypoint"),
        explicit_env: env_svc,
        resource_kind: ResourceKind::Service,
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: svc_ctrl_secret_path,
        canaries_secret_path: None,
        max_segment_bytes: 256,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 256,
    };
    let bs_svc = rt_svc.join("bootstrap.json");
    write_bootstrap(&bs_svc, &svc_bootstrap);
    let (mut svc_host, _uuid_svc, _sock_svc, _rx_svc) = spawn_real_host_binary(&bs_svc, &[], false);
    let (mut svc_conn, mut svc_reader) = accept_sync_client(&sync_svc_listener);
    assert_eq!(read_line_timeout(&mut svc_reader), "PTY_READY");
    svc_host.kill().unwrap();
    let _ = svc_host.wait().unwrap();
    let recon_svc =
        reconcile_after_lost_host(&rt_svc.join(HOST_MANIFEST_FILENAME), &OsProcessInspector)
            .unwrap();
    assert_eq!(recon_svc.reconciled_state, HostLaunchState::Running);
    assert!(
        recon_svc
            .signal_refusal_reason
            .contains("IDENTITY_UNVERIFIED")
    );
    svc_conn.write_all(b"EXIT\n").unwrap();
    svc_conn.flush().unwrap();

    // Simulation reboot (boot_identity différente) -> clos en `lost`
    let reboot_recon = reconcile_after_lost_host(&manifest_path, &DifferentBootInspector).unwrap();
    assert_eq!(reboot_recon.reconciled_state, HostLaunchState::Lost);

    let _ = pty_sock.write_all(b"EXIT\n");
    let _ = pty_sock.flush();

    // 2. Arbre enfant + petit-enfant même PGID + descendant détaché (setsid)
    let dir2 = local_test_dir("sp-tree").unwrap();
    let rt2 = dir2.path().join("host");
    ensure_private_rendezvous_dir(&rt2).unwrap();
    let sock2_path = dir2.path().join("sync2.sock");
    let listener2 = UnixListener::bind(&sock2_path).unwrap();

    let sec_tree = "ctrl-secret-tree";
    let sec2_path = rt2.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&sec2_path, &HostControlSecret::new(sec_tree).unwrap()).unwrap();

    let run2 = ProcessRunUuid::from_u128(0x4444);
    let mut env2 = BTreeMap::new();
    env2.insert(ENV_ROLE.to_owned(), "descendant_tree".to_owned());
    env2.insert(ENV_SYNC_SOCK.to_owned(), sock2_path.display().to_string());

    let spec2 = HostBootstrapSpec {
        runtime_dir: rt2.clone(),
        host_id: HostId::from_u128(2),
        process_id: ProcessId::from_u128(3),
        terminal_id: None,
        process_run_uuid: run2,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(5),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir,
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_descendant_tree_entrypoint"),
        explicit_env: env2,
        resource_kind: ResourceKind::Service,
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: sec2_path,
        canaries_secret_path: None,
        max_segment_bytes: 256,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 256,
    };

    let server = ArcHostGuard::start(&spec2);
    let mut same_pid = None;
    let mut detached_pid = None;
    let mut detached_sock = None;
    let mut leader_seen = false;

    for _ in 0..3 {
        let (s, mut r) = accept_sync_client(&listener2);
        let msg = read_line_timeout(&mut r);
        if msg.starts_with("LEADER_READY:") {
            leader_seen = true;
        } else if let Some(p) = msg.strip_prefix("SAME:") {
            same_pid = Some(p.parse::<u32>().unwrap());
        } else if let Some(p) = msg.strip_prefix("DETACHED:") {
            detached_pid = Some(p.parse::<u32>().unwrap());
            detached_sock = Some(s);
        }
    }
    assert!(leader_seen);
    let same_pid = same_pid.unwrap();
    let detached_pid = detached_pid.unwrap();

    let mut c2 = HostClient::connect(
        server.socket_path(),
        run2,
        gen_a,
        DaemonEpoch::new(1),
        Fence::new(1),
    )
    .unwrap();
    let stop_resp = c2
        .call(&HostRequest::Stop {
            header: mk_header(run2, gen_a, 1, 1, 200),
            secret: sec_tree.to_owned(),
            escalate_to_sigkill: true,
            known_descendant_pids: vec![same_pid, detached_pid],
        })
        .unwrap();
    let HostResponse::StopObserved { observation } = stop_resp else {
        panic!("réponse Stop inattendue : {stop_resp:?}");
    };
    assert!(observation.direct_exit.is_some());
    assert!(observation.group_signaled);
    assert!(
        observation
            .detached_or_unknown_descendants
            .contains(&detached_pid),
        "descendant détaché non signalé : {observation:?}"
    );
    assert!(!observation.confirmed_group_clear);

    // Le descendant détaché doit être nettoyé via son propre canal de test
    drop(detached_sock);
}

// T-F011-008 / T-F011-004 / REQ-TERM-008 / REQ-TERM-004
#[test]
fn t_f011_008_t_f011_004_spool_redaction_gap_and_zero_model() {
    let dir = local_test_dir("sp-red").unwrap();
    let runtime_dir = dir.path().join("host");
    let work_dir = dir.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    ensure_private_rendezvous_dir(&runtime_dir).unwrap();

    let sync_sock_path = dir.path().join("sync.sock");
    let sync_listener = UnixListener::bind(&sync_sock_path).unwrap();

    let secret = "ctrl-secret-red";
    let secret_path = runtime_dir.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &HostControlSecret::new(secret).unwrap()).unwrap();

    let canaries_path = runtime_dir.join("canaries.secret");
    store_canary_secrets(
        &canaries_path,
        &["CANARY_SECRET_ALPHA", "CANARY_MULTI\nLINE_BETA"],
    )
    .unwrap();

    let run_uuid = ProcessRunUuid::from_u128(0x5555);
    let gen_a = AuthorityGeneration::from_u128(0xAAAA);
    let mut explicit_env = BTreeMap::new();
    explicit_env.insert(ENV_ROLE.to_owned(), "pty_worker".to_owned());
    explicit_env.insert(
        ENV_SYNC_SOCK.to_owned(),
        sync_sock_path.display().to_string(),
    );

    let spec = HostBootstrapSpec {
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: Some(TerminalId::from_u128(3)),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(4),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir,
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_pty_worker_entrypoint"),
        explicit_env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: secret_path,
        canaries_secret_path: Some(canaries_path),
        max_segment_bytes: 128,
        max_total_log_bytes: 512,
        max_volatile_bytes: 128,
    };

    let guard = ArcHostGuard::start(&spec);
    let (mut pty_sock, mut pty_reader) = accept_sync_client(&sync_listener);
    assert_eq!(read_line_timeout(&mut pty_reader), "PTY_READY");

    pty_sock.write_all(b"EMIT_FLOOD_AND_CANARIES\n").unwrap();
    pty_sock.flush().unwrap();
    assert_eq!(read_line_timeout(&mut pty_reader), "FLOOD_DONE");

    pty_sock.write_all(b"EXIT\n").unwrap();
    pty_sock.flush().unwrap();

    let mut client = HostClient::connect(
        guard.socket_path(),
        run_uuid,
        gen_a,
        DaemonEpoch::new(1),
        Fence::new(1),
    )
    .unwrap();

    // Rejeter toute tentative d'appel modèle autonome dans le process-host (REQ-TERM-004)
    let model_resp = client
        .call(&HostRequest::ModelCallAttempt {
            header: mk_header(run_uuid, gen_a, 1, 1, 300),
            secret: secret.to_owned(),
            prompt_summary: String::from("summarize logs"),
        })
        .unwrap();
    assert!(matches!(
        model_resp,
        HostResponse::Error { error }
            if error.code == ErrorCode::CapabilityDenied
                && error.reason_code == Some(ReasonCode::ModelCallsForbidden)
    ));

    let _ = client
        .call(&HostRequest::Stop {
            header: mk_header(run_uuid, gen_a, 1, 2, 301),
            secret: secret.to_owned(),
            escalate_to_sigkill: true,
            known_descendant_pids: vec![],
        })
        .unwrap();

    let read_resp = client
        .call(&HostRequest::ReadLog {
            header: mk_header(run_uuid, gen_a, 1, 3, 302),
            secret: secret.to_owned(),
            after_offset: 0,
            max_bytes: 64 * 1024,
            strict_cursor: false,
        })
        .unwrap();
    let HostResponse::LogChunk { result } = read_resp else {
        panic!("réponse ReadLog inattendue : {read_resp:?}");
    };
    assert!(
        !result.gaps.is_empty(),
        "saturation attendue -> gap explicite"
    );
    assert!(result.redaction_count >= 2);

    // Vérifier absence totale des secrets canaris dans tous les fichiers générés sur disque
    for entry in fs::read_dir(runtime_dir.join("spool")).unwrap() {
        let p = entry.unwrap().path();
        if p.is_file() {
            let content = fs::read_to_string(&p).unwrap();
            assert!(!content.contains("CANARY_SECRET_ALPHA"));
            assert!(!content.contains("LINE_BETA"));
        }
    }
    let manifest_text = fs::read_to_string(runtime_dir.join(HOST_MANIFEST_FILENAME)).unwrap();
    assert!(!manifest_text.contains(secret));
    assert!(!manifest_text.contains("CANARY_SECRET_ALPHA"));
}

// T-SEC-03 / T-SEC-04 / Q-05
#[cfg(target_os = "macos")]
#[test]
fn t_sec_macos_seatbelt_strict_mode_and_isolation_unavailable_negative() {
    let dir = local_test_dir("sp-sec").unwrap();
    let runtime_dir = dir.path().join("host");
    let data_root = dir.path().join("data_root");
    let socket_dir = dir.path().join("socket_dir");
    let cred_dir = dir.path().join("creds");
    let work_dir = dir.path().join("work");
    let tmp_dir = dir.path().join("tmp");
    let prof_dir = dir.path().join("prof");
    let outside_dir = dir.path().join("outside");
    for d in [
        &data_root,
        &socket_dir,
        &cred_dir,
        &work_dir,
        &tmp_dir,
        &outside_dir,
    ] {
        fs::create_dir_all(d).unwrap();
    }
    ensure_private_rendezvous_dir(&runtime_dir).unwrap();
    ensure_private_rendezvous_dir(&prof_dir).unwrap();

    let data_secret = data_root.join("db.sqlite");
    let sock_secret = socket_dir.join("daemon.sock");
    let cred_secret = cred_dir.join("id_ed25519");
    fs::write(&data_secret, b"TOP_SECRET_DB").unwrap();
    fs::write(&sock_secret, b"TOP_SECRET_SOCK").unwrap();
    fs::write(&cred_secret, b"TOP_SECRET_SSH").unwrap();

    let keychain_dir = dir.path().join("fake_keychains");
    fs::create_dir_all(&keychain_dir).unwrap();
    fs::write(keychain_dir.join("login.keychain-db"), b"KEYCHAIN_DATA").unwrap();

    let allowed_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let blocked_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed_addr = allowed_listener.local_addr().unwrap();
    let blocked_port = blocked_listener.local_addr().unwrap().port();

    let done_fifo = work_dir.join("done.fifo");
    let fifo_status = Command::new("mkfifo").arg(&done_fifo).status().unwrap();
    assert!(fifo_status.success());
    // Ouvrir en O_RDWR côté parent évite tout blocage d'ouverture avant démarrage de l'enfant
    let mut fifo_rd = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&done_fifo)
        .unwrap();

    let report_path = work_dir.join("probe_report.json");
    let outside_path = outside_dir.join("escape.txt");
    let probe_cfg = StrictProbeConfig {
        allowed_work_dir: work_dir.clone(),
        outside_forbidden_path: outside_path.clone(),
        data_secret_path: data_secret,
        socket_secret_path: sock_secret,
        cred_secret_path: cred_secret,
        keychain_probe_path: keychain_dir.clone(),
        allowed_tcp_port: allowed_addr.port(),
        blocked_tcp_port: blocked_port,
        done_fifo_path: done_fifo,
        report_path: report_path.clone(),
    };
    let probe_cfg_path = work_dir.join("probe_cfg.json");
    fs::write(&probe_cfg_path, serde_json::to_vec(&probe_cfg).unwrap()).unwrap();

    let secret = "ctrl-secret-strict";
    let secret_path = runtime_dir.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &HostControlSecret::new(secret).unwrap()).unwrap();

    let run_uuid = ProcessRunUuid::from_u128(0x6666);
    let gen_a = AuthorityGeneration::from_u128(0xAAAA);
    let mut explicit_env = BTreeMap::new();
    explicit_env.insert(ENV_ROLE.to_owned(), "strict_sandbox_probe".to_owned());
    explicit_env.insert(
        ENV_PROBE_CFG.to_owned(),
        probe_cfg_path.display().to_string(),
    );

    let mut keychain_paths = vec![keychain_dir];
    if let Some(home) = std::env::var_os("HOME") {
        let user_kc = PathBuf::from(home).join("Library/Keychains");
        if user_kc.exists() {
            keychain_paths.push(user_kc);
        }
    }

    let strict_spec = StrictIsolationSpec {
        profile_kind: IsolationProfileKind::WorkspaceCommand,
        assigned_write_roots: vec![work_dir.clone(), tmp_dir],
        data_root,
        socket_dir,
        keychain_paths,
        declared_credential_paths: vec![cred_dir],
        egress_proxy_addr: Some(allowed_addr),
        extra_loopback_control_ports: vec![],
        profile_output_dir: prof_dir,
    };

    let spec = HostBootstrapSpec {
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: Some(TerminalId::from_u128(3)),
        process_run_uuid: run_uuid,
        authority_generation: gen_a,
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(4),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: work_dir.clone(),
        program: std::env::current_exe().unwrap().display().to_string(),
        arguments: current_exe_fixture_args("fixture_strict_sandbox_probe_entrypoint"),
        explicit_env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Strict,
        strict_spec: Some(strict_spec.clone()),
        isolation_profile_kind: Some(IsolationProfileKind::WorkspaceCommand),
        control_secret_path: secret_path,
        canaries_secret_path: None,
        max_segment_bytes: 4096,
        max_total_log_bytes: 64 * 1024,
        max_volatile_bytes: 4096,
    };

    // 1. Positif : lancement réel sous /usr/bin/sandbox-exec dans PTY
    let guard = ArcHostGuard::start(&spec);

    let mut byte = [0_u8; 1];
    fifo_rd.read_exact(&mut byte).unwrap();
    assert_eq!(byte[0], b'1');

    let report: StrictProbeReport =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert!(report.write_inside_ok);
    assert!(report.write_outside_denied);
    assert!(!outside_path.exists());
    assert!(report.read_data_root_denied);
    assert!(report.read_socket_dir_denied);
    assert!(report.read_cred_denied);
    assert!(report.read_keychain_denied);
    assert!(report.allowed_tcp_ok);
    assert!(report.blocked_tcp_denied);
    assert!(report.inherited_secret_env_absent);

    let mf = HostManifest::load_private(&runtime_dir.join(HOST_MANIFEST_FILENAME)).unwrap();
    assert_eq!(mf.isolation_mode, IsolationMode::Strict);
    assert_eq!(mf.isolation_backend.as_deref(), Some("macos-seatbelt"));
    assert!(mf.isolation_profile_digest.is_some());
    drop(guard);

    // 2. Négatifs : backend absent -> IsolationBackendMissing, jamais de repli vers standard
    let dir_neg = local_test_dir("sp-neg").unwrap();
    let rt_neg = dir_neg.path().join("host");
    ensure_private_rendezvous_dir(&rt_neg).unwrap();
    let neg_marker = work_dir.join("must_not_exist.txt");
    let neg_secret_path = rt_neg.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&neg_secret_path, &HostControlSecret::new("s-neg").unwrap()).unwrap();

    let mut neg_spec = spec.clone();
    neg_spec.runtime_dir = rt_neg;
    neg_spec.control_secret_path = neg_secret_path;
    neg_spec.program = String::from("/usr/bin/touch");
    neg_spec.arguments = vec![neg_marker.display().to_string()];

    let missing_probe = OsIsolationBackendProbe::with_paths(
        PathBuf::from("/nonexistent/sandbox-exec"),
        PathBuf::from("/nonexistent/bwrap"),
    );
    let err_missing = ProcessHostServer::start(
        &neg_spec,
        &missing_probe,
        Box::new(OsProcessInspector),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err_missing,
        ProcessHostError::IsolationBackendMissing { .. }
    ));
    assert_eq!(
        err_missing.to_ipc_error(None).code,
        ErrorCode::IsolationUnavailable
    );
    assert!(!neg_marker.exists());

    // 3. Négatif : obligation impossible -> IsolationObligationUnenforceable, zéro lancement
    let err_unenforceable = ProcessHostServer::start(
        &neg_spec,
        &UnenforceableProbe,
        Box::new(OsProcessInspector),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err_unenforceable,
        ProcessHostError::IsolationObligationUnenforceable { .. }
    ));
    assert_eq!(
        err_unenforceable.to_ipc_error(None).reason_code,
        Some(ReasonCode::IsolationObligationUnenforceable)
    );
    assert!(!neg_marker.exists());
}

// ================================================================================================
// UTILITAIRE GARDE RAII POUR SERVEUR EN PROCESSUS IN-THREAD DANS LES TESTS
// ================================================================================================

struct ArcHostGuard {
    server: std::sync::Arc<ProcessHostServer>,
    thread: Option<std::thread::JoinHandle<Result<(), ProcessHostError>>>,
}

impl ArcHostGuard {
    fn start(spec: &HostBootstrapSpec) -> Self {
        let probe = OsIsolationBackendProbe::new();
        let server = std::sync::Arc::new(
            ProcessHostServer::start(spec, &probe, Box::new(OsProcessInspector), None).unwrap(),
        );
        let srv_clone = std::sync::Arc::clone(&server);
        let thread = std::thread::spawn(move || srv_clone.serve_until_shutdown());
        Self {
            server,
            thread: Some(thread),
        }
    }

    fn socket_path(&self) -> &Path {
        self.server.socket_path()
    }
}

impl Drop for ArcHostGuard {
    fn drop(&mut self) {
        self.server.request_shutdown();
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

struct DifferentBootInspector;

impl ProcessInspector for DifferentBootInspector {
    fn current_boot(&self) -> Result<BootIdentity, ProcessHostError> {
        BootIdentity::new("rebooted-machine-boot-2").map_err(ProcessHostError::from)
    }

    fn inspect_pid(&self, pid: u32) -> Result<OsProcessIdentity, ProcessHostError> {
        inspect_process_identity(pid).map_err(ProcessHostError::from)
    }

    fn verify_identity(
        &self,
        _expected: &OsProcessIdentity,
    ) -> Result<ProcessVerification, ProcessHostError> {
        Ok(ProcessVerification::BootChanged)
    }

    fn list_group_members(&self, _pgid: u32) -> Vec<OsProcessIdentity> {
        vec![]
    }
}

struct UnenforceableProbe;

impl IsolationBackendProbe for UnenforceableProbe {
    fn macos_sandbox_exec_path(&self) -> &Path {
        Path::new("/usr/bin/sandbox-exec")
    }

    fn probe_macos_seatbelt(&self, _: &Path, _: &Path) -> SandboxProbeOutcome {
        SandboxProbeOutcome::Unenforceable {
            detail: String::from("sandbox_apply refusé par environnement hôte"),
        }
    }

    fn linux_bwrap_path(&self) -> &Path {
        Path::new("/usr/bin/bwrap")
    }

    fn probe_linux_bwrap(&self, _: &Path) -> SandboxProbeOutcome {
        SandboxProbeOutcome::Unenforceable {
            detail: String::from("userns désactivés"),
        }
    }
}

#[allow(dead_code)]
fn _unused_secret_string_import(_: SecretString) {}
