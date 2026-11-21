// T-F018-002 / AC-F018-002 / REQ-REC-002
//! Recovery d'actions en vol, approbation humaine stricte (`ResolveActionApproval`)
//! et suspension/révocation locale de sujet (`SuspendSubject`/`ResumeSubject`/`RevokeSubject`).

use std::sync::Arc;

use serde_json::Map;
use unleash_domain::{
    ActionId, ActionState, ActorId, CommandId, CompletionPolicy, DaemonEpoch, Fence,
    IdGeneratorPort, MissionId, MissionState, ProjectId, RequestId, Revision, RunId, RunState,
    Sha256Digest,
};
use unleash_protocol::{CommandEnvelope, ErrorCode, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    ActionDispatchInput, AggregateKind, ApprovalDecision, CallerAuthority, CallerRole,
    CommandMutation, CommitCommandRequest, ReconcileOutcome, StoreConfig, SubjectKind,
    SubjectStatus, UnleashStore,
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
fn commit_ok(store: &UnleashStore, req: CommitCommandRequest) -> Revision {
    match store.commit_command(req).unwrap() {
        unleash_store::CommitOutcome::Committed(r)
        | unleash_store::CommitOutcome::IdempotentReplay(r) => r.new_revision,
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario F018-002 complet : recovery en vol, approbation humaine stricte et contrôle de sujets"
)]
fn test_t_f018_002_recovery_approval_and_subject_controls() {
    let dir = local_test_dir("f018-002").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0002));

    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock;
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg.clone()).unwrap();
    store
        .register_principal("p-louis", "user", "user:louis")
        .unwrap();
    let auth = store.active_authority_generation().unwrap();
    let admin = ActorId::new("user:louis").unwrap();
    let human_auth = CallerAuthority {
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
                name: "proj-f018-002".to_owned(),
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

    let wf_ver = "wf-v1".to_owned();
    let run_id = RunId::from_uuid(ids.new_uuid());
    let bundle_hash = Sha256Digest::from_bytes(b"bundle-f018-002");
    store
        .execute_in_writer_tx(unleash_store::WriteCriticality::Critical, {
            let wf_ver = wf_ver.clone();
            let bh = bundle_hash.as_prefixed_str();
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

    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
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
                workflow_version_id: wf_ver,
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
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
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

    // 1. Action en vol (dispatching -> in_flight sous daemon_epoch=1) puis crash / redémarrage même lignée
    let act_inflight = ActionId::from_uuid(ids.new_uuid());
    let payload_inflight = Sha256Digest::from_bytes(b"payload-inflight");
    let mut rev_inf = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_inflight.as_uuid(),
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
                action_id: act_inflight,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some(format!("project:{project_id}")),
                run_id: Some(run_id),
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "external_http_post".to_owned(),
                payload_hash: payload_inflight,
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::InFlight,
                subject: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    store
        .record_action_dispatch(ActionDispatchInput {
            dispatch_id: "disp-inflight-1".to_owned(),
            action_id: act_inflight,
            ordinal: 1,
            adapter_id: "http-adapter".to_owned(),
            request_hash: payload_inflight,
            provider_request_id: Some("prov-req-1".to_owned()),
            transport_state: "sent".to_owned(),
            receipt_hash: None,
        })
        .unwrap();

    // Redémarrage du store : epoch avance à 2, classification de reprise passe l'action en vol à `outcome_unknown`
    let store = store.restart_same_lineage().unwrap();
    assert_eq!(
        store.daemon_state().unwrap().daemon_epoch,
        DaemonEpoch::new(2)
    );
    let transitioned = store.reconcile_recovered_actions_after_restart().unwrap();
    assert_eq!(transitioned, vec![act_inflight]);
    rev_inf = Revision::new(rev_inf.as_u64() + 1);

    let report = store.inspect_recovery().unwrap();
    let item = report
        .items
        .iter()
        .find(|i| i.action_id == act_inflight)
        .expect("action en vol doit figurer dans InspectRecovery");
    assert_eq!(item.state, ActionState::OutcomeUnknown);
    assert!(matches!(
        item.classification,
        unleash_domain::RecoveryClassification::Unknown
    ));
    assert!(
        item.allowed_operations
            .contains(&"ReconcileAction".to_owned())
    );
    assert!(
        item.allowed_operations
            .contains(&"ResolveActionOutcome".to_owned())
    );

    // Un résultat tardif portant l'ancien daemon_epoch/fence=1 est rejeté StaleFence !
    let stale_callback_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_inflight.as_uuid(),
                rev_inf,
                "ReconcileAction",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ReconcileAction {
                action_id: act_inflight,
                caller: CallerAuthority {
                    actor_id: admin.clone(),
                    role: CallerRole::ReconcileCoordinator,
                },
                outcome: ReconcileOutcome::ConfirmedSuccess,
                evidence_digest: Some(Sha256Digest::from_bytes(b"late-proof")),
                receipt_hash: Some(Sha256Digest::from_bytes(b"late-rcpt")),
                reason: "late callback epoch 1".to_owned(),
                expected_source_epoch: Some(DaemonEpoch::new(1)),
                expected_source_fence: Some(Fence::new(0)),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(stale_callback_err.error_code(), ErrorCode::StaleFence);

    // 2. ResolveActionApproval : scope et digest exacts, humain uniquement, jamais agent/modèle
    let act_approve = ActionId::from_uuid(ids.new_uuid());
    let act_deny = ActionId::from_uuid(ids.new_uuid());
    let digest_approve = Sha256Digest::from_bytes(b"deploy-prod-payload");
    let digest_wrong = Sha256Digest::from_bytes(b"other-payload");

    for (aid, dig, scope) in [
        (act_approve, digest_approve, "scope:deploy:prod"),
        (
            act_deny,
            Sha256Digest::from_bytes(b"drop-table"),
            "scope:db:admin",
        ),
    ] {
        commit_ok(
            &store,
            CommitCommandRequest {
                actor_id: admin.clone(),
                envelope: make_envelope(
                    &ids,
                    auth,
                    ids.new_command_id(),
                    Some(project_id),
                    aid.as_uuid(),
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
                    action_id: aid,
                    origin_kind: "control_plane".to_owned(),
                    control_principal_id: Some("p-louis".to_owned()),
                    project_id: Some(project_id),
                    scope_key: Some(scope.to_owned()),
                    run_id: Some(run_id),
                    step_id: None,
                    invocation_id: None,
                    attempt_id: None,
                    replay_worker_id: None,
                    action_type: "guarded_deploy".to_owned(),
                    payload_hash: dig,
                    reconcile_strategy: "observe_then_decide".to_owned(),
                    policy_revision: 1,
                    resource_lease_id: None,
                    approval_scope: Some(scope.to_owned()),
                    initial_state: ActionState::ApprovalRequired,
                    subject: None,
                },
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            },
        );
    }

    // Rejet si décideur est un modèle ou un agent
    for forbidden_role in [CallerRole::Model, CallerRole::Agent] {
        let unauth_err = store
            .commit_command(CommitCommandRequest {
                actor_id: ActorId::new("agent:worker-1").unwrap(),
                envelope: make_envelope(
                    &ids,
                    auth,
                    ids.new_command_id(),
                    Some(project_id),
                    act_approve.as_uuid(),
                    Revision::new(1),
                    "ResolveActionApproval",
                ),
                target_aggregate: AggregateKind::Action,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::ResolveActionApproval {
                    action_id: act_approve,
                    caller: CallerAuthority {
                        actor_id: ActorId::new("agent:worker-1").unwrap(),
                        role: forbidden_role,
                    },
                    approval_scope: "scope:deploy:prod".to_owned(),
                    payload_digest: digest_approve,
                    decision: ApprovalDecision::Approve,
                    reason: "self approve".to_owned(),
                },
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })
            .unwrap_err();
        assert_eq!(unauth_err.error_code(), ErrorCode::CapabilityDenied);
        assert_eq!(
            unauth_err.reason_code(),
            Some(ReasonCode::OperationNotAuthorized)
        );
    }

    // Rejet si approval_scope ou payload_digest divergent
    let bad_scope_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_approve.as_uuid(),
                Revision::new(1),
                "ResolveActionApproval",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionApproval {
                action_id: act_approve,
                caller: human_auth.clone(),
                approval_scope: "scope:deploy:staging".to_owned(),
                payload_digest: digest_approve,
                decision: ApprovalDecision::Approve,
                reason: "wrong scope".to_owned(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(bad_scope_err.error_code(), ErrorCode::ContractViolation);

    let bad_digest_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_approve.as_uuid(),
                Revision::new(1),
                "ResolveActionApproval",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionApproval {
                action_id: act_approve,
                caller: human_auth.clone(),
                approval_scope: "scope:deploy:prod".to_owned(),
                payload_digest: digest_wrong,
                decision: ApprovalDecision::Approve,
                reason: "wrong digest".to_owned(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(bad_digest_err.error_code(), ErrorCode::ContractViolation);

    // Approbation valide -> authorized + approval_resolutions
    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_approve.as_uuid(),
                Revision::new(1),
                "ResolveActionApproval",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionApproval {
                action_id: act_approve,
                caller: human_auth.clone(),
                approval_scope: "scope:deploy:prod".to_owned(),
                payload_digest: digest_approve,
                decision: ApprovalDecision::Approve,
                reason: "validé par opérateur".to_owned(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    // Refus valide -> denied sans authorization_ref
    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                act_deny.as_uuid(),
                Revision::new(1),
                "ResolveActionApproval",
            ),
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResolveActionApproval {
                action_id: act_deny,
                caller: human_auth.clone(),
                approval_scope: "scope:db:admin".to_owned(),
                payload_digest: Sha256Digest::from_bytes(b"drop-table"),
                decision: ApprovalDecision::Deny,
                reason: "trop risqué".to_owned(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    let (st_app, auth_ref_app, st_den, auth_ref_den, res_cnt): (
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
    ) = store
        .read_snapshot(|conn, _| {
            let (s1, a1): (String, Option<String>) = conn.query_row(
                "SELECT state, authorization_ref FROM actions WHERE action_id = ?1",
                [act_approve.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let (s2, a2): (String, Option<String>) = conn.query_row(
                "SELECT state, authorization_ref FROM actions WHERE action_id = ?1",
                [act_deny.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let cnt: i64 =
                conn.query_row("SELECT COUNT(*) FROM approval_resolutions", [], |r| {
                    r.get(0)
                })?;
            Ok((s1, a1, s2, a2, cnt))
        })
        .unwrap();
    assert_eq!(st_app, "authorized");
    assert!(auth_ref_app.is_some());
    assert_eq!(st_den, "denied");
    assert!(auth_ref_den.is_none());
    assert_eq!(res_cnt, 2);

    // 3. SuspendSubject / ResumeSubject / RevokeSubject
    let subj_agg = uuid::Uuid::from_u128(0x50B1);
    let subj_rev = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                subj_agg,
                Revision::INITIAL,
                "SuspendSubject",
            ),
            target_aggregate: AggregateKind::SubjectSuspension,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SuspendSubject {
                subject_kind: SubjectKind::Agent,
                subject_id: "agent-alpha".to_owned(),
                scope_kind: "project".to_owned(),
                scope_id: project_id.to_string(),
                reason: "incident boucle".to_owned(),
                caller: human_auth.clone(),
                guard_propagated: false,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    assert_eq!(subj_rev, Revision::new(1));
    assert_eq!(
        store
            .get_subject_suspension(
                SubjectKind::Agent,
                "agent-alpha",
                "project",
                &project_id.to_string()
            )
            .unwrap()
            .unwrap()
            .status,
        SubjectStatus::Suspended
    );

    // Nouvelle action sur agent-alpha bloquée immédiatement !
    let blocked_act = ActionId::from_uuid(ids.new_uuid());
    let susp_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                blocked_act.as_uuid(),
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
                action_id: blocked_act,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some("s1".to_owned()),
                run_id: Some(run_id),
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "tool_exec".to_owned(),
                payload_hash: Sha256Digest::from_bytes(b"b1"),
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::Prepared,
                subject: Some((SubjectKind::Agent, "agent-alpha".to_owned())),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(susp_err.reason_code(), Some(ReasonCode::SubjectSuspended));

    // ResumeSubject rétablit l'admissibilité
    let subj_rev2 = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                subj_agg,
                subj_rev,
                "ResumeSubject",
            ),
            target_aggregate: AggregateKind::SubjectSuspension,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResumeSubject {
                subject_kind: SubjectKind::Agent,
                subject_id: "agent-alpha".to_owned(),
                scope_kind: "project".to_owned(),
                scope_id: project_id.to_string(),
                reason: "situation maîtrisée".to_owned(),
                caller: human_auth.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                blocked_act.as_uuid(),
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
                action_id: blocked_act,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some("s1".to_owned()),
                run_id: Some(run_id),
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "tool_exec".to_owned(),
                payload_hash: Sha256Digest::from_bytes(b"b1"),
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::Prepared,
                subject: Some((SubjectKind::Agent, "agent-alpha".to_owned())),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );

    // RevokeSubject reste distinct de la suspension et n'est pas réactivable par ResumeSubject
    let subj_rev3 = commit_ok(
        &store,
        CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                subj_agg,
                subj_rev2,
                "RevokeSubject",
            ),
            target_aggregate: AggregateKind::SubjectSuspension,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::RevokeSubject {
                subject_kind: SubjectKind::Agent,
                subject_id: "agent-alpha".to_owned(),
                scope_kind: "project".to_owned(),
                scope_id: project_id.to_string(),
                reason: "compromission confirmée".to_owned(),
                caller: human_auth.clone(),
                guard_propagated: true,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        },
    );
    let resume_revoked_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin,
            envelope: make_envelope(
                &ids,
                auth,
                ids.new_command_id(),
                Some(project_id),
                subj_agg,
                subj_rev3,
                "ResumeSubject",
            ),
            target_aggregate: AggregateKind::SubjectSuspension,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ResumeSubject {
                subject_kind: SubjectKind::Agent,
                subject_id: "agent-alpha".to_owned(),
                scope_kind: "project".to_owned(),
                scope_id: project_id.to_string(),
                reason: "tentative réouverture".to_owned(),
                caller: human_auth,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(resume_revoked_err.error_code(), ErrorCode::CapabilityDenied);
    assert_eq!(
        resume_revoked_err.reason_code(),
        Some(ReasonCode::SubjectRevoked)
    );
}
