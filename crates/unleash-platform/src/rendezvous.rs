// SPDX-License-Identifier: Apache-2.0
//! Validation sécurisée du répertoire de rendez-vous (`0700`, UID effectif, sans symlink),
//! du socket Unix (`0600`) et de la limite `SUN_LEN` sur macOS (104 octets).

use std::fs::{self, DirBuilder, File};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{FileType, Mode, OFlags, RawMode, fstat, open, openat};
use rustix::io::Errno;

use crate::errors::PlatformError;
use crate::paths::{assert_not_under_claude, canonicalize_with_existing_prefix, effective_uid};

static ATOMIC_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Taille de `sockaddr_un.sun_path` sur macOS (incluant le caractère NUL terminal).
pub const MACOS_SUN_LEN: usize = 104;

/// Longueur maximale admissible en octets pour un chemin de socket Unix (`SUN_LEN - 1`).
pub const MAX_UNIX_SOCKET_PATH_BYTES: usize = MACOS_SUN_LEN - 1;

/// Nom fixe de la preuve administrative dans le rendez-vous privé.
pub const ADMIN_PROOF_FILENAME: &str = "admin-proof";

/// Lit le credential sur un fichier régulier appartenant à cet UID et de mode strict `0600`.
/// Le fichier est ouvert relativement au descripteur vérifié du rendez-vous et la lecture
/// est bornée à 65 octets. Une absence retourne `None` ; aucune valeur invalide ne figure
/// dans les erreurs.
///
/// # Errors
/// Retourne une erreur de validation ou de lecture si le répertoire ou le fichier est invalide.
pub fn read_admin_credential(
    dir: &Path,
) -> Result<Option<zeroize::Zeroizing<String>>, PlatformError> {
    validate_rendezvous_dir(dir)?;
    let dir_fd = open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| PlatformError::OsCall {
        op: "open(admin rendezvous)",
        source: std::io::Error::from(e),
    })?;
    let dir_stat = fstat(&dir_fd).map_err(|e| PlatformError::OsCall {
        op: "fstat(admin rendezvous)",
        source: std::io::Error::from(e),
    })?;
    if FileType::from_raw_mode(dir_stat.st_mode) != FileType::Directory
        || dir_stat.st_uid != effective_uid()
        || raw_mode_to_u32(dir_stat.st_mode) & 0o7777 != 0o700
    {
        return Err(PlatformError::InvalidPath {
            path: dir.display().to_string(),
            reason: String::from("Répertoire de credential invalide"),
        });
    }
    let fd = match openat(
        &dir_fd,
        ADMIN_PROOF_FILENAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(e) => {
            return Err(PlatformError::OsCall {
                op: "openat(admin credential)",
                source: std::io::Error::from(e),
            });
        }
    };
    let st = fstat(&fd).map_err(|e| PlatformError::OsCall {
        op: "fstat(admin credential)",
        source: std::io::Error::from(e),
    })?;
    let path = dir.join(ADMIN_PROOF_FILENAME).display().to_string();
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err(PlatformError::NotARegularFile { path });
    }
    if st.st_uid != effective_uid() {
        return Err(PlatformError::OwnershipMismatch {
            path,
            expected_uid: effective_uid(),
            actual_uid: st.st_uid,
        });
    }
    let mode = raw_mode_to_u32(st.st_mode) & 0o7777;
    if mode != 0o600 {
        return Err(PlatformError::InsecurePermissions {
            path,
            expected_mode: 0o600,
            actual_mode: mode,
        });
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    File::from(fd).take(65).read_to_end(&mut bytes)?;
    if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii) {
        return Err(PlatformError::InvalidPath {
            path,
            reason: String::from("Credential de longueur ou encodage invalide"),
        });
    }
    let value = std::str::from_utf8(&bytes).map_err(|_| PlatformError::InvalidPath {
        path,
        reason: String::from("Credential invalide"),
    })?;
    Ok(Some(zeroize::Zeroizing::new(value.to_owned())))
}

#[cfg(target_os = "macos")]
const fn raw_mode_to_u32(raw: RawMode) -> u32 {
    raw as u32
}

#[cfg(not(target_os = "macos"))]
const fn raw_mode_to_u32(raw: RawMode) -> u32 {
    raw
}

/// Vérifie qu'un chemin de socket Unix respecte la limite `SUN_LEN` (`<= 103` octets brut et
/// canonique) et ne cible pas `.claude/`.
///
/// # Errors
/// - [`PlatformError::ForbiddenDocumentationPath`] si le chemin touche `.claude/`.
/// - [`PlatformError::InvalidPath`] si le nom de socket est absent ou contient NUL.
/// - [`PlatformError::SocketPathTooLong`] si le chemin brut ou canonisé dépasse `103` octets.
pub fn validate_unix_socket_path_len(socket_path: &Path) -> Result<(), PlatformError> {
    assert_not_under_claude(socket_path)?;

    let raw_bytes = socket_path.as_os_str().as_encoded_bytes();
    if raw_bytes.is_empty() || raw_bytes.contains(&0) {
        return Err(PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Chemin de socket vide ou contenant un octet nul"),
        });
    }

    let Some(file_name) = socket_path.file_name() else {
        return Err(PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Nom de fichier de socket manquant"),
        });
    };
    if file_name == "." || file_name == ".." {
        return Err(PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Nom de fichier de socket invalide"),
        });
    }

    if socket_path.as_os_str().len() > MAX_UNIX_SOCKET_PATH_BYTES {
        return Err(PlatformError::SocketPathTooLong {
            path: socket_path.display().to_string(),
            len: socket_path.as_os_str().len(),
            max: MAX_UNIX_SOCKET_PATH_BYTES,
        });
    }

    if let Some(canon) = canonicalize_with_existing_prefix(socket_path)
        && canon.as_os_str().len() > MAX_UNIX_SOCKET_PATH_BYTES
    {
        return Err(PlatformError::SocketPathTooLong {
            path: canon.display().to_string(),
            len: canon.as_os_str().len(),
            max: MAX_UNIX_SOCKET_PATH_BYTES,
        });
    }

    Ok(())
}

/// Vérifie qu'aucun ancêtre existant de `target` (hors liens système root comme `/tmp -> private/tmp`)
/// n'est un lien symbolique contrôlé par un utilisateur non-root.
pub(crate) fn check_ancestor_symlinks(target: &Path) -> Result<(), PlatformError> {
    for anc in target.ancestors().skip(1) {
        if anc.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(anc) {
            Ok(meta) => {
                if meta.file_type().is_symlink() && meta.uid() != 0 {
                    return Err(PlatformError::SymlinkNotAllowed {
                        path: anc.display().to_string(),
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(PlatformError::Io(e)),
        }
    }
    Ok(())
}

/// Crée si nécessaire le répertoire de rendez-vous en mode `0700` puis vérifie strictement
/// propriétaire, permissions `0700` et absence de lien symbolique.
///
/// # Errors
/// Voir [`validate_rendezvous_dir`].
pub fn ensure_private_rendezvous_dir(dir: &Path) -> Result<(), PlatformError> {
    assert_not_under_claude(dir)?;
    check_ancestor_symlinks(dir)?;

    match fs::symlink_metadata(dir) {
        Ok(_) => validate_rendezvous_dir(dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            match DirBuilder::new().mode(0o700).create(dir) {
                Ok(()) => {}
                Err(create_err) if create_err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(create_err) => return Err(PlatformError::Io(create_err)),
            }
            validate_rendezvous_dir(dir)
        }
        Err(e) => Err(PlatformError::Io(e)),
    }
}

/// Nom de l'unique sous-répertoire applicatif hébergé par une racine runtime qualifiée.
pub(crate) const RUNTIME_ROOT_RENDEZVOUS_DIR: &str = "unleash";

/// Remédiation indiquée quand la racine runtime manque ou n'est pas qualifiée.
const RUNTIME_ROOT_REMEDIATION: &str = "ouvrez une session qui fournit cette racine (logind ou elogind ; \
     activez linger pour qu'elle survive à la fin de session) ou faites-la créer au démarrage par \
     l'administrateur : répertoire réel, propriété de l'utilisateur, mode 0700, sans lien symbolique";

fn runtime_root_refusal(root: &Path, reason: impl Into<String>) -> PlatformError {
    PlatformError::RuntimeRootUnavailable {
        root: root.display().to_string(),
        rendezvous_dir: root.join(RUNTIME_ROOT_RENDEZVOUS_DIR).display().to_string(),
        reason: reason.into(),
        remediation: String::from(RUNTIME_ROOT_REMEDIATION),
    }
}

/// Qualifie une racine runtime avant toute création, sans rien créer : répertoire réel (ni lien
/// symbolique ni fichier), sans ancêtre lien symbolique non root, propriétaire égal à
/// `expected_uid`, mode exact `0700`. Tout écart est un refus (`REQ-CORE-001`).
pub(crate) fn qualify_runtime_root(root: &Path, expected_uid: u32) -> Result<(), PlatformError> {
    let meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(runtime_root_refusal(root, "absente"));
        }
        Err(e) => return Err(PlatformError::Io(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(runtime_root_refusal(
            root,
            "la racine runtime est un lien symbolique",
        ));
    }
    if !meta.is_dir() {
        return Err(runtime_root_refusal(
            root,
            "la racine runtime n'est pas un répertoire",
        ));
    }
    if let Err(rejection) = check_ancestor_symlinks(root) {
        return Err(runtime_root_refusal(
            root,
            format!("ancêtre refusé ({rejection})"),
        ));
    }
    let actual_uid = meta.uid();
    if actual_uid != expected_uid {
        return Err(runtime_root_refusal(
            root,
            format!("propriétaire UID {actual_uid}, UID attendu {expected_uid}"),
        ));
    }
    let mode = meta.mode() & 0o7777;
    if mode != 0o700 {
        return Err(runtime_root_refusal(
            root,
            format!("mode {mode:#o}, attendu 0o700"),
        ));
    }
    Ok(())
}

/// Crée le sous-répertoire de rendez-vous d'une racine runtime déjà qualifiée, en `0700`, puis le
/// valide. Ne crée jamais la racine runtime ni un ancêtre : un sous-répertoire existant est
/// seulement revalidé ; une racine disparue depuis sa qualification est un refus (`REQ-CORE-001`).
pub(crate) fn create_rendezvous_dir_in_qualified_root(root: &Path) -> Result<(), PlatformError> {
    let dir = root.join(RUNTIME_ROOT_RENDEZVOUS_DIR);
    assert_not_under_claude(&dir)?;
    check_ancestor_symlinks(&dir)?;
    match fs::symlink_metadata(&dir) {
        Ok(_) => validate_rendezvous_dir(&dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {}
                Err(create_err) if create_err.kind() == std::io::ErrorKind::NotFound => {
                    return Err(runtime_root_refusal(
                        root,
                        "la racine runtime a disparu pendant le démarrage",
                    ));
                }
                Err(create_err) if create_err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(create_err) => return Err(PlatformError::Io(create_err)),
            }
            validate_rendezvous_dir(&dir)
        }
        Err(e) => Err(PlatformError::Io(e)),
    }
}

/// Valide un répertoire de rendez-vous existant : non-symlink (`O_NOFOLLOW`), véritable
/// répertoire, propriétaire égal à l'UID effectif courant et permissions exactes `0700`.
///
/// # Errors
/// - [`PlatformError::ForbiddenDocumentationPath`] si `.claude/` est touché.
/// - [`PlatformError::SymlinkNotAllowed`] si `dir` ou un ancêtre non-root est un symlink.
/// - [`PlatformError::NotADirectory`] si `dir` n'est pas un répertoire.
/// - [`PlatformError::OwnershipMismatch`] si le propriétaire diffère de `geteuid()`.
/// - [`PlatformError::InsecurePermissions`] si les droits diffèrent de `0o700`.
pub fn validate_rendezvous_dir(dir: &Path) -> Result<(), PlatformError> {
    assert_not_under_claude(dir)?;
    check_ancestor_symlinks(dir)?;

    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(PlatformError::SymlinkNotAllowed {
            path: dir.display().to_string(),
        });
    }
    if !meta.is_dir() {
        return Err(PlatformError::NotADirectory {
            path: dir.display().to_string(),
        });
    }

    let fd = open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| PlatformError::OsCall {
        op: "open(rendezvous_dir, O_NOFOLLOW|O_DIRECTORY)",
        source: std::io::Error::from(e),
    })?;

    let st = fstat(&fd).map_err(|e| PlatformError::OsCall {
        op: "fstat(rendezvous_dir)",
        source: std::io::Error::from(e),
    })?;

    if FileType::from_raw_mode(st.st_mode) != FileType::Directory {
        return Err(PlatformError::NotADirectory {
            path: dir.display().to_string(),
        });
    }

    let expected_uid = effective_uid();
    if st.st_uid != expected_uid {
        return Err(PlatformError::OwnershipMismatch {
            path: dir.display().to_string(),
            expected_uid,
            actual_uid: st.st_uid,
        });
    }

    let mode_bits = raw_mode_to_u32(st.st_mode) & 0o7777;
    if mode_bits != 0o700 {
        return Err(PlatformError::InsecurePermissions {
            path: dir.display().to_string(),
            expected_mode: 0o700,
            actual_mode: mode_bits,
        });
    }

    Ok(())
}

/// Lie un [`UnixListener`] sur `socket_path` après validation du répertoire parent (`0700`) et
/// de `SUN_LEN`, puis impose immédiatement les permissions `0600` sur le socket.
///
/// # Errors
/// Retourne une [`PlatformError`] si le parent n'est pas conforme `0700`, si `SUN_LEN` est
/// dépassé, si un symlink/répertoire occupe déjà la cible ou si `bind`/`chmod` échoue.
pub fn bind_private_unix_listener(socket_path: &Path) -> Result<UnixListener, PlatformError> {
    bind_private_unix_listener_with(socket_path, || Ok(()))
}

struct BoundSocketCleanup<'a> {
    path: &'a Path,
    device: u64,
    inode: u64,
    armed: bool,
}

impl Drop for BoundSocketCleanup<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Ok(metadata) = fs::symlink_metadata(self.path)
            && metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(self.path);
        }
    }
}

fn bind_private_unix_listener_with(
    socket_path: &Path,
    after_bind: impl FnOnce() -> Result<(), PlatformError>,
) -> Result<UnixListener, PlatformError> {
    validate_unix_socket_path_len(socket_path)?;
    let parent = socket_path
        .parent()
        .ok_or_else(|| PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Le chemin de socket doit avoir un répertoire parent"),
        })?;
    validate_rendezvous_dir(parent)?;

    match fs::symlink_metadata(socket_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(PlatformError::SymlinkNotAllowed {
                    path: socket_path.display().to_string(),
                });
            }
            if meta.is_dir() {
                return Err(PlatformError::InvalidPath {
                    path: socket_path.display().to_string(),
                    reason: String::from("Un répertoire occupe déjà le chemin du socket"),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(PlatformError::Io(e)),
    }

    let listener = UnixListener::bind(socket_path)?;
    let metadata = fs::symlink_metadata(socket_path)?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Socket créé non attribuable"),
        });
    }
    let mut cleanup = BoundSocketCleanup {
        path: socket_path,
        device: metadata.dev(),
        inode: metadata.ino(),
        armed: true,
    };
    after_bind()?;
    fs::set_permissions(socket_path, PermissionsExt::from_mode(0o600))?;
    validate_socket_file(socket_path)?;
    cleanup.armed = false;
    Ok(listener)
}

/// Valide un fichier socket Unix existant : parent `0700`, `SUN_LEN`, non-symlink, type socket,
/// propriétaire égal à `geteuid()` et permissions exactes `0600`.
///
/// # Errors
/// Retourne [`PlatformError`] si l'une des garanties de sécurité du socket ou de son répertoire
/// parent n'est pas satisfaite.
pub fn validate_socket_file(socket_path: &Path) -> Result<(), PlatformError> {
    validate_unix_socket_path_len(socket_path)?;
    let parent = socket_path
        .parent()
        .ok_or_else(|| PlatformError::InvalidPath {
            path: socket_path.display().to_string(),
            reason: String::from("Le chemin de socket doit avoir un répertoire parent"),
        })?;
    validate_rendezvous_dir(parent)?;

    let meta = fs::symlink_metadata(socket_path)?;
    if meta.file_type().is_symlink() {
        return Err(PlatformError::SymlinkNotAllowed {
            path: socket_path.display().to_string(),
        });
    }
    if !meta.file_type().is_socket() {
        return Err(PlatformError::NotASocket {
            path: socket_path.display().to_string(),
        });
    }

    let expected_uid = effective_uid();
    if meta.uid() != expected_uid {
        return Err(PlatformError::OwnershipMismatch {
            path: socket_path.display().to_string(),
            expected_uid,
            actual_uid: meta.uid(),
        });
    }

    let mode_bits = meta.mode() & 0o7777;
    if mode_bits != 0o600 {
        return Err(PlatformError::InsecurePermissions {
            path: socket_path.display().to_string(),
            expected_mode: 0o600,
            actual_mode: mode_bits,
        });
    }

    Ok(())
}

/// Lit un fichier privé régulier sans suivre les liens symboliques (`O_NOFOLLOW`),
/// en vérifiant la garde `.claude/` et la propriété par l'UID effectif courant.
///
/// Retourne `Ok(None)` si le fichier n'existe pas encore.
///
/// # Errors
/// - [`PlatformError::ForbiddenDocumentationPath`] si `path` touche `.claude/`.
/// - [`PlatformError::SymlinkNotAllowed`] si `path` ou un ancêtre non-root est un lien symbolique.
/// - [`PlatformError::NotARegularFile`] ou [`PlatformError::OwnershipMismatch`].
pub fn read_private_file_to_string(path: &Path) -> Result<Option<String>, PlatformError> {
    assert_not_under_claude(path)?;
    check_ancestor_symlinks(path)?;

    let fd = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(errno) if errno == Errno::NOENT => return Ok(None),
        Err(errno) if errno == Errno::LOOP => {
            return Err(PlatformError::SymlinkNotAllowed {
                path: path.display().to_string(),
            });
        }
        Err(errno) => {
            return Err(PlatformError::OsCall {
                op: "open(private_file, O_RDONLY|O_NOFOLLOW)",
                source: std::io::Error::from(errno),
            });
        }
    };

    let st = fstat(&fd).map_err(|e| PlatformError::OsCall {
        op: "fstat(private_file)",
        source: std::io::Error::from(e),
    })?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err(PlatformError::NotARegularFile {
            path: path.display().to_string(),
        });
    }
    let uid = effective_uid();
    if st.st_uid != uid {
        return Err(PlatformError::OwnershipMismatch {
            path: path.display().to_string(),
            expected_uid: uid,
            actual_uid: st.st_uid,
        });
    }

    let mut file = File::from(fd);
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(Some(contents))
}

/// Écrit atomiquement `bytes` dans un fichier privé `0600` (création exclusive temporaire
/// `O_NOFOLLOW`, `sync_all`, renommage atomique et `fsync` du répertoire parent),
/// en appliquant la garde `.claude/`.
///
/// # Errors
/// Retourne [`PlatformError`] si `.claude/` est ciblé, si un symlink non-root bloque le chemin,
/// ou si la création/écriture/synchronisation échoue.
pub fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), PlatformError> {
    assert_not_under_claude(path)?;
    check_ancestor_symlinks(path)?;

    let parent = path.parent().ok_or_else(|| PlatformError::InvalidPath {
        path: path.display().to_string(),
        reason: String::from("Le chemin de fichier doit avoir un répertoire parent"),
    })?;
    if !parent.as_os_str().is_empty() && !parent.exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }

    if let Ok(existing) = fs::symlink_metadata(path)
        && existing.file_type().is_symlink()
    {
        return Err(PlatformError::SymlinkNotAllowed {
            path: path.display().to_string(),
        });
    }

    let seq = ATOMIC_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".unleash-tmp-{}-{seq}", std::process::id());
    let tmp_path = parent.join(tmp_name);

    let tmp_fd = open(
        &tmp_path,
        OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| PlatformError::OsCall {
        op: "open(tmp_private_file, O_CREAT|O_EXCL|O_NOFOLLOW)",
        source: std::io::Error::from(e),
    })?;

    let mut tmp_file = File::from(tmp_fd);
    tmp_file.write_all(bytes)?;
    tmp_file.sync_all()?;
    drop(tmp_file);

    fs::rename(&tmp_path, path)?;
    if let Ok(dir_fd) = open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        let dir_file = File::from(dir_fd);
        let _ = dir_file.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod bind_cleanup_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use unleash_testkit::local_test_dir;

    fn injected() -> PlatformError {
        PlatformError::InvalidPath {
            path: String::new(),
            reason: "injected".into(),
        }
    }

    #[test]
    fn failed_post_bind_removes_owned_socket_and_preserves_foreign_entry() {
        let dir = local_test_dir("m1-bind-cleanup").unwrap();
        let data = local_test_dir("m1-bind-cleanup-data").unwrap();
        fs::set_permissions(dir.path(), PermissionsExt::from_mode(0o700)).unwrap();
        let lock = crate::AuthorityLock::acquire_in(dir.path(), data.path()).unwrap();
        let socket = dir.path().join("control.sock");
        assert!(bind_private_unix_listener_with(&socket, || Err(injected())).is_err());
        assert!(!socket.exists());
        drop(lock);
        let _lock = crate::AuthorityLock::acquire_in(dir.path(), data.path()).unwrap();

        let foreign = UnixListener::bind(&socket).unwrap();
        let identity = fs::symlink_metadata(&socket).unwrap();
        assert!(bind_private_unix_listener_with(&socket, || Err(injected())).is_err());
        let after = fs::symlink_metadata(&socket).unwrap();
        assert_eq!((after.dev(), after.ino()), (identity.dev(), identity.ino()));
        drop(foreign);
        fs::remove_file(&socket).unwrap();
    }

    #[test]
    fn post_bind_cleanup_does_not_remove_replaced_file() {
        let dir = local_test_dir("m1-bind-replaced").unwrap();
        fs::set_permissions(dir.path(), PermissionsExt::from_mode(0o700)).unwrap();
        let socket = dir.path().join("control.sock");
        assert!(
            bind_private_unix_listener_with(&socket, || {
                fs::remove_file(&socket).unwrap();
                fs::write(&socket, b"foreign-fixture").unwrap();
                Err(injected())
            })
            .is_err()
        );
        assert_eq!(fs::read(&socket).unwrap(), b"foreign-fixture");
    }
}
