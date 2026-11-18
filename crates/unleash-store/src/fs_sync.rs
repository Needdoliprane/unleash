//! Primitives sûres de persistance fichier, synchronisation durable et permissions POSIX.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::StoreError;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Lit un fichier borné depuis un dossier privé ancré, sans suivre ses liens finaux.
/// L'identité, le type et les modes sont vérifiés sur les descripteurs réellement ouverts.
#[cfg(unix)]
pub(crate) fn read_private_file_bounded(
    path: &Path,
    expected_uid: u32,
    limit: usize,
) -> Result<Option<Vec<u8>>, StoreError> {
    use rustix::fs::{Mode, OFlags, openat};
    use rustix::io::Errno;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    let refused = |message: &'static str| StoreError::EvidenceUnavailable(message.to_owned());
    let name = path
        .file_name()
        .ok_or_else(|| refused("nom du manifeste maintenance invalide"))?;
    let parent = path
        .parent()
        .ok_or_else(|| refused("parent du manifeste maintenance absent"))?;
    if path.components().any(|component| {
        matches!(component, Component::ParentDir | Component::Prefix(_))
            || matches!(component,Component::Normal(name) if name==".claude")
    }) {
        return Err(refused("chemin du manifeste maintenance non qualifié"));
    }
    let Some(directory) = open_manifest_parent(parent)? else {
        return Ok(None);
    };
    let fd = match openat(
        &directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(Errno::LOOP) => return Err(refused("lien symbolique de manifeste maintenance refusé")),
        Err(error) => return Err(StoreError::Io(error.into())),
    };
    let directory = File::from(directory);
    let parent_metadata = directory.metadata()?;
    if !parent_metadata.is_dir()
        || parent_metadata.uid() != expected_uid
        || parent_metadata.permissions().mode() & 0o7777 != 0o700
    {
        return Err(refused(
            "parent du manifeste maintenance non privé ou mauvais propriétaire",
        ));
    }
    let file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(refused("manifeste maintenance non régulier"));
    }
    if metadata.uid() != expected_uid {
        return Err(refused("propriétaire du manifeste maintenance invalide"));
    }
    if metadata.permissions().mode() & 0o7777 != 0o600 {
        return Err(refused("mode privé 0600 du manifeste maintenance requis"));
    }
    let limit =
        u64::try_from(limit).map_err(|_| refused("limite du manifeste maintenance invalide"))?;
    if metadata.len() > limit {
        return Err(refused(
            "manifeste maintenance dépasse la limite de lecture",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())
        .map_err(|_| refused("taille du manifeste maintenance invalide"))?
        > limit
    {
        return Err(refused(
            "manifeste maintenance dépasse la limite de lecture",
        ));
    }
    Ok(Some(bytes))
}

#[cfg(unix)]
fn open_manifest_parent(parent: &Path) -> Result<Option<std::os::fd::OwnedFd>, StoreError> {
    use rustix::fs::{Mode, OFlags, open, openat};
    use rustix::io::Errno;
    use std::path::Component;
    let refused = |message: &'static str| StoreError::EvidenceUnavailable(message.to_owned());
    let directory_flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let anchor = if parent.is_absolute() {
        Path::new("/")
    } else {
        Path::new(".")
    };
    let mut directory =
        open(anchor, directory_flags, Mode::empty()).map_err(|e| StoreError::Io(e.into()))?;
    let normal_count = parent
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    let mut visited = 0;
    for component in parent.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                visited += 1;
                directory = match openat(&directory, name, directory_flags, Mode::empty()) {
                    Ok(fd) => fd,
                    Err(Errno::NOENT) => return Ok(None),
                    Err(Errno::LOOP | Errno::NOTDIR) => open_protected_system_alias(
                        &directory,
                        name,
                        directory_flags,
                        visited < normal_count,
                    )?,
                    Err(error) => return Err(StoreError::Io(error.into())),
                };
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(refused("parent du manifeste maintenance non qualifié"));
            }
        }
    }
    Ok(Some(directory))
}

/// Garde les alias système contrôlés par root, avec une vérification du parent ancré.
#[cfg(unix)]
fn open_protected_system_alias(
    parent: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
    flags: rustix::fs::OFlags,
    ancestor: bool,
) -> Result<std::os::fd::OwnedFd, StoreError> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};
    let link = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| StoreError::Io(error.into()))?;
    let parent_stat = fstat(parent).map_err(|error| StoreError::Io(error.into()))?;
    if !ancestor
        || FileType::from_raw_mode(link.st_mode) != FileType::Symlink
        || link.st_uid != 0
        || parent_stat.st_uid != 0
        || parent_stat.st_mode & 0o022 != 0
    {
        return Err(StoreError::EvidenceUnavailable(
            "parent du manifeste maintenance non réel".to_owned(),
        ));
    }
    let directory = openat(parent, name, flags & !OFlags::NOFOLLOW, Mode::empty())
        .map_err(|error| StoreError::Io(error.into()))?;
    let target = fstat(&directory).map_err(|error| StoreError::Io(error.into()))?;
    if target.st_uid != 0 {
        return Err(StoreError::EvidenceUnavailable(
            "alias système du manifeste maintenance non qualifié".to_owned(),
        ));
    }
    Ok(directory)
}

#[cfg(not(unix))]
pub(crate) fn read_private_file_bounded(
    path: &Path,
    _expected_uid: u32,
    _limit: usize,
) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => Err(StoreError::EvidenceUnavailable(
            "lecture privée de maintenance non qualifiée sur cette plateforme".to_owned(),
        )),
    }
}

/// Écrit atomiquement un fichier via un fichier temporaire privé, `sync_all`, renommage et synchronisation du dossier parent.
///
/// # Errors
/// Retourne [`StoreError::Io`] si la création, l'écriture, la synchronisation ou le renommage échoue.
pub fn write_atomic_durable(dest: &Path, bytes: &[u8], mode: u32) -> Result<(), StoreError> {
    let parent = dest.parent().ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "chemin destination sans dossier parent",
        ))
    })?;
    fs::create_dir_all(parent)?;
    set_dir_permissions_0700(parent)?;

    let file_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("blob");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(".{file_name}.{seq}.tmp"));

    {
        let mut tmp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        set_file_permissions(&tmp_file, mode)?;
        tmp_file.write_all(bytes)?;
        tmp_file.sync_all()?;
    }

    fs::rename(&tmp_path, dest)?;
    sync_dir(parent)?;
    Ok(())
}

/// Synchronise les métadonnées d'un répertoire sur disque (`fsync` sur le descripteur de dossier).
///
/// # Errors
/// Retourne [`StoreError::Io`] en cas d'échec de synchronisation non bénin.
pub fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    let dir_file = File::open(dir)?;
    match dir_file.sync_all() {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(StoreError::Io(err)),
    }
}

/// Applique les permissions `0700` sur un répertoire sous Unix.
///
/// # Errors
/// Retourne [`StoreError::Io`] si le changement de permissions échoue.
pub fn set_dir_permissions_0700(dir: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(dir, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

/// Applique les permissions POSIX sur un fichier ouvert.
///
/// # Errors
/// Retourne [`StoreError::Io`] si le changement de permissions échoue.
pub fn set_file_permissions(file: &File, mode: u32) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(mode);
        file.set_permissions(perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (file, mode);
    }
    Ok(())
}

/// Synchronise le répertoire parent d'un fichier sur disque (`fsync` sur le descripteur de dossier).
///
/// # Errors
/// Retourne [`StoreError::Io`] en cas d'échec de synchronisation non bénin.
pub fn sync_parent_dir(path: &Path) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

/// Applique les permissions POSIX sur un chemin de fichier.
///
/// # Errors
/// Retourne [`StoreError::Io`] si le changement de permissions échoue.
pub fn set_path_permissions(path: &Path, mode: u32) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod private_reader_tests {
    use super::*;

    #[test]
    fn mismatched_expected_owner_is_refused_without_changing_file() {
        let directory = unleash_testkit::local_test_dir("private-reader-owner").unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("fixture.json");
        fs::write(&path, b"private-fixture").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let current = rustix::process::geteuid().as_raw();
        let wrong = current.wrapping_add(1);
        let error = read_private_file_bounded(&path, wrong, 1024).unwrap_err();
        assert!(error.to_string().contains("propriétaire"));
        assert_eq!(fs::read(&path).unwrap(), b"private-fixture");
        assert_eq!(
            read_private_file_bounded(&path, current, 1024)
                .unwrap()
                .unwrap(),
            b"private-fixture"
        );
    }
}
