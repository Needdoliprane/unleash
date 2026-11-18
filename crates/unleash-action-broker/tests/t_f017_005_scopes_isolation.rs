// T-F017-005 / AC-F017-005 / REQ-SEC-005
//! Tests de scopes fichiers (symlink, traversal, TOCTOU), isolation stricte fail-closed
//! et application des obligations R6 (`none_accepted`, inconnue, non imposable, contradictoire, perte de capacité).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::symlink;
use std::sync::Arc;

use secrecy::SecretString;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, CallerIdentity, ConfigureGuardCommand, EffectsClass,
    GuardConstraint, GuardMappingProfile, GuardMode, IsolationMode, IsolationRequirements,
    NormalizedResourceRef, PlatformKeyringSecretStore, ScopeKind, compute_effective_authority,
};
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence, IdGeneratorPort, LineageFence,
    ModelPolicy, ProjectId, ReconcileStrategy, Revision, SecretRef, TimestampMs,
};
use unleash_platform::{KeyringPort, LocalSecureKeyring};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, ScriptedGuardOutcome, SecretResolutionSpy, TestIsolationProbe,
    local_test_dir,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(5),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One ordered isolation and obligations acceptance scenario shares independent execution and secret spies"
)]
async fn t_f017_005_scopes_toctou_strict_isolation_and_r6_obligations() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(505));
    let dir = local_test_dir("t005").unwrap();
    let root = dir.path().join("worktree");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"top-secret").unwrap();
    std::fs::write(root.join("legit.txt"), b"ok").unwrap();
    symlink(outside.join("secret.txt"), root.join("escape_link")).unwrap();

    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let keyring = Arc::new(LocalSecureKeyring::new());
    let secret_ref = SecretRef::new("os-secret:t005/token").unwrap();
    keyring
        .store(&secret_ref, SecretString::from("canary-t005".to_owned()))
        .unwrap();
    let secret_spy = Arc::new(SecretResolutionSpy::new(Arc::new(
        PlatformKeyringSecretStore::new(keyring),
    )));

    let project_id = ProjectId::from_u128(50);
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
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::from([secret_ref.as_opaque_str().to_owned()])),
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
        probe.clone(),
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(test_fence()),
    );

    let make_file_intent = |verb: &str, effects: EffectsClass, path: std::path::PathBuf| {
        let payload = serde_json::json!({ "verb": verb, "path": path.display().to_string() });
        ActionIntent {
            action_id: ids.new_action_id(),
            subject: ActionSubject::new_control_plane(
                ActorId::new("uid:501").unwrap(),
                String::from("cmd:t005"),
            )
            .unwrap(),
            project_id: Some(project_id),
            verb: verb.to_owned(),
            resource_refs: vec![NormalizedResourceRef::Path(path)],
            payload_digest: ActionIntent::compute_payload_digest(&payload).unwrap(),
            reconcile_strategy: ReconcileStrategy::LocalCompareAndSet,
            effects_class: effects,
            scope_revision: Revision::new(1),
            policy_revision: Revision::INITIAL,
            lineage_fence: test_fence(),
            action_deadline: TimestampMs::new(1_767_225_660_000),
            isolation_requirements: IsolationRequirements::from_authority(&authority),
            secret_refs: vec![secret_ref.clone()],
            provenance: ActionProvenance::EngineInternal,
        }
    };

    // 1. Symlink sortant refusé.
    let err_symlink = broker
        .authorize_action(
            make_file_intent("fs.read", EffectsClass::LocalRead, root.join("escape_link")),
            &authority,
        )
        .await
        .unwrap_err();
    assert_eq!(err_symlink.to_ipc_error(None).code, ErrorCode::ScopeDenied);

    // 2. Traversal `..` refusé.
    let err_traversal = broker
        .authorize_action(
            make_file_intent(
                "fs.write",
                EffectsClass::LocalWrite,
                root.join("../outside/secret.txt"),
            ),
            &authority,
        )
        .await
        .unwrap_err();
    assert_eq!(
        err_traversal.to_ipc_error(None).code,
        ErrorCode::ScopeDenied
    );

    // 3. TOCTOU : autorisé sur fichier légitime, puis remplacé par symlink sortant avant dispatch -> refusé.
    let toctou_target = root.join("swap.txt");
    std::fs::write(&toctou_target, b"legit").unwrap();
    let toctou_intent =
        make_file_intent("fs.write", EffectsClass::LocalWrite, toctou_target.clone());
    let toctou_id = toctou_intent.action_id;
    let AuthorizationOutcome::Authorized(toctou_dec) = broker
        .authorize_action(toctou_intent, &authority)
        .await
        .unwrap()
    else {
        panic!("expected initial authorization before swap");
    };
    std::fs::remove_file(&toctou_target).unwrap();
    symlink(outside.join("secret.txt"), &toctou_target).unwrap();
    let toctou_err = broker
        .execute_authorized(
            toctou_id,
            &toctou_dec.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(toctou_err.to_ipc_error(None).code, ErrorCode::ScopeDenied);
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // 4. Mode strict sans backend -> IsolationUnavailable/ISOLATION_BACKEND_MISSING (jamais repli standard).
    let strict_auth = compute_effective_authority(
        Some(project_id),
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: String::from("strict_project"),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([String::from("process.spawn")])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![root.clone()]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::from([secret_ref.as_opaque_str().to_owned()])),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Strict,
            require_action_isolation: true,
            max_scope_kind: ScopeKind::Project,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );

    probe.set_missing_backend("macos-seatbelt", "installer sandbox-exec");
    let mut strict_intent = make_file_intent(
        "process.spawn",
        EffectsClass::ProcessExec,
        root.join("legit.txt"),
    );
    strict_intent.isolation_requirements = IsolationRequirements::from_authority(&strict_auth);
    let missing_iso_err = broker
        .authorize_action(strict_intent.clone(), &strict_auth)
        .await
        .unwrap_err();
    let missing_iso_wire = missing_iso_err.to_ipc_error(None);
    assert_eq!(missing_iso_wire.code, ErrorCode::IsolationUnavailable);
    assert_eq!(
        missing_iso_wire.reason_code,
        Some(ReasonCode::IsolationBackendMissing)
    );

    // Backend incomplet -> IsolationUnavailable/ISOLATION_OBLIGATION_UNENFORCEABLE.
    probe.set_unenforceable("linux-bubblewrap", "seccomp absent");
    strict_intent.action_id = ids.new_action_id();
    let unenf_iso_err = broker
        .authorize_action(strict_intent, &strict_auth)
        .await
        .unwrap_err();
    let unenf_iso_wire = unenf_iso_err.to_ipc_error(None);
    assert_eq!(unenf_iso_wire.code, ErrorCode::IsolationUnavailable);
    assert_eq!(
        unenf_iso_wire.reason_code,
        Some(ReasonCode::IsolationObligationUnenforceable)
    );

    // 5. R6 — Obligations Allow (unknown, unenforceable, contradictory, applicable sous none_accepted vs qualifié, perte de capacité).
    probe.set_available("macos-seatbelt");
    let admin = CallerIdentity::admin(ActorId::new("uid:501").unwrap());
    let double = Arc::new(DeterministicGuardDouble::with_profile(
        clock.clone(),
        GuardMappingProfile::NoneAccepted,
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

    // Obligation applicable sous profil initial `none_accepted` -> refusé GUARD_OBLIGATION_UNSATISFIED, 0 dispatch, 0 secret.
    let applicable_constraint = GuardConstraint {
        constraint_id: String::from("c-net-ro"),
        kind: String::from("egress_loopback_only"),
        parameters: BTreeMap::from([(String::from("proxy"), String::from("127.0.0.1:8080"))]),
        contradiction_group: Some(String::from("network_mode")),
        continuous: true,
    };
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![applicable_constraint.clone()],
    });
    let mut r6_intent = make_file_intent(
        "process.spawn",
        EffectsClass::ProcessExec,
        root.join("legit.txt"),
    );
    r6_intent.isolation_requirements = IsolationRequirements::from_authority(&strict_auth);
    let err_none_accepted = broker
        .authorize_action(r6_intent.clone(), &strict_auth)
        .await
        .unwrap_err();
    assert_eq!(
        err_none_accepted.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // Passer à un profil qualifié supportant `egress_loopback_only` et `strict_fs`.
    let qualified_profile = GuardMappingProfile::Qualified {
        profile_id: String::from("unleash.guard_mapping.qualified_test"),
        version: 2,
        supported_obligations: BTreeSet::from([
            String::from("egress_loopback_only"),
            String::from("strict_fs"),
            String::from("known_unimplemented"),
        ]),
        supports_remote_approval: true,
    };
    double.set_profile(qualified_profile.clone());
    broker
        .configure_guard(
            &admin,
            ConfigureGuardCommand {
                endpoint: Some(String::from("unix:///tmp/guard.sock")),
                guard_mode: Some(GuardMode::GuardRequired),
                mapping_profile: Some(qualified_profile),
                ..ConfigureGuardCommand::default()
            },
            Some(double.clone()),
        )
        .await
        .unwrap();

    spy.set_obligation_supported("egress_loopback_only", true);
    spy.set_obligation_supported("strict_fs", true);
    spy.set_obligation_supported("known_unimplemented", false);

    // 5a. Obligation inconnue -> GUARD_OBLIGATION_UNSATISFIED.
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![GuardConstraint {
            constraint_id: String::from("c-unk"),
            kind: String::from("unknown_magic_obligation"),
            parameters: BTreeMap::new(),
            contradiction_group: None,
            continuous: false,
        }],
    });
    r6_intent.action_id = ids.new_action_id();
    assert_eq!(
        broker
            .authorize_action(r6_intent.clone(), &strict_auth)
            .await
            .unwrap_err()
            .to_ipc_error(None)
            .reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // 5b. Obligation connue mais non imposable -> GUARD_OBLIGATION_UNSATISFIED.
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![GuardConstraint {
            constraint_id: String::from("c-unenforceable"),
            kind: String::from("known_unimplemented"),
            parameters: BTreeMap::new(),
            contradiction_group: None,
            continuous: false,
        }],
    });
    r6_intent.action_id = ids.new_action_id();
    assert_eq!(
        broker
            .authorize_action(r6_intent.clone(), &strict_auth)
            .await
            .unwrap_err()
            .to_ipc_error(None)
            .reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // 5c. Obligations contradictoires -> GUARD_OBLIGATION_UNSATISFIED.
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![
            GuardConstraint {
                constraint_id: String::from("c-1"),
                kind: String::from("egress_loopback_only"),
                parameters: BTreeMap::from([(String::from("mode"), String::from("deny_all"))]),
                contradiction_group: Some(String::from("network_mode")),
                continuous: true,
            },
            GuardConstraint {
                constraint_id: String::from("c-2"),
                kind: String::from("strict_fs"),
                parameters: BTreeMap::from([(String::from("mode"), String::from("allow_all"))]),
                contradiction_group: Some(String::from("network_mode")),
                continuous: true,
            },
        ],
    });
    r6_intent.action_id = ids.new_action_id();
    assert_eq!(
        broker
            .authorize_action(r6_intent.clone(), &strict_auth)
            .await
            .unwrap_err()
            .to_ipc_error(None)
            .reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // 5d. Perte de capacité entre évaluation et engagement -> GUARD_OBLIGATION_UNSATISFIED, 0 dispatch, 0 secret.
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![applicable_constraint.clone()],
    });
    r6_intent.action_id = ids.new_action_id();
    let aid_lost = r6_intent.action_id;
    let AuthorizationOutcome::Authorized(dec_lost) = broker
        .authorize_action(r6_intent.clone(), &strict_auth)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    spy.set_obligation_supported("egress_loopback_only", false);
    let lost_err = broker
        .execute_authorized(
            aid_lost,
            &dec_lost.decision_ref,
            test_fence(),
            &strict_auth,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        lost_err.to_ipc_error(None).reason_code,
        Some(ReasonCode::GuardObligationUnsatisfied)
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();

    // 5e. Cas entièrement applicable avec capacité maintenue -> exécuté avec preuve corrélée à la décision.
    spy.set_obligation_supported("egress_loopback_only", true);
    double.push_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![applicable_constraint],
    });
    r6_intent.action_id = ids.new_action_id();
    let aid_ok = r6_intent.action_id;
    let AuthorizationOutcome::Authorized(dec_ok) = broker
        .authorize_action(r6_intent, &strict_auth)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    assert_eq!(dec_ok.applied_obligations.len(), 1);
    let receipt = broker
        .execute_authorized(
            aid_ok,
            &dec_ok.decision_ref,
            test_fence(),
            &strict_auth,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.decision_ref, dec_ok.decision_ref);
    assert_eq!(receipt.enforcement_refs.len(), 1);
    assert_eq!(spy.dispatch_count(), 1);
    assert_eq!(secret_spy.successful_resolutions(), 1);
}
