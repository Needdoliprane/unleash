//! Local daemon/profile/mandate fixture; all mutations use authenticated IPC.

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::error::Error;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, CommandReceipt, ProtocolVersion};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

use super::oracles::{NetworkExpectation, NetworkWitness};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

pub(super) fn require(condition: bool, message: &'static str) -> TestResult {
    if !condition {
        return Err(message.into());
    }
    Ok(())
}

pub(super) const INVENTORY_BODY: &[u8] = br#"{"models":[{"name":"synthetic:inventory","model":"synthetic:inventory","size":1024,"digest":"fixture-declared-digest","details":{"format":"gguf","family":"synthetic","parameter_size":"fixture","quantization_level":"fixture-q"}}]}"#;

pub(super) struct InventoryFixture {
    rendezvous: tempfile::TempDir,
    data: tempfile::TempDir,
    handle: unleash_daemon::DaemonHandle,
    endpoint: TcpListener,
}

impl InventoryFixture {
    pub(super) async fn start() -> TestResult<(Self, UnleashClient)> {
        let rendezvous = local_test_dir("f012-inventory-rdv")?;
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))?;
        let data = local_test_dir("f012-inventory-data")?;
        let endpoint = TcpListener::bind(("127.0.0.1", 0))?;
        endpoint.set_nonblocking(true)?;
        let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
            data_root: Some(data.path().to_path_buf()),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            ..DaemonConfig::default()
        })
        .await?
        else {
            return Err("fresh inventory daemon expected".into());
        };
        let client = UnleashClient::connect(ClientConfig::new(
            handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "inventory-test",
        ))
        .await?;
        Ok((
            Self {
                rendezvous,
                data,
                handle,
                endpoint,
            },
            client,
        ))
    }

    pub(super) async fn stop(self) -> TestResult {
        let stopped = self.handle.shutdown_graceful("inventory-test").await;
        let Self {
            rendezvous,
            data,
            handle,
            endpoint,
        } = self;
        tokio::task::spawn_blocking(move || drop((handle, endpoint, data, rendezvous))).await?;
        stopped?;
        Ok(())
    }

    pub(super) fn endpoint_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }
    pub(super) fn data_root(&self) -> &Path {
        self.data.path()
    }
    pub(super) fn db(&self) -> rusqlite::Result<Connection> {
        open_db(self.data.path())
    }

    pub(super) async fn configure(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        id: u128,
    ) -> TestResult<CommandReceipt> {
        let endpoint = format!("http://{}/", self.endpoint_addr()?);
        let payload = json!({"provider_kind":"ollama","dialect":{"kind":"ollama_chat","version":1},"endpoint":endpoint,"model":"synthetic:inventory","locality":"local_only","server_management":"user_managed","auth":{"kind":"none"},"limits":{"max_inflight":1,"connect_timeout_ms":2_000,"queue_timeout_ms":1_000,"load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":60_000},"declared_capabilities":["text"],"pricing":{"kind":"none_local"}});
        let receipt = client
            .send_command(&envelope(
                client,
                profile,
                id,
                "SetProviderProfile",
                payload,
            )?)
            .await?;
        require(
            receipt.new_revision == Revision::new(1),
            "exact configured profile version 1 required",
        )?;
        Ok(receipt)
    }

    pub(super) async fn issue(
        &self,
        client: &mut UnleashClient,
        profile: Uuid,
        configured: &CommandReceipt,
        phase: &str,
        id: u128,
        expected: u64,
    ) -> TestResult<CommandReceipt> {
        let principal: String = self.db()?.query_row("SELECT p.principal_id FROM principals p JOIN command_receipts c ON c.actor_id=p.actor_id WHERE c.receipt_id=?1 AND p.state='active' AND p.principal_kind='user'", [configured.receipt_id.to_string()], |row| row.get(0))?;
        let expires =
            u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())? + 60_000;
        let change = json!({"kind":"issue","principal_id":principal,"profile_id":profile,"profile_version":1,"phase":phase,"locality":"local_only","capability_set":[],"limits":{"timeout_ms":2_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},"expires_at_ms":expires});
        let mut command = envelope(
            client,
            client.hello().authority_generation.as_uuid(),
            id,
            "SetPolicy",
            json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"inventory-fixture-v1","reason":"explicit passive inventory fixture permission","provider_probe_mandate":change}),
        )?;
        command.expected_revision = Revision::new(expected);
        Ok(client.send_command(&command).await?)
    }

    pub(super) fn assert_no_pending_connection(&self) -> TestResult {
        require(
            matches!(self.endpoint.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock),
            "unexpected second inventory connection",
        )
    }

    pub(super) fn assert_no_probe_effect(&self) -> TestResult {
        self.assert_no_pending_connection()?;
        let db = self.db()?;
        super::oracles::assert_no_model_or_secret_rows(&db)?;
        for table in [
            "actions",
            "action_dispatches",
            "provider_probes",
            "capacity_reservations",
            "resource_leases",
        ] {
            let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
            require(
                count == 0,
                "closed inventory entry must not create effects or execution ownership",
            )?;
        }
        let receipts: i64 = db.query_row(
            "SELECT COUNT(*) FROM command_receipts WHERE command_type='TestProviderProfile'",
            [],
            |row| row.get(0),
        )?;
        require(
            receipts == 0,
            "a closed inventory route must not fabricate a probe receipt",
        )?;
        let candidates: i64 = db.query_row(
            "SELECT COUNT(*) FROM object_references WHERE owner_kind LIKE 'provider_probe%'",
            [],
            |row| row.get(0),
        )?;
        require(
            candidates == 0,
            "a closed inventory route must not publish a probe candidate",
        )?;
        Ok(())
    }

    pub(super) fn probe_counts(&self, probe: Uuid) -> TestResult<Vec<i64>> {
        let db = self.db()?;
        ["SELECT COUNT(*) FROM provider_probes WHERE probe_id=?1", "SELECT COUNT(*) FROM command_receipts WHERE command_type='TestProviderProfile' AND aggregate_id=?1", "SELECT COUNT(*) FROM provider_probe_transport_contexts WHERE probe_id=?1", "SELECT COUNT(*) FROM actions WHERE action_id=(SELECT action_id FROM provider_probe_transport_contexts WHERE probe_id=?1)", "SELECT COUNT(*) FROM action_dispatches WHERE action_id=(SELECT action_id FROM provider_probe_transport_contexts WHERE probe_id=?1)", "SELECT COUNT(*) FROM broker_phase_records WHERE action_id=(SELECT action_id FROM provider_probe_transport_contexts WHERE probe_id=?1)", "SELECT COUNT(*) FROM events WHERE aggregate_id=?1 OR aggregate_id=(SELECT action_id FROM provider_probe_transport_contexts WHERE probe_id=?1)", "SELECT COUNT(*) FROM capacity_reservations WHERE owner_type='provider_probe' AND owner_id=?1", "SELECT COUNT(*) FROM resource_leases WHERE owner_type='provider_probe' AND owner_id=?1"]
            .into_iter().map(|sql| db.query_row(sql, [probe.to_string()], |row| row.get(0)).map_err(Into::into)).collect()
    }

    pub(super) fn capture(&self, expected: NetworkExpectation) -> TestResult<CaptureTask> {
        self.capture_with_body(expected, INVENTORY_BODY)
    }

    pub(super) fn capture_with_body(
        &self,
        expected: NetworkExpectation,
        body: &'static [u8],
    ) -> TestResult<CaptureTask> {
        let listener = tokio::net::TcpListener::from_std(self.endpoint.try_clone()?)?;
        let root = self.data.path().to_path_buf();
        let task = tokio::spawn(capture_one(listener, root, expected, body));
        Ok(CaptureTask { task: Some(task) })
    }
}

pub(super) fn open_db(root: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        root.join("unleash-authority.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
}

pub(super) fn envelope(
    client: &UnleashClient,
    aggregate: Uuid,
    id: u128,
    kind: &str,
    payload: Value,
) -> TestResult<CommandEnvelope> {
    let Value::Object(payload) = payload else {
        return Err("command payload must be an object".into());
    };
    Ok(CommandEnvelope {
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
    })
}

pub(super) fn probe(
    client: &UnleashClient,
    profile: Uuid,
    id: u128,
    mandate: &CommandReceipt,
    phase: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: Uuid::from_u128(id),
        expected_revision: Revision::INITIAL,
        command_type: "TestProviderProfile".to_owned(),
        payload: serde_json::Map::from_iter([
            ("profile_id".to_owned(), json!(profile)),
            ("profile_version".to_owned(), json!(1)),
            ("phase".to_owned(), json!(phase)),
            ("capability_set".to_owned(), json!([])),
            (
                "limits".to_owned(),
                json!({"timeout_ms":2_000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0}),
            ),
            ("locality".to_owned(), json!("local_only")),
            ("mandate_ref".to_owned(), json!(mandate.receipt_id)),
        ]),
    }
}

pub(super) struct Capture {
    pub(super) bytes: Vec<u8>,
    pub(super) witness: NetworkWitness,
}

impl Capture {
    pub(super) fn assert_one_get(&self, endpoint: SocketAddr) -> TestResult {
        let request = std::str::from_utf8(&self.bytes)?;
        let (head, body) = request
            .split_once("\r\n\r\n")
            .ok_or("incomplete native inventory GET")?;
        require(
            head.starts_with("GET /api/tags HTTP/1.1\r\n"),
            "inventory must send the exact passive GET route",
        )?;
        require(
            head.to_ascii_lowercase()
                .contains(&format!("\r\nhost: {endpoint}")),
            "GET Host must match the configured fixture",
        )?;
        require(
            !head.to_ascii_lowercase().contains("authorization:")
                && !head.to_ascii_lowercase().contains("transfer-encoding:"),
            "inventory fixture sends neither credential nor upload framing",
        )?;
        require(body.is_empty(), "passive inventory must not upload a body")
    }
}

pub(super) struct CaptureTask {
    task: Option<JoinHandle<TestResult<Capture>>>,
}

impl CaptureTask {
    pub(super) async fn finish(mut self) -> TestResult<Capture> {
        self.task.take().ok_or("capture already consumed")?.await?
    }
}

impl Drop for CaptureTask {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn capture_one(
    listener: tokio::net::TcpListener,
    root: PathBuf,
    expected: NetworkExpectation,
    body: &[u8],
) -> TestResult<Capture> {
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await??;
    let witness = super::oracles::assert_at_network(&root, &expected)?;
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(tokio::time::timeout(Duration::from_secs(3), socket.read_u8()).await??);
        require(
            bytes.len() <= 8192,
            "fixture request header exceeds its bound",
        )?;
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.shutdown().await?;
    let mut remaining = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        socket.take(8192).read_to_end(&mut remaining),
    )
    .await??;
    bytes.extend(remaining);
    Ok(Capture { bytes, witness })
}
