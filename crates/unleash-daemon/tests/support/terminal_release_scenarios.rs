//! T-F011-010 / REQ-TERM-010: release control, retain the actual process and PTY.
//! The causal disposition follows the existing `ActionObserved` result
//! pattern. No fixture updates the product ledger or manufactures a native ACK.
use super::super::super::resize_scenarios::{observed_acquisition, terminal_event};
use super::*;
use unleash_process_host::{HostClient, HostControlSecret, HostRequest, HostResponse};
use unleash_protocol::{ErrorCode, EventEnvelope};
use unleash_store::terminal_allocations::terminal_acquisition_query::TerminalAcquisitionResult;

#[path = "terminal_release_recovery_scenarios.rs"]
mod recovery_scenarios;

#[path = "terminal_release_c2_scenarios.rs"]
mod c2_scenarios;

async fn acquire(fixture: &mut LiveFixture) -> TerminalAcquisitionResult {
    fixture.start_terminal().await;
    let receipt = fixture
        .client
        .get_command_receipt(
            RequestId::from_u128(0xf011_8101),
            CommandId::from_u128(0xf011_4105),
        )
        .await
        .unwrap()
        .receipt;
    observed_acquisition(fixture, &receipt).await
}

fn release_request(
    fixture: &LiveFixture,
    acquired: &TerminalAcquisitionResult,
    id: u128,
) -> CommandEnvelope {
    command(
        fixture.authority,
        id,
        fixture.project,
        acquired.terminal_id.as_uuid(),
        acquired.control_revision.as_u64(),
        "ReleaseTerminal",
        json!({
            "terminal_id":acquired.terminal_id,"process_run_uuid":acquired.process_run_uuid,
            "lease_id":acquired.lease_id,"fence":acquired.fence,
        }),
    )
}

#[derive(Debug, PartialEq, Eq)]
struct ControlFacts {
    lease: String,
    expiry: i64,
    fence: i64,
    lease_state: String,
    revision: i64,
    processes: i64,
    active_holds: i64,
    leases: i64,
    dispatches: i64,
}

fn facts(fixture: &LiveFixture, acquired: &TerminalAcquisitionResult) -> ControlFacts {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT l.lease_id,l.expires_at_ms,l.fence,l.state,s.control_revision,
         (SELECT COUNT(*) FROM processes),
         (SELECT COUNT(*) FROM capacity_reservations WHERE state IN ('held','reconciling')),
         (SELECT COUNT(*) FROM resource_leases WHERE resource_kind='terminal'),
         (SELECT COUNT(*) FROM action_dispatches)
         FROM resource_leases l JOIN terminal_sessions s ON s.terminal_id=?2 WHERE l.lease_id=?1",
            [
                acquired.lease_id.to_string(),
                acquired.terminal_id.to_string(),
            ],
            |row| {
                Ok(ControlFacts {
                    lease: row.get(0)?,
                    expiry: row.get(1)?,
                    fence: row.get(2)?,
                    lease_state: row.get(3)?,
                    revision: row.get(4)?,
                    processes: row.get(5)?,
                    active_holds: row.get(6)?,
                    leases: row.get(7)?,
                    dispatches: row.get(8)?,
                })
            },
        )
        .unwrap()
}

fn no_receipt(fixture: &LiveFixture, request: &CommandEnvelope) {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM command_receipts WHERE command_id=?1",
            [request.command_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "rejected release must not prepare a new effect");
}

fn pending_marker(fixture: &LiveFixture, receipt: &CommandReceipt) -> Value {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection.query_row(
        "SELECT r.action_id,b.dispatch_id,b.evaluation_id,d.request_hash,p.phase_digest
         FROM terminal_renewal_intents r JOIN broker_dispatches b ON b.action_id=r.action_id
         JOIN action_dispatches d ON d.dispatch_id=b.dispatch_id AND d.action_id=r.action_id
         JOIN broker_phase_records p ON p.action_id=r.action_id AND p.evaluation_id=b.evaluation_id AND p.phase_key='dispatch'
         WHERE r.receipt_id=?1",
        [receipt.receipt_id.to_string()],
        |row| Ok(json!({"action_id":row.get::<_,String>(0)?,"dispatch_id":row.get::<_,String>(1)?,
            "evaluation_id":row.get::<_,String>(2)?,"request_hash":row.get::<_,String>(3)?,"phase_digest":row.get::<_,String>(4)?})),
    ).unwrap()
}

fn same_child(fixture: &LiveFixture, before: &unleash_process_host::HostManifest) {
    physical(before, &fixture.host_manifest());
    assert_eq!(fixture.host_manifest().launch_state, before.launch_state);
    let expected = unleash_platform::OsProcessIdentity {
        pid: before.pid.unwrap(),
        pgid: before.pgid.unwrap(),
        owner_uid: before.owner_uid,
        boot_identity: before.boot_identity.clone().unwrap(),
        start_identity: before.start_identity.clone().unwrap(),
        backend: before.identity_backend,
    };
    assert_eq!(
        unleash_platform::verify_process_identity(&expected).unwrap(),
        unleash_platform::ProcessVerification::Match
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n"
    );
    assert!(!fixture.project_root.path().join("worker-effects").exists());
}

struct OldNativeControl {
    client: HostClient,
    secret: HostControlSecret,
    source: unleash_process_host::HostManifest,
}

impl OldNativeControl {
    fn authenticate(fixture: &LiveFixture) -> Self {
        let source = fixture.host_manifest();
        let secret = unleash_process_host::load_control_secret(
            &source
                .socket_path
                .parent()
                .unwrap()
                .join(unleash_process_host::CONTROL_SECRET_FILENAME),
        )
        .unwrap();
        let mut client = HostClient::connect(
            &source.socket_path,
            source.process_run_uuid,
            source.authority_generation,
            source.daemon_epoch,
            source.highest_fence,
        )
        .unwrap();
        let header = client.make_header(
            CommandId::from_u128(0xf011_8102),
            source.action_id,
            source.command_digest,
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
        Self {
            client,
            secret,
            source,
        }
    }

    fn assert_revoked(&mut self) {
        self.expect_resize_refused(CommandId::from_u128(0xf011_8103));
        // Invalidating only the old connection is insufficient: the same old
        // secret and coordinates must not regain control on a fresh channel.
        self.client = HostClient::connect(
            &self.source.socket_path,
            self.source.process_run_uuid,
            self.source.authority_generation,
            self.source.daemon_epoch,
            self.source.highest_fence,
        )
        .unwrap();
        self.expect_resize_refused(CommandId::from_u128(0xf011_8104));
    }

    fn expect_resize_refused(&mut self, command: CommandId) {
        let header =
            self.client
                .make_header(command, self.source.action_id, self.source.command_digest);
        // A SQL-only release leaves this previously authenticated native channel
        // effective. A typed refusal, not EOF or a lost ACK, proves rejection.
        let response = self
            .client
            .call(&HostRequest::Resize {
                header,
                secret: self.secret.expose_secret().into(),
                rows: 43,
                cols: 119,
            })
            .unwrap();
        assert!(
            matches!(response, HostResponse::Error {error}
            if matches!(error.code, ErrorCode::AuthenticationFailed | ErrorCode::StaleFence | ErrorCode::CapabilityDenied)),
            "Release must retire the previously authenticated native control before reporting success"
        );
    }
}

async fn disposition(fixture: &LiveFixture, receipt: &CommandReceipt) -> (Value, EventEnvelope) {
    let event = terminal_event(fixture, receipt, |event| {
        event.event_type == "ActionObserved"
            && event
                .payload
                .get("terminal_release")
                .and_then(|result| result.get("receipt_id"))
                == Some(&json!(receipt.receipt_id))
    })
    .await;
    assert_eq!(event.payload.get("state"), Some(&json!("observed_success")));
    let result = event.payload.get("terminal_release").unwrap().clone();
    assert_eq!(result.get("command_id"), Some(&json!(receipt.command_id)));
    assert_eq!(result.get("disposition"), Some(&json!("released")));
    (result, event)
}

fn disposition_counts(fixture: &LiveFixture, receipt: &CommandReceipt) -> (i64, i64, i64) {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=e.aggregate_id),
         (SELECT COUNT(*) FROM broker_phase_records p WHERE p.action_id=e.aggregate_id AND p.phase_key LIKE 'observation:%'
          AND json_extract(p.phase_json,'$.phase.receipt.state')='observed_success'), COUNT(*)
         FROM events e WHERE e.event_type='ActionObserved'
          AND json_extract(e.payload_json,'$.terminal_release.receipt_id')=?1",
        [receipt.receipt_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap()
}

async fn release(
    fixture: &mut LiveFixture,
    acquired: &TerminalAcquisitionResult,
    id: u128,
) -> (CommandEnvelope, CommandReceipt, Value, EventEnvelope) {
    let request = release_request(fixture, acquired, id);
    let receipt = fixture
        .client
        .send_command(&request)
        .await
        .expect("the exact held terminal accepts a release intention");
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let (result, event) = disposition(fixture, &receipt).await;
    for (name, expected) in [
        ("terminal_id", json!(acquired.terminal_id)),
        ("process_run_uuid", json!(acquired.process_run_uuid)),
        ("lease_id", json!(acquired.lease_id)),
        ("fence", json!(acquired.fence)),
    ] {
        assert_eq!(result.get(name), Some(&expected));
    }
    (request, receipt, result, event)
}

async fn old_commands_refused(
    fixture: &mut LiveFixture,
    acquired: &TerminalAcquisitionResult,
    revision: Revision,
) {
    for (id, kind, extra) in [
        (
            0xf011_8110,
            "SendTerminalInput",
            json!({"input_seq":acquired.input_seq+1,"payload_bytes":b"apply-effect\n"}),
        ),
        (0xf011_8111, "ResizeTerminal", json!({"rows":37,"cols":112})),
    ] {
        let mut request = release_request(fixture, acquired, id);
        request.command_type = kind.into();
        request.expected_revision = revision;
        request.payload.extend(object(extra));
        assert!(matches!(fixture.client.send_command(&request).await,
            Err(unleash_client::ClientError::Remote(error)) if error.code==ErrorCode::StaleFence));
        no_receipt(fixture, &request);
    }
}

#[tokio::test]
async fn t_f011_010_release_disposes_control_and_replays_without_stopping_the_pty() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire(&mut fixture).await;
    let native = fixture.host_manifest();
    let mut old = OldNativeControl::authenticate(&fixture);
    let before = facts(&fixture, &acquired);
    assert_eq!(
        before.active_holds, 3,
        "the live host, child and PTY retain their distinct holds"
    );
    let (request, receipt, result, event) = release(&mut fixture, &acquired, 0xf011_8120).await;
    let after = facts(&fixture, &acquired);
    assert_eq!(after.lease_state, "released");
    assert_eq!(
        (
            &after.lease,
            after.expiry,
            after.fence,
            after.processes,
            after.active_holds,
            after.leases
        ),
        (
            &before.lease,
            before.expiry,
            before.fence,
            before.processes,
            before.active_holds,
            before.leases
        )
    );
    assert_eq!(after.dispatches, before.dispatches + 1);
    assert_eq!(disposition_counts(&fixture, &receipt), (1, 1, 1));
    assert_eq!(result.get("control_revision"), Some(&json!(after.revision)));
    old.assert_revoked();
    old_commands_refused(
        &mut fixture,
        &acquired,
        Revision::new(u64::try_from(after.revision).unwrap()),
    )
    .await;
    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_8121), request.command_id)
            .await
            .unwrap()
            .receipt,
        receipt
    );
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap(),
        receipt
    );
    let (replayed_result, replayed_event) = disposition(&fixture, &receipt).await;
    assert_eq!(replayed_result, result);
    assert_eq!(
        serde_json::to_value(replayed_event).unwrap(),
        serde_json::to_value(event).unwrap()
    );
    drain(&mut fixture).await;
    assert_eq!(disposition_counts(&fixture, &receipt), (1, 1, 1));
    assert_eq!(facts(&fixture, &acquired), after);
    same_child(&fixture, &native);
}

#[tokio::test]
#[ignore = "requires UNLEASH_TEST_CLI_BIN and the process host built from the same tree"]
async fn t_f011_cli_release_returns_receipt_and_keeps_the_terminal_open() {
    let binary = PathBuf::from(std::env::var_os("UNLEASH_TEST_CLI_BIN").unwrap());
    assert!(binary.is_absolute() && binary.is_file());
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire(&mut fixture).await;
    let native = fixture.host_manifest();
    let before = facts(&fixture, &acquired);
    let socket = fixture.rendezvous.path().join("daemon.sock");
    let wire = release_request(&fixture, &acquired, 0xf011_8170);
    let options = [
        ("--project-id", fixture.project.to_string()),
        ("--terminal-id", acquired.terminal_id.to_string()),
        ("--process-run-uuid", acquired.process_run_uuid.to_string()),
        ("--lease-id", acquired.lease_id.to_string()),
        ("--fence", acquired.fence.to_string()),
        (
            "--control-revision",
            acquired.control_revision.as_u64().to_string(),
        ),
        ("--command-id", wire.command_id.to_string()),
        ("--authority-generation", fixture.authority.to_string()),
    ];
    let send = || async {
        let mut command = tokio::process::Command::new(&binary);
        command
            .arg("--socket")
            .arg(&socket)
            .args(["--json", "process", "release"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        for (flag, value) in &options {
            command.arg(flag).arg(value);
        }
        let output = tokio::time::timeout(Duration::from_secs(10), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "CLI release: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        serde_json::from_value::<CommandReceipt>(response.get("receipt").unwrap().clone()).unwrap()
    };
    let receipt = send().await;
    assert_eq!(receipt.command_id, wire.command_id);
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let (result, _) = disposition(&fixture, &receipt).await;
    assert_eq!(result.get("disposition"), Some(&json!("released")));
    assert_eq!(send().await, receipt);
    drain(&mut fixture).await;
    let after = facts(&fixture, &acquired);
    assert_eq!(after.lease_state, "released");
    assert_eq!(after.dispatches, before.dispatches + 1);
    assert_eq!(after.active_holds, before.active_holds);
    same_child(&fixture, &native);
}

#[tokio::test]
async fn t_f011_010_released_terminal_can_be_explicitly_reacquired_without_spawn() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire(&mut fixture).await;
    let native = fixture.host_manifest();
    let (_, _, released, _) = release(&mut fixture, &acquired, 0xf011_8130).await;
    let request = command(
        fixture.authority,
        0xf011_8131,
        fixture.project,
        acquired.terminal_id.as_uuid(),
        released
            .get("control_revision")
            .and_then(Value::as_u64)
            .unwrap(),
        "AcquireTerminal",
        json!({"terminal_id":acquired.terminal_id,"process_run_uuid":acquired.process_run_uuid,"scope_revision":2,"control_mode":"exclusive"}),
    );
    let receipt = fixture
        .client
        .send_command(&request)
        .await
        .expect("released PTY admits a distinct explicit acquisition");
    let current = observed_acquisition(&mut fixture, &receipt).await;
    assert_eq!(current.process_run_uuid, acquired.process_run_uuid);
    assert_ne!(current.lease_id, acquired.lease_id);
    assert!(current.fence > acquired.fence);
    assert_eq!(facts(&fixture, &acquired).lease_state, "released");
    old_commands_refused(&mut fixture, &acquired, current.control_revision).await;
    same_child(&fixture, &native);
}

#[tokio::test]
async fn t_f011_010_release_rejects_wrong_scope_actor_generation_and_fence_without_effect() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire(&mut fixture).await;
    let native = fixture.host_manifest();
    let before = facts(&fixture, &acquired);
    let mut wrong_scope = release_request(&fixture, &acquired, 0xf011_8140);
    wrong_scope.project_id = Some(ProjectId::from_u128(0xf011_ffff));
    assert!(matches!(fixture.client.send_command(&wrong_scope).await,
        Err(unleash_client::ClientError::Remote(error)) if matches!(error.code,ErrorCode::NotFoundOrHidden|ErrorCode::ScopeDenied)));
    no_receipt(&fixture, &wrong_scope);
    let mut wrong_actor = release_request(&fixture, &acquired, 0xf011_8141);
    wrong_actor.actor_ref = Some("foreign-terminal-owner".into());
    assert!(matches!(fixture.client.send_command(&wrong_actor).await,
        Err(unleash_client::ClientError::Remote(error)) if error.code==ErrorCode::AuthenticationFailed));
    no_receipt(&fixture, &wrong_actor);
    for (id, field, value) in [
        (
            0xf011_8142,
            "process_run_uuid",
            json!(Uuid::from_u128(0xf011_eeee)),
        ),
        (0xf011_8143, "fence", json!(acquired.fence.as_u64() + 1)),
    ] {
        let mut request = release_request(&fixture, &acquired, id);
        request.payload.insert(field.into(), value);
        assert!(matches!(fixture.client.send_command(&request).await,
            Err(unleash_client::ClientError::Remote(error)) if error.code==ErrorCode::StaleFence));
        no_receipt(&fixture, &request);
    }
    assert_eq!(facts(&fixture, &acquired), before);
    same_child(&fixture, &native);
}

async fn pending_release_refused(restart: bool) {
    let mut harness = harness(true).await;
    let fixture = &mut harness.fixture;
    let acquired = acquire(fixture).await;
    let native = fixture.host_manifest();
    let original = serde_json::to_value(&acquired).unwrap();
    unleash_platform::write_private_file_atomic(&harness.paths.arm, b"armed").unwrap();
    let pending = fixture
        .client
        .send_command(&registered(fixture, &original, 0xf011_8150))
        .await
        .unwrap();
    let mut retained = Some(receive(harness.listener.as_ref().unwrap(), DISPATCH).await);
    if restart {
        fixture.daemon.kill_for_restart();
        std::fs::remove_file(&harness.paths.arm).unwrap();
        drop(retained.take());
        fixture.start_successor().await;
        unchanged_unknown(fixture, &pending, acquired.lease_expires_at.as_i64());
    }
    let before = facts(fixture, &acquired);
    let unresolved = state(fixture, &pending);
    let marker = pending_marker(fixture, &pending);
    let native_count = count(&harness.paths);
    assert_eq!((unresolved.dispatches, unresolved.proofs), (1, 0));
    if !restart {
        assert_eq!(
            (unresolved.intent.as_str(), unresolved.action.as_str()),
            ("prepared", "dispatching")
        );
    }
    let request = release_request(fixture, &acquired, 0xf011_8151);
    assert!(matches!(fixture.client.send_command(&request).await,
        Err(unleash_client::ClientError::Remote(error))
        if error.code==ErrorCode::ResourceBusy || (restart && error.code==ErrorCode::StaleFence)));
    no_receipt(fixture, &request);
    assert_eq!(facts(fixture, &acquired), before);
    assert_eq!(
        state(fixture, &pending),
        unresolved,
        "Release must preserve the exact unresolved intention"
    );
    assert_eq!(pending_marker(fixture, &pending), marker);
    assert_eq!(
        count(&harness.paths),
        native_count,
        "a refused release must not invoke a native handshake"
    );
    if restart {
        unchanged_unknown(fixture, &pending, acquired.lease_expires_at.as_i64());
    }
    same_child(fixture, &native);
    if !restart {
        fixture.daemon.kill_for_restart();
        std::fs::remove_file(&harness.paths.arm).unwrap();
        drop(retained.take());
    }
}

#[tokio::test]
async fn t_f011_010_release_cannot_dispose_a_pending_dispatch() {
    pending_release_refused(false).await;
}

#[tokio::test]
async fn t_f011_010_release_cannot_dispose_an_unknown_dispatch_after_restart() {
    pending_release_refused(true).await;
}

fn release_policy_snapshot(connection: &rusqlite::Connection) -> ([i64; 6], String) {
    let counts = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM command_receipts),
             (SELECT COUNT(*) FROM terminal_release_intents),
             (SELECT COUNT(*) FROM actions),
             (SELECT COUNT(*) FROM broker_phase_records),
             (SELECT COUNT(*) FROM action_dispatches),
             (SELECT COUNT(*) FROM broker_dispatches)",
            [],
            |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ])
            },
        )
        .unwrap();
    let reservations = connection
        .query_row(
            "SELECT json_group_array(json_object(
               'id',reservation_id,'pool',pool_id,'state',state,'amount',amount,
               'owner_type',owner_type,'owner_id',owner_id,
               'authority_generation',authority_generation,'daemon_epoch',daemon_epoch))
             FROM (SELECT * FROM capacity_reservations ORDER BY reservation_id)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    (counts, reservations)
}

#[tokio::test]
async fn t_f011_010_strict_policy_refuses_release_before_receipt_without_effect() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire(&mut fixture).await;
    let mut old = OldNativeControl::authenticate(&fixture);
    let mut policy = command(
        fixture.authority,
        0xf011_8190,
        fixture.project,
        fixture.authority.as_uuid(),
        0,
        "SetPolicy",
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"strict",
            "policy_version":"release-strict-qualification",
            "reason":"strict policy committed before release"}),
    );
    policy.project_id = None;
    let policy_receipt = fixture.client.send_command(&policy).await.unwrap();
    assert_eq!(policy_receipt.state, ReceiptState::Accepted);
    assert_eq!(policy_receipt.new_revision, Revision::new(1));

    // Baseline AFTER SetPolicy: its own receipt must not pollute the delta.
    let native = fixture.host_manifest();
    assert!(matches!(
        native.control_state,
        Some(unleash_process_host::HostControlState::Active { .. })
    ));
    let before = facts(&fixture, &acquired);
    assert_eq!(before.lease_state, "held");
    assert_eq!(before.active_holds, 3);
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before_ledger = release_policy_snapshot(&connection);
    let request = release_request(&fixture, &acquired, 0xf011_8191);
    let response = fixture.client.send_command(&request).await;
    assert!(
        matches!(response,
        Err(unleash_client::ClientError::Remote(ref error))
        if error.code == ErrorCode::IsolationUnavailable
            && error.reason_code == Some(
                unleash_protocol::ReasonCode::IsolationObligationUnenforceable)
            && error.command_id == Some(request.command_id)),
        "Strict must refuse ReleaseTerminal before any Accepted receipt"
    );
    no_receipt(&fixture, &request);
    assert_eq!(
        release_policy_snapshot(&connection),
        before_ledger,
        "refusal leaves receipts, intents, actions, phases, dispatches and reservations intact"
    );
    assert_eq!(
        facts(&fixture, &acquired),
        before,
        "same lease, expiry, fence, revision, processes and holds"
    );
    same_child(&fixture, &native);
    let after_native = fixture.host_manifest();
    assert_eq!(after_native.control_state, native.control_state);
    assert_eq!(after_native.daemon_epoch, native.daemon_epoch);
    assert_eq!(after_native.highest_fence, native.highest_fence);
    let current_secret = unleash_process_host::load_control_secret(
        &native
            .socket_path
            .parent()
            .unwrap()
            .join(unleash_process_host::CONTROL_SECRET_FILENAME),
    )
    .unwrap();
    assert!(
        old.secret.expose_secret() == current_secret.expose_secret(),
        "refusal must not rotate the native capability"
    );
    let header = old.client.make_header(
        CommandId::from_u128(0xf011_8192),
        old.source.action_id,
        old.source.command_digest,
    );
    assert!(
        matches!(
            old.client
                .call(&HostRequest::Handshake {
                    header,
                    current_secret: old.secret.expose_secret().into(),
                    rotate_to_secret: None,
                    require_higher_epoch: false,
                })
                .unwrap(),
            HostResponse::HandshakeAck { .. }
        ),
        "the already authenticated native control is still effective"
    );
}
