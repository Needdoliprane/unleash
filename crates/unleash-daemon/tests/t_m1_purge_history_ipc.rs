//! REQ-REC-010 / REQ-PERSIST-006 : purge globale IPC et résultat physique observé.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]
use serde_json::{Map, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision, Sequence, Sha256Digest};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
    RetryClass, SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_store::{
    CutpointObserver, PrepareObjectRequest, PublishObjectReferenceRequest, RetentionClass,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

struct CutAfterCommit(AtomicBool);
impl CutpointObserver for CutAfterCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C2AfterCommandCommit && self.0.swap(false, Ordering::SeqCst) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C2".into(),
                message: "test".into(),
            });
        }
        Ok(())
    }
}

#[tokio::test]
async fn purge_is_authorized_and_reports_actual_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-purge-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-purge-data").unwrap();
    let clock = Arc::new(ManualClock::default());
    let cutpoint = Arc::new(CutAfterCommit(AtomicBool::new(false)));
    let ids = Arc::new(DeterministicIdGenerator::new(0x617));
    let dir = data.path().to_path_buf();
    let clock_for_fixture = Arc::clone(&clock);
    let ids_for_fixture = Arc::clone(&ids);
    let object_path = tokio::task::spawn_blocking(move || {
        let mut cfg = StoreConfig::new(&dir);
        cfg.clock = clock_for_fixture;
        cfg.id_generator = ids_for_fixture;
        let store = UnleashStore::open(cfg.clone()).unwrap();
        let bytes = b"expired-q12-object";
        let hash = Sha256Digest::from_bytes(bytes);
        let object = store
            .prepare_object(PrepareObjectRequest {
                object_hash: hash,
                byte_length: bytes.len() as u64,
                media_type: "text/plain".into(),
                owner_kind: "diagnostic".into(),
                owner_id: "fixture".into(),
            })
            .unwrap();
        store.write_prepared_object_bytes(&object, bytes).unwrap();
        store
            .publish_object_reference(PublishObjectReferenceRequest {
                pin_id: object.pin_id,
                object_hash: hash,
                object_generation: object.object_generation,
                owner_kind: "diagnostic".into(),
                owner_id: "fixture".into(),
                source_field: "payload".into(),
            })
            .unwrap();
        store
            .tag_object_retention(hash, RetentionClass::LogsTelemetry, false, None)
            .unwrap();
        cfg.artifacts_dir.join(object.relative_path)
    })
    .await
    .unwrap();
    clock.advance(Duration::from_secs(40 * 86_400));
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: StoreOverrides {
            clock: Some(clock),
            id_generator: Some(ids),
            cutpoints: Some(cutpoint.clone()),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon attendu")
    };
    assert!(object_path.is_file());
    let socket = handle.socket_path().to_path_buf();
    let mut client =
        UnleashClient::connect(ClientConfig::new(socket.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    let mut unauthorized = UnleashClient::connect(
        ClientConfig::new(socket, ClientKind::Cli, "test").without_admin_credential(),
    )
    .await
    .unwrap();
    let (_, cursor) = client
        .get_state_snapshot(RequestId::from_u128(1), None, Map::new())
        .await
        .unwrap();
    let mut events = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: cursor.stream_id,
            after_sequence: Sequence::new(cursor.sequence.as_u64().saturating_sub(1)),
            project_id: None,
            aggregate_id: None,
            batch_limit: 32,
        })
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let purge_aggregate = uuid::Uuid::from_u128(0x5000);
    let mut cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(2),
        command_id: CommandId::from_u128(3),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: purge_aggregate,
        expected_revision: Revision::INITIAL,
        command_type: "PurgeHistory".into(),
        payload: json!({"scope_kind":"user", "scope_id":"default", "reason":"retention test"})
            .as_object()
            .unwrap()
            .clone(),
    };
    assert!(
        matches!(unauthorized.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(object_path.is_file());
    let mut invalid = cmd.clone();
    invalid.command_id = CommandId::from_u128(4);
    invalid
        .payload
        .insert("unexpected".into(), json!("DO_NOT_ECHO"));
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO"))
    );
    invalid.payload = cmd.payload.clone();
    invalid
        .payload
        .insert("scope_kind".into(), json!("project"));
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    let receipt = client.send_command(&cmd).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert!(
        receipt
            .result_ref
            .as_deref()
            .unwrap()
            .starts_with("retention-purge:")
    );
    let expected_result_ref = format!("retention-purge:{purge_aggregate}");
    assert_eq!(
        receipt.result_ref.as_deref(),
        Some(expected_result_ref.as_str())
    );
    assert!(!object_path.exists());
    assert_eq!(
        client.send_command(&cmd).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    let report = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(9),
            query_type: "InspectRecovery".into(),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let purges = report.result["retention_purges"].as_array().unwrap();
    assert_eq!(purges.len(), 1);
    assert_eq!(
        purges[0]["command_receipt_id"],
        receipt.receipt_id.to_string()
    );
    assert_eq!(purges[0]["state"], "completed");
    assert_eq!(purges[0]["wal_checkpoint_truncated"], true);
    let frame = tokio::time::timeout(Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(frame, SubscribeEventsFrame::Events {events, ..} if events.iter().any(|e| e.event_type == "HistoryPurged" && e.aggregate_id == purge_aggregate && e.sequence == receipt.event_cursor.sequence))
    );
    let mut reused_aggregate = cmd.clone();
    reused_aggregate.command_id = CommandId::from_u128(6);
    assert!(
        matches!(client.send_command(&reused_aggregate).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::RevisionConflict)
    );
    cmd.command_id = CommandId::from_u128(5);
    cmd.expected_revision = Revision::new(1);
    assert!(
        matches!(client.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::RevisionConflict)
    );
    cmd.command_id = CommandId::from_u128(8);
    cmd.expected_revision = Revision::INITIAL;
    cmd.aggregate_id = uuid::Uuid::from_u128(0x5001);
    cutpoint.0.store(true, Ordering::SeqCst);
    assert!(
        matches!(client.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.retry_class == RetryClass::AfterReconciliation)
    );
    let committed = client
        .get_command_receipt(RequestId::from_u128(30), cmd.command_id)
        .await
        .unwrap();
    assert_eq!(committed.receipt.state, ReceiptState::Accepted);
    let after_error = tokio::time::timeout(Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(after_error, SubscribeEventsFrame::Events {events, ..} if events.iter().any(|e| e.event_type == "HistoryPurged" && e.aggregate_id == cmd.aggregate_id && e.sequence == committed.receipt.event_cursor.sequence))
    );
    handle.shutdown_graceful("test").await.unwrap();
}
