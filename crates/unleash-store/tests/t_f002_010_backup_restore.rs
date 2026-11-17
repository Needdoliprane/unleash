// T-F002-010 / AC-F002-010 / REQ-PERSIST-010
//! Sauvegarde Online Backup API, rotation 7+4, restore offline hostile et propriétés.

use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;
use serde_json::{Map, Value};
use unleash_domain::{
    AccountId, ActionId, ActorId, AuthorityGeneration, BackupId, CommandId, CompletionPolicy,
    DaemonEpoch, DaemonMode, IdGeneratorPort, MicrosAmount, MissionId, MissionState, ProjectId,
    RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReasonCode, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, BackupRetentionTier, BudgetAccountCharge, BudgetReservationRequest,
    BudgetScopeType, CapsuleCoverage, CommandMutation, CommitCommandRequest, CutpointObserver,
    EmptyExecutorNeutralizer, ExecutorInventoryItem, ExecutorNeutralizer, InboxApplyOutcome,
    MonotonicClockPort, PrepareBackupRequest, PrepareObjectDeletionRequest, PrepareObjectRequest,
    PricingCheckStatus, PublishObjectReferenceRequest, ReadEventsOutcome, RestoreBackupRequest,
    RestoreCapsule, RestoreRequestState, StoreConfig, StoreCutpoint, StoreError, UnleashStore,
    WriteCriticality,
};
use unleash_testkit::{
    Cutpoint, CutpointAction, CutpointController, DeterministicIdGenerator, ManualClock,
    local_test_dir,
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
            StoreCutpoint::C11aAfterBackupBarrier => Cutpoint::C11a,
            StoreCutpoint::C11bAfterDbSnapshotBeforePins => Cutpoint::C11b,
            StoreCutpoint::C11cAfterPinsDuringCopy => Cutpoint::C11c,
            StoreCutpoint::C11dAfterManifestBeforeComplete => Cutpoint::C11d,
            StoreCutpoint::C11RestoreBeforeActivate => Cutpoint::C11,
            _ => return Ok(()),
        };
        self.0.hit(cp).map_err(|e| StoreError::CutpointInjected {
            cutpoint: cp.as_str().to_owned(),
            message: e.to_string(),
        })
    }
}

struct Host100Neutralizer {
    allow_neutralization: bool,
    source_auth: AuthorityGeneration,
}

impl ExecutorNeutralizer for Host100Neutralizer {
    fn inventory_and_neutralize(
        &self,
        source_authority_generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, StoreError> {
        if !self.allow_neutralization {
            return Err(StoreError::RestorationBlocked(
                "host100 impossible à neutraliser".to_owned(),
            ));
        }
        Ok(vec![ExecutorInventoryItem {
            executor_id: "host-100".to_owned(),
            authority_generation: source_authority_generation,
            daemon_epoch: DaemonEpoch::new(100),
            boot_identity: "boot-1".to_owned(),
            neutralized: self.source_auth == source_authority_generation,
        }])
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Suite bout-en-bout T-F002-010 : C11a..d, pins, rotation 7+4, restore hostile/corrompu, C11, inbox/budgets invocation et PurgeHistory"
)]
fn test_t_f002_010_backup_c11_rotation_event_tombstone_and_hostile_restore() {
    let dir = local_test_dir("f002-010").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0010));
    let ctrl = CutpointController::new();

    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    let mut store = UnleashStore::open(cfg.clone()).unwrap();
    store
        .register_principal("p1", "user", "user:louis")
        .unwrap();

    // Avancer l'époque source jusqu'à 40 pour simuler backup epoch 40
    for _ in 1..40 {
        store = store.restart_same_lineage().unwrap();
    }
    assert_eq!(
        store.daemon_state().unwrap().daemon_epoch,
        DaemonEpoch::new(40)
    );
    let source_auth = store.active_authority_generation().unwrap();

    // Préparer et publier un objet live référencé + un projet + une mission + compte budget
    let obj_bytes = b"snapshot-evidence-blob".to_vec();
    let obj_hash = Sha256Digest::from_bytes(&obj_bytes);
    let h = store
        .prepare_object(PrepareObjectRequest {
            object_hash: obj_hash,
            byte_length: u64::try_from(obj_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-snap".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&h, &obj_bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: h.pin_id,
            object_hash: obj_hash,
            object_generation: h.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-snap".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();

    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let acc_id = AccountId::from_uuid(ids.new_uuid());
    store
        .upsert_budget_account(
            acc_id,
            BudgetScopeType::Project,
            &project_id.to_string(),
            "USD",
            MicrosAmount::new(10_000),
        )
        .unwrap();
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: source_auth,
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
                name: "proj-bkp".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Créer un événement qui aura une entrée outbox pending dans le snapshot
    let mission_snap = MissionId::from_uuid(ids.new_uuid());
    let mut ev_p = Map::new();
    ev_p.insert("scope".into(), Value::String(project_id.to_string()));
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: source_auth,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: mission_snap.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateMission".to_owned(),
                payload: ev_p.clone(),
            },
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateMission {
                mission_id: mission_snap,
                goal_id: None,
                project_id,
                owner_principal_id: Some("p1".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Ready,
            },
            events: vec![build_event_append_spec(
                mission_snap,
                project_id,
                ev_p,
                vec!["worker-post-snap".to_owned()],
            )],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Capturer l'événement de mission créé dans le snapshot + pré-semer invocation & tentative snapshot
    let stream_snap = store.daemon_state().unwrap().stream_id;
    let snap_ev = match store
        .read_events(stream_snap, unleash_domain::Sequence::new(0), 20, None)
        .unwrap()
    {
        ReadEventsOutcome::Events { events: evs, .. } => evs
            .into_iter()
            .find(|e| e.aggregate_id == mission_snap.as_uuid())
            .expect("événement mission snapshot"),
        ReadEventsOutcome::ResyncRequired { reason, .. } => {
            panic!("resync inattendu : {reason}")
        }
    };

    let inv_snap_id = ids.new_invocation_id();
    let att_snap_id = ids.new_uuid();
    let agent_snap_id = ids.new_agent_id();
    let sess_snap_id = ids.new_session_id();
    let man_snap_id = ids.new_uuid();
    let bundle_snap_hash = Sha256Digest::from_bytes(b"bundle-snap").to_string();
    let snap_ev_id_str = snap_ev.event_id.to_string();
    let snap_proj_str = project_id.to_string();
    let snap_obj_hash_str = obj_hash.as_prefixed_str();
    let snap_auth_str = source_auth.to_string();
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            tx.execute(
                "INSERT INTO agents(agent_id, level, organization_parent_id, created_by_agent_id, project_id, state, revision)
                 VALUES (?1, 'N', NULL, NULL, ?2, 'active', 1)",
                rusqlite::params![agent_snap_id.to_string(), snap_proj_str],
            )?;
            tx.execute(
                "INSERT INTO sessions(session_id, agent_id, revision, state, created_at_ms)
                 VALUES (?1, ?2, 0, 'open', ?3)",
                rusqlite::params![sess_snap_id.to_string(), agent_snap_id.to_string(), now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO context_manifests(manifest_id, session_id, base_session_revision, content_hash, schema_version, created_at_ms)
                 VALUES (?1, ?2, 0, ?3, 1, ?4)",
                rusqlite::params![man_snap_id.to_string(), sess_snap_id.to_string(), snap_obj_hash_str, now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO effective_bundles(bundle_hash, schema_version, role_version_id, requested_model_profile_version_id, allowed_model_policy_hash, initial_context_manifest_id, scope_id, scope_revision, content_object_hash)
                 VALUES (?1, 1, 'rv-snap', 'mp-snap', ?2, ?3, 's-snap', 0, ?2)",
                rusqlite::params![bundle_snap_hash, snap_obj_hash_str, man_snap_id.to_string()],
            )?;
            tx.execute(
                "INSERT INTO invocations(invocation_id, agent_id, session_id, trigger_event_id, effective_bundle_hash, context_manifest_id, state, admission_class, revision, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', 'work', 1, ?7)",
                rusqlite::params![inv_snap_id.to_string(), agent_snap_id.to_string(), sess_snap_id.to_string(), snap_ev_id_str, bundle_snap_hash, man_snap_id.to_string(), now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO invocation_attempts(attempt_id, invocation_id, ordinal, authority_generation, daemon_epoch, fence, state, started_at_ms)
                 VALUES (?1, ?2, 1, ?3, 40, 0, 'running', ?4)",
                rusqlite::params![att_snap_id.to_string(), inv_snap_id.to_string(), snap_auth_str, now.as_i64()],
            )?;
            Ok(())
        })
        .unwrap();

    // Préparer un objet sacrifiable sans référence live pour vérifier la conservation des pins backup_barrier/backup après C11a..c
    let gc_guard_bytes = b"gc-protected-by-c11-pins".to_vec();
    let gc_guard_hash = Sha256Digest::from_bytes(&gc_guard_bytes);
    let h_gc = store
        .prepare_object(PrepareObjectRequest {
            object_hash: gc_guard_hash,
            byte_length: u64::try_from(gc_guard_bytes.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-c11-guard".to_owned(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&h_gc, &gc_guard_bytes)
        .unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: h_gc.pin_id,
            object_hash: gc_guard_hash,
            object_generation: h_gc.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-c11-guard".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    store
        .tombstone_object_references(gc_guard_hash, "ready_for_pin_check")
        .unwrap();

    // Injecter C11a..d et vérifier conservation barrière/pins contre la collecte (y compris après redémarrage) avant abandon explicite
    for cp in [
        Cutpoint::C11a,
        Cutpoint::C11b,
        Cutpoint::C11c,
        Cutpoint::C11d,
    ] {
        ctrl.set_action_on_hit(cp, 1, CutpointAction::Fail("c11-interrupt"))
            .unwrap();
        let b_fail_dir = dir.path().join(format!("backups/fail-{}", cp.as_str()));
        assert!(
            store
                .prepare_backup(PrepareBackupRequest {
                    destination_dir: b_fail_dir.clone(),
                    retention_tier: BackupRetentionTier::Manual,
                    coverage: "db_and_objects".to_owned(),
                })
                .is_err()
        );
        if b_fail_dir.join("manifest.json").exists() {
            assert!(UnleashStore::validate_backup(&b_fail_dir, &cfg).is_err());
        }
        ctrl.clear(cp);

        let aborted_id_str: String = store
            .read_snapshot(|conn, _| {
                Ok(conn.query_row(
                    "SELECT backup_id FROM backup_jobs WHERE state != 'complete' ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        let aborted_id: BackupId = aborted_id_str.parse().unwrap();

        match cp {
            Cutpoint::C11a | Cutpoint::C11b => {
                // Barrière backup_barrier conservée et bloque la collecte
                let barrier_err = store
                    .prepare_object_deletion(PrepareObjectDeletionRequest {
                        object_hash: gc_guard_hash,
                        expected_generation: 1,
                        reason: "gc_during_c11ab".to_owned(),
                        decision_receipt_id: None,
                    })
                    .unwrap_err();
                assert!(matches!(barrier_err, StoreError::BackupBarrierActive));
                store = store.restart_same_lineage().unwrap();
                let barrier_after_restart = store
                    .prepare_object_deletion(PrepareObjectDeletionRequest {
                        object_hash: gc_guard_hash,
                        expected_generation: 1,
                        reason: "gc_after_restart_c11ab".to_owned(),
                        decision_receipt_id: None,
                    })
                    .unwrap_err();
                assert!(matches!(
                    barrier_after_restart,
                    StoreError::BackupBarrierActive
                ));
            }
            Cutpoint::C11c => {
                // Pins `backup` conservés sur les objets du snapshot et protègent contre la collecte
                let pin_err = store
                    .prepare_object_deletion(PrepareObjectDeletionRequest {
                        object_hash: gc_guard_hash,
                        expected_generation: 1,
                        reason: "gc_during_c11c".to_owned(),
                        decision_receipt_id: None,
                    })
                    .unwrap_err();
                assert!(matches!(pin_err, StoreError::ObjectPinned { .. }));
                store = store.restart_same_lineage().unwrap();
                let pin_after_restart = store
                    .prepare_object_deletion(PrepareObjectDeletionRequest {
                        object_hash: gc_guard_hash,
                        expected_generation: 1,
                        reason: "gc_after_restart_c11c".to_owned(),
                        decision_receipt_id: None,
                    })
                    .unwrap_err();
                assert!(matches!(pin_after_restart, StoreError::ObjectPinned { .. }));
            }
            Cutpoint::C11d => {}
            _ => unreachable!(),
        }
        store.abandon_backup_job(aborted_id).unwrap();
    }

    // Collecter l'objet sacrifiable avant backup40 (pour garder exactement 1 objet dans backup40)
    let del_guard = store
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: gc_guard_hash,
            expected_generation: 1,
            reason: "cleanup_before_backup40".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();
    store
        .execute_and_finalize_object_deletion(&del_guard)
        .unwrap();

    // Produire backup40 complet et validé
    let bkp40_dir = dir.path().join("backups/backup40");
    let bkp40 = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: bkp40_dir.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    let val40 = UnleashStore::validate_backup(&bkp40_dir, &cfg).unwrap();
    assert!(val40.integrity_check_ok);
    assert_eq!(val40.foreign_key_violations, 0);
    assert_eq!(val40.verified_object_count, 1);

    // Rotation Q-12 : 7 quotidiennes + 4 hebdomadaires
    for d in 0..9 {
        clock.advance(Duration::from_secs(60));
        store
            .prepare_backup(PrepareBackupRequest {
                destination_dir: dir.path().join(format!("backups/daily-{d}")),
                retention_tier: BackupRetentionTier::Daily,
                coverage: "db_and_objects".to_owned(),
            })
            .unwrap();
    }
    for w in 0..6 {
        clock.advance(Duration::from_secs(60));
        store
            .prepare_backup(PrepareBackupRequest {
                destination_dir: dir.path().join(format!("backups/weekly-{w}")),
                retention_tier: BackupRetentionTier::Weekly,
                coverage: "db_and_objects".to_owned(),
            })
            .unwrap();
    }
    let tier_counts: (i64, i64) = store
        .read_snapshot(|conn, _| {
            let d_cnt: i64 = conn.query_row(
                "SELECT COUNT(*) FROM backup_jobs WHERE retention_tier = 'daily' AND state = 'complete'",
                [],
                |r| r.get(0),
            )?;
            let w_cnt: i64 = conn.query_row(
                "SELECT COUNT(*) FROM backup_jobs WHERE retention_tier = 'weekly' AND state = 'complete'",
                [],
                |r| r.get(0),
            )?;
            Ok((d_cnt, w_cnt))
        })
        .unwrap();
    assert_eq!(tier_counts, (7, 4));

    // Effets et consommations postérieurs au snapshot (host100) + coût réglé de 250
    let current_epoch = store.daemon_state().unwrap().daemon_epoch.as_u64();
    for _ in current_epoch..100 {
        store = store.restart_same_lineage().unwrap();
    }
    assert_eq!(
        store.daemon_state().unwrap().daemon_epoch,
        DaemonEpoch::new(100)
    );
    let post_action_id = ActionId::from_u128(99_001);
    let post_cmd_id = ids.new_command_id();
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: post_cmd_id,
                authority_generation: source_auth,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: post_action_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateAction".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateAction {
                action_id: post_action_id,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p1".to_owned()),
                run_id: None,
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "external_post_snap".to_owned(),
                payload_hash: Sha256Digest::from_bytes(b"post"),
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![BudgetReservationRequest {
                action_id: post_action_id,
                model_id: "claude-haiku-5-5".to_owned(),
                pricing_version: "pv1".to_owned(),
                pricing_status: PricingCheckStatus::Valid,
                max_request_hash: Sha256Digest::from_bytes(b"maxpost"),
                charges: vec![BudgetAccountCharge {
                    account_id: acc_id,
                    max_micros: MicrosAmount::new(250),
                }],
            }],
        })
        .unwrap();
    store
        .settle_action_budget(post_action_id, MicrosAmount::new(250))
        .unwrap();

    // Consommation inbox post-snapshot + action `invocation` post-snapshot avec réservation budget `unknown`
    let inbox_outcome = store
        .apply_inbox_event(
            "worker-inbox-post",
            &snap_ev,
            Sha256Digest::from_bytes(b"inbox-post-snap-res"),
            |_tx| Ok(()),
        )
        .unwrap();
    assert!(matches!(inbox_outcome, InboxApplyOutcome::Applied { .. }));

    let post_inv_action_id = ActionId::from_u128(99_002);
    let post_inv_res_id = ids.new_uuid().to_string();
    let inv_s = inv_snap_id.to_string();
    let att_s = att_snap_id.to_string();
    let post_inv_s = post_inv_action_id.to_string();
    let acc_s = acc_id.to_string();
    let auth_s = source_auth.to_string();
    let pay_h = Sha256Digest::from_bytes(b"inv-post").as_prefixed_str();
    let max_h = Sha256Digest::from_bytes(b"inv-max").as_prefixed_str();
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            tx.execute(
                "INSERT INTO actions
                 (action_id, origin_kind, control_principal_id, run_id, step_id, invocation_id, attempt_id,
                  replay_worker_id, action_type, payload_hash, authorization_ref, policy_revision, resource_lease_id,
                  authority_generation, daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms)
                 VALUES (?1, 'invocation', NULL, NULL, NULL, ?2, ?3, NULL, 'model_call_post_snap', ?4, NULL, 1, NULL, ?5, 100, 0, 'in_flight', 'observe_then_decide', NULL, 1, ?6)",
                rusqlite::params![post_inv_s, inv_s, att_s, pay_h, auth_s, now.as_i64()],
            )?;
            tx.execute(
                "INSERT INTO budget_reservations
                 (reservation_id, account_id, action_id, reserved_micros, settled_micros, state, pricing_version, max_request_hash)
                 VALUES (?1, ?2, ?3, 400, NULL, 'held', 'pv1', ?4)",
                rusqlite::params![post_inv_res_id, acc_s, post_inv_s, max_h],
            )?;
            tx.execute(
                "UPDATE budget_accounts SET reserved_micros = reserved_micros + 400, revision = revision + 1 WHERE account_id = ?1",
                [&acc_s],
            )?;
            Ok(())
        })
        .unwrap();
    store
        .mark_action_budget_unknown(post_inv_action_id)
        .unwrap();

    // 1. Si host100 NE PEUT PAS être neutralisé -> RestorationBlocked dans restore_requests, ZERO restore_imports !
    let restore_root_blocked = local_test_dir("f002-010-blocked").unwrap();
    let restore_cmd_blocked = ids.new_command_id();
    let blocked_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp40_dir.clone(),
            target_data_root: restore_root_blocked.path().to_path_buf(),
            command_id: restore_cmd_blocked,
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore-blocked").unwrap(),
            source_authority_generation: source_auth,
            source_db_unreadable: false,
            source_db_path: Some(cfg.db_path.clone()),
            source_artifacts_dir: Some(cfg.artifacts_dir.clone()),
            neutralizer: Arc::new(Host100Neutralizer {
                allow_neutralization: false,
                source_auth,
            }),
        },
        StoreConfig::new(restore_root_blocked.path()),
    )
    .unwrap_err();
    assert!(matches!(blocked_err, StoreError::RestorationBlocked(_)));
    let blocked_state: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT state FROM restore_requests ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(blocked_state, RestoreRequestState::Blocked.as_str());

    // 2. Sauvegarde avec objet manquant refusée
    let bkp_missing_dir = dir.path().join("backups/backup40-corrupt-obj");
    copy_dir_recursive(&bkp40_dir, &bkp_missing_dir).unwrap();
    let obj_rel = &val40
        .manifest
        .included_objects
        .first()
        .unwrap()
        .relative_path;
    std::fs::remove_file(bkp_missing_dir.join("objects").join(obj_rel)).unwrap();
    assert!(UnleashStore::validate_backup(&bkp_missing_dir, &cfg).is_err());

    // 3. Injection de C11RestoreBeforeActivate avant activation réussie -> rollback sans bascule de lignée
    let restore_root = local_test_dir("f002-010-restored").unwrap();
    let mut restore_cfg = StoreConfig::new(restore_root.path());
    restore_cfg.clock = clock.clone();
    restore_cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    restore_cfg.id_generator = ids.clone();
    restore_cfg.cutpoints = Arc::new(CutpointAdapter(ctrl.clone()));

    ctrl.set_action_on_hit(
        Cutpoint::C11,
        1,
        CutpointAction::Fail("c11-before-activate"),
    )
    .unwrap();
    let c11_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp40_dir.clone(),
            target_data_root: restore_root.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore-c11").unwrap(),
            source_authority_generation: source_auth,
            source_db_unreadable: false,
            source_db_path: Some(cfg.db_path.clone()),
            source_artifacts_dir: Some(cfg.artifacts_dir.clone()),
            neutralizer: Arc::new(Host100Neutralizer {
                allow_neutralization: true,
                source_auth,
            }),
        },
        restore_cfg.clone(),
    )
    .unwrap_err();
    assert!(matches!(c11_err, StoreError::CutpointInjected { .. }));
    ctrl.clear(Cutpoint::C11);

    // Vérifier qu'après échec à C11RestoreBeforeActivate, la cible n'a pas été activée sur une nouvelle lignée
    let ro_c11 = rusqlite::Connection::open_with_flags(
        &restore_cfg.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let pre_act_auth: String = ro_c11
        .query_row(
            "SELECT authority_generation FROM daemon_state WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pre_act_auth, source_auth.to_string());
    let activated_cnt: i64 = ro_c11
        .query_row(
            "SELECT COUNT(*) FROM restore_imports WHERE state = 'activated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activated_cnt, 0);
    drop(ro_c11);

    // 3b. Restauration réussie après neutralisation de host100 et levée C11
    let act_res = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp40_dir.clone(),
            target_data_root: restore_root.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore-ok").unwrap(),
            source_authority_generation: source_auth,
            source_db_unreadable: false,
            source_db_path: Some(cfg.db_path.clone()),
            source_artifacts_dir: Some(cfg.artifacts_dir.clone()),
            neutralizer: Arc::new(Host100Neutralizer {
                allow_neutralization: true,
                source_auth,
            }),
        },
        restore_cfg.clone(),
    )
    .unwrap();
    assert_ne!(act_res.target_authority_generation, source_auth);
    assert_ne!(act_res.target_stream_id, bkp40.snapshot_stream_id);
    assert_eq!(act_res.target_daemon_epoch, DaemonEpoch::new(1));
    assert_eq!(act_res.coverage, CapsuleCoverage::Complete);
    assert!(!act_res.global_freeze_active);
    assert!(act_res.frozen_scopes.contains(&project_id.to_string()));

    store.shutdown().unwrap();

    // Ouvrir la base restaurée : en read_only_recovery, coûts postérieurs, inbox et actions invocation/budgets conservés !
    let restored = UnleashStore::open(restore_cfg).unwrap();
    assert_eq!(
        restored.daemon_state().unwrap().mode,
        DaemonMode::ReadOnlyRecovery
    );
    let restored_acc = restored.budget_account(acc_id).unwrap();
    assert_eq!(restored_acc.spent_micros, MicrosAmount::new(250));
    assert_eq!(restored_acc.reserved_micros, MicrosAmount::new(400));

    // Vérifier la capsule épinglée + import DB (inbox post-snapshot + action invocation + budget_reservations)
    let capsule_bytes = restored
        .read_verified_object_bytes(act_res.capsule_object_hash)
        .unwrap();
    let capsule: RestoreCapsule = serde_json::from_slice(&capsule_bytes).unwrap();
    assert!(
        capsule
            .inbox_consumptions
            .iter()
            .any(|(c, ev, _)| c == "worker-inbox-post" && *ev == snap_ev.event_id.to_string())
    );
    assert!(
        capsule
            .budget_reservations
            .iter()
            .any(|br| br.action_id == post_inv_action_id.to_string() && br.state == "unknown")
    );
    let (imported_inbox_cnt, imported_inv_act_origin, imported_inv_br_state): (
        i64,
        String,
        String,
    ) = restored
        .read_snapshot(|conn, _| {
            let ib_cnt: i64 = conn.query_row(
                "SELECT COUNT(*) FROM inbox WHERE consumer_id = 'worker-inbox-post' AND event_id = ?1",
                [snap_ev.event_id.to_string()],
                |r| r.get(0),
            )?;
            let origin: String = conn.query_row(
                "SELECT origin_kind FROM actions WHERE action_id = ?1",
                [post_inv_action_id.to_string()],
                |r| r.get(0),
            )?;
            let br_st: String = conn.query_row(
                "SELECT state FROM budget_reservations WHERE action_id = ?1",
                [post_inv_action_id.to_string()],
                |r| r.get(0),
            )?;
            Ok((ib_cnt, origin, br_st))
        })
        .unwrap();
    assert_eq!(imported_inbox_cnt, 1);
    assert_eq!(imported_inv_act_origin, "invocation");
    assert_eq!(imported_inv_br_state, "unknown");
    // Outbox héritée du snapshot dans le scope gelé NE doit PAS être relancée avant disposition !
    assert!(
        restored
            .lease_outbox_batch("worker-post-snap", 10)
            .unwrap()
            .is_empty()
    );

    // Ancienne commande / ancien token de host100 refusés avec STALE_AUTHORITY_GENERATION
    let old_cmd_err = restored
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: post_cmd_id,
                authority_generation: source_auth,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: project_id.as_uuid(),
                expected_revision: Revision::new(1),
                command_type: "NoopCommand".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::NoopDaemonBump,
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(
        old_cmd_err.reason_code(),
        Some(ReasonCode::StaleAuthorityGeneration)
    );

    // 4. Cas source DB illisible (StorageCorrupt) -> manifeste privé 0600 + coverage=Unknown + gel global
    // 4a. Variante bloquée avant bascule sous source_db_unreadable=true -> RestorationBlocked dans le manifeste 0600, zéro DB cible activée
    let restore_corrupt_blocked_root = local_test_dir("f002-010-corrupt-blocked").unwrap();
    let corrupt_blocked_maint = restore_corrupt_blocked_root.path().join("src-maint");
    let mut corrupt_blocked_cfg = StoreConfig::new(restore_corrupt_blocked_root.path());
    corrupt_blocked_cfg.maintenance_dir = corrupt_blocked_maint.clone();
    let err_unreadable_blocked = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp40_dir.clone(),
            target_data_root: restore_corrupt_blocked_root.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore-corrupt-blocked").unwrap(),
            source_authority_generation: source_auth,
            source_db_unreadable: true,
            source_db_path: None,
            source_artifacts_dir: None,
            neutralizer: Arc::new(Host100Neutralizer {
                allow_neutralization: false,
                source_auth,
            }),
        },
        corrupt_blocked_cfg.clone(),
    )
    .unwrap_err();
    assert!(matches!(
        err_unreadable_blocked,
        StoreError::RestorationBlocked(_)
    ));
    assert!(!corrupt_blocked_cfg.db_path.exists());
    let blocked_maint = UnleashStore::read_maintenance_restore_manifest(&corrupt_blocked_maint)
        .unwrap()
        .expect("manifeste privé blocked attendu");
    assert_eq!(blocked_maint.state, RestoreRequestState::Blocked);
    assert!(blocked_maint.blocked_reason.is_some());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(corrupt_blocked_maint.join("restore-manifest.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // 4b. Variante débloquée sous source_db_unreadable=true -> capsule Unknown + gel global
    let restore_corrupt_root = local_test_dir("f002-010-corrupt-restore").unwrap();
    let corrupt_maint_dir = restore_corrupt_root.path().join("src-maint");
    let mut corrupt_target_cfg = StoreConfig::new(restore_corrupt_root.path());
    corrupt_target_cfg.maintenance_dir = corrupt_maint_dir.clone();
    let act_unknown = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp40_dir,
            target_data_root: restore_corrupt_root.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore-corrupt").unwrap(),
            source_authority_generation: source_auth,
            source_db_unreadable: true,
            source_db_path: None,
            source_artifacts_dir: None,
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        corrupt_target_cfg,
    )
    .unwrap();
    assert_eq!(act_unknown.coverage, CapsuleCoverage::Unknown);
    assert!(act_unknown.global_freeze_active);
    let maint = UnleashStore::read_maintenance_restore_manifest(&corrupt_maint_dir)
        .unwrap()
        .expect("manifeste privé maintenance attendu");
    assert_eq!(maint.restore_id, act_unknown.restore_id);

    // 5. PurgeHistory R7 : remplace le payload d'un événement référencé en tombstone SANS supprimer la ligne
    clock.advance(Duration::from_secs(8 * 86_400));
    restored
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute("UPDATE outbox SET state = 'acked', acked_at_ms = 1000", [])?;
            Ok(())
        })
        .unwrap();
    let purge_ev = restored
        .purge_history(TimestampMs::new(clock.utc_ms() - 7 * 86_400_000))
        .unwrap();
    assert!(purge_ev.wal_truncated);
    let (ev_total, ev_tomb): (i64, i64) = restored
        .read_snapshot(|conn, _| {
            let tot: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
            let tomb: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE payload_state = 'tombstone'",
                [],
                |r| r.get(0),
            )?;
            Ok((tot, tomb))
        })
        .unwrap();
    assert!(ev_total >= 1);
    assert!(ev_tomb >= 1);
}

fn build_event_append_spec(
    mission_id: MissionId,
    project_id: ProjectId,
    payload: Map<String, Value>,
    outbox_destinations: Vec<String>,
) -> unleash_store::EventAppendSpec {
    unleash_store::EventAppendSpec {
        event_type: "MissionCreated".to_owned(),
        aggregate_type: AggregateKind::Mission,
        aggregate_id: mission_id.as_uuid(),
        aggregate_revision: Revision::new(1),
        project_id: Some(project_id),
        correlation_id: Uuid::from_u128(55),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload,
        sensitivity: Sensitivity::Project,
        outbox_destinations,
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn prop_store_fences_budgets_and_receipts_never_regress(
        ops in prop::collection::vec(0u8..4u8, 1..10)
    ) {
        let dir = local_test_dir("f002-prop").unwrap();
        let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
        let auth = store.active_authority_generation().unwrap();
        let acc = AccountId::from_u128(1);
        store.upsert_budget_account(acc, BudgetScopeType::Global, "global", "USD", MicrosAmount::new(500)).unwrap();

        let mut last_fence = 0u64;
        let mut expected_rev = 0u64;
        for (idx, op) in ops.into_iter().enumerate() {
            match op {
                0 => {
                    let lease = store.acquire_lease(unleash_store::AcquireLeaseRequest {
                        resource_kind: "prop".to_owned(),
                        resource_key: "k1".to_owned(),
                        owner_type: "worker".to_owned(),
                        owner_id: format!("w-{idx}"),
                        ttl_ms: 1000,
                    }).unwrap();
                    prop_assert!(lease.fence.as_u64() > last_fence);
                    last_fence = lease.fence.as_u64();
                    store.release_lease(lease.lease_id).unwrap();
                }
                1 => {
                    let cmd = CommandId::from_u128((idx + 1) as u128);
                    let r = store.commit_command(CommitCommandRequest {
                        actor_id: ActorId::new("user:prop").unwrap(),
                        envelope: CommandEnvelope {
                            protocol_version: ProtocolVersion,
                            request_id: RequestId::from_u128((idx + 1) as u128),
                            command_id: cmd,
                            authority_generation: auth,
                            actor_ref: Some("user:prop".to_owned()),
                            project_id: None,
                            aggregate_id: auth.as_uuid(),
                            expected_revision: Revision::new(expected_rev),
                            command_type: "NoopCommand".to_owned(),
                            payload: Map::new(),
                        },
                        target_aggregate: AggregateKind::Daemon,
                        operation_id: None,
                        receipt_state: ReceiptState::Completed,
                        result_ref: None,
                        retention_days_override: None,
                        fence_assertion: None,
                        mutation: CommandMutation::NoopDaemonBump,
                        events: vec![],
                        capacity_reservations: vec![],
                        budget_reservations: vec![],
                    });
                    prop_assert!(r.is_ok());
                    expected_rev += 1;
                }
                _ => {
                    let snap = store.budget_account(acc).unwrap();
                    prop_assert!(snap.spent_micros.as_u64() <= snap.limit_micros.as_u64());
                }
            }
        }
    }
}
