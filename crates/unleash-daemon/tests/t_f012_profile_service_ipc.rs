//! T-F012-002/011/013 : vrai daemon et UDS ; aucun fournisseur, coffre ou réseau.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales synthétiques et assertions de contrat"
)]
#![expect(
    clippy::panic,
    reason = "Une fixture qui ne démarre pas le vrai daemon doit échouer"
)]

use serde_json::{Map, Value, json};
use std::path::Path;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, ReceiptState};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

fn payload(model: &str) -> Map<String, Value> {
    json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":"http://LOCALHOST:9/a%2Fb/v1","model":model,"locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":300_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}}).as_object().unwrap().clone()
}

fn command(
    client: &UnleashClient,
    profile: Uuid,
    id: u128,
    revision: u64,
    model: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: profile,
        expected_revision: Revision::new(revision),
        command_type: "SetProviderProfile".to_owned(),
        payload: payload(model),
    }
}

fn counts(root: &Path) -> (i64, i64, i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        root.join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    (db.query_row("SELECT COUNT(*) FROM provider_profiles WHERE profile_id IS NOT NULL",[],|r|r.get(0)).unwrap(),db.query_row("SELECT COUNT(*) FROM command_receipts WHERE command_type='SetProviderProfile'",[],|r|r.get(0)).unwrap(),db.query_row("SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_profile_preparation' AND state='live'",[],|r|r.get(0)).unwrap(),db.query_row("SELECT COUNT(*) FROM actions",[],|r|r.get(0)).unwrap())
}

struct RunningFixture {
    _rdv: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
}

async fn running_fixture(label: &str) -> (RunningFixture, UnleashClient) {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir(label).unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f012-profile-service").unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-test",
    ))
    .await
    .unwrap();
    (
        RunningFixture {
            _rdv: rdv,
            data,
            handle,
        },
        client,
    )
}

#[tokio::test]
async fn t_f012_002_ipc_cas_race_leaves_one_v2_and_no_live_candidate_reference() {
    let (fixture, mut first) = running_fixture("f012-cas").await;
    let mut second = UnleashClient::connect(ClientConfig::new(
        fixture.handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-other",
    ))
    .await
    .unwrap();
    let profile = Uuid::from_u128(0xf012_0003);
    let initial = command(&first, profile, 300, 0, "initial");
    first.send_command(&initial).await.unwrap();
    let a = command(&first, profile, 301, 1, "candidate-a");
    let b = command(&second, profile, 302, 1, "candidate-b");
    let (a, b) = tokio::join!(first.send_command(&a), second.send_command(&b));
    let results = [a, b];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result|matches!(result,Err(ClientError::Remote(error)) if error.code==ErrorCode::RevisionConflict)).count(),1);
    assert_eq!(counts(fixture.data.path()), (2, 2, 0, 0));
    let db = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let held:i64=db.query_row("SELECT COUNT(*) FROM object_pins WHERE owner_kind='provider_profile_preparation' AND state IN ('held','reconciling')",[],|r|r.get(0)).unwrap();
    assert_eq!(held, 0);
    fixture
        .handle
        .shutdown_graceful("profile-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_002_ipc_same_wire_concurrent_calls_have_one_configuration() {
    let (fixture, mut first) = running_fixture("f012-same-wire").await;
    let mut second = UnleashClient::connect(ClientConfig::new(
        fixture.handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-other",
    ))
    .await
    .unwrap();
    let request = command(
        &first,
        Uuid::from_u128(0xf012_0004),
        400,
        0,
        "identical-bytes",
    );
    let mut other = request.clone();
    other.request_id = RequestId::from_u128(401);
    let (a, b) = tokio::join!(first.send_command(&request), second.send_command(&other));
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a, b);
    assert_eq!(counts(fixture.data.path()), (1, 1, 0, 0));
    fixture
        .handle
        .shutdown_graceful("profile-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_011_ipc_missing_catalog_producer_does_not_invent_a_signed_price() {
    let (fixture, mut client) = running_fixture("f012-no-catalog").await;
    let mut request = command(
        &client,
        Uuid::from_u128(0xf012_0005),
        500,
        0,
        "catalogue-model",
    );
    request.payload.insert("pricing".to_owned(),json!({"kind":"catalog","catalog_id":"not-installed","catalog_version":"1","catalog_digest":unleash_domain::Sha256Digest::from_bytes(b"not-a-signed-catalogue"),"entry_digest":unleash_domain::Sha256Digest::from_bytes(b"not-a-verified-entry"),"stale_policy":"block"}));
    assert!(
        matches!(client.send_command(&request).await,Err(ClientError::Remote(error)) if error.code==ErrorCode::NotFoundOrHidden)
    );
    assert_eq!(counts(fixture.data.path()), (0, 0, 0, 0));
    let db = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let catalogues: i64 = db
        .query_row("SELECT COUNT(*) FROM model_catalog_snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    let objects: i64 = db
        .query_row("SELECT COUNT(*) FROM object_records", [], |r| r.get(0))
        .unwrap();
    assert_eq!((catalogues, objects), (0, 0));
    fixture
        .handle
        .shutdown_graceful("profile-test")
        .await
        .unwrap();
}

struct DropProfileReplyOnce(std::sync::atomic::AtomicBool);
impl unleash_daemon::PostCommitHook for DropProfileReplyOnce {
    fn after_commit(&self, envelope: &CommandEnvelope) -> unleash_daemon::PostCommitAction {
        if envelope.command_type == "SetProviderProfile"
            && self.0.swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            unleash_daemon::PostCommitAction::DropConnection
        } else {
            unleash_daemon::PostCommitAction::Continue
        }
    }
}

#[tokio::test]
async fn t_f012_002_ipc_lost_reply_then_missing_object_does_not_repeat_commit() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("f012-lost-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f012-lost-data").unwrap();
    let hook = std::sync::Arc::new(DropProfileReplyOnce(std::sync::atomic::AtomicBool::new(
        true,
    )));
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        post_commit_hook: Some(hook),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-test",
    ))
    .await
    .unwrap();
    let profile = Uuid::from_u128(0xf012_0008);
    let mut request = command(&client, profile, 800, 0, "synthetic-model");
    assert!(client.send_command(&request).await.is_err());
    assert_eq!(counts(data.path()), (1, 1, 0, 0));
    let db = rusqlite::Connection::open_with_flags(
        data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (relative,response):(String,String)=db.query_row("SELECT o.relative_path,c.response_json FROM provider_profiles p JOIN object_records o ON o.object_hash=p.config_object_hash JOIN command_receipts c ON c.receipt_id=p.receipt_id WHERE p.profile_id=?1",[profile.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let original: unleash_protocol::CommandReceipt = serde_json::from_str(&response).unwrap();
    std::fs::remove_file(data.path().join("artifacts").join(relative)).unwrap();
    let mut reconnected = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-reconnect",
    ))
    .await
    .unwrap();
    request.request_id = RequestId::from_u128(801);
    assert_eq!(reconnected.send_command(&request).await.unwrap(), original);
    let events: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM events WHERE event_type='ProviderProfileConfigured'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
    assert_eq!(counts(data.path()), (1, 1, 0, 0));
    handle.shutdown_graceful("profile-test").await.unwrap();
}

fn seed_authority_binding(
    root: &Path,
    profile: Uuid,
    project: unleash_domain::ProjectId,
    other: unleash_domain::ProjectId,
) -> unleash_domain::SecretRef {
    use unleash_store::{
        AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
        SecretReferenceInput, StoreConfig, UnleashStore,
    };
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    let actor =
        unleash_domain::ActorId::new(format!("uid:{}", unleash_platform::effective_uid())).unwrap();
    store
        .register_principal(actor.as_str(), "user", actor.as_str())
        .unwrap();
    let base = |id: u128, aggregate: Uuid, kind: &str| CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: store.active_authority_generation().unwrap(),
        actor_ref: Some(actor.to_string()),
        project_id: None,
        aggregate_id: aggregate,
        expected_revision: Revision::INITIAL,
        command_type: kind.to_owned(),
        payload: Map::new(),
    };
    for (id, project) in [(700, project), (701, other)] {
        store
            .commit_command(CommitCommandRequest {
                actor_id: actor.clone(),
                envelope: base(id, project.as_uuid(), "CreateProject"),
                target_aggregate: AggregateKind::Project,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::CreateProject {
                    project_id: project,
                    name: "native-user-fixture-project".to_owned(),
                    weight: 1,
                },
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
            .unwrap();
    }
    let reference =
        unleash_domain::SecretRef::new("secret://fixture/provider-profile-auth").unwrap();
    let mut envelope = base(702, Uuid::from_u128(702), "SetSecretReference");
    envelope.project_id = Some(project);
    // Metadata fixture through the existing authority mutation; no vault/value is read.
    store
        .commit_command(CommitCommandRequest {
            actor_id: actor.clone(),
            envelope,
            target_aggregate: AggregateKind::SecretReference,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSecretReference {
                input: SecretReferenceInput {
                    secret_ref_id: "fixture-provider-auth".to_owned(),
                    secret_ref: reference.clone(),
                    namespace: "provider".to_owned(),
                    purpose: "api_key".to_owned(),
                    secret_kind: "bearer_token".to_owned(),
                    target_kind: "provider_profile".to_owned(),
                    target_id: profile.to_string(),
                    scope_kind: "project".to_owned(),
                    scope_id: project.to_string(),
                    backend_kind: "os_keychain".to_owned(),
                },
                caller: CallerAuthority {
                    actor_id: actor,
                    role: CallerRole::HumanOperator,
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    store.shutdown().unwrap();
    reference
}

fn assert_stored_binding_and_euro_prices(
    root: &Path,
    profile: Uuid,
    project: unleash_domain::ProjectId,
) {
    let db = rusqlite::Connection::open_with_flags(
        root.join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let content: String = db
        .query_row(
            "SELECT config_json FROM provider_profiles WHERE profile_id=?1",
            [profile.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let object: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        object.pointer("/payload/pricing/entry/currency").unwrap(),
        "EUR"
    );
    assert!(
        object
            .pointer("/payload/pricing/entry/input_micros_per_million")
            .unwrap()
            .is_null()
    );
    assert_eq!(
        object
            .pointer("/secret_binding/reference/target_id")
            .unwrap(),
        &json!(profile)
    );
    assert_eq!(
        object
            .pointer("/secret_binding/reference/scope_id")
            .unwrap(),
        &json!(project)
    );
    assert_eq!(
        object.pointer("/secret_binding/revision").unwrap(),
        &json!(1)
    );
}

#[tokio::test]
async fn t_f012_013_ipc_scope_f017_binding_and_euro_unknown_prices_are_exact() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("f012-bound-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f012-bound-data").unwrap();
    let profile = Uuid::from_u128(0xf012_0006);
    let project = unleash_domain::ProjectId::from_u128(700);
    let other = unleash_domain::ProjectId::from_u128(701);
    let seed_root = data.path().to_path_buf();
    let reference = tokio::task::spawn_blocking(move || {
        seed_authority_binding(&seed_root, profile, project, other)
    })
    .await
    .unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-test",
    ))
    .await
    .unwrap();
    let mut request = command(&client, profile, 710, 0, "synthetic-model");
    request.project_id = Some(project);
    request
        .payload
        .insert("provider_kind".to_owned(), json!("openai"));
    request.payload.insert(
        "dialect".to_owned(),
        json!({"kind":"openai_responses","version":1}),
    );
    request.payload.insert(
        "endpoint".to_owned(),
        json!("https://FIXTURE.invalid:443/a%2Fb/v1/responses"),
    );
    request
        .payload
        .insert("locality".to_owned(), json!("approved_remote"));
    request.payload.insert(
        "auth".to_owned(),
        json!({"kind":"secret_ref","secret_ref":reference}),
    );
    request.payload.insert("pricing".to_owned(),json!({"kind":"user_declared","entry":{"currency":"EUR","input_micros_per_million":null,"cached_input_micros_per_million":null,"cache_write_micros_per_million":null,"output_micros_per_million":123,"observed_at_ms":1,"valid_until_ms":2,"source_url":null},"stale_policy":"block"}));
    let mut raw = request.clone();
    raw.command_id = CommandId::from_u128(711);
    raw.payload.insert(
        "auth".to_owned(),
        json!({"kind":"secret_ref","secret_ref":"sk-SYNTHETIC-DO-NOT-PERSIST"}),
    );
    assert!(
        matches!(client.send_command(&raw).await,Err(ClientError::Remote(error)) if !error.message.contains("SYNTHETIC-DO-NOT-PERSIST"))
    );
    assert_eq!(counts(data.path()), (0, 0, 0, 0));
    let first = client.send_command(&request).await.unwrap();
    let before = counts(data.path());
    assert_eq!(before, (1, 1, 0, 0));
    assert_stored_binding_and_euro_prices(data.path(), profile, project);
    assert_eq!(client.send_command(&request).await.unwrap(), first);
    let mut wrong = request.clone();
    wrong.command_id = CommandId::from_u128(712);
    wrong.aggregate_id = Uuid::from_u128(0xf012_0007);
    assert!(client.send_command(&wrong).await.is_err());
    wrong = request;
    wrong.command_id = CommandId::from_u128(713);
    wrong.project_id = Some(other);
    wrong.expected_revision = Revision::new(1);
    assert!(client.send_command(&wrong).await.is_err());
    assert_eq!(counts(data.path()), before);
    handle.shutdown_graceful("profile-test").await.unwrap();
}

#[tokio::test]
async fn t_f012_002_ipc_profile_receipt_replays_before_missing_object_preparation() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("f012-profile-ipc-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f012-profile-ipc-data").unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "profile-test",
    ))
    .await
    .unwrap();
    let profile = Uuid::from_u128(0xf012_0001);
    let mut request = command(&client, profile, 100, 0, "synthetic-model");
    let first = client.send_command(&request).await.unwrap();
    assert_eq!(first.state, ReceiptState::Accepted);
    assert_eq!(first.new_revision, Revision::new(1));
    let before = counts(data.path());
    assert_eq!(before, (1, 1, 0, 0));
    let db = rusqlite::Connection::open_with_flags(
        data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let relative:String=db.query_row("SELECT o.relative_path FROM provider_profiles p JOIN object_records o ON o.object_hash=p.config_object_hash WHERE p.profile_id=?1 AND p.profile_version=1",[profile.to_string()],|r|r.get(0)).unwrap();
    drop(db);
    let object: Value = serde_json::from_slice(
        &std::fs::read(data.path().join("artifacts").join(&relative)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        object.get("payload").unwrap().get("endpoint").unwrap(),
        "http://localhost:9/a%2Fb/v1"
    );
    assert_eq!(
        request.payload.get("endpoint").unwrap(),
        "http://LOCALHOST:9/a%2Fb/v1"
    );
    std::fs::remove_file(data.path().join("artifacts").join(relative)).unwrap();
    request.request_id = RequestId::from_u128(101);
    assert_eq!(client.send_command(&request).await.unwrap(), first);
    assert_eq!(counts(data.path()), before);
    request
        .payload
        .get_mut("model")
        .unwrap()
        .clone_from(&json!("another-wire"));
    assert!(
        matches!(client.send_command(&request).await,Err(ClientError::Remote(error)) if error.code==ErrorCode::IdempotencyConflict)
    );
    assert_eq!(counts(data.path()), before);
    handle.shutdown_graceful("profile-test").await.unwrap();
}

#[tokio::test]
async fn t_f012_013_ipc_non_admin_never_prepares_profile_objects() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("f012-profile-denied-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f012-profile-denied-data").unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let mut client = UnleashClient::connect(
        ClientConfig::new(
            handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "profile-test",
        )
        .without_admin_credential(),
    )
    .await
    .unwrap();
    let before = counts(data.path());
    let request = command(
        &client,
        Uuid::from_u128(0xf012_0002),
        200,
        0,
        "synthetic-model",
    );
    assert!(
        matches!(client.send_command(&request).await,Err(ClientError::Remote(error)) if error.code==ErrorCode::CapabilityDenied)
    );
    assert_eq!(counts(data.path()), before);
    let db = rusqlite::Connection::open_with_flags(
        data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let objects: i64 = db
        .query_row("SELECT COUNT(*) FROM object_records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(objects, 0);
    handle.shutdown_graceful("profile-test").await.unwrap();
}
