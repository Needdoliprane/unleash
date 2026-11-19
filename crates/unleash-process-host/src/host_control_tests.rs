//! Component regression: actual Unix control socket and owned PTY child.
//! The host runs in this test process; this is not the daemon crash E2E proof.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Local owned process and socket fixture assertions"
)]
use super::*;
use crate::{EnvironmentMode, IsolationMode, OsIsolationBackendProbe, OsProcessInspector};
use unleash_domain::{
    ActionId, CommandId, HostId, ProcessId, Sha256Digest, TerminalId, TimestampMs,
};
use unleash_testkit::local_test_dir;

const SECRET: &str = "synthetic-control-regression-secret";
struct FailAtC5;
impl HostCutpointObserver for FailAtC5 {
    fn hit(&self, point: HostInputCutpoint) -> Result<(), ProcessHostError> {
        if point == HostInputCutpoint::C5AfterPtyWriteBeforeReceipt {
            return Err(ProcessHostError::TerminalUnavailable {
                reason: "synthetic C5 after actual PTY write".into(),
            });
        }
        Ok(())
    }
}

struct Fixture<T> {
    server: Arc<ProcessHostServer>,
    thread: Option<std::thread::JoinHandle<Result<(), ProcessHostError>>>,
    spec: HostBootstrapSpec,
    _directory: T,
}
impl<T> Drop for Fixture<T> {
    fn drop(&mut self) {
        let _ = self.server.shared.anchor.stop_and_observe(true, &[]);
        self.server.request_shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn fixture() -> Fixture<impl std::fmt::Debug> {
    fixture_with_observer(Some(Arc::new(FailAtC5)))
}
fn fixture_with_observer(
    observer: Option<Arc<dyn HostCutpointObserver>>,
) -> Fixture<impl std::fmt::Debug> {
    let directory = local_test_dir("host-unknown").unwrap();
    let runtime = directory.path().join("runtime");
    unleash_platform::ensure_private_rendezvous_dir(&runtime).unwrap();
    let runtime = runtime.canonicalize().unwrap();
    let environment = std::collections::BTreeMap::from([("LANG".into(), "C".into())]);
    let expected = crate::effective_environment_digest_v1(
        &crate::explicit_environment_v1(&environment, &runtime.join("child-tmp")).unwrap(),
    )
    .unwrap();
    let spec = HostBootstrapSpec {
        environment_mode: Some(EnvironmentMode::ExplicitV1),
        expected_effective_environment_digest: Some(expected),
        allocation_context_digest: Some(Sha256Digest::from_bytes(
            b"synthetic host component context",
        )),
        runtime_dir: runtime.clone(),
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
        cwd: runtime.clone(),
        program: "/bin/cat".into(),
        arguments: vec![],
        explicit_env: environment,
        resource_kind: ResourceKind::Pty { rows: 24, cols: 80 },
        isolation_mode: IsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: runtime.join(crate::CONTROL_SECRET_FILENAME),
        canaries_secret_path: None,
        max_segment_bytes: 1024,
        max_total_log_bytes: 65536,
        max_volatile_bytes: 4096,
    };
    store_control_secret(
        &spec.control_secret_path,
        &HostControlSecret::new(SECRET).unwrap(),
    )
    .unwrap();
    let server = Arc::new(
        ProcessHostServer::start(
            &spec,
            &OsIsolationBackendProbe::new(),
            Box::new(OsProcessInspector),
            observer,
        )
        .unwrap(),
    );
    let served = server.clone();
    let thread = std::thread::spawn(move || served.serve_until_shutdown());
    Fixture {
        server,
        thread: Some(thread),
        spec,
        _directory: directory,
    }
}

#[test]
fn t_f011_control_rotation_waits_for_previously_admitted_input() {
    let fixture = fixture_with_observer(None);
    let spec = &fixture.spec;
    let mut old = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        Fence::new(2),
    )
    .unwrap();
    let writer = fixture.server.shared.writer.lock().unwrap();
    let header = old.make_header(
        CommandId::from_u128(20),
        ActionId::from_u128(21),
        Sha256Digest::from_bytes(b"admitted-before-transfer\n"),
    );
    let input = std::thread::spawn(move || {
        old.call(&HostRequest::SendInput {
            header,
            secret: SECRET.into(),
            input_seq: 1,
            payload_bytes: b"admitted-before-transfer\n".to_vec(),
            now_ms: TimestampMs::new(1),
        })
        .unwrap()
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fixture
        .server
        .shared
        .fencing
        .lock()
        .unwrap()
        .current()
        .fence
        != Fence::new(2)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "old input reaches the effect gate"
        );
        std::thread::yield_now();
    }
    let mut successor = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        DaemonEpoch::new(2),
        Fence::new(3),
    )
    .unwrap();
    let header = successor.make_header(
        CommandId::from_u128(22),
        ActionId::from_u128(23),
        Sha256Digest::from_bytes(b"transfer"),
    );
    let (sent, observed) = std::sync::mpsc::channel();
    let transfer = std::thread::spawn(move || {
        let response = successor
            .call(&HostRequest::Handshake {
                header,
                current_secret: SECRET.into(),
                rotate_to_secret: Some("synthetic-successor-control-secret".into()),
                require_higher_epoch: true,
            })
            .unwrap();
        sent.send(response).unwrap();
    });
    // Observe the server actually contend for the operation gate. A delayed
    // client/server thread cannot produce a false green by never arriving.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut premature = None;
    while fixture
        .server
        .shared
        .operation_contentions
        .load(Ordering::SeqCst)
        == 0
    {
        if let Ok(response) = observed.try_recv() {
            premature = Some(response);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "transfer reaches server operation gate"
        );
        std::thread::yield_now();
    }
    // The old PTY writer is deliberately still unavailable. An ACK at this
    // point would let a superseded input write after transfer confirmation.
    let premature = premature.or_else(|| observed.recv_timeout(Duration::from_millis(150)).ok());
    drop(writer);
    assert!(matches!(
        input.join().unwrap(),
        HostResponse::InputReceipt { .. }
    ));
    let response = premature
        .clone()
        .unwrap_or_else(|| observed.recv_timeout(Duration::from_secs(2)).unwrap());
    transfer.join().unwrap();
    assert!(matches!(
        response,
        HostResponse::HandshakeAck {
            secret_rotated: true,
            ..
        }
    ));
    assert!(
        premature.is_none(),
        "rotation ACK preceded the admitted PTY write"
    );
}

#[test]
fn t_f011_control_correlated_lost_ack_replays_without_rotation_or_spawn() {
    use crate::{ControlTransferRequest, ControlTransferResponse};
    let fixture = fixture_with_observer(None);
    let spec = &fixture.spec;
    let mut first = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        DaemonEpoch::new(2),
        Fence::new(2),
    )
    .unwrap();
    let request = ControlTransferRequest {
        header: first.make_header(
            CommandId::from_u128(30),
            ActionId::from_u128(31),
            Sha256Digest::from_bytes(b"accepted-control-context"),
        ),
        dispatch_id: unleash_domain::DispatchId::from_u128(32),
        initial_context_digest: spec.allocation_context_digest.unwrap(),
        current_secret: SECRET.into(),
        next_secret: "synthetic-correlated-successor-secret".into(),
    };
    let original = fixture.server.shared.anchor.expected_identity();
    // Write the real request, then abandon the connection without reading its
    // ACK. The next client must inspect/replay this same transfer identity.
    let bytes = serde_json::to_vec(
        &crate::control_transfer::TransferEnvelope::TransferControlV1(request.clone()),
    )
    .unwrap();
    write_frame(&mut first.stream, &bytes).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fixture.server.shared.auth_generation.load(Ordering::SeqCst) == 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "correlated transfer must reach rotation"
        );
        std::thread::yield_now();
    }
    drop(first);
    let mut retry = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        DaemonEpoch::new(2),
        Fence::new(2),
    )
    .unwrap();
    let mut replay = request.clone();
    replay.current_secret = request.next_secret.clone();
    let ControlTransferResponse::Ack { ack } = retry.transfer_control(&replay).unwrap() else {
        panic!("same committed transfer must return its correlated facts");
    };
    assert_eq!(ack.header, request.header);
    assert_eq!(ack.dispatch_id, request.dispatch_id);
    assert_eq!(ack.os_identity, original);
    assert_eq!(
        fixture.server.shared.auth_generation.load(Ordering::SeqCst),
        2
    );
    let ControlTransferResponse::Ack { ack: again } = retry.transfer_control(&replay).unwrap()
    else {
        panic!("exact ACK replay must remain stable");
    };
    assert_eq!(again, ack);
    replay.header.action_id = ActionId::from_u128(33);
    assert!(matches!(
        retry.transfer_control(&replay).unwrap(),
        ControlTransferResponse::Refused { .. }
    ));
    assert_eq!(
        fixture.server.shared.auth_generation.load(Ordering::SeqCst),
        2
    );
    let manifest =
        HostManifest::load_private(&spec.runtime_dir.join(HOST_MANIFEST_FILENAME)).unwrap();
    assert_eq!(manifest.pid, Some(original.pid));
    assert_eq!(manifest.process_run_uuid, spec.process_run_uuid);
    assert_eq!(manifest.daemon_epoch, DaemonEpoch::new(2));
    assert_eq!(manifest.highest_fence, Fence::new(2));
    let debug = format!("{request:?}");
    assert!(!debug.contains(SECRET) && !debug.contains(&request.next_secret));
}
fn handshake(client: &mut HostClient, id: u128, action: ActionId) -> HostResponse {
    let header = client.make_header(
        CommandId::from_u128(id),
        action,
        Sha256Digest::from_bytes(b"handshake"),
    );
    client
        .call(&HostRequest::Handshake {
            header,
            current_secret: SECRET.into(),
            rotate_to_secret: None,
            require_higher_epoch: false,
        })
        .unwrap()
}

#[test]
fn t_f011_control_repeated_host_observation_keeps_durable_unknown_inputs() {
    let fixture = fixture();
    let spec = &fixture.spec;
    let mut client = HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        spec.resource_fence,
    )
    .unwrap();
    assert!(matches!(
        handshake(&mut client, 10, spec.action_id),
        HostResponse::HandshakeAck { .. }
    ));
    let header = client.make_header(
        CommandId::from_u128(11),
        ActionId::from_u128(7),
        Sha256Digest::from_bytes(b"one-effect\n"),
    );
    let sent = client
        .call(&HostRequest::SendInput {
            header,
            secret: SECRET.into(),
            input_seq: 1,
            payload_bytes: b"one-effect\n".to_vec(),
            now_ms: TimestampMs::new(1),
        })
        .unwrap();
    assert!(
        matches!(sent, HostResponse::Error { .. }),
        "C5 refuses confirmation after actual write"
    );
    let first = handshake(&mut client, 12, spec.action_id);
    assert_eq!(
        fixture.server.shared.inputs.summary().1,
        vec![1],
        "ambiguity is durable in the tracker"
    );
    let second = handshake(&mut client, 13, spec.action_id);
    let header = client.make_header(
        CommandId::from_u128(14),
        spec.action_id,
        Sha256Digest::from_bytes(b"reconcile"),
    );
    let third = client
        .call(&HostRequest::Reconcile {
            header,
            secret: SECRET.into(),
        })
        .unwrap();
    let observed = [first, second, third].map(|response| match response {
        HostResponse::HandshakeAck {
            unknown_input_seqs, ..
        }
        | HostResponse::ReconcileState {
            unknown_input_seqs, ..
        } => unknown_input_seqs,
        _ => panic!("authenticated observation required"),
    });
    assert_eq!(
        observed,
        [vec![1], vec![1], vec![1]],
        "a repeated observation cannot erase an existing ambiguity"
    );
}
