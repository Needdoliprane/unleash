//! Gestion de répertoires temporaires locaux sous `/tmp` compatibles `SUN_LEN`.
//!
//! Garantit que les bases `SQLite` de test, journaux d'oracles et sockets Unix ne sont jamais
//! créés sur un partage réseau SMB ni dans `.claude/` (`REQ-CORE-008`), et que leurs chemins
//! respectent la limite `SUN_LEN` de 104 octets sur macOS (`sockaddr_un.sun_path`).

use std::path::{Component, Path, PathBuf};

use tempfile::{Builder, TempDir};
use thiserror::Error;

/// Taille maximale de `sockaddr_un.sun_path` sur macOS (incluant l'octet nul terminal).
pub const MACOS_SUN_LEN: usize = 104;

/// Longueur maximale utilisable en octets pour un chemin de socket Unix sur macOS (`SUN_LEN - 1`).
pub const MAX_UNIX_SOCKET_PATH_BYTES: usize = MACOS_SUN_LEN - 1;

/// Longueur maximale imposée au répertoire temporaire créé par [`local_test_dir`]
/// afin de laisser au moins 39 octets disponibles pour un nom de socket Unix.
pub const MAX_TEMP_DIR_PATH_BYTES: usize = 64;

const MAX_PREFIX_BYTES: usize = 16;

/// Erreur survenue lors de la création ou de la validation d'un chemin temporaire local.
#[derive(Debug, Error)]
pub enum TempDirError {
    /// Erreur d'I/O système lors de la création ou de la canonisation du répertoire.
    #[error("Erreur I/O répertoire temporaire local : {0}")]
    Io(#[from] std::io::Error),

    /// Le chemin créé ou canonisé dépasse la borne de sûreté pour `SUN_LEN`.
    #[error("Chemin temporaire trop long ({len} octets, maximum {max}) : '{path}'")]
    PathTooLong {
        /// Chemin rejeté.
        path: String,
        /// Longueur observée en octets.
        len: usize,
        /// Borne maximale autorisée en octets.
        max: usize,
    },

    /// Nom de socket Unix invalide (vide, traversée ou séparateur de chemin).
    #[error("Nom de socket Unix invalide : '{name}'")]
    InvalidSocketName {
        /// Nom de socket fourni.
        name: String,
    },

    /// Chemin interdit sous le dossier documentaire `.claude/`.
    #[error("Chemin runtime interdit sous '.claude/' (REQ-CORE-008) : '{path}'")]
    ForbiddenDocumentationPath {
        /// Chemin rejeté.
        path: String,
    },
}

/// Crée un répertoire temporaire local sous `/tmp` (sur Unix) avec un chemin court
/// compatible avec la limite macOS `SUN_LEN` (104 octets pour les sockets Unix).
///
/// Le préfixe fourni est assaini (caractères ASCII alphanumériques, `-` et `_` uniquement)
/// et borné à 16 caractères pour garantir une longueur finale maîtrisée, jamais sur SMB.
///
/// # Errors
/// - [`TempDirError::Io`] si la création ou la canonisation sous `/tmp` échoue.
/// - [`TempDirError::PathTooLong`] si le chemin brut ou canonique dépasse [`MAX_TEMP_DIR_PATH_BYTES`].
/// - [`TempDirError::ForbiddenDocumentationPath`] si le chemin résolu touche `.claude/`.
pub fn local_test_dir(prefix: &str) -> Result<TempDir, TempDirError> {
    let base = local_tmp_base();
    let sanitized = sanitize_prefix(prefix);
    let dir = Builder::new()
        .prefix(&sanitized)
        .rand_bytes(6)
        .tempdir_in(&base)?;

    validate_no_claude_component(dir.path())?;
    check_dir_len(dir.path(), MAX_TEMP_DIR_PATH_BYTES)?;

    let canonical = std::fs::canonicalize(dir.path())?;
    validate_no_claude_component(&canonical)?;
    check_dir_len(&canonical, MAX_TEMP_DIR_PATH_BYTES)?;

    Ok(dir)
}

/// Construit et valide un chemin de socket Unix à l'intérieur de `dir`, en garantissant
/// que ni le chemin brut ni le chemin canonique ne dépassent [`MAX_UNIX_SOCKET_PATH_BYTES`] (103 octets).
///
/// # Errors
/// - [`TempDirError::InvalidSocketName`] si `socket_name` est vide, contient un séparateur,
///   un octet nul, `.` ou `..`.
/// - [`TempDirError::ForbiddenDocumentationPath`] si `dir` se situe sous `.claude/`.
/// - [`TempDirError::PathTooLong`] si le chemin complet dépasse 103 octets (`< 104` `SUN_LEN`).
/// - [`TempDirError::Io`] si `dir` ne peut pas être canonisé.
pub fn socket_path_in(dir: &Path, socket_name: &str) -> Result<PathBuf, TempDirError> {
    if socket_name.is_empty()
        || socket_name == "."
        || socket_name == ".."
        || socket_name.contains('/')
        || socket_name.contains('\\')
        || socket_name.contains('\0')
    {
        return Err(TempDirError::InvalidSocketName {
            name: socket_name.to_owned(),
        });
    }

    validate_no_claude_component(dir)?;

    let joined = dir.join(socket_name);
    check_dir_len(&joined, MAX_UNIX_SOCKET_PATH_BYTES)?;

    let canonical_dir = std::fs::canonicalize(dir)?;
    validate_no_claude_component(&canonical_dir)?;
    let canonical_socket = canonical_dir.join(socket_name);
    check_dir_len(&canonical_socket, MAX_UNIX_SOCKET_PATH_BYTES)?;

    Ok(joined)
}

pub(crate) fn validate_no_claude_component(path: &Path) -> Result<(), TempDirError> {
    for component in path.components() {
        if let Component::Normal(segment) = component
            && segment == ".claude"
        {
            return Err(TempDirError::ForbiddenDocumentationPath {
                path: path.display().to_string(),
            });
        }
    }
    Ok(())
}

fn local_tmp_base() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir()
    }
}

fn sanitize_prefix(prefix: &str) -> String {
    let mut out = String::with_capacity(MAX_PREFIX_BYTES + 1);
    for ch in prefix.chars() {
        if out.len() >= MAX_PREFIX_BYTES {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        }
    }
    if out.is_empty() {
        out.push_str("utk");
    }
    if !out.ends_with('-') && !out.ends_with('_') {
        out.push('-');
    }
    out
}

fn check_dir_len(path: &Path, max: usize) -> Result<(), TempDirError> {
    let len = path.as_os_str().len();
    if len > max {
        return Err(TempDirError::PathTooLong {
            path: path.display().to_string(),
            len,
            max,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_test_dir_creates_short_path_under_tmp() {
        let dir = local_test_dir("my_very_long_prefix_exceeding_limits_123456789").unwrap();
        let raw = dir.path();
        assert!(raw.starts_with("/tmp"));
        assert!(raw.as_os_str().len() <= MAX_TEMP_DIR_PATH_BYTES);
        let sock = socket_path_in(raw, "daemon.sock").unwrap();
        assert!(sock.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_BYTES);
    }

    #[test]
    fn test_socket_path_rejects_traversal_and_too_long() {
        let dir = local_test_dir("sock").unwrap();
        assert!(matches!(
            socket_path_in(dir.path(), "../evil.sock"),
            Err(TempDirError::InvalidSocketName { .. })
        ));
        assert!(matches!(
            socket_path_in(dir.path(), ""),
            Err(TempDirError::InvalidSocketName { .. })
        ));
        let long_name = format!("{}.sock", "a".repeat(95));
        assert!(matches!(
            socket_path_in(dir.path(), &long_name),
            Err(TempDirError::PathTooLong { .. })
        ));
    }
}
