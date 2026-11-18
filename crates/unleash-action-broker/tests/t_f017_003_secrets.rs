// T-F017-003 / AC-F017-003 / REQ-SEC-003
//! Tests du port `SecretStore`, zéro fuite de canari dans Debug/logs/erreurs/payloads et rejets verrouillé/indisponible/refusé/absent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use secrecy::SecretString;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, EffectsClass, IsolationMode, IsolationRequirements,
    NormalizedResourceRef, PlatformKeyringSecretStore, ScopeKind, SecretStore,
    compute_effective_authority,
};
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence, IdGeneratorPort, LineageFence,
    ModelPolicy, ProjectId, ReconcileStrategy, Revision, SecretRef, TimestampMs,
};
use unleash_platform::{KeyringPort, LocalSecureKeyring, SecretStatus};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe,
};

const CANARY_TOKEN: &str = "sk-unleash-canary-9f8e7d6c5b4a3210";

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(3),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_003_secret_canary_never_leaks_and_vault_errors_map_strictly() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(303));
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());

    let keyring = Arc::new(LocalSecureKeyring::new());
    let secret_ref = SecretRef::new("os-secret:ovpt-unleash/openai/canary").unwrap();
    keyring
        .store(&secret_ref, SecretString::from(CANARY_TOKEN.to_owned()))
        .unwrap();

    // Debug/Display de SecretRef et LocalSecureKeyring ne doivent jamais contenir le canari.
    assert!(!format!("{secret_ref:?}").contains(CANARY_TOKEN));
    assert!(!format!("{secret_ref}").contains(CANARY_TOKEN));
    assert!(!format!("{keyring:?}").contains(CANARY_TOKEN));

    let secret_spy = Arc::new(SecretResolutionSpy::new(Arc::new(
        PlatformKeyringSecretStore::new(keyring.clone()),
    )));
    assert_eq!(
        secret_spy.status(&secret_ref).unwrap(),
        SecretStatus::Present
    );

    let project_id = ProjectId::from_u128(30);
    let layer = AuthorityLayer {
        layer_name: String::from("project"),
        allowed_effects: Some(BTreeSet::from([EffectsClass::ModelDispatch])),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from([String::from("model.dispatch")])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: None,
        allowed_network_hosts: Some(BTreeSet::from([String::from("api.openai.com")])),
        allowed_secret_refs: Some(BTreeSet::from([secret_ref.as_opaque_str().to_owned()])),
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

    let mut cfg = BrokerConfig::for_test(test_fence());
    cfg.canaries_to_redact = vec![SecretString::from(CANARY_TOKEN.to_owned())];
    let broker = ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        cfg,
    );

    let make_intent = |sref: SecretRef| {
        let payload = serde_json::json!({ "provider": "openai", "model": "gpt-6-luna" });
        ActionIntent {
            action_id: ids.new_action_id(),
            subject: ActionSubject::new_control_plane(
                ActorId::new("uid:501").unwrap(),
                String::from("probe:1"),
            )
            .unwrap(),
            project_id: Some(project_id),
            verb: String::from("model.dispatch"),
            resource_refs: vec![NormalizedResourceRef::ModelTarget {
                provider: String::from("openai"),
                model_id: String::from("gpt-6-luna"),
                locality: DataLocality::ApprovedRemote,
            }],
            payload_digest: ActionIntent::compute_payload_digest(&payload).unwrap(),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::ModelDispatch,
            scope_revision: Revision::new(1),
            policy_revision: Revision::INITIAL,
            lineage_fence: test_fence(),
            action_deadline: TimestampMs::new(1_767_225_660_000),
            isolation_requirements: IsolationRequirements::from_authority(&authority),
            secret_refs: vec![sref],
            provenance: ActionProvenance::EngineInternal,
        }
    };

    // 1. Coffre verrouillé -> SECRET_STORE_LOCKED, 0 dispatch.
    keyring.lock();
    let intent_locked = make_intent(secret_ref.clone());
    let aid_locked = intent_locked.action_id;
    let AuthorizationOutcome::Authorized(dec_locked) = broker
        .authorize_action(intent_locked, &authority)
        .await
        .unwrap()
    else {
        panic!("expected authorized before secret resolution");
    };
    let locked_err = broker
        .execute_authorized(
            aid_locked,
            &dec_locked.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    let locked_wire = locked_err.to_ipc_error(None);
    assert_eq!(locked_wire.code, ErrorCode::CapabilityDenied);
    assert_eq!(locked_wire.reason_code, Some(ReasonCode::SecretStoreLocked));
    assert!(!format!("{locked_err:?}").contains(CANARY_TOKEN));
    spy.assert_zero_dispatches();

    // 2. Coffre indisponible -> SECRET_STORE_UNAVAILABLE.
    keyring.unlock();
    keyring.set_unavailable(true);
    let intent_unavail = make_intent(secret_ref.clone());
    let aid_unavail = intent_unavail.action_id;
    let AuthorizationOutcome::Authorized(dec_unavail) = broker
        .authorize_action(intent_unavail, &authority)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    let unavail_err = broker
        .execute_authorized(
            aid_unavail,
            &dec_unavail.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    let unavail_wire = unavail_err.to_ipc_error(None);
    assert_eq!(unavail_wire.code, ErrorCode::InternalFailure);
    assert_eq!(
        unavail_wire.reason_code,
        Some(ReasonCode::SecretStoreUnavailable)
    );
    spy.assert_zero_dispatches();

    // 3. Accès refusé par ACL coffre -> SECRET_ACCESS_DENIED.
    keyring.set_unavailable(false);
    keyring.deny_ref(&secret_ref);
    let intent_denied = make_intent(secret_ref.clone());
    let aid_denied = intent_denied.action_id;
    let AuthorizationOutcome::Authorized(dec_denied) = broker
        .authorize_action(intent_denied, &authority)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    let denied_err = broker
        .execute_authorized(
            aid_denied,
            &dec_denied.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    let denied_wire = denied_err.to_ipc_error(None);
    assert_eq!(denied_wire.code, ErrorCode::CapabilityDenied);
    assert_eq!(
        denied_wire.reason_code,
        Some(ReasonCode::SecretAccessDenied)
    );
    spy.assert_zero_dispatches();

    // 4. Référence inexistante (dans scope autorisé mais absente du coffre) -> SECRET_REF_NOT_FOUND.
    keyring.allow_ref(&secret_ref);
    let missing_ref = SecretRef::new("os-secret:ovpt-unleash/openai/missing").unwrap();
    let layer_with_missing = AuthorityLayer {
        layer_name: String::from("project"),
        allowed_effects: Some(BTreeSet::from([EffectsClass::ModelDispatch])),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from([String::from("model.dispatch")])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: None,
        allowed_network_hosts: Some(BTreeSet::from([String::from("api.openai.com")])),
        allowed_secret_refs: Some(BTreeSet::from([
            secret_ref.as_opaque_str().to_owned(),
            missing_ref.as_opaque_str().to_owned(),
        ])),
        model_policy: ModelPolicy::AllowedBounded,
        data_locality: DataLocality::ApprovedRemote,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: ScopeKind::Project,
        allow_delegation: false,
        allow_cross_repository: false,
    };
    let auth_missing = compute_effective_authority(
        Some(project_id),
        Revision::INITIAL,
        Revision::new(1),
        &[layer_with_missing],
    );
    let intent_missing = make_intent(missing_ref);
    let aid_missing = intent_missing.action_id;
    let AuthorizationOutcome::Authorized(dec_missing) = broker
        .authorize_action(intent_missing, &auth_missing)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    let missing_err = broker
        .execute_authorized(
            aid_missing,
            &dec_missing.decision_ref,
            test_fence(),
            &auth_missing,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap_err();
    let missing_wire = missing_err.to_ipc_error(None);
    assert_eq!(missing_wire.code, ErrorCode::NotFoundOrHidden);
    assert_eq!(
        missing_wire.reason_code,
        Some(ReasonCode::SecretRefNotFound)
    );
    spy.assert_zero_dispatches();

    // 5. Chemin nominal autorisé : résolution uniquement au transport, zéro canari dans events/audit/dispatch.
    let intent_ok = make_intent(secret_ref);
    let aid_ok = intent_ok.action_id;
    let AuthorizationOutcome::Authorized(dec_ok) = broker
        .authorize_action(intent_ok, &authority)
        .await
        .unwrap()
    else {
        panic!("expected authorized");
    };
    let receipt = broker
        .execute_authorized(
            aid_ok,
            &dec_ok.decision_ref,
            test_fence(),
            &authority,
            spy.as_ref(),
            secret_spy.as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.action_id, aid_ok);
    assert_eq!(secret_spy.successful_resolutions(), 1);
    spy.assert_no_canary_in_dispatches(CANARY_TOKEN);
    let events_json = serde_json::to_string(&broker.audit_events()).unwrap();
    assert!(!events_json.contains(CANARY_TOKEN));
}
