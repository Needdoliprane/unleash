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
    let cli = unleash_cli::Cli {
        socket: Some(handle.socket_path().to_path_buf()),
        json: true,
        timeout_ms: 5000,
        command: unleash_cli::Commands::Status,
    };
    let output = unleash_cli::run_cli(&cli).await.unwrap();
    assert_eq!(output["mode"], "read_only_recovery");
    assert_eq!(output["storage_corrupt"], true);
    let socket_path = handle.socket_path().to_path_buf();
    tokio::time::timeout(Duration::from_secs(5), handle.shutdown_graceful())
        .await
        .unwrap();
    assert!(!socket_path.exists());
    assert_eq!(std::fs::read(&cfg_store.db_path).unwrap(), corrupt_bytes);
    assert_eq!(std::fs::read(&wal).unwrap(), b"retained-wal-fixture");
}
