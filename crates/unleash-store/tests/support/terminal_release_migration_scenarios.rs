//! REQ-PERSIST-005 / REQ-TERM-010: populated V017 -> V018, with an actual local
//! backup and C11 rollback. Historical process facts remain synthetic `SQLite` rows.
use super::*;

fn checksums_through_v017(store: &UnleashStore) -> Vec<(i64, String)> {
    store.read_snapshot(|conn,_| {
        let mut query=conn.prepare("SELECT version,checksum_sha256 FROM schema_migrations WHERE version<=17 ORDER BY version")?;
        Ok(query.query_map([],|row|Ok((row.get(0)?,row.get(1)?)))?.collect::<Result<Vec<_>,_>>()?)
    }).unwrap()
}

fn v018_absent(store: &UnleashStore) {
    store
        .read_snapshot(|conn, _| {
            let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            let tables: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'terminal_release_%'",
                [],
                |row| row.get(0),
            )?;
            let migrations: i64 = conn.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version=18",
                [],
                |row| row.get(0),
            )?;
            assert_eq!((version, tables, migrations), (17, 0, 0));
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_f011_010_v018_rolls_back_then_preserves_populated_v017_and_receipts() {
    let (store, old_config, _clock, mut preparation, _directory) =
        fixture_at_schema(Arc::new(NoopCutpointObserver), 17);
    seed_historical_terminal(&store, preparation.process_spec.environment.digest);
    let first = request(&store, preparation.clone(), 18001);
    preparation.purpose_key = "second-v018-migration".into();
    let second = request(&store, preparation, 18002);
    let actor = first.actor_id.clone();
    let commands = [first.envelope.command_id, second.envelope.command_id];
    store.commit_command(first).unwrap();
    store.commit_command(second).unwrap();
    let allocations = [read(&store, 18001), read(&store, 18002)];
    let receipts = commands.map(|id| store.inspect_command_receipt(&actor, id).unwrap());
    let history = legacy_rows(&store);
    let checksums = checksums_through_v017(&store);
    assert_eq!(checksums.len(), 17);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: old_config.data_root.join("before-v018"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    Arc::try_unwrap(store).unwrap().shutdown().unwrap();
    let mut next = old_config.clone();
    next.max_supported_schema_version = 18;
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
    assert_eq!(rolled.daemon_state().unwrap().schema_version, 17);
    v018_absent(&rolled);
    assert_eq!(checksums_through_v017(&rolled), checksums);
    assert_eq!(legacy_rows(&rolled), history);
    assert_eq!([read(&rolled, 18001), read(&rolled, 18002)], allocations);
    assert_eq!(
        commands.map(|id| rolled.inspect_command_receipt(&actor, id).unwrap()),
        receipts
    );
    rolled.shutdown().unwrap();
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 18);
    assert_eq!(checksums_through_v017(&upgraded), checksums);
    assert_eq!(legacy_rows(&upgraded), history);
    assert_eq!(
        [read(&upgraded, 18001), read(&upgraded, 18002)],
        allocations
    );
    assert_eq!(
        commands.map(|id| upgraded.inspect_command_receipt(&actor, id).unwrap()),
        receipts
    );
    upgraded
        .read_snapshot(|conn, _| {
            let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            assert_eq!(version, 18);
            let applied: (String, Option<String>) = conn.query_row(
                "SELECT state,pre_migration_backup_id FROM schema_migrations WHERE version=18",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(
                applied,
                ("applied".into(), Some(backup.backup_id.to_string()))
            );
            for name in [
                "terminal_release_intents",
                "terminal_release_one_pending",
                "terminal_release_initial",
                "terminal_release_immutable",
                "terminal_release_transition",
                "terminal_release_durable",
                "terminal_release_exclusive",
                "terminal_input_no_pending_release",
                "terminal_control_no_pending_release",
                "terminal_renewal_no_pending_release",
            ] {
                assert!(
                    conn.prepare("SELECT 1 FROM sqlite_schema WHERE name=?1")?
                        .exists([name])?,
                    "missing V018 object {name}"
                );
            }
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM terminal_release_intents", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(count, 0);
            assert!(!conn.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
}
