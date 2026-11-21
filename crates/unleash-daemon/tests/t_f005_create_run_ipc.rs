//! REQ-WF-001: prepare and find the same workflow run through the local socket.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Assertions and setup for the isolated daemon fixture"
)]

#[path = "../../unleash-store/tests/support/workflow_run.rs"]
mod workflow_run;

#[path = "support/workflow_start.rs"]
mod start_run;

#[path = "support/workflow_agent.rs"]
mod create_agent;

use serde_json::{Map, Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{
    ActorId, AgentLevel, CommandId, MissionId, ReceiptId, RequestId, Revision, Sha256Digest,
};
use unleash_platform::{current_boot_identity, effective_uid};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_store::{StoreConfig, UnleashStore};
use unleash_testkit::local_test_dir;
use workflow_run::{PUBLISHED, RUN};

const MISSION: MissionId = MissionId::from_u128(0x5910);

fn local_store_config(path: &Path) -> StoreConfig {
    let mut config = StoreConfig::new(path);
    config.owner_uid = effective_uid();
    current_boot_identity()
        .unwrap()
        .as_str()
        .clone_into(&mut config.boot_identity);
    config
}

fn wire_command(config: StoreConfig) -> (CommandEnvelope, Sha256Digest) {
    wire_command_with_workflow(config, AgentLevel::N, |_| {})
}

fn wire_command_with_workflow(
    config: StoreConfig,
    role_level: AgentLevel,
    customize: impl FnOnce(&mut Value),
) -> (CommandEnvelope, Sha256Digest) {
    let actor = ActorId::new(format!("uid:{}", effective_uid())).unwrap();
    let (fixture, mut envelope, _) =
        workflow_run::prepare(config, actor, role_level, |workflow, _, _| {
            customize(workflow);
        });
    let hash: String = fixture
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT content_hash FROM workflow_versions WHERE workflow_version_id=?1",
                [PUBLISHED],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let hash: Sha256Digest = hash.parse().unwrap();
    envelope.payload = serde_json::from_value(json!({
        "mission_id": MISSION,
        "source": { "kind": "workflow_version", "id": "workflow.merge-only",
                    "version": 1, "digest": hash },
        "purpose": "operational",
        "predecessor_run_id": null,
        "selection_reason": "fixture workflow selected",
    }))
    .unwrap();
    fixture.store.shutdown().unwrap();
    (envelope, hash)
}

async fn start(config: DaemonConfig) -> (DaemonHandle, UnleashClient) {
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("isolated daemon must start");
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

fn assign_scoped_mission(run: &CommandEnvelope) -> CommandEnvelope {
    CommandEnvelope {
        request_id: RequestId::from_u128(0x5911),
        command_id: CommandId::from_u128(0x5911),
        aggregate_id: MISSION.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "AssignMission".to_owned(),
        payload: serde_json::from_value(json!({
            "completion_policy": "mechanical",
            "scopes": {"implementation": {"kind":"project", "revision":1}},
        }))
        .unwrap(),
        ..run.clone()
    }
}

async fn publish_mandate(
    client: &mut UnleashClient,
    run: &mut CommandEnvelope,
) -> (CommandEnvelope, unleash_protocol::CommandReceipt) {
    let mission = assign_scoped_mission(run);
    let receipt = client.send_command(&mission).await.unwrap();
    assert_eq!(receipt.new_revision, Revision::new(1));
    let hash: Sha256Digest = receipt
        .result_ref
        .as_deref()
        .unwrap()
        .strip_prefix("mission-scope:")
        .unwrap()
        .parse()
        .unwrap();
    run.payload.insert("scope_ref".to_owned(), json!(hash));

    let mut stale = mission.clone();
    stale.request_id = RequestId::from_u128(0x5912);
    stale.command_id = CommandId::from_u128(0x5912);
    stale.aggregate_id = MissionId::from_u128(0x5912).as_uuid();
    stale.payload.insert(
        "scopes".to_owned(),
        json!({"implementation":{"kind":"project","revision":2}}),
    );
    let error = client.send_command(&stale).await.unwrap_err();
    assert!(matches!(error, unleash_client::ClientError::Remote(error)
        if error.code == unleash_protocol::ErrorCode::RevisionConflict));
    (mission, receipt)
}

fn scoped_run_query(command: &CommandEnvelope) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x5903),
        query_type: "GetRun".to_owned(),
        project_id: command.project_id,
        payload: Map::from_iter([("run_id".to_owned(), json!(RUN))]),
    }
}

async fn assert_refuses_different_workflow_version(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
) {
    let mut different_version = command.clone();
    different_version.command_id = CommandId::from_u128(0x5902);
    different_version.request_id = RequestId::from_u128(0x5905);
    different_version
        .payload
        .get_mut("source")
        .and_then(Value::as_object_mut)
        .unwrap()
        .insert("version".to_owned(), json!(2));
    assert!(client.send_command(&different_version).await.is_err());
}

// T-F004-003 couvre ici la portée initiale et le refus d’une révision périmée.
// T-F005-001 et T-F005-009 couvrent ici le plan épinglé et le rejeu.
#[tokio::test]
async fn t_f005_creates_one_prepared_run_and_replays_its_wire_receipt() {
    let data = local_test_dir("f005-run-data").unwrap();
    let rendezvous = local_test_dir("f005-run-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), PermissionsExt::from_mode(0o700)).unwrap();
    let store_config = local_store_config(data.path());
    let prepare = store_config.clone();
    let (mut command, definition) = tokio::task::spawn_blocking(move || wire_command(prepare))
        .await
        .unwrap();

    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let (daemon, mut client) = start(config.clone()).await;
    let epoch = client.hello().daemon_epoch;
    let (mut assignment, assigned) = publish_mandate(&mut client, &mut command).await;
    assert_refuses_different_workflow_version(&mut client, &command).await;
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.new_revision, Revision::new(1));
    assert_eq!(receipt.result_ref, Some(format!("workflow-run:{RUN}")));

    let query = scoped_run_query(&command);
    let before = client.query(&query).await.unwrap();
    assert!(before.watermark.is_some());
    assert_eq!(before.result.get("mission_id"), Some(&json!(MISSION)));
    assert_eq!(
        before
            .result
            .pointer("/plan/scopes/implementation/scope_id"),
        Some(&json!(command.project_id.unwrap()))
    );
    assert_eq!(
        before
            .result
            .pointer("/plan/scopes/implementation/scope_revision"),
        Some(&json!(1))
    );

    assert_eq!(before.result.get("state"), Some(&json!("prepared")));
    assert_eq!(
        before.result.get("definition_hash"),
        Some(&json!(definition))
    );
    assert_eq!(
        before
            .result
            .pointer("/plan/bindings/developer/role/registry_version_id"),
        Some(&json!("role-v1"))
    );
    assert_eq!(
        before.result.pointer("/plan/bindings/future/state"),
        Some(&json!("unbound"))
    );
    drop(client);
    daemon.shutdown_graceful("run prepared").await.unwrap();
    drop(daemon);

    let (daemon, mut client) = start(config).await;
    assert!(client.hello().daemon_epoch.as_u64() > epoch.as_u64());
    assignment.request_id = RequestId::from_u128(0x5913);
    assert_eq!(client.send_command(&assignment).await.unwrap(), assigned);
    command.request_id = RequestId::from_u128(0x5904);
    assert_eq!(client.send_command(&command).await.unwrap(), receipt);
    let after = client.query(&query).await.unwrap();
    assert_eq!(before.result, after.result);
    assert!(after.watermark.is_some());
    let mut changed_intent = command.clone();
    changed_intent
        .payload
        .insert("selection_reason".to_owned(), json!("different selection"));
    drop(client);
    let mut new_connection = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let conflict = new_connection
        .send_command(&changed_intent)
        .await
        .unwrap_err();
    assert!(
        matches!(conflict, unleash_client::ClientError::Remote(error)
        if error.code == unleash_protocol::ErrorCode::IdempotencyConflict)
    );
    drop(new_connection);
    daemon
        .shutdown_graceful("run replay verified")
        .await
        .unwrap();
    drop(daemon);

    tokio::task::spawn_blocking(move || {
        assert_durable_run(store_config, receipt.receipt_id, assigned.receipt_id);
    })
    .await
    .unwrap();
}

fn assert_durable_run(
    store_config: StoreConfig,
    run_receipt: ReceiptId,
    mission_receipt: ReceiptId,
) {
    let store = UnleashStore::open(store_config).unwrap();
    let effects: (i64, i64, i64, i64) = store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT (SELECT count(*) FROM workflow_runs WHERE run_id=?1 AND state='prepared' AND revision=1),
              (SELECT count(*) FROM command_receipts WHERE command_type='CreateRun' AND aggregate_id=?1),
              (SELECT count(*) FROM events WHERE event_type='RunCreated' AND causation_receipt_id=?2),
              (SELECT count(*) FROM invocations WHERE run_id=?1)",
            rusqlite::params![RUN.to_string(), run_receipt.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    }).unwrap();
    assert_eq!(effects, (1, 1, 1, 0));
    let mandate: (i64, i64, i64) = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT (SELECT count(*) FROM missions WHERE mission_id=?1 AND revision=1),
              (SELECT count(*) FROM object_references
                WHERE owner_kind='mission' AND owner_id=?1 AND source_field='scope_manifest:1'
                  AND state='live'),
              (SELECT count(*) FROM events WHERE event_type='MissionAssigned'
                AND aggregate_id=?1 AND causation_receipt_id=?2)",
                rusqlite::params![MISSION.to_string(), mission_receipt.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .unwrap();
    assert_eq!(mandate, (1, 1, 1));

    store.shutdown().unwrap();
}
