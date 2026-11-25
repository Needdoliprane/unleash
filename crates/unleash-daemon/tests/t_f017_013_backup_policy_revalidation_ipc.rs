//! T-F017-013 / AC-F017-007 / REQ-SEC-007 : sauvegarde interne liée à la politique engagée.
//!
//! Le courtier réservé à `RequestBackup` est lié à la révision de politique que la demande a
//! engagée. Une politique validée ne la refuse pas. Une politique qui change avant le dispatch
//! laisse la demande dans l'état terminal `blocked`, avec un motif fermé attesté par le magasin,
//! un événement publié, aucun fichier de sauvegarde et aucun dispatch.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use serde_json::{Map, json};
use std::time::Duration;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    PolicyIsolationMode, PolicyScopeKind, StoreConfig, UnleashStore,
};
use unleash_testkit::{local_test_dir, private_test_dir};

#[tokio::test]
async fn standard_policy_does_not_deny_internal_backup() {
    use unleash_domain::ActorId;
    let rdv = private_test_dir("f017-007-rdv").unwrap();
    let data = local_test_dir("f017-007-data").unwrap();
    let data_dir = data.path().to_path_buf();
    let seed_dir = data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(StoreConfig::new(seed_dir)).unwrap();
        let generation = store.daemon_state().unwrap().authority_generation;
        let actor = ActorId::new("fixture-admin").unwrap();
        store
            .commit_command(CommitCommandRequest {
                actor_id: actor.clone(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(701),
                    command_id: CommandId::from_u128(702),
                    authority_generation: generation,
                    actor_ref: None,
                    project_id: None,
                    aggregate_id: generation.as_uuid(),
                    expected_revision: Revision::INITIAL,
                    command_type: "SetPolicy".into(),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::SecurityPolicy,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::SetPolicy {
                    scope_kind: PolicyScopeKind::Global,
                    scope_id: "global".into(),
                    isolation_mode: PolicyIsolationMode::Standard,
                    policy_version: "test-standard-v1".into(),
                    reason: "test fixture".into(),
                    caller: CallerAuthority {
                        actor_id: actor.clone(),
                        role: CallerRole::PolicyAdmin,
                    },
                },
                events: Vec::new(),
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap();
    })
    .await
    .unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data_dir),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "policy-backup-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(703);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(704),
        command_id: CommandId::from_u128(705),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);

    let store_cfg = StoreConfig::new(data.path());
    let db_path = store_cfg.db_path;
    let mut last = ("aucune ligne".to_owned(), "aucune ligne".to_owned());
    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let db_path = db_path.clone();
            let observed = tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open_with_flags(
                    db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                conn.query_row(
                    "SELECT b.state, a.state FROM backup_requests r JOIN backup_jobs b USING(backup_id) JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
                    [backup_id.to_string()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .ok()
            })
            .await
            .unwrap();
            if let Some((job, action)) = observed.as_ref() {
                if job == "complete" && action == "observed_success" {
                    return;
                }
                last = (job.clone(), action.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        completed.is_ok(),
        "une politique standard validée ne doit pas refuser la sauvegarde ; derniers états observés : job='{}', action='{}'",
        last.0,
        last.1
    );

    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(706),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id": backup_id}).as_object().unwrap().clone(),
    };
    let result = client.query(&query).await.unwrap();
    assert_eq!(result.result["state"], "completed");
    handle.shutdown_graceful("test").await.unwrap();
}
