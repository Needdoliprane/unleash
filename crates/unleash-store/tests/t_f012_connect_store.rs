//! T-F012-013 : préparation réseau authentifiée, sans effet OS/fournisseur.
//! Les profils et mandats de ce banc passent par le vrai CommitCommand.
#![expect(clippy::unwrap_used, reason = "Fixtures SQLite locales et assertions")]

use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{
    ActorId, CommandId, DataLocality, MicrosAmount, ProviderProbeId, ReceiptId, RequestId,
    Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ProviderTestLimits, ProviderTestPhase, ReceiptState,
    canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CutpointObserver, PolicyIsolationMode, PolicyScopeKind, PrepareObjectRequest,
    ProviderProbeMandateChange, ProviderProbeMandateIssue, ProviderProfileConfiguration,
    ProviderProfilePreparation, PublishObjectReferenceRequest, StoreConfig, StoreCutpoint,
    StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROFILE: Uuid = Uuid::from_u128(0xf012_c501);
const PRINCIPAL: &str = "connect-store-principal";

struct FailBeforeCommit(AtomicBool);

impl CutpointObserver for FailBeforeCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::SeqCst) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".into(),
                message: "synthetic connect rollback".into(),
            });
        }
        Ok(())
    }
}

fn caller() -> CallerAuthority {
    CallerAuthority {
        actor_id: ActorId::new("user:connect-store").unwrap(),
        role: CallerRole::HumanOperator,
    }
}

fn envelope(
    store: &UnleashStore,
    id: u128,
    aggregate: Uuid,
    kind: &str,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: store.active_authority_generation().unwrap(),
        actor_ref: Some(caller().actor_id.to_string()),
        project_id: None,
        aggregate_id: aggregate,
        expected_revision: Revision::INITIAL,
        command_type: kind.into(),
        payload: payload.as_object().unwrap().clone(),
    }
}

fn command(
    envelope: CommandEnvelope,
    target: AggregateKind,
    mutation: CommandMutation,
) -> CommitCommandRequest {
    CommitCommandRequest {
        actor_id: caller().actor_id,
        envelope,
        target_aggregate: target,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn configure(store: &UnleashStore) {
    configure_profile(store, PROFILE, 1);
}

fn configure_profile(store: &UnleashStore, profile: Uuid, command_id: u128) {
    let configuration = ProviderProfileConfiguration {
        schema_version:1, profile_id:profile, profile_version:1, project_id:None,
        payload:serde_json::from_value(json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":"http://127.0.0.1:1234/v1","model":"synthetic-connect-store","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,"load_timeout_ms":180000,"first_token_timeout_ms":60000,"generation_timeout_ms":60000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}})).unwrap(),
        secret_binding:None, catalog_binding:None,
    };
    let bytes = canonicalize_to_vec(&configuration).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let prepared = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "connect_store_fixture".into(),
            owner_id: profile.to_string(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&prepared, &bytes)
        .unwrap();
    store
        .publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: prepared.pin_id,
            object_hash: hash,
            object_generation: prepared.object_generation,
            owner_kind: "connect_store_fixture".into(),
            owner_id: profile.to_string(),
            source_field: "configuration".into(),
        })
        .unwrap();
    let wire = envelope(
        store,
        command_id,
        profile,
        "SetProviderProfile",
        serde_json::to_value(&configuration.payload).unwrap(),
    );
    store
        .commit_command(command(
            wire,
            AggregateKind::ProviderProfile,
            CommandMutation::SetProviderProfile {
                caller: caller(),
                preparation: Box::new(ProviderProfilePreparation {
                    configuration,
                    content_object_hash: hash,
                }),
            },
        ))
        .unwrap();
}

fn change(store: &UnleashStore) -> ProviderProbeMandateChange {
    ProviderProbeMandateChange::Issue(ProviderProbeMandateIssue {
        principal_id: ActorId::new(PRINCIPAL).unwrap(),
        profile_id: PROFILE,
        profile_version: 1,
        phase: ProviderTestPhase::Connect,
        locality: DataLocality::LocalOnly,
        capability_set: vec![],
        limits: ProviderTestLimits::new(1000, 0, 0, MicrosAmount::ZERO),
        expires_at_ms: TimestampMs::new(store.config().clock.now_ms().as_i64() + 60_000),
    })
}

fn policy(
    store: &UnleashStore,
    id: u128,
    revision: u64,
    change: ProviderProbeMandateChange,
) -> CommitCommandRequest {
    let mut wire = envelope(
        store,
        id,
        store.active_authority_generation().unwrap().as_uuid(),
        "SetPolicy",
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"connect-fixture-v1","reason":"synthetic connect permission","provider_probe_mandate":change}),
    );
    wire.expected_revision = Revision::new(revision);
    command(
        wire,
        AggregateKind::SecurityPolicy,
        CommandMutation::SetPolicyWithProviderProbeMandate {
            scope_kind: PolicyScopeKind::Global,
            scope_id: "global".into(),
            isolation_mode: PolicyIsolationMode::Standard,
            policy_version: "connect-fixture-v1".into(),
            reason: "synthetic connect permission".into(),
            caller: caller(),
            mandate: change,
        },
    )
}

fn fixture() -> (
    UnleashStore,
    impl std::fmt::Debug,
    ReceiptId,
    Arc<FailBeforeCommit>,
) {
    let directory = local_test_dir("f012-connect-store").unwrap();
    let failure = Arc::new(FailBeforeCommit(AtomicBool::new(false)));
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = failure.clone();
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal(PRINCIPAL, "user", caller().actor_id.as_str())
        .unwrap();
    store
        .register_principal("connect-other", "user", "user:connect-other")
        .unwrap();
    configure(&store);
    let receipt = store
        .commit_command(policy(&store, 2, 0, change(&store)))
        .unwrap()
        .receipt()
        .receipt_id;
    (store, directory, receipt, failure)
}

fn connect(store: &UnleashStore, id: u128, mandate: ReceiptId) -> CommandEnvelope {
    envelope(
        store,
        id,
        Uuid::from_u128(id),
        "TestProviderProfile",
        json!({"profile_id":PROFILE,"profile_version":1,"phase":"connect","capability_set":[],"limits":{"timeout_ms":1000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"locality":"local_only","mandate_ref":mandate}),
    )
}

fn prepare(store: &UnleashStore, wire: &CommandEnvelope) -> CommitCommandRequest {
    let preparation = store
        .prepare_provider_probe_connect(wire, &caller())
        .unwrap();
    command(
        wire.clone(),
        AggregateKind::ProviderProbe,
        CommandMutation::PrepareProviderProbeConnect {
            caller: caller(),
            preparation: Box::new(preparation),
        },
    )
}

fn counts(store: &UnleashStore) -> Vec<i64> {
    store
        .read_snapshot(|db, _| {
            [
                "command_receipts",
                "provider_probes",
                "provider_probe_transport_contexts",
                "actions",
                "model_calls",
                "model_call_manifests",
                "capacity_reservations",
                "budget_reservations",
                "events",
                "outbox",
            ]
            .iter()
            .map(|table| {
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(StoreError::from)
            })
            .collect()
        })
        .unwrap()
}

#[test]
fn t_f012_013_connect_receipt_context_and_replay_have_no_effect() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 10, mandate);
    let before = counts(&store);
    let request = prepare(&store, &wire);
    assert_eq!(
        counts(&store),
        before,
        "résoudre n'est pas préparer une intention"
    );
    let committed = store.commit_command(request.clone()).unwrap();
    let record = store
        .inspect_provider_probe_connect_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))
        .unwrap()
        .value
        .unwrap();
    assert_eq!(committed.receipt().state, ReceiptState::Accepted);
    assert_eq!(record.context.receipt_id, committed.receipt().receipt_id);
    assert_eq!(record.context.mandate_ref, mandate);
    assert_eq!(record.context.actor_id, caller().actor_id);
    assert_eq!(record.context.principal_id.as_str(), PRINCIPAL);
    assert_eq!(record.context.destination, "http://127.0.0.1:1234/v1");
    let after = counts(&store);
    assert_eq!(
        &after[3..8],
        &[0, 0, 0, 0, 0],
        "aucune action, ModelCall ou réservation avant Authorizing/admission"
    );
    assert_eq!(
        store.commit_command(request.clone()).unwrap().receipt(),
        committed.receipt()
    );
    assert_eq!(counts(&store), after);
    let mut divergent = request;
    divergent
        .envelope
        .payload
        .insert("capability_set".into(), json!(["text"]));
    assert!(matches!(
        store.commit_command(divergent),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert_eq!(counts(&store), after);
}

#[test]
fn t_f012_013_connect_mandate_owner_profile_and_phase_are_exact() {
    let (store, _directory, mandate, _) = fixture();
    let other_profile = Uuid::from_u128(0xf012_c502);
    configure_profile(&store, other_profile, 4);
    let mut inventory = change(&store);
    if let ProviderProbeMandateChange::Issue(permission) = &mut inventory {
        permission.phase = ProviderTestPhase::Inventory;
    }
    let inventory_receipt = store
        .commit_command(policy(&store, 5, 1, inventory))
        .unwrap()
        .receipt()
        .receipt_id;
    let wire = connect(&store, 11, mandate);
    let before = counts(&store);
    let mut foreign = wire.clone();
    foreign
        .payload
        .insert("profile_id".into(), json!(other_profile));
    assert!(matches!(
        store.prepare_provider_probe_connect(&foreign, &caller()),
        Err(StoreError::NotFound(_))
    ));
    let mut phase = wire.clone();
    phase.payload.insert("phase".into(), json!("inventory"));
    assert!(
        store
            .prepare_provider_probe_connect(&phase, &caller())
            .is_err()
    );
    let mut wrong_grant = wire.clone();
    wrong_grant
        .payload
        .insert("mandate_ref".into(), json!(inventory_receipt));
    assert!(matches!(
        store.prepare_provider_probe_connect(&wrong_grant, &caller()),
        Err(StoreError::NotFound(_))
    ));
    let other = CallerAuthority {
        actor_id: ActorId::new("user:connect-other").unwrap(),
        role: CallerRole::HumanOperator,
    };
    let mut other_wire = wire;
    other_wire.actor_ref = Some(other.actor_id.to_string());
    assert!(matches!(
        store.prepare_provider_probe_connect(&other_wire, &other),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_caller_cannot_be_replaced_by_request_metadata() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 17, mandate);
    let request = prepare(&store, &wire);
    let before = counts(&store);
    let mut forged_actor = request;
    forged_actor.actor_id = ActorId::new("user:connect-other").unwrap();
    assert!(matches!(
        store.commit_command(forged_actor),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut forged_ref = wire;
    forged_ref.actor_ref = Some("user:connect-other".to_owned());
    assert!(matches!(
        store.prepare_provider_probe_connect(&forged_ref, &caller()),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_c1_rolls_back_receipt_context_and_outbox() {
    let (store, _directory, mandate, failure) = fixture();
    let request = prepare(&store, &connect(&store, 12, mandate));
    let before = counts(&store);
    failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), before);
    store.commit_command(request).unwrap();
}

#[test]
fn t_f012_013_connect_rechecks_mandate_at_commit_but_replays_original_receipt() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 13, mandate);
    let first = prepare(&store, &wire);
    let pending = prepare(&store, &connect(&store, 14, mandate));
    let receipt = store
        .commit_command(first.clone())
        .unwrap()
        .receipt()
        .clone();
    store
        .commit_command(policy(
            &store,
            3,
            1,
            ProviderProbeMandateChange::Revoke {
                mandate_ref: mandate,
            },
        ))
        .unwrap();
    let before = counts(&store);
    assert!(matches!(
        store.commit_command(pending),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(store.commit_command(first).unwrap().receipt(), &receipt);
    // A reconnect has only the original wire, not an old in-memory preparation.
    // The authenticated generic TestProviderProfile lookup precedes resolution.
    assert!(matches!(
        store.prepare_provider_probe_connect(&wire, &caller()),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(
        store
            .lookup_provider_validation_command(&wire, &caller())
            .unwrap()
            .as_ref(),
        Some(&receipt)
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_preparation_cannot_invent_an_operation() {
    let (store, _directory, mandate, _) = fixture();
    let mut request = prepare(&store, &connect(&store, 19, mandate));
    request.operation_id = Some(unleash_domain::OperationId::from_u128(19));
    let before = counts(&store);
    let outcome = store.commit_command(request);
    assert!(
        matches!(&outcome, Err(StoreError::InvalidArgument { .. })),
        "aucune opération n'a été créée : {outcome:?}"
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_other_command_cannot_reuse_probe_or_old_lineage() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 15, mandate);
    store.commit_command(prepare(&store, &wire)).unwrap();
    let mut second = wire.clone();
    second.command_id = CommandId::from_u128(16);
    second.request_id = RequestId::from_u128(16);
    let before = counts(&store);
    assert!(matches!(
        store.commit_command(prepare(&store, &second)),
        Err(StoreError::RevisionConflict { .. })
    ));
    let mut old = wire;
    old.authority_generation = unleash_domain::AuthorityGeneration::from_u128(1);
    assert!(matches!(
        store.prepare_provider_probe_connect(&old, &caller()),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert_eq!(counts(&store), before);
}

fn connect_intent(
    record: &unleash_store::ProviderProbeConnectRecord,
) -> unleash_store::BrokerIntent {
    let c = &record.context;
    let fence = unleash_domain::LineageFence::new(
        c.authority_generation,
        c.daemon_epoch,
        unleash_domain::Fence::ZERO,
    );
    let manifest = json!({
        "action_id":c.action_id,
        "subject":{"kind":"control_plane","principal_id":c.principal_id,"cause_ref":format!("receipt:{}",c.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id":c.project_id,"verb":"model.provider.connect",
        "resource_refs":[{"kind":"network_endpoint","host":"127.0.0.1","port":1234,"is_private_or_loopback":true}],
        "payload_digest":c.request_digest,"reconcile_strategy":"observe_then_decide","effects_class":"network_egress",
        "scope_revision":c.scope_revision,"policy_revision":c.policy_revision,"lineage_fence":fence,"action_deadline":c.deadline,
        "isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},
        "secret_refs":[],"provenance":{"origin":"engine_internal"},
    }).as_object().unwrap().clone();
    unleash_store::BrokerIntent {
        action_id: c.action_id,
        origin: unleash_domain::ActionOrigin::ControlPlane {
            control_principal_id: c.principal_id.clone(),
        },
        principal_id: c.principal_id.clone(),
        cause: unleash_store::BrokerCause::CommandReceipt {
            receipt_id: c.receipt_id,
        },
        project_id: c.project_id,
        payload_digest: c.request_digest,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.deadline,
        reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
        intent_digest: unleash_protocol::canonical_sha256(&manifest).unwrap(),
        manifest,
    }
}

fn authorizing(
    intent: unleash_store::BrokerIntent,
    evaluation: u128,
) -> unleash_store::BrokerPhaseRequest {
    unleash_store::BrokerPhaseRequest {
        authority_generation: intent.lineage_fence.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: unleash_store::BrokerEvaluation {
            evaluation_id: unleash_domain::AuthorizationEvaluationId::from_u128(evaluation),
            previous_evaluation_id: None,
        },
        phase: unleash_store::BrokerPhase::Authorizing { intent },
    }
}

#[test]
fn t_f012_013_connect_without_claim_cannot_enter_generic_broker() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 18, mandate);
    store.commit_command(prepare(&store, &wire)).unwrap();
    let record = store
        .inspect_provider_probe_connect_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))
        .unwrap()
        .value
        .unwrap();
    let intent = connect_intent(&record);
    let request = authorizing(intent, 18);
    let before = counts(&store);
    let outcome = store.commit_broker_phase(request);
    assert!(
        matches!(&outcome, Err(StoreError::EvidenceUnavailable(_))),
        "un contexte reserved ne prouve pas une claim d'exécution : {outcome:?}"
    );
    assert_eq!(
        counts(&store),
        before,
        "aucune intention, place ou ModelCall sans claim"
    );
}

#[test]
fn t_f012_013_connect_evidence_missing_context_cannot_use_another_receipt() {
    let (store, _directory, mandate, _) = fixture();
    let wire = connect(&store, 20, mandate);
    store.commit_command(prepare(&store, &wire)).unwrap();
    let record = store
        .inspect_provider_probe_connect_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))
        .unwrap()
        .value
        .unwrap();
    let configured = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let mut intent = connect_intent(&record);
    // A real receipt from another operation is not a transport preparation.
    intent.action_id = unleash_domain::ActionId::from_uuid(store.config().id_generator.new_uuid());
    intent.cause = unleash_store::BrokerCause::CommandReceipt {
        receipt_id: configured.receipt_id,
    };
    intent
        .manifest
        .insert("action_id".into(), json!(intent.action_id));
    intent
        .manifest
        .get_mut("subject")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "cause_ref".into(),
            json!(format!("receipt:{}", configured.receipt_id)),
        );
    intent.intent_digest = unleash_protocol::canonical_sha256(&intent.manifest).unwrap();
    let before = counts(&store);
    let outcome = store.commit_broker_phase(authorizing(intent, 20));
    assert!(
        matches!(&outcome, Err(StoreError::NotFound(_))),
        "un reçu de configuration ne remplace pas le contexte : {outcome:?}"
    );
    assert_eq!(counts(&store), before);
}

#[test]
fn t_f012_013_connect_evidence_blob_is_rechecked_before_new_commit_but_not_replay() {
    for absent in [true, false] {
        let (store, _directory, mandate, _) = fixture();
        let wire = connect(&store, 21, mandate);
        let first = prepare(&store, &wire);
        let pending = prepare(&store, &connect(&store, 22, mandate));
        let receipt = store
            .commit_command(first.clone())
            .unwrap()
            .receipt()
            .clone();
        let profile = store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap();
        let hash = profile.configuration_digest;
        let relative: String = store
            .read_snapshot(|db, _| {
                db.query_row(
                    "SELECT relative_path FROM object_records WHERE object_hash=?1",
                    [hash.to_string()],
                    |row| row.get(0),
                )
                .map_err(StoreError::from)
            })
            .unwrap();
        let path = store.config().artifacts_dir.join(relative);
        // Only this fixture's exact source object is made unavailable/corrupt.
        if absent {
            std::fs::remove_file(&path).unwrap();
        } else {
            std::fs::write(&path, b"synthetic-corrupt-configuration").unwrap();
        }
        let before = counts(&store);
        let outcome = store.commit_command(pending);
        assert!(
            matches!(&outcome, Err(StoreError::EvidenceUnavailable(_))),
            "commit sans blob valide, absent={absent} : {outcome:?}"
        );
        assert_eq!(
            store
                .lookup_provider_validation_command(&wire, &caller())
                .unwrap()
                .as_ref(),
            Some(&receipt)
        );
        assert_eq!(store.commit_command(first).unwrap().receipt(), &receipt);
        assert_eq!(counts(&store), before);
    }
}
