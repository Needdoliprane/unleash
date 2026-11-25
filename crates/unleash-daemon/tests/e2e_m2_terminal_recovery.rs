// T-F011-003 / AC-F011-003 / REQ-TERM-003
//! Product IPC regression: real `unleashd`, authenticated client and local fixtures.
//!
//! Nominal acquisition is qualified separately from daemon crash and input recovery.
//! Configuration must be accepted through the actual authenticated product entry.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Assertions and exclusively owned local fixture processes"
)]

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(20);

struct FixtureDaemon {
    child: Child,
    ready: mpsc::Receiver<std::io::Result<String>>,
    data_root: PathBuf,
    cleanup_hosts: bool,
}

impl FixtureDaemon {
    fn spawn_driver(
        entry: &str,
        data: &Path,
        variables: &[(&str, &Path)],
        ready_prefix: &'static str,
    ) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", entry, "--nocapture"])
            .env_clear()
            .env("HOME", std::env::var_os("HOME").unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in variables {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, ready) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut sender = Some(sender);
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if let Some((_, json)) = line.split_once(ready_prefix)
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

    fn spawn(data: &Path, rendezvous: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
            .args(["--data-root"])
            .arg(data)
            .args(["--rendezvous-dir"])
            .arg(rendezvous)
            .args(["--foreground", "--ready-json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the real fixture daemon");
        let stdout = child.stdout.take().expect("fixture daemon stdout");
        let (tx, ready) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        Self {
            child,
            ready,
            data_root: data.to_path_buf(),
            cleanup_hosts: true,
        }
    }

    fn readiness(&self) -> Value {
        let line = self
            .ready
            .recv_timeout(DEADLINE)
            .expect("daemon readiness deadline")
            .expect("read daemon readiness");
        let ready: Value = serde_json::from_str(&line).expect("daemon readiness JSON");
        assert_eq!(ready.get("status").and_then(Value::as_str), Some("ready"));
        ready
    }

    fn kill_for_restart(&mut self) {
        self.child.kill().expect("SIGKILL the owned daemon only");
        self.child.wait().expect("reap the owned daemon");
        self.cleanup_hosts = false;
    }
}

impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        // Close the owner's C5 controller pipe before contacting a host whose
        // input gate may be waiting for that EOF. Keep private runtime files
        // available for the authenticated host/child cleanup below.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + DEADLINE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => return,
            }
        }
        // Only this fixture's private hosts, authenticated by their actual capability.
        // Never signal a guessed/reused PID as cleanup.
        if self.cleanup_hosts
            && let Ok(entries) = std::fs::read_dir(self.data_root.join("process-hosts"))
        {
            for entry in entries.flatten() {
                let _ = stop_fixture_host(&entry.path());
            }
        }
    }
}

#[path = "support/terminal_reattach_scenarios.rs"]
mod reattach_scenarios;

fn stop_fixture_host(runtime: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use unleash_process_host::*;
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        manifest.process_run_uuid,
        manifest.authority_generation,
        manifest.daemon_epoch,
        manifest.highest_fence,
    )?;
    let digest = manifest.command_digest;
    let header = client.make_header(
        CommandId::from_u128(0xf011_ff01),
        manifest.action_id,
        digest,
    );
    let reply = client.call(&HostRequest::Handshake {
        header,
        current_secret: secret.expose_secret().into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?;
    if !matches!(reply, HostResponse::HandshakeAck { .. }) {
        return Err("fixture host not authenticated".into());
    }
    let header = client.make_header(
        CommandId::from_u128(0xf011_ff02),
        manifest.action_id,
        digest,
    );
    let _ = client.call(&HostRequest::Stop {
        header,
        secret: secret.expose_secret().into(),
        escalate_to_sigkill: true,
        known_descendant_pids: vec![],
    })?;
    let header = client.make_header(
        CommandId::from_u128(0xf011_ff03),
        manifest.action_id,
        digest,
    );
    let _ = client.call(&HostRequest::Shutdown {
        header,
        secret: secret.expose_secret().into(),
    })?;
    Ok(())
}

fn object(value: Value) -> Map<String, Value> {
    let Value::Object(object) = value else {
        panic!("fixture payload object expected");
    };
    object
}

fn terminal_diagnostic(data: &Path) -> String {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        data.join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return "diagnostic unavailable".into();
    };
    let mut result = Vec::new();
    for (label, sql) in [
        (
            "allocations",
            "SELECT group_concat(state) FROM terminal_allocations",
        ),
        ("actions", "SELECT group_concat(state) FROM actions"),
        (
            "dispatches",
            "SELECT CAST(COUNT(*) AS TEXT) FROM broker_dispatches",
        ),
        (
            "phases",
            "SELECT group_concat(phase_key) FROM broker_phase_records",
        ),
        ("processes", "SELECT CAST(COUNT(*) AS TEXT) FROM processes"),
        (
            "published_observation",
            "SELECT CAST(EXISTS(SELECT 1 FROM object_references WHERE owner_kind='terminal_launch_observation' AND state='live') AS TEXT)",
        ),
        (
            "sessions",
            "SELECT CAST(COUNT(*) AS TEXT) FROM terminal_sessions",
        ),
    ] {
        let value = conn
            .query_row(sql, [], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten();
        result.push(format!("{label}={}", value.as_deref().unwrap_or("none")));
    }
    if let Ok(entries) = std::fs::read_dir(data.join("process-hosts")) {
        for entry in entries.flatten() {
            use unleash_process_host::*;
            let runtime = entry.path();
            let stage = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))
                .map(|m| match m.launch_state {
                    HostLaunchState::Prepared => "prepared",
                    HostLaunchState::Running => "running",
                    HostLaunchState::Exited { .. } => "exited",
                    HostLaunchState::LaunchOutcomeUnknown => "unknown",
                    HostLaunchState::Lost => "lost",
                })
                .unwrap_or("no_manifest");
            result.push(format!(
                "host_stage={stage} bootstrap={} socket={}",
                runtime.join("bootstrap.json").is_file(),
                runtime.join(HOST_SOCKET_FILENAME).exists()
            ));
        }
    }
    result.join(" ")
}

fn command(
    authority_generation: AuthorityGeneration,
    id: u128,
    project_id: ProjectId,
    aggregate_id: Uuid,
    revision: u64,
    name: &str,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation,
        actor_ref: None,
        project_id: Some(project_id),
        aggregate_id,
        expected_revision: Revision::new(revision),
        command_type: name.into(),
        payload: object(payload),
    }
}

fn terminal_configuration() -> Value {
    json!({"schema_version":1,"profiles":[{"profile_id":"terminal-test","version":1,
        "variables":{"LANG":"C","TERM":"xterm"}}],
        "max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000})
}

/// The packaged daemon launches `unleash-process-host` next to its own executable.
/// A narrow `cargo test -p unleash-daemon` does not build that other package: say so
/// instead of reporting an unknown launch outcome at the readiness deadline.
fn require_prebuilt_native_host() {
    let host = Path::new(env!("CARGO_BIN_EXE_unleashd"))
        .parent()
        .unwrap()
        .join("unleash-process-host");
    assert!(
        host.is_file(),
        "the exact prebuilt native host is required: {}",
        host.display()
    );
}

#[tokio::test]
async fn t_f011_001_configuration_is_durable_idempotent_and_legacy_compatible() {
    let data = local_test_dir("f011-settings-data").unwrap();
    let rdv = local_test_dir("f011-settings-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = FixtureDaemon::spawn(data.path(), rdv.path());
    daemon.readiness();
    let mut client = UnleashClient::connect(ClientConfig::new(
        rdv.path().join("daemon.sock"),
        ClientKind::Cli,
        "f011-settings",
    ))
    .await
    .unwrap();
    let authority = client.hello().authority_generation;
    let mut configure = command(
        authority,
        0xf011_0100,
        ProjectId::from_u128(1),
        authority.as_uuid(),
        0,
        "SetSettings",
        json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":terminal_configuration()}}),
    );
    configure.project_id = None;
    let first = client
        .send_command(&configure)
        .await
        .expect("persist actual terminal configuration");
    let replay = client
        .send_command(&configure)
        .await
        .expect("same command returns durable receipt");
    assert_eq!(first.receipt_id, replay.receipt_id);
    let mut legacy = configure.clone();
    legacy.command_id = CommandId::from_u128(0xf011_0101);
    legacy.expected_revision = Revision::new(1);
    legacy.payload = object(
        json!({"scope_kind":"user","scope_id":"default","patch":{"confirm_retention_shortening":false}}),
    );
    client
        .send_command(&legacy)
        .await
        .expect("legacy patch preserves terminal configuration");
    let mut conflicting = configure.clone();
    conflicting.command_id = CommandId::from_u128(0xf011_0102);
    conflicting.expected_revision = Revision::new(2);
    let mut changed = terminal_configuration();
    *changed.pointer_mut("/profiles/0/variables/LANG").unwrap() = json!("C.UTF-8");
    conflicting.payload = object(
        json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":changed}}),
    );
    assert!(
        client.send_command(&conflicting).await.is_err(),
        "version cannot acquire another digest"
    );
    for bad in [
        Value::Null,
        json!({"schema_version":1,"profiles":[],"max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000}),
    ] {
        conflicting.command_id = CommandId::from_u128(0xf011_0103);
        conflicting.payload = object(
            json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":bad}}),
        );
        assert!(client.send_command(&conflicting).await.is_err());
    }
    drop(client);
    drop(daemon);
    let daemon = FixtureDaemon::spawn(data.path(), rdv.path());
    daemon.readiness();
    let mut client = UnleashClient::connect(ClientConfig::new(
        rdv.path().join("daemon.sock"),
        ClientKind::Cli,
        "f011-settings-restart",
    ))
    .await
    .unwrap();
    let snapshot = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_0104),
            query_type: "ReadSettings".into(),
            project_id: None,
            payload: object(json!({"scope_kind":"user","scope_id":"default"})),
        })
        .await
        .unwrap();
    assert_eq!(
        snapshot.result.pointer("/extra_typed/terminal_runtime"),
        Some(&terminal_configuration())
    );
}

async fn configure_nominal_fixture(
    client: &mut UnleashClient,
    project: ProjectId,
    mission: Uuid,
    project_path: &str,
) {
    let authority = client.hello().authority_generation;
    let setup = [
        command(
            authority,
            0xf011_0011,
            project,
            project.as_uuid(),
            0,
            "CreateProject",
            json!({"name":"F011 fixture", "weight":1}),
        ),
        command(
            authority,
            0xf011_0012,
            project,
            project.as_uuid(),
            1,
            "ReviseProject",
            json!({"name":"F011 fixture", "description":"", "weight":1,
                "primary_root_path":project_path,
                "sources":[{"kind":"directory", "path_or_uri":project_path,
                            "label":"fixture", "read_only":false}],
                "repositories":[], "settings_json":{}}),
        ),
        command(
            authority,
            0xf011_0013,
            project,
            mission,
            0,
            "AssignMission",
            json!({}),
        ),
    ];
    for cmd in &setup {
        let receipt = tokio::time::timeout(DEADLINE, client.send_command(cmd))
            .await
            .expect("setup command deadline")
            .expect("setup command accepted");
        assert_eq!(receipt.state, ReceiptState::Accepted);
    }
    let project_snapshot = tokio::time::timeout(
        DEADLINE,
        client.query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_0014),
            query_type: "GetProject".into(),
            project_id: Some(project),
            payload: Map::new(),
        }),
    )
    .await
    .expect("project query deadline")
    .expect("read created project");
    assert_eq!(
        project_snapshot
            .result
            .get("primary_root_path")
            .and_then(Value::as_str),
        Some(project_path)
    );

    let mut configuration = command(
        authority,
        0xf011_0016,
        project,
        authority.as_uuid(),
        0,
        "SetSettings",
        json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":{
            "schema_version":1,"profiles":[{"profile_id":"terminal-test","version":1,
                "variables":{"LANG":"C","TERM":"xterm"}}],
            "max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000
        }}}),
    );
    configuration.project_id = None;
    let configured = tokio::time::timeout(DEADLINE, client.send_command(&configuration))
        .await
        .expect("configuration deadline")
        .expect("authenticated terminal configuration");
    assert_eq!(configured.state, ReceiptState::Accepted);
}

#[tokio::test]
async fn t_f011_003_real_daemon_accepts_authenticated_terminal_acquisition() {
    let data = local_test_dir("f011-data").unwrap();
    let rendezvous = local_test_dir("f011-rdv").unwrap();
    let project_root = local_test_dir("f011-project").unwrap();
    std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = FixtureDaemon::spawn(data.path(), rendezvous.path());
    let ready = daemon.readiness();
    let socket: PathBuf = rendezvous.path().join("daemon.sock");
    let mut client = tokio::time::timeout(
        DEADLINE,
        UnleashClient::connect(ClientConfig::new(socket, ClientKind::Cli, "f011-e2e")),
    )
    .await
    .expect("IPC connect deadline")
    .expect("authenticated IPC hello");
    let authority = client.hello().authority_generation;
    assert_eq!(
        ready.get("authority_generation"),
        Some(&json!(authority.to_string()))
    );

    let project = ProjectId::from_u128(0xf011_0001);
    let mission = Uuid::from_u128(0xf011_0002);
    let canonical_project_root = project_root.path().canonicalize().unwrap();
    let project_path = canonical_project_root.to_str().unwrap();
    configure_nominal_fixture(&mut client, project, mission, project_path).await;
    require_prebuilt_native_host();

    let acquire = command(
        authority,
        0xf011_0015,
        project,
        Uuid::from_u128(0xf011_0003),
        0,
        "AcquireTerminal",
        json!({"mission_id":mission, "invocation_id":null,
            "purpose_key":"fixture-terminal", "reuse_policy":"reuse_compatible",
            "scope_id":project.to_string(), "scope_revision":2, "worktree_id":null,
            "cwd":".", "program":"/usr/bin/printf", "arguments":["terminal-ready\n"],
            "environment_profile_id":"terminal-test", "pty":{"rows":24,"cols":80},
            "readiness":{"kind":"process_started","timeout_ms":10000}}),
    );
    let receipt = tokio::time::timeout(DEADLINE, client.send_command(&acquire))
        .await
        .expect("AcquireTerminal deadline")
        .expect("T-F011-003: authenticated AcquireTerminal must be supported");
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.command_id, acquire.command_id);
    assert_eq!(receipt.aggregate_id, acquire.aggregate_id);
    wait_for_nominal_terminal_output(&mut client, project, acquire.aggregate_id, data.path()).await;
}

async fn wait_for_nominal_terminal_output(
    client: &mut UnleashClient,
    project: ProjectId,
    terminal: Uuid,
    data: &Path,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let queried = client
            .query(&QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xf011_0017),
                query_type: "ReadTerminalLog".into(),
                project_id: Some(project),
                payload: object(json!({"terminal_id":terminal,"after_offset":0,"max_bytes":65536})),
            })
            .await;
        let output = match queried {
            Ok(output) => output,
            Err(unleash_client::ClientError::Remote(error))
                if error.code == unleash_protocol::ErrorCode::ResourceBusy
                    && error.retry_class == unleash_protocol::RetryClass::AfterRefresh =>
            {
                assert!(
                    Instant::now() < deadline,
                    "durable launch observation deadline: {}",
                    terminal_diagnostic(data)
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(error) => panic!(
                "real terminal log query failed: {error:?}; {}",
                terminal_diagnostic(data)
            ),
        };
        if output
            .result
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains("terminal-ready"))
        {
            assert!(
                output
                    .result
                    .get("process_run_uuid")
                    .and_then(Value::as_str)
                    .is_some()
            );
            assert!(
                output
                    .result
                    .get("pid")
                    .and_then(Value::as_u64)
                    .is_some_and(|p| p > 0)
            );
            break;
        }
        assert!(Instant::now() < deadline, "observed PTY output deadline");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
