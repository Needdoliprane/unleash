// T-F017-002 / AC-F017-002 / REQ-SEC-002
//! Tests que chaque effet passe par `ActionBroker` avec intent, décision, `action_id` et receipt,
//! et qu'un appel direct non autorisé est refusé.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionState, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, EffectsClass, GuardDecisionRef, IsolationMode,
    IsolationRequirements, NormalizedResourceRef, ScopeKind, compute_effective_authority,
};
use unleash_domain::{
    ActorId, AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, InvocationId, LineageFence, ModelPolicy, ProjectId, ReconcileStrategy,
    Revision, TerminalId, TimestampMs,
};
use unleash_protocol::ErrorCode;
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe, local_test_dir,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(2),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_002_all_effect_classes_produce_intent_decision_receipt_and_direct_bypass_fails() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(202));
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let secret_spy = Arc::new(SecretResolutionSpy::in_memory());

    let dir = local_test_dir("t002").unwrap();
    let repo_root = dir.path().to_path_buf();
    let project_id = ProjectId::from_u128(22);

    let layer = AuthorityLayer {
        layer_name: String::from("mandate"),
        allowed_effects: Some(BTreeSet::from([
            EffectsClass::GitLocalWrite,
            EffectsClass::ProcessExec,
            EffectsClass::NetworkEgress,
            EffectsClass::TerminalInput,
        ])),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from([
            String::from("git.commit"),
            String::from("process.spawn"),
            String::from("http.post"),
            String::from("pty.send"),
        ])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: Some(vec![repo_root.clone()]),
        allowed_network_hosts: Some(BTreeSet::from([String::from("api.example.com")])),
        allowed_secret_refs: Some(BTreeSet::new()),
        model_policy: ModelPolicy::AllowedBounded,
        data_locality: DataLocality::ApprovedRemote,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: ScopeKind::Project,
        allow_delegation: false,
        allow_cross_repository: false,
    };
    let authority = compute_effective_authority(
        Some(project_id),
        Revision::INITIAL,
        Revision::new(1),
        &[layer],
    );

    let broker = ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(test_fence()),
    );

    let cases = [
        (
            "git.commit",
            EffectsClass::GitLocalWrite,
            NormalizedResourceRef::GitRef {
                repo: repo_root.clone(),
                ref_name: String::from("refs/heads/feature"),
            },
        ),
        (
            "process.spawn",
            EffectsClass::ProcessExec,
            NormalizedResourceRef::Path(repo_root.join("build.sh")),
        ),
        (
            "http.post",
            EffectsClass::NetworkEgress,
            NormalizedResourceRef::NetworkEndpoint {
                host: String::from("api.example.com"),
                port: Some(443),
                is_private_or_loopback: false,
            },
        ),
        (
            "pty.send",
            EffectsClass::TerminalInput,
            NormalizedResourceRef::Terminal(TerminalId::from_u128(99)),
        ),
    ];

    for (verb, effects_class, res) in cases {
        let action_id = ids.new_action_id();
        let digest =
            ActionIntent::compute_payload_digest(&serde_json::json!({ "verb": verb })).unwrap();
        let intent = ActionIntent {
            action_id,
            subject: ActionSubject::new_invocation(
                ActorId::new("worker-1").unwrap(),
                String::from("inv-002"),
                InvocationId::from_u128(20),
                AttemptId::from_u128(21),
                Some(AgentId::from_u128(22)),
                None,
                None,
            )
            .unwrap(),
            project_id: Some(project_id),
            verb: verb.to_owned(),
            resource_refs: vec![res],
            payload_digest: digest,
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class,
            scope_revision: Revision::new(1),
            policy_revision: Revision::INITIAL,
            lineage_fence: test_fence(),
            action_deadline: TimestampMs::new(1_767_225_660_000),
            isolation_requirements: IsolationRequirements::from_authority(&authority),
            secret_refs: Vec::new(),
            provenance: ActionProvenance::EngineInternal,
        };

        let AuthorizationOutcome::Authorized(decision) =
            broker.authorize_action(intent, &authority).await.unwrap()
        else {
            panic!("expected Authorized in BrokerOnly mode");
        };
        assert_eq!(decision.action_id, action_id);
        assert_eq!(
            broker.action_state(action_id),
            Some(ActionState::Authorized)
        );

        let receipt = broker
            .execute_authorized(
                action_id,
                &decision.decision_ref,
                test_fence(),
                &authority,
                spy.as_ref(),
                secret_spy.as_ref(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.action_id, action_id);
        assert_eq!(receipt.decision_ref, decision.decision_ref);
        assert_eq!(
            broker.action_state(action_id),
            Some(ActionState::ObservedSuccess)
        );
    }

    assert_eq!(spy.dispatch_count(), 4);

    // Tentative d'exécution directe sans autorisation préalable du broker (contournement) : refusée.
    let unbrokered_id = ids.new_action_id();
    let forged_ref = GuardDecisionRef::new("forged-direct-call").unwrap();
    let bypass_err = broker
        .execute_authorized(
            unbrokered_id,
            &forged_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        bypass_err.to_ipc_error(None).code,
        ErrorCode::NotFoundOrHidden
    );
    assert_eq!(spy.dispatch_count(), 4);
    let _ = PathBuf::new();
}
