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

/// Ligne `backup_requests` telle qu'elle est stockée : identité, état, motif, révision.
type BackupRequestRow = (String, String, Option<String>, i64);

fn backup_request_command(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
    backup: unleash_domain::BackupId,
) -> unleash_store::CommitCommandRequest {
    use unleash_domain::{ActorId, IdGeneratorPort, Revision};
    use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
    use unleash_store::{AggregateKind, CommandMutation, CommitCommandRequest};
    CommitCommandRequest {
        actor_id: ActorId::new("p-sys").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: unleash_domain::RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some("p-sys".to_owned()),
            project_id: None,
            aggregate_id: backup.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "RequestBackup".to_owned(),
            payload: serde_json::Map::new(),
        },
        target_aggregate: AggregateKind::BackupJob,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::RequestBackup { backup_id: backup },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn backup_request_rows(
    db: &rusqlite::Connection,
    ids: &[unleash_domain::BackupId],
) -> Result<Vec<BackupRequestRow>, StoreError> {
    ids.iter()
        .map(|id| {
            db.query_row(
                "SELECT backup_id, state, block_reason, revision FROM backup_requests WHERE backup_id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .collect()
}

/// Déclencheurs de `backup_requests`, nombre de ses index et objets restés sur le nom temporaire de la V022.
fn backup_request_schema(
    db: &rusqlite::Connection,
) -> Result<(Vec<String>, i64, i64), StoreError> {
    let triggers = db
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name='backup_requests' ORDER BY name")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let indexes = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='backup_requests'",
        [],
        |row| row.get(0),
    )?;
    let stray = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'backup_requests_v022%'",
        [],
        |row| row.get(0),
    )?;
    Ok((triggers, indexes, stray))
}

fn backup_request_table_sql(db: &rusqlite::Connection) -> String {
    db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='backup_requests'",
        [],
        |row| row.get(0),
    )
    .map_err(StoreError::from_rusqlite)
    .unwrap()
}

/// Écrit un motif par SQL brut : seule la contrainte CHECK de la colonne peut refuser.
fn set_backup_block_reason(
    store: &UnleashStore,
    id: unleash_domain::BackupId,
    reason: &str,
) -> Result<(), StoreError> {
    let reason = reason.to_owned();
    store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
        tx.execute(
            "UPDATE backup_requests SET block_reason=?2 WHERE backup_id=?1",
            rusqlite::params![id.to_string(), reason],
        )?;
        Ok(())
    })
}

/// Enregistrement de la V022 dans `schema_migrations` : nom, somme de contrôle, état, sauvegarde préalable, nombre de lignes.
fn v022_record(
    db: &rusqlite::Connection,
) -> Result<(String, String, String, Option<String>, i64), StoreError> {
    Ok((
        db.query_row(
            "SELECT name FROM schema_migrations WHERE version=22",
            [],
            |row| row.get(0),
        )?,
        db.query_row(
            "SELECT checksum_sha256 FROM schema_migrations WHERE version=22",
            [],
            |row| row.get(0),
        )?,
        db.query_row(
            "SELECT state FROM schema_migrations WHERE version=22",
            [],
            |row| row.get(0),
        )?,
        db.query_row(
            "SELECT pre_migration_backup_id FROM schema_migrations WHERE version=22",
            [],
            |row| row.get(0),
        )?,
        db.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version=22",
            [],
            |row| row.get(0),
        )?,
    ))
}

fn read_only(db_path: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}

/// Entier unique renvoyé par une requête de contrôle en lecture.
fn scalar(db: &rusqlite::Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Base V021 peuplée, lue telle quelle : une demande réservée, une réclamée,
/// une bloquée avant dispatch par un motif de la V007.
struct V021Fixture {
    order: [unleash_domain::BackupId; 3],
    rows: Vec<BackupRequestRow>,
    schema: (Vec<String>, i64, i64),
    table_sql: String,
}

fn populated_v021(
    root: &std::path::Path,
    ids: &Arc<DeterministicIdGenerator>,
) -> (UnleashStore, V021Fixture) {
    use unleash_domain::{BackupId, IdGeneratorPort};
    let mut v21 = StoreConfig::new(root);
    v21.max_supported_schema_version = 21;
    v21.id_generator = ids.clone();
    let store = UnleashStore::open(v21).unwrap();
    assert_eq!(store.daemon_state().unwrap().schema_version, 21);
    store.register_principal("p-sys", "system", "p-sys").unwrap();
    store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, _| {
            tx.execute("INSERT INTO guard_configs (config_id,guard_mode,endpoint,transport,updated_by_actor,updated_at_ms) VALUES ('default','guard_required','https://guard.invalid','https_tls','p-sys',0)",[]).map_err(StoreError::from_rusqlite)?;
            Ok(())
        })
        .unwrap();
    let reserved = BackupId::from_uuid(ids.new_uuid());
    store
        .commit_command(backup_request_command(&store, ids, reserved))
        .unwrap();
    let claimed = BackupId::from_uuid(ids.new_uuid());
    store
        .commit_command(backup_request_command(&store, ids, claimed))
        .unwrap();
    store
        .claim_backup_request(claimed, store.active_authority_generation().unwrap())
        .unwrap();
    let refused = BackupId::from_uuid(ids.new_uuid());
    store
        .commit_command(backup_request_command(&store, ids, refused))
        .unwrap();
    store
        .claim_backup_request(refused, store.active_authority_generation().unwrap())
        .unwrap();
    store
        .block_claimed_backup(
            refused,
            store.active_authority_generation().unwrap(),
            unleash_store::BackupBlockReason::GuardProfileUnavailable,
        )
        .unwrap();
    let order = [reserved, claimed, refused];
    let (rows, schema, table_sql) = store
        .read_snapshot(|db, _| {
            Ok((
                backup_request_rows(db, &order)?,
                backup_request_schema(db)?,
                backup_request_table_sql(db),
            ))
        })
        .unwrap();
    (
        store,
        V021Fixture {
            order,
            rows,
            schema,
            table_sql,
        },
    )
}

// T-F002-005 / AC-F002-005 / REQ-PERSIST-005 ; ensemble fermé des motifs : REQ-SEC-007.
// La V022 reconstruit `backup_requests` pour étendre la contrainte CHECK de `block_reason`
// (V007) sans perdre une seule demande : base V021 peuplée, rollback injecté, rejeu exact.
#[test]
fn migration_v021_to_v022_keeps_every_backup_request_and_widens_the_closed_block_reasons() {
    use sha2::{Digest, Sha256};
    const ORDER: [&str; 3] = ["reserved", "claimed", "blocked"];
    const TRIGGERS: [&str; 3] = [
        "backup_block_reason_immutable",
        "backup_requests_immutable",
        "backup_requests_no_delete",
    ];
    const V007_CHECK: &str =
        "IN ('guard_profile_unavailable','isolation_profile_unavailable','authorization_denied')";
    let dir = local_test_dir("f017-013-v021-v022").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF017_1322));
    let (store, fixture) = populated_v021(dir.path(), &ids);
    let V021Fixture {
        order,
        rows: before,
        schema: schema_before,
        table_sql: sql_before,
    } = fixture;
    let [reserved, claimed, _refused] = order;
    assert_eq!(
        before.iter().map(|row| row.1.as_str()).collect::<Vec<_>>(),
        ORDER
    );
    assert_eq!(before[2].2.as_deref(), Some("guard_profile_unavailable"));
    assert_eq!(schema_before.0, TRIGGERS);
    assert_eq!(schema_before.2, 0);
    assert!(sql_before.contains(V007_CHECK));
    // La CHECK de la V007 ferme l'ensemble à trois motifs : SQLite lui-même refuse tout autre texte.
    assert!(set_backup_block_reason(&store, reserved, "policy_changed").is_err());
    assert!(set_backup_block_reason(&store, claimed, "preflight_failed").is_err());
    assert_eq!(
        store
            .read_snapshot(|db, _| backup_request_rows(db, &order))
            .unwrap(),
        before
    );
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("v021-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    let db_path = StoreConfig::new(dir.path()).db_path;
    store.shutdown().unwrap();

    // Rollback injecté pendant la V022 : la base reste une V021 intacte, table comprise.
    let controller = CutpointController::new();
    controller
        .set_action_on_hit(Cutpoint::C11, 1, CutpointAction::Fail("abort-v022"))
        .unwrap();
    let mut interrupted = StoreConfig::new(dir.path());
    interrupted.cutpoints = Arc::new(CutpointAdapter(controller));
    assert!(matches!(
        UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let disk = read_only(&db_path);
    assert_eq!(
        disk.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        21
    );
    assert_eq!(scalar(&disk, "SELECT COUNT(*) FROM schema_migrations WHERE version=22"), 0);
    assert_eq!(backup_request_rows(&disk, &order).unwrap(), before);
    assert_eq!(backup_request_schema(&disk).unwrap(), schema_before);
    assert_eq!(backup_request_table_sql(&disk), sql_before);

    // Montée réelle : lignes, déclencheurs et index conservés ; ensemble fermé étendu, jamais ouvert.
    let upgraded = UnleashStore::open_with_migration_backup(
        StoreConfig::new(dir.path()),
        Some(backup.backup_id),
    )
    .unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 22);
    let (after, schema_after) = upgraded
        .read_snapshot(|db, _| {
            Ok((
                backup_request_rows(db, &order)?,
                backup_request_schema(db)?,
            ))
        })
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(schema_after, schema_before);
    assert!(set_backup_block_reason(&upgraded, reserved, "texte libre").is_err());
    set_backup_block_reason(&upgraded, reserved, "policy_changed").unwrap();
    assert!(set_backup_block_reason(&upgraded, claimed, "").is_err());
    set_backup_block_reason(&upgraded, claimed, "preflight_failed").unwrap();
    // Un motif engagé reste immutable (déclencheur de la V007, recréé).
    assert!(set_backup_block_reason(&upgraded, reserved, "preflight_failed").is_err());
    let recorded = upgraded
        .read_snapshot(|db, _| v022_record(db))
        .unwrap();
    upgraded.shutdown().unwrap();
    assert_eq!(recorded.0, "v022_backup_block_reasons");
    assert_eq!(recorded.2, "applied");
    assert_eq!(recorded.3.as_deref(), Some(backup.backup_id.to_string().as_str()));
    assert_eq!(recorded.4, 1);
    let ddl = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/migrations/v022_backup_block_reasons.sql"
    ))
    .expect("DDL de la V022 présent dans les sources");
    assert_eq!(
        recorded.1,
        format!("sha256:{}", hex::encode(Sha256::digest(ddl.as_bytes())))
    );

    // Rejeu exact : réouverture sans nouvelle migration, même somme de contrôle ; binaire ancien refusé.
    let replayed = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    assert_eq!(replayed.daemon_state().unwrap().schema_version, 22);
    assert_eq!(
        replayed
            .read_snapshot(|db, _| v022_record(db))
            .unwrap(),
        recorded
    );
    replayed.shutdown().unwrap();
    let mut older = StoreConfig::new(dir.path());
    older.max_supported_schema_version = 21;
    assert!(matches!(
        UnleashStore::open(older),
        Err(StoreError::UnsupportedSchemaVersion {
            found: 22,
            supported_max: 21
        })
    ));
}
