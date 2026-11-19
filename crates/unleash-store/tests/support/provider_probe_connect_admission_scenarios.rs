//! `SQLite` admission oracle only. No TCP/TLS or provider executor is invoked.
//! The privileged eligibility observer is explicitly synthetic; the product
//! composition must supply its real preflight producer and existing scheduler.
#![expect(
    clippy::panic,
    reason = "The exact ownership/admission variant is the oracle"
)]

#[path = "provider_probe_connect_completion_scenarios.rs"]
mod completion_scenarios;

use super::mixed_admission_fixture as mixed;
use super::{
    PRINCIPAL, PROFILE, caller, change, command, configure, connect, connect_intent, counts,
    envelope, policy, prepare,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{
    AccountId, ActionState, AuthorizationEvaluationId, DispatchId, MicrosAmount, PoolId,
    ProviderProbeId, ReceiptId, Revision, Sha256Digest,
};
use unleash_protocol::{ProviderTestLimits, canonical_sha256, canonicalize_to_vec};
use unleash_store::{
    AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit,
    AdmissionUrgency, AggregateKind, BrokerAuthorizationCause, BrokerAuthorizationDecision,
    BrokerAuthorizationOutcome, BrokerEvaluation, BrokerExecutor, BrokerIntent, BrokerPhase,
    BrokerPhaseRequest, BudgetScopeType, ClaimProviderProbeConnectRequest, CommandMutation,
    CutpointObserver, ExecutionPoolParams, ExecutionPoolSettings, NoopCutpointObserver,
    PrepareObjectRequest, ProviderProbeConnectClaim, ProviderProbeConnectClaimOutcome,
    ProviderProbeConnectRecord, ProviderProbeMandateChange, ProviderProfilePreparation,
    PublishObjectReferenceRequest, SettingsPatch, SettingsScopeKind, StoreConfig, StoreCutpoint,
    StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{ManualClock, local_test_dir};

const OBSERVER: &str = "fixture:connect-admission-observer";

struct LostDispatchAck(AtomicBool);

impl CutpointObserver for LostDispatchAck {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C4AfterBrokerDispatchCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C4".into(),
                message: "synthetic lost dispatch acknowledgement, no executor invoked".into(),
            });
        }
        Ok(())
    }
}

fn publish(store: &UnleashStore, document: &impl serde::Serialize) -> Sha256Digest {
    let bytes = canonicalize_to_vec(document).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let owner = store.config().id_generator.new_uuid().to_string();
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "connect_admission_fixture".into(),
            owner_id: owner.clone(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, &bytes).unwrap();
    store
        .publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "connect_admission_fixture".into(),
            owner_id: owner,
            source_field: "observed_input".into(),
        })
        .unwrap();
    hash
}

fn fixture(
    observer: Arc<dyn CutpointObserver>,
) -> (UnleashStore, impl std::fmt::Debug, ReceiptId, PoolId) {
    fixture_with_clock(observer, None)
}

fn fixture_with_clock(
    observer: Arc<dyn CutpointObserver>,
    clock: Option<Arc<ManualClock>>,
) -> (UnleashStore, impl std::fmt::Debug, ReceiptId, PoolId) {
    fixture_with_options(observer, clock, None)
}

fn fixture_with_options(
    observer: Arc<dyn CutpointObserver>,
    clock: Option<Arc<ManualClock>>,
    endpoint: Option<String>,
) -> (UnleashStore, impl std::fmt::Debug, ReceiptId, PoolId) {
    let directory = local_test_dir("f012-connect-admission").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = observer;
    if let Some(clock) = clock {
        config.clock = clock;
    }
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal(PRINCIPAL, "user", caller().actor_id.as_str())
        .unwrap();
    store
        .register_principal(OBSERVER, "system", OBSERVER)
        .unwrap();
    configure(&store);
    let mut configuration = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap()
        .configuration;
    configuration.profile_version = 2;
    let mut payload = serde_json::to_value(&configuration.payload).unwrap();
    if let Some(endpoint) = endpoint {
        payload
            .as_object_mut()
            .unwrap()
            .insert("endpoint".into(), json!(endpoint));
    }
    let limits = payload.get_mut("limits").unwrap().as_object_mut().unwrap();
    limits.insert("max_inflight".into(), json!(6));
    limits.insert("connect_timeout_ms".into(), json!(30_000));
    configuration.payload = serde_json::from_value(payload).unwrap();
    let hash = publish(&store, &configuration);
    let mut wire = envelope(
        &store,
        200,
        PROFILE,
        "SetProviderProfile",
        serde_json::to_value(&configuration.payload).unwrap(),
    );
    wire.expected_revision = Revision::new(1);
    store
        .commit_command(command(
            wire,
            AggregateKind::ProviderProfile,
            CommandMutation::SetProviderProfile {
                caller: caller(),
                preparation: Box::new(ProviderProfilePreparation {
                    configuration,
                    content_object_hash: hash,
                }),
            },
        ))
        .unwrap();
    let mut permission = change(&store);
    if let ProviderProbeMandateChange::Issue(permission) = &mut permission {
        permission.profile_version = 2;
        permission.limits = ProviderTestLimits::new(30_000, 0, 0, MicrosAmount::ZERO);
    }
    let mandate = store
        .commit_command(policy(&store, 201, 0, permission))
        .unwrap()
        .receipt()
        .receipt_id;
    let pool = store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap()
        .pool_id;
    (store, directory, mandate, pool)
}

struct ReadyProbe {
    record: ProviderProbeConnectRecord,
    intent: BrokerIntent,
    evaluation: BrokerEvaluation,
    claim: ProviderProbeConnectClaim,
}

fn ready(store: &UnleashStore, mandate: ReceiptId, id: u128) -> ReadyProbe {
    ready_for_profile(store, mandate, id, PROFILE, 2)
}

fn ready_for_profile(
    store: &UnleashStore,
    mandate: ReceiptId,
    id: u128,
    profile: uuid::Uuid,
    version: u64,
) -> ReadyProbe {
    let mut wire = connect(store, id, mandate);
    wire.payload.insert("profile_id".into(), json!(profile));
    wire.payload
        .insert("profile_version".into(), json!(version));
    wire.payload.get_mut("limits").unwrap()["timeout_ms"] = json!(30_000);
    let receipt = store
        .commit_command(prepare(store, &wire))
        .unwrap()
        .receipt()
        .clone();
    let record = store
        .inspect_provider_probe_connect_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))
        .unwrap()
        .value
        .unwrap();
    let request = ClaimProviderProbeConnectRequest {
        envelope: wire,
        caller: caller(),
        receipt_id: receipt.receipt_id,
        context_digest: record.context_digest,
        control_fence: unleash_domain::LineageFence::new(
            record.context.authority_generation,
            record.context.daemon_epoch,
            unleash_domain::Fence::ZERO,
        ),
    };
    let ProviderProbeConnectClaimOutcome::FreshClaim(claim) =
        store.claim_provider_probe_connect(request).unwrap()
    else {
        panic!("fresh fixture owner required");
    };
    let mut intent = connect_intent(&record);
    let destination = store
        .resolve_provider_profile_snapshot(profile, version)
        .unwrap()
        .value
        .unwrap()
        .configuration
        .payload
        .endpoint_destination()
        .unwrap();
    intent.manifest.insert(
        "resource_refs".into(),
        json!([{
            "kind":"network_endpoint", "host":destination.host(), "port":destination.port(),
            "is_private_or_loopback":destination.is_loopback_literal(),
        }]),
    );
    intent.intent_digest = canonical_sha256(&intent.manifest).unwrap();
    let evaluation = BrokerEvaluation {
        evaluation_id: AuthorizationEvaluationId::from_uuid(store.config().id_generator.new_uuid()),
        previous_evaluation_id: None,
    };
    ReadyProbe {
        record,
        intent,
        evaluation,
        claim,
    }
}

fn phase(ready: &ReadyProbe, revision: u64, phase: BrokerPhase) -> BrokerPhaseRequest {
    BrokerPhaseRequest {
        authority_generation: ready.intent.lineage_fence.authority_generation,
        expected_revision: Revision::new(revision),
        evaluation: ready.evaluation,
        phase,
    }
}

fn authorizing(store: &UnleashStore, ready: &ReadyProbe) {
    store
        .commit_provider_probe_connect_phase(
            &ready.claim,
            phase(
                ready,
                0,
                BrokerPhase::Authorizing {
                    intent: ready.intent.clone(),
                },
            ),
        )
        .unwrap();
}

fn observe_eligibility(store: &UnleashStore, ready: &ReadyProbe, pool: PoolId) {
    observe_eligibility_until(store, ready, pool, ready.record.context.deadline);
}

fn observe_eligibility_until(
    store: &UnleashStore,
    ready: &ReadyProbe,
    pool: PoolId,
    valid_until: unleash_domain::TimestampMs,
) {
    let c = &ready.record.context;
    let bytes = store
        .read_verified_object_bytes(c.configuration_digest)
        .unwrap();
    assert_eq!(Sha256Digest::from_bytes(&bytes), c.configuration_digest);
    let hash = publish(
        store,
        &json!({"schema_version":1,"provenance":"synthetic_sqlite_admission_observer","probe_id":c.probe_id,"receipt_id":c.receipt_id,"action_id":c.action_id,"context_digest":ready.record.context_digest,"claim_id":ready.claim.claim_id(),"configuration_object_hash":c.configuration_digest,"policy_digest":c.policy_digest,"mandate_digest":c.mandate_digest,"transport_constraint":"loopback_only","phase":"connect"}),
    );
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    let candidate = snapshot
        .queued
        .iter()
        .find(|candidate| candidate.unit == AdmissionUnit::Probe(c.probe_id))
        .unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: store.config().id_generator.new_uuid(),
            observer_principal_id: OBSERVER.into(),
            object_hash: hash,
            unit: candidate.unit,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
}

fn admit(
    store: &UnleashStore,
    ready: &ReadyProbe,
    pool: PoolId,
) -> Result<AdmissionOutcome, StoreError> {
    let snapshot = store.execution_admission_snapshot(pool)?;
    let unit = AdmissionUnit::Probe(ready.record.context.probe_id);
    // A test proposal, not another product scheduling policy. The real daemon
    // uses schedule_execution; this oracle exercises the same V008 writer/CAS.
    store.commit_execution_admission(pool, snapshot.token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit,
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![],
        })
    })
}

fn assert_no_model(store: &UnleashStore) {
    let values = counts(store);
    assert_eq!(
        values.get(4..6).unwrap(),
        &[0, 0],
        "a transport probe has no ModelCall or manifest"
    );
    assert_eq!(
        values.get(7).copied().unwrap(),
        0,
        "this local passive phase invents no monetary reservation"
    );
}

fn lease_rows(store: &UnleashStore) -> i64 {
    store
        .read_snapshot(|db, _| {
            db.query_row("SELECT COUNT(*) FROM resource_leases", [], |row| row.get(0))
                .map_err(StoreError::from)
        })
        .unwrap()
}

fn held_capacity(store: &UnleashStore) -> i64 {
    store
        .read_snapshot(|db, _| {
            db.query_row(
                "SELECT COUNT(*) FROM capacity_reservations WHERE state IN ('held','reconciling')",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
        })
        .unwrap()
}

fn assert_generic_phase_refused(store: &UnleashStore, request: BrokerPhaseRequest) {
    let before = counts(store);
    let dispatches = || {
        store
            .read_snapshot(|db, _| {
                Ok(
                    db.query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .unwrap()
    };
    let markers = dispatches();
    assert!(
        matches!(
            store.commit_broker_phase(request),
            Err(StoreError::EvidenceUnavailable(_))
        ),
        "the generic port cannot acknowledge connect, including an exact replay"
    );
    assert_eq!(counts(store), before);
    assert_eq!(dispatches(), markers);
}

#[test]
fn t_f012_013_admission_current_claim_authorizing_then_real_v008_reservation() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let ready = ready(&store, mandate, 300);
    authorizing(&store, &ready);
    assert_generic_phase_refused(
        &store,
        phase(
            &ready,
            0,
            BrokerPhase::Authorizing {
                intent: ready.intent.clone(),
            },
        ),
    );
    assert_eq!(
        counts(&store).get(6).copied().unwrap(),
        0,
        "Authorizing is not a capacity reservation"
    );
    observe_eligibility(&store, &ready, pool);
    let AdmissionOutcome::Admitted(commit) = admit(&store, &ready, pool).unwrap() else {
        panic!("the real V008 port must admit the eligible probe");
    };
    assert_eq!(
        commit.unit,
        AdmissionUnit::Probe(ready.record.context.probe_id)
    );
    assert!(commit.lease.fence.as_u64() > 0);
    assert_eq!(
        ready.intent.lineage_fence.fence,
        unleash_domain::Fence::ZERO
    );
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    assert_eq!(snapshot.active.len(), 1);
    assert_no_model(&store);
}

#[test]
fn t_f012_013_admission_scope_and_control_fence_are_rechecked_before_intention() {
    for changed_scope in [false, true] {
        let (store, _directory, mandate, _) = fixture(Arc::new(NoopCutpointObserver));
        let mut ready = ready(&store, mandate, 301);
        if changed_scope {
            let patch = SettingsPatch {
                execution_pool: Some(ExecutionPoolSettings {
                    max_active_executions: 4,
                    ..ExecutionPoolSettings::default()
                }),
                ..SettingsPatch::default()
            };
            let wire = envelope(
                &store,
                302,
                store.active_authority_generation().unwrap().as_uuid(),
                "SetSettings",
                json!({"scope_kind":"user","scope_id":"default","patch":patch}),
            );
            store
                .commit_command(command(
                    wire,
                    AggregateKind::Settings,
                    CommandMutation::SetSettings {
                        scope_kind: SettingsScopeKind::User,
                        scope_id: "default".into(),
                        caller: caller(),
                        patch,
                    },
                ))
                .unwrap();
        } else {
            ready.intent.lineage_fence.fence = unleash_domain::Fence::new(1);
            ready
                .intent
                .manifest
                .insert("lineage_fence".into(), json!(ready.intent.lineage_fence));
            ready.intent.intent_digest = canonical_sha256(&ready.intent.manifest).unwrap();
        }
        let before = counts(&store);
        assert!(matches!(
            store.commit_provider_probe_connect_phase(
                &ready.claim,
                phase(
                    &ready,
                    0,
                    BrokerPhase::Authorizing {
                        intent: ready.intent.clone()
                    }
                )
            ),
            Err(StoreError::StaleFence { .. })
        ));
        assert_eq!(counts(&store), before);
    }
}

#[test]
fn t_f012_013_admission_destination_is_exact_not_just_any_loopback() {
    let (store, _directory, mandate, _) = fixture(Arc::new(NoopCutpointObserver));
    let ready = ready(&store, mandate, 303);
    let before = counts(&store);
    for (host, port) in [("127.0.0.2", 1234), ("127.0.0.1", 1235)] {
        let mut intent = ready.intent.clone();
        intent.manifest.insert("resource_refs".into(), json!([{"kind":"network_endpoint","host":host,"port":port,"is_private_or_loopback":true}]));
        intent.intent_digest = canonical_sha256(&intent.manifest).unwrap();
        assert!(matches!(
            store.commit_provider_probe_connect_phase(
                &ready.claim,
                phase(&ready, 0, BrokerPhase::Authorizing { intent })
            ),
            Err(StoreError::InvalidArgument { .. })
        ));
        assert_eq!(counts(&store), before);
    }
}

#[test]
fn t_f012_013_admission_global_five_and_sixth_waits_without_a_hidden_place() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let object = seed_four_other_units(&store, pool);
    let model_baseline = counts(&store);
    assert_generation_fixture_model_parents(&store);
    let connect = ready(&store, mandate, 315);
    authorizing(&store, &connect);
    observe_eligibility(&store, &connect, pool);
    assert!(matches!(
        admit(&store, &connect, pool),
        Ok(AdmissionOutcome::Admitted(_))
    ));
    mixed::observe(&store, object);
    let before = counts(&store);
    let leases = lease_rows(&store);
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    assert!(matches!(
        store.commit_execution_admission(pool, snapshot.token, |_| Ok(mixed_proposal(8))),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert_eq!(
        counts(&store),
        before,
        "the rejected sixth proposal leaves no lease or fairness delta"
    );
    assert_eq!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .active
            .len(),
        5
    );
    assert_eq!(lease_rows(&store), leases);
    assert_eq!(
        store.execution_admission_snapshot(pool).unwrap().fairness,
        snapshot.fairness
    );
    assert_eq!(held_capacity(&store), 5);
    assert_eq!(counts(&store).get(4..6), model_baseline.get(4..6));
    assert_generation_fixture_model_parents(&store);
    let calls: i64 = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM model_calls WHERE provider_probe_id=?1",
                [connect.record.context.probe_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let manifests:i64=store.read_snapshot(|db,_| Ok(db.query_row(
        "SELECT COUNT(*) FROM model_call_manifests m JOIN model_calls c ON c.model_call_id=m.model_call_id WHERE c.provider_probe_id=?1",
        [connect.record.context.probe_id.to_string()],|row|row.get(0))?)).unwrap();
    assert_eq!(
        (calls, manifests),
        (0, 0),
        "connect does not acquire model parents from the mixed fixture"
    );
}

fn mixed_proposal(id: u128) -> AdmissionDecision {
    AdmissionDecision::Admit {
        unit: mixed::unit(id),
        deficit_deltas: BTreeMap::from([(
            Some(unleash_domain::ProjectId::from_uuid(mixed::PROJECT)),
            0,
        )]),
        consecutive_controls: 0,
        budgets: vec![],
    }
}

fn seed_four_other_units(store: &UnleashStore, pool: PoolId) -> Sha256Digest {
    // Reuse the existing explicit F003 synthetic executor, never an OS/model
    // producer. Only the two invocations and two replay workers are admitted.
    let object = mixed::seed(store, false);
    assert_eq!(pool, mixed::POOL);
    for id in 1..=4 {
        mixed::observe(store, object);
        let snapshot = store.execution_admission_snapshot(pool).unwrap();
        let AdmissionOutcome::Admitted(commit) = store
            .commit_execution_admission(pool, snapshot.token, move |_| Ok(mixed_proposal(id)))
            .unwrap()
        else {
            panic!("the fixture's actual V008 unit must be admitted");
        };
        assert_eq!(commit.unit, mixed::unit(id));
    }
    let owners:i64=store.read_snapshot(|db,_| Ok(db.query_row(
        "SELECT COUNT(*) FROM capacity_reservations r JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id WHERE r.state='held' AND ((r.owner_type='agent_invocation' AND r.owner_id=o.invocation_id AND o.attempt_id IS NOT NULL AND o.replay_worker_id IS NULL AND o.provider_probe_id IS NULL) OR (r.owner_type='replay_worker' AND r.owner_id=o.replay_worker_id AND o.invocation_id IS NULL AND o.attempt_id IS NULL AND o.provider_probe_id IS NULL))",[],|row|row.get(0))?)).unwrap();
    assert_eq!(owners, 4);
    object
}

fn assert_generation_fixture_model_parents(store: &UnleashStore) {
    // seed() also prepares two queued generation probes; their preexisting
    // request manifests remain a stable baseline, never attributed to connect.
    for id in [5, 6] {
        let probe = ProviderProbeId::from_u128(id);
        let parents:(i64,i64)=store.read_snapshot(|db,_| Ok(db.query_row(
            "SELECT (SELECT COUNT(*) FROM model_calls WHERE provider_probe_id=?1), (SELECT COUNT(*) FROM model_call_manifests m JOIN model_calls c ON c.model_call_id=m.model_call_id WHERE c.provider_probe_id=?1)",
            [probe.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?)).unwrap();
        assert_eq!(parents, (1, 1));
    }
}

fn authorize(store: &UnleashStore, ready: &ReadyProbe) -> BrokerAuthorizationDecision {
    let c = &ready.record.context;
    let decision = BrokerAuthorizationDecision {
        action_id: c.action_id,
        evaluation_id: ready.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: format!("synthetic-sqlite-decision:{}", c.action_id),
        policy_revision: c.policy_revision,
        scope_revision: c.scope_revision,
        payload_digest: c.request_digest,
        expiry: Some(c.deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    store
        .commit_provider_probe_connect_phase(
            &ready.claim,
            phase(
                ready,
                1,
                BrokerPhase::Authorization {
                    action_id: c.action_id,
                    intent_digest: ready.intent.intent_digest,
                    outcome: BrokerAuthorizationOutcome::Authorized {
                        decision: decision.clone(),
                    },
                    cause: BrokerAuthorizationCause::Evaluation,
                },
            ),
        )
        .unwrap();
    decision
}

#[test]
fn t_f012_013_admission_c4_uncertainty_and_restart_keep_the_real_reservation() {
    let lost = Arc::new(LostDispatchAck(AtomicBool::new(false)));
    let (store, _directory, mandate, pool) = fixture(lost.clone());
    let other_profile = uuid::Uuid::from_u128(0xf012_c502);
    let other_mandate = configure_second_profile(&store, other_profile);
    let ready = ready(&store, mandate, 320);
    authorizing(&store, &ready);
    observe_eligibility(&store, &ready, pool);
    assert!(matches!(
        admit(&store, &ready, pool),
        Ok(AdmissionOutcome::Admitted(_))
    ));
    let decision = authorize(&store, &ready);
    let dispatch = phase(
        &ready,
        2,
        BrokerPhase::Dispatch {
            action_id: ready.intent.action_id,
            intent_digest: ready.intent.intent_digest,
            decision,
            dispatch_id: DispatchId::from_uuid(store.config().id_generator.new_uuid()),
            fence: ready.intent.lineage_fence,
        },
    );
    lost.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_provider_probe_connect_phase(&ready.claim, dispatch.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store).get(6).copied().unwrap(), 1);
    assert_eq!(held_capacity(&store), 1);
    assert_generic_phase_refused(&store, dispatch.clone());
    assert!(
        store
            .commit_provider_probe_connect_phase(&ready.claim, dispatch.clone())
            .is_err(),
        "even the live owner cannot replay a durable dispatch marker"
    );
    let store = store.restart_same_lineage().unwrap();
    assert!(
        store
            .reconcile_recovered_actions_after_restart()
            .unwrap()
            .contains(&ready.intent.action_id)
    );
    assert_eq!(
        store
            .inspect_broker_action(ready.intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::OutcomeUnknown
    );
    assert_eq!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .active
            .len(),
        1
    );
    assert_eq!(
        counts(&store).get(6).copied().unwrap(),
        1,
        "unknown is not capacity release"
    );
    assert_eq!(held_capacity(&store), 1);
    assert!(
        store
            .commit_provider_probe_connect_phase(&ready.claim, dispatch)
            .is_err(),
        "the durable marker never authorizes a second effect"
    );
    assert_other_profile_waits_after_unknown(&store, other_profile, other_mandate, pool);
    assert_no_model(&store);
}

fn assert_other_profile_waits_after_unknown(
    store: &UnleashStore,
    profile: uuid::Uuid,
    mandate: ReceiptId,
    pool: PoolId,
) {
    let next = ready_for_profile(store, mandate, 354, profile, 1);
    authorizing(store, &next);
    for reconciling in [false, true] {
        if reconciling {
            // The action above became Unknown through real C4/restart recovery.
            // This explicit fixture labels its retained reservation conservatively;
            // it does not manufacture a stop, disposition or neutralization proof.
            store.execute_in_writer_tx(WriteCriticality::Critical, |tx,_| {
                assert_eq!(tx.execute("UPDATE capacity_reservations SET state='reconciling' WHERE state='held'",[])?,1);
                Ok(())
            }).unwrap();
        }
        observe_eligibility(store, &next, pool);
        let before = counts(store);
        let leases = lease_rows(store);
        let result = admit(store, &next, pool);
        assert!(
            matches!(result, Err(StoreError::CapacityBlocked(_))),
            "another incarnation retains the unknown connect's slot (reconciling={reconciling}): {result:?}"
        );
        assert_eq!(counts(store), before);
        assert_eq!(lease_rows(store), leases);
        assert_eq!(held_capacity(store), 1);
    }
}

#[test]
fn t_f012_013_admission_local_no_price_still_observes_applicable_overrun() {
    let (store, _directory, mandate, pool) = fixture(Arc::new(NoopCutpointObserver));
    let ready = ready(&store, mandate, 330);
    authorizing(&store, &ready);
    let account = AccountId::from_u128(330);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(10),
        )
        .unwrap();
    // Explicit synthetic existing usage, not a claim of observed provider usage.
    store.execute_in_writer_tx(WriteCriticality::Critical, move |tx,_| {
        tx.execute("UPDATE budget_accounts SET spent_micros=11,revision=revision+1 WHERE account_id=?1", [account.to_string()])?;
        Ok(())
    }).unwrap();
    observe_eligibility(&store, &ready, pool);
    let before = counts(&store);
    assert!(matches!(
        admit(&store, &ready, pool),
        Err(StoreError::BudgetExhausted { .. })
    ));
    assert_eq!(counts(&store), before);
    assert_no_model(&store);
}

#[test]
fn t_f012_013_admission_context_expiry_is_rechecked_after_decide() {
    let clock = Arc::new(ManualClock::new(1_700_000_000_000));
    let (store, _directory, mandate, pool) =
        fixture_with_clock(Arc::new(NoopCutpointObserver), Some(clock.clone()));
    let ready = ready(&store, mandate, 340);
    authorizing(&store, &ready);
    // This synthetic observer's lifetime outlasts the transport context. Its
    // evidence cannot extend either the request deadline or the effect mandate.
    observe_eligibility_until(
        &store,
        &ready,
        pool,
        unleash_domain::TimestampMs::new(ready.record.context.deadline.as_i64() + 10_000),
    );
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    let before = counts(&store);
    let leases = lease_rows(&store);
    let unit = AdmissionUnit::Probe(ready.record.context.probe_id);
    let delta = u64::try_from(
        ready.record.context.deadline.as_i64() - store.config().clock.now_ms().as_i64() + 1,
    )
    .unwrap();
    let callback_ran = Arc::new(AtomicBool::new(false));
    let callback_flag = callback_ran.clone();
    let result = store.commit_execution_admission(pool, snapshot.token, move |_| {
        callback_flag.store(true, Ordering::SeqCst);
        clock.advance_ms(delta);
        Ok(AdmissionDecision::Admit {
            unit,
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![],
        })
    });
    assert!(
        callback_ran.load(Ordering::SeqCst),
        "the genuine writer decision must be reached"
    );
    assert!(
        matches!(result, Err(StoreError::StaleFence { .. })),
        "the context expires inside decide while the eligibility token remains live: {result:?}"
    );
    assert_eq!(counts(&store), before);
    assert_eq!(lease_rows(&store), leases);
    let after = store.execution_admission_snapshot(pool).unwrap();
    assert_eq!(after.source_revision, snapshot.source_revision);
    assert_eq!(after.fairness, snapshot.fairness);
    assert!(after.active.is_empty());
    assert_no_model(&store);
}

fn configure_second_profile(store: &UnleashStore, profile: uuid::Uuid) -> ReceiptId {
    let mut configuration = store
        .resolve_provider_profile_snapshot(PROFILE, 2)
        .unwrap()
        .value
        .unwrap()
        .configuration;
    configuration.profile_id = profile;
    configuration.profile_version = 1;
    let hash = publish(store, &configuration);
    let wire = envelope(
        store,
        350,
        profile,
        "SetProviderProfile",
        serde_json::to_value(&configuration.payload).unwrap(),
    );
    store
        .commit_command(command(
            wire,
            AggregateKind::ProviderProfile,
            CommandMutation::SetProviderProfile {
                caller: caller(),
                preparation: Box::new(ProviderProfilePreparation {
                    configuration,
                    content_object_hash: hash,
                }),
            },
        ))
        .unwrap();
    let mut permission = change(store);
    if let ProviderProbeMandateChange::Issue(permission) = &mut permission {
        permission.profile_id = profile;
        permission.profile_version = 1;
        permission.limits = ProviderTestLimits::new(30_000, 0, 0, MicrosAmount::ZERO);
    }
    store
        .commit_command(policy(store, 351, 1, permission))
        .unwrap()
        .receipt()
        .receipt_id
}

#[test]
fn t_f012_013_admission_unresolved_server_capacity_is_shared_across_profiles() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let (store, _directory, mandate_a, pool) =
        fixture_with_options(Arc::new(NoopCutpointObserver), None, Some(endpoint.clone()));
    let profile_b = uuid::Uuid::from_u128(0xf012_c502);
    let mandate_b = configure_second_profile(&store, profile_b);
    let first = ready(&store, mandate_a, 352);
    let second = ready_for_profile(&store, mandate_b, 353, profile_b, 1);
    assert_eq!(first.record.context.destination, endpoint);
    assert_eq!(second.record.context.destination, endpoint);
    assert_ne!(
        first.record.context.profile_id,
        second.record.context.profile_id
    );
    authorizing(&store, &first);
    authorizing(&store, &second);
    observe_eligibility(&store, &first, pool);
    assert!(matches!(
        admit(&store, &first, pool),
        Ok(AdmissionOutcome::Admitted(_))
    ));
    observe_eligibility(&store, &second, pool);
    let before = counts(&store);
    let leases = lease_rows(&store);
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    let result = admit(&store, &second, pool);
    assert!(
        matches!(result, Err(StoreError::CapacityBlocked(_))),
        "an unresolved server has one conservative slot across profiles: {result:?}"
    );
    assert_eq!(counts(&store), before);
    assert_eq!(lease_rows(&store), leases);
    assert_eq!(held_capacity(&store), 1);
    let after = store.execution_admission_snapshot(pool).unwrap();
    assert_eq!(after.active.len(), 1);
    assert_eq!(after.source_revision, snapshot.source_revision);
    assert_eq!(after.fairness, snapshot.fairness);
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    assert_no_model(&store);
}
