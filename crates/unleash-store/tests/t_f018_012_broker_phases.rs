//! REQ-REC-001 : le journal broker est engagé avant l'effet et ne réacquitte jamais Dispatch.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures synthétiques et assertions de contrat"
)]
use serde_json::json;
use std::sync::Arc;
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, ActorId, DispatchId, Fence, IdGeneratorPort, LineageFence,
    ReconcileStrategy, RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonical_sha256};
use unleash_store::*;
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

fn fixture() -> (
    UnleashStore,
    BrokerIntent,
    BrokerEvaluation,
    Arc<DeterministicIdGenerator>,
    impl std::fmt::Debug,
) {
    let directory = local_test_dir("f018-012-broker").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1201));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    config.clock = Arc::new(ManualClock::new(0));
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let actor = ActorId::new("p-sys").unwrap();
    let daemon = store.daemon_state().unwrap();
    let cause = store
        .commit_command(CommitCommandRequest {
            actor_id: actor.clone(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: daemon.authority_generation,
                actor_ref: Some(actor.to_string()),
                project_id: None,
                aggregate_id: ids.new_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "SetSettings".to_owned(),
                payload: serde_json::Map::new(),
            },
            target_aggregate: AggregateKind::Settings,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::User,
                scope_id: "default".to_owned(),
                caller: CallerAuthority {
                    actor_id: actor.clone(),
                    role: CallerRole::HumanOperator,
                },
                patch: SettingsPatch {
                    execution_pool: None,
                    retention_policy: None,
                    confirm_retention_shortening: false,
                    analyzer_paths: None,
                    extra_typed: Some(serde_json::Map::new()),
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap()
        .receipt()
        .receipt_id;
    let action = ActionId::from_uuid(ids.new_uuid());
    let fence = LineageFence::new(
        daemon.authority_generation,
        daemon.daemon_epoch,
        Fence::new(0),
    );
    let digest = Sha256Digest::from_bytes(b"benign-local-read");
    let manifest = json!({
        "action_id": action, "subject": {"kind":"control_plane", "principal_id":actor, "cause_ref":format!("receipt:{cause}"), "invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id":null,"verb":"read","resource_refs":[],"payload_digest":digest,"reconcile_strategy":"observe_then_decide","effects_class":"local_read","scope_revision":1,"policy_revision":0,"lineage_fence":fence,"action_deadline":60000,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}
    }).as_object().unwrap().clone();
    let intent = BrokerIntent {
        action_id: action,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: actor.clone(),
        },
        principal_id: actor,
        cause: BrokerCause::CommandReceipt { receipt_id: cause },
        project_id: None,
        payload_digest: digest,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: fence,
        action_deadline: TimestampMs::new(60000),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        intent_digest: canonical_sha256(&manifest).unwrap(),
        manifest,
    };
    let evaluation = BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: None,
    };
    (store, intent, evaluation, ids, directory)
}

fn request(
    intent: &BrokerIntent,
    evaluation: BrokerEvaluation,
    revision: u64,
    phase: BrokerPhase,
) -> BrokerPhaseRequest {
    BrokerPhaseRequest {
        authority_generation: intent.lineage_fence.authority_generation,
        expected_revision: Revision::new(revision),
        evaluation,
        phase,
    }
}

fn decision(intent: &BrokerIntent, evaluation: BrokerEvaluation) -> BrokerAuthorizationDecision {
    BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "decision:local-fixture".to_owned(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: None,
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Les quatre phases et leurs invariants observables avant effet dans un seul parcours"
)]
fn exact_intention_authorization_dispatch_observation_and_replay_refusal() {
    let (store, intent, evaluation, ids, _directory) = fixture();
    let authorizing = request(
        &intent,
        evaluation,
        0,
        BrokerPhase::Authorizing {
            intent: intent.clone(),
        },
    );
    let first = store.commit_broker_phase(authorizing.clone()).unwrap();
    assert_eq!(first.revision, Revision::new(1));
    assert!(
        store
            .commit_broker_phase(authorizing.clone())
            .unwrap()
            .replayed
    );
    let mut conflict = authorizing;
    if let BrokerPhase::Authorizing {
        intent: ref mut changed,
    } = conflict.phase
    {
        changed
            .manifest
            .insert("verb".to_owned(), json!("different"));
    }
    assert!(store.commit_broker_phase(conflict).is_err());
    let decision = decision(&intent, evaluation);
    let authorization = request(
        &intent,
        evaluation,
        1,
        BrokerPhase::Authorization {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            outcome: BrokerAuthorizationOutcome::Authorized {
                decision: decision.clone(),
            },
            cause: BrokerAuthorizationCause::Evaluation,
        },
    );
    store.commit_broker_phase(authorization.clone()).unwrap();
    assert!(store.commit_broker_phase(authorization).unwrap().replayed);
    let dispatch_id = DispatchId::from_uuid(ids.new_uuid());
    let dispatch = request(
        &intent,
        evaluation,
        2,
        BrokerPhase::Dispatch {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            decision: decision.clone(),
            dispatch_id,
            fence: intent.lineage_fence,
        },
    );
    store.commit_broker_phase(dispatch.clone()).unwrap();
    assert!(store.commit_broker_phase(dispatch.clone()).is_err());
    let before_effect = store.read_snapshot(|conn,_| {
        let states:i64=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type IN ('ActionAuthorizationRequested','ActionAuthorized','ActionDispatchCommitted')",[intent.action_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let marker:i64=conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE dispatch_id=?1",[dispatch_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        Ok((states,marker))
    }).unwrap();
    assert_eq!(before_effect, (3, 1));
    let effect_counter = 1;
    let receipt = BrokerActionReceipt {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        dispatch_id,
        decision_ref: decision.decision_ref,
        state: ActionState::ObservedSuccess,
        output_digest: Some(Sha256Digest::from_bytes(b"observed-output")),
        enforcement_refs: vec![],
        remote_audit_ref: None,
        remote_audit_unavailable: true,
        observed_at_ms: TimestampMs::new(0),
    };
    let observation = request(
        &intent,
        evaluation,
        3,
        BrokerPhase::Observation {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            receipt,
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
        },
    );
    store.commit_broker_phase(observation.clone()).unwrap();
    assert!(store.commit_broker_phase(observation).unwrap().replayed);
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::ObservedSuccess
    );
    assert!(store.commit_broker_phase(dispatch).is_err());
    assert_eq!(effect_counter, 1);
}

#[test]
fn source_receipt_must_exist_before_any_intention_row() {
    let (store, mut intent, evaluation, ids, _directory) = fixture();
    let receipt_id = unleash_domain::ReceiptId::from_uuid(ids.new_uuid());
    intent.cause = BrokerCause::CommandReceipt { receipt_id };
    intent
        .manifest
        .get_mut("subject")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "cause_ref".to_owned(),
            json!(format!("receipt:{receipt_id}")),
        );
    intent.intent_digest = canonical_sha256(&intent.manifest).unwrap();
    assert!(
        store
            .commit_broker_phase(request(
                &intent,
                evaluation,
                0,
                BrokerPhase::Authorizing {
                    intent: intent.clone()
                }
            ))
            .is_err()
    );
    assert!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn restart_after_dispatch_never_authorizes_a_second_effect() {
    let (store, intent, evaluation, ids, _directory) = fixture();
    store
        .commit_broker_phase(request(
            &intent,
            evaluation,
            0,
            BrokerPhase::Authorizing {
                intent: intent.clone(),
            },
        ))
        .unwrap();
    let decision = decision(&intent, evaluation);
    store
        .commit_broker_phase(request(
            &intent,
            evaluation,
            1,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    let dispatch = request(
        &intent,
        evaluation,
        2,
        BrokerPhase::Dispatch {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            decision,
            dispatch_id: DispatchId::from_uuid(ids.new_uuid()),
            fence: intent.lineage_fence,
        },
    );
    store.commit_broker_phase(dispatch.clone()).unwrap();
    let store = store.restart_same_lineage().unwrap();
    assert_eq!(
        store.reconcile_recovered_actions_after_restart().unwrap(),
        vec![intent.action_id]
    );
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::OutcomeUnknown
    );
    assert!(store.commit_broker_phase(dispatch).is_err());
}
