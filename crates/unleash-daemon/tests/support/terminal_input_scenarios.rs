//! Independent UDS oracles for the next input tranche. No input route/helper is
//! implemented by this file. The expected assertions come from the actual
//! committed ledger and host, not from the proposed result projection.
use super::*;

fn ledger_result(
    fixture: &LiveFixture,
    command: unleash_domain::CommandId,
    reference: &str,
) -> Value {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT receipt.receipt_id,receipt.command_id,receipt.authority_generation,base.terminal_id,base.process_id,base.process_run_uuid,control.action_id IS NOT NULL,lease.lease_id,lease.fence,lease.expires_at_ms,lease.daemon_epoch,session.control_revision,session.input_seq,session.state,daemon.stream_id,phase.event_sequence FROM command_receipts receipt LEFT JOIN terminal_allocations initial ON initial.receipt_id=receipt.receipt_id LEFT JOIN terminal_control_intents control ON control.receipt_id=receipt.receipt_id JOIN terminal_allocations base ON base.terminal_id=COALESCE(control.terminal_id,initial.terminal_id) JOIN terminal_sessions session ON session.terminal_id=base.terminal_id JOIN resource_leases lease ON lease.lease_id=session.lease_id JOIN daemon_state daemon ON daemon.singleton=1 JOIN broker_phase_records phase ON phase.action_id=COALESCE(control.action_id,initial.action_id) AND phase.phase_key LIKE 'observation:%' WHERE receipt.command_id=?1 ORDER BY phase.event_sequence DESC LIMIT 1",[command.to_string()],|row|Ok(json!({
        "schema_version":1,"receipt_id":row.get::<_,String>(0)?,"command_id":row.get::<_,String>(1)?,"result_ref":reference,
        "authority_generation":row.get::<_,String>(2)?,"terminal_id":row.get::<_,String>(3)?,"process_id":row.get::<_,String>(4)?,"process_run_uuid":row.get::<_,String>(5)?,"reused":row.get::<_,bool>(6)?,
        "lease_id":row.get::<_,String>(7)?,"fence":row.get::<_,i64>(8)?,"lease_expires_at":row.get::<_,i64>(9)?,"daemon_epoch":row.get::<_,i64>(10)?,"control_revision":row.get::<_,i64>(11)?,"input_seq":row.get::<_,i64>(12)?,"state":row.get::<_,String>(13)?,
        "cursor":{"stream_id":row.get::<_,String>(14)?,"sequence":row.get::<_,i64>(15)?},
    }))).unwrap()
}

fn result_query(
    fixture: &LiveFixture,
    command: unleash_domain::CommandId,
    reference: &str,
    id: u128,
) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        query_type: "GetCommandReceipt".into(),
        project_id: Some(fixture.project),
        payload: object(json!({"command_id":command,"result_ref":reference})),
    }
}

#[tokio::test]
async fn t_f011_005_acquisition_result_is_opt_in_exact_and_causal() {
    let mut fixture = LiveFixture::new(true).await;
    fixture.start_terminal().await;
    let command = unleash_domain::CommandId::from_u128(0xf011_4105);
    let ordinary = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_5101), command)
        .await
        .expect("the existing strict client still decodes the ordinary receipt");
    let reference = ordinary
        .receipt
        .result_ref
        .clone()
        .expect("accepted acquisition has its original result reference");
    let expected = ledger_result(&fixture, command, &reference);
    assert_eq!(
        fixture.host_manifest().last_confirmed_input_seq,
        expected["input_seq"].as_u64().unwrap(),
        "input_seq is the last host-receipted durable input, not a fabricated next slot"
    );
    let query = result_query(&fixture, command, &reference, 0xf011_5102);
    let response = fixture
        .client
        .query(&query)
        .await
        .expect("the exact observed acquisition has a qualified result");
    assert_eq!(
        response.result, expected,
        "the response is closed and bound to the actual lease and observation, without a host secret"
    );
    let watermark = response
        .watermark
        .expect("qualified result carries its transaction watermark");
    let cause: unleash_protocol::EventCursor =
        serde_json::from_value(expected["cursor"].clone()).unwrap();
    assert_eq!(watermark.stream_id, cause.stream_id);
    assert!(watermark.sequence >= cause.sequence);
    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_5103), command)
            .await
            .unwrap(),
        ordinary,
        "result inspection never changes the accepted receipt"
    );
}

#[tokio::test]
async fn t_f011_005_acquisition_result_cannot_be_borrowed_by_another_receipt() {
    let mut fixture = LiveFixture::new(true).await;
    fixture.start_terminal().await;
    let command = unleash_domain::CommandId::from_u128(0xf011_4105);
    let owned = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_5201), command)
        .await
        .unwrap();
    let reference = owned.receipt.result_ref.unwrap();
    let other = unleash_domain::CommandId::from_u128(0xf011_4101);
    let query = result_query(&fixture, other, &reference, 0xf011_5202);
    let error = fixture
        .client
        .query(&query)
        .await
        .expect_err("a real project receipt cannot borrow an acquisition result reference");
    assert!(
        matches!(error,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::NotFoundOrHidden)
    );
}

async fn short_lease_fixture() -> LiveFixture {
    let mut fixture = LiveFixture::new(false).await;
    let mut runtime = terminal_configuration();
    runtime["lease_ttl_ms"] = json!(5000);
    let mut settings = command(
        fixture.authority,
        0xf011_4104,
        fixture.project,
        fixture.authority.as_uuid(),
        0,
        "SetSettings",
        json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":runtime}}),
    );
    settings.project_id = None;
    assert_eq!(
        fixture.client.send_command(&settings).await.unwrap().state,
        ReceiptState::Accepted
    );
    let mut request = fixture.allocation(0xf011_4105);
    request
        .payload
        .get_mut("readiness")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("timeout_ms".into(), json!(2500));
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap().state,
        ReceiptState::Accepted
    );
    let deadline = Instant::now() + DEADLINE;
    loop {
        let response = fixture
            .client
            .query(&QueryEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(0xf011_5301),
                query_type: "ReadTerminalLog".into(),
                project_id: Some(fixture.project),
                payload: object(
                    json!({"terminal_id":request.aggregate_id,"after_offset":0,"max_bytes":65536}),
                ),
            })
            .await;
        if response.as_ref().is_ok_and(|reply| {
            reply.result["text"]
                .as_str()
                .is_some_and(|text| text.contains("live-terminal-ready"))
        }) {
            return fixture;
        }
        if let Err(error) = response {
            assert!(
                matches!(error,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::ResourceBusy),
                "unexpected initial observation: {error:?}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "owned short-lease PTY starts before its bounded readiness deadline"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn t_f011_005_acquisition_result_does_not_release_expired_lease_assertions() {
    let mut fixture = short_lease_fixture().await;
    let command = unleash_domain::CommandId::from_u128(0xf011_4105);
    let ordinary = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_5302), command)
        .await
        .unwrap();
    let reference = ordinary.receipt.result_ref.unwrap();
    let expected = ledger_result(&fixture, command, &reference);
    let query = result_query(&fixture, command, &reference, 0xf011_5303);
    assert_eq!(
        fixture
            .client
            .query(&query)
            .await
            .expect("same owner can inspect the genuinely live result first")
            .result,
        expected
    );
    let expiry = expected["lease_expires_at"].as_i64().unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    if let Ok(remaining) = u64::try_from(expiry - now + 10) {
        assert!(
            remaining <= 5000,
            "wait only for this fixture configured lease"
        );
        tokio::time::sleep(Duration::from_millis(remaining)).await;
    }
    let query = result_query(&fixture, command, &reference, 0xf011_5304);
    let error = fixture
        .client
        .query(&query)
        .await
        .expect_err("the same observation/actor cannot make an expired lease usable");
    assert!(
        matches!(error,unleash_client::ClientError::Remote(ref error) if error.code==unleash_protocol::ErrorCode::StaleFence)
    );
}
