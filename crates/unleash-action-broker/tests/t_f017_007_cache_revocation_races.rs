// T-F017-007 / AC-F017-007 / REQ-SEC-007
//! Tests de revalidation mandat/politique/fence juste avant engagement, cache borné (<=300s et <=expiry),
//! invalidation instantanée sur révocation/suspension/politique, non-résurrection d'ancien receipt et course multi-thread.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionState, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, CallerIdentity, ConfigureGuardCommand, EffectsClass,
    GuardMode, IsolationMode, IsolationRequirements, NormalizedResourceRef, RevokeSubjectCommand,
    ScopeKind, SetPolicyCommand, SuspendSubjectCommand, TypedSubjectRef,
    compute_effective_authority,
};
use unleash_domain::{
    ActorId, AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, InvocationId, LineageFence, ModelPolicy, ProjectId, ReconcileStrategy,
    Revision, TimestampMs,
};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, ScriptedGuardOutcome, SecretResolutionSpy, TestIsolationProbe,
    local_test_dir,
};

fn fence(val: u64) -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(7),
        DaemonEpoch::INITIAL,
        Fence::new(val),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_007_cache_bounds_invalidation_revocation_and_multithread_race() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(707));
    let dir = local_test_dir("t007").unwrap();
    let root = dir.path().to_path_buf();
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let secret_spy = Arc::new(SecretResolutionSpy::in_memory());

    let project_id = ProjectId::from_u128(70);
    let agent_id = AgentId::from_u128(71);
    let actor_id = ActorId::new("agent-71").unwrap();

    let make_authority = |policy_rev: u64, scope_rev: u64| {
        compute_effective_authority(
            Some(project_id),
            Revision::new(policy_rev),
            Revision::new(scope_rev),
            &[AuthorityLayer {
                layer_name: String::from("project"),
                allowed_effects: Some(BTreeSet::from([EffectsClass::LocalWrite])),
                denied_effects: BTreeSet::new(),
                allowed_verbs: Some(BTreeSet::from([String::from("fs.write")])),
                denied_verbs: BTreeSet::new(),
                allowed_roots: Some(vec![root.clone()]),
                allowed_network_hosts: None,
                allowed_secret_refs: Some(BTreeSet::new()),
                model_policy: ModelPolicy::Forbidden,
                data_locality: DataLocality::LocalOnly,
                isolation_mode: IsolationMode::Standard,
                require_action_isolation: false,
                max_scope_kind: ScopeKind::Project,
                allow_delegation: false,
                allow_cross_repository: false,
            }],
        )
    };

    let broker = Arc::new(ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(fence(1)),
    ));

    let admin = CallerIdentity::admin(ActorId::new("uid:501").unwrap());
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        clock.clone(),
    ));
    broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/guard.sock")),
                guard_mode: Some(GuardMode::GuardRequired),
                ..ConfigureGuardCommand::default()
            },
            Some(double.clone()),
        )
        .await
        .unwrap();

    let build_intent = |payload_tag: &str, policy_rev: u64, scope_rev: u64, deadline_ms: i64| {
        let payload = serde_json::json!({ "file": "a.txt", "tag": payload_tag });
        let auth = make_authority(policy_rev, scope_rev);
        ActionIntent {
            action_id: ids.new_action_id(),
            subject: ActionSubject::new_invocation(
                actor_id.clone(),
                String::from("inv:707"),
                InvocationId::from_u128(72),
                AttemptId::from_u128(73),
                Some(agent_id),
                None,
                None,
            )
            .unwrap(),
            project_id: Some(project_id),
            verb: String::from("fs.write"),
            resource_refs: vec![NormalizedResourceRef::Path(root.join("a.txt"))],
            payload_digest: ActionIntent::compute_payload_digest(&payload).unwrap(),
            reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
            effects_class: EffectsClass::LocalWrite,
            scope_revision: Revision::new(scope_rev),
            policy_revision: Revision::new(policy_rev),
            lineage_fence: fence(1),
            action_deadline: TimestampMs::new(deadline_ms),
            isolation_requirements: IsolationRequirements::from_authority(&auth),
            secret_refs: Vec::new(),
            provenance: ActionProvenance::EngineInternal,
        }
    };

    // 1. Décision sans expiry -> JAMAIS mise en cache.
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: None,
        constraints: Vec::new(),
    });
    let auth1 = make_authority(0, 1);
    let i1 = build_intent("same", 0, 1, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i1, &auth1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 1);

    // Avec expiry = 600s -> mise en cache, mais plafonnée à 300s (<= 300s).
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(600_000),
        constraints: Vec::new(),
    });
    let i2 = build_intent("same", 0, 1, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i2, &auth1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 2);

    // Même digest / policy_revision / scope_revision à t+100s -> HIT cache (toujours 2 appels).
    clock.advance(Duration::from_secs(100));
    let i3 = build_intent("same", 0, 1, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i3, &auth1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 2);

    // Digest différent -> MISS cache (3 appels).
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(600_000),
        constraints: Vec::new(),
    });
    let i4 = build_intent("different", 0, 1, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i4, &auth1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 3);

    // À t+301s (> 300s alors que expiry était 600s) -> MISS cache car plafond dur 300s!
    clock.advance(Duration::from_secs(201));
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(600_000),
        constraints: Vec::new(),
    });
    let i5 = build_intent("same", 0, 1, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i5, &auth1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 4);

    // Changement scope_revision -> MISS cache.
    let auth_scope2 = make_authority(0, 2);
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(600_000),
        constraints: Vec::new(),
    });
    let i6 = build_intent("same", 0, 2, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i6, &auth_scope2).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 5);

    // Changement de politique (SetPolicy) -> invalidation instantanée du cache.
    broker
        .set_policy(
            &admin,
            SetPolicyCommand {
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                isolation_mode: IsolationMode::Standard,
                reason: String::from("policy bump"),
            },
        )
        .unwrap();
    let auth_pol1 = make_authority(1, 2);
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: Vec::new(),
    });
    let i7 = build_intent("same", 1, 2, clock.utc_ms() + 1_000_000);
    assert!(broker.authorize_action(i7, &auth_pol1).await.is_ok());
    assert_eq!(double.evaluate_call_count(), 6);

    // Suspension invalide le cache et bloque toute nouvelle action, puis ResumeSubject rétablit (sans cache résiduel).
    broker
        .suspend_subject(
            &admin,
            SuspendSubjectCommand {
                subject: TypedSubjectRef::Agent(agent_id),
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("pause"),
            },
        )
        .await
        .unwrap();
    let i_susp = build_intent("same", 1, 2, clock.utc_ms() + 1_000_000);
    let susp_err = broker
        .authorize_action(i_susp, &auth_pol1)
        .await
        .unwrap_err();
    assert_eq!(
        susp_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::SubjectSuspended)
    );
    broker
        .resume_subject(
            &admin,
            unleash_action_broker::ResumeSubjectCommand {
                subject: TypedSubjectRef::Agent(agent_id),
                project_id: Some(project_id),
                expected_revision: Revision::new(1),
                reason: String::from("resume"),
            },
        )
        .await
        .unwrap();
    // L'entrée précédemment en cache ne doit plus être servie (réévaluation réelle = 7e appel).
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: Vec::new(),
    });
    let i_resumed = build_intent("same", 1, 2, clock.utc_ms() + 1_000_000);
    let aid_resumed = i_resumed.action_id;
    let AuthorizationOutcome::Authorized(dec_before_revoke) = broker
        .authorize_action(i_resumed, &auth_pol1)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    assert_eq!(double.evaluate_call_count(), 7);

    // 2. Révocation entre Allow et ExecuteAuthorized -> refusée, ancien receipt ne ressuscite pas le droit.
    broker
        .revoke_subject(
            &admin,
            RevokeSubjectCommand {
                subject: TypedSubjectRef::Agent(agent_id),
                project_id: Some(project_id),
                expected_revision: Revision::new(2),
                reason: String::from("compromised"),
            },
        )
        .await
        .unwrap();
    let rev_err = broker
        .execute_authorized(
            aid_resumed,
            &dec_before_revoke.decision_ref,
            fence(1),
            &auth_pol1,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        rev_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::SubjectRevoked)
    );
    assert_eq!(broker.action_state(aid_resumed), Some(ActionState::Denied));
    // Rejouer une seconde fois le même ancien decision_ref reste rejeté.
    let replay_err = broker
        .execute_authorized(
            aid_resumed,
            &dec_before_revoke.decision_ref,
            fence(1),
            &auth_pol1,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        replay_err.to_ipc_error(None).code,
        ErrorCode::ContractViolation
    );
    spy.assert_zero_dispatches();

    // 3. Course multi-thread déterministe entre autorisation/dispatch et révocation d'un sujet frais.
    let race_agent = AgentId::from_u128(777);
    let race_actor = ActorId::new("agent-race").unwrap();
    let race_intent_id = ids.new_action_id();
    let race_payload = serde_json::json!({ "race": true });
    let race_intent = ActionIntent {
        action_id: race_intent_id,
        subject: ActionSubject::new_invocation(
            race_actor,
            String::from("inv:race"),
            InvocationId::from_u128(778),
            AttemptId::from_u128(779),
            Some(race_agent),
            None,
            None,
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.write"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("race.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&race_payload).unwrap(),
        reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
        effects_class: EffectsClass::LocalWrite,
        scope_revision: Revision::new(2),
        policy_revision: Revision::new(1),
        lineage_fence: fence(1),
        action_deadline: TimestampMs::new(clock.utc_ms() + 1_000_000),
        isolation_requirements: IsolationRequirements::from_authority(&auth_pol1),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: Vec::new(),
    });
    let AuthorizationOutcome::Authorized(race_dec) = broker
        .authorize_action(race_intent, &auth_pol1)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let b1 = broker.clone();
    let b2 = broker.clone();
    let bar1 = barrier.clone();
    let bar2 = barrier.clone();
    let spy_race = spy.clone();
    let sec_race = secret_spy.clone();
    let auth_race = auth_pol1.clone();
    let admin_race = admin.clone();
    let dec_ref = race_dec.decision_ref.clone();

    let h_exec = tokio::spawn(async move {
        bar1.wait().await;
        b1.execute_authorized(
            race_intent_id,
            &dec_ref,
            fence(1),
            &auth_race,
            spy_race.as_ref(),
            sec_race.as_ref(),
        )
        .await
    });
    let h_revoke = tokio::spawn(async move {
        bar2.wait().await;
        b2.revoke_subject(
            &admin_race,
            RevokeSubjectCommand {
                subject: TypedSubjectRef::Agent(race_agent),
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("race revoke"),
            },
        )
        .await
    });

    let (exec_res, rev_res) = tokio::join!(h_exec, h_revoke);
    rev_res.unwrap().unwrap();
    // Soit la révocation a gagné avant CommitDispatchMarker -> SubjectRevoked & 0 dispatch,
    // soit le dispatch était déjà committé -> ObservedSuccess (jamais d'état corrompu).
    match exec_res.unwrap() {
        Ok(receipt) => {
            assert_eq!(receipt.action_id, race_intent_id);
            assert_eq!(spy.dispatch_count(), 1);
        }
        Err(err) => {
            assert_eq!(
                err.to_ipc_error(None).reason_code,
                Some(ReasonCode::SubjectRevoked)
            );
            spy.assert_zero_dispatches();
        }
    }
}
