//! Store portion of T-F003-001 and T-F004-002: durable identity before any worker starts.
#![expect(
    clippy::unwrap_used,
    reason = "Local setup and assertions for the Store contract"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Map;
use unleash_domain::{ActorId, AgentId, AgentLevel, AgentState, CommandId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, CutpointObserver,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;

const AGENT: AgentId = AgentId::from_u128(1);
const COMMAND: CommandId = CommandId::from_u128(2);

fn create_agent(store: &UnleashStore) -> CommitCommandRequest {
    let actor = ActorId::new("user:fixture").unwrap();
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(3),
            command_id: COMMAND,
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some(actor.to_string()),
            project_id: None,
            aggregate_id: AGENT.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateAgent".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Agent,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::CreateAgent {
            agent_id: AGENT,
            level: AgentLevel::NPlus2,
            organization_parent_id: None,
            created_by_agent_id: None,
            project_id: None,
            state: AgentState::Dormant,
        },
        events: Vec::new(),
        capacity_reservations: Vec::new(),
        budget_reservations: Vec::new(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SessionRow {
    id: String,
    state: String,
    revision: i64,
    head_manifest_id: Option<String>,
}

fn agent_sessions(store: &UnleashStore) -> Vec<SessionRow> {
    store
        .read_snapshot(|conn, _| {
            let state: String = conn.query_row(
                "SELECT state FROM agents WHERE agent_id = ?1",
                [AGENT.to_string()],
                |row| row.get(0),
            )?;
            assert_eq!(state, "dormant");
            let mut select = conn.prepare(
                "SELECT session_id, state, revision, head_manifest_id FROM sessions
                 WHERE agent_id = ?1 ORDER BY session_id",
            )?;
            let sessions = select.query_map([AGENT.to_string()], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    state: row.get(1)?,
                    revision: row.get(2)?,
                    head_manifest_id: row.get(3)?,
                })
            })?;
            Ok(sessions.collect::<rusqlite::Result<_>>()?)
        })
        .unwrap()
}

#[test]
fn t_f003_001_create_agent_persists_one_distinct_session_across_replay_and_restart() {
    let dir = local_test_dir("f003-agent-session").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let command = create_agent(&store);
    let created = store.commit_command(command.clone()).unwrap();
    assert!(matches!(created, CommitOutcome::Committed(_)));
    let sessions = agent_sessions(&store);
    assert_eq!(
        sessions.len(),
        1,
        "agent and primary session must commit together"
    );
    let primary = sessions.first().unwrap();
    assert_ne!(primary.id, AGENT.to_string());
    assert_eq!(primary.state, "open");
    assert_eq!(primary.revision, 0);
    assert_eq!(primary.head_manifest_id, None);
    store.shutdown().unwrap();

    let reopened = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let replay = reopened.commit_command(command).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(replay.receipt(), created.receipt());
    assert_eq!(agent_sessions(&reopened), sessions);
    let counts = reopened
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT (SELECT COUNT(*) FROM invocations),
                        (SELECT COUNT(*) FROM capacity_reservations)",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(
        counts,
        (0, 0),
        "registering an agent does not start an execution"
    );
    reopened.shutdown().unwrap();
}

struct RejectCommit(AtomicBool);

impl CutpointObserver for RejectCommit {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        if cutpoint == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::AcqRel)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "fixture interrupts the combined transaction".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f003_001_rollback_leaves_no_agent_session_or_receipt() {
    let dir = local_test_dir("f003-agent-session-c1").unwrap();
    let cutpoint = Arc::new(RejectCommit(AtomicBool::new(false)));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.cutpoints = cutpoint.clone();
    let store = UnleashStore::open(cfg).unwrap();
    let command = create_agent(&store);
    cutpoint.0.store(true, Ordering::Release);
    let error = store.commit_command(command.clone()).unwrap_err();
    assert!(matches!(error, StoreError::CutpointInjected { .. }));
    let counts = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT (SELECT COUNT(*) FROM agents WHERE agent_id = ?1),
                        (SELECT COUNT(*) FROM sessions WHERE agent_id = ?1),
                        (SELECT COUNT(*) FROM command_receipts WHERE command_id = ?2)",
                [AGENT.to_string(), COMMAND.to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?)
        })
        .unwrap();
    assert_eq!(counts, (0, 0, 0));
    assert!(matches!(
        store.commit_command(command).unwrap(),
        CommitOutcome::Committed(_)
    ));
    assert_eq!(agent_sessions(&store).len(), 1);
    store.shutdown().unwrap();
}
