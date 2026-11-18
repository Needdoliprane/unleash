// T-F017-004 / AC-F017-004 / REQ-SEC-004
//! Tests de distinction client administratif vs capability worker, rejet d'UID étranger et déclaration explicite same-UID.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use unleash_action_broker::{
    ActionBroker, ApprovalDecision, BrokerAuditEvent, BrokerConfig, CallerIdentity,
    ConfigureGuardCommand, GuardMode, IsolationMode, ResolveActionApprovalCommand,
    RevokeSubjectCommand, SetPolicyCommand, SuspendSubjectCommand, TypedSubjectRef,
};
use unleash_domain::{
    ActorId, ApprovalScopeId, AuthorityGeneration, DaemonEpoch, Fence, IdGeneratorPort,
    LineageFence, ProjectId, Revision, Sha256Digest,
};
use unleash_platform::{PeerCredentials, effective_uid, verify_peer_uid};
use unleash_protocol::{ClientKind, ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer, TestIsolationProbe,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(4),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_004_worker_cannot_invoke_admin_operations_and_other_uid_rejected() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(404));
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());

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

    let worker = CallerIdentity::worker(ActorId::new("worker:uid:501:pid:9999").unwrap());
    let project_id = ProjectId::from_u128(40);

    // Worker tente SetPolicy -> OPERATION_NOT_AUTHORIZED + IncidentOpened.
    let err_policy = broker
        .set_policy(
            &worker,
            SetPolicyCommand {
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                isolation_mode: IsolationMode::Standard,
                reason: String::from("escalade worker"),
            },
        )
        .unwrap_err();
    let wire_policy = err_policy.to_ipc_error(None);
    assert_eq!(wire_policy.code, ErrorCode::CapabilityDenied);
    assert_eq!(
        wire_policy.reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    // Worker tente ConfigureGuard -> OPERATION_NOT_AUTHORIZED.
    let err_guard = broker
        .configure_guard(
            &worker,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/evil.sock")),
                guard_mode: Some(GuardMode::BrokerOnly),
                ..ConfigureGuardCommand::default()
            },
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        err_guard.to_ipc_error(None).reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    // Worker tente SuspendSubject et RevokeSubject -> OPERATION_NOT_AUTHORIZED.
    let subject = TypedSubjectRef::Principal(ActorId::new("user-1").unwrap());
    let err_suspend = broker
        .suspend_subject(
            &worker,
            SuspendSubjectCommand {
                subject: subject.clone(),
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("worker forgery"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        err_suspend.to_ipc_error(None).reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    let err_revoke = broker
        .revoke_subject(
            &worker,
            RevokeSubjectCommand {
                subject,
                project_id: Some(project_id),
                expected_revision: Revision::INITIAL,
                reason: String::from("worker forgery"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        err_revoke.to_ipc_error(None).reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    // Worker tente ResolveActionApproval -> OPERATION_NOT_AUTHORIZED.
    let err_approve = broker
        .resolve_action_approval(
            &worker,
            ResolveActionApprovalCommand {
                action_id: ids.new_action_id(),
                approval_scope: ApprovalScopeId::from_u128(1),
                payload_digest: Sha256Digest::from_bytes(b"payload"),
                decision: ApprovalDecision::Approve,
                expected_revision: Revision::INITIAL,
                reason: String::from("auto-approve"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        err_approve.to_ipc_error(None).reason_code,
        Some(ReasonCode::OperationNotAuthorized)
    );

    // Vérifier qu'un incident traçable a bien été ouvert.
    let events = broker.audit_events();
    let incidents = events
        .iter()
        .filter(|ev| matches!(ev, BrokerAuditEvent::IncidentOpened { .. }))
        .count();
    assert!(incidents >= 5);

    // Vérification autre UID via unleash_platform::verify_peer_uid et déclaration explicite same-UID.
    let current_uid = effective_uid();
    let foreign_peer = PeerCredentials {
        uid: current_uid.saturating_add(1),
        gid: 0,
        pid: Some(12345),
    };
    assert!(verify_peer_uid(foreign_peer, current_uid).is_err());

    let probe_caller = CallerIdentity {
        actor_id: ActorId::new("probe:1").unwrap(),
        client_kind: ClientKind::Probe,
        peer_uid: Some(current_uid),
        capabilities: std::collections::BTreeSet::default(),
    };
    assert!(
        broker
            .set_policy(
                &probe_caller,
                SetPolicyCommand {
                    project_id: Some(project_id),
                    expected_revision: Revision::INITIAL,
                    isolation_mode: IsolationMode::Strict,
                    reason: String::from("probe unauthorized"),
                }
            )
            .is_err()
    );

    // En mode Standard, la politique affiche explicitement "non confiné" (aucune garantie same-UID forte).
    let summary = broker.read_policy_summary(Some(project_id));
    assert_eq!(summary.isolation_mode, IsolationMode::Standard);
    assert_eq!(summary.confinement_label, "non confiné");
}
