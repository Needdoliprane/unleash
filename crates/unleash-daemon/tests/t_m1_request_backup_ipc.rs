//! REQ-REC-002 / REQ-PERSIST-010 : sauvegarde IPC authentifiée et journalisée.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use serde_json::{Map, json};
use std::time::Duration;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_store::{StoreConfig, UnleashStore};
use unleash_testkit::local_test_dir;

#[tokio::test]
async fn request_backup_is_authenticated_idempotent_and_physically_verified() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-request-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-request-backup-data").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon attendu");
    };
    let socket = handle.socket_path().to_path_buf();
    let mut admin = UnleashClient::connect(ClientConfig::new(
        socket.clone(),
        ClientKind::Cli,
        "backup-test",
    ))
    .await
    .unwrap();
    let mut no_proof = UnleashClient::connect(
        ClientConfig::new(socket, ClientKind::Cli, "backup-test").without_admin_credential(),
    )
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0xb001);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xb002),
        command_id: CommandId::from_u128(0xb003),
        authority_generation: admin.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    assert!(matches!(no_proof.send_command(&command).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied));
    let mut invalid = command.clone();
    invalid.command_id = CommandId::from_u128(0xb004);
    invalid
        .payload
        .insert("destination".into(), json!("SHOULD_NOT_BE_ECHOED"));
    assert!(matches!(admin.send_command(&invalid).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("SHOULD_NOT_BE_ECHOED")));
    let mut invalid_id = command.clone();
    invalid_id.command_id = CommandId::from_u128(0xb005);
    invalid_id.aggregate_id = uuid::Uuid::nil();
    assert!(matches!(admin.send_command(&invalid_id).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument));
    let mut invalid_scope = command.clone();
    invalid_scope.command_id = CommandId::from_u128(0xb006);
    invalid_scope.project_id = Some(unleash_domain::ProjectId::from_u128(1));
    assert!(matches!(admin.send_command(&invalid_scope).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument));
    let mut invalid_revision = command.clone();
    invalid_revision.command_id = CommandId::from_u128(0xb007);
    invalid_revision.expected_revision = Revision::new(1);
    assert!(matches!(admin.send_command(&invalid_revision).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::RevisionConflict));

    let preflight_cfg = StoreConfig::new(data.path());
    assert!(
        !preflight_cfg
            .backups_dir
            .join(backup_id.to_string())
            .exists()
    );
    let preflight_db = preflight_cfg.db_path;
    let reserved_before = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(preflight_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backup_requests')",
            [], |row| row.get(0),
        ).unwrap();
        if table_exists {
            conn.query_row("SELECT COUNT(*) FROM backup_requests WHERE backup_id=?1",
                [backup_id.to_string()], |row| row.get::<_, i64>(0)).unwrap()
        } else { 0 }
    }).await.unwrap();
    assert_eq!(reserved_before, 0);
    let mut missing_query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xb009),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
    };
    assert!(matches!(no_proof.query(&missing_query).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied));
    assert!(matches!(admin.query(&missing_query).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::NotFoundOrHidden));
    missing_query
        .payload
        .insert("unexpected".into(), json!("DO_NOT_ECHO"));
    assert!(matches!(admin.query(&missing_query).await,
        Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("DO_NOT_ECHO")));
    let receipt = admin.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        admin.send_command(&command).await.unwrap().receipt_id,
        receipt.receipt_id
    );

    let store_cfg = StoreConfig::new(data.path());
    let db_path = store_cfg.db_path.clone();
    let backup_dir = store_cfg.backups_dir.join(backup_id.to_string());
    let observed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let db_path = db_path.clone();
            let status = tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                conn.query_row("SELECT b.state, a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=a.action_id) FROM backup_requests r JOIN backup_jobs b USING(backup_id) JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
                    [backup_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,i64>(2)?))).ok()
            }).await.unwrap();
            if status.as_ref().is_some_and(|(job, action, _)| job == "complete" && action == "observed_success") {
                break status.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("le reçu Accepted ne prouve pas la sauvegarde ; job/observation attendus");
    assert_eq!(observed.2, 1);
    let verified =
        tokio::task::spawn_blocking(move || UnleashStore::validate_backup(&backup_dir, &store_cfg))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(verified.backup_id.as_uuid(), backup_id);
    assert!(verified.integrity_check_ok);
    assert_eq!(verified.foreign_key_violations, 0);
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xb008),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
    };
    let result = admin.query(&query).await.unwrap();
    assert_eq!(result.result["backup_id"], backup_id.to_string());
    assert_eq!(result.result["state"], "completed");
    assert_eq!(
        result.result["manifest"]["backup_id"],
        backup_id.to_string()
    );
    assert!(result.result["manifest_digest"].as_str().is_some());
    assert!(result.watermark.is_some());
    handle.shutdown_graceful("test").await.unwrap();
}
