//! Sous-cas T-F005-001/009 : matérialisation locale d'une activation publiée.
#![expect(
    clippy::unwrap_used,
    reason = "Scénario de persistance dans un Store jetable"
)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::Value;
use unleash_domain::{
    ActorId, CommandId, CompletionPolicy, MissionId, MissionState, ProjectId, RequestId, Revision,
    RunId, RunState, Sha256Digest, StepId, TaskId,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonicalize_to_vec};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, CutpointObserver,
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WorkflowTaskActivationPlan,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const VERSION: &str = "workflow.fixture.m2-task:v1";

#[derive(Default)]
struct ActivationCommitCutpoint(AtomicBool);

impl CutpointObserver for ActivationCommitCutpoint {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::SeqCst) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "activation fixture before commit".to_owned(),
            });
        }
        Ok(())
    }
}

struct Fixture {
    cutpoint: Arc<ActivationCommitCutpoint>,
    store: UnleashStore,
    project: ProjectId,
    mission: MissionId,
    run: RunId,
    plan: WorkflowTaskActivationPlan,
}

impl Fixture {
    fn new(retain_definition: bool) -> (impl Drop, Self) {
        let directory = local_test_dir("f005-task-activation").unwrap();
        let cutpoint = Arc::new(ActivationCommitCutpoint::default());
        let mut config = StoreConfig::new(directory.path());
        config.cutpoints = cutpoint.clone();
        let store = UnleashStore::open(config).unwrap();
        store
            .register_principal("fixture-owner", "user", "user:fixture-owner")
            .unwrap();
        let definition: Value = serde_json::from_str(include_str!(
            "../../../.claude/examples/merge-only-workflow.json"
        ))
        .unwrap();
        let bytes = canonicalize_to_vec(&definition).unwrap();
        let definition_hash = Sha256Digest::from_bytes(&bytes);
        let bundle = Sha256Digest::from_bytes(b"effective run plan for the fixture");
        let plan = WorkflowTaskActivationPlan {
            daemon_epoch: store.daemon_state().unwrap().daemon_epoch,
            definition_hash,
            effective_bundle_hash: bundle,
            definition_node_id: "develop".to_owned(),
            iteration_id: None,
            step_attempt: 1,
            step_id: StepId::from_u128(0x5004),
            task_id: TaskId::from_u128(0x5005),
            contract_version_id: "fixture-contract:v1".to_owned(),
            scope_id: "fixture-project-scope".to_owned(),
            scope_revision: Revision::new(1),
        };
        let fixture = Self {
            cutpoint,
            store,
            project: ProjectId::from_u128(0x5001),
            mission: MissionId::from_u128(0x5002),
            run: RunId::from_u128(0x5003),
            plan,
        };
        let owner = if retain_definition {
            VERSION
        } else {
            "another-workflow:v1"
        };
        fixture.retain_definition(&bytes, owner);
        fixture.start_run();
        (directory, fixture)
    }

    fn retain_definition(&self, bytes: &[u8], owner: &str) {
        let source = self
            .store
            .prepare_object(PrepareObjectRequest {
                object_hash: self.plan.definition_hash,
                byte_length: u64::try_from(bytes.len()).unwrap(),
                media_type: "application/json".to_owned(),
                owner_kind: "workflow_version".to_owned(),
                owner_id: owner.to_owned(),
            })
            .unwrap();
        self.store
            .write_prepared_object_bytes(&source, bytes)
            .unwrap();
        self.store
            .publish_object_reference(PublishObjectReferenceRequest {
                pin_id: source.pin_id,
                object_hash: source.object_hash,
                object_generation: source.object_generation,
                owner_kind: "workflow_version".to_owned(),
                owner_id: owner.to_owned(),
                source_field: "content_hash".to_owned(),
            })
            .unwrap();
    }

    fn start_run(&self) {
        self.commit_setup(
            1,
            (
                AggregateKind::Project,
                self.project.as_uuid(),
                Revision::INITIAL,
            ),
            "CreateProject",
            CommandMutation::CreateProject {
                project_id: self.project,
                name: "task activation fixture".to_owned(),
                weight: 1,
            },
        );
        self.commit_setup(
            2,
            (
                AggregateKind::Mission,
                self.mission.as_uuid(),
                Revision::INITIAL,
            ),
            "CreateMission",
            CommandMutation::CreateMission {
                mission_id: self.mission,
                goal_id: None,
                project_id: self.project,
                owner_principal_id: Some("fixture-owner".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Ready,
            },
        );
        self.store
            .register_workflow_version(VERSION, "published", self.plan.definition_hash)
            .unwrap();
        self.commit_setup(
            3,
            (
                AggregateKind::WorkflowRun,
                self.run.as_uuid(),
                Revision::INITIAL,
            ),
            "CreateWorkflowRun",
            CommandMutation::CreateWorkflowRun {
                run_id: self.run,
                mission_id: self.mission,
                workflow_version_id: VERSION.to_owned(),
                purpose: "operational".to_owned(),
                candidate_ref: None,
                predecessor_run_id: None,
                selection_reason: "fixture version".to_owned(),
                effective_bundle_hash: self.plan.effective_bundle_hash,
            },
        );
        self.commit_setup(
            4,
            (
                AggregateKind::WorkflowRun,
                self.run.as_uuid(),
                Revision::new(1),
            ),
            "StartRun",
            CommandMutation::TransitionWorkflowRun {
                run_id: self.run,
                target_state: RunState::Running,
                ended_at_ms: None,
            },
        );
    }

    fn commit_setup(
        &self,
        command: u128,
        (kind, target, revision): (AggregateKind, Uuid, Revision),
        name: &str,
        mutation: CommandMutation,
    ) {
        let actor = ActorId::new("user:fixture-owner").unwrap();
        let mut envelope = self.envelope(command, target, revision, name);
        envelope.actor_ref = Some(actor.to_string());
        let outcome = self
            .store
            .commit_command(CommitCommandRequest {
                actor_id: actor,
                envelope,
                target_aggregate: kind,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation,
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
            .unwrap();
        assert!(matches!(outcome, CommitOutcome::Committed(_)));
    }

    fn assert_unmaterialized(&self) {
        self.store.read_snapshot(|conn, _| {
            let state: (i64, i64, i64, i64, i64) = conn.query_row(
                "SELECT
                    (SELECT revision FROM workflow_runs WHERE run_id=?1),
                    (SELECT COUNT(*) FROM step_instances),
                    (SELECT COUNT(*) FROM tasks),
                    (SELECT COUNT(*) FROM events WHERE event_type='NodeActivated'),
                    (SELECT COUNT(*) FROM command_receipts WHERE command_type='ActivateWorkflowTask')",
                [self.run.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;
            assert_eq!(state, (2, 0, 0, 0, 0));
            Ok(())
        }).unwrap();
    }

    fn envelope(
        &self,
        command: u128,
        target: Uuid,
        revision: Revision,
        name: &str,
    ) -> CommandEnvelope {
        CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(command + 10),
            command_id: CommandId::from_u128(command),
            authority_generation: self.store.active_authority_generation().unwrap(),
            actor_ref: None,
            project_id: Some(self.project),
            aggregate_id: target,
            expected_revision: revision,
            command_type: name.to_owned(),
            payload: serde_json::Map::new(),
        }
    }

    fn activation(
        &self,
        command: u128,
        revision: u64,
        plan: &WorkflowTaskActivationPlan,
    ) -> CommandEnvelope {
        let mut envelope = self.envelope(
            command,
            self.run.as_uuid(),
            Revision::new(revision),
            "ActivateWorkflowTask",
        );
        envelope.payload = serde_json::from_value(serde_json::to_value(plan).unwrap()).unwrap();
        envelope
    }
}

#[test]
fn commits_ready_task_and_reuses_receipt_on_replay() {
    let (_directory, fixture) = Fixture::new(true);
    let mut request = fixture.activation(5, 2, &fixture.plan);
    let committed = fixture
        .store
        .activate_workflow_task(request.clone())
        .unwrap();
    assert!(matches!(committed, CommitOutcome::Committed(_)));
    let receipt = committed.receipt();
    assert_eq!(receipt.new_revision, Revision::new(3));
    assert_eq!(
        receipt.result_ref.as_deref(),
        Some(format!("task:{}", fixture.plan.task_id).as_str()),
    );
    request.request_id = RequestId::from_u128(0xABC);
    let replay = fixture.store.activate_workflow_task(request).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(receipt, replay.receipt());

    let mut duplicate = fixture.plan.clone();
    duplicate.step_id = StepId::from_u128(0x5006);
    duplicate.task_id = TaskId::from_u128(0x5007);
    assert!(matches!(
        fixture
            .store
            .activate_workflow_task(fixture.activation(6, 3, &duplicate)),
        Err(StoreError::InvalidArgument { .. })
    ));

    fixture.store.read_snapshot(|conn, _| {
        let task: (String, String, String, String, i64) = conn.query_row(
            "SELECT t.mission_id, s.definition_node_id, s.state, t.state, t.revision
               FROM tasks t JOIN step_instances s ON s.step_id=t.step_id
              WHERE t.task_id=?1 AND t.run_id=?2 AND s.run_id=t.run_id AND s.step_id=?3",
            [
                fixture.plan.task_id.to_string(),
                fixture.run.to_string(),
                fixture.plan.step_id.to_string(),
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        assert_eq!(task, (fixture.mission.to_string(), "develop".to_owned(), "ready".to_owned(), "ready".to_owned(), 1));
        let effects: (i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM tasks),
                (SELECT COUNT(*) FROM events WHERE event_type='NodeActivated' AND causation_receipt_id=?1),
                (SELECT COUNT(*) FROM outbox WHERE event_id IN (SELECT event_id FROM events
                   WHERE event_type='NodeActivated' AND causation_receipt_id=?1)),
                (SELECT COUNT(*) FROM invocations),
                (SELECT COUNT(*) FROM capacity_reservations)",
            [receipt.receipt_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        assert_eq!(effects, (1, 1, 2, 0, 0));
        let payload: String = conn.query_row(
            "SELECT payload_json FROM events
              WHERE event_type='NodeActivated' AND causation_receipt_id=?1",
            [receipt.receipt_id.to_string()],
            |row| row.get(0),
        )?;
        assert_eq!(
            serde_json::from_str::<Value>(&payload)?,
            serde_json::json!({
                "run_id": fixture.run,
                "workflow_version_id": VERSION,
                "definition_hash": fixture.plan.definition_hash,
                "definition_node_id": "develop",
                "step_id": fixture.plan.step_id,
                "task_id": fixture.plan.task_id,
                "iteration_vector": [],
                "step_attempt": 1,
                "state": "ready"
            }),
        );

        Ok(())
    }).unwrap();
}

#[test]
fn refuses_a_definition_retained_by_another_version() {
    let (_directory, fixture) = Fixture::new(false);
    assert!(
        fixture
            .store
            .read_verified_object_bytes(fixture.plan.definition_hash)
            .is_ok()
    );
    assert!(matches!(
        fixture
            .store
            .activate_workflow_task(fixture.activation(5, 2, &fixture.plan)),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    fixture.assert_unmaterialized();
}

#[test]
fn requires_an_exact_task_node_and_bounded_attempt() {
    let (_directory, fixture) = Fixture::new(true);
    for (node, attempt) in [("delivery", 1), ("not-in-definition", 1), ("develop", 3)] {
        let mut plan = fixture.plan.clone();
        plan.definition_node_id = node.to_owned();
        plan.step_attempt = attempt;
        assert!(matches!(
            fixture
                .store
                .activate_workflow_task(fixture.activation(5, 2, &plan)),
            Err(StoreError::InvalidArgument { .. })
        ));
        fixture.assert_unmaterialized();
    }
    let mut plan = fixture.plan.clone();
    plan.effective_bundle_hash = Sha256Digest::from_bytes(b"different run plan");
    assert!(matches!(
        fixture
            .store
            .activate_workflow_task(fixture.activation(5, 2, &plan)),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    fixture.assert_unmaterialized();
}

#[test]
fn rolls_back_activation_and_retries_the_same_command() {
    let (_directory, fixture) = Fixture::new(true);
    let request = fixture.activation(5, 2, &fixture.plan);
    fixture.cutpoint.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture.store.activate_workflow_task(request.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    fixture.assert_unmaterialized();
    assert!(matches!(
        fixture.store.activate_workflow_task(request).unwrap(),
        CommitOutcome::Committed(_)
    ));
}
