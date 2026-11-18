// T-F017-009 / AC-F017-009 / REQ-SEC-009
//! Tests d'administration AI-Guard, santé/capacités, suspension/reprise locale vs propagation négociée,
//! distinction d'avec la révocation définitive et panne pendant révocation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer, BrokerAuditEvent,
    BrokerConfig, CallerIdentity, ConfigureGuardCommand, EffectsClass, GuardCapabilities,
    GuardMappingProfile, GuardMode, IsolationMode, IsolationRequirements, NormalizedResourceRef,
    RemotePropagationStatus, ResumeSubjectCommand, RevokeSubjectCommand, ScopeKind,
    SuspendSubjectCommand, TypedSubjectRef, compute_effective_authority,
};
use unleash_domain::{
    ActorId, AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, InvocationId, LineageFence, ModelPolicy, ProjectId, ReconcileStrategy,
    Revision, TimestampMs,
};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, TestIsolationProbe, local_test_dir,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(9),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_009_suspend_resume_distinct_from_revoke_and_remote_capabilities() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(909));
    let dir = local_test_dir("t009").unwrap();
    let root = dir.path().to_path_buf();
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());

    let project_id = ProjectId::from_u128(90);
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
        spy,
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(test_fence()),
    );

    let admin = CallerIdentity::admin(ActorId::new("uid:501").unwrap());
    let worker = CallerIdentity::worker(ActorId::new("worker:1").unwrap());

    // 1. Permission admin absente refusée pour SuspendSubject.
    let agent_id = AgentId::from_u128(91);
    let subject = TypedSubjectRef::Agent(agent_id);
    let unauth_err = broker
        .suspend_subject(
            &worker,
            SuspendSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("unauthorized"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        unauth_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    // 2. GuardPort qui NE négocie PAS SuspendSubject : suspension locale fonctionne + limitation visible.
    let double_no_susp = Arc::new(DeterministicGuardDouble::with_capabilities(
        clock.clone(),
        GuardCapabilities {
            protocol_version: String::from("unleash.guard_port.v1"),
            mapping_profile: GuardMappingProfile::NoneAccepted,
            supports_suspend_resume: false,
            supports_execute_authorized_action: false,
            supports_remote_approval: false,
            supports_remote_policy_write: false,
            supported_obligations: BTreeSet::new(),
        },
    ));
    let health_no_susp = broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/guard.sock")),
                guard_mode: Some(GuardMode::GuardRequired),
                ..ConfigureGuardCommand::default()
            },
            Some(double_no_susp.clone()),
        )
        .await
        .unwrap();
    assert!(
        !health_no_susp
            .capabilities
            .as_ref()
            .unwrap()
            .supports_suspend_resume
    );
    assert!(!health_no_susp.remote_limitations.is_empty());

    let susp_receipt = broker
        .suspend_subject(
            &admin,
            SuspendSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("revue sécurité"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        susp_receipt.remote_propagation,
        RemotePropagationStatus::UnsupportedByGuard
    );
    assert_eq!(double_no_susp.suspend_call_count(), 0);

    let build_agent_intent = || ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_invocation(
            ActorId::new("agent-91").unwrap(),
            String::from("inv:909"),
            InvocationId::from_u128(92),
            AttemptId::from_u128(93),
            Some(agent_id),
            None,
            None,
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.write"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("out.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"ok": 1}))
            .unwrap(),
        reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
        effects_class: EffectsClass::LocalWrite,
        scope_revision: Revision::new(1),
        policy_revision: Revision::new(0),
        lineage_fence: test_fence(),
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };

    // Pendant suspension -> SUBJECT_SUSPENDED.
    let err_susp = broker
        .authorize_action(build_agent_intent(), &authority)
        .await
        .unwrap_err();
    assert_eq!(
        err_susp.to_ipc_error(None).reason_code,
        Some(ReasonCode::SubjectSuspended)
    );

    // ResumeSubject rétablit le sujet avec la MÊME identité.
    let res_receipt = broker
        .resume_subject(
            &admin,
            ResumeSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::new(1),
                reason: String::from("fin revue"),
            },
        )
        .await
        .unwrap();
    assert_eq!(res_receipt.subject, subject);
    assert!(matches!(
        broker
            .authorize_action(build_agent_intent(), &authority)
            .await
            .unwrap(),
        unleash_action_broker::AuthorizationOutcome::Authorized(_)
    ));

    // 3. GuardPort QUI négocie SuspendSubject + panne pendant RevokeSubject.
    let double_with_susp = Arc::new(DeterministicGuardDouble::with_capabilities(
        clock.clone(),
        GuardCapabilities {
            protocol_version: String::from("unleash.guard_port.v1"),
            mapping_profile: GuardMappingProfile::NoneAccepted,
            supports_suspend_resume: true,
            supports_execute_authorized_action: false,
            supports_remote_approval: false,
            supports_remote_policy_write: false,
            supported_obligations: BTreeSet::new(),
        },
    ));
    broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/guard2.sock")),
                guard_mode: Some(GuardMode::GuardRequired),
                ..ConfigureGuardCommand::default()
            },
            Some(double_with_susp.clone()),
        )
        .await
        .unwrap();

    let susp2 = broker
        .suspend_subject(
            &admin,
            SuspendSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::new(2),
                reason: String::from("suspension propagée"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        susp2.remote_propagation,
        RemotePropagationStatus::Propagated
    );
    assert_eq!(double_with_susp.suspend_call_count(), 1);

    // Simuler une panne du guard pendant RevokeSubject : la révocation locale reste engagée,
    // un incident est ouvert, et ResumeSubject refuse car la révocation est définitive.
    double_with_susp.set_fail_revoke(true);
    let revocation_receipt = broker
        .revoke_subject(
            &admin,
            RevokeSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::new(3),
                reason: String::from("révocation définitive"),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        revocation_receipt.remote_propagation,
        RemotePropagationStatus::PropagationFailed { .. }
    ));

    let post_rev_err = broker
        .authorize_action(build_agent_intent(), &authority)
        .await
        .unwrap_err();
    assert_eq!(
        post_rev_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::SubjectRevoked)
    );

    let resume_after_revoke = broker
        .resume_subject(
            &admin,
            ResumeSubjectCommand {
                subject,
                project_id: Some(project_id),
                expected_revision: Revision::new(4),
                reason: String::from("interdit"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        resume_after_revoke.to_ipc_error(None).reason_code,
        Some(ReasonCode::SubjectRevoked)
    );
    assert_eq!(
        resume_after_revoke.to_ipc_error(None).code,
        ErrorCode::CapabilityDenied
    );
    assert!(
        broker
            .audit_events()
            .iter()
            .any(|e| matches!(e, BrokerAuditEvent::IncidentOpened { .. }))
    );
}
