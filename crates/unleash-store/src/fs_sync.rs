//! Primitives sûres de persistance fichier, synchronisation durable et permissions POSIX.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::StoreError;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

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
