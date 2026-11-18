// T-F017-010 / AC-F017-010 / REQ-SEC-010
//! Tests de traçabilité causale, issue inconnue (`OutcomeUnknown`), journal distant indisponible
//! et déduplication/rejet de receipt dupliqué ou obsolète.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionState, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, CallerIdentity, ConfigureGuardCommand, EffectsClass,
    GuardMode, IsolationMode, IsolationRequirements, NormalizedResourceRef, ScopeKind,
    compute_effective_authority,
};
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence, IdGeneratorPort, LineageFence,
    ModelPolicy, ProjectId, ReconcileOutcome, ReconcileStrategy, Revision, TimestampMs,
};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, SecretResolutionSpy, SimulatedExecutionFailure, TestIsolationProbe,
    local_test_dir,
};

fn fence(val: u64) -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(10),
        DaemonEpoch::INITIAL,
        Fence::new(val),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_010_crash_after_dispatch_produces_outcome_unknown_and_duplicate_receipt_rejected() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(1010));
    let dir = local_test_dir("t010").unwrap();
    let root = dir.path().to_path_buf();
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let secret_spy = Arc::new(SecretResolutionSpy::in_memory());

    let project_id = ProjectId::from_u128(100);
    let authority = compute_effective_authority(
        Some(project_id),
        Revision::INITIAL,
        Revision::new(1),
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
    );

    let broker = ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(fence(1)),
    );

    let admin = CallerIdentity::admin(ActorId::new("uid:501").unwrap());
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        clock.clone(),
    ));
    double.set_fail_audit_read(true);
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

    // 1. Action autorisée puis crash/ambiguïté après effet (C4/C5) -> OutcomeUnknown, jamais succès.
    let intent = ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_control_plane(
            ActorId::new("uid:501").unwrap(),
            String::from("cmd:1010"),
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.write"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("crash.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"crash": true}))
            .unwrap(),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::LocalWrite,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: fence(1),
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    let aid = intent.action_id;
    let AuthorizationOutcome::Authorized(dec) =
        broker.authorize_action(intent, &authority).await.unwrap()
    else {
        panic!("expected authorized");
    };

    spy.set_next_execution_failure(SimulatedExecutionFailure::OutcomeUnknown {
        reason: String::from("crash après envoi avant receipt"),
    });
    let unknown_err = broker
        .execute_authorized(
            aid,
            &dec.decision_ref,
            fence(1),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    let wire_unknown = unknown_err.to_ipc_error(None);
    assert_eq!(wire_unknown.code, ErrorCode::OutcomeUnknown);
    assert_eq!(wire_unknown.reason_code, Some(ReasonCode::UnresolvedEffect));
    assert_eq!(broker.action_state(aid), Some(ActionState::OutcomeUnknown));

    // Journal distant indisponible lors de la réconciliation -> reste OutcomeUnknown ou conserve diagnostic.
    let rec_unk = broker
        .reconcile_action(aid, ReconcileOutcome::Unknown, None)
        .await
        .unwrap();
    assert_eq!(rec_unk.state, ActionState::OutcomeUnknown);
    assert!(rec_unk.remote_audit_unavailable);

    // Puis réconciliation avec preuve confirmée -> ObservedSuccess avec corrélation complète.
    let rec_ok = broker
        .reconcile_action(
            aid,
            ReconcileOutcome::ConfirmedSuccess,
            Some(String::from("evidence:sha256:abc")),
        )
        .await
        .unwrap();
    assert_eq!(rec_ok.state, ActionState::ObservedSuccess);
    assert_eq!(broker.action_state(aid), Some(ActionState::ObservedSuccess));

    let audit = broker.action_audit(aid).unwrap();
    assert_eq!(audit.action_id, aid);
    assert_eq!(audit.decision_ref.as_ref(), Some(&dec.decision_ref));
    assert_eq!(
        audit.mapping_profile_id,
        "unleash.guard_mapping.none_accepted.v1"
    );

    // 2. Livraison d'un receipt dupliqué sur action déjà terminale -> rejeté (TerminalImmutable), zéro double effet.
    let dup_err = broker
        .apply_external_receipt(
            aid,
            rec_ok.dispatch_id,
            fence(1),
            ReconcileOutcome::ConfirmedSuccess,
            Some(String::from("duplicate")),
        )
        .await
        .unwrap_err();
    assert_eq!(
        dup_err.to_ipc_error(None).code,
        ErrorCode::ContractViolation
    );

    // Livraison d'un receipt avec ancien fence (C9) -> StaleFence.
    let stale_err = broker
        .apply_external_receipt(
            aid,
            rec_ok.dispatch_id,
            fence(0),
            ReconcileOutcome::ConfirmedSuccess,
            Some(String::from("late-c9")),
        )
        .await
        .unwrap_err();
    assert_eq!(stale_err.to_ipc_error(None).code, ErrorCode::StaleFence);
}
