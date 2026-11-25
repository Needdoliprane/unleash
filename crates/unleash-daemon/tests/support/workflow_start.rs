//! Initial progression of a prepared run through the local daemon.

use std::{os::unix::fs::PermissionsExt, path::Path};

use serde_json::{Map, Value, json};
use unleash_client::ClientError;
use unleash_daemon::DaemonConfig;
use unleash_domain::{AgentLevel, CommandId, ReceiptId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ErrorCode, ReasonCode, ReceiptState};
use unleash_store::{StoreConfig, UnleashStore};

use super::{
    RUN, local_store_config, publish_mandate, scoped_run_query, start, wire_command_with_workflow,
};

pub(super) async fn fixture(
    data: &Path,
    rendezvous: &Path,
    role_level: AgentLevel,
    customize: impl FnOnce(&mut Value) + Send + 'static,
) -> (StoreConfig, DaemonConfig, CommandEnvelope) {
    std::fs::set_permissions(rendezvous, PermissionsExt::from_mode(0o700)).unwrap();
    let store = local_store_config(data);
    let seed = store.clone();
    let (create, _) = tokio::task::spawn_blocking(move || {
        wire_command_with_workflow(seed, role_level, customize)
    })
    .await
    .unwrap();
    let config = DaemonConfig {
        data_root: Some(data.to_owned()),
        rendezvous_dir: Some(rendezvous.to_owned()),
        ..DaemonConfig::default()
    };
    (store, config, create)
}

pub(super) fn start_command(create: &CommandEnvelope) -> CommandEnvelope {
    CommandEnvelope {
        request_id: RequestId::from_u128(0x5920),
        command_id: CommandId::from_u128(0x5920),
        command_type: "StartRun".to_owned(),
        expected_revision: Revision::new(1),
        payload: Map::new(),
        ..create.clone()
    }
}

fn sequence_at_budget(workflow: &mut Value, activation_limit: u32) {
    let mut task = workflow.get_mut("body").unwrap().take();
    *task.get_mut("max_attempts").unwrap() = json!(1);
    *workflow.get_mut("body").unwrap() =
        json!({"id": "delivery", "kind": "sequence", "steps": [task]});
    *workflow
        .pointer_mut("/budget/max_step_activations")
        .unwrap() = json!(activation_limit);
}

#[tokio::test]
async fn t_f005_004_starts_at_exact_activation_budget_and_recovers_once() {
    let data = unleash_testkit::local_test_dir("f005-start-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |workflow| {
            sequence_at_budget(workflow, 2);
        })
        .await;
    let (daemon, mut client) = start(config.clone()).await;
    let initial_epoch = client.hello().daemon_epoch;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let query = scoped_run_query(&create);
    let prepared = client.query(&query).await.unwrap();
    let mut command = start_command(&create);
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.new_revision, Revision::new(2));
    assert_eq!(receipt.result_ref, Some(format!("workflow-run:{RUN}")));
    let running = client.query(&query).await.unwrap();
    assert_eq!(running.result.get("state"), Some(&json!("running")));
    for key in ["definition_hash", "effective_bundle_hash", "plan"] {
        assert_eq!(running.result.get(key), prepared.result.get(key), "{key}");
    }

    drop(client);
    daemon
        .shutdown_graceful("restart after StartRun")
        .await
        .unwrap();
    drop(daemon);
    let (daemon, mut client) = start(config).await;
    assert!(client.hello().daemon_epoch.as_u64() > initial_epoch.as_u64());
    command.request_id = RequestId::from_u128(0x5921);
    assert_eq!(client.send_command(&command).await.unwrap(), receipt);
    let mut second_start = command;
    second_start.request_id = RequestId::from_u128(0x5922);
    second_start.command_id = CommandId::from_u128(0x5922);
    let error = client.send_command(&second_start).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::RevisionConflict)
    );
    assert_eq!(client.query(&query).await.unwrap().result, running.result);

    drop(client);
    daemon
        .shutdown_graceful("inspect initial activation")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some((receipt.receipt_id, 2))))
        .await
        .unwrap();
}

fn revise_project(create: &CommandEnvelope) -> CommandEnvelope {
    CommandEnvelope {
        request_id: RequestId::from_u128(0x5930),
        command_id: CommandId::from_u128(0x5930),
        aggregate_id: create.project_id.unwrap().as_uuid(),
        command_type: "ReviseProject".to_owned(),
        expected_revision: Revision::new(1),
        payload: serde_json::from_value(json!({
            "name": "Revised project",
            "description": "A new scope revision for the local fixture",
            "weight": 1,
            "primary_root_path": null,
            "sources": [],
            "repositories": [],
            "settings_json": {}
        }))
        .unwrap(),
        ..create.clone()
    }
}

// T-F005-001 — Refus de démarrer avec une portée épinglée périmée.
#[tokio::test]
async fn refuses_a_stale_project_scope_without_allocating_work() {
    let data = unleash_testkit::local_test_dir("f005-stale-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-stale-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |_| {}).await;
    let (daemon, mut client) = start(config).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let revised = client.send_command(&revise_project(&create)).await.unwrap();
    assert_eq!(revised.new_revision, Revision::new(2));
    let error = client
        .send_command(&start_command(&create))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::RevisionConflict)
    );
    let snapshot = client.query(&scoped_run_query(&create)).await.unwrap();
    assert_eq!(snapshot.result.get("state"), Some(&json!("prepared")));
    assert_eq!(snapshot.result.get("revision"), Some(&json!(1)));
    drop(client);
    daemon
        .shutdown_graceful("inspect rejected start")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || assert_start_effects(store, None))
        .await
        .unwrap();
}

fn assert_start_effects(config: StoreConfig, receipt: Option<(ReceiptId, i64)>) {
    let store = UnleashStore::open(config).unwrap();
    let causal_receipt = receipt.map_or_else(String::new, |(id, _)| id.to_string());
    let effects: (i64, i64, i64, i64, i64) = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT
                    (SELECT COUNT(*) FROM command_receipts WHERE command_type='StartRun' AND aggregate_id=?1),
                    (SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND causation_receipt_id=?2
                       AND event_type IN ('RunStarted','NodeActivated')),
                    (SELECT COUNT(*) FROM tasks t JOIN step_instances s USING(step_id)
                       JOIN workflow_runs r ON r.run_id=t.run_id JOIN missions m ON m.mission_id=t.mission_id
                       WHERE t.run_id=?1 AND s.run_id=t.run_id AND s.definition_node_id='develop'
                         AND t.contract_version_id='contract-v1' AND t.scope_id=m.project_id
                         AND t.scope_revision=1 AND t.state='ready' AND s.state='ready' AND s.step_attempt=1),
                    (SELECT COUNT(*) FROM workflow_timers t JOIN workflow_runs r USING(run_id)
                       WHERE t.run_id=?1 AND t.state='armed'
                         AND ((t.timer_kind='run_deadline' AND t.step_id IS NULL AND t.due_at_ms-r.started_at_ms=14400000)
                              OR (t.timer_kind='step_timeout' AND t.step_id IS NOT NULL AND t.due_at_ms-r.started_at_ms=1800000))),
                    (SELECT COUNT(*) FROM actions WHERE run_id=?1)",
                rusqlite::params![RUN.to_string(), causal_receipt],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?)
        })
        .unwrap();
    assert_eq!(
        effects,
        if receipt.is_some() {
            (1, 2, 1, 2, 0)
        } else {
            (0, 0, 0, 0, 0)
        }
    );
    let charged: Option<i64> = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT step_activations_used FROM workflow_runs WHERE run_id=?1",
                [RUN.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(charged, receipt.map(|(_, activations)| activations));
    store.shutdown().unwrap();
}

struct InterruptStart(std::sync::atomic::AtomicBool);

impl unleash_store::CutpointObserver for InterruptStart {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), unleash_store::StoreError> {
        if point == unleash_store::StoreCutpoint::C1BeforeCommandCommit
            && self.0.swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(unleash_store::StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "fixture interrupts workflow start".to_owned(),
            });
        }
        Ok(())
    }
}

// T-F005-009 — Coupure avant commit et rejeu de l’ordre de démarrage.
#[tokio::test]
async fn rolls_back_the_start_transaction_and_retries_the_same_command() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let data = unleash_testkit::local_test_dir("f005-start-c1-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-c1-rdv").unwrap();
    let (store, mut config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |_| {}).await;
    let interrupt = Arc::new(InterruptStart(AtomicBool::new(false)));
    config.store_overrides.cutpoints = Some(interrupt.clone());
    let (daemon, mut client) = start(config.clone()).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let mut command = start_command(&create);
    interrupt.0.store(true, Ordering::Release);
    let error = client.send_command(&command).await.unwrap_err();
    assert!(
        matches!(error, ClientError::Remote(error) if error.code == ErrorCode::InternalFailure)
    );
    let snapshot = client.query(&scoped_run_query(&create)).await.unwrap();
    assert_eq!(snapshot.result.get("state"), Some(&json!("prepared")));
    drop(client);
    daemon
        .shutdown_graceful("inspect start rollback")
        .await
        .unwrap();
    drop(daemon);
    let rolled_back = store.clone();
    tokio::task::spawn_blocking(move || assert_start_effects(rolled_back, None))
        .await
        .unwrap();

    let (daemon, mut client) = start(config).await;
    command.request_id = RequestId::from_u128(0x5923);
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.new_revision, Revision::new(2));
    drop(client);
    daemon
        .shutdown_graceful("inspect committed retry")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some((receipt.receipt_id, 1))))
        .await
        .unwrap();
}

// T-F005-001 — Versions épinglées après dépréciation.
#[tokio::test]
async fn uses_the_same_retained_plan_after_its_versions_are_deprecated() {
    use unleash_store::WriteCriticality;

    let data = unleash_testkit::local_test_dir("f005-start-pinned-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-pinned-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |_| {}).await;
    let (daemon, mut client) = start(config.clone()).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();
    let query = scoped_run_query(&create);
    let before = client.query(&query).await.unwrap();
    drop(client);
    daemon
        .shutdown_graceful("deprecate fixture publications")
        .await
        .unwrap();
    drop(daemon);

    let update = store.clone();
    tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(update).unwrap();
        store
            .execute_in_writer_tx(WriteCriticality::Normal, |tx, _| {
                assert_eq!(tx.execute(
                "UPDATE workflow_versions SET visibility='deprecated' WHERE workflow_version_id=?1",
                [super::PUBLISHED],
            )?, 1);
                assert_eq!(
                    tx.execute(
                        "UPDATE registry_versions SET publication_state='deprecated'
                    WHERE registry_version_id IN ('role-v1','skill-v1','contract-v1')",
                        [],
                    )?,
                    3
                );
                Ok(())
            })
            .unwrap();
        store.shutdown().unwrap();
    })
    .await
    .unwrap();

    let (daemon, mut client) = start(config).await;
    let receipt = client.send_command(&start_command(&create)).await.unwrap();
    let after = client.query(&query).await.unwrap();
    assert_eq!(after.result.get("state"), Some(&json!("running")));
    for key in ["definition_hash", "effective_bundle_hash", "plan"] {
        assert_eq!(before.result.get(key), after.result.get(key), "{key}");
    }
    drop(client);
    daemon
        .shutdown_graceful("inspect pinned activation")
        .await
        .unwrap();
    drop(daemon);
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some((receipt.receipt_id, 1))))
        .await
        .unwrap();
}

#[tokio::test]
async fn t_f005_004_refuses_the_entry_above_the_activation_budget() {
    let data = unleash_testkit::local_test_dir("f005-budget-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-budget-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |workflow| {
            sequence_at_budget(workflow, 1);
        })
        .await;
    let (daemon, mut client) = start(config).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let result = client.send_command(&start_command(&create)).await;
    let snapshot = client.query(&scoped_run_query(&create)).await.unwrap();
    drop(client);
    daemon
        .shutdown_graceful("inspect refused budget")
        .await
        .unwrap();
    drop(daemon);

    let error = result.expect_err("the sequence and its first task cost two activations");
    assert!(matches!(error, ClientError::Remote(error)
        if error.code == ErrorCode::BudgetExceeded
            && error.reason_code == Some(ReasonCode::BudgetExhausted)));
    assert_eq!(snapshot.result.get("state"), Some(&json!("prepared")));
    assert_eq!(snapshot.result.get("revision"), Some(&json!(1)));
    tokio::task::spawn_blocking(move || assert_start_effects(store, None))
        .await
        .unwrap();
}

// T-F012-018 / AC-F012-018 / REQ-MODEL-018 — l'entrée d'un run `forbidden` qui active
// une tâche d'agent est refusée avant tout effet de démarrage.
#[tokio::test]
async fn t_f012_018_forbidden_run_refuses_agent_task_activation() {
    let data = unleash_testkit::local_test_dir("f012-model-policy-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f012-model-policy-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |workflow| {
            let definition = workflow.as_object_mut().unwrap();
            definition.insert("schema_version".to_owned(), json!("unleash.workflow.v1.1"));
            // An inconsistent version imported before semantic validation.
            definition.insert("model_policy".to_owned(), json!("forbidden"));
        })
        .await;
    let (daemon, mut client) = start(config).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let result = client.send_command(&start_command(&create)).await;
    let snapshot = client.query(&scoped_run_query(&create)).await.unwrap();
    drop(client);
    daemon
        .shutdown_graceful("inspect run without model authorization")
        .await
        .unwrap();
    drop(daemon);

    let error = result.expect_err("a no-model workflow cannot activate an agent task");
    assert!(matches!(error, ClientError::Remote(error)
        if error.code == ErrorCode::CapabilityDenied
            && error.reason_code == Some(ReasonCode::ModelCallsForbidden)));
    assert_eq!(snapshot.result.get("state"), Some(&json!("prepared")));
    assert_eq!(snapshot.result.get("revision"), Some(&json!(1)));
    tokio::task::spawn_blocking(move || assert_start_effects(store, None))
        .await
        .unwrap();
}

// T-F012-018 / AC-F012-018 / REQ-MODEL-018 — la politique interdit l'activation
// d'une tâche d'agent, pas le démarrage en soi : une entrée mécanique suivie de
// la tâche d'agent n'est pas refusée au titre du modèle. Le moteur ne matérialise
// aujourd'hui qu'une entrée résoluble en tâche ; le refus observé est le sien. Ce
// test doit réclamer le démarrage réel dès que le moteur active une entrée d'action.
#[tokio::test]
async fn t_f012_018_forbidden_run_with_a_mechanical_entry_is_not_refused_by_the_model_policy() {
    let data = unleash_testkit::local_test_dir("f012-mechanical-entry-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f012-mechanical-entry-rdv").unwrap();
    let (store, config, mut create) =
        fixture(data.path(), rendezvous.path(), AgentLevel::N, |workflow| {
            let task = workflow.get_mut("body").unwrap().take();
            let definition = workflow.as_object_mut().unwrap();
            definition.insert("schema_version".to_owned(), json!("unleash.workflow.v1.1"));
            // An inconsistent version imported before semantic validation.
            definition.insert("model_policy".to_owned(), json!("forbidden"));
            definition.insert(
                "body".to_owned(),
                json!({"id": "intake", "kind": "sequence", "steps": [
                    {"id": "collect", "kind": "action",
                     "operation": {"id": "demo.collect", "version": 1},
                     "contract_profile": {"id": "demo.contract", "version": 1},
                     "scope_key": "implementation", "inputs": {},
                     "timeout_seconds": 60, "max_attempts": 1, "retry_policy": "never"},
                    task,
                ]}),
            );
        })
        .await;
    let (daemon, mut client) = start(config).await;
    publish_mandate(&mut client, &mut create).await;
    client.send_command(&create).await.unwrap();

    let result = client.send_command(&start_command(&create)).await;
    let snapshot = client.query(&scoped_run_query(&create)).await.unwrap();
    drop(client);
    daemon
        .shutdown_graceful("inspect run whose entry is mechanical")
        .await
        .unwrap();
    drop(daemon);

    let error = result.expect_err("the engine resolves no activation for a mechanical entry");
    assert!(
        matches!(error, ClientError::Remote(ref remote)
            if remote.code == ErrorCode::ContractViolation
                && remote.reason_code == Some(ReasonCode::EvidenceUnavailable)),
        "the refusal must be the engine's, not the model policy's: {error:?}"
    );
    assert_eq!(snapshot.result.get("state"), Some(&json!("prepared")));
    assert_eq!(snapshot.result.get("revision"), Some(&json!(1)));
    tokio::task::spawn_blocking(move || assert_start_effects(store, None))
        .await
        .unwrap();
}
