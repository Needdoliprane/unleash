//! `SQLite` control-port oracles only. The OS identity below is synthetic;
//! no running host, PTY or actual capability rotation is claimed by this fixture.
use super::*;

#[path = "terminal_acquisition_legacy_scenarios.rs"]
mod legacy_query_scenarios;

#[path = "terminal_acquisition_time_scenarios.rs"]
mod time_query_scenarios;

#[path = "terminal_input_writer_scenarios.rs"]
mod input_writer_scenarios;

#[path = "terminal_renewal_time_scenarios.rs"]
mod renewal_time_scenarios;

#[path = "terminal_control_history_scenarios.rs"]
mod history_scenarios;

#[path = "terminal_control_history_adoption_scenarios.rs"]
mod history_adoption_scenarios;
use unleash_domain::{ActionState, TimestampMs};
use unleash_store::terminal_allocations::{
    RegisteredTerminalControl, ResolveTerminalControlRequest, TERMINAL_CONTROL_VERB,
    TerminalControlMode, TerminalControlRecord, TerminalControlState,
};

const TERMINAL: u128 = 0xf011_c001;
const CONTROL: u128 = 0xf011_c002;

fn registered_request(
    store: &UnleashStore,
    allocation: &TerminalAllocationRecord,
    n: u128,
) -> CommitCommandRequest {
    let actor = allocation.context.actor_id.clone();
    let request = RegisteredTerminalControl {
        terminal_id: allocation.context.terminal_id,
        process_run_uuid: allocation.context.process_run_uuid,
        scope_revision: Revision::new(2),
        control_mode: TerminalControlMode::Exclusive,
    };
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(n + 1000),
            command_id: unleash_domain::CommandId::from_u128(n),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: None,
            project_id: Some(PROJECT),
            aggregate_id: allocation.context.terminal_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "AcquireTerminal".into(),
            payload: serde_json::to_value(&request)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        },
        target_aggregate: AggregateKind::TerminalControl,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalControl {
            caller: CallerAuthority {
                actor_id: actor,
                role: CallerRole::HumanOperator,
            },
            request,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn observed(
    successor: bool,
) -> (
    UnleashStore,
    Arc<ManualClock>,
    Arc<FailAt>,
    TerminalAllocationRecord,
    impl std::fmt::Debug,
) {
    observed_with_cutpoint(successor, StoreCutpoint::C1BeforeCommandCommit)
}

fn observed_with_cutpoint(
    successor: bool,
    point: StoreCutpoint,
) -> (
    UnleashStore,
    Arc<ManualClock>,
    Arc<FailAt>,
    TerminalAllocationRecord,
    impl std::fmt::Debug,
) {
    let fault = Arc::new(FailAt {
        armed: AtomicBool::new(false),
        point,
    });
    let (store, clock, allocation, directory) = observed_with_observer(successor, fault.clone());
    (store, clock, fault, allocation, directory)
}

fn observed_with_observer(
    successor: bool,
    observer: Arc<dyn CutpointObserver>,
) -> (
    UnleashStore,
    Arc<ManualClock>,
    TerminalAllocationRecord,
    impl std::fmt::Debug,
) {
    let (store, _cfg, clock, preparation, directory) = fixture(observer);
    store
        .commit_command(request(&store, preparation, TERMINAL))
        .unwrap();
    let allocation = read(&store, TERMINAL);
    clock.set_utc_ms(100);
    let marker = dispatch(&store, &allocation);
    store
        .resolve_terminal_launch(resolution(&allocation, marker))
        .unwrap();
    clock.set_utc_ms(105);
    let (observation, phase) =
        launch_observation_state_fixture(&store, &allocation, marker, 101, 103, false);
    store
        .commit_terminal_launch_observation(observation, phase)
        .unwrap();
    let store = Arc::try_unwrap(store).unwrap();
    // SQLite-only successor fixture: preserve the physical boot identity.
    // restart_same_lineage() is a legacy helper that invents a different boot,
    // so it cannot represent a daemon-only restart with a surviving PTY.
    if successor {
        store
            .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
                tx.execute(
                    "UPDATE daemon_state SET daemon_epoch=daemon_epoch+1 WHERE singleton=1",
                    [],
                )?;
                tx.execute(
                    "UPDATE resource_leases SET state='suspect' WHERE state='held'",
                    [],
                )?;
                tx.execute(
                    "UPDATE capacity_reservations SET state='reconciling' WHERE state='held'",
                    [],
                )?;
                tx.execute(
                    "UPDATE processes SET state='suspect' WHERE state='live'",
                    [],
                )?;
                tx.execute(
                    "UPDATE terminal_sessions SET state='suspect' WHERE state='ready'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
    }
    let allocation = read(&store, TERMINAL);
    (store, clock, allocation, directory)
}

fn prepared() -> (
    UnleashStore,
    Arc<ManualClock>,
    Arc<FailAt>,
    TerminalControlRecord,
    CommitCommandRequest,
    impl std::fmt::Debug,
) {
    let (store, clock, fault, allocation, directory) = observed(true);
    let request = registered_request(&store, &allocation, CONTROL);
    store.commit_command(request.clone()).unwrap();
    let control = store
        .inspect_terminal_control_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    (store, clock, fault, control, request, directory)
}

fn state(store: &UnleashStore, control: &TerminalControlRecord) -> (i64, String, i64, i64, i64) {
    store.read_snapshot(|conn,_|Ok((
        conn.query_row("SELECT control_revision FROM terminal_sessions WHERE terminal_id=?1",[control.context.terminal_id.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT lease_id FROM terminal_sessions WHERE terminal_id=?1",[control.context.terminal_id.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT last_fence FROM resource_fences WHERE authority_generation=?1 AND resource_kind='terminal'",[control.context.authority_generation.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT COUNT(*) FROM resource_leases",[],|row|row.get(0))?,
        conn.query_row("SELECT COUNT(*) FROM capacity_reservations WHERE state IN ('held','reconciling')",[],|row|row.get(0))?,
    ))).unwrap()
}

fn metadata(store: &UnleashStore, control: &TerminalControlRecord) -> serde_json::Value {
    serde_json::to_value(
        store
            .read_snapshot(|conn, _| {
                terminal_allocations::read_terminal_metadata(
                    conn,
                    &control.context.actor_id,
                    Some(PROJECT),
                    None::<&std::collections::HashSet<ProjectId>>,
                )
            })
            .unwrap(),
    )
    .unwrap()
}

fn control_authorizing(control: &TerminalControlRecord) -> BrokerPhaseRequest {
    let c = &control.context;
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let manifest=json!({"action_id":c.action_id,"subject":{"kind":"control_plane","principal_id":c.principal_id,"cause_ref":format!("receipt:{}",c.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},"project_id":c.project_id,"verb":TERMINAL_CONTROL_VERB,"resource_refs":[{"kind":"terminal","value":c.terminal_id}],"payload_digest":control.context_digest,"reconcile_strategy":"observe_then_decide","effects_class":"process_exec","scope_revision":c.scope_revision,"policy_revision":c.policy_revision,"lineage_fence":fence,"action_deadline":c.expires_at_ms,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}}).as_object().unwrap().clone();
    BrokerPhaseRequest {
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
                    receipt_id: c.receipt_id,
                },
                project_id: Some(c.project_id),
                payload_digest: control.context_digest,
                scope_revision: c.scope_revision,
                policy_revision: c.policy_revision,
                lineage_fence: fence,
                action_deadline: c.expires_at_ms,
                reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
                intent_digest: canonical_sha256(&manifest).unwrap(),
                manifest,
            },
        },
    }
}

fn control_dispatch(
    store: &UnleashStore,
    control: &TerminalControlRecord,
) -> (BrokerPhaseRequest, BrokerPhaseRequest) {
    let first = control_authorizing(control);
    dispatch_authorizing(store, first)
}

fn dispatch_authorizing(
    store: &UnleashStore,
    first: BrokerPhaseRequest,
) -> (BrokerPhaseRequest, BrokerPhaseRequest) {
    let committed = store.commit_broker_phase(first.clone()).unwrap();
    let BrokerPhase::Authorizing { intent } = &first.phase else {
        unreachable!()
    };
    let decision = BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: first.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "synthetic-sqlite-control-allow".into(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(intent.action_deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let authorization = store
        .commit_broker_phase(BrokerPhaseRequest {
            expected_revision: committed.revision,
            phase: BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
            ..first.clone()
        })
        .unwrap();
    let dispatch_id = DispatchId::from_u128(CONTROL + 100);
    let dispatch = BrokerPhaseRequest {
        expected_revision: authorization.revision,
        phase: BrokerPhase::Dispatch {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            decision,
            dispatch_id,
            fence: intent.lineage_fence,
        },
        ..first.clone()
    };
    let marker = store.commit_broker_phase(dispatch.clone()).unwrap();
    let unknown = BrokerPhaseRequest {
        expected_revision: marker.revision,
        phase: BrokerPhase::Observation {
            action_id: intent.action_id,
            intent_digest: intent.intent_digest,
            receipt: BrokerActionReceipt {
                action_id: intent.action_id,
                evaluation_id: first.evaluation.evaluation_id,
                dispatch_id,
                decision_ref: "synthetic-sqlite-control-allow".into(),
                state: ActionState::OutcomeUnknown,
                output_digest: None,
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: false,
                observed_at_ms: TimestampMs::new(110),
            },
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
        },
        ..first
    };
    (dispatch, unknown)
}

#[test]
fn t_f011_003_prepared_control_has_no_dispatch_or_effective_revision_change() {
    let (store, _clock, _fault, control, request, _directory) = prepared();
    assert_eq!(
        state(&store, &control),
        (0, control.context.previous_lease_id.to_string(), 1, 1, 3)
    );
    assert_eq!(
        metadata(&store, &control).pointer("/items/0/control_state"),
        Some(&json!("prepared"))
    );
    assert_eq!(
        metadata(&store, &control).pointer("/items/0/control_revision"),
        Some(&json!(0))
    );
    assert!(control.broker_action_state.is_none() && control.broker_dispatch_id.is_none());
    assert!(matches!(
        store.commit_command(request),
        Ok(CommitOutcome::IdempotentReplay(_))
    ));
    assert!(matches!(
        store.commit_command(registered_request(
            &store,
            &read(&store, TERMINAL),
            CONTROL + 1
        )),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert!(!store.config().data_root.join("process-hosts").exists());
}

#[test]
fn t_f011_003_control_resolution_rollback_keeps_unknown_visible_without_a_new_permit() {
    let (store, clock, fault, control, request, _directory) = prepared();
    clock.set_utc_ms(110);
    let before = state(&store, &control);
    let (dispatch, unknown) = control_dispatch(&store, &control);
    let BrokerPhase::Dispatch { dispatch_id, .. } = dispatch.phase else {
        unreachable!()
    };
    fault.armed.store(true, Ordering::Release);
    let result = store.resolve_terminal_control(ResolveTerminalControlRequest {
        action_id: control.context.action_id,
        dispatch_id,
        authority_generation: control.context.authority_generation,
        expected_revision: control.revision,
        context_digest: control.context_digest,
        caller: CallerAuthority {
            actor_id: control.context.actor_id.clone(),
            role: CallerRole::HumanOperator,
        },
    });
    assert!(
        matches!(result,Err(StoreError::CutpointInjected {ref cutpoint,..}) if cutpoint=="C1BeforeCommandCommit")
    );
    assert_eq!(state(&store, &control), before);
    store.commit_broker_phase(unknown).unwrap();
    let inspected = store
        .inspect_terminal_control_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(inspected.state, TerminalControlState::Prepared);
    assert!(inspected.dispatch_id.is_none());
    assert_eq!(
        inspected.broker_action_state,
        Some(ActionState::OutcomeUnknown)
    );
    assert_eq!(inspected.broker_dispatch_id, Some(dispatch_id));
    assert_eq!(
        metadata(&store, &control).pointer("/items/0/control_state"),
        Some(&json!("outcome_unknown"))
    );
    assert_eq!(
        metadata(&store, &control).pointer("/items/0/control_revision"),
        Some(&json!(0))
    );
    assert!(store.commit_broker_phase(dispatch).is_err());
    assert!(
        store
            .commit_command(registered_request(
                &store,
                &read(&store, TERMINAL),
                CONTROL + 1
            ))
            .is_err()
    );
    assert!(matches!(
        store.commit_command(request),
        Ok(CommitOutcome::IdempotentReplay(_))
    ));
    assert_eq!(state(&store, &control), before);
    assert!(!store.config().data_root.join("process-hosts").exists());
}

#[test]
fn t_f011_003_same_epoch_classifies_renewal_without_successor_or_effect() {
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let before = counts(&store);
    let request = registered_request(&store, &allocation, CONTROL);
    let result = store.commit_command(request.clone()).unwrap();
    let CommitOutcome::Committed(receipt) = result else {
        unreachable!()
    };
    let renewal = store
        .inspect_terminal_renewal_request(&allocation.context.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(renewal.context.receipt_id, receipt.receipt_id);
    assert_eq!(renewal.context.lease_id, allocation.context.lease_id);
    assert_eq!(
        renewal.context.resource_fence,
        allocation.context.resource_fence
    );
    assert_eq!(
        renewal.state,
        unleash_store::terminal_allocations::terminal_renewal_types::TerminalRenewalState::Prepared
    );
    assert!(renewal.dispatch_id.is_none());
    assert!(
        store
            .inspect_terminal_control_request(
                &allocation.context.actor_id,
                request.envelope.command_id
            )
            .unwrap()
            .value
            .is_none()
    );
    assert!(
        matches!(store.commit_command(request).unwrap(),CommitOutcome::IdempotentReplay(ref replay) if replay==&receipt)
    );
    let mut expected = before;
    expected.6 += 1;
    assert_eq!(counts(&store), expected);
    store.read_snapshot(|conn,_| {
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM terminal_control_intents",[],|row|row.get::<_,i64>(0))?,0);
        let control:(i64,String,String)=conn.query_row("SELECT s.control_revision,s.lease_id,l.state FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        assert_eq!(control,(0,allocation.context.lease_id.to_string(),"held".into()));
        let expiry:i64=conn.query_row("SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",[allocation.context.lease_id.to_string()],|row|row.get(0))?;
        assert_eq!(expiry,allocation.context.lease_expires_at.as_i64());
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM terminal_renewal_intents",[],|row|row.get::<_,i64>(0))?,1);
        assert!(!conn.prepare("SELECT 1 FROM action_dispatches WHERE action_id=?1")?.exists([renewal.context.action_id.to_string()])?);
        Ok(())
    }).unwrap();
    assert!(!store.config().data_root.join("process-hosts").exists());
}

#[test]
fn t_f011_005_acquisition_result_does_not_cross_actor_ownership() {
    // ACL unit oracle only: this fixture's OS observation is synthetic. It does
    // not claim a second real peer UID or forge a daemon session/admission.
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let alice = &allocation.context.actor_id;
    let bob = ActorId::new("uid:fixture-bob").unwrap();
    let command = unleash_domain::CommandId::from_u128(TERMINAL + 1000);
    let ReceiptInspectionOutcome::Full(original) =
        store.inspect_command_receipt(alice, command).unwrap()
    else {
        unreachable!()
    };
    let reference = original.receipt.result_ref.unwrap();
    assert!(matches!(
        store.inspect_command_receipt(&bob, command),
        Err(StoreError::NotFound(_))
    ));
    let before = counts(&store);
    assert!(
        store
            .inspect_terminal_acquisition_result(alice, command, &reference, PROJECT)
            .is_ok(),
        "same-actor fixture is genuinely eligible before checking denial"
    );
    assert!(
        matches!(
            store.inspect_terminal_acquisition_result(&bob, command, &reference, PROJECT),
            Err(StoreError::NotFound(_))
        ),
        "even the exact known reference cannot bypass actor ownership"
    );
    assert_eq!(
        counts(&store),
        before,
        "result lookup is not a lease renewal or effect"
    );
}
