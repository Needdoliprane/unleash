// T-F018-004 / AC-F018-004 / REQ-REC-004
//! `InspectRecovery`, `ReconcileAction` et `ResolveActionOutcome` :
//! action maintenue `outcome_unknown` après disposition et `EffectOutcomeResolved`
//! écrit atomiquement sur le Run bloqué dans la même transaction.

use std::sync::Arc;

use serde_json::Map;
use unleash_domain::{
    ActionId, ActionState, ActorId, CommandId, CompletionPolicy, IdGeneratorPort, MissionId,
    MissionState, ProjectId, ReconcileOutcome, RecoveryClassification, RequestId, Revision, RunId,
    RunState, Sha256Digest,
};
use unleash_protocol::{CommandEnvelope, ErrorCode, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    ActionOutcomeDisposition, AggregateKind, CallerAuthority, CallerRole, CommandMutation,
    CommitCommandRequest, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

fn make_envelope(
    ids: &DeterministicIdGenerator,
    auth: unleash_domain::AuthorityGeneration,
    command_id: CommandId,
    project_id: Option<ProjectId>,
    aggregate_id: uuid::Uuid,
    expected_revision: Revision,
    command_type: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(ids.new_uuid()),
        command_id,
        authority_generation: auth,
        actor_ref: Some("user:louis".to_owned()),
        project_id,
        aggregate_id,
        expected_revision,
        command_type: command_type.to_owned(),
        payload: Map::new(),
    }
}

#[expect(clippy::unwrap_used, reason = "Helper d'assertion de test")]
fn commit_ok(store: &UnleashStore, req: CommitCommandRequest) -> unleash_protocol::CommandReceipt {
    match store.commit_command(req).unwrap() {
        unleash_store::CommitOutcome::Committed(r)
        | unleash_store::CommitOutcome::IdempotentReplay(r) => r,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Préparation de fixture Run bloqué + Action outcome_unknown pour T-F018-004"
)]
fn seed_blocked_run_with_unknown_action(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
    auth: unleash_domain::AuthorityGeneration,
    admin: &ActorId,
    project_id: ProjectId,
    mission_id: MissionId,
    wf_ver: &str,
) -> (RunId, ActionId) {
    let run_id = RunId::from_uuid(ids.new_uuid());
    let bundle_hash = Sha256Digest::from_bytes(b"bundle-004");
    commit_ok(
        store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                run_id.as_uuid(),
                Revision::INITIAL,
                "CreateWorkflowRun",
            ),
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateWorkflowRun {
                run_id,
                mission_id,
                workflow_version_id: wf_ver.to_owned(),
                purpose: "operational".to_owned(),
                candidate_ref: None,
                predecessor_run_id: None,
                selection_reason: "primary".to_owned(),
                effective_bundle_hash: bundle_hash,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    commit_ok(
        store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                run_id.as_uuid(),
                Revision::new(1),
                "FixtureTransitionRun",
            ),
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::TransitionWorkflowRun {
                run_id,
                target_state: RunState::Running,
                ended_at_ms: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    commit_ok(
        store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                run_id.as_uuid(),
                Revision::new(2),
                "BlockRun",
            ),
            target_aggregate: AggregateKind::WorkflowRun,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::TransitionWorkflowRun {
                run_id,
                target_state: RunState::Blocked,
                ended_at_ms: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    let act_id = ActionId::from_uuid(ids.new_uuid());
    commit_ok(
        store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_id.as_uuid(),
                Revision::INITIAL,
                "CreateAction",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateGovernedAction {
                action_id: act_id,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some(format!("run:{run_id}")),
                run_id: Some(run_id),
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "git_publish".to_owned(),
                payload_hash: Sha256Digest::from_bytes(b"git-push-main"),
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::OutcomeUnknown,
                subject: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    (run_id, act_id)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario F018-004 complet : InspectRecovery, ReconcileAction et ResolveActionOutcome atomique"
)]
fn test_t_f018_004_reconcile_action_and_resolve_action_outcome_atomic_tx() {
    let dir = local_test_dir("f018-004").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0004));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock;
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg).unwrap();
    store
        .register_principal("p-louis", "user", "user:louis")
        .unwrap();
    let auth = store.active_authority_generation().unwrap();
    let admin = ActorId::new("user:louis").unwrap();
    let human = CallerAuthority {
        actor_id: admin.clone(),
        role: CallerRole::HumanOperator,
    };

    let project_id = ProjectId::from_uuid(ids.new_uuid());
    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                project_id.as_uuid(),
                Revision::INITIAL,
                "CreateProject",
            ),
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id,
                name: "proj-004".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    let mission_id = MissionId::from_uuid(ids.new_uuid());
    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                mission_id.as_uuid(),
                Revision::INITIAL,
                "CreateMission",
            ),
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateMission {
                mission_id,
                goal_id: None,
                project_id,
                owner_principal_id: Some("p-louis".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Ready,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    let wf_ver = "wf-004-v1".to_owned();
    let bh = Sha256Digest::from_bytes(b"bundle-004").as_prefixed_str();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, {
            let wf_ver = wf_ver.clone();
            move |tx, now| {
                tx.execute(
                    "INSERT INTO workflow_versions(workflow_version_id, schema_version, visibility, content_hash, created_at_ms)
                     VALUES (?1, 1, 'published', ?2, ?3)",
                    rusqlite::params![wf_ver, bh, now.as_i64()],
                )?;
                Ok(())
            }
        })
        .unwrap();

    let (run_1, act_1) = seed_blocked_run_with_unknown_action(
        &store, &ids, auth, &admin, project_id, mission_id, &wf_ver,
    );
    let (run_2, act_2) = seed_blocked_run_with_unknown_action(
        &store, &ids, auth, &admin, project_id, mission_id, &wf_ver,
    );

    // 1. InspectRecovery classe l'item comme Unknown + liste run bloqué
    let rec = store.inspect_recovery().unwrap();
    assert!(rec.blocked_runs.contains(&run_1));
    let item1 = rec
        .items
        .iter()
        .find(|i| i.action_id == act_1)
        .expect("item act_1");
    assert_eq!(item1.classification, RecoveryClassification::Unknown);
    assert_eq!(item1.state, ActionState::OutcomeUnknown);
    assert_eq!(
        item1.allowed_operations,
        vec![
            "ReconcileAction".to_owned(),
            "ResolveActionOutcome".to_owned()
        ]
    );

    // 2. Appelant sans autorité -> CapabilityDenied / OPERATION_NOT_AUTHORIZED
    let unauth = store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("agent:n-worker").unwrap(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_1.as_uuid(),
                Revision::new(1),
                "ResolveActionOutcome",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionOutcome {
                action_id: act_1,
                caller: CallerAuthority {
                    actor_id: ActorId::new("agent:n-worker").unwrap(),
                    role: CallerRole::Agent,
                },
                disposition: ActionOutcomeDisposition::AcceptRiskRetry,
                reason: "essai non autorisé".to_owned(),
                evidence_ref: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(unauth.error_code(), ErrorCode::CapabilityDenied);
    assert_eq!(
        unauth.reason_code(),
        Some(ReasonCode::OperationNotAuthorized)
    );

    // 3. ResolveActionOutcome par opérateur habilité :
    //    - action RESTE `outcome_unknown` (jamais `observed_success`)
    //    - disposition enregistrée
    //    - `RecoveryDecisionRecorded` et `EffectOutcomeResolved` écrits dans la MÊME transaction
    //    - Run bloqué débloqué vers `running`
    let res_rcpt = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_1.as_uuid(),
                Revision::new(1),
                "ResolveActionOutcome",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionOutcome {
                action_id: act_1,
                caller: human.clone(),
                disposition: ActionOutcomeDisposition::AcceptRiskRetry,
                reason: "vérifié manuellement hors bande, réautoriser nouvelle tentative"
                    .to_owned(),
                evidence_ref: Some("manual:check-1".to_owned()),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    let (act1_state, act1_disp, act1_rcpt_id, run1_state, ev_pair_cnt): (
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
    ) = store
        .read_snapshot(|conn, _| {
            let (s, d, r): (String, Option<String>, Option<String>) = conn.query_row(
                "SELECT state, disposition, disposition_receipt_id FROM actions WHERE action_id = ?1",
                [act_1.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let rs: String = conn.query_row(
                "SELECT state FROM workflow_runs WHERE run_id = ?1",
                [run_1.to_string()],
                |row| row.get(0),
            )?;
            let cnt: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events
                 WHERE causation_receipt_id = ?1
                   AND event_type IN ('RecoveryDecisionRecorded', 'EffectOutcomeResolved')",
                [res_rcpt.receipt_id.to_string()],
                |row| row.get(0),
            )?;
            Ok((s, d, r, rs, cnt))
        })
        .unwrap();
    assert_eq!(act1_state, "outcome_unknown");
    assert_eq!(act1_disp.as_deref(), Some("accept_risk_retry"));
    assert_eq!(
        act1_rcpt_id.as_deref(),
        Some(res_rcpt.receipt_id.to_string().as_str())
    );
    assert_eq!(run1_state, "running");
    assert_eq!(ev_pair_cnt, 2);

    // 4. ReconcileAction sur act_2 :
    //    - ConfirmedSuccess SANS preuve/receipt est rejeté
    let missing_proof = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_2.as_uuid(),
                Revision::new(1),
                "ReconcileAction",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ReconcileAction {
                action_id: act_2,
                caller: human.clone(),
                outcome: ReconcileOutcome::ConfirmedSuccess,
                evidence_digest: None,
                receipt_hash: None,
                reason: "sans preuve".to_owned(),
                expected_source_epoch: None,
                expected_source_fence: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(
        missing_proof.reason_code(),
        Some(ReasonCode::EvidenceUnavailable)
    );

    //    - ConfirmedSuccess AVEC preuve/receipt passe à `observed_success` et débloque `run_2`
    let reconcile_rcpt = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin,
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_2.as_uuid(),
                Revision::new(1),
                "ReconcileAction",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ReconcileAction {
                action_id: act_2,
                caller: human,
                outcome: ReconcileOutcome::ConfirmedSuccess,
                evidence_digest: Some(Sha256Digest::from_bytes(b"cas-proof-2")),
                receipt_hash: Some(Sha256Digest::from_bytes(b"cas-rcpt-2")),
                reason: "CAS observé côté dépôt".to_owned(),
                expected_source_epoch: None,
                expected_source_fence: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    let (act2_state, run2_state, resolved_cnt): (String, String, i64) = store
        .read_snapshot(|conn, _| {
            let s: String = conn.query_row(
                "SELECT state FROM actions WHERE action_id = ?1",
                [act_2.to_string()],
                |row| row.get(0),
            )?;
            let rs: String = conn.query_row(
                "SELECT state FROM workflow_runs WHERE run_id = ?1",
                [run_2.to_string()],
                |row| row.get(0),
            )?;
            let cnt: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events
                 WHERE causation_receipt_id = ?1
                   AND event_type IN ('ActionReconciled', 'EffectOutcomeResolved')",
                [reconcile_rcpt.receipt_id.to_string()],
                |row| row.get(0),
            )?;
            Ok((s, rs, cnt))
        })
        .unwrap();
    assert_eq!(act2_state, "observed_success");
    assert_eq!(run2_state, "running");
    assert_eq!(resolved_cnt, 2);
}
