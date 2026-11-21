//! End-to-end creation of a dormant worker from a task produced on IPC.

use std::time::Duration;

use serde_json::json;
use unleash_client::{ClientError, UnleashClient};
use unleash_domain::{
    AgentId, AgentLevel, CommandId, ReceiptId, RequestId, Revision, Sequence, TaskId,
};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
use unleash_store::{StoreConfig, UnleashStore};

use super::{MISSION, RUN, publish_mandate, start, start_run};

const AGENT: AgentId = AgentId::from_u128(0x5941);

async fn ready_task(client: &mut UnleashClient, create: &mut CommandEnvelope) -> TaskId {
    publish_mandate(client, create).await;
    client.send_command(create).await.unwrap();
    let command = start_run::start_command(create);
    let started = client.send_command(&command).await.unwrap();
    let mut stream = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: started.event_cursor.stream_id,
            after_sequence: Sequence::new(started.event_cursor.sequence.as_u64() - 1),
            project_id: create.project_id,
            aggregate_id: Some(RUN.as_uuid()),
            batch_limit: 16,
        })
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let SubscribeEventsFrame::Events { events, .. } = frame else {
        panic!("the activation must be visible in the current stream");
    };
    let event = events
        .iter()
        .find(|event| {
            event.event_type == "NodeActivated" && event.command_id == Some(command.command_id)
        })
        .unwrap();
    serde_json::from_value(event.payload.get("task_id").unwrap().clone()).unwrap()
}

fn create_agent(create: &CommandEnvelope, task: TaskId) -> CommandEnvelope {
    CommandEnvelope {
        command_type: "CreateAgent".to_owned(),
        request_id: RequestId::from_u128(0x5941),
        command_id: CommandId::from_u128(0x5941),
        aggregate_id: AGENT.as_uuid(),
        expected_revision: Revision::INITIAL,
        payload: serde_json::from_value(json!({
            "mission_id": MISSION,
            "mission_revision": 1,
            "task_id": task,
            "task_revision": 1,
        }))
        .unwrap(),
        ..create.clone()
    }
}

fn query_agent(command: &CommandEnvelope) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x5942),
        query_type: "GetAgent".to_owned(),
        project_id: command.project_id,
        payload: serde_json::from_value(json!({"agent_id": AGENT})).unwrap(),
    }
}

// T-F003-001 — Identité et session primaire durables au rejeu IPC.
#[tokio::test]
async fn creates_one_agent_and_recovers_its_primary_session() {
    let data = unleash_testkit::local_test_dir("f003-create-agent-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f003-create-agent-rdv").unwrap();
    let (store, config, mut create) =
        start_run::fixture(data.path(), rendezvous.path(), AgentLevel::N, |_| {}).await;
    let (daemon, mut client) = start(config.clone()).await;
    let epoch = client.hello().daemon_epoch;
    let task = ready_task(&mut client, &mut create).await;
    let mut command = create_agent(&create, task);
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.new_revision, Revision::new(1));
    let assignment = receipt
        .result_ref
        .as_deref()
        .unwrap()
        .strip_prefix("mission-assignment:")
        .unwrap()
        .to_owned();
    assert!(!assignment.is_empty());
    let query = query_agent(&command);
    let before = client.query(&query).await.unwrap().result;
    assert_eq!(before.get("agent_id"), Some(&json!(AGENT)));
    assert_eq!(before.get("level"), Some(&json!("N")));
    assert_eq!(before.get("state"), Some(&json!("eligible")));
    assert!(
        before
            .get("session_id")
            .is_some_and(serde_json::Value::is_string)
    );

    drop(client);
    daemon
        .shutdown_graceful("restart after agent registration")
        .await
        .unwrap();
    drop(daemon);
    let (daemon, mut client) = start(config).await;
    assert!(client.hello().daemon_epoch.as_u64() > epoch.as_u64());
    command.request_id = RequestId::from_u128(0x5943);
    assert_eq!(client.send_command(&command).await.unwrap(), receipt);
    assert_eq!(client.query(&query).await.unwrap().result, before);

    let mut changed = command;
    changed.request_id = RequestId::from_u128(0x5944);
    changed.payload.insert("task_revision".to_owned(), json!(2));
    let error = client.send_command(&changed).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::IdempotencyConflict)
    );
    drop(client);
    daemon
        .shutdown_graceful("inspect registered agent")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || {
        assert_effects(store, Some((receipt.receipt_id, assignment.as_str())));
    })
    .await
    .unwrap();
}

// T-F004-003 — L'affectation initiale exige la tâche et le plan exacts.
#[tokio::test]
async fn refuses_stale_task_and_client_supplied_binding() {
    let data = unleash_testkit::local_test_dir("f003-stale-agent-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f003-stale-agent-rdv").unwrap();
    let (store, config, mut create) =
        start_run::fixture(data.path(), rendezvous.path(), AgentLevel::N, |_| {}).await;
    let (daemon, mut client) = start(config).await;
    let task = ready_task(&mut client, &mut create).await;
    let mut command = create_agent(&create, task);
    command.payload.insert("task_revision".to_owned(), json!(2));
    let error = client.send_command(&command).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::RevisionConflict)
    );
    command.command_id = CommandId::from_u128(0x5945);
    command.request_id = RequestId::from_u128(0x5945);
    command.payload.insert("task_revision".to_owned(), json!(1));
    command
        .payload
        .insert("role_version_id".to_owned(), json!("unselected-role"));
    let error = client.send_command(&command).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::InvalidArgument)
    );
    drop(client);
    daemon
        .shutdown_graceful("inspect rejected agent creation")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || assert_effects(store, None))
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f007_016_refuses_a_leader_role_for_an_execution_agent() {
    for level in [AgentLevel::NPlus1, AgentLevel::NPlus2] {
        let data = unleash_testkit::local_test_dir("f003-role-level-data").unwrap();
        let rendezvous = unleash_testkit::local_test_dir("f003-role-level-rdv").unwrap();
        let (store, config, mut create) =
            start_run::fixture(data.path(), rendezvous.path(), level, |_| {}).await;
        let (daemon, mut client) = start(config).await;
        let task = ready_task(&mut client, &mut create).await;
        let result = client.send_command(&create_agent(&create, task)).await;
        drop(client);
        daemon
            .shutdown_graceful("inspect rejected role level")
            .await
            .unwrap();
        drop(daemon);
        let error =
            result.expect_err("the retained leadership role must not bind to an execution agent");
        assert!(matches!(error, ClientError::Remote(error)
            if error.code == ErrorCode::InvalidArgument));
        tokio::task::spawn_blocking(move || assert_effects(store, None))
            .await
            .unwrap();
    }
}

fn assert_effects(config: StoreConfig, accepted: Option<(ReceiptId, &str)>) {
    let store = UnleashStore::open(config).unwrap();
    let (receipt, assignment) = accepted.map_or_else(
        || (String::new(), ""),
        |(receipt, assignment)| (receipt.to_string(), assignment),
    );
    let effects: (i64, i64, i64, i64, i64) = store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT
              (SELECT COUNT(*) FROM agents WHERE agent_id=?1),
              (SELECT COUNT(*) FROM sessions WHERE agent_id=?1 AND state='open'),
              (SELECT COUNT(*) FROM mission_assignments a JOIN missions m USING(mission_id)
                 WHERE a.agent_id=?1 AND a.mission_id=?2 AND a.assignment_id=?3 AND a.state='active'
                   AND a.scope_id=m.project_id AND a.scope_revision=1 AND a.role_version_id='role-v1'),
              (SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='AgentCreated'
                 AND causation_receipt_id=?4),
              (SELECT COUNT(*) FROM invocations WHERE agent_id=?1)",
            rusqlite::params![AGENT.to_string(), MISSION.to_string(), assignment, receipt],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        )?)
    }).unwrap();
    assert_eq!(
        effects,
        if accepted.is_some() {
            (1, 1, 1, 1, 0)
        } else {
            (0, 0, 0, 0, 0)
        }
    );
    store.shutdown().unwrap();
}
