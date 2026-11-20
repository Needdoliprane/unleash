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
fn t_f012_013_adoption_refuses_changed_dispatch_request_hash() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let original: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT request_hash FROM action_dispatches WHERE dispatch_id=?1",
                [execution.dispatch.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let modified =
        Sha256Digest::from_bytes(b"synthetic corruption of normalized dispatch envelope");
    assert_ne!(original, modified.to_string());
    let dispatch = execution.dispatch;
    // One explicit storage-fault injection; no receipt, state, fence or proof is fabricated.
    let changed=store.execute_in_writer_tx(WriteCriticality::Normal,move |tx,_| {
        Ok(tx.execute("UPDATE action_dispatches SET request_hash=?2 WHERE dispatch_id=?1 AND request_hash=?3",
            rusqlite::params![dispatch.to_string(),modified.to_string(),original])?)
    }).unwrap();
    assert_eq!(
        changed, 1,
        "only the real dispatch's normalized request hash changes"
    );
    let before = store
        .inspect_broker_action(execution.ready.intent.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(before.state, ActionState::Dispatching);
    assert_eq!(held_capacity(&store), 1);
    let input = candidate(&store, &execution);
    let result = store.adopt_provider_probe_connect_observation(&execution.ready.claim, input);
    assert!(
        result.is_err(),
        "an altered normalized dispatch must not acquire a native fact capability"
    );
    let facts:i64=store.read_snapshot(|conn,_|Ok(conn.query_row(
        "SELECT COUNT(*) FROM evidence WHERE action_id=?1 AND snapshot_ref LIKE 'provider-connect-fact:v1:%'",
        [execution.ready.intent.action_id.to_string()],|row|row.get(0),
    )?)).unwrap();
    assert_eq!(facts, 0);
    assert_eq!(
        store
            .inspect_broker_action(execution.ready.intent.action_id)
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    assert_no_model(&store);
}

#[test]
fn t_f012_013_completion_rechecks_marker_after_fact_adoption() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let fact = store
        .adopt_provider_probe_connect_observation(
            &execution.ready.claim,
            candidate(&store, &execution),
        )
        .unwrap();
    let request = final_request(&store, &execution, Some(fact.clone()));
    let dispatch = execution.dispatch;
    let modified = Sha256Digest::from_bytes(b"synthetic marker corruption after fact adoption");
    let changed = store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            Ok(tx.execute(
                "UPDATE action_dispatches SET request_hash=?2 WHERE dispatch_id=?1",
                rusqlite::params![dispatch.to_string(), modified.to_string()],
            )?)
        })
        .unwrap();
    assert_eq!(changed, 1);
    let before = counts(&store);
    assert!(
        store
            .commit_provider_probe_connect_observation(&execution.ready.claim, request)
            .is_err()
    );
    assert_eq!(counts(&store), before);
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(disposition_count(&store), 0);
    assert_eq!(
        store
            .inspect_broker_action(execution.ready.intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Dispatching
    );
    let retained:i64=store.read_snapshot(|conn,_|Ok(conn.query_row(
        "SELECT COUNT(*) FROM evidence e JOIN object_references r ON r.object_hash=e.object_hash AND r.owner_kind='provider_probe' AND r.owner_id=?1 AND r.source_field='connect_observation' AND r.state='live' WHERE e.action_id=?2 AND e.object_hash=?3 AND e.availability='available'",
        rusqlite::params![execution.ready.record.context.probe_id.to_string(),execution.ready.intent.action_id.to_string(),fact.content_hash().to_string()],|row|row.get(0),
    )?)).unwrap();
    assert_eq!(
        retained, 1,
        "an adopted native fact survives a refused final disposition"
    );
    assert_no_model(&store);
}

#[test]
fn t_f012_013_native_fact_after_policy_change_does_not_reauthorize_io() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let mut changed = change(&store);
    if let ProviderProbeMandateChange::Issue(issue) = &mut changed {
        issue.profile_version = 2;
        issue.limits = ProviderTestLimits::new(30_000, 0, 0, MicrosAmount::ZERO);
    }
    let receipt = store
        .commit_command(policy(&store, 202, 1, changed))
        .unwrap();
    assert_eq!(receipt.receipt().new_revision, Revision::new(2));
    // Trusted-observer fixture only: this asserts SQLite attribution, not real I/O.
    let fact = store
        .adopt_provider_probe_connect_observation(
            &execution.ready.claim,
            candidate(&store, &execution),
        )
        .unwrap();
    store
        .commit_provider_probe_connect_observation(
            &execution.ready.claim,
            final_request(&store, &execution, Some(fact)),
        )
        .unwrap();
    assert_eq!(
        store
            .inspect_broker_action(execution.ready.intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::ObservedSuccess
    );
    assert_eq!(held_capacity(&store), 0);
    assert_eq!(disposition_count(&store), 1);
    assert_no_model(&store);
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

fn pre_dispatch_dispositions(store: &UnleashStore) -> i64 {
    store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM events WHERE event_type='ProviderProbeDispatchRefused'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

#[test]
fn t_f012_013_pre_dispatch_disposal_refuses_active_marker_and_unknown() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let execution = dispatched(&store, mandate, pool);
    let before = counts(&store);
    assert!(
        store
            .dispose_provider_probe_connect_without_dispatch(
                &execution.ready.claim,
                &execution.continuation
            )
            .is_err()
    );
    assert_eq!(counts(&store), before);
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(pre_dispatch_dispositions(&store), 0);
    let mut unknown = final_request(&store, &execution, None);
    let BrokerPhase::Observation { receipt, .. } = &mut unknown.request.phase else {
        panic!("observation fixture");
    };
    receipt.state = ActionState::OutcomeUnknown;
    store
        .commit_provider_probe_connect_observation(&execution.ready.claim, unknown)
        .unwrap();
    let before = counts(&store);
    assert!(
        store
            .dispose_provider_probe_connect_without_dispatch(
                &execution.ready.claim,
                &execution.continuation
            )
            .is_err()
    );
    assert_eq!(counts(&store), before);
    assert_eq!(held_capacity(&store), 1);
    assert_eq!(pre_dispatch_dispositions(&store), 0);
    assert_eq!(
        store
            .inspect_broker_action(execution.ready.intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::OutcomeUnknown
    );
}

#[test]
fn t_f012_013_pre_dispatch_disposal_replays_after_release_and_rejects_other_fence() {
    let clock = Arc::new(ManualClock::default());
    let (store, _directory, mandate, pool) =
        fixture_with_clock(Arc::new(NoopCutpointObserver), Some(clock.clone()));
    let earlier = dispatched(&store, mandate, pool);
    let fact = store
        .adopt_provider_probe_connect_observation(&earlier.ready.claim, candidate(&store, &earlier))
        .unwrap();
    store
        .commit_provider_probe_connect_observation(
            &earlier.ready.claim,
            final_request(&store, &earlier, Some(fact)),
        )
        .unwrap();
    let prior = store
        .get_provider_profile_snapshot(PROFILE, 2, None)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(
        prior
            .observations
            .iter()
            .filter(|observation| observation.phase == unleash_protocol::ProviderTestPhase::Connect)
            .count(),
        1
    );
    let ready = ready(&store, mandate, 0xf012_0401);
    authorizing(&store, &ready);
    observe_eligibility(&store, &ready, pool);
    let AdmissionOutcome::Admitted(commit) = admit(&store, &ready, pool).unwrap() else {
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
    authorize(&store, &ready);
    let mut changed = change(&store);
    if let ProviderProbeMandateChange::Issue(issue) = &mut changed {
        issue.profile_version = 2;
        issue.limits = ProviderTestLimits::new(30_000, 0, 0, MicrosAmount::ZERO);
    }
    store
        .commit_command(policy(&store, 202, 1, changed))
        .unwrap();
    let committed = store
        .dispose_provider_probe_connect_without_dispatch(&ready.claim, &continuation)
        .unwrap();
    assert!(!committed.replayed);
    assert_eq!(held_capacity(&store), 0);
    assert_eq!(pre_dispatch_dispositions(&store), 1);
    assert_no_connected_view(&store);
    let before = counts(&store);
    let replay = store
        .dispose_provider_probe_connect_without_dispatch(&ready.claim, &continuation)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(replay.revision, committed.revision);
    assert_eq!(counts(&store), before);
    let mut other = continuation.clone();
    other.fence = unleash_domain::Fence::new(other.fence.as_u64() + 1);
    assert!(
        store
            .dispose_provider_probe_connect_without_dispatch(&ready.claim, &other)
            .is_err()
    );
    assert_eq!(counts(&store), before);
    assert_eq!(pre_dispatch_dispositions(&store), 1);
    assert_no_model(&store);
    purge_disposition_payload(&store, &clock, &committed);
    let before = counts(&store);
    let retained = store
        .dispose_provider_probe_connect_without_dispatch(&ready.claim, &other)
        .unwrap_err();
    assert!(matches!(retained, StoreError::EvidenceUnavailable(_)));
    let replay = store
        .dispose_provider_probe_connect_without_dispatch(&ready.claim, &continuation)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(counts(&store), before);
    assert_no_connected_view(&store);
}

fn assert_no_connected_view(store: &UnleashStore) {
    let view = store
        .get_provider_profile_snapshot(PROFILE, 2, None)
        .unwrap()
        .value
        .unwrap();
    assert!(
        view.observations
            .iter()
            .all(|observation| observation.phase != unleash_protocol::ProviderTestPhase::Connect),
        "a newer proven refusal never falls back to an earlier connected result"
    );
}

fn purge_disposition_payload(
    store: &UnleashStore,
    clock: &ManualClock,
    committed: &unleash_store::ProviderConnectDispositionCommit,
) {
    let sequence = i64::try_from(committed.event_cursor.sequence.as_u64()).unwrap();
    let before: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT payload_hash FROM events WHERE sequence=?1",
                [sequence],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    // Synthetic local consumers read every envelope before fixture acknowledgement.
    let destinations: Vec<String> = store
        .read_snapshot(|conn, _| {
            let mut statement = conn.prepare("SELECT DISTINCT destination FROM outbox")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .unwrap();
    for destination in destinations {
        loop {
            let batch = store.lease_outbox_batch(&destination, 32).unwrap();
            if batch.is_empty() {
                break;
            }
            for item in batch {
                assert!(serde_json::to_value(&item.envelope).unwrap().is_object());
                store
                    .ack_outbox_delivery(item.event_id, &item.destination)
                    .unwrap();
            }
        }
    }
    clock.advance(std::time::Duration::from_secs(8 * 86_400));
    assert!(
        store
            .purge_history(store.config().clock.now_ms())
            .unwrap()
            .affected_rows
            > 0
    );
    let after: (String, String) = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT payload_state,payload_hash FROM events WHERE sequence=?1",
                [sequence],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(
        after,
        ("tombstone".into(), before),
        "Q12 retains the real canonical event hash used for replay"
    );
}
