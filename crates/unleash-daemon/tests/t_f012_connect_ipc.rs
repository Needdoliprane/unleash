//! F012/F017 : une configuration n'est pas un mandat d'effet réseau.
//! Listener loopback synthétique uniquement ; aucun fournisseur ni coffre réel.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et assertions de contrat"
)]
#![expect(clippy::panic, reason = "Le vrai daemon doit démarrer dans ce banc")]

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{AuthorityGeneration, CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    ReasonCode, ReceiptState,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

struct ConnectFixture {
    _rendezvous: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
    endpoint: TcpListener,
}

impl ConnectFixture {
    async fn start() -> (Self, UnleashClient) {
        use std::os::unix::fs::PermissionsExt;
        let rendezvous = local_test_dir("f012-connect-rdv").unwrap();
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let data = local_test_dir("f012-connect-data").unwrap();
        let endpoint = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        endpoint.set_nonblocking(true).unwrap();
        let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
            data_root: Some(data.path().to_path_buf()),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            ..DaemonConfig::default()
        })
        .await
        .unwrap() else {
            panic!("daemon attendu");
        };
        let client = UnleashClient::connect(ClientConfig::new(
            handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "connect-test",
        ))
        .await
        .unwrap();
        (
            Self {
                _rendezvous: rendezvous,
                data,
                handle,
                endpoint,
            },
            client,
        )
    }

    async fn configure(&self, client: &mut UnleashClient, profile: Uuid) -> CommandReceipt {
        self.configure_with_id(client, profile, 10).await
    }

    async fn configure_with_id(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        id: u128,
    ) -> CommandReceipt {
        let endpoint = format!("http://{}/v1", self.endpoint.local_addr().unwrap());
        let payload = json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":"synthetic-connect-profile","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
        client
            .send_command(&envelope(
                client,
                profile,
                id,
                "SetProviderProfile",
                payload,
            ))
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

    async fn issue_connect_mandate(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        configured: &CommandReceipt,
        id: u128,
    ) -> CommandReceipt {
        let principal: String = self.db().query_row(
            "SELECT p.principal_id FROM principals p JOIN command_receipts c ON c.actor_id=p.actor_id WHERE c.receipt_id=?1 AND p.state='active'",
            [configured.receipt_id.to_string()], |row| row.get(0),
        ).unwrap();
        let expires = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
            + 60_000;
        let request = policy_command(
            client,
            id,
            0,
            json!({"kind":"issue","principal_id":principal,"profile_id":profile,"profile_version":1,"phase":"connect","locality":"local_only","capability_set":[],"limits":{"timeout_ms":1_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"expires_at_ms":expires}),
        );
        client.send_command(&request).await.unwrap()
    }

    fn assert_no_dispatch_or_active_unit(&self) {
        assert!(
            matches!(self.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
        );
        let db = Connection::open_with_flags(
            self.data.path().join("unleash-authority.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        for table in [
            "actions",
            "action_dispatches",
            "provider_probes",
            "model_calls",
            "model_call_manifests",
            "capacity_reservations",
            "budget_reservations",
            "resource_leases",
            "secret_references",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "un mandat absent ne permet aucune ligne {table}");
        }
        let receipts: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_type='TestProviderProfile'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipts, 0, "aucun reçu de probe ne doit être fabriqué");
        let references: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM object_references WHERE owner_kind LIKE 'provider_probe%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            references, 0,
            "aucune référence de probe ne doit être préparée"
        );
    }
}

fn envelope(
    client: &UnleashClient,
    aggregate: Uuid,
    id: u128,
    kind: &str,
    payload: Value,
) -> CommandEnvelope {
    let Value::Object(payload) = payload else {
        panic!("payload objet attendu");
    };
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
        payload,
    }
}

fn connect(
    client: &UnleashClient,
    profile: Uuid,
    id: u128,
    configured: &CommandReceipt,
) -> CommandEnvelope {
    envelope(
        client,
        Uuid::from_u128(id),
        id,
        "TestProviderProfile",
        json!({"profile_id":profile,"profile_version":1,"phase":"connect","capability_set":[],"limits":{"timeout_ms":1_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"locality":"local_only","mandate_ref":configured.receipt_id.to_string()}),
    )
}

fn policy_command(
    client: &UnleashClient,
    id: u128,
    revision: u64,
    change: Value,
) -> CommandEnvelope {
    let mut request = envelope(
        client,
        client.hello().authority_generation.as_uuid(),
        id,
        "SetPolicy",
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"connect-fixture-v1","reason":"local transport fixture permission","provider_probe_mandate":change}),
    );
    request.expected_revision = Revision::new(revision);
    request
}

struct ConnectedTransport {
    bytes: Vec<u8>,
    dispatches: i64,
    reservations: i64,
}

fn capture_authorized_connect(
    fixture: &ConnectFixture,
    probe: Uuid,
) -> tokio::task::JoinHandle<ConnectedTransport> {
    let listener =
        tokio::net::TcpListener::from_std(fixture.endpoint.try_clone().unwrap()).unwrap();
    let database = fixture.data.path().join("unleash-authority.db");
    tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let dispatches = db
            .query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
                row.get(0)
            })
            .unwrap();
        let reservations = db
            .query_row(
                "SELECT COUNT(*) FROM execution_reservation_owners WHERE provider_probe_id=?1",
                [probe.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        drop(db);
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        ConnectedTransport {
            bytes,
            dispatches,
            reservations,
        }
    })
}

async fn assert_connect_observation(
    client: &mut UnleashClient,
    profile: Uuid,
    probe: Uuid,
    receipt: &CommandReceipt,
) {
    let result = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(32),
            query_type: "GetProviderProfile".to_owned(),
            project_id: None,
            payload: json!({"profile_id":profile,"profile_version":1})
                .as_object()
                .unwrap()
                .clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        result.result["qualification"], "required",
        "TCP ne prouve pas une capacité de génération"
    );
    let observations = result.result["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 1, "seul connect a été exécuté");
    let observed = observations
        .iter()
        .find(|entry| entry["phase"] == "connect")
        .unwrap();
    assert_eq!(observed["probe_id"], json!(probe));
    assert_eq!(observed["result_ref"], json!(receipt.result_ref));
}

fn assert_completed_probe_without_model(
    fixture: &ConnectFixture,
    request: &CommandEnvelope,
    receipt: &CommandReceipt,
) {
    assert!(matches!(fixture.endpoint.accept(), Err(error) if error.kind()==ErrorKind::WouldBlock));
    let db = fixture.db();
    let probes: i64 = db.query_row("SELECT COUNT(*) FROM provider_probes WHERE phase='connect' AND probe_id=?1 AND command_receipt_id=?2 AND state='ended'", [request.aggregate_id.to_string(), receipt.receipt_id.to_string()], |row|row.get(0)).unwrap();
    assert_eq!(probes, 1);
    let actions: i64 = db.query_row("SELECT COUNT(*) FROM actions a JOIN broker_intents i ON i.action_id=a.action_id JOIN action_dispatches d ON d.action_id=a.action_id WHERE a.origin_kind='control_plane' AND a.state='observed_success' AND json_extract(i.intent_json,'$.cause.kind')='command_receipt' AND json_extract(i.intent_json,'$.cause.receipt_id')=?1", [receipt.receipt_id.to_string()], |row|row.get(0)).unwrap();
    assert_eq!(
        actions, 1,
        "le succès reste corrélé au reçu réel de la commande connect"
    );
    for table in [
        "agents",
        "invocations",
        "model_calls",
        "model_call_manifests",
        "secret_references",
        "budget_reservations",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 0,
            "ce connect local sans secret ne fabrique pas {table}"
        );
    }
}

#[tokio::test]
async fn t_f012_013_connect_active_mandate_uses_durable_dispatch_once() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c101);
    let configured = fixture.configure(&mut admin, profile).await;
    let mandate = fixture
        .issue_connect_mandate(&mut admin, profile, &configured, 30)
        .await;
    fixture.assert_no_dispatch_or_active_unit();
    let request = connect(&admin, profile, 31, &mandate);
    let capture = capture_authorized_connect(&fixture, request.aggregate_id);
    let response = admin.send_command(&request).await;
    if response.is_err() {
        fixture
            .handle
            .shutdown_graceful("connect-red")
            .await
            .unwrap();
        capture.abort();
    }
    assert!(
        response.is_ok(),
        "un vrai mandat actif doit atteindre le connect autorisé : {response:?}"
    );
    let receipt = response.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-probe:{}", request.aggregate_id))
    );
    let captured = capture.await.unwrap();
    assert!(
        captured.bytes.is_empty(),
        "connect n'envoie aucun octet HTTP"
    );
    assert_eq!(captured.dispatches, 1, "le marqueur précède la connexion");
    assert_eq!(
        captured.reservations, 1,
        "l'unité du pool commun précède la connexion"
    );
    assert_eq!(admin.send_command(&request).await.unwrap(), receipt);
    assert_connect_observation(&mut admin, profile, request.aggregate_id, &receipt).await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    assert_completed_probe_without_model(&fixture, &request, &receipt);
}

#[tokio::test]
async fn t_f012_013_connect_revoked_or_other_profile_mandate_has_no_effect() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c102);
    let other = Uuid::from_u128(0xf012_c103);
    let configured = fixture.configure(&mut admin, profile).await;
    fixture.configure_with_id(&mut admin, other, 20).await;
    let mandate = fixture
        .issue_connect_mandate(&mut admin, profile, &configured, 30)
        .await;
    let foreign = admin
        .send_command(&connect(&admin, other, 31, &mandate))
        .await;
    let project = ProjectId::from_u128(0xf012_c104);
    admin
        .send_command(&envelope(
            &admin,
            project.as_uuid(),
            34,
            "CreateProject",
            json!({"name":"connect-other-scope"}),
        ))
        .await
        .unwrap();
    let mut scoped = connect(&admin, profile, 35, &mandate);
    scoped.project_id = Some(project);
    let wrong_scope = admin.send_command(&scoped).await;
    let revoke = policy_command(
        &admin,
        32,
        1,
        json!({"kind":"revoke","mandate_ref":mandate.receipt_id}),
    );
    admin.send_command(&revoke).await.unwrap();
    let revoked = admin
        .send_command(&connect(&admin, profile, 33, &mandate))
        .await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    fixture.assert_no_dispatch_or_active_unit();
    for result in [foreign, wrong_scope, revoked] {
        assert!(
            matches!(result, Err(ClientError::Remote(error)) if error.code==ErrorCode::NotFoundOrHidden)
        );
    }
}

#[tokio::test]
async fn t_f012_013_connect_configuration_receipt_is_not_network_permission() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c001);
    let configured = fixture.configure(&mut admin, profile).await;
    let request = connect(&admin, profile, 11, &configured);
    let response = admin.send_command(&request).await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    fixture.assert_no_dispatch_or_active_unit();
    assert!(
        matches!(&response, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden),
        "un mandat réseau absent doit rester introuvable sans inventer une politique : {response:?}"
    );
}

#[tokio::test]
async fn t_f012_013_connect_configuration_receipt_does_not_elevate_current_caller() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c002);
    let configured = fixture.configure(&mut admin, profile).await;
    let mut limited = UnleashClient::connect(
        ClientConfig::new(
            fixture.handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "connect-no-admin",
        )
        .without_admin_credential(),
    )
    .await
    .unwrap();
    let response = limited
        .send_command(&connect(&limited, profile, 12, &configured))
        .await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    fixture.assert_no_dispatch_or_active_unit();
    assert!(
        matches!(response, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
}

#[tokio::test]
async fn t_f012_013_connect_unknown_mandate_has_no_preparation() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c003);
    let configured = fixture.configure(&mut admin, profile).await;
    let mut request = connect(&admin, profile, 13, &configured);
    request.payload.insert(
        "mandate_ref".to_owned(),
        json!(Uuid::from_u128(0xf012_ffff)),
    );
    let response = admin.send_command(&request).await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    fixture.assert_no_dispatch_or_active_unit();
    assert!(
        matches!(&response, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden),
        "un mandat inexistant doit rester introuvable : {response:?}"
    );
}

#[tokio::test]
async fn t_f012_013_connect_old_lineage_is_rejected_before_mandate_resolution() {
    let (fixture, mut admin) = ConnectFixture::start().await;
    let profile = Uuid::from_u128(0xf012_c004);
    let configured = fixture.configure(&mut admin, profile).await;
    let mut request = connect(&admin, profile, 14, &configured);
    request.authority_generation = AuthorityGeneration::from_u128(0xf012_dead);
    let response = admin.send_command(&request).await;
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    fixture.assert_no_dispatch_or_active_unit();
    assert!(
        matches!(&response, Err(ClientError::Remote(error)) if error.reason_code == Some(ReasonCode::StaleAuthorityGeneration)),
        "l'ancienne lignée doit être refusée avant le lookup de mandat : {response:?}"
    );
}
