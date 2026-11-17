// T-F001-006 / AC-F001-006 / REQ-CORE-006
//! Barrière de démarrage : verrou OS, vérification `sqlite_source_id()`,
//! classification des ressources avant toute admission métier, et inspection `GetDaemonStatus`
//! seule autorisée pendant `recovering`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{
    DaemonConfig, DaemonError, DaemonStartOutcome, RecoveryGate, StoreOverrides, UnleashDaemon,
};
use unleash_domain::{
    AccountId, ActionId, ActorId, CommandId, DaemonMode, IdGeneratorPort, MicrosAmount, ProjectId,
    RequestId, Revision, Sha256Digest, StopOutcome,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReasonCode,
    ReceiptState,
};
use unleash_store::{
    AcquireLeaseRequest, AggregateKind, BudgetScopeType, CommandMutation, CommitCommandRequest,
    EventAppendSpec, SqliteBuildPolicy, StoreConfig, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, local_test_dir};

#[tokio::test]
async fn t_f001_006_startup_barrier_rejects_admission_during_recovering_and_unqualified_sqlite() {
    let rdv_bad = local_test_dir("f001-06-brdv").unwrap();
    std::fs::set_permissions(
        rdv_bad.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data_bad = local_test_dir("f001-06-bdat").unwrap();

    // 1. Rejet avant ouverture socket si sqlite_source_id() diverge de la politique compilée
    let bad_cfg = DaemonConfig {
        data_root: Some(data_bad.path().to_path_buf()),
        rendezvous_dir: Some(rdv_bad.path().to_path_buf()),
        store_overrides: StoreOverrides {
            build_policy: Some(SqliteBuildPolicy {
                expected_source_id: String::from("00000000-unqualified-source-id"),
                enforce_source_id_match: true,
                extra_approved_backports: Vec::new(),
            }),
            ..StoreOverrides::default()
        },
        ..DaemonConfig::default()
    };
    let build_err = UnleashDaemon::start(bad_cfg).await.unwrap_err();
    assert!(matches!(
        build_err,
        DaemonError::Store(StoreError::BuildUnqualified { .. })
    ));
    assert!(!rdv_bad.path().join("daemon.sock").exists());

    // 2. Pré-peupler une base avec une lease `held`, une action `in_flight` et une réserve `held`
    let rdv = local_test_dir("f001-06-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-06-dat").unwrap();
    let ids = DeterministicIdGenerator::new(0xF001_0006);
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let account_id = AccountId::from_uuid(ids.new_uuid());
    let action_id = ActionId::from_uuid(ids.new_uuid());

    let data_seed_path = data.path().to_path_buf();
    let (lease_id, res_id) = tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(StoreConfig::new(&data_seed_path)).unwrap();
        let auth = store.active_authority_generation().unwrap();
        store
            .register_principal("principal-test", "user", "uid:test")
            .unwrap();
        store
            .commit_command(CommitCommandRequest {
                actor_id: ActorId::new("uid:test").unwrap(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_uuid(ids.new_uuid()),
                    command_id: CommandId::from_uuid(ids.new_uuid()),
                    authority_generation: auth,
                    actor_ref: None,
                    project_id: Some(project_id),
                    aggregate_id: project_id.as_uuid(),
                    expected_revision: Revision::INITIAL,
                    command_type: String::from("CreateProject"),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::Project,
                operation_id: None,
                receipt_state: unleash_protocol::ReceiptState::Accepted,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::CreateProject {
                    project_id,
                    name: String::from("RecoveryBarrier"),
                    weight: 1,
                },
                events: Vec::new(),
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap();

        let lease = store
            .acquire_lease(AcquireLeaseRequest {
                resource_kind: String::from("worktree"),
                resource_key: String::from("wt-1"),
                owner_type: String::from("worker"),
                owner_id: String::from("worker-old"),
                ttl_ms: 60_000,
            })
            .unwrap();

        store
            .upsert_budget_account(
                account_id,
                BudgetScopeType::Project,
                &project_id.to_string(),
                "USD",
                MicrosAmount::new(100_000),
            )
            .unwrap();

        let res_uuid = ids.new_uuid().to_string();
        let res_id_clone = res_uuid.clone();
        let act_id_str = action_id.to_string();
        let acc_id_str = account_id.to_string();
        let lease_id_str = lease.lease_id.to_string();
        let cmd_uuid = CommandId::from_uuid(ids.new_uuid());
        let payload_hash = Sha256Digest::from_bytes(b"in_flight_action");

        store
            .commit_command(CommitCommandRequest {
                actor_id: ActorId::new("uid:test").unwrap(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_uuid(ids.new_uuid()),
                    command_id: cmd_uuid,
                    authority_generation: auth,
                    actor_ref: None,
                    project_id: Some(project_id),
                    aggregate_id: action_id.as_uuid(),
                    expected_revision: Revision::INITIAL,
                    command_type: String::from("SeedAction"),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::Action,
                operation_id: None,
                receipt_state: unleash_protocol::ReceiptState::Accepted,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::CreateAction {
                    action_id,
                    origin_kind: String::from("control_plane"),
                    control_principal_id: Some(String::from("principal-test")),
                    run_id: None,
                    step_id: None,
                    invocation_id: None,
                    attempt_id: None,
                    replay_worker_id: None,
                    action_type: String::from("process_exec"),
                    payload_hash,
                    reconcile_strategy: String::from("observe_then_decide"),
                    policy_revision: 1,
                    resource_lease_id: Some(lease_id_str),
                },
                events: vec![EventAppendSpec {
                    event_type: String::from("ActionPrepared"),
                    aggregate_type: AggregateKind::Action,
                    aggregate_id: action_id.as_uuid(),
                    aggregate_revision: Revision::new(1),
                    project_id: Some(project_id),
                    correlation_id: action_id.as_uuid(),
                    causation_kind: unleash_protocol::EventCausationKind::Command,
                    causation_id: None,
                    payload: Map::new(),
                    sensitivity: unleash_protocol::Sensitivity::Project,
                    outbox_destinations: Vec::new(),
                }],
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap();

        store
            .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _now| {
                tx.execute(
                    "UPDATE actions SET state = 'in_flight', revision = revision + 1 WHERE action_id = ?1",
                    [&act_id_str],
                )?;
                tx.execute(
                    "INSERT INTO budget_reservations (reservation_id, account_id, action_id, reserved_micros, settled_micros, state, pricing_version, max_request_hash)
                     VALUES (?1, ?2, ?3, 5000, NULL, 'held', 'v1', ?4)",
                    rusqlite::params![res_id_clone, acc_id_str, act_id_str, payload_hash.as_prefixed_str()],
                )?;
                tx.execute(
                    "UPDATE budget_accounts SET reserved_micros = 5000 WHERE account_id = ?1",
                    [&acc_id_str],
                )?;
                Ok(())
            })
            .unwrap();

        store.shutdown().unwrap();
        (lease.lease_id.to_string(), res_uuid)
    })
    .await
    .unwrap();

    // 3. Démarrer avec RecoveryGate -> observer mode = recovering
    let gate = RecoveryGate::new();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        recovery_gate: Some(Arc::clone(&gate)),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu en cours de démarrage");
    };
    gate.wait_until_entered().await;
    assert_eq!(handle.mode(), DaemonMode::Recovering);

    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let auth = client.hello().authority_generation;

    // 4. GetDaemonStatus seul autorisé en recovering
    let status_resp = client
        .query(&unleash_protocol::QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x501),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        status_resp.result.get("mode").and_then(Value::as_str),
        Some("recovering")
    );

    // 5. Mutations et GetStateSnapshot refusées avec RecoveryInProgress pendant recovering
    let blocked_cmd_err = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x502),
            command_id: CommandId::from_u128(0xC502),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: uuid::Uuid::from_u128(0x9999),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "InterditEnRecovery", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        blocked_cmd_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::ResourceBusy
            && e.reason_code == Some(ReasonCode::RecoveryInProgress)
    ));

    let blocked_snap_err = client
        .get_state_snapshot(RequestId::from_u128(0x503), None, Map::new())
        .await
        .unwrap_err();
    assert!(matches!(
        blocked_snap_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::ResourceBusy
            && e.reason_code == Some(ReasonCode::RecoveryInProgress)
    ));

    // 6. Libérer la barrière et attendre Ready
    gate.release();
    handle.wait_ready().await.unwrap();
    assert_eq!(handle.mode(), DaemonMode::Ready);

    let ready_status = client
        .query(&unleash_protocol::QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x504),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        ready_status.result.get("mode").and_then(Value::as_str),
        Some("ready")
    );

    // Une nouvelle admission métier passe maintenant
    let new_proj = ProjectId::from_u128(0x7777);
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x505),
            command_id: CommandId::from_u128(0xC505),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(new_proj),
            aggregate_id: new_proj.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "PostRecovery", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    handle.shutdown_graceful("done").await.unwrap();

    // 7. Vérifier en base que lease -> suspect, action -> outcome_unknown, budget_reservation -> unknown
    let verify_data = data.path().to_path_buf();
    let (lease_state, act_state, res_state): (String, String, String) =
        tokio::task::spawn_blocking(move || {
            let verify_store = UnleashStore::open(StoreConfig::new(&verify_data)).unwrap();
            let tuple = verify_store
                .read_snapshot(|conn, _ctx| {
                    let ls: String = conn.query_row(
                        "SELECT state FROM resource_leases WHERE lease_id = ?1",
                        [&lease_id],
                        |r| r.get(0),
                    )?;
                    let ac: String = conn.query_row(
                        "SELECT state FROM actions WHERE action_id = ?1",
                        [action_id.to_string()],
                        |r| r.get(0),
                    )?;
                    let rs: String = conn.query_row(
                        "SELECT state FROM budget_reservations WHERE reservation_id = ?1",
                        [&res_id],
                        |r| r.get(0),
                    )?;
                    Ok((ls, ac, rs))
                })
                .unwrap();
            verify_store.shutdown().unwrap();
            tuple
        })
        .await
        .unwrap();
    assert_eq!(lease_state, "suspect");
    assert_eq!(act_state, "outcome_unknown");
    assert_eq!(res_state, "unknown");
}

#[tokio::test]
async fn t_f001_006_allows_stop_daemon_during_recovering() {
    let rdv = local_test_dir("f001-06-stop-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-06-stop-dat").unwrap();

    let gate = RecoveryGate::new();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        recovery_gate: Some(Arc::clone(&gate)),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu en cours de démarrage");
    };
    gate.wait_until_entered().await;
    assert_eq!(handle.mode(), DaemonMode::Recovering);

    let sock = handle.socket_path().to_path_buf();
    let mut client =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth = client.hello().authority_generation;

    let status = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x601),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        status.result.get("mode").and_then(Value::as_str),
        Some("recovering")
    );
    let rev = status
        .result
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap();

    // StopDaemon doit être accepté directement en mode `recovering` sans libérer le RecoveryGate
    let rcpt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x602),
            command_id: CommandId::from_u128(0xC602),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(rev),
            command_type: String::from("StopDaemon"),
            payload: match json!({ "reason": "stop during recovering" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    assert_eq!(rcpt.state, ReceiptState::Accepted);

    let report = tokio::time::timeout(Duration::from_secs(5), handle.wait_stopped())
        .await
        .expect("timeout arrêt pendant recovering")
        .expect("rapport d'arrêt attendu");
    assert_eq!(report.outcome, StopOutcome::Complete);
    assert!(!sock.exists());
}
