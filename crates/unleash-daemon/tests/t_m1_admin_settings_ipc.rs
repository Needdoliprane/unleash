//! REQ-CORE-003/004/005 et REQ-SEC-009 : paramètres et politique par IPC.
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
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReasonCode,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_testkit::local_test_dir;
fn obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("objet attendu"),
    }
}
#[tokio::test]
async fn settings_and_policy_are_authorized_typed_and_revisioned() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-settings-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-settings-data").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let sock = handle.socket_path().to_path_buf();
    let mut admin =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    let mut unauthorized = UnleashClient::connect(
        ClientConfig::new(sock, ClientKind::Cli, "test").without_admin_credential(),
    )
    .await
    .unwrap();
    let generation = admin.hello().authority_generation;
    let read = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(1),
        query_type: "ReadSettings".into(),
        project_id: None,
        payload: obj(json!({"scope_kind":"user","scope_id":"default"})),
    };
    assert_eq!(
        admin.query(&read).await.unwrap().result["execution_pool"]["max_active_executions"],
        5
    );
    let (_, cursor) = admin
        .get_state_snapshot(RequestId::from_u128(11), None, Map::new())
        .await
        .unwrap();
    let mut events = admin
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: cursor.stream_id,
            after_sequence: Sequence::new(cursor.sequence.as_u64().saturating_sub(1)),
            project_id: None,
            aggregate_id: None,
            batch_limit: 32,
        })
        .await
        .unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(2),
        command_id: CommandId::from_u128(3),
        authority_generation: generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: generation.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "SetSettings".into(),
        payload: obj(
            json!({"scope_kind":"user", "scope_id":"default", "patch":{"execution_pool":{"max_active_executions":4,"control_reserve_mode":"soft","control_reserved_units":1,"control_burst_limit":2,"checkpoint_escalation_s":60,"project_weight":1}, "confirm_retention_shortening":false}}),
        ),
    };
    assert!(
        matches!(unauthorized.send_command(&cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    let receipt = admin.send_command(&cmd).await.unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(receipt.new_revision, Revision::new(1));
    assert_eq!(
        admin.query(&read).await.unwrap().result["execution_pool"]["max_active_executions"],
        4
    );
    assert_eq!(
        admin.send_command(&cmd).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    let mut invalid = cmd.clone();
    invalid.command_id = CommandId::from_u128(4);
    invalid.expected_revision = Revision::new(1);
    invalid.payload.get_mut("patch").unwrap()["execution_pool"]["unknown"] = json!("DO_NOT_ECHO");
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO"))
    );
    invalid.payload = cmd.payload.clone();
    invalid.payload.get_mut("patch").unwrap()["extra_typed"] = json!({"secret":"x"});
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    invalid.payload = cmd.payload.clone();
    invalid.payload.get_mut("patch").unwrap()["execution_pool"]["max_active_executions"] = json!(1);
    invalid.payload.get_mut("patch").unwrap()["execution_pool"]["control_reserve_mode"] =
        json!("hard");
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && e.reason_code == Some(ReasonCode::ControlReserveIncoherent))
    );
    invalid.payload = cmd.payload.clone();
    invalid.aggregate_id = uuid::Uuid::from_u128(1234);
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    invalid.aggregate_id = generation.as_uuid();
    invalid.expected_revision = Revision::INITIAL;
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::RevisionConflict)
    );
    invalid.expected_revision = Revision::new(1);
    invalid.actor_ref = Some("forged-actor".into());
    assert!(
        matches!(admin.send_command(&invalid).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::AuthenticationFailed)
    );
    cmd.command_id = CommandId::from_u128(5);
    cmd.command_type = "SetPolicy".into();
    cmd.payload = obj(
        json!({"scope_kind":"global","scope_id":"global","isolation_mode":"standard","policy_version":"v1","reason":"test"}),
    );
    let policy_receipt = admin.send_command(&cmd).await.unwrap();
    assert_eq!(policy_receipt.new_revision, Revision::new(1));
    let policy_event = tokio::time::timeout(std::time::Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(policy_event, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "SecurityPolicyUpdated" && e.aggregate_id == generation.as_uuid() && e.sequence == policy_receipt.event_cursor.sequence))
    );
    cmd.command_id = CommandId::from_u128(6);
    cmd.command_type = "ConfigureGuard".into();
    cmd.payload = obj(
        json!({"config":{"config_id":"default","guard_mode":"broker_only","endpoint":null,"transport":"none","secret_ref_id":null,"evaluate_timeout_ms":2000,"health_check_interval_s":30,"decision_cache_max_age_s":300,"fallback_policy_ref":null,"guard_mapping_profile":"none_accepted","reason":"test"}}),
    );
    let guard_receipt = admin.send_command(&cmd).await.unwrap();
    assert_eq!(guard_receipt.new_revision, Revision::new(1));
    let guard_event = tokio::time::timeout(std::time::Duration::from_secs(5), events.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(guard_event, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "GuardConfigured" && e.aggregate_id == generation.as_uuid() && e.sequence == guard_receipt.event_cursor.sequence))
    );
    let health_query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(25),
        query_type: "ReadGuardHealth".into(),
        project_id: None,
        payload: Map::new(),
    };
    let health = admin.query(&health_query).await.unwrap();
    assert_eq!(health.result["guard_mode"], "broker_only");
    assert_eq!(health.result["guard_mapping_profile"], "none_accepted");
    assert_eq!(health.result["health_status"], "unknown");
    assert!(health.result["negotiated_version"].is_null());
    assert!(health.result.get("endpoint").is_none());
    assert_eq!(health.watermark, Some(guard_receipt.event_cursor));
    let mut bad_guard = cmd.clone();
    bad_guard.command_id = CommandId::from_u128(7);
    bad_guard.expected_revision = Revision::new(1);
    bad_guard.payload.get_mut("config").unwrap()["unknown"] = json!("DO_NOT_ECHO");
    assert!(
        matches!(admin.send_command(&bad_guard).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO"))
    );
    bad_guard.payload = cmd.payload.clone();
    bad_guard.payload.get_mut("config").unwrap()["guard_mode"] = json!("guard_required");
    bad_guard.payload.get_mut("config").unwrap()["endpoint"] = json!("https://DO_NOT_ECHO.invalid");
    assert!(
        matches!(admin.send_command(&bad_guard).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO"))
    );
    let project = ProjectId::from_u128(500);
    let mut project_cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(15),
        command_id: CommandId::from_u128(16),
        authority_generation: generation,
        actor_ref: None,
        project_id: Some(project),
        aggregate_id: project.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".into(),
        payload: obj(json!({"name":"projet","weight":1})),
    };
    let created = admin.send_command(&project_cmd).await.unwrap();
    let mut project_events = admin
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: created.event_cursor.stream_id,
            after_sequence: Sequence::new(created.event_cursor.sequence.as_u64().saturating_sub(1)),
            project_id: None,
            aggregate_id: None,
            batch_limit: 32,
        })
        .await
        .unwrap();
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        project_events.next_frame(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    project_cmd.command_id = CommandId::from_u128(17);
    project_cmd.command_type = "SetSettings".into();
    project_cmd.payload = obj(
        json!({"scope_kind":"project", "scope_id":project.to_string(), "patch":{"confirm_retention_shortening":false}}),
    );
    let project_receipt = admin.send_command(&project_cmd).await.unwrap();
    let project_event = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        project_events.next_frame(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    project_cmd.command_id = CommandId::from_u128(19);
    project_cmd.command_type = "SetPolicy".into();
    project_cmd.payload = obj(
        json!({"scope_kind":"project", "scope_id":project.to_string(), "isolation_mode":"standard", "policy_version":"v1", "reason":"test"}),
    );
    let project_policy_receipt = admin.send_command(&project_cmd).await.unwrap();
    let project_policy_event = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        project_events.next_frame(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(
        matches!(project_policy_event, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "SecurityPolicyUpdated" && e.aggregate_id == project.as_uuid() && e.project_id == Some(project) && e.sequence == project_policy_receipt.event_cursor.sequence))
    );
    let missing = ProjectId::from_u128(501);
    project_cmd.command_id = CommandId::from_u128(18);
    project_cmd.project_id = Some(missing);
    project_cmd.aggregate_id = missing.as_uuid();
    project_cmd.command_type = "SetPolicy".into();
    project_cmd.payload = obj(
        json!({"scope_kind":"project", "scope_id":missing.to_string(), "isolation_mode":"standard", "policy_version":"v1", "reason":"test"}),
    );
    assert!(
        matches!(admin.send_command(&project_cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::NotFoundOrHidden)
    );
    project_cmd.command_id = CommandId::from_u128(20);
    project_cmd.command_type = "SetSettings".into();
    project_cmd.payload = obj(
        json!({"scope_kind":"project", "scope_id":missing.to_string(), "patch":{"confirm_retention_shortening":false}}),
    );
    assert!(
        matches!(admin.send_command(&project_cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::NotFoundOrHidden)
    );
    assert!(
        matches!(project_event, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "SettingsUpdated" && e.aggregate_id == project.as_uuid() && e.project_id == Some(project) && e.sequence == project_receipt.event_cursor.sequence))
    );
    assert!(
        matches!(event, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.event_type == "SettingsUpdated" && e.aggregate_id == generation.as_uuid() && e.sequence == receipt.event_cursor.sequence))
    );
    handle.shutdown_graceful("test").await.unwrap();
    // Une observation legacy simulée et périmée ne devient pas une preuve Guard réelle.
    let fixture_db = rusqlite::Connection::open(data.path().join("unleash-authority.db")).unwrap();
    fixture_db.execute_batch("UPDATE guard_configs SET health_status='healthy', negotiated_version='stale', capabilities_json='[\"allow\"]', last_health_check_ms=1 WHERE config_id='default'").unwrap();
    drop(fixture_db);
    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon redémarré attendu")
    };
    let mut reopened = UnleashClient::connect(ClientConfig::new(
        restarted.socket_path().to_path_buf(),
        ClientKind::Cli,
        "test",
    ))
    .await
    .unwrap();
    let health_after_restart = reopened.query(&health_query).await.unwrap();
    assert_eq!(health_after_restart.result["health_status"], "unknown");
    assert!(health_after_restart.result["negotiated_version"].is_null());
    assert_eq!(health_after_restart.result["capabilities"], json!([]));
    let policy = reopened
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(26),
            query_type: "ReadPolicySummary".into(),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(policy.result["guard_config"]["health_status"], "unknown");
    assert!(policy.result["guard_config"]["negotiated_version"].is_null());
    restarted.shutdown_graceful("test").await.unwrap();
}
