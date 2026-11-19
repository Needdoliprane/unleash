//! T-F012-013 / REQ-SEC-007 : émission de mandat via la politique authentifiée.
//! Le listener est un double local ; émettre ou révoquer ne doit pas le contacter.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et assertions de contrat"
)]
#![expect(clippy::panic, reason = "Le daemon et les payloads objets sont requis")]

use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{
    AuthorityGeneration, CommandId, ProjectId, RequestId, Revision, Sha256Digest,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    QueryResponse,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

struct Fixture {
    _rendezvous: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
    endpoint: TcpListener,
}

struct Profile {
    id: Uuid,
    project: Option<ProjectId>,
    principal: String,
    authority: AuthorityGeneration,
    configuration_digest: Value,
}

impl Fixture {
    async fn start() -> (Self, UnleashClient) {
        use std::os::unix::fs::PermissionsExt;
        let rendezvous = local_test_dir("f012-mandates-rdv").unwrap();
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let data = local_test_dir("f012-mandates-data").unwrap();
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
            "probe-mandates-test",
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

    fn db(&self) -> Connection {
        Connection::open_with_flags(
            self.data.path().join("unleash-authority.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }

    async fn configure(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        project: Option<ProjectId>,
        id: u128,
    ) -> Result<Profile, ClientError> {
        let endpoint = format!("http://{}/v1", self.endpoint.local_addr().unwrap());
        let payload = json!({"provider_kind":"lmstudio","dialect":{"kind":"openai_chat_completions","version":1},"endpoint":endpoint,"model":"synthetic-mandate-profile","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
        let mut request = command(client, id, "SetProviderProfile", profile, payload);
        request.project_id = project;
        let receipt = client.send_command(&request).await?;
        let principal: String = self.db().query_row(
            "SELECT p.principal_id FROM command_receipts c JOIN principals p ON p.actor_id=c.actor_id WHERE c.receipt_id=?1 AND p.state='active'",
            [receipt.receipt_id.to_string()],
            |row| row.get(0),
        ).unwrap();
        let view = client
            .query(&query(
                id + 1,
                "GetProviderProfile",
                project,
                json!({"profile_id":profile,"profile_version":1}),
            ))
            .await?;
        Ok(Profile {
            id: profile,
            project,
            principal,
            authority: client.hello().authority_generation,
            configuration_digest: field(&view.result, "configuration_digest").clone(),
        })
    }

    async fn shutdown(&self) {
        self.handle
            .shutdown_graceful("mandates-test")
            .await
            .unwrap();
    }

    fn assert_no_effect(&self) {
        assert!(
            matches!(self.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
        );
        let db = self.db();
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
            assert_eq!(count, 0, "aucun effet de probe à l'émission : {table}");
        }
    }

    fn policy_receipts(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_type='SetPolicy'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

fn object(value: Value) -> Map<String, Value> {
    let Value::Object(value) = value else {
        panic!("objet attendu")
    };
    value
}

fn command(
    client: &UnleashClient,
    id: u128,
    kind: &str,
    aggregate: Uuid,
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
        payload: object(payload),
    }
}

fn query(id: u128, kind: &str, project: Option<ProjectId>, payload: Value) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        query_type: kind.to_owned(),
        project_id: project,
        payload: object(payload),
    }
}

fn policy(
    client: &UnleashClient,
    id: u128,
    project: Option<ProjectId>,
    expected: u64,
    mandate: Option<Value>,
) -> CommandEnvelope {
    let (scope, scope_id, aggregate) = project.map_or_else(
        || {
            (
                "global",
                "global".to_owned(),
                client.hello().authority_generation.as_uuid(),
            )
        },
        |id| ("project", id.to_string(), id.as_uuid()),
    );
    let mut payload = json!({"scope_kind":scope,"scope_id":scope_id,"isolation_mode":"standard","policy_version":"mandate-fixture-v1","reason":"synthetic provider permission"});
    if let Some(mandate) = mandate {
        payload
            .as_object_mut()
            .unwrap()
            .insert("provider_probe_mandate".into(), mandate);
    }
    let mut request = command(client, id, "SetPolicy", aggregate, payload);
    request.project_id = project;
    request.expected_revision = Revision::new(expected);
    request
}

fn limits() -> Value {
    json!({"timeout_ms":1_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0})
}

fn issue(profile: &Profile, phase: &str, expires: u64) -> Value {
    json!({"kind":"issue","principal_id":profile.principal,"profile_id":profile.id,"profile_version":1,"phase":phase,"locality":"local_only","capability_set":[],"limits":limits(),"expires_at_ms":expires})
}

fn expires_at() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
        + 60_000
}

async fn summary(
    client: &mut UnleashClient,
    id: u128,
    project: Option<ProjectId>,
) -> Result<QueryResponse, ClientError> {
    client
        .query(&query(id, "ReadPolicySummary", project, json!({})))
        .await
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value.get(name).unwrap()
}

fn mandate<'a>(view: &'a QueryResponse, receipt: &CommandReceipt) -> &'a Value {
    let rows: Vec<_> = field(&view.result, "provider_probe_mandates")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| field(row, "mandate_ref") == &json!(receipt.receipt_id))
        .collect();
    assert_eq!(rows.len(), 1, "mandat exact visible une seule fois");
    rows.first().copied().unwrap()
}

fn assert_binding(row: &Value, profile: &Profile, phase: &str, expires: u64) {
    let (scope, scope_id) = profile.project.map_or_else(
        || ("global", "global".to_owned()),
        |id| ("project", id.to_string()),
    );
    assert_eq!(field(row, "scope_kind"), &json!(scope));
    assert_eq!(field(row, "scope_id"), &json!(scope_id));
    assert_eq!(
        field(row, "authority_generation"),
        &json!(profile.authority)
    );
    assert_eq!(field(row, "principal_id"), &json!(profile.principal));
    assert_eq!(field(row, "profile_id"), &json!(profile.id));
    assert_eq!(field(row, "profile_version"), &json!(1));
    assert_eq!(
        field(row, "configuration_digest"),
        &profile.configuration_digest
    );
    assert_eq!(field(row, "phase"), &json!(phase));
    assert_eq!(field(row, "locality"), &json!("local_only"));
    assert_eq!(field(row, "capability_set"), &json!([]));
    assert_eq!(field(row, "limits"), &limits());
    assert_eq!(field(row, "expires_at_ms"), &json!(expires));
    field(row, "mandate_digest")
        .as_str()
        .unwrap()
        .parse::<Sha256Digest>()
        .unwrap();
}

struct Lifecycle {
    legacy: CommandReceipt,
    a: CommandReceipt,
    b: CommandReceipt,
    before: QueryResponse,
    revoke: CommandReceipt,
    replay: CommandReceipt,
    divergent: Result<CommandReceipt, ClientError>,
    after: QueryResponse,
    profile: QueryResponse,
}

async fn lifecycle(
    client: &mut UnleashClient,
    profile: &Profile,
    expires: u64,
) -> Result<Lifecycle, ClientError> {
    let legacy = client
        .send_command(&policy(client, 200, None, 0, None))
        .await?;
    let a_wire = policy(
        client,
        201,
        None,
        1,
        Some(issue(profile, "connect", expires)),
    );
    let a = client.send_command(&a_wire).await?;
    let b = client
        .send_command(&policy(
            client,
            202,
            None,
            2,
            Some(issue(profile, "inventory", expires)),
        ))
        .await?;
    let before = summary(client, 203, None).await?;
    let revoke = client
        .send_command(&policy(
            client,
            204,
            None,
            3,
            Some(json!({"kind":"revoke","mandate_ref":a.receipt_id})),
        ))
        .await?;
    let mut replay_wire = a_wire.clone();
    replay_wire.request_id = RequestId::from_u128(205);
    let replay = client.send_command(&replay_wire).await?;
    let mut divergent_wire = a_wire;
    divergent_wire.request_id = RequestId::from_u128(206);
    *divergent_wire
        .payload
        .get_mut("provider_probe_mandate")
        .and_then(|mandate| mandate.get_mut("limits"))
        .and_then(|limits| limits.get_mut("timeout_ms"))
        .unwrap() = json!(1_001);
    let divergent = client.send_command(&divergent_wire).await;
    let after = summary(client, 207, None).await?;
    let profile = client
        .query(&query(
            208,
            "GetProviderProfile",
            None,
            json!({"profile_id":profile.id,"profile_version":1}),
        ))
        .await?;
    Ok(Lifecycle {
        legacy,
        a,
        b,
        before,
        revoke,
        replay,
        divergent,
        after,
        profile,
    })
}

fn assert_lifecycle(flow: &Lifecycle, profile: &Profile, expires: u64) {
    assert_eq!(flow.legacy.new_revision, Revision::new(1));
    assert_eq!(flow.a.new_revision, Revision::new(2));
    assert_eq!(flow.b.new_revision, Revision::new(3));
    assert_eq!(flow.revoke.new_revision, Revision::new(4));
    assert_eq!(flow.replay, flow.a);
    assert!(
        matches!(&flow.divergent, Err(ClientError::Remote(error)) if error.code == ErrorCode::IdempotencyConflict)
    );
    assert_eq!(flow.before.watermark.as_ref(), Some(&flow.b.event_cursor));
    assert_eq!(
        flow.after.watermark.as_ref(),
        Some(&flow.revoke.event_cursor)
    );
    for view in [&flow.before, &flow.after] {
        assert_eq!(
            field(&view.result, "provider_probe_mandates")
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    for (receipt, phase) in [(&flow.a, "connect"), (&flow.b, "inventory")] {
        let before = mandate(&flow.before, receipt);
        let after = mandate(&flow.after, receipt);
        assert_binding(before, profile, phase, expires);
        assert_binding(after, profile, phase, expires);
        assert_eq!(field(before, "status"), &json!("active"));
        assert!(field(before, "revocation_receipt_id").is_null());
        assert_eq!(
            field(before, "mandate_digest"),
            field(after, "mandate_digest")
        );
    }
    let a = mandate(&flow.after, &flow.a);
    assert_eq!(field(a, "status"), &json!("revoked"));
    assert_eq!(
        field(a, "revocation_receipt_id"),
        &json!(flow.revoke.receipt_id)
    );
    assert_eq!(
        field(mandate(&flow.after, &flow.b), "status"),
        &json!("active")
    );
    assert!(field(mandate(&flow.after, &flow.b), "revocation_receipt_id").is_null());
    assert_eq!(
        field(&flow.profile.result, "qualification"),
        &json!("required")
    );
    assert_eq!(field(&flow.profile.result, "observations"), &json!([]));
}

#[tokio::test]
async fn t_f012_013_policy_mandates_are_causal_scoped_and_do_not_reactivate_on_replay() {
    let (fixture, mut admin) = Fixture::start().await;
    let setup = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_d001), None, 100)
        .await;
    if setup.is_err() {
        fixture.shutdown().await;
    }
    let profile = setup.unwrap();
    let expires = expires_at();
    let result = lifecycle(&mut admin, &profile, expires).await;
    fixture.shutdown().await;
    fixture.assert_no_effect();
    let flow =
        result.expect("SetPolicy doit émettre les mandats demandés après le contrôle legacy");
    assert_lifecycle(&flow, &profile, expires);
    assert_eq!(
        fixture.policy_receipts(),
        4,
        "le rejeu et le conflit ne créent pas de reçu"
    );
}

#[tokio::test]
async fn t_f012_013_policy_mandate_requires_current_administrative_authority() {
    let (fixture, mut admin) = Fixture::start().await;
    let setup = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_d002), None, 100)
        .await;
    if setup.is_err() {
        fixture.shutdown().await;
    }
    let profile = setup.unwrap();
    let mut limited = UnleashClient::connect(
        ClientConfig::new(
            fixture.handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "mandates-no-admin",
        )
        .without_admin_credential(),
    )
    .await
    .unwrap();
    let response = limited
        .send_command(&policy(
            &limited,
            200,
            None,
            0,
            Some(issue(&profile, "connect", expires_at())),
        ))
        .await;
    fixture.shutdown().await;
    fixture.assert_no_effect();
    assert!(
        matches!(response, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(fixture.policy_receipts(), 0);
}

#[tokio::test]
async fn t_f012_013_policy_mandate_null_is_not_a_classic_update() {
    let (fixture, mut admin) = Fixture::start().await;
    let principals: i64 = fixture
        .db()
        .query_row("SELECT COUNT(*) FROM principals", [], |row| row.get(0))
        .unwrap();
    let request = policy(&admin, 200, None, 0, Some(Value::Null));
    let response = admin.send_command(&request).await;
    fixture.shutdown().await;
    fixture.assert_no_effect();
    assert!(
        matches!(response,Err(ClientError::Remote(error)) if error.code == ErrorCode::InvalidArgument)
    );
    assert_eq!(fixture.policy_receipts(), 0);
    let db = fixture.db();
    let policies: i64 = db
        .query_row("SELECT COUNT(*) FROM security_policies", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        policies, 0,
        "null ne doit pas être interprété comme une absence"
    );
    let after: i64 = db
        .query_row("SELECT COUNT(*) FROM principals", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, principals, "aucune identité créée au rejet de forme");
}

struct ScopedLifecycle {
    issued: CommandReceipt,
    visible: QueryResponse,
    hidden: QueryResponse,
    foreign_revoke: Result<CommandReceipt, ClientError>,
    after: QueryResponse,
    other_after: QueryResponse,
}

async fn scoped_setup(
    fixture: &Fixture,
    client: &mut UnleashClient,
) -> Result<(Profile, ProjectId), ClientError> {
    let a = ProjectId::from_u128(0xf012_da00);
    let b = ProjectId::from_u128(0xf012_db00);
    for (id, project, name) in [(90, a, "mandate-project-a"), (91, b, "mandate-project-b")] {
        client
            .send_command(&command(
                client,
                id,
                "CreateProject",
                project.as_uuid(),
                json!({"name":name}),
            ))
            .await?;
    }
    let profile = fixture
        .configure(client, Uuid::from_u128(0xf012_d003), Some(a), 100)
        .await?;
    Ok((profile, b))
}

async fn scoped_lifecycle(
    client: &mut UnleashClient,
    profile: &Profile,
    other: ProjectId,
    expires: u64,
) -> Result<ScopedLifecycle, ClientError> {
    let issued = client
        .send_command(&policy(
            client,
            200,
            profile.project,
            0,
            Some(issue(profile, "connect", expires)),
        ))
        .await?;
    let visible = summary(client, 201, profile.project).await?;
    let hidden = summary(client, 202, Some(other)).await?;
    let foreign_revoke = client
        .send_command(&policy(
            client,
            203,
            Some(other),
            0,
            Some(json!({"kind":"revoke","mandate_ref":issued.receipt_id})),
        ))
        .await;
    let after = summary(client, 204, profile.project).await?;
    let other_after = summary(client, 205, Some(other)).await?;
    Ok(ScopedLifecycle {
        issued,
        visible,
        hidden,
        foreign_revoke,
        after,
        other_after,
    })
}

#[tokio::test]
async fn t_f012_013_policy_mandate_visibility_and_revocation_keep_the_authenticated_scope() {
    let (fixture, mut admin) = Fixture::start().await;
    let setup = scoped_setup(&fixture, &mut admin).await;
    if setup.is_err() {
        fixture.shutdown().await;
    }
    let (profile, other) = setup.unwrap();
    let expires = expires_at();
    let result = scoped_lifecycle(&mut admin, &profile, other, expires).await;
    fixture.shutdown().await;
    fixture.assert_no_effect();
    let flow = result.expect("le mandat de projet doit être émis avant le contrôle des vues");
    assert_binding(
        mandate(&flow.visible, &flow.issued),
        &profile,
        "connect",
        expires,
    );
    for view in [&flow.hidden, &flow.other_after] {
        assert_eq!(field(&view.result, "provider_probe_mandates"), &json!([]));
        assert!(
            field(&view.result, "project_policy").is_null(),
            "aucune politique hors scope créée par le refus"
        );
    }
    assert!(
        matches!(flow.foreign_revoke, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden)
    );
    let after = mandate(&flow.after, &flow.issued);
    assert_eq!(field(after, "status"), &json!("active"));
    assert!(field(after, "revocation_receipt_id").is_null());
    assert_eq!(
        flow.after.watermark.as_ref(),
        Some(&flow.issued.event_cursor)
    );
    assert_eq!(fixture.policy_receipts(), 1);
}
