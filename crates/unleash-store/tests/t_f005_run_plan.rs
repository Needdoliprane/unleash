//! REQ-WF-001: an exact plan for the prepared run.

#![expect(
    clippy::unwrap_used,
    reason = "Isolated Store fixture and contract assertions"
)]

#[path = "support/workflow_run.rs"]
mod workflow_run;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};
use unleash_domain::{ActorId, AgentLevel, Revision};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    CallerAuthority, CommitOutcome, CutpointObserver, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use workflow_run::{Fixture, PUBLISHED, RUN};

const OWNER: &str = "user:workflow-fixture";

#[derive(Default)]
struct InterruptCommit(AtomicBool);

impl CutpointObserver for InterruptCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "fixture interrupts prepared run".to_owned(),
            });
        }
        Ok(())
    }
}

fn prepare(config: StoreConfig) -> (Fixture, CommandEnvelope, CallerAuthority) {
    workflow_run::prepare(
        config,
        ActorId::new(OWNER).unwrap(),
        AgentLevel::N,
        |_, _, _| {},
    )
}

fn effects(store: &UnleashStore) -> (i64, i64, i64, i64) {
    store.read_snapshot(|db,_| {
        Ok(db.query_row(
            "SELECT (SELECT count(*) FROM workflow_runs WHERE run_id=?1),
                    (SELECT count(*) FROM object_references WHERE owner_kind='workflow_run' AND owner_id=?1 AND state='live'),
                    (SELECT count(*) FROM command_receipts WHERE aggregate_id=?1 AND command_type='CreateRun'),
                    (SELECT count(*) FROM invocations WHERE run_id=?1)",
            [RUN.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        )?)
    }).unwrap()
}

#[test]
fn t_f005_run_preparation_pins_the_resolved_manifest_before_start() {
    let directory = local_test_dir("f005-run-plan").unwrap();
    let config = StoreConfig::new(directory.path());
    let (fixture, command, caller) = prepare(config.clone());
    let first = fixture
        .store
        .create_prepared_workflow_run(command.clone(), caller.clone())
        .unwrap();
    assert!(matches!(first, CommitOutcome::Committed(_)));
    assert_eq!(first.receipt().new_revision, Revision::new(1));
    let (hash, state): (String, String) = fixture
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT effective_bundle_hash,state FROM workflow_runs WHERE run_id=?1",
                [RUN.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(state, "prepared");
    let bytes = fixture
        .store
        .read_verified_object_bytes(hash.parse().unwrap())
        .unwrap();
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        manifest.pointer("/workflow_version_id"),
        Some(&json!(PUBLISHED))
    );
    assert_eq!(
        manifest.pointer("/scopes/implementation/scope_revision"),
        Some(&json!(7))
    );
    assert_eq!(
        manifest.pointer("/bindings/developer/role/registry_version_id"),
        Some(&json!("role-v1"))
    );
    assert_eq!(
        manifest.pointer("/bindings/developer/entry_skill/registry_version_id"),
        Some(&json!("skill-v1"))
    );
    assert_eq!(
        manifest.pointer("/bindings/developer/contracts/0/registry_version_id"),
        Some(&json!("contract-v1"))
    );
    assert_eq!(
        manifest.pointer("/bindings/future/state"),
        Some(&json!("unbound"))
    );
    let proof = effects(&fixture.store);
    assert_eq!(proof, (1, 6, 1, 0));
    fixture.store.shutdown().unwrap();
    let reopened = UnleashStore::open(config).unwrap();
    let replay = reopened
        .create_prepared_workflow_run(command, caller)
        .unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(first.receipt(), replay.receipt());
    assert_eq!(effects(&reopened), proof);
    let restored_bytes = reopened
        .read_verified_object_bytes(hash.parse().unwrap())
        .unwrap();
    assert_eq!(restored_bytes, bytes);
    reopened.shutdown().unwrap();
}

#[test]
fn t_f005_run_preparation_requires_the_mandate_and_current_publication() {
    let directory = local_test_dir("f005-run-plan-inputs").unwrap();
    let (fixture, command, caller) = prepare(StoreConfig::new(directory.path()));
    let mut wrong_scope = command.clone();
    wrong_scope
        .payload
        .insert("mission_revision".to_owned(), 2.into());
    assert!(
        fixture
            .store
            .create_prepared_workflow_run(wrong_scope, caller.clone())
            .is_err()
    );
    assert_eq!(effects(&fixture.store), (0, 0, 0, 0));
    fixture.store.execute_in_writer_tx(WriteCriticality::Normal, |tx,_| {
        tx.execute("UPDATE registry_versions SET publication_state='revoked' WHERE registry_version_id='skill-v1'",[])?;
        Ok(())
    }).unwrap();
    assert!(
        fixture
            .store
            .create_prepared_workflow_run(command, caller)
            .is_err()
    );
    assert_eq!(effects(&fixture.store), (0, 0, 0, 0));
    fixture.store.shutdown().unwrap();
}

#[test]
fn t_f005_run_preparation_rolls_back_the_run_and_its_manifest_together() {
    let directory = local_test_dir("f005-run-plan-c1").unwrap();
    let stopper = Arc::new(InterruptCommit::default());
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = stopper.clone();
    let (fixture, command, caller) = prepare(config);
    stopper.0.store(true, Ordering::Release);
    assert!(matches!(
        fixture
            .store
            .create_prepared_workflow_run(command.clone(), caller.clone()),
        Err(StoreError::CutpointInjected { .. }),
    ));
    assert_eq!(effects(&fixture.store), (0, 0, 0, 0));
    fixture
        .store
        .create_prepared_workflow_run(command, caller)
        .unwrap();
    assert_eq!(effects(&fixture.store), (1, 6, 1, 0));
    fixture.store.shutdown().unwrap();
}

#[test]
fn t_f005_activation_uses_the_scope_and_contract_pinned_to_its_slot() {
    use unleash_domain::RunState;
    use unleash_protocol::ReceiptState;
    use unleash_store::{AggregateKind, CommandMutation, CommitCommandRequest};

    let directory = local_test_dir("f005-run-plan-activation").unwrap();
    let (fixture, command, caller) = prepare(StoreConfig::new(directory.path()));
    fixture
        .store
        .create_prepared_workflow_run(command, caller.clone())
        .unwrap();

    // The StartRun service has its own acceptance. This test establishes
    // a running run through the existing internal state writer.
    fixture
        .store
        .commit_command(CommitCommandRequest {
            actor_id: caller.actor_id,
            envelope: fixture.envelope(23, RUN.as_uuid(), Revision::new(1), "FixtureTransitionRun"),
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::TransitionWorkflowRun {
                run_id: RUN,
                target_state: RunState::Running,
                ended_at_ms: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    fixture.seed_activation_budget(RUN, 100);

    let (definition, plan_hash): (String, String) = fixture
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT v.content_hash,r.effective_bundle_hash FROM workflow_runs r
             JOIN workflow_versions v ON v.workflow_version_id=r.workflow_version_id
             WHERE r.run_id=?1",
                [RUN.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    let mut plan = fixture.plan.clone();
    plan.definition_hash = definition.parse().unwrap();
    plan.effective_bundle_hash = plan_hash.parse().unwrap();
    plan.contract_version_id = "contract-v1".to_owned();
    plan.scope_id = "fixture.restricted".to_owned();
    plan.scope_revision = Revision::new(7);

    let activation = |plan: &unleash_store::WorkflowTaskActivationPlan| {
        let mut envelope =
            fixture.envelope(24, RUN.as_uuid(), Revision::new(2), "ActivateWorkflowTask");
        envelope.payload = serde_json::from_value(serde_json::to_value(plan).unwrap()).unwrap();
        envelope
    };
    let mut wrong_scope = plan.clone();
    wrong_scope.scope_revision = Revision::new(8);
    assert!(
        fixture
            .store
            .activate_workflow_task(activation(&wrong_scope))
            .is_err()
    );
    let mut wrong_contract = plan.clone();
    wrong_contract.contract_version_id = "other-contract".to_owned();
    assert!(
        fixture
            .store
            .activate_workflow_task(activation(&wrong_contract))
            .is_err()
    );
    let api = fixture
        .store
        .activate_workflow_task(activation(&plan))
        .unwrap();
    assert_eq!(api.receipt().new_revision, Revision::new(3));
    let task: (String, i64, String, String) = fixture
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
            "SELECT scope_id,scope_revision,contract_version_id,state FROM tasks WHERE run_id=?1",
            [RUN.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        )?)
        })
        .unwrap();
    assert_eq!(
        task,
        (
            "fixture.restricted".to_owned(),
            7,
            "contract-v1".to_owned(),
            "ready".to_owned()
        )
    );
    fixture.store.shutdown().unwrap();
}

#[test]
fn t_f005_004_debits_later_tasks_once_and_refuses_exhaustion_after_reopen() {
    use unleash_domain::{StepId, TaskId};

    let directory = local_test_dir("f005-budget-continuation").unwrap();
    let (fixture, create, caller) = workflow_run::prepare(
        StoreConfig::new(directory.path()),
        ActorId::new(OWNER).unwrap(),
        AgentLevel::N,
        |workflow, scope, project| {
            *scope.pointer_mut("/scopes/implementation").unwrap() = json!({
                "scope_id": project,
                "scope_revision": 1,
            });
            let mut first = workflow.get_mut("body").unwrap().take();
            *first.get_mut("max_attempts").unwrap() = json!(1);
            let mut second = first.clone();
            *second.get_mut("id").unwrap() = json!("second");
            let mut third = first.clone();
            *third.get_mut("id").unwrap() = json!("third");
            *workflow.get_mut("body").unwrap() = json!({
                "id": "delivery", "kind": "sequence", "steps": [first, second, third],
            });
            *workflow
                .pointer_mut("/budget/max_step_activations")
                .unwrap() = json!(3);
        },
    );
    let project = create.project_id.unwrap();
    fixture
        .store
        .create_prepared_workflow_run(create, caller.clone())
        .unwrap();
    fixture
        .store
        .start_prepared_workflow_run(
            fixture.envelope(23, RUN.as_uuid(), Revision::new(1), "StartRun"),
            caller,
        )
        .unwrap();

    let mut plan = fixture.plan.clone();
    (plan.definition_hash, plan.effective_bundle_hash) = fixture
        .store
        .read_snapshot(|db, _| {
            let (definition, bundle): (String, String) = db.query_row(
                "SELECT v.content_hash,r.effective_bundle_hash FROM workflow_runs r
               JOIN workflow_versions v USING(workflow_version_id) WHERE r.run_id=?1",
                [RUN.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((definition.parse().unwrap(), bundle.parse().unwrap()))
        })
        .unwrap();
    plan.scope_id = project.to_string();
    plan.scope_revision = Revision::new(1);
    plan.contract_version_id = "contract-v1".to_owned();
    plan.definition_node_id = "second".to_owned();
    plan.step_id = StepId::from_u128(0x5933);
    plan.task_id = TaskId::from_u128(0x5934);
    let mut second = fixture.envelope(24, RUN.as_uuid(), Revision::new(2), "ActivateWorkflowTask");
    second.payload = serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
    let receipt = fixture
        .store
        .activate_workflow_task(second.clone())
        .unwrap();
    assert_eq!(receipt.receipt().new_revision, Revision::new(3));

    let mut third = fixture.envelope(25, RUN.as_uuid(), Revision::new(3), "ActivateWorkflowTask");
    let store = fixture.store.restart_same_lineage().unwrap();
    let replay = store.activate_workflow_task(second).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(receipt.receipt(), replay.receipt());

    plan.daemon_epoch = store.daemon_state().unwrap().daemon_epoch;
    plan.definition_node_id = "third".to_owned();
    plan.step_id = StepId::from_u128(0x5935);
    plan.task_id = TaskId::from_u128(0x5936);
    third.payload = serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
    assert!(matches!(
        store.activate_workflow_task(third),
        Err(StoreError::WorkflowActivationBudgetExhausted)
    ));
    let state: (i64, i64, i64, i64, i64) = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT revision,max_step_activations,step_activations_used,
                    (SELECT count(*) FROM tasks WHERE run_id=?1),
                    (SELECT count(*) FROM command_receipts WHERE aggregate_id=?1
                        AND command_type='ActivateWorkflowTask')
             FROM workflow_runs WHERE run_id=?1",
                [RUN.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?)
        })
        .unwrap();
    assert_eq!(state, (3, 3, 3, 2, 1));
    store.shutdown().unwrap();
}

#[test]
fn t_f002_005_preserves_legacy_run_and_refuses_activation_with_unknown_budget() {
    use unleash_store::{BackupRetentionTier, PrepareBackupRequest};

    let directory = local_test_dir("f005-budget-v019").unwrap();
    let mut previous = StoreConfig::new(directory.path());
    previous.max_supported_schema_version = 19;
    let actor = ActorId::new(OWNER).unwrap();
    let fixture = Fixture::on_store(UnleashStore::open(previous).unwrap(), OWNER, actor, true);
    let run = fixture.run;
    let plan = fixture.activation(26, 2, &fixture.plan);
    let backup = fixture
        .store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: directory.path().join("v019-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    fixture.store.shutdown().unwrap();

    let mut config = StoreConfig::new(directory.path());
    config.max_supported_schema_version = 20;
    let store = UnleashStore::open_with_migration_backup(config, Some(backup.backup_id)).unwrap();
    assert_eq!(store.daemon_state().unwrap().schema_version, 20);
    let inspect = || {
        store
            .read_snapshot(|db, _| {
                Ok(db.query_row(
                    "SELECT state,revision,max_step_activations,step_activations_used
               FROM workflow_runs WHERE run_id=?1",
                    [run.to_string()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )?)
            })
            .unwrap()
    };
    assert_eq!(inspect(), ("running".to_owned(), 2, None, None));
    assert!(matches!(
        store.activate_workflow_task(plan),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(inspect(), ("running".to_owned(), 2, None, None));
    store.shutdown().unwrap();
}
