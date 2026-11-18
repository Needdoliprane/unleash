//! REQ-CORE-003/004/010 et REQ-REC-009 : projections IPC et watermark cohérent.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]
use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{
    AuthVerifierPort, AuthenticatedSession, DaemonConfig, DaemonStartOutcome, PeerUidAuthVerifier,
    UnleashDaemon,
};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope};
use unleash_testkit::local_test_dir;

struct ScopedTestVerifier;
impl AuthVerifierPort for ScopedTestVerifier {
    fn authenticate_peer(
        &self,
        peer: unleash_platform::PeerCredentials,
        hello: &unleash_protocol::HelloRequest,
    ) -> Result<AuthenticatedSession, unleash_protocol::IpcError> {
        let mut session = PeerUidAuthVerifier.authenticate_peer(peer, hello)?;
        if hello.client_kind == ClientKind::Tui {
            session.allowed_projects = Some([ProjectId::from_u128(0x91)].into_iter().collect());
        }
        Ok(session)
    }
}

fn query(name: &str, project_id: Option<ProjectId>, payload: Value) -> QueryEnvelope {
    let Value::Object(payload) = payload else {
        panic!("objet attendu")
    };
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(10),
        query_type: name.into(),
        project_id,
        payload,
    }
}

#[tokio::test]
async fn projections_share_watermark_and_reject_unknown_payload_and_unauthorized_peer() {
    use std::os::unix::fs::PermissionsExt;
    let rdv = local_test_dir("m1-proj-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-proj-data").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        auth_verifier: std::sync::Arc::new(ScopedTestVerifier),
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
    let pid = ProjectId::from_u128(0x91);
    let receipt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: CommandId::from_u128(2),
            authority_generation: client.hello().authority_generation,
            actor_ref: None,
            project_id: Some(pid),
            aggregate_id: pid.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateProject".into(),
            payload: json!({"name":"IPC", "weight":1})
                .as_object()
                .unwrap()
                .clone(),
        })
        .await
        .unwrap();
    let list = client
        .query(&query("ListProjects", None, json!({})))
        .await
        .unwrap();
    assert_eq!(list.watermark, Some(receipt.event_cursor));
    assert_eq!(list.result.as_array().unwrap().len(), 1);
    let one = client
        .query(&query("GetProject", Some(pid), json!({})))
        .await
        .unwrap();
    assert_eq!(one.watermark, Some(receipt.event_cursor));
    assert_eq!(one.result["name"], "IPC");
    for q in [
        query(
            "ReadSettings",
            None,
            json!({"scope_kind":"user", "scope_id":"default"}),
        ),
        query("ReadPolicySummary", None, json!({})),
        query("InspectRecovery", None, json!({})),
    ] {
        let result = client.query(&q).await.unwrap();
        assert_eq!(result.watermark, Some(receipt.event_cursor));
        assert!(result.result.is_object());
    }
    let absent_guard = client
        .query(&query("ReadGuardHealth", None, json!({})))
        .await
        .unwrap();
    assert!(absent_guard.result.is_null());
    assert_eq!(absent_guard.watermark, Some(receipt.event_cursor));
    for name in [
        "ListProjects",
        "GetProject",
        "ReadPolicySummary",
        "InspectRecovery",
        "ReadGuardHealth",
    ] {
        assert!(
            matches!(client.query(&query(name, Some(pid), json!({"surprise":"secret"}))).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument && !e.message.contains("secret"))
        );
    }
    assert!(
        matches!(client.query(&query("GetProject", None, json!({}))).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::InvalidArgument)
    );
    let mut probe = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Probe,
        "test",
    ))
    .await
    .unwrap();
    assert!(
        matches!(probe.query(&query("ListProjects", None, json!({}))).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::CapabilityDenied)
    );
    let socket = handle.socket_path().to_path_buf();
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_unleashd")).with_file_name("unleash");
    let cli_list = tokio::task::spawn_blocking(move || {
        std::process::Command::new(binary)
            .arg("--socket")
            .arg(socket)
            .args(["--json", "project", "list"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        cli_list.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&cli_list.stderr)
    );
    let list_json: Value = serde_json::from_slice(&cli_list.stdout).unwrap();
    assert_eq!(list_json["result"][0]["name"], "IPC");
    assert_eq!(
        list_json["watermark"]["sequence"],
        receipt.event_cursor.sequence.as_u64()
    );
    let other_pid = ProjectId::from_u128(0x92);
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(11),
            command_id: CommandId::from_u128(12),
            authority_generation: client.hello().authority_generation,
            actor_ref: None,
            project_id: Some(other_pid),
            aggregate_id: other_pid.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateProject".into(),
            payload: json!({"name":"autre", "weight":1})
                .as_object()
                .unwrap()
                .clone(),
        })
        .await
        .unwrap();
    let mut scoped = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Tui,
        "test",
    ))
    .await
    .unwrap();
    let restricted = scoped
        .query(&query("ListProjects", None, json!({})))
        .await
        .unwrap();
    assert_eq!(restricted.result.as_array().unwrap().len(), 1);
    assert_eq!(restricted.result[0]["project_id"], pid.to_string());
    assert!(
        matches!(scoped.query(&query("GetProject", Some(other_pid), json!({}))).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::ScopeDenied)
    );
    // Déclarer un projet permis ne prouve pas la provenance du reçu créé dans un autre projet.
    for claimed_project in [Some(pid), None] {
        let receipt_query = query(
            "GetCommandReceipt",
            claimed_project,
            json!({"command_id":CommandId::from_u128(12)}),
        );
        assert!(
            matches!(scoped.query(&receipt_query).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::ScopeDenied)
        );
    }
    for q in [
        query("ReadPolicySummary", Some(pid), json!({})),
        query("InspectRecovery", None, json!({})),
        query("ReadGuardHealth", None, json!({})),
        query(
            "ReadSettings",
            None,
            json!({"scope_kind":"user", "scope_id":"default"}),
        ),
    ] {
        assert!(
            matches!(scoped.query(&q).await, Err(ClientError::Remote(e)) if e.code == ErrorCode::ScopeDenied)
        );
    }
    let _ = Map::<String, Value>::new();
    handle.shutdown_graceful("test").await.unwrap();
}
