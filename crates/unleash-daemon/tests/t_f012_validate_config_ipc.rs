//! F012 : validation locale par UDS, sans qualification de connexion ou génération.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales synthétiques et assertions de contrat"
)]
#![expect(clippy::panic, reason = "Le vrai daemon doit démarrer dans ce banc")]

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    ReasonCode, ReceiptState, canonical_sha256,
};
use unleash_testkit::{ManualClock, local_test_dir};
use uuid::Uuid;

struct StoreMonotonic(Arc<ManualClock>);

impl unleash_store::MonotonicClockPort for StoreMonotonic {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct Fixture {
    _rendezvous: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
    clock: Arc<ManualClock>,
    endpoint: TcpListener,
}

struct LocalObservationRow {
    state: String,
    admission_class: Option<String>,
    hash: String,
    relative_path: String,
    authority_generation: String,
    receipt_id: String,
    principal_id: String,
    actor_id: String,
}

impl Fixture {
    async fn start() -> (Self, UnleashClient) {
        use std::os::unix::fs::PermissionsExt;
        let rendezvous = local_test_dir("f012-validate-rdv").unwrap();
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let data = local_test_dir("f012-validate-data").unwrap();
        let clock = Arc::new(ManualClock::default());
        let endpoint = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        endpoint.set_nonblocking(true).unwrap();
        let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
            data_root: Some(data.path().to_path_buf()),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            store_overrides: StoreOverrides {
                clock: Some(clock.clone()),
                monotonic_clock: Some(Arc::new(StoreMonotonic(clock.clone()))),
                ..StoreOverrides::default()
            },
            ..DaemonConfig::default()
        })
        .await
        .unwrap() else {
            panic!("daemon attendu");
        };
        let client = Self::connect(&handle, true).await;
        (
            Self {
                _rendezvous: rendezvous,
                data,
                handle,
                clock,
                endpoint,
            },
            client,
        )
    }

    async fn connect(handle: &unleash_daemon::DaemonHandle, admin: bool) -> UnleashClient {
        let config = ClientConfig::new(
            handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "validate-config-test",
        );
        UnleashClient::connect(if admin {
            config
        } else {
            config.without_admin_credential()
        })
        .await
        .unwrap()
    }

    fn db(&self) -> Connection {
        Connection::open_with_flags(
            self.data.path().join("unleash-authority.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }

    fn assert_no_provider_effect(&self) {
        assert!(
            matches!(self.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
        );
        let db = self.db();
        for table in [
            "actions",
            "action_dispatches",
            "model_calls",
            "model_call_manifests",
            "provider_probe_contexts",
            "capacity_reservations",
            "budget_reservations",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "validate_config ne doit pas créer {table}");
        }
    }

    fn validation_counts(&self) -> (i64, i64, i64) {
        let db = self.db();
        (
            db.query_row("SELECT COUNT(*) FROM provider_probes", [], |row| row.get(0))
                .unwrap(),
            db.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_type='TestProviderProfile'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE aggregate_id IN (SELECT probe_id FROM provider_probes)",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        )
    }

    fn live_references(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM object_references WHERE state='live'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    async fn configure(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        id: u128,
        revision: u64,
        project: Option<ProjectId>,
    ) -> CommandReceipt {
        let endpoint = format!("http://{}/v1", self.endpoint.local_addr().unwrap());
        let payload = json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":format!("synthetic-v{}",revision+1),"locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
        let mut command = envelope(client, profile, id, "SetProviderProfile", payload);
        command.expected_revision = Revision::new(revision);
        command.project_id = project;
        client.send_command(&command).await.unwrap()
    }
}

fn envelope(
    client: &UnleashClient,
    aggregate: Uuid,
    id: u128,
    kind: &str,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: aggregate,
        expected_revision: Revision::INITIAL,
        command_type: kind.to_owned(),
        payload: payload.as_object().unwrap().clone(),
    }
}

fn validation(
    client: &UnleashClient,
    profile: Uuid,
    version: u64,
    id: u128,
    source: &CommandReceipt,
) -> CommandEnvelope {
    envelope(
        client,
        Uuid::from_u128(id),
        id,
        "TestProviderProfile",
        json!({"profile_id":profile,"profile_version":version,"phase":"validate_config","capability_set":["text"],"limits":{"timeout_ms":1_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"locality":"local_only","mandate_ref":source.receipt_id.to_string()}),
    )
}

fn profile_query(profile: Uuid, version: u64, project: Option<ProjectId>) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf012_9900 + u128::from(version)),
        query_type: "GetProviderProfile".to_owned(),
        project_id: project,
        payload: json!({"profile_id":profile,"profile_version":version})
            .as_object()
            .unwrap()
            .clone(),
    }
}

fn assert_local_observation(
    fixture: &Fixture,
    request: &CommandEnvelope,
    receipt: &CommandReceipt,
) {
    let db = fixture.db();
    let row = db.query_row(
        "SELECT p.state,p.admission_class,p.context_hash,o.relative_path,p.authority_generation,p.command_receipt_id,p.principal_id,c.actor_id FROM provider_probes p JOIN object_records o ON o.object_hash=p.context_hash JOIN command_receipts c ON c.receipt_id=p.command_receipt_id WHERE p.probe_id=?1 AND o.state='live'",
        [request.aggregate_id.to_string()],
        |row| Ok(LocalObservationRow {
            state: row.get(0)?,
            admission_class: row.get(1)?,
            hash: row.get(2)?,
            relative_path: row.get(3)?,
            authority_generation: row.get(4)?,
            receipt_id: row.get(5)?,
            principal_id: row.get(6)?,
            actor_id: row.get(7)?,
        }),
    ).unwrap();
    assert_eq!(row.state, "ended");
    assert_eq!(row.admission_class, None);
    assert_eq!(row.receipt_id, receipt.receipt_id.to_string());
    assert_eq!(
        row.authority_generation,
        request.authority_generation.to_string()
    );
    let bytes = std::fs::read(
        fixture
            .data
            .path()
            .join("artifacts")
            .join(&row.relative_path),
    )
    .unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        unleash_domain::Sha256Digest::from_bytes(&bytes).to_string(),
        row.hash
    );
    assert_eq!(canonical_sha256(&document).unwrap().to_string(), row.hash);
    assert_observation_facts(&db, &document, request, receipt, &row);
    let linked: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts c JOIN events e ON e.sequence=c.last_event_seq JOIN outbox o ON o.event_id=e.event_id WHERE c.receipt_id=?1 AND e.aggregate_id=?2 AND e.causation_kind='command' AND e.causation_id=c.receipt_id)",
        [receipt.receipt_id.to_string(),request.aggregate_id.to_string()],
        |row| row.get(0),
    ).unwrap();
    assert!(
        linked,
        "observation/reçu/événement/outbox doivent être engagés ensemble"
    );
}

fn assert_observation_facts(
    db: &Connection,
    document: &Value,
    request: &CommandEnvelope,
    receipt: &CommandReceipt,
    row: &LocalObservationRow,
) {
    let (reference, digest, configured_receipt): (String, String, String) = db.query_row(
        "SELECT profile_version_ref,config_digest,receipt_id FROM provider_profiles WHERE profile_id=?1 AND profile_version=?2",
        rusqlite::params![request.payload.get("profile_id").unwrap().as_str().unwrap(),request.payload.get("profile_version").unwrap().as_u64().unwrap()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-probe:{}", request.aggregate_id))
    );
    for (key, value) in [
        ("schema_version", json!(1)),
        ("phase", json!("validate_config")),
        ("status", json!("validated")),
        ("probe_id", json!(request.aggregate_id)),
        (
            "profile_id",
            request.payload.get("profile_id").unwrap().clone(),
        ),
        (
            "profile_version",
            request.payload.get("profile_version").unwrap().clone(),
        ),
        ("profile_version_ref", json!(reference)),
        ("configuration_digest", json!(digest)),
        ("configuration_receipt_id", json!(configured_receipt)),
        ("command_receipt_id", json!(receipt.receipt_id)),
        ("actor_id", json!(row.actor_id)),
        ("principal_id", json!(row.principal_id)),
        ("project_id", json!(request.project_id)),
        ("authority_generation", json!(request.authority_generation)),
    ] {
        assert_eq!(
            document.get(key),
            Some(&value),
            "fait local divergent : {key}"
        );
    }
    assert!(document.get("observed_at_ms").unwrap().as_i64().unwrap() > 0);
    assert!(document.get("agent_id").is_none());
    assert!(document.get("session_id").is_none());
}

#[tokio::test]
async fn t_f012_013_validate_config_commits_local_observation_and_exact_version_read() {
    let (fixture, mut client) = Fixture::start().await;
    let profile = Uuid::from_u128(0xf012_1000);
    let configured = fixture.configure(&mut client, profile, 1001, 0, None).await;
    let request = validation(&client, profile, 1, 1002, &configured);
    let outcome = client.send_command(&request).await;
    assert!(
        outcome.is_ok(),
        "route validate_config locale attendue : {outcome:?}"
    );
    let receipt = outcome.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert!(receipt.result_ref.is_some());
    assert_local_observation(&fixture, &request, &receipt);
    fixture.assert_no_provider_effect();
    let read = client
        .query(&profile_query(profile, 1, None))
        .await
        .unwrap();
    assert_eq!(read.watermark, Some(receipt.event_cursor));
    assert_eq!(read.result.get("profile_id"), Some(&json!(profile)));
    assert_eq!(read.result.get("profile_version"), Some(&json!(1)));
    assert_eq!(
        read.result.get("configuration_receipt_id"),
        Some(&json!(configured.receipt_id))
    );
    assert_eq!(read.result.get("qualification"), Some(&json!("required")));
    assert!(read.result.get("profile_version_ref").unwrap().is_string());
    assert!(read.result.get("configuration_digest").unwrap().is_string());
    let observations = read.result.get("observations").unwrap().as_array().unwrap();
    assert_eq!(observations.len(), 1);
    let observation = observations.first().unwrap();
    assert_eq!(observation.get("phase"), Some(&json!("validate_config")));
    assert_eq!(observation.get("status"), Some(&json!("validated")));
    assert_eq!(
        observation.get("probe_id"),
        Some(&json!(request.aggregate_id))
    );
    assert_eq!(
        observation.get("result_ref"),
        Some(&json!(receipt.result_ref))
    );
    assert!(observation.get("observed_at_ms").unwrap().as_i64().unwrap() > 0);
    fixture.configure(&mut client, profile, 1003, 1, None).await;
    let old = client
        .query(&profile_query(profile, 1, None))
        .await
        .unwrap();
    assert_eq!(old.result, read.result);
    let newer = client
        .query(&profile_query(profile, 2, None))
        .await
        .unwrap();
    assert_eq!(newer.result.get("profile_version"), Some(&json!(2)));
    assert_eq!(newer.result.get("observations"), Some(&json!([])));
    assert_eq!(fixture.validation_counts(), (1, 1, 1));
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_013_validate_config_replay_precedes_missing_bytes_and_changed_wire_conflicts() {
    let (fixture, mut client) = Fixture::start().await;
    let profile = Uuid::from_u128(0xf012_2000);
    let configured = fixture.configure(&mut client, profile, 2001, 0, None).await;
    let mut request = validation(&client, profile, 1, 2002, &configured);
    let receipt = client.send_command(&request).await.unwrap();
    let before = fixture.validation_counts();
    let references = fixture.live_references();
    let relative: String = fixture.db().query_row(
        "SELECT o.relative_path FROM provider_profiles p JOIN object_records o ON o.object_hash=p.config_object_hash WHERE p.profile_id=?1 AND p.profile_version=1",
        [profile.to_string()], |row| row.get(0),
    ).unwrap();
    std::fs::remove_file(fixture.data.path().join("artifacts").join(relative)).unwrap();
    request.request_id = RequestId::from_u128(2003);
    assert_eq!(client.send_command(&request).await.unwrap(), receipt);
    request
        .payload
        .insert("capability_set".to_owned(), json!([]));
    assert!(
        matches!(client.send_command(&request).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::IdempotencyConflict)
    );
    assert_eq!(fixture.validation_counts(), before);
    assert_eq!(fixture.live_references(), references);
    fixture.assert_no_provider_effect();
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_013_validate_config_rejects_other_source_version_actor_and_lineage() {
    let (fixture, mut client) = Fixture::start().await;
    let profile = Uuid::from_u128(0xf012_3000);
    let source = fixture.configure(&mut client, profile, 3001, 0, None).await;
    let other = fixture
        .configure(&mut client, Uuid::from_u128(0xf012_3002), 3002, 0, None)
        .await;
    let next_version = fixture.configure(&mut client, profile, 3003, 1, None).await;
    let settings = envelope(
        &client,
        Uuid::from_u128(3004),
        3004,
        "SetSettings",
        json!({"scope_kind":"user","scope_id":"default","patch":{"confirm_retention_shortening":false}}),
    );
    let unrelated = client.send_command(&settings).await.unwrap();
    let control = validation(&client, profile, 1, 3005, &source);
    client.send_command(&control).await.unwrap();
    let before = fixture.validation_counts();
    assert_eq!(before, (1, 1, 1));
    let references = fixture.live_references();
    let base = validation(&client, profile, 1, 3010, &source);
    let mut requests = Vec::new();
    let mut wrong = base.clone();
    wrong
        .payload
        .insert("mandate_ref".to_owned(), json!(other.receipt_id));
    requests.push(wrong);
    wrong = base.clone();
    wrong
        .payload
        .insert("mandate_ref".to_owned(), json!(Uuid::from_u128(0xdead)));
    requests.push(wrong);
    wrong = base.clone();
    wrong.payload.insert("profile_version".to_owned(), json!(2));
    requests.push(wrong);
    wrong = base.clone();
    wrong
        .payload
        .insert("mandate_ref".to_owned(), json!(next_version.receipt_id));
    requests.push(wrong);
    wrong = base.clone();
    wrong
        .payload
        .insert("mandate_ref".to_owned(), json!(unrelated.receipt_id));
    requests.push(wrong);
    wrong = base.clone();
    wrong.actor_ref = Some("untrusted:other-actor".to_owned());
    requests.push(wrong);
    for request in requests {
        assert!(client.send_command(&request).await.is_err());
        assert_eq!(fixture.validation_counts(), before);
        assert_eq!(fixture.live_references(), references);
    }
    let mut stale = base;
    stale.authority_generation = AuthorityGeneration::from_u128(0xdead);
    assert!(
        matches!(client.send_command(&stale).await, Err(ClientError::Remote(error)) if error.reason_code == Some(ReasonCode::StaleAuthorityGeneration))
    );
    assert_eq!(fixture.validation_counts(), before);
    fixture.assert_no_provider_effect();
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_013_validate_config_requires_current_admin_before_preparation() {
    let (fixture, mut admin) = Fixture::start().await;
    let profile = Uuid::from_u128(0xf012_4000);
    let source = fixture.configure(&mut admin, profile, 4001, 0, None).await;
    let mut limited = Fixture::connect(&fixture.handle, false).await;
    let request = validation(&limited, profile, 1, 4002, &source);
    let objects: i64 = fixture
        .db()
        .query_row("SELECT COUNT(*) FROM object_records", [], |row| row.get(0))
        .unwrap();
    assert!(
        matches!(limited.send_command(&request).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(fixture.validation_counts(), (0, 0, 0));
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT COUNT(*) FROM object_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        objects
    );
    assert!(
        matches!(limited.query(&profile_query(profile, 1, None)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    fixture.assert_no_provider_effect();
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_013_validate_config_refuses_cross_project_source_and_read() {
    let (fixture, mut client) = Fixture::start().await;
    let a = ProjectId::from_u128(5001);
    let b = ProjectId::from_u128(5002);
    for project in [a, b] {
        let mut create = envelope(
            &client,
            project.as_uuid(),
            project.as_uuid().as_u128(),
            "CreateProject",
            json!({"name":"synthetic-probe-scope","weight":1}),
        );
        create.project_id = Some(project);
        client.send_command(&create).await.unwrap();
    }
    let profile = Uuid::from_u128(0xf012_5000);
    let source = fixture
        .configure(&mut client, profile, 5003, 0, Some(a))
        .await;
    let mut control = validation(&client, profile, 1, 5005, &source);
    control.project_id = Some(a);
    client.send_command(&control).await.unwrap();
    let before = fixture.validation_counts();
    assert_eq!(before, (1, 1, 1));
    let mut wrong = validation(&client, profile, 1, 5004, &source);
    wrong.project_id = Some(b);
    assert!(client.send_command(&wrong).await.is_err());
    assert!(
        matches!(client.query(&profile_query(profile, 1, Some(b))).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden)
    );
    assert_eq!(fixture.validation_counts(), before);
    fixture.assert_no_provider_effect();
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f012_013_validate_config_purged_response_never_recreates_an_observation() {
    let (fixture, mut client) = Fixture::start().await;
    let profile = Uuid::from_u128(0xf012_6000);
    let source = fixture.configure(&mut client, profile, 6001, 0, None).await;
    let request = validation(&client, profile, 1, 6002, &source);
    let receipt = client.send_command(&request).await.unwrap();
    let before = fixture.validation_counts();
    fixture.clock.advance(Duration::from_secs(8 * 86_400));
    let purge = envelope(
        &client,
        Uuid::from_u128(6003),
        6003,
        "PurgeHistory",
        json!({"scope_kind":"user","scope_id":"default","reason":"synthetic-validation-retention"}),
    );
    client.send_command(&purge).await.unwrap();
    let state: String = fixture
        .db()
        .query_row(
            "SELECT response_state FROM command_receipts WHERE receipt_id=?1",
            [receipt.receipt_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "tombstone");
    assert!(
        matches!(client.send_command(&request).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::ContractViolation && error.reason_code == Some(ReasonCode::IdempotencyWindowExpired))
    );
    assert_eq!(fixture.validation_counts(), before);
    fixture.assert_no_provider_effect();
    fixture
        .handle
        .shutdown_graceful("validate-config-test")
        .await
        .unwrap();
}
