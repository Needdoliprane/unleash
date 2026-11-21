//! Durable creation of an agent, its session, and the initial mission assignment.

#![expect(
    clippy::unwrap_used,
    reason = "Isolated Store fixture and contract assertions"
)]

#[expect(
    dead_code,
    reason = "The workflow fixture also serves the task and IPC suites"
)]
#[path = "support/workflow_task.rs"]
mod workflow_task;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use unleash_domain::{ActorId, AgentId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ReceiptState};
use unleash_store::{
    CallerAuthority, CommitOutcome, CutpointObserver, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;
use workflow_task::{Fixture, ROLE};

const AGENT: AgentId = AgentId::from_u128(0x53_001);
const ASSIGNMENT: Uuid = Uuid::from_u128(0x53_002);
const OWNER: &str = "user:fixture-owner";

#[derive(Default)]
struct InterruptCommit(AtomicBool);

impl CutpointObserver for InterruptCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "fixture interrupts agent creation".to_owned(),
            });
        }
        Ok(())
    }
}

fn prepare(config: StoreConfig) -> (Fixture, CommandEnvelope, CallerAuthority) {
    let store = UnleashStore::open(config).unwrap();
    let actor = ActorId::new(OWNER).unwrap();
    let fixture = Fixture::on_store(store, OWNER, actor.clone(), true);
    fixture
        .store
        .activate_workflow_task(fixture.activation(5, 2, &fixture.plan))
        .unwrap();
    let (command, caller) = fixture.prepare_agent_creation(11, AGENT, ASSIGNMENT);
    (fixture, command, caller)
}

fn rows(store: &UnleashStore) -> (i64, i64, i64, i64, i64, i64, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT (SELECT count(*) FROM agents), (SELECT count(*) FROM sessions),
                         (SELECT count(*) FROM mission_assignments),
                         (SELECT count(*) FROM command_receipts WHERE command_type='CreateAgent'),
                         (SELECT count(*) FROM invocations),
                         (SELECT count(*) FROM capacity_reservations),
                         (SELECT count(*) FROM events WHERE event_type='AgentCreated')",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )?)
        })
        .unwrap()
}

#[test]
fn t_f003_creates_agent_session_and_assignment_once_across_reopen() {
    let dir = local_test_dir("f003-mission-agent").unwrap();
    let config = StoreConfig::new(dir.path());
    let (fixture, mut command, caller) = prepare(config.clone());
    let first = fixture
        .store
        .create_assigned_agent(command.clone(), caller.clone())
        .unwrap();
    assert!(matches!(first, CommitOutcome::Committed(_)));
    assert_eq!(first.receipt().state, ReceiptState::Completed);
    assert_eq!(first.receipt().new_revision, Revision::new(1));
    assert_eq!(
        first.receipt().result_ref,
        Some(format!("mission-assignment:{ASSIGNMENT}"))
    );
    assert_eq!(rows(&fixture.store), (1, 1, 1, 1, 0, 0, 1));
    let scope = fixture.plan.scope_id;
    let mission = fixture.mission.to_string();
    fixture.store.shutdown().unwrap();

    let reopened = UnleashStore::open(config).unwrap();
    command.request_id = RequestId::from_u128(0x53_003);
    let replay = reopened.create_assigned_agent(command, caller).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(first.receipt(), replay.receipt());
    assert_eq!(rows(&reopened), (1, 1, 1, 1, 0, 0, 1));
    reopened
        .read_snapshot(|conn, _| {
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM mission_assignments a
                 JOIN agents g ON g.agent_id=a.agent_id
                 JOIN sessions s ON s.agent_id=g.agent_id
                 JOIN object_references rr ON rr.owner_kind='mission_assignment'
                    AND rr.owner_id=a.assignment_id AND rr.source_field='role_version_id'
                    AND rr.state='live'
                 JOIN registry_versions rv ON rv.registry_version_id=a.role_version_id
                    AND rv.content_hash=rr.object_hash
                 JOIN events e ON e.aggregate_id=g.agent_id AND e.event_type='AgentCreated'
                    AND json_extract(e.payload_json, '$.session_id')=s.session_id
                    AND json_extract(e.payload_json, '$.assignment_id')=a.assignment_id
                 JOIN command_receipts cr ON cr.receipt_id=e.causation_receipt_id
                    AND cr.command_type='CreateAgent'
                 WHERE a.assignment_id=?1 AND a.mission_id=?2 AND a.agent_id=?3
                    AND a.role_version_id=?4 AND a.scope_id=?5 AND a.scope_revision=1
                    AND a.state='active' AND a.assignment_epoch=1 AND a.revision=1
                    AND g.state='eligible' AND s.state='open' AND s.revision=0",
                rusqlite::params![
                    ASSIGNMENT.to_string(),
                    mission,
                    AGENT.to_string(),
                    ROLE,
                    scope
                ],
                |row| row.get(0),
            )?;
            assert_eq!(count, 1);
            Ok(())
        })
        .unwrap();
    reopened.shutdown().unwrap();
}

#[test]
fn t_f003_rolls_back_the_agent_and_assignment_together() {
    let dir = local_test_dir("f003-mission-agent-rollback").unwrap();
    let cutpoint = Arc::new(InterruptCommit::default());
    let mut config = StoreConfig::new(dir.path());
    config.cutpoints = cutpoint.clone();
    let (fixture, command, caller) = prepare(config);
    cutpoint.0.store(true, Ordering::Release);
    assert!(matches!(
        fixture
            .store
            .create_assigned_agent(command.clone(), caller.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(rows(&fixture.store), (0, 0, 0, 0, 0, 0, 0));
    fixture
        .store
        .create_assigned_agent(command, caller)
        .unwrap();
    assert_eq!(rows(&fixture.store), (1, 1, 1, 1, 0, 0, 1));
    fixture.store.shutdown().unwrap();
}

#[test]
fn t_f003_declines_a_plan_when_the_published_role_or_scope_changes() {
    let dir = local_test_dir("f003-mission-agent-preconditions").unwrap();
    let (fixture, command, caller) = prepare(StoreConfig::new(dir.path()));
    let mut stale = command.clone();
    stale.payload.insert("scope_revision".to_owned(), 2.into());
    assert!(
        fixture
            .store
            .create_assigned_agent(stale, caller.clone())
            .is_err()
    );
    assert_eq!(rows(&fixture.store), (0, 0, 0, 0, 0, 0, 0));
    fixture
        .store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, _| {
            tx.execute(
                "UPDATE registry_versions SET publication_state='revoked' WHERE registry_version_id=?1",
                [ROLE],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        fixture
            .store
            .create_assigned_agent(command, caller)
            .is_err()
    );
    assert_eq!(rows(&fixture.store), (0, 0, 0, 0, 0, 0, 0));
    fixture.store.shutdown().unwrap();
}
