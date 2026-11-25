//! Native receipt and C5 product oracles. The C5 daemon library runs in a real owned subprocess;
//! authentication, `SQLite`, broker, native host binary and PTY remain real.
//! Replays inspect an ambiguous write without using the native `SendInput` helper.

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
        let mut command = Command::new(&self.executable);
        command
            .arg("--bootstrap")
            .arg(bootstrap)
            .env_clear()
            .env("UNLEASH_HOST_CUTPOINT_TARGET", "C5")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        unleash_process_host::configure_setsid_pre_exec(&mut command);
        let mut child = command.spawn()?;
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
                    send_signal(&notify, MARKER_SIGNAL);
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
        Self::spawn_driver(
            FIXTURE_ENTRY,
            data,
            &[
                (ENV_DATA, data),
                (ENV_RENDEZVOUS, rendezvous),
                (ENV_MARKER, marker),
                (ENV_NOTIFY, notify),
            ],
            READY_PREFIX,
        )
    }
}

fn signal_listener(path: &Path) -> tokio::net::UnixListener {
    let listener = unleash_platform::bind_private_unix_listener(path).unwrap();
    listener.set_nonblocking(true).unwrap();
    tokio::net::UnixListener::from_std(listener).unwrap()
}

fn send_signal(path: &Path, message: &[u8]) {
    let mut stream = std::os::unix::net::UnixStream::connect(path).unwrap();
    stream.set_read_timeout(Some(DEADLINE)).unwrap();
    stream.set_write_timeout(Some(DEADLINE)).unwrap();
    stream.write_all(message).unwrap();
    stream.flush().unwrap();
    // Keep the socket connected until the receiver checked kernel peer UID
    // and the entire expected message. macOS can reject peer_cred after EOF.
    let mut ack = [0];
    stream.read_exact(&mut ack).unwrap();
    assert_eq!(ack, [1]);
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
    tokio::io::AsyncWriteExt::write_all(&mut stream, &[1])
        .await
        .unwrap();
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

/// Durable outcome of one terminal action: its state, the policy revision its
/// intent engaged, and the broker authorization outcome once journaled.
#[derive(Debug)]
struct Disposition {
    state: String,
    policy_revision: Option<i64>,
    authorization: Option<Value>,
}

const LAUNCH_DISPOSITION: &str = "SELECT a.state,json_extract(t.context_json,'$.policy_revision'),p.phase_json FROM terminal_allocations t JOIN actions a ON a.action_id=t.action_id LEFT JOIN broker_phase_records p ON p.action_id=a.action_id AND p.phase_key='authorization' WHERE t.terminal_id=?1";
const INPUT_DISPOSITION: &str = "SELECT a.state,json_extract(i.context_json,'$.policy_revision'),p.phase_json FROM terminal_input_intents i JOIN actions a ON a.action_id=i.action_id LEFT JOIN broker_phase_records p ON p.action_id=a.action_id AND p.phase_key='authorization' WHERE i.command_id=?1";

fn disposition(fixture: &LiveFixture, source: &str, key: &str) -> Option<Disposition> {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row(source, [key], |row| {
        let phase: Option<String> = row.get(2)?;
        Ok(Disposition {
            state: row.get(0)?,
            policy_revision: row.get(1)?,
            authorization: phase.and_then(|json| {
                serde_json::from_str::<Value>(&json)
                    .ok()?
                    .pointer("/phase/outcome")
                    .cloned()
            }),
        })
    })
    .ok()
}

/// Waits until one terminal action is durably `observed_success`, then returns its
/// disposition. A conclusive refusal or an unknown outcome fails at once and names
/// the broker decision: such an action never produces the effect a caller awaits.
async fn await_observed(fixture: &LiveFixture, source: &str, key: &str) -> Disposition {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = disposition(fixture, source, key) {
            assert!(
                !matches!(
                    found.state.as_str(),
                    "denied" | "observed_failure" | "outcome_unknown" | "cancelled"
                ),
                "the terminal action ended as '{}': {} ; {}",
                found.state,
                found.authorization.clone().unwrap_or(Value::Null),
                terminal_diagnostic(fixture.data.path())
            );
            if found.state == "observed_success" {
                return found;
            }
        }
        assert!(
            Instant::now() < deadline,
            "terminal action has no observed outcome: {}",
            terminal_diagnostic(fixture.data.path())
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Number of rows of one ledger query, read outside the stopped daemon.
fn ledger_count(fixture: &LiveFixture, sql: &str) -> i64 {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Stops the owned daemon through its actual drain and waits for its exit. Every
/// terminal task has then ended, so a missing effect observed afterwards is final.
async fn stop_and_await_exit(fixture: &mut LiveFixture) {
    let revision = fixture
        .snapshot()
        .await
        .pointer("/daemon/revision")
        .and_then(Value::as_u64)
        .unwrap();
    let stop = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_6206),
        command_id: CommandId::from_u128(0xf011_6206),
        authority_generation: fixture.authority,
        actor_ref: None,
        project_id: None,
        aggregate_id: fixture.authority.as_uuid(),
        expected_revision: Revision::new(revision),
        command_type: "StopDaemon".into(),
        payload: object(json!({"reason":"terminal refusal drain","drain_timeout_ms":10000})),
    };
    assert_eq!(
        fixture.client.send_command(&stop).await.unwrap().state,
        ReceiptState::Accepted
    );
    let deadline = Instant::now() + DEADLINE;
    loop {
        if fixture.daemon.child.try_wait().unwrap().is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "drain did not close the writer and broker tasks"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn start_c5_terminal(fixture: &mut LiveFixture) -> Value {
    let mut request = fixture.allocation(0xf011_4105);
    request.payload.insert(
        "arguments".into(),
        json!(["--ignored", "--exact", WORKER_ENTRY, "--nocapture"]),
    );
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    await_observed(
        fixture,
        LAUNCH_DISPOSITION,
        &request.aggregate_id.to_string(),
    )
    .await;
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
    let host_id = manifest.host_id.to_string();
    let registered = registrations
        .get("executors")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|item| item.get("executor_id").and_then(Value::as_str) == Some(host_id.as_str()))
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
        let conn = rusqlite::Connection::open_with_flags(
            fixture.data.path().join("unleash-authority.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let observations: i64 = conn.query_row(
            "SELECT COUNT(*) FROM terminal_control_intents c \
             JOIN actions a ON a.action_id=c.action_id \
             JOIN action_dispatches d ON d.dispatch_id=c.dispatch_id AND d.action_id=c.action_id \
             JOIN broker_dispatches b ON b.dispatch_id=d.dispatch_id AND b.action_id=c.action_id \
             JOIN object_references r ON r.owner_kind='terminal_control_observation' AND r.owner_id=c.action_id AND r.source_field=c.dispatch_id AND r.object_hash=c.observation_object_hash AND r.state='live' \
             JOIN object_records o ON o.object_hash=r.object_hash AND o.state='live' \
             WHERE c.command_id=?1 AND c.receipt_id=?2 AND c.terminal_id=?3 \
               AND json_extract(c.context_json,'$.process_run_uuid')=?4 \
               AND c.state='observed' AND a.state='observed_success' \
               AND b.authority_generation=c.authority_generation AND b.daemon_epoch=c.daemon_epoch",
            rusqlite::params![request.command_id.to_string(), accepted.receipt_id.to_string(),
                acquired["terminal_id"].as_str().unwrap(), acquired["process_run_uuid"].as_str().unwrap()],
            |row| row.get(0),
        ).unwrap();
        assert!(observations <= 1, "one exact transfer observation");
        if observations == 1 {
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
        matches!(refused,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::OutcomeUnknown)
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
        if response
            .result
            .get("text")
            .and_then(Value::as_str)
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
    assert_eq!(
        journal
            .get("entries")
            .and_then(Value::as_object)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        journal
            .pointer("/entries/1/entry_state/state")
            .and_then(Value::as_str),
        Some("outcome_unknown")
    );
    assert!(
        journal
            .pointer("/entries/1/entry_state")
            .unwrap()
            .get("receipt_id")
            .is_none()
    );
}

async fn wait_known_input(
    fixture: &LiveFixture,
    command: &CommandEnvelope,
    receipt: &unleash_protocol::CommandReceipt,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let conn = rusqlite::Connection::open_with_flags(
            fixture.data.path().join("unleash-authority.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let observations: i64 = conn.query_row(
            "SELECT COUNT(*) FROM terminal_input_intents i \
             JOIN actions a ON a.action_id=i.action_id \
             JOIN action_dispatches d ON d.dispatch_id=i.dispatch_id AND d.action_id=i.action_id \
             JOIN object_references r ON r.owner_kind='terminal_input_observation' AND r.owner_id=i.action_id AND r.source_field=i.dispatch_id AND r.object_hash=i.observation_object_hash AND r.state='live' \
             JOIN object_records o ON o.object_hash=r.object_hash AND o.state='live' \
             WHERE i.command_id=?1 AND i.receipt_id=?2 AND i.terminal_id=?3 \
               AND i.state='observed' AND a.state='observed_success' AND i.renewal='renewed'",
            rusqlite::params![command.command_id.to_string(),receipt.receipt_id.to_string(),command.aggregate_id.to_string()],
            |row| row.get(0),
        ).unwrap();
        assert!(observations <= 1, "one native input observation");
        if observations == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "actual native receipt and object must be durably committed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_confirmed_native_input(journal: &Value, command_id: CommandId) {
    let command_id = command_id.to_string();
    let payload_hash = unleash_domain::Sha256Digest::from_bytes(b"apply-effect\n").to_string();
    assert_eq!(
        journal
            .get("entries")
            .and_then(Value::as_object)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        journal
            .pointer("/entries/1/entry_state/state")
            .and_then(Value::as_str),
        Some("receipt_confirmed")
    );
    assert_eq!(
        journal
            .pointer("/entries/1/command_id")
            .and_then(Value::as_str),
        Some(command_id.as_str())
    );
    assert_eq!(
        journal
            .pointer("/entries/1/payload_hash")
            .and_then(Value::as_str),
        Some(payload_hash.as_str())
    );
}

#[tokio::test]
async fn t_f011_005_real_daemon_commits_native_receipt_before_causal_sequence_one() {
    // This path uses the packaged daemon and its default packaged host launcher.
    let mut fixture = LiveFixture::new(true).await;
    std::fs::set_permissions(
        fixture.project_root.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let effect = signal_listener(&fixture.project_root.path().join("e.sock"));
    let acquired = start_c5_terminal(&mut fixture).await;
    let before = fixture.host_manifest();
    let request = input_command(&fixture, &acquired);
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    wait_signal(&effect, EFFECT_SIGNAL).await;
    wait_known_input(&fixture, &request, &accepted).await;
    let journal = read_native_journal(&before);
    assert_confirmed_native_input(&journal, request.command_id);
    let query = result_query(
        &fixture,
        acquired
            .get("command_id")
            .and_then(Value::as_str)
            .unwrap()
            .parse()
            .unwrap(),
        acquired.get("result_ref").and_then(Value::as_str).unwrap(),
        0xf011_6101,
    );
    let known = fixture.client.query(&query).await.unwrap();
    assert_eq!(
        known.result.get("input_seq").and_then(Value::as_u64),
        Some(1)
    );
    for field in ["terminal_id", "process_run_uuid", "lease_id", "fence"] {
        assert_eq!(
            known.result.get(field).unwrap(),
            acquired.get(field).unwrap(),
            "same observed controller and physical generation"
        );
    }
    assert!(
        known
            .result
            .get("lease_expires_at")
            .and_then(Value::as_i64)
            .unwrap()
            > acquired
                .get("lease_expires_at")
                .and_then(Value::as_i64)
                .unwrap()
    );
    assert!(
        known.watermark.unwrap().sequence.as_u64()
            >= known
                .result
                .get("cursor")
                .and_then(|cursor| cursor.get("sequence"))
                .and_then(Value::as_u64)
                .unwrap()
    );
    for _ in 0..3 {
        assert_eq!(
            fixture.client.send_command(&request).await.unwrap(),
            accepted
        );
    }
    let after = fixture.host_manifest();
    assert_eq!(after.last_confirmed_input_seq, 1);
    assert_eq!(
        read_native_journal(&after),
        journal,
        "command replay never contacts native input again"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-effects")).unwrap(),
        b"effect\n"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n"
    );
}

// T-F017-007 / AC-F017-007 / REQ-SEC-007
#[tokio::test]
async fn t_f017_007_standard_policy_does_not_deny_terminal_acquisition_nor_input() {
    // This path uses the packaged daemon and its default packaged host launcher.
    let mut fixture = LiveFixture::new(true).await;
    std::fs::set_permissions(
        fixture.project_root.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let effect = signal_listener(&fixture.project_root.path().join("e.sock"));
    // The standard policy is committed first: the launch and the input intents
    // engage its revision, so each reserved broker must be bound to that snapshot.
    super::super::resize_scenarios::set_standard_policy(&mut fixture).await;
    let acquired = start_c5_terminal(&mut fixture).await;
    let terminal = acquired["terminal_id"].as_str().unwrap();
    assert_eq!(
        await_observed(&fixture, LAUNCH_DISPOSITION, terminal)
            .await
            .policy_revision,
        Some(1),
        "the launch intent engages the committed standard policy revision"
    );
    let before = fixture.host_manifest();
    let request = input_command(&fixture, &acquired);
    let accepted = fixture
        .client
        .send_command(&request)
        .await
        .expect("a committed standard policy does not refuse SendTerminalInput");
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let input = await_observed(&fixture, INPUT_DISPOSITION, &request.command_id.to_string()).await;
    assert_eq!(
        input.policy_revision,
        Some(1),
        "the input intent engages the committed standard policy revision"
    );
    wait_signal(&effect, EFFECT_SIGNAL).await;
    wait_known_input(&fixture, &request, &accepted).await;
    let journal = read_native_journal(&before);
    assert_confirmed_native_input(&journal, request.command_id);
    let known = fixture
        .client
        .query(&result_query(
            &fixture,
            acquired["command_id"].as_str().unwrap().parse().unwrap(),
            acquired["result_ref"].as_str().unwrap(),
            0xf011_6201,
        ))
        .await
        .unwrap();
    assert_eq!(
        known.result.get("input_seq").and_then(Value::as_u64),
        Some(1)
    );
    for _ in 0..3 {
        assert_eq!(
            fixture.client.send_command(&request).await.unwrap(),
            accepted
        );
    }
    let after = fixture.host_manifest();
    assert_eq!(after.last_confirmed_input_seq, 1);
    assert_eq!(
        read_native_journal(&after),
        journal,
        "command replay never contacts native input again"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-effects")).unwrap(),
        b"effect\n",
        "one native write, one child effect"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n",
        "one physical launch"
    );
}

// T-F017-007 / AC-F017-007 / REQ-SEC-007
#[tokio::test]
async fn t_f017_007_policy_change_after_launch_engagement_never_launches() {
    use super::super::{renewal_fixture, resize_scenarios};
    let root = local_test_dir("launch-policy-boundary").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let armed = root.path().join("armed");
    let barrier = root.path().join("intent.sock");
    let listener = signal_listener(&barrier);
    let mut fixture = renewal_fixture::start_with_intent_cutpoint(&armed, &barrier).await;
    fixture.provision(true).await;
    let mut request = fixture.allocation(0xf011_6301);
    request.payload.insert(
        "arguments".into(),
        json!(["--ignored", "--exact", WORKER_ENTRY, "--nocapture"]),
    );
    unleash_platform::write_private_file_atomic(&armed, b"armed").unwrap();
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let mut stalled = renewal_fixture::wait_after_intent(&listener).await;
    let engaged = disposition(
        &fixture,
        LAUNCH_DISPOSITION,
        &request.aggregate_id.to_string(),
    )
    .unwrap();
    assert_eq!(
        (engaged.state.as_str(), engaged.policy_revision),
        ("authorizing", Some(0)),
        "the launch intent is engaged under the policy in force when it was accepted"
    );
    assert_eq!(
        ledger_count(&fixture, "SELECT COUNT(*) FROM broker_dispatches"),
        0
    );
    resize_scenarios::set_standard_policy(&mut fixture).await;
    tokio::io::AsyncWriteExt::write_all(&mut stalled, &[1])
        .await
        .unwrap();
    stop_and_await_exit(&mut fixture).await;
    assert!(
        !fixture.data.path().join("process-hosts").exists(),
        "no host runtime or bootstrap was ever created"
    );
    assert!(
        !fixture.project_root.path().join("worker-starts").exists(),
        "the child was never launched"
    );
    for table in ["processes", "terminal_sessions", "broker_dispatches"] {
        assert_eq!(
            ledger_count(&fixture, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "no launch effect in {table}"
        );
    }
}

// T-F017-007 / AC-F017-007 / REQ-SEC-007
#[tokio::test]
async fn t_f017_007_policy_change_after_input_engagement_never_writes() {
    use super::super::{renewal_fixture, resize_scenarios};
    let root = local_test_dir("input-policy-boundary").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let armed = root.path().join("armed");
    let barrier = root.path().join("intent.sock");
    let listener = signal_listener(&barrier);
    let mut fixture = renewal_fixture::start_with_intent_cutpoint(&armed, &barrier).await;
    fixture.provision(true).await;
    std::fs::set_permissions(
        fixture.project_root.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let acquired = start_c5_terminal(&mut fixture).await;
    let before = fixture.host_manifest();
    let request = input_command(&fixture, &acquired);
    unleash_platform::write_private_file_atomic(&armed, b"armed").unwrap();
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let mut stalled = renewal_fixture::wait_after_intent(&listener).await;
    let engaged = disposition(
        &fixture,
        INPUT_DISPOSITION,
        &request.command_id.to_string(),
    )
    .unwrap();
    assert_eq!(
        (engaged.state.as_str(), engaged.policy_revision),
        ("authorizing", Some(0)),
        "the input intent is engaged under the policy in force when it was accepted"
    );
    resize_scenarios::set_standard_policy(&mut fixture).await;
    tokio::io::AsyncWriteExt::write_all(&mut stalled, &[1])
        .await
        .unwrap();
    stop_and_await_exit(&mut fixture).await;
    assert!(
        !fixture.project_root.path().join("worker-effects").exists(),
        "the child never received the input"
    );
    assert!(
        read_native_journal(&before)
            .get("entries")
            .and_then(Value::as_object)
            .unwrap()
            .is_empty(),
        "the host journal holds no native input entry"
    );
    assert_eq!(
        ledger_count(
            &fixture,
            "SELECT COUNT(*) FROM action_dispatches d JOIN terminal_input_intents i ON i.action_id=d.action_id"
        ),
        0,
        "the refused input was never dispatched"
    );
}

// T-F017-007 / AC-F017-007 / REQ-SEC-007
#[tokio::test]
async fn t_f017_007_unqualified_strict_policy_never_writes_nor_launches() {
    let mut fixture = LiveFixture::new(true).await;
    std::fs::set_permissions(
        fixture.project_root.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let acquired = start_c5_terminal(&mut fixture).await;
    let before = fixture.host_manifest();
    let mut policy = command(
        fixture.authority,
        0xf011_6320,
        fixture.project,
        fixture.authority.as_uuid(),
        0,
        "SetPolicy",
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"strict",
            "policy_version":"terminal-strict-qualification",
            "reason":"strict policy committed before input and launch"}),
    );
    policy.project_id = None;
    let committed = fixture.client.send_command(&policy).await.unwrap();
    assert_eq!(committed.state, ReceiptState::Accepted);
    assert_eq!(committed.new_revision, Revision::new(1));
    let input = input_command(&fixture, &acquired);
    let accepted = fixture.client.send_command(&input).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let mut launch = fixture.allocation(0xf011_6321);
    launch.aggregate_id = Uuid::from_u128(0xf011_6322);
    launch.payload.insert(
        "purpose_key".into(),
        json!("strict-unqualified-qualification"),
    );
    launch.payload.insert(
        "arguments".into(),
        json!(["--ignored", "--exact", WORKER_ENTRY, "--nocapture"]),
    );
    let accepted = fixture.client.send_command(&launch).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    stop_and_await_exit(&mut fixture).await;
    assert!(
        !fixture.project_root.path().join("worker-effects").exists(),
        "the child never received the input under a strict policy"
    );
    assert!(
        read_native_journal(&before)
            .get("entries")
            .and_then(Value::as_object)
            .unwrap()
            .is_empty(),
        "the host journal holds no native input entry"
    );
    assert_eq!(
        ledger_count(
            &fixture,
            "SELECT COUNT(*) FROM action_dispatches d JOIN terminal_input_intents i ON i.action_id=d.action_id"
        ),
        0,
        "the input was never dispatched under a strict policy"
    );
    assert_eq!(
        ledger_count(&fixture, "SELECT COUNT(*) FROM processes"),
        1,
        "only the terminal acquired before the strict policy was ever launched"
    );
    assert_eq!(fixture.host_manifest().last_confirmed_input_seq, 0);
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
    assert_eq!(acquired.get("input_seq").and_then(Value::as_u64), Some(0));
    assert!(!marker.exists(), "C5 has not been reached without an input");
    assert!(
        read_native_journal(&manifest)
            .get("entries")
            .and_then(Value::as_object)
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
        read_native_journal(&before)
            .pointer("/entries/1/entry_state/state")
            .and_then(Value::as_str),
        Some("write_dispatched")
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
            send_signal(Path::new("e.sock"), EFFECT_SIGNAL);
        }
    }
}
