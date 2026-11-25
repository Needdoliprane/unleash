//! Tests the Store boundary with a synthetic observation from a test-only observer.

use super::*;
use serde_json::Value;
use std::collections::BTreeMap;
use unleash_domain::{
    ActionOrigin, AuthorizationEvaluationId, Fence, LineageFence, PoolId, ProviderProbeId,
    ReconcileStrategy, TimestampMs,
};
use unleash_protocol::{ProviderDialectKind, canonical_sha256, canonicalize_to_vec};
use unleash_store::{
    AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit,
    AdmissionUrgency, BrokerCause, BrokerEvaluation, BrokerIntent, BrokerPhase, BrokerPhaseRequest,
    ExecutionAdmissionSnapshot, ExecutionPoolSettings, PolicyIsolationMode, PreparedProviderProbe,
    SettingsPatch, SettingsScopeKind, objects::PublishedObjectReference,
};
use unleash_testkit::ManualClock;

const OBSERVER: &str = "system:provider-generation-placement";

#[path = "provider_probe_generation_placement/outcome.rs"]
mod outcome;

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

fn setup(
    dialect: ProviderDialectKind,
) -> (
    UnleashStore,
    impl std::fmt::Debug,
    Arc<ManualClock>,
    CallerAuthority,
    PoolId,
    unleash_domain::ReceiptId,
) {
    setup_with(dialect, |_| {})
}

fn setup_with(
    dialect: ProviderDialectKind,
    adapt: impl FnOnce(&mut StoreConfig),
) -> (
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
    adapt(&mut config);
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
    if dialect == ProviderDialectKind::OllamaChat {
        *payload.get_mut("provider_kind").unwrap() = json!("ollama");
        *payload.get_mut("dialect").unwrap() = json!({"kind":"ollama_chat","version":1});
        *payload.get_mut("endpoint").unwrap() = json!("http://127.0.0.1:11434/api/chat");
    }
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
fn t_f012_generation_ollama_pins_canonical_post_body() {
    let (store, _directory, _clock, caller, _pool, mandate) =
        setup(ProviderDialectKind::OllamaChat);
    let (_command, record) = prepare_probe(&store, &caller, mandate, 666);
    let expected = canonicalize_to_vec(&json!({
        "model":"exact-model",
        "messages":[{"role":"user","content":"Reply with the word OK."}],
        "stream":true,
        "options":{"num_predict":1},
        "tools":[]
    }))
    .unwrap();
    let stored = store
        .read_verified_object_bytes(record.payload_digest())
        .unwrap();
    assert!(
        stored == expected,
        "published payload differs from the canonical Ollama POST body"
    );
    let hash = Sha256Digest::from_bytes(&expected);
    assert_eq!(record.payload_digest(), hash);

    let manifest_hash = canonical_sha256(&record.request_manifest().unwrap()).unwrap();
    let manifest: Value =
        serde_json::from_slice(&store.read_verified_object_bytes(manifest_hash).unwrap()).unwrap();
    assert_eq!(manifest.get("request_payload_hash"), Some(&json!(hash)));

    let (state, intent, dispatch_count) = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT a.state, b.intent_json,
                        (SELECT COUNT(*) FROM broker_dispatches WHERE action_id=a.action_id)
                 FROM actions a JOIN broker_intents b USING(action_id) WHERE a.action_id=?1",
                [record.context.action_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?)
        })
        .unwrap();
    let intent: BrokerIntent = serde_json::from_str(&intent).unwrap();
    assert_eq!(intent.payload_digest, hash);
    assert_eq!(state, "authorizing");
    assert_eq!(dispatch_count, 0);
}

#[test]
fn t_f012_placement_with_an_unknown_launch_preserves_the_wait() {
    let (store, _root, _clock, caller, pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    store
        .register_principal(OBSERVER, "system", OBSERVER)
        .unwrap();
    let (command, record) = prepare_probe(&store, &caller, mandate, 0xf012_0610);
    let mut fact = observation(&store, &record);
    fact.as_object_mut().unwrap().insert(
        "managed_launch".into(),
        json!({
            "dispatch_id":unleash_domain::DispatchId::from_u128(0xf012_0611),
            "attestation_digest":Sha256Digest::from_bytes(b"no durable launch"),
        }),
    );
    assert!(matches!(
        store.adopt_provider_generation_placement(&command, &caller, candidate(&store, &fact)),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert!(
        store
            .execution_admission_snapshot(pool)
            .unwrap()
            .queued
            .is_empty()
    );
}

fn recorded_service(
    store: &UnleashStore,
    caller: &CallerAuthority,
    template: &CommandEnvelope,
) -> Result<unleash_store::ManagedProviderListener, Box<dyn std::error::Error>> {
    use unleash_store::{
        BrokerAuthorizationCause, BrokerAuthorizationDecision, BrokerAuthorizationOutcome,
        BrokerExecutor, ProviderServiceLaunchFingerprint, ProviderServiceLaunchOutcome,
    };
    let id = 0xf012_0620;
    let mut command = template.clone();
    command.request_id = RequestId::from_u128(id);
    command.command_id = CommandId::from_u128(id);
    command.aggregate_id = Uuid::from_u128(id);
    command.command_type = "PrepareProviderServiceLaunch".into();
    command.payload = serde_json::from_value(json!({
        "profile_id":PROFILE, "profile_version":1, "backend":"macos_seatbelt_strict_service_v1",
        "executable":"/fixture/inert", "executable_digest":Sha256Digest::from_bytes(b"build"),
        "command_digest":Sha256Digest::from_bytes(b"command"),
        "environment_digest":Sha256Digest::from_bytes(b"environment"),
        "confinement_digest":Sha256Digest::from_bytes(b"confinement"),
        "deadline":store.config().clock.now_ms().as_i64()+60_000,
    }))?;
    store.prepare_provider_service_launch(caller.clone(), command.clone())?;
    let context = store.read_provider_service_launch_for_authorization(&command, caller)?;
    store.commit_provider_service_launch_authorizing(&command, caller)?;
    let current = store
        .inspect_broker_action(context.action_id)?
        .ok_or("missing launch action")?;
    let intent = context.broker_intent()?;
    let evaluation = BrokerEvaluation {
        evaluation_id: current.evaluation_id,
        previous_evaluation_id: None,
    };
    let decision = BrokerAuthorizationDecision {
        action_id: context.action_id,
        evaluation_id: evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "placement-fixture-allow".into(),
        policy_revision: context.policy_revision,
        scope_revision: context.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(context.plan.deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let authorization = BrokerPhaseRequest {
        authority_generation: command.authority_generation,
        expected_revision: current.revision,
        evaluation,
        phase: BrokerPhase::Authorization {
            action_id: context.action_id,
            intent_digest: intent.intent_digest,
            outcome: BrokerAuthorizationOutcome::Authorized {
                decision: decision.clone(),
            },
            cause: BrokerAuthorizationCause::Evaluation,
        },
    };
    store.commit_provider_service_launch_broker_phase(&command, caller, authorization.clone())?;
    let dispatch_id = unleash_domain::DispatchId::from_u128(id);
    store.commit_provider_service_launch_dispatch(
        &command,
        caller,
        BrokerPhaseRequest {
            expected_revision: Revision::new(2),
            phase: BrokerPhase::Dispatch {
                action_id: context.action_id,
                intent_digest: intent.intent_digest,
                decision,
                dispatch_id,
                fence: context.lineage_fence,
            },
            ..authorization
        },
        ProviderServiceLaunchFingerprint {
            context_digest: intent.payload_digest,
            executable_digest: context.plan.executable_digest,
            command_digest: context.plan.command_digest,
            environment_digest: context.plan.environment_digest,
            confinement_digest: context.plan.confinement_digest,
        },
    )?;
    let proof = fixture_service_attestation(store, &context)?;
    let child = proof.child.clone();
    store.commit_provider_service_launch_observation(
        context.launch_id,
        dispatch_id,
        ProviderServiceLaunchOutcome::Attested(Box::new(proof)),
    )?;
    Ok(unleash_store::ManagedProviderListener {
        dispatch_id,
        child,
        endpoint: "127.0.0.1:11434".parse()?,
        observed_at: store.config().clock.now_ms(),
    })
}

fn fixture_service_attestation(
    store: &UnleashStore,
    context: &unleash_store::ProviderServiceLaunchContext,
) -> Result<unleash_store::ProviderServiceLaunchAttestation, StoreError> {
    use unleash_store::{ProviderServiceLaunchAttestation, ProviderServiceProcessIdentity};
    let boot_identity = store.read_snapshot(|conn, _| {
        Ok(
            conn.query_row("SELECT boot_identity FROM daemon_state", [], |row| {
                row.get(0)
            })?,
        )
    })?;
    let host = ProviderServiceProcessIdentity {
        pid: 10,
        pgid: 10,
        owner_uid: store.config().owner_uid,
        boot_identity,
        start_identity: "inert-fixture-host".into(),
    };
    let child = ProviderServiceProcessIdentity {
        pid: 20,
        start_identity: "inert-fixture-child".into(),
        ..host.clone()
    };
    Ok(ProviderServiceLaunchAttestation {
        context_digest: canonical_sha256(context)?,
        environment_digest: context.plan.environment_digest,
        confinement_digest: context.plan.confinement_digest,
        host,
        child,
    })
}

#[test]
fn t_f012_staged_managed_admission_is_atomic() -> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, _clock, caller, pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    let (command, probe) = prepare_probe(&store, &caller, mandate, 0xf012_0640);
    let listener = recorded_service(&store, &caller, &command)?;
    let dispatch = listener.dispatch_id;
    let evidence =
        store.stage_managed_provider_generation_placement(&command, &caller, listener)?;
    let before = store.execution_admission_snapshot(pool)?;
    assert!(before.queued.is_empty());
    let expected = AdmissionUnit::Probe(probe.context.probe_id);
    let (waiting, report) = store.admit_staged_managed_generation(
        &command,
        &caller,
        evidence,
        move |snapshot, unit| {
            assert_eq!(unit, expected);
            assert!(
                snapshot
                    .queued
                    .iter()
                    .any(|row| row.unit == unit && row.eligibility_evidence.is_some())
            );
            Ok((AdmissionDecision::Wait, "same queue"))
        },
    )?;
    assert!(waiting.is_none());
    assert_eq!(report, "same queue");
    assert_eq!(
        store.execution_admission_snapshot(pool)?.token,
        before.token
    );
    assert!(
        store
            .admit_staged_managed_generation(&command, &caller, evidence, |_, _| {
                Ok((
                    admit(AdmissionUnit::Probe(ProviderProbeId::from_u128(
                        0xf012_0650,
                    ))),
                    (),
                ))
            },)
            .is_err()
    );
    assert_eq!(
        store.execution_admission_snapshot(pool)?.token,
        before.token
    );
    let (admitted, ()) =
        store.admit_staged_managed_generation(&command, &caller, evidence, |_, unit| {
            Ok((admit(unit), ()))
        })?;
    let admitted = admitted.ok_or("fixture admission missing")?;
    assert_eq!(admitted.commit.unit, expected);
    assert_eq!(admitted.inputs.probe.context_digest, probe.context_digest);
    let placement: Value = serde_json::from_slice(&admitted.inputs.placement)?;
    assert_eq!(
        placement.pointer("/managed_launch/dispatch_id"),
        Some(&json!(dispatch))
    );
    assert_eq!(
        Sha256Digest::from_bytes(&admitted.inputs.body),
        probe.payload_digest()
    );
    assert!(
        store
            .admit_staged_managed_generation(&command, &caller, evidence, |_, _| Ok((
                AdmissionDecision::Wait,
                ()
            )),)
            .is_err()
    );
    let after = store.execution_admission_snapshot(pool)?;
    assert_eq!(after.active.len(), 1);
    assert!(after.queued.is_empty());
    Ok(())
}

#[test]
fn t_f012_staged_placement_expiry_during_admission_restores_the_queue()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, clock, caller, pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    let (command, _) = prepare_probe(&store, &caller, mandate, 0xf012_0660);
    let listener = recorded_service(&store, &caller, &command)?;
    let evidence =
        store.stage_managed_provider_generation_placement(&command, &caller, listener)?;
    let before = store.execution_admission_snapshot(pool)?;
    let clock_for_policy = Arc::clone(&clock);
    assert!(
        store
            .admit_staged_managed_generation(&command, &caller, evidence, move |snapshot, unit| {
                assert!(
                    snapshot
                        .queued
                        .iter()
                        .any(|row| row.unit == unit && row.eligibility_evidence.is_some())
                );
                clock_for_policy.advance_ms(60_000);
                Ok((admit(unit), ()))
            })
            .is_err()
    );
    assert!(
        store
            .admit_staged_managed_generation(&command, &caller, evidence, |_, unit| Ok((
                admit(unit),
                ()
            )),)
            .is_err()
    );
    let after = store.execution_admission_snapshot(pool)?;
    assert_eq!(after.token.inputs, before.token.inputs);
    assert_eq!(after.token.observations, before.token.observations);
    assert!(after.active.is_empty() && after.queued.is_empty());
    Ok(())
}

#[test]
fn t_f012_managed_dispatch_binds_native_request_once() -> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, _clock, caller, _pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    let (handoff, request) = authorized_generation_fixture(&store, &caller, mandate)?;
    let transport = handoff
        .inputs
        .managed_transport
        .as_ref()
        .ok_or("missing native binding")?;
    let digest = transport.request_digest;
    let action = handoff.inputs.probe.context.action_id;
    assert!(store.commit_broker_phase(request.clone()).is_err());
    assert!(
        store
            .commit_managed_generation_dispatch(
                &handoff.commit,
                request.clone(),
                Sha256Digest::from_bytes(b"another request"),
            )
            .is_err()
    );
    let mut divergent = (*handoff.commit).clone();
    divergent.reservation_id = unleash_domain::ReservationId::from_u128(73);
    assert!(
        store
            .commit_managed_generation_dispatch(&divergent, request.clone(), digest)
            .is_err()
    );
    assert!(
        store
            .inspect_broker_action(action)?
            .ok_or("missing action")?
            .dispatch_id
            .is_none()
    );

    let permit =
        store.commit_managed_generation_dispatch(&handoff.commit, request.clone(), digest)?;
    assert_eq!(permit.transport_digest(), digest);
    let observed = store
        .inspect_broker_action(action)?
        .ok_or("missing action")?;
    assert_eq!(observed.dispatch_id, Some(permit.dispatch_id()));
    assert_eq!(observed.last_event_cursor, permit.commit().event_cursor);
    let sequence = i64::try_from(permit.commit().event_cursor.sequence.as_u64())?;
    let event: Value = store.read_snapshot(move |conn, _| {
        let raw: String = conn.query_row(
            "SELECT e.payload_json FROM events e JOIN outbox o ON o.event_id=e.event_id
             WHERE e.sequence=?1 AND e.event_type='ActionDispatchCommitted' AND o.destination='clients'",
            [sequence], |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    })?;
    assert_eq!(
        event.pointer("/managed_generation/transport_digest"),
        Some(&json!(digest))
    );
    assert_eq!(
        event.pointer("/managed_generation/admission_digest"),
        Some(&json!(handoff.inputs.transport_placement.admission_digest))
    );
    assert_eq!(
        event.pointer("/managed_generation/body_digest"),
        Some(&json!(handoff.inputs.probe.payload_digest()))
    );
    assert!(
        store
            .commit_managed_generation_dispatch(&handoff.commit, request.clone(), digest)
            .is_err()
    );
    assert_managed_dispatch_requires_typed_observation(&store, &request)?;
    Ok(())
}

#[test]
fn t_f012_managed_dispatch_reply_loss_preserves_the_only_marker()
-> Result<(), Box<dyn std::error::Error>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use unleash_store::{CutpointObserver, StoreCutpoint};

    #[derive(Default)]
    struct LoseReply(AtomicBool);
    impl CutpointObserver for LoseReply {
        fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
            if point == StoreCutpoint::C4AfterBrokerDispatchCommit
                && self.0.swap(false, Ordering::AcqRel)
            {
                return Err(StoreError::CutpointInjected {
                    cutpoint: "C4".into(),
                    message: "lost fixture acknowledgement".into(),
                });
            }
            Ok(())
        }
    }

    let failure = Arc::new(LoseReply::default());
    let (store, _root, _clock, caller, _pool, mandate) =
        setup_with(ProviderDialectKind::OllamaChat, |config| {
            config.cutpoints = failure.clone();
        });
    let (handoff, request) = authorized_generation_fixture(&store, &caller, mandate)?;
    let digest = handoff
        .inputs
        .managed_transport
        .ok_or("missing native binding")?
        .request_digest;
    failure.0.store(true, Ordering::Release);
    assert!(matches!(
        store.commit_managed_generation_dispatch(&handoff.commit, request.clone(), digest),
        Err(StoreError::CutpointInjected { cutpoint, .. }) if cutpoint == "C4"
    ));
    let action = handoff.inputs.probe.context.action_id;
    let committed = store
        .inspect_broker_action(action)?
        .ok_or("missing durable action")?;
    assert_eq!(committed.state, unleash_domain::ActionState::Dispatching);
    assert!(committed.dispatch_id.is_some());
    assert!(
        store
            .commit_managed_generation_dispatch(&handoff.commit, request, digest)
            .is_err()
    );
    assert_eq!(store.inspect_broker_action(action)?, Some(committed));
    Ok(())
}

fn authorized_generation_fixture(
    store: &UnleashStore,
    caller: &CallerAuthority,
    mandate: unleash_domain::ReceiptId,
) -> Result<
    (
        unleash_store::ManagedGenerationAdmission,
        BrokerPhaseRequest,
    ),
    Box<dyn std::error::Error>,
> {
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    let (command, probe) = prepare_probe(store, caller, mandate, 0xf012_0670);
    let listener = recorded_service(store, caller, &command)?;
    let evidence = store.stage_managed_provider_generation_placement(&command, caller, listener)?;
    let (handoff, ()) =
        store.admit_staged_managed_generation(&command, caller, evidence, |_, unit| {
            Ok((admit(unit), ()))
        })?;
    let handoff = handoff.ok_or("fixture admission missing")?;
    let request = authorize_generation_dispatch(store, &probe)?;
    Ok((handoff, request))
}

fn authorize_generation_dispatch(
    store: &UnleashStore,
    probe: &PreparedProviderProbe,
) -> Result<BrokerPhaseRequest, Box<dyn std::error::Error>> {
    use unleash_store::{
        BrokerAuthorizationCause, BrokerAuthorizationDecision, BrokerAuthorizationOutcome,
        BrokerExecutor,
    };
    let context = &probe.context;
    let current = store
        .inspect_broker_action(context.action_id)?
        .ok_or("missing probe action")?;
    let evaluation = BrokerEvaluation {
        evaluation_id: current.evaluation_id,
        previous_evaluation_id: None,
    };
    let decision = BrokerAuthorizationDecision {
        action_id: context.action_id,
        evaluation_id: current.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "generation-transport-fixture".into(),
        policy_revision: context.policy_revision,
        scope_revision: context.scope_revision,
        payload_digest: probe.payload_digest(),
        expiry: Some(context.deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let authorization = BrokerPhaseRequest {
        authority_generation: context.authority_generation,
        expected_revision: current.revision,
        evaluation,
        phase: BrokerPhase::Authorization {
            action_id: context.action_id,
            intent_digest: current.intent_digest,
            outcome: BrokerAuthorizationOutcome::Authorized {
                decision: decision.clone(),
            },
            cause: BrokerAuthorizationCause::Evaluation,
        },
    };
    store.commit_broker_phase(authorization.clone())?;
    Ok(BrokerPhaseRequest {
        expected_revision: Revision::new(2),
        phase: BrokerPhase::Dispatch {
            action_id: context.action_id,
            intent_digest: current.intent_digest,
            decision,
            dispatch_id: unleash_domain::DispatchId::from_u128(0xf012_0680),
            fence: current.fence,
        },
        ..authorization
    })
}

fn assert_managed_dispatch_requires_typed_observation(
    store: &UnleashStore,
    request: &BrokerPhaseRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    use unleash_store::{BrokerActionReceipt, BrokerObservationCause};
    let BrokerPhase::Dispatch {
        action_id,
        intent_digest,
        decision,
        dispatch_id,
        ..
    } = &request.phase
    else {
        return Err("expected fixture dispatch".into());
    };
    let observation = BrokerPhaseRequest {
        expected_revision: Revision::new(3),
        phase: BrokerPhase::Observation {
            action_id: *action_id,
            intent_digest: *intent_digest,
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
            receipt: BrokerActionReceipt {
                action_id: *action_id,
                evaluation_id: request.evaluation.evaluation_id,
                dispatch_id: *dispatch_id,
                decision_ref: decision.decision_ref.clone(),
                state: unleash_domain::ActionState::OutcomeUnknown,
                output_digest: None,
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: true,
                observed_at_ms: store.config().clock.now_ms(),
            },
        },
        ..request.clone()
    };
    assert!(store.commit_broker_phase(observation).is_err());
    assert_eq!(
        store
            .inspect_broker_action(*action_id)?
            .ok_or("missing dispatch")?
            .state,
        unleash_domain::ActionState::Dispatching,
    );
    Ok(())
}

#[test]
fn t_f012_admitted_managed_ollama_retains_launch_binding() -> Result<(), Box<dyn std::error::Error>>
{
    let (store, _root, _clock, caller, pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    let (command, probe) = prepare_probe(&store, &caller, mandate, 0xf012_0630);
    let mut listener = recorded_service(&store, &caller, &command)?;
    let correct_endpoint = listener.endpoint;
    listener.endpoint.set_port(11435);
    assert!(
        store
            .adopt_managed_provider_generation_placement(&command, &caller, listener.clone())
            .is_err()
    );
    listener.endpoint = correct_endpoint;
    let correct_child = listener.child.clone();
    listener.child.start_identity = "other-fixture-child".into();
    assert!(
        store
            .adopt_managed_provider_generation_placement(&command, &caller, listener.clone())
            .is_err()
    );
    assert!(store.execution_admission_snapshot(pool)?.queued.is_empty());
    listener.child = correct_child;
    let launch = listener.dispatch_id;
    store.adopt_managed_provider_generation_placement(&command, &caller, listener)?;
    let snapshot = store.execution_admission_snapshot(pool)?;
    let unit = AdmissionUnit::Probe(probe.context.probe_id);
    let AdmissionOutcome::Admitted(admission) =
        store.commit_execution_admission(pool, snapshot.token, move |_| Ok(admit(unit)))?
    else {
        return Err("fixture admission missing".into());
    };
    let inputs = store.read_admitted_managed_ollama_chat(&admission)?;
    let fact: Value = serde_json::from_slice(&inputs.placement)?;
    assert_eq!(
        fact.pointer("/managed_launch/dispatch_id"),
        Some(&json!(launch))
    );
    assert_transport_placement(&store, &admission, &inputs, &fact)?;
    Ok(())
}

#[test]
fn t_f012_generation_exact_placement_admits_only_the_bound_probe() {
    let (store, _directory, _clock, caller, pool, mandate) =
        setup(ProviderDialectKind::OpenaiChatCompletions);
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

fn assert_transport_placement(
    store: &UnleashStore,
    admission: &unleash_store::AdmissionCommit,
    inputs: &unleash_store::AdmittedOllamaChatInputs,
    fact: &Value,
) -> Result<(), StoreError> {
    let selected = &inputs.transport_placement;
    for (path, value) in [
        ("/destination", json!(selected.endpoint)),
        ("/server/boot_identity", json!(selected.boot_identity)),
        ("/server/process_id", json!(selected.process_id)),
        (
            "/server/process_start_identity",
            json!(selected.process_start_identity),
        ),
    ] {
        assert_eq!(fact.pointer(path), Some(&value));
    }
    assert_eq!(
        selected.evidence_digest,
        Sha256Digest::from_bytes(&inputs.placement)
    );
    let event_digest: String = store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT payload_hash FROM events WHERE stream_id=?1 AND sequence=?2",
            rusqlite::params![
                admission.event_cursor.stream_id.to_string(),
                i64::try_from(admission.event_cursor.sequence.as_u64()).unwrap()
            ],
            |row| row.get(0),
        )?)
    })?;
    assert_eq!(selected.admission_digest.to_string(), event_digest);
    Ok(())
}

#[test]
fn t_f012_admitted_ollama_reads_original_body_and_placement()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _directory, clock, caller, pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    let (command, record) = prepare_probe(&store, &caller, mandate, 667);
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    let mut fact = observation(&store, &record);
    let placement_expiry = TimestampMs::new(store.config().clock.now_ms().as_i64() + 10);
    *fact.get_mut("valid_until_ms").unwrap() = json!(placement_expiry);
    let expected_placement = canonicalize_to_vec(&fact)?;
    let proof = candidate(&store, &fact);
    store.adopt_provider_generation_placement(&command, &caller, proof)?;
    let snapshot = store.execution_admission_snapshot(pool)?;
    let unit = AdmissionUnit::Probe(record.context.probe_id);
    let AdmissionOutcome::Admitted(admission) =
        store.commit_execution_admission(pool, snapshot.token, move |_| Ok(admit(unit)))?
    else {
        return Err("fixture admission absent".into());
    };

    let inputs = store.read_admitted_ollama_chat(&admission)?;
    assert!(matches!(
        store.read_admitted_managed_ollama_chat(&admission),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(inputs.probe.context_digest, record.context_digest);
    assert_eq!(inputs.placement, expected_placement);
    assert!(placement_expiry < admission.lease.expires_at_ms);
    assert!(placement_expiry < record.context.deadline);
    assert_eq!(inputs.transport_placement.valid_until_ms, placement_expiry);
    assert_transport_placement(&store, &admission, &inputs, &fact)?;
    let expected_body = canonicalize_to_vec(&json!({
        "model":"exact-model","messages":[{"role":"user","content":"Reply with the word OK."}],
        "stream":true,"options":{"num_predict":1},"tools":[]
    }))?;
    assert_eq!(inputs.body, expected_body);
    assert_eq!(
        Sha256Digest::from_bytes(&inputs.body),
        record.payload_digest()
    );

    let mut wrong_reservation = admission.as_ref().clone();
    wrong_reservation.reservation_id = unleash_domain::ReservationId::from_u128(901);
    let mut wrong_fence = admission.as_ref().clone();
    wrong_fence.lease.fence = Fence::new(admission.lease.fence.as_u64() + 1);
    let mut wrong_event = admission.as_ref().clone();
    wrong_event.event_cursor.sequence =
        unleash_domain::Sequence::new(admission.event_cursor.sequence.as_u64() - 1);
    let mut wrong_expiry = admission.as_ref().clone();
    wrong_expiry.lease.expires_at_ms = TimestampMs::new(admission.lease.expires_at_ms.as_i64() + 1);
    let mut wrong_owner = admission.as_ref().clone();
    wrong_owner.lease.owner_id = ProviderProbeId::from_u128(902).to_string();
    let mut wrong_key = admission.as_ref().clone();
    wrong_key.lease.resource_key = "provider_probe:other-fixture".to_owned();
    let mut wrong_state = admission.as_ref().clone();
    wrong_state.lease.state = unleash_store::LeaseState::Released;
    for (field, mismatch) in [
        ("reservation", wrong_reservation),
        ("fence", wrong_fence),
        ("event", wrong_event),
        ("lease expiry", wrong_expiry),
        ("lease owner", wrong_owner),
        ("lease key", wrong_key),
        ("lease state", wrong_state),
    ] {
        assert!(
            matches!(
                store.read_admitted_ollama_chat(&mismatch),
                Err(StoreError::EvidenceUnavailable(_))
            ),
            "{field} must match the held lease"
        );
    }
    clock.advance_ms(10);
    assert!(matches!(
        store.read_admitted_ollama_chat(&admission),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(store.execution_admission_snapshot(pool)?.active.len(), 1);
    let dispatches: i64 = store.read_snapshot(|db, _| {
        Ok(
            db.query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
                row.get(0)
            })?,
        )
    })?;
    assert_eq!(dispatches, 0);
    Ok(())
}

#[test]
fn t_f012_generation_placement_expiry_during_decision_leaves_no_slot() {
    let (store, _directory, clock, caller, pool, mandate) =
        setup(ProviderDialectKind::OpenaiChatCompletions);
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
    let (store, _directory, _clock, caller, pool, mandate) =
        setup(ProviderDialectKind::OpenaiChatCompletions);
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
