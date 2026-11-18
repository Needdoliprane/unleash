// T-F011-003 / AC-F011-003 / REQ-TERM-003
//! Product IPC regression: real `unleashd`, authenticated client and local fixtures.
//!
//! The first red exercises acquisition only. The environment-profile reference below is a
//! documented-shaped fixture, not an attested runtime profile: the current product exposes no
//! API to create one. Host recovery and input replay require that missing contract before use.

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
}

impl FixtureDaemon {
    fn spawn(data: &Path, rendezvous: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
            .args(["--data-root"])
            .arg(data)
            .args(["--rendezvous-dir"])
            .arg(rendezvous)
            .args(["--foreground", "--ready-json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the real fixture daemon");
        let stdout = child.stdout.take().expect("fixture daemon stdout");
        let (tx, ready) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        Self { child, ready }
    }

    fn readiness(&self) -> Value {
        let line = self
            .ready
            .recv_timeout(DEADLINE)
            .expect("daemon readiness deadline")
            .expect("read daemon readiness");
        let ready: Value = serde_json::from_str(&line).expect("daemon readiness JSON");
        assert_eq!(ready["status"], "ready");
        ready
    }
}

impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + DEADLINE;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().expect("fixture payload object").clone()
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
    assert_eq!(ready["authority_generation"], authority.to_string());

    let project = ProjectId::from_u128(0xf011_0001);
    let mission = Uuid::from_u128(0xf011_0002);
    let project_path = project_root.path().to_str().unwrap();
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
    assert_eq!(project_snapshot.result["primary_root_path"], project_path);

    // The public F011 example names a profile but the current Store has no profile producer.
    // This value exercises the absent handler; it is not an attestation of a usable environment.
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
            "cwd":".", "program":"/usr/bin/true", "arguments":[],
            "environment_profile_id":"fixture-env", "pty":{"rows":24,"cols":80},
            "readiness":{"kind":"process_started","timeout_ms":10000}}),
    );
    let receipt = tokio::time::timeout(DEADLINE, client.send_command(&acquire))
        .await
        .expect("AcquireTerminal deadline")
        .expect("T-F011-003: authenticated AcquireTerminal must be supported");
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.command_id, acquire.command_id);
    assert_eq!(receipt.aggregate_id, acquire.aggregate_id);
}
