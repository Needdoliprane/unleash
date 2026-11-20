//! Tests the Store boundary with a synthetic observation from a test-only observer.

use super::*;
use std::collections::BTreeMap;
use unleash_domain::{
    ActionOrigin, AuthorizationEvaluationId, Fence, LineageFence, PoolId, ProviderProbeId,
    ReconcileStrategy,
};
use unleash_protocol::canonical_sha256;
use unleash_store::{
    AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit,
    AdmissionUrgency, BrokerCause, BrokerEvaluation, BrokerIntent, BrokerPhase, BrokerPhaseRequest,
    ExecutionAdmissionSnapshot, ExecutionPoolSettings, PolicyIsolationMode, PreparedProviderProbe,
    SettingsPatch, SettingsScopeKind, objects::PublishedObjectReference,
};
use unleash_testkit::ManualClock;

const OBSERVER: &str = "system:provider-generation-placement";

fn initialize_pool(store: &UnleashStore, caller: &CallerAuthority) -> PoolId {
    let execution_pool = ExecutionPoolSettings::default();
    let authority = store.active_authority_generation().unwrap();
    store.commit_command(CommitCommandRequest {
        actor_id: caller.actor_id.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion, request_id: RequestId::from_u128(660),
            command_id: CommandId::from_u128(660), authority_generation: authority,
            actor_ref: Some(caller.actor_id.to_string()), project_id: None,
            aggregate_id: authority.as_uuid(), expected_revision: Revision::INITIAL,
            command_type: "SetSettings".to_owned(),
            payload: json!({"scope_kind":"user","scope_id":"default","patch":{"execution_pool":execution_pool}})
                .as_object().unwrap().clone(),
        },
        target_aggregate: AggregateKind::Settings,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::SetSettings {
            scope_kind: SettingsScopeKind::User, scope_id: "default".to_owned(), caller: caller.clone(),
            patch: SettingsPatch {
                execution_pool: Some(execution_pool), retention_policy: None, confirm_retention_shortening: false,
                analyzer_paths: None, extra_typed: Some(serde_json::Map::new()),
            },
        },
        events: vec![], capacity_reservations: vec![], budget_reservations: vec![],
    }).unwrap();
    store.global_execution_pool_id().unwrap()
}

fn first_phase(
    record: &PreparedProviderProbe,
    isolation: PolicyIsolationMode,
) -> Result<BrokerPhaseRequest, StoreError> {
    let c = &record.context;
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::ZERO);
    let manifest = json!({
        "action_id":c.action_id,"subject":{"kind":"control_plane","principal_id":c.principal_id,
            "cause_ref":format!("receipt:{}",c.command_receipt_id),"invocation_id":null,"attempt_id":null,
            "replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id":c.project_id,"verb":"model.provider_probe","resource_refs":record.resource_refs(),
        "payload_digest":record.payload_digest(),"reconcile_strategy":"observe_then_decide","effects_class":"model_dispatch",
        "scope_revision":c.scope_revision,"policy_revision":c.policy_revision,"lineage_fence":fence,
        "action_deadline":c.deadline,"isolation_requirements":{"mode":isolation.as_str(),"require_process_confinement":false,"required_capabilities":[]},
        "secret_refs":[],"provenance":{"origin":"engine_internal"},
    }).as_object().unwrap().clone();
    Ok(BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_uuid(c.action_id.as_uuid()),
            previous_evaluation_id: None,
        },
        phase: BrokerPhase::Authorizing {
            intent: BrokerIntent {
                action_id: c.action_id,
                origin: ActionOrigin::ControlPlane {
                    control_principal_id: c.principal_id.clone(),
                },
                principal_id: c.principal_id.clone(),
                cause: BrokerCause::CommandReceipt {
                    receipt_id: c.command_receipt_id,
                },
                project_id: c.project_id,
                payload_digest: record.payload_digest(),
                scope_revision: c.scope_revision,
                policy_revision: c.policy_revision,
                lineage_fence: fence,
                action_deadline: c.deadline,
                reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
                intent_digest: canonical_sha256(&manifest)?,
                manifest,
            },
        },
    })
}

fn observation(store: &UnleashStore, record: &PreparedProviderProbe) -> serde_json::Value {
    let c = &record.context;
    let profile = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let boot: String = store
        .read_snapshot(|conn, _| {
            Ok(
                conn.query_row("SELECT boot_identity FROM daemon_state", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    json!({
        "schema_version":1,"method":"managed_process_no_egress",
        "probe_id":c.probe_id,"action_id":c.action_id,"context_digest":record.context_digest,
        "request_manifest_hash":canonical_sha256(&record.request_manifest().unwrap()).unwrap(),
        "profile_configuration_digest":profile.configuration_digest,"model_id":c.preparation.resolved_model_id,
        "destination":profile.configuration.payload.endpoint(),
        "server":{"boot_identity":boot,"process_id":771,"process_start_identity":"synthetic-identity",
            "build_digest":Sha256Digest::from_bytes(b"synthetic build"),
            "confinement_digest":Sha256Digest::from_bytes(b"synthetic confinement")},
        "observed_at_ms":store.config().clock.now_ms(),"valid_until_ms":c.deadline,
    })
}

fn candidate(store: &UnleashStore, fact: &serde_json::Value) -> PublishedObjectReference {
    let bytes = canonicalize_to_vec(fact).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let owner = store.config().id_generator.new_uuid().to_string();
    let prepared = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".to_owned(),
            owner_kind: "provider_generation_placement_preparation".to_owned(),
            owner_id: owner.clone(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&prepared, &bytes)
        .unwrap();
    store
        .publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: prepared.pin_id,
            object_hash: hash,
            object_generation: prepared.object_generation,
            owner_kind: "provider_generation_placement_preparation".to_owned(),
            owner_id: owner,
            source_field: "placement".to_owned(),
        })
        .unwrap()
}

fn setup() -> (
    UnleashStore,
    impl std::fmt::Debug,
    Arc<ManualClock>,
    CallerAuthority,
    PoolId,
    unleash_domain::ReceiptId,
) {
    let directory = local_test_dir("f012-generation-placement").unwrap();
    let clock = Arc::new(ManualClock::new(10_000));
    let mut config = StoreConfig::new(directory.path());
    config.clock = clock.clone();
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    let caller = CallerAuthority {
        actor_id: actor(),
        role: CallerRole::HumanOperator,
    };
    let pool = initialize_pool(&store, &caller);
    let mut profile = prepare(&store, 1, None, "exact-model");
    let mut payload = serde_json::to_value(&profile.configuration.payload).unwrap();
    *payload.get_mut("server_management").unwrap() = json!("unleash_managed");
    *payload
        .get_mut("limits")
        .unwrap()
        .get_mut("max_inflight")
        .unwrap() = json!(3);
    profile.configuration.payload = serde_json::from_value(payload).unwrap();
    profile.content_object_hash = publish(&store, &profile.configuration);
    store.commit_command(request(&store, profile, 662)).unwrap();
    let mandate = generation_permission(&store, &caller);
    (store, directory, clock, caller, pool, mandate)
}

fn prepare_probe(
    store: &UnleashStore,
    caller: &CallerAuthority,
    mandate: unleash_domain::ReceiptId,
    id: u128,
) -> (CommandEnvelope, PreparedProviderProbe) {
    let profile = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let mut envelope = probe_command(
        store,
        &profile.profile_version_ref,
        &PROFILE.to_string(),
        "exact-model",
        id,
    )
    .envelope;
    envelope
        .payload
        .insert("mandate_ref".to_owned(), json!(mandate));
    store
        .commit_provider_generation_preparation(&envelope, caller)
        .unwrap();
    store
        .commit_provider_generation_authorizing(&envelope, caller, first_phase)
        .unwrap()
        .unwrap();
    let probe = ProviderProbeId::from_uuid(envelope.aggregate_id);
    let record = store.inspect_provider_probe(probe).unwrap().unwrap();
    (envelope, record)
}

fn admit(unit: AdmissionUnit) -> AdmissionDecision {
    AdmissionDecision::Admit {
        unit,
        deficit_deltas: BTreeMap::from([(None, 0)]),
        consecutive_controls: 0,
        budgets: vec![],
    }
}

fn observe_fixture_eligibility(
    store: &UnleashStore,
    snapshot: &ExecutionAdmissionSnapshot,
    record: &PreparedProviderProbe,
    fact: &serde_json::Value,
    observer: &str,
) -> Uuid {
    let unit = AdmissionUnit::Probe(record.context.probe_id);
    let row = snapshot.queued.iter().find(|row| row.unit == unit).unwrap();
    let evidence_id = store.config().id_generator.new_uuid();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id,
            observer_principal_id: observer.to_owned(),
            object_hash: Sha256Digest::from_bytes(&canonicalize_to_vec(fact).unwrap()),
            unit,
            unit_revision: row.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: record.context.deadline,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    evidence_id
}

#[test]
fn t_f012_generation_exact_placement_admits_only_the_bound_probe() {
    let (store, _directory, _clock, caller, pool, mandate) = setup();
    let (envelope, record) = prepare_probe(&store, &caller, mandate, 663);
    let unit = AdmissionUnit::Probe(record.context.probe_id);
    assert!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .queued
            .is_empty()
    );
    let fact = observation(&store, &record);
    let proof = candidate(&store, &fact);
    assert!(matches!(
        store.adopt_provider_generation_placement(&envelope, &caller, proof.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    store
        .register_principal(OBSERVER, "system", OBSERVER)
        .unwrap();
    for (field, value) in [
        ("model_id", json!("a-different-model")),
        (
            "request_manifest_hash",
            json!(Sha256Digest::from_bytes(b"other")),
        ),
        ("destination", json!("http://127.0.0.1:9999/v1")),
    ] {
        let mut different = fact.clone();
        *different.get_mut(field).unwrap() = value;
        assert!(
            matches!(
                store.adopt_provider_generation_placement(
                    &envelope,
                    &caller,
                    candidate(&store, &different)
                ),
                Err(StoreError::EvidenceUnavailable(_))
            ),
            "{field}"
        );
    }
    assert!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .queued
            .is_empty()
    );
    let id = store
        .adopt_provider_generation_placement(&envelope, &caller, proof.clone())
        .unwrap();
    store
        .register_principal("system:other-placement", "system", "system:other-placement")
        .unwrap();
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    let other =
        observe_fixture_eligibility(&store, &snapshot, &record, &fact, "system:other-placement");
    let substituted = store.execution_admission_snapshot(pool).unwrap();
    assert_eq!(
        substituted.queued.first().unwrap().eligibility_evidence,
        Some(other)
    );
    assert!(matches!(
        store.commit_execution_admission(pool, substituted.token, move |_| Ok(admit(unit))),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .active
            .is_empty()
    );
    assert_eq!(
        store
            .adopt_provider_generation_placement(&envelope, &caller, proof)
            .unwrap(),
        id
    );
    let ready = store.execution_admission_snapshot(pool).unwrap();
    let available = ready.queued.iter().find(|row| row.unit == unit).unwrap();
    assert!(available.eligibility_evidence.is_some());
    let admitted = store
        .commit_execution_admission(pool, ready.token, move |_| Ok(admit(unit)))
        .unwrap();
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    let dispatches: i64 = store
        .read_snapshot(|conn, _| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(dispatches, 0);
}

#[test]
fn t_f012_generation_placement_expiry_during_decision_leaves_no_slot() {
    let (store, _directory, clock, caller, pool, mandate) = setup();
    let (envelope, record) = prepare_probe(&store, &caller, mandate, 663);
    store
        .register_principal(OBSERVER, "system", OBSERVER)
        .unwrap();
    let mut fact = observation(&store, &record);
    *fact.get_mut("valid_until_ms").unwrap() = json!(store.config().clock.now_ms().as_i64() + 10);
    let proof = candidate(&store, &fact);
    store
        .adopt_provider_generation_placement(&envelope, &caller, proof.clone())
        .unwrap();
    let ready = store.execution_admission_snapshot(pool).unwrap();
    let unit = AdmissionUnit::Probe(record.context.probe_id);
    assert!(ready.queued.first().unwrap().eligibility_evidence.is_some());
    assert!(
        store
            .commit_execution_admission(pool, ready.token, move |_| {
                clock.advance_ms(10);
                Ok(admit(unit))
            })
            .is_err()
    );
    let expired = store.execution_admission_snapshot(pool).unwrap();
    assert!(expired.active.is_empty());
    assert_eq!(expired.queued.first().unwrap().eligibility_evidence, None);
    assert!(matches!(
        store.adopt_provider_generation_placement(&envelope, &caller, proof),
        Err(StoreError::EvidenceUnavailable(_))
    ));
}

#[test]
fn t_f012_generation_placement_shares_capacity_across_build_observations() {
    let (store, _directory, _clock, caller, pool, mandate) = setup();
    let first = prepare_probe(&store, &caller, mandate, 663);
    let second = prepare_probe(&store, &caller, mandate, 664);
    let third = prepare_probe(&store, &caller, mandate, 665);
    store
        .register_principal(OBSERVER, "system", OBSERVER)
        .unwrap();
    for ((envelope, record), same_process, admitted) in [
        (&first, true, true),
        (&second, true, false),
        (&third, false, true),
    ] {
        let mut fact = observation(&store, record);
        let server = fact.get_mut("server").unwrap();
        *server.get_mut("build_digest").unwrap() = json!(Sha256Digest::from_bytes(
            record.context.probe_id.to_string().as_bytes()
        ));
        if !same_process {
            *server.get_mut("process_id").unwrap() = json!(1009);
        }
        store
            .adopt_provider_generation_placement(envelope, &caller, candidate(&store, &fact))
            .unwrap();
        let unit = AdmissionUnit::Probe(record.context.probe_id);
        let mut ready = store.execution_admission_snapshot(pool).unwrap();
        if !admitted {
            let row = ready.queued.iter().find(|row| row.unit == unit).unwrap();
            assert!(row.eligibility_evidence.is_none());
            // An otherwise attributable observation must still be clamped by
            // the capacity check inside the committing writer.
            observe_fixture_eligibility(&store, &ready, record, &fact, OBSERVER);
            ready = store.execution_admission_snapshot(pool).unwrap();
        }
        let outcome = store.commit_execution_admission(pool, ready.token, move |_| Ok(admit(unit)));
        if admitted {
            assert!(
                matches!(outcome, Ok(AdmissionOutcome::Admitted(_))),
                "{outcome:?}"
            );
        } else {
            assert!(
                matches!(outcome, Err(StoreError::CapacityBlocked(_))),
                "{outcome:?}"
            );
        }
    }
    assert_eq!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .active
            .len(),
        2
    );
}
