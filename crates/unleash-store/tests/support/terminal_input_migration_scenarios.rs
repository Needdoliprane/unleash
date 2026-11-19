//! REQ-PERSIST-005 / REQ-TERM-005: populated V013 -> V014, with an actual local
//! backup and C11 rollback. Historical process facts remain synthetic SQLite rows.
use super::*;

fn checksums_through_v013(store: &UnleashStore) -> Vec<(i64, String)> {
    store.read_snapshot(|conn,_| {
        let mut query=conn.prepare("SELECT version,checksum_sha256 FROM schema_migrations WHERE version<=13 ORDER BY version")?;
        Ok(query.query_map([],|row|Ok((row.get(0)?,row.get(1)?)))?.collect::<Result<Vec<_>,_>>()?)
    }).unwrap()
}

fn v014_absent(store: &UnleashStore) {
    store
        .read_snapshot(|conn, _| {
            let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            let tables: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'terminal_input_%'",
                [],
                |row| row.get(0),
            )?;
            let migrations: i64 = conn.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version=14",
                [],
                |row| row.get(0),
            )?;
            assert_eq!((version, tables, migrations), (13, 0, 0));
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_f011_005_v014_rolls_back_then_preserves_populated_v013_and_receipts() {
    let (store, old_config, _clock, mut preparation, _directory) =
        fixture_at_schema(Arc::new(NoopCutpointObserver), 13);
    seed_historical_terminal(&store, preparation.process_spec.environment.digest);
    let first = request(&store, preparation.clone(), 14001);
    preparation.purpose_key = "second-v014-migration".into();
    let second = request(&store, preparation, 14002);
    let actor = first.actor_id.clone();
    let commands = [first.envelope.command_id, second.envelope.command_id];
    store.commit_command(first).unwrap();
    store.commit_command(second).unwrap();
    let allocations = [read(&store, 14001), read(&store, 14002)];
    let receipts = commands.map(|id| store.inspect_command_receipt(&actor, id).unwrap());
    let history = legacy_rows(&store);
    let checksums = checksums_through_v013(&store);
    assert_eq!(checksums.len(), 13);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: old_config.data_root.join("before-v014"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    Arc::try_unwrap(store).unwrap().shutdown().unwrap();
    let mut next = old_config.clone();
    next.max_supported_schema_version = 14;
    let mut interrupted = next.clone();
    interrupted.cutpoints = Arc::new(FailAt {
        armed: AtomicBool::new(true),
        point: StoreCutpoint::C11MigrationInProgress,
    });
    assert!(matches!(
        UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let rolled = UnleashStore::open(old_config).unwrap();
    assert_eq!(rolled.daemon_state().unwrap().schema_version, 13);
    v014_absent(&rolled);
    assert_eq!(checksums_through_v013(&rolled), checksums);
    assert_eq!(legacy_rows(&rolled), history);
    assert_eq!([read(&rolled, 14001), read(&rolled, 14002)], allocations);
    assert_eq!(
        commands.map(|id| rolled.inspect_command_receipt(&actor, id).unwrap()),
        receipts
    );
    rolled.shutdown().unwrap();
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 14);
    assert_eq!(checksums_through_v013(&upgraded), checksums);
    assert_eq!(legacy_rows(&upgraded), history);
    assert_eq!(
        [read(&upgraded, 14001), read(&upgraded, 14002)],
        allocations
    );
    assert_eq!(
        commands.map(|id| upgraded.inspect_command_receipt(&actor, id).unwrap()),
        receipts
    );
    upgraded
        .read_snapshot(|conn, _| {
            let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            assert_eq!(version, 14);
            let applied: (String, Option<String>) = conn.query_row(
                "SELECT state,pre_migration_backup_id FROM schema_migrations WHERE version=14",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(
                applied,
                ("applied".into(), Some(backup.backup_id.to_string()))
            );
            for name in [
                "terminal_input_intents",
                "terminal_input_one_pending",
                "terminal_input_sequence_reserved",
                "terminal_input_initial",
                "terminal_input_immutable",
                "terminal_input_transition",
                "terminal_input_durable",
            ] {
                assert!(
                    conn.prepare("SELECT 1 FROM sqlite_schema WHERE name=?1")?
                        .exists([name])?,
                    "missing V014 object {name}"
                );
            }
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM terminal_input_intents", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(count, 0);
            assert!(!conn.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
}
