// T-F002-008 / AC-F002-008 / REQ-PERSIST-008
//! Réservations atomiques budgets multi-comptes, coûts inconnus et pool de capacité 5/50.

use std::sync::{Arc, Barrier};

use serde_json::Map;
use unleash_domain::{
    AccountId, ActionId, ActorId, AdmissionClass, AgentId, AgentLevel, AgentState, CommandId,
    IdGeneratorPort, InvocationId, MicrosAmount, PoolId, ProjectId, RequestId, ReservationId,
    Revision, Sha256Digest,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    AggregateKind, BudgetAccountCharge, BudgetReservationRequest, BudgetScopeType,
    CapacityReservationRequest, CommandMutation, CommitCommandRequest, ControlReserveMode,
    ExecutionOwner, ExecutionPoolParams, PricingCheckStatus, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, local_test_dir};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Suite complète T-F002-008 : budgets multi-comptes, unknown non libéré, dépassement réel et pool 5/50 soft/hard"
)]
#[expect(
    clippy::similar_names,
    reason = "Identifiants distincts des deux comptes/projets en concurrence dans la suite budgétaire"
)]
fn test_t_f002_008_atomic_multi_account_budgets_unknown_and_capacity_pool() {
    let dir = local_test_dir("f002-008").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0008));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.id_generator = ids.clone();
    let store = Arc::new(UnleashStore::open(cfg).unwrap());

    let authority = store.active_authority_generation().unwrap();
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
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
                name: "proj-budget".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Contrôle incohérent : hard reserve avec C=1 refusé sans ajustement silencieux
    let incoherent = store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id: PoolId::from_u128(1),
            requested_limit: 1,
            control_reserve_mode: ControlReserveMode::Hard,
            control_reserved_units: 1,
            control_burst_limit: 2,
            checkpoint_escalation_s: 60,
        })
        .unwrap_err();
    assert!(matches!(
        incoherent,
        StoreError::ControlReserveIncoherent(_)
    ));
    assert_eq!(
        incoherent.reason_code(),
        Some(ReasonCode::ControlReserveIncoherent)
    );

    // Configurer pool execution C=5
    let pool_id = PoolId::from_u128(1);
    store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id,
            ..ExecutionPoolParams::default()
        })
        .unwrap();

    // Comptes global=1000, project=1000, provider=600 (USD) :
    // Deux actions demandant chacune 400 -> 400+400 = 800 > provider(600), donc UNE SEULE passe !
    let acc_global = AccountId::from_u128(1);
    let acc_proj = AccountId::from_u128(2);
    let acc_prov = AccountId::from_u128(3);
    store
        .upsert_budget_account(
            acc_global,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1000),
        )
        .unwrap();
    store
        .upsert_budget_account(
            acc_proj,
            BudgetScopeType::Project,
            &project_id.to_string(),
            "USD",
            MicrosAmount::new(1000),
        )
        .unwrap();
    store
        .upsert_budget_account(
            acc_prov,
            BudgetScopeType::Provider,
            "anthropic",
            "USD",
            MicrosAmount::new(600),
        )
        .unwrap();

    let act1 = ActionId::from_u128(1001);
    let act2 = ActionId::from_u128(1002);
    let make_budget_cmd = |action_id: ActionId, cmd_num: u128| CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(cmd_num),
            command_id: CommandId::from_u128(cmd_num),
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: action_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateAction".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Action,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::CreateAction {
            action_id,
            origin_kind: "control_plane".to_owned(),
            control_principal_id: Some("p-sys".to_owned()),
            run_id: None,
            step_id: None,
            invocation_id: None,
            attempt_id: None,
            replay_worker_id: None,
            action_type: "model_call".to_owned(),
            payload_hash: Sha256Digest::from_bytes(b"act"),
            reconcile_strategy: "observe_then_decide".to_owned(),
            policy_revision: 1,
            resource_lease_id: None,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![BudgetReservationRequest {
            action_id,
            model_id: "claude-haiku-5-5".to_owned(),
            pricing_version: "pv1".to_owned(),
            pricing_status: PricingCheckStatus::Valid,
            max_request_hash: Sha256Digest::from_bytes(b"maxreq"),
            charges: vec![
                BudgetAccountCharge {
                    account_id: acc_global,
                    max_micros: MicrosAmount::new(400),
                },
                BudgetAccountCharge {
                    account_id: acc_proj,
                    max_micros: MicrosAmount::new(400),
                },
                BudgetAccountCharge {
                    account_id: acc_prov,
                    max_micros: MicrosAmount::new(400),
                },
            ],
        }],
    };
    store
        .register_principal("p-sys", "system", "system:broker")
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let s1 = Arc::clone(&store);
    let b1 = Arc::clone(&barrier);
    let r1_cmd = make_budget_cmd(act1, 9001);
    let t1 = std::thread::spawn(move || {
        b1.wait();
        s1.commit_command(r1_cmd)
    });
    let s2 = Arc::clone(&store);
    let b2 = Arc::clone(&barrier);
    let r2_cmd = make_budget_cmd(act2, 9002);
    let t2 = std::thread::spawn(move || {
        b2.wait();
        s2.commit_command(r2_cmd)
    });

    let res1 = t1.join().unwrap();
    let res2 = t2.join().unwrap();
    let (winner_act, loser_res) = match (res1, res2) {
        (Ok(_), Err(e)) => (act1, e),
        (Err(e), Ok(_)) => (act2, e),
        other => panic!("exactement 1 gagnant et 1 perdant attendus, observé: {other:?}"),
    };
    assert!(matches!(loser_res, StoreError::BudgetExhausted { .. }));

    // Vérifier ZÉRO réservation partielle sur global/project/provider pour le perdant
    assert_eq!(
        store.budget_account(acc_global).unwrap().reserved_micros,
        MicrosAmount::new(400)
    );
    assert_eq!(
        store.budget_account(acc_proj).unwrap().reserved_micros,
        MicrosAmount::new(400)
    );
    assert_eq!(
        store.budget_account(acc_prov).unwrap().reserved_micros,
        MicrosAmount::new(400)
    );

    // Tarifs absents / périmés (Q-14)
    let mut act_missing = make_budget_cmd(ActionId::from_u128(1003), 9003);
    act_missing
        .budget_reservations
        .first_mut()
        .unwrap()
        .pricing_status = PricingCheckStatus::Missing;
    assert!(matches!(
        store.commit_command(act_missing),
        Err(StoreError::PricingUnknown { .. })
    ));

    let mut act_stale_block = make_budget_cmd(ActionId::from_u128(1004), 9004);
    act_stale_block
        .budget_reservations
        .first_mut()
        .unwrap()
        .pricing_status = PricingCheckStatus::Stale {
        stale_policy_warn: false,
    };
    assert!(matches!(
        store.commit_command(act_stale_block),
        Err(StoreError::PricingStale { .. })
    ));

    // Coût inconnu conserve sa réserve et n'est jamais libéré automatiquement
    store.mark_action_budget_unknown(winner_act).unwrap();
    assert_eq!(
        store.budget_account(acc_prov).unwrap().reserved_micros,
        MicrosAmount::new(400)
    );

    // Règlement réel dépassant le plafond (700 > 600) : enregistré et bloque futures admissions
    store
        .settle_action_budget(winner_act, MicrosAmount::new(700))
        .unwrap();
    let snap_prov = store.budget_account(acc_prov).unwrap();
    assert_eq!(snap_prov.spent_micros, MicrosAmount::new(700));
    assert_eq!(snap_prov.reserved_micros, MicrosAmount::ZERO);
    let mut act_after_overage = make_budget_cmd(ActionId::from_u128(1005), 9005);
    act_after_overage
        .budget_reservations
        .first_mut()
        .unwrap()
        .charges
        .get_mut(2)
        .unwrap()
        .max_micros = MicrosAmount::new(1);
    assert!(matches!(
        store.commit_command(act_after_overage),
        Err(StoreError::BudgetExhausted { .. })
    ));

    // R7 : Réadmission d'une même invocation après waiting_durable (ancienne réservation released conservée, nouvelle held créée)
    let agent_id = AgentId::from_u128(5001);
    let inv_id = InvocationId::from_u128(6001);
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(9100),
                command_id: CommandId::from_u128(9100),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: agent_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateAgent".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Agent,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateAgent {
                agent_id,
                level: AgentLevel::N,
                organization_parent_id: None,
                created_by_agent_id: None,
                project_id: Some(project_id),
                state: AgentState::Eligible,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Créer les lignes parentes session/context_manifest/effective_bundle/event nécessaires à invocations
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let sid = unleash_domain::SessionId::from_u128(7001).to_string();
            let mid = unleash_domain::ManifestId::from_u128(7002).to_string();
            let obj_hash = Sha256Digest::from_bytes(b"ctx").to_string();
            let b_hash = Sha256Digest::from_bytes(b"bundle").to_string();
            let ev_id = uuid::Uuid::from_u128(7003).to_string();
            let rcpt_id = unleash_domain::ReceiptId::from_u128(7004).to_string();
            tx.execute(
                "INSERT INTO object_records(object_hash, object_generation, relative_path, byte_length, media_type, state, created_at_ms) VALUES (?1, 1, 'blobs/ctx', 3, 'application/json', 'live', ?2)",
                rusqlite::params![obj_hash, now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO sessions(session_id, agent_id, revision, state, created_at_ms) VALUES (?1, ?2, 0, 'open', ?3)",
                rusqlite::params![sid, agent_id.to_string(), now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO context_manifests(manifest_id, session_id, base_session_revision, content_hash, schema_version, created_at_ms) VALUES (?1, ?2, 0, ?3, 1, ?4)",
                rusqlite::params![mid, sid, obj_hash, now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO effective_bundles(bundle_hash, schema_version, role_version_id, requested_model_profile_version_id, allowed_model_policy_hash, initial_context_manifest_id, scope_id, scope_revision, content_object_hash) VALUES (?1, 1, 'rv1', 'mp1', ?2, ?3, 's1', 0, ?4)",
                rusqlite::params![b_hash, obj_hash, mid, obj_hash],
            )?;
            tx.execute(
                "INSERT INTO command_receipts(actor_id, command_id, authority_generation, receipt_id, request_id, command_type, command_digest, aggregate_id, state, response_state, response_json, response_retained_until_ms, committed_at_ms) VALUES ('system', ?1, ?2, ?3, ?4, 'Trigger', 'd', ?5, 'completed', 'full', '{}', ?6, ?7)",
                rusqlite::params![ev_id, authority.to_string(), rcpt_id, ev_id, inv_id.to_string(), now.as_i64() + 1000, now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO events(event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal, event_type, schema_version, actor_ref, actor_id, occurred_at_ms, recorded_at_ms, payload_json, payload_state, payload_hash, sensitivity, causation_kind, causation_id, causation_receipt_id, correlation_id) VALUES (?1, ?2, 'invocation', ?3, 1, 0, 'InvocationQueued', 1, 'system', 'system', ?4, ?4, '{}', 'full', ?5, 'project', 'command', ?6, ?6, ?1)",
                rusqlite::params![ev_id, uuid::Uuid::from_u128(1).to_string(), inv_id.to_string(), now.as_i64(), obj_hash, rcpt_id],
            )?;
            tx.execute(
                "INSERT INTO invocations(invocation_id, agent_id, session_id, trigger_event_id, effective_bundle_hash, context_manifest_id, state, admission_class, revision, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 'work', 0, ?7)",
                rusqlite::params![inv_id.to_string(), agent_id.to_string(), sid, ev_id, b_hash, mid, now.as_i64()],
            )?;
            Ok(())
        })
        .unwrap();

    let res_first = ReservationId::from_u128(8001);
    let res_second = ReservationId::from_u128(8002);
    let admit_cmd = |res_id: ReservationId, cmd_num: u128| CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(cmd_num),
            command_id: CommandId::from_u128(cmd_num),
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: authority.as_uuid(),
            expected_revision: Revision::new(0),
            command_type: "AdmitInvocation".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Daemon,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![],
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
        budget_reservations: vec![],
    };

    // 1re admission
    let mut cmd1 = admit_cmd(res_first, 9201);
    cmd1.envelope.expected_revision =
        Revision::new(store.daemon_state().unwrap().revision.as_u64());
    store.commit_command(cmd1).unwrap();
    // Passage waiting_durable -> libère slot (ligne conservée en released)
    store.release_capacity_reservation(res_first).unwrap();

    // 2e admission du même invocation_id -> nouvelle ligne held, ancienne ligne released conservée !
    let mut cmd2 = admit_cmd(res_second, 9202);
    cmd2.envelope.expected_revision =
        Revision::new(store.daemon_state().unwrap().revision.as_u64());
    store.commit_command(cmd2).unwrap();

    let counts: (i64, i64) = store
        .read_snapshot(|conn, _| {
            let rel: i64 = conn.query_row(
                "SELECT COUNT(*) FROM capacity_reservations WHERE owner_id = ?1 AND state = 'released'",
                [inv_id.to_string()],
                |r| r.get(0),
            )?;
            let held: i64 = conn.query_row(
                "SELECT COUNT(*) FROM capacity_reservations WHERE owner_id = ?1 AND state = 'held'",
                [inv_id.to_string()],
                |r| r.get(0),
            )?;
            Ok((rel, held))
        })
        .unwrap();
    assert_eq!(counts, (1, 1));

    // Baisse de limite sous occupation : occupation=2, baisser requested_limit à 1 -> effective reste 5, admissions_frozen=1
    store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id,
            requested_limit: 5,
            ..ExecutionPoolParams::default()
        })
        .unwrap();
    let res_third = ReservationId::from_u128(8003);
    // A historical reservation which has provably never started: real queued
    // invocation, no physical attempt and no action. Unknown replay owners are
    // no longer considered evidence that execution has stopped (REQ-AGENT-004).
    let drain_inv = InvocationId::from_u128(8803);
    store.execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
        let old=inv_id.to_string();
        let agent=AgentId::from_u128(8803).to_string();
        let session=unleash_domain::SessionId::from_u128(8803).to_string();
        let manifest=unleash_domain::ManifestId::from_u128(8803).to_string();
        tx.execute("INSERT INTO agents(agent_id,level,project_id,state,revision) SELECT ?1,'N',a.project_id,'eligible',0 FROM agents a JOIN invocations i ON i.agent_id=a.agent_id WHERE i.invocation_id=?2",rusqlite::params![agent,old])?;
        tx.execute("INSERT INTO sessions(session_id,agent_id,revision,state,created_at_ms) SELECT ?1,?2,0,'open',s.created_at_ms FROM sessions s JOIN invocations i ON i.session_id=s.session_id WHERE i.invocation_id=?3",rusqlite::params![session,agent,old])?;
        tx.execute("INSERT INTO context_manifests(manifest_id,session_id,base_session_revision,content_hash,schema_version,created_at_ms) SELECT ?1,?2,0,m.content_hash,1,m.created_at_ms FROM context_manifests m JOIN invocations i ON i.context_manifest_id=m.manifest_id WHERE i.invocation_id=?3",rusqlite::params![manifest,session,old])?;
        tx.execute("INSERT INTO invocations(invocation_id,agent_id,session_id,trigger_event_id,effective_bundle_hash,context_manifest_id,state,admission_class,revision,created_at_ms) SELECT ?1,?2,?3,trigger_event_id,effective_bundle_hash,?4,'queued','work',0,created_at_ms FROM invocations WHERE invocation_id=?5",rusqlite::params![drain_inv.to_string(),agent,session,manifest,old])?;
        Ok(())
    }).unwrap();
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_u128(9203),
                command_id: CommandId::from_u128(9203),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: authority.as_uuid(),
                expected_revision: Revision::new(store.daemon_state().unwrap().revision.as_u64()),
                command_type: "AdmitReplayWorker".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Daemon,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::NoopDaemonBump,
            events: vec![],
            capacity_reservations: vec![CapacityReservationRequest {
                reservation_id: res_third,
                pool_id,
                execution_owner: Some(ExecutionOwner::AgentInvocation {
                    invocation_id: drain_inv,
                    attempt_id: None,
                }),
                owner_type: "agent_invocation".to_owned(),
                owner_id: drain_inv.to_string(),
                admission_class: Some(AdmissionClass::Work),
                amount: 1,
            }],
            budget_reservations: vec![],
        })
        .unwrap();
    let lower_state = store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id,
            requested_limit: 1,
            control_reserve_mode: ControlReserveMode::Soft,
            control_reserved_units: 1,
            control_burst_limit: 2,
            checkpoint_escalation_s: 60,
        })
        .unwrap();
    assert_eq!(lower_state.current_occupation, 2);
    assert!(lower_state.draining_to_limit);
    assert_eq!(lower_state.effective_limit, 5);
    assert!(lower_state.admissions_frozen);

    store.release_capacity_reservation(res_third).unwrap();
    let drained_state = store.execution_pool_state(pool_id).unwrap();
    assert_eq!(drained_state.effective_limit, 1);
    assert!(!drained_state.draining_to_limit);
    assert!(!drained_state.admissions_frozen);
}
