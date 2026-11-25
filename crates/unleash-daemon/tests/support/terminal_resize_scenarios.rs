//! Native resize observation through a real daemon, host, and PTY.
use super::*;
use tokio::io::AsyncWriteExt;
use unleash_protocol::{
    CommandReceipt, ErrorCode, EventEnvelope, SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_store::terminal_allocations::terminal_acquisition_query::TerminalAcquisitionResult;

const RESIZE_WORKER: &str = "reattach_scenarios::resize_scenarios::resize_worker";

#[test]
#[ignore = "owned resize worker selected by exact argv only"]
fn resize_worker() {
    let deadline = Instant::now() + DEADLINE;
    let mut last_size = String::new();
    let (checks, pending_checks) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if checks.send(line).is_err() {
                break;
            }
        }
    });
    while Instant::now() < deadline {
        let output = Command::new("/bin/stty")
            .arg("size")
            .stdin(Stdio::inherit())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stty must read the child's native PTY"
        );
        let size = std::str::from_utf8(&output.stdout).unwrap().trim();
        let mut stdout = std::io::stdout().lock();
        if size != last_size {
            writeln!(stdout, "\nnative-pty-size={size}").unwrap();
            size.clone_into(&mut last_size);
        }
        for check in pending_checks.try_iter() {
            writeln!(stdout, "\nnative-pty-size={size} {check}").unwrap();
        }
        stdout.flush().unwrap();
        drop(stdout);
        std::thread::sleep(Duration::from_millis(25));
    }
}

async fn resize_result(fixture: &LiveFixture, receipt: &CommandReceipt) -> Value {
    let event = terminal_event(fixture, receipt, |event| {
        event.event_type == "ActionObserved"
            && event
                .payload
                .get("terminal_resize")
                .and_then(|result| result.get("receipt_id"))
                == Some(&json!(receipt.receipt_id))
    })
    .await;
    assert_eq!(event.payload.get("state"), Some(&json!("observed_success")));
    let result = event.payload.get("terminal_resize").unwrap();
    assert_eq!(result.get("command_id"), Some(&json!(receipt.command_id)));
    result.clone()
}

pub(super) async fn terminal_event(
    fixture: &LiveFixture,
    receipt: &CommandReceipt,
    matches: impl Fn(&EventEnvelope) -> bool,
) -> EventEnvelope {
    let mut reader = fixture
        .client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: receipt.event_cursor.stream_id,
            after_sequence: receipt.event_cursor.sequence,
            project_id: Some(fixture.project),
            aggregate_id: None,
            batch_limit: 16,
        })
        .await
        .unwrap();
    tokio::time::timeout(DEADLINE, async {
        loop {
            let SubscribeEventsFrame::Events { events, .. } =
                reader.next_frame().await.unwrap().unwrap()
            else {
                panic!("expected the resize observation from the same durable stream");
            };
            for event in events {
                if matches(&event) {
                    assert!(event.sequence > receipt.event_cursor.sequence);
                    return event;
                }
            }
        }
    })
    .await
    .expect("terminal operation gets a causal result over IPC")
}

fn dispatch_count(fixture: &LiveFixture, command: CommandId) -> i64 {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM action_dispatches d JOIN terminal_renewal_intents r ON r.action_id=d.action_id WHERE r.command_id=?1",
        [command.to_string()],
        |row| row.get(0),
    ).unwrap()
}

pub(super) fn stored_dimensions(fixture: &LiveFixture, terminal: Uuid) -> (u16, u16) {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row(
        "SELECT rows,cols FROM terminal_sessions WHERE terminal_id=?1",
        [terminal.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

pub(super) async fn wait_for_native_size(fixture: &mut LiveFixture, terminal: Uuid, size: &str) {
    let deadline = Instant::now() + DEADLINE;
    let marker = format!("native-pty-size={size}");
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_7502),
        query_type: "ReadTerminalLog".into(),
        project_id: Some(fixture.project),
        payload: object(json!({"terminal_id":terminal,"after_offset":0,"max_bytes":65536})),
    };
    loop {
        match fixture.client.query(&query).await {
            Ok(reply)
                if reply
                    .result
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.lines().any(|line| line == marker)) =>
            {
                return;
            }
            Ok(_) => {}
            Err(unleash_client::ClientError::Remote(error))
                if error.code == unleash_protocol::ErrorCode::ResourceBusy
                    && error.retry_class == unleash_protocol::RetryClass::AfterRefresh => {}
            Err(error) => panic!("native PTY log unavailable: {error:?}"),
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {marker}: {}",
            terminal_diagnostic(fixture.data.path())
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn acquire_resize_worker(fixture: &mut LiveFixture) -> TerminalAcquisitionResult {
    let mut acquire = fixture.allocation(0xf011_4105);
    acquire.payload.insert(
        "arguments".into(),
        json!(["--ignored", "--exact", RESIZE_WORKER, "--nocapture"]),
    );
    let original = fixture.client.send_command(&acquire).await.unwrap();
    assert_eq!(original.state, ReceiptState::Accepted);
    let terminal = acquire.aggregate_id;
    wait_for_native_size(fixture, terminal, "24 80").await;
    observed_acquisition(fixture, &original).await
}

pub(super) async fn observed_acquisition(
    fixture: &mut LiveFixture,
    receipt: &CommandReceipt,
) -> TerminalAcquisitionResult {
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_7503),
        query_type: "GetCommandReceipt".into(),
        project_id: Some(fixture.project),
        payload: object(json!({
            "command_id": receipt.command_id,
            "result_ref": receipt.result_ref.as_ref().unwrap(),
        })),
    };
    let deadline = Instant::now() + DEADLINE;
    loop {
        match fixture.client.query(&query).await {
            Ok(reply) => return serde_json::from_value(reply.result).unwrap(),
            Err(unleash_client::ClientError::Remote(error))
                if matches!(error.code, ErrorCode::ResourceBusy | ErrorCode::StaleFence) => {}
            Err(error) => panic!("actual acquisition evidence refused: {error:?}"),
        }
        assert!(
            Instant::now() < deadline,
            "actual acquisition exposes its observed lease"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) fn resize_request(
    fixture: &LiveFixture,
    acquired: &TerminalAcquisitionResult,
    id: u128,
    rows: u16,
    cols: u16,
) -> CommandEnvelope {
    command(
        fixture.authority,
        id,
        fixture.project,
        acquired.terminal_id.as_uuid(),
        acquired.control_revision.as_u64(),
        "ResizeTerminal",
        json!({
            "terminal_id":acquired.terminal_id,
            "process_run_uuid":acquired.process_run_uuid,
            "lease_id":acquired.lease_id,
            "fence":acquired.fence,
            "rows":rows,
            "cols":cols,
        }),
    )
}

#[tokio::test]
async fn t_f011_003_resize_changes_the_size_observed_by_the_live_child() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire_resize_worker(&mut fixture).await;
    let terminal = acquired.terminal_id.as_uuid();
    let resize = resize_request(&fixture, &acquired, 0xf011_7504, 37, 112);
    let accepted = fixture
        .client
        .send_command(&resize)
        .await
        .expect("ResizeTerminal accepts the current authenticated control");
    assert_eq!(accepted.state, ReceiptState::Accepted);
    wait_for_native_size(&mut fixture, terminal, "37 112").await;
    let observed = resize_result(&fixture, &accepted).await;
    assert_eq!(stored_dimensions(&fixture, terminal), (37, 112));
    assert_eq!(observed.get("terminal_id"), Some(&json!(terminal)));
    assert_eq!(
        observed.get("process_run_uuid"),
        Some(&json!(acquired.process_run_uuid))
    );
    assert_eq!(
        observed.get("dimensions"),
        Some(&json!({"rows":37,"cols":112}))
    );
    assert!(
        observed
            .get("lease_expires_at")
            .and_then(Value::as_i64)
            .unwrap()
            > acquired.lease_expires_at.as_i64()
    );

    let mut stale = resize.clone();
    stale.command_id = CommandId::from_u128(0xf011_7505);
    stale.request_id = RequestId::from_u128(0xf011_7505);
    stale
        .payload
        .insert("fence".into(), json!(acquired.fence.as_u64() + 1));
    assert!(matches!(
        fixture.client.send_command(&stale).await,
        Err(unleash_client::ClientError::Remote(error)) if error.code == ErrorCode::StaleFence
    ));
    assert_eq!(dispatch_count(&fixture, stale.command_id), 0);

    let successor = resize_request(&fixture, &acquired, 0xf011_7506, 43, 119);
    let successor = fixture.client.send_command(&successor).await.unwrap();
    assert_eq!(
        resize_result(&fixture, &successor).await.get("dimensions"),
        Some(&json!({"rows":43,"cols":119}))
    );
    assert_eq!(
        fixture.client.send_command(&resize).await.unwrap(),
        accepted
    );
    assert_eq!(dispatch_count(&fixture, resize.command_id), 1);
    assert_eq!(dispatch_count(&fixture, successor.command_id), 1);

    let check = command(
        fixture.authority,
        0xf011_7507,
        fixture.project,
        terminal,
        acquired.control_revision.as_u64(),
        "SendTerminalInput",
        json!({
            "terminal_id":terminal,"process_run_uuid":acquired.process_run_uuid,
            "lease_id":acquired.lease_id,"fence":acquired.fence,
            "input_seq":1,"payload_bytes":b"after-replay\n",
        }),
    );
    assert_eq!(
        fixture.client.send_command(&check).await.unwrap().state,
        ReceiptState::Accepted
    );
    wait_for_native_size(&mut fixture, terminal, "43 119 after-replay").await;
    assert_eq!(stored_dimensions(&fixture, terminal), (43, 119));
}

pub(super) async fn set_standard_policy(fixture: &mut LiveFixture) {
    let mut policy = command(
        fixture.authority,
        0xf011_7510,
        fixture.project,
        fixture.authority.as_uuid(),
        0,
        "SetPolicy",
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard",
            "policy_version":"resize-qualification","reason":"exercise configured controller policy"}),
    );
    policy.project_id = None;
    let policy = fixture
        .client
        .send_command(&policy)
        .await
        .expect("policy committed over UDS");
    assert_eq!(policy.new_revision, Revision::new(1));
}

#[tokio::test]
async fn t_f011_003_resize_uses_the_committed_standard_policy() {
    let mut fixture = LiveFixture::new(true).await;
    let acquired = acquire_resize_worker(&mut fixture).await;
    set_standard_policy(&mut fixture).await;
    let resize = resize_request(&fixture, &acquired, 0xf011_7511, 37, 112);
    let accepted = fixture.client.send_command(&resize).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let result = resize_result(&fixture, &accepted).await;
    assert_eq!(
        result.get("dimensions"),
        Some(&json!({"rows":37,"cols":112}))
    );
    wait_for_native_size(&mut fixture, acquired.terminal_id.as_uuid(), "37 112").await;
}

#[tokio::test]
async fn t_f011_003_resize_policy_change_before_dispatch_releases_control() {
    let root = local_test_dir("resize-policy-boundary").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let armed = root.path().join("armed");
    let barrier = root.path().join("intent.sock");
    let listener = unleash_platform::bind_private_unix_listener(&barrier).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
    let mut fixture = renewal_fixture::start_with_intent_cutpoint(&armed, &barrier).await;
    fixture.provision(true).await;
    let acquired = acquire_resize_worker(&mut fixture).await;
    let terminal = acquired.terminal_id.as_uuid();
    let original = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_7530), acquired.command_id)
        .await
        .unwrap()
        .receipt;
    unleash_platform::write_private_file_atomic(&armed, b"armed").unwrap();
    let resize = resize_request(&fixture, &acquired, 0xf011_7531, 37, 112);
    let accepted = fixture.client.send_command(&resize).await.unwrap();
    let mut stalled = renewal_fixture::wait_after_intent(&listener).await;
    assert_eq!(dispatch_count(&fixture, resize.command_id), 0);
    set_standard_policy(&mut fixture).await;
    stalled.write_all(&[1]).await.unwrap();
    let refused = terminal_event(&fixture, &accepted, |event| {
        event.event_type == "TerminalControlDispatchRefused"
            && event.payload.get("receipt_id") == Some(&json!(accepted.receipt_id))
    })
    .await;
    assert_eq!(
        refused.payload.get("reason"),
        Some(&json!("policy_changed"))
    );
    assert_eq!(dispatch_count(&fixture, resize.command_id), 0);
    assert_eq!(stored_dimensions(&fixture, terminal), (24, 80));
    assert_eq!(
        observed_acquisition(&mut fixture, &original)
            .await
            .lease_expires_at,
        acquired.lease_expires_at
    );

    let next = resize_request(&fixture, &acquired, 0xf011_7532, 43, 119);
    let next = fixture.client.send_command(&next).await.unwrap();
    assert_eq!(
        resize_result(&fixture, &next).await.get("dimensions"),
        Some(&json!({"rows":43,"cols":119}))
    );
    wait_for_native_size(&mut fixture, terminal, "43 119").await;
    assert_eq!(
        fixture.client.send_command(&resize).await.unwrap(),
        accepted
    );
    assert_eq!(dispatch_count(&fixture, resize.command_id), 0);
}

#[tokio::test]
async fn t_f011_003_resize_lost_receipt_allows_successor_control_without_redispatch() {
    let arm_root = local_test_dir("resize-receipt-c2").unwrap();
    let armed = arm_root.path().join("armed");
    let mut fixture = renewal_fixture::start_with_receipt_cutpoint(&armed).await;
    fixture.provision(true).await;
    let acquired = acquire_resize_worker(&mut fixture).await;
    let terminal = acquired.terminal_id.as_uuid();
    let orphan = resize_request(&fixture, &acquired, 0xf011_7520, 37, 112);

    unleash_platform::write_private_file_atomic(&armed, b"armed").unwrap();
    assert!(fixture.client.send_command(&orphan).await.is_err());
    let committed = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_7521), orphan.command_id)
        .await
        .expect("the command committed even though its acknowledgement was lost")
        .receipt;
    assert_eq!(committed.state, ReceiptState::Accepted);
    assert_eq!(dispatch_count(&fixture, orphan.command_id), 0);
    let original = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_7523), acquired.command_id)
        .await
        .unwrap()
        .receipt;
    assert_eq!(
        observed_acquisition(&mut fixture, &original)
            .await
            .lease_expires_at,
        acquired.lease_expires_at,
    );

    fixture.restart_daemon_only().await;
    let takeover = command(
        fixture.authority,
        0xf011_7522,
        fixture.project,
        terminal,
        acquired.control_revision.as_u64(),
        "AcquireTerminal",
        json!({"terminal_id":terminal,"process_run_uuid":acquired.process_run_uuid,
            "scope_revision":2,"control_mode":"exclusive"}),
    );
    let accepted = fixture
        .client
        .send_command(&takeover)
        .await
        .expect("a definitely undispatched resize must not block successor control");
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let successor = observed_acquisition(&mut fixture, &accepted).await;
    assert_eq!(successor.process_run_uuid, acquired.process_run_uuid);
    assert!(successor.fence > acquired.fence);
    assert_eq!(
        fixture.client.send_command(&orphan).await.unwrap(),
        committed
    );
    assert_eq!(dispatch_count(&fixture, orphan.command_id), 0);
}
