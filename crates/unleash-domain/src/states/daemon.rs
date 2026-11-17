//! Machine d'état du daemon Unleash (`runtime-lifecycle.md`).
//!
//! Définit les 7 modes officiels du daemon, les issues de démarrage et d'arrêt,
//! ainsi que la fonction pure de transition.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// Les 7 modes opérationnels du daemon Unleash (`daemon_state.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    /// Démarrage : acquisition du verrou OS, vérification rendez-vous et permissions.
    Starting,
    /// Récupération : vérification intégrité, acquisition nouvel epoch, inventaire durable.
    Recovering,
    /// Prêt : toutes opérations normales, admission et exécution d'effets autorisées.
    Ready,
    /// Dégradé : diagnostics et lectures actifs, nouvelles admissions restreintes aux sous-systèmes sûrs.
    Degraded,
    /// Récupération en lecture seule : aucune écriture métier ordinaire ; restauration acceptée.
    ReadOnlyRecovery,
    /// Drainage : arrêt des nouvelles admissions, checkpoints et terminaison des exécuteurs.
    Draining,
    /// Arrêté : socket retiré après fermeture propre, données locales conservées.
    Stopped,
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Recovering => write!(f, "recovering"),
            Self::Ready => write!(f, "ready"),
            Self::Degraded => write!(f, "degraded"),
            Self::ReadOnlyRecovery => write!(f, "read_only_recovery"),
            Self::Draining => write!(f, "draining"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// Issues possibles lors de la tentative de démarrage d'une instance de daemon.
///
/// Ce sont des statuts du processus de démarrage, JAMAIS des modes persistés dans `daemon_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonStartupIssue {
    /// Une instance active existe déjà sur le même socket/rendez-vous (instance rejointe).
    AlreadyRunning,
    /// Une autorité est déjà active pour cet utilisateur sur une autre racine de données.
    AuthorityAlreadyActive,
    /// Échec technique critique lors de la séquence de démarrage (permissions, verrou, IO).
    FailedStart,
}

impl fmt::Display for DaemonStartupIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "already_running"),
            Self::AuthorityAlreadyActive => write!(f, "AuthorityAlreadyActive"),
            Self::FailedStart => write!(f, "failed_start"),
        }
    }
}

/// Résultat d'un arrêt de daemon (`DaemonStopped.stop_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    /// Tous les services et exécuteurs ont été neutralisés proprement.
    Complete,
    /// Certains services non critiques n'ont pas répondu dans le délai imparti.
    Partial,
    /// L'état final de certains processus externes reste inconnu (à réconcilier au boot).
    Unknown,
}

impl fmt::Display for StopOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete => write!(f, "complete"),
            Self::Partial => write!(f, "partial"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Effectue une transition pure entre deux modes de daemon selon `runtime-lifecycle.md`.
///
/// # Errors
/// Retourne `DomainTransitionError::InvalidTransition` si la transition n'est pas autorisée.
pub fn transition_daemon_mode(
    current: DaemonMode,
    target: DaemonMode,
) -> Result<DaemonMode, DomainTransitionError> {
    let allowed = matches!(
        (current, target),
        (DaemonMode::Starting, DaemonMode::Recovering)
            | (
                DaemonMode::Recovering,
                DaemonMode::Ready
                    | DaemonMode::Degraded
                    | DaemonMode::ReadOnlyRecovery
                    | DaemonMode::Draining,
            )
            | (
                DaemonMode::Ready,
                DaemonMode::Draining | DaemonMode::Degraded | DaemonMode::ReadOnlyRecovery,
            )
            | (
                DaemonMode::Degraded,
                DaemonMode::Ready | DaemonMode::ReadOnlyRecovery | DaemonMode::Draining,
            )
            | (
                DaemonMode::ReadOnlyRecovery,
                DaemonMode::Draining | DaemonMode::Stopped,
            )
            | (DaemonMode::Draining, DaemonMode::Stopped)
            | (DaemonMode::Stopped, DaemonMode::Starting)
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Daemon",
            from: current.to_string(),
            to: target.to_string(),
            reason:
                "Transition non autorisée par la machine d'état du daemon (runtime-lifecycle.md)"
                    .into(),
        })
    }
}
