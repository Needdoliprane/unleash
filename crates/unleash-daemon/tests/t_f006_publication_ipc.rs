//! T-F006-009: publication respects the existing actor-scoped idempotency key.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use serde_json::{Map, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_testkit::local_test_dir;

#[tokio::test]
async fn t_f006_009_publish_rejects_an_existing_command_id_with_different_intent() {
    let rendezvous = local_test_dir("f006-publish-idempotency-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), PermissionsExt::from_mode(0o700)).unwrap();
    let data = local_test_dir("f006-publish-idempotency-data").unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("isolated daemon must start");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let authority = client.hello().authority_generation;
    let project = ProjectId::from_u128(0x6601);
    let command_id = CommandId::from_u128(0x6602);
    let create = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x6603),
        command_id,
        authority_generation: authority,
        actor_ref: None,
        project_id: Some(project),
        aggregate_id: project.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "CreateProject".to_owned(),
        payload: serde_json::from_value(json!({"name": "F006 publication", "weight": 1})).unwrap(),
    };
    let original = client.send_command(&create).await.unwrap();
    assert_eq!(original.state, ReceiptState::Accepted);
    // `Accepted` engage le projet sans le terminer : la commande suivante attend qu'il soit lisible.
    let readable = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x6606),
        query_type: "GetProject".to_owned(),
        project_id: Some(project),
        payload: Map::new(),
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(5), async {
            while client.query(&readable).await.is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok(),
        "le projet devient lisible"
    );

    let mut publish: CommandEnvelope = serde_json::from_str(include_str!(
        "../../../.claude/examples/ipc-publish-workflow-version.json"
    ))
    .unwrap();
    publish.request_id = RequestId::from_u128(0x6604);
    publish.command_id = command_id;
    publish.authority_generation = authority;
    publish.project_id = Some(project);
    let conflict = client.send_command(&publish).await.unwrap_err();
    assert!(matches!(
        conflict,
        ClientError::Remote(ref error)
            if error.code == ErrorCode::IdempotencyConflict
                && error.command_id == Some(command_id)
    ));

    let retained = client
        .get_command_receipt(RequestId::from_u128(0x6605), command_id)
        .await
        .unwrap();
    assert_eq!(retained.receipt, original);
    drop(client);
    daemon
        .shutdown_graceful("publication conflict checked")
        .await
        .unwrap();
}
