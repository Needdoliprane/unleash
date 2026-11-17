// SPDX-License-Identifier: Apache-2.0
//! Extraction des identités locales du pair sur socket Unix :
//! `LOCAL_PEERCRED` (+ `LOCAL_PEERPID`) sur macOS et `SO_PEERCRED` sur Linux (`REQ-CORE-004`).

use std::os::fd::AsFd;

use crate::errors::PlatformError;
use crate::paths::effective_uid;

/// Identifiants noyau authentifiés du processus pair connecté sur un socket Unix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerCredentials {
    /// UID effectif du pair extrait par le noyau.
    pub uid: u32,
    /// GID principal du pair extrait par le noyau.
    pub gid: u32,
    /// PID du pair si fourni par le noyau (`LOCAL_PEERPID` sur macOS, `SO_PEERCRED` sur Linux).
    pub pid: Option<u32>,
}

/// Extrait les identifiants noyau (`uid`, `gid`, `pid` optionnel) d'un socket Unix connecté.
///
/// # Errors
/// Retourne [`PlatformError::OsCall`] si l'appel `getsockopt` (`LOCAL_PEERCRED` / `SO_PEERCRED`)
/// échoue ou renvoie une structure de taille/version inattendue.
pub fn extract_peer_credentials<Fd: AsFd>(fd: Fd) -> Result<PeerCredentials, PlatformError> {
    extract_peer_credentials_os(fd.as_fd())
}

/// Vérifie que le pair connecté sur `fd` possède le même UID effectif que le processus courant.
///
/// # Errors
/// - Propage [`extract_peer_credentials`].
/// - Retourne [`PlatformError::PeerUidMismatch`] si `creds.uid != geteuid()`.
pub fn verify_same_uid_peer<Fd: AsFd>(fd: Fd) -> Result<PeerCredentials, PlatformError> {
    let creds = extract_peer_credentials(fd)?;
    verify_peer_uid(creds, effective_uid())
}

/// Vérifie purement que `creds.uid == expected_uid`.
///
/// # Errors
/// Retourne [`PlatformError::PeerUidMismatch`] en cas d'écart d'UID.
pub const fn verify_peer_uid(
    creds: PeerCredentials,
    expected_uid: u32,
) -> Result<PeerCredentials, PlatformError> {
    if creds.uid == expected_uid {
        Ok(creds)
    } else {
        Err(PlatformError::PeerUidMismatch {
            expected_uid,
            actual_uid: creds.uid,
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn extract_peer_credentials_os(
    fd: std::os::fd::BorrowedFd<'_>,
) -> Result<PeerCredentials, PlatformError> {
    use std::mem::{MaybeUninit, size_of};
    use std::os::fd::AsRawFd;

    let raw_fd = fd.as_raw_fd();
    let mut cred = MaybeUninit::<libc::xucred>::zeroed();
    let expected_len =
        u32::try_from(size_of::<libc::xucred>()).map_err(|_| PlatformError::OsCall {
            op: "getsockopt(LOCAL_PEERCRED)",
            source: std::io::Error::from_raw_os_error(libc::EINVAL),
        })?;
    let mut len: libc::socklen_t = expected_len;

    // SAFETY: `raw_fd` est un descripteur emprunté valide (`BorrowedFd`), `cred` est aligné et
    // dimensionné à `size_of::<libc::xucred>()`, et `len` est initialisé avec cette exacte taille.
    let rc = unsafe {
        libc::getsockopt(
            raw_fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERCRED,
            cred.as_mut_ptr().cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(PlatformError::OsCall {
            op: "getsockopt(LOCAL_PEERCRED)",
            source: std::io::Error::last_os_error(),
        });
    }
    if len != expected_len {
        return Err(PlatformError::OsCall {
            op: "getsockopt(LOCAL_PEERCRED, size_mismatch)",
            source: std::io::Error::from_raw_os_error(libc::EINVAL),
        });
    }

    // SAFETY: `getsockopt` a retourné `0` et écrit exactement `size_of::<libc::xucred>()` octets.
    let cred = unsafe { cred.assume_init() };
    if cred.cr_version != libc::XUCRED_VERSION {
        return Err(PlatformError::OsCall {
            op: "getsockopt(LOCAL_PEERCRED, version_mismatch)",
            source: std::io::Error::from_raw_os_error(libc::EINVAL),
        });
    }
    let gid = if cred.cr_ngroups > 0 {
        cred.cr_groups.first().copied().unwrap_or(0)
    } else {
        0
    };

    let pid = query_macos_peer_pid(raw_fd);

    Ok(PeerCredentials {
        uid: cred.cr_uid,
        gid,
        pid,
    })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn query_macos_peer_pid(raw_fd: std::os::fd::RawFd) -> Option<u32> {
    use std::mem::{MaybeUninit, size_of};

    let mut pid = MaybeUninit::<libc::pid_t>::zeroed();
    let expected_len = u32::try_from(size_of::<libc::pid_t>()).ok()?;
    let mut len: libc::socklen_t = expected_len;

    // SAFETY: `raw_fd` provient d'un `BorrowedFd` actif pendant l'appel ; `pid` et `len` sont
    // valides et mutables pour `size_of::<libc::pid_t>()`.
    let rc = unsafe {
        libc::getsockopt(
            raw_fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            pid.as_mut_ptr().cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 || len != expected_len {
        return None;
    }

    // SAFETY: `getsockopt(LOCAL_PEERPID)` a réussi et rempli `size_of::<libc::pid_t>()` octets.
    let pid_val = unsafe { pid.assume_init() };
    u32::try_from(pid_val).ok().filter(|&p| p > 0)
}

#[cfg(target_os = "linux")]
fn extract_peer_credentials_os(
    fd: std::os::fd::BorrowedFd<'_>,
) -> Result<PeerCredentials, PlatformError> {
    let ucred = rustix::net::sockopt::socket_peercred(fd).map_err(|e| PlatformError::OsCall {
        op: "getsockopt(SO_PEERCRED)",
        source: std::io::Error::from(e),
    })?;

    Ok(PeerCredentials {
        uid: ucred.uid.as_raw(),
        gid: ucred.gid.as_raw(),
        pid: u32::try_from(ucred.pid.as_raw_nonzero().get()).ok(),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn extract_peer_credentials_os(
    _fd: std::os::fd::BorrowedFd<'_>,
) -> Result<PeerCredentials, PlatformError> {
    Err(PlatformError::OsCall {
        op: "peer_credentials_unsupported_os",
        source: std::io::Error::from(std::io::ErrorKind::Unsupported),
    })
}
