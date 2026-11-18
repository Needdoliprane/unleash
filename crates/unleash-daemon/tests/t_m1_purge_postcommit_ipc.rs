//! REQ-PERSIST-001 : diffusion du commit même si la finalisation retourne une issue incertaine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use serde_json::{Map, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, StoreOverrides, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision, Sequence};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, RetryClass, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};
use unleash_testkit::local_test_dir;
struct AfterCommit(AtomicBool);
impl CutpointObserver for AfterCommit {
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
async fn committed_event_is_published_after_uncertain_response() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-c2-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-c2-data").unwrap();
    let cut = Arc::new(AfterCommit(AtomicBool::new(false)));
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: StoreOverrides {
            cutpoints: Some(cut.clone()),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "test",
    ))
    .await
    .unwrap();
    let (_, cursor) = client
        .get_state_snapshot(RequestId::from_u128(1), None, Map::new())
        .await
        .unwrap();
    let mut stream = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: cursor.stream_id,
            after_sequence: Sequence::new(cursor.sequence.as_u64().saturating_sub(1)),
            project_id: None,
            aggregate_id: None,
            batch_limit: 32,
        })
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(2),
        command_id: CommandId::from_u128(3),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: uuid::Uuid::from_u128(42),
        expected_revision: Revision::INITIAL,
        command_type: "PurgeHistory".into(),
        payload: json!({"scope_kind":"user","scope_id":"default","reason":"test"})
            .as_object()
            .unwrap()
            .clone(),
    };
    cut.0.store(true, Ordering::SeqCst);
    assert!(
        matches!(client.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.retry_class == RetryClass::AfterReconciliation)
    );
    let receipt = client
        .get_command_receipt(RequestId::from_u128(4), cmd.command_id)
        .await
        .unwrap();
    let report = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(5),
            query_type: "InspectRecovery".into(),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let purges = report
        .result
        .get("retention_purges")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(purges.len(), 1);
    let purge = purges.first().unwrap();
    assert_eq!(purge.get("state").unwrap(), "checkpoint_pending");
    assert_eq!(purge.get("wal_checkpoint_truncated").unwrap(), false);
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(frame, SubscribeEventsFrame::Events {events, ..} if events.iter().any(|e| e.event_type == "HistoryPurged" && e.sequence == receipt.receipt.event_cursor.sequence))
    );
    handle.shutdown_graceful("test").await.unwrap();
}
