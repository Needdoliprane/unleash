// T-F018-007 / AC-F018-007 / REQ-REC-007
//! Restauration F018 complète : neutre obligatoire (`RestorationBlocked` sans `restore_imports`),
//! passage `read_only_recovery` étapes 1..5, capsule delta (`actions`, `action_dispatches`, `receipts`,
//! `tombstones`, `inbox`, coûts + gel des scopes), seconde panne `C11`, blocage de ré-exécution
//! post-bascule jusqu'à réconciliation/disposition.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_domain::{
    AccountId, ActionId, ActionState, ActorId, AuthorityGeneration, CompletionPolicy, DaemonEpoch,
    DaemonMode, IdGeneratorPort, MicrosAmount, MissionId, MissionState, ProjectId, RequestId,
    Revision, SecretRef, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, EventCausationKind, ProtocolVersion, ReasonCode, ReceiptState,
    Sensitivity,
};
use unleash_store::{
    ActionDispatchInput, AggregateKind, BackupRetentionTier, BudgetAccountCharge,
    BudgetReservationRequest, BudgetScopeType, CallerAuthority, CallerRole, CapsuleCoverage,
    CommandMutation, CommitCommandRequest, CutpointObserver, ExecutorInventoryItem,
    ExecutorNeutralizer, InboxApplyOutcome, PrepareBackupRequest, PrepareObjectDeletionRequest,
    PrepareObjectRequest, PricingCheckStatus, PublishObjectReferenceRequest, ReadEventsOutcome,
    ReconcileOutcome, RestoreBackupRequest, RestoreCapsule, RestoreRequestState,
    SecretReferenceInput, StoreConfig, StoreCutpoint, StoreError, SubjectKind, SubjectStatus,
    UnleashStore, WriteCriticality,
};
use unleash_testkit::{
    Cutpoint, CutpointAction, CutpointController, DeterministicIdGenerator, ManualClock,
    local_test_dir,
};
use uuid::Uuid;

struct CutpointAdapter(CutpointController);
impl CutpointObserver for CutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        let cp = match cutpoint {
            StoreCutpoint::C11RestoreBeforeActivate => Cutpoint::C11,
            _ => return Ok(()),
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

struct LineagedNeutralizer {
    allow: bool,
}

impl ExecutorNeutralizer for LineagedNeutralizer {
    fn inventory_and_neutralize(
        &self,
        source_authority_generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, StoreError> {
        if !self.allow {
            return Err(StoreError::RestorationBlocked(
                "exécuteur source actif non neutralisé".to_owned(),
            ));
        }
        Ok(vec![ExecutorInventoryItem {
            executor_id: "exec-lineage-1".to_owned(),
            authority_generation: source_authority_generation,
            daemon_epoch: DaemonEpoch::new(1),
            boot_identity: "boot-f018-007".to_owned(),
            neutralized: true,
        }])
    }
}

fn make_env(
    ids: &DeterministicIdGenerator,
    auth: AuthorityGeneration,
    cmd: unleash_domain::CommandId,
    proj: Option<ProjectId>,
    agg: Uuid,
    rev: Revision,
    cmd_type: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(ids.new_uuid()),
        command_id: cmd,
        authority_generation: auth,
        actor_ref: Some("user:louis".to_owned()),
        project_id: proj,
        aggregate_id: agg,
        expected_revision: rev,
        command_type: cmd_type.to_owned(),
        payload: Map::new(),
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario F018-007 complet : snapshot, delta complet dispatches/tombstones/coûts, blocage, crash C11 et gel post-restore"
)]
fn test_t_f018_007_full_capsule_dispatches_tombstones_neutralization_and_freeze() {
    let src_dir = local_test_dir("f018-007-src").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0007));
    let ctrl = CutpointController::new();

    let mut src_cfg = StoreConfig::new(src_dir.path());
    src_cfg.clock = clock.clone();
    src_cfg.id_generator = ids.clone();
    src_cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    let store = UnleashStore::open(src_cfg.clone()).unwrap();
    store
        .register_principal("p-louis", "user", "user:louis")
        .unwrap();
    let auth_1 = store.active_authority_generation().unwrap();
    let admin = ActorId::new("user:louis").unwrap();
    let human = CallerAuthority {
        actor_id: admin.clone(),
        role: CallerRole::HumanOperator,
    };

    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let acc_id = AccountId::from_uuid(ids.new_uuid());
    let sec_agg = ids.new_uuid();
    let subj_agg = ids.new_uuid();

    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
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
                name: "proj-007".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    store
        .upsert_budget_account(
            acc_id,
            BudgetScopeType::Project,
            &project_id.to_string(),
            "USD",
            MicrosAmount::new(10_000),
        )
        .unwrap();

    // Référence secrète et suspension sujet créées AVANT snapshot
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                ids.new_command_id(),
                Some(project_id),
                sec_agg,
                Revision::INITIAL,
                "SetSecretReference",
            ),
            target_aggregate: AggregateKind::SecretReference,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSecretReference {
                input: SecretReferenceInput {
                    secret_ref_id: "sec-openai-1".to_owned(),
                    secret_ref: SecretRef::new("secret://keychain/openai/1").unwrap(),
                    namespace: "provider".to_owned(),
                    purpose: "api_key".to_owned(),
                    secret_kind: "bearer_token".to_owned(),
                    target_kind: "provider_profile".to_owned(),
                    target_id: "openai-main".to_owned(),
                    scope_kind: "project".to_owned(),
                    scope_id: project_id.to_string(),
                    backend_kind: "os_keychain".to_owned(),
                },
                caller: human.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Objet à supprimer post-snapshot pour générer un tombstone d'objet
    let del_bytes = b"ephemeral-temp-blob".to_vec();
    let del_hash = Sha256Digest::from_bytes(&del_bytes);
    let del_h = store
        .prepare_object(PrepareObjectRequest {
            object_hash: del_hash,
            byte_length: del_bytes.len() as u64,
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "temp".to_owned(),
            owner_id: "temp-1".to_owned(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&del_h, &del_bytes)
        .unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: del_h.pin_id,
            object_hash: del_hash,
            object_generation: del_h.object_generation,
            owner_kind: "temp".to_owned(),
            owner_id: "temp-1".to_owned(),
            source_field: "blob".to_owned(),
        })
        .unwrap();

    // Événement pending dans outbox au moment du snapshot
    let m_snap = MissionId::from_uuid(ids.new_uuid());
    let mut ev_payload = Map::new();
    ev_payload.insert("scope".into(), Value::String(project_id.to_string()));
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                ids.new_command_id(),
                Some(project_id),
                m_snap.as_uuid(),
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
                mission_id: m_snap,
                goal_id: None,
                project_id,
                owner_principal_id: Some("p-louis".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Ready,
            },
            events: vec![unleash_store::EventAppendSpec {
                event_type: "MissionCreated".to_owned(),
                aggregate_type: AggregateKind::Mission,
                aggregate_id: m_snap.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(project_id),
                correlation_id: Uuid::from_u128(777),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: ev_payload,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec!["effect-dispatcher".to_owned()],
            }],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let stream_snap = store.daemon_state().unwrap().stream_id;
    let snap_ev = match store
        .read_events(stream_snap, unleash_domain::Sequence::new(0), 20, None)
        .unwrap()
    {
        ReadEventsOutcome::Events { events, .. } => events
            .into_iter()
            .find(|e| e.aggregate_id == m_snap.as_uuid())
            .unwrap(),
        ReadEventsOutcome::ResyncRequired { reason, .. } => panic!("resync inattendu: {reason}"),
    };

    // Snapshot backup
    let bkp_dir = src_dir.path().join("backups/bkp-007");
    store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: bkp_dir.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();

    // --- ACTIVITÉ POST-SNAPSHOT ---
    // a) Consommation inbox
    assert_eq!(
        store
            .apply_inbox_event("effect-dispatcher", &snap_ev, snap_ev.payload_hash, |_tx| {
                Ok(())
            },)
            .unwrap(),
        InboxApplyOutcome::Applied {
            result_hash: snap_ev.payload_hash,
        }
    );

    // b) Action post-snapshot + dispatch + réservation budget
    let post_act = ActionId::from_uuid(ids.new_uuid());
    let post_payload = Sha256Digest::from_bytes(b"post-snap-effect-1");
    let post_act_cmd = ids.new_command_id();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                post_act_cmd,
                Some(project_id),
                post_act.as_uuid(),
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
                action_id: post_act,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some(project_id.to_string()),
                run_id: None,
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "send_webhook".to_owned(),
                payload_hash: post_payload,
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::OutcomeUnknown,
                subject: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![BudgetReservationRequest {
                action_id: post_act,
                model_id: "claude-haiku-5-5".to_owned(),
                pricing_version: "pv-1".to_owned(),
                pricing_status: PricingCheckStatus::Valid,
                max_request_hash: post_payload,
                charges: vec![BudgetAccountCharge {
                    account_id: acc_id,
                    max_micros: MicrosAmount::new(300),
                }],
            }],
        })
        .unwrap();

    store
        .record_action_dispatch(ActionDispatchInput {
            dispatch_id: "disp-post-007".to_owned(),
            action_id: post_act,
            ordinal: 1,
            adapter_id: "webhook-adapter".to_owned(),
            request_hash: post_payload,
            provider_request_id: Some("wh-99".to_owned()),
            transport_state: "unknown_after_send".to_owned(),
            receipt_hash: Some(Sha256Digest::from_bytes(b"disp-rcpt")),
        })
        .unwrap();
    store.mark_action_budget_unknown(post_act).unwrap();

    // c) Tombstones & révocations post-snapshot :
    //    - Révocation du secret et du sujet
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                ids.new_command_id(),
                Some(project_id),
                sec_agg,
                Revision::new(1),
                "RevokeSecretReference",
            ),
            target_aggregate: AggregateKind::SecretReference,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::RevokeSecretReference {
                secret_ref_id: "sec-openai-1".to_owned(),
                reason: "fuite suspectée post-snapshot".to_owned(),
                caller: human.clone(),
                remote_revocation_supported: false,
                remote_revocation_confirmed: false,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                ids.new_command_id(),
                Some(project_id),
                subj_agg,
                Revision::INITIAL,
                "RevokeSubject",
            ),
            target_aggregate: AggregateKind::SubjectSuspension,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::RevokeSubject {
                subject_kind: SubjectKind::ProviderAccount,
                subject_id: "prov-bad".to_owned(),
                scope_kind: "project".to_owned(),
                scope_id: project_id.to_string(),
                reason: "clé compromise post-snapshot".to_owned(),
                caller: human.clone(),
                guard_propagated: false,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    //    - Tombstone de réponse sur un receipt post-snapshot
    let expired_cmd_id = ids.new_command_id();
    let daemon_rev = store.daemon_state().unwrap().revision;
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_1,
                expired_cmd_id,
                Some(project_id),
                Uuid::from_u128(1),
                daemon_rev,
                "NoopCommand",
            ),
            target_aggregate: AggregateKind::Daemon,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: Some(1),
            fence_assertion: None,
            mutation: CommandMutation::NoopDaemonBump,
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    // Vieillir suffisamment pour obtenir un tombstone de receipt + purge
    clock.advance(Duration::from_secs(9 * 86_400));
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            tx.execute(
                "UPDATE command_receipts SET response_retained_until_ms = ?1 WHERE command_id = ?2",
                rusqlite::params![now.as_i64() - 1000, expired_cmd_id.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    store.purge_expired_command_receipts().unwrap();

    //    - Suppression tombstone d'un objet post-snapshot
    store
        .tombstone_object_references(del_hash, "post-snapshot deletion")
        .unwrap();
    let del_intent = store
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: del_hash,
            expected_generation: del_h.object_generation,
            reason: "post-snapshot deletion".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();
    store
        .execute_and_finalize_object_deletion(&del_intent)
        .unwrap();

    // 1. Blocage restauration si exécuteur non neutralisé :
    //    - source DB passe en `read_only_recovery`
    //    - `restore_requests` = `blocked`, ZERO ligne `restore_imports`
    let target_blocked = local_test_dir("f018-007-blocked").unwrap();
    let blocked_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir.clone(),
            target_data_root: target_blocked.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: Sha256Digest::from_bytes(b"restore-blocked-007"),
            source_authority_generation: auth_1,
            source_db_unreadable: false,
            source_db_path: Some(src_cfg.db_path.clone()),
            source_artifacts_dir: Some(src_cfg.artifacts_dir.clone()),
            neutralizer: Arc::new(LineagedNeutralizer { allow: false }),
        },
        StoreConfig::new(target_blocked.path()),
    )
    .unwrap_err();
    assert!(matches!(blocked_err, StoreError::RestorationBlocked(_)));
    assert_eq!(
        store.daemon_state().unwrap().mode,
        DaemonMode::ReadOnlyRecovery
    );

    let (req_st, src_imp_cnt): (String, i64) = store
        .read_snapshot(|conn, _| {
            let st: String = conn.query_row(
                "SELECT state FROM restore_requests ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            let ic: i64 =
                conn.query_row("SELECT COUNT(*) FROM restore_imports", [], |r| r.get(0))?;
            Ok((st, ic))
        })
        .unwrap();
    assert_eq!(req_st, RestoreRequestState::Blocked.as_str());
    assert_eq!(src_imp_cnt, 0);

    // 2. Seconde panne à C11RestoreBeforeActivate -> pas de bascule
    let target_ok = local_test_dir("f018-007-ok").unwrap();
    let mut target_cfg = StoreConfig::new(target_ok.path());
    target_cfg.clock = clock;
    target_cfg.id_generator = ids.clone();
    target_cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    ctrl.set_action_on_hit(Cutpoint::C11, 1, CutpointAction::Fail("second-crash"))
        .unwrap();
    let c11_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir.clone(),
            target_data_root: target_ok.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: Sha256Digest::from_bytes(b"restore-c11-007"),
            source_authority_generation: auth_1,
            source_db_unreadable: false,
            source_db_path: Some(src_cfg.db_path.clone()),
            source_artifacts_dir: Some(src_cfg.artifacts_dir.clone()),
            neutralizer: Arc::new(LineagedNeutralizer { allow: true }),
        },
        target_cfg.clone(),
    )
    .unwrap_err();
    assert!(matches!(c11_err, StoreError::CutpointInjected { .. }));
    ctrl.clear(Cutpoint::C11);

    // 3. Restauration réussie : capsule complète (actions + dispatches + receipts + tombstones + coûts + scopes gelés)
    let act_res = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir,
            target_data_root: target_ok.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: Sha256Digest::from_bytes(b"restore-ok-007"),
            source_authority_generation: auth_1,
            source_db_unreadable: false,
            source_db_path: Some(src_cfg.db_path),
            source_artifacts_dir: Some(src_cfg.artifacts_dir),
            neutralizer: Arc::new(LineagedNeutralizer { allow: true }),
        },
        target_cfg.clone(),
    )
    .unwrap();
    assert_eq!(act_res.coverage, CapsuleCoverage::Complete);
    assert!(act_res.frozen_scopes.contains(&project_id.to_string()));
    store.shutdown().unwrap();

    let restored = UnleashStore::open(target_cfg).unwrap();
    assert_eq!(
        restored.daemon_state().unwrap().mode,
        DaemonMode::ReadOnlyRecovery
    );

    let capsule_bytes = restored
        .read_verified_object_bytes(act_res.capsule_object_hash)
        .unwrap();
    let capsule: RestoreCapsule = serde_json::from_slice(&capsule_bytes).unwrap();
    assert!(
        capsule
            .dispatches
            .iter()
            .any(|d| d.dispatch_id == "disp-post-007")
    );
    assert!(!capsule.tombstones.is_empty());

    // Vérifier en base restaurée : dispatch, tombstone secret, tombstone sujet, tombstone objet, coûts
    let (disp_cnt, del_obj_st): (i64, String) = restored
        .read_snapshot(|conn, _| {
            let dc: i64 = conn.query_row(
                "SELECT COUNT(*) FROM action_dispatches WHERE dispatch_id = 'disp-post-007'",
                [],
                |r| r.get(0),
            )?;
            let os: String = conn.query_row(
                "SELECT state FROM object_records WHERE object_hash = ?1",
                [del_hash.as_prefixed_str()],
                |r| r.get(0),
            )?;
            Ok((dc, os))
        })
        .unwrap();
    assert_eq!(disp_cnt, 1);
    assert_eq!(del_obj_st, "tombstone");

    let sec_st = restored
        .get_secret_reference_status("sec-openai-1")
        .unwrap();
    assert_eq!(sec_st.status, unleash_store::SecretRefState::RevokedLocal);
    let subj_st = restored
        .get_subject_suspension(
            SubjectKind::ProviderAccount,
            "prov-bad",
            "project",
            &project_id.to_string(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(subj_st.status, SubjectStatus::Revoked);

    // Outbox pending dans scope gelé NE doit PAS être relancée ni nouvelle action admise avant disposition
    assert!(
        restored
            .lease_outbox_batch("effect-dispatcher", 10)
            .unwrap()
            .is_empty()
    );

    let auth_2 = restored.active_authority_generation().unwrap();
    let dup_action = ActionId::from_uuid(ids.new_uuid());
    let frozen_err = restored
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth_2,
                ids.new_command_id(),
                Some(project_id),
                dup_action.as_uuid(),
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
                action_id: dup_action,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-louis".to_owned()),
                project_id: Some(project_id),
                scope_key: Some(project_id.to_string()),
                run_id: None,
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "send_webhook".to_owned(),
                payload_hash: post_payload,
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
                approval_scope: None,
                initial_state: ActionState::Prepared,
                subject: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(frozen_err.error_code(), ErrorCode::ResourceBusy);
    assert_eq!(
        frozen_err.reason_code(),
        Some(ReasonCode::RecoveryInProgress)
    );

    // Une fois `post_act` réconciliée, le gel du scope est levé !
    restored
        .commit_command(CommitCommandRequest {
            actor_id: admin,
            envelope: make_env(
                &ids,
                auth_2,
                ids.new_command_id(),
                Some(project_id),
                post_act.as_uuid(),
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
                action_id: post_act,
                caller: human,
                outcome: ReconcileOutcome::ConfirmedFailureNoEffect,
                evidence_digest: None,
                receipt_hash: None,
                reason: "webhook jamais envoyé".to_owned(),
                expected_source_epoch: None,
                expected_source_fence: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    assert!(
        restored
            .list_frozen_scopes()
            .unwrap()
            .into_iter()
            .all(|(_, _, st)| st == "disposed")
    );
}
