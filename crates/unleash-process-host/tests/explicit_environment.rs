//! T-F011-003: real host and child, explicit environment; allocation digest is synthetic here.
#![expect(clippy::unwrap_used, reason = "Local owned host fixture assertions")]
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use unleash_domain::{
    ActionId, AuthorityGeneration, CommandId, DaemonEpoch, Fence, HostId, ProcessId,
    ProcessRunUuid, Sha256Digest, TerminalId,
};
use unleash_platform::write_private_file_atomic;
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, EnvironmentMode, HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME,
    HostBootstrapSpec, HostClient, HostControlSecret, HostRequest, HostResponse, IsolationMode,
    ResourceKind, effective_environment_digest_v1, explicit_environment_v1, store_control_secret,
};
use unleash_testkit::local_test_dir;

const SECRET: &str = "synthetic-terminal-control-secret";
fn private_runtime(parent: &Path) -> PathBuf {
    let runtime = parent.join("runtime");
    unleash_platform::ensure_private_rendezvous_dir(&runtime).unwrap();
    unleash_platform::validate_rendezvous_dir(&runtime).unwrap();
    runtime.canonicalize().unwrap()
}
struct OwnedHost(Child, HostBootstrapSpec);
impl Drop for OwnedHost {
    fn drop(&mut self) {
        let spec = &self.1;
        if let Ok(mut client) = HostClient::connect(
            &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
            spec.process_run_uuid,
            spec.authority_generation,
            spec.daemon_epoch,
            spec.resource_fence,
        ) {
            let header = client.make_header(
                CommandId::from_u128(0xff01),
                spec.action_id,
                Sha256Digest::from_bytes(b"cleanup-handshake"),
            );
            let _ = client.call(&HostRequest::Handshake {
                header,
                current_secret: SECRET.into(),
                rotate_to_secret: None,
                require_higher_epoch: false,
            });
            let header = client.make_header(
                CommandId::from_u128(0xff02),
                spec.action_id,
                Sha256Digest::from_bytes(b"cleanup-stop"),
            );
            let _ = client.call(&HostRequest::Stop {
                header,
                secret: SECRET.into(),
                escalate_to_sigkill: true,
                known_descendant_pids: vec![],
            });
            let header = client.make_header(
                CommandId::from_u128(0xff03),
                spec.action_id,
                Sha256Digest::from_bytes(b"cleanup-shutdown"),
            );
            let _ = client.call(&HostRequest::Shutdown {
                header,
                secret: SECRET.into(),
            });
        }
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn spec(runtime: &Path) -> HostBootstrapSpec {
    let env = BTreeMap::from([("TERM".into(), "vt100".into())]);
    HostBootstrapSpec {
        environment_mode: Some(EnvironmentMode::ExplicitV1),
        expected_effective_environment_digest: Some(
            effective_environment_digest_v1(
                &explicit_environment_v1(&env, &runtime.join("child-tmp")).unwrap(),
            )
            .unwrap(),
        ),
        allocation_context_digest: Some(Sha256Digest::from_bytes(
            b"synthetic-component-allocation",
        )),
        runtime_dir: runtime.into(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: Some(TerminalId::from_u128(3)),
        process_run_uuid: ProcessRunUuid::from_u128(4),
        authority_generation: AuthorityGeneration::from_u128(5),
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(6),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: runtime.into(),
        program: std::env::current_exe().unwrap().to_str().unwrap().into(),
        arguments: vec![
            "child_environment_fixture".into(),
            "--exact".into(),
            "--nocapture".into(),
        ],
        explicit_env: env,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: runtime.join(CONTROL_SECRET_FILENAME),
        canaries_secret_path: None,
        max_segment_bytes: 1024,
        max_total_log_bytes: 65536,
        max_volatile_bytes: 4096,
    }
}
#[cfg(target_os = "macos")]
#[path = "explicit_environment/strict_service.rs"]
mod strict_service;

fn spawn(spec: &HostBootstrapSpec) -> OwnedHost {
    store_control_secret(
        &spec.control_secret_path,
        &HostControlSecret::new(SECRET).unwrap(),
    )
    .unwrap();
    let bootstrap = spec.runtime_dir.join("bootstrap.json");
    write_private_file_atomic(&bootstrap, &serde_json::to_vec(spec).unwrap()).unwrap();
    OwnedHost(
        Command::new(env!("CARGO_BIN_EXE_unleash-process-host"))
            .arg("--bootstrap")
            .arg(bootstrap)
            .env_clear()
            .env("PATH", "parent-path-must-not-reach-child")
            .env("LANG", "parent-locale-must-not-reach-child")
            .env("LC_ALL", "parent-locale-must-not-reach-child")
            .env("TERM", "parent-terminal-must-not-reach-child")
            .env("__CF_USER_TEXT_ENCODING", "synthetic-parent-text-encoding")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        spec.clone(),
    )
}

// A fixture entry, selected only by the host's exact test-harness argv.
#[test]
fn child_environment_fixture() {
    if !std::env::args().any(|a| a == "--exact") {
        return;
    }
    let tmp = PathBuf::from(std::env::var_os("TMPDIR").expect("managed TMPDIR"));
    std::fs::write(tmp.join("child-ran"), b"actual child entry").unwrap();
    assert!(std::env::var_os("PATH").is_none());
    assert!(std::env::var_os("LANG").is_none());
    assert!(std::env::var_os("LC_ALL").is_none());
    assert_eq!(std::env::var("TERM").unwrap(), "vt100");
    writeln!(std::io::stdout().lock(), "EXPLICIT_ENV_OK").unwrap();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

#[test]
fn t_f011_003_real_explicit_host_drops_inherited_environment_and_attests_applied_digest() {
    let dir = local_test_dir("host-explicit-env").unwrap();
    let runtime = private_runtime(dir.path());
    let spec = spec(&runtime);
    let mut host = spawn(&spec);
    let stdout = host.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .starts_with(unleash_process_host::HOST_READY_PREFIX)
    );
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        spec.resource_fence,
    )
    .unwrap();
    let header = client.make_header(
        CommandId::from_u128(10),
        spec.action_id,
        Sha256Digest::from_bytes(b"handshake"),
    );
    assert!(
        matches!(client.call(&HostRequest::Handshake {header,current_secret:SECRET.into(),rotate_to_secret:None,require_higher_epoch:false}).unwrap(),HostResponse::HandshakeAck {effective_environment_digest,allocation_context_digest,..} if effective_environment_digest==spec.expected_effective_environment_digest && allocation_context_digest==spec.allocation_context_digest)
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let header = client.make_header(
            CommandId::from_u128(11),
            spec.action_id,
            Sha256Digest::from_bytes(b"read"),
        );
        let HostResponse::LogChunk { result } = client
            .call(&HostRequest::ReadLog {
                header,
                secret: SECRET.into(),
                after_offset: 0,
                max_bytes: 65536,
                strict_cursor: true,
            })
            .unwrap()
        else {
            panic!("log response");
        };
        let text = String::from_utf8_lossy(
            &result
                .segments
                .iter()
                .flat_map(|s| s.bytes.iter().copied())
                .collect::<Vec<_>>(),
        )
        .into_owned();
        if text.contains("EXPLICIT_ENV_OK") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child did not confirm exact environment"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(runtime.join("child-tmp/child-ran").is_file());
    let header = client.make_header(
        CommandId::from_u128(12),
        spec.action_id,
        Sha256Digest::from_bytes(b"stop"),
    );
    let _ = client.call(&HostRequest::Stop {
        header,
        secret: SECRET.into(),
        escalate_to_sigkill: true,
        known_descendant_pids: vec![],
    });
    let header = client.make_header(
        CommandId::from_u128(13),
        spec.action_id,
        Sha256Digest::from_bytes(b"shutdown"),
    );
    let _ = client.call(&HostRequest::Shutdown {
        header,
        secret: SECRET.into(),
    });
}

#[test]
fn t_f011_003_divergent_effective_digest_refuses_before_prepared_or_child() {
    let dir = local_test_dir("host-explicit-digest-refusal").unwrap();
    let runtime = private_runtime(dir.path());
    let mut spec = spec(&runtime);
    spec.expected_effective_environment_digest = Some(Sha256Digest::from_bytes(b"wrong digest"));
    let mut host = spawn(&spec);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = host.0.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "invalid environment host did not refuse"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!runtime.join(HOST_MANIFEST_FILENAME).exists());
    assert!(!runtime.join(HOST_SOCKET_FILENAME).exists());
    assert!(!runtime.join("child-tmp/child-ran").exists());
}
