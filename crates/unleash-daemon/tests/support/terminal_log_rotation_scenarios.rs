//! Typed cursor expiration from an owned real PTY through the daemon IPC.

use super::*;
use unleash_client::ClientError;
use unleash_protocol::{ErrorCode, QueryResponse, ReasonCode, RetryClass};

const TAIL: &str = "retention-fixture-complete";

#[test]
#[ignore = "owned PTY worker fixture selected by exact argv only"]
fn terminal_log_rotation_worker() {
    let mut output = std::io::stdout().lock();
    let block = [b'x'; 8192];
    // Two MiB exceeds the regular daemon's one MiB retained spool.
    for _ in 0..256 {
        output.write_all(&block).unwrap();
    }
    writeln!(output, "\n{TAIL}").unwrap();
    output.flush().unwrap();
    drop(output);
    let _ = std::io::stdin().lock().lines().next();
}

async fn read_log_at(
    client: &mut UnleashClient,
    project: ProjectId,
    terminal: Uuid,
    offset: u64,
) -> Result<QueryResponse, ClientError> {
    client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_4802),
            project_id: Some(project),
            query_type: "ReadTerminalLog".into(),
            payload: object(json!({
                "terminal_id": terminal, "after_offset": offset, "max_bytes": 256,
            })),
        })
        .await
}

async fn wait_for_rotation(client: &mut UnleashClient, project: ProjectId, terminal: Uuid) -> u64 {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match read_log_at(client, project, terminal, 9_007_199_254_740_991).await {
            Ok(head) => {
                let offset = head
                    .result
                    .pointer("/log/volatile_head_offset")
                    .and_then(Value::as_u64)
                    .unwrap();
                if let Ok(tail) =
                    read_log_at(client, project, terminal, offset.saturating_sub(128)).await
                    && tail
                        .result
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.contains(TAIL))
                {
                    let oldest = tail
                        .result
                        .pointer("/log/oldest_available_offset")
                        .and_then(Value::as_u64)
                        .unwrap();
                    assert!(oldest > 0, "the live host must have rotated its spool");
                    return oldest;
                }
            }
            Err(ClientError::Remote(error))
                if error.code == ErrorCode::ResourceBusy
                    && error.retry_class == RetryClass::AfterRefresh => {}
            Err(error) => panic!("unable to observe owned PTY rotation: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "observe final marker after rotation"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn rotated_fixture() -> (LiveFixture, Uuid, u64) {
    let mut fixture = LiveFixture::new(true).await;
    let mut request = fixture.allocation(0xf011_4801);
    request.payload.insert(
        "arguments".into(),
        json!([
            "--ignored",
            "--exact",
            "reattach_scenarios::log_rotation_scenarios::terminal_log_rotation_worker",
            "--nocapture",
        ]),
    );
    let terminal = request.aggregate_id;
    let project = fixture.project;
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap().state,
        ReceiptState::Accepted
    );
    let oldest = wait_for_rotation(&mut fixture.client, project, terminal).await;
    (fixture, terminal, oldest)
}

#[tokio::test]
async fn t_f011_008_real_daemon_exposes_expired_log_cursor_to_authorized_reader() {
    let (mut fixture, terminal, oldest) = rotated_fixture().await;
    let project = fixture.project;
    let other_project = ProjectId::from_u128(0xf011_4fff);
    assert!(matches!(
        read_log_at(&mut fixture.client, other_project, terminal, 0).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden
            && error.reason_code.is_none()
    ));
    let expired = read_log_at(&mut fixture.client, project, terminal, 0)
        .await
        .unwrap_err();
    assert!(
        matches!(&expired, ClientError::Remote(error) if error.code == ErrorCode::ResyncRequired
            && error.reason_code == Some(ReasonCode::CursorExpired)
            && error.retry_class == RetryClass::AfterRefresh),
        "expected the host's typed cursor expiration, got {expired:?}"
    );
    let resumed = read_log_at(&mut fixture.client, project, terminal, oldest)
        .await
        .unwrap();
    assert_eq!(
        resumed
            .result
            .pointer("/log/segments/0/start_offset")
            .and_then(Value::as_u64),
        Some(oldest)
    );
}

#[tokio::test]
#[ignore = "fournir UNLEASH_TEST_CLI_BIN et le process-host construits depuis le même arbre"]
async fn t_f011_008_cli_preserves_structured_cursor_expiration_and_scope_refusal() {
    let binary = PathBuf::from(
        std::env::var_os("UNLEASH_TEST_CLI_BIN").expect("explicit absolute path to the real CLI"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let (fixture, terminal, oldest) = rotated_fixture().await;
    let socket = fixture.rendezvous.path().join("daemon.sock");
    let read = |project: ProjectId, offset: u64| {
        Command::new(&binary)
            .arg("--socket")
            .arg(&socket)
            .args([
                "--json",
                "--timeout-ms",
                "5000",
                "process",
                "log",
                "--project-id",
            ])
            .arg(project.to_string())
            .arg("--terminal-id")
            .arg(terminal.to_string())
            .arg("--after-offset")
            .arg(offset.to_string())
            .args(["--max-bytes", "16"])
            .output()
            .expect("read with the real CLI over the owned socket")
    };
    let expired = read(fixture.project, 0);
    assert!(!expired.status.success());
    assert!(expired.stdout.is_empty());
    let error: Value = serde_json::from_slice(&expired.stderr).unwrap();
    assert!(error.get("error").and_then(Value::as_str).is_some());
    assert_eq!(error.get("code"), Some(&json!(ErrorCode::ResyncRequired)));
    assert_eq!(
        error.get("reason_code"),
        Some(&json!(ReasonCode::CursorExpired))
    );
    assert_eq!(
        error.get("retry_class"),
        Some(&json!(RetryClass::AfterRefresh))
    );
    assert!(error.get("details_ref").is_none());
    let denied = read(ProjectId::from_u128(0xf011_4fff), 0);
    assert!(!denied.status.success());
    let hidden: Value = serde_json::from_slice(&denied.stderr).unwrap();
    assert_eq!(
        hidden.get("code"),
        Some(&json!(ErrorCode::NotFoundOrHidden))
    );
    assert_eq!(hidden.get("reason_code"), Some(&Value::Null));
    let resumed = read(fixture.project, oldest);
    assert!(resumed.status.success());
    let page: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(
        page.pointer("/result/log/segments/0/start_offset")
            .and_then(Value::as_u64),
        Some(oldest)
    );
}
