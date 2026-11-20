//! Native component only: real Unix frames and owned PTY, host in the test
//! process. No Store observation, lease activation or daemon recovery is claimed.
use super::*;
use crate::{ControlTransferRequest, ControlTransferResponse};
use std::sync::mpsc;

struct LoseFirstTransferAck {
    rotations: AtomicU64,
    waiting: AtomicBool,
    reached: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl HostCutpointObserver for LoseFirstTransferAck {
    fn hit(&self, point: HostInputCutpoint) -> Result<(), ProcessHostError> {
        if point == HostInputCutpoint::TransferAppliedBeforeAck {
            let previous = self.rotations.fetch_add(1, Ordering::SeqCst);
            if previous == 0 {
                self.waiting.store(true, Ordering::SeqCst);
                self.reached.send(()).unwrap();
                let released = self
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
                self.waiting.store(false, Ordering::SeqCst);
                released.map_err(|_| ProcessHostError::TerminalUnavailable {
                    reason: "owned transfer barrier timed out".into(),
                })?;
                return Err(ProcessHostError::TerminalUnavailable {
                    reason: "owned fixture loses the ACK after actual rotation".into(),
                });
            }
        }
        Ok(())
    }
}

struct ReleaseOnDrop(mpsc::SyncSender<()>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn request(client: &HostClient, spec: &HostBootstrapSpec) -> ControlTransferRequest {
    ControlTransferRequest {
        header: client.make_header(
            CommandId::from_u128(0xf011_a001),
            ActionId::from_u128(0xf011_a002),
            Sha256Digest::from_bytes(b"synthetic accepted transfer context"),
        ),
        dispatch_id: unleash_domain::DispatchId::from_u128(0xf011_a003),
        initial_context_digest: spec.allocation_context_digest.unwrap(),
        current_secret: SECRET.into(),
        next_secret: "synthetic-transfer-ack-fixture-next".into(),
    }
}

fn client(spec: &HostBootstrapSpec) -> HostClient {
    HostClient::connect(
        &spec.runtime_dir.join(HOST_SOCKET_FILENAME),
        spec.process_run_uuid,
        spec.authority_generation,
        DaemonEpoch::new(2),
        Fence::new(2),
    )
    .unwrap()
}

#[derive(Clone, Copy)]
enum Inspection {
    Exact,
    CacheAbsent,
    DifferentCapability,
    DifferentNextCapability,
}

fn exercise(inspection: Inspection) {
    let (reached, arrival) = mpsc::sync_channel(1);
    let (release, allow_close) = mpsc::sync_channel(1);
    let observer = Arc::new(LoseFirstTransferAck {
        rotations: AtomicU64::new(0),
        waiting: AtomicBool::new(false),
        reached,
        release: Mutex::new(allow_close),
    });
    let fixture = fixture_with_observer(Some(observer.clone()));
    // Release the response barrier before Fixture::drop ever joins the server.
    let release_on_drop = ReleaseOnDrop(release);
    let original = fixture.server.shared.anchor.expected_identity();
    let initial = HostManifest::load_private(&fixture.server.shared.manifest_path).unwrap();
    assert_eq!(
        (initial.daemon_epoch, initial.highest_fence),
        (fixture.spec.daemon_epoch, fixture.spec.resource_fence)
    );
    let mut first = client(&fixture.spec);
    let transfer = request(&first, &fixture.spec);
    assert!(transfer.header.daemon_epoch > initial.daemon_epoch);
    assert!(transfer.header.resource_fence > initial.highest_fence);
    let before = transfer.clone();
    let close = first.stream.try_clone().unwrap();
    let first_response = std::thread::spawn(move || first.transfer_control(&before));
    arrival.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(observer.rotations.load(Ordering::SeqCst), 1);
    let cached_ack = fixture
        .server
        .shared
        .last_transfer
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .ack
        .clone()
        .unwrap();
    let published = HostManifest::load_private(&fixture.server.shared.manifest_path).unwrap();
    assert_eq!(
        (published.daemon_epoch, published.highest_fence),
        (DaemonEpoch::new(2), Fence::new(2))
    );
    assert_eq!(published.pid, Some(original.pid));
    // Native effect, complete cache and manifest exist, but the first response
    // cannot yet have been sent. Close only this owned client connection.
    close.shutdown(std::net::Shutdown::Both).unwrap();
    drop(close);
    if matches!(inspection, Inspection::CacheAbsent) {
        let _operation = lock_operation(&fixture.server.shared);
        *fixture.server.shared.last_transfer.lock().unwrap() = None;
    }
    let mut inspect = client(&fixture.spec);
    let mut exact = transfer.clone();
    // The same opaque value in both fields cannot authorize a new rotation.
    exact.current_secret = transfer.next_secret.clone();
    if matches!(inspection, Inspection::DifferentCapability) {
        exact.current_secret = "synthetic-divergent-current-capability".into();
        exact.next_secret = exact.current_secret.clone();
    }
    if matches!(inspection, Inspection::DifferentNextCapability) {
        exact.next_secret = "synthetic-divergent-next-capability".into();
    }
    // This completes while the original ACK handler is still at its barrier;
    // therefore the native operation gate has actually been released.
    let reply = inspect.transfer_control(&exact).unwrap();
    assert!(
        observer.waiting.load(Ordering::SeqCst),
        "inspection must finish while the original response remains deterministically blocked"
    );
    assert_response(
        inspection,
        reply,
        &transfer,
        &cached_ack,
        &original,
        &exact,
        &mut inspect,
    );
    assert_native_state(&fixture, &observer, &published, &original);
    drop(inspect);
    drop(release_on_drop);
    assert!(
        first_response.join().unwrap().is_err(),
        "the cut response must never become an ACK"
    );
    wait_for_native_drain(&fixture, &observer);
    assert_native_state(&fixture, &observer, &published, &original);
}

fn assert_response(
    inspection: Inspection,
    observed: ControlTransferResponse,
    transfer: &ControlTransferRequest,
    cached_ack: &crate::ControlTransferAck,
    original: &unleash_platform::OsProcessIdentity,
    exact: &ControlTransferRequest,
    inspect: &mut HostClient,
) {
    let expected_response = match (inspection, observed) {
        (Inspection::Exact, ControlTransferResponse::Ack { ack }) => {
            assert_eq!(
                *ack, *cached_ack,
                "inspection must return the complete original native ACK"
            );
            assert_eq!(ack.header, transfer.header);
            assert_eq!(ack.dispatch_id, transfer.dispatch_id);
            assert_eq!(ack.initial_context_digest, transfer.initial_context_digest);
            assert_eq!(ack.os_identity, *original);
            let again = inspect.transfer_control(exact).unwrap();
            assert!(
                matches!(again,ControlTransferResponse::Ack {ack:ref repeated} if repeated==&ack)
            );
            true
        }
        (Inspection::CacheAbsent, ControlTransferResponse::Refused { error }) => {
            assert_eq!(error.code, unleash_protocol::ErrorCode::InvalidArgument);
            true
        }
        (
            Inspection::DifferentCapability | Inspection::DifferentNextCapability,
            ControlTransferResponse::Refused { error },
        ) => {
            assert_eq!(
                error.code,
                unleash_protocol::ErrorCode::AuthenticationFailed
            );
            true
        }
        _ => false,
    };
    assert!(
        expected_response,
        "inspection did not return its expected exact fact or closed refusal"
    );
}

fn assert_native_state<T>(
    fixture: &Fixture<T>,
    observer: &LoseFirstTransferAck,
    published: &HostManifest,
    original: &unleash_platform::OsProcessIdentity,
) {
    assert_eq!(
        observer.rotations.load(Ordering::SeqCst),
        1,
        "inspection never rotates again"
    );
    assert_eq!(
        fixture.server.shared.auth_generation.load(Ordering::SeqCst),
        2
    );
    let after = HostManifest::load_private(&fixture.server.shared.manifest_path).unwrap();
    assert_eq!(
        unleash_protocol::canonical_sha256(&after).unwrap(),
        unleash_protocol::canonical_sha256(&published).unwrap(),
        "inspection must not mutate the native manifest"
    );
    assert_eq!(fixture.server.shared.anchor.expected_identity(), *original);
}

fn wait_for_native_drain<T>(fixture: &Fixture<T>, observer: &LoseFirstTransferAck) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while fixture
        .server
        .shared
        .active_connections
        .load(Ordering::SeqCst)
        != 0
    {
        assert!(
            std::time::Instant::now() < deadline,
            "owned native handlers must finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!observer.waiting.load(Ordering::SeqCst));
    assert_eq!(
        observer.rotations.load(Ordering::SeqCst),
        1,
        "no second rotation after all native handlers finish"
    );
    assert_eq!(
        fixture.server.shared.auth_generation.load(Ordering::SeqCst),
        2
    );
}

#[test]
fn t_f011_transfer_lost_ack_can_be_inspected_without_rotation() {
    exercise(Inspection::Exact);
}
#[test]
fn t_f011_transfer_lost_ack_cache_absent_is_closed_without_rotation() {
    exercise(Inspection::CacheAbsent);
}
#[test]
fn t_f011_transfer_lost_ack_different_capability_is_closed_without_rotation() {
    exercise(Inspection::DifferentCapability);
}

#[test]
fn t_f011_transfer_lost_ack_different_next_capability_is_closed_without_rotation() {
    exercise(Inspection::DifferentNextCapability);
}
