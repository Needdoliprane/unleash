//! T-F012-002 / AC-F012-002 / REQ-MODEL-002 : politique de modèle versionnée par les réglages.
//!
//! Banc `SQLite` local, sans réseau ni coffre : le vrai writer publie et relit l'historique du
//! scope. La forme attendue vient du contrat IPC, jamais du code sous test.
#![expect(clippy::unwrap_used, reason = "Fixtures locales et assertions de contrat")]

use std::path::Path;

use serde_json::{Value, json};
use unleash_domain::{ActorId, CommandId, ProjectId, ReceiptId, RequestId, Revision, Sha256Digest};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ProtocolVersion, ReasonCode, ReceiptState,
    SetProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
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
// La portée d'un profil est immuable : chaque portée de la fixture a son propre profil.
const FOREIGN_PROFILE: Uuid = Uuid::from_u128(0xf012_0005);
const SHARED_PROFILE: Uuid = Uuid::from_u128(0xf012_0007);

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

/// Politique demandée par l'appelant, écrite depuis le contrat IPC.
#[derive(Clone)]
struct Policy {
    profiles: Vec<Value>,
    capabilities: Vec<&'static str>,
    fallbacks: Vec<Value>,
    limits: Value,
}

impl Policy {
    fn bounded(profiles: &[Value]) -> Self {
        Self {
            profiles: profiles.to_vec(),
            capabilities: vec!["text"],
            fallbacks: vec![],
            limits: json!({"max_llm_calls":1,"max_output_tokens":128,"max_cost_microusd":10000,
                "max_duration_seconds":60}),
        }
    }

    fn patch(&self) -> Value {
        json!({"operation":"publish","policy_id":POLICY,"allowed_profiles":self.profiles,
            "allowed_capabilities":self.capabilities,"fallback_profiles":self.fallbacks,
            "limits":self.limits})
    }

    fn document(&self, version: u64, receipt: &ReceiptId) -> Value {
        json!({"policy_id":POLICY,"version":version,"allowed_profiles":self.profiles,
            "allowed_capabilities":self.capabilities,"fallback_profiles":self.fallbacks,
            "limits":self.limits,"principal_id":actor(),"command_receipt_id":receipt})
    }

    /// Version attendue : le digest couvre le document de publication, sans sa référence.
    fn expected(&self, version: u64, receipt: &ReceiptId, revocation: &Value) -> Expected {
        let digest = canonical_sha256(&self.document(version, receipt)).unwrap();
        let reference = format!("model-policy:{POLICY}:{version}:{digest}");
        let version = serde_json::from_value(json!({
            "policy_id":POLICY,"version":version,"digest":digest,"policy_version_ref":reference,
            "allowed_profiles":self.profiles,"allowed_capabilities":self.capabilities,
            "fallback_profiles":self.fallbacks,"limits":self.limits,"principal_id":actor(),
            "command_receipt_id":receipt,"revocation":revocation
        }))
        .unwrap();
        Expected { version, reference }
    }
}

fn revoke_patch(version: u64) -> Value {
    json!({"operation":"revoke","policy_id":POLICY,"version":version})
}

#[derive(Debug, PartialEq, Eq)]
struct Expected {
    version: ModelPolicyVersion,
    reference: String,
}

fn actor() -> ActorId {
    ActorId::new("user:profile").unwrap()
}

fn authority(role: CallerRole) -> CallerAuthority {
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
    payload: &Value,
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

fn create_project(store: &UnleashStore, project: ProjectId, command: u128) -> Scope {
    let scope = Scope::project(project);
    let payload = json!({"name":"fixture-project","weight":1});
    let envelope = envelope(
        store,
        command,
        &scope,
        project.as_uuid(),
        Revision::INITIAL,
        "CreateProject",
        &payload,
    );
    store
        .commit_command(request(
            &authority(CallerRole::HumanOperator),
            envelope,
            AggregateKind::Project,
            ReceiptState::Completed,
            CommandMutation::CreateProject {
                project_id: project,
                name: "fixture-project".to_owned(),
                weight: 1,
            },
        ))
        .unwrap();
    scope
}

fn configure_profile(
    store: &UnleashStore,
    scope: &Scope,
    profile_id: Uuid,
    version: u64,
    command: u128,
) -> Profile {
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
        profile_id,
        profile_version: version,
        project_id: scope.project,
        payload,
        secret_binding: None,
        catalog_binding: None,
    };
    let digest = retain(store, &configuration);
    let wire = serde_json::to_value(&configuration.payload).unwrap();
    let envelope = envelope(
        store,
        command,
        scope,
        profile_id,
        Revision::new(version - 1),
        "SetProviderProfile",
        &wire,
    );
    store
        .commit_command(request(
            &authority(CallerRole::HumanOperator),
            envelope,
            AggregateKind::ProviderProfile,
            ReceiptState::Completed,
            CommandMutation::SetProviderProfile {
                caller: authority(CallerRole::HumanOperator),
                preparation: Box::new(ProviderProfilePreparation {
                    configuration,
                    content_object_hash: digest,
                }),
            },
        ))
        .unwrap();
    Profile {
        id: profile_id,
        version,
        digest,
    }
}

/// Un projet, un profil configuré dans sa portée et aucune politique publiée.
fn ready(root: &Path) -> (UnleashStore, Scope, Profile) {
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    store
        .register_principal("profile-user", "user", actor().as_str())
        .unwrap();
    let scope = create_project(&store, PROJECT, 0xf012_0003);
    let profile = configure_profile(&store, &scope, PROFILE, 1, 0xf012_0004);
    (store, scope, profile)
}

fn history(store: &UnleashStore, scope: &Scope) -> Vec<ModelPolicyVersion> {
    store
        .read_settings(scope.kind, &scope.id)
        .unwrap()
        .model_policy_versions
}

fn revision(store: &UnleashStore, scope: &Scope) -> Revision {
    store
        .read_settings(scope.kind, &scope.id)
        .unwrap()
        .revision
}

/// Envoie le patch fermé `model_policy_version` au vrai writer, comme le fait le démon.
fn commit_policy(
    store: &UnleashStore,
    caller: &CallerAuthority,
    scope: &Scope,
    command: u128,
    expected: Revision,
    policy: &Value,
) -> Result<CommandReceipt, StoreError> {
    let wire = json!({"scope_kind":scope.kind,"scope_id":scope.id,
        "patch":{"model_policy_version":policy}});
    let envelope = envelope(
        store,
        command,
        scope,
        scope.aggregate(store),
        expected,
        "SetSettings",
        &wire,
    );
    let mutation = CommandMutation::SetSettingsWithModelConfiguration {
        scope_kind: scope.kind,
        scope_id: scope.id.clone(),
        caller: caller.clone(),
        patch: SettingsPatch::default(),
        binding: None,
        policy: Some(Box::new(serde_json::from_value(policy.clone()).unwrap())),
    };
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

fn is_conflict(result: Result<CommandReceipt, StoreError>) -> bool {
    matches!(
        result,
        Err(StoreError::ContractViolation {
            reason: Some(ReasonCode::ModelConfigurationConflict),
            ..
        })
    )
}

fn is_invalid(result: Result<CommandReceipt, StoreError>) -> bool {
    matches!(result, Err(StoreError::InvalidArgument { reason: None, .. }))
}

// T-F012-002 — Le vrai writer engage une première version, son reçu et son digest ; la relecture
// après réouverture rend la même version, sans fuir dans un autre scope (REQ-MODEL-002).
#[test]
fn t_f012_002_publishes_a_model_policy_version_and_reads_it_after_reopen() {
    let directory = local_test_dir("f012-model-policy-publish").unwrap();
    let (store, scope, profile) = ready(directory.path());
    let author = authority(CallerRole::HumanOperator);
    let policy = Policy::bounded(&[profile.reference()]);
    assert!(history(&store, &scope).is_empty());

    let receipt =
        commit_policy(&store, &author, &scope, 0xf012_0010, Revision::INITIAL, &policy.patch())
            .unwrap();

    let expected = policy.expected(1, &receipt.receipt_id, &Value::Null);
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

// T-F012-002 — Une version suivante n'efface pas la précédente ; la révocation est un fait
// distinct qui garde le document, son digest et le reçu de publication (REQ-MODEL-002).
#[test]
fn t_f012_002_publishes_the_next_version_and_revokes_an_exact_version() {
    let directory = local_test_dir("f012-model-policy-revoke").unwrap();
    let (store, scope, profile) = ready(directory.path());
    let author = authority(CallerRole::HumanOperator);
    let first = Policy::bounded(&[profile.reference()]);
    let second = Policy {
        limits: json!({"max_llm_calls":2,"max_output_tokens":256,"max_cost_microusd":10000,
            "max_duration_seconds":120}),
        ..first.clone()
    };

    let published =
        commit_policy(&store, &author, &scope, 0xf012_0020, Revision::INITIAL, &first.patch())
            .unwrap();
    let expected_first = first.expected(1, &published.receipt_id, &Value::Null);
    let next = commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0021,
        Revision::new(1),
        &second.patch(),
    )
    .unwrap();
    let expected_second = second.expected(2, &next.receipt_id, &Value::Null);
    assert_eq!(next.result_ref, Some(expected_second.reference.clone()));
    let retained = vec![expected_first.version.clone(), expected_second.version.clone()];
    assert_eq!(history(&store, &scope), retained);

    let revocation = commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0022,
        Revision::new(2),
        &revoke_patch(1),
    )
    .unwrap();
    let revoked_first = first.expected(
        1,
        &published.receipt_id,
        &json!({"principal_id":actor(),"command_receipt_id":revocation.receipt_id}),
    );
    assert_eq!(
        revocation.result_ref,
        Some(format!("{}:revoked", expected_first.reference))
    );
    let resulting = vec![revoked_first.version.clone(), expected_second.version.clone()];
    assert_eq!(history(&store, &scope), resulting);

    // La révocation répétée et une version absente sont refusées sans rien engager.
    let repeat = revoke_patch(1);
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0023,
        Revision::new(3),
        &repeat
    )));
    let missing = revoke_patch(3);
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0024,
        Revision::new(3),
        &missing
    )));
    assert_eq!(revision(&store, &scope), Revision::new(3));
    assert_eq!(history(&store, &scope), resulting);

    store.shutdown().unwrap();
    let reopened = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    assert_eq!(history(&reopened, &scope), resulting);
    reopened.shutdown().unwrap();
}

// T-F012-002 — La configuration n'élargit jamais les droits : profils exacts de la portée de
// l'éditeur, digest concordant, chaque repli parmi les profils autorisés (REQ-MODEL-002).
#[test]
fn t_f012_002_refuses_out_of_scope_profiles_divergent_digests_and_outsides() {
    let directory = local_test_dir("f012-model-policy-references").unwrap();
    let (store, scope, profile) = ready(directory.path());
    let author = authority(CallerRole::HumanOperator);
    let other = create_project(&store, ProjectId::from_u128(0xf012_0005), 0xf012_0005);
    let foreign = configure_profile(&store, &other, FOREIGN_PROFILE, 1, 0xf012_0006);
    let shared = configure_profile(&store, &Scope::user(), SHARED_PROFILE, 1, 0xf012_0007);
    let divergent = json!({"profile_id":PROFILE,"profile_version":1,
        "digest":Sha256Digest::from_bytes(b"other fixture version")});
    let mut widened = Policy::bounded(&[profile.reference()]);
    widened.fallbacks = vec![foreign.reference()];

    // Profil réservé à un autre projet.
    let reserved = Policy::bounded(&[foreign.reference()]).patch();
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0030,
        Revision::INITIAL,
        &reserved
    )));
    // Digest divergent de la version configurée.
    let stale_digest = Policy::bounded(&[divergent]).patch();
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0031,
        Revision::INITIAL,
        &stale_digest
    )));
    // Repli qui ne figure pas parmi les profils autorisés.
    let outside = widened.patch();
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &scope,
        0xf012_0032,
        Revision::INITIAL,
        &outside
    )));
    // Profil réservé au projet, publié depuis les réglages utilisateur.
    let user_scope = Scope::user();
    let project_patch = Policy::bounded(&[profile.reference()]).patch();
    assert!(is_conflict(commit_policy(
        &store,
        &author,
        &user_scope,
        0xf012_0033,
        Revision::INITIAL,
        &project_patch
    )));
    assert_eq!(revision(&store, &scope), Revision::INITIAL);
    assert_eq!(revision(&store, &user_scope), Revision::INITIAL);
    assert!(history(&store, &scope).is_empty());
    assert!(history(&store, &user_scope).is_empty());

    // Un réglage projet vise les profils globaux ou de ce projet.
    let mixed = Policy::bounded(&[profile.reference(), shared.reference()]);
    let receipt =
        commit_policy(&store, &author, &scope, 0xf012_0034, Revision::INITIAL, &mixed.patch())
            .unwrap();
    let expected = mixed.expected(1, &receipt.receipt_id, &Value::Null);
    assert_eq!(history(&store, &scope), vec![expected.version]);
    store.shutdown().unwrap();
}

// T-F012-002 — La forme hors contrat est refusée avant tout fait, et seuls les rôles humains
// publient une autorité de modèle (REQ-MODEL-002).
#[test]
fn t_f012_002_refuses_out_of_contract_limits_and_non_human_authority() {
    let directory = local_test_dir("f012-model-policy-form").unwrap();
    let (store, scope, profile) = ready(directory.path());
    let author = authority(CallerRole::HumanOperator);
    let mut command = 0xf012_0040u128;
    let mut refuse = |policy: &Policy| {
        command += 1;
        assert!(is_invalid(commit_policy(
            &store,
            &author,
            &scope,
            command,
            Revision::INITIAL,
            &policy.patch()
        )));
    };
    for limits in [
        json!({"max_llm_calls":0,"max_output_tokens":128,"max_cost_microusd":10000,
            "max_duration_seconds":0}),
        json!({"max_llm_calls":0,"max_output_tokens":128,"max_cost_microusd":10000,
            "max_duration_seconds":2_592_001}),
        json!({"max_llm_calls":9_007_199_254_740_992u64,"max_output_tokens":128,
            "max_cost_microusd":10000,"max_duration_seconds":60}),
    ] {
        let mut policy = Policy::bounded(&[profile.reference()]);
        policy.limits = limits;
        refuse(&policy);
    }
    let duplicated = Policy::bounded(&[profile.reference(), profile.reference()]);
    refuse(&duplicated);
    for capability in ["", " ", "bad\u{0}capability", "a\u{7f}"] {
        let mut policy = Policy::bounded(&[profile.reference()]);
        policy.capabilities = vec![capability];
        refuse(&policy);
    }
    assert_eq!(revision(&store, &scope), Revision::INITIAL);
    assert!(history(&store, &scope).is_empty());

    let bounded = Policy::bounded(&[profile.reference()]).patch();
    for role in [
        CallerRole::Agent,
        CallerRole::Model,
        CallerRole::ReconcileCoordinator,
        CallerRole::SystemMaintenance,
    ] {
        command += 1;
        let outsider = authority(role);
        assert!(matches!(
            commit_policy(&store, &outsider, &scope, command, Revision::INITIAL, &bounded),
            Err(StoreError::Unauthorized(_))
        ));
    }
    assert_eq!(revision(&store, &scope), Revision::INITIAL);

    // Les deux rôles humains publient.
    let receipt =
        commit_policy(&store, &authority(CallerRole::PolicyAdmin), &scope, 0xf012_0050, Revision::INITIAL, &bounded)
            .unwrap();
    let expected = Policy::bounded(&[profile.reference()]).expected(1, &receipt.receipt_id, &Value::Null);
    assert_eq!(history(&store, &scope), vec![expected.version]);
    store.shutdown().unwrap();
}
