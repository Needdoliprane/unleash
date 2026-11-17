//! Tests d'intégration de `unleash-browser` contre un serveur `WebDriver` W3C fidèle
//! en Rust et un site marchand synthétique local (zéro réseau externe).

#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use unleash_browser::{
    AssertionVerdict, BrowserActionIntent, BrowserActionKind, BrowserEngineKind, BrowserError,
    BrowserPort, BusinessAssertion, CheckpointInput, CloseReason, ConfinementProof,
    EffectJournalPort, EgressPolicy, EgressProxy, ExecutionMode, ExpectedTargetIdentity,
    FantocciniBrowserAdapter, InMemoryEffectJournal, IsolationLevel, LocalInventorySearchPaths,
    LocatorStrategy, ModelPolicyEnforcer, ObserveScope, OpenContextRequest, PinnedBrowserRuntime,
    RawLocatorRequest, ScriptOrigin, TargetSpec, inventory_local_runtimes,
};
use unleash_domain::{
    ActionId, AuthorityGeneration, DaemonEpoch, Fence, LeaseId, LineageFence, ModelPolicy, RunId,
    SessionId,
};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{DeterministicIdGenerator, ManualClock};
use url::Url;

use support::{
    FaithfulW3cWebDriverServer, HitCounterServer, MerchantMode, SyntheticMerchantServer,
    create_pinned_dummy_runtime,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(100),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

fn new_adapter() -> (
    FantocciniBrowserAdapter,
    Arc<InMemoryEffectJournal>,
    Arc<ModelPolicyEnforcer>,
) {
    let clock = Arc::new(ManualClock::new(1_763_300_000_000));
    let ids = Arc::new(DeterministicIdGenerator::new(0x1234_5678));
    let journal = Arc::new(InMemoryEffectJournal::new());
    let guard = Arc::new(ModelPolicyEnforcer::new());
    let adapter = FantocciniBrowserAdapter::new(
        clock,
        ids,
        Arc::clone(&journal) as Arc<dyn EffectJournalPort>,
        Arc::clone(&guard),
    );
    (adapter, journal, guard)
}

async fn open_standard_context(
    adapter: &FantocciniBrowserAdapter,
    wd: &FaithfulW3cWebDriverServer,
    proxy: &EgressProxy,
    merchant: &SyntheticMerchantServer,
    runtime: PinnedBrowserRuntime,
    profile_dir: &std::path::Path,
) -> unleash_browser::BrowserContextHandle {
    let policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    adapter
        .open_context(OpenContextRequest {
            run_id: RunId::from_u128(1),
            session_id: SessionId::from_u128(10),
            lease_id: LeaseId::from_u128(20),
            fence: test_fence(),
            webdriver_endpoint: wd.endpoint_url(),
            runtime,
            execution_mode: ExecutionMode::Mediated,
            model_policy: ModelPolicy::Forbidden,
            isolation_level: IsolationLevel::Standard,
            confinement_proof: ConfinementProof::Unconfined,
            egress_policy: policy,
            proxy_url: Some(Url::parse(&proxy.proxy_url_string()).unwrap()),
            user_data_dir: profile_dir.to_path_buf(),
            headless: true,
            script_timeout: Duration::from_secs(5),
            page_load_timeout: Duration::from_secs(5),
        })
        .await
        .unwrap()
}

// T-F019-009 / AC-F019-009 / REQ-AUTO-009
#[tokio::test]
async fn t_f019_009_cardinality_ambiguous_missing_drift_and_reordered_identity() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let merchant = SyntheticMerchantServer::start().await;
    let wd = FaithfulW3cWebDriverServer::start().await;
    let policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    let proxy = EgressProxy::start(
        RunId::from_u128(1),
        LeaseId::from_u128(20),
        ModelPolicy::Forbidden,
        policy,
    )
    .await
    .unwrap();
    let (adapter, _, _) = new_adapter();

    let handle = open_standard_context(
        &adapter,
        &wd,
        &proxy,
        &merchant,
        runtime,
        &dir.path().join("profile-t009"),
    )
    .await;

    let nav = BrowserActionIntent {
        action_id: ActionId::from_u128(1),
        logical_effect_key: "nav.catalog.1".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };

    // 1. Cible ambiguë (2 boutons identiques) -> bloquée avant tout clic
    merchant.set_mode(MerchantMode::AmbiguousDom);
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs_ambig = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let target_role = TargetSpec {
        strategy: LocatorStrategy::RoleName {
            role: "button".into(),
            name: "Add Apple".into(),
            exact: true,
        },
        frame_scope: None,
        expected_identity: Some(ExpectedTargetIdentity {
            expected_sku: Some("SYNTHETIC-APPLE-1KG".into()),
            ..Default::default()
        }),
    };
    let err_ambig = adapter
        .resolve_target(&handle, &target_role, &obs_ambig)
        .await
        .unwrap_err();
    assert!(matches!(
        err_ambig,
        BrowserError::AmbiguousTarget {
            candidate_count: 2,
            ..
        }
    ));
    assert_eq!(merchant.mutation_count(), 0);

    // 2. Cible absente -> TargetMissing
    let missing_spec = TargetSpec {
        strategy: LocatorStrategy::TestId {
            attribute: None,
            value: "does-not-exist".into(),
        },
        frame_scope: None,
        expected_identity: None,
    };
    let err_missing = adapter
        .resolve_target(&handle, &missing_spec, &obs_ambig)
        .await
        .unwrap_err();
    assert!(matches!(err_missing, BrowserError::TargetMissing { .. }));

    // 3. Cible unique au CSS/label inchangé mais mauvais SKU -> TargetDrift avant clic
    merchant.set_mode(MerchantMode::DriftedSkuDom);
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs_drift = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let err_drift = adapter
        .resolve_target(&handle, &target_role, &obs_drift)
        .await
        .unwrap_err();
    assert!(matches!(err_drift, BrowserError::TargetDrift { .. }));
    assert_eq!(merchant.mutation_count(), 0);

    // 4. DOM réordonné mais identité unique conforme -> résolution RoleName + Label et clic réussi
    merchant.set_mode(MerchantMode::ReorderedValidDom);
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs_valid = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let resolved = adapter
        .resolve_target(&handle, &target_role, &obs_valid)
        .await
        .unwrap();
    assert_eq!(resolved.candidate_count, 1);
    assert_eq!(resolved.computed_role.as_deref(), Some("button"));
    assert_eq!(resolved.computed_label.as_deref(), Some("Add Apple"));

    let click = BrowserActionIntent {
        action_id: ActionId::from_u128(2),
        logical_effect_key: "cart.add.apple.reordered".into(),
        kind: BrowserActionKind::Click,
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let outcome = adapter
        .perform_action(&handle, &click, Some(&resolved))
        .await
        .unwrap();
    assert!(outcome.dispatch_confirmed);
    assert_eq!(merchant.mutation_count(), 1);

    // Une cible résolue sur une génération de page antérieure ne peut plus être réutilisée
    let stale_reuse = adapter
        .perform_action(&handle, &click, Some(&resolved))
        .await
        .unwrap_err();
    assert!(matches!(
        stale_reuse,
        BrowserError::StaleFenceOrContext { .. }
    ));

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    merchant.shutdown().await;
}

// T-F019-010 / AC-F019-010 / REQ-AUTO-010
#[tokio::test]
async fn t_f019_010_valid_click_wrong_business_outcome_blocks_without_retry() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let merchant = SyntheticMerchantServer::start().await;
    let wd = FaithfulW3cWebDriverServer::start().await;
    let policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    let proxy = EgressProxy::start(
        RunId::from_u128(2),
        LeaseId::from_u128(21),
        ModelPolicy::Forbidden,
        policy,
    )
    .await
    .unwrap();
    let (adapter, _, _) = new_adapter();

    let handle = open_standard_context(
        &adapter,
        &wd,
        &proxy,
        &merchant,
        runtime,
        &dir.path().join("profile-t010"),
    )
    .await;

    let scenarios = [
        (
            MerchantMode::WrongQuantity,
            "wrongqty",
            10_u128,
            11_u128,
            "apple_qty_2",
            99_u64,
            350_u64,
            "EUR",
        ),
        (
            MerchantMode::WrongPrice,
            "wrongprice",
            12_u128,
            13_u128,
            "price_cap",
            2_u64,
            999_u64,
            "EUR",
        ),
        (
            MerchantMode::WrongCurrency,
            "wrongcurrency",
            14_u128,
            15_u128,
            "currency_eur",
            2_u64,
            350_u64,
            "USD",
        ),
    ];

    let btn_spec = TargetSpec {
        strategy: LocatorStrategy::TestId {
            attribute: None,
            value: "add-apple".into(),
        },
        frame_scope: None,
        expected_identity: Some(ExpectedTargetIdentity {
            expected_sku: Some("SYNTHETIC-APPLE-1KG".into()),
            ..Default::default()
        }),
    };
    let assertions = vec![
        BusinessAssertion::UrlPathEquals {
            rule_id: "on_cart_page".into(),
            expected_path: "/cart".into(),
        },
        BusinessAssertion::QuantityEquals {
            rule_id: "apple_qty_2".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-qty".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            expected: 2,
        },
        BusinessAssertion::NumericMinorAtMost {
            rule_id: "price_cap".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-unit-price".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            max_minor: 350,
        },
        BusinessAssertion::ElementTextEquals {
            rule_id: "currency_eur".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-currency".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            expected: "EUR".into(),
        },
    ];

    for (mode, tag, nav_id, click_id, expected_fail_rule, exp_qty, exp_price, exp_currency) in
        scenarios
    {
        merchant.reset_for_mode(mode);

        let nav = BrowserActionIntent {
            action_id: ActionId::from_u128(nav_id),
            logical_effect_key: format!("nav.catalog.{tag}"),
            kind: BrowserActionKind::Navigate {
                url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
            },
            timeout: Duration::from_secs(5),
            requires_model_call: false,
        };
        adapter.perform_action(&handle, &nav, None).await.unwrap();
        let catalog_obs = adapter
            .observe(&handle, ObserveScope::default())
            .await
            .unwrap();

        let resolved = adapter
            .resolve_target(&handle, &btn_spec, &catalog_obs)
            .await
            .unwrap();
        let click = BrowserActionIntent {
            action_id: ActionId::from_u128(click_id),
            logical_effect_key: format!("cart.add.{tag}"),
            kind: BrowserActionKind::Click,
            timeout: Duration::from_secs(5),
            requires_model_call: false,
        };
        let outcome = adapter
            .perform_action(&handle, &click, Some(&resolved))
            .await
            .unwrap();
        assert!(outcome.dispatch_confirmed);
        assert_eq!(merchant.mutation_count(), 1);

        // Une cible résolue sur une génération de page antérieure ne peut plus être réutilisée
        let stale_reuse = adapter
            .perform_action(&handle, &click, Some(&resolved))
            .await
            .unwrap_err();
        assert!(matches!(
            stale_reuse,
            BrowserError::StaleFenceOrContext { .. }
        ));

        let cart_obs = adapter
            .observe(&handle, ObserveScope::default())
            .await
            .unwrap();
        let report = adapter
            .evaluate_assertions(&handle, &assertions, &cart_obs)
            .await
            .unwrap();
        assert!(!report.all_passed);
        for rule_res in &report.results {
            if rule_res.rule_id == expected_fail_rule {
                assert_eq!(rule_res.verdict, AssertionVerdict::Fail);
            } else {
                assert_eq!(
                    rule_res.verdict,
                    AssertionVerdict::Pass,
                    "règle '{}' inattendue en mode {tag}: {}",
                    rule_res.rule_id,
                    rule_res.detail
                );
            }
        }
        let post_err = report.ensure_passed(true).unwrap_err();
        assert!(matches!(
            post_err,
            BrowserError::BusinessPostconditionFailed { .. }
        ));

        let oracle_cart = merchant.cart_snapshot();
        let entry = &oracle_cart["SYNTHETIC-APPLE-1KG"];
        assert_eq!(entry.sku, "SYNTHETIC-APPLE-1KG");
        assert_eq!(entry.quantity, exp_qty);
        assert_eq!(entry.unit_price_minor, exp_price);
        assert_eq!(entry.currency, exp_currency);
        // Aucun retry compensatoire automatique malgré l'échec métier
        assert_eq!(merchant.mutation_count(), 1);
    }

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    merchant.shutdown().await;
}

// B-AC13 / REQ-AUTO-006 / EgressProxy
#[tokio::test]
async fn t_egress_proxy_blocks_unauthorized_origins_redirects_and_model_canary() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let merchant = SyntheticMerchantServer::start().await;
    let evil = HitCounterServer::start().await;
    let canary = HitCounterServer::start().await;
    merchant.set_redirect_targets(&evil.base_url(), &canary.base_url());

    let mut policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    policy.add_model_canary_url(&canary.base_url()).unwrap();

    let proxy = EgressProxy::start(
        RunId::from_u128(3),
        LeaseId::from_u128(22),
        ModelPolicy::Forbidden,
        policy.clone(),
    )
    .await
    .unwrap();
    let wd = FaithfulW3cWebDriverServer::start().await;
    let (adapter, _, _) = new_adapter();

    let handle = adapter
        .open_context(OpenContextRequest {
            run_id: RunId::from_u128(3),
            session_id: SessionId::from_u128(30),
            lease_id: LeaseId::from_u128(22),
            fence: test_fence(),
            webdriver_endpoint: wd.endpoint_url(),
            runtime,
            execution_mode: ExecutionMode::Mediated,
            model_policy: ModelPolicy::Forbidden,
            isolation_level: IsolationLevel::Standard,
            confinement_proof: ConfinementProof::Unconfined,
            egress_policy: policy,
            proxy_url: Some(Url::parse(&proxy.proxy_url_string()).unwrap()),
            user_data_dir: dir.path().join("profile-egress"),
            headless: true,
            script_timeout: Duration::from_secs(5),
            page_load_timeout: Duration::from_secs(5),
        })
        .await
        .unwrap();

    // Navigation autorisée + tentative de redirection 302 vers evil et canary
    let nav_catalog = BrowserActionIntent {
        action_id: ActionId::from_u128(20),
        logical_effect_key: "nav.catalog.egress".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter
        .perform_action(&handle, &nav_catalog, None)
        .await
        .unwrap();
    let obs = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();

    // Clic sur lien /redirect-evil -> le marchand renvoie 302 vers evil, intercepté et bloqué par EgressProxy
    let evil_link = adapter
        .resolve_target(
            &handle,
            &TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "leak-evil".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            &obs,
        )
        .await
        .unwrap();
    let click_evil = BrowserActionIntent {
        action_id: ActionId::from_u128(21),
        logical_effect_key: "click.evil.redirect".into(),
        kind: BrowserActionKind::Click,
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter
        .perform_action(&handle, &click_evil, Some(&evil_link))
        .await
        .unwrap();

    // Requête directe via le proxy vers le canari modèle -> 403
    let stream = TcpStream::connect(proxy.addr()).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{}/v1/chat/completions", canary.base_url()))
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    let _ = resp.into_body().collect().await;

    // Navigation directe hors mandat refusée avant dispatch WebDriver
    let blocked_nav = BrowserActionIntent {
        action_id: ActionId::from_u128(22),
        logical_effect_key: "nav.evil.direct".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/steal", evil.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let err_nav = adapter
        .perform_action(&handle, &blocked_nav, None)
        .await
        .unwrap_err();
    assert!(matches!(err_nav, BrowserError::EgressBlocked { .. }));

    assert_eq!(evil.hits(), 0);
    assert_eq!(canary.hits(), 0);
    assert!(proxy.blocked_count() >= 2);

    // Tunnel CONNECT vers origine non autorisée -> 403
    {
        let stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method("CONNECT")
            .uri(evil.addr().to_string())
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        let _ = resp.into_body().collect().await;
    }

    // Révocation du mandat bloque même l'origine auparavant autorisée
    proxy.revoke();
    {
        let stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method("GET")
            .uri(format!("{}/catalog", merchant.base_url()))
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        let _ = resp.into_body().collect().await;
    }
    assert_eq!(evil.hits(), 0);

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    canary.shutdown().await;
    evil.shutdown().await;
    merchant.shutdown().await;
}

// T-F019-011 / AC-F019-011 / REQ-AUTO-011
#[tokio::test]
async fn t_f019_011_cut_after_dispatch_returns_outcome_unknown_without_retry() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let merchant = SyntheticMerchantServer::start().await;
    let wd = FaithfulW3cWebDriverServer::start().await;
    let policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    let proxy = EgressProxy::start(
        RunId::from_u128(4),
        LeaseId::from_u128(23),
        ModelPolicy::Forbidden,
        policy,
    )
    .await
    .unwrap();
    let (adapter, journal, _) = new_adapter();

    let handle = open_standard_context(
        &adapter,
        &wd,
        &proxy,
        &merchant,
        runtime,
        &dir.path().join("profile-t011"),
    )
    .await;

    let nav = BrowserActionIntent {
        action_id: ActionId::from_u128(30),
        logical_effect_key: "nav.catalog.cut".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let btn = adapter
        .resolve_target(
            &handle,
            &TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "add-apple".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            &obs,
        )
        .await
        .unwrap();

    // Injecter une coupure TCP juste après la mutation sur le marchand
    wd.arm_drop_after_next_click();
    let click = BrowserActionIntent {
        action_id: ActionId::from_u128(31),
        logical_effect_key: "order.cart.add.apple.line1".into(),
        kind: BrowserActionKind::Click,
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let err_cut = adapter
        .perform_action(&handle, &click, Some(&btn))
        .await
        .unwrap_err();
    assert!(matches!(err_cut, BrowserError::OutcomeUnknown { .. }));
    assert_eq!(merchant.mutation_count(), 1);

    // Une tentative de répétition sur la même clé logique doit être refusée sans dispatch
    let retry_click = BrowserActionIntent {
        action_id: ActionId::from_u128(32),
        logical_effect_key: "order.cart.add.apple.line1".into(),
        kind: BrowserActionKind::Click,
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let err_repeat = adapter
        .perform_action(&handle, &retry_click, Some(&btn))
        .await
        .unwrap_err();
    assert!(matches!(err_repeat, BrowserError::OutcomeUnknown { .. }));
    assert_eq!(merchant.mutation_count(), 1);

    let cp = adapter
        .checkpoint(
            &handle,
            CheckpointInput {
                step_ref: "step.cart.add".into(),
                last_observation_id: Some(obs.observation_id),
                metadata: BTreeMap::default(),
            },
        )
        .await
        .unwrap();
    assert!(cp.ephemeral_handles_invalidated);
    assert_eq!(cp.unknown_effects, vec!["order.cart.add.apple.line1"]);

    // Réconciliation explicite depuis l'oracle observé sans second clic
    journal
        .reconcile_outcome(
            "order.cart.add.apple.line1",
            unleash_browser::EffectState::Confirmed,
            "oracle:cart_count_1",
        )
        .unwrap();
    assert_eq!(merchant.mutation_count(), 1);

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    merchant.shutdown().await;
}

// T-F019-006 / AC-F019-006 / REQ-AUTO-006 / REQ-AUTO-014
#[tokio::test]
async fn t_f019_006_zero_model_calls_under_forbidden_with_canary_and_isolation_checks() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let merchant = SyntheticMerchantServer::start().await;
    let canary = HitCounterServer::start().await;
    let wd = FaithfulW3cWebDriverServer::start().await;

    // 1. Contrôle positif hors run : le canari détecte effectivement un appel HTTP direct
    {
        let canary_url = Url::parse(&canary.base_url()).unwrap();
        let addr = format!(
            "{}:{}",
            canary_url.host_str().unwrap(),
            canary_url.port().unwrap()
        );
        let stream = TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
        let _ = resp.into_body().collect().await;
        assert_eq!(canary.hits(), 1);
        canary.reset_hits();
    }

    // 2. Rejeu complet sous forbidden
    let mut policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    policy.add_model_canary_url(&canary.base_url()).unwrap();
    let proxy = EgressProxy::start(
        RunId::from_u128(5),
        LeaseId::from_u128(24),
        ModelPolicy::Forbidden,
        policy,
    )
    .await
    .unwrap();
    let (adapter, _, guard) = new_adapter();

    let handle = open_standard_context(
        &adapter,
        &wd,
        &proxy,
        &merchant,
        runtime.clone(),
        &dir.path().join("profile-t006"),
    )
    .await;

    let nav = BrowserActionIntent {
        action_id: ActionId::from_u128(40),
        logical_effect_key: "nav.catalog.nomodel".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();

    // Tentative indirecte d'appel modèle via ActionIntent -> refusée
    let model_attempt = BrowserActionIntent {
        action_id: ActionId::from_u128(41),
        logical_effect_key: "bad.model.call".into(),
        kind: BrowserActionKind::TakeScreenshot,
        timeout: Duration::from_secs(5),
        requires_model_call: true,
    };
    let err_model = adapter
        .perform_action(&handle, &model_attempt, None)
        .await
        .unwrap_err();
    assert!(matches!(err_model, BrowserError::ModelUseForbidden { .. }));

    // Tentative d'injection de script modèle -> refusée
    let script_attempt = BrowserActionIntent {
        action_id: ActionId::from_u128(42),
        logical_effect_key: "bad.model.script".into(),
        kind: BrowserActionKind::ModelSuppliedExecuteScript {
            script: "return document.title".into(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let err_script = adapter
        .perform_action(&handle, &script_attempt, None)
        .await
        .unwrap_err();
    assert!(matches!(err_script, BrowserError::ModelUseForbidden { .. }));

    // Exécution mécanique nominale par Label + Click + Assertions
    let qty_label = adapter
        .resolve_target(
            &handle,
            &TargetSpec {
                strategy: LocatorStrategy::Label {
                    text: "Quantity".into(),
                    exact: true,
                },
                frame_scope: None,
                expected_identity: None,
            },
            &obs,
        )
        .await
        .unwrap();
    let fill = BrowserActionIntent {
        action_id: ActionId::from_u128(43),
        logical_effect_key: "fill.qty".into(),
        kind: BrowserActionKind::Fill {
            value: "2".into(),
            clear_first: true,
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter
        .perform_action(&handle, &fill, Some(&qty_label))
        .await
        .unwrap();

    let obs_after_fill = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let add_btn = adapter
        .resolve_target(
            &handle,
            &TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "add-apple".into(),
                },
                frame_scope: None,
                expected_identity: Some(ExpectedTargetIdentity {
                    expected_sku: Some("SYNTHETIC-APPLE-1KG".into()),
                    ..Default::default()
                }),
            },
            &obs_after_fill,
        )
        .await
        .unwrap();
    let click_nominal = BrowserActionIntent {
        action_id: ActionId::from_u128(44),
        logical_effect_key: "cart.add.apple.nominal".into(),
        kind: BrowserActionKind::Click,
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    let click_outcome = adapter
        .perform_action(&handle, &click_nominal, Some(&add_btn))
        .await
        .unwrap();
    assert!(click_outcome.dispatch_confirmed);
    assert_eq!(merchant.mutation_count(), 1);

    let obs_cart = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();
    let nominal_assertions = vec![
        BusinessAssertion::UrlOriginEquals {
            rule_id: "same_merchant_origin".into(),
            expected_origin: merchant.base_url(),
        },
        BusinessAssertion::UrlPathEquals {
            rule_id: "on_cart_page".into(),
            expected_path: "/cart".into(),
        },
        BusinessAssertion::TitleContains {
            rule_id: "cart_title".into(),
            substring: "Synthetic Cart".into(),
        },
        BusinessAssertion::ElementTextEquals {
            rule_id: "apple_sku".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-sku".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            expected: "SYNTHETIC-APPLE-1KG".into(),
        },
        BusinessAssertion::QuantityEquals {
            rule_id: "apple_qty_2".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-qty".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            expected: 2,
        },
        BusinessAssertion::NumericMinorAtMost {
            rule_id: "price_cap".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-unit-price".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            max_minor: 350,
        },
        BusinessAssertion::ElementTextEquals {
            rule_id: "currency_eur".into(),
            target: TargetSpec {
                strategy: LocatorStrategy::TestId {
                    attribute: None,
                    value: "cart-currency".into(),
                },
                frame_scope: None,
                expected_identity: None,
            },
            expected: "EUR".into(),
        },
    ];
    let report = adapter
        .evaluate_assertions(&handle, &nominal_assertions, &obs_cart)
        .await
        .unwrap();
    assert!(report.all_passed);
    assert!(
        report
            .results
            .iter()
            .all(|r| r.verdict == AssertionVerdict::Pass)
    );
    report.ensure_passed(true).unwrap();

    let proof = guard.build_zero_model_proof(
        handle.run_id,
        ExecutionMode::Mediated,
        ModelPolicy::Forbidden,
        &ConfinementProof::Unconfined,
        proxy.blocked_count(),
    );
    assert_eq!(proof.model_dispatch_count, 0);
    assert_eq!(proof.model_denied_attempt_count, 2);
    assert_eq!(canary.hits(), 0);

    // 3. project_script sous model_policy=forbidden sans confinement -> IsolationUnavailable
    let iso_err = adapter
        .open_context(OpenContextRequest {
            run_id: RunId::from_u128(6),
            session_id: SessionId::from_u128(60),
            lease_id: LeaseId::from_u128(25),
            fence: test_fence(),
            webdriver_endpoint: wd.endpoint_url(),
            runtime,
            execution_mode: ExecutionMode::ProjectScript,
            model_policy: ModelPolicy::Forbidden,
            isolation_level: IsolationLevel::Standard,
            confinement_proof: ConfinementProof::Unconfined,
            egress_policy: EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap(),
            proxy_url: None,
            user_data_dir: dir.path().join("profile-script"),
            headless: true,
            script_timeout: Duration::from_secs(5),
            page_load_timeout: Duration::from_secs(5),
        })
        .await
        .unwrap_err();
    assert!(matches!(iso_err, BrowserError::IsolationUnavailable { .. }));

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    canary.shutdown().await;
    merchant.shutdown().await;
}

// Rejets Safari, XPath, coordonnées, script modèle, runtime mismatch et inventaire passif
#[tokio::test]
async fn t_runtime_pinning_safari_xpath_coordinates_and_model_script_rejections() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, _, _) = new_adapter();

    let mut safari = create_pinned_dummy_runtime(dir.path(), "18.0");
    safari.engine = BrowserEngineKind::SafariUnsupported;
    let safari_err = adapter.inspect_runtime(&safari).await.unwrap_err();
    assert!(matches!(safari_err, BrowserError::CapabilityMissing { .. }));

    let mut bad_sha = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    bad_sha.browser.expected_sha256 = unleash_domain::Sha256Digest::from_bytes(b"wrong-sha");
    let sha_err = adapter.inspect_runtime(&bad_sha).await.unwrap_err();
    assert!(matches!(sha_err, BrowserError::RuntimeMismatch { .. }));

    let valid = create_pinned_dummy_runtime(dir.path(), "156.0.8078.4");
    let ok = adapter.inspect_runtime(&valid).await.unwrap();
    assert!(ok.passive_inspection_only);
    assert_eq!(ok.verified_version, "156.0.8078.4");

    assert!(matches!(
        TargetSpec::try_from_raw(
            RawLocatorRequest::XPath {
                expr: "//button".into()
            },
            None,
            None,
        ),
        Err(BrowserError::UnsupportedLocatorOrScript { .. })
    ));
    assert!(matches!(
        TargetSpec::try_from_raw(
            RawLocatorRequest::Coordinates { x: 10.0, y: 20.0 },
            None,
            None,
        ),
        Err(BrowserError::UnsupportedLocatorOrScript { .. })
    ));
    assert!(matches!(
        TargetSpec::try_from_raw(
            RawLocatorRequest::ExecuteScript {
                script: "alert(1)".into(),
                origin: ScriptOrigin::ModelSupplied,
            },
            None,
            None,
        ),
        Err(BrowserError::UnsupportedLocatorOrScript { .. })
    ));

    assert!(matches!(
        TargetSpec::try_from_raw(
            RawLocatorRequest::TestId {
                attribute: None,
                value: "add-apple".into(),
            },
            Some("   ".into()),
            None,
        ),
        Err(BrowserError::InvalidConfiguration { .. })
    ));

    // Rejet explicite de TargetSpec::frame_scope (au lieu de l'ignorer silencieusement)
    let merchant = SyntheticMerchantServer::start().await;
    let wd = FaithfulW3cWebDriverServer::start().await;
    let policy = EgressPolicy::from_urls(&[&merchant.base_url()]).unwrap();
    let proxy = EgressProxy::start(
        RunId::from_u128(7),
        LeaseId::from_u128(26),
        ModelPolicy::Forbidden,
        policy,
    )
    .await
    .unwrap();
    let handle = open_standard_context(
        &adapter,
        &wd,
        &proxy,
        &merchant,
        valid,
        &dir.path().join("profile-frame-scope"),
    )
    .await;
    let nav = BrowserActionIntent {
        action_id: ActionId::from_u128(70),
        logical_effect_key: "nav.catalog.framescope".into(),
        kind: BrowserActionKind::Navigate {
            url: Url::parse(&format!("{}/catalog", merchant.base_url())).unwrap(),
        },
        timeout: Duration::from_secs(5),
        requires_model_call: false,
    };
    adapter.perform_action(&handle, &nav, None).await.unwrap();
    let obs = adapter
        .observe(&handle, ObserveScope::default())
        .await
        .unwrap();

    let scoped_spec = TargetSpec::try_from_raw(
        RawLocatorRequest::TestId {
            attribute: None,
            value: "add-apple".into(),
        },
        Some("iframe#checkout".into()),
        None,
    )
    .unwrap();
    let scope_err = adapter
        .resolve_target(&handle, &scoped_spec, &obs)
        .await
        .unwrap_err();
    assert!(matches!(
        scope_err,
        BrowserError::CapabilityMissing { ref details_ref, .. } if details_ref == "target.frame_scope"
    ));
    assert_eq!(scope_err.error_code(), ErrorCode::CapabilityDenied);
    assert_eq!(scope_err.reason_code(), ReasonCode::CapabilityUnavailable);

    let scoped_assert_err = adapter
        .evaluate_assertions(
            &handle,
            &[BusinessAssertion::ElementTextEquals {
                rule_id: "scoped_text".into(),
                target: scoped_spec,
                expected: "Add Apple".into(),
            }],
            &obs,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        scoped_assert_err,
        BrowserError::CapabilityMissing { ref details_ref, .. } if details_ref == "target.frame_scope"
    ));

    adapter
        .close_context(handle, CloseReason::Completed)
        .await
        .unwrap();
    proxy.shutdown().await;
    wd.shutdown().await;
    merchant.shutdown().await;

    let inv = inventory_local_runtimes(&LocalInventorySearchPaths {
        unleash_runtimes_dir: Some(dir.path().to_path_buf()),
        ..Default::default()
    });
    assert!(!inv.paired_chrome_driver_ready);
}
