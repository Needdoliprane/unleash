//! Native component: actual socket, owned child and PTY. No Store/daemon claim.
use super::*;
use crate::{
    ControlReacquisitionRequest, ControlReacquisitionResponse, ControlReleaseKey,
    ControlReleaseRequest, ControlReleaseResponse, ControlTransferRequest, HostControlState,
};
use unleash_domain::{DispatchId, LeaseId, Revision};
const NEXT: &str = "synthetic-release-component-replacement";

fn connection(spec: &HostBootstrapSpec, fence: u64) -> HostClient {
    HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        spec.daemon_epoch,
        Fence::new(fence),
    )
    .unwrap()
}
fn request(client: &HostClient, spec: &HostBootstrapSpec) -> ControlReleaseRequest {
    ControlReleaseRequest {
        key: ControlReleaseKey {
            header: client.make_header(
                CommandId::from_u128(0xf011_e001),
                ActionId::from_u128(0xf011_e002),
                Sha256Digest::from_bytes(b"synthetic release context"),
            ),
            dispatch_id: DispatchId::from_u128(0xf011_e003),
            initial_context_digest: spec.allocation_context_digest.unwrap(),
            lease_id: LeaseId::from_u128(0xf011_e004),
            control_revision: Revision::INITIAL,
        },
        current_secret: SECRET.into(),
        next_secret: NEXT.into(),
    }
}
fn reject_input(client: &mut HostClient) {
    let header = client.make_header(
        CommandId::from_u128(0xf011_e101),
        ActionId::from_u128(0xf011_e102),
        Sha256Digest::from_bytes(b"component refused operation"),
    );
    assert!(matches!(
        client
            .call(&HostRequest::SendInput {
                header: header.clone(),
                secret: NEXT.into(),
                input_seq: 1,
                payload_bytes: b"must-not-reach-pty".to_vec(),
                now_ms: TimestampMs::new(1)
            })
            .unwrap(),
        HostResponse::Error { error }
            if error.code == unleash_protocol::ErrorCode::AuthenticationFailed
    ));
}

fn reject_resize(client: &mut HostClient) {
    let header = client.make_header(
        CommandId::from_u128(0xf011_e103),
        ActionId::from_u128(0xf011_e104),
        Sha256Digest::from_bytes(b"component refused resize"),
    );
    assert!(matches!(
        client
            .call(&HostRequest::Resize {
                header,
                secret: NEXT.into(),
                rows: 37,
                cols: 112
            })
            .unwrap(),
        HostResponse::Error { error }
            if error.code == unleash_protocol::ErrorCode::AuthenticationFailed
    ));
}

#[test]
fn release_rotates_old_channels_and_reacquires_same_pty_at_same_epoch() {
    let fixture = fixture_with_observer(None);
    let original = fixture.server.shared.anchor.expected_identity();
    let mut old_input = connection(&fixture.spec, 1);
    let mut old_resize = connection(&fixture.spec, 1);
    for (id, client) in [
        (0xf011_e105, &mut old_input),
        (0xf011_e106, &mut old_resize),
    ] {
        assert!(matches!(
            handshake(client, id, fixture.spec.action_id),
            HostResponse::HandshakeAck { .. }
        ));
    }
    let mut release_client = connection(&fixture.spec, 1);
    let req = request(&release_client, &fixture.spec);
    let ack = match release_client.release_control(&req).unwrap() {
        ControlReleaseResponse::Ack { ack } => Some(ack),
        ControlReleaseResponse::Refused { .. } => None,
    }
    .unwrap();
    assert_eq!(ack.key, req.key);
    reject_input(&mut old_input);
    reject_resize(&mut old_resize);
    reject_input(&mut release_client);
    reject_resize(&mut release_client);
    let mut fresh = connection(&fixture.spec, 1);
    let mut inspect = req.clone();
    inspect.current_secret = NEXT.into();
    inspect.next_secret = NEXT.into();
    let generation = fixture.server.shared.auth_generation.load(Ordering::SeqCst);
    assert_eq!(
        fresh.release_control(&inspect).unwrap(),
        ControlReleaseResponse::Ack { ack: ack.clone() }
    );
    assert_eq!(
        fixture.server.shared.auth_generation.load(Ordering::SeqCst),
        generation
    );
    let handshake = fresh.make_header(
        CommandId::from_u128(0xf011_e110),
        ActionId::from_u128(0xf011_e111),
        Sha256Digest::from_bytes(b"old secret attempt"),
    );
    assert!(matches!(
        fresh
            .call(&HostRequest::Handshake {
                header: handshake,
                current_secret: SECRET.into(),
                rotate_to_secret: None,
                require_higher_epoch: false
            })
            .unwrap(),
        HostResponse::Error { .. }
    ));
    let mut current = connection(&fixture.spec, 2);
    let transfer = ControlTransferRequest {
        header: current.make_header(
            CommandId::from_u128(0xf011_e201),
            ActionId::from_u128(0xf011_e202),
            Sha256Digest::from_bytes(b"synthetic reactivation context"),
        ),
        dispatch_id: DispatchId::from_u128(0xf011_e203),
        initial_context_digest: fixture.spec.allocation_context_digest.unwrap(),
        current_secret: NEXT.into(),
        next_secret: "synthetic-reacquired-component-capability".into(),
    };
    let reactivate = ControlReacquisitionRequest {
        transfer,
        released_by: ack.key.dispatch_id,
    };
    let new_ack = match current.reacquire_released_control(&reactivate).unwrap() {
        ControlReacquisitionResponse::Ack { ack } => Some(ack),
        ControlReacquisitionResponse::Refused { .. } => None,
    }
    .unwrap();
    assert_eq!(new_ack.released_by, ack.key.dispatch_id);
    assert_eq!(fixture.server.shared.anchor.expected_identity(), original);
    assert_eq!(
        OsProcessInspector.verify_identity(&original).unwrap(),
        ProcessVerification::Match
    );
    let manifest = HostManifest::load_private(&fixture.server.shared.manifest_path).unwrap();
    assert!(matches!(
        manifest.control_state,
        Some(HostControlState::Active { .. })
    ));
    assert_eq!(
        (manifest.daemon_epoch, manifest.highest_fence),
        (fixture.spec.daemon_epoch, Fence::new(2))
    );
    assert_eq!(fixture.server.shared.inputs.summary(), (0, vec![]));
}

#[test]
fn failed_final_retirement_manifest_never_leaves_an_inspectable_success_in_ram() {
    let fixture = fixture_with_observer(None);
    let mut client = connection(&fixture.spec, 1);
    let req = request(&client, &fixture.spec);
    let ack = match client.release_control(&req).unwrap() {
        ControlReleaseResponse::Ack { ack } => Some(ack),
        ControlReleaseResponse::Refused { .. } => None,
    }
    .unwrap();
    // Exercise the final-commit helper with a real rename failure on this
    // fixture's private path. No second retirement or PTY effect is performed.
    let path = &fixture.server.shared.manifest_path;
    let retained = path.with_extension("retained");
    std::fs::rename(path, &retained).unwrap();
    std::fs::create_dir(path).unwrap();
    assert!(release::commit_retired(&fixture.server.shared, &ack).is_err());
    std::fs::remove_dir(path).unwrap();
    std::fs::rename(&retained, path).unwrap();
    assert!(matches!(
        fixture.server.shared.manifest.lock().unwrap().control_state,
        Some(HostControlState::Retiring { .. })
    ));
    let mut inspect = req.clone();
    inspect.current_secret = NEXT.into();
    inspect.next_secret = NEXT.into();
    assert!(matches!(
        client.release_control(&inspect).unwrap(),
        ControlReleaseResponse::Refused { .. }
    ));
    reject_input(&mut client);
    reject_resize(&mut client);
    assert_eq!(fixture.server.shared.inputs.summary(), (0, vec![]));
}
