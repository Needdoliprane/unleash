//! REQ-AGENT-002 : remise IPC de l'affectation et reçu durable après redémarrage.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Fixture IPC isolée et assertions de contrat"
)]

#[path = "../../unleash-store/tests/support/workflow_task.rs"]
mod workflow_task;

use serde_json::{Map, Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{ActorId, RequestId, Revision, Sha256Digest};
use unleash_platform::{current_boot_identity, effective_uid};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ProtocolVersion, QueryEnvelope,
};
use unleash_store::{StoreConfig, UnleashStore};
use unleash_testkit::local_test_dir;
use workflow_task::Fixture;

#[tokio::test]
async fn t_f003_assign_task_queues_once_across_daemon_restart() {
    let data = local_test_dir("f003a-data").unwrap();
    let rendezvous = local_test_dir("f003a-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), PermissionsExt::from_mode(0o700)).unwrap();
    let store_config = local_store_config(data.path());
    let actor = ActorId::new(format!("uid:{}", effective_uid())).unwrap();
    let preparation_config = store_config.clone();
    let (mut command, bundle) = tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(preparation_config).unwrap();
        let fixture = Fixture::on_store(store, actor.as_str(), actor.clone(), true);
        let (command, _, preparation) = fixture.prepared_assignment();
        (command, preparation.effective_bundle_hash)
    })
    .await
    .unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config.clone()).await.unwrap()
    else {
        panic!("Le daemon local doit être prêt");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let original_epoch = client.hello().daemon_epoch;
    assert_eq!(
        client.hello().authority_generation,
        command.authority_generation
    );
    let accepted = client.send_command(&command).await.unwrap();
    assert_eq!(accepted.new_revision, Revision::new(2));
    drop(client);
    daemon
        .shutdown_graceful("first assignment receipt")
        .await
        .unwrap();
    drop(daemon);

    let DaemonStartOutcome::Started(restarted) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("Le daemon local doit rouvrir les données");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        restarted.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    assert!(client.hello().daemon_epoch.as_u64() > original_epoch.as_u64());
    assert_eq!(
        client.hello().authority_generation,
        command.authority_generation
    );
    command.request_id = RequestId::from_u128(0xA155);
    let replay = client.send_command(&command).await.unwrap();
    assert_eq!(replay, accepted);
    assert_visible_queue(&mut client, &command, &accepted, bundle).await;
    drop(client);
    restarted
        .shutdown_graceful("assignment replay verified")
        .await
        .unwrap();
    drop(restarted);

    tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(store_config).unwrap();
        assert_one_queued_assignment(&store, &accepted, bundle);
    })
    .await
    .unwrap();
}

async fn assert_visible_queue(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
    accepted: &CommandReceipt,
    bundle: Sha256Digest,
) {
    let invocation = accepted
        .result_ref
        .as_deref()
        .unwrap()
        .strip_prefix("invocation:")
        .unwrap();
    let observed = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xA156),
            query_type: "GetInvocation".to_owned(),
            project_id: command.project_id,
            payload: Map::from_iter([("invocation_id".to_owned(), json!(invocation))]),
        })
        .await
        .unwrap()
        .result;
    assert_eq!(
        observed.get("invocation_id").and_then(Value::as_str),
        Some(invocation)
    );
    assert_eq!(
        observed.get("state").and_then(Value::as_str),
        Some("queued")
    );
    assert_eq!(observed.get("effective_bundle_hash"), Some(&json!(bundle)));
    assert_eq!(
        observed
            .get("attempts")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
}

fn local_store_config(path: &Path) -> StoreConfig {
    let mut config = StoreConfig::new(path);
    config.owner_uid = effective_uid();
    current_boot_identity()
        .unwrap()
        .as_str()
        .clone_into(&mut config.boot_identity);
    config
}

fn assert_one_queued_assignment(
    store: &UnleashStore,
    accepted: &CommandReceipt,
    bundle: Sha256Digest,
) {
    store
        .read_snapshot(|conn, _| {
            let queue: (i64, String, String, String) = conn.query_row(
                "SELECT count(*),MIN(invocation_id),MIN(effective_bundle_hash),MIN(state)
             FROM invocations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert_eq!(queue.0, 1);
            assert_eq!(
                accepted.result_ref.as_deref(),
                Some(format!("invocation:{}", queue.1).as_str())
            );
            assert_eq!(queue.2, bundle.to_string());
            assert_eq!(queue.3, "queued");
            let durable: (i64, i64, i64) = conn.query_row(
                "SELECT
                (SELECT count(*) FROM command_receipts WHERE command_type='AssignTask'),
                (SELECT count(*) FROM events WHERE causation_receipt_id=?1
                    AND event_type IN ('TaskAssigned','InvocationQueued')),
                (SELECT count(*) FROM tasks WHERE state='queued' AND revision=2)",
                [accepted.receipt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(durable, (1, 2, 1));
            Ok(())
        })
        .unwrap();
}
