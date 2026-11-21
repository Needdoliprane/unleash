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
    ActorId, AgentId, AgentLevel, AgentState, CommandId, CompletionPolicy, InvocationId, MissionId,
    MissionState, ProjectId, RequestId, Revision, RunId, RunState, Sha256Digest, StepId, TaskId,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonicalize_to_vec};
use unleash_store::{
    AdmissionUnit, AggregateKind, CallerAuthority, CallerRole, CommandMutation,
    CommitCommandRequest, CommitOutcome, CutpointObserver, ExecutionPoolParams, InitialContextPlan,
    PrepareObjectRequest, PreparedTaskAssignment, PublishObjectReferenceRequest, StoreConfig,
    StoreCutpoint, StoreError, UnleashStore, WorkflowTaskActivationPlan, WriteCriticality,
    objects::PublishedObjectReference,
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
        fixture.retain_object(&bytes, "workflow_version", owner, "content_hash");
        fixture.start_run();
        (directory, fixture)
    }

    fn retain_object(
        &self,
        bytes: &[u8],
        kind: &str,
        owner: &str,
        field: &str,
    ) -> PublishedObjectReference {
        let source = self
            .store
            .prepare_object(PrepareObjectRequest {
                object_hash: Sha256Digest::from_bytes(bytes),
                byte_length: u64::try_from(bytes.len()).unwrap(),
                media_type: "application/json".to_owned(),
                owner_kind: kind.to_owned(),
                owner_id: owner.to_owned(),
            })
            .unwrap();
        self.store
            .write_prepared_object_bytes(&source, bytes)
            .unwrap();
        self.store
            .publish_owned_object_reference(PublishObjectReferenceRequest {
                pin_id: source.pin_id,
                object_hash: source.object_hash,
                object_generation: source.object_generation,
                owner_kind: kind.to_owned(),
                owner_id: owner.to_owned(),
                source_field: field.to_owned(),
            })
            .unwrap()
    }

    fn activate_with_assignee(&self) -> (AgentId, String, String) {
        self.store
            .activate_workflow_task(self.activation(5, 2, &self.plan))
            .unwrap();
        let agent = AgentId::from_u128(0x5008);
        self.commit_setup(
            7,
            (AggregateKind::Agent, agent.as_uuid(), Revision::INITIAL),
            "CreateAgent",
            CommandMutation::CreateAgent {
                agent_id: agent,
                level: AgentLevel::N,
                organization_parent_id: None,
                created_by_agent_id: None,
                project_id: Some(self.project),
                state: AgentState::Eligible,
            },
        );
        let (session_id, trigger_event_id): (String, String) = self
            .store
            .read_snapshot(|conn, _| {
                let session = conn.query_row(
                    "SELECT session_id FROM sessions WHERE agent_id=?1",
                    [agent.to_string()],
                    |row| row.get(0),
                )?;
                let trigger = conn.query_row(
                "SELECT event_id FROM events WHERE event_type='NodeActivated' AND aggregate_id=?1",
                [self.run.to_string()],
                |row| row.get(0),
            )?;
                Ok((session, trigger))
            })
            .unwrap();
        (agent, session_id, trigger_event_id)
    }

    fn prepared_assignment(&self) -> (CommandEnvelope, CallerAuthority, PreparedTaskAssignment) {
        let (agent, session_id, trigger_event_id) = self.activate_with_assignee();
        let assignment_id = Uuid::from_u128(0x500a);
        let (context_plan, context) = initial_context_fixture(&self.store, &session_id, &[]);
        let manifest = context_plan.manifest_id.to_string();
        let policy_hash = Sha256Digest::from_bytes(b"policy defined for the assignment fixture");
        self.store
            .publish_initial_context(context_plan, &context, vec![])
            .unwrap();
        let bundle = canonicalize_to_vec(&serde_json::json!({
            "schema_version": 1,
            "agent_id": agent,
            "session_id": session_id,
            "trigger_event_id": trigger_event_id,
            "mission_id": self.mission,
            "run_id": self.run,
            "step_id": self.plan.step_id,
            "task_id": self.plan.task_id,
            "role_version_id": "fixture-role:v1",
            "workflow_version_id": VERSION,
            "contract_version_ids": [&self.plan.contract_version_id],
            "requested_model_profile_version_id": "fixture-profile:v1",
            "allowed_model_policy_hash": policy_hash,
            "scope_id": self.plan.scope_id,
            "scope_revision": self.plan.scope_revision,
            "initial_context_manifest_id": manifest,
        }))
        .unwrap();
        let bundle_hash = Sha256Digest::from_bytes(&bundle);
        self.retain_object(
            &bundle,
            "effective_bundle",
            &bundle_hash.to_string(),
            "content_object_hash",
        );
        // The real activation and session start are above. Seed only the already
        // resolved mission binding and bundle expected from the harness.
        let mission = self.mission;
        let scope = self.plan.scope_id.clone();
        self.store
            .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
                tx.execute(
                    "INSERT INTO mission_assignments
                 (assignment_id, mission_id, agent_id, role_version_id, scope_id,
                  scope_revision, assignment_epoch, state, revision)
                 VALUES (?1, ?2, ?3, 'fixture-role:v1', ?4, 1, 1, 'active', 1)",
                    rusqlite::params![
                        assignment_id.to_string(),
                        mission.to_string(),
                        agent.to_string(),
                        scope
                    ],
                )?;
                tx.execute(
                    "INSERT INTO effective_bundles
                 (bundle_hash, schema_version, role_version_id, workflow_version_id,
                  requested_model_profile_version_id, allowed_model_policy_hash,
                  initial_context_manifest_id, scope_id, scope_revision, content_object_hash)
                 VALUES (?1, 1, 'fixture-role:v1', ?2, 'fixture-profile:v1', ?3, ?4, ?5, 1, ?1)",
                    rusqlite::params![
                        bundle_hash.to_string(),
                        VERSION,
                        policy_hash.to_string(),
                        manifest,
                        scope
                    ],
                )?;
                Ok(())
            })
            .unwrap();
        let mut envelope = self.envelope(
            8,
            self.plan.task_id.as_uuid(),
            Revision::new(1),
            "AssignTask",
        );
        envelope
            .payload
            .insert("agent_id".to_owned(), serde_json::to_value(agent).unwrap());
        let caller = CallerAuthority {
            actor_id: ActorId::new("user:fixture-owner").unwrap(),
            role: CallerRole::HumanOperator,
        };
        let preparation = PreparedTaskAssignment {
            daemon_epoch: self.plan.daemon_epoch,
            assignment_id,
            assignment_epoch: 1,
            assignment_revision: Revision::new(1),
            trigger_event_id: trigger_event_id.parse().unwrap(),
            effective_bundle_hash: bundle_hash,
            invocation_id: InvocationId::from_u128(0x500c),
        };
        (envelope, caller, preparation)
    }

    fn assert_assignment_pending(&self) {
        self.store.read_snapshot(|conn, _| {
            let revisions: (i64, i64) = conn.query_row(
                "SELECT t.revision,s.revision FROM tasks t JOIN step_instances s ON s.step_id=t.step_id
                 WHERE t.task_id=?1 AND t.state='ready' AND s.state='ready'",
                [self.plan.task_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(revisions, (1, 1));
            let effects: (i64, i64, i64, i64) = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM invocations),
                        (SELECT COUNT(*) FROM events WHERE event_type IN ('TaskAssigned','InvocationQueued')),
                        (SELECT COUNT(*) FROM object_references WHERE owner_kind='invocation'),
                        (SELECT COUNT(*) FROM command_receipts WHERE command_type='AssignTask')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert_eq!(effects, (0, 0, 0, 0));
            Ok(())
        }).unwrap();
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

#[test]
fn publishes_initial_context_once_with_its_ordered_input() {
    let (_directory, fixture) = Fixture::new(true);
    let (_, session, _) = fixture.activate_with_assignee();
    let data = b"operator input retained in the session fixture";
    let input = fixture.retain_object(data, "session", &session, "fixture_input");
    let (plan, document) =
        initial_context_fixture(&fixture.store, &session, &[Sha256Digest::from_bytes(data)]);
    let bytes = canonicalize_to_vec(&document).unwrap();
    let digest = fixture
        .store
        .publish_initial_context(plan, &document, vec![input.clone()])
        .unwrap();
    assert_eq!(digest, Sha256Digest::from_bytes(&bytes));
    assert_eq!(
        fixture
            .store
            .publish_initial_context(plan, &document, vec![input])
            .unwrap(),
        digest
    );
    assert_eq!(
        fixture.store.read_verified_object_bytes(digest).unwrap(),
        bytes
    );
    fixture
        .store
        .read_snapshot(|conn, _| {
            let state: (i64, i64, i64, Option<String>, i64) = conn.query_row(
                "SELECT
                (SELECT COUNT(*) FROM context_manifests WHERE manifest_id=?1 AND content_hash=?2
                    AND session_id=?3 AND base_session_revision=0 AND parent_manifest_id IS NULL),
                (SELECT COUNT(*) FROM object_references WHERE owner_kind='context_manifest'
                    AND owner_id=?1 AND state='live'),
                revision,head_manifest_id,
                (SELECT COUNT(*) FROM object_pins WHERE owner_kind='context_manifest'
                    AND owner_id=?1 AND state='held')
             FROM sessions WHERE session_id=?3",
                [plan.manifest_id.to_string(), digest.to_string(), session],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
            assert_eq!(state, (1, 2, 0, None, 0));
            Ok(())
        })
        .unwrap();
}

#[test]
fn preserves_the_published_context_when_the_session_moves() {
    let (_directory, fixture) = Fixture::new(true);
    let (_, session, _) = fixture.activate_with_assignee();
    let (plan, document) = initial_context_fixture(&fixture.store, &session, &[]);
    let digest = fixture
        .store
        .publish_initial_context(plan, &document, vec![])
        .unwrap();

    let mut changed = document.clone();
    changed.as_object_mut().unwrap().insert(
        "selection_reason".to_owned(),
        serde_json::json!("another legitimate selection"),
    );
    assert!(matches!(
        fixture
            .store
            .publish_initial_context(plan, &changed, vec![]),
        Err(StoreError::InvalidArgument { .. })
    ));
    let original_session = session.clone();
    fixture
        .store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute(
                "UPDATE sessions SET revision=1 WHERE session_id=?1",
                [original_session],
            )?;
            Ok(())
        })
        .unwrap();
    let pending = InitialContextPlan {
        manifest_id: Uuid::from_u128(0x5011),
        ..plan
    };
    assert!(matches!(
        fixture
            .store
            .publish_initial_context(pending, &document, vec![]),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(
        fixture
            .store
            .publish_initial_context(plan, &document, vec![])
            .unwrap(),
        digest
    );
    assert_unpublished_context(&fixture.store, pending.manifest_id);
}

#[test]
fn releases_the_context_pin_if_an_input_is_retired() {
    let (_directory, fixture) = Fixture::new(true);
    let (_, session, _) = fixture.activate_with_assignee();
    let first = b"first authorized fixture entry";
    let last = b"last authorized fixture entry";
    let first_ref = fixture.retain_object(first, "session", &session, "first_entry");
    let last_ref = fixture.retain_object(last, "session", &session, "last_entry");
    let (plan, document) = initial_context_fixture(
        &fixture.store,
        &session,
        &[
            Sha256Digest::from_bytes(first),
            Sha256Digest::from_bytes(last),
        ],
    );
    fixture
        .store
        .retire_owned_object_reference(last_ref.clone(), "fixture_input_retired")
        .unwrap();

    assert!(matches!(
        fixture
            .store
            .publish_initial_context(plan, &document, vec![first_ref, last_ref]),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_unpublished_context(&fixture.store, plan.manifest_id);
}

fn initial_context_fixture(
    store: &UnleashStore,
    session: &str,
    hashes: &[Sha256Digest],
) -> (InitialContextPlan, Value) {
    let daemon = store.daemon_state().unwrap();
    let plan = InitialContextPlan {
        manifest_id: Uuid::from_u128(0x5010),
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
    };
    let entries: Vec<_> = hashes
        .iter()
        .map(|hash| serde_json::json!({"object_hash": hash}))
        .collect();
    let document = serde_json::json!({
        "schema_version": 1,
        "session_id": session,
        "base_session_revision": 0,
        "parent_manifest_id": null,
        "entries": entries,
    });
    (plan, document)
}

fn assert_unpublished_context(store: &UnleashStore, manifest: Uuid) {
    store.read_snapshot(|conn, _| {
        let retained: (i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM context_manifests WHERE manifest_id=?1),
                (SELECT COUNT(*) FROM object_references WHERE owner_kind='context_manifest' AND owner_id=?1),
                (SELECT COUNT(*) FROM object_pins WHERE owner_kind='context_manifest' AND owner_id=?1 AND state='held')",
            [manifest.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(retained, (0, 0, 0));
        Ok(())
    }).unwrap();
}

#[test]
fn refuses_a_stale_assignment_revision_before_queueing() {
    let (_directory, fixture) = Fixture::new(true);
    let (envelope, caller, preparation) = fixture.prepared_assignment();
    let mut stale = preparation.clone();
    stale.assignment_revision = Revision::new(2);
    assert!(matches!(
        fixture
            .store
            .assign_task(envelope.clone(), caller.clone(), stale),
        Err(StoreError::StaleFence { .. })
    ));
    fixture.assert_assignment_pending();
    fixture
        .store
        .assign_task(envelope, caller, preparation)
        .unwrap();
}

#[test]
fn rolls_back_assignment_and_retries_the_same_command() {
    let (_directory, fixture) = Fixture::new(true);
    let (envelope, caller, preparation) = fixture.prepared_assignment();
    fixture.cutpoint.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture
            .store
            .assign_task(envelope.clone(), caller.clone(), preparation.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    fixture.assert_assignment_pending();
    assert!(matches!(
        fixture
            .store
            .assign_task(envelope, caller, preparation)
            .unwrap(),
        CommitOutcome::Committed(_)
    ));
}

#[test]
fn commits_assignment_and_queues_the_exact_invocation() {
    let (_directory, fixture) = Fixture::new(true);
    let (mut envelope, caller, preparation) = fixture.prepared_assignment();
    let outcome = fixture
        .store
        .assign_task(envelope.clone(), caller.clone(), preparation.clone())
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    assert_eq!(outcome.receipt().new_revision, Revision::new(2));
    assert_eq!(
        outcome.receipt().result_ref.as_deref(),
        Some(format!("invocation:{}", preparation.invocation_id).as_str())
    );
    envelope.request_id = RequestId::from_u128(0xDEF);
    let replay = fixture
        .store
        .assign_task(envelope, caller, preparation.clone())
        .unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(outcome.receipt(), replay.receipt());

    fixture
        .store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap();
    let pool_id = fixture.store.global_execution_pool_id().unwrap();
    let snapshot = fixture.store.execution_admission_snapshot(pool_id).unwrap();
    assert_eq!(snapshot.queued.len(), 1);
    assert_eq!(
        snapshot.queued.first().unwrap().unit,
        AdmissionUnit::Invocation(preparation.invocation_id)
    );
    assert!(snapshot.active.is_empty());
    fixture.store.read_snapshot(|conn, _| {
        let lineage: (String, String, String, String, String, String) = conn.query_row(
            "SELECT i.task_id, i.run_id, i.step_id, i.effective_bundle_hash, t.state, s.state
             FROM invocations i JOIN tasks t ON t.task_id=i.task_id
             JOIN step_instances s ON s.step_id=i.step_id
             WHERE i.invocation_id=?1 AND i.state='queued'",
            [preparation.invocation_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?;
        assert_eq!(lineage, (
            fixture.plan.task_id.to_string(), fixture.run.to_string(), fixture.plan.step_id.to_string(),
            preparation.effective_bundle_hash.to_string(), "queued".to_owned(), "queued".to_owned(),
        ));
        let committed: (i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM invocations WHERE task_id=?1),
                (SELECT COUNT(*) FROM events WHERE causation_receipt_id=?2
                    AND event_type IN ('TaskAssigned', 'InvocationQueued')),
                (SELECT COUNT(*) FROM object_references WHERE owner_kind='invocation' AND owner_id=?3 AND state='live')",
            [fixture.plan.task_id.to_string(), outcome.receipt().receipt_id.to_string(), preparation.invocation_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(committed, (1, 2, 2));
        Ok(())
    }).unwrap();
}
