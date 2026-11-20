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
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{
    AuthorityGeneration, CommandId, ProjectId, RequestId, Revision, Sha256Digest,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    QueryResponse,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};
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
        Self::start_with_cutpoints(None).await
    }

    async fn start_with_cutpoints(
        cutpoints: Option<Arc<dyn CutpointObserver>>,
    ) -> (Self, UnleashClient) {
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
            store_overrides: StoreOverrides {
                cutpoints,
                ..StoreOverrides::default()
            },
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

fn generation_limits() -> Value {
    json!({"timeout_ms":5_000,"max_calls":1,"max_output_tokens":32,"max_cost_usd_micros":0})
}

fn generation_issue(profile: &Profile) -> Value {
    json!({
        "kind":"issue","principal_id":profile.principal,"profile_id":profile.id,"profile_version":1,
        "phase":"generation_probe","locality":"local_only","capability_set":["text"],
        "limits":generation_limits(),"expires_at_ms":expires_at(),
    })
}

fn generation_request(
    client: &UnleashClient,
    profile: &Profile,
    grant: &CommandReceipt,
    id: u128,
) -> CommandEnvelope {
    command(
        client,
        id,
        "TestProviderProfile",
        Uuid::from_u128(id),
        json!({"profile_id":profile.id,"profile_version":1,"phase":"generation_probe",
               "locality":"local_only","capability_set":["text"],"limits":generation_limits(),
               "mandate_ref":grant.receipt_id}),
    )
}

fn assert_retained_generation_inputs(fixture: &Fixture, probe: Uuid) {
    let db = fixture.db();
    let (inputs, temporary): (i64, i64) = db.query_row(
        "SELECT
          (SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_probe' AND owner_id=?1
           AND source_field IN ('fixture','request_payload','capabilities','model_policy','data_policy') AND state='live'),
          (SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_generation_preparation' AND state='live')",
        [probe.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((inputs, temporary), (5, 0));
    let (path, hash): (String, String) = db
        .query_row(
            "SELECT o.relative_path,o.object_hash FROM provider_probe_contexts c
         JOIN object_references r ON r.owner_kind='provider_probe' AND r.owner_id=c.probe_id
             AND r.source_field='capabilities' AND r.state='live'
         JOIN object_records o ON o.object_hash=r.object_hash
             AND o.object_hash=json_extract(c.context_json,'$.preparation.capability_snapshot_hash')
         WHERE c.probe_id=?1",
            [probe.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let bytes = std::fs::read(fixture.data.path().join("artifacts").join(path)).unwrap();
    assert_eq!(Sha256Digest::from_bytes(&bytes).to_string(), hash);
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed,
        json!({"schema_version":1,"requested":["text"],"declared":["text"],"observed":[],"unknown":["text"]})
    );
}

#[tokio::test]
async fn t_f012_generation_exact_mandate_prepares_the_autonomous_context_once() {
    let (fixture, mut admin) = Fixture::start().await;
    let profile = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_9e01), None, 310)
        .await
        .unwrap();
    let grant = generation_issue(&profile);
    let issued = admin
        .send_command(&policy(&admin, 312, None, 0, Some(grant)))
        .await
        .unwrap();
    fixture.assert_no_effect();
    let request = generation_request(&admin, &profile, &issued, 313);
    let response = admin.send_command(&request).await;
    let replay = admin.send_command(&request).await;
    fixture.shutdown().await;
    let receipt = response.expect("un mandat de génération exact doit préparer le contexte");
    assert_eq!(receipt.state, unleash_protocol::ReceiptState::Accepted);
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-probe:{}", request.aggregate_id))
    );
    assert_eq!(replay.unwrap(), receipt);
    let db = fixture.db();
    let prepared: i64 = db.query_row(
        "SELECT COUNT(*) FROM provider_probe_contexts c JOIN provider_probes p USING(probe_id)
         WHERE c.probe_id=?1 AND c.receipt_id=?2 AND p.phase='generation_probe'
          AND json_extract(c.context_json,'$.preparation.mandate_ref')=?3
          AND json_extract(c.context_json,'$.preparation.limits.max_output_tokens')=32
          AND json_extract(c.context_json,'$.preparation.resolved_model_id')='synthetic-mandate-profile'
          AND json_extract(c.context_json,'$.preparation.source_run_id') IS NULL",
        [request.aggregate_id.to_string(), receipt.receipt_id.to_string(), issued.receipt_id.to_string()],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(prepared, 1);
    assert_retained_generation_inputs(&fixture, request.aggregate_id);
    assert!(
        matches!(fixture.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
    );
    let dispatches: i64 = db
        .query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(dispatches, 0);
}

async fn wait_for_generation_intent(fixture: &Fixture, probe: Uuid) -> bool {
    use rusqlite::OptionalExtension;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let published = fixture
                .db()
                .query_row(
                    "SELECT a.state FROM provider_probe_contexts p
                 JOIN actions a ON a.action_id=p.action_id
                 WHERE p.probe_id=?1",
                    [probe.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .unwrap();
            if published.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

fn assert_generation_waiting_for_placement(
    fixture: &Fixture,
    probe: Uuid,
    receipt: &CommandReceipt,
) {
    let db = fixture.db();
    let (path, hash): (String,String) = db.query_row(
        "SELECT o.relative_path,o.object_hash FROM provider_probe_contexts p
         JOIN provider_probes u USING(probe_id)
         JOIN actions a ON a.action_id=p.action_id AND a.state='authorizing'
         JOIN model_calls c ON c.model_call_id=p.model_call_id AND c.action_id=a.action_id
          AND c.provider_probe_id=p.probe_id AND c.state='prepared'
         JOIN model_call_manifests m ON m.manifest_id=p.manifest_id AND m.model_call_id=c.model_call_id
          AND m.context_kind='provider_probe_context' AND m.provider_probe_context_id=p.probe_id
         JOIN object_records o ON o.object_hash=m.content_object_hash AND o.state='live'
         JOIN object_references r ON r.object_hash=o.object_hash AND r.owner_id=p.probe_id
          AND r.owner_kind='provider_probe' AND r.source_field='request_manifest' AND r.state='live'
         JOIN broker_phase_records b ON b.action_id=a.action_id AND b.phase_key='authorizing' AND b.event_id=c.trigger_event_id
         WHERE p.probe_id=?1 AND p.receipt_id=?2 AND u.state='queued' AND u.wait_reason='provider_probe_transport_unqualified'
           AND NOT EXISTS(SELECT 1 FROM action_dispatches)
           AND NOT EXISTS(SELECT 1 FROM capacity_reservations)
           AND NOT EXISTS(SELECT 1 FROM budget_reservations)
           AND (SELECT COUNT(*) FROM broker_phase_records WHERE action_id=a.action_id)=1",
        [probe.to_string(), receipt.receipt_id.to_string()],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    let bytes = std::fs::read(fixture.data.path().join("artifacts").join(path)).unwrap();
    assert_eq!(Sha256Digest::from_bytes(&bytes).to_string(), hash);
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(field(&manifest, "manifest_kind"), &json!("request"));
    assert_eq!(
        field(field(&manifest, "origin_ref"), "provider_probe_id"),
        &json!(probe)
    );
}

#[tokio::test]
async fn t_f012_generation_materializes_authorizing_and_waits_for_placement() {
    let (fixture, mut admin) = Fixture::start().await;
    let profile = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_9e03), None, 330)
        .await
        .unwrap();
    let grant = admin
        .send_command(&policy(
            &admin,
            332,
            None,
            0,
            Some(generation_issue(&profile)),
        ))
        .await
        .unwrap();
    let request = generation_request(&admin, &profile, &grant, 333);
    let receipt = admin.send_command(&request).await.unwrap();
    let appeared = wait_for_generation_intent(&fixture, request.aggregate_id).await;
    let replay = admin.send_command(&request).await;
    fixture.shutdown().await;
    assert!(appeared, "le reçu doit atteindre le writer d'Authorizing");
    assert_eq!(replay.unwrap(), receipt);
    assert_generation_waiting_for_placement(&fixture, request.aggregate_id, &receipt);
    assert!(
        matches!(fixture.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
    );
}

struct FailGenerationAuthorizing(AtomicUsize);

impl CutpointObserver for FailGenerationAuthorizing {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit
            && self
                .0
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                == Ok(1)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "generation authorizing rollback".to_owned(),
            });
        }
        Ok(())
    }
}

async fn wait_for_generation_rollback(
    fixture: &Fixture,
    fault: &FailGenerationAuthorizing,
) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if fault.0.load(Ordering::SeqCst) == 0 {
                let remaining: i64 = fixture.db().query_row(
                    "SELECT (SELECT COUNT(*) FROM actions)
                          + (SELECT COUNT(*) FROM model_calls)
                          + (SELECT COUNT(*) FROM model_call_manifests)
                          + (SELECT COUNT(*) FROM object_references WHERE source_field='request_manifest' AND state='live')",
                    [], |row| row.get(0),
                ).unwrap();
                if remaining == 0 {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.is_ok()
}

#[tokio::test]
async fn t_f012_generation_replay_recovers_authorizing_after_atomic_rollback() {
    let fault = Arc::new(FailGenerationAuthorizing(AtomicUsize::new(0)));
    let (fixture, mut admin) = Fixture::start_with_cutpoints(Some(fault.clone())).await;
    let profile = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_9e04), None, 340)
        .await
        .unwrap();
    let grant = admin
        .send_command(&policy(
            &admin,
            342,
            None,
            0,
            Some(generation_issue(&profile)),
        ))
        .await
        .unwrap();
    let request = generation_request(&admin, &profile, &grant, 343);
    // The first C1 is the successful receipt. The second is Authorizing.
    fault.0.store(2, Ordering::SeqCst);
    let receipt = admin.send_command(&request).await.unwrap();
    let rolled_back = wait_for_generation_rollback(&fixture, &fault).await;
    let replay = admin.send_command(&request).await;
    let appeared = wait_for_generation_intent(&fixture, request.aggregate_id).await;
    fixture.shutdown().await;
    assert!(
        rolled_back,
        "aucun fragment du broker ni manifeste visible après C1"
    );
    assert_eq!(replay.unwrap(), receipt);
    assert!(
        appeared,
        "le rejeu doit reprendre la préparation au même agrégat"
    );
    assert_generation_waiting_for_placement(&fixture, request.aggregate_id, &receipt);
    assert!(
        matches!(fixture.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn t_f012_generation_inventory_mandate_is_hidden_before_preparation() {
    let (fixture, mut admin) = Fixture::start().await;
    let profile = fixture
        .configure(&mut admin, Uuid::from_u128(0xf012_9e02), None, 320)
        .await
        .unwrap();
    let permission = issue(&profile, "inventory", expires_at());
    let issued = admin
        .send_command(&policy(&admin, 322, None, 0, Some(permission)))
        .await
        .unwrap();
    let request = generation_request(&admin, &profile, &issued, 323);
    let result = admin.send_command(&request).await;
    fixture.shutdown().await;
    fixture.assert_no_effect();
    assert!(
        matches!(&result, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden),
        "un mandat Inventory doit rester caché pour la génération : {result:?}",
    );
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
