//! Décision humaine, couverture et reçu durables sans dépôt Git (T-F009-005 et T-F009-008).

use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, ProjectId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "La fixture exige le démarrage du daemon et de son client socket"
)]
async fn start(config: DaemonConfig) -> (DaemonHandle, UnleashClient) {
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("daemon attendu démarré");
    };
    let client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "test",
    ))
    .await
    .unwrap();
    (daemon, client)
}

#[allow(
    clippy::unwrap_used,
    reason = "Les charges utiles de la fixture sont des objets JSON construits localement"
)]
fn command(
    client: &UnleashClient,
    project: ProjectId,
    id: u128,
    aggregate_id: Uuid,
    command_type: &str,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: Some(project),
        aggregate_id,
        expected_revision: Revision::INITIAL,
        command_type: command_type.to_owned(),
        payload: serde_json::from_value(payload).unwrap(),
    }
}

#[allow(
    clippy::unwrap_used,
    reason = "Les requêtes de la fixture sont des objets JSON construits localement"
)]
fn query(project: ProjectId, id: u128, query_type: &str, payload: Value) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        query_type: query_type.to_owned(),
        project_id: Some(project),
        payload: serde_json::from_value(payload).unwrap(),
    }
}

#[tokio::test]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "Le parcours socket compare ensemble recherche, décision et reprise durable"
)]
async fn t_f009_005_008_reloads_human_discovery_receipt_without_git() {
    let data = local_test_dir("f009-discovery-data").unwrap();
    let rendezvous = local_test_dir("f009-discovery-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let (daemon, mut client) = start(config.clone()).await;
    let epoch = client.hello().daemon_epoch;
    let project = ProjectId::from_u128(0xf009_0010);
    let other_project = ProjectId::from_u128(0xf009_0011);
    for (project_id, command_id) in [(project, 0xf009_0020), (other_project, 0xf009_0021)] {
        let create = command(
            &client,
            project_id,
            command_id,
            project_id.as_uuid(),
            "CreateProject",
            json!({"name": format!("F009-{command_id}"), "weight": 1}),
        );
        let receipt = client.send_command(&create).await.unwrap();
        assert_eq!(receipt.state, ReceiptState::Completed);
        assert_eq!(receipt.new_revision, Revision::new(1));
    }

    let intent = command(
        &client,
        project,
        0xf009_0030,
        Uuid::from_u128(0xf009_0031),
        "DeclareWorkIntent",
        json!({
            "category": "technical_resource",
            "objective": "Créer une procédure locale documentée",
            "mission_id": null,
            "constraints": [],
            "deadline_at": null
        }),
    );
    let intent_receipt = client.send_command(&intent).await.unwrap();
    assert_eq!(intent_receipt.state, ReceiptState::Completed);
    assert_eq!(intent_receipt.aggregate_id, intent.aggregate_id);
    assert_eq!(intent_receipt.new_revision, Revision::new(1));

    let other_intent = command(
        &client,
        other_project,
        0xf009_0032,
        Uuid::from_u128(0xf009_0033),
        "DeclareWorkIntent",
        json!({
            "category": "technical_resource",
            "objective": "Documenter une procédure dans le second projet",
            "mission_id": null,
            "constraints": [],
            "deadline_at": null
        }),
    );
    let other_intent_receipt = client.send_command(&other_intent).await.unwrap();
    assert_eq!(other_intent_receipt.state, ReceiptState::Completed);

    let scope = json!({
        "project_id": project,
        "repo_ids": [],
        "source_binding_ids": [],
        "registry_scopes": ["project"]
    });
    let discover = command(
        &client,
        project,
        0xf009_0040,
        Uuid::from_u128(0xf009_0041),
        "DiscoverResources",
        json!({
            "intent_id": intent_receipt.aggregate_id,
            "intent_revision": intent_receipt.new_revision,
            "resource_kinds": ["technical_resource"],
            "query": "procédure locale documentée",
            "scope": scope,
            "consumer_context_ref": null,
            "include_unmerged": false,
            "required_freshness": "current",
            "limit": 20
        }),
    );
    let discovery_command_receipt = client.send_command(&discover).await.unwrap();
    assert_eq!(discovery_command_receipt.state, ReceiptState::Completed);
    assert_eq!(discovery_command_receipt.aggregate_id, discover.aggregate_id);
    let discovery_ref = discovery_command_receipt.result_ref.as_deref().unwrap();
    let query_id =
        Uuid::parse_str(discovery_ref.strip_prefix("discovery-query:").unwrap()).unwrap();
    assert_eq!(query_id, discover.aggregate_id);
    let projection_query = query(
        project,
        0xf009_0042,
        "GetCommandReceipt",
        json!({"command_id": discover.command_id, "result_ref": discovery_ref}),
    );
    let projection = client.query(&projection_query).await.unwrap();
    assert!(projection.watermark.is_some());
    assert_eq!(projection.result["project_id"], json!(project));
    assert_eq!(projection.result["command_id"], json!(discover.command_id));
    assert_eq!(projection.result["result_ref"], json!(discovery_ref));
    assert_eq!(projection.result["query_id"], json!(query_id));
    assert_eq!(projection.result["intent_id"], json!(intent_receipt.aggregate_id));
    assert_eq!(projection.result["intent_revision"], json!(intent_receipt.new_revision));
    let coverage = &projection.result["coverage"];
    assert_eq!(coverage["status"], "complete");
    assert_eq!(coverage["scope"], scope);
    assert_eq!(coverage["limitations"], json!([]));
    let observations = coverage["registry_observations"].as_array().unwrap();
    assert_eq!(observations.len(), 1);
    let observed_registry = &observations[0];
    assert_eq!(observed_registry["scope_kind"], "project");
    assert_eq!(observed_registry["scope_owner_key"], json!(project));
    assert_eq!(observed_registry["entry_kind"], "technical_resource");
    assert!(observed_registry["observed_at"].is_string());
    assert_eq!(projection.result["results"], json!([]));
    assert!(projection.result["warnings"].is_array());
    assert!(projection.result["next_cursor"].is_null());
    let wrong_project = query(
        other_project,
        0xf009_0044,
        "GetCommandReceipt",
        json!({"command_id": discover.command_id, "result_ref": discovery_ref}),
    );
    let error = client.query(&wrong_project).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(ref error) if error.code == ErrorCode::NotFoundOrHidden)
    );
    let coverage_ref = projection.result["coverage_ref"].as_str().unwrap();
    assert!(!coverage_ref.is_empty());
    let catalog_revision = projection.result["catalog_revision"].as_u64().unwrap();

    let mut decision = command(
        &client,
        project,
        0xf009_0050,
        Uuid::from_u128(0xf009_0051),
        "RecordDiscoveryDecision",
        json!({
            "intent_id": intent_receipt.aggregate_id,
            "intent_revision": intent_receipt.new_revision,
            "query_ids": [query_id],
            "purpose": "pre_creation",
            "decision": "create",
            "strategy": "new",
            "candidate_ids": [],
            "justification": "Aucune ressource pertinente dans le registre du projet.",
            "target_resource_id": null,
            "target_version_ref": null,
            "supersedes_receipt_id": null,
            "creation_use_id": null,
            "expected_catalog_revision": catalog_revision
        }),
    );
    let mut cross_project_payload = decision.payload.clone();
    cross_project_payload.insert(
        "intent_id".into(),
        json!(other_intent_receipt.aggregate_id),
    );
    cross_project_payload.insert(
        "intent_revision".into(),
        json!(other_intent_receipt.new_revision),
    );
    let cross_project_decision = CommandEnvelope {
        request_id: RequestId::from_u128(0xf009_0053),
        command_id: CommandId::from_u128(0xf009_0053),
        project_id: Some(other_project),
        aggregate_id: Uuid::from_u128(0xf009_0054),
        payload: cross_project_payload,
        ..decision.clone()
    };
    let error = client.send_command(&cross_project_decision).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(ref error) if error.code == ErrorCode::NotFoundOrHidden)
    );

    let decision_receipt = client.send_command(&decision).await.unwrap();
    assert_eq!(decision_receipt.state, ReceiptState::Completed);
    let discovery_receipt_id =
        Uuid::parse_str(decision_receipt.result_ref.as_deref().unwrap()).unwrap();
    assert_ne!(discovery_receipt_id, Uuid::nil());
    assert_ne!(discovery_receipt_id, decision_receipt.receipt_id.as_uuid());
    let mut read = query(
        project,
        0xf009_0060,
        "GetDiscoveryReceipt",
        json!({"receipt_id": discovery_receipt_id, "page_size": 50}),
    );
    let before = client.query(&read).await.unwrap();
    let before_cursor = before.watermark.unwrap();
    let historical = &before.result["receipt"];
    assert_eq!(before.result["project_id"], json!(project));
    assert_eq!(historical["receipt_id"], json!(discovery_receipt_id));
    assert_eq!(historical["intent_id"], json!(intent_receipt.aggregate_id));
    assert_eq!(
        historical["validation_basis"]["intent_revision"],
        json!(intent_receipt.new_revision)
    );
    assert_eq!(historical["query_ids"], json!([query_id]));
    assert_eq!(historical["catalog_revision"], json!(catalog_revision));
    assert_eq!(historical["coverage_ref"], json!(coverage_ref));
    assert_eq!(historical["decision"], "create");
    assert_eq!(historical["strategy"], "new");
    assert!(historical["source_refs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source["source_kind"] != "git_snapshot"));
    assert_eq!(historical["origin"]["origin_kind"], "control_plane");
    assert!(
        historical["origin"]["principal_id"]
            .as_str()
            .is_some_and(|principal| !principal.is_empty())
    );
    assert_eq!(
        historical["origin"]["principal_id"],
        historical["origin"]["control_principal_id"]
    );
    assert_eq!(historical["origin"]["cause"]["kind"], "command_receipt");
    assert_eq!(historical["origin"]["cause"]["id"], json!(decision_receipt.receipt_id));
    assert!(historical["origin"].get("invocation_id").is_none());

    drop(client);
    daemon.shutdown_graceful("F009 reprise").await.unwrap();
    drop(daemon);
    let (daemon, mut client) = start(config).await;
    assert!(client.hello().daemon_epoch > epoch);
    let mut reread_projection = projection_query.clone();
    reread_projection.request_id = RequestId::from_u128(0xf009_0043);
    let persisted_projection = client.query(&reread_projection).await.unwrap();
    assert!(persisted_projection.watermark.is_some());
    assert_eq!(persisted_projection.result, projection.result);
    decision.request_id = RequestId::from_u128(0xf009_0052);
    assert_eq!(client.send_command(&decision).await.unwrap(), decision_receipt);
    read.request_id = RequestId::from_u128(0xf009_0061);
    let after = client.query(&read).await.unwrap();
    assert_eq!(after.result["receipt"], *historical);
    let after_cursor = after.watermark.unwrap();
    assert_eq!(after_cursor.stream_id, before_cursor.stream_id);
    assert!(after_cursor.sequence >= before_cursor.sequence);
    read.project_id = Some(other_project);
    read.request_id = RequestId::from_u128(0xf009_0062);
    let error = client.query(&read).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(ref error) if error.code == ErrorCode::NotFoundOrHidden)
    );
    drop(client);
    daemon.shutdown_graceful("F009 terminé").await.unwrap();
}
