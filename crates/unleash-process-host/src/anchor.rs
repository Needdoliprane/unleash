// SPDX-License-Identifier: Apache-2.0
//! Ancrage de cible non réutilisable et sérialisation stricte récolte vs signal (`REQ-TERM-002`, `REQ-TERM-006`, `ADR-05`).
//!
//! Invariants (`process-supervision.md`) :
//! - Le host ne signale que son enfant direct non récolté et le groupe dont cet enfant est leader
//!   (`setsid`), tant que l'enfant n'est pas récolté.
//! - Conditions OS vérifiées : `SIGCHLD` non ignoré, pas de `SA_NOCLDWAIT`, signaux et récolte
//!   sérialisés par le seul handle propriétaire (et `pidfd` sous Linux lorsqu'il est qualifié).
//! - Un PID/PGID observé sans ancrage vivant non récolté est TOUJOURS refusé en `IdentityUnverified` :
//!   jamais de `kill(pid)` ou `kill(-pgid)` sur simple PID observé.

use std::os::fd::OwnedFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus, kill_process, kill_process_group, waitid,
};
use serde::{Deserialize, Serialize};
use unleash_platform::{
    BootIdentity, OsProcessIdentity, ProcessVerification, current_boot_identity,
    inspect_process_identity, verify_process_identity,
};

use crate::error::ProcessHostError;

/// Qualification de la disposition `SIGCHLD` du processus host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigchldQualification {
    /// `true` si `SIGCHLD` était configuré sur `SIG_IGN` (inacceptable pour conserver les zombies).
    pub sigchld_ignored: bool,
    /// `true` si le drapeau `SA_NOCLDWAIT` était actif (récolte automatique noyau = danger de réutilisation PID).
    pub sa_nocldwait: bool,
    /// `true` si la disposition courante garantit la rétention des zombies jusqu'à `waitpid`.
    pub qualified: bool,
}

static SIGCHLD_NOTIFIED: AtomicBool = AtomicBool::new(false);

extern "C" fn sigchld_handler(_sig: libc::c_int) {
    // Opération asynchrone-signal-safe pure sur variable atomique statique (`store` Relaxed).
    SIGCHLD_NOTIFIED.store(true, Ordering::Relaxed);
}

/// Inspecte la disposition `SIGCHLD` actuelle du processus sans la modifier.
///
/// # Errors
/// Retourne [`ProcessHostError::Io`] si `sigaction` échoue.
#[allow(unsafe_code)]
pub fn inspect_sigchld_disposition() -> Result<SigchldQualification, ProcessHostError> {
    let mut old = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: `old` est un tampon valide et aligné pour `libc::sigaction`, avec `act == null`.
    let rc = unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), old.as_mut_ptr()) };
    if rc != 0 {
        return Err(ProcessHostError::Io(std::io::Error::last_os_error()));
    }
    // SAFETY: `sigaction` a retourné `0` et initialisé `old`.
    let old = unsafe { old.assume_init() };
    let sigchld_ignored = old.sa_sigaction == libc::SIG_IGN;
    let sa_nocldwait = (old.sa_flags & libc::SA_NOCLDWAIT) != 0;
    Ok(SigchldQualification {
        sigchld_ignored,
        sa_nocldwait,
        qualified: !sigchld_ignored && !sa_nocldwait,
    })
}

/// Vérifie et installe si besoin un handler `SIGCHLD` sûr (`SA_RESTART | SA_NOCLDSTOP`, sans
/// `SA_NOCLDWAIT` et sans `SIG_IGN`) garantissant le maintien en zombie des enfants directs non récoltés.
///
/// # Errors
/// Retourne [`ProcessHostError::IdentityUnverified`] si après installation la disposition ne
/// respecte toujours pas les conditions requises.
#[allow(unsafe_code)]
pub fn ensure_sigchld_qualified() -> Result<SigchldQualification, ProcessHostError> {
    let current = inspect_sigchld_disposition()?;
    if current.qualified {
        return Ok(current);
    }

    // SAFETY: `libc::sigaction` est une structure C POD (`repr(C)`) initialisable à zéro avant
    // d'assigner `sa_sigaction`, `sa_flags` et `sa_mask`.
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = sigchld_handler as *const () as libc::sighandler_t;
    sa.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
    // SAFETY: `sa_mask` est initialisé par `sigemptyset` avant passage à `sigaction`.
    unsafe {
        libc::sigemptyset(&raw mut sa.sa_mask);
        if libc::sigaction(libc::SIGCHLD, &raw const sa, std::ptr::null_mut()) != 0 {
            return Err(ProcessHostError::Io(std::io::Error::last_os_error()));
        }
    }

    let verified = inspect_sigchld_disposition()?;
    if !verified.qualified {
        return Err(ProcessHostError::IdentityUnverified {
            reason: String::from(
                "disposition SIGCHLD non qualifiable (SIG_IGN ou SA_NOCLDWAIT actif)",
            ),
        });
    }
    Ok(verified)
}

/// Configure `pre_exec` pour un service enfant non-PTY afin qu'il devienne leader de session
/// et de groupe (`setsid()`) avec dispositions de signaux réinitialisées, sans allocation heap.
///
/// # Safety
/// Doit être appelé uniquement sur un [`std::process::Command`] avant `spawn()`.
#[allow(unsafe_code)]
pub fn configure_setsid_pre_exec(cmd: &mut Command) {
    // SAFETY: La fermeture `pre_exec` n'exécute exclusivement que des appels async-signal-safe
    // (`signal`, `sigemptyset`, `sigprocmask`, `setsid`) entre `fork` et `exec`.
    unsafe {
        cmd.pre_exec(|| {
            for signo in [
                libc::SIGCHLD,
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGALRM,
            ] {
                libc::signal(signo, libc::SIG_DFL);
            }
            // `sigset_t` est un type C POD initialisé immédiatement via `sigemptyset`.
            let mut empty_set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut empty_set);
            libc::sigprocmask(
                libc::SIG_SETMASK,
                &raw const empty_set,
                std::ptr::null_mut(),
            );
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Port d'inspection OS injectable pour qualifier et tester de façon déterministe la détection
/// de réutilisation PID/PGID (`T-F011-002`).
pub trait ProcessInspector: Send + Sync {
    /// Retourne l'identité de boot OS actuelle.
    ///
    /// # Errors
    /// Propage les erreurs système de lecture de boot.
    fn current_boot(&self) -> Result<BootIdentity, ProcessHostError>;

    /// Inspecte l'identité courante d'un PID.
    ///
    /// # Errors
    /// Propage [`unleash_platform::PlatformError::ProcessNotFound`] ou autres erreurs OS.
    fn inspect_pid(&self, pid: u32) -> Result<OsProcessIdentity, ProcessHostError>;

    /// Compare l'identité attendue à l'état OS actuel.
    ///
    /// # Errors
    /// Propage les erreurs d'inspection hors `ProcessNotFound`.
    fn verify_identity(
        &self,
        expected: &OsProcessIdentity,
    ) -> Result<ProcessVerification, ProcessHostError>;

    /// Liste les PIDs vivants observés dans le même groupe `pgid` (best-effort pour rapport de descendants).
    fn list_group_members(&self, pgid: u32) -> Vec<OsProcessIdentity>;
}

/// Inspecteur réel s'appuyant sur `unleash-platform` (`proc_pidinfo` macOS / `/proc` Linux).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsProcessInspector;

impl ProcessInspector for OsProcessInspector {
    fn current_boot(&self) -> Result<BootIdentity, ProcessHostError> {
        Ok(current_boot_identity()?)
    }

    fn inspect_pid(&self, pid: u32) -> Result<OsProcessIdentity, ProcessHostError> {
        Ok(inspect_process_identity(pid)?)
    }

    fn verify_identity(
        &self,
        expected: &OsProcessIdentity,
    ) -> Result<ProcessVerification, ProcessHostError> {
        Ok(verify_process_identity(expected)?)
    }

    fn list_group_members(&self, pgid: u32) -> Vec<OsProcessIdentity> {
        list_group_members_os(pgid)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn list_group_members_os(pgid: u32) -> Vec<OsProcessIdentity> {
    // SAFETY: `proc_listallpids(null, 0)` retourne le nombre de PIDs ou -1.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let cap = usize::try_from(count).unwrap_or(0).saturating_add(64);
    let mut pids = vec![0_i32; cap];
    let buf_bytes = libc::c_int::try_from(
        pids.len()
            .saturating_mul(std::mem::size_of::<libc::c_int>()),
    )
    .unwrap_or(libc::c_int::MAX);
    // SAFETY: `pids` dispose d'au moins `buf_bytes` octets alloués et alignés ; `proc_listallpids`
    // retourne le nombre de PIDs écrits ou -1.
    let pid_count =
        unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<libc::c_void>(), buf_bytes) };
    if pid_count <= 0 {
        return Vec::new();
    }
    let filled = usize::try_from(pid_count).unwrap_or(0).min(pids.len());
    let mut members = Vec::new();
    for &raw_pid in pids.get(..filled).unwrap_or(&[]) {
        if let Ok(pid_u32) = u32::try_from(raw_pid)
            && pid_u32 > 0
            && let Ok(id) = inspect_process_identity(pid_u32)
            && id.pgid == pgid
        {
            members.push(id);
        }
    }
    members
}

#[cfg(target_os = "linux")]
fn list_group_members_os(pgid: u32) -> Vec<OsProcessIdentity> {
    let mut members = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return members;
    };
    for entry in entries.flatten() {
        if let Ok(name) = entry.file_name().into_string()
            && let Ok(pid) = name.parse::<u32>()
            && pid > 0
            && let Ok(id) = inspect_process_identity(pid)
            && id.pgid == pgid
        {
            members.push(id);
        }
    }
    members
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn list_group_members_os(_pgid: u32) -> Vec<OsProcessIdentity> {
    Vec::new()
}

/// Signal POSIX autorisé sur une cible ancrée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSignal {
    /// `SIGINT` (interruption coopérative).
    Int,
    /// `SIGTERM` (arrêt coopératif).
    Term,
    /// `SIGHUP` (fermeture session).
    Hup,
    /// `SIGKILL` (arrêt forcé).
    Kill,
}

impl HostSignal {
    const fn to_rustix(self) -> Signal {
        match self {
            Self::Int => Signal::INT,
            Self::Term => Signal::TERM,
            Self::Hup => Signal::HUP,
            Self::Kill => Signal::KILL,
        }
    }
}

/// Statut de sortie observé lors de la récolte du processus enfant direct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildExitStatus {
    /// Code de sortie normal si terminé par `exit`.
    pub exit_code: Option<i32>,
    /// Signal POSIX ayant terminé le processus si tué par signal.
    pub exit_signal: Option<i32>,
    /// Vrai si le code de sortie vaut `0` et aucun signal.
    pub success: bool,
}

impl ChildExitStatus {
    fn from_std(status: ExitStatus) -> Self {
        Self {
            exit_code: status.code(),
            exit_signal: status.signal(),
            success: status.success(),
        }
    }

    fn from_portable(status: &portable_pty::ExitStatus) -> Self {
        let exit_code = if status.signal().is_some() {
            None
        } else {
            i32::try_from(status.exit_code()).ok()
        };
        let exit_signal = status.signal().and_then(parse_portable_signal_number);
        Self {
            exit_code,
            exit_signal,
            success: status.success(),
        }
    }
}

fn parse_portable_signal_number(name: &str) -> Option<i32> {
    if let Some(num) = name.strip_prefix("Signal ") {
        return num.trim().parse::<i32>().ok();
    }
    let upper = name.to_ascii_uppercase();
    if upper.contains("KILL") {
        Some(libc::SIGKILL)
    } else if upper.contains("TERM") {
        Some(libc::SIGTERM)
    } else if upper.contains("INT") {
        Some(libc::SIGINT)
    } else if upper.contains("HUP") || upper.contains("HANGUP") {
        Some(libc::SIGHUP)
    } else if upper.contains("QUIT") {
        Some(libc::SIGQUIT)
    } else if upper.contains("ABRT") || upper.contains("ABORT") {
        Some(libc::SIGABRT)
    } else {
        None
    }
}

/// Handle propriétaire privé d'un enfant direct (PTY ou service non-PTY).
pub enum OwnedChildHandle {
    /// Enfant lancé dans un PTY via `portable-pty` (aucun appel à `ChildKiller::kill` n'est utilisé).
    Pty(Box<dyn portable_pty::Child + Send + Sync>),
    /// Enfant direct de service lancé par `std::process::Command` avec `setsid`.
    Service(Child),
}

impl std::fmt::Debug for OwnedChildHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pty(c) => f.debug_tuple("Pty").field(&c.process_id()).finish(),
            Self::Service(c) => f.debug_tuple("Service").field(&c.id()).finish(),
        }
    }
}

impl OwnedChildHandle {
    fn raw_pid(&self) -> Option<u32> {
        match self {
            Self::Pty(c) => c.process_id(),
            Self::Service(c) => Some(c.id()),
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ChildExitStatus>> {
        match self {
            Self::Pty(c) => Ok(c.try_wait()?.map(|s| ChildExitStatus::from_portable(&s))),
            Self::Service(c) => Ok(c.try_wait()?.map(ChildExitStatus::from_std)),
        }
    }

    fn wait(&mut self) -> std::io::Result<ChildExitStatus> {
        match self {
            Self::Pty(c) => Ok(ChildExitStatus::from_portable(&c.wait()?)),
            Self::Service(c) => Ok(ChildExitStatus::from_std(c.wait()?)),
        }
    }
}

/// État détaillé de la vérification d'identité avant signalisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorVerificationReport {
    /// PID de l'enfant direct ancré.
    pub pid: u32,
    /// PGID de la session `setsid` menée par l'enfant direct.
    pub pgid: u32,
    /// Indique si l'enfant était déjà récolté (`reaped`).
    pub reaped: bool,
    /// Indique si un `pidfd` Linux était détenu.
    pub has_pidfd: bool,
    /// Qualification `SIGCHLD` au moment du contrôle.
    pub sigchld_qualified: bool,
    /// Résultat de vérification OS au moment du contrôle.
    pub identity_status: String,
}

/// Rapport d'observation de l'arrêt d'un processus et de sa descendance (`REQ-TERM-006`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopObservation {
    /// Statut de sortie de l'enfant direct s'il a été observé/récolté.
    pub direct_exit: Option<ChildExitStatus>,
    /// Indique si l'arrêt du groupe `pgid` a été tenté sous ancrage vérifié.
    pub group_signaled: bool,
    /// PIDs observés dans le même PGID encore vivants après la phase de signal.
    pub remaining_group_pids: Vec<u32>,
    /// Descendants signalés comme potentiellement détachés (hors PGID ou après perte de leader).
    pub detached_or_unknown_descendants: Vec<u32>,
    /// Vrai uniquement si l'enfant direct est récolté ET aucun survivant du groupe ni détaché connu n'est observé.
    pub confirmed_group_clear: bool,
    /// Note explicite sur la portée de l'observation (jamais de promesse de sandbox universelle).
    pub observation_note: String,
}

struct AnchorInner {
    handle: Option<OwnedChildHandle>,
    expected_identity: OsProcessIdentity,
    pidfd: Option<OwnedFd>,
    sigchld: SigchldQualification,
    reaped: bool,
    exit_status: Option<ChildExitStatus>,
}

/// Ancrage propriétaire exclusif sérialisant **toutes** les opérations de récolte et de signalisation
/// sur un enfant direct non récolté (`REQ-TERM-002`, `ADR-05`).
pub struct OwnedChildAnchor {
    inner: Mutex<AnchorInner>,
    inspector: Box<dyn ProcessInspector>,
}

impl std::fmt::Debug for OwnedChildAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("OwnedChildAnchor")
            .field("pid", &guard.expected_identity.pid)
            .field("pgid", &guard.expected_identity.pgid)
            .field("reaped", &guard.reaped)
            .field("has_pidfd", &guard.pidfd.is_some())
            .finish_non_exhaustive()
    }
}

impl OwnedChildAnchor {
    /// Capture l'ancrage d'un enfant direct immédiatement après son spawn avec `setsid()`.
    ///
    /// Vérifie que :
    /// 1. `SIGCHLD` n'est pas ignoré ni configuré avec `SA_NOCLDWAIT`.
    /// 2. Le PID est valide et son identité OS complète (`boot_identity`, `start_identity`, `owner_uid`,
    ///    `pgid == pid`) est observée tant que l'enfant n'est pas récolté.
    /// 3. Sous Linux, un `pidfd` est ouvert et revalidé contre `start_identity`.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::IdentityUnverified`] si l'enfant n'a pas de PID, si `setsid`
    /// n'a pas fait de lui le leader de groupe (`pgid != pid`), ou si `SIGCHLD` est invalide.
    pub fn capture_direct_child(
        handle: OwnedChildHandle,
        sigchld: SigchldQualification,
        inspector: Box<dyn ProcessInspector>,
    ) -> Result<Self, ProcessHostError> {
        if !sigchld.qualified {
            return Err(ProcessHostError::IdentityUnverified {
                reason: String::from(
                    "SIGCHLD ignoré ou SA_NOCLDWAIT actif : non-réutilisation PID non garantie",
                ),
            });
        }
        let pid = handle
            .raw_pid()
            .ok_or_else(|| ProcessHostError::IdentityUnverified {
                reason: String::from("PID manquant sur le handle enfant direct"),
            })?;

        let observed =
            inspector
                .inspect_pid(pid)
                .map_err(|err| ProcessHostError::IdentityUnverified {
                    reason: format!(
                        "impossible de qualifier l'identité OS au spawn (pid={pid}) : {err}"
                    ),
                })?;

        if observed.pgid != observed.pid {
            return Err(ProcessHostError::IdentityUnverified {
                reason: format!(
                    "l'enfant direct pid={} n'est pas leader de son groupe (pgid={}) : setsid requis",
                    observed.pid, observed.pgid
                ),
            });
        }

        #[cfg(target_os = "linux")]
        let pidfd = open_verified_pidfd(pid, &observed, inspector.as_ref())?;
        #[cfg(not(target_os = "linux"))]
        let pidfd = None;

        Ok(Self {
            inner: Mutex::new(AnchorInner {
                handle: Some(handle),
                expected_identity: observed,
                pidfd,
                sigchld,
                reaped: false,
                exit_status: None,
            }),
            inspector,
        })
    }

    /// Construit l'erreur [`ProcessHostError::IdentityUnverified`] pour un signal sans ancrage.
    #[must_use]
    pub fn unanchored_signal_error(
        observed: Option<&OsProcessIdentity>,
        reason: &str,
    ) -> ProcessHostError {
        let target_desc = observed.map_or_else(
            || String::from("aucun handle"),
            |id| {
                format!(
                    "pid={}, pgid={}, start={}",
                    id.pid, id.pgid, id.start_identity
                )
            },
        );
        ProcessHostError::IdentityUnverified {
            reason: format!(
                "signalisation refusée sans ancrage direct non récolté ({target_desc}) : {reason}"
            ),
        }
    }

    /// Refus explicite de signaler un processus depuis un simple PID observé sans ancrage parent
    /// vivant (`REQ-TERM-002`, `process-supervision.md`).
    ///
    /// # Errors
    /// Retourne toujours [`ProcessHostError::IdentityUnverified`].
    pub fn refuse_unanchored_signal(
        observed: Option<&OsProcessIdentity>,
        reason: &str,
    ) -> Result<(), ProcessHostError> {
        Err(Self::unanchored_signal_error(observed, reason))
    }

    /// Retourne une copie de l'identité OS capturée au spawn.
    #[must_use]
    pub fn expected_identity(&self) -> OsProcessIdentity {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .expected_identity
            .clone()
    }

    /// Indique si l'enfant direct a déjà été récolté.
    #[must_use]
    pub fn is_reaped(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reaped
    }

    /// Indique si l'enfant direct est déjà terminé (soit récolté, soit zombie en attente de récolte).
    #[must_use]
    pub fn has_exited_or_reaped(&self) -> bool {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.reaped
            || guard.exit_status.is_some()
            || is_direct_child_zombie_without_reaping(guard.expected_identity.pid)
    }

    /// Retourne le statut de sortie si l'enfant a déjà été récolté.
    #[must_use]
    pub fn exit_status(&self) -> Option<ChildExitStatus> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .exit_status
    }

    /// Vérifie si l'enfant direct s'est terminé sans bloquer, et conserve son statut.
    ///
    /// ATTENTION : si `reap_if_exited` vaut `false`, consulte sans libérer l'ancrage (`waitid(WNOWAIT)`)
    /// tant que le contrôle de groupe peut encore être requis ; si `true`, récolte l'enfant terminé.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::Io`] en cas d'erreur système `waitpid`.
    pub fn poll_exit(
        &self,
        reap_if_exited: bool,
    ) -> Result<Option<ChildExitStatus>, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(status) = guard.exit_status {
            return Ok(Some(status));
        }
        if !reap_if_exited {
            return Ok(observe_exit_without_reaping_locked(&guard));
        }
        let Some(handle) = guard.handle.as_mut() else {
            return Ok(guard.exit_status);
        };
        if let Some(status) = handle.try_wait()? {
            guard.reaped = true;
            guard.exit_status = Some(status);
            guard.pidfd = None;
            return Ok(Some(status));
        }
        Ok(None)
    }

    /// Injecte un défaut de qualification `SIGCHLD` (pour tests de cas négatifs `T-F011-002`).
    #[cfg(test)]
    pub(crate) fn set_sigchld_qualified_for_test(&self, qualified: bool) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.sigchld.qualified = qualified;
    }

    /// Vérifie l'ancrage et émet un signal vers l'enfant direct sous le verrou exclusif.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::IdentityUnverified`] si l'enfant est déjà récolté, si
    /// `SIGCHLD` est invalide, si le boot a changé ou si l'identité OS diverge (`IdentityMismatch`).
    pub fn signal_direct_child(&self, sig: HostSignal) -> Result<(), ProcessHostError> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        verify_anchor_locked(&guard, self.inspector.as_ref())?;

        #[cfg(target_os = "linux")]
        if let Some(pidfd) = &guard.pidfd {
            return match rustix::process::pidfd_send_signal(pidfd, sig.to_rustix()) {
                Ok(()) => Ok(()),
                Err(e) if e == rustix::io::Errno::SRCH => Ok(()),
                Err(e) => Err(ProcessHostError::Io(std::io::Error::from(e))),
            };
        }

        let Some(raw_pid) = i32::try_from(guard.expected_identity.pid)
            .ok()
            .and_then(Pid::from_raw)
        else {
            return Err(ProcessHostError::IdentityUnverified {
                reason: String::from("PID invalide pour signalisation"),
            });
        };

        // Si l'enfant est déjà zombie non récolté, signaler ce PID ne fait rien et est sûr tant que
        // le mutex de récolte est tenu.
        match kill_process(raw_pid, sig.to_rustix()) {
            Ok(()) => Ok(()),
            Err(e) if e == rustix::io::Errno::SRCH => Ok(()),
            Err(e) => Err(ProcessHostError::Io(std::io::Error::from(e))),
        }
    }

    /// Vérifie l'ancrage et signale le groupe `setsid` dont l'enfant direct est leader (`killpg`).
    ///
    /// Retourne `Ok(true)` si au moins un processus du groupe a reçu le signal, `Ok(false)` si le
    /// groupe ne contenait plus aucun processus signalable (ex. leader zombie seul sur macOS : `EPERM`/`ESRCH`).
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::IdentityUnverified`] si l'ancrage n'est plus valide ou si
    /// l'identité OS a divergé (`IdentityMismatch` / `BootChanged`).
    pub fn signal_process_group(&self, sig: HostSignal) -> Result<bool, ProcessHostError> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        verify_anchor_locked(&guard, self.inspector.as_ref())?;
        send_group_signal_checked(
            guard.expected_identity.pgid,
            guard.expected_identity.pid,
            sig.to_rustix(),
        )
    }

    /// Récolte explicitement l'enfant direct sous verrou exclusif (une seule fois).
    ///
    /// Après récolte, `reaped` passe à `true` et toute signalisation future est refusée
    /// en `IdentityUnverified`.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::Io`] en cas d'erreur `waitpid`.
    pub fn wait_and_reap(&self) -> Result<ChildExitStatus, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reap_locked(&mut guard)
    }

    /// Exécute la séquence d'arrêt observé (`StopProcess` / `REQ-TERM-006`) :
    /// 1. Observe les membres du groupe avant signal.
    /// 2. Envoie `SIGTERM` au groupe sous ancrage vérifié (puis `SIGKILL` si `escalate_to_sigkill` est vrai
    ///    et que le processus ou le groupe reste vivant après la phase coopérative).
    /// 3. Récolte l'enfant direct.
    /// 4. Ré-observe les membres du groupe et rapporte tout descendant restant/détaché sans jamais
    ///    prétendre à une sandbox totale.
    ///
    /// # Errors
    /// Retourne [`ProcessHostError::IdentityUnverified`] si l'ancrage est perdu/récolté/divergent.
    pub fn stop_and_observe(
        &self,
        escalate_to_sigkill: bool,
        known_descendant_pids: &[u32],
    ) -> Result<StopObservation, ProcessHostError> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        verify_anchor_locked(&guard, self.inspector.as_ref())?;

        let target_pgid = guard.expected_identity.pgid;
        let leader_pid = guard.expected_identity.pid;
        let pre_members = self.inspector.list_group_members(target_pgid);

        let mut group_signaled = send_group_signal_checked(target_pgid, leader_pid, Signal::TERM)?;

        // Envoyer SIGKILL au groupe AVANT la récolte pour préserver l'ancrage PGID non réutilisable.
        if escalate_to_sigkill && child_or_group_alive(&guard, self.inspector.as_ref(), target_pgid)
        {
            group_signaled |= send_group_signal_checked(target_pgid, leader_pid, Signal::KILL)?;
        }

        let direct_exit = Some(reap_locked(&mut guard)?);
        Ok(self.collect_stop_observation(
            &guard,
            direct_exit,
            group_signaled,
            pre_members.len(),
            known_descendant_pids,
        ))
    }

    fn collect_stop_observation(
        &self,
        guard: &AnchorInner,
        direct_exit: Option<ChildExitStatus>,
        group_signaled: bool,
        pre_member_count: usize,
        known_descendant_pids: &[u32],
    ) -> StopObservation {
        let leader_pid = guard.expected_identity.pid;
        let target_pgid = guard.expected_identity.pgid;
        let post_members = self.inspector.list_group_members(target_pgid);
        let remaining_group_pids: Vec<u32> = post_members
            .iter()
            .filter(|m| m.pid != leader_pid)
            .map(|m| m.pid)
            .collect();

        let mut detached_or_unknown = Vec::new();
        for &desc_pid in known_descendant_pids {
            if desc_pid == leader_pid {
                continue;
            }
            if let Ok(desc_id) = self.inspector.inspect_pid(desc_pid)
                && (desc_id.pgid != target_pgid || !remaining_group_pids.contains(&desc_pid))
            {
                detached_or_unknown.push(desc_pid);
            }
        }

        let confirmed_group_clear =
            guard.reaped && remaining_group_pids.is_empty() && detached_or_unknown.is_empty();

        let observation_note = if detached_or_unknown.is_empty() {
            format!(
                "Enfant direct pid={} récolté ; {} membre(s) de groupe observé(s) avant arrêt, {} restant(s) dans pgid={}.",
                leader_pid,
                pre_member_count,
                remaining_group_pids.len(),
                target_pgid
            )
        } else {
            format!(
                "Enfant direct pid={} récolté ; {} descendant(s) détaché(s) ou hors PGID observé(s) ({detached_or_unknown:?}) : une supervision Unix ne constitue pas une sandbox contre un descendant détaché.",
                leader_pid,
                detached_or_unknown.len(),
            )
        };

        StopObservation {
            direct_exit,
            group_signaled,
            remaining_group_pids,
            detached_or_unknown_descendants: detached_or_unknown,
            confirmed_group_clear,
            observation_note,
        }
    }

    /// Produit un diagnostic structuré de l'état de l'ancrage sans effectuer de signal.
    pub fn verification_report(&self) -> AnchorVerificationReport {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let identity_status = if guard.reaped {
            String::from("reaped")
        } else {
            match self.inspector.verify_identity(&guard.expected_identity) {
                #[cfg(target_os = "linux")]
                Ok(ProcessVerification::Match)
                    if observe_exit_without_reaping_locked(&guard).is_some() =>
                {
                    String::from("exited_unreaped_zombie")
                }
                Ok(ProcessVerification::Match) => String::from("match"),
                Ok(ProcessVerification::BootChanged) => String::from("boot_changed"),
                Ok(ProcessVerification::IdentityMismatch { .. }) => {
                    String::from("identity_mismatch")
                }
                Ok(ProcessVerification::Exited) => String::from("exited_unreaped_zombie"),
                Ok(_) => String::from("unrecognized_verification"),
                Err(e) => format!("error:{e}"),
            }
        };
        AnchorVerificationReport {
            pid: guard.expected_identity.pid,
            pgid: guard.expected_identity.pgid,
            reaped: guard.reaped,
            has_pidfd: guard.pidfd.is_some(),
            sigchld_qualified: guard.sigchld.qualified,
            identity_status,
        }
    }
}

fn verify_anchor_locked(
    guard: &AnchorInner,
    inspector: &dyn ProcessInspector,
) -> Result<(), ProcessHostError> {
    if guard.reaped || guard.handle.is_none() {
        return Err(ProcessHostError::IdentityUnverified {
            reason: String::from(
                "l'enfant direct a déjà été récolté : PID/PGID potentiellement réattribuable",
            ),
        });
    }
    if !guard.sigchld.qualified {
        return Err(ProcessHostError::IdentityUnverified {
            reason: String::from("SIGCHLD ignoré ou SA_NOCLDWAIT actif : ancrage invalidé"),
        });
    }

    match inspector.verify_identity(&guard.expected_identity)? {
        ProcessVerification::Match => Ok(()),
        ProcessVerification::BootChanged => Err(ProcessHostError::IdentityUnverified {
            reason: String::from("identité de boot modifiée : processus d'un boot antérieur"),
        }),
        ProcessVerification::IdentityMismatch { observed } => {
            Err(ProcessHostError::IdentityUnverified {
                reason: format!(
                    "réutilisation PID/PGID détectée : attendu start='{}' uid={}, observé start='{}' uid={}",
                    guard.expected_identity.start_identity,
                    guard.expected_identity.owner_uid,
                    observed.start_identity,
                    observed.owner_uid
                ),
            })
        }
        ProcessVerification::Exited => {
            // Sur macOS XNU, `proc_pidinfo(PROC_PIDTBSDINFO)` renvoie ESRCH dès que l'enfant direct
            // entre dans `exit1()` ou devient zombie (`SZOMB`), même avant `waitpid`.
            // `waitid(P_PID, pid, WEXITED|WNOHANG|WNOWAIT) == 0` prouve que le noyau détient toujours
            // cet enfant direct comme non récolté (`!ECHILD`), garantissant que PID et PGID restent
            // réservés par cet ancrage.
            if is_unreaped_direct_child(guard.expected_identity.pid) {
                Ok(())
            } else {
                Err(ProcessHostError::IdentityUnverified {
                    reason: String::from(
                        "enfant absent du système et introuvable par waitid(WNOWAIT) comme enfant direct non récolté",
                    ),
                })
            }
        }
        _ => Err(ProcessHostError::IdentityUnverified {
            reason: String::from("verdict d'identité OS inconnu : refus fail-closed"),
        }),
    }
}

fn send_group_signal_checked(
    target_pgid: u32,
    leader_pid: u32,
    sig: Signal,
) -> Result<bool, ProcessHostError> {
    let _ = leader_pid;
    let Some(pgid_pid) = i32::try_from(target_pgid).ok().and_then(Pid::from_raw) else {
        return Err(ProcessHostError::IdentityUnverified {
            reason: String::from("PGID invalide pour signalisation de groupe"),
        });
    };

    match kill_process_group(pgid_pid, sig) {
        Ok(()) => Ok(true),
        // Sur macOS XNU, kill(-pgid) lorsque le seul membre du groupe est un zombie non récolté
        // renvoie EPERM (et ESRCH si aucun processus). Dans les deux cas sous ancrage non récolté,
        // aucun processus actif du groupe n'est restant.
        Err(e) if e == rustix::io::Errno::SRCH => Ok(false),
        #[cfg(target_os = "macos")]
        Err(e)
            if e == rustix::io::Errno::PERM
                && is_direct_child_zombie_without_reaping(leader_pid) =>
        {
            Ok(false)
        }
        Err(e) => Err(ProcessHostError::Io(std::io::Error::from(e))),
    }
}

fn child_or_group_alive(
    guard: &AnchorInner,
    inspector: &dyn ProcessInspector,
    target_pgid: u32,
) -> bool {
    // Inspecter sans jamais appeler `try_wait()` ici afin de ne pas récolter l'enfant direct
    // avant l'éventuelle escalade `SIGKILL` au groupe.
    let direct_still_running = !guard.reaped
        && guard.exit_status.is_none()
        && !is_direct_child_zombie_without_reaping(guard.expected_identity.pid);
    let group_has_live = !inspector.list_group_members(target_pgid).is_empty();
    direct_still_running || group_has_live
}

fn reap_locked(guard: &mut AnchorInner) -> Result<ChildExitStatus, ProcessHostError> {
    if let Some(status) = guard.exit_status {
        guard.reaped = true;
        guard.pidfd = None;
        return Ok(status);
    }
    let Some(handle) = guard.handle.as_mut() else {
        return Err(ProcessHostError::IdentityUnverified {
            reason: String::from("handle enfant déjà consommé"),
        });
    };
    let status = handle.wait()?;
    guard.reaped = true;
    guard.exit_status = Some(status);
    guard.pidfd = None;
    Ok(status)
}

fn inspect_unreaped_waitid(pid: u32) -> Result<Option<WaitIdStatus>, ()> {
    let raw_pid = i32::try_from(pid).ok().and_then(Pid::from_raw).ok_or(())?;
    waitid(
        WaitId::Pid(raw_pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    )
    .map_err(|_| ())
}

fn observe_exit_without_reaping_locked(guard: &AnchorInner) -> Option<ChildExitStatus> {
    if guard.reaped {
        return guard.exit_status;
    }
    let Ok(Some(status)) = inspect_unreaped_waitid(guard.expected_identity.pid) else {
        return None;
    };
    child_exit_from_waitid(&status)
}

fn child_exit_from_waitid(status: &WaitIdStatus) -> Option<ChildExitStatus> {
    if status.exited() {
        let code = status.exit_status();
        Some(ChildExitStatus {
            exit_code: code,
            exit_signal: None,
            success: code == Some(0),
        })
    } else if status.killed() || status.dumped() {
        Some(ChildExitStatus {
            exit_code: None,
            exit_signal: status.terminating_signal(),
            success: false,
        })
    } else {
        None
    }
}

fn is_unreaped_direct_child(pid: u32) -> bool {
    inspect_unreaped_waitid(pid).is_ok()
}

fn is_direct_child_zombie_without_reaping(pid: u32) -> bool {
    matches!(inspect_unreaped_waitid(pid), Ok(Some(_)))
}

#[cfg(target_os = "linux")]
fn open_verified_pidfd(
    pid: u32,
    expected: &OsProcessIdentity,
    inspector: &dyn ProcessInspector,
) -> Result<Option<OwnedFd>, ProcessHostError> {
    use rustix::process::{PidfdFlags, pidfd_open};

    let Some(raw_pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return Err(ProcessHostError::IdentityUnverified {
            reason: format!("PID invalide {pid} pour pidfd_open"),
        });
    };
    let fd = pidfd_open(raw_pid, PidfdFlags::NONBLOCK).map_err(|e| {
        ProcessHostError::IdentityUnverified {
            reason: format!("pidfd_open(pid={pid}) refusé : {e}"),
        }
    })?;
    // Revalider l'identité après pidfd_open pour éliminer toute course spawn -> pidfd_open.
    match inspector.verify_identity(expected)? {
        ProcessVerification::Match => Ok(Some(fd)),
        other => Err(ProcessHostError::IdentityUnverified {
            reason: format!("course spawn->pidfd_open détectée : {other:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::Arc;
    use unleash_platform::StartIdentity;

    struct InjectedReuseInspector {
        boot: BootIdentity,
        injected_mismatch: Arc<Mutex<Option<OsProcessIdentity>>>,
    }

    impl ProcessInspector for InjectedReuseInspector {
        fn current_boot(&self) -> Result<BootIdentity, ProcessHostError> {
            Ok(self.boot.clone())
        }

        fn inspect_pid(&self, pid: u32) -> Result<OsProcessIdentity, ProcessHostError> {
            inspect_process_identity(pid).map_err(ProcessHostError::from)
        }

        fn verify_identity(
            &self,
            expected: &OsProcessIdentity,
        ) -> Result<ProcessVerification, ProcessHostError> {
            if let Some(mismatch) = self
                .injected_mismatch
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                return Ok(ProcessVerification::IdentityMismatch { observed: mismatch });
            }
            verify_process_identity(expected).map_err(ProcessHostError::from)
        }

        fn list_group_members(&self, pgid: u32) -> Vec<OsProcessIdentity> {
            list_group_members_os(pgid)
        }
    }

    #[test]
    fn t_f011_002_refuses_unanchored_signal_and_pid_reuse_keeping_witness_alive() {
        // 1. Témoin bloqué sur pipe (aucun sleep)
        let mut witness_cmd = Command::new("/bin/cat");
        witness_cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        configure_setsid_pre_exec(&mut witness_cmd);
        let mut witness = witness_cmd.spawn().unwrap();
        let witness_id = inspect_process_identity(witness.id()).unwrap();

        // Refus sans ancrage direct non récolté
        let err = OwnedChildAnchor::refuse_unanchored_signal(
            Some(&witness_id),
            "crash host : seul PID observé disponible",
        )
        .unwrap_err();
        assert!(matches!(err, ProcessHostError::IdentityUnverified { .. }));
        assert!(witness.try_wait().unwrap().is_none());

        // 2. Capture d'ancrage réel puis injection de réutilisation PID entre observation et signal
        let mut child_cmd = Command::new("/bin/cat");
        child_cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        configure_setsid_pre_exec(&mut child_cmd);
        let mut child = child_cmd.spawn().unwrap();
        let child_stdin = child.stdin.take();
        let mut child_stdout = child.stdout.take().unwrap();
        let child_pid = child.id();

        let boot = current_boot_identity().unwrap();
        let mut mismatch_obs = inspect_process_identity(child_pid).unwrap();
        mismatch_obs.start_identity = StartIdentity::new("reused-pid-other-start").unwrap();
        let injected_mismatch = Arc::new(Mutex::new(None));
        let inspector = Box::new(InjectedReuseInspector {
            boot,
            injected_mismatch: Arc::clone(&injected_mismatch),
        });

        let sigchld = ensure_sigchld_qualified().unwrap();
        let anchor = OwnedChildAnchor::capture_direct_child(
            OwnedChildHandle::Service(child),
            sigchld,
            inspector,
        )
        .unwrap();
        *injected_mismatch.lock().unwrap() = Some(mismatch_obs);

        let reuse_err = anchor.signal_direct_child(HostSignal::Term).unwrap_err();
        assert!(matches!(
            reuse_err,
            ProcessHostError::IdentityUnverified { .. }
        ));
        let group_err = anchor.signal_process_group(HostSignal::Kill).unwrap_err();
        assert!(matches!(
            group_err,
            ProcessHostError::IdentityUnverified { .. }
        ));

        // Fermer stdin pour laisser /bin/cat se terminer proprement et récolter
        drop(child_stdin);
        let mut drain = Vec::new();
        let _ = child_stdout.read_to_end(&mut drain);
        let st = anchor.wait_and_reap().unwrap();
        assert!(st.success);

        // 3. Après récolte, toute signalisation ultérieure est refusée en IdentityUnverified
        let post_reap = anchor.signal_direct_child(HostSignal::Term).unwrap_err();
        assert!(matches!(
            post_reap,
            ProcessHostError::IdentityUnverified { .. }
        ));

        // Le témoin est resté intact tout au long du test
        assert!(witness.try_wait().unwrap().is_none());
        let _ = witness.kill();
        let _ = witness.wait();
    }

    #[test]
    fn t_f011_002_refuses_when_sigchld_unqualified() {
        let mut cmd = Command::new("/bin/cat");
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        configure_setsid_pre_exec(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let child_stdin = child.stdin.take();

        let sigchld = ensure_sigchld_qualified().unwrap();
        let anchor = OwnedChildAnchor::capture_direct_child(
            OwnedChildHandle::Service(child),
            sigchld,
            Box::new(OsProcessInspector),
        )
        .unwrap();

        anchor.set_sigchld_qualified_for_test(false);
        let err = anchor.signal_direct_child(HostSignal::Term).unwrap_err();
        assert!(matches!(err, ProcessHostError::IdentityUnverified { .. }));

        drop(child_stdin);
        let _ = anchor.wait_and_reap().unwrap();
    }

    #[test]
    fn poll_exit_non_reaping_reports_sigkill_without_releasing_anchor() {
        let mut cmd = Command::new("/bin/cat");
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        configure_setsid_pre_exec(&mut cmd);
        let child = cmd.spawn().unwrap();
        let pid = child.id();

        let sigchld = ensure_sigchld_qualified().unwrap();
        let anchor = OwnedChildAnchor::capture_direct_child(
            OwnedChildHandle::Service(child),
            sigchld,
            Box::new(OsProcessInspector),
        )
        .unwrap();
        assert_eq!(anchor.verification_report().identity_status, "match");

        let raw_pid = Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        kill_process(raw_pid, Signal::KILL).unwrap();
        while !anchor.has_exited_or_reaped() {
            std::thread::yield_now();
        }

        let observed = anchor
            .poll_exit(false)
            .unwrap()
            .expect("poll_exit(false) doit retourner le statut SIGKILL sans récolter");
        assert!(!anchor.is_reaped());
        assert_eq!(observed.exit_code, None);
        assert_eq!(observed.exit_signal, Some(libc::SIGKILL));
        assert!(!observed.success);
        assert_eq!(
            anchor.verification_report().identity_status,
            "exited_unreaped_zombie"
        );

        let reaped = anchor.wait_and_reap().unwrap();
        assert!(anchor.is_reaped());
        assert_eq!(reaped, observed);
        assert_eq!(anchor.verification_report().identity_status, "reaped");
    }

    #[test]
    fn stop_escalates_sigkill_to_group_without_premature_reap_when_leader_exits_on_sigterm() {
        use std::fs::OpenOptions;
        use std::io::{BufRead, BufReader};
        use unleash_testkit::local_test_dir;

        let dir = local_test_dir("anc-pg").unwrap();
        let hold_fifo = dir.path().join("hold.fifo");
        let fifo_status = Command::new("mkfifo").arg(&hold_fifo).status().unwrap();
        assert!(fifo_status.success());
        let _hold_guard = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&hold_fifo)
            .unwrap();

        let script = r#"/bin/sh -c 'trap "" TERM; printf "GC:%s\n" "$$"; exec /bin/cat < "$1"' gc "$1" & wait"#;
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .arg("leader")
            .arg(&hold_fifo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_setsid_pre_exec(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let child_stdout = child.stdout.take().unwrap();

        let sigchld = ensure_sigchld_qualified().unwrap();
        let anchor = OwnedChildAnchor::capture_direct_child(
            OwnedChildHandle::Service(child),
            sigchld,
            Box::new(OsProcessInspector),
        )
        .unwrap();

        let mut reader = BufReader::new(child_stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).unwrap();
            assert!(n > 0, "EOF avant réception de GC:<pid>");
            if line.trim_end().starts_with("GC:") {
                break;
            }
        }
        let gc_pid: u32 = line.trim().strip_prefix("GC:").unwrap().parse().unwrap();
        let leader_id = anchor.expected_identity();
        assert_ne!(gc_pid, leader_id.pid);
        let grandchild_id = OsProcessInspector.inspect_pid(gc_pid).unwrap();
        assert_eq!(grandchild_id.pgid, leader_id.pgid);

        // Tue le leader seul avec SIGTERM pendant que le petit-enfant ignore SIGTERM.
        anchor.signal_direct_child(HostSignal::Term).unwrap();
        while !anchor.has_exited_or_reaped() {
            std::thread::yield_now();
        }
        assert!(!anchor.is_reaped());
        let pre_stop = anchor.poll_exit(false).unwrap().unwrap();
        assert_eq!(pre_stop.exit_signal, Some(libc::SIGTERM));

        // stop_and_observe doit escalader à SIGKILL sur le PGID AVANT de récolter le leader zombie.
        let obs = anchor.stop_and_observe(true, &[gc_pid]).unwrap();
        assert!(anchor.is_reaped());
        assert_eq!(obs.direct_exit, Some(pre_stop));
        assert!(obs.group_signaled);

        // `hold.fifo` est encore ouvert et `gc_pid` ignore `SIGTERM` : l'EOF du tube `stdout`
        // hérité par `gc_pid` ne peut provenir que de sa terminaison réelle par `SIGKILL` du PGID.
        let mut eof_drain = Vec::new();
        reader.read_to_end(&mut eof_drain).unwrap();
        while matches!(
            OsProcessInspector.verify_identity(&grandchild_id),
            Ok(ProcessVerification::Match)
        ) {
            std::thread::yield_now();
        }
        assert!(
            OsProcessInspector
                .list_group_members(leader_id.pgid)
                .is_empty()
        );
        assert!(matches!(
            OsProcessInspector.verify_identity(&grandchild_id),
            Ok(ProcessVerification::Exited)
        ));
    }
}
