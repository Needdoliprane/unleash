use super::*;
use unleash_domain::{ActionId, ActionOrigin, ReconcileStrategy};
use unleash_store::{BrokerCause, BrokerIntent, ProviderServiceLaunchContext};

const MANAGED: Uuid = Uuid::from_u128(0xf012_c519);
const LAUNCH: Uuid = Uuid::from_u128(0xf012_c520);

fn configure_ollama(store: &UnleashStore, endpoint: &str) {
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: MANAGED,
        profile_version: 1,
        project_id: None,
        payload: serde_json::from_value::<unleash_protocol::SetProviderProfilePayload>(json!({
            "provider_kind": "ollama",
            "dialect": {"kind": "ollama_chat", "version": 1},
            "endpoint": endpoint,
            "model": "fixture",
            "locality": "local_only",
            "server_management": "unleash_managed",
            "auth": {"kind": "none"},
            "limits": {"max_inflight": 1, "connect_timeout_ms": 2000, "queue_timeout_ms": 1000,
                "load_timeout_ms": 180_000, "first_token_timeout_ms": 60000, "generation_timeout_ms": 60000},
            "declared_capabilities": ["text"],
            "pricing": {"kind": "none_local"}
        }))
        .unwrap().with_normalized_endpoint().unwrap(),
        secret_binding: None,
        catalog_binding: None,
    };
    publish_profile(store, configuration, 0xc519);
}

fn launch(store: &UnleashStore, command: u128, profile: Uuid) -> CommandEnvelope {
    envelope(
        store,
        command,
        LAUNCH,
        "PrepareProviderServiceLaunch",
        json!({
            "profile_id": profile,
            "profile_version": 1,
            "backend": "macos_seatbelt_strict_service_v1",
            "executable": "/fixture/ollama",
            "executable_digest": Sha256Digest::from_bytes(b"fixture executable"),
            "command_digest": Sha256Digest::from_bytes(b"fixture command"),
            "environment_digest": Sha256Digest::from_bytes(b"fixture environment"),
            "confinement_digest": Sha256Digest::from_bytes(b"fixture confinement"),
            "deadline": store.config().clock.now_ms().as_i64() + 60_000,
        }),
    )
}

#[test]
fn durable_preparation_rolls_back_and_replays_without_an_effect() {
    let (store, _directory, _mandate, failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let wire = launch(&store, 0xc520, MANAGED);
    let before = counts(&store);
    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), wire.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);

    let committed = store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    let receipt = committed.receipt();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-service-launch:{LAUNCH}"))
    );
    assert_eq!(context.receipt_id, receipt.receipt_id);
    assert_eq!(context.plan.profile_id, MANAGED);
    assert_eq!(context.plan.profile_version, 1);
    assert_eq!(context.endpoint, "http://127.0.0.1:11434/");
    assert_eq!(context.principal_id.as_str(), PRINCIPAL);
    let mut expected = before.clone();
    for (table, delta) in [(0, 1), (8, 1), (9, 2)] {
        *expected.get_mut(table).unwrap() += delta;
    }
    assert_eq!(counts(&store), expected);
    let process_count: i64 = store
        .read_snapshot(|db, _| {
            Ok(db.query_row("SELECT COUNT(*) FROM processes", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(process_count, 0);

    let replay = store
        .prepare_provider_service_launch(caller(), wire.clone())
        .unwrap();
    assert!(matches!(
        replay,
        unleash_store::CommitOutcome::IdempotentReplay(_)
    ));
    assert_eq!(replay.receipt(), receipt);
    assert_eq!(
        store.read_provider_service_launch(LAUNCH).unwrap(),
        Some(context)
    );
    let mut changed = wire;
    changed
        .payload
        .insert("executable".to_owned(), json!("/other/ollama"));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), changed),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), launch(&store, 0xc521, MANAGED)),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(counts(&store), expected);
}

#[test]
fn preparation_rejects_unmanaged_profiles_expired_plans_and_non_admins() {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let before = counts(&store);
    let wire = launch(&store, 0xc520, MANAGED);
    let mut agent = caller();
    agent.role = CallerRole::Agent;
    assert!(matches!(
        store.prepare_provider_service_launch(agent, wire.clone()),
        Err(StoreError::Unauthorized(_))
    ));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), launch(&store, 0xc520, PROFILE)),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut expired = wire.clone();
    expired.payload.insert("deadline".to_owned(), json!(1));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), expired),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut with_probe_receipt = wire;
    with_probe_receipt
        .payload
        .insert("probe_receipt".to_owned(), json!(Uuid::nil()));
    assert!(matches!(
        store.prepare_provider_service_launch(caller(), with_probe_receipt),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&store), before);
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);
}

fn intent(
    context: &ProviderServiceLaunchContext,
    action: ActionId,
    receipt: ReceiptId,
    verb: &str,
) -> BrokerIntent {
    let payload_digest = unleash_protocol::canonical_sha256(context).unwrap();
    let manifest = json!({
        "action_id": action,
        "subject": {"kind":"control_plane","principal_id":context.principal_id,"cause_ref":format!("receipt:{receipt}"),
            "invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id": context.project_id, "verb": verb,
        "resource_refs": [{"kind": "path", "value": context.plan.executable}],
        "payload_digest": payload_digest, "reconcile_strategy":"observe_then_decide", "effects_class":"process_exec",
        "scope_revision": context.scope_revision, "policy_revision": context.policy_revision,
        "lineage_fence": context.lineage_fence, "action_deadline": context.plan.deadline,
        "isolation_requirements": {"mode":"standard","require_process_confinement":false,"required_capabilities":[]},
        "secret_refs": [], "provenance": {"origin":"engine_internal"}
    }).as_object().unwrap().clone();
    BrokerIntent {
        action_id: action,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: context.principal_id.clone(),
        },
        principal_id: context.principal_id.clone(),
        cause: BrokerCause::CommandReceipt {
            receipt_id: receipt,
        },
        project_id: context.project_id,
        payload_digest,
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: context.lineage_fence,
        action_deadline: context.plan.deadline,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        intent_digest: unleash_protocol::canonical_sha256(&manifest).unwrap(),
        manifest,
    }
}

#[test]
fn broker_refuses_service_intents_and_repurposing_prepared_action_or_receipt() {
    let (store, _directory, mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    store
        .prepare_provider_service_launch(caller(), launch(&store, 0xc520, MANAGED))
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let other_action = ActionId::from_u128(0xc522);
    let before = counts(&store);
    for (action, receipt, verb, refusal) in [
        (
            other_action,
            mandate,
            "model.provider.service.launch",
            "service launch execution is not qualified",
        ),
        (
            context.action_id,
            context.receipt_id,
            "model.provider.service.launch",
            "service launch execution is not qualified",
        ),
        (
            other_action,
            context.receipt_id,
            "fixture.other",
            "service launch preparation cannot authorize another effect",
        ),
        (
            context.action_id,
            mandate,
            "fixture.other",
            "service launch preparation cannot authorize another effect",
        ),
    ] {
        let result =
            store.commit_broker_phase(authorizing(intent(&context, action, receipt, verb), 0xc523));
        assert!(
            matches!(result, Err(StoreError::InvalidArgument { message, .. }) if message == refusal)
        );
    }
    assert_eq!(counts(&store), before);
}

#[test]
fn broker_refuses_preparation_event_as_cause_for_another_effect() {
    let (store, _directory, _mandate, _failure) = fixture();
    configure_ollama(&store, "http://127.0.0.1:11434");
    store
        .prepare_provider_service_launch(caller(), launch(&store, 0xc520, MANAGED))
        .unwrap();
    let context = store.read_provider_service_launch(LAUNCH).unwrap().unwrap();
    let event: String = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT event_id FROM events WHERE causation_receipt_id=?1",
                [context.receipt_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let mut action = intent(
        &context,
        ActionId::from_u128(0xc524),
        context.receipt_id,
        "fixture.other",
    );
    action.cause = BrokerCause::Event {
        event_id: event.parse().unwrap(),
    };
    action.manifest.get_mut("subject").unwrap()["cause_ref"] = json!(format!("event:{event}"));
    action.intent_digest = unleash_protocol::canonical_sha256(&action.manifest).unwrap();
    let before = counts(&store);
    let result = store.commit_broker_phase(authorizing(action, 0xc525));
    assert!(
        matches!(result, Err(StoreError::InvalidArgument { message, .. })
        if message == "service launch preparation cannot authorize another effect")
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn v019_upgrade_preserves_existing_profile_and_does_not_prepare_it() {
    let directory = local_test_dir("f012-service-launch-migration").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.max_supported_schema_version = 18;
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal(PRINCIPAL, "user", caller().actor_id.as_str())
        .unwrap();
    configure_ollama(&store, "http://127.0.0.1:11434");
    let old_receipt = store
        .inspect_command_receipt(&caller().actor_id, CommandId::from_u128(0xc519))
        .unwrap();
    assert!(matches!(
        old_receipt,
        unleash_store::ReceiptInspectionOutcome::Full(_)
    ));
    let old_profile = store
        .resolve_provider_profile_snapshot(MANAGED, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(store.read_provider_service_launch(LAUNCH).unwrap(), None);
    let backup = store
        .prepare_backup(unleash_store::PrepareBackupRequest {
            destination_dir: directory.path().join("before-v019"),
            retention_tier: unleash_store::BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    store.shutdown().unwrap();
    config.max_supported_schema_version = 19;
    let upgraded =
        UnleashStore::open_with_migration_backup(config, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 19);
    assert_eq!(upgraded.read_provider_service_launch(LAUNCH).unwrap(), None);
    assert_eq!(
        upgraded
            .resolve_provider_profile_snapshot(MANAGED, 1)
            .unwrap()
            .value,
        Some(old_profile)
    );
    assert_eq!(
        upgraded
            .inspect_command_receipt(&caller().actor_id, CommandId::from_u128(0xc519))
            .unwrap(),
        old_receipt
    );
    let wire = launch(&upgraded, 0xc520, MANAGED);
    upgraded
        .prepare_provider_service_launch(caller(), wire)
        .unwrap();
    assert_eq!(
        upgraded
            .read_provider_service_launch(LAUNCH)
            .unwrap()
            .unwrap()
            .plan
            .profile_id,
        MANAGED
    );
    upgraded
        .read_snapshot(|db, _| {
            assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
}
