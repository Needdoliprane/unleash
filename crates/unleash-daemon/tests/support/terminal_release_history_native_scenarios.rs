//! Native prerequisite of historical Release reconciliation, with no proposed wire.
//! The observed historical ACK grants no current terminal capability.
use super::*;
use unleash_client::{ClientError, UnleashClient};
use unleash_process_host::{ControlReleaseAck, ControlReleaseRequest, ControlReleaseResponse};
use unleash_store::terminal_allocations::terminal_release_history::{
    SealTerminalReleaseHistoryRequest, TERMINAL_RELEASE_HISTORY_EVENT,
    TerminalReleaseHistoryTarget, qualify_terminal_release_history_source,
};
use unleash_store::terminal_allocations::terminal_release_types::{
    TerminalReleaseContext, TerminalReleaseObservation,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError, UnleashStore};

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

fn history_target(
    lost: &LostRelease,
) -> (
    unleash_store::CallerAuthority,
    TerminalReleaseHistoryTarget,
    TerminalReleaseContext,
) {
    let context: TerminalReleaseContext = serde_json::from_str(&lost.context_json).unwrap();
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
    (caller, target, context)
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
    let (caller, target, context) = history_target(lost);
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
) -> ControlReleaseAck {
    match client.release_control(request).unwrap() {
        ControlReleaseResponse::Ack { ack } => {
            assert_eq!(*ack, *expected);
            *ack
        }
        other @ ControlReleaseResponse::Refused { .. } => {
            panic!("exact historical native acknowledgement unavailable: {other:?}")
        }
    }
}

fn prepare_history(
    store: &unleash_store::UnleashStore,
    caller: unleash_store::CallerAuthority,
    target: TerminalReleaseHistoryTarget,
    observation: &TerminalReleaseObservation,
) -> SealTerminalReleaseHistoryRequest {
    let bytes = unleash_protocol::canonicalize_to_vec(observation).unwrap();
    let prepared_proof = store
        .prepare_object(unleash_store::PrepareObjectRequest {
            object_hash: unleash_domain::Sha256Digest::from_bytes(&bytes),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "action".into(),
            owner_id: target.action_id.to_string(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&prepared_proof, &bytes)
        .unwrap();
    SealTerminalReleaseHistoryRequest {
        caller,
        target,
        prepared_proof,
    }
}

#[tokio::test]
async fn t_f011_010_historical_native_release_can_be_sealed_without_disposition() {
    let mut lost = lose_native_ack().await;
    assert_quarantined(&lost);
    let (mut client, request) = inspector(&lost);
    drain(&mut lost.harness.fixture).await;
    let ack = exact_native_ack(&mut client, &request, &lost.ack);
    let manifest_digest =
        unleash_protocol::canonical_sha256(&lost.harness.fixture.host_manifest()).unwrap();
    let (caller, target, _) = history_target(&lost);
    let data = lost.harness.fixture.data.path().to_path_buf();
    let accepted = lost.receipt.receipt_id;
    tokio::task::spawn_blocking(move || {
        assert_history_seals(data, caller, target, ack, manifest_digest, accepted);
    })
    .await
    .unwrap();
    assert_quarantined(&lost);
}

fn assert_history_seals(
    data: PathBuf,
    caller: unleash_store::CallerAuthority,
    target: TerminalReleaseHistoryTarget,
    ack: ControlReleaseAck,
    manifest_digest: unleash_domain::Sha256Digest,
    accepted: unleash_domain::ReceiptId,
) {
    let fault = std::sync::Arc::new(HistoryFault::default());
    let mut config = unleash_store::StoreConfig::new(data);
    config.cutpoints = fault.clone();
    let store = UnleashStore::open(config.clone()).unwrap();
    let observation = TerminalReleaseObservation {
        schema_version: 1,
        ack: serde_json::from_value(serde_json::to_value(ack).unwrap()).unwrap(),
        manifest_digest,
        captured_at_ms: store.config().clock.now_ms(),
    };
    let seal = prepare_history(&store, caller.clone(), target, &observation);
    let (events, refs, outbox, revision) = history_counts(&store, target);
    assert_eq!((events, refs, outbox), (0, 0, 0));
    fault.arm(StoreCutpoint::C1BeforeCommandCommit);
    assert!(matches!(
        store.seal_terminal_release_history(seal.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(history_counts(&store, target), (0, 0, 0, revision));

    fault.arm(StoreCutpoint::C2AfterCommandCommit);
    assert!(matches!(
        store.seal_terminal_release_history(seal.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(history_counts(&store, target), (1, 1, 1, revision));
    store.shutdown().unwrap();
    let store = UnleashStore::open(config).unwrap();
    let first = store.seal_terminal_release_history(seal).unwrap();
    assert!(first.replayed);
    assert_eq!(first.fact.source_receipt_id, accepted);
    assert_eq!(
        first.fact.native_ack_digest,
        unleash_protocol::canonical_sha256(&observation.ack).unwrap()
    );

    let duplicate = prepare_history(&store, caller.clone(), target, &observation);
    assert_eq!(
        store.seal_terminal_release_history(duplicate).unwrap(),
        first
    );
    let mut later = observation.clone();
    // Simulate a later inspection of the same complete cached ACK.
    std::thread::sleep(std::time::Duration::from_millis(2));
    later.captured_at_ms = store.config().clock.now_ms();
    assert!(later.captured_at_ms > observation.captured_at_ms);
    let update = prepare_history(&store, caller.clone(), target, &later);
    assert_ne!(
        update.prepared_proof.object_hash,
        first.fact.observation_object_hash
    );
    assert_eq!(store.seal_terminal_release_history(update).unwrap(), first);
    later.ack.durable_log_seq += 1;
    let divergent = prepare_history(&store, caller, target, &later);
    assert!(matches!(
        store.seal_terminal_release_history(divergent),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(history_counts(&store, target), (1, 1, 1, revision));
    let event_receipt: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT causation_receipt_id FROM events
             WHERE aggregate_id=?1 AND event_type=?2 AND sequence=?3",
                rusqlite::params![
                    target.action_id.to_string(),
                    TERMINAL_RELEASE_HISTORY_EVENT,
                    i64::try_from(first.fact.event_cursor.sequence.as_u64()).unwrap()
                ],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(event_receipt, accepted.to_string());
    store.shutdown().unwrap();
}

#[derive(Default)]
struct HistoryFault(std::sync::Mutex<Option<StoreCutpoint>>);

impl HistoryFault {
    fn arm(&self, cutpoint: StoreCutpoint) {
        *self.0.lock().unwrap() = Some(cutpoint);
    }
}

impl CutpointObserver for HistoryFault {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let mut armed = self.0.lock().unwrap();
        if *armed == Some(cutpoint) {
            armed.take();
            return Err(StoreError::CutpointInjected {
                cutpoint: format!("{cutpoint:?}"),
                message: "fixture interruption of historical release publication".into(),
            });
        }
        Ok(())
    }
}

fn history_counts(
    store: &UnleashStore,
    target: TerminalReleaseHistoryTarget,
) -> (i64, i64, i64, i64) {
    store.read_snapshot(|conn, _| {
        Ok(conn.query_row(
            "SELECT
             (SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type=?2),
             (SELECT COUNT(*) FROM object_references WHERE owner_kind='action' AND owner_id=?1 AND source_field=?3),
             (SELECT COUNT(*) FROM outbox JOIN events USING(event_id) WHERE aggregate_id=?1 AND event_type=?2),
             (SELECT revision FROM terminal_release_intents WHERE action_id=?1)",
            rusqlite::params![target.action_id.to_string(), TERMINAL_RELEASE_HISTORY_EVENT,
                format!("terminal_release_history:{}", target.dispatch_id)],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        )?)
    }).unwrap()
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

#[tokio::test]
async fn t_f011_010_reconcile_process_publishes_the_release_history_receipt() {
    let mut lost = lose_native_ack().await;
    let (_, target, _) = history_target(&lost);
    let envelope = command(
        lost.harness.fixture.authority,
        0xf011_8210,
        target.project_id,
        target.terminal_id.as_uuid(),
        lost.acquired.control_revision.as_u64(),
        "ReconcileProcess",
        json!({
            "kind":"attest_terminal_release",
            "terminal_id":target.terminal_id,
            "process_run_uuid":target.process_run_uuid,
            "source_action_id":target.action_id,
            "source_dispatch_id":target.dispatch_id,
        }),
    );
    let mut wrong_revision = envelope.clone();
    wrong_revision.expected_revision = envelope.expected_revision.next();
    let client = &mut lost.harness.fixture.client;
    let rejection = client.send_command(&wrong_revision).await.unwrap_err();
    assert!(
        matches!(&rejection, ClientError::Remote(error) if error.code == ErrorCode::RevisionConflict),
        "{rejection:?}"
    );
    let receipt = client.send_command(&envelope).await.unwrap();
    let reference = format!(
        "terminal-release-history:{}:{}",
        target.action_id, target.dispatch_id
    );
    assert_eq!(receipt.result_ref.as_deref(), Some(reference.as_str()));
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.new_revision, lost.acquired.control_revision);
    let query = unleash_protocol::QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf011_8211),
        query_type: "GetCommandReceipt".into(),
        project_id: Some(target.project_id),
        payload: object(json!({"command_id":receipt.command_id,"result_ref":reference})),
    };
    let first = client.query(&query).await.unwrap();
    assert_release_history_projection(&lost, &receipt, &first);
    let original = lost.receipt.command_id;
    let client = &mut lost.harness.fixture.client;
    assert_reconciliation_refusals(client, &envelope, &query, original).await;
    assert_reconciliation_replay(client, &envelope, &query, &receipt, &first.result).await;

    let mut another = envelope.clone();
    another.command_id = CommandId::from_u128(0xf011_8214);
    let referenced = client.send_command(&another).await.unwrap();
    assert_ne!(referenced.receipt_id, receipt.receipt_id);
    assert_eq!(referenced.result_ref, receipt.result_ref);
    assert_eq!(referenced.state, ReceiptState::Completed);

    assert_quarantined(&lost);
    lost.harness.fixture.daemon.kill_for_restart();
    lost.harness.fixture.start_successor().await;
    assert_reconciliation_replay(
        &mut lost.harness.fixture.client,
        &envelope,
        &query,
        &receipt,
        &first.result,
    )
    .await;
    assert_quarantined(&lost);
    drain(&mut lost.harness.fixture).await;
}

fn assert_release_history_projection(
    lost: &LostRelease,
    receipt: &CommandReceipt,
    response: &unleash_protocol::QueryResponse,
) {
    let result = &response.result;
    assert_eq!(result.get("receipt_id"), Some(&json!(receipt.receipt_id)));
    assert_eq!(
        result.get("source_receipt_id"),
        Some(&json!(lost.receipt.receipt_id))
    );
    assert_eq!(
        result.get("native_ack_digest"),
        Some(&json!(
            unleash_protocol::canonical_sha256(&lost.ack).unwrap()
        ))
    );
    let fact_cursor: unleash_protocol::EventCursor =
        serde_json::from_value(result.get("cursor").unwrap().clone()).unwrap();
    assert_eq!(fact_cursor.stream_id, receipt.event_cursor.stream_id);
    assert!(fact_cursor.sequence < receipt.event_cursor.sequence);
    assert!(
        response
            .watermark
            .is_some_and(|cursor| cursor.sequence >= receipt.event_cursor.sequence)
    );
}

async fn assert_reconciliation_refusals(
    client: &mut UnleashClient,
    envelope: &CommandEnvelope,
    query: &unleash_protocol::QueryEnvelope,
    original: CommandId,
) {
    let mut conflicting = envelope.clone();
    conflicting.payload.insert(
        "source_dispatch_id".into(),
        json!(unleash_domain::DispatchId::from_u128(0xf011_8212)),
    );
    assert!(matches!(
        client.send_command(&conflicting).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::IdempotencyConflict
    ));
    let mut foreign_query = query.clone();
    foreign_query.project_id = Some(unleash_domain::ProjectId::from_u128(0xf011_8213));
    assert!(client.query(&foreign_query).await.is_err());
    let mut old_receipt = query.clone();
    old_receipt
        .payload
        .insert("command_id".into(), json!(original));
    assert!(client.query(&old_receipt).await.is_err());
}

async fn assert_reconciliation_replay(
    client: &mut UnleashClient,
    envelope: &CommandEnvelope,
    query: &unleash_protocol::QueryEnvelope,
    receipt: &CommandReceipt,
    result: &Value,
) {
    assert_eq!(client.send_command(envelope).await.unwrap(), *receipt);
    assert_eq!(&client.query(query).await.unwrap().result, result);
}
