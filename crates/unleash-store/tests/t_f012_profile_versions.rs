//! T-F012-002/011/013 : configuration immuable ; banc `SQLite` sans réseau/coffre.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et assertions de contrat"
)]

use serde_json::{Value, json};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
use unleash_domain::{
    ActorId, CommandId, ProjectId, RequestId, Revision, SecretRef, Sha256Digest, TaskId,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ReasonCode, ReceiptState, SetProviderProfilePayload,
    canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CutpointObserver, NoopCutpointObserver, PrepareObjectRequest, ProviderProfileConfiguration,
    ProviderProfilePreparation, ProviderProfileQualification, ProviderSecretBinding,
    PublishObjectReferenceRequest, SecretReferenceInput, StoreCutpoint, StoreError,
};
use unleash_store::{
    BackupRetentionTier, PrepareBackupRequest, PreparedAgentCreation, SettingsPatch,
    SettingsScopeKind, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROFILE: Uuid = Uuid::from_u128(0xf012);

#[path = "support/provider_probe_generation_placement.rs"]
mod generation_placement;

#[path = "support/workflow_run.rs"]
mod workflow_run;

fn ready_assignment(
    config: StoreConfig,
) -> (workflow_run::Fixture, CommandEnvelope, CallerAuthority) {
    let (fixture, create, caller) = workflow_run::prepare(
        config,
        actor(),
        unleash_domain::AgentLevel::N,
        |_, scope, project| {
            *scope.pointer_mut("/scopes/implementation").unwrap() =
                json!({"scope_id":project,"scope_revision":1});
        },
    );
    fixture
        .store
        .create_prepared_workflow_run(create, caller.clone())
        .unwrap();
    fixture
        .store
        .start_prepared_workflow_run(
            fixture.envelope(
                40,
                workflow_run::RUN.as_uuid(),
                Revision::new(1),
                "StartRun",
            ),
            caller.clone(),
        )
        .unwrap();
    let task: Uuid = fixture
        .store
        .read_snapshot(|db, _| {
            let id: String = db.query_row(
                "SELECT task_id FROM tasks WHERE run_id=?1",
                [workflow_run::RUN.to_string()],
                |row| row.get(0),
            )?;
            Ok(id.parse().unwrap())
        })
        .unwrap();
    let agent = Uuid::from_u128(0xf013);
    let mut create_agent = fixture.envelope(41, agent, Revision::INITIAL, "CreateAgent");
    create_agent.payload = serde_json::from_value(json!({
        "mission_id": fixture.mission, "mission_revision": 1, "task_id": task, "task_revision": 1,
    }))
    .unwrap();
    fixture
        .store
        .create_agent_from_task(create_agent, caller.clone())
        .unwrap();
    let mut assign = fixture.envelope(42, task, Revision::new(1), "AssignTask");
    assign.actor_ref = Some(caller.actor_id.to_string());
    assign.payload.insert("agent_id".to_owned(), json!(agent));
    (fixture, assign, caller)
}

fn set_skill_preference(
    fixture: &workflow_run::Fixture,
    caller: &CallerAuthority,
    id: u128,
    config_project: Option<ProjectId>,
    expected: Revision,
    profile: Option<(u64, Sha256Digest)>,
) -> unleash_protocol::CommandReceipt {
    let (kind, scope_id, aggregate, config_scope) = config_project.map_or_else(
        || {
            (
                SettingsScopeKind::User,
                "default".to_owned(),
                fixture
                    .store
                    .active_authority_generation()
                    .unwrap()
                    .as_uuid(),
                json!({"kind":"global","owner_id":null}),
            )
        },
        |project| {
            (
                SettingsScopeKind::Project,
                project.to_string(),
                project.as_uuid(),
                json!({"kind":"project","owner_id":project}),
            )
        },
    );
    let mut envelope = fixture.envelope(id, aggregate, expected, "SetSettings");
    let mut binding = json!({
        "operation": if profile.is_some() {"upsert"} else {"remove"},
        "configuration_scope": config_scope,
        "skill_ref": {"skill_id":"skill.fixture","skill_version":1,
            "skill_scope":{"kind":"project","owner_id":envelope.project_id}},
    });
    if let Some((version, digest)) = profile {
        binding.as_object_mut().unwrap().insert(
            "preferred_profile_ref".to_owned(),
            json!({"profile_id":PROFILE,"profile_version":version,"digest":digest}),
        );
    }
    envelope.project_id = config_project;
    envelope.actor_ref = Some(caller.actor_id.to_string());
    envelope.payload = serde_json::from_value(json!({
        "scope_kind":kind,"scope_id":scope_id,"patch":{"model_selection_binding":binding}
    }))
    .unwrap();
    fixture
        .store
        .commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: AggregateKind::Settings,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSettingsWithModelSelectionBinding {
                scope_kind: kind,
                scope_id,
                caller: caller.clone(),
                patch: SettingsPatch::default(),
                binding: Box::new(serde_json::from_value(binding).unwrap()),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap()
        .receipt()
        .clone()
}

// T-F012-002 / T-F003-002 — An assigned role supplies the skill and Settings supplies the preference.
#[test]
fn t_f012_002_resolves_the_preference_of_the_assigned_primary_skill() {
    let directory = local_test_dir("f012-primary-skill-selection").unwrap();
    let (fixture, assign, caller) = ready_assignment(StoreConfig::new(directory.path()));
    let resolve = || {
        fixture
            .store
            .resolve_task_skill_model_binding(&assign, &caller)
    };
    assert_eq!(resolve().unwrap(), None);
    let first = prepare(&fixture.store, 1, None, "user-preference");
    let first_digest = first.content_object_hash;
    fixture
        .store
        .commit_command(request(&fixture.store, first, 43))
        .unwrap();
    let user = set_skill_preference(
        &fixture,
        &caller,
        45,
        None,
        Revision::INITIAL,
        Some((1, first_digest)),
    );
    let second = prepare(&fixture.store, 2, None, "project-preference");
    let second_digest = second.content_object_hash;
    fixture
        .store
        .commit_command(request(&fixture.store, second, 44))
        .unwrap();
    let project = set_skill_preference(
        &fixture,
        &caller,
        46,
        assign.project_id,
        Revision::INITIAL,
        Some((2, second_digest)),
    );
    let binding: Value = serde_json::to_value(resolve().unwrap().unwrap()).unwrap();
    assert_eq!(
        binding.get("command_receipt_id"),
        Some(&json!(project.receipt_id)),
    );
    assert_eq!(
        binding.get("preferred_profile_ref"),
        Some(&json!({"profile_id":PROFILE,"profile_version":2,"digest":second_digest})),
    );

    fixture
        .store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute(
            "UPDATE object_references SET state='tombstone', tombstone_reason='fixture-retirement'
              WHERE owner_kind='provider_profile' AND object_hash=?1",
            [second_digest.to_string()],
        )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        resolve(),
        Err(StoreError::ContractViolation {
            reason: Some(ReasonCode::ModelConfigurationConflict),
            ..
        })
    ));

    set_skill_preference(
        &fixture,
        &caller,
        47,
        assign.project_id,
        Revision::new(1),
        None,
    );
    let fallback: Value = serde_json::to_value(resolve().unwrap().unwrap()).unwrap();
    assert_eq!(
        fallback.get("command_receipt_id"),
        Some(&json!(user.receipt_id)),
    );
    assert_eq!(
        fallback.get("preferred_profile_ref"),
        Some(&json!({"profile_id":PROFILE,"profile_version":1,"digest":first_digest})),
    );

    let mut other = assign.clone();
    let outsider = CallerAuthority {
        actor_id: ActorId::new("user:other").unwrap(),
        role: CallerRole::HumanOperator,
    };
    other.actor_ref = Some(outsider.actor_id.to_string());
    assert!(matches!(
        fixture
            .store
            .resolve_task_skill_model_binding(&other, &outsider),
        Err(StoreError::NotFound(_))
    ));
    fixture.store.shutdown().unwrap();
}

// T-F012-002 — A role retained for another slot may have the same project scope.
#[test]
fn t_f012_002_refuses_an_assignment_with_a_role_from_another_slot() {
    let directory = local_test_dir("f012-primary-skill-other-role").unwrap();
    let (fixture, mut assign, caller) = ready_assignment(StoreConfig::new(directory.path()));
    let project = assign.project_id.unwrap();
    let scope = json!({"kind":"project","owner_id":project});
    let skill = json!({"id":"skill.fixture","version":1,"scope":scope});
    let document = json!({
        "schema_version":"unleash.role.v1", "role_id":"role.other", "version":1,
        "level":"N", "scope":scope, "entry_skill":skill, "skills":[skill],
        "contracts":[{"id":"contract.fixture","version":1,"scope":scope}],
    });
    workflow_run::publish_fixture_version(
        &fixture,
        caller.actor_id.as_str(),
        "role",
        "role.other",
        "role-other-v1",
        &document,
    );
    let agent = Uuid::from_u128(0xf014);
    let prepared = PreparedAgentCreation {
        daemon_epoch: fixture.store.daemon_state().unwrap().daemon_epoch,
        assignment_id: Uuid::from_u128(0xf015),
        mission_id: fixture.mission,
        mission_revision: Revision::new(1),
        task_id: TaskId::from_uuid(assign.aggregate_id),
        task_revision: assign.expected_revision,
        scope_id: project.to_string(),
        scope_revision: Revision::new(1),
        role_version_id: "role-other-v1".to_owned(),
        role_content_hash: canonical_sha256(&document).unwrap(),
    };
    let mut create = fixture.envelope(48, agent, Revision::INITIAL, "CreateAgent");
    create.payload = serde_json::from_value(serde_json::to_value(prepared).unwrap()).unwrap();
    fixture
        .store
        .create_assigned_agent(create, caller.clone())
        .unwrap();
    assign.payload.insert("agent_id".to_owned(), json!(agent));
    assert!(matches!(
        fixture
            .store
            .resolve_task_skill_model_binding(&assign, &caller),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    fixture.store.shutdown().unwrap();
}

// T-F012-002 — Deprecation preserves a pinned skill, while revocation invalidates it.
#[test]
fn t_f012_002_keeps_the_preference_for_a_deprecated_pinned_skill() {
    let directory = local_test_dir("f012-primary-skill-deprecated").unwrap();
    let (fixture, assign, caller) = ready_assignment(StoreConfig::new(directory.path()));
    let preference = prepare(&fixture.store, 1, None, "pinned-skill-preference");
    let digest = preference.content_object_hash;
    fixture
        .store
        .commit_command(request(&fixture.store, preference, 43))
        .unwrap();
    set_skill_preference(
        &fixture,
        &caller,
        44,
        assign.project_id,
        Revision::INITIAL,
        Some((1, digest)),
    );
    let resolve = || {
        fixture
            .store
            .resolve_task_skill_model_binding(&assign, &caller)
    };
    let before = resolve().unwrap().unwrap();

    let set_state = |state: &'static str| {
        fixture.store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute("UPDATE registry_versions SET publication_state=?1 WHERE registry_version_id='skill-v1'",[state])?;
            Ok(())
        }).unwrap();
    };
    set_state("deprecated");
    assert_eq!(resolve().unwrap(), Some(before));
    set_state("revoked");
    assert!(matches!(
        resolve(),
        Err(StoreError::ContractViolation {
            reason: Some(ReasonCode::ModelConfigurationConflict),
            ..
        })
    ));
    fixture.store.shutdown().unwrap();
}

#[test]
fn t_f012_002_authenticated_command_lookup_needs_no_preparation() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let command = request(&store, prepare(&store, 1, None, "v1"), 17_000);
    let caller = CallerAuthority {
        actor_id: actor(),
        role: CallerRole::HumanOperator,
    };
    assert!(
        store
            .lookup_provider_profile_command(&command.envelope, &caller)
            .unwrap()
            .is_none()
    );
    let first = store.commit_command(command.clone()).unwrap();
    assert_eq!(
        store
            .lookup_provider_profile_command(&command.envelope, &caller)
            .unwrap()
            .unwrap(),
        *first.receipt()
    );
    let mut changed = command.envelope.clone();
    changed
        .payload
        .insert("model".to_owned(), json!("different-wire"));
    assert!(matches!(
        store.lookup_provider_profile_command(&changed, &caller),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    let mut stale = command.envelope.clone();
    stale.authority_generation = unleash_domain::AuthorityGeneration::from_u128(1);
    assert!(matches!(
        store.lookup_provider_profile_command(&stale, &caller),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let mut denied = caller.clone();
    denied.role = CallerRole::Agent;
    assert!(
        store
            .lookup_provider_profile_command(&command.envelope, &denied)
            .is_err()
    );
    let other = CallerAuthority {
        actor_id: ActorId::new("user:other").unwrap(),
        role: CallerRole::HumanOperator,
    };
    let mut another_actor = command.envelope.clone();
    another_actor.actor_ref = Some(other.actor_id.to_string());
    assert!(
        store
            .lookup_provider_profile_command(&another_actor, &other)
            .unwrap()
            .is_none()
    );
    // Explicit retention fixture: classification must retain the canonical tombstone error.
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,_| {tx.execute("UPDATE command_receipts SET response_state='tombstone',response_json=NULL WHERE command_id=?1",[command.envelope.command_id.to_string()])?;Ok(())}).unwrap();
    assert!(matches!(
        store.lookup_provider_profile_command(&another_actor, &caller),
        Err(StoreError::InvalidArgument { .. })
    ));
    another_actor.actor_ref = Some(actor().to_string());
    assert!(matches!(
        store.lookup_provider_profile_command(&another_actor, &caller),
        Err(StoreError::IdempotencyWindowExpired { .. })
    ));
}

#[test]
fn t_f012_002_retiring_one_candidate_preserves_committed_profile_and_other_attempt() {
    let (store, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let preparation = prepare(&store, 1, None, "v1");
    let hash = preparation.content_object_hash;
    store
        .commit_command(request(&store, preparation, 18_000))
        .unwrap();
    let original = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let bytes = store.read_verified_object_bytes(hash).unwrap();
    let stage = |owner: &str| {
        let handle = store
            .prepare_object(PrepareObjectRequest {
                object_hash: hash,
                byte_length: bytes.len() as u64,
                media_type: "application/json".to_owned(),
                owner_kind: "provider_profile_preparation".to_owned(),
                owner_id: owner.to_owned(),
            })
            .unwrap();
        store
            .publish_owned_object_reference(PublishObjectReferenceRequest {
                pin_id: handle.pin_id,
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: "provider_profile_preparation".to_owned(),
                owner_id: owner.to_owned(),
                source_field: "configuration".to_owned(),
            })
            .unwrap()
    };
    let first = stage("attempt-a");
    let second = stage("attempt-b");
    assert_eq!(
        store
            .retire_owned_object_reference(first.clone(), "candidate-finished")
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .retire_owned_object_reference(first, "candidate-finished")
            .unwrap(),
        0
    );
    store.read_snapshot(|conn,_| {
        let business:i64=conn.query_row("SELECT COUNT(*) FROM object_references WHERE object_hash=?1 AND owner_kind='provider_profile' AND state='live'",[hash.to_string()],|row|row.get(0))?;
        let other:i64=conn.query_row("SELECT COUNT(*) FROM object_references WHERE object_hash=?1 AND owner_kind='provider_profile_preparation' AND owner_id='attempt-b' AND state='live'",[hash.to_string()],|row|row.get(0))?;
        assert_eq!((business,other),(1,1));Ok(())
    }).unwrap();
    store
        .retire_owned_object_reference(second, "candidate-finished")
        .unwrap();
    assert_eq!(store.read_verified_object_bytes(hash).unwrap(), bytes);
    assert_eq!(
        store
            .resolve_provider_profile_snapshot(PROFILE, 1)
            .unwrap()
            .value
            .unwrap(),
        original
    );
}

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

fn generation_permission(
    store: &UnleashStore,
    caller: &CallerAuthority,
) -> unleash_domain::ReceiptId {
    use unleash_domain::{DataLocality, MicrosAmount, TimestampMs};
    use unleash_protocol::{ProviderTestLimits, ProviderTestPhase};
    use unleash_store::{ProviderProbeMandateChange, ProviderProbeMandateIssue};
    let grant = ProviderProbeMandateChange::Issue(ProviderProbeMandateIssue {
        principal_id: ActorId::new("profile-user").unwrap(),
        profile_id: PROFILE,
        profile_version: 1,
        phase: ProviderTestPhase::GenerationProbe,
        locality: DataLocality::LocalOnly,
        capability_set: vec!["text".to_owned()],
        limits: ProviderTestLimits::new(1000, 1, 1, MicrosAmount::ZERO),
        expires_at_ms: TimestampMs::new(store.config().clock.now_ms().as_i64() + 30_000),
    });
    generation_policy(store, caller, grant, 604, Revision::INITIAL)
}

fn generation_policy(
    store: &UnleashStore,
    caller: &CallerAuthority,
    grant: unleash_store::ProviderProbeMandateChange,
    id: u128,
    expected: Revision,
) -> unleash_domain::ReceiptId {
    use unleash_store::{PolicyIsolationMode, PolicyScopeKind};
    let authority = store.active_authority_generation().unwrap();
    store
        .commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(id),
                command_id: CommandId::from_u128(id),
                authority_generation: authority,
                actor_ref: Some(caller.actor_id.to_string()),
                project_id: None,
                aggregate_id: authority.as_uuid(),
                expected_revision: expected,
                command_type: "SetPolicy".to_owned(),
                payload:
                    json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard",
                "policy_version":"generation-profile-test","reason":"synthetic profile permission",
                "provider_probe_mandate":grant})
                    .as_object()
                    .unwrap()
                    .clone(),
            },
            target_aggregate: AggregateKind::SecurityPolicy,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetPolicyWithProviderProbeMandate {
                scope_kind: PolicyScopeKind::Global,
                scope_id: "global".to_owned(),
                isolation_mode: PolicyIsolationMode::Standard,
                policy_version: "generation-profile-test".to_owned(),
                reason: "synthetic profile permission".to_owned(),
                caller: caller.clone(),
                mandate: grant,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap()
        .receipt()
        .receipt_id
}

struct PauseGenerationBeforeCommit {
    armed: AtomicBool,
    entered: mpsc::SyncSender<()>,
    proceed: Mutex<mpsc::Receiver<()>>,
}

impl CutpointObserver for PauseGenerationBeforeCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C0BeforeCommandTx && self.armed.swap(false, Ordering::SeqCst) {
            let error = || StoreError::CutpointInjected {
                cutpoint: "C0".to_owned(),
                message: "generation test synchronization".to_owned(),
            };
            self.entered.send(()).map_err(|_| error())?;
            self.proceed
                .lock()
                .map_err(|_| error())?
                .recv_timeout(Duration::from_secs(8))
                .map_err(|_| error())?;
        }
        Ok(())
    }
}

#[test]
fn t_f012_generation_revocation_between_artifact_publication_and_commit() {
    let (arrived, wait) = mpsc::sync_channel(0);
    let (proceed, resume) = mpsc::sync_channel(1);
    let fault = Arc::new(PauseGenerationBeforeCommit {
        armed: AtomicBool::new(false),
        entered: arrived,
        proceed: Mutex::new(resume),
    });
    let (store, _directory) = fixture(fault.clone());
    store
        .commit_command(request(
            &store,
            prepare(&store, 1, None, "exact-model"),
            605,
        ))
        .unwrap();
    let caller = CallerAuthority {
        actor_id: actor(),
        role: CallerRole::HumanOperator,
    };
    let mandate = generation_permission(&store, &caller);
    let profile = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let mut envelope = probe_command(
        &store,
        &profile.profile_version_ref,
        &PROFILE.to_string(),
        "exact-model",
        606,
    )
    .envelope;
    envelope
        .payload
        .insert("mandate_ref".to_owned(), json!(mandate));
    fault.armed.store(true, Ordering::SeqCst);
    std::thread::scope(|scope| {
        let in_flight =
            scope.spawn(|| store.commit_provider_generation_preparation(&envelope, &caller));
        wait.recv_timeout(Duration::from_secs(8)).unwrap();
        let candidates = store.read_snapshot(|conn, _| Ok(conn.query_row(
            "SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_generation_preparation' AND state='live'",
            [], |row| row.get::<_,i64>(0),
        )?)).unwrap();
        assert_eq!(candidates, 5);
        generation_policy(
            &store,
            &caller,
            unleash_store::ProviderProbeMandateChange::Revoke {
                mandate_ref: mandate,
            },
            607,
            Revision::new(1),
        );
        proceed.send(()).unwrap();
        assert!(matches!(
            in_flight.join().unwrap(),
            Err(StoreError::NotFound(_))
        ));
    });
    assert!(
        store
            .lookup_provider_validation_command(&envelope, &caller)
            .unwrap()
            .is_none()
    );
    let residues = store.read_snapshot(|conn, _| Ok(conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM provider_probe_contexts) +
            (SELECT COUNT(*) FROM object_references WHERE owner_kind IN ('provider_generation_preparation','provider_probe') AND state='live') +
            (SELECT COUNT(*) FROM action_dispatches)",
        [], |row| row.get::<_,i64>(0),
    )?)).unwrap();
    assert_eq!(residues, 0);
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
    let raw = probe_command(
        &store,
        &record.profile_version_ref,
        &PROFILE.to_string(),
        "exact-model",
        603,
    );
    assert!(matches!(
        store.commit_command(raw.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let caller = CallerAuthority {
        actor_id: actor(),
        role: CallerRole::HumanOperator,
    };
    let mandate = generation_permission(&store, &caller);
    let mut envelope = raw.envelope;
    envelope
        .payload
        .insert("mandate_ref".to_owned(), json!(mandate));
    store
        .commit_provider_generation_preparation(&envelope, &caller)
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
fn t_f012_002_restore_preserves_historical_command_key_without_reactivating_receipt() {
    let source_directory = local_test_dir("f012-profile-source-lineage").unwrap();
    let source_config = StoreConfig::new(source_directory.path());
    let source = UnleashStore::open(source_config.clone()).unwrap();
    source
        .register_principal("profile-user", "user", "user:profile")
        .unwrap();
    // The configured version is newer than the backup. Restore retains its receipt,
    // not a fabricated reconstruction of that configuration/object in the snapshot.
    let backup_dir = source_directory.path().join("before-profile");
    source
        .prepare_backup(PrepareBackupRequest {
            destination_dir: backup_dir.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    let command = request(
        &source,
        prepare(&source, 1, None, "post-snapshot-profile"),
        16_000,
    );
    let first = source.commit_command(command.clone()).unwrap();
    let active_before = counts(&source);
    assert_eq!(
        source.commit_command(command.clone()).unwrap().receipt(),
        first.receipt()
    );
    assert_eq!(counts(&source), active_before);
    let source_authority = source.active_authority_generation().unwrap();
    let target_directory = local_test_dir("f012-profile-restored-lineage").unwrap();
    let target_config = StoreConfig::new(target_directory.path());
    UnleashStore::restore_backup(
        unleash_store::RestoreBackupRequest {
            backup_dir,
            target_data_root: target_directory.path().to_path_buf(),
            command_id: CommandId::from_u128(16_001),
            actor_id: actor().to_string(),
            command_digest: unleash_protocol::canonical_sha256(&"explicit synthetic restore")
                .unwrap(),
            source_authority_generation: source_authority,
            source_db_unreadable: false,
            source_db_path: Some(source_config.db_path.clone()),
            source_artifacts_dir: Some(source_config.artifacts_dir.clone()),
            neutralizer: Arc::new(unleash_store::EmptyExecutorNeutralizer),
        },
        target_config.clone(),
    )
    .unwrap();
    source.shutdown().unwrap();
    let restored = UnleashStore::open(target_config).unwrap();
    assert_ne!(
        restored.active_authority_generation().unwrap(),
        source_authority
    );
    restored.read_snapshot(|conn,_| {
        let (generation,json):(String,String)=conn.query_row("SELECT authority_generation,response_json FROM command_receipts WHERE actor_id=?1 AND command_id=?2",rusqlite::params![actor().as_str(),command.envelope.command_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
        assert_eq!(generation,source_authority.to_string());
        assert_eq!(serde_json::from_str::<unleash_protocol::CommandReceipt>(&json).unwrap(),*first.receipt());
        Ok(())
    }).unwrap();
    let before = counts(&restored);
    assert_eq!(before.0, 0);
    assert!(matches!(
        restored.commit_command(command.clone()),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let mut active_wire = command;
    active_wire.envelope.authority_generation = restored.active_authority_generation().unwrap();
    active_wire.envelope.request_id = RequestId::from_u128(16_002);
    let historical_error = restored.commit_command(active_wire).unwrap_err();
    assert!(
        matches!(historical_error, StoreError::IdempotencyConflict { .. }),
        "active lineage must classify the occupied historical PK before reading its missing object: {historical_error:?}"
    );
    assert_eq!(counts(&restored), before);
    assert_eq!(
        restored.daemon_state().unwrap().mode,
        unleash_domain::DaemonMode::ReadOnlyRecovery
    );
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
    next.max_supported_schema_version = 11;
    let mut interrupted = next.clone();
    interrupted.cutpoints = Arc::new(FailOnce {
        point: StoreCutpoint::C11MigrationInProgress,
        armed: AtomicBool::new(true),
    });
    assert!(UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)).is_err());
    let historical = UnleashStore::open(old).unwrap();
    assert_eq!(historical.daemon_state().unwrap().schema_version, 10);
    historical.read_snapshot(|conn,_| {assert!(conn.prepare("SELECT profile_id FROM provider_profiles").is_err());let count:i64=conn.query_row("SELECT COUNT(*) FROM provider_profiles WHERE profile_version_ref='legacy' AND created_at_ms=17",[],|r|r.get(0))?;assert_eq!(count,1);Ok(())}).unwrap();
    historical.shutdown().unwrap();
    let current = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(current.daemon_state().unwrap().schema_version, 11);
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
    let mut next = StoreConfig::new(directory.path());
    next.max_supported_schema_version = 11;
    let store = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
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
