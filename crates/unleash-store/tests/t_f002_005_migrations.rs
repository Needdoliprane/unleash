// T-F002-005 / AC-F002-005 / REQ-PERSIST-005
//! Migrations versionnées, checksums, refus binaire ancien et sauvegarde préalable.

use std::sync::Arc;
use std::time::Duration;

use unleash_store::{
    BackupRetentionTier, CutpointObserver, ExecutionPoolSettings, MonotonicClockPort,
    PrepareBackupRequest, RetentionPolicySettings, SettingsScopeKind, StoreConfig, StoreCutpoint,
    StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{
    Cutpoint, CutpointAction, CutpointController, DeterministicIdGenerator, ManualClock,
    local_test_dir,
};

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct CutpointAdapter(CutpointController);
impl CutpointObserver for CutpointAdapter {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        if matches!(cutpoint, StoreCutpoint::C11MigrationInProgress) {
            self.0
                .hit(Cutpoint::C11)
                .map_err(|e| StoreError::CutpointInjected {
                    cutpoint: "C11".to_owned(),
                    message: e.to_string(),
                })?;
        }
        Ok(())
    }
}

#[test]
fn test_t_f002_005_migration_interrupt_older_binary_checksum_and_backup_gate() {
    // 1. Interrompre la migration V001 via C11 (hit 1)
    let dir_v1 = local_test_dir("f002-005-v1").unwrap();
    let ctrl1 = CutpointController::new();
    ctrl1
        .set_action_on_hit(Cutpoint::C11, 1, CutpointAction::Fail("abort-v1"))
        .unwrap();
    let mut cfg1 = StoreConfig::new(dir_v1.path());
    cfg1.cutpoints = Arc::new(CutpointAdapter(ctrl1));
    assert!(UnleashStore::open(cfg1).is_err());

    // 2. Simuler une migration V002 interrompue (state = 'applying') -> refus à l'ouverture ordinaire
    let dir_v2 = local_test_dir("f002-005-v2").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0005));
    let mut cfg_base = StoreConfig::new(dir_v2.path());
    cfg_base.clock = clock.clone();
    cfg_base.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg_base.id_generator = ids.clone();

    // Ouvrir d'abord à version 1 seulement
    cfg_base.max_supported_schema_version = 1;
    let store_v1 = UnleashStore::open(cfg_base.clone()).unwrap();

    // Tenter une migration v1 -> v2 SANS sauvegarde vérifiée préalable -> refus MigrationBackupRequired
    store_v1.shutdown().unwrap();
    let mut cfg_v2_no_backup = cfg_base.clone();
    cfg_v2_no_backup.max_supported_schema_version = 2;
    assert!(matches!(
        UnleashStore::open(cfg_v2_no_backup.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));

    // Produire une sauvegarde vérifiée sur la v1 puis tenter la montée v1 -> v2 avec interruption C11
    let store_v1_again = UnleashStore::open(cfg_base.clone()).unwrap();
    let backup = store_v1_again
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir_v2.path().join("backups/pre-mig"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store_v1_again.shutdown().unwrap();

    let ctrl2 = CutpointController::new();
    ctrl2
        .set_action_on_hit(Cutpoint::C11, 1, CutpointAction::Fail("abort-v2"))
        .unwrap();
    let mut cfg_v2_interrupt = cfg_v2_no_backup.clone();
    cfg_v2_interrupt.cutpoints = Arc::new(CutpointAdapter(ctrl2));
    assert!(
        UnleashStore::open_with_migration_backup(cfg_v2_interrupt, Some(backup.backup_id)).is_err()
    );

    // Ouverture propre avec sauvegarde validée -> passe en v2
    let store_v2 =
        UnleashStore::open_with_migration_backup(cfg_v2_no_backup.clone(), Some(backup.backup_id))
            .unwrap();
    assert_eq!(store_v2.daemon_state().unwrap().schema_version, 2);
    store_v2.shutdown().unwrap();

    // 3. Ouvrir une DB v2 avec un binaire plus ancien (max_supported_schema_version = 1) -> refus sans écriture !
    let db_mtime_before = std::fs::metadata(dir_v2.path().join("unleash-authority.db"))
        .unwrap()
        .modified()
        .unwrap();
    let err_old_bin = UnleashStore::open(cfg_base.clone()).unwrap_err();
    assert!(matches!(
        err_old_bin,
        StoreError::UnsupportedSchemaVersion {
            found: 2,
            supported_max: 1
        }
    ));
    let db_mtime_after = std::fs::metadata(dir_v2.path().join("unleash-authority.db"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(db_mtime_before, db_mtime_after);

    // 4. Altérer un checksum dans schema_migrations -> refus MigrationChecksumMismatch sans écriture !
    let raw_conn = rusqlite::Connection::open(dir_v2.path().join("unleash-authority.db")).unwrap();
    raw_conn
        .execute(
            "UPDATE schema_migrations SET checksum_sha256 = 'sha256:0000' WHERE version = 1",
            [],
        )
        .unwrap();
    drop(raw_conn);
    let err_csum = UnleashStore::open(cfg_v2_no_backup.clone()).unwrap_err();
    assert!(matches!(
        err_csum,
        StoreError::MigrationChecksumMismatch { version: 1, .. }
    ));

    // 5. Remettre une ligne en état 'applying' -> refus MigrationInterrupted sans écriture !
    let raw_conn2 = rusqlite::Connection::open(dir_v2.path().join("unleash-authority.db")).unwrap();
    raw_conn2
        .execute(
            "UPDATE schema_migrations SET state = 'applying' WHERE version = 2",
            [],
        )
        .unwrap();
    drop(raw_conn2);
    let err_int = UnleashStore::open(cfg_v2_no_backup).unwrap_err();
    assert!(matches!(
        err_int,
        StoreError::MigrationInterrupted { version: 2, .. }
    ));
}

#[test]
fn migration_v003_to_v004_preserves_history_and_requires_backup() {
    let dir = local_test_dir("f018-v003-v004").unwrap();
    let mut v3 = StoreConfig::new(dir.path());
    v3.max_supported_schema_version = 3;
    let store = UnleashStore::open(v3.clone()).unwrap();
    let original: String = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT checksum_sha256 FROM schema_migrations WHERE version = 3",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("backups/v003"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let mut v4 = StoreConfig::new(dir.path());
    v4.max_supported_schema_version = 4;
    assert!(matches!(
        UnleashStore::open(v4.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let store = UnleashStore::open_with_migration_backup(v4, Some(backup.backup_id)).unwrap();
    assert_eq!(store.daemon_state().unwrap().schema_version, 4);
    let preserved: String = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT checksum_sha256 FROM schema_migrations WHERE version = 3",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(preserved, original);
    let snapshot = store.inspect_recovery_snapshot().unwrap();
    assert_eq!(
        snapshot.cursor.stream_id,
        store.daemon_state().unwrap().stream_id
    );
    assert_eq!(snapshot.cursor.sequence.as_u64(), 0);
}

#[test]
fn migration_v004_to_v005_requires_backup_and_preserves_v004_checksum() {
    let dir = local_test_dir("f018-v004-v005").unwrap();
    let mut v4 = StoreConfig::new(dir.path());
    v4.max_supported_schema_version = 4;
    let store = UnleashStore::open(v4).unwrap();
    let checksum: String = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT checksum_sha256 FROM schema_migrations WHERE version=4",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("backup-v004"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let mut v5 = StoreConfig::new(dir.path());
    v5.max_supported_schema_version = 5;
    assert!(matches!(
        UnleashStore::open(v5.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let store = UnleashStore::open_with_migration_backup(v5, Some(backup.backup_id)).unwrap();
    assert_eq!(store.daemon_state().unwrap().schema_version, 5);
    let (retained,tables):(String,i64)=store.read_snapshot(|conn,_| {
        let retained=conn.query_row("SELECT checksum_sha256 FROM schema_migrations WHERE version=4",[],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let tables=conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('broker_intents','broker_evaluations','broker_phase_records','broker_dispatches') AND type='table'",[],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        Ok((retained,tables))
    }).unwrap();
    assert_eq!(retained, checksum);
    assert_eq!(tables, 4);
}

#[test]
fn migration_v020_to_v021_preserves_settings_without_granting_model_authority() {
    let dir = local_test_dir("f012-policy-v020").unwrap();
    let mut previous = StoreConfig::new(dir.path());
    previous.max_supported_schema_version = 20;
    let store = UnleashStore::open(previous).unwrap();
    store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            tx.execute(
                "INSERT INTO settings (scope_kind, scope_id, revision, execution_pool_json,
                     retention_policy_json, extra_typed_json, updated_at_ms, updated_by_actor)
                 VALUES ('user', 'default', 7, ?1, ?2, ?3, ?4, 'fixture')",
                rusqlite::params![
                    serde_json::to_string(&ExecutionPoolSettings::default())?,
                    serde_json::to_string(&RetentionPolicySettings::default())?,
                    r#"{"model_policy_versions":[{"policy_id":"legacy-metadata"}]}"#,
                    now.as_i64(),
                ],
            )?;
            Ok(())
        })
        .unwrap();
    let before = store
        .read_settings(SettingsScopeKind::User, "default")
        .unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("v020-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();

    let mut next = StoreConfig::new(dir.path());
    next.max_supported_schema_version = 21;
    assert!(matches!(
        UnleashStore::open(next.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 21);
    assert_eq!(
        upgraded
            .read_settings(SettingsScopeKind::User, "default")
            .unwrap(),
        before
    );
    let history: String = upgraded
        .read_snapshot(|db, _| {
            db.query_row(
                "SELECT model_policy_history_json FROM settings
                 WHERE scope_kind='user' AND scope_id='default'",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(history, "[]");
    upgraded.shutdown().unwrap();
}

#[test]
fn migrations_v005_to_v007_preserve_legacy_jobs_without_fabricated_requests() {
    let dir = local_test_dir("f018-v005-v007").unwrap();
    let mut v5 = StoreConfig::new(dir.path());
    v5.max_supported_schema_version = 5;
    let store = UnleashStore::open(v5).unwrap();
    let checksum: String = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT checksum_sha256 FROM schema_migrations WHERE version=5",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("v005-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let v7 = StoreConfig::new(dir.path());
    assert!(matches!(
        UnleashStore::open(v7.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let store = UnleashStore::open_with_migration_backup(v7, Some(backup.backup_id)).unwrap();
    assert_eq!(
        store.daemon_state().unwrap().schema_version,
        unleash_store::CURRENT_SCHEMA_VERSION
    );
    let (retained, requests, jobs): (String, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT checksum_sha256 FROM schema_migrations WHERE version=5",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?,
                conn.query_row("SELECT COUNT(*) FROM backup_requests", [], |r| r.get(0))
                    .map_err(StoreError::from_rusqlite)?,
                conn.query_row(
                    "SELECT COUNT(*) FROM backup_jobs WHERE state='complete'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?,
            ))
        })
        .unwrap();
    assert_eq!(retained, checksum);
    assert_eq!(requests, 0);
    assert_eq!(jobs, 1);
    assert!(
        store
            .inspect_backup_request(backup.backup_id)
            .unwrap()
            .is_none()
    );
}
