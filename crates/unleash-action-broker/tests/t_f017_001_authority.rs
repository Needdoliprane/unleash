// T-F017-001 / AC-F017-001 / REQ-SEC-001
//! Tests d'intersection stricte d'autorité et refus d'auto-élévation des skills/rôles.

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
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer, BrokerConfig,
    ConfigureGuardCommand, EffectiveAuthority, EffectsClass, GuardMode, IsolationMode,
    IsolationRequirements, NormalizedResourceRef, compute_effective_authority,
    validate_no_permission_expansion,
};
use unleash_domain::{
    ActorId, AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, InvocationId, LineageFence, MissionId, ModelPolicy, ProjectId,
    ReconcileStrategy, Revision, TimestampMs,
};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, SecretResolutionSpy, TestIsolationProbe,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(1),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_001_skill_cannot_elevate_prohibited_effect_or_contradict_project_policy() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(101));
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let secret_spy = Arc::new(SecretResolutionSpy::in_memory());

    let project_id = ProjectId::from_u128(10);
    let worktree = PathBuf::from("/tmp/unleash-t001/worktree");

    // Politique projet interdit GitRemote / NetworkEgress, impose LocalOnly et Forbidden sur le run.
    let project_layer = AuthorityLayer {
        layer_name: String::from("project_policy"),
        allowed_effects: Some(BTreeSet::from([
            EffectsClass::LocalRead,
            EffectsClass::LocalWrite,
            EffectsClass::GitLocalWrite,
        ])),
        denied_effects: BTreeSet::from([EffectsClass::GitRemote, EffectsClass::NetworkEgress]),
        allowed_verbs: Some(BTreeSet::from([
            String::from("fs.read"),
            String::from("fs.write"),
            String::from("git.commit"),
        ])),
        denied_verbs: BTreeSet::from([String::from("git.push"), String::from("net.fetch")]),
        allowed_roots: Some(vec![worktree.clone()]),
        allowed_network_hosts: Some(BTreeSet::new()),
        allowed_secret_refs: Some(BTreeSet::new()),
        model_policy: ModelPolicy::Forbidden,
        data_locality: DataLocality::LocalOnly,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: unleash_action_broker::ScopeKind::Project,
        allow_delegation: false,
        allow_cross_repository: false,
    };

    // Rôle composé contradictoire + skill malveillant réclamant GitRemote, ModelDispatch, ApprovedRemote.
    let role_layer = AuthorityLayer {
        layer_name: String::from("composed_role_v1"),
        allowed_effects: Some(BTreeSet::from([
            EffectsClass::LocalRead,
            EffectsClass::GitLocalWrite,
            EffectsClass::ModelDispatch,
        ])),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from([
            String::from("git.commit"),
            String::from("model.call"),
        ])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: Some(vec![worktree.clone()]),
        allowed_network_hosts: Some(BTreeSet::from([String::from("api.external.invalid")])),
        allowed_secret_refs: None,
        model_policy: ModelPolicy::AllowedBounded,
        data_locality: DataLocality::ApprovedRemote,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: unleash_action_broker::ScopeKind::Global,
        allow_delegation: true,
        allow_cross_repository: true,
    };

    let skill_layer = AuthorityLayer {
        layer_name: String::from("malicious_skill_v1"),
        allowed_effects: Some(BTreeSet::from([
            EffectsClass::GitRemote,
            EffectsClass::NetworkEgress,
            EffectsClass::LocalRead,
        ])),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from([
            String::from("git.push"),
            String::from("fs.read"),
        ])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: Some(vec![worktree.clone()]),
        allowed_network_hosts: Some(BTreeSet::from([String::from("evil.example.com")])),
        allowed_secret_refs: None,
        model_policy: ModelPolicy::AllowedBounded,
        data_locality: DataLocality::ApprovedRemote,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: unleash_action_broker::ScopeKind::Global,
        allow_delegation: true,
        allow_cross_repository: false,
    };

    let effective: EffectiveAuthority = compute_effective_authority(
        Some(project_id),
        Revision::new(1),
        Revision::new(1),
        &[project_layer.clone(), role_layer, skill_layer],
    );

    // L'intersection stricte doit conserver ModelPolicy::Forbidden, DataLocality::LocalOnly,
    // aucun hôte réseau, et refuser GitRemote/NetworkEgress/ModelDispatch.
    assert_eq!(effective.model_policy, ModelPolicy::Forbidden);
    assert_eq!(effective.data_locality, DataLocality::LocalOnly);
    assert!(effective.allowed_network_hosts.is_empty());
    assert!(!effective.allows_effect(EffectsClass::GitRemote));
    assert!(!effective.allows_effect(EffectsClass::ModelDispatch));
    assert!(!effective.allows_effect(EffectsClass::NetworkEgress));
    assert!(!effective.limiting_traces.is_empty());
    assert!(
        effective
            .limiting_traces
            .iter()
            .any(|t| t.layer_name == "project_policy")
    );

    // Une tentative explicite d'élargissement lors de la publication/liaison échoue PERMISSION_EXPANSION_DENIED.
    let project_ceiling = compute_effective_authority(
        Some(project_id),
        Revision::new(1),
        Revision::new(1),
        &[project_layer],
    );
    let expansion_err = validate_no_permission_expansion(
        &BTreeSet::from([EffectsClass::GitRemote]),
        &BTreeSet::from([String::from("git.push")]),
        &BTreeSet::from([String::from("evil.example.com")]),
        Some(ModelPolicy::AllowedBounded),
        Some(DataLocality::ApprovedRemote),
        &project_ceiling,
    )
    .unwrap_err();
    let ipc_err = expansion_err.to_ipc_error(None);
    assert_eq!(ipc_err.code, ErrorCode::CapabilityDenied);
    assert_eq!(
        ipc_err.reason_code,
        Some(ReasonCode::PermissionExpansionDenied)
    );

    // Soumission d'une action git.push via le broker -> refusée avant effet et avant GuardPort.
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
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        clock.clone(),
    ));
    let admin = unleash_action_broker::CallerIdentity::admin(ActorId::new("uid:501").unwrap());
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

    let payload = serde_json::json!({ "remote": "origin", "branch": "main" });
    let digest = ActionIntent::compute_payload_digest(&payload).unwrap();
    let intent = ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_invocation(
            ActorId::new("agent-n").unwrap(),
            String::from("invocation:1"),
            InvocationId::from_u128(1),
            AttemptId::from_u128(2),
            Some(AgentId::from_u128(3)),
            Some(MissionId::from_u128(4)),
            None,
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("git.push"),
        resource_refs: vec![NormalizedResourceRef::GitRef {
            repo: worktree,
            ref_name: String::from("refs/heads/main"),
        }],
        payload_digest: digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::GitRemote,
        scope_revision: Revision::new(1),
        policy_revision: Revision::new(1),
        lineage_fence: test_fence(),
        action_deadline: TimestampMs::new(1_767_225_660_000),
        isolation_requirements: IsolationRequirements::from_authority(&effective),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };

    let err = broker
        .authorize_action(intent, &effective)
        .await
        .unwrap_err();
    let wire = err.to_ipc_error(None);
    assert_eq!(wire.code, ErrorCode::CapabilityDenied);
    assert_eq!(wire.reason_code, Some(ReasonCode::PolicyDenied));
    assert_eq!(double.evaluate_call_count(), 0);
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();
}
