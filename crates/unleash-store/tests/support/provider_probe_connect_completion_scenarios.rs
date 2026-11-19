//! Trusted-observer injection tests for SQLite only. No socket closure is
//! attested by this fixture; the real native producer is tested separately by UDS.

use super::*;
use unleash_store::{
    AdoptProviderConnectObservation, AdoptedProviderConnectObservation, BrokerActionReceipt,
    BrokerObservationCause, CommitProviderConnectObservation, ConnectCertificateValidation,
    ExecutionContinuation, ProviderConnectObservation,
};

struct Dispatched {
    ready: ReadyProbe,
    continuation: ExecutionContinuation,
    decision: BrokerAuthorizationDecision,
    dispatch: DispatchId,
}

fn dispatched(store: &UnleashStore, mandate: ReceiptId, pool: PoolId) -> Dispatched {
    store
        .register_principal(
            unleash_store::CONNECT_OBSERVER_PRINCIPAL,
            "system",
            unleash_store::CONNECT_OBSERVER_PRINCIPAL,
        )
        .unwrap();
    let ready = ready(store, mandate, 0xf012_0400);
    authorizing(store, &ready);
    observe_eligibility(store, &ready, pool);
    let AdmissionOutcome::Admitted(commit) = admit(store, &ready, pool).unwrap() else {
        panic!("actual V008 admission required");
    };
    let continuation = ExecutionContinuation {
        reservation_id: commit.reservation_id,
        unit: commit.unit,
        unit_revision: commit.revision,
        lease_id: commit.lease.lease_id,
        authority_generation: commit.lease.authority_generation,
        daemon_epoch: commit.lease.daemon_epoch,
        fence: commit.lease.fence,
        boot_identity: commit.lease.boot_identity.clone(),
    };
    let decision = authorize(store, &ready);
    let dispatch = DispatchId::from_uuid(store.config().id_generator.new_uuid());
    store
        .commit_provider_probe_connect_phase(
            &ready.claim,
            phase(
                &ready,
                2,
                BrokerPhase::Dispatch {
                    action_id: ready.intent.action_id,
                    intent_digest: ready.intent.intent_digest,
                    decision: decision.clone(),
                    dispatch_id: dispatch,
                    fence: ready.intent.lineage_fence,
                },
            ),
        )
        .unwrap();
    Dispatched {
        ready,
        continuation,
        decision,
        dispatch,
    }
}

fn candidate(store: &UnleashStore, execution: &Dispatched) -> AdoptProviderConnectObservation {
    let document = ProviderConnectObservation {
        schema_version: 1,
        action_id: execution.ready.intent.action_id,
        dispatch_id: execution.dispatch,
        request_digest: execution.ready.record.context.request_digest,
        peer: "127.0.0.1:1234".parse().unwrap(),
        certificate_validation: ConnectCertificateValidation::NotTls,
        revocation_checked: false,
        http_application_bytes_sent: 0,
        observed_at_ms: store.config().clock.now_ms(),
    };
    let bytes = canonicalize_to_vec(&document).unwrap();
    let hash = canonical_sha256(&document).unwrap();
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "provider_probe_connect_observation_preparation".into(),
            owner_id: owner.clone(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, &bytes).unwrap();
    let candidate = store
        .publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "provider_probe_connect_observation_preparation".into(),
            owner_id: owner,
            source_field: "observation".into(),
        })
        .unwrap();
    AdoptProviderConnectObservation {
        continuation: execution.continuation.clone(),
        document,
        content_hash: hash,
        candidate,
    }
}

fn final_request(
    store: &UnleashStore,
    execution: &Dispatched,
    fact: Option<AdoptedProviderConnectObservation>,
) -> CommitProviderConnectObservation {
    CommitProviderConnectObservation {
        continuation: execution.continuation.clone(),
        fact: fact.clone(),
        request: phase(
            &execution.ready,
            3,
            BrokerPhase::Observation {
                action_id: execution.ready.intent.action_id,
                intent_digest: execution.ready.intent.intent_digest,
                receipt: BrokerActionReceipt {
                    action_id: execution.ready.intent.action_id,
                    evaluation_id: execution.ready.evaluation.evaluation_id,
                    dispatch_id: execution.dispatch,
                    decision_ref: execution.decision.decision_ref.clone(),
                    state: ActionState::ObservedSuccess,
                    output_digest: fact
                        .as_ref()
                        .map(AdoptedProviderConnectObservation::content_hash),
                    enforcement_refs: vec![],
                    remote_audit_ref: None,
                    remote_audit_unavailable: true,
                    observed_at_ms: store.config().clock.now_ms(),
                },
                observation_revision: Revision::new(1),
                cause: BrokerObservationCause::Execution,
                reason: None,
            },
        ),
    }
}

fn disposition_count(store: &UnleashStore) -> i64 {
    store.read_snapshot(|conn,_|Ok(conn.query_row("SELECT COUNT(*) FROM evidence WHERE snapshot_ref LIKE 'provider-connect-disposition:v1:%'",[],|r|r.get(0))?)).unwrap()
}

#[test]
fn t_f012_013_completion_requires_adopted_fact_and_original_continuation() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let before = counts(&store);
    assert!(matches!(
        store.commit_provider_probe_connect_observation(
            &execution.ready.claim,
            final_request(&store, &execution, None)
        ),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    let mut input = candidate(&store, &execution);
    input.continuation.fence =
        unleash_domain::Fence::new(execution.continuation.fence.as_u64() + 1);
    assert!(
        store
            .adopt_provider_probe_connect_observation(&execution.ready.claim, input)
            .is_err()
    );
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    assert_no_model(&store);
}

#[test]
fn t_f012_013_completion_replay_precedes_released_lease_and_missing_object() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let fact = store
        .adopt_provider_probe_connect_observation(
            &execution.ready.claim,
            candidate(&store, &execution),
        )
        .unwrap();
    assert_eq!(held_capacity(&store), 1, "adoption alone never releases");
    assert_eq!(disposition_count(&store), 0);
    let request = final_request(&store, &execution, Some(fact.clone()));
    let committed = store
        .commit_provider_probe_connect_observation(&execution.ready.claim, request.clone())
        .unwrap();
    assert_eq!(held_capacity(&store), 0);
    assert_eq!(disposition_count(&store), 1);
    let before = counts(&store);
    let relative: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT relative_path FROM object_records WHERE object_hash=?1",
                [fact.content_hash().to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    // Deliberately make only this local fixture's native bytes unavailable after
    // a completed commit. A genuine exact replay must not re-verify them.
    std::fs::remove_file(store.config().artifacts_dir.join(relative)).unwrap();
    let replay = store
        .commit_provider_probe_connect_observation(&execution.ready.claim, request)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(counts(&store), before);
    assert_eq!(disposition_count(&store), 1);
    assert_no_model(&store);
}

struct FinishRollback(AtomicBool);
impl CutpointObserver for FinishRollback {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeProviderProbeConnectObservationCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".into(),
                message: "synthetic final rollback".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f012_013_completion_rollback_retains_fact_and_restart_keeps_capacity() {
    let failure = Arc::new(FinishRollback(AtomicBool::new(false)));
    let (store, _directory, mandate, pool) = fixture(failure.clone());
    let execution = dispatched(&store, mandate, pool);
    let fact = store
        .adopt_provider_probe_connect_observation(
            &execution.ready.claim,
            candidate(&store, &execution),
        )
        .unwrap();
    failure.0.store(true, Ordering::SeqCst);
    let before = counts(&store);
    let result = store.commit_provider_probe_connect_observation(
        &execution.ready.claim,
        final_request(&store, &execution, Some(fact.clone())),
    );
    assert!(matches!(result, Err(StoreError::CutpointInjected { .. })));
    assert_eq!(counts(&store), before);
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    let retained:i64=store.read_snapshot(|conn,_|Ok(conn.query_row("SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_probe' AND owner_id=?1 AND source_field='connect_observation' AND state='live' AND object_hash=?2",rusqlite::params![execution.ready.record.context.probe_id.to_string(),fact.content_hash().to_string()],|r|r.get(0))?)).unwrap();
    assert_eq!(retained, 1);
    let store = store.restart_same_lineage().unwrap();
    assert!(
        store
            .reconcile_recovered_actions_after_restart()
            .unwrap()
            .contains(&execution.ready.intent.action_id)
    );
    assert_eq!(
        store
            .inspect_broker_action(execution.ready.intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::OutcomeUnknown
    );
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    assert_no_model(&store);
}
