//! Shared preparation of one workflow task for the Store and local daemon tests.

use serde_json::Value;
use unleash_domain::{
    ActorId, AgentId, CommandId, CompletionPolicy, InvocationId, MissionId, MissionState,
    ProjectId, RequestId, Revision, RunId, RunState, Sha256Digest, StepId, TaskId,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonicalize_to_vec};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CommitOutcome, InitialContextPlan, PrepareObjectRequest, PreparedAgentCreation,
    PreparedTaskAssignment, PublishObjectReferenceRequest, UnleashStore,
    WorkflowTaskActivationPlan, WriteCriticality, objects::PublishedObjectReference,
};
use uuid::Uuid;

pub(crate) const VERSION: &str = "workflow.fixture.m2-task:v1";
pub(crate) const ROLE: &str = "fixture-role:v1";
const ASSIGNMENT: Uuid = Uuid::from_u128(0x500a);

pub(crate) struct Fixture {
    pub(crate) store: UnleashStore,
    actor: ActorId,
    principal: String,
    project: ProjectId,
    pub(crate) mission: MissionId,
    pub(crate) run: RunId,
    pub(crate) plan: WorkflowTaskActivationPlan,
}

impl Fixture {
    pub(crate) fn on_store(
        store: UnleashStore,
        principal: &str,
        actor: ActorId,
        retain_definition: bool,
    ) -> Self {
        store
            .register_principal(principal, "user", actor.as_str())
            .unwrap();
        let definition: Value = serde_json::from_str(include_str!(
            "../../../../.claude/examples/merge-only-workflow.json"
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
            actor,
            principal: principal.to_owned(),
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
        if fixture.store.daemon_state().unwrap().schema_version >= 20 {
            let limit = definition
                .pointer("/budget/max_step_activations")
                .and_then(Value::as_i64)
                .unwrap();
            fixture.seed_activation_budget(fixture.run, limit);
        }
        fixture
    }

    pub(crate) fn seed_activation_budget(&self, run: RunId, limit: i64) {
        self.store
            .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
                tx.execute(
                    "UPDATE workflow_runs SET max_step_activations=?1,step_activations_used=0
                      WHERE run_id=?2",
                    rusqlite::params![limit, run.to_string()],
                )?;
                Ok(())
            })
            .unwrap();
    }

    pub(crate) fn retain_object(
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

    pub(crate) fn prepare_agent_creation(
        &self,
        command: u128,
        agent: AgentId,
        assignment: Uuid,
    ) -> (CommandEnvelope, CallerAuthority) {
        let bytes = br#"{"role":"fixture.developer","version":1}"#;
        let role_content_hash = Sha256Digest::from_bytes(bytes);
        self.retain_object(bytes, "registry_version", ROLE, "content_hash");
        let lineage = self.store.active_authority_generation().unwrap();
        let project = self.project;
        let principal = self.principal.clone();
        self.store
            .execute_in_writer_tx(WriteCriticality::Normal, move |tx, time| {
                tx.execute(
                    "INSERT INTO discovery_receipts
                        (discovery_receipt_id, authority_generation, decision, content_hash, created_at_ms)
                     VALUES ('fixture-discovery', ?1, 'reuse', ?2, ?3)",
                    rusqlite::params![lineage.to_string(), role_content_hash.to_string(), time.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO evaluation_reports
                        (evaluation_report_id, subject_kind, subject_id, required_checks, judge_policy,
                         result, content_hash, created_at_ms)
                     VALUES ('fixture-evaluation', 'role', ?1, '[]', 'fixture', 'passed', ?2, ?3)",
                    rusqlite::params![ROLE, role_content_hash.to_string(), time.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO registry_entries
                        (entry_kind, scope_kind, scope_owner_key, entry_id, state, revision)
                     VALUES ('role', 'project', ?1, 'fixture.developer', 'active', 1)",
                    [project.to_string()],
                )?;
                tx.execute(
                    "INSERT INTO registry_versions
                        (registry_version_id, entry_kind, scope_kind, scope_owner_key, entry_id,
                         version, schema_version, content_hash, publication_state, author_principal_id,
                         discovery_receipt_id, evaluation_report_id, published_at_ms)
                     VALUES (?1, 'role', 'project', ?2, 'fixture.developer', 1, 1, ?3, 'published', ?4,
                             'fixture-discovery', 'fixture-evaluation', ?5)",
                    rusqlite::params![
                        ROLE,
                        project.to_string(),
                        role_content_hash.to_string(),
                        principal,
                        time.as_i64()
                    ],
                )?;
                Ok(())
            })
            .unwrap();
        let plan = PreparedAgentCreation {
            daemon_epoch: self.plan.daemon_epoch,
            assignment_id: assignment,
            mission_id: self.mission,
            mission_revision: Revision::new(1),
            task_id: self.plan.task_id,
            task_revision: Revision::new(1),
            scope_id: self.plan.scope_id.clone(),
            scope_revision: self.plan.scope_revision,
            role_version_id: ROLE.to_owned(),
            role_content_hash,
        };
        let mut envelope =
            self.envelope(command, agent.as_uuid(), Revision::INITIAL, "CreateAgent");
        envelope.payload = serde_json::from_value(serde_json::to_value(plan).unwrap()).unwrap();
        let caller = CallerAuthority {
            actor_id: self.actor.clone(),
            role: CallerRole::HumanOperator,
        };
        (envelope, caller)
    }

    pub(crate) fn activate_with_assignee(&self) -> (AgentId, String, String) {
        self.store
            .activate_workflow_task(self.activation(5, 2, &self.plan))
            .unwrap();
        let agent = AgentId::from_u128(0x5008);
        let (envelope, caller) = self.prepare_agent_creation(7, agent, ASSIGNMENT);
        self.store.create_assigned_agent(envelope, caller).unwrap();
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

    pub(crate) fn prepared_assignment(
        &self,
    ) -> (CommandEnvelope, CallerAuthority, PreparedTaskAssignment) {
        let (agent, session_id, trigger_event_id) = self.activate_with_assignee();
        let (context_plan, context) = initial_context_fixture(&self.store, &session_id, &[]);
        let manifest = context_plan.manifest_id.to_string();
        let policy_hash = Sha256Digest::from_bytes(b"policy defined for the assignment fixture");
        self.store
            .publish_initial_context(context_plan, &context, vec![])
            .unwrap();
        let bundle = serde_json::json!({
            "schema_version": 1,
            "agent_id": agent,
            "session_id": session_id,
            "trigger_event_id": trigger_event_id,
            "mission_id": self.mission,
            "run_id": self.run,
            "step_id": self.plan.step_id,
            "task_id": self.plan.task_id,
            "role_version_id": ROLE,
            "workflow_version_id": VERSION,
            "contract_version_ids": [&self.plan.contract_version_id],
            "requested_model_profile_version_id": "fixture-profile:v1",
            "allowed_model_policy_hash": policy_hash,
            "scope_id": self.plan.scope_id,
            "scope_revision": self.plan.scope_revision,
            "initial_context_manifest_id": manifest,
        });
        let bundle_hash = Sha256Digest::from_bytes(&canonicalize_to_vec(&bundle).unwrap());
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
            actor_id: self.actor.clone(),
            role: CallerRole::HumanOperator,
        };
        let preparation = PreparedTaskAssignment {
            daemon_epoch: self.plan.daemon_epoch,
            assignment_id: ASSIGNMENT,
            assignment_epoch: 1,
            assignment_revision: Revision::new(1),
            trigger_event_id: trigger_event_id.parse().unwrap(),
            effective_bundle_hash: bundle_hash,
            invocation_id: InvocationId::from_u128(0x500c),
        };
        self.store
            .publish_effective_task_bundle(
                envelope.clone(),
                caller.clone(),
                preparation.clone(),
                &bundle,
            )
            .unwrap();
        (envelope, caller, preparation)
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
                owner_principal_id: Some(self.principal.clone()),
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
            "FixtureTransitionRun",
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
        let actor = self.actor.clone();
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

    pub(crate) fn envelope(
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

    pub(crate) fn activation(
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

pub(crate) fn initial_context_fixture(
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
