//! REQ-WF-001: an exact plan for the prepared run.

#![expect(
    clippy::unwrap_used,
    reason = "Isolated Store fixture and contract assertions"
)]

#[expect(
    dead_code,
    reason = "The workflow fixture also serves other integration suites"
)]
#[path = "support/workflow_task.rs"]
mod workflow_task;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};
use unleash_domain::{ActorId, Revision, RunId, Sha256Digest};
use unleash_protocol::{CommandEnvelope, canonicalize_to_vec};
use unleash_store::{
    CallerAuthority, CallerRole, CommitOutcome, CutpointObserver, StoreConfig, StoreCutpoint,
    StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use workflow_task::Fixture;

const RUN: RunId = RunId::from_u128(0x5901);
const PUBLISHED: &str = "fixture.prepared-workflow:v1";
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
    let store = UnleashStore::open(config).unwrap();
    let actor = ActorId::new(OWNER).unwrap();
    let fixture = Fixture::on_store(store, OWNER, actor.clone(), true);
    let mut command = fixture.envelope(21, RUN.as_uuid(), Revision::INITIAL, "CreateRun");
    let project = command.project_id.unwrap();
    let mut workflow: Value = serde_json::from_str(include_str!(
        "../../../.claude/examples/merge-only-workflow.json"
    ))
    .unwrap();
    let task = workflow.pointer("/body/steps/0").unwrap().clone();
    let workflow_fields = workflow.as_object_mut().unwrap();
    workflow_fields.insert("bindings".to_owned(), json!({
        "developer": {"mode":"fixed","role":{"id":"role.fixture","version":1}},
        "future": {"mode":"late","level":"N","capability_ceiling":[],"contract_profile":{"id":"contract.fixture","version":1}}
    }));
    workflow_fields.insert("body".to_owned(), task);
    let definition = canonicalize_to_vec(&workflow).unwrap();
    let definition_hash = Sha256Digest::from_bytes(&definition);
    fixture.retain_object(&definition, "workflow_version", PUBLISHED, "content_hash");
    fixture
        .store
        .register_workflow_version(PUBLISHED, "published", definition_hash)
        .unwrap();

    let scope_document = json!({
        "schema_version": 1,
        "mission_id": fixture.mission,
        "mission_revision": 1,
        "scopes": {"implementation": {"scope_id":"fixture.restricted", "scope_revision":7}},
    });
    let scope_bytes = canonicalize_to_vec(&scope_document).unwrap();
    let scope_hash = Sha256Digest::from_bytes(&scope_bytes);
    fixture.retain_object(
        &scope_bytes,
        "mission",
        &fixture.mission.to_string(),
        "scope_manifest:1",
    );

    let role_scope = json!({"kind": "project", "owner_id": project});
    let skill_ref = json!({"id": "skill.fixture", "version": 1, "scope": role_scope});
    let contract_ref = json!({"id": "contract.fixture", "version": 1, "scope": role_scope});
    publish_fixture_version(
        &fixture,
        "role",
        "role.fixture",
        "role-v1",
        &json!({
            "schema_version": "unleash.role.v1", "role_id": "role.fixture", "version": 1,
            "scope": role_scope, "entry_skill": skill_ref, "skills": [skill_ref], "contracts": [contract_ref],
        }),
    );
    publish_fixture_version(
        &fixture,
        "skill",
        "skill.fixture",
        "skill-v1",
        &json!({
            "schema_version": "unleash.skill.v1", "skill_id": "skill.fixture", "version": 1,
            "scope": role_scope, "contract_profile": contract_ref,
        }),
    );
    publish_fixture_version(
        &fixture,
        "contract",
        "contract.fixture",
        "contract-v1",
        &json!({
            "schema_version": "unleash.contract-profile.v1", "profile_id": "contract.fixture",
            "version": 1, "scope": role_scope,
        }),
    );
    command.payload = serde_json::from_value(json!({
        "mission_id": fixture.mission,
        "mission_revision": 1,
        "workflow_version_id": PUBLISHED,
        "scope_manifest_hash": scope_hash,
        "predecessor_run_id": null,
        "selection_reason": "selected exact fixture version",
    }))
    .unwrap();
    let caller = CallerAuthority {
        actor_id: actor,
        role: CallerRole::HumanOperator,
    };
    (fixture, command, caller)
}

fn publish_fixture_version(
    fixture: &Fixture,
    kind: &str,
    entry: &str,
    version: &str,
    document: &Value,
) {
    let bytes = canonicalize_to_vec(document).unwrap();
    let digest = Sha256Digest::from_bytes(&bytes);
    fixture.retain_object(&bytes, "registry_version", version, "content_hash");
    let lineage = fixture.store.active_authority_generation().unwrap();
    let project = fixture
        .envelope(22, RUN.as_uuid(), Revision::INITIAL, "CreateRun")
        .project_id
        .unwrap();
    let (kind, entry, version) = (kind.to_owned(), entry.to_owned(), version.to_owned());
    fixture.store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, time| {
        let discovery = format!("{version}-discovery");
        let evaluation = format!("{version}-evaluation");
        tx.execute(
            "INSERT INTO discovery_receipts
              (discovery_receipt_id,authority_generation,decision,content_hash,created_at_ms)
             VALUES (?1,?2,'reuse',?3,?4)",
            rusqlite::params![discovery,lineage.to_string(),digest.to_string(),time.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO evaluation_reports
              (evaluation_report_id,subject_kind,subject_id,required_checks,judge_policy,result,
               content_hash,created_at_ms)
             VALUES (?1,?2,?3,'[]','fixture','passed',?4,?5)",
            rusqlite::params![evaluation,kind,version,digest.to_string(),time.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO registry_entries (entry_kind,scope_kind,scope_owner_key,entry_id,state,revision)
             VALUES (?1,'project',?2,?3,'active',1)",
            rusqlite::params![kind,project.to_string(),entry],
        )?;
        tx.execute(
            "INSERT INTO registry_versions
              (registry_version_id,entry_kind,scope_kind,scope_owner_key,entry_id,version,
               schema_version,content_hash,publication_state,author_principal_id,
               discovery_receipt_id,evaluation_report_id,published_at_ms)
             VALUES (?1,?2,'project',?3,?4,1,1,?5,'published',?6,?7,?8,?9)",
            rusqlite::params![version,kind,project.to_string(),entry,digest.to_string(),OWNER,
                              discovery,evaluation,time.as_i64()],
        )?;
        Ok(())
    }).unwrap();
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
            envelope: fixture.envelope(23, RUN.as_uuid(), Revision::new(1), "StartRun"),
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
