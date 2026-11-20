//! Native prerequisite of historical Release reconciliation, with no proposed wire.
//! This test does not seal a Store fact or grant a current terminal capability.
use super::*;
use unleash_process_host::{ControlReleaseAck, ControlReleaseRequest, ControlReleaseResponse};
use unleash_store::terminal_allocations::terminal_release_history::{
    TerminalReleaseHistoryTarget, qualify_terminal_release_history_source,
};

struct LostRelease {
    harness: Harness,
    acquired: TerminalAcquisitionResult,
    request: CommandEnvelope,
    receipt: CommandReceipt,
    before: ControlFacts,
    pending: ReleaseSnapshot,
    context_json: String,
    native: unleash_process_host::HostManifest,
    host: unleash_platform::OsProcessIdentity,
    ack: ControlReleaseAck,
    old: OldNativeControl,
    handshakes: usize,
    output: u64,
}

fn original_context(fixture: &LiveFixture, receipt: &CommandReceipt) -> String {
    rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT context_json FROM terminal_release_intents WHERE receipt_id=?1",
        [receipt.receipt_id.to_string()],
        |row| row.get(0),
    )
    .unwrap()
}

async fn lose_native_ack() -> LostRelease {
    let mut harness = harness(false).await;
    let fixture = &mut harness.fixture;
    let acquired = acquire(fixture).await;
    let native = fixture.host_manifest();
    let host = host_identity(&harness.paths);
    let mut old = OldNativeControl::authenticate(fixture);
    let handshakes = count(&harness.paths);
    assert_eq!(release_count(&harness.paths), 0);
    let before = facts(fixture, &acquired);
    let output = fresh_pty_output(&native, 0).await;
    unleash_platform::write_private_file_atomic(&harness.paths.release_arm(), b"armed").unwrap();
    let request = release_request(fixture, &acquired, 0xf011_8200);
    let receipt = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let withheld = receive(harness.listener.as_ref().unwrap(), RELEASE).await;
    let pending = release_state(fixture, &receipt);
    assert_eq!(
        (pending.intent.as_str(), pending.action.as_str()),
        ("retire_resolved", "dispatching")
    );
    assert_eq!(release_count(&harness.paths), 1);
    let ack = native_retirement(fixture);
    assert_eq!(ack.key.header.command_id, request.command_id);
    assert_eq!(ack.key.header.action_id.to_string(), pending.action_id);
    assert_eq!(ack.key.dispatch_id.to_string(), pending.dispatch_id);
    assert_eq!(
        ack.key.header.payload_hash.to_string(),
        pending.context_digest
    );
    let context_json = original_context(fixture, &receipt);
    let context: Value = serde_json::from_str(&context_json).unwrap();
    assert_eq!(
        unleash_protocol::canonical_sha256(&context).unwrap(),
        ack.key.header.payload_hash
    );
    old.assert_revoked();
    assert!(!harness.paths.release_done().try_exists().unwrap());
    assert!(!harness.paths.release_eof().try_exists().unwrap());
    fixture.daemon.kill_for_restart();
    assert_eq!(
        fixture.daemon.child.try_wait().unwrap().unwrap().signal(),
        Some(9)
    );
    std::fs::remove_file(harness.paths.release_arm()).unwrap();
    drop(withheld);
    wait_for_release_eof(&harness.paths).await;
    fixture.start_successor().await;
    assert_eq!(
        fixture.client.hello().authority_generation,
        native.authority_generation
    );
    assert!(fixture.client.hello().daemon_epoch > native.daemon_epoch);
    LostRelease {
        harness,
        acquired,
        request,
        receipt,
        before,
        pending,
        context_json,
        native,
        host,
        ack,
        old,
        handshakes,
        output,
    }
}

fn assert_quarantined(lost: &LostRelease) {
    let fixture = &lost.harness.fixture;
    quarantined(&lost.pending, &release_state(fixture, &lost.receipt));
    retained_resources(&lost.before, &facts(fixture, &lost.acquired));
    same_live_host(fixture, &lost.harness.paths, &lost.host, &lost.native);
    assert_eq!(native_retirement(fixture), lost.ack);
    assert_eq!(original_context(fixture, &lost.receipt), lost.context_json);
    assert_eq!(
        (
            count(&lost.harness.paths),
            release_count(&lost.harness.paths)
        ),
        (lost.handshakes, 1)
    );
}

fn qualify_source(lost: &LostRelease) -> unleash_process_host::ControlReleaseKey {
    let conn = rusqlite::Connection::open_with_flags(
        lost.harness
            .fixture
            .data
            .path()
            .join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let snapshot = conn.unchecked_transaction().unwrap();
    let context: unleash_store::terminal_allocations::terminal_release_types::TerminalReleaseContext =
        serde_json::from_str(&lost.context_json).unwrap();
    let caller = unleash_store::CallerAuthority {
        actor_id: context.actor_id.clone(),
        role: unleash_store::CallerRole::HumanOperator,
    };
    let target = TerminalReleaseHistoryTarget {
        project_id: lost.harness.fixture.project,
        terminal_id: lost.acquired.terminal_id,
        process_run_uuid: lost.acquired.process_run_uuid,
        action_id: lost.ack.key.header.action_id,
        dispatch_id: lost.ack.key.dispatch_id,
    };
    assert_scope_refused(&snapshot, &caller, target);
    let source = qualify_terminal_release_history_source(&snapshot, &caller, target).unwrap();
    assert_eq!(source.context, context);
    assert_eq!(source.context.receipt_id, lost.receipt.receipt_id);
    let key: unleash_process_host::ControlReleaseKey =
        serde_json::from_value(serde_json::to_value(source.key).unwrap()).unwrap();
    assert_eq!(key, lost.ack.key);
    key
}

fn assert_scope_refused(
    conn: &rusqlite::Connection,
    caller: &unleash_store::CallerAuthority,
    target: TerminalReleaseHistoryTarget,
) {
    use unleash_store::StoreError;

    for foreign in [
        TerminalReleaseHistoryTarget {
            project_id: unleash_domain::ProjectId::from_u128(0xf011_8203),
            ..target
        },
        TerminalReleaseHistoryTarget {
            process_run_uuid: unleash_domain::ProcessRunUuid::from_u128(0xf011_8204),
            ..target
        },
    ] {
        assert!(matches!(
            qualify_terminal_release_history_source(conn, caller, foreign),
            Err(StoreError::NotFound(_))
        ));
    }
    let wrong_actor = unleash_store::CallerAuthority {
        actor_id: unleash_domain::ActorId::new("user:unrelated").unwrap(),
        ..caller.clone()
    };
    let wrong_role = unleash_store::CallerAuthority {
        role: unleash_store::CallerRole::Agent,
        ..caller.clone()
    };
    let wrong_dispatch = TerminalReleaseHistoryTarget {
        dispatch_id: unleash_domain::DispatchId::from_u128(0xf011_8205),
        ..target
    };
    assert!(matches!(
        qualify_terminal_release_history_source(conn, &wrong_actor, target),
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        qualify_terminal_release_history_source(conn, &wrong_role, target),
        Err(StoreError::Unauthorized(_))
    ));
    assert!(matches!(
        qualify_terminal_release_history_source(conn, caller, wrong_dispatch),
        Err(StoreError::InvalidArgument { .. })
    ));
}

fn inspector(lost: &LostRelease) -> (HostClient, ControlReleaseRequest) {
    let key = qualify_source(lost);
    let header = &key.header;
    let secret = unleash_process_host::load_control_secret(
        &lost
            .native
            .socket_path
            .parent()
            .unwrap()
            .join(unleash_process_host::CONTROL_SECRET_FILENAME),
    )
    .unwrap();
    let client = HostClient::connect(
        &lost.native.socket_path,
        header.process_run_uuid,
        header.authority_generation,
        header.daemon_epoch,
        header.resource_fence,
    )
    .unwrap();
    // The CURRENT effective credential is opaque. Reusing it in both fields
    // makes the Active/cache-missing route fail before any new rotation.
    let request = ControlReleaseRequest {
        key,
        current_secret: secret.expose_secret().into(),
        next_secret: secret.expose_secret().into(),
    };
    (client, request)
}

fn exact_native_ack(
    client: &mut HostClient,
    request: &ControlReleaseRequest,
    expected: &ControlReleaseAck,
) {
    match client.release_control(request).unwrap() {
        ControlReleaseResponse::Ack { ack } => assert_eq!(*ack, *expected),
        other @ ControlReleaseResponse::Refused { .. } => {
            panic!("exact historical native acknowledgement unavailable: {other:?}")
        }
    }
}

fn foreign_key_refused(client: &mut HostClient, request: &ControlReleaseRequest) {
    let mut different = request.clone();
    different.key.header.command_id = CommandId::from_u128(0xf011_8201);
    assert!(matches!(client.release_control(&different).unwrap(),
        ControlReleaseResponse::Refused { error } if error.code == ErrorCode::OutcomeUnknown));
    // A different key is not evidence of the original retirement. Do not expect
    // StaleFence here: the original old epoch/fence is intentionally historical.
}

async fn replay_original_receipt(lost: &mut LostRelease) {
    let fixture = &mut lost.harness.fixture;
    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_8202), lost.request.command_id,)
            .await
            .unwrap()
            .receipt,
        lost.receipt
    );
    assert_eq!(
        fixture.client.send_command(&lost.request).await.unwrap(),
        lost.receipt
    );
}

#[tokio::test]
async fn t_f011_010_historical_native_release_ack_is_exact_after_restart_without_second_retirement()
{
    let mut lost = lose_native_ack().await;
    assert_quarantined(&lost);
    let (mut client, request) = inspector(&lost);
    exact_native_ack(&mut client, &request, &lost.ack);
    foreign_key_refused(&mut client, &request);
    exact_native_ack(&mut client, &request, &lost.ack);
    lost.old.assert_revoked();
    assert_quarantined(&lost);
    let output = fresh_pty_output(&lost.native, lost.output).await;
    assert!(
        output > lost.output,
        "the retained live child still writes to the same PTY"
    );
    replay_original_receipt(&mut lost).await;
    refuse_new_control(&mut lost.harness.fixture, &lost.acquired).await;
    assert_quarantined(&lost);
    // Completed/reaped daemon drain excludes a delayed native retry; no sleep
    // alone is used as evidence that the outstanding mutators have stopped.
    drain(&mut lost.harness.fixture).await;
    exact_native_ack(&mut client, &request, &lost.ack);
    assert_quarantined(&lost);
}
