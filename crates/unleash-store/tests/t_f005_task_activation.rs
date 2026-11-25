//! Sous-cas T-F005-001/009 : matérialisation locale d'une activation publiée.
#![expect(
    clippy::unwrap_used,
    reason = "Scénario de persistance dans un Store jetable"
)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[path = "support/workflow_task.rs"]
mod workflow_task;

use serde_json::Value;
use unleash_domain::{ActorId, RequestId, Revision, Sha256Digest, StepId, TaskId};
use unleash_protocol::{ErrorCode, ReasonCode, RetryClass, canonicalize_to_vec};
use unleash_store::{
    AdmissionUnit, CommitOutcome, CutpointObserver, ExecutionPoolParams, InitialContextPlan,
    PreparedTaskAssignment, StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;
use workflow_task::{Fixture, VERSION, initial_context_fixture};

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

impl Fixture {
    fn new(retain_definition: bool) -> (impl Drop, Self, Arc<ActivationCommitCutpoint>) {
        let directory = local_test_dir("f005-task-activation").unwrap();
        let cutpoint = Arc::new(ActivationCommitCutpoint::default());
        let mut config = StoreConfig::new(directory.path());
        config.cutpoints = cutpoint.clone();
        let store = UnleashStore::open(config).unwrap();
        let actor = ActorId::new("user:fixture-owner").unwrap();
        (
            directory,
            Self::on_store(store, "fixture-owner", actor, retain_definition),
            cutpoint,
        )
    }

    /// Même run en exécution, avec une définition de workflow adaptée par `customize`.
    fn with_definition(
        retain_definition: bool,
        customize: impl FnOnce(&mut Value),
    ) -> (impl Drop, Self, Arc<ActivationCommitCutpoint>) {
        let directory = local_test_dir("f005-task-activation").unwrap();
        let cutpoint = Arc::new(ActivationCommitCutpoint::default());
        let mut config = StoreConfig::new(directory.path());
        config.cutpoints = cutpoint.clone();
        let store = UnleashStore::open(config).unwrap();
        let actor = ActorId::new("user:fixture-owner").unwrap();
        (
            directory,
            Self::on_store_with_definition(
                store,
                "fixture-owner",
                actor,
                retain_definition,
                customize,
            ),
            cutpoint,
        )
    }

    fn bundle_document(&self, hash: Sha256Digest) -> Value {
        serde_json::from_slice(&self.store.read_verified_object_bytes(hash).unwrap()).unwrap()
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
}

#[test]
fn commits_ready_task_and_reuses_receipt_on_replay() {
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(false);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, cutpoint) = Fixture::new(true);
    let request = fixture.activation(5, 2, &fixture.plan);
    cutpoint.0.store(true, Ordering::SeqCst);
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

/// A mechanical action before the agent tasks: `develop` is reached further in
/// the run, it is not the entry leaf.
fn mechanical_entry_then_tasks(definition: &mut Value, model_policy: &str) {
    let delivery = definition.get_mut("body").unwrap().take();
    let fields = definition.as_object_mut().unwrap();
    fields.insert(
        "schema_version".to_owned(),
        serde_json::json!("unleash.workflow.v1.1"),
    );
    // An inconsistent version imported before semantic validation.
    fields.insert("model_policy".to_owned(), serde_json::json!(model_policy));
    fields.insert(
        "body".to_owned(),
        serde_json::json!({"id": "intake", "kind": "sequence", "steps": [
            {"id": "collect", "kind": "action",
             "operation": {"id": "demo.collect", "version": 1},
             "contract_profile": {"id": "demo.contract", "version": 1},
             "scope_key": "implementation", "inputs": {},
             "timeout_seconds": 60, "max_attempts": 1, "retry_policy": "never"},
            delivery,
        ]}),
    );
}

// T-F012-018 / AC-F012-018 / REQ-MODEL-018 — la règle vaut pour toute activation
// d'une tâche d'agent du run, pas seulement de celle de l'entrée.
#[test]
fn t_f012_018_forbidden_run_refuses_a_later_agent_task_activation() {
    let (_directory, fixture, _cutpoint) = Fixture::with_definition(true, |definition| {
        mechanical_entry_then_tasks(definition, "forbidden");
    });
    let error = fixture
        .store
        .activate_workflow_task(fixture.activation(5, 2, &fixture.plan))
        .expect_err("a forbidden run cannot activate an agent task");
    assert!(matches!(
        error,
        StoreError::CapabilityDenied {
            reason: ReasonCode::ModelCallsForbidden,
            ..
        }
    ));
    assert_eq!(error.error_code(), ErrorCode::CapabilityDenied);
    assert_eq!(error.retry_class(), RetryClass::Never);
    fixture.assert_unmaterialized();
    let debited: i64 = fixture
        .store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT step_activations_used FROM workflow_runs WHERE run_id=?1",
                [fixture.run.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(debited, 0);
}

// T-F012-018 / AC-F012-018 / REQ-MODEL-018 — seule `forbidden` restreint : une
// politique déclarée `allowed_bounded` laisse les tâches d'agent s'activer.
#[test]
fn t_f012_018_bounded_policy_still_activates_agent_tasks() {
    let (_directory, fixture, _cutpoint) = Fixture::with_definition(true, |definition| {
        mechanical_entry_then_tasks(definition, "allowed_bounded");
    });
    let activated = fixture
        .store
        .activate_workflow_task(fixture.activation(5, 2, &fixture.plan));
    assert!(
        matches!(activated, Ok(CommitOutcome::Committed(_))),
        "{activated:?}"
    );
}

#[test]
fn publishes_initial_context_once_with_its_ordered_input() {
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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
    let (_directory, fixture, cutpoint) = Fixture::new(true);
    let (envelope, caller, preparation) = fixture.prepared_assignment();
    cutpoint.0.store(true, Ordering::SeqCst);
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
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
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

#[test]
fn queues_with_the_published_bundle_without_a_client_selector() {
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
    let (mut envelope, caller, preparation) = fixture.prepared_assignment();
    assert_eq!(envelope.payload.len(), 1);
    let published = fixture
        .store
        .assign_published_task(envelope.clone(), caller.clone())
        .unwrap();
    assert!(matches!(published, CommitOutcome::Committed(_)));
    envelope.request_id = RequestId::from_u128(0x9091);
    let replay = fixture
        .store
        .assign_published_task(envelope, caller)
        .unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(published.receipt(), replay.receipt());

    fixture
        .store
        .read_snapshot(|conn, _| {
            let queued: (i64, String, i64) = conn.query_row(
                "SELECT COUNT(*),i.effective_bundle_hash,
                (SELECT COUNT(*) FROM object_references
                 WHERE owner_kind='task' AND owner_id=i.task_id
                   AND object_hash=i.effective_bundle_hash AND state='live')
             FROM invocations i WHERE i.task_id=?1 AND i.state='queued'
             GROUP BY i.task_id,i.effective_bundle_hash",
                [fixture.plan.task_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(
                queued,
                (1, preparation.effective_bundle_hash.to_string(), 1)
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn keeps_one_bundle_for_the_same_durable_selection() {
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
    let (envelope, caller, preparation) = fixture.prepared_assignment();
    let mut alternate = fixture.bundle_document(preparation.effective_bundle_hash);
    assert_eq!(
        fixture
            .store
            .publish_effective_task_bundle(
                envelope.clone(),
                caller.clone(),
                preparation.clone(),
                &alternate,
            )
            .unwrap(),
        preparation.effective_bundle_hash
    );
    alternate.as_object_mut().unwrap().insert(
        "selection_reason".to_owned(),
        serde_json::json!("a different resolved document"),
    );
    let another = PreparedTaskAssignment {
        effective_bundle_hash: Sha256Digest::from_bytes(&canonicalize_to_vec(&alternate).unwrap()),
        ..preparation.clone()
    };
    assert!(matches!(
        fixture
            .store
            .publish_effective_task_bundle(envelope, caller, another, &alternate),
        Err(StoreError::InvalidArgument { .. })
    ));
    fixture.assert_assignment_pending();
    fixture
        .store
        .read_snapshot(|conn, _| {
            let (count, hash): (i64, String) = conn.query_row(
                "SELECT COUNT(*),MIN(bundle_hash) FROM effective_bundles",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(
                (count, hash),
                (1, preparation.effective_bundle_hash.to_string())
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn resolves_a_new_bundle_after_the_session_revision_changes() {
    let (_directory, fixture, _cutpoint) = Fixture::new(true);
    let (envelope, caller, first) = fixture.prepared_assignment();
    let mut document = fixture.bundle_document(first.effective_bundle_hash);
    let session = document
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let changed_session = session.clone();
    fixture
        .store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute(
                "UPDATE sessions SET revision=1 WHERE session_id=?1",
                [changed_session],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        fixture
            .store
            .assign_published_task(envelope.clone(), caller.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    fixture.assert_assignment_pending();

    let (mut context_plan, mut context) = initial_context_fixture(&fixture.store, &session, &[]);
    context_plan.manifest_id = Uuid::from_u128(0x9095);
    context
        .as_object_mut()
        .unwrap()
        .insert("base_session_revision".to_owned(), serde_json::json!(1));
    fixture
        .store
        .publish_initial_context(context_plan, &context, vec![])
        .unwrap();
    document.as_object_mut().unwrap().insert(
        "initial_context_manifest_id".to_owned(),
        serde_json::json!(context_plan.manifest_id),
    );
    let replacement = PreparedTaskAssignment {
        effective_bundle_hash: Sha256Digest::from_bytes(&canonicalize_to_vec(&document).unwrap()),
        ..first.clone()
    };
    fixture
        .store
        .publish_effective_task_bundle(
            envelope.clone(),
            caller.clone(),
            replacement.clone(),
            &document,
        )
        .unwrap();
    fixture
        .store
        .assign_published_task(envelope, caller)
        .unwrap();
    fixture
        .store
        .read_snapshot(|conn, _| {
            let hash: String = conn.query_row(
                "SELECT effective_bundle_hash FROM invocations WHERE task_id=?1",
                [fixture.plan.task_id.to_string()],
                |row| row.get(0),
            )?;
            assert_eq!(hash, replacement.effective_bundle_hash.to_string());
            Ok(())
        })
        .unwrap();
    fixture
        .store
        .read_verified_object_bytes(first.effective_bundle_hash)
        .unwrap();
}
