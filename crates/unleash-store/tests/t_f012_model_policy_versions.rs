//! T-F012-002 / AC-F012-002 / REQ-MODEL-002 : politique de modèle versionnée par les réglages.
//!
//! Banc `SQLite` local, sans réseau ni coffre : le vrai writer publie et relit l'historique du
//! scope. La forme attendue vient du contrat IPC, jamais du code sous test.
#![expect(clippy::unwrap_used, reason = "Fixtures locales et assertions de contrat")]

use std::path::Path;

use serde_json::{Value, json};
use unleash_domain::{ActorId, CommandId, ProjectId, ReceiptId, RequestId, Revision, Sha256Digest};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ProtocolVersion, ReceiptState, SetProviderProfilePayload,
    canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    ModelPolicyVersion, PrepareObjectRequest, ProviderProfileConfiguration,
    ProviderProfilePreparation, PublishObjectReferenceRequest, SettingsPatch, SettingsScopeKind,
    StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROFILE: Uuid = Uuid::from_u128(0xf012_0001);
const PROJECT: ProjectId = ProjectId::from_u128(0xf012_0002);
const POLICY: &str = "policy.fixture";

struct Profile {
    id: Uuid,
    version: u64,
    digest: Sha256Digest,
}

impl Profile {
    fn reference(&self) -> Value {
        json!({"profile_id": self.id, "profile_version": self.version, "digest": self.digest})
    }
}

struct Scope {
    kind: SettingsScopeKind,
    id: String,
    project: Option<ProjectId>,
}

impl Scope {
    fn project(project: ProjectId) -> Self {
        Self {
            kind: SettingsScopeKind::Project,
            id: project.to_string(),
            project: Some(project),
        }
    }

    fn user() -> Self {
        Self {
            kind: SettingsScopeKind::User,
            id: "default".to_owned(),
            project: None,
        }
    }

    fn aggregate(&self, store: &UnleashStore) -> Uuid {
        self.project.map_or_else(
            || store.active_authority_generation().unwrap().as_uuid(),
            |project| project.as_uuid(),
        )
    }
}

fn actor() -> ActorId {
    ActorId::new("user:profile").unwrap()
}

fn caller(role: CallerRole) -> CallerAuthority {
    CallerAuthority {
        actor_id: actor(),
        role,
    }
}

fn envelope(
    store: &UnleashStore,
    command: u128,
    scope: &Scope,
    aggregate: Uuid,
    expected: Revision,
    command_type: &str,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(command),
        command_id: CommandId::from_u128(command),
        authority_generation: store.active_authority_generation().unwrap(),
        actor_ref: Some(actor().to_string()),
        project_id: scope.project,
        aggregate_id: aggregate,
        expected_revision: expected,
        command_type: command_type.to_owned(),
        payload: payload.as_object().unwrap().clone(),
    }
}

fn request(
    caller: &CallerAuthority,
    envelope: CommandEnvelope,
    aggregate: AggregateKind,
    state: ReceiptState,
    mutation: CommandMutation,
) -> CommitCommandRequest {
    CommitCommandRequest {
        actor_id: caller.actor_id.clone(),
        envelope,
        target_aggregate: aggregate,
        operation_id: None,
        receipt_state: state,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn retain(store: &UnleashStore, value: &impl serde::Serialize) -> Sha256Digest {
    let bytes = canonicalize_to_vec(value).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
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

fn configure_profile(store: &UnleashStore, scope: &Scope, version: u64, command: u128) -> Profile {
    let payload: SetProviderProfilePayload = serde_json::from_value(json!({
        "provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},
        "endpoint":"http://127.0.0.1:1234/v1","model":"policy-fixture","locality":"local_only",
        "server_management":"user_managed","auth":{"kind":"none"},
        "limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,
            "load_timeout_ms":180_000,"first_token_timeout_ms":60_000,
            "generation_timeout_ms":300_000},
        "declared_capabilities":["text"],"pricing":{"kind":"none_local"}
    }))
    .unwrap();
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: PROFILE,
        profile_version: version,
        project_id: scope.project,
        payload,
        secret_binding: None,
        catalog_binding: None,
    };
    let digest = retain(store, &configuration);
    let envelope = envelope(
        store,
        command,
        scope,
        PROFILE,
        Revision::new(version - 1),
        "SetProviderProfile",
        serde_json::to_value(&configuration.payload).unwrap(),
    );
    store
        .commit_command(request(
            &caller(CallerRole::HumanOperator),
            envelope,
            AggregateKind::ProviderProfile,
            ReceiptState::Completed,
            CommandMutation::SetProviderProfile {
                caller: caller(CallerRole::HumanOperator),
                preparation: Box::new(ProviderProfilePreparation {
                    configuration,
                    content_object_hash: digest,
                }),
            },
        ))
        .unwrap();
    Profile {
        id: PROFILE,
        version,
        digest,
    }
}

fn ready(root: &Path) -> (UnleashStore, Profile) {
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    store
        .register_principal("profile-user", "user", actor().as_str())
        .unwrap();
    let scope = Scope::project(PROJECT);
    let envelope = envelope(
        &store,
        0xf012_0003,
        &scope,
        PROJECT.as_uuid(),
        Revision::INITIAL,
        "CreateProject",
        json!({"name":"fixture-project","weight":1}),
    );
    store
        .commit_command(request(
            &caller(CallerRole::HumanOperator),
            envelope,
            AggregateKind::Project,
            ReceiptState::Completed,
            CommandMutation::CreateProject {
                project_id: PROJECT,
                name: "fixture-project".to_owned(),
                weight: 1,
            },
        ))
        .unwrap();
    let profile = configure_profile(&store, &scope, 1, 0xf012_0004);
    (store, profile)
}

fn limits() -> Value {
    json!({"max_llm_calls":1,"max_output_tokens":128,"max_cost_microusd":10000,
        "max_duration_seconds":60})
}

fn publish_patch(profiles: &[Value], fallbacks: &[Value]) -> Value {
    json!({"operation":"publish","policy_id":POLICY,"allowed_profiles":profiles,
        "allowed_capabilities":["text"],"fallback_profiles":fallbacks,"limits":limits()})
}

fn commit(
    store: &UnleashStore,
    caller: &CallerAuthority,
    scope: &Scope,
    command: u128,
    expected: Revision,
    patch: Value,
    mutation: CommandMutation,
) -> Result<CommandReceipt, StoreError> {
    let envelope = envelope(
        store,
        command,
        scope,
        scope.aggregate(store),
        expected,
        "SetSettings",
        json!({"scope_kind":scope.kind,"scope_id":scope.id,"patch":patch}),
    );
    store
        .commit_command(request(
            caller,
            envelope,
            AggregateKind::Settings,
            ReceiptState::Accepted,
            mutation,
        ))
        .map(|outcome| outcome.receipt().clone())
}

fn publish(
    store: &UnleashStore,
    caller: &CallerAuthority,
    scope: &Scope,
    command: u128,
    expected: Revision,
    policy: Value,
) -> Result<CommandReceipt, StoreError> {
    commit(
        store,
        caller,
        scope,
        command,
        expected,
        json!({"model_policy_version":policy.clone()}),
        CommandMutation::SetSettingsWithModelConfiguration {
            scope_kind: scope.kind,
            scope_id: scope.id.clone(),
            caller: caller.clone(),
            patch: SettingsPatch::default(),
            binding: None,
            policy: Some(Box::new(serde_json::from_value(policy).unwrap())),
        },
    )
}

fn history(store: &UnleashStore, scope: &Scope) -> Vec<ModelPolicyVersion> {
    store
        .read_settings(scope.kind, &scope.id)
        .unwrap()
        .model_policy_versions
}

/// Version attendue, écrite depuis le contrat ; le digest couvre le document de publication sans
/// sa référence ni sa révocation.
#[derive(Debug, PartialEq, Eq)]
struct Expected {
    version: ModelPolicyVersion,
    reference: String,
}

fn expected(profiles: &[Value], fallbacks: &[Value], receipt: &ReceiptId, revocation: Value) -> Expected {
    let digest = canonical_sha256(&json!({
        "policy_id":POLICY,"version":1,"allowed_profiles":profiles,
        "allowed_capabilities":["text"],"fallback_profiles":fallbacks,"limits":limits(),
        "principal_id":actor(),"command_receipt_id":receipt
    }))
    .unwrap();
    let reference = format!("model-policy:{POLICY}:1:{digest}");
    let version = serde_json::from_value(json!({
        "policy_id":POLICY,"version":1,"digest":digest,"policy_version_ref":reference,
        "allowed_profiles":profiles,"allowed_capabilities":["text"],"fallback_profiles":fallbacks,
        "limits":limits(),"principal_id":actor(),"command_receipt_id":receipt,
        "revocation":revocation
    }))
    .unwrap();
    Expected { version, reference }
}

// T-F012-002 — Le vrai writer engage une première version, son reçu et son digest ; la relecture
// après réouverture rend la même version, sans fuir dans un autre scope (REQ-MODEL-002).
#[test]
fn t_f012_002_publishes_a_model_policy_version_and_reads_it_after_reopen() {
    let directory = local_test_dir("f012-model-policy-publish").unwrap();
    let (store, profile) = ready(directory.path());
    let scope = Scope::project(PROJECT);
    let author = caller(CallerRole::HumanOperator);
    let profiles = [profile.reference()];
    assert!(history(&store, &scope).is_empty());

    let receipt = publish(
        &store,
        &author,
        &scope,
        0xf012_0010,
        Revision::INITIAL,
        publish_patch(&profiles, &[]),
    )
    .unwrap();

    let expected = expected(&profiles, &[], &receipt.receipt_id, Value::Null);
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(1));
    assert_eq!(receipt.result_ref, Some(expected.reference));
    assert_eq!(history(&store, &scope), vec![expected.version.clone()]);
    assert!(history(&store, &Scope::user()).is_empty());

    store.shutdown().unwrap();
    let reopened = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    assert_eq!(history(&reopened, &scope), vec![expected.version]);
    reopened.shutdown().unwrap();
}
