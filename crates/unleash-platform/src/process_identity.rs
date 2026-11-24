// SPDX-License-Identifier: Apache-2.0
//! Observation de l'identité OS de boot (`boot_identity`) et de processus (`start_identity`)
//! via `proc_pidinfo` (macOS) et `/proc` (Linux).
//!
//! L'observation seule n'autorise jamais un `kill(pid)` direct sans ancrage parent / `pidfd`
//! (`process-supervision.md`, `F011`).

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::PlatformError;

/// Identité de session de boot du système d'exploitation (`boot_identity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct BootIdentity(String);

impl BootIdentity {
    /// Construit une `BootIdentity` non vide.
    ///
    /// # Errors
    /// Retourne [`PlatformError::InvalidPath`] si la valeur est vide.
    pub fn new(value: impl Into<String>) -> Result<Self, PlatformError> {
        let s = value.into();
        if s.trim().is_empty() {
            return Err(PlatformError::InvalidPath {
                path: String::from("<boot_identity>"),
                reason: String::from("boot_identity ne peut pas être vide"),
            });
        }
        Ok(Self(s))
    }

    /// Représentation textuelle canonique.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BootIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identité de démarrage OS propre à un PID (`start_identity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct StartIdentity(String);

impl StartIdentity {
    /// Construit une `StartIdentity` non vide.
    ///
    /// # Errors
    /// Retourne [`PlatformError::InvalidPath`] si la valeur est vide.
    pub fn new(value: impl Into<String>) -> Result<Self, PlatformError> {
        let s = value.into();
        if s.trim().is_empty() {
            return Err(PlatformError::InvalidPath {
                path: String::from("<start_identity>"),
                reason: String::from("start_identity ne peut pas être vide"),
            });
        }
        Ok(Self(s))
    }

    /// Représentation textuelle canonique.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StartIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Backend OS utilisé pour observer l'identité d'un processus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IdentityBackend {
    /// Apple Darwin `proc_pidinfo(PROC_PIDTBSDINFO)` + `kern.boottime`.
    MacosProcPidinfo,
    /// Linux `/proc/<pid>/stat` + `/proc/sys/kernel/random/boot_id`.
    LinuxProcfs,
}

/// Empreinte d'identité OS observée pour un processus à un instant donné.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OsProcessIdentity {
    /// Identifiant PID.
    pub pid: u32,
    /// Identifiant de groupe de processus PGID.
    pub pgid: u32,
    /// UID effectif/réel propriétaire observé.
    pub owner_uid: u32,
    /// Identité de boot de la machine au moment de l'observation.
    pub boot_identity: BootIdentity,
    /// Identité de démarrage unique du processus dans ce boot.
    pub start_identity: StartIdentity,
    /// Backend OS utilisé.
    pub backend: IdentityBackend,
}

/// Verdict de comparaison entre une identité de processus attendue et l'état OS actuel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProcessVerification {
    /// Le processus vivant correspond à `boot_identity`, `start_identity` et `owner_uid`.
    Match,
    /// La machine a redémarré depuis la capture (`boot_identity` différent).
    BootChanged,
    /// Le PID existe mais appartient à un autre processus (`start_identity` ou `owner_uid` divergent).
    IdentityMismatch {
        /// Identité observée actuellement sur ce PID.
        observed: OsProcessIdentity,
    },
    /// Aucun processus actif ne porte ce PID.
    Exited,
}

/// Lit l'identité de boot courante du système d'exploitation.
///
/// # Errors
/// Retourne [`PlatformError::OsCall`] ou [`PlatformError::Io`] si la lecture échoue.
pub fn current_boot_identity() -> Result<BootIdentity, PlatformError> {
    current_boot_identity_os()
}

/// Observe l'identité OS complète du processus `pid`.
///
/// # Errors
/// - [`PlatformError::ProcessNotFound`] si `pid` n'existe pas (ou est invalide).
/// - [`PlatformError::PermissionDenied`] si les droits OS refusent l'inspection.
/// - [`PlatformError::OsCall`] / [`PlatformError::Io`] pour toute autre erreur système.
pub fn inspect_process_identity(pid: u32) -> Result<OsProcessIdentity, PlatformError> {
    if pid == 0 {
        return Err(PlatformError::ProcessNotFound { pid });
    }
    inspect_process_identity_os(pid)
}

/// Observe l'identité OS complète du processus courant.
///
/// # Errors
/// Propage [`inspect_process_identity`].
pub fn current_process_identity() -> Result<OsProcessIdentity, PlatformError> {
    inspect_process_identity(std::process::id())
}

/// Compare une identité de processus enregistrée à l'état OS courant.
///
/// # Errors
/// Propage les erreurs système autres que `ProcessNotFound` (`ESRCH`).
pub fn verify_process_identity(
    expected: &OsProcessIdentity,
) -> Result<ProcessVerification, PlatformError> {
    let current_boot = current_boot_identity()?;
    if current_boot != expected.boot_identity {
        return Ok(ProcessVerification::BootChanged);
    }

    match inspect_process_identity(expected.pid) {
        Ok(observed) => {
            if observed.start_identity == expected.start_identity
                && observed.owner_uid == expected.owner_uid
                && observed.boot_identity == expected.boot_identity
            {
                Ok(ProcessVerification::Match)
            } else {
                Ok(ProcessVerification::IdentityMismatch { observed })
            }
        }
        Err(PlatformError::ProcessNotFound { .. }) => Ok(ProcessVerification::Exited),
        Err(other) => Err(other),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn current_boot_identity_os() -> Result<BootIdentity, PlatformError> {
    use std::mem::{MaybeUninit, size_of};

    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let mut boottime = MaybeUninit::<libc::timeval>::zeroed();
    let mut len: libc::size_t = size_of::<libc::timeval>();

    // SAFETY: `mib` est un tableau de 2 `c_int`, `boottime` est aligné et dimensionné à
    // `size_of::<libc::timeval>()`, et `len` est initialisé à cette exacte taille.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            boottime.as_mut_ptr().cast::<libc::c_void>(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len != size_of::<libc::timeval>() {
        return Err(PlatformError::OsCall {
            op: "sysctl(CTL_KERN, KERN_BOOTTIME)",
            source: std::io::Error::last_os_error(),
        });
    }

    // SAFETY: `sysctl` a réussi et rempli `size_of::<libc::timeval>()` octets dans `boottime`.
    let tv = unsafe { boottime.assume_init() };
    BootIdentity::new(format!("macos-boottime:{}.{:06}", tv.tv_sec, tv.tv_usec))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn inspect_process_identity_os(pid: u32) -> Result<OsProcessIdentity, PlatformError> {
    use std::mem::{MaybeUninit, size_of};

    let pid_i32 = libc::pid_t::try_from(pid).map_err(|_| PlatformError::ProcessNotFound { pid })?;
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let expected_size = libc::c_int::try_from(size_of::<libc::proc_bsdinfo>()).map_err(|_| {
        PlatformError::OsCall {
            op: "proc_pidinfo(PROC_PIDTBSDINFO)",
            source: std::io::Error::from_raw_os_error(libc::EINVAL),
        }
    })?;

    // SAFETY: `info` est un tampon mutable dimensionné exactement à `size_of::<libc::proc_bsdinfo>()`,
    // passé à `libc::proc_pidinfo` avec flavor `PROC_PIDTBSDINFO`.
    let written = unsafe {
        libc::proc_pidinfo(
            pid_i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast::<libc::c_void>(),
            expected_size,
        )
    };

    if written <= 0 {
        let err = std::io::Error::last_os_error();
        return match err.raw_os_error() {
            Some(libc::ESRCH) | None => Err(PlatformError::ProcessNotFound { pid }),
            Some(libc::EPERM | libc::EACCES) => Err(PlatformError::PermissionDenied {
                context: format!("proc_pidinfo(pid={pid})"),
            }),
            Some(_) => Err(PlatformError::OsCall {
                op: "proc_pidinfo(PROC_PIDTBSDINFO)",
                source: err,
            }),
        };
    }

    if written != expected_size {
        return Err(PlatformError::OsCall {
            op: "proc_pidinfo(PROC_PIDTBSDINFO, short_read)",
            source: std::io::Error::from_raw_os_error(libc::EINVAL),
        });
    }

    // SAFETY: `proc_pidinfo` a écrit exactement `expected_size` octets dans `info`.
    let bsd = unsafe { info.assume_init() };
    let boot_identity = current_boot_identity_os()?;
    let start_identity = StartIdentity::new(format!(
        "macos-pbi:{pid}:{}.{:06}",
        bsd.pbi_start_tvsec, bsd.pbi_start_tvusec
    ))?;

    Ok(OsProcessIdentity {
        pid,
        pgid: bsd.pbi_pgid,
        owner_uid: bsd.pbi_uid,
        boot_identity,
        start_identity,
        backend: IdentityBackend::MacosProcPidinfo,
    })
}

#[cfg(target_os = "linux")]
fn current_boot_identity_os() -> Result<BootIdentity, PlatformError> {
    if let Ok(raw) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return BootIdentity::new(format!("linux-bootid:{trimmed}"));
        }
    }
    let stat = std::fs::read_to_string("/proc/stat")?;
    for line in stat.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return BootIdentity::new(format!("linux-btime:{trimmed}"));
            }
        }
    }
    Err(PlatformError::OsCall {
        op: "read(/proc/sys/kernel/random/boot_id)",
        source: std::io::Error::from(std::io::ErrorKind::NotFound),
    })
}

#[cfg(target_os = "linux")]
fn inspect_process_identity_os(pid: u32) -> Result<OsProcessIdentity, PlatformError> {
    use std::os::unix::fs::MetadataExt;

    let proc_dir = format!("/proc/{pid}");
    let stat_path = format!("{proc_dir}/stat");
    let stat_content = std::fs::read_to_string(&stat_path)
        .map_err(|error| procfs_read_error(pid, &stat_path, error))?;

    let (process_group_id, start_ticks) = parse_linux_proc_stat(&stat_content, pid)?;
    let proc_meta = std::fs::metadata(&proc_dir)
        .map_err(|error| procfs_read_error(pid, &proc_dir, error))?;

    let boot_identity = current_boot_identity_os()?;
    let start_identity = StartIdentity::new(format!("linux-proc:{pid}:{start_ticks}"))?;

    Ok(OsProcessIdentity {
        pid,
        pgid: process_group_id,
        owner_uid: proc_meta.uid(),
        boot_identity,
        start_identity,
        backend: IdentityBackend::LinuxProcfs,
    })
}

/// Classe l'échec d'une lecture procfs du processus `pid` (REQ-TERM-002).
///
/// Une tâche peut disparaître entre la résolution de `/proc/<pid>/stat` et sa lecture :
/// le noyau rend alors `ESRCH` et non `ENOENT`. Les deux expriment la même disparition
/// observée, `ProcessNotFound`, qu'un appelant distingue d'une erreur d'E/S.
#[cfg(any(target_os = "linux", test))]
fn procfs_read_error(pid: u32, path: &str, error: std::io::Error) -> PlatformError {
    match error.kind() {
        std::io::ErrorKind::NotFound => PlatformError::ProcessNotFound { pid },
        std::io::ErrorKind::PermissionDenied => PlatformError::PermissionDenied {
            context: format!("read({path})"),
        },
        _ if error.raw_os_error() == Some(libc::ESRCH) => PlatformError::ProcessNotFound { pid },
        _ => PlatformError::Io(error),
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_proc_stat(stat_content: &str, pid: u32) -> Result<(u32, u64), PlatformError> {
    // Le nom du processus (`comm`, champ 2) est entouré de `(...)` et peut contenir espaces et `)`.
    let Some(close_paren) = stat_content.rfind(')') else {
        return Err(PlatformError::OsCall {
            op: "parse(/proc/<pid>/stat)",
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Format /proc/{pid}/stat invalide"),
            ),
        });
    };
    let after = stat_content
        .get(close_paren + 1..)
        .ok_or_else(|| PlatformError::OsCall {
            op: "parse(/proc/<pid>/stat)",
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Troncature /proc/{pid}/stat"),
            ),
        })?;
    let mut fields = after.split_ascii_whitespace();
    // Index après ')': 0=state, 1=ppid, 2=pgrp (champ 5 global), ..., 19=starttime (champ 22 global).
    let pgrp_str = fields.nth(2).ok_or_else(|| PlatformError::OsCall {
        op: "parse(/proc/<pid>/stat, pgrp)",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "pgrp manquant"),
    })?;
    let process_group_id = pgrp_str.parse::<u32>().map_err(|_| PlatformError::OsCall {
        op: "parse(/proc/<pid>/stat, pgrp)",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "pgrp non numérique"),
    })?;
    let starttime_str = fields.nth(16).ok_or_else(|| PlatformError::OsCall {
        op: "parse(/proc/<pid>/stat, starttime)",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "starttime manquant"),
    })?;
    let starttime = starttime_str
        .parse::<u64>()
        .map_err(|_| PlatformError::OsCall {
            op: "parse(/proc/<pid>/stat, starttime)",
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "starttime non numérique"),
        })?;
    Ok((process_group_id, starttime))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn current_boot_identity_os() -> Result<BootIdentity, PlatformError> {
    Err(PlatformError::OsCall {
        op: "boot_identity_unsupported_os",
        source: std::io::Error::from(std::io::ErrorKind::Unsupported),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn inspect_process_identity_os(_pid: u32) -> Result<OsProcessIdentity, PlatformError> {
    Err(PlatformError::OsCall {
        op: "process_identity_unsupported_os",
        source: std::io::Error::from(std::io::ErrorKind::Unsupported),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_linux_proc_stat, procfs_read_error};
    use crate::errors::PlatformError;

    #[test]
    fn linux_proc_stat_handles_parens_and_spaces_in_comm() {
        let sample = "4210 (my ) weird (proc)) S 100 4200 100 0 -1 4194304 10 0 0 0 0 0 0 0 20 0 1 0 987654321 12345 67";
        let (pgid, starttime) = parse_linux_proc_stat(sample, 4210).unwrap();
        assert_eq!(pgid, 4200);
        assert_eq!(starttime, 987_654_321);
    }

    // T-F011-002 / AC-F011-002 / REQ-TERM-002
    // Linux rend ESRCH, et non ENOENT, pour une tâche disparue entre la résolution de
    // `/proc/<pid>/stat` et sa lecture : c'est une disparition observée, pas une erreur d'E/S.
    #[test]
    fn t_f011_002_linux_vanished_task_is_process_not_found() {
        for raw in [libc::ENOENT, libc::ESRCH] {
            assert!(
                matches!(
                    procfs_read_error(
                        4210,
                        "/proc/4210/stat",
                        std::io::Error::from_raw_os_error(raw)
                    ),
                    PlatformError::ProcessNotFound { pid: 4210 }
                ),
                "errno {raw} must report a vanished process"
            );
        }
        assert!(matches!(
            procfs_read_error(
                4210,
                "/proc/4210/stat",
                std::io::Error::from_raw_os_error(libc::EACCES)
            ),
            PlatformError::PermissionDenied { .. }
        ));
        assert!(matches!(
            procfs_read_error(
                4210,
                "/proc/4210/stat",
                std::io::Error::from_raw_os_error(libc::EIO)
            ),
            PlatformError::Io(_)
        ));
    }
}
