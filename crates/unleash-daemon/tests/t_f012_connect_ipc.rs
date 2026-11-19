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
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{AuthorityGeneration, CommandId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, ReasonCode,
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
        let endpoint = format!("http://{}/v1", self.endpoint.local_addr().unwrap());
        let payload = json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":"synthetic-connect-profile","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
        client
            .send_command(&envelope(
                client,
                profile,
                10,
                "SetProviderProfile",
                payload,
            ))
            .await
            .unwrap()
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
