//! T-F012-002/011/013 : configuration immuable ; banc `SQLite` sans réseau/coffre.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et assertions de contrat"
)]

use serde_json::json;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{ActorId, CommandId, ProjectId, RequestId, Revision, SecretRef, Sha256Digest};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ReceiptState, SetProviderProfilePayload, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CutpointObserver, NoopCutpointObserver, PrepareObjectRequest, ProviderProfileConfiguration,
    ProviderProfilePreparation, ProviderProfileQualification, ProviderSecretBinding,
    PublishObjectReferenceRequest, SecretReferenceInput, StoreCutpoint, StoreError,
};
use unleash_store::{
    BackupRetentionTier, PrepareBackupRequest, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROFILE: Uuid = Uuid::from_u128(0xf012);

fn publish(store: &UnleashStore, value: &impl serde::Serialize) -> Sha256Digest {
    let bytes = canonicalize_to_vec(value).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "f012_fixture".to_owned(),
            owner_id: hash.to_string(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "f012_fixture".to_owned(),
            owner_id: hash.to_string(),
            source_field: "content".to_owned(),
        })
        .unwrap();
    hash
}

fn fixture(cutpoints: Arc<dyn CutpointObserver>) -> (UnleashStore, impl std::fmt::Debug) {
    let directory = local_test_dir("f012-profile-versions").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = cutpoints;
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    store
        .register_principal("other-user", "user", "user:other")
        .unwrap();
    (store, directory)
}

fn actor() -> ActorId {
    ActorId::new("user:profile").unwrap()
}

fn payload(model: &str) -> SetProviderProfilePayload {
    serde_json::from_value(json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":"http://127.0.0.1:1234/v1","model":model,"locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":300_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}})).unwrap()
}

fn prepare(
    store: &UnleashStore,
    version: u64,
    project: Option<ProjectId>,
    model: &str,
) -> ProviderProfilePreparation {
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: PROFILE,
        profile_version: version,
        project_id: project,
        payload: payload(model),
        secret_binding: None,
        catalog_binding: None,
    };
    let content_object_hash = publish(store, &configuration);
    ProviderProfilePreparation {
        configuration,
        content_object_hash,
    }
}

fn request(
    store: &UnleashStore,
    preparation: ProviderProfilePreparation,
    id: u128,
) -> CommitCommandRequest {
    let doc = &preparation.configuration;
    CommitCommandRequest {
        actor_id: actor(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(id),
            command_id: CommandId::from_u128(id),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some(actor().to_string()),
            project_id: doc.project_id,
            aggregate_id: doc.profile_id,
            expected_revision: Revision::new(doc.profile_version - 1),
            command_type: "SetProviderProfile".to_owned(),
            payload: serde_json::to_value(&doc.payload)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        },
        target_aggregate: AggregateKind::ProviderProfile,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::SetProviderProfile {
            caller: CallerAuthority {
                actor_id: actor(),
                role: CallerRole::HumanOperator,
            },
            preparation: Box::new(preparation),
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn counts(store: &UnleashStore) -> (i64, i64, i64, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT COUNT(*) FROM provider_profiles WHERE profile_id IS NOT NULL",
                    [],
                    |r| r.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM command_receipts WHERE command_type='SetProviderProfile'",
                    [],
                    |r| r.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type='ProviderProfileConfigured'",
                    [],
                    |r| r.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_profile'",
                    [],
                    |r| r.get(0),
                )?,
            ))
        })
        .unwrap()
}

fn probe_command(
    store: &UnleashStore,
    reference: &str,
    requested_id: &str,
    model: &str,
    id: u128,
) -> CommitCommandRequest {
    let hash = publish(
        store,
        &json!({"schema_version":1,"fixture":"synthetic-sqlite-profile-test","tested":[],"unknown":["text"]}),
    );
    let preparation = unleash_store::ProviderProbePreparation {
        requested_profile_id: requested_id.to_owned(),
        requested_profile_version: 1,
        profile_version_ref: reference.to_owned(),
        resolved_model_id: model.to_owned(),
        capability_set: vec!["text".to_owned()],
        capability_snapshot_hash: hash,
        fixture_id: "profile-version-fixture".to_owned(),
        fixture_digest: hash,
        payload_manifest_hash: hash,
        model_policy_hash: hash,
        data_policy_hash: hash,
        model_policy: unleash_domain::ModelPolicy::AllowedBounded,
        locality: unleash_domain::DataLocality::LocalOnly,
        mandate_ref: "synthetic-profile-bench".to_owned(),
        source_run_id: None,
        secret_refs: vec![],
        limits: unleash_store::ProbeGenerationLimits {
            timeout_ms: 1000,
            max_calls: 1,
            max_output_tokens: 1,
            max_cost_usd_micros: unleash_domain::MicrosAmount::ZERO,
        },
    };
    let mut command = request(store, prepare(store, 1, None, "unused"), id);
    "TestProviderProfile".clone_into(&mut command.envelope.command_type);
    command.envelope.aggregate_id = Uuid::from_u128(id);
    command.target_aggregate = AggregateKind::ProviderProbe;
    command.envelope.payload.clone_from(json!({"profile_id":requested_id,"profile_version":1,"phase":"generation_probe","capability_set":preparation.capability_set,"limits":preparation.limits,"locality":preparation.locality,"mandate_ref":preparation.mandate_ref}).as_object().unwrap());
    command.mutation = CommandMutation::PrepareProviderProbe {
        probe_id: unleash_domain::ProviderProbeId::from_u128(id),
        caller: CallerAuthority {
            actor_id: actor(),
            role: CallerRole::HumanOperator,
        },
        preparation,
    };
    command
}

#[test]
fn t_f012_002_probe_cannot_substitute_configured_uuid_version_model_or_scope() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(
            &store,
            prepare(&store, 1, None, "exact-model"),
            600,
        ))
        .unwrap();
    let record = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let wrong = probe_command(
        &store,
        &record.profile_version_ref,
        &Uuid::from_u128(99).to_string(),
        "exact-model",
        601,
    );
    assert!(
        store.commit_command(wrong).is_err(),
        "configured reference must not attest another UUID"
    );
    let wrong = probe_command(
        &store,
        &record.profile_version_ref,
        &PROFILE.to_string(),
        "another-model",
        602,
    );
    assert!(store.commit_command(wrong).is_err());
    store
        .commit_command(probe_command(
            &store,
            &record.profile_version_ref,
            &PROFILE.to_string(),
            "exact-model",
            603,
        ))
        .unwrap();
}

#[test]
fn t_f012_002_configured_probe_identity_cannot_be_substituted_by_legacy_reference() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(
            &store,
            prepare(&store, 1, None, "exact-model"),
            12_000,
        ))
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_| {tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms) VALUES('legacy-substitution','provider','local_only','none_local','user_declared',0)",[])?;Ok(())}).unwrap();
    assert!(
        store
            .commit_command(probe_command(
                &store,
                "legacy-substitution",
                &PROFILE.to_string(),
                "exact-model",
                12_001
            ))
            .is_err(),
        "legacy ref must not attest configured UUID/version"
    );
    assert!(
        store
            .inspect_provider_probe(unleash_domain::ProviderProbeId::from_u128(12_001))
            .unwrap()
            .is_none()
    );
}

#[test]
fn t_f012_002_probe_timeout_cannot_exceed_configured_generation_bound() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let mut prep = prepare(&store, 1, None, "exact-model");
    let mut value = serde_json::to_value(&prep.configuration.payload).unwrap();
    *value
        .get_mut("limits")
        .unwrap()
        .get_mut("generation_timeout_ms")
        .unwrap() = json!(10);
    prep.configuration.payload = serde_json::from_value(value).unwrap();
    prep.content_object_hash = publish(&store, &prep.configuration);
    store.commit_command(request(&store, prep, 13_000)).unwrap();
    let record = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    assert!(
        store
            .commit_command(probe_command(
                &store,
                &record.profile_version_ref,
                &PROFILE.to_string(),
                "exact-model",
                13_001
            ))
            .is_err(),
        "probe timeout must not exceed this version's bound"
    );
}

#[test]
fn t_f012_002_versions_receipts_exact_resolution_and_immutability() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let command = request(&store, prepare(&store, 1, None, "model-v1"), 100);
    let first = store.commit_command(command.clone()).unwrap();
    let v1 = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(first.receipt().state, ReceiptState::Accepted);
    assert_eq!(v1.receipt_id, first.receipt().receipt_id);
    assert_eq!(v1.principal_id.as_str(), "profile-user");
    assert_eq!(v1.qualification, ProviderProfileQualification::Required);
    let mut retry = command.clone();
    retry.envelope.request_id = RequestId::from_u128(999);
    assert_eq!(
        store.commit_command(retry).unwrap().receipt(),
        first.receipt()
    );
    let second = store
        .commit_command(request(&store, prepare(&store, 2, None, "model-v2"), 101))
        .unwrap();
    assert_eq!(second.receipt().new_revision, Revision::new(2));
    let v2 = store
        .resolve_provider_profile_snapshot(PROFILE, 2)
        .unwrap()
        .value
        .unwrap();
    assert_ne!(v1.profile_version_ref, v2.profile_version_ref);
    assert_ne!(v1.configuration_digest, v2.configuration_digest);
    assert_eq!(
        store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap(),
        v1
    );
    assert!(
        store
            .resolve_provider_profile_snapshot(PROFILE, 3)
            .unwrap()
            .value
            .is_none()
    );
    assert!(
        store
            .resolve_provider_profile_snapshot(Uuid::from_u128(123), 1)
            .unwrap()
            .value
            .is_none()
    );
    let mut different = command;
    different
        .envelope
        .payload
        .insert("model".to_owned(), json!("other"));
    assert!(store.commit_command(different).is_err());
    assert_eq!(counts(&store), (2, 2, 2, 2));
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_| {
        assert!(tx.execute("UPDATE provider_profiles SET provider_id='replacement' WHERE profile_id IS NOT NULL",[]).is_err());
        assert!(tx.execute("DELETE FROM provider_profiles WHERE profile_id IS NOT NULL",[]).is_err());
        Ok(())
    }).unwrap();
}

#[test]
fn t_f012_002_concurrent_cas_commits_one_version_and_one_event() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, prepare(&store, 1, None, "v1"), 200))
        .unwrap();
    let a = request(&store, prepare(&store, 2, None, "v2-a"), 201);
    let b = request(&store, prepare(&store, 2, None, "v2-b"), 202);
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(3));
    let threads = [a, b]
        .into_iter()
        .map(|command| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.commit_command(command)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let outcomes = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(StoreError::RevisionConflict { .. })))
            .count(),
        1
    );
    assert_eq!(counts(&store), (2, 2, 2, 2));
}

struct FailOnce {
    point: StoreCutpoint,
    armed: AtomicBool,
}
impl CutpointObserver for FailOnce {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == self.point && self.armed.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: format!("{point:?}"),
                message: "synthetic profile cutpoint".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f012_002_c1_rolls_back_configuration_receipt_reference_and_event() {
    let failure = Arc::new(FailOnce {
        point: StoreCutpoint::C1BeforeCommandCommit,
        armed: AtomicBool::new(false),
    });
    let (store, _directory) = fixture(failure.clone());
    let command = request(&store, prepare(&store, 1, None, "v1"), 300);
    failure.armed.store(true, Ordering::Release);
    assert!(store.commit_command(command.clone()).is_err());
    assert_eq!(counts(&store), (0, 0, 0, 0));
    store.commit_command(command).unwrap();
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

#[test]
fn t_f012_002_c2_returns_original_receipt_without_new_version() {
    let failure = Arc::new(FailOnce {
        point: StoreCutpoint::C2AfterCommandCommit,
        armed: AtomicBool::new(false),
    });
    let (store, _directory) = fixture(failure.clone());
    let command = request(&store, prepare(&store, 1, None, "v1"), 301);
    failure.armed.store(true, Ordering::Release);
    assert!(store.commit_command(command.clone()).is_err());
    let before = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let retry = store.commit_command(command).unwrap();
    assert_eq!(retry.receipt().receipt_id, before.receipt_id);
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

fn secret(store: &UnleashStore, project: Option<ProjectId>) -> ProviderSecretBinding {
    let reference = SecretReferenceInput {
        secret_ref_id: "provider-credential".to_owned(),
        secret_ref: SecretRef::new("secret://keychain/provider/configuration").unwrap(),
        namespace: "provider".to_owned(),
        purpose: "api_key".to_owned(),
        secret_kind: "bearer_token".to_owned(),
        target_kind: "provider_profile".to_owned(),
        target_id: PROFILE.to_string(),
        scope_kind: if project.is_some() { "project" } else { "user" }.to_owned(),
        scope_id: project.map_or("default".to_owned(), |id| id.to_string()),
        backend_kind: "os_keychain".to_owned(),
    };
    let mut command = request(store, prepare(store, 1, project, "unused"), 400);
    "SetSecretReference".clone_into(&mut command.envelope.command_type);
    command.envelope.aggregate_id = Uuid::from_u128(400);
    command.envelope.payload.clear();
    command.target_aggregate = AggregateKind::SecretReference;
    command.mutation = CommandMutation::SetSecretReference {
        input: reference.clone(),
        caller: CallerAuthority {
            actor_id: actor(),
            role: CallerRole::HumanOperator,
        },
    };
    let outcome = store.commit_command(command).unwrap();
    ProviderSecretBinding {
        reference,
        revision: outcome.receipt().new_revision,
    }
}

fn with_secret(store: &UnleashStore, binding: ProviderSecretBinding) -> ProviderProfilePreparation {
    let mut prep = prepare(store, 1, None, "fixture-model");
    let mut value = serde_json::to_value(&prep.configuration.payload).unwrap();
    *value.get_mut("auth").unwrap() =
        json!({"kind":"secret_ref","secret_ref":binding.reference.secret_ref});
    prep.configuration.payload = serde_json::from_value(value).unwrap();
    prep.configuration.secret_binding = Some(binding);
    prep.content_object_hash = publish(store, &prep.configuration);
    prep
}

#[test]
fn t_f012_013_secret_binding_is_exact_and_never_resolved_or_rebound() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let binding = secret(&store, None);
    for variant in 0..7 {
        let mut wrong = binding.clone();
        match variant {
            0 => wrong.reference.target_id = Uuid::from_u128(88).to_string(),
            1 => wrong.reference.scope_id = "another".to_owned(),
            2 => wrong.reference.backend_kind = "another".to_owned(),
            3 => wrong.reference.secret_kind = "another".to_owned(),
            4 => wrong.revision = Revision::new(0),
            5 => wrong.reference.secret_ref_id = "absent".to_owned(),
            _ => wrong.reference.secret_ref = SecretRef::new("secret://keychain/missing").unwrap(),
        }
        assert!(
            store
                .commit_command(request(&store, with_secret(&store, wrong), 410 + variant))
                .is_err()
        );
        assert_eq!(counts(&store), (0, 0, 0, 0));
    }
    let command = request(&store, with_secret(&store, binding), 420);
    let first = store.commit_command(command.clone()).unwrap();
    let original = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_| {tx.execute("UPDATE secret_references SET target_id='retargeted',revision=revision+1 WHERE secret_ref_id='provider-credential'",[])?;Ok(())}).unwrap();
    assert_eq!(
        store.commit_command(command).unwrap().receipt(),
        first.receipt()
    );
    assert_eq!(
        store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap(),
        original
    );
    let mut next = original.configuration;
    next.profile_version = 2;
    let prep = ProviderProfilePreparation {
        content_object_hash: publish(&store, &next),
        configuration: next,
    };
    assert!(store.commit_command(request(&store, prep, 421)).is_err());
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

#[test]
fn t_f012_002_scope_actor_generation_and_object_are_not_payload_grants() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let base = request(&store, prepare(&store, 1, None, "v1"), 500);
    let mut wrong = base.clone();
    wrong.envelope.project_id = Some(ProjectId::from_u128(44));
    assert!(store.commit_command(wrong).is_err());
    let mut wrong = base.clone();
    if let CommandMutation::SetProviderProfile { caller, .. } = &mut wrong.mutation {
        caller.role = CallerRole::Agent;
    }
    assert!(store.commit_command(wrong).is_err());
    let mut wrong = base.clone();
    wrong.actor_id = ActorId::new("user:other").unwrap();
    assert!(store.commit_command(wrong).is_err());
    let mut wrong = base.clone();
    wrong.envelope.authority_generation = unleash_domain::AuthorityGeneration::from_u128(1);
    assert!(matches!(
        store.commit_command(wrong),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let mut wrong = base;
    if let CommandMutation::SetProviderProfile { preparation, .. } = &mut wrong.mutation {
        preparation.content_object_hash = publish(&store, &json!({"not":"configuration"}));
    }
    assert!(store.commit_command(wrong).is_err());
    let absent = request(
        &store,
        prepare(&store, 1, Some(ProjectId::from_u128(44)), "v1"),
        501,
    );
    assert!(store.commit_command(absent).is_err());
    assert_eq!(counts(&store), (0, 0, 0, 0));
}

#[test]
fn t_f012_011_catalog_exact_entry_provenance_tiers_and_conflicts_are_pinned() {
    use unleash_domain::TimestampMs;
    use unleash_store::{ProviderCatalogBinding, ProviderCatalogSource};
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    // Explicit synthetic import supplied by a server resolver; no signature is claimed.
    let entry = json!({"model":"exact-model","currency":"USD","input_micros_per_million":100,"tiers":[{"from_tokens":0,"rate":100},{"from_tokens":100_000,"rate":200}],"valid_until_ms":500_000});
    let catalog = json!({"schema_version":1,"entries":{"exact-model":entry}});
    let catalog_hash = publish(&store, &catalog);
    let entry_hash = unleash_protocol::canonical_sha256(&entry).unwrap();
    let mut prep = prepare(&store, 1, None, "exact-model");
    let mut config = serde_json::to_value(&prep.configuration.payload).unwrap();
    *config.get_mut("pricing").unwrap() = json!({"kind":"catalog","catalog_id":"synthetic-import","catalog_version":"r1","catalog_digest":catalog_hash,"entry_digest":entry_hash,"stale_policy":"block"});
    prep.configuration.payload = serde_json::from_value(config).unwrap();
    prep.configuration.catalog_binding = Some(ProviderCatalogBinding {
        catalog_id: "synthetic-import".to_owned(),
        catalog_version: "r1".to_owned(),
        catalog_digest: catalog_hash,
        content_object_hash: catalog_hash,
        entry_json_pointer: "/entries/exact-model".to_owned(),
        source: ProviderCatalogSource::UserImport,
        published_at_ms: TimestampMs::new(0),
        valid_until_ms: TimestampMs::new(500_000),
        imported_at_ms: TimestampMs::new(0),
    });
    prep.content_object_hash = publish(&store, &prep.configuration);
    let mut wrong = prep.clone();
    wrong
        .configuration
        .catalog_binding
        .as_mut()
        .unwrap()
        .entry_json_pointer = "/entries/missing".to_owned();
    wrong.content_object_hash = publish(&store, &wrong.configuration);
    assert!(store.commit_command(request(&store, wrong, 800)).is_err());
    assert_eq!(counts(&store), (0, 0, 0, 0));
    store
        .commit_command(request(&store, prep.clone(), 801))
        .unwrap();
    let record = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(record.configuration, prep.configuration);
    assert_eq!(
        store.read_verified_object_bytes(catalog_hash).unwrap(),
        canonicalize_to_vec(&catalog).unwrap()
    );
    let mut conflicting = prep;
    conflicting.configuration.profile_version = 2;
    conflicting
        .configuration
        .catalog_binding
        .as_mut()
        .unwrap()
        .source = ProviderCatalogSource::UserDeclared;
    conflicting.content_object_hash = publish(&store, &conflicting.configuration);
    assert!(
        store
            .commit_command(request(&store, conflicting, 802))
            .is_err()
    );
    assert_eq!(counts(&store), (1, 1, 1, 2));
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            assert!(
                tx.execute(
                    "UPDATE model_catalog_snapshots SET source_kind='ovpt_release'",
                    []
                )
                .is_err()
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_f012_011_inline_unknown_rates_remain_unknown_without_catalogue_or_grant() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let mut prep = prepare(&store, 1, None, "model");
    let mut value = serde_json::to_value(&prep.configuration.payload).unwrap();
    *value.get_mut("pricing").unwrap() = json!({"kind":"user_declared","entry":{"currency":"USD","input_micros_per_million":null,"cached_input_micros_per_million":null,"cache_write_micros_per_million":null,"output_micros_per_million":123,"observed_at_ms":0,"valid_until_ms":500_000,"source_url":null},"stale_policy":"block"});
    prep.configuration.payload = serde_json::from_value(value).unwrap();
    prep.content_object_hash = publish(&store, &prep.configuration);
    store
        .commit_command(request(&store, prep.clone(), 900))
        .unwrap();
    let record = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(record.configuration, prep.configuration);
    assert_eq!(record.qualification, ProviderProfileQualification::Required);
    store
        .read_snapshot(|conn, _| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM model_catalog_snapshots", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
}

fn create_project(store: &UnleashStore, id: ProjectId, command_id: u128) {
    let mut command = request(store, prepare(store, 1, None, "unused"), command_id);
    "CreateProject".clone_into(&mut command.envelope.command_type);
    command.envelope.aggregate_id = id.as_uuid();
    command.envelope.payload.clone_from(
        json!({"name":"fixture-project","weight":1})
            .as_object()
            .unwrap(),
    );
    command.target_aggregate = AggregateKind::Project;
    command.mutation = CommandMutation::CreateProject {
        project_id: id,
        name: "fixture-project".to_owned(),
        weight: 1,
    };
    store.commit_command(command).unwrap();
}

#[test]
fn t_f012_002_project_scope_is_persisted_and_cannot_be_moved_by_a_new_actor() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let project = ProjectId::from_u128(10_000);
    let other = ProjectId::from_u128(10_001);
    create_project(&store, project, 10_000);
    create_project(&store, other, 10_001);
    let first = store
        .commit_command(request(
            &store,
            prepare(&store, 1, Some(project), "v1"),
            10_002,
        ))
        .unwrap();
    let original = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(original.configuration.project_id, Some(project));
    let mut conflict = request(
        &store,
        prepare(&store, 1, Some(project), "other-attempt"),
        10_003,
    );
    conflict.actor_id = ActorId::new("user:other").unwrap();
    conflict.envelope.actor_ref = Some(conflict.actor_id.to_string());
    if let CommandMutation::SetProviderProfile { caller, .. } = &mut conflict.mutation {
        caller.actor_id = conflict.actor_id.clone();
    }
    assert!(matches!(
        store.commit_command(conflict),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert!(
        store
            .commit_command(request(
                &store,
                prepare(&store, 2, Some(other), "v2"),
                10_004
            ))
            .is_err()
    );
    store.read_snapshot(|conn,_| {let event_project:String=conn.query_row("SELECT project_id FROM events WHERE causation_receipt_id=?1 AND event_type='ProviderProfileConfigured'",[first.receipt().receipt_id.to_string()],|row|row.get(0))?;assert_eq!(event_project,project.to_string());Ok(())}).unwrap();
    assert_eq!(
        store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap(),
        original
    );
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

#[test]
fn t_f012_002_same_wire_command_replays_despite_recalculated_server_preparation() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let command = request(&store, prepare(&store, 1, None, "v1"), 11_000);
    let first = store.commit_command(command.clone()).unwrap();
    let mut different = command;
    let binding = secret(&store, None);
    // The wire digest identifies the request; a resolver must not change its receipt.
    if let CommandMutation::SetProviderProfile { preparation, .. } = &mut different.mutation {
        preparation.configuration.secret_binding = Some(binding);
        preparation.content_object_hash = publish(&store, &preparation.configuration);
    }
    assert_eq!(
        store.commit_command(different).unwrap().receipt(),
        first.receipt()
    );
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

#[test]
fn t_f012_002_replay_returns_receipt_when_original_configuration_bytes_are_missing() {
    let directory = local_test_dir("f012-profile-replay-missing-object").unwrap();
    let config = StoreConfig::new(directory.path());
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    let preparation = prepare(&store, 1, None, "v1");
    let hash = preparation.content_object_hash;
    let command = request(&store, preparation, 14_000);
    let first = store.commit_command(command.clone()).unwrap();
    let before = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))?)
        })
        .unwrap();
    let relative = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT relative_path FROM object_records WHERE object_hash=?1",
                [hash.to_string()],
                |r| r.get::<_, String>(0),
            )?)
        })
        .unwrap();
    std::fs::remove_file(config.artifacts_dir.join(relative)).unwrap();
    assert!(store.read_verified_object_bytes(hash).is_err());
    assert_eq!(
        store.commit_command(command.clone()).unwrap().receipt(),
        first.receipt()
    );
    let mut changed = command;
    changed
        .envelope
        .payload
        .insert("model".to_owned(), json!("different-wire"));
    assert!(matches!(
        store.commit_command(changed),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert_eq!(counts(&store), (1, 1, 1, 1));
    let after = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get::<_, i64>(0))?)
        })
        .unwrap();
    assert_eq!(before, after);
}

struct ReplayRaceClock {
    armed: AtomicBool,
    calls: std::sync::atomic::AtomicUsize,
    missing_snapshot_read: Barrier,
    committed: Barrier,
}

impl unleash_store::MonotonicClockPort for ReplayRaceClock {
    fn monotonic_now(&self) -> std::time::Duration {
        // Freeze exactly after the first lookup read, while its result is still None.
        if std::thread::current().name() == Some("profile-replay-race")
            && self.armed.load(Ordering::Acquire)
            && self.calls.fetch_add(1, Ordering::AcqRel) == 1
        {
            self.armed.store(false, Ordering::Release);
            self.missing_snapshot_read.wait();
            self.committed.wait();
        }
        std::time::Duration::ZERO
    }
}

#[test]
fn t_f012_002_failed_object_preflight_rechecks_a_concurrently_committed_receipt() {
    let directory = local_test_dir("f012-profile-concurrent-replay").unwrap();
    let clock = Arc::new(ReplayRaceClock {
        armed: AtomicBool::new(false),
        calls: std::sync::atomic::AtomicUsize::new(0),
        missing_snapshot_read: Barrier::new(2),
        committed: Barrier::new(2),
    });
    let mut config = StoreConfig::new(directory.path());
    config.monotonic_clock = clock.clone();
    let store = Arc::new(UnleashStore::open(config).unwrap());
    store
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    let good = request(&store, prepare(&store, 1, None, "v1"), 15_000);
    let mut unreadable = good.clone();
    if let CommandMutation::SetProviderProfile { preparation, .. } = &mut unreadable.mutation {
        preparation.content_object_hash = Sha256Digest::from_bytes(b"missing-preparation");
    }
    clock.armed.store(true, Ordering::Release);
    let background_store = Arc::clone(&store);
    let background = std::thread::Builder::new()
        .name("profile-replay-race".to_owned())
        .spawn(move || background_store.commit_command(unreadable))
        .unwrap();
    clock.missing_snapshot_read.wait();
    let committed = store.commit_command(good).unwrap();
    clock.committed.wait();
    assert_eq!(
        background.join().unwrap().unwrap().receipt(),
        committed.receipt()
    );
    assert_eq!(counts(&store), (1, 1, 1, 1));
}

#[test]
fn t_f012_002_v011_interrupted_migration_is_atomic_and_legacy_cannot_be_promoted() {
    let directory = local_test_dir("f012-profile-v011-rollback").unwrap();
    let mut old = StoreConfig::new(directory.path());
    old.max_supported_schema_version = 10;
    let store = UnleashStore::open(old.clone()).unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_| {tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms) VALUES('legacy','provider','local_only','none_local','user_declared',17)",[])?;Ok(())}).unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("pre-migration"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let mut next = StoreConfig::new(directory.path());
    next.cutpoints = Arc::new(FailOnce {
        point: StoreCutpoint::C11MigrationInProgress,
        armed: AtomicBool::new(true),
    });
    assert!(UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).is_err());
    let historical = UnleashStore::open(old).unwrap();
    assert_eq!(historical.daemon_state().unwrap().schema_version, 10);
    historical.read_snapshot(|conn,_| {assert!(conn.prepare("SELECT profile_id FROM provider_profiles").is_err());let count:i64=conn.query_row("SELECT COUNT(*) FROM provider_profiles WHERE profile_version_ref='legacy' AND created_at_ms=17",[],|r|r.get(0))?;assert_eq!(count,1);Ok(())}).unwrap();
    historical.shutdown().unwrap();
    let current = UnleashStore::open_with_migration_backup(
        StoreConfig::new(directory.path()),
        Some(backup.backup_id),
    )
    .unwrap();
    current.execute_in_writer_tx(WriteCriticality::Critical,|tx,_| {assert!(tx.execute("UPDATE provider_profiles SET profile_id=?1 WHERE profile_version_ref='legacy'",[PROFILE.to_string()]).is_err());assert!(tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms,profile_id) VALUES('partial','provider','local_only','none_local','user_declared',0,?1)",[PROFILE.to_string()]).is_err());Ok(())}).unwrap();
}

#[test]
fn t_f012_002_v011_preserves_legacy_profile_and_probe_links() {
    let directory = local_test_dir("f012-profile-v011").unwrap();
    let mut old = StoreConfig::new(directory.path());
    old.max_supported_schema_version = 10;
    let store = UnleashStore::open(old).unwrap();
    store
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
        tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,price_valid_until_ms,source_kind,pricing_stale_policy,created_at_ms) VALUES('historical opaque ref','old-provider','local_only','none_local',NULL,'user_declared','warn',17)", [])?;
        Ok(())
    }).unwrap();
    store
        .commit_command(probe_command(
            &store,
            "historical opaque ref",
            "historical-id",
            "old-model",
            700,
        ))
        .unwrap();
    let context_before = store
        .inspect_provider_probe(unleash_domain::ProviderProbeId::from_u128(700))
        .unwrap()
        .unwrap();
    let before = store
        .read_snapshot(|conn, _| {
            let mut statement = conn.prepare("SELECT * FROM provider_profiles")?;
            let count = statement.column_count();
            Ok(statement.query_row([], |row| {
                (0..count)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })?)
        })
        .unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("pre-v011"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let store = UnleashStore::open_with_migration_backup(
        StoreConfig::new(directory.path()),
        Some(backup.backup_id),
    )
    .unwrap();
    assert_eq!(store.daemon_state().unwrap().schema_version, 11);
    assert_eq!(
        store
            .inspect_provider_probe(unleash_domain::ProviderProbeId::from_u128(700))
            .unwrap()
            .unwrap(),
        context_before
    );
    store.read_snapshot(|conn, _| {
        let after = conn.query_row("SELECT profile_version_ref,provider_id,locality,pricing_version,price_valid_until_ms,source_kind,pricing_stale_policy,created_at_ms FROM provider_profiles", [], |row| (0..8).map(|i| row.get::<_,rusqlite::types::Value>(i)).collect::<Result<Vec<_>,_>>())?;
        assert_eq!(after, before);
        let identity: (Option<String>,Option<i64>) = conn.query_row("SELECT profile_id,profile_version FROM provider_profiles", [], |row| Ok((row.get(0)?,row.get(1)?)))?;
        assert_eq!(identity, (None,None));
        assert!(!conn.prepare("PRAGMA foreign_key_check")?.exists([])?);
        Ok(())
    }).unwrap();
}
