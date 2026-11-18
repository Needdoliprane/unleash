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
    fixture_with_ports(
        Arc::new(ManualClock::new(0)),
        Arc::new(NoopCutpointObserver),
    )
}

fn fixture_with_ports(
    clock: Arc<ManualClock>,
    cutpoints: Arc<dyn CutpointObserver>,
) -> (
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
    config.clock = clock;
    config.cutpoints = cutpoints;
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
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .last_event_cursor,
        first.event_cursor
    );
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
    let effect_counter = std::sync::atomic::AtomicUsize::new(0);
    let execute = |request: BrokerPhaseRequest| -> Result<(), StoreError> {
        store.commit_broker_phase(request)?;
        let before_effect = store.read_snapshot(|conn,_| {
        let states:i64=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type IN ('ActionAuthorizationRequested','ActionAuthorized','ActionDispatchCommitted')",[intent.action_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let marker:i64=conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE dispatch_id=?1",[dispatch_id.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        Ok((states,marker))
        })?;
        assert_eq!(before_effect, (3, 1));
        effect_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    };
    execute(dispatch.clone()).unwrap();
    assert!(execute(dispatch.clone()).is_err());
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
    assert!(
        store
            .commit_broker_phase(observation.clone())
            .unwrap()
            .replayed
    );
    let mut divergent_reason = observation;
    if let BrokerPhase::Observation { reason, .. } = &mut divergent_reason.phase {
        *reason = Some("different local classification".to_owned());
    }
    assert!(store.commit_broker_phase(divergent_reason).is_err());
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::ObservedSuccess
    );
    assert!(execute(dispatch).is_err());
    assert_eq!(effect_counter.load(std::sync::atomic::Ordering::SeqCst), 1);
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

struct CutpointControl(std::sync::atomic::AtomicU8);
impl CutpointObserver for CutpointControl {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        let selected = match point {
            StoreCutpoint::C1BeforeCommandCommit => 1,
            StoreCutpoint::C3AfterBrokerIntentCommit => 3,
            StoreCutpoint::C4AfterBrokerDispatchCommit => 4,
            _ => 0,
        };
        if selected != 0 && selected == self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err(StoreError::CutpointInjected {
                cutpoint: format!("C{selected}"),
                message: "broker fixture".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Les trois frontières de commit et les invariants SQLite sont vérifiés dans le même parcours partagé"
)]
fn broker_commit_boundaries_are_atomic_and_uncertain_dispatch_is_not_replayed() {
    for cut in [1, 3, 4] {
        let cutpoints = Arc::new(CutpointControl(std::sync::atomic::AtomicU8::new(0)));
        let (store, intent, evaluation, ids, _directory) =
            fixture_with_ports(Arc::new(ManualClock::new(0)), cutpoints.clone());
        let authorizing = request(
            &intent,
            evaluation,
            0,
            BrokerPhase::Authorizing {
                intent: intent.clone(),
            },
        );
        if cut != 4 {
            cutpoints.0.store(cut, std::sync::atomic::Ordering::SeqCst);
        }
        let result = store.commit_broker_phase(authorizing.clone());
        if cut == 4 {
            result.unwrap();
        } else {
            assert!(result.is_err());
        }
        cutpoints.0.store(0, std::sync::atomic::Ordering::SeqCst);
        if cut == 1 {
            assert!(
                store
                    .inspect_broker_action(intent.action_id)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                store
                    .read_snapshot(|conn, _| conn
                        .query_row(
                            "SELECT COUNT(*) FROM events WHERE aggregate_id=?1",
                            [intent.action_id.to_string()],
                            |r| r.get::<_, i64>(0)
                        )
                        .map_err(StoreError::from_rusqlite))
                    .unwrap(),
                0
            );
            assert!(!store.commit_broker_phase(authorizing).unwrap().replayed);
        } else if cut == 3 {
            assert!(store.commit_broker_phase(authorizing).unwrap().replayed);
        }
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
        cutpoints.0.store(
            if cut == 4 { 4 } else { 1 },
            std::sync::atomic::Ordering::SeqCst,
        );
        assert!(store.commit_broker_phase(dispatch.clone()).is_err());
        cutpoints.0.store(0, std::sync::atomic::Ordering::SeqCst);
        let (markers,events,outboxes)=store.read_snapshot(|conn,_| {
            let markers=conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1",[intent.action_id.to_string()],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
            let events=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionDispatchCommitted'",[intent.action_id.to_string()],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
            let outboxes=conn.query_row("SELECT COUNT(*) FROM outbox o JOIN events e USING(event_id) WHERE e.aggregate_id=?1 AND e.event_type='ActionDispatchCommitted'",[intent.action_id.to_string()],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
            Ok((markers,events,outboxes))
        }).unwrap();
        if cut == 4 {
            assert_eq!((markers, events, outboxes), (1, 1, 1));
            assert!(store.commit_broker_phase(dispatch).is_err());
            let store = store.restart_same_lineage().unwrap();
            store.reconcile_recovered_actions_after_restart().unwrap();
            assert_eq!(
                store
                    .inspect_broker_action(intent.action_id)
                    .unwrap()
                    .unwrap()
                    .state,
                ActionState::OutcomeUnknown
            );
        } else {
            assert_eq!((markers, events, outboxes), (0, 0, 0));
            assert_eq!(
                store
                    .inspect_broker_action(intent.action_id)
                    .unwrap()
                    .unwrap()
                    .state,
                ActionState::Authorized
            );
            store.commit_broker_phase(dispatch).unwrap();
        }
    }
}

#[test]
fn retryable_evaluation_requires_durable_result_backoff_and_explicit_predecessor() {
    let clock = Arc::new(ManualClock::new(0));
    let (store, intent, evaluation, ids, _directory) =
        fixture_with_ports(clock.clone(), Arc::new(NoopCutpointObserver));
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
    let next = BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: Some(evaluation.evaluation_id),
    };
    let retry = request(
        &intent,
        next,
        1,
        BrokerPhase::Authorizing {
            intent: intent.clone(),
        },
    );
    assert!(store.commit_broker_phase(retry).is_err());
    let result = request(
        &intent,
        evaluation,
        1,
        BrokerPhase::Authorization {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            outcome: BrokerAuthorizationOutcome::Failed {
                failure: BrokerAuthorizationFailure {
                    code: unleash_protocol::ErrorCode::ResourceBusy,
                    reason_code: Some(unleash_protocol::ReasonCode::GuardUnavailable),
                    retry_class: unleash_protocol::RetryClass::AfterBackoff,
                    details_ref: None,
                    retry_not_before_ms: Some(TimestampMs::new(1000)),
                },
            },
            cause: BrokerAuthorizationCause::Evaluation,
        },
    );
    store.commit_broker_phase(result.clone()).unwrap();
    assert!(store.commit_broker_phase(result).unwrap().replayed);
    let retry = request(
        &intent,
        next,
        2,
        BrokerPhase::Authorizing {
            intent: intent.clone(),
        },
    );
    assert!(store.commit_broker_phase(retry.clone()).is_err());
    clock.advance_ms(1000);
    let mut no_predecessor = retry.clone();
    no_predecessor.evaluation.previous_evaluation_id = None;
    assert!(store.commit_broker_phase(no_predecessor).is_err());
    store.commit_broker_phase(retry.clone()).unwrap();
    assert!(store.commit_broker_phase(retry).unwrap().replayed);
    let decision = decision(&intent, next);
    store
        .commit_broker_phase(request(
            &intent,
            next,
            3,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized { decision },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorized
    );
}

#[test]
fn failure_requiring_refresh_is_retained_without_terminal_denial_or_new_evaluation() {
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
    store
        .commit_broker_phase(request(
            &intent,
            evaluation,
            1,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Failed {
                    failure: BrokerAuthorizationFailure {
                        code: unleash_protocol::ErrorCode::ResourceBusy,
                        reason_code: None,
                        retry_class: unleash_protocol::RetryClass::AfterRefresh,
                        details_ref: None,
                        retry_not_before_ms: None,
                    },
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorizing
    );
    let next = BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: Some(evaluation.evaluation_id),
    };
    assert!(
        store
            .commit_broker_phase(request(
                &intent,
                next,
                2,
                BrokerPhase::Authorizing {
                    intent: intent.clone()
                }
            ))
            .is_err()
    );
}

#[test]
fn cached_authorization_cannot_cite_a_failed_evaluation() {
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
    store
        .commit_broker_phase(request(
            &intent,
            evaluation,
            1,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Failed {
                    failure: BrokerAuthorizationFailure {
                        code: unleash_protocol::ErrorCode::InvalidArgument,
                        reason_code: None,
                        retry_class: unleash_protocol::RetryClass::Never,
                        details_ref: None,
                        retry_not_before_ms: None,
                    },
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    let mut target = intent;
    target.action_id = ActionId::from_uuid(ids.new_uuid());
    target
        .manifest
        .insert("action_id".to_owned(), json!(target.action_id));
    target.intent_digest = canonical_sha256(&target.manifest).unwrap();
    let target_eval = BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: None,
    };
    store
        .commit_broker_phase(request(
            &target,
            target_eval,
            0,
            BrokerPhase::Authorizing {
                intent: target.clone(),
            },
        ))
        .unwrap();
    let mut cached = decision(&target, target_eval);
    cached.from_cache = true;
    cached.source_evaluation_id = Some(evaluation.evaluation_id);
    assert!(
        store
            .commit_broker_phase(request(
                &target,
                target_eval,
                1,
                BrokerPhase::Authorization {
                    action_id: target.action_id,
                    intent_digest: target.intent_digest,
                    outcome: BrokerAuthorizationOutcome::Authorized { decision: cached },
                    cause: BrokerAuthorizationCause::Evaluation
                }
            ))
            .is_err()
    );
    assert_eq!(
        store
            .inspect_broker_action(target.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorizing
    );
}

#[test]
fn authorized_cache_source_is_refused_without_attested_monotonic_ttl() {
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
    let mut source = decision(&intent, evaluation);
    source.expiry = Some(TimestampMs::new(60000));
    store
        .commit_broker_phase(request(
            &intent,
            evaluation,
            1,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: source.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    let mut target = intent;
    target.action_id = ActionId::from_uuid(ids.new_uuid());
    target
        .manifest
        .insert("action_id".to_owned(), json!(target.action_id));
    target.intent_digest = canonical_sha256(&target.manifest).unwrap();
    let next = BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: None,
    };
    store
        .commit_broker_phase(request(
            &target,
            next,
            0,
            BrokerPhase::Authorizing {
                intent: target.clone(),
            },
        ))
        .unwrap();
    source.action_id = target.action_id;
    source.evaluation_id = next.evaluation_id;
    source.source_evaluation_id = Some(evaluation.evaluation_id);
    source.from_cache = true;
    let error = store
        .commit_broker_phase(request(
            &target,
            next,
            1,
            BrokerPhase::Authorization {
                action_id: target.action_id,
                intent_digest: target.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized { decision: source },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap_err();
    assert!(error.to_string().contains("TTL de cache non attestés"));
    assert_eq!(
        store
            .inspect_broker_action(target.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Authorizing
    );
}

fn authorize(
    store: &UnleashStore,
    intent: &BrokerIntent,
    evaluation: BrokerEvaluation,
) -> BrokerAuthorizationDecision {
    store
        .commit_broker_phase(request(
            intent,
            evaluation,
            0,
            BrokerPhase::Authorizing {
                intent: intent.clone(),
            },
        ))
        .unwrap();
    let decision = decision(intent, evaluation);
    store
        .commit_broker_phase(request(
            intent,
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
    decision
}

#[test]
fn writer_revalidates_principal_and_exact_guard_snapshot_before_marker() {
    for suspend in [false, true] {
        let (store, intent, evaluation, ids, _directory) = fixture();
        let decision = authorize(&store, &intent, evaluation);
        store.execute_in_writer_tx(WriteCriticality::Normal,move |tx,_| {
            if suspend {
                tx.execute("INSERT INTO subject_suspensions (suspension_id,subject_kind,subject_id,scope_kind,scope_id,status,reason,acted_by_actor,created_at_ms,updated_at_ms) VALUES ('suspend-fixture','actor','p-sys','global','global','suspended','private-reason-canary','user:fixture',0,0)",[]).map_err(StoreError::from_rusqlite)?;
            } else {
                // La révision de policy reste 0 ; seule l’empreinte complète permet de voir ce changement.
                tx.execute("INSERT INTO guard_configs (config_id,guard_mode,updated_by_actor,updated_at_ms) VALUES ('default','broker_only','user:fixture',0)",[]).map_err(StoreError::from_rusqlite)?;
            }
            Ok(())
        }).unwrap();
        let result = store.commit_broker_phase(request(
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
        ));
        let error = result.unwrap_err().to_string();
        assert!(!error.contains("private-reason-canary"));
        assert_eq!(
            store
                .inspect_broker_action(intent.action_id)
                .unwrap()
                .unwrap()
                .state,
            ActionState::Authorized
        );
        assert!(
            store
                .inspect_broker_action(intent.action_id)
                .unwrap()
                .unwrap()
                .dispatch_id
                .is_none()
        );
    }
}

#[test]
fn writer_rejects_wrong_fence_decision_and_digest_even_with_current_cas() {
    let (store, intent, evaluation, ids, _directory) = fixture();
    let decision = authorize(&store, &intent, evaluation);
    let base = request(
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
    for variant in 0..3 {
        let mut changed = base.clone();
        if let BrokerPhase::Dispatch {
            fence,
            decision,
            intent_digest,
            ..
        } = &mut changed.phase
        {
            match variant {
                0 => fence.fence = Fence::new(1),
                1 => decision.decision_ref = "another-decision".to_owned(),
                _ => *intent_digest = Sha256Digest::from_bytes(b"other-intention"),
            }
        }
        assert!(store.commit_broker_phase(changed).is_err());
    }
    store.commit_broker_phase(base).unwrap();
}

#[test]
fn real_cause_cannot_be_reused_by_another_actor_or_project() {
    for change_project in [false, true] {
        let (store, mut intent, evaluation, ids, _directory) = fixture();
        if change_project {
            let project = unleash_domain::ProjectId::from_uuid(ids.new_uuid());
            store.execute_in_writer_tx(WriteCriticality::Normal,move |tx,_| {
                tx.execute("INSERT INTO projects (project_id,name,created_at_ms) VALUES (?1,'project fixture',0)",[project.to_string()]).map_err(StoreError::from_rusqlite)?;
                Ok(())
            }).unwrap();
            intent.project_id = Some(project);
            intent.scope_revision = Revision::INITIAL;
            intent
                .manifest
                .insert("project_id".to_owned(), json!(project));
            intent
                .manifest
                .insert("scope_revision".to_owned(), json!(0));
        } else {
            store
                .register_principal("p-other", "system", "p-other")
                .unwrap();
            let actor = ActorId::new("p-other").unwrap();
            intent.principal_id = actor.clone();
            intent.origin = ActionOrigin::ControlPlane {
                control_principal_id: actor.clone(),
            };
            intent
                .manifest
                .get_mut("subject")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("principal_id".to_owned(), json!(actor));
        }
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
}

#[test]
fn unverified_reconciliation_and_mismatched_observations_cannot_publish_success() {
    let (store, intent, evaluation, ids, _directory) = fixture();
    let decision = authorize(&store, &intent, evaluation);
    let dispatch_id = DispatchId::from_uuid(ids.new_uuid());
    store
        .commit_broker_phase(request(
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
        ))
        .unwrap();
    let receipt = BrokerActionReceipt {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        dispatch_id,
        decision_ref: decision.decision_ref,
        state: ActionState::ObservedSuccess,
        output_digest: None,
        enforcement_refs: vec![],
        remote_audit_ref: None,
        remote_audit_unavailable: true,
        observed_at_ms: TimestampMs::new(0),
    };
    for variant in 0..3 {
        let mut observation = receipt.clone();
        if variant == 0 {
            observation.dispatch_id = DispatchId::from_uuid(ids.new_uuid());
        }
        let cause = if variant == 1 {
            BrokerObservationCause::Reconciliation {
                evidence_ref: Some("unverified-reference".to_owned()),
            }
        } else {
            BrokerObservationCause::Execution
        };
        let ordinal = Revision::new(if variant == 2 { 3 } else { 1 });
        assert!(
            store
                .commit_broker_phase(request(
                    &intent,
                    evaluation,
                    3,
                    BrokerPhase::Observation {
                        action_id: intent.action_id,
                        intent_digest: intent.intent_digest,
                        receipt: observation,
                        observation_revision: ordinal,
                        cause,
                        reason: None
                    }
                ))
                .is_err()
        );
    }
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Dispatching
    );
}

#[test]
fn restored_action_cannot_observe_or_replay_when_numeric_epoch_matches_new_lineage() {
    let (store, intent, evaluation, ids, _directory) = fixture();
    let decision = authorize(&store, &intent, evaluation);
    let dispatch_id = DispatchId::from_uuid(ids.new_uuid());
    store
        .commit_broker_phase(request(
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
        ))
        .unwrap();
    let new_authority = unleash_domain::AuthorityGeneration::from_uuid(ids.new_uuid());
    // Image après restauration : les actions gardent leur origine, et les epochs peuvent coïncider.
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute(
                "UPDATE daemon_state SET authority_generation=?1 WHERE singleton=1",
                [new_authority.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(())
        })
        .unwrap();
    let receipt = BrokerActionReceipt {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        dispatch_id,
        decision_ref: decision.decision_ref,
        state: ActionState::ObservedSuccess,
        output_digest: None,
        enforcement_refs: vec![],
        remote_audit_ref: None,
        remote_audit_unavailable: true,
        observed_at_ms: TimestampMs::new(0),
    };
    let mut observation = request(
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
    observation.authority_generation = new_authority;
    assert!(matches!(
        store.commit_broker_phase(observation),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let mut replay = request(
        &intent,
        evaluation,
        0,
        BrokerPhase::Authorizing {
            intent: intent.clone(),
        },
    );
    replay.authority_generation = new_authority;
    assert!(matches!(
        store.commit_broker_phase(replay),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::Dispatching
    );
}
