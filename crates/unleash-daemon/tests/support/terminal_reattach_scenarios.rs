//! Real daemon subprocess scenarios with a real long-lived owned PTY child.
use super::*;
use std::io::Write;

#[path = "terminal_input_scenarios.rs"]
mod input_scenarios;

#[path = "terminal_cli_scenarios.rs"]
mod cli_scenarios;

#[path = "terminal_log_rotation_scenarios.rs"]
mod log_rotation_scenarios;

#[path = "terminal_renewal_fixture.rs"]
mod renewal_fixture;

mod renewal_scenarios {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct RenewalSnapshot {
        lease_id: String,
        expires_at: i64,
        fence: i64,
        revision: i64,
        epoch: i64,
        actions: i64,
        dispatches: i64,
        proofs: i64,
        proof_sequence: i64,
        terminal_leases: i64,
    }

    fn utc_ms() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    async fn cross_utc_boundary(target: i64, expiry: i64) {
        assert!(
            target < expiry,
            "the clock boundary must leave the held lease valid"
        );
        let safety = Instant::now() + DEADLINE;
        while utc_ms() < target {
            assert!(
                Instant::now() < safety,
                "real UTC did not cross the explicit boundary"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            utc_ms() < expiry,
            "the positive renewal must not cross expiry"
        );
    }

    async fn ttl(fixture: &mut LiveFixture) -> i64 {
        let settings = fixture
            .client
            .query(&QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xf011_7001),
                query_type: "ReadSettings".into(),
                project_id: None,
                payload: object(json!({"scope_kind":"user","scope_id":"default"})),
            })
            .await
            .unwrap();
        settings
            .result
            .pointer("/extra_typed/terminal_runtime/lease_ttl_ms")
            .and_then(Value::as_i64)
            .unwrap()
    }

    async fn result(
        fixture: &mut LiveFixture,
        receipt: &unleash_protocol::CommandReceipt,
    ) -> Value {
        let reply = fixture
            .client
            .query(&QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xf011_7002),
                query_type: "GetCommandReceipt".into(),
                project_id: Some(fixture.project),
                payload: object(json!({"command_id":receipt.command_id,
                "result_ref":receipt.result_ref.as_ref().unwrap()})),
            })
            .await
            .unwrap();
        let cursor: unleash_protocol::EventCursor =
            serde_json::from_value(reply.result.get("cursor").unwrap().clone()).unwrap();
        let watermark = reply.watermark.unwrap();
        assert_eq!(watermark.stream_id, cursor.stream_id);
        assert!(watermark.sequence >= cursor.sequence);
        reply.result
    }

    async fn durable_log(fixture: &mut LiveFixture, mut page: Value) -> Value {
        let safety = Instant::now() + DEADLINE;
        loop {
            let log: unleash_process_host::LogReadResult =
                serde_json::from_value(page.get("log").unwrap().clone()).unwrap();
            if log.durable_watermark_offset > 0 {
                return page;
            }
            assert!(
                Instant::now() < safety,
                "initial PTY output must really be durable"
            );
            page = fixture
                .client
                .query(&QueryEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(0xf011_7005),
                    query_type: "ReadTerminalLog".into(),
                    project_id: Some(fixture.project),
                    payload: object(json!({"terminal_id":page.get("terminal_id").unwrap(),
                    "after_offset":0,"max_bytes":65536})),
                })
                .await
                .unwrap()
                .result;
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    fn snapshot(
        fixture: &LiveFixture,
        receipt: &unleash_protocol::CommandReceipt,
    ) -> RenewalSnapshot {
        let conn = rusqlite::Connection::open_with_flags(
            fixture.data.path().join("unleash-authority.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        // One SQL statement observes the lease and its exact durable cause in
        // the same snapshot. It never manufactures a native observation.
        conn.query_row(
            "WITH renewal AS (
                SELECT a.action_id FROM broker_intents i JOIN actions a ON a.action_id=i.action_id
                WHERE json_extract(i.intent_json,'$.cause.receipt_id')=?2 AND a.project_id=?3
             ), proof AS (
                SELECT DISTINCT p.event_sequence FROM renewal x JOIN actions a ON a.action_id=x.action_id
                JOIN broker_phase_records p ON p.action_id=a.action_id AND p.phase_key LIKE 'observation:%'
                JOIN object_references r ON r.owner_id=a.action_id
                  AND r.source_field=json_extract(p.phase_json,'$.phase.receipt.dispatch_id')
                  AND r.object_hash=json_extract(p.phase_json,'$.phase.receipt.output_digest') AND r.state='live'
                JOIN object_records o ON o.object_hash=r.object_hash AND o.state='live'
                WHERE a.state='observed_success'
             ) SELECT l.lease_id,l.expires_at_ms,l.fence,s.control_revision,l.daemon_epoch,
                (SELECT COUNT(*) FROM renewal),
                (SELECT COUNT(*) FROM action_dispatches d JOIN renewal x ON x.action_id=d.action_id),
                (SELECT COUNT(*) FROM proof),(SELECT COALESCE(MAX(event_sequence),0) FROM proof),
                (SELECT COUNT(*) FROM resource_leases WHERE resource_kind='terminal')
             FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id
             WHERE s.terminal_id=?1",
            rusqlite::params![receipt.aggregate_id.to_string(),receipt.receipt_id.to_string(),fixture.project.to_string()],
            |row| Ok(RenewalSnapshot {lease_id:row.get(0)?,expires_at:row.get(1)?,fence:row.get(2)?,
                revision:row.get(3)?,epoch:row.get(4)?,actions:row.get(5)?,dispatches:row.get(6)?,
                proofs:row.get(7)?,proof_sequence:row.get(8)?,terminal_leases:row.get(9)?}),
        ).unwrap()
    }

    async fn wait_observed(
        fixture: &LiveFixture,
        receipt: &unleash_protocol::CommandReceipt,
        previous_expiry: i64,
    ) -> RenewalSnapshot {
        let safety = Instant::now() + DEADLINE;
        loop {
            let state = snapshot(fixture, receipt);
            if state.expires_at != previous_expiry {
                assert_eq!(
                    (state.actions, state.dispatches, state.proofs),
                    (1, 1, 1),
                    "a changed deadline requires the same receipt's dispatch and published native proof"
                );
                assert!(state.expires_at > previous_expiry);
                return state;
            }
            assert!(
                Instant::now() < safety,
                "registered acquisition did not renew after observation"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    fn same_controller(before: &Value, after: &Value) {
        for field in [
            "terminal_id",
            "process_id",
            "process_run_uuid",
            "lease_id",
            "fence",
            "control_revision",
            "daemon_epoch",
            "input_seq",
        ] {
            assert_eq!(
                after.get(field).unwrap(),
                before.get(field).unwrap(),
                "renewal preserves {field}"
            );
        }
    }

    fn same_native(
        before: &unleash_process_host::HostManifest,
        after: &unleash_process_host::HostManifest,
    ) {
        assert_eq!(
            (
                &after.host_id,
                &after.process_id,
                &after.process_run_uuid,
                &after.pid,
                &after.pgid,
                &after.owner_uid,
                &after.boot_identity,
                &after.start_identity,
                &after.identity_backend,
                &after.socket_path
            ),
            (
                &before.host_id,
                &before.process_id,
                &before.process_run_uuid,
                &before.pid,
                &before.pgid,
                &before.owner_uid,
                &before.boot_identity,
                &before.start_identity,
                &before.identity_backend,
                &before.socket_path
            )
        );
        assert_eq!(after.daemon_epoch, before.daemon_epoch);
        assert_eq!(after.highest_fence, before.highest_fence);
    }

    fn assert_known_deadline(
        before: &Value,
        after: &Value,
        state: &RenewalSnapshot,
        sent_at: i64,
        completed_at: i64,
        ttl: i64,
    ) {
        assert!(state.expires_at >= sent_at.checked_add(ttl).unwrap());
        assert!(state.expires_at <= completed_at.checked_add(ttl).unwrap());
        same_controller(before, after);
        assert_eq!(after.get("reused"), Some(&json!(true)));
        assert_eq!(
            after.get("lease_expires_at"),
            Some(&json!(state.expires_at))
        );
        assert_eq!(
            after.pointer("/cursor/sequence").and_then(Value::as_i64),
            Some(state.proof_sequence)
        );
        assert_eq!(state.terminal_leases, 1);
    }

    async fn exact_replay(
        fixture: &mut LiveFixture,
        request: &CommandEnvelope,
        receipt: &unleash_protocol::CommandReceipt,
        committed: &RenewalSnapshot,
        after: &Value,
    ) {
        cross_utc_boundary(utc_ms().checked_add(100).unwrap(), committed.expires_at).await;
        assert_eq!(
            fixture.client.send_command(request).await.unwrap(),
            *receipt,
            "exact replay returns the original receipt after UTC has advanced"
        );
        assert_eq!(
            snapshot(fixture, receipt),
            *committed,
            "replay never extends or appends another action/proof"
        );
        assert_eq!(result(fixture, receipt).await, *after);
    }

    async fn stop_and_check_quiescent_replay(
        fixture: &mut LiveFixture,
        receipt: &unleash_protocol::CommandReceipt,
        committed: &RenewalSnapshot,
    ) {
        let revision = fixture
            .snapshot()
            .await
            .pointer("/daemon_state/revision")
            .and_then(Value::as_u64)
            .unwrap();
        let stop = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_7006),
            command_id: CommandId::from_u128(0xf011_7006),
            authority_generation: fixture.authority,
            actor_ref: None,
            project_id: None,
            aggregate_id: fixture.authority.as_uuid(),
            expected_revision: Revision::new(revision),
            command_type: "StopDaemon".into(),
            payload: object(json!({"reason":"renewal replay drain","drain_timeout_ms":10000})),
        };
        assert_eq!(
            fixture.client.send_command(&stop).await.unwrap().state,
            ReceiptState::Accepted
        );
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = fixture.daemon.child.try_wait().unwrap() {
                assert!(status.success(), "the daemon must finish the actual drain");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "drain did not close the writer and broker tasks"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // The default drain changes daemon mode, not resource lease deadlines.
        // Reaping the owner closes its writer and broker tasks before this read.
        assert_eq!(
            snapshot(fixture, receipt),
            *committed,
            "no deferred replay task may extend or publish after the immediate checks"
        );
    }

    #[tokio::test]
    async fn t_f011_registered_acquisition_same_daemon_renews_once_after_proof() {
        let mut fixture = LiveFixture::new(true).await;
        exercise(&mut fixture, None).await;
    }

    pub(super) async fn exercise(
        fixture: &mut LiveFixture,
        barrier: Option<&tokio::net::UnixListener>,
    ) {
        let initial_log = fixture.start_terminal().await;
        let original_log = durable_log(fixture, initial_log).await;
        let allocation = fixture
            .client
            .get_command_receipt(
                RequestId::from_u128(0xf011_7003),
                CommandId::from_u128(0xf011_4105),
            )
            .await
            .unwrap()
            .receipt;
        let before = result(fixture, &allocation).await;
        let before_native = fixture.host_manifest();
        let daemon_pid = fixture.daemon.child.id();
        let before_expiry = before
            .get("lease_expires_at")
            .and_then(Value::as_i64)
            .unwrap();
        let ttl = ttl(fixture).await;
        let anchor = before_expiry.checked_sub(ttl).unwrap();
        cross_utc_boundary(
            utc_ms().max(anchor).checked_add(100).unwrap(),
            before_expiry,
        )
        .await;
        let log_after_boundary = fixture
            .client
            .query(&QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xf011_7007),
                query_type: "ReadTerminalLog".into(),
                project_id: Some(fixture.project),
                payload: object(json!({"terminal_id":before.get("terminal_id").unwrap(),
                "after_offset":0,"max_bytes":65536})),
            })
            .await
            .unwrap();
        assert!(
            log_after_boundary.result.get("log").is_some(),
            "the post-boundary passive operation must be a real host log read"
        );
        assert_eq!(
            result(fixture, &allocation).await.get("lease_expires_at"),
            Some(&json!(before_expiry)),
            "output and causal reads never renew a lease"
        );
        let request = command(
            fixture.authority,
            0xf011_7004,
            fixture.project,
            allocation.aggregate_id,
            before
                .get("control_revision")
                .and_then(Value::as_u64)
                .unwrap(),
            "AcquireTerminal",
            json!({"terminal_id":before.get("terminal_id").unwrap(),
                "process_run_uuid":before.get("process_run_uuid").unwrap(),
                "scope_revision":2,"control_mode":"exclusive"}),
        );
        let sent_at = utc_ms();
        let accepted = fixture
            .client
            .send_command(&request)
            .await
            .expect("the same authenticated holder must be able to renew in the same daemon");
        assert_eq!(accepted.state, ReceiptState::Accepted);
        if let Some(barrier) = barrier {
            let mut blocked = renewal_fixture::wait_before_commit(barrier).await;
            let pending = snapshot(fixture, &accepted);
            assert_eq!(
                pending.expires_at, before_expiry,
                "native observation cannot grant time before its commit"
            );
            assert_eq!(
                (pending.actions, pending.dispatches, pending.proofs),
                (1, 1, 0)
            );
            assert_eq!(
                pending.lease_id,
                before.get("lease_id").unwrap().as_str().unwrap()
            );
            assert_eq!(
                pending.revision,
                before.get("control_revision").unwrap().as_i64().unwrap()
            );
            tokio::io::AsyncWriteExt::write_all(&mut blocked, &[1])
                .await
                .unwrap();
        }
        let committed = wait_observed(fixture, &accepted, before_expiry).await;
        let observed_at = utc_ms();
        let after = result(fixture, &accepted).await;
        assert_known_deadline(&before, &after, &committed, sent_at, observed_at, ttl);
        exact_replay(fixture, &request, &accepted, &committed, &after).await;
        assert_eq!(fixture.daemon.child.id(), daemon_pid);
        assert!(fixture.daemon.child.try_wait().unwrap().is_none());
        same_native(&before_native, &fixture.host_manifest());
        assert_preserved_output(fixture, &original_log).await;
        assert_eq!(
            std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
            b"started\n"
        );
        assert!(
            !fixture.project_root.path().join("worker-effects").exists(),
            "renewal never sends input"
        );
        stop_and_check_quiescent_replay(fixture, &accepted, &committed).await;
    }
}

struct LiveFixture {
    daemon: FixtureDaemon,
    client: UnleashClient,
    data: tempfile::TempDir,
    rendezvous: tempfile::TempDir,
    project_root: tempfile::TempDir,
    project: ProjectId,
    mission: Uuid,
    authority: AuthorityGeneration,
}
impl Drop for LiveFixture {
    fn drop(&mut self) {
        // A failure between killing the old daemon and starting its successor
        // still cleans up the fixture's authenticated host and owned child.
        self.daemon.cleanup_hosts = true;
    }
}
impl LiveFixture {
    async fn empty() -> Self {
        Self::empty_with(FixtureDaemon::spawn).await
    }
    async fn empty_with(spawn: impl FnOnce(&Path, &Path) -> FixtureDaemon) -> Self {
        let data = local_test_dir("f011-live-data").unwrap();
        let rendezvous = local_test_dir("f011-live-rdv").unwrap();
        let project_root = local_test_dir("f011-live-root").unwrap();
        std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let daemon = spawn(data.path(), rendezvous.path());
        daemon.readiness();
        let client = UnleashClient::connect(ClientConfig::new(
            rendezvous.path().join("daemon.sock"),
            ClientKind::Cli,
            "f011-real-recovery",
        ))
        .await
        .unwrap();
        let authority = client.hello().authority_generation;
        let project = ProjectId::from_u128(0xf011_4001);
        let mission = Uuid::from_u128(0xf011_4002);
        Self {
            daemon,
            client,
            data,
            rendezvous,
            project_root,
            project,
            mission,
            authority,
        }
    }

    async fn new(configured: bool) -> Self {
        let mut fixture = Self::empty().await;
        fixture.provision(configured).await;
        fixture
    }
    async fn provision(&mut self, configured: bool) {
        let fixture = self;
        let (authority, project, mission) = (fixture.authority, fixture.project, fixture.mission);
        let root = fixture.project_root.path().canonicalize().unwrap();
        for request in [
            command(
                authority,
                0xf011_4101,
                project,
                project.as_uuid(),
                0,
                "CreateProject",
                json!({"name":"real terminal recovery","weight":1}),
            ),
            command(
                authority,
                0xf011_4102,
                project,
                project.as_uuid(),
                1,
                "ReviseProject",
                json!({"name":"real terminal recovery","description":"","weight":1,"primary_root_path":root,"sources":[{"kind":"directory","path_or_uri":root,"label":"owned test root","read_only":false}],"repositories":[],"settings_json":{}}),
            ),
            command(
                authority,
                0xf011_4103,
                project,
                mission,
                0,
                "AssignMission",
                json!({}),
            ),
        ] {
            assert_eq!(
                fixture.client.send_command(&request).await.unwrap().state,
                ReceiptState::Accepted
            );
        }
        if configured {
            fixture.configure().await;
        }
    }
    async fn configure(&mut self) {
        let mut request = command(
            self.authority,
            0xf011_4104,
            self.project,
            self.authority.as_uuid(),
            0,
            "SetSettings",
            json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":terminal_configuration()}}),
        );
        request.project_id = None;
        assert_eq!(
            self.client.send_command(&request).await.unwrap().state,
            ReceiptState::Accepted
        );
    }
    fn allocation(&self, id: u128) -> CommandEnvelope {
        command(
            self.authority,
            id,
            self.project,
            Uuid::from_u128(0xf011_4003),
            0,
            "AcquireTerminal",
            json!({"mission_id":self.mission,"invocation_id":null,"purpose_key":"real-recovery","reuse_policy":"reuse_compatible","scope_id":self.project.to_string(),"scope_revision":2,"worktree_id":null,"cwd":".","program":std::env::current_exe().unwrap(),"arguments":["--ignored","--exact","reattach_scenarios::terminal_worker_entrypoint","--nocapture"],"environment_profile_id":"terminal-test","pty":{"rows":24,"cols":80},"readiness":{"kind":"process_started","timeout_ms":10000}}),
        )
    }
    async fn start_terminal(&mut self) -> Value {
        let request = self.allocation(0xf011_4105);
        assert_eq!(
            self.client.send_command(&request).await.unwrap().state,
            ReceiptState::Accepted
        );
        let deadline = Instant::now() + DEADLINE;
        loop {
            match self.client.query(&QueryEnvelope {protocol_version:ProtocolVersion,request_id:RequestId::from_u128(0xf011_4106),query_type:"ReadTerminalLog".into(),project_id:Some(self.project),payload:object(json!({"terminal_id":request.aggregate_id,"after_offset":0,"max_bytes":65536}))}).await {
                Ok(reply) if reply.result.get("text").and_then(Value::as_str).is_some_and(|text|text.contains("live-terminal-ready"))=>return reply.result,
                Ok(_)=>{},
                Err(unleash_client::ClientError::Remote(error)) if error.code==unleash_protocol::ErrorCode::ResourceBusy && error.retry_class==unleash_protocol::RetryClass::AfterRefresh=>{},
                Err(error)=>panic!("initial real PTY observation failed: {error:?}; {}",terminal_diagnostic(self.data.path())),
            }
            assert!(
                Instant::now() < deadline,
                "real child readiness deadline: {}",
                terminal_diagnostic(self.data.path())
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    async fn restart_daemon_only(&mut self) {
        self.daemon.kill_for_restart();
        self.start_successor().await;
    }
    async fn start_successor(&mut self) {
        let next = FixtureDaemon::spawn(self.data.path(), self.rendezvous.path());
        next.readiness();
        self.daemon = next;
        self.client = UnleashClient::connect(ClientConfig::new(
            self.rendezvous.path().join("daemon.sock"),
            ClientKind::Cli,
            "f011-real-successor",
        ))
        .await
        .unwrap();
        assert_eq!(self.client.hello().authority_generation, self.authority);
    }
    fn host_manifest(&self) -> unleash_process_host::HostManifest {
        let mut hosts = std::fs::read_dir(self.data.path().join("process-hosts")).unwrap();
        let runtime = hosts.next().unwrap().unwrap().path();
        assert!(hosts.next().is_none(), "one actual host directory");
        unleash_process_host::HostManifest::load_private(
            &runtime.join(unleash_process_host::HOST_MANIFEST_FILENAME),
        )
        .unwrap()
    }
    async fn snapshot(&mut self) -> Value {
        self.client
            .get_state_snapshot(
                RequestId::from_u128(0xf011_4107),
                Some(self.project),
                Map::new(),
            )
            .await
            .unwrap()
            .0
    }
}

fn assert_no_terminal_effects(fixture: &LiveFixture) {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for table in [
        "terminal_allocations",
        "actions",
        "broker_dispatches",
        "resource_leases",
        "capacity_reservations",
        "processes",
        "terminal_sessions",
    ] {
        let count = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(count, 0, "no terminal effect in {table}");
    }
    assert!(
        !fixture.data.path().join("process-hosts").exists(),
        "no host runtime or bootstrap created"
    );
}

#[tokio::test]
async fn t_f011_001_real_daemon_refuses_missing_configuration_scope_paths_and_proof_before_effect()
{
    let mut fixture = LiveFixture::new(false).await;
    let request = fixture.allocation(0xf011_4301);
    let refused = fixture.client.send_command(&request).await.unwrap_err();
    assert!(
        matches!(refused,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::InvalidArgument && error.message.contains("terminal runtime is not configured"))
    );
    assert_no_terminal_effects(&fixture);
    fixture.configure().await;
    let outside = local_test_dir("f011-owned-outside").unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        fixture.project_root.path().join("outside-link"),
    )
    .unwrap();
    for (id, key, value) in [
        (
            0xf011_4302,
            "scope_id",
            Value::String(ProjectId::from_u128(0xf011_ffff).to_string()),
        ),
        (0xf011_4303, "cwd", Value::String("../".into())),
        (0xf011_4304, "cwd", Value::String("outside-link".into())),
    ] {
        let mut request = fixture.allocation(id);
        request.payload.insert(key.into(), value);
        let refused = fixture.client.send_command(&request).await.unwrap_err();
        assert!(
            matches!(refused,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::InvalidArgument)
        );
        assert_no_terminal_effects(&fixture);
    }
    let mut unprivileged = UnleashClient::connect(
        ClientConfig::new(
            fixture.rendezvous.path().join("daemon.sock"),
            ClientKind::Cli,
            "f011-no-admin-proof",
        )
        .without_admin_credential(),
    )
    .await
    .unwrap();
    let nominal = fixture.allocation(0xf011_4305);
    let refused = unprivileged.send_command(&nominal).await.unwrap_err();
    assert!(
        matches!(refused,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::CapabilityDenied && error.reason_code==Some(unleash_protocol::ReasonCode::OperationNotAuthorized))
    );
    assert_no_terminal_effects(&fixture);
}

#[test]
#[ignore = "owned PTY worker fixture selected by exact argv only"]
fn terminal_worker_entrypoint() {
    let mut starts = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("worker-starts")
        .unwrap();
    starts.write_all(b"started\n").unwrap();
    starts.sync_all().unwrap();
    writeln!(std::io::stdout().lock(), "live-terminal-ready").unwrap();
    std::io::stdout().flush().unwrap();
    std::thread::spawn(|| {
        loop {
            writeln!(std::io::stdout().lock(), "live-terminal-output").unwrap();
            let _ = std::io::stdout().flush();
            std::thread::sleep(Duration::from_millis(25));
        }
    });
    for line in std::io::stdin().lock().lines() {
        if line.unwrap() == "apply-effect" {
            let mut effects = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("worker-effects")
                .unwrap();
            effects.write_all(b"effect\n").unwrap();
            effects.sync_all().unwrap();
            writeln!(std::io::stdout().lock(), "effect-recorded").unwrap();
            std::io::stdout().flush().unwrap();
        }
    }
}

#[tokio::test]
async fn t_f011_003_successor_reads_control_revision_without_taking_control() {
    let mut fixture = LiveFixture::new(true).await;
    let original = fixture.start_terminal().await;
    let before = fixture.host_manifest();
    let old_epoch = fixture.client.hello().daemon_epoch;
    fixture.restart_daemon_only().await;
    assert!(fixture.client.hello().daemon_epoch > old_epoch);
    let snapshot = fixture.snapshot().await;
    let terminal_id = original.get("terminal_id").unwrap();
    let item = snapshot
        .pointer("/terminals/items")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|item| item.get("terminal_id") == Some(terminal_id))
        .expect("owner sees the existing terminal even under a suspect lease");
    assert_eq!(
        item.get("control_revision").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        item.get("lease_state").and_then(Value::as_str),
        Some("suspect")
    );
    assert_eq!(
        item.get("process_run_uuid"),
        Some(original.get("process_run_uuid").unwrap())
    );
    assert!(item.get("lease_id").is_none() && item.get("fence").is_none());
    let after = fixture.host_manifest();
    assert_eq!(after.process_run_uuid, before.process_run_uuid);
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.start_identity, before.start_identity);
    assert_eq!(
        after.daemon_epoch, before.daemon_epoch,
        "a metadata read never handshakes or transfers authority"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.project_root.path().join("worker-starts")).unwrap(),
        "started\n"
    );
}

#[tokio::test]
async fn t_f011_003_snapshot_hides_incomplete_identity_without_contacting_host() {
    for injection in [
        "UPDATE processes SET pid=NULL",
        "UPDATE processes SET start_identity=NULL",
        "UPDATE actions SET action_type='different-effect'",
        "UPDATE resource_leases SET owner_id='foreign-controller'",
        "UPDATE resource_leases SET boot_identity='foreign-boot'",
    ] {
        let mut fixture = LiveFixture::new(true).await;
        fixture.start_terminal().await;
        let before = fixture.host_manifest();
        fixture.daemon.kill_for_restart();
        // No daemon writer is alive while injecting this incomplete/foreign
        // historical registration after a genuine observed launch.
        {
            let connection =
                rusqlite::Connection::open(fixture.data.path().join("unleash-authority.db"))
                    .unwrap();
            assert_eq!(connection.execute(injection, []).unwrap(), 1);
        }
        fixture.start_successor().await;
        assert!(
            fixture
                .snapshot()
                .await
                .pointer("/terminals/items")
                .and_then(Value::as_array)
                .unwrap()
                .is_empty(),
            "inadmissible registration is excluded: {injection}"
        );
        let after = fixture.host_manifest();
        assert_eq!(after.daemon_epoch, before.daemon_epoch);
        assert_eq!(after.highest_fence, before.highest_fence);
        assert_eq!(after.process_run_uuid, before.process_run_uuid);
    }
}

#[tokio::test]
async fn t_f011_003_registered_acquisition_after_real_daemon_crash_reuses_the_same_pty() {
    let mut fixture = LiveFixture::new(true).await;
    let original = fixture.start_terminal().await;
    let terminal_id = original.get("terminal_id").expect("observed terminal id");
    let process_run_uuid = original
        .get("process_run_uuid")
        .expect("observed process run id");
    let before = fixture.host_manifest();
    let (old_secret, mut old_channel) = open_controller(&before);
    fixture.restart_daemon_only().await;
    let request = command(
        fixture.authority,
        0xf011_4201,
        fixture.project,
        Uuid::from_u128(0xf011_4003),
        0,
        "AcquireTerminal",
        json!({"terminal_id":terminal_id,"process_run_uuid":process_run_uuid,"scope_revision":2,"control_mode":"exclusive"}),
    );
    let accepted = fixture
        .client
        .send_command(&request)
        .await
        .expect("registered acquisition must be accepted by the real successor daemon");
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let deadline = Instant::now() + DEADLINE;
    loop {
        let snapshot = fixture.snapshot().await;
        let observed = snapshot
            .pointer("/terminals/items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("terminal_id") == Some(terminal_id))
            });
        if observed.is_some_and(|item| {
            item.get("lease_state").and_then(Value::as_str) == Some("held")
                && item.get("control_revision").and_then(Value::as_u64) == Some(1)
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual control activation deadline"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap(),
        accepted,
        "exact wire retry does not transfer again"
    );
    let after = fixture.host_manifest();
    assert_eq!(after.host_id, before.host_id);
    assert_eq!(after.process_run_uuid, before.process_run_uuid);
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.pgid, before.pgid);
    assert_eq!(after.start_identity, before.start_identity);
    assert_eq!(after.boot_identity, before.boot_identity);
    assert_eq!(
        after.allocation_context_digest,
        before.allocation_context_digest
    );
    assert!(after.daemon_epoch > before.daemon_epoch);
    assert!(after.highest_fence > before.highest_fence);
    assert_closed_old_controller(&before, &after, &old_secret, &mut old_channel);
    assert_preserved_output(&mut fixture, &original).await;
    assert_eq!(
        std::fs::read_to_string(fixture.project_root.path().join("worker-starts")).unwrap(),
        "started\n"
    );
}

fn open_controller(
    manifest: &unleash_process_host::HostManifest,
) -> (
    unleash_process_host::HostControlSecret,
    unleash_process_host::HostClient,
) {
    use unleash_process_host::{HostClient, HostRequest, HostResponse, load_control_secret};
    let secret = load_control_secret(&manifest.control_secret_path).unwrap();
    let mut client = HostClient::connect(
        &manifest.socket_path,
        manifest.process_run_uuid,
        manifest.authority_generation,
        manifest.daemon_epoch,
        manifest.highest_fence,
    )
    .unwrap();
    let header = client.make_header(
        unleash_domain::CommandId::from_u128(0xf011_4401),
        manifest.action_id,
        manifest.allocation_context_digest.unwrap(),
    );
    assert!(matches!(
        client
            .call(&HostRequest::Handshake {
                header,
                current_secret: secret.expose_secret().into(),
                rotate_to_secret: None,
                require_higher_epoch: false
            })
            .unwrap(),
        HostResponse::HandshakeAck { .. }
    ));
    (secret, client)
}

fn assert_closed_old_controller(
    before: &unleash_process_host::HostManifest,
    after: &unleash_process_host::HostManifest,
    old_secret: &unleash_process_host::HostControlSecret,
    old_channel: &mut unleash_process_host::HostClient,
) {
    use unleash_process_host::{HostClient, HostRequest, HostResponse, load_control_secret};
    let digest = before.allocation_context_digest.unwrap();
    let header = old_channel.make_header(
        unleash_domain::CommandId::from_u128(0xf011_4402),
        before.action_id,
        digest,
    );
    assert!(
        matches!(old_channel.call(&HostRequest::ReadLog {header,secret:old_secret.expose_secret().into(),after_offset:0,max_bytes:1024,strict_cursor:true}).unwrap(),HostResponse::Error {ref error} if error.code==unleash_protocol::ErrorCode::AuthenticationFailed)
    );
    let mut stale = HostClient::connect(
        &after.socket_path,
        after.process_run_uuid,
        after.authority_generation,
        after.daemon_epoch,
        after.highest_fence,
    )
    .unwrap();
    let header = stale.make_header(
        unleash_domain::CommandId::from_u128(0xf011_4403),
        before.action_id,
        digest,
    );
    assert!(
        matches!(stale.call(&HostRequest::ReadLog {header,secret:old_secret.expose_secret().into(),after_offset:0,max_bytes:1024,strict_cursor:true}).unwrap(),HostResponse::Error {ref error} if error.code==unleash_protocol::ErrorCode::AuthenticationFailed)
    );
    let current = load_control_secret(&after.control_secret_path).unwrap();
    let mut foreign = HostClient::connect(
        &after.socket_path,
        after.process_run_uuid,
        AuthorityGeneration::from_u128(0xf011_ffff),
        unleash_domain::DaemonEpoch::new(after.daemon_epoch.as_u64() + 100),
        unleash_domain::Fence::new(after.highest_fence.as_u64() + 100),
    )
    .unwrap();
    let header = foreign.make_header(
        unleash_domain::CommandId::from_u128(0xf011_4404),
        before.action_id,
        digest,
    );
    assert!(
        matches!(foreign.call(&HostRequest::Handshake {header,current_secret:current.expose_secret().into(),rotate_to_secret:None,require_higher_epoch:false}).unwrap(),HostResponse::Error {ref error} if error.reason_code==Some(unleash_protocol::ReasonCode::StaleAuthorityGeneration))
    );
}

async fn assert_preserved_output(fixture: &mut LiveFixture, original: &Value) {
    let terminal_id = original.get("terminal_id").expect("observed terminal id");
    let response = fixture
        .client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_4405),
            query_type: "ReadTerminalLog".into(),
            project_id: Some(fixture.project),
            payload: object(json!({"terminal_id":terminal_id,"after_offset":0,"max_bytes":65536})),
        })
        .await
        .expect("the successor reads the same PTY through its committed controller");
    let old: unleash_process_host::LogReadResult =
        serde_json::from_value(original.get("log").expect("original PTY log").clone()).unwrap();
    let new: unleash_process_host::LogReadResult = serde_json::from_value(
        response
            .result
            .get("log")
            .expect("successor PTY log")
            .clone(),
    )
    .unwrap();
    let bytes = |log: &unleash_process_host::LogReadResult| {
        log.segments
            .iter()
            .flat_map(|segment| segment.bytes.iter().copied())
            .collect::<Vec<_>>()
    };
    assert!(
        bytes(&new).starts_with(&bytes(&old)),
        "daemon crash preserves output bytes and offsets"
    );
    assert!(
        old.durable_watermark_offset > 0,
        "initial real output is durable before killing only daemon"
    );
    assert!(new.durable_watermark_offset >= old.durable_watermark_offset);
    for segment in old.segments.iter().filter(|segment| !segment.is_volatile) {
        let same = new
            .segments
            .iter()
            .find(|other| other.segment_seq == segment.segment_seq)
            .expect("old durable segment remains present");
        assert_eq!(
            (same.start_offset, same.end_offset, same.sha256),
            (segment.start_offset, segment.end_offset, segment.sha256)
        );
    }
}
