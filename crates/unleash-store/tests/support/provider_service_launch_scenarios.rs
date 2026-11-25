use super::*;
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, AuthorizationEvaluationId, DispatchId, ReconcileStrategy,
};
use unleash_store::{
    BrokerAuthorizationCause, BrokerAuthorizationDecision, BrokerAuthorizationOutcome, BrokerCause,
    BrokerEvaluation, BrokerExecutor, BrokerIntent, BrokerPhase, BrokerPhaseRequest,
    ProviderServiceLaunchContext, ProviderServiceLaunchFingerprint,
};

const MANAGED: Uuid = Uuid::from_u128(0xf012_c519);
const LAUNCH: Uuid = Uuid::from_u128(0xf012_c520);

fn configure_ollama(store: &UnleashStore, endpoint: &str) {
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: MANAGED,
        profile_version: 1,
        project_id: None,
        payload: serde_json::from_value::<unleash_protocol::SetProviderProfilePayload>(json!({
            "provider_kind": "ollama",
            "dialect": {"kind": "ollama_chat", "version": 1},
            "endpoint": endpoint,
            "model": "fixture",
            "locality": "local_only",
            "server_management": "unleash_managed",
            "auth": {"kind": "none"},
            "limits": {"max_inflight": 1, "connect_timeout_ms": 2000, "queue_timeout_ms": 1000,
                "load_timeout_ms": 180_000, "first_token_timeout_ms": 60000, "generation_timeout_ms": 60000},
            "declared_capabilities": ["text"],
            "pricing": {"kind": "none_local"}
        }))
        .unwrap().with_normalized_endpoint().unwrap(),
        secret_binding: None,
        catalog_binding: None,
    };
    publish_profile(store, configuration, 0xc519);
}

fn launch(store: &UnleashStore, command: u128, profile: Uuid) -> CommandEnvelope {
    envelope(
        store,
        command,
        LAUNCH,
        "PrepareProviderServiceLaunch",
        json!({
            "profile_id": profile,
            "profile_version": 1,
            "backend": "macos_seatbelt_strict_service_v1",
            "executable": "/fixture/ollama",
            "executable_digest": Sha256Digest::from_bytes(b"fixture executable"),
            "command_digest": Sha256Digest::from_bytes(b"fixture command"),
            "environment_digest": Sha256Digest::from_bytes(b"fixture environment"),
            "confinement_digest": Sha256Digest::from_bytes(b"fixture confinement"),
            "deadline": store.config().clock.now_ms().as_i64() + 60_000,
        }),
    )
}

#[test]
fn permit_commits_once_and_requires_exact_bootstrap_and_decision() {
    let (store, _directory, _mandate, failure) = fixture();
    let (command, context, request, fingerprint) = authorize_for_permit(&store);
    let mut wrong = fingerprint;
    wrong.confinement_digest = Sha256Digest::from_bytes(b"different fixture profile");
    assert!(
        store
            .commit_provider_service_launch_dispatch(&command, &caller(), request.clone(), wrong)
            .is_err()
    );
    let mut wrong_decision = request.clone();
    if let BrokerPhase::Dispatch { decision, .. } = &mut wrong_decision.phase {
        decision.decision_ref = "different-fixture-decision".into();
    }
    assert!(
        store
            .commit_provider_service_launch_dispatch(
                &command,
                &caller(),
                wrong_decision,
                fingerprint
            )
            .is_err()
    );
    assert_eq!(launch_dispatch_counts(&store), (0, 0, 0, 0));

    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_provider_service_launch_dispatch(
            &command,
            &caller(),
            request.clone(),
            fingerprint
        ),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(launch_dispatch_counts(&store), (0, 0, 0, 0));
    let permit = store
        .commit_provider_service_launch_dispatch(&command, &caller(), request.clone(), fingerprint)
        .unwrap();
    assert_eq!(permit.context(), &context);
    let state = store
        .inspect_broker_action(context.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.dispatch_id, Some(permit.dispatch_id()));
    assert_eq!(state.last_event_cursor, permit.commit().event_cursor);
    assert_eq!(permit.expires_at(), context.plan.deadline);
    assert_eq!(launch_dispatch_counts(&store), (1, 1, 0, 1));
    assert!(
        store
            .commit_provider_service_launch_dispatch(
                &command,
                &caller(),
                request.clone(),
                fingerprint
            )
            .is_err()
    );
    assert!(store.commit_broker_phase(request).is_err());
    assert_eq!(launch_dispatch_counts(&store), (1, 1, 0, 1));
}

#[test]
fn permit_reply_loss_does_not_create_another_launch_permit() {
    let (store, _directory, _mandate, failure) = fixture();
    let (command, context, request, fingerprint) = authorize_for_permit(&store);
    failure.1.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_provider_service_launch_dispatch(&command, &caller(), request.clone(), fingerprint),
        Err(StoreError::CutpointInjected { cutpoint, .. }) if cutpoint == "C4"
    ));
    let state = store
        .inspect_broker_action(context.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.state, ActionState::Dispatching);
    assert!(state.dispatch_id.is_some());
    assert!(
        store
            .commit_provider_service_launch_dispatch(&command, &caller(), request, fingerprint)
            .is_err()
    );
    assert_eq!(launch_dispatch_counts(&store), (1, 1, 0, 1));
}

fn authorize_for_permit(
    store: &UnleashStore,
) -> (
    CommandEnvelope,
    ProviderServiceLaunchContext,
    BrokerPhaseRequest,
    ProviderServiceLaunchFingerprint,
) {
    configure_ollama(store, "http://127.0.0.1:11434");
    let command = launch(store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), command.clone())
        .unwrap();
    let context = store
        .read_provider_service_launch_for_authorization(&command, &caller())
        .unwrap();
    let (first, authorization, decision) = evaluation_entries(&context);
    store
        .commit_provider_service_launch_broker_phase(&command, &caller(), first)
        .unwrap();
    store
        .commit_provider_service_launch_broker_phase(&command, &caller(), authorization.clone())
        .unwrap();
    let dispatch = BrokerPhaseRequest {
        expected_revision: Revision::new(2),
        phase: BrokerPhase::Dispatch {
            action_id: context.action_id,
            intent_digest: context.broker_intent().unwrap().intent_digest,
            decision,
            dispatch_id: DispatchId::from_u128(0xc541),
            fence: context.lineage_fence,
        },
        ..authorization
    };
    let fingerprint = ProviderServiceLaunchFingerprint {
        context_digest: unleash_protocol::canonical_sha256(&context).unwrap(),
        executable_digest: context.plan.executable_digest,
        command_digest: context.plan.command_digest,
        environment_digest: context.plan.environment_digest,
        confinement_digest: context.plan.confinement_digest,
    };
    (command, context, dispatch, fingerprint)
}

fn launch_dispatch_counts(store: &UnleashStore) -> (i64, i64, i64, i64) {
    store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT (SELECT COUNT(*) FROM action_dispatches), (SELECT COUNT(*) FROM broker_dispatches),
                (SELECT COUNT(*) FROM processes),
                (SELECT COUNT(*) FROM events WHERE event_type='ActionDispatchCommitted')",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    }).unwrap()
}

#[test]
fn decision_commits_once_and_the_launch_port_stays_closed() {
    let (store, _directory, _mandate, failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let context = store
        .read_provider_service_launch_for_authorization(&wire, &caller())
        .unwrap();
    let (first, result, decision) = evaluation_entries(&context);
    store
        .commit_provider_service_launch_broker_phase(&wire, &caller(), first)
        .unwrap();
    let before = counts(&store);
    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_provider_service_launch_broker_phase(&wire, &caller(), result.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(
        store
            .inspect_broker_action(context.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorizing
    );
    assert_eq!(counts(&store), before);
    let committed = store
        .commit_provider_service_launch_broker_phase(&wire, &caller(), result.clone())
        .unwrap();
    let state = store
        .inspect_broker_action(context.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.state, ActionState::Authorized);
    assert_eq!(state.evaluation_id, result.evaluation.evaluation_id);
    let after = counts(&store);
    let replay = store
        .commit_provider_service_launch_broker_phase(&wire, &caller(), result.clone())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert!(matches!(
        store.commit_broker_phase(result.clone()),
        Err(StoreError::InvalidArgument { .. })
    ));
    let dispatch = BrokerPhaseRequest {
        expected_revision: committed.revision,
        phase: BrokerPhase::Dispatch {
            action_id: state.action_id,
            intent_digest: state.intent_digest,
            decision,
            dispatch_id: DispatchId::from_u128(0xc541),
            fence: state.fence,
        },
        ..result
    };
    assert!(
        store
            .commit_provider_service_launch_broker_phase(&wire, &caller(), dispatch.clone())
            .is_err()
    );
    assert!(store.commit_broker_phase(dispatch).is_err());
    assert_eq!(counts(&store), after);
    let writes: (i64, i64) = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT (SELECT COUNT(*) FROM action_dispatches), (SELECT COUNT(*) FROM processes)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(writes, (0, 0));
}

fn evaluation_entries(
    context: &ProviderServiceLaunchContext,
) -> (
    BrokerPhaseRequest,
    BrokerPhaseRequest,
    BrokerAuthorizationDecision,
) {
    let intent = context.broker_intent().unwrap();
    let evaluation = BrokerEvaluation {
        evaluation_id: AuthorizationEvaluationId::from_u128(0xc540),
        previous_evaluation_id: None,
    };
    let decision = BrokerAuthorizationDecision {
        action_id: context.action_id,
        evaluation_id: evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "inert-service-fixture".into(),
        policy_revision: context.policy_revision,
        scope_revision: context.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(context.plan.deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let result = BrokerPhaseRequest {
        authority_generation: context.lineage_fence.authority_generation,
        expected_revision: Revision::new(1),
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
    let first = BrokerPhaseRequest {
        expected_revision: Revision::INITIAL,
        phase: BrokerPhase::Authorizing { intent },
        ..result.clone()
    };
    (first, result, decision)
}

#[test]
fn decision_requires_exact_payload_and_current_policy() {
    let (store, _directory, mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let context = store
        .read_provider_service_launch_for_authorization(&wire, &caller())
        .unwrap();
    let (first, result, _) = evaluation_entries(&context);
    store
        .commit_provider_service_launch_broker_phase(&wire, &caller(), first)
        .unwrap();
    let mut different = result.clone();
    if let BrokerPhase::Authorization {
        outcome: BrokerAuthorizationOutcome::Authorized { decision },
        ..
    } = &mut different.phase
    {
        decision.payload_digest = Sha256Digest::from_bytes(b"separate fixture");
    }
    let before = counts(&store);
    assert!(matches!(
        store.commit_provider_service_launch_broker_phase(&wire, &caller(), different),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&store), before);
    store
        .commit_command(policy(
            &store,
            3,
            1,
            ProviderProbeMandateChange::Revoke {
                mandate_ref: mandate,
            },
        ))
        .unwrap();
    let after_policy_change = counts(&store);
    assert!(matches!(
        store.commit_provider_service_launch_broker_phase(&wire, &caller(), result),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(counts(&store), after_policy_change);
    assert_eq!(
        store
            .inspect_broker_action(context.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorizing
    );
}

#[test]
fn authorizing_rolls_back_and_replays_one_durable_phase() {
    let (store, _directory, _mandate, failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let before = counts(&store);
    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_provider_service_launch_authorizing(&wire, &caller()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert!(
        store
            .inspect_broker_action(context.action_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(counts(&store), before);

    let committed = store
        .commit_provider_service_launch_authorizing(&wire, &caller())
        .unwrap();
    assert!(!committed.replayed);
    assert_eq!(committed.action_id, context.action_id);
    let state = store
        .inspect_broker_action(context.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(state.state, ActionState::Authorizing);
    assert_eq!(state.last_event_cursor, committed.event_cursor);
    let after = counts(&store);
    let replay = store
        .commit_provider_service_launch_authorizing(&wire, &caller())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(replay.revision, committed.revision);
    assert_eq!(counts(&store), after);
    let native_effects: (i64, i64, i64) = store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT (SELECT COUNT(*) FROM broker_phase_records), (SELECT COUNT(*) FROM action_dispatches),
                (SELECT COUNT(*) FROM processes)",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }).unwrap();
    assert_eq!(native_effects, (1, 0, 0));
}

#[test]
fn authorizing_records_strict_context_while_generic_writer_stays_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    store
        .commit_provider_service_launch_authorizing(&wire, &caller())
        .unwrap();
    let encoded: String = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT phase_json FROM broker_phase_records WHERE action_id=?1",
                [context.action_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let value: Value = serde_json::from_str(&encoded).unwrap();
    let phase: BrokerPhase = serde_json::from_value(value.get("phase").unwrap().clone()).unwrap();
    let BrokerPhase::Authorizing { intent } = &phase else {
        return Err("expected first phase".into());
    };
    assert_eq!(
        intent.payload_digest,
        unleash_protocol::canonical_sha256(&context).unwrap()
    );
    assert_eq!(
        intent.cause,
        BrokerCause::CommandReceipt {
            receipt_id: context.receipt_id
        }
    );
    assert_eq!(
        intent
            .manifest
            .get("isolation_requirements")
            .and_then(|requirements| requirements.get("mode")),
        Some(&json!("strict"))
    );
    let before = counts(&store);
    assert!(matches!(
        store.commit_broker_phase(BrokerPhaseRequest {
            authority_generation: context.lineage_fence.authority_generation,
            expected_revision: Revision::INITIAL,
            evaluation: serde_json::from_value(value.get("evaluation").unwrap().clone()).unwrap(),
            phase,
        }),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&store), before);
    Ok(())
}

#[test]
fn read_for_authorization_requires_the_original_command_and_admin() {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let before = counts(&store);
    let historical = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    assert_eq!(
        store
            .read_provider_service_launch_for_authorization(&wire, &caller())
            .unwrap(),
        historical
    );
    let mut different_plan = wire.clone();
    different_plan
        .payload
        .insert("executable".to_owned(), json!("/fixture/second-ollama"));
    assert!(matches!(
        store.read_provider_service_launch_for_authorization(&different_plan, &caller()),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    let mut other = caller();
    other.actor_id = ActorId::new("user:connect-other").unwrap();
    let mut other_wire = wire.clone();
    other_wire.actor_ref = Some(other.actor_id.to_string());
    assert!(matches!(
        store.read_provider_service_launch_for_authorization(&other_wire, &other),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let mut agent = caller();
    agent.role = CallerRole::Agent;
    assert!(matches!(
        store.read_provider_service_launch_for_authorization(&wire, &agent),
        Err(StoreError::Unauthorized(_))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn read_for_authorization_refuses_clock_or_policy_change_and_preserves_history() {
    let (store, _directory, mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let historical = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let before = counts(&store);
    store
        .config()
        .clock_uncertain
        .store(true, Ordering::Release);
    assert!(matches!(
        store.read_provider_service_launch_for_authorization(&wire, &caller()),
        Err(StoreError::RecoveryInProgress(_))
    ));
    assert_eq!(counts(&store), before);
    store
        .config()
        .clock_uncertain
        .store(false, Ordering::Release);
    store
        .commit_command(policy(
            &store,
            3,
            1,
            ProviderProbeMandateChange::Revoke {
                mandate_ref: mandate,
            },
        ))
        .unwrap();
    let before_read = counts(&store);
    assert!(matches!(
        store.read_provider_service_launch_for_authorization(&wire, &caller()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(counts(&store), before_read);
    assert_eq!(
        store.read_provider_service_launch(LAUNCH).unwrap(),
        Some(historical)
    );
}

#[test]
fn durable_preparation_rolls_back_and_replays_without_an_effect() {
    let (store, _directory, _mandate, failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    let before = counts(&store);
    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), wire.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);

    let committed = store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let receipt = committed.receipt();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-service-launch:{LAUNCH}"))
    );
    assert_eq!(context.receipt_id, receipt.receipt_id);
    assert_eq!(context.plan.profile_id, MANAGED);
    assert_eq!(context.plan.profile_version, 1);
    assert_eq!(context.endpoint, "http://127.0.0.1:11434/");
    assert_eq!(context.principal_id.as_str(), PRINCIPAL);
    let mut expected = before.clone();
    for (table, delta) in [(0, 1), (8, 1), (9, 2)] {
        *expected.get_mut(table).unwrap() += delta;
    }
    assert_eq!(counts(&store), expected);
    let process_count: i64 = store
        .read_snapshot(|db, _| {
            Ok(db.query_row("SELECT COUNT(*) FROM processes", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(process_count, 0);

    let replay = store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    assert!(matches!(
        replay,
        unleash_store::CommitOutcome::IdempotentReplay(_)
    ));
    assert_eq!(replay.receipt(), receipt);
    assert_eq!(
        store.read_provider_service_launch(LAUNCH).unwrap(),
        Some(context)
    );
    let mut changed = wire;
    changed
        .payload
        .insert("executable".to_owned(), json!("/other/ollama"));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), changed),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), launch(&store, 0xc521, MANAGED)),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(counts(&store), expected);
}

#[test]
fn preparation_rejects_unmanaged_profiles_expired_plans_and_non_admins() {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let before = counts(&store);
    let wire = launch(&store, 0xc520, MANAGED);
    let mut agent = caller();
    agent.role = CallerRole::Agent;
    assert!(matches!(
        store.prepare_provider_service_launch(agent, wire.clone()),
        Err(StoreError::Unauthorized(_))
    ));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), launch(&store, 0xc520, PROFILE)),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut expired = wire.clone();
    expired.payload.insert("deadline".to_owned(), json!(1));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), expired),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut with_probe_receipt = wire;
    with_probe_receipt
        .payload
        .insert("probe_receipt".to_owned(), json!(Uuid::nil()));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), with_probe_receipt),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);
}

fn intent(
    context: &ProviderServiceLaunchContext,
    action: ActionId,
    receipt: ReceiptId,
    verb: &str,
) -> BrokerIntent {
    let payload_digest = unleash_protocol::canonical_sha256(context).unwrap();
    let manifest = json!({
        "action_id": action,
        "subject": {"kind":"control_plane","principal_id":context.principal_id,"cause_ref":format!("receipt:{receipt}"),
            "invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id": context.project_id, "verb": verb,
        "resource_refs": [{"kind": "path", "value": context.plan.executable}],
        "payload_digest": payload_digest, "reconcile_strategy":"observe_then_decide", "effects_class":"process_exec",
        "scope_revision": context.scope_revision, "policy_revision": context.policy_revision,
        "lineage_fence": context.lineage_fence, "action_deadline": context.plan.deadline,
        "isolation_requirements": {"mode":"standard","require_process_confinement":false,"required_capabilities":[]},
        "secret_refs": [], "provenance": {"origin":"engine_internal"}
    }).as_object().unwrap().clone();
    BrokerIntent {
        action_id: action,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: context.principal_id.clone(),
        },
        principal_id: context.principal_id.clone(),
        cause: BrokerCause::CommandReceipt {
            receipt_id: receipt,
        },
        project_id: context.project_id,
        payload_digest,
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: context.lineage_fence,
        action_deadline: context.plan.deadline,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        intent_digest: unleash_protocol::canonical_sha256(&manifest).unwrap(),
        manifest,
    }
}

#[test]
fn broker_refuses_service_intents_and_repurposing_prepared_action_or_receipt() {
    let (store, _directory, mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    store
        .prepare_provider_service_launch(caller(), launch(&store, 0xc520, MANAGED))
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let other_action = ActionId::from_u128(0xc522);
    let before = counts(&store);
    for (action, receipt, verb, refusal) in [
        (
            other_action,
            mandate,
            "model.provider.service.launch",
            "service launch execution is not qualified",
        ),
        (
            context.action_id,
            context.receipt_id,
            "model.provider.service.launch",
            "service launch execution is not qualified",
        ),
        (
            other_action,
            context.receipt_id,
            "fixture.other",
            "service launch preparation cannot authorize another effect",
        ),
        (
            context.action_id,
            mandate,
            "fixture.other",
            "service launch preparation cannot authorize another effect",
        ),
    ] {
        let result =
            store.commit_broker_phase(authorizing(intent(&context, action, receipt, verb), 0xc523));
        assert!(
            matches!(result, Err(StoreError::InvalidArgument { message, .. }) if message == refusal)
        );
    }
    assert_eq!(counts(&store), before);
}

#[test]
fn broker_refuses_preparation_event_as_cause_for_another_effect() {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    store
        .prepare_provider_service_launch(caller(), launch(&store, 0xc520, MANAGED))
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let event: String = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT event_id FROM events WHERE causation_receipt_id=?1",
                [context.receipt_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let mut action = intent(
        &context,
        ActionId::from_u128(0xc524),
        context.receipt_id,
        "fixture.other",
    );
    action.cause = BrokerCause::Event {
        event_id: event.parse().unwrap(),
    };
    action.manifest.get_mut("subject").unwrap()["cause_ref"] = json!(format!("event:{event}"));
    action.intent_digest = unleash_protocol::canonical_sha256(&action.manifest).unwrap();
    let before = counts(&store);
    let result = store.commit_broker_phase(authorizing(action, 0xc525));
    assert!(
        matches!(result, Err(StoreError::InvalidArgument { message, .. })
        if message == "service launch preparation cannot authorize another effect")
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn v019_upgrade_preserves_existing_profile_and_does_not_prepare_it() {
    let directory = local_test_dir("f012-service-launch-migration").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.max_supported_schema_version = 18;
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal(PRINCIPAL, "user", caller().actor_id.as_str())
        .unwrap();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let old_receipt = store
        .inspect_command_receipt(&caller().actor_id, CommandId::from_u128(0xc519))
        .unwrap();
    assert!(matches!(
        old_receipt,
        unleash_store::ReceiptInspectionOutcome::Full(_)
    ));
    let old_profile = store
        .resolve_provider_profile_snapshot(MANAGED, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);
    let backup = store
        .prepare_backup(unleash_store::PrepareBackupRequest {
            destination_dir: directory.path().join("before-v019"),
            retention_tier: unleash_store::BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    store.shutdown().unwrap();
    config.max_supported_schema_version = 19;
    let upgraded =
        UnleashStore::open_with_migration_backup(config, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 19);
    assert_eq!(upgraded.read_provider_service_launch(LAUNCH).unwrap(), None);
    assert_eq!(
        upgraded
            .resolve_provider_profile_snapshot(MANAGED, 1)
            .unwrap()
            .value,
        Some(old_profile)
    );
    assert_eq!(
        upgraded
            .inspect_command_receipt(&caller().actor_id, CommandId::from_u128(0xc519))
            .unwrap(),
        old_receipt
    );
    let wire = launch(&upgraded, 0xc520, MANAGED);
    upgraded
        .prepare_provider_service_launch(caller(), wire)
        .unwrap();
    assert_eq!(
        upgraded
            .read_provider_service_launch(LAUNCH)
            .unwrap()
            .unwrap()
            .plan
            .profile_id,
        MANAGED
    );
    upgraded
        .read_snapshot(|db, _| {
            assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
}
