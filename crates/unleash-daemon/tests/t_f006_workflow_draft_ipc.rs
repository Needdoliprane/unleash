// T-F006-010 : conservation d’un brouillon texte par le parcours IPC public.
use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_testkit::local_test_dir;

const DRAFT_TEXT: &str = "{\r\n  \"nom\": \"été\",\r\n  \"nom\": \"deux\"\r\n}\r\n";
const DRAFT_HASH: &str = "sha256:14d14d88e93a53254026a0c0adfaf3db12ef7cb681b1b6c8711b4316755b21e6";

#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "La fixture doit échouer si le daemon ou le client ne démarre pas"
)]
async fn start(config: DaemonConfig) -> (DaemonHandle, UnleashClient) {
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("daemon attendu démarré");
    };
    let client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    (daemon, client)
}

#[allow(
    clippy::unwrap_used,
    reason = "La création des projets est un prérequis de la fixture"
)]
async fn create_project(client: &mut UnleashClient, project: ProjectId, id: u128) {
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: Some(project),
        aggregate_id: project.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".to_owned(),
        payload: serde_json::from_value(json!({"name": format!("F006-{id}"), "weight": 1})).unwrap(),
    };
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
}

#[tokio::test]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "Les assertions vérifient les valeurs et les champs de la fixture IPC"
)]
async fn t_f006_010_manual_draft_preserves_utf8_and_project_across_restart() {
    let rdv = local_test_dir("f006-draft-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f006-draft-data").unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let (daemon, mut client) = start(config.clone()).await;
    let authority = client.hello().authority_generation;
    let epoch = client.hello().daemon_epoch;
    let project = ProjectId::from_u128(0x6610);
    let other_project = ProjectId::from_u128(0x6611);
    create_project(&mut client, project, 0x6620).await;
    create_project(&mut client, other_project, 0x6621).await;

    let mut create: CommandEnvelope = serde_json::from_str(include_str!(
        "../../../.claude/examples/ipc-create-workflow-text-draft.json"
    ))
    .unwrap();
    create.authority_generation = authority;
    create.project_id = Some(project);
    create.payload = serde_json::from_value(json!({
        "source": {"kind": "text", "text": DRAFT_TEXT}
    }))
    .unwrap();
    let receipt = client.send_command(&create).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.aggregate_id, create.aggregate_id);
    assert_eq!(receipt.new_revision, Revision::new(1));

    let mut query: QueryEnvelope = serde_json::from_str(include_str!(
        "../../../.claude/examples/ipc-get-workflow-draft-query.json"
    ))
    .unwrap();
    query.project_id = Some(project);
    assert_eq!(query.payload.get("draft_id"), Some(&json!(create.aggregate_id)));
    let before = client.query(&query).await.unwrap();
    let mut expected: Value = serde_json::from_str(include_str!(
        "../../../.claude/examples/ipc-get-workflow-text-draft-response.json"
    ))
    .unwrap();
    expected["result"]["project_id"] = json!(project);
    expected["result"]["draft_id"] = json!(create.aggregate_id);
    expected["result"]["revision"] = json!(receipt.new_revision);
    expected["result"]["text"] = json!(DRAFT_TEXT);
    expected["result"]["text_hash"] = json!(DRAFT_HASH);
    assert_eq!(before.result, expected["result"]);
    assert_eq!(before.result["text"].as_str().unwrap().as_bytes(), DRAFT_TEXT.as_bytes());
    let before_cursor = before.watermark.unwrap();
    assert_eq!(before_cursor.stream_id, receipt.event_cursor.stream_id);
    assert!(before_cursor.sequence >= receipt.event_cursor.sequence);

    drop(client);
    daemon.shutdown_graceful("F006 restart").await.unwrap();
    drop(daemon);
    let (daemon, mut client) = start(config).await;
    assert_eq!(client.hello().authority_generation, authority);
    assert!(client.hello().daemon_epoch > epoch);
    query.request_id = RequestId::from_u128(0x6622);
    let after = client.query(&query).await.unwrap();
    assert_eq!(after.result, before.result);
    let after_cursor = after.watermark.unwrap();
    assert_eq!(after_cursor.stream_id, before_cursor.stream_id);
    assert!(after_cursor.sequence >= before_cursor.sequence);

    // L’acteur possède accès aux deux projets ; seul le projet du brouillon change.
    query.project_id = Some(other_project);
    query.request_id = RequestId::from_u128(0x6623);
    let error = client.query(&query).await.unwrap_err();
    assert!(matches!(
        error,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::NotFoundOrHidden
    ));
    query.project_id = Some(project);
    query.request_id = RequestId::from_u128(0x6624);
    assert_eq!(client.query(&query).await.unwrap().result, before.result);
    drop(client);
    daemon.shutdown_graceful("F006 done").await.unwrap();
}
