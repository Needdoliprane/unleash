// T-F018-005 / AC-F018-005 / REQ-REC-005
//! Rejet `STALE_AUTHORITY_GENERATION` avant lookup même si `expected_revision` coïncide,
//! et persistance/import du manifeste privé de maintenance `0600` quand la base source est corrompue.

use std::sync::Arc;

use serde_json::Map;
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, IdGeneratorPort, ProjectId, RequestId, Revision,
    Sha256Digest,
};
use unleash_protocol::{CommandEnvelope, ErrorCode, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    AggregateKind, BackupRetentionTier, CommandMutation, CommitCommandRequest,
    EmptyExecutorNeutralizer, ExecutorInventoryItem, ExecutorNeutralizer, PrepareBackupRequest,
    RestoreBackupRequest, RestoreRequestState, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

struct ConditionalNeutralizer {
    allow: bool,
}

impl ExecutorNeutralizer for ConditionalNeutralizer {
    fn inventory_and_neutralize(
        &self,
        source_authority_generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, StoreError> {
        if !self.allow {
            return Err(StoreError::RestorationBlocked(
                "exécuteur source actif".to_owned(),
            ));
        }
        Ok(vec![ExecutorInventoryItem {
            executor_id: "exec-1".to_owned(),
            authority_generation: source_authority_generation,
            daemon_epoch: DaemonEpoch::new(1),
            boot_identity: "boot-005".to_owned(),
            neutralized: true,
        }])
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario F018-005 complet : lignée stale avant lookup et manifeste privé 0600"
)]
fn test_t_f018_005_stale_generation_before_lookup_and_private_manifest_import() {
    let src_dir = local_test_dir("f018-005-src").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0005));
    let mut src_cfg = StoreConfig::new(src_dir.path());
    src_cfg.clock = clock.clone();
    src_cfg.id_generator = ids.clone();

    let store = UnleashStore::open(src_cfg.clone()).unwrap();
    let auth_1 = store.active_authority_generation().unwrap();
    let stream_1 = store.daemon_state().unwrap().stream_id;
    let admin = ActorId::new("user:louis").unwrap();

    // Créer projet p1 révision 1 (enregistré dans le snapshot) + autre commande postérieure
    let p1 = ProjectId::from_uuid(ids.new_uuid());
    let known_cmd_id = ids.new_command_id();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: known_cmd_id,
                authority_generation: auth_1,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(p1),
                aggregate_id: p1.as_uuid(),
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
                project_id: p1,
                name: "proj-005".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let bkp_dir = src_dir.path().join("bkp");
    store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: bkp_dir.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();

    // 1. Scénario base source illisible (`StorageCorrupt` / non inscriptible) :
    //    - 1ere tentative RestoreBackup bloquée -> écrit manifeste privé `0600`
    let target_dir = local_test_dir("f018-005-dst").unwrap();
    let mut dst_cfg = StoreConfig::new(target_dir.path());
    dst_cfg.clock = clock;
    dst_cfg.id_generator = ids.clone();

    let cmd_blocked = ids.new_command_id();
    let dig_blocked = Sha256Digest::from_bytes(b"restore-blocked-005");
    let err_blocked = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir.clone(),
            target_data_root: target_dir.path().to_path_buf(),
            command_id: cmd_blocked,
            actor_id: "user:louis".to_owned(),
            command_digest: dig_blocked,
            source_authority_generation: auth_1,
            source_db_unreadable: true,
            source_db_path: Some(src_cfg.db_path.clone()),
            source_artifacts_dir: None,
            neutralizer: Arc::new(ConditionalNeutralizer { allow: false }),
        },
        dst_cfg.clone(),
    )
    .unwrap_err();
    assert!(matches!(err_blocked, StoreError::RestorationBlocked(_)));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::metadata(dst_cfg.maintenance_dir.join("restore-manifest.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(perm & 0o777, 0o600);
    }

    // Recherche idempotente & InspectRecovery offline lisent le manifeste privé avant bascule
    let found_manifest = UnleashStore::lookup_restore_receipt(&dst_cfg, "user:louis", cmd_blocked)
        .unwrap()
        .expect("manifeste privé doit retrouver la tentative bloquée");
    assert_eq!(found_manifest.state, RestoreRequestState::Blocked);
    let rid_first = found_manifest.restore_id;

    let offline_report = UnleashStore::inspect_recovery_offline(&dst_cfg).unwrap();
    assert_eq!(
        offline_report
            .maintenance_manifest
            .as_ref()
            .map(|m| m.restore_id),
        Some(rid_first)
    );

    // Retry exact de la commande bloquée -> renvoie le même blocage sans créer un second restore_id !
    let retry_blocked_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir.clone(),
            target_data_root: target_dir.path().to_path_buf(),
            command_id: cmd_blocked,
            actor_id: "user:louis".to_owned(),
            command_digest: dig_blocked,
            source_authority_generation: auth_1,
            source_db_unreadable: true,
            source_db_path: Some(src_cfg.db_path.clone()),
            source_artifacts_dir: None,
            neutralizer: Arc::new(ConditionalNeutralizer { allow: true }),
        },
        dst_cfg.clone(),
    )
    .unwrap_err();
    assert!(matches!(
        retry_blocked_err,
        StoreError::RestorationBlocked(_)
    ));
    let after_retry = UnleashStore::lookup_restore_receipt(&dst_cfg, "user:louis", cmd_blocked)
        .unwrap()
        .unwrap();
    assert_eq!(after_retry.restore_id, rid_first);

    // Retry même command_id avec digest différent -> IdempotencyConflict !
    let conflict_err = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir.clone(),
            target_data_root: target_dir.path().to_path_buf(),
            command_id: cmd_blocked,
            actor_id: "user:louis".to_owned(),
            command_digest: Sha256Digest::from_bytes(b"different-digest"),
            source_authority_generation: auth_1,
            source_db_unreadable: true,
            source_db_path: Some(src_cfg.db_path),
            source_artifacts_dir: None,
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        dst_cfg.clone(),
    )
    .unwrap_err();
    assert_eq!(conflict_err.error_code(), ErrorCode::IdempotencyConflict);

    // Nouvelle commande RestoreBackup après neutralisation réussit et importe l'historique du manifeste dans restore_imports / restore_manifest_records
    let cmd_ok = ids.new_command_id();
    let act_res = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: bkp_dir,
            target_data_root: target_dir.path().to_path_buf(),
            command_id: cmd_ok,
            actor_id: "user:louis".to_owned(),
            command_digest: Sha256Digest::from_bytes(b"restore-ok-005"),
            source_authority_generation: auth_1,
            source_db_unreadable: true,
            source_db_path: None,
            source_artifacts_dir: None,
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        dst_cfg.clone(),
    )
    .unwrap();
    assert_ne!(act_res.target_authority_generation, auth_1);
    assert_ne!(act_res.target_stream_id, stream_1);

    let restored = UnleashStore::open(dst_cfg).unwrap();
    let rec = restored.inspect_recovery().unwrap();
    assert!(rec.restore_imports.iter().any(|imp| {
        imp.restore_id == act_res.restore_id
            && imp
                .imported_manifest_entries
                .iter()
                .any(|e| e.restore_id == rid_first && e.state == RestoreRequestState::Blocked)
    }));

    // 2. Rejet STALE_AUTHORITY_GENERATION AVANT lookup même quand :
    //    - expected_revision (1) coïncide exactement avec p1.revision (1)
    //    - (actor_id, command_id) existe dans command_receipts (known_cmd_id) OU est nouveau !
    for (cid, label) in [
        (known_cmd_id, "rejeu command_id existant"),
        (ids.new_command_id(), "nouvelle commande ancienne lignée"),
    ] {
        let stale_err = restored
            .commit_command(CommitCommandRequest {
                actor_id: admin.clone(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_uuid(ids.new_uuid()),
                    command_id: cid,
                    authority_generation: auth_1,
                    actor_ref: Some("user:louis".to_owned()),
                    project_id: Some(p1),
                    aggregate_id: p1.as_uuid(),
                    expected_revision: Revision::new(1),
                    command_type: "ReviseProject".to_owned(),
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
            stale_err.error_code(),
            ErrorCode::StaleFence,
            "échec {label}"
        );
        assert_eq!(
            stale_err.reason_code(),
            Some(ReasonCode::StaleAuthorityGeneration),
            "raison attendue pour {label}"
        );
    }

    assert_eq!(restored.get_project(p1).unwrap().revision, Revision::new(1));
}
