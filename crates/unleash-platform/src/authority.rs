// SPDX-License-Identifier: Apache-2.0
//! Verrou exclusif d'autorité OS par UID effectif (`flock`), indépendant de `data_root` et
//! des surcharges `XDG_RUNTIME_DIR` (`REQ-CORE-001`, `AC-F001-001`).

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags, fchmod, flock, fstat, open};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};

use crate::errors::PlatformError;
use crate::paths::{
    assert_not_under_claude, canonical_rendezvous_dir, canonicalize_with_existing_prefix,
    effective_uid, normalize_lexical,
};
use crate::process_identity::{BootIdentity, StartIdentity, current_process_identity};
use crate::rendezvous::{ensure_private_rendezvous_dir, validate_unix_socket_path_len};

/// Nom du fichier de verrou d'autorité dans le répertoire de rendez-vous.
pub const AUTHORITY_LOCK_FILENAME: &str = "authority.lock";

/// Nom du socket Unix de rendez-vous dans le répertoire de rendez-vous.
pub const SOCKET_FILENAME: &str = "daemon.sock";

/// Métadonnées publiques non secrètes inscrites dans le fichier verrou sous protection `flock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityLockMetadata {
    /// UID effectif propriétaire de l'autorité.
    pub owner_uid: u32,
    /// Racine de données canonique servie par cette autorité.
    pub data_root: PathBuf,
    /// Chemin du socket Unix associé.
    pub socket_path: PathBuf,
    /// PID de l'instance détentrice.
    pub pid: u32,
    /// Identité de boot OS au moment de l'acquisition.
    pub boot_identity: BootIdentity,
    /// Identité de démarrage OS du processus détenteur.
    pub start_identity: StartIdentity,
}

/// Garde RAII détenant le verrou exclusif OS `flock` d'autorité pour un UID effectif.
///
/// Tant que cette garde est vivante, toute autre tentative d'acquisition pour le même UID
/// échoue avec [`PlatformError::DaemonAlreadyRunning`] (même `data_root`) ou
/// [`PlatformError::AuthorityAlreadyActive`] (autre `data_root`).
#[must_use = "Libérer immédiatement AuthorityLock relâcherait l'exclusivité d'autorité daemon"]
#[derive(Debug)]
pub struct AuthorityLock {
    file: File,
    lock_path: PathBuf,
    rendezvous_dir: PathBuf,
    metadata: AuthorityLockMetadata,
}

impl AuthorityLock {
    /// Acquiert le verrou exclusif d'autorité pour `data_root` dans le répertoire de rendez-vous
    /// canonique de l'UID effectif courant.
    ///
    /// # Errors
    /// - [`PlatformError::AuthorityAlreadyActive`] si une instance tourne déjà sur une autre racine.
    /// - [`PlatformError::DaemonAlreadyRunning`] si une instance tourne déjà sur la même racine.
    /// - [`PlatformError::ForbiddenDocumentationPath`] si un chemin touche `.claude/`.
    pub fn acquire(data_root: &Path) -> Result<Self, PlatformError> {
        let rdv = canonical_rendezvous_dir()?;
        Self::acquire_in(&rdv, data_root)
    }

    /// Acquiert le verrou exclusif d'autorité dans un répertoire de rendez-vous explicite
    /// (utilisé notamment pour les tests isolés sous `/tmp`).
    ///
    /// # Errors
    /// Identique à [`Self::acquire`], plus erreurs de validation de permissions `0700`/`0600`
    /// et `SUN_LEN`.
    pub fn acquire_in(rendezvous_dir: &Path, data_root: &Path) -> Result<Self, PlatformError> {
        assert_not_under_claude(data_root)?;
        ensure_private_rendezvous_dir(rendezvous_dir)?;

        let canonical_root = canonicalize_data_root(data_root)?;
        assert_not_under_claude(&canonical_root)?;

        let socket_path = rendezvous_dir.join(SOCKET_FILENAME);
        validate_unix_socket_path_len(&socket_path)?;

        let lock_path = rendezvous_dir.join(AUTHORITY_LOCK_FILENAME);
        assert_not_under_claude(&lock_path)?;

        // IMPORTANT : ne jamais passer O_TRUNC à l'ouverture avant d'avoir acquis flock,
        // sans quoi un second démarrage concurrent écraserait les métadonnées du détenteur actif.
        let owned_fd = open(
            &lock_path,
            OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| PlatformError::OsCall {
            op: "open(authority.lock, O_CREAT|O_RDWR|O_NOFOLLOW)",
            source: std::io::Error::from(e),
        })?;

        let st = fstat(&owned_fd).map_err(|e| PlatformError::OsCall {
            op: "fstat(authority.lock)",
            source: std::io::Error::from(e),
        })?;
        if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
            return Err(PlatformError::NotARegularFile {
                path: lock_path.display().to_string(),
            });
        }
        let uid = effective_uid();
        if st.st_uid != uid {
            return Err(PlatformError::OwnershipMismatch {
                path: lock_path.display().to_string(),
                expected_uid: uid,
                actual_uid: st.st_uid,
            });
        }
        fchmod(&owned_fd, Mode::from_raw_mode(0o600)).map_err(|e| PlatformError::OsCall {
            op: "fchmod(authority.lock, 0600)",
            source: std::io::Error::from(e),
        })?;

        match flock(&owned_fd, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                let proc_id = current_process_identity()?;
                let metadata = AuthorityLockMetadata {
                    owner_uid: uid,
                    data_root: canonical_root,
                    socket_path,
                    pid: proc_id.pid,
                    boot_identity: proc_id.boot_identity,
                    start_identity: proc_id.start_identity,
                };
                let mut file = File::from(owned_fd);
                let payload = serde_json::to_vec(&metadata)?;
                file.set_len(0)?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&payload)?;
                file.sync_all()?;

                Ok(Self {
                    file,
                    lock_path,
                    rendezvous_dir: rendezvous_dir.to_path_buf(),
                    metadata,
                })
            }
            Err(errno) if errno == Errno::WOULDBLOCK || errno == Errno::AGAIN => {
                let mut file = File::from(owned_fd);
                let active_meta = read_active_metadata_bounded(&mut file)?;
                if let Some(meta) = active_meta {
                    let active_canon =
                        canonicalize_data_root(&meta.data_root).unwrap_or(meta.data_root);
                    if active_canon == canonical_root {
                        return Err(PlatformError::DaemonAlreadyRunning {
                            active_data_root: active_canon,
                            socket_path: meta.socket_path,
                        });
                    }
                    return Err(PlatformError::AuthorityAlreadyActive {
                        active_data_root: active_canon,
                        requested_data_root: canonical_root,
                        socket_path: meta.socket_path,
                    });
                }

                // Fallback conservateur si l'autre détenteur n'a pas encore publié ses octets :
                // le verrou appartient bien à une autorité active inconnue ; on échoue explicitement.
                Err(PlatformError::AuthorityAlreadyActive {
                    active_data_root: PathBuf::from("<active-authority-pending-metadata>"),
                    requested_data_root: canonical_root,
                    socket_path,
                })
            }
            Err(errno) => Err(PlatformError::OsCall {
                op: "flock(LOCK_EX|LOCK_NB)",
                source: std::io::Error::from(errno),
            }),
        }
    }

    /// Métadonnées de l'autorité actuellement détenue.
    #[must_use]
    pub const fn metadata(&self) -> &AuthorityLockMetadata {
        &self.metadata
    }

    /// Chemin du répertoire de rendez-vous verrouillé.
    #[must_use]
    pub fn rendezvous_dir(&self) -> &Path {
        &self.rendezvous_dir
    }

    /// Chemin du fichier verrou.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Chemin du socket de rendez-vous associé à ce verrou.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.metadata.socket_path
    }

    /// Supprime un ancien socket orphelin uniquement après acquisition du verrou exclusif,
    /// en vérifiant qu'il ne s'agit ni d'un lien symbolique ni d'un répertoire.
    ///
    /// # Errors
    /// Retourne [`PlatformError`] si la cible est un symlink/répertoire ou si la suppression échoue.
    pub fn remove_stale_socket(&self) -> Result<(), PlatformError> {
        let sock = &self.metadata.socket_path;
        match fs::symlink_metadata(sock) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(PlatformError::SymlinkNotAllowed {
                        path: sock.display().to_string(),
                    });
                }
                if meta.is_dir() {
                    return Err(PlatformError::InvalidPath {
                        path: sock.display().to_string(),
                        reason: String::from("Impossible de retirer un répertoire comme socket"),
                    });
                }
                fs::remove_file(sock)?;
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PlatformError::Io(e)),
        }
    }
}

impl Drop for AuthorityLock {
    fn drop(&mut self) {
        // Ne JAMAIS unlink `authority.lock` (race flock + unlink sur nouvel inode).
        // Effacer les métadonnées pendant que flock est détenu, puis fermer le descripteur.
        let _ = self.file.set_len(0);
        let _ = self.file.sync_data();
    }
}

fn read_active_metadata_bounded(
    file: &mut File,
) -> Result<Option<AuthorityLockMetadata>, PlatformError> {
    let mut raw = Vec::new();
    for attempt in 0..32_u8 {
        raw.clear();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut raw)?;
        if !raw.is_empty()
            && let Ok(parsed) = serde_json::from_slice::<AuthorityLockMetadata>(&raw)
        {
            return Ok(Some(parsed));
        }
        if attempt < 31 {
            std::thread::yield_now();
        }
    }
    Ok(None)
}

fn canonicalize_data_root(data_root: &Path) -> Result<PathBuf, PlatformError> {
    if !data_root.is_absolute() {
        return Err(PlatformError::InvalidPath {
            path: data_root.display().to_string(),
            reason: String::from("data_root doit être un chemin absolu"),
        });
    }
    let normalized = normalize_lexical(data_root);
    Ok(canonicalize_with_existing_prefix(&normalized).unwrap_or(normalized))
}
