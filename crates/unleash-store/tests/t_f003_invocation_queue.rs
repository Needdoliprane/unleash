//! Store part of REQ-AGENT-002. The fixture supplies already resolved inputs.
#![expect(clippy::unwrap_used, reason = "Fixture setup and contract assertions")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Map;
use unleash_domain::{
    ActorId, AgentId, AgentLevel, AgentState, CommandId, InvocationId, ProjectId, RequestId,
    Revision, SessionId, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, CutpointObserver,
    EventAppendSpec, InvocationQueuePlan, PrepareObjectRequest, PublishObjectReferenceRequest,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROJECT: ProjectId = ProjectId::from_u128(1);
const AGENT: AgentId = AgentId::from_u128(2);
const INVOCATION: InvocationId = InvocationId::from_u128(3);
const MANIFEST: Uuid = Uuid::from_u128(4);

fn envelope(
    store: &UnleashStore,
    id: u128,
    aggregate: Uuid,
    actor: &str,
    command: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: store.active_authority_generation().unwrap(),
        actor_ref: Some(actor.to_owned()),
        project_id: Some(PROJECT),
        aggregate_id: aggregate,
        expected_revision: Revision::INITIAL,
        command_type: command.to_owned(),
        payload: Map::new(),
    }
}

fn setup_command(
    envelope: CommandEnvelope,
    target_aggregate: AggregateKind,
    mutation: CommandMutation,
    events: Vec<EventAppendSpec>,
) -> CommitCommandRequest {
    CommitCommandRequest {
        actor_id: ActorId::new("user:fixture").unwrap(),
        envelope,
        target_aggregate,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events,
        capacity_reservations: Vec::new(),
        budget_reservations: Vec::new(),
    }
}

fn publish(store: &UnleashStore, bytes: &[u8], field: &str) -> Sha256Digest {
    let hash = Sha256Digest::from_bytes(bytes);
    let pin = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "evidence".into(),
            owner_id: "queue-fixture".into(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&pin, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: pin.pin_id,
            object_hash: hash,
            object_generation: pin.object_generation,
            owner_kind: "evidence".into(),
            owner_id: "queue-fixture".into(),
            source_field: field.into(),
        })
        .unwrap();
    hash
}

fn resolved_invocation(store: &UnleashStore) -> CommandEnvelope {
    store
        .commit_command(setup_command(
            envelope(
                store,
                10,
                PROJECT.as_uuid(),
                "user:fixture",
                "CreateProject",
            ),
            AggregateKind::Project,
            CommandMutation::CreateProject {
                project_id: PROJECT,
                name: "fixture".into(),
                weight: 1,
            },
            Vec::new(),
        ))
        .unwrap();
    store
        .commit_command(setup_command(
            envelope(store, 11, AGENT.as_uuid(), "user:fixture", "CreateAgent"),
            AggregateKind::Agent,
            CommandMutation::CreateAgent {
                agent_id: AGENT,
                level: AgentLevel::NPlus2,
                organization_parent_id: None,
                created_by_agent_id: None,
                project_id: Some(PROJECT),
                state: AgentState::Eligible,
            },
            vec![EventAppendSpec {
                event_type: "AgentReadyFixture".into(),
                aggregate_type: AggregateKind::Agent,
                aggregate_id: AGENT.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(PROJECT),
                correlation_id: AGENT.as_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: Map::new(),
                sensitivity: Sensitivity::Project,
                outbox_destinations: Vec::new(),
            }],
        ))
        .unwrap();
    let (session, trigger): (String, String) = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT s.session_id,e.event_id FROM sessions s
             JOIN events e ON e.aggregate_id=s.agent_id AND e.event_type='AgentReadyFixture'
             WHERE s.agent_id=?1",
                [AGENT.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    let context = publish(store, br#"{"entries":[],"schema_version":1}"#, "context");
    let bundle = publish(
        store,
        br#"{"fixture":"resolved","schema_version":1}"#,
        "bundle",
    );
    let session_id: SessionId = session.parse().unwrap();
    let policy = Sha256Digest::from_bytes(b"resolved fixture policy");
    store.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
        tx.execute(
            "INSERT INTO context_manifests(manifest_id,session_id,base_session_revision,content_hash,schema_version,created_at_ms)
             VALUES(?1,?2,0,?3,1,?4)",
            rusqlite::params![MANIFEST.to_string(), session_id.to_string(), context.to_string(), now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO effective_bundles(bundle_hash,schema_version,role_version_id,requested_model_profile_version_id,
                 allowed_model_policy_hash,initial_context_manifest_id,scope_id,scope_revision,content_object_hash)
             VALUES(?1,1,'fixture-role','fixture-model',?2,?3,'fixture-scope',1,?1)",
            rusqlite::params![bundle.to_string(), policy.to_string(), MANIFEST.to_string()],
        )?;
        Ok(())
    }).unwrap();
    let mut wire = envelope(
        store,
        12,
        INVOCATION.as_uuid(),
        "system:harness",
        "CreateInvocation",
    );
    wire.payload = serde_json::from_value(
        serde_json::to_value(InvocationQueuePlan {
            daemon_epoch: store.daemon_state().unwrap().daemon_epoch,
            agent_id: AGENT,
            agent_revision: Revision::new(1),
            session_id,
            session_revision: Revision::INITIAL,
            trigger_event_id: trigger.parse().unwrap(),
            effective_bundle_hash: bundle,
        })
        .unwrap(),
    )
    .unwrap();
    wire
}

type Counts = (i64, i64, i64, i64, i64);

fn counts(store: &UnleashStore) -> Counts {
    store.read_snapshot(|conn, _| Ok(conn.query_row(
        "SELECT (SELECT count(*) FROM invocations),
                (SELECT count(*) FROM invocation_attempts),
                (SELECT count(*) FROM capacity_reservations),
                (SELECT count(*) FROM events WHERE event_type='InvocationQueued'),
                (SELECT count(*) FROM outbox o JOIN events e ON e.event_id=o.event_id WHERE e.event_type='InvocationQueued')",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))
    )?)).unwrap()
}

#[test]
fn resolved_work_is_queued_once_without_an_admission() {
    let directory = local_test_dir("f003-invocation-queue").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let wire = resolved_invocation(&store);
    let committed = store.queue_invocation(wire.clone()).unwrap();
    let receipt = committed.receipt();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.result_ref, Some(format!("invocation:{INVOCATION}")));

    // T-F003-002 / AC-F003-002 / REQ-AGENT-002 — Même audience que l'événement
    // `InvocationQueued` d'`AssignTask` (`task_assignment`) : le harnais seul. L'événement est
    // `Restricted` ; son audience n'est pas élargie aux clients sans décision du dossier.
    let destinations: Vec<String> = store
        .read_snapshot(|conn, _| {
            let mut statement = conn.prepare(
                "SELECT o.destination FROM outbox o JOIN events e ON e.event_id=o.event_id
                 WHERE e.event_type='InvocationQueued' ORDER BY o.destination",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap();
    assert_eq!(destinations, ["harness"]);
    assert_eq!(counts(&store), (1, 0, 0, 1, 1));

    let invocation = store
        .get_invocation_snapshot(INVOCATION, Some(PROJECT), None, 0, 25)
        .unwrap();
    assert_eq!(invocation.value.state, "queued");
    assert_eq!(invocation.value.context_manifest_id, MANIFEST);
    let event: (i64, String, i64) = store.read_snapshot(|conn, _| {
        Ok(conn.query_row("SELECT sequence,causation_id,
            (SELECT count(*) FROM object_references WHERE owner_kind='invocation' AND owner_id=?1 AND state='live')
            FROM events WHERE event_type='InvocationQueued'",
            [INVOCATION.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?)
    }).unwrap();
    assert_eq!(
        u64::try_from(event.0).unwrap(),
        receipt.event_cursor.sequence.as_u64()
    );
    assert_eq!(
        event.1,
        wire.payload
            .get("trigger_event_id")
            .unwrap()
            .as_str()
            .unwrap()
    );
    assert_eq!(event.2, 2);
    let replay = store.queue_invocation(wire.clone()).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(replay.receipt(), receipt);
    let mut competing = wire;
    competing.command_id = CommandId::from_u128(13);
    competing.aggregate_id = InvocationId::from_u128(5).as_uuid();
    assert!(matches!(
        store.queue_invocation(competing),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert_eq!(counts(&store), (1, 0, 0, 1, 1));
}

struct RejectCommit(AtomicBool);
impl CutpointObserver for RejectCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".into(),
                message: "fixture".into(),
            });
        }
        Ok(())
    }
}

#[test]
fn queue_cutpoint_and_stale_session_leave_no_partial_work() {
    let directory = local_test_dir("f003-invocation-queue-rollback").unwrap();
    let failures = Arc::new(RejectCommit(AtomicBool::new(false)));
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = failures.clone();
    let store = UnleashStore::open(config).unwrap();
    let wire = resolved_invocation(&store);
    let mut stale = wire.clone();
    stale
        .payload
        .insert("session_revision".into(), serde_json::json!(1));
    assert!(matches!(
        store.queue_invocation(stale),
        Err(StoreError::RevisionConflict { .. })
    ));
    let mut missing_trigger = wire.clone();
    missing_trigger.payload.insert(
        "trigger_event_id".into(),
        serde_json::json!(Uuid::from_u128(999)),
    );
    assert!(matches!(
        store.queue_invocation(missing_trigger),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    failures.0.store(true, Ordering::Release);
    assert!(matches!(
        store.queue_invocation(wire.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), (0, 0, 0, 0, 0));
    assert!(matches!(
        store.queue_invocation(wire),
        Ok(CommitOutcome::Committed(_))
    ));
    assert_eq!(counts(&store), (1, 0, 0, 1, 1));
}
