//! REQ-REC-002 / REQ-PERSIST-010 : sauvegarde IPC authentifiée et journalisée.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use serde_json::{Map, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_store::{StoreConfig, UnleashStore};
use unleash_testkit::local_test_dir;

#[derive(Default)]
struct PauseBackup {
    entered: tokio::sync::Notify,
    release: Mutex<bool>,
    condvar: Condvar,
}

#[derive(Default)]
struct PauseDispatch {
    entered: tokio::sync::Notify,
    release: Mutex<bool>,
    condvar: Condvar,
}
impl unleash_store::CutpointObserver for PauseDispatch {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C4AfterBrokerDispatchCommit {
            self.entered.notify_one();
            let mut release = self.release.lock().unwrap();
            while !*release {
                release = self.condvar.wait(release).unwrap();
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct LoseBackupReceipt(AtomicBool);
impl unleash_store::CutpointObserver for LoseBackupReceipt {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C2AfterCommandCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(unleash_store::StoreError::CutpointInjected {
                cutpoint: "C2".into(),
                message: "test".into(),
            });
        }
        Ok(())
    }
}
impl unleash_store::CutpointObserver for PauseBackup {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C11aAfterBackupBarrier {
            self.entered.notify_one();
            let mut release = self.release.lock().unwrap();
            while !*release {
                release = self.condvar.wait(release).unwrap();
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct LoseBackupDispatch(AtomicBool);
impl unleash_store::CutpointObserver for LoseBackupDispatch {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C4AfterBrokerDispatchCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(unleash_store::StoreError::CutpointInjected {
                cutpoint: "C4".into(),
                message: "test".into(),
            });
        }
        Ok(())
    }
}

#[derive(Default)]
struct LoseBackupCompletion(AtomicBool);
impl unleash_store::CutpointObserver for LoseBackupCompletion {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C11fAfterBackupJobCommit
            && self.0.swap(false, Ordering::SeqCst)
        {
            return Err(unleash_store::StoreError::CutpointInjected {
                cutpoint: "C11f".into(),
                message: "test".into(),
            });
        }
        Ok(())
    }
}

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
        ClientConfig::new(socket.clone(), ClientKind::Cli, "backup-test")
            .without_admin_credential(),
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
    let mut concurrent = UnleashClient::connect(ClientConfig::new(
        socket,
        ClientKind::Cli,
        "backup-concurrent-test",
    ))
    .await
    .unwrap();
    let (receipt, duplicate) = tokio::join!(
        admin.send_command(&command),
        concurrent.send_command(&command)
    );
    let receipt = receipt.unwrap();
    assert_eq!(duplicate.unwrap().receipt_id, receipt.receipt_id);
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

#[tokio::test]
async fn required_guard_without_adapter_blocks_backup_before_dispatch() {
    use std::os::unix::fs::PermissionsExt;
    use unleash_domain::ActorId;
    use unleash_store::{
        AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
        GuardConfigInput, GuardPolicyMode, GuardTransport,
    };
    let rdv = local_test_dir("m1-guard-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-guard-backup-data").unwrap();
    let data_dir = data.path().to_path_buf();
    let seed_dir = data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(StoreConfig::new(seed_dir)).unwrap();
        let generation = store.daemon_state().unwrap().authority_generation;
        let actor = ActorId::new("fixture-admin").unwrap();
        store
            .commit_command(CommitCommandRequest {
                actor_id: actor.clone(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(910),
                    command_id: CommandId::from_u128(911),
                    authority_generation: generation,
                    actor_ref: None,
                    project_id: None,
                    aggregate_id: generation.as_uuid(),
                    expected_revision: Revision::INITIAL,
                    command_type: "ConfigureGuard".into(),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::GuardConfig,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::ConfigureGuard {
                    config: GuardConfigInput {
                        config_id: "default".into(),
                        guard_mode: GuardPolicyMode::GuardRequired,
                        endpoint: Some("unix:///nonexistent-test-guard.sock".into()),
                        transport: GuardTransport::UnixSocket,
                        secret_ref_id: None,
                        evaluate_timeout_ms: 2000,
                        health_check_interval_s: 30,
                        decision_cache_max_age_s: 0,
                        fallback_policy_ref: None,
                        guard_mapping_profile: "none_accepted".into(),
                        reason: "test fixture".into(),
                    },
                    caller: CallerAuthority {
                        actor_id: actor,
                        role: CallerRole::PolicyAdmin,
                    },
                },
                events: Vec::new(),
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap();
    })
    .await
    .unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data_dir.clone()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "guard-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(920);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(921),
        command_id: CommandId::from_u128(922),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(923),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
    };
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = client.query(&query).await.unwrap();
            if result.result["state"] == "blocked" {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("refus Guard durable attendu");
    assert_eq!(result.result["blocked_reason"], "guard_profile_unavailable");
    assert!(result.result["manifest"].is_null());
    let store_cfg = StoreConfig::new(&data_dir);
    assert!(!store_cfg.backups_dir.join(backup_id.to_string()).exists());
    let count = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(store_cfg.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT COUNT(*) FROM action_dispatches d JOIN backup_requests r ON r.action_id=d.action_id WHERE r.backup_id=?1", [backup_id.to_string()], |r| r.get::<_,i64>(0)).unwrap()
    }).await.unwrap();
    assert_eq!(count, 0);
    handle.shutdown_graceful("test").await.unwrap();
}

#[tokio::test]
async fn stopping_during_real_backup_retains_authority_until_writer_finishes() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-stop-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-stop-backup-data").unwrap();
    let pause = Arc::new(PauseBackup::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        shutdown_drain_timeout: Duration::from_millis(200),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(pause.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "stop-backup-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0xc001);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xc002),
        command_id: CommandId::from_u128(0xc003),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    assert_eq!(
        client.send_command(&command).await.unwrap().state,
        ReceiptState::Accepted
    );
    tokio::time::timeout(Duration::from_secs(10), pause.entered.notified())
        .await
        .unwrap();
    let in_progress = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xc004),
            query_type: "GetBackupManifest".into(),
            project_id: None,
            payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
        })
        .await
        .unwrap();
    assert_eq!(in_progress.result["state"], "executing");
    assert!(in_progress.result["manifest"].is_null());
    assert!(handle.shutdown_graceful("backup actif").await.is_err());
    assert!(matches!(
        UnleashDaemon::start(cfg.clone()).await,
        Err(unleash_daemon::DaemonError::DaemonAlreadyRunning { .. }
            | unleash_daemon::DaemonError::AuthorityAlreadyActive { .. })
    ));
    *pause.release.lock().unwrap() = true;
    pause.condvar.notify_one();
    drop(client);
    handle.shutdown_graceful("backup libéré").await.unwrap();
    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("redémarrage attendu")
    };
    let db_path = StoreConfig::new(data.path()).db_path;
    let (action_state, dispatches) = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=a.action_id) FROM backup_requests r JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
            [backup_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?))).unwrap()
    }).await.unwrap();
    assert_eq!(action_state, "outcome_unknown");
    assert_eq!(dispatches, 1);
    restarted.shutdown_graceful("test").await.unwrap();
}

#[tokio::test]
async fn lost_backup_receipt_is_reconciled_before_a_single_dispatch() {
    use std::os::unix::fs::PermissionsExt;
    use unleash_protocol::RetryClass;
    let rdv = local_test_dir("m1-lost-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-lost-backup-data").unwrap();
    let cut = Arc::new(LoseBackupReceipt::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(cut.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "lost-backup-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0xd001);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xd002),
        command_id: CommandId::from_u128(0xd003),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    cut.0.store(true, Ordering::SeqCst);
    assert!(
        matches!(client.send_command(&command).await, Err(ClientError::Remote(e))
        if e.retry_class == RetryClass::AfterReconciliation)
    );
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xd004),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
    };
    let pending = client.query(&query).await.unwrap();
    if pending.result["state"] != "completed" {
        assert!(pending.result["manifest"].is_null());
    }
    let store_cfg = StoreConfig::new(data.path());
    // La reprise doit se faire depuis la réservation attestée, sans attendre
    // qu'un client renvoie la commande.
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let result = client.query(&query).await.unwrap();
            if result.result["state"] == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(
        client.send_command(&command).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    let count = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(store_cfg.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT COUNT(*) FROM action_dispatches d JOIN backup_requests r ON r.action_id=d.action_id WHERE r.backup_id=?1",
            [backup_id.to_string()], |r| r.get::<_,i64>(0)).unwrap()
    }).await.unwrap();
    assert_eq!(count, 1);
    handle.shutdown_graceful("test").await.unwrap();
}

#[tokio::test]
async fn lost_dispatch_ack_never_launches_or_reissues_backup() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-c4-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-c4-backup-data").unwrap();
    let cut = Arc::new(LoseBackupDispatch::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(cut.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "c4-backup-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0xe001);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xe002),
        command_id: CommandId::from_u128(0xe003),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    cut.0.store(true, Ordering::SeqCst);
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let cfg_store = StoreConfig::new(data.path());
    let db_path = cfg_store.db_path.clone();
    let backup_dir = cfg_store.backups_dir.join(backup_id.to_string());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let db_path = db_path.clone();
            let count = tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                conn.query_row("SELECT COUNT(*) FROM action_dispatches d JOIN backup_requests r ON r.action_id=d.action_id WHERE r.backup_id=?1",
                    [backup_id.to_string()], |r| r.get::<_, i64>(0)).unwrap()
            }).await.unwrap();
            if count == 1 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(!backup_dir.exists());
    assert_eq!(
        client.send_command(&command).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    assert!(!backup_dir.exists());
    drop(client);
    handle.shutdown_graceful("test").await.unwrap();
    assert!(!backup_dir.exists());
    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("redémarrage attendu")
    };
    let state = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(cfg_store.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=a.action_id) FROM backup_requests r JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
            [backup_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?))).unwrap()
    }).await.unwrap();
    assert_eq!(state, ("outcome_unknown".into(), 1));
    restarted.shutdown_graceful("test").await.unwrap();
}

#[tokio::test]
async fn stop_after_dispatch_keeps_lock_until_the_admitted_backup_finishes() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-stop-c4-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-stop-c4-data").unwrap();
    let pause = Arc::new(PauseDispatch::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        shutdown_drain_timeout: Duration::from_millis(200),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(pause.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "stop-c4-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0xf001);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0xf002),
        command_id: CommandId::from_u128(0xf003),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    assert_eq!(
        client.send_command(&command).await.unwrap().state,
        ReceiptState::Accepted
    );
    tokio::time::timeout(Duration::from_secs(10), pause.entered.notified())
        .await
        .unwrap();
    let store_cfg = StoreConfig::new(data.path());
    let backup_dir = store_cfg.backups_dir.join(backup_id.to_string());
    let db_path = store_cfg.db_path.clone();
    let before = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=a.action_id) FROM backup_requests r JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
            [backup_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?))).unwrap()
    }).await.unwrap();
    assert_eq!(before, ("dispatching".into(), 1));
    assert!(!backup_dir.exists());
    assert!(handle.shutdown_graceful("stop au marqueur").await.is_err());
    assert!(matches!(
        UnleashDaemon::start(cfg.clone()).await,
        Err(unleash_daemon::DaemonError::DaemonAlreadyRunning { .. }
            | unleash_daemon::DaemonError::AuthorityAlreadyActive { .. })
    ));
    *pause.release.lock().unwrap() = true;
    pause.condvar.notify_one();
    drop(client);
    handle.shutdown_graceful("drain terminé").await.unwrap();
    assert!(backup_dir.join("manifest.json").is_file());
    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("redémarrage attendu")
    };
    let after = tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open_with_flags(store_cfg.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=a.action_id) FROM backup_requests r JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
            [backup_id.to_string()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?))).unwrap()
    }).await.unwrap();
    assert_eq!(after, ("outcome_unknown".into(), 1));
    restarted.shutdown_graceful("test").await.unwrap();
}

#[tokio::test]
async fn completed_backup_is_distinguished_from_unknown_broker_observation() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-c11f-backup-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-c11f-backup-data").unwrap();
    let cut = Arc::new(LoseBackupCompletion::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(cut.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "c11f-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(0x1101);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x1102),
        command_id: CommandId::from_u128(0x1103),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x1104),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id":backup_id}).as_object().unwrap().clone(),
    };
    cut.0.store(true, Ordering::SeqCst);
    let receipt = client.send_command(&command).await.unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let result = client.query(&query).await.unwrap();
            if result.result["state"] == "completed" {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(completed.result["action_state"], "dispatching");
    assert_eq!(completed.result["lineage_status"], "current");
    assert_eq!(
        completed.result["authority_generation"],
        command.authority_generation.to_string()
    );
    assert!(completed.result["manifest"].is_object());
    assert!(completed.result["manifest_digest"].is_string());
    assert_eq!(
        client.send_command(&command).await.unwrap().receipt_id,
        receipt.receipt_id
    );
    drop(client);
    handle.shutdown_graceful("test").await.unwrap();
    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("redémarrage attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        restarted.socket_path().to_path_buf(),
        ClientKind::Cli,
        "c11f-test",
    ))
    .await
    .unwrap();
    let result = client.query(&query).await.unwrap();
    assert_eq!(result.result["state"], "completed");
    assert_eq!(result.result["action_state"], "outcome_unknown");
    assert_eq!(result.result["lineage_status"], "current");
    assert!(result.result["manifest"].is_object());
    restarted.shutdown_graceful("test").await.unwrap();
}
