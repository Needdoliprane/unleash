//! REQ-CORE-004 : lecture sûre du credential local.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::os::unix::fs::{PermissionsExt, symlink};
use unleash_platform::{ADMIN_PROOF_FILENAME, read_admin_credential};
use unleash_testkit::local_test_dir;

#[test]
fn credential_must_be_private_regular_file_in_private_rendezvous() {
    let dir = local_test_dir("admin-credential").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join(ADMIN_PROOF_FILENAME);
    assert!(read_admin_credential(dir.path()).unwrap().is_none());
    std::fs::write(&path, "ab".repeat(32)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        &**read_admin_credential(dir.path()).unwrap().unwrap(),
        "ab".repeat(32)
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_admin_credential(dir.path()).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, "short").unwrap();
    assert!(read_admin_credential(dir.path()).is_err());
    std::fs::remove_file(&path).unwrap();
    symlink("/dev/null", &path).unwrap();
    assert!(read_admin_credential(dir.path()).is_err());
}
