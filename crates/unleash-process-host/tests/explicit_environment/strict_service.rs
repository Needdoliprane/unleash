// SPDX-License-Identifier: Apache-2.0
//! Native F012 component evidence on macOS with loopback-only fixture traffic.

#![expect(
    clippy::panic,
    reason = "Native test failures report fixture diagnostics"
)]

use super::{OwnedHost, SECRET, private_runtime, spawn, spec};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unleash_domain::{CommandId, Sha256Digest};
use unleash_platform::{
    TcpListenerError, inspect_process_identity, observe_owned_tcp_connection,
    observe_owned_tcp_listener, verify_owned_tcp_connection, write_private_file_atomic,
};
use unleash_process_host::{
    EnvironmentMode, HOST_MANIFEST_FILENAME, HOST_READY_PREFIX, HOST_SOCKET_FILENAME,
    HostBootstrapSpec, HostClient, HostManifest, HostRequest, HostResponse, IsolationMode,
    IsolationProfileKind, ResourceKind, StrictIsolationSpec, effective_environment_digest_v1,
    explicit_environment_v1,
};
use unleash_testkit::local_test_dir;

const WAIT: Duration = Duration::from_secs(8);
const IO: Duration = Duration::from_secs(2);

#[derive(serde::Serialize, serde::Deserialize)]
struct FixtureRequest {
    egress_witness: SocketAddr,
    protected_sentinel: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FixtureObservation {
    endpoint: SocketAddr,
    environment_digest: Sha256Digest,
    environment_keys: Vec<String>,
    egress_denied: bool,
    protected_read_denied: bool,
}

#[test]
fn child() {
    if !std::env::args().any(|arg| arg == "--exact") {
        return;
    }
    let mut environment: BTreeMap<_, _> = std::env::vars().collect();
    // CoreFoundation initializes its own encoding variable during process startup.
    // Every launcher-supplied entry remains subject to exact comparison.
    if let Some(encoding) = environment.remove("__CF_USER_TEXT_ENCODING") {
        assert_ne!(encoding, "synthetic-parent-text-encoding");
    }
    let request: FixtureRequest =
        serde_json::from_slice(&std::fs::read("request.json").unwrap()).unwrap();
    assert!(request.egress_witness.ip().is_loopback());
    let egress_denied = TcpStream::connect_timeout(&request.egress_witness, IO)
        .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    let protected_read_denied = std::fs::read(request.protected_sentinel)
        .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let observation = FixtureObservation {
        endpoint: listener.local_addr().unwrap(),
        environment_digest: effective_environment_digest_v1(&environment).unwrap(),
        environment_keys: environment.keys().cloned().collect(),
        egress_denied,
        protected_read_denied,
    };
    let tmp = PathBuf::from(std::env::var_os("TMPDIR").unwrap());
    write_private_file_atomic(
        &tmp.join("observation.json"),
        &serde_json::to_vec(&observation).unwrap(),
    )
    .unwrap();
    let mut stream = until(|| match listener.accept() {
        Ok((stream, _)) => Some(stream),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(e) => panic!("fixture accept: {e}"),
    });
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(IO)).unwrap();
    stream.set_write_timeout(Some(IO)).unwrap();
    let mut input = [0; 4];
    stream.read_exact(&mut input).unwrap();
    assert_eq!(&input, b"ping");
    stream.write_all(b"pong").unwrap();
}

fn until<T>(mut observation: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(result) = observation() {
            return result;
        }
        assert!(Instant::now() < deadline, "fixture observation timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn strict_spec(root: &Path) -> HostBootstrapSpec {
    let runtime = private_runtime(root);
    let work = root.join("work");
    unleash_platform::ensure_private_rendezvous_dir(&work).unwrap();
    let data = root.join("data");
    let sockets = root.join("sockets");
    for path in [&data, &sockets] {
        std::fs::create_dir(path).unwrap();
    }
    let mut launch = spec(&runtime);
    launch.environment_mode = Some(EnvironmentMode::StrictServiceV1);
    launch.cwd = work.canonicalize().unwrap();
    launch.arguments = vec![
        "strict_service::child".into(),
        "--exact".into(),
        "--nocapture".into(),
    ];
    launch.terminal_id = None;
    launch.resource_kind = ResourceKind::Service;
    launch.isolation_mode = IsolationMode::Strict;
    launch.isolation_profile_kind = Some(IsolationProfileKind::WorkspaceCommand);
    launch.explicit_env = BTreeMap::from([("LANG".into(), "C".into())]);
    let env = explicit_environment_v1(&launch.explicit_env, &launch.cwd.join("child-tmp")).unwrap();
    launch.expected_effective_environment_digest =
        Some(effective_environment_digest_v1(&env).unwrap());
    launch.strict_spec = Some(StrictIsolationSpec {
        profile_kind: IsolationProfileKind::WorkspaceCommand,
        assigned_write_roots: vec![launch.cwd.clone()],
        data_root: data,
        socket_dir: sockets,
        keychain_paths: vec![],
        declared_credential_paths: vec![],
        egress_proxy_addr: None,
        extra_loopback_control_ports: vec![],
        profile_output_dir: runtime,
    });
    launch
}

fn await_readiness(host: &mut OwnedHost) {
    let stdout = host.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let ready = rx.recv_timeout(WAIT).unwrap().unwrap();
    if !ready.starts_with(HOST_READY_PREFIX) {
        let mut details = String::new();
        host.0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut details)
            .unwrap();
        panic!("native host readiness refused: {details}");
    }
}

#[test]
fn t_f012_015_service_strict_accepts_owned_loopback_fd_and_denies_egress() {
    let root = local_test_dir("strict-service").unwrap();
    let spec = strict_spec(root.path());
    let witness = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    witness.set_nonblocking(true).unwrap();
    let protected_sentinel = spec.runtime_dir.join("protected-sentinel");
    write_private_file_atomic(&protected_sentinel, b"fixture-only").unwrap();
    std::fs::write(
        spec.cwd.join("request.json"),
        serde_json::to_vec(&FixtureRequest {
            egress_witness: witness.local_addr().unwrap(),
            protected_sentinel,
        })
        .unwrap(),
    )
    .unwrap();
    let mut host = spawn(&spec);
    await_readiness(&mut host);
    let manifest =
        HostManifest::load_private(&spec.runtime_dir.join(HOST_MANIFEST_FILENAME)).unwrap();
    let tmp = spec.cwd.join("child-tmp");
    let report: FixtureObservation = until(|| match std::fs::read(tmp.join("observation.json")) {
        Ok(raw) => Some(serde_json::from_slice(&raw).unwrap()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("fixture observation: {e}"),
    });
    assert!(report.egress_denied && report.protected_read_denied);
    assert!(
        witness
            .accept()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert_eq!(
        manifest.isolation_backend.as_deref(),
        Some("macos-seatbelt")
    );
    assert_eq!(manifest.isolation_mode, IsolationMode::Strict);
    assert_eq!(manifest.terminal_id, None);
    assert_eq!(
        manifest.effective_environment_digest,
        spec.expected_effective_environment_digest
    );
    assert_eq!(
        manifest.effective_environment_digest,
        Some(report.environment_digest),
        "observed child environment keys: {:?}",
        report.environment_keys
    );
    assert_eq!(
        manifest.allocation_context_digest,
        spec.allocation_context_digest
    );
    let profile = std::fs::read(spec.runtime_dir.join("seatbelt-strict.sb")).unwrap();
    assert_eq!(
        manifest.isolation_profile_digest,
        Some(Sha256Digest::from_bytes(&profile))
    );

    let identity = inspect_process_identity(manifest.pid.unwrap()).unwrap();
    assert_eq!(manifest.pgid, Some(identity.pgid));
    assert_eq!(
        manifest.boot_identity.as_ref(),
        Some(&identity.boot_identity)
    );
    assert_eq!(
        manifest.start_identity.as_ref(),
        Some(&identity.start_identity)
    );
    assert_authenticated_manifest(&spec, &manifest, &identity);
    assert_same_connection(&identity, report.endpoint);
}

fn assert_same_connection(identity: &unleash_platform::OsProcessIdentity, endpoint: SocketAddr) {
    let listener = observe_owned_tcp_listener(identity, endpoint).unwrap();
    let mut stream = TcpStream::connect_timeout(&endpoint, IO).unwrap();
    stream.set_read_timeout(Some(IO)).unwrap();
    stream.set_write_timeout(Some(IO)).unwrap();
    let owned = until(
        || match observe_owned_tcp_connection(identity, &listener, stream.as_fd()) {
            Ok(observed) => Some(observed),
            Err(TcpListenerError::ConnectionNotFound) => None,
            Err(e) => panic!("same-FD expected: {e}"),
        },
    );
    verify_owned_tcp_connection(&owned, stream.as_fd()).unwrap();
    assert_eq!(owned.client_peer_addr(), endpoint);
    stream.write_all(b"ping").unwrap();
    let mut response = [0; 4];
    stream.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"pong");
}

fn assert_authenticated_manifest(
    spec: &HostBootstrapSpec,
    manifest: &HostManifest,
    identity: &unleash_platform::OsProcessIdentity,
) {
    let mut client = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        spec.resource_fence,
    )
    .unwrap();
    let header = client.make_header(
        CommandId::from_u128(0xf012_0151),
        spec.action_id,
        Sha256Digest::from_bytes(b"fixture-handshake"),
    );
    let ack = client
        .call(&HostRequest::Handshake {
            header,
            current_secret: SECRET.into(),
            rotate_to_secret: None,
            require_higher_epoch: false,
        })
        .unwrap();
    let HostResponse::HandshakeAck {
        process_run_uuid,
        authority_generation,
        effective_environment_digest,
        allocation_context_digest,
        os_identity,
        ..
    } = ack
    else {
        panic!("native handshake refused")
    };
    assert_eq!(os_identity, *identity);
    assert_eq!(process_run_uuid, spec.process_run_uuid);
    assert_eq!(authority_generation, spec.authority_generation);
    assert_eq!(
        effective_environment_digest,
        manifest.effective_environment_digest
    );
    assert_eq!(
        allocation_context_digest,
        manifest.allocation_context_digest
    );
}

#[test]
fn t_f012_015_strict_service_refuses_incompatible_launch_before_prepared() {
    type Reconfigure = fn(&mut HostBootstrapSpec);
    let cases: [(&str, Reconfigure); 8] = [
        ("same env key, different value", |spec| {
            spec.explicit_env.insert("LANG".into(), "C.UTF-8".into());
        }),
        ("PTY resource", |spec| {
            spec.resource_kind = ResourceKind::Pty { rows: 24, cols: 80 };
        }),
        ("standard isolation", |spec| {
            spec.isolation_mode = IsolationMode::Standard;
        }),
        ("outbound proxy exception", |spec| {
            spec.strict_spec.as_mut().unwrap().egress_proxy_addr =
                Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 1)));
        }),
        ("host runtime is writable", |spec| {
            spec.strict_spec
                .as_mut()
                .unwrap()
                .assigned_write_roots
                .push(spec.runtime_dir.clone());
        }),
        ("control token outside host runtime", |spec| {
            spec.control_secret_path = spec.cwd.join("control.secret");
        }),
        ("canaries outside host runtime", |spec| {
            let path = spec.cwd.join("canaries.secret");
            unleash_process_host::store_canary_secrets(&path, &["synthetic-canary"]).unwrap();
            spec.canaries_secret_path = Some(path);
        }),
        ("legacy ExplicitV1 refuses Strict", |spec| {
            spec.environment_mode = Some(EnvironmentMode::ExplicitV1);
        }),
    ];
    for (label, reconfigure) in cases {
        let root = local_test_dir("strict-service-refusal").unwrap();
        let mut spec = strict_spec(root.path());
        reconfigure(&mut spec);
        let mut host = spawn(&spec);
        let rejected = until(|| {
            assert!(
                !spec.runtime_dir.join(HOST_MANIFEST_FILENAME).exists(),
                "{label}: host published Prepared"
            );
            host.0.try_wait().unwrap()
        });
        assert!(!rejected.success(), "{label}");
        assert!(
            !spec.runtime_dir.join(HOST_MANIFEST_FILENAME).exists(),
            "{label}"
        );
        assert!(
            !spec.cwd.join("child-tmp/observation.json").exists(),
            "{label}"
        );
    }
}
