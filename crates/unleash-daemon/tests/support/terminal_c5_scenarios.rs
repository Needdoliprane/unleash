//! C5 product oracles; the input scenario remains red until the writer/route
//! seams are available. The daemon library runs in a real owned subprocess;
//! authentication, SQLite, broker, native host binary and PTY remain real.
//! No replay uses the native SendInput helper to inspect an ambiguous write.

use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;

const FIXTURE_ENTRY: &str =
    "reattach_scenarios::input_scenarios::c5_scenarios::terminal_c5_daemon_fixture";
const WORKER_ENTRY: &str =
    "reattach_scenarios::input_scenarios::c5_scenarios::terminal_c5_worker_fixture";
const ENV_DATA: &str = "UNLEASH_C5_FIXTURE_DATA";
const ENV_RENDEZVOUS: &str = "UNLEASH_C5_FIXTURE_RENDEZVOUS";
const ENV_MARKER: &str = "UNLEASH_C5_FIXTURE_MARKER";
const ENV_NOTIFY: &str = "UNLEASH_C5_FIXTURE_NOTIFY";
const READY_PREFIX: &str = "UNLEASH_C5_FIXTURE_READY:";
const EFFECT_SIGNAL: &[u8] = b"EFFECT_FSYNCED:1\n";
const MARKER_SIGNAL: &[u8] = b"C5_MARKER_FSYNCED\n";
const PROBE_NONCE: &[u8; 16] = b"post-restart-001";

/// Test-only Rust composition; no IPC flag or production environment switch
/// selects this launcher. The product default keeps its packaged sibling path.
struct C5FixtureLauncher {
    executable: PathBuf,
    marker: PathBuf,
    notify: PathBuf,
    controller_inputs: Mutex<Vec<std::process::ChildStdin>>,
}

impl unleash_daemon::terminal_host_launcher::TerminalHostLauncher for C5FixtureLauncher {
    fn spawn(&self, _packaged: &Path, bootstrap: &Path) -> std::io::Result<Child> {
        let mut child = Command::new(&self.executable)
            .arg("--bootstrap")
            .arg(bootstrap)
            .env_clear()
            .env("UNLEASH_HOST_CUTPOINT_TARGET", "C5")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        // Only the daemon subprocess owns this writer. Its SIGKILL must close
        // every copy so the still-live host sees EOF at C5 before any receipt.
        let controller_input = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("C5 controller pipe absent"))?;
        self.controller_inputs
            .lock()
            .map_err(|_| std::io::Error::other("C5 pipe owner poisoned"))?
            .push(controller_input);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("C5 marker pipe absent"))?;
        let marker = self.marker.clone();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if line.ends_with("UNLEASH_CRASH_REACHED:C5") {
                    // Notify only after this marker is durably written. The
                    // worker independently notifies after effect.sync_all().
                    unleash_platform::write_private_file_atomic(&marker, MARKER_SIGNAL).unwrap();
                    let mut stream = std::os::unix::net::UnixStream::connect(&notify).unwrap();
                    stream.write_all(MARKER_SIGNAL).unwrap();
                    stream.flush().unwrap();
                }
            }
        });
        Ok(child)
    }
}

fn exact_host_binary() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_unleashd"))
        .parent()
        .unwrap()
        .join("unleash-process-host")
}

#[tokio::test]
#[ignore = "owned daemon-library C5 fixture selected by exact argv only"]
async fn terminal_c5_daemon_fixture() {
    let data = PathBuf::from(std::env::var_os(ENV_DATA).unwrap());
    let rendezvous = PathBuf::from(std::env::var_os(ENV_RENDEZVOUS).unwrap());
    let marker = PathBuf::from(std::env::var_os(ENV_MARKER).unwrap());
    let notify = PathBuf::from(std::env::var_os(ENV_NOTIFY).unwrap());
    let executable = exact_host_binary();
    assert!(
        executable.is_file(),
        "the exact prebuilt native host is required"
    );
    let config = unleash_daemon::DaemonConfig {
        data_root: Some(data),
        rendezvous_dir: Some(rendezvous),
        terminal_host_launcher: Arc::new(C5FixtureLauncher {
            executable,
            marker,
            notify,
            controller_inputs: Mutex::new(vec![]),
        }),
        ..unleash_daemon::DaemonConfig::default()
    };
    let unleash_daemon::DaemonStartOutcome::Started(handle) =
        unleash_daemon::UnleashDaemon::start(config).await.unwrap()
    else {
        unreachable!("an exclusively owned fixture must start its actual authority");
    };
    handle.wait_ready().await.unwrap();
    writeln!(std::io::stdout(), "{READY_PREFIX}{}",
        json!({"status":"ready","authority_generation":handle.authority_generation(),"daemon_epoch":handle.daemon_epoch()})).unwrap();
    std::io::stdout().flush().unwrap();
    let _ = handle.wait_stopped().await;
}

impl FixtureDaemon {
    fn spawn_c5(data: &Path, rendezvous: &Path, marker: &Path, notify: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", FIXTURE_ENTRY, "--nocapture"])
            .env_clear()
            .env("HOME", std::env::var_os("HOME").unwrap())
            .env(ENV_DATA, data)
            .env(ENV_RENDEZVOUS, rendezvous)
            .env(ENV_MARKER, marker)
            .env(ENV_NOTIFY, notify)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, ready) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut sender = Some(sender);
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if let Some((_, json)) = line.split_once(READY_PREFIX)
                    && let Some(sender) = sender.take()
                {
                    let _ = sender.send(Ok(json.to_owned()));
                }
            }
        });
        Self {
            child,
            ready,
            data_root: data.to_path_buf(),
            cleanup_hosts: true,
        }
    }
}

fn signal_listener(path: &Path) -> tokio::net::UnixListener {
    let listener = unleash_platform::bind_private_unix_listener(path).unwrap();
    listener.set_nonblocking(true).unwrap();
    tokio::net::UnixListener::from_std(listener).unwrap()
}

async fn wait_signal(listener: &tokio::net::UnixListener, expected: &[u8]) {
    let (mut stream, _) = tokio::time::timeout(DEADLINE, listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stream.peer_cred().unwrap().uid(),
        unleash_platform::effective_uid()
    );
    let mut message = vec![0; expected.len()];
    tokio::time::timeout(DEADLINE, stream.read_exact(&mut message))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message, expected);
}

async fn prepare_c5_authority() -> (
    LiveFixture,
    tokio::net::UnixListener,
    tokio::net::UnixListener,
    PathBuf,
) {
    // Existing real producers create project, scope, mission, versioned profile
    // and pools before replacing only the daemon composition (no host yet).
    let mut fixture = LiveFixture::new(true).await;
    std::fs::set_permissions(
        fixture.project_root.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let effect = signal_listener(&fixture.project_root.path().join("e.sock"));
    let marker_socket = fixture.rendezvous.path().join("c5.sock");
    let notification = signal_listener(&marker_socket);
    let marker = fixture.data.path().join("c5-reached");
    fixture.daemon.kill_for_restart();
    let daemon = FixtureDaemon::spawn_c5(
        fixture.data.path(),
        fixture.rendezvous.path(),
        &marker,
        &marker_socket,
    );
    daemon.readiness();
    fixture.daemon = daemon;
    fixture.client = UnleashClient::connect(ClientConfig::new(
        fixture.rendezvous.path().join("daemon.sock"),
        ClientKind::Cli,
        "f011-c5-real-authority",
    ))
    .await
    .unwrap();
    assert_eq!(
        fixture.client.hello().authority_generation,
        fixture.authority
    );
    (fixture, effect, notification, marker)
}

async fn start_c5_terminal(fixture: &mut LiveFixture) -> Value {
    let mut request = fixture.allocation(0xf011_4105);
    request.payload.insert(
        "arguments".into(),
        json!(["--ignored", "--exact", WORKER_ENTRY, "--nocapture"]),
    );
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let reference = accepted.result_ref.unwrap();
    let query = result_query(fixture, request.command_id, &reference, 0xf011_6001);
    let deadline = Instant::now() + DEADLINE;
    loop {
        match fixture.client.query(&query).await {
            Ok(response) => return response.result,
            Err(unleash_client::ClientError::Remote(error))
                if [
                    unleash_protocol::ErrorCode::StaleFence,
                    unleash_protocol::ErrorCode::ResourceBusy,
                ]
                .contains(&error.code) => {}
            Err(error) => unreachable!("fresh actual acquisition was refused: {error:?}"),
        }
        assert!(
            Instant::now() < deadline,
            "qualified actual acquisition before input"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn input_command(fixture: &LiveFixture, acquired: &Value) -> CommandEnvelope {
    command(
        fixture.authority,
        0xf011_6002,
        fixture.project,
        acquired["terminal_id"].as_str().unwrap().parse().unwrap(),
        acquired["control_revision"].as_u64().unwrap(),
        "SendTerminalInput",
        json!({"terminal_id":acquired["terminal_id"],"process_run_uuid":acquired["process_run_uuid"],
            "lease_id":acquired["lease_id"],"fence":acquired["fence"],
            "input_seq":acquired["input_seq"].as_u64().unwrap().checked_add(1).unwrap(),
            "payload_bytes":b"apply-effect\n".to_vec()}),
    )
}

fn read_native_journal(manifest: &unleash_process_host::HostManifest) -> Value {
    let path = manifest
        .socket_path
        .parent()
        .unwrap()
        .join(unleash_process_host::INPUT_JOURNAL_FILENAME);
    let encoded = unleash_platform::read_private_file_to_string(&path)
        .unwrap()
        .unwrap();
    serde_json::from_str(&encoded).unwrap()
}

fn host_registration(
    fixture: &LiveFixture,
    manifest: &unleash_process_host::HostManifest,
) -> unleash_platform::OsProcessIdentity {
    let encoded = unleash_platform::read_private_file_to_string(
        &fixture.rendezvous.path().join("hosts.json"),
    )
    .unwrap()
    .unwrap();
    let registrations: Value = serde_json::from_str(&encoded).unwrap();
    let registered = registrations["executors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["executor_id"] == manifest.host_id.to_string())
        .unwrap();
    let registered: unleash_daemon::ExecutorRegistration =
        serde_json::from_value(registered.clone()).unwrap();
    assert_eq!(
        registered.data_root.canonicalize().unwrap(),
        fixture.data.path().canonicalize().unwrap()
    );
    let identity = registered.os_identity.unwrap();
    assert_eq!(registered.pid, Some(identity.pid));
    identity
}

async fn transfer_after_c5(fixture: &mut LiveFixture, acquired: &Value) {
    let request = command(
        fixture.authority,
        0xf011_6003,
        fixture.project,
        acquired["terminal_id"].as_str().unwrap().parse().unwrap(),
        0,
        "AcquireTerminal",
        json!({"terminal_id":acquired["terminal_id"],"process_run_uuid":acquired["process_run_uuid"],"scope_revision":2,"control_mode":"exclusive"}),
    );
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let deadline = Instant::now() + DEADLINE;
    loop {
        let snapshot = fixture.snapshot().await;
        if snapshot["terminals"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["terminal_id"] == acquired["terminal_id"]
                    && item["control_revision"] == 1
                    && item["lease_state"] == "held"
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual transfer after ambiguous input"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The new controller is observed, but its native summary still has the
    // ambiguous input: it must not publish usable input assertions.
    let query = result_query(
        fixture,
        request.command_id,
        accepted.result_ref.as_ref().unwrap(),
        0xf011_6004,
    );
    let refused = fixture.client.query(&query).await.unwrap_err();
    assert!(
        matches!(refused,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::StaleFence)
    );
}

async fn assert_pty_probe_output(fixture: &mut LiveFixture, terminal: &Value) {
    let mut probe = tokio::net::UnixStream::connect(fixture.project_root.path().join("p.sock"))
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut probe, PROBE_NONCE)
        .await
        .unwrap();
    let mut ack = [0; 8];
    tokio::time::timeout(DEADLINE, probe.read_exact(&mut ack))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ack, b"PRINTED\n");
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_6005),
        query_type: "ReadTerminalLog".into(),
        project_id: Some(fixture.project),
        payload: object(
            json!({"terminal_id":terminal["terminal_id"],"after_offset":0,"max_bytes":65536}),
        ),
    };
    let deadline = Instant::now() + DEADLINE;
    loop {
        let response = fixture.client.query(&query).await.unwrap();
        if response.result["text"]
            .as_str()
            .unwrap()
            .contains("c5-post-restart:post-restart-001")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fresh worker output traverses the real PTY"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_one_unknown_input(fixture: &LiveFixture, original: &CommandEnvelope) {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (actions,dispatches):(i64,i64)=conn.query_row("SELECT (SELECT COUNT(*) FROM actions WHERE action_type='terminal.send_input'),(SELECT COUNT(*) FROM action_dispatches d JOIN actions a ON a.action_id=d.action_id WHERE a.action_type='terminal.send_input')",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!((actions, dispatches), (1, 1));
    let input_seq: i64 = conn
        .query_row(
            "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
            [original.aggregate_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        input_seq, 0,
        "Unknown never promotes a native receipt sequence"
    );
    let state:String=conn.query_row("SELECT a.state FROM actions a JOIN broker_intents i ON i.action_id=a.action_id JOIN command_receipts r ON json_extract(i.intent_json,'$.cause.kind')='command_receipt' AND json_extract(i.intent_json,'$.cause.receipt_id')=r.receipt_id WHERE a.action_type='terminal.send_input' AND r.command_id=?1",[original.command_id.to_string()],|row|row.get(0)).unwrap();
    assert_eq!(state, "outcome_unknown");
    let journal = read_native_journal(&fixture.host_manifest());
    assert_eq!(journal["entries"].as_object().unwrap().len(), 1);
    assert_eq!(
        journal["entries"]["1"]["entry_state"]["state"],
        "outcome_unknown"
    );
    assert!(
        journal["entries"]["1"]["entry_state"]
            .get("receipt_id")
            .is_none()
    );
}

#[tokio::test]
async fn t_f011_005_c5_fixture_uses_real_configured_scope_and_native_pty() {
    let (mut fixture, _effect, _notification, marker) = prepare_c5_authority().await;
    let acquired = start_c5_terminal(&mut fixture).await;
    let manifest = fixture.host_manifest();
    let host = host_registration(&fixture, &manifest);
    assert!(matches!(
        unleash_platform::verify_process_identity(&host).unwrap(),
        unleash_platform::ProcessVerification::Match
    ));
    assert!(manifest.pid.is_some_and(|pid| pid > 0));
    assert!(manifest.allocation_context_digest.is_some());
    assert!(manifest.effective_environment_digest.is_some());
    assert_eq!(acquired["input_seq"], 0);
    assert!(!marker.exists(), "C5 has not been reached without an input");
    assert!(
        read_native_journal(&manifest)["entries"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_pty_probe_output(&mut fixture, &acquired).await;
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n"
    );
    assert!(!fixture.project_root.path().join("worker-effects").exists());
}

#[tokio::test]
async fn t_f011_005_real_daemon_c5_preserves_the_same_pty_and_never_repeats_input() {
    let (mut fixture, effect, notification, marker) = prepare_c5_authority().await;
    let acquired = start_c5_terminal(&mut fixture).await;
    let before = fixture.host_manifest();
    let host_before = host_registration(&fixture, &before);
    let request = input_command(&fixture, &acquired);
    let receipt = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    wait_signal(&effect, EFFECT_SIGNAL).await;
    wait_signal(&notification, MARKER_SIGNAL).await;
    assert_eq!(std::fs::read(&marker).unwrap(), MARKER_SIGNAL);
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-effects")).unwrap(),
        b"effect\n"
    );
    assert_eq!(
        read_native_journal(&before)["entries"]["1"]["entry_state"]["state"],
        "write_dispatched"
    );
    fixture.restart_daemon_only().await;
    transfer_after_c5(&mut fixture, &acquired).await;
    let after = fixture.host_manifest();
    assert_eq!(after.host_id, before.host_id);
    assert_eq!(after.process_run_uuid, before.process_run_uuid);
    assert_eq!(
        (
            after.pid,
            after.pgid,
            after.owner_uid,
            after.boot_identity.clone(),
            after.start_identity.clone(),
            after.identity_backend
        ),
        (
            before.pid,
            before.pgid,
            before.owner_uid,
            before.boot_identity.clone(),
            before.start_identity.clone(),
            before.identity_backend
        )
    );
    let host_after = host_registration(&fixture, &after);
    assert_eq!(host_after, host_before, "same observed host OS incarnation");
    assert!(matches!(
        unleash_platform::verify_process_identity(&host_after).unwrap(),
        unleash_platform::ProcessVerification::Match
    ));
    assert!(after.daemon_epoch > before.daemon_epoch);
    assert!(after.highest_fence > before.highest_fence);
    assert_pty_probe_output(&mut fixture, &acquired).await;
    for _ in 0..3 {
        assert_eq!(
            fixture.client.send_command(&request).await.unwrap(),
            receipt,
            "exact original wire replay yields the original intent receipt before any new effect attempt"
        );
    }
    assert_one_unknown_input(&fixture, &request);
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-effects")).unwrap(),
        b"effect\n"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n"
    );
}

#[test]
#[ignore = "owned C5 PTY worker selected by exact argv only"]
fn terminal_c5_worker_fixture() {
    let mut starts = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("worker-starts")
        .unwrap();
    starts.write_all(b"started\n").unwrap();
    starts.sync_all().unwrap();
    let probe = unleash_platform::bind_private_unix_listener(
        &std::env::current_dir().unwrap().join("p.sock"),
    )
    .unwrap();
    std::thread::spawn(move || {
        for stream in probe.incoming() {
            let mut stream = stream.unwrap();
            let mut nonce = [0; 16];
            stream.read_exact(&mut nonce).unwrap();
            let mut output = std::io::stdout().lock();
            output.write_all(b"c5-post-restart:").unwrap();
            output.write_all(&nonce).unwrap();
            output.write_all(b"\n").unwrap();
            output.flush().unwrap();
            stream.write_all(b"PRINTED\n").unwrap();
        }
    });
    writeln!(std::io::stdout(), "live-terminal-ready").unwrap();
    std::io::stdout().flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        if line.unwrap() == "apply-effect" {
            let mut effects = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("worker-effects")
                .unwrap();
            effects.write_all(b"effect\n").unwrap();
            effects.sync_all().unwrap();
            let mut signal = std::os::unix::net::UnixStream::connect("e.sock").unwrap();
            signal.write_all(EFFECT_SIGNAL).unwrap();
            signal.flush().unwrap();
        }
    }
}
