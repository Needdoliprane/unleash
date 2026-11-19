//! Pending declaration after the ten preparation oracles have been replayed.
//! Uses a real receipt/context; the future claim API alone changes ownership.
#![expect(
    clippy::panic,
    reason = "The exact result variant is the ownership oracle"
)]

use super::{PRINCIPAL, PROFILE, caller, change, configure, connect, counts, policy, prepare};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{ActorId, DaemonEpoch, Fence, LineageFence, Sha256Digest};
use unleash_store::{
    CallerAuthority, CallerRole, ClaimProviderProbeConnectRequest, CutpointObserver,
    NoopCutpointObserver, ProviderProbeConnectClaimOutcome, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore,
};
use unleash_testkit::local_test_dir;

struct LostClaimAck(AtomicBool);

impl CutpointObserver for LostClaimAck {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        if cutpoint == StoreCutpoint::C3AfterProviderProbeConnectClaimCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C3-connect-claim".to_owned(),
                message: "synthetic acknowledgement loss after durable claim".to_owned(),
            });
        }
        Ok(())
    }
}

fn fixture(
    observer: Arc<dyn CutpointObserver>,
) -> (
    UnleashStore,
    impl std::fmt::Debug,
    ClaimProviderProbeConnectRequest,
) {
    let directory = local_test_dir("f012-connect-claim").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = observer;
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal(PRINCIPAL, "user", caller().actor_id.as_str())
        .unwrap();
    store
        .register_principal("connect-other", "user", "user:connect-other")
        .unwrap();
    configure(&store);
    let mandate = store
        .commit_command(policy(&store, 2, 0, change(&store)))
        .unwrap()
        .receipt()
        .receipt_id;
    let envelope = connect(&store, 100, mandate);
    let receipt = store
        .commit_command(prepare(&store, &envelope))
        .unwrap()
        .receipt()
        .clone();
    let record = store
        .inspect_provider_probe_connect_snapshot(unleash_domain::ProviderProbeId::from_uuid(
            envelope.aggregate_id,
        ))
        .unwrap()
        .value
        .unwrap();
    let request = ClaimProviderProbeConnectRequest {
        envelope,
        caller: caller(),
        receipt_id: receipt.receipt_id,
        context_digest: record.context_digest,
        control_fence: LineageFence::new(
            record.context.authority_generation,
            record.context.daemon_epoch,
            Fence::ZERO,
        ),
    };
    (store, directory, request)
}

fn claim_count(store: &UnleashStore) -> i64 {
    store.read_snapshot(|db,_| db.query_row("SELECT COUNT(*) FROM provider_probe_transport_contexts WHERE claim_state='claimed' AND claim_revision=1", [], |row|row.get(0)).map_err(StoreError::from)).unwrap()
}

#[test]
fn t_f012_013_connect_claim_is_fresh_once_and_replay_is_inspection() {
    let (store, _directory, request) = fixture(Arc::new(NoopCutpointObserver));
    let first = store.claim_provider_probe_connect(request.clone()).unwrap();
    let ProviderProbeConnectClaimOutcome::FreshClaim(first) = first else {
        panic!("first writer must own the claim");
    };
    assert_eq!(first.context().receipt_id, request.receipt_id);
    assert_eq!(first.context_digest(), request.context_digest);
    assert_eq!(first.claim_revision(), unleash_domain::Revision::new(1));
    let committed = counts(&store);
    assert_eq!(
        &committed[3..8],
        &[0, 0, 0, 0, 0],
        "claim alone creates no action, ModelCall or reservation"
    );
    let second = store.claim_provider_probe_connect(request).unwrap();
    let ProviderProbeConnectClaimOutcome::AlreadyClaimed(record) = second else {
        panic!("inspection must never mint a second execution token");
    };
    assert_eq!(record.claim.as_ref().unwrap().claim_id, first.claim_id());
    assert_eq!(claim_count(&store), 1);
    assert_eq!(counts(&store), committed);
}

#[test]
fn t_f012_013_connect_lost_claim_ack_never_becomes_a_new_fresh_claim() {
    let lost = Arc::new(LostClaimAck(AtomicBool::new(false)));
    let (store, _directory, request) = fixture(lost.clone());
    lost.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.claim_provider_probe_connect(request.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(
        claim_count(&store),
        1,
        "the first claim really committed before acknowledgement was lost"
    );
    let committed = counts(&store);
    assert!(matches!(
        store.claim_provider_probe_connect(request),
        Ok(ProviderProbeConnectClaimOutcome::AlreadyClaimed(_))
    ));
    assert_eq!(counts(&store), committed);
}

#[test]
fn t_f012_013_connect_two_claimants_get_only_one_execution_token() {
    let (store, _directory, request) = fixture(Arc::new(NoopCutpointObserver));
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(2));
    let launch = |request: ClaimProviderProbeConnectRequest| {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            store.claim_provider_probe_connect(request).unwrap()
        })
    };
    let first = launch(request.clone());
    let second = launch(request);
    let results = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ProviderProbeConnectClaimOutcome::FreshClaim(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ProviderProbeConnectClaimOutcome::AlreadyClaimed(_)))
            .count(),
        1
    );
    assert_eq!(claim_count(&store), 1);
}

#[test]
fn t_f012_013_connect_claim_rejects_other_caller_receipt_context_and_control_fence() {
    let (store, _directory, request) = fixture(Arc::new(NoopCutpointObserver));
    let before = counts(&store);
    let mut other = request.clone();
    other.caller = CallerAuthority {
        actor_id: ActorId::new("user:connect-other").unwrap(),
        role: CallerRole::HumanOperator,
    };
    other.envelope.actor_ref = Some(other.caller.actor_id.to_string());
    assert!(matches!(
        store.claim_provider_probe_connect(other),
        Err(StoreError::NotFound(_))
    ));
    let mut wrong_receipt = request.clone();
    wrong_receipt.receipt_id = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap()
        .receipt_id;
    assert_ne!(
        wrong_receipt.receipt_id, request.receipt_id,
        "wrong receipt is a real configured receipt"
    );
    assert!(matches!(
        store.claim_provider_probe_connect(wrong_receipt),
        Err(StoreError::NotFound(_))
    ));
    let mut wrong_context = request.clone();
    wrong_context.context_digest = Sha256Digest::from_bytes(b"unrelated-connect-context");
    assert!(matches!(
        store.claim_provider_probe_connect(wrong_context),
        Err(StoreError::StaleFence { .. })
    ));
    let mut wrong_epoch = request.clone();
    wrong_epoch.control_fence.epoch =
        DaemonEpoch::new(wrong_epoch.control_fence.epoch.as_u64() + 1);
    assert!(matches!(
        store.claim_provider_probe_connect(wrong_epoch),
        Err(StoreError::StaleFence { .. })
    ));
    let mut resource_is_not_control = request;
    resource_is_not_control.control_fence.fence = Fence::new(1);
    assert!(matches!(
        store.claim_provider_probe_connect(resource_is_not_control),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(claim_count(&store), 0);
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_claim_does_not_rehydrate_after_real_same_lineage_restart() {
    let (store, _directory, request) = fixture(Arc::new(NoopCutpointObserver));
    let authority = request.control_fence.authority_generation;
    let store = store.restart_same_lineage().unwrap();
    let current = store.daemon_state().unwrap();
    assert_eq!(current.authority_generation, authority);
    assert_ne!(current.daemon_epoch, request.control_fence.epoch);
    let before = counts(&store);
    assert!(matches!(
        store.claim_provider_probe_connect(request),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(claim_count(&store), 0);
    assert_eq!(counts(&store), before);
}
