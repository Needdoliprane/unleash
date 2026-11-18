//! REQ-REC-005/007 : une ancienne commande `RestoreBackup` ne réactive jamais une cible déjà activée.
use std::sync::Arc;
use unleash_domain::{AuthorityGeneration, CommandId, Sha256Digest};
use unleash_store::{
    EmptyExecutorNeutralizer, RestoreBackupRequest, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;

#[test]
fn stale_restore_is_rejected_before_backup_lookup_or_new_effect() {
    let directory = local_test_dir("f018-007-stale-restore").unwrap();
    let config = StoreConfig::new(directory.path());
    let store = UnleashStore::open(config.clone()).unwrap();
    let before = store.daemon_state().unwrap();
    let stale = AuthorityGeneration::from_u128(9);
    assert_ne!(stale, before.authority_generation);
    let error = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: directory.path().join("missing-backup"),
            target_data_root: directory.path().to_owned(),
            actor_id: "user:louis".to_owned(),
            command_id: CommandId::from_u128(7),
            command_digest: Sha256Digest::from_bytes(b"old-restore"),
            source_authority_generation: stale,
            source_db_unreadable: true,
            source_db_path: None,
            source_artifacts_dir: None,
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        config.clone(),
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::StaleAuthorityGeneration { .. }));
    let after = store.daemon_state().unwrap();
    assert_eq!(before.authority_generation, after.authority_generation);
    assert_eq!(before.stream_id, after.stream_id);
    assert_eq!(before.revision, after.revision);
    let count: i64 = store
        .read_snapshot(|connection, _| {
            connection
                .query_row("SELECT COUNT(*) FROM restore_imports", [], |row| row.get(0))
                .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        UnleashStore::read_maintenance_restore_manifest(&config.maintenance_dir)
            .unwrap()
            .is_none()
    );
}
