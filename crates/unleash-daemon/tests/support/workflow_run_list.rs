//! T-F005-001: discover and open the pinned run through the local socket.

use std::os::unix::fs::PermissionsExt;

use serde_json::{Map, Value, json};
use unleash_client::{ClientError, UnleashClient};
use unleash_daemon::DaemonConfig;
use unleash_domain::{AgentLevel, MissionId, ProjectId, RequestId, RunId, Sha256Digest};
use unleash_platform::effective_uid;
use unleash_protocol::{
    ErrorCode, ProtocolVersion, QueryEnvelope, QueryResponse, ReceiptState, canonicalize_to_vec,
};
use unleash_store::{
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;

use super::{MISSION, PUBLISHED, local_store_config, publish_mandate, start, wire_run_fixture};

const PINNED_RUN: RunId = RunId::from_u128(0x5000);
const OTHER_RUN: RunId = RunId::from_u128(0x5001);
const OTHER_PROJECT: ProjectId = ProjectId::from_u128(0x5a01);
const EMPTY_PROJECT: ProjectId = ProjectId::from_u128(0x5a02);
const OTHER_MISSION: MissionId = MissionId::from_u128(0x5a03);
const PUBLISHED_V2: &str = "fixture.prepared-workflow:v2";

fn publish_v2_and_seed_other_projects(config: StoreConfig, v1: Sha256Digest) {
    let store = UnleashStore::open(config).unwrap();
    let mut v2: Value =
        serde_json::from_slice(&store.read_verified_object_bytes(v1).unwrap()).unwrap();
    v2.as_object_mut()
        .unwrap()
        .insert("version".to_owned(), json!(2));
    let bytes = canonicalize_to_vec(&v2).unwrap();
    let digest = Sha256Digest::from_bytes(&bytes);
    assert_ne!(digest, v1);
    let object = store
        .prepare_object(PrepareObjectRequest {
            object_hash: digest,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".to_owned(),
            owner_kind: "workflow_version".to_owned(),
            owner_id: PUBLISHED_V2.to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&object, &bytes).unwrap();
    store
        .publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: object.object_hash,
            object_generation: object.object_generation,
            owner_kind: "workflow_version".to_owned(),
            owner_id: PUBLISHED_V2.to_owned(),
            source_field: "content_hash".to_owned(),
        })
        .unwrap();
    store
        .register_workflow_version(PUBLISHED_V2, "published", digest)
        .unwrap();

    let owner = format!("uid:{}", effective_uid());
    let bundle = Sha256Digest::from_bytes(b"retained run in the other project");
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            for project in [OTHER_PROJECT, EMPTY_PROJECT] {
                tx.execute(
                    "INSERT INTO projects (project_id,name,created_at_ms)
                     VALUES (?1,'workflow list fixture',?2)",
                    rusqlite::params![project.to_string(), now.as_i64()],
                )?;
            }
            tx.execute(
                "INSERT INTO missions (mission_id,project_id,owner_kind,owner_user_id,
                    completion_policy,state,revision)
                 VALUES (?1,?2,'user',?3,'mechanical','active',1)",
                rusqlite::params![OTHER_MISSION.to_string(), OTHER_PROJECT.to_string(), owner],
            )?;
            tx.execute(
                "INSERT INTO workflow_runs (run_id,mission_id,workflow_version_id,
                    selection_reason,effective_bundle_hash,state,revision,started_at_ms)
                 VALUES (?1,?2,?3,'other project fixture',?4,'prepared',1,?5)",
                rusqlite::params![
                    OTHER_RUN.to_string(),
                    OTHER_MISSION.to_string(),
                    PUBLISHED_V2,
                    bundle.to_string(),
                    now.as_i64()
                ],
            )?;
            Ok(())
        })
        .unwrap();
    store.shutdown().unwrap();
}

async fn list(
    client: &mut UnleashClient,
    project: ProjectId,
    cursor: Option<&str>,
    request_id: u128,
) -> Result<QueryResponse, ClientError> {
    client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(request_id),
            query_type: "ListRuns".to_owned(),
            project_id: Some(project),
            payload: Map::from_iter([
                ("page_size".to_owned(), json!(1)),
                ("opaque_cursor".to_owned(), json!(cursor)),
            ]),
        })
        .await
}

#[tokio::test]
async fn t_f005_001_discovers_project_runs_and_opens_the_pinned_version() {
    let ((data, rendezvous, store_config), (mut create, v1, seeded_run, seeded_mission)) =
        tokio::task::spawn_blocking(|| {
            let data = local_test_dir("f005-list-runs-data").unwrap();
            let rendezvous = local_test_dir("f005-list-runs-rdv").unwrap();
            std::fs::set_permissions(rendezvous.path(), PermissionsExt::from_mode(0o700)).unwrap();
            let store_config = local_store_config(data.path());
            let (fixture, command, digest) =
                wire_run_fixture(store_config.clone(), AgentLevel::N, |_| {});
            let existing = (command, digest, fixture.run, fixture.mission);
            fixture.store.shutdown().unwrap();
            ((data, rendezvous, store_config), existing)
        })
        .await
        .unwrap();
    let project = create.project_id.unwrap();
    create.aggregate_id = PINNED_RUN.as_uuid();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let (daemon, mut client) = start(config.clone()).await;
    publish_mandate(&mut client, &mut create).await;
    let receipt = client.send_command(&create).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.result_ref, Some(format!("workflow-run:{PINNED_RUN}")));
    let detail_query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x5a04),
        query_type: "GetRun".to_owned(),
        project_id: Some(project),
        payload: Map::from_iter([("run_id".to_owned(), json!(PINNED_RUN))]),
    };
    let before = client.query(&detail_query).await.unwrap();
    assert_eq!(before.result.get("definition_hash"), Some(&json!(v1)));
    drop(client);
    daemon.shutdown_graceful("v1 run prepared").await.unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || publish_v2_and_seed_other_projects(store_config, v1))
        .await
        .unwrap();

    let (daemon, mut client) = start(config).await;
    let after = check_pages_and_open_selected_run(&mut client, detail_query, seeded_run, seeded_mission)
        .await;
    assert_eq!(after.result, before.result);
    drop(client);
    daemon.shutdown_graceful("run list verified").await.unwrap();
}

async fn check_pages_and_open_selected_run(
    client: &mut UnleashClient,
    detail_query: QueryEnvelope,
    seeded_run: RunId,
    seeded_mission: MissionId,
) -> QueryResponse {
    let project = detail_query.project_id.unwrap();
    let first = list(client, project, None, 0x5a05)
        .await
        .unwrap_or_else(|error| panic!("ListRuns must return its first page: {error}"));
    assert_eq!(first.result.get("project_id"), Some(&json!(project)));
    assert_eq!(
        first.result.pointer("/coverage/status"),
        Some(&json!("complete"))
    );
    assert_eq!(
        first.result.get("items"),
        Some(&json!([{
            "run_id": PINNED_RUN, "mission_id": MISSION,
            "workflow_version_id": PUBLISHED, "state": "prepared",
            "revision": 1, "purpose": "operational",
        }]))
    );
    let cursor = first
        .result
        .get("next_cursor")
        .and_then(Value::as_str)
        .unwrap();
    assert!(!cursor.is_empty());

    let second = list(client, project, Some(cursor), 0x5a06).await.unwrap();
    let first_mark = first.watermark.unwrap();
    let second_mark = second.watermark.unwrap();
    assert_eq!(first_mark.stream_id, second_mark.stream_id);
    assert!(first_mark.sequence <= second_mark.sequence);
    assert_eq!(second.result.get("project_id"), Some(&json!(project)));
    assert_eq!(second.result.get("next_cursor"), Some(&Value::Null));
    assert_eq!(
        second.result.pointer("/coverage/status"),
        Some(&json!("complete"))
    );
    let entries = second.result.get("items").and_then(Value::as_array).unwrap();
    assert_eq!(entries.len(), 1);
    let retained = entries.first().unwrap();
    assert_eq!(retained.get("run_id"), Some(&json!(seeded_run)));
    assert_eq!(retained.get("mission_id"), Some(&json!(seeded_mission)));

    let other = list(client, OTHER_PROJECT, None, 0x5a07).await.unwrap();
    assert_eq!(
        other.result.pointer("/items/0/run_id"),
        Some(&json!(OTHER_RUN))
    );
    assert_eq!(
        other.result.pointer("/items/0/workflow_version_id"),
        Some(&json!(PUBLISHED_V2))
    );
    assert_eq!(other.result.get("next_cursor"), Some(&Value::Null));
    let wrong_project = list(client, OTHER_PROJECT, Some(cursor), 0x5a08)
        .await
        .unwrap_err();
    assert!(matches!(wrong_project, ClientError::Remote(error)
        if error.code == ErrorCode::InvalidArgument));

    let empty = list(client, EMPTY_PROJECT, None, 0x5a09).await.unwrap();
    assert_eq!(
        empty.result,
        json!({"project_id": EMPTY_PROJECT, "items": [], "next_cursor": null,
            "coverage": {"status": "complete"}})
    );
    assert_eq!(empty.watermark.unwrap().stream_id, first_mark.stream_id);
    let selected_run = first.result.pointer("/items/0/run_id").unwrap().clone();
    client
        .query(&QueryEnvelope {
            request_id: RequestId::from_u128(0x5a0a),
            payload: Map::from_iter([("run_id".to_owned(), selected_run)]),
            ..detail_query
        })
        .await
        .unwrap()
}
