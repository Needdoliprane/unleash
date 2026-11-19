//! Two distinct daemon-only losses. Neither case claims recovery of an
//! ambiguous successor-transfer ACK, nor the packaged-host injection path.
use super::*;
use unleash_protocol::CommandReceipt;

struct Harness {
    fixture: LiveFixture,
    paths: Paths,
    listener: Option<tokio::net::UnixListener>,
    _controller: tempfile::TempDir,
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.paths.arm);
        // Release even an unaccepted queued barrier before authenticated host
        // cleanup; runtime credentials/counter files remain alive until last.
        drop(self.listener.take());
    }
}

async fn harness(before: bool) -> Harness {
    let controller = local_test_dir("renewal-owned-controller").unwrap();
    std::fs::set_permissions(controller.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = Paths {
        counter: controller.path().join("native-count"),
        arm: controller.path().join("armed"),
        barrier: controller.path().join("barrier.sock"),
    };
    let listener = unleash_platform::bind_private_unix_listener(&paths.barrier).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
    let mut fixture = LiveFixture::empty_with(|data, rdv| {
        FixtureDaemon::spawn_driver(
            if before { BEFORE } else { AFTER },
            data,
            &[
                (DATA, data),
                (RDV, rdv),
                (COUNTER, &paths.counter),
                (ARM, &paths.arm),
                (BARRIER, &paths.barrier),
            ],
            READY,
        )
    })
    .await;
    fixture.provision(true).await;
    Harness {
        fixture,
        paths,
        listener: Some(listener),
        _controller: controller,
    }
}

fn result_query(fixture: &LiveFixture, receipt: &CommandReceipt) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_7201),
        query_type: "GetCommandReceipt".into(),
        project_id: Some(fixture.project),
        payload: object(
            json!({"command_id":receipt.command_id,"result_ref":receipt.result_ref.as_ref().unwrap()}),
        ),
    }
}

async fn result(fixture: &mut LiveFixture, receipt: &CommandReceipt) -> Value {
    let deadline = Instant::now() + DEADLINE;
    let query = result_query(fixture, receipt);
    loop {
        match fixture.client.query(&query).await {
            Ok(reply) => return reply.result,
            Err(unleash_client::ClientError::Remote(error))
                if matches!(
                    error.code,
                    unleash_protocol::ErrorCode::ResourceBusy
                        | unleash_protocol::ErrorCode::StaleFence
                ) => {}
            Err(error) => panic!("observed registered result refused: {error:?}"),
        }
        assert!(
            Instant::now() < deadline,
            "registered acquisition did not become observed"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn registered(fixture: &LiveFixture, before: &Value, n: u128) -> CommandEnvelope {
    command(
        fixture.authority,
        n,
        fixture.project,
        serde_json::from_value::<Uuid>(before.get("terminal_id").unwrap().clone()).unwrap(),
        before.get("control_revision").unwrap().as_u64().unwrap(),
        "AcquireTerminal",
        json!({"terminal_id":before.get("terminal_id").unwrap(),
            "process_run_uuid":before.get("process_run_uuid").unwrap(),"scope_revision":2,"control_mode":"exclusive"}),
    )
}

#[derive(Debug)]
struct State {
    intent: String,
    action: String,
    dispatches: i64,
    proofs: i64,
    expiry: i64,
    disposed_by: Option<String>,
    disposition_proof: Option<String>,
    control_revision: i64,
}

fn state(fixture: &LiveFixture, receipt: &CommandReceipt) -> State {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT r.state,a.state,(SELECT COUNT(*) FROM action_dispatches WHERE action_id=r.action_id),(SELECT COUNT(*) FROM broker_phase_records WHERE action_id=r.action_id AND phase_key LIKE 'observation:%' AND json_extract(phase_json,'$.phase.receipt.state')='observed_success'),l.expires_at_ms,r.disposition_receipt_id,r.disposition_object_hash,s.control_revision FROM terminal_renewal_intents r JOIN actions a ON a.action_id=r.action_id JOIN resource_leases l ON l.lease_id=r.lease_id JOIN terminal_sessions s ON s.terminal_id=r.terminal_id WHERE r.receipt_id=?1",
        [receipt.receipt_id.to_string()],|row|Ok(State {intent:row.get(0)?,action:row.get(1)?,dispatches:row.get(2)?,proofs:row.get(3)?,expiry:row.get(4)?,disposed_by:row.get(5)?,disposition_proof:row.get(6)?,control_revision:row.get(7)?})).unwrap()
}

fn unchanged_unknown(fixture: &LiveFixture, receipt: &CommandReceipt, expiry: i64) {
    let snapshot = state(fixture, receipt);
    assert_eq!(snapshot.action, "outcome_unknown", "{snapshot:?}");
    assert!(matches!(
        snapshot.intent.as_str(),
        "prepared" | "handshake_resolved" | "outcome_unknown"
    ));
    assert_eq!(
        (
            snapshot.dispatches,
            snapshot.proofs,
            snapshot.expiry,
            snapshot.control_revision
        ),
        (1, 0, expiry, 0)
    );
    assert!(snapshot.disposed_by.is_none() && snapshot.disposition_proof.is_none());
}

fn physical(
    before: &unleash_process_host::HostManifest,
    after: &unleash_process_host::HostManifest,
) {
    assert_eq!(
        (
            &before.host_id,
            &before.process_id,
            &before.process_run_uuid,
            &before.pid,
            &before.pgid,
            &before.owner_uid,
            &before.boot_identity,
            &before.start_identity,
            &before.socket_path
        ),
        (
            &after.host_id,
            &after.process_id,
            &after.process_run_uuid,
            &after.pid,
            &after.pgid,
            &after.owner_uid,
            &after.boot_identity,
            &after.start_identity,
            &after.socket_path
        )
    );
}

async fn drain(fixture: &mut LiveFixture) {
    let revision = fixture
        .snapshot()
        .await
        .pointer("/daemon/revision")
        .unwrap()
        .as_u64()
        .unwrap();
    let stop = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_7209),
        command_id: CommandId::from_u128(0xf011_7209),
        authority_generation: fixture.authority,
        actor_ref: None,
        project_id: None,
        aggregate_id: fixture.authority.as_uuid(),
        expected_revision: Revision::new(revision),
        command_type: "StopDaemon".into(),
        payload: object(json!({"reason":"renewal recovery replay drain","drain_timeout_ms":10000})),
    };
    assert_eq!(
        fixture.client.send_command(&stop).await.unwrap().state,
        ReceiptState::Accepted
    );
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(status) = fixture.daemon.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the successor writer must really stop"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn exercise(before_effect: bool) {
    let mut harness = harness(before_effect).await;
    let fixture = &mut harness.fixture;
    fixture.start_terminal().await;
    let allocation = fixture
        .client
        .get_command_receipt(
            RequestId::from_u128(0xf011_7202),
            CommandId::from_u128(0xf011_4105),
        )
        .await
        .unwrap()
        .receipt;
    let original = result(fixture, &allocation).await;
    let old_native = fixture.host_manifest();
    let expiry = original.get("lease_expires_at").unwrap().as_i64().unwrap();
    let baseline = count(&harness.paths);
    assert!(
        baseline > 0,
        "the real initial host handshake must already have been counted"
    );
    let request = registered(fixture, &original, 0xf011_7203);
    unleash_platform::write_private_file_atomic(&harness.paths.arm, b"armed").unwrap();
    let accepted = fixture.client.send_command(&request).await.unwrap();
    let cut = receive(
        harness.listener.as_ref().unwrap(),
        if before_effect { DISPATCH } else { HANDSHAKE },
    )
    .await;
    let expected = baseline + usize::from(!before_effect);
    assert_eq!(count(&harness.paths), expected);
    let pending = state(fixture, &accepted);
    assert_eq!(
        (
            pending.dispatches,
            pending.proofs,
            pending.expiry,
            pending.control_revision
        ),
        (1, 0, expiry, 0)
    );
    assert_eq!(
        pending.intent,
        if before_effect {
            "prepared"
        } else {
            "handshake_resolved"
        }
    );
    physical(&old_native, &fixture.host_manifest());
    // Only this daemon dies; the independently instrumented native host, child,
    // PTY and callback counter remain alive and retain the actual applied fact.
    fixture.daemon.kill_for_restart();
    std::fs::remove_file(&harness.paths.arm).unwrap();
    drop(cut); // EOF releases the actual native ACK barrier without an ACK.
    fixture.start_successor().await;
    unchanged_unknown(fixture, &accepted, expiry);
    assert_eq!(
        count(&harness.paths),
        expected,
        "startup must not repeat the renewal handshake"
    );
    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_7204), request.command_id)
            .await
            .unwrap()
            .receipt,
        accepted
    );
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap(),
        accepted
    );
    let causal = result_query(fixture, &accepted);
    assert!(matches!(fixture.client.query(&causal).await,
        Err(unleash_client::ClientError::Remote(error)) if error.code==unleash_protocol::ErrorCode::StaleFence));
    unchanged_unknown(fixture, &accepted, expiry);
    assert_eq!(
        count(&harness.paths),
        expected,
        "exact receipt replay must not contact the host"
    );
    refuse_input_until_disposed(fixture, &harness.paths, &original, expected).await;
    explicit_transfer(
        fixture,
        &harness.paths,
        &original,
        &old_native,
        &accepted,
        expiry,
        expected,
    )
    .await;
}

async fn refuse_input_until_disposed(
    fixture: &mut LiveFixture,
    paths: &Paths,
    original: &Value,
    expected: usize,
) {
    let terminal: Uuid =
        serde_json::from_value(original.get("terminal_id").unwrap().clone()).unwrap();
    let input = command(
        fixture.authority,
        0xf011_7207,
        fixture.project,
        terminal,
        0,
        "SendTerminalInput",
        json!({"terminal_id":original.get("terminal_id").unwrap(),"process_run_uuid":original.get("process_run_uuid").unwrap(),
            "lease_id":original.get("lease_id").unwrap(),"fence":original.get("fence").unwrap(),
            "input_seq":1,"payload_bytes":b"apply-effect\n".to_vec()}),
    );
    assert!(matches!(fixture.client.send_command(&input).await,
        Err(unleash_client::ClientError::Remote(error))
        if error.code==unleash_protocol::ErrorCode::ResourceBusy
            && error.retry_class==unleash_protocol::RetryClass::AfterBackoff
            && error.message.contains("terminal renewal requires observation or disposition")));
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM terminal_input_intents WHERE command_id=?1",
            [input.command_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        count(paths),
        expected,
        "an unknown renewal cannot authorize an input handshake"
    );
    assert!(!fixture.project_root.path().join("worker-effects").exists());
}

async fn explicit_transfer(
    fixture: &mut LiveFixture,
    paths: &Paths,
    original: &Value,
    old_native: &unleash_process_host::HostManifest,
    unknown: &CommandReceipt,
    expiry: i64,
    expected: usize,
) {
    // A wrong physical source must not exploit the quarantine exception.
    let mut wrong = registered(fixture, original, 0xf011_7205);
    wrong.payload.insert(
        "process_run_uuid".into(),
        json!(Uuid::from_u128(0xf011_ffff)),
    );
    assert!(matches!(fixture.client.send_command(&wrong).await,
        Err(unleash_client::ClientError::Remote(error)) if error.code==unleash_protocol::ErrorCode::InvalidArgument));
    unchanged_unknown(fixture, unknown, expiry);
    assert_eq!(count(paths), expected);
    let transfer = registered(fixture, original, 0xf011_7206);
    let accepted = fixture.client.send_command(&transfer).await.unwrap();
    let current = result(fixture, &accepted).await;
    assert_ne!(current.get("lease_id"), original.get("lease_id"));
    assert!(
        current.get("fence").unwrap().as_u64().unwrap()
            > original.get("fence").unwrap().as_u64().unwrap()
    );
    assert_eq!(current.get("control_revision"), Some(&json!(1)));
    let after = fixture.host_manifest();
    physical(old_native, &after);
    assert!(
        after.daemon_epoch > old_native.daemon_epoch
            && after.highest_fence > old_native.highest_fence
    );
    assert_eq!(
        count(paths),
        expected,
        "rotation is a distinct TransferControlV1, never renewal replay"
    );
    let disposed = state(fixture, unknown);
    assert_eq!(
        (disposed.intent.as_str(), disposed.action.as_str()),
        ("disposed", "outcome_unknown")
    );
    assert_eq!(
        (
            disposed.expiry,
            disposed.dispatches,
            disposed.proofs,
            disposed.control_revision
        ),
        (expiry, 1, 0, 1)
    );
    assert_eq!(disposed.disposed_by, Some(accepted.receipt_id.to_string()));
    assert!(disposed.disposition_proof.is_some());
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let bound:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_renewal_intents r JOIN terminal_renewal_transfer_links link ON link.renewal_action_id=r.action_id AND link.renewal_dispatch_id=r.dispatch_id AND link.renewal_context_digest=r.context_digest JOIN terminal_control_intents c ON c.action_id=link.transfer_action_id AND c.receipt_id=r.disposition_receipt_id JOIN actions a ON a.action_id=c.action_id JOIN broker_phase_records p ON p.action_id=c.action_id AND p.phase_key LIKE 'observation:%' JOIN object_references f ON f.owner_kind='terminal_control_observation' AND f.owner_id=c.action_id AND f.source_field=c.dispatch_id AND f.object_hash=r.disposition_object_hash AND f.state='live' WHERE r.receipt_id=?1 AND c.receipt_id=?2 AND c.observation_object_hash=r.disposition_object_hash AND json_extract(p.phase_json,'$.phase.receipt.output_digest')=r.disposition_object_hash AND a.state='observed_success')",
        rusqlite::params![unknown.receipt_id.to_string(),accepted.receipt_id.to_string()],|row|row.get(0)).unwrap();
    assert!(
        bound,
        "disposition requires the exact published V012 observation and phase"
    );
    drop(conn);
    assert_eq!(
        fixture.client.send_command(&transfer).await.unwrap(),
        accepted
    );
    let replay = registered(fixture, original, 0xf011_7203);
    assert_eq!(
        fixture.client.send_command(&replay).await.unwrap(),
        *unknown
    );
    drain(fixture).await;
    let final_state = state(fixture, unknown);
    assert_eq!(
        (
            final_state.expiry,
            final_state.dispatches,
            final_state.proofs
        ),
        (expiry, 1, 0)
    );
    assert_eq!(
        count(paths),
        expected,
        "no late renewal task may survive the successor drain"
    );
    assert_eq!(
        std::fs::read(fixture.project_root.path().join("worker-starts")).unwrap(),
        b"started\n"
    );
    assert!(!fixture.project_root.path().join("worker-effects").exists());
}

#[tokio::test]
async fn t_f011_renewal_c4_loss_quarantines_then_observed_transfer_disposes() {
    exercise(true).await;
}
#[tokio::test]
async fn t_f011_renewal_lost_native_ack_never_repeats_handshake_then_transfers() {
    exercise(false).await;
}
