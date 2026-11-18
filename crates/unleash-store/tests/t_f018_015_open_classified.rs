//! REQ-REC-005 : le diagnostic n'est possible qu'après un refus dans le vrai préflight RO.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales et injections de panne contrôlées"
)]
use std::sync::Arc;
use unleash_store::{
    CutpointObserver, StoreConfig, StoreCutpoint, StoreError, StoreOpenStage, UnleashStore,
};
use unleash_testkit::local_test_dir;

struct CorruptAfterReadOnly;
impl CutpointObserver for CorruptAfterReadOnly {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C11MigrationInProgress {
            return Err(StoreError::StorageCorrupt(
                "late fixture corruption".to_owned(),
            ));
        }
        Ok(())
    }
}

#[test]
fn readonly_corruption_never_initializes_directories_or_changes_source() {
    let directory = local_test_dir("f018-015-readonly").unwrap();
    let config = StoreConfig::new(directory.path());
    let bytes = b"synthetic non sqlite fixture";
    std::fs::write(&config.db_path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let failure = UnleashStore::open_classified(config.clone()).unwrap_err();
    assert_eq!(failure.stage, StoreOpenStage::ReadOnlyPreflight);
    assert!(matches!(failure.error, StoreError::StorageCorrupt(_)));
    assert_eq!(std::fs::read(&config.db_path).unwrap(), bytes);
    assert!(!config.artifacts_dir.exists());
    assert!(!config.maintenance_dir.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
    }
}

#[test]
fn late_storage_corruption_is_not_a_readonly_diagnostic_admission() {
    let directory = local_test_dir("f018-015-late").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.cutpoints = Arc::new(CorruptAfterReadOnly);
    let failure = UnleashStore::open_classified(config.clone()).unwrap_err();
    assert_eq!(failure.stage, StoreOpenStage::AfterPreflight);
    assert!(matches!(failure.error, StoreError::StorageCorrupt(_)));
    assert!(config.db_path.exists());
}

#[test]
fn healthy_existing_store_opens_with_existing_schema_and_authority() {
    let directory = local_test_dir("f018-015-healthy").unwrap();
    let config = StoreConfig::new(directory.path());
    let store = UnleashStore::open(config.clone()).unwrap();
    let authority = store.active_authority_generation().unwrap();
    store.shutdown().unwrap();
    let store = UnleashStore::open_classified(config).unwrap();
    assert_eq!(store.active_authority_generation().unwrap(), authority);
}
