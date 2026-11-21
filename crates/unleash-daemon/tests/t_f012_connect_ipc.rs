//! F012/F017 : une configuration n'est pas un mandat d'effet réseau.
//! Listener loopback synthétique uniquement ; aucun fournisseur ni coffre réel.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et assertions de contrat"
)]
#![expect(clippy::panic, reason = "Le vrai daemon doit démarrer dans ce banc")]

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::future::Future;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::sync::{Semaphore, SemaphorePermit};
use unleash_client::{ClientConfig, ClientError, EventStreamReader, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{
    AuthorityGeneration, ClockPort, CommandId, ProjectId, RequestId, Revision, TimestampMs,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    ReasonCode, ReceiptState, SubscribeEventsRequest,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};
use unleash_testkit::{Cutpoint, CutpointBarrier, CutpointController, ManualClock, local_test_dir};
use uuid::Uuid;

#[path = "support/provider_probe_connect_observation_scenarios.rs"]
mod observation_scenarios;

#[path = "support/provider_probe_connect_trace.rs"]
mod connect_trace;

// Each fixture opens its own daemon and SQLite writer. Limit contention
// between independent scenarios while their IPC timing assertions run.
static CONNECT_FIXTURE_SLOTS: Semaphore = Semaphore::const_new(2);

struct ConnectFixture {
    rendezvous: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
    endpoint: TcpListener,
    _fixture_slot: SemaphorePermit<'static>,
}

struct DispatchPause {
    controller: CutpointController,
    barrier: Arc<CutpointBarrier>,
    entered: tokio::sync::Notify,
}

impl DispatchPause {
    fn new() -> Arc<Self> {
        let controller = CutpointController::new();
        let barrier = controller.arm_barrier(Cutpoint::C4, Duration::from_secs(10));
        Arc::new(Self {
            controller,
            barrier,
            entered: tokio::sync::Notify::new(),
        })
    }
}

impl CutpointObserver for DispatchPause {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        if cutpoint == StoreCutpoint::C4AfterBrokerDispatchCommit {
            self.entered.notify_one();
            self.controller
                .hit(Cutpoint::C4)
                .map_err(|error| StoreError::CutpointInjected {
                    cutpoint: "C4".to_owned(),
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }
}

struct DispatchGuard {
    barrier: Arc<CutpointBarrier>,
    armed: bool,
}

impl DispatchGuard {
    fn release(&mut self) {
        self.barrier.release();
        self.armed = false;
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        if self.armed {
            self.barrier.abort();
        }
    }
}

impl ConnectFixture {
    async fn start() -> (Self, UnleashClient) {
        Self::start_with_cutpoints(None).await
    }

    async fn start_with_cutpoints(
        cutpoints: Option<Arc<dyn CutpointObserver>>,
    ) -> (Self, UnleashClient) {
        Self::start_with_store_overrides(StoreOverrides {
            cutpoints,
            ..StoreOverrides::default()
        })
        .await
    }

    async fn start_with_store_overrides(store_overrides: StoreOverrides) -> (Self, UnleashClient) {
        let fixture_slot = CONNECT_FIXTURE_SLOTS.acquire().await.unwrap();
        connect_trace::install();
        let rendezvous = local_test_dir("f012-connect-rdv").unwrap();
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let data = local_test_dir("f012-connect-data").unwrap();
        let endpoint = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        endpoint.set_nonblocking(true).unwrap();
        let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
            data_root: Some(data.path().to_path_buf()),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            store_overrides,
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
                rendezvous,
                data,
                handle,
                endpoint,
                _fixture_slot: fixture_slot,
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
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"connect-fixture-v1","reason":"local transport fixture permission","provider_probe_mandate":null}),
    );
    request
        .payload
        .insert("provider_probe_mandate".into(), change);
    request.expected_revision = Revision::new(revision);
    request
}

fn capture_authorized_connect(fixture: &ConnectFixture) -> tokio::task::JoinHandle<Vec<u8>> {
    let listener =
        tokio::net::TcpListener::from_std(fixture.endpoint.try_clone().unwrap()).unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        bytes
    })
}

fn connect_diagnostic(fixture: &ConnectFixture) -> Value {
    let db = fixture.db();
    let rows = |query: &str| {
        db.prepare(query)
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    json!({"claims":rows("SELECT claim_state FROM provider_probe_transport_contexts"),
        "probes":rows("SELECT state||':'||COALESCE(wait_reason,'none') FROM provider_probes"),
        "actions":rows("SELECT state FROM actions"),
        "capacity":rows("SELECT state FROM capacity_reservations"),
        "pools":rows("SELECT pool_kind||':'||scope_key||':'||effective_limit FROM capacity_pools"),
        "events":rows("SELECT event_type FROM events ORDER BY sequence")})
}

async fn wait_for_dispatch_or_reply<F>(
    fixture: &ConnectFixture,
    pause: &DispatchPause,
    command: &mut F,
) -> Option<CommandReceipt>
where
    F: Future<Output = Result<CommandReceipt, ClientError>> + Unpin,
{
    tokio::select! {
        response = command => {
            match response {
                Ok(receipt) => {
                    let reached=tokio::time::timeout(Duration::from_secs(3), pause.entered.notified()).await;
                    assert!(reached.is_ok(),"C4 not reached after accepted receipt; durable states: {}",connect_diagnostic(fixture));
                    Some(receipt)
                }
                Err(error) => {
                    pause.barrier.abort();
                    fixture.handle.shutdown_graceful("connect-red").await.unwrap();
                    fixture.assert_no_dispatch_or_active_unit();
                    assert!(matches!(&error, ClientError::Remote(remote) if remote.code==ErrorCode::NotFoundOrHidden), "rupture de fixture inattendue : {error:?}");
                    panic!("route connect fermée malgré un vrai mandat actif : {error:?}");
                }
            }
        }
        entered = tokio::time::timeout(Duration::from_secs(3), pause.entered.notified()) => {
            entered.unwrap();
            None
        }
    }
}

fn assert_held_dispatch_before_socket(
    fixture: &ConnectFixture,
    request: &CommandEnvelope,
    now: TimestampMs,
) {
    // C4 blocks after the marker's commit and before its ACK authorizes any I/O.
    assert!(matches!(fixture.endpoint.accept(), Err(error) if error.kind()==ErrorKind::WouldBlock));
    let held: i64 = fixture.db().query_row(r"
SELECT COUNT(*) FROM provider_probes p
JOIN command_receipts c ON c.receipt_id=p.command_receipt_id
JOIN broker_intents i ON json_extract(i.intent_json,'$.cause.kind')='command_receipt'
 AND json_extract(i.intent_json,'$.cause.receipt_id')=p.command_receipt_id
JOIN actions a ON a.action_id=i.action_id
JOIN provider_probe_transport_contexts context ON context.probe_id=p.probe_id
 AND context.action_id=a.action_id AND context.receipt_id=c.receipt_id
JOIN broker_phase_records authorizing ON authorizing.action_id=a.action_id
 AND authorizing.phase_key='authorizing'
JOIN events requested ON requested.event_id=authorizing.event_id
 AND requested.event_type='ActionAuthorizationRequested'
JOIN action_dispatches d ON d.action_id=a.action_id
JOIN broker_phase_records dispatch ON dispatch.action_id=a.action_id AND dispatch.phase_key='dispatch'
JOIN events dispatched ON dispatched.sequence=dispatch.event_sequence AND dispatched.event_type='ActionDispatchCommitted'
JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id
JOIN capacity_reservations r ON r.reservation_id=o.reservation_id
JOIN capacity_pools pool ON pool.pool_id=r.pool_id
JOIN resource_leases l ON l.resource_kind='execution' AND l.resource_key='provider_probe:'||p.probe_id
WHERE p.probe_id=?1 AND p.phase='connect' AND p.state IN ('admitted','running')
 AND c.command_type='TestProviderProfile' AND c.aggregate_id=p.probe_id
 AND c.command_digest=?3 AND c.actor_id=context.actor_id
 AND c.authority_generation=p.authority_generation
 AND context.context_digest=p.context_hash AND context.phase='connect'
 AND context.principal_id=p.principal_id AND context.profile_version_ref=p.profile_version_ref
 AND context.claim_state='claimed' AND context.claim_revision=1
 AND context.claim_id IS NOT NULL AND context.claimed_at_ms<=requested.recorded_at_ms
 AND context.authority_generation=p.authority_generation AND context.daemon_epoch=p.daemon_epoch
 AND context.project_id IS a.project_id
 AND a.origin_kind='control_plane' AND a.control_principal_id=p.principal_id AND a.state='dispatching'
 AND a.action_type='model.provider.connect' AND a.fence=0
 AND a.authority_generation=p.authority_generation AND a.daemon_epoch=p.daemon_epoch
 AND json_extract(i.intent_json,'$.manifest.effects_class')='network_egress'
 AND json_array_length(json_extract(i.intent_json,'$.manifest.secret_refs'))=0
 AND json_extract(i.intent_json,'$.payload_digest')=json_extract(context.context_json,'$.request_digest')
 AND r.owner_type='provider_probe' AND r.owner_id=p.probe_id AND r.admission_class='work'
 AND r.amount=1 AND r.state='held' AND r.authority_generation=p.authority_generation AND r.daemon_epoch=p.daemon_epoch
 AND pool.pool_kind='execution' AND pool.scope_key='global' AND pool.unit='slot'
 AND l.owner_type='provider_probe' AND l.owner_id=p.probe_id AND l.state='held'
 AND l.authority_generation=p.authority_generation AND l.daemon_epoch=p.daemon_epoch
 AND l.fence=p.fence AND l.expires_at_ms>?2
 AND l.boot_identity=(SELECT boot_identity FROM daemon_state WHERE singleton=1)
 AND p.fence>0 AND json_extract(dispatched.payload_json,'$.dispatch_id')=d.dispatch_id
 AND EXISTS(SELECT 1 FROM events admitted WHERE admitted.aggregate_id=p.probe_id
  AND admitted.event_type='ProviderProbeAdmitted' AND admitted.stream_id=dispatched.stream_id
  AND admitted.sequence<dispatched.sequence
  AND requested.sequence<admitted.sequence
  AND json_extract(admitted.payload_json,'$.reservation_id')=r.reservation_id
  AND json_extract(admitted.payload_json,'$.lease_id')=l.lease_id
  AND json_extract(admitted.payload_json,'$.fence')=p.fence)
", rusqlite::params![request.aggregate_id.to_string(),now.as_i64(),unleash_protocol::compute_command_digest_v2(request).unwrap().to_string()], |row|row.get(0)).unwrap();
    assert_eq!(
        held, 1,
        "le dispatch exact exige la place held et la lease vivante du vrai probe"
    );
    assert_no_model_or_secret_rows(fixture);
}

async fn assert_connect_observation(
    client: &mut UnleashClient,
    profile: Uuid,
    probe: Uuid,
    receipt: &CommandReceipt,
    events: &mut EventStreamReader,
) {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
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
            if result
                .result
                .get("observations")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .any(|entry| entry["phase"] == "connect")
            {
                break result;
            }
            assert!(
                events.next_frame().await.unwrap().is_some(),
                "flux fermé avant observation durable"
            );
        }
    })
    .await
    .unwrap();
    assert_eq!(
        result.result.get("qualification").unwrap_or(&Value::Null),
        "required",
        "TCP ne prouve pas une capacité de génération"
    );
    let observations = result
        .result
        .get("observations")
        .and_then(Value::as_array)
        .unwrap();
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
    assert_no_model_or_secret_rows(fixture);
}

fn assert_no_model_or_secret_rows(fixture: &ConnectFixture) {
    let db = fixture.db();
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

struct CommittedConnectProof {
    action_id: String,
    dispatch_id: String,
    request_digest: String,
    output_digest: String,
    started_at_ms: i64,
    observed_at_ms: i64,
    committed_at_ms: i64,
    relative_path: String,
}

fn assert_committed_connect_proof(fixture: &ConnectFixture, receipt: &CommandReceipt) {
    let db = fixture.db();
    let proof = db
        .query_row(
            r"
SELECT a.action_id,d.dispatch_id,json_extract(i.intent_json,'$.payload_digest'),
 json_extract(e.payload_json,'$.output_digest'),d.started_at_ms,d.ended_at_ms,
 e.recorded_at_ms,o.relative_path
FROM actions a
JOIN broker_intents i ON i.action_id=a.action_id
JOIN action_dispatches d ON d.action_id=a.action_id
JOIN broker_phase_records b ON b.action_id=a.action_id AND b.phase_key='observation:1'
JOIN events e ON e.event_id=b.event_id AND e.event_type='ActionObserved'
JOIN object_records o ON o.object_hash=json_extract(e.payload_json,'$.output_digest')
 AND o.state='live'
WHERE a.state='observed_success'
 AND json_extract(i.intent_json,'$.cause.receipt_id')=?1
 AND json_extract(e.payload_json,'$.dispatch_id')=d.dispatch_id
 AND EXISTS(SELECT 1 FROM object_references r
  JOIN provider_probes p ON p.probe_id=r.owner_id
  WHERE r.object_hash=o.object_hash AND r.owner_kind='provider_probe' AND r.state='live'
   AND p.command_receipt_id=?1 AND p.phase='connect')
",
            [receipt.receipt_id.to_string()],
            |row| {
                Ok(CommittedConnectProof {
                    action_id: row.get(0)?,
                    dispatch_id: row.get(1)?,
                    request_digest: row.get(2)?,
                    output_digest: row.get(3)?,
                    started_at_ms: row.get(4)?,
                    observed_at_ms: row.get(5)?,
                    committed_at_ms: row.get(6)?,
                    relative_path: row.get(7)?,
                })
            },
        )
        .unwrap();
    assert!(proof.started_at_ms <= proof.observed_at_ms);
    assert!(proof.observed_at_ms <= proof.committed_at_ms);
    let relative = std::path::Path::new(&proof.relative_path);
    assert!(
        relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    );
    let bytes = std::fs::read(fixture.data.path().join("artifacts").join(relative)).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        unleash_protocol::canonicalize_to_vec(&document).unwrap(),
        bytes
    );
    assert_eq!(
        unleash_protocol::canonical_sha256(&document)
            .unwrap()
            .to_string(),
        proof.output_digest
    );
    let native_observed = document
        .get("observed_at_ms")
        .and_then(Value::as_i64)
        .unwrap();
    assert!(proof.started_at_ms <= native_observed);
    assert!(native_observed <= proof.observed_at_ms);
    assert_eq!(
        document,
        json!({
            "schema_version":1,
            "action_id":proof.action_id,
            "dispatch_id":proof.dispatch_id,
            "request_digest":proof.request_digest,
            "peer":fixture.endpoint.local_addr().unwrap().to_string(),
            "certificate_validation":"not_tls",
            "revocation_checked":false,
            "http_application_bytes_sent":0,
            "observed_at_ms":native_observed,
        })
    );
}

#[tokio::test]
async fn t_f012_013_connect_active_mandate_uses_durable_dispatch_once() {
    let pause = DispatchPause::new();
    // The injected C4 pause is controlled by the fixture. Use a stable UTC
    // clock so unrelated test load does not use up the mandate during the pause.
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let clock = Arc::new(ManualClock::new(now));
    let (fixture, mut admin) = ConnectFixture::start_with_store_overrides(StoreOverrides {
        cutpoints: Some(pause.clone()),
        clock: Some(clock.clone()),
        ..StoreOverrides::default()
    })
    .await;
    let mut dispatch_guard = DispatchGuard {
        barrier: Arc::clone(&pause.barrier),
        armed: true,
    };
    let profile = Uuid::from_u128(0xf012_c101);
    let configured = fixture.configure(&mut admin, profile).await;
    let mandate = fixture
        .issue_connect_mandate(&mut admin, profile, &configured, 30)
        .await;
    fixture.assert_no_dispatch_or_active_unit();
    let mut events = admin
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: mandate.event_cursor.stream_id,
            after_sequence: mandate.event_cursor.sequence,
            project_id: None,
            aggregate_id: None,
            batch_limit: 64,
        })
        .await
        .unwrap();
    let request = connect(&admin, profile, 31, &mandate);
    let (capture, receipt) = {
        let mut pending = Box::pin(admin.send_command(&request));
        let early = wait_for_dispatch_or_reply(&fixture, &pause, &mut pending).await;
        assert_held_dispatch_before_socket(&fixture, &request, clock.now_ms());
        let capture = capture_authorized_connect(&fixture);
        dispatch_guard.release();
        let receipt = if let Some(receipt) = early {
            receipt
        } else {
            pending.await.unwrap()
        };
        (capture, receipt)
    };
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        receipt.result_ref,
        Some(format!("provider-probe:{}", request.aggregate_id))
    );
    let captured = capture.await.unwrap();
    assert!(captured.is_empty(), "connect n'envoie aucun octet HTTP");
    assert_eq!(admin.send_command(&request).await.unwrap(), receipt);
    assert_connect_observation(
        &mut admin,
        profile,
        request.aggregate_id,
        &receipt,
        &mut events,
    )
    .await;
    drop(events);
    fixture
        .handle
        .shutdown_graceful("connect-test")
        .await
        .unwrap();
    assert_completed_probe_without_model(&fixture, &request, &receipt);
    assert_committed_connect_proof(&fixture, &receipt);
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
