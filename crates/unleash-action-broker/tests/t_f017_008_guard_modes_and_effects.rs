// T-F017-008 / AC-F017-008 / REQ-SEC-008
//! Tests complets des modes `GuardMode` (`BrokerOnly`, `GuardOptional`, `GuardRequired`), timeouts,
//! réponses malformées/incompatibles, `RequireApproval` (`ResolveActionApproval`), `LocalRead`,
//! `EffectsClass` inconnu et sujets `Replay`/`ControlPlane`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionState, ActionSubject, ApprovalDecision,
    AuthorityLayer, AuthorizationOutcome, BrokerConfig, CallerIdentity, ConfigureGuardCommand,
    EffectsClass, GuardCapabilities, GuardConstraint, GuardMappingProfile, GuardMode,
    IsolationMode, IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef,
    ResolveActionApprovalCommand, ScopeKind, UnvalidatedActionIntent, compute_effective_authority,
};
use unleash_domain::{
    ActorId, AgentId, ApprovalScopeId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, LineageFence, ModelPolicy, ProjectId, ReconcileStrategy, ReplayWorkerId,
    Revision, TimestampMs,
};
use unleash_protocol::{ErrorCode, ReasonCode, RetryClass};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, ScriptedGuardOutcome, SecretResolutionSpy, TestIsolationProbe,
    local_test_dir,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(8),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_008_guard_modes_fallbacks_approvals_and_closed_effects_class() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(808));
    let dir = local_test_dir("t008").unwrap();
    let root = dir.path().to_path_buf();
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let secret_spy = Arc::new(SecretResolutionSpy::in_memory());
    let fallback = Arc::new(LocalAuthorityFallbackPolicy::with_authorized_refs([
        "policy.local.fallback.v1",
    ]));

    let project_id = ProjectId::from_u128(80);
    let authority = compute_effective_authority(
        Some(project_id),
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: String::from("project"),
            allowed_effects: Some(BTreeSet::from([
                EffectsClass::LocalRead,
                EffectsClass::LocalWrite,
                EffectsClass::ProcessExec,
            ])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([
                String::from("fs.read"),
                String::from("fs.write"),
                String::from("process.spawn"),
            ])),
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

    let broker = Arc::new(ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        fallback,
        BrokerConfig::for_test(test_fence()),
    ));

    // 1. Défaut sans endpoint = BrokerOnly (visible, jamais une fausse connexion saine).
    let initial_health = broker.read_guard_health().await.unwrap();
    assert_eq!(initial_health.mode, GuardMode::BrokerOnly);
    assert!(!initial_health.connected);

    // 2. Classe d'effet inconnue refusée AVANT GuardPort (InvalidArgument/EFFECTS_CLASS_UNKNOWN).
    let admin = CallerIdentity::admin(ActorId::new("uid:501").unwrap());
    let double = Arc::new(DeterministicGuardDouble::with_capabilities(
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
    // Configurer un endpoint sans guard_mode explicite bascule par défaut vers GuardRequired (Q-04).
    let configured_health = broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/ai-guard.sock")),
                guard_mode: None,
                ..ConfigureGuardCommand::default()
            },
            Some(double.clone()),
        )
        .await
        .unwrap();
    assert_eq!(configured_health.mode, GuardMode::GuardRequired);

    let raw_unknown = UnvalidatedActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_replay(
            ActorId::new("replay-engine").unwrap(),
            String::from("run:808"),
            ReplayWorkerId::from_u128(81),
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.write"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("x.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"k":1})).unwrap(),
        reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
        effects_class: String::from("quantum_teleport"),
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: test_fence(),
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    let unk_err = broker
        .authorize_raw_intent(raw_unknown, &authority)
        .await
        .unwrap_err();
    let unk_wire = unk_err.to_ipc_error(None);
    assert_eq!(unk_wire.code, ErrorCode::InvalidArgument);
    assert_eq!(unk_wire.reason_code, Some(ReasonCode::EffectsClassUnknown));
    assert_eq!(double.evaluate_call_count(), 0);

    // Un sujet Replay ne peut jamais fabriquer un agent_id factice.
    let fake_agent_err = ActionSubject::from_parts(
        unleash_action_broker::SubjectKind::Replay,
        ActorId::new("replay").unwrap(),
        String::from("run:1"),
        None,
        None,
        Some(ReplayWorkerId::from_u128(1)),
        Some(AgentId::from_u128(999)),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(
        fake_agent_err.to_ipc_error(None).code,
        ErrorCode::InvalidArgument
    );

    // 3. `local_read` n'appelle pas GuardPort mais reste autorisé sous autorité locale (même avec sujet Replay).
    let replay_read = ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_replay(
            ActorId::new("replay-engine").unwrap(),
            String::from("run:808"),
            ReplayWorkerId::from_u128(81),
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.read"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("r.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"read": true}))
            .unwrap(),
        reconcile_strategy: ReconcileStrategy::ReadOnlyRetryable,
        effects_class: EffectsClass::LocalRead,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: test_fence(),
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    assert!(matches!(
        broker
            .authorize_action(replay_read, &authority)
            .await
            .unwrap(),
        AuthorizationOutcome::Authorized(_)
    ));
    assert_eq!(double.evaluate_call_count(), 0);

    let make_write_intent = || ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_control_plane(
            ActorId::new("uid:501").unwrap(),
            String::from("cmd:808"),
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: String::from("fs.write"),
        resource_refs: vec![NormalizedResourceRef::Path(root.join("w.txt"))],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"write": 1}))
            .unwrap(),
        reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
        effects_class: EffectsClass::LocalWrite,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: test_fence(),
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };

    // 4. Sous GuardRequired : timeout (2000ms sans sleep via ManualClockTimer) -> action reste Authorizing,
    //    GUARD_UNAVAILABLE / retry_class=after_backoff, aucune relance immédiate de l'appel expiré.
    double.push_outcome(ScriptedGuardOutcome::HangUntilCancelled);
    let timeout_intent = make_write_intent();
    let timeout_id = timeout_intent.action_id;
    let b_clone = broker.clone();
    let auth_clone = authority.clone();
    let auth_task =
        tokio::spawn(async move { b_clone.authorize_action(timeout_intent, &auth_clone).await });
    // Attendre que l'horloge enregistre le réveil de 2000ms, puis avancer exactement de 2000ms.
    for _ in 0..100 {
        if clock.pending_wakeup_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    clock.advance(Duration::from_millis(2_000));
    let timeout_err = auth_task.await.unwrap().unwrap_err();
    let timeout_wire = timeout_err.to_ipc_error(None);
    assert_eq!(timeout_wire.code, ErrorCode::CapabilityDenied);
    assert_eq!(timeout_wire.reason_code, Some(ReasonCode::GuardUnavailable));
    assert_eq!(timeout_wire.retry_class, RetryClass::AfterBackoff);
    assert_eq!(
        broker.action_state(timeout_id),
        Some(ActionState::Authorizing)
    );
    assert_eq!(double.evaluate_call_count(), 1); // pas de retry immédiat !

    // Réponse malformée sous GuardRequired -> GUARD_UNAVAILABLE, reste Authorizing.
    double.push_outcome(ScriptedGuardOutcome::Malformed {
        detail: String::from("invalid json"),
    });
    let mal_intent = make_write_intent();
    let mal_id = mal_intent.action_id;
    let mal_err = broker
        .authorize_action(mal_intent, &authority)
        .await
        .unwrap_err();
    assert_eq!(
        mal_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardUnavailable)
    );
    assert_eq!(broker.action_state(mal_id), Some(ActionState::Authorizing));

    // Version/capacité incompatible sous GuardRequired -> Denied (GUARD_PROTOCOL_UNSUPPORTED).
    double.push_outcome(ScriptedGuardOutcome::IncompatibleProtocol {
        expected: String::from("unleash.guard_port.v1"),
        observed: String::from("legacy.v0"),
    });
    let proto_intent = make_write_intent();
    let proto_id = proto_intent.action_id;
    let proto_err = broker
        .authorize_action(proto_intent, &authority)
        .await
        .unwrap_err();
    let proto_wire = proto_err.to_ipc_error(None);
    assert_eq!(proto_wire.code, ErrorCode::ProtocolIncompatible);
    assert_eq!(
        proto_wire.reason_code,
        Some(ReasonCode::GuardProtocolUnsupported)
    );
    assert_eq!(broker.action_state(proto_id), Some(ActionState::Denied));

    // RequireApproval sous profil non qualifié -> GUARD_APPROVAL_UNSUPPORTED -> Denied.
    double.push_outcome(ScriptedGuardOutcome::RequireApproval {
        approval_scope: ApprovalScopeId::from_u128(10),
        reason: String::from("human confirm"),
    });
    let app_unsup_intent = make_write_intent();
    let app_unsup_id = app_unsup_intent.action_id;
    let app_unsup_err = broker
        .authorize_action(app_unsup_intent, &authority)
        .await
        .unwrap_err();
    assert_eq!(
        app_unsup_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardApprovalUnsupported)
    );
    assert_eq!(broker.action_state(app_unsup_id), Some(ActionState::Denied));

    // 5. Sous GuardOptional : repli uniquement sur Unavailable/Timeout/Malformed/UnsupportedCapability,
    //    JAMAIS sur Deny, RequireApproval ou GUARD_OBLIGATION_UNSATISFIED.
    let qualified_profile = GuardMappingProfile::Qualified {
        profile_id: String::from("unleash.guard_mapping.v2"),
        version: 2,
        supported_obligations: BTreeSet::from([String::from("strict_fs")]),
        supports_remote_approval: true,
    };
    double.set_profile(qualified_profile.clone());
    double.set_supports_remote_approval(true);
    broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/ai-guard.sock")),
                guard_mode: Some(GuardMode::GuardOptional {
                    fallback_policy_ref: String::from("policy.local.fallback.v1"),
                }),
                mapping_profile: Some(qualified_profile),
                ..ConfigureGuardCommand::default()
            },
            Some(double.clone()),
        )
        .await
        .unwrap();

    // Unavailable -> repli autorisé vers politique locale préautorisée.
    double.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: String::from("guard down"),
    });
    let opt_unavail = make_write_intent();
    let AuthorizationOutcome::Authorized(fb_dec) = broker
        .authorize_action(opt_unavail, &authority)
        .await
        .unwrap()
    else {
        panic!("expected fallback authorization under GuardOptional");
    };
    assert_eq!(
        fb_dec.fallback_policy_ref.as_deref(),
        Some("policy.local.fallback.v1")
    );

    // Deny sous GuardOptional -> REFUS (jamais de repli).
    double.push_outcome(ScriptedGuardOutcome::Deny {
        reason_code: String::from("RISK_TOO_HIGH"),
        detail: String::from("blocked"),
    });
    let opt_deny = make_write_intent();
    let deny_err = broker
        .authorize_action(opt_deny, &authority)
        .await
        .unwrap_err();
    assert_eq!(
        deny_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardDenied)
    );

    // Obligation insatisfaite sous GuardOptional -> REFUS (jamais de repli).
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![GuardConstraint {
            constraint_id: String::from("c-unk"),
            kind: String::from("unknown_constraint"),
            parameters: BTreeMap::new(),
            contradiction_group: None,
            continuous: false,
        }],
    });
    let opt_obl = make_write_intent();
    let obl_err = broker
        .authorize_action(opt_obl, &authority)
        .await
        .unwrap_err();
    assert_eq!(
        obl_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );

    // RequireApproval sous profil qualifié -> ApprovalRequired, puis ResolveActionApproval
    // vérifie approval_scope + payload_digest exacts (approuvé vs refusé).
    let scope_id = ApprovalScopeId::from_u128(55);
    double.push_outcome(ScriptedGuardOutcome::RequireApproval {
        approval_scope: scope_id,
        reason: String::from("validation humaine"),
    });
    let app_intent = make_write_intent();
    let app_id = app_intent.action_id;
    let app_digest = app_intent.payload_digest;
    let AuthorizationOutcome::ApprovalRequired {
        approval_scope,
        revision,
        ..
    } = broker
        .authorize_action(app_intent, &authority)
        .await
        .unwrap()
    else {
        panic!("expected ApprovalRequired");
    };
    assert_eq!(approval_scope, scope_id);
    assert_eq!(
        broker.action_state(app_id),
        Some(ActionState::ApprovalRequired)
    );

    // Mauvais digest rejeté.
    let bad_digest_err = broker
        .resolve_action_approval(
            &admin,
            ResolveActionApprovalCommand {
                decision_receipt_id: unleash_domain::ReceiptId::from_u128(8001),
                decision_command_digest: unleash_domain::Sha256Digest::from_bytes(
                    b"synthetic-human-decision",
                ),
                action_id: app_id,
                approval_scope,
                payload_digest: ActionIntent::compute_payload_digest(
                    &serde_json::json!({"other": 2}),
                )
                .unwrap(),
                decision: ApprovalDecision::Approve,
                expected_revision: revision,
                reason: String::from("mismatch"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        bad_digest_err.to_ipc_error(None).code,
        ErrorCode::ContractViolation
    );

    // Bon digest + bon scope -> Authorized puis exécutable.
    let approved_dec = broker
        .resolve_action_approval(
            &admin,
            ResolveActionApprovalCommand {
                decision_receipt_id: unleash_domain::ReceiptId::from_u128(8001),
                decision_command_digest: unleash_domain::Sha256Digest::from_bytes(
                    b"synthetic-human-decision",
                ),
                action_id: app_id,
                approval_scope,
                payload_digest: app_digest,
                decision: ApprovalDecision::Approve,
                expected_revision: revision,
                reason: String::from("ok humain"),
            },
        )
        .await
        .unwrap();
    assert_eq!(broker.action_state(app_id), Some(ActionState::Authorized));
    let receipt = broker
        .execute_authorized(
            app_id,
            &approved_dec.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.action_id, app_id);

    // Cas ResolveActionApproval -> Deny mène à Denied terminal sans dispatch.
    double.push_outcome(ScriptedGuardOutcome::RequireApproval {
        approval_scope: ApprovalScopeId::from_u128(56),
        reason: String::from("confirm 2"),
    });
    let app_deny_intent = make_write_intent();
    let app_deny_id = app_deny_intent.action_id;
    let app_deny_digest = app_deny_intent.payload_digest;
    let AuthorizationOutcome::ApprovalRequired {
        approval_scope: scope2,
        revision: rev2,
        ..
    } = broker
        .authorize_action(app_deny_intent, &authority)
        .await
        .unwrap()
    else {
        panic!("expected ApprovalRequired");
    };
    broker
        .resolve_action_approval(
            &admin,
            ResolveActionApprovalCommand {
                decision_receipt_id: unleash_domain::ReceiptId::from_u128(8001),
                decision_command_digest: unleash_domain::Sha256Digest::from_bytes(
                    b"synthetic-human-decision",
                ),
                action_id: app_deny_id,
                approval_scope: scope2,
                payload_digest: app_deny_digest,
                decision: ApprovalDecision::Deny,
                expected_revision: rev2,
                reason: String::from("humain refuse"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(broker.action_state(app_deny_id), Some(ActionState::Denied));
}
