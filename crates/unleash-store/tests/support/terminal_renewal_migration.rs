//! Additive renewal schema, separately qualified from native daemon behavior.
use super::*;

fn checksums(store: &UnleashStore) -> Vec<(i64, String)> {
    store.read_snapshot(|conn,_| {
        let mut query = conn.prepare("SELECT version,checksum_sha256 FROM schema_migrations WHERE version<=15 ORDER BY version")?;
        Ok(query.query_map([],|row| Ok((row.get(0)?,row.get(1)?)))?
            .collect::<Result<Vec<_>,_>>()?)
    }).unwrap()
}

#[test]
fn t_f011_v016_rolls_back_then_preserves_populated_v015() {
    let (store, old, _clock, preparation, _root) =
        fixture_at_schema(Arc::new(NoopCutpointObserver), 15);
    seed_historical_terminal(&store, preparation.process_spec.environment.digest);
    let mut second = preparation.clone();
    second.purpose_key = "second-renewal-migration-terminal".into();
    store
        .commit_command(request(&store, preparation, 1601))
        .unwrap();
    store.commit_command(request(&store, second, 1602)).unwrap();
    let first_before = read(&store, 1601);
    let second_before = read(&store, 1602);
    let rows = legacy_rows(&store);
    let before = checksums(&store);
    assert_eq!(before.len(), 15);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: old.data_root.join("before-v016"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".into(),
        })
        .unwrap();
    Arc::try_unwrap(store).unwrap().shutdown().unwrap();
    let mut next = old.clone();
    next.max_supported_schema_version = 16;
    let mut interrupted = next.clone();
    interrupted.cutpoints = Arc::new(FailAt {
        armed: AtomicBool::new(true),
        point: StoreCutpoint::C11MigrationInProgress,
    });
    assert!(matches!(
        UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let rolled_back = UnleashStore::open(old).unwrap();
    assert_eq!(rolled_back.daemon_state().unwrap().schema_version, 15);
    assert_eq!(checksums(&rolled_back), before);
    assert_eq!(legacy_rows(&rolled_back), rows);
    assert_eq!(read(&rolled_back, 1601), first_before);
    assert_eq!(read(&rolled_back, 1602), second_before);
    rolled_back
        .read_snapshot(|conn, _| {
            assert!(
                !conn
                    .prepare("SELECT 1 FROM sqlite_master WHERE name='terminal_renewal_intents'")?
                    .exists([])?
            );
            assert!(
                !conn
                    .prepare("SELECT 1 FROM schema_migrations WHERE version=16")?
                    .exists([])?
            );
            Ok(())
        })
        .unwrap();
    rolled_back.shutdown().unwrap();
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 16);
    assert_eq!(checksums(&upgraded), before);
    assert_eq!(legacy_rows(&upgraded), rows);
    assert_eq!(read(&upgraded, 1601), first_before);
    assert_eq!(read(&upgraded, 1602), second_before);
    upgraded.read_snapshot(|conn,_| {
        assert_eq!(conn.query_row("PRAGMA user_version",[],|row|row.get::<_,i64>(0))?,16);
        assert!(!conn.prepare("SELECT 1 FROM terminal_renewal_intents")?.exists([])?);
        assert!(!conn.prepare("PRAGMA foreign_key_check")?.exists([])?);
        for name in ["terminal_renewal_one_pending","terminal_renewal_command",
            "terminal_renewal_initial","terminal_renewal_immutable","terminal_renewal_transition",
            "terminal_renewal_durable","terminal_renewal_exclusive",
            "terminal_renewal_transfer_links","terminal_renewal_transfer_link_initial",
            "terminal_renewal_transfer_link_immutable","terminal_renewal_transfer_link_durable",
            "terminal_control_no_pending_renewal","terminal_input_no_pending_renewal"] {
            assert!(conn.prepare("SELECT 1 FROM sqlite_master WHERE name=?1")?.exists([name])?);
        }
        let tied:bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=16 AND pre_migration_backup_id=?1 AND state='applied')",
            [backup.backup_id.to_string()],|row|row.get(0))?;
        assert!(tied);
        Ok(())
    }).unwrap();
}
