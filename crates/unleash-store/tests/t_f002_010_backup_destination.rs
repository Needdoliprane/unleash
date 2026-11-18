// T-F002-010 / AC-F002-010 / REQ-PERSIST-010 / REQ-REC-007
//! Exclusive backup destinations, before and after the durable C11a barrier.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use unleash_store::{
    BackupJobState, BackupRetentionTier, CutpointObserver, PrepareBackupRequest, StoreConfig,
    StoreCutpoint, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

fn config(root: &Path) -> StoreConfig {
    let mut config = StoreConfig::new(root);
    config.clock = Arc::new(ManualClock::default());
    config.id_generator = Arc::new(DeterministicIdGenerator::new(0xBACC_0010));
    config
}

fn request(destination: &Path) -> PrepareBackupRequest {
    PrepareBackupRequest {
        destination_dir: destination.to_owned(),
        retention_tier: BackupRetentionTier::Manual,
        coverage: "db_and_objects".into(),
    }
}

fn job_and_barrier_counts(store: &UnleashStore) -> Result<(i64, i64), StoreError> {
    store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM backup_jobs", [], |row| row.get(0))?,
                conn.query_row(
                    "SELECT COUNT(*) FROM object_pins WHERE pin_kind = 'backup_barrier' AND state IN ('held', 'reconciling')",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
}

#[test]
fn t_f002_010_existing_directory_is_untouched_without_acquiring_a_barrier() {
    let dir = local_test_dir("backup-existing-directory").unwrap();
    let store = UnleashStore::open(config(dir.path())).unwrap();
    let destination = dir.path().join("backups/existing");
    fs::create_dir_all(&destination).unwrap();
    let sentinel = destination.join("db.sqlite");
    fs::write(&sentinel, b"existing database sentinel").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o750)).unwrap();
    }

    assert!(matches!(
        store.prepare_backup(request(&destination)),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(fs::read(&sentinel).unwrap(), b"existing database sentinel");
    assert!(!destination.join("manifest.json").exists());
    assert_eq!(job_and_barrier_counts(&store).unwrap(), (0, 0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
}

#[cfg(unix)]
#[test]
fn t_f002_010_existing_or_dangling_symlink_is_untouched_without_a_barrier() {
    use std::os::unix::fs::symlink;

    for dangling in [false, true] {
        let dir = local_test_dir("backup-existing-symlink").unwrap();
        let store = UnleashStore::open(config(dir.path())).unwrap();
        let target = dir.path().join("target");
        if !dangling {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("db.sqlite"), b"symlink target sentinel").unwrap();
        }
        let destination = dir.path().join("backups/link");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        symlink(&target, &destination).unwrap();

        assert!(matches!(
            store.prepare_backup(request(&destination)),
            Err(StoreError::InvalidArgument { .. })
        ));
        assert_eq!(fs::read_link(&destination).unwrap(), target);
        assert_eq!(job_and_barrier_counts(&store).unwrap(), (0, 0));
        if dangling {
            assert!(!target.exists());
        } else {
            assert_eq!(
                fs::read(target.join("db.sqlite")).unwrap(),
                b"symlink target sentinel"
            );
            assert!(!target.join("manifest.json").exists());
        }
    }
}

struct DestinationCollision(PathBuf);

impl CutpointObserver for DestinationCollision {
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        if matches!(cutpoint, StoreCutpoint::C11aAfterBackupBarrier) {
            fs::create_dir(&self.0)?;
            fs::write(self.0.join("db.sqlite"), b"concurrent destination sentinel")?;
        }
        Ok(())
    }
}

#[test]
fn t_f002_010_destination_collision_after_c11a_preserves_target_and_durable_job() {
    let dir = local_test_dir("backup-target-race").unwrap();
    let destination = dir.path().join("backups/racing");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let mut config = config(dir.path());
    config.cutpoints = Arc::new(DestinationCollision(destination.clone()));
    let mut store = UnleashStore::open(config).unwrap();

    assert!(matches!(
        store.prepare_backup(request(&destination)),
        Err(StoreError::Io(ref error)) if error.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(
        fs::read(destination.join("db.sqlite")).unwrap(),
        b"concurrent destination sentinel"
    );
    assert!(!destination.join("manifest.json").exists());
    assert_eq!(job_and_barrier_counts(&store).unwrap(), (1, 1));
    store = store.restart_same_lineage().unwrap();
    assert_eq!(job_and_barrier_counts(&store).unwrap(), (1, 1));
    let state: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row("SELECT state FROM backup_jobs", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(state, "preparing");
}

#[test]
fn t_f002_010_new_leaf_is_private_complete_and_validated() {
    let dir = local_test_dir("backup-exclusive-success").unwrap();
    let config = config(dir.path());
    let store = UnleashStore::open(config.clone()).unwrap();
    let destination = dir.path().join("backups/new-parent/new-leaf");
    let manifest = store.prepare_backup(request(&destination)).unwrap();

    assert_eq!(manifest.state, BackupJobState::Complete);
    let report = UnleashStore::validate_backup(&destination, &config).unwrap();
    assert!(report.integrity_check_ok);
    assert_eq!(report.foreign_key_violations, 0);
    assert_eq!(report.backup_id, manifest.backup_id);
    assert_eq!(job_and_barrier_counts(&store).unwrap(), (1, 0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
