//! REQ-REC-005 : lecture privée, sans symlink et bornée du diagnostic hors base.
#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures locales synthétiques sans secrets"
)]
use std::os::unix::fs::PermissionsExt;
use unleash_domain::{
    AuthorityGeneration, BackupId, CommandId, ReceiptId, RestoreId, Sha256Digest, TimestampMs,
};
use unleash_store::{MaintenanceRestoreManifest, RestoreRequestState, UnleashStore};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

fn manifest() -> MaintenanceRestoreManifest {
    MaintenanceRestoreManifest {
        restore_id: RestoreId::from_uuid(Uuid::from_u128(1)),
        command_receipt_id: ReceiptId::from_uuid(Uuid::from_u128(2)),
        actor_id: "user:fixture".to_owned(),
        command_id: CommandId::from_uuid(Uuid::from_u128(3)),
        command_digest: Sha256Digest::from_bytes(b"fixture"),
        backup_id: BackupId::from_uuid(Uuid::from_u128(4)),
        source_authority_generation: AuthorityGeneration::from_uuid(Uuid::from_u128(5)),
        state: RestoreRequestState::Blocked,
        blocked_reason: Some("fixture".to_owned()),
        updated_at_ms: TimestampMs::new(0),
        history: vec![],
    }
}

fn write_fixture(parent: &std::path::Path, bytes: &[u8]) -> std::path::PathBuf {
    std::fs::create_dir_all(parent).unwrap();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = parent.join("restore-manifest.json");
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[test]
fn missing_manifest_is_none_and_private_regular_manifest_is_typed() {
    let directory = local_test_dir("f018-014-valid").unwrap();
    let parent = directory.path().join("maintenance");
    assert!(
        UnleashStore::read_maintenance_restore_manifest(&parent)
            .unwrap()
            .is_none()
    );
    let expected = manifest();
    write_fixture(&parent, &serde_json::to_vec(&expected).unwrap());
    assert_eq!(
        UnleashStore::read_maintenance_restore_manifest(&parent).unwrap(),
        Some(expected)
    );
}

#[test]
fn public_mode_and_final_symlink_are_refused_without_modifying_source() {
    for symlink in [false, true] {
        let directory = local_test_dir("f018-014-refused").unwrap();
        let parent = directory.path().join("maintenance");
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        let path = write_fixture(&parent, &bytes);
        if symlink {
            let target = parent.join("real-fixture.json");
            std::fs::rename(&path, &target).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
        } else {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(UnleashStore::read_maintenance_restore_manifest(&parent).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        if !symlink {
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
                0o644
            );
        }
    }
}

#[test]
fn symlinked_or_public_maintenance_parent_is_refused() {
    for symlink in [false, true] {
        let directory = local_test_dir("f018-014-parent").unwrap();
        let parent = directory.path().join("maintenance");
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        write_fixture(&parent, &bytes);
        if symlink {
            let target = directory.path().join("private-real-parent");
            std::fs::rename(&parent, &target).unwrap();
            std::os::unix::fs::symlink(&target, &parent).unwrap();
        } else {
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(UnleashStore::read_maintenance_restore_manifest(&parent).is_err());
        assert_eq!(
            std::fs::read(parent.join("restore-manifest.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn oversized_manifest_is_rejected_explicitly_and_unknown_fields_are_not_echoed() {
    let directory = local_test_dir("f018-014-limit").unwrap();
    let parent = directory.path().join("maintenance");
    let path = write_fixture(&parent, b"{}");
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(u64::try_from(unleash_protocol::MAX_FRAME_BYTES / 4 + 1).unwrap())
        .unwrap();
    let error = UnleashStore::read_maintenance_restore_manifest(&parent).unwrap_err();
    assert!(error.to_string().contains("limite"));
    assert_eq!(
        file.metadata().unwrap().len(),
        u64::try_from(unleash_protocol::MAX_FRAME_BYTES / 4 + 1).unwrap()
    );
    drop(file);
    let mut value = serde_json::to_value(manifest()).unwrap();
    value.as_object_mut().unwrap().insert(
        "private_canary_extra".to_owned(),
        serde_json::json!("private_canary_value"),
    );
    write_fixture(&parent, &serde_json::to_vec(&value).unwrap());
    let error = UnleashStore::read_maintenance_restore_manifest(&parent).unwrap_err();
    assert!(!error.to_string().contains("private_canary"));
}

#[test]
fn exact_read_limit_is_accepted_and_non_regular_input_is_refused() {
    let directory = local_test_dir("f018-014-exact-limit").unwrap();
    let parent = directory.path().join("maintenance");
    let mut value = manifest();
    value.blocked_reason = Some(String::new());
    let overhead = serde_json::to_vec(&value).unwrap().len();
    let limit = unleash_store::restore::MAX_MAINTENANCE_MANIFEST_BYTES;
    value.blocked_reason = Some("x".repeat(limit - overhead));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(bytes.len(), limit);
    let path = write_fixture(&parent, &bytes);
    assert_eq!(
        UnleashStore::read_maintenance_restore_manifest(&parent).unwrap(),
        Some(value)
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let error = UnleashStore::read_maintenance_restore_manifest(&parent).unwrap_err();
    assert!(error.to_string().contains("non régulier"));
}
