// T-F002-001 / AC-F002-001 / REQ-PERSIST-001
//! Atomicité transactionnelle et injection des cutpoints C0/C1/C2.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_domain::{
    AccountId, ActionId, ActorId, AdmissionClass, AgentId, AgentState, AuthorityGeneration,
    CommandId, CompletionPolicy, IdGeneratorPort, InvocationId, ManifestId, MicrosAmount,
    MissionId, MissionState, PoolId, ProjectId, ReceiptId, RequestId, ReservationId, Revision,
    SessionId, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, BudgetAccountCharge, BudgetReservationRequest, BudgetScopeType,
    CapacityReservationRequest, CommandMutation, CommitCommandRequest, CommitOutcome,
    CutpointObserver, EventAppendSpec, ExecutionOwner, ExecutionPoolParams, MonotonicClockPort,
    PricingCheckStatus, StoreConfig, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{
    ChildCrashContext, ChildCrashHarness, CrashKillMode, Cutpoint, CutpointController,
    DeterministicIdGenerator, ExternalEffectOracle, ManualClock, NewExternalEffect, local_test_dir,
};
use uuid::Uuid;

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct CutpointAdapter(CutpointController);
impl CutpointObserver for CutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let cp = match cutpoint {
            StoreCutpoint::C0BeforeCommandTx => Cutpoint::C0,
            StoreCutpoint::C1BeforeCommandCommit => Cutpoint::C1,
            StoreCutpoint::C2AfterCommandCommit => Cutpoint::C2,
            StoreCutpoint::C3AfterBrokerIntentCommit => Cutpoint::C3,
            StoreCutpoint::C4AfterBrokerDispatchCommit => Cutpoint::C4,
            StoreCutpoint::C10aAfterPrepPin => Cutpoint::C10a,
            StoreCutpoint::C10bBeforeDeletingReserve => Cutpoint::C10b,
            StoreCutpoint::C10cAfterDeletingIntent => Cutpoint::C10c,
            StoreCutpoint::C10dAfterUnlinkSyncBeforeTombstone => Cutpoint::C10d,
            StoreCutpoint::C11aAfterBackupBarrier => Cutpoint::C11a,
            StoreCutpoint::C11bAfterDbSnapshotBeforePins => Cutpoint::C11b,
            StoreCutpoint::C11cAfterPinsDuringCopy => Cutpoint::C11c,
            StoreCutpoint::C11dAfterManifestBeforeComplete => Cutpoint::C11d,
            StoreCutpoint::C11MigrationInProgress
            | StoreCutpoint::C11RestoreBeforeActivate
            | StoreCutpoint::C11eAfterCompleteManifestBeforeJobCommit
            | StoreCutpoint::C11fAfterBackupJobCommit
            | StoreCutpoint::C11gBeforeManagedBackupRootCreate => Cutpoint::C11,
            StoreCutpoint::C7AfterInboxBeforeOutboxAck => Cutpoint::C7,
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

fn build_test_config(
    root: &std::path::Path,
    clock: Arc<ManualClock>,
    ids: Arc<DeterministicIdGenerator>,
    ctrl: CutpointController,
) -> StoreConfig {
    let mut cfg = StoreConfig::new(root);
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(clock));
    cfg.id_generator = ids;
    cfg.cutpoints = Arc::new(CutpointAdapter(ctrl));
    cfg
}

fn seed_action_and_invocation(
    store: &UnleashStore,
    authority: AuthorityGeneration,
    ids: &DeterministicIdGenerator,
    project_id: ProjectId,
    action_id: ActionId,
    inv_id: InvocationId,
) -> Result<(), StoreError> {
    let agent_id = AgentId::from_uuid(ids.new_uuid());
    let sess_id = SessionId::from_uuid(ids.new_uuid());
    let man_id = ManifestId::from_uuid(ids.new_uuid());
    let ev_uuid = ids.new_uuid();
    let rcpt_id = ReceiptId::from_uuid(ids.new_uuid());
    store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
        let obj_hash = Sha256Digest::from_bytes(action_id.as_uuid().as_bytes()).to_string();
        let bundle_hash = Sha256Digest::from_bytes(inv_id.as_uuid().as_bytes()).to_string();
        tx.execute(
            "INSERT OR IGNORE INTO object_records(object_hash, object_generation, relative_path, byte_length, media_type, state, created_at_ms) VALUES (?1, 1, ?2, 3, 'application/json', 'live', ?3)",
            rusqlite::params![obj_hash, format!("blobs/{action_id}"), now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO agents(agent_id, level, organization_parent_id, created_by_agent_id, project_id, state, revision) VALUES (?1, 'N', NULL, NULL, ?2, ?3, 1)",
            rusqlite::params![agent_id.to_string(), project_id.to_string(), AgentState::Eligible.to_string()],
        )?;
        tx.execute(
            "INSERT INTO sessions(session_id, agent_id, revision, state, created_at_ms) VALUES (?1, ?2, 0, 'open', ?3)",
            rusqlite::params![sess_id.to_string(), agent_id.to_string(), now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO context_manifests(manifest_id, session_id, base_session_revision, content_hash, schema_version, created_at_ms) VALUES (?1, ?2, 0, ?3, 1, ?4)",
            rusqlite::params![man_id.to_string(), sess_id.to_string(), obj_hash, now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO effective_bundles(bundle_hash, schema_version, role_version_id, requested_model_profile_version_id, allowed_model_policy_hash, initial_context_manifest_id, scope_id, scope_revision, content_object_hash) VALUES (?1, 1, 'rv1', 'mp1', ?2, ?3, 's1', 0, ?2)",
            rusqlite::params![bundle_hash, obj_hash, man_id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO command_receipts(actor_id, command_id, authority_generation, receipt_id, request_id, command_type, command_digest, aggregate_id, state, response_state, response_json, response_retained_until_ms, committed_at_ms) VALUES ('system', ?1, ?2, ?3, ?4, 'SeedInv', 'd', ?5, 'completed', 'full', '{}', ?6, ?7)",
            rusqlite::params![ev_uuid.to_string(), authority.to_string(), rcpt_id.to_string(), ev_uuid.to_string(), inv_id.to_string(), now.as_i64() + 86_400_000, now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO events(event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal, event_type, schema_version, actor_ref, actor_id, occurred_at_ms, recorded_at_ms, payload_json, payload_state, payload_hash, sensitivity, causation_kind, causation_id, causation_receipt_id, correlation_id) VALUES (?1, ?2, 'invocation', ?3, 1, 0, 'InvocationQueued', 1, 'system', 'system', ?4, ?4, '{}', 'full', ?5, 'project', 'command', ?6, ?6, ?1)",
            rusqlite::params![ev_uuid.to_string(), Uuid::from_u128(1).to_string(), inv_id.to_string(), now.as_i64(), obj_hash, rcpt_id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO invocations(invocation_id, agent_id, session_id, trigger_event_id, effective_bundle_hash, context_manifest_id, state, admission_class, revision, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 'work', 0, ?7)",
            rusqlite::params![inv_id.to_string(), agent_id.to_string(), sess_id.to_string(), ev_uuid.to_string(), bundle_hash, man_id.to_string(), now.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO actions(action_id, origin_kind, control_principal_id, run_id, step_id, invocation_id, attempt_id, replay_worker_id, action_type, payload_hash, authorization_ref, policy_revision, resource_lease_id, authority_generation, daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms) VALUES (?1, 'control_plane', 'principal-louis', NULL, NULL, NULL, NULL, NULL, 'seed_act', ?2, NULL, 1, NULL, ?3, 1, 0, 'prepared', 'observe_then_decide', NULL, 1, ?4)",
            rusqlite::params![action_id.to_string(), obj_hash, authority.to_string(), now.as_i64()],
        )?;
        Ok(())
    })
}

struct SetupCommandArgs<'a> {
    authority: AuthorityGeneration,
    ids: &'a DeterministicIdGenerator,
    project_id: ProjectId,
    mission_id: MissionId,
    action_id: ActionId,
    inv_id: InvocationId,
    pool_id: PoolId,
    account_id: AccountId,
    command_id: CommandId,
}

fn setup_command(args: &SetupCommandArgs<'_>) -> Result<CommitCommandRequest, StoreError> {
    let authority = args.authority;
    let ids = args.ids;
    let project_id = args.project_id;
    let mission_id = args.mission_id;
    let action_id = args.action_id;
    let inv_id = args.inv_id;
    let pool_id = args.pool_id;
    let account_id = args.account_id;
    let command_id = args.command_id;
    let mut payload = Map::new();
    payload.insert("mission_id".into(), Value::String(mission_id.to_string()));
    let mut event_payload = Map::new();
    event_payload.insert("mission_id".into(), Value::String(mission_id.to_string()));

    let res_id = ReservationId::from_uuid(ids.new_uuid());

    Ok(CommitCommandRequest {
        actor_id: ActorId::new("user:louis")?,
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id,
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateMission".to_owned(),
            payload,
        },
        target_aggregate: AggregateKind::Mission,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::CreateMission {
            mission_id,
            goal_id: None,
            project_id,
            owner_principal_id: Some("principal-louis".to_owned()),
            owner_agent_id: None,
            completion_policy: CompletionPolicy::Mechanical,
            state: MissionState::Ready,
        },
        events: vec![EventAppendSpec {
            event_type: "MissionCreated".to_owned(),
            aggregate_type: AggregateKind::Mission,
            aggregate_id: mission_id.as_uuid(),
            aggregate_revision: Revision::new(1),
            project_id: Some(project_id),
            correlation_id: Uuid::from_u128(99),
            causation_kind: EventCausationKind::Command,
            causation_id: None,
            payload: event_payload,
            sensitivity: Sensitivity::Project,
            outbox_destinations: vec!["scheduler".to_owned()],
        }],
        capacity_reservations: vec![CapacityReservationRequest {
            reservation_id: res_id,
            pool_id,
            execution_owner: Some(ExecutionOwner::AgentInvocation {
                invocation_id: inv_id,
                attempt_id: None,
            }),
            owner_type: "agent_invocation".to_owned(),
            owner_id: inv_id.to_string(),
            admission_class: Some(AdmissionClass::Work),
            amount: 1,
        }],
        budget_reservations: vec![BudgetReservationRequest {
            action_id,
            model_id: "claude-haiku-5-5".to_owned(),
            pricing_version: "pv-1".to_owned(),
            pricing_status: PricingCheckStatus::Valid,
            max_request_hash: Sha256Digest::from_bytes(b"req"),
            charges: vec![BudgetAccountCharge {
                account_id,
                max_micros: MicrosAmount::new(500),
            }],
        }],
    })
}

fn prepare_prerequisites(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
) -> Result<(ProjectId, PoolId, AccountId), StoreError> {
    store.register_principal("principal-louis", "user", "user:louis")?;
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let pool_params = ExecutionPoolParams::default();
    let pool_id = pool_params.pool_id;
    store.configure_execution_pool(pool_params)?;
    let account_id = AccountId::from_uuid(ids.new_uuid());
    store.upsert_budget_account(
        account_id,
        BudgetScopeType::Project,
        &project_id.to_string(),
        "USD",
        MicrosAmount::new(10_000),
    )?;

    let authority = store.active_authority_generation()?;
    let proj_cmd = CommitCommandRequest {
        actor_id: ActorId::new("user:louis")?,
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: project_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateProject".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Project,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::CreateProject {
            project_id,
            name: "proj-atomic".to_owned(),
            weight: 1,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    };
    store.commit_command(proj_cmd)?;
    Ok((project_id, pool_id, account_id))
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario complet C0/C1/C2 vérifiant rollback/rejeu idempotent sur les 5 tables"
)]
fn test_t_f002_001_c0_c1_c2_atomic_commit_and_cutpoints() {
    let dir = local_test_dir("f002-001").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0001));
    let ctrl = CutpointController::new();
    let (_oracle_dir, oracle) = ExternalEffectOracle::create_in_temp().unwrap();

    let store = UnleashStore::open(build_test_config(
        dir.path(),
        Arc::clone(&clock),
        Arc::clone(&ids),
        ctrl.clone(),
    ))
    .unwrap();
    let (project_id, pool_id, account_id) = prepare_prerequisites(&store, &ids).unwrap();
    let authority = store.active_authority_generation().unwrap();

    // C0 : échec avant ouverture de transaction -> aucune ligne, 0 effet externe
    let mission_c0 = MissionId::from_uuid(ids.new_uuid());
    let action_c0 = ids.new_action_id();
    let inv_c0 = ids.new_invocation_id();
    seed_action_and_invocation(&store, authority, &ids, project_id, action_c0, inv_c0).unwrap();
    ctrl.set_action_on_hit(
        Cutpoint::C0,
        ctrl.hit_count(Cutpoint::C0) + 1,
        unleash_testkit::CutpointAction::Fail("c0"),
    )
    .unwrap();
    let cmd_c0 = ids.new_command_id();
    let req_c0 = setup_command(&SetupCommandArgs {
        authority,
        ids: &ids,
        project_id,
        mission_id: mission_c0,
        action_id: action_c0,
        inv_id: inv_c0,
        pool_id,
        account_id,
        command_id: cmd_c0,
    })
    .unwrap();
    assert!(store.commit_command(req_c0).is_err());
    assert_eq!(oracle.count().unwrap(), 0);
    ctrl.clear(Cutpoint::C0);

    // C1 : échec dans la transaction avant COMMIT -> rollback atomique complet
    let mission_c1 = MissionId::from_uuid(ids.new_uuid());
    let action_c1 = ids.new_action_id();
    let inv_c1 = ids.new_invocation_id();
    seed_action_and_invocation(&store, authority, &ids, project_id, action_c1, inv_c1).unwrap();
    ctrl.set_action_on_hit(
        Cutpoint::C1,
        ctrl.hit_count(Cutpoint::C1) + 1,
        unleash_testkit::CutpointAction::Fail("c1"),
    )
    .unwrap();
    let cmd_c1 = ids.new_command_id();
    let req_c1 = setup_command(&SetupCommandArgs {
        authority,
        ids: &ids,
        project_id,
        mission_id: mission_c1,
        action_id: action_c1,
        inv_id: inv_c1,
        pool_id,
        account_id,
        command_id: cmd_c1,
    })
    .unwrap();
    assert!(store.commit_command(req_c1).is_err());
    assert_eq!(oracle.count().unwrap(), 0);
    store
        .read_snapshot(|conn, _| {
            let missions: i64 = conn.query_row(
                "SELECT COUNT(*) FROM missions WHERE mission_id = ?1",
                [mission_c1.to_string()],
                |r| r.get(0),
            )?;
            let receipts: i64 = conn.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_id = ?1",
                [cmd_c1.to_string()],
                |r| r.get(0),
            )?;
            let events: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE aggregate_id = ?1",
                [mission_c1.to_string()],
                |r| r.get(0),
            )?;
            let outbox: i64 = conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE event_id IN (SELECT event_id FROM events WHERE aggregate_id = ?1)",
                [mission_c1.to_string()],
                |r| r.get(0),
            )?;
            let budgets: i64 = conn.query_row(
                "SELECT COUNT(*) FROM budget_reservations WHERE action_id = ?1",
                [action_c1.to_string()],
                |r| r.get(0),
            )?;
            assert_eq!((missions, receipts, events, outbox, budgets), (0, 0, 0, 0, 0));
            Ok(())
        })
        .unwrap();
    ctrl.clear(Cutpoint::C1);

    // C2 : échec après COMMIT et avant retour client -> toutes les 5 tables présentes et rejeu idempotent
    let mission_c2 = MissionId::from_uuid(ids.new_uuid());
    let action_c2 = ids.new_action_id();
    let inv_c2 = ids.new_invocation_id();
    seed_action_and_invocation(&store, authority, &ids, project_id, action_c2, inv_c2).unwrap();
    ctrl.set_action_on_hit(
        Cutpoint::C2,
        ctrl.hit_count(Cutpoint::C2) + 1,
        unleash_testkit::CutpointAction::Fail("c2"),
    )
    .unwrap();
    let cmd_c2 = ids.new_command_id();
    let req_c2 = setup_command(&SetupCommandArgs {
        authority,
        ids: &ids,
        project_id,
        mission_id: mission_c2,
        action_id: action_c2,
        inv_id: inv_c2,
        pool_id,
        account_id,
        command_id: cmd_c2,
    })
    .unwrap();
    assert!(store.commit_command(req_c2.clone()).is_err());
    ctrl.clear(Cutpoint::C2);

    // Même commande rejouée après perte de réponse -> IdempotentReplay sans doublon
    let replay = store.commit_command(req_c2).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    store
        .read_snapshot(|conn, _| {
            let missions: i64 = conn.query_row(
                "SELECT COUNT(*) FROM missions WHERE mission_id = ?1",
                [mission_c2.to_string()],
                |r| r.get(0),
            )?;
            let receipts: i64 = conn.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_id = ?1",
                [cmd_c2.to_string()],
                |r| r.get(0),
            )?;
            let events: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE aggregate_id = ?1",
                [mission_c2.to_string()],
                |r| r.get(0),
            )?;
            let outbox: i64 = conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE event_id IN (SELECT event_id FROM events WHERE aggregate_id = ?1)",
                [mission_c2.to_string()],
                |r| r.get(0),
            )?;
            let budgets: i64 = conn.query_row(
                "SELECT COUNT(*) FROM budget_reservations WHERE action_id = ?1",
                [action_c2.to_string()],
                |r| r.get(0),
            )?;
            assert_eq!((missions, receipts, events, outbox, budgets), (1, 1, 1, 1, 1));
            Ok(())
        })
        .unwrap();
}

const ENV_STORE_DIR: &str = "UNLEASH_T001_STORE_DIR";
const ENV_CMD_ID: &str = "UNLEASH_T001_CMD_ID";

#[test]
fn t001_child_crash_entrypoint() {
    let Ok(Some(ctx)) = ChildCrashContext::from_env() else {
        return;
    };
    let store_dir = std::env::var_os(ENV_STORE_DIR).unwrap();
    let cmd_id: CommandId = std::env::var(ENV_CMD_ID).unwrap().parse().unwrap();

    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0x1122_3344));
    let setup_store = UnleashStore::open(build_test_config(
        std::path::Path::new(&store_dir),
        Arc::clone(&clock),
        ids.clone(),
        CutpointController::new(),
    ))
    .unwrap();
    let (project_id, pool_id, account_id) = prepare_prerequisites(&setup_store, &ids).unwrap();
    let authority = setup_store.active_authority_generation().unwrap();
    let mission_id = MissionId::from_uuid(ids.new_uuid());
    let action_id = ids.new_action_id();
    let inv_id = ids.new_invocation_id();
    seed_action_and_invocation(&setup_store, authority, &ids, project_id, action_id, inv_id)
        .unwrap();
    setup_store.shutdown().unwrap();

    let ctrl = CutpointController::new();
    let _bridge = ctx.arm_controller(&ctrl).unwrap();
    let store = UnleashStore::open(build_test_config(
        std::path::Path::new(&store_dir),
        clock,
        ids.clone(),
        ctrl,
    ))
    .unwrap();
    let req = setup_command(&SetupCommandArgs {
        authority,
        ids: &ids,
        project_id,
        mission_id,
        action_id,
        inv_id,
        pool_id,
        account_id,
        command_id: cmd_id,
    })
    .unwrap();
    let _ = store.commit_command(req);
    ctx.oracle()
        .record_effect_detailed(NewExternalEffect {
            effect_key: cmd_id.to_string(),
            kind: "after_commit_dispatch".to_owned(),
            timestamp_ms: 1000,
            action_id: Some(action_id.to_string()),
            receipt_lost: false,
        })
        .unwrap();
}

#[test]
fn test_t_f002_001_sigkill_at_c1_and_c2_real_child() {
    // Crash SIGKILL au point C1 (avant COMMIT) : rien n'est persisté, oracle == 0
    let db_dir_c1 = local_test_dir("f002-001-sigc1").unwrap();
    let (_ora_dir_c1, oracle_c1) = ExternalEffectOracle::create_in_temp().unwrap();
    let cmd_id_c1 = CommandId::from_u128(101);
    let mut spec_c1 = ChildCrashHarness::for_current_exe_test(
        "t001_child_crash_entrypoint",
        Cutpoint::C1,
        CrashKillMode::Sigkill,
        oracle_c1.dir().to_path_buf(),
    )
    .unwrap();
    spec_c1
        .env
        .push((ENV_STORE_DIR.into(), db_dir_c1.path().display().to_string()));
    spec_c1.env.push((ENV_CMD_ID.into(), cmd_id_c1.to_string()));
    let outcome_c1 = ChildCrashHarness::run_until_crash(&spec_c1).unwrap();
    assert!(outcome_c1.is_sigkill());
    assert_eq!(oracle_c1.count().unwrap(), 0);

    let reopened_c1 = UnleashStore::open(StoreConfig::new(db_dir_c1.path())).unwrap();
    let receipt_count_c1: i64 = reopened_c1
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_id = ?1",
                [cmd_id_c1.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(receipt_count_c1, 0);

    // Crash SIGKILL au point C2 (après COMMIT, avant retour client / effet) : reçu durable présent, oracle == 0
    let db_dir_c2 = local_test_dir("f002-001-sigc2").unwrap();
    let (_ora_dir_c2, oracle_c2) = ExternalEffectOracle::create_in_temp().unwrap();
    let cmd_id_c2 = CommandId::from_u128(202);
    let mut spec_c2 = ChildCrashHarness::for_current_exe_test(
        "t001_child_crash_entrypoint",
        Cutpoint::C2,
        CrashKillMode::Sigkill,
        oracle_c2.dir().to_path_buf(),
    )
    .unwrap();
    spec_c2
        .env
        .push((ENV_STORE_DIR.into(), db_dir_c2.path().display().to_string()));
    spec_c2.env.push((ENV_CMD_ID.into(), cmd_id_c2.to_string()));
    let outcome_c2 = ChildCrashHarness::run_until_crash(&spec_c2).unwrap();
    assert!(outcome_c2.is_sigkill());
    assert_eq!(oracle_c2.count().unwrap(), 0);

    let reopened_c2 = UnleashStore::open(StoreConfig::new(db_dir_c2.path())).unwrap();
    let receipt_count_c2: i64 = reopened_c2
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_id = ?1",
                [cmd_id_c2.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(receipt_count_c2, 1);
}
