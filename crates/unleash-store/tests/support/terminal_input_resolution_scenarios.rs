//! One-use Store permits only. Native effects remain outside this SQLite fixture.
use super::*;

#[path = "terminal_input_unknown_scenarios.rs"]
mod unknown_scenarios;
use unleash_store::terminal_allocations::terminal_input_types::ResolveTerminalInputRequest;

fn input_dispatch(store: &UnleashStore, record: &TerminalInputRecord) -> DispatchId {
    let first = input_authorizing(record);
    let committed = store.commit_broker_phase(first.clone()).unwrap();
    let BrokerPhase::Authorizing { intent } = &first.phase else {
        unreachable!()
    };
    let decision = BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: first.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "synthetic-sqlite-input-allow".into(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(intent.action_deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let authorized = store
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
    let dispatch_id = DispatchId::from_uuid(record.context.action_id.as_uuid());
    store
        .commit_broker_phase(BrokerPhaseRequest {
            expected_revision: authorized.revision,
            phase: BrokerPhase::Dispatch {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                decision,
                dispatch_id,
                fence: intent.lineage_fence,
            },
            ..first.clone()
        })
        .unwrap();
    dispatch_id
}

fn resolution(
    record: &TerminalInputRecord,
    dispatch_id: DispatchId,
) -> ResolveTerminalInputRequest {
    ResolveTerminalInputRequest {
        action_id: record.context.action_id,
        dispatch_id,
        authority_generation: record.context.authority_generation,
        expected_revision: record.revision,
        context_digest: record.context_digest,
        caller: CallerAuthority {
            actor_id: record.context.actor_id.clone(),
            role: CallerRole::HumanOperator,
        },
    }
}

#[test]
fn t_f011_005_input_resolution_needs_exact_dispatch_and_is_one_shot() {
    let (store, record, _directory) = prepared_input();
    let action = record.context.action_id;
    let missing = DispatchId::from_u128(INPUT + 70);
    assert!(
        store
            .resolve_terminal_input(resolution(&record, missing))
            .is_err()
    );
    let dispatch = input_dispatch(&store, &record);
    for case in 0..3 {
        let mut wrong = resolution(&record, dispatch);
        match case {
            0 => wrong.dispatch_id = missing,
            1 => wrong.context_digest = canonical_sha256(&json!({"wrong":"context"})).unwrap(),
            _ => wrong.caller.actor_id = ActorId::new("uid:fixture-bob").unwrap(),
        }
        assert!(
            store.resolve_terminal_input(wrong).is_err(),
            "resolution guard {case}"
        );
        let pending = store
            .inspect_terminal_input_request(&record.context.actor_id, record.context.command_id)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(pending.state, TerminalInputState::Prepared);
        assert_eq!(pending.revision, Revision::new(1));
        assert_eq!(pending.broker_dispatch_id, Some(dispatch));
    }
    let resolved = store
        .resolve_terminal_input(resolution(&record, dispatch))
        .unwrap();
    assert_eq!(resolved.input.context.action_id, action);
    assert_eq!(resolved.input.state, TerminalInputState::WriteResolved);
    assert_eq!(resolved.input.revision, Revision::new(2));
    assert_eq!(resolved.input.dispatch_id, Some(dispatch));
    assert!(resolved.expected_os_identity.pid > 0);
    assert_eq!(
        resolved.expected_os_identity.boot_identity,
        record.context.daemon_boot_identity
    );
    assert!(
        store
            .resolve_terminal_input(resolution(&record, dispatch))
            .is_err()
    );
    let facts:(i64,i64,i64,i64)=store.read_snapshot(|conn,_|Ok((
        conn.query_row("SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",[record.context.terminal_id.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",[record.context.lease_id.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1",[action.to_string()],|row|row.get(0))?,
        conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_type='terminal_input_intent' AND aggregate_id=?1 AND aggregate_revision=2",[action.to_string()],|row|row.get(0))?,
    ))).unwrap();
    assert_eq!(
        facts,
        (0, record.context.previous_lease_expires_at.as_i64(), 1, 1)
    );
}

#[test]
fn t_f011_005_input_resolution_commit_uncertainty_never_promotes_sequence_or_lease() {
    for point in [
        StoreCutpoint::C1BeforeCommandCommit,
        StoreCutpoint::C2AfterCommandCommit,
    ] {
        let (store, _clock, fault, allocation, _directory) = observed_with_cutpoint(false, point);
        let before = control_facts(&store, &allocation);
        let request = input_request(&store, &allocation, INPUT + 80);
        store.commit_command(request.clone()).unwrap();
        let record = store
            .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
            .unwrap()
            .value
            .unwrap();
        let dispatch = input_dispatch(&store, &record);
        fault.armed.store(true, Ordering::Release);
        let result = store.resolve_terminal_input(resolution(&record, dispatch));
        assert!(
            matches!(result, Err(StoreError::CutpointInjected { .. })),
            "point={point:?}: {result:?}"
        );
        let after = store
            .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(
            after.state,
            if point == StoreCutpoint::C1BeforeCommandCommit {
                TerminalInputState::Prepared
            } else {
                TerminalInputState::WriteResolved
            }
        );
        assert_eq!(after.broker_dispatch_id, Some(dispatch));
        let facts = control_facts(&store, &allocation);
        assert_eq!((facts.0, facts.1), (before.0, before.1));
        assert_eq!((facts.2, facts.3), (before.2 + 1, before.3 + 1));
        if point == StoreCutpoint::C2AfterCommandCommit {
            assert!(
                store
                    .resolve_terminal_input(resolution(&record, dispatch))
                    .is_err()
            );
        }
    }
}
