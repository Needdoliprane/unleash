//! REQ-CORE-003/004/005, REQ-PERSIST-001/004: mutation IPC réelle et refus strict.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]
use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision, Sequence};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
use unleash_testkit::local_test_dir;

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(map) => map,
        _ => panic!("objet attendu"),
    }
}

#[tokio::test]
async fn revise_project_is_atomic_idempotent_and_rejects_unknown_fields() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-revise-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-revise-data").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
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
    let pid = ProjectId::from_u128(0x70);
    let mut cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(1),
        command_id: CommandId::from_u128(2),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: Some(pid),
        aggregate_id: pid.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".into(),
        payload: object(json!({"name":"avant", "weight":1})),
    };
    let mut invalid_create = cmd.clone();
    invalid_create.command_id = CommandId::from_u128(10);
    invalid_create
        .payload
        .insert("unexpected".into(), json!("DO_NOT_ECHO"));
    assert!(
        matches!(client.send_command(&invalid_create).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO"))
    );
    invalid_create.payload = object(json!({"name":"avant", "weight":"not-a-number"}));
    assert!(
        matches!(client.send_command(&invalid_create).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    let created = client.send_command(&cmd).await.unwrap();
    let mut stream = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: created.event_cursor.stream_id,
            after_sequence: Sequence::new(created.event_cursor.sequence.as_u64() - 1),
            project_id: Some(pid),
            aggregate_id: None,
            batch_limit: 16,
        })
        .await
        .unwrap();
    // La première trame confirme que le flux est actif avant la mutation.
    let initial = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(initial, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "ProjectCreated"))
    );
    cmd.command_type = "ReviseProject".into();
    cmd.command_id = CommandId::from_u128(3);
    cmd.expected_revision = Revision::new(1);
    cmd.payload = object(
        json!({"name":"après", "description":"description", "weight":2, "primary_root_path":null, "sources":[], "repositories":[], "settings_json":{}}),
    );
    let receipt = client.send_command(&cmd).await.unwrap();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(frame, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "ProjectRevised" && e.sequence == receipt.event_cursor.sequence))
    );
    assert_eq!(
        client.send_command(&cmd).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    let mut invalid = cmd.clone();
    invalid.command_id = CommandId::from_u128(4);
    invalid.expected_revision = Revision::new(2);
    invalid.payload.insert("unexpected".into(), json!(true));
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    invalid.payload.remove("unexpected");
    invalid.payload.insert("sources".into(), json!([{"kind":"directory", "path_or_uri":"/", "label":"root", "read_only":true, "unexpected":true}]));
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    invalid.payload = cmd.payload.clone();
    invalid
        .payload
        .insert("settings_json".into(), json!({"unknown_option":"secret"}));
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("secret"))
    );
    invalid.payload = cmd.payload.clone();
    invalid.aggregate_id = uuid::Uuid::from_u128(0x71);
    assert!(
        matches!(client.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    let mut probe = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Probe,
        "test",
    ))
    .await
    .unwrap();
    assert!(
        matches!(probe.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    let (snapshot, cursor) = client
        .get_state_snapshot(RequestId::from_u128(9), Some(pid), Map::new())
        .await
        .unwrap();
    assert_eq!(snapshot["projects"][0]["name"], "après");
    assert_eq!(snapshot["projects"][0]["revision"], 2);
    assert_eq!(cursor, receipt.event_cursor);
    let mut stale = cmd.clone();
    stale.command_id = CommandId::from_u128(5);
    assert!(
        matches!(client.send_command(&stale).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::RevisionConflict)
    );
    handle.shutdown_graceful("test").await.unwrap();
}
