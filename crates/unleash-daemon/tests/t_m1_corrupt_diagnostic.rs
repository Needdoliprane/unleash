//! REQ-REC-001 : autorité de diagnostic sans fausse lignée.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use unleash_client::{ClientConfig, ClientError, DiagnosticClient, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonError, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::RequestId;
use unleash_protocol::{ClientKind, ErrorCode, ReasonCode};
use unleash_store::StoreConfig;
use unleash_testkit::local_test_dir;

async fn exchange(
    stream: &mut tokio::net::UnixStream,
    value: serde_json::Value,
) -> serde_json::Value {
    let bytes = serde_json::to_vec(&value).unwrap();
    stream
        .write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&bytes).await.unwrap();
    let len = stream.read_u32().await.unwrap();
    let mut response = vec![0; usize::try_from(len).unwrap()];
    stream.read_exact(&mut response).await.unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[tokio::test]
async fn corrupt_store_starts_locked_diagnostic_without_touching_database() {
    use std::os::unix::fs::PermissionsExt;
    let data = local_test_dir("m1-corrupt-data").unwrap();
    let rdv = local_test_dir("m1-corrupt-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let cfg_store = StoreConfig::new(data.path());
    std::fs::create_dir_all(cfg_store.db_path.parent().unwrap()).unwrap();
    let corrupt_bytes = b"not a database - fixture";
    std::fs::write(&cfg_store.db_path, corrupt_bytes).unwrap();
    let wal = std::path::PathBuf::from(format!("{}-wal", cfg_store.db_path.display()));
    std::fs::write(&wal, b"retained-wal-fixture").unwrap();
    std::fs::create_dir_all(&cfg_store.maintenance_dir).unwrap();
    std::fs::set_permissions(
        &cfg_store.maintenance_dir,
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let manifest = unleash_store::MaintenanceRestoreManifest {
        restore_id: unleash_domain::RestoreId::from_u128(10),
        command_receipt_id: unleash_domain::ReceiptId::from_u128(11),
        actor_id: "fixture:offline".into(),
        command_id: unleash_domain::CommandId::from_u128(12),
        command_digest: unleash_domain::Sha256Digest::from_bytes(b"fixture"),
        backup_id: unleash_domain::BackupId::from_u128(13),
        source_authority_generation: unleash_domain::AuthorityGeneration::from_u128(14),
        state: unleash_store::RestoreRequestState::Blocked,
        blocked_reason: Some("fixture".into()),
        updated_at_ms: unleash_domain::TimestampMs::new(42),
        history: Vec::new(),
    };
    let manifest_path = cfg_store.maintenance_dir.join("restore-manifest.json");
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    unleash_platform::write_private_file_atomic(&manifest_path, &manifest_bytes).unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Diagnostic(handle) =
        UnleashDaemon::start(config.clone()).await.unwrap()
    else {
        panic!("diagnostic attendu");
    };
    assert_eq!(std::fs::read(&cfg_store.db_path).unwrap(), corrupt_bytes);
    assert_eq!(std::fs::read(&wal).unwrap(), b"retained-wal-fixture");
    assert!(matches!(
        UnleashDaemon::start(config).await,
        Err(DaemonError::DaemonAlreadyRunning { .. } | DaemonError::AuthorityAlreadyActive { .. })
    ));

    let hello = |capabilities: Vec<&str>| {
        serde_json::json!({
            "protocol_version":"unleash.ipc.v2", "client_kind":"cli",
            "client_version":"test", "capabilities":capabilities
        })
    };
    let mut legacy = tokio::net::UnixStream::connect(handle.socket_path())
        .await
        .unwrap();
    let error = exchange(&mut legacy, hello(vec![])).await;
    assert_eq!(error["reason_code"], "STORAGE_CORRUPT");
    let mut stream = tokio::net::UnixStream::connect(handle.socket_path())
        .await
        .unwrap();
    let response = exchange(&mut stream, hello(vec!["read_only_recovery_v1"])).await;
    assert_eq!(response["mode"], "read_only_recovery");
    for key in [
        "authority_generation",
        "daemon_epoch",
        "daemon_instance_id",
        "stream_id",
    ] {
        assert!(response.get(key).is_none());
    }
    let status = exchange(&mut stream, serde_json::json!({
        "protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000001",
        "query_type":"GetDaemonStatus", "project_id":null, "payload":{}
    })).await;
    assert_eq!(status["result"]["mode"], "read_only_recovery");
    assert_eq!(status["result"]["storage_corrupt"], true);
    let invalid = exchange(&mut stream, serde_json::json!({
        "protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000007",
        "query_type":"GetDaemonStatus", "project_id":null, "payload":{"unknown":"DO_NOT_ECHO"}
    })).await;
    assert_eq!(invalid["code"], "InvalidArgument");
    assert!(!invalid.to_string().contains("DO_NOT_ECHO"));
    assert!(
        status
            .get("watermark")
            .is_none_or(serde_json::Value::is_null)
    );
    let denied_mutation = exchange(&mut stream, serde_json::json!({
        "protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000005",
        "command_type":"CreateProject", "payload":{}
    })).await;
    assert_eq!(denied_mutation["reason_code"], "STORAGE_CORRUPT");
    let denied_subscription = exchange(
        &mut stream,
        serde_json::json!({
            "stream_id":"00000000-0000-0000-0000-000000000000", "after_sequence":0, "batch_limit":1
        }),
    )
    .await;
    assert_eq!(denied_subscription["reason_code"], "STORAGE_CORRUPT");
    let legacy_config =
        ClientConfig::new(handle.socket_path().to_path_buf(), ClientKind::Cli, "test");
    assert!(
        matches!(UnleashClient::connect(legacy_config.clone()).await,
        Err(ClientError::Remote(error)) if error.reason_code == Some(ReasonCode::StorageCorrupt))
    );
    let mut diagnostic = DiagnosticClient::connect(legacy_config.without_admin_credential())
        .await
        .unwrap();
    let status = diagnostic.status(RequestId::from_u128(2)).await.unwrap();
    assert_eq!(status.result["mode"], "read_only_recovery");
    assert!(status.watermark.is_none());
    assert!(
        matches!(diagnostic.inspect_recovery(RequestId::from_u128(3)).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    let mut authorized = DiagnosticClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "admin-test",
    ))
    .await
    .unwrap();
    let inspection = authorized
        .inspect_recovery(RequestId::from_u128(8))
        .await
        .unwrap();
    assert!(inspection.watermark.is_none());
    assert_eq!(inspection.result["inventory_coverage"], "unknown");
    assert_eq!(inspection.result["report"]["storage_corrupt"], true);
    assert!(inspection.result["report"]["active_authority_generation"].is_null());
    assert!(inspection.result["report"]["active_stream_id"].is_null());
    assert_eq!(
        inspection.result["report"]["maintenance_manifest"]["restore_id"],
        manifest.restore_id.to_string()
    );
    let mut wrong = tokio::net::UnixStream::connect(handle.socket_path())
        .await
        .unwrap();
    let mut wrong_hello = hello(vec!["read_only_recovery_v1"]);
    wrong_hello["admin_proof"] = serde_json::json!("0".repeat(64));
    let _ = exchange(&mut wrong, wrong_hello).await;
    let refusal = exchange(&mut wrong, serde_json::json!({
        "protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000004",
        "query_type":"InspectRecovery", "project_id":null, "payload":{}
    })).await;
    assert_eq!(refusal["code"], "CapabilityDenied");
    let mut worker = tokio::net::UnixStream::connect(handle.socket_path())
        .await
        .unwrap();
    let proof = unleash_platform::read_admin_credential(rdv.path())
        .unwrap()
        .unwrap();
    let mut worker_hello = hello(vec!["read_only_recovery_v1"]);
    worker_hello["client_kind"] = serde_json::json!("worker");
    worker_hello["admin_proof"] = serde_json::json!(proof.as_str());
    let _ = exchange(&mut worker, worker_hello).await;
    let refusal = exchange(&mut worker, serde_json::json!({
        "protocol_version":"unleash.ipc.v2", "request_id":"00000000-0000-0000-0000-000000000006",
        "query_type":"InspectRecovery", "project_id":null, "payload":{}
    })).await;
    assert_eq!(refusal["code"], "CapabilityDenied");
    let cli = unleash_cli::Cli {
        socket: Some(handle.socket_path().to_path_buf()),
        json: true,
        timeout_ms: 5000,
        command: unleash_cli::Commands::Status,
    };
    let output = unleash_cli::run_cli(&cli).await.unwrap();
    assert_eq!(output["mode"], "read_only_recovery");
    assert_eq!(output["storage_corrupt"], true);
    let cli = unleash_cli::Cli {
        command: unleash_cli::Commands::Recovery,
        ..cli
    };
    let output = unleash_cli::run_cli(&cli).await.unwrap();
    assert!(output["watermark"].is_null());
    assert_eq!(output["result"]["inventory_coverage"], "unknown");
    std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(authorized.inspect_recovery(RequestId::from_u128(9)).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::InternalFailure && !error.message.contains("fixture"))
    );
    std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let socket_path = handle.socket_path().to_path_buf();
    tokio::time::timeout(Duration::from_secs(5), handle.shutdown_graceful())
        .await
        .unwrap();
    assert!(!socket_path.exists());
    assert_eq!(std::fs::read(&cfg_store.db_path).unwrap(), corrupt_bytes);
    assert_eq!(std::fs::read(&wal).unwrap(), b"retained-wal-fixture");
    assert_eq!(std::fs::read(manifest_path).unwrap(), manifest_bytes);
}

#[tokio::test]
async fn diagnostic_recovery_refuses_project_scoped_session_even_with_proof() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use unleash_daemon::{AuthVerifierPort, AuthenticatedSession, PeerUidAuthVerifier};
    use unleash_platform::PeerCredentials;
    use unleash_protocol::{HelloRequest, IpcError};
    struct Scoped;
    impl AuthVerifierPort for Scoped {
        fn authenticate_peer(
            &self,
            peer: PeerCredentials,
            hello: &HelloRequest,
        ) -> Result<AuthenticatedSession, IpcError> {
            let mut session = PeerUidAuthVerifier.authenticate_peer(peer, hello)?;
            session.allowed_projects = Some(std::collections::HashSet::new());
            Ok(session)
        }
    }
    let data = local_test_dir("m1-corrupt-scoped-data").unwrap();
    let rdv = local_test_dir("m1-corrupt-scoped-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = StoreConfig::new(data.path());
    std::fs::create_dir_all(store.db_path.parent().unwrap()).unwrap();
    std::fs::write(store.db_path, b"not a database").unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        auth_verifier: Arc::new(Scoped),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Diagnostic(handle) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("diagnostic attendu")
    };
    let mut client = DiagnosticClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "scoped-test",
    ))
    .await
    .unwrap();
    assert!(client.status(RequestId::from_u128(20)).await.is_ok());
    assert!(
        matches!(client.inspect_recovery(RequestId::from_u128(21)).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    handle.shutdown_graceful().await;
}

#[tokio::test]
async fn unqualified_sqlite_build_is_not_reported_as_corruption() {
    use std::os::unix::fs::PermissionsExt;
    let data = local_test_dir("m1-unqualified-data").unwrap();
    let rdv = local_test_dir("m1-unqualified-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = data.path().to_path_buf();
    tokio::task::spawn_blocking(move || {
        drop(unleash_store::UnleashStore::open(StoreConfig::new(root)).unwrap());
    })
    .await
    .unwrap();
    let build_policy = unleash_store::SqliteBuildPolicy {
        expected_source_id: "unqualified-fixture".into(),
        enforce_source_id_match: true,
        extra_approved_backports: Vec::new(),
    };
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: unleash_daemon::StoreOverrides {
            build_policy: Some(build_policy),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    assert!(matches!(
        UnleashDaemon::start(config).await,
        Err(DaemonError::Store(
            unleash_store::StoreError::BuildUnqualified { .. }
        ))
    ));
    assert!(!rdv.path().join(unleash_platform::SOCKET_FILENAME).exists());
    assert!(
        !rdv.path()
            .join(unleash_platform::ADMIN_PROOF_FILENAME)
            .exists()
    );
}

#[tokio::test]
async fn corruption_after_readonly_preflight_does_not_start_diagnostic() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    struct LateCorruption;
    impl unleash_store::CutpointObserver for LateCorruption {
        fn hit(
            &self,
            point: unleash_store::StoreCutpoint,
        ) -> Result<(), unleash_store::StoreError> {
            if point == unleash_store::StoreCutpoint::C11MigrationInProgress {
                return Err(unleash_store::StoreError::StorageCorrupt(
                    "late fixture".into(),
                ));
            }
            Ok(())
        }
    }
    let data = local_test_dir("m1-late-corrupt-data").unwrap();
    let rdv = local_test_dir("m1-late-corrupt-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(Arc::new(LateCorruption)),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    assert!(matches!(
        UnleashDaemon::start(config).await,
        Err(DaemonError::Store(
            unleash_store::StoreError::StorageCorrupt(_)
        ))
    ));
    assert!(!rdv.path().join(unleash_platform::SOCKET_FILENAME).exists());
    assert!(
        !rdv.path()
            .join(unleash_platform::ADMIN_PROOF_FILENAME)
            .exists()
    );
}
