//! REQ-CORE-004 : preuve administrative liée au daemon et contrôlée sur UDS.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]
use serde_json::{Map, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_platform::read_admin_credential;
use unleash_protocol::{
    AdminProof, ClientKind, CommandEnvelope, ErrorCode, HelloRequest, HelloResponse,
    ProtocolVersion, QueryEnvelope,
};
use unleash_testkit::local_test_dir;

async fn raw_hello(socket: &std::path::Path, kind: ClientKind, proof: AdminProof) -> HelloResponse {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let request = HelloRequest {
        protocol_version: ProtocolVersion,
        client_kind: kind,
        client_version: "test".into(),
        capabilities: Vec::new(),
        admin_proof: Some(proof),
    };
    let bytes = serde_json::to_vec(&request).unwrap();
    stream
        .write_all(&(u32::try_from(bytes.len()).unwrap()).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&bytes).await.unwrap();
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await.unwrap();
    let mut response = vec![0; u32::from_be_bytes(length) as usize];
    stream.read_exact(&mut response).await.unwrap();
    serde_json::from_slice(&response).unwrap()
}

async fn malformed_hello_does_not_echo(socket: &std::path::Path) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let marker = "sensitive-invalid-proof";
    let body = json!({"protocol_version":"unleash.ipc.v2", "client_kind":"cli", "client_version":"test", "capabilities":[], "admin_proof":marker}).to_string();
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(&u32::try_from(body.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut len = [0_u8; 4];
    stream.read_exact(&mut len).await.unwrap();
    let mut response = vec![0_u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut response).await.unwrap();
    let response = std::str::from_utf8(&response).unwrap();
    assert!(!response.contains(marker));
    assert!(response.contains("Handshake Hello invalide"));
}

#[tokio::test]
async fn admin_requires_proof_and_reloads_after_daemon_rotation() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-admin-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-admin-data").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg.clone()).await.unwrap()
    else {
        panic!("daemon attendu")
    };
    let proof1 = read_admin_credential(rdv.path()).unwrap().unwrap();
    let socket = handle.socket_path().to_path_buf();
    malformed_hello_does_not_echo(&socket).await;
    let wrong = raw_hello(&socket, ClientKind::Cli, AdminProof::from_bytes([0; 32])).await;
    assert!(!wrong.capabilities.iter().any(|cap| cap == "administer"));
    let probe_with_proof = raw_hello(
        &socket,
        ClientKind::Probe,
        AdminProof::from_hex(&proof1).unwrap(),
    )
    .await;
    assert!(
        !probe_with_proof
            .capabilities
            .iter()
            .any(|cap| cap == "administer")
    );
    let worker_with_proof = raw_hello(
        &socket,
        ClientKind::Worker,
        AdminProof::from_hex(&proof1).unwrap(),
    )
    .await;
    assert!(
        !worker_with_proof
            .capabilities
            .iter()
            .any(|cap| cap == "administer")
    );
    let mut unprivileged = UnleashClient::connect(
        ClientConfig::new(socket.clone(), ClientKind::Cli, "test").without_admin_credential(),
    )
    .await
    .unwrap();
    let status = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(1),
        query_type: "GetDaemonStatus".into(),
        project_id: None,
        payload: Map::new(),
    };
    assert!(unprivileged.query(&status).await.is_ok());
    let private = QueryEnvelope {
        query_type: "ListProjects".into(),
        ..status.clone()
    };
    assert!(
        matches!(unprivileged.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    let credential_path = rdv.path().join(unleash_platform::ADMIN_PROOF_FILENAME);
    std::fs::remove_file(&credential_path).unwrap();
    let mut missing =
        UnleashClient::connect(ClientConfig::new(socket.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    assert!(missing.query(&status).await.is_ok());
    assert!(
        matches!(missing.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    unleash_platform::write_private_file_atomic(&credential_path, b"invalid").unwrap();
    let mut malformed =
        UnleashClient::connect(ClientConfig::new(socket.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    assert!(malformed.query(&status).await.is_ok());
    assert!(
        matches!(malformed.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied && !e.message.contains("invalid"))
    );
    unleash_platform::write_private_file_atomic(&credential_path, proof1.as_bytes()).unwrap();
    std::fs::set_permissions(&credential_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut insecure_file =
        UnleashClient::connect(ClientConfig::new(socket.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    assert!(
        matches!(insecure_file.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    std::fs::set_permissions(&credential_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut unsafe_config = ClientConfig::new(socket.clone(), ClientKind::Cli, "test");
    unsafe_config.verify_socket_security = false;
    let mut unsafe_connection = UnleashClient::connect(unsafe_config).await.unwrap();
    assert!(
        matches!(unsafe_connection.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    let pid = ProjectId::from_u128(0xa1);
    let cmd = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(2),
        command_id: CommandId::from_u128(3),
        authority_generation: unprivileged.hello().authority_generation,
        actor_ref: None,
        project_id: Some(pid),
        aggregate_id: pid.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".into(),
        payload: json!({"name":"authed", "weight":1})
            .as_object()
            .unwrap()
            .clone(),
    };
    let mut denied_cmd = cmd.clone();
    denied_cmd
        .payload
        .insert("untrusted_secret".into(), json!("DO_NOT_RECORD_ME"));
    for _ in 0..2 {
        assert!(
            matches!(unprivileged.send_command(&denied_cmd).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
        );
    }
    // Une panne du writer ne transforme pas le refus en succès ni en faux audit.
    let blocker = rusqlite::Connection::open(data.path().join("unleash-authority.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut failed_audit = denied_cmd.clone();
    failed_audit.command_id = CommandId::from_u128(99);
    assert!(
        matches!(unprivileged.send_command(&failed_audit).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied && e.message.contains("audit"))
    );
    blocker.execute_batch("ROLLBACK").unwrap();
    let mut admin =
        UnleashClient::connect(ClientConfig::new(socket.clone(), ClientKind::Cli, "test"))
            .await
            .unwrap();
    let before = admin.query(&private).await.unwrap();
    assert!(before.result.as_array().unwrap().is_empty());
    admin.send_command(&cmd).await.unwrap();
    assert_eq!(
        admin
            .query(&private)
            .await
            .unwrap()
            .result
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut probe = UnleashClient::connect(ClientConfig::new(socket, ClientKind::Probe, "test"))
        .await
        .unwrap();
    assert!(
        matches!(probe.query(&private).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    handle.shutdown_graceful("test").await.unwrap();
    assert!(read_admin_credential(rdv.path()).unwrap().is_none());
    let DaemonStartOutcome::Started(handle2) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon redémarré attendu")
    };
    let proof2 = read_admin_credential(rdv.path()).unwrap().unwrap();
    assert!(*proof1 != *proof2);
    let conn = rusqlite::Connection::open(data.path().join("unleash-authority.db")).unwrap();
    let (count, text): (i64, String) = conn.query_row("SELECT COUNT(*), COALESCE(GROUP_CONCAT(alert_id || subject_kind || subject_id || summary), '') FROM alerts WHERE kind = 'security_denial'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(count, 1);
    assert!(!text.contains("DO_NOT_RECORD_ME"));
    let stale = raw_hello(
        handle2.socket_path(),
        ClientKind::Cli,
        AdminProof::from_hex(&proof1).unwrap(),
    )
    .await;
    assert!(!stale.capabilities.iter().any(|cap| cap == "administer"));
    admin.reconnect().await.unwrap();
    assert_eq!(
        admin
            .query(&private)
            .await
            .unwrap()
            .result
            .as_array()
            .unwrap()
            .len(),
        1
    );
    handle2.shutdown_graceful("test").await.unwrap();
}
