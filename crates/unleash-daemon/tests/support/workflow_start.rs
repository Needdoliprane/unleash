//! Initial progression of a prepared run through the local daemon.

use std::{os::unix::fs::PermissionsExt, path::Path};

use serde_json::{Map, json};
use unleash_client::ClientError;
use unleash_daemon::DaemonConfig;
use unleash_domain::{CommandId, ReceiptId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ErrorCode, ReceiptState};
use unleash_store::{StoreConfig, UnleashStore};

use super::{RUN, local_store_config, publish_mandate, scoped_run_query, start, wire_command};

async fn fixture(data: &Path, rendezvous: &Path) -> (StoreConfig, DaemonConfig, CommandEnvelope) {
    std::fs::set_permissions(rendezvous, PermissionsExt::from_mode(0o700)).unwrap();
    let store = local_store_config(data);
    let seed = store.clone();
    let (create, _) = tokio::task::spawn_blocking(move || wire_command(seed))
        .await
        .unwrap();
    let config = DaemonConfig {
        data_root: Some(data.to_owned()),
        rendezvous_dir: Some(rendezvous.to_owned()),
        ..DaemonConfig::default()
    };
    (store, config, create)
}

fn start_command(create: &CommandEnvelope) -> CommandEnvelope {
    CommandEnvelope {
        request_id: RequestId::from_u128(0x5920),
        command_id: CommandId::from_u128(0x5920),
        command_type: "StartRun".to_owned(),
        expected_revision: Revision::new(1),
        payload: Map::new(),
        ..create.clone()
    }
}

#[tokio::test]
async fn starts_once_and_recovers_the_ready_task() {
    let data = unleash_testkit::local_test_dir("f005-start-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-rdv").unwrap();
    let (store, config, mut create) = fixture(data.path(), rendezvous.path()).await;
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
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some(receipt.receipt_id)))
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

#[tokio::test]
async fn refuses_a_stale_project_scope_without_allocating_work() {
    let data = unleash_testkit::local_test_dir("f005-stale-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-stale-rdv").unwrap();
    let (store, config, mut create) = fixture(data.path(), rendezvous.path()).await;
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

fn assert_start_effects(config: StoreConfig, receipt: Option<ReceiptId>) {
    let store = UnleashStore::open(config).unwrap();
    let causal_receipt = receipt.map_or_else(String::new, |id| id.to_string());
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

#[tokio::test]
async fn rolls_back_the_start_transaction_and_retries_the_same_command() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let data = unleash_testkit::local_test_dir("f005-start-c1-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-c1-rdv").unwrap();
    let (store, mut config, mut create) = fixture(data.path(), rendezvous.path()).await;
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
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some(receipt.receipt_id)))
        .await
        .unwrap();
}

#[tokio::test]
async fn uses_the_same_retained_plan_after_its_versions_are_deprecated() {
    use unleash_store::WriteCriticality;

    let data = unleash_testkit::local_test_dir("f005-start-pinned-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f005-start-pinned-rdv").unwrap();
    let (store, config, mut create) = fixture(data.path(), rendezvous.path()).await;
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
    tokio::task::spawn_blocking(move || assert_start_effects(store, Some(receipt.receipt_id)))
        .await
        .unwrap();
}
