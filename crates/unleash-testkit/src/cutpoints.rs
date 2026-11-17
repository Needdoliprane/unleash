//! Points d'injection de pannes nommés (`C0`–`C12`, `C10a`–`C10d`, `C11a`–`C11d`) et contrôleur associé.
//!
//! Modélise exactement la matrice de cutpoints de `.claude/testing/crash-recovery-tests.md`
//! sans aucun `thread::sleep` : les barrières reposent exclusivement sur `Mutex` et `Condvar` bornés.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Énumération exhaustive des 21 points de coupure déterministes de la campagne de pannes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Cutpoint {
    /// Avant toute transaction locale.
    C0,
    /// Pendant écriture locale avant commit.
    C1,
    /// Après commit local et avant émission de la réponse client.
    C2,
    /// Intention durable enregistrée, avant marqueur durable de dispatch.
    C3,
    /// Après marqueur durable de dispatch et avant/pendant I/O externe.
    C4,
    /// Effet externe accompli, receipt perdu avant observation locale.
    C5,
    /// Receipt observé, transaction locale interrompue.
    C6,
    /// Observation committée, notification outbox/inbox absente.
    C7,
    /// Checkpoint ou libération de slot en cours.
    C8,
    /// Réaffectation de lease puis arrivée tardive d'un résultat ancien.
    C9,
    /// Objet durable écrit avant référence, collecte concurrente.
    C10,
    /// Pin de préparation committé, octets ou référence non publiés.
    C10a,
    /// GC sélectionne H puis publication sous pin avant réservation deleting (ou ordre inverse).
    C10b,
    /// Intention deleting/prepared committée, avant/après unlink.
    C10c,
    /// Synchronisation effectuée, tombstone non committé.
    C10d,
    /// Snapshot ou migration incomplet.
    C11,
    /// Barrière de sauvegarde durable, suppression antérieure encore en cours.
    C11a,
    /// Snapshot DB terminé, pins exacts non encore acquis.
    C11b,
    /// Pins acquis puis copie d'objets interrompue.
    C11c,
    /// Octets/manifeste synchronisés, état `complete` non committé.
    C11d,
    /// Reboot ou perte de machine.
    C12,
}

impl Cutpoint {
    /// Tableau exhaustif des 21 points de coupure canoniques dans l'ordre du contrat.
    pub const ALL: [Self; 21] = [
        Self::C0,
        Self::C1,
        Self::C2,
        Self::C3,
        Self::C4,
        Self::C5,
        Self::C6,
        Self::C7,
        Self::C8,
        Self::C9,
        Self::C10,
        Self::C10a,
        Self::C10b,
        Self::C10c,
        Self::C10d,
        Self::C11,
        Self::C11a,
        Self::C11b,
        Self::C11c,
        Self::C11d,
        Self::C12,
    ];

    /// Renvoie la représentation canonique du point de coupure (ex. `"C4"`, `"C10b"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C0 => "C0",
            Self::C1 => "C1",
            Self::C2 => "C2",
            Self::C3 => "C3",
            Self::C4 => "C4",
            Self::C5 => "C5",
            Self::C6 => "C6",
            Self::C7 => "C7",
            Self::C8 => "C8",
            Self::C9 => "C9",
            Self::C10 => "C10",
            Self::C10a => "C10a",
            Self::C10b => "C10b",
            Self::C10c => "C10c",
            Self::C10d => "C10d",
            Self::C11 => "C11",
            Self::C11a => "C11a",
            Self::C11b => "C11b",
            Self::C11c => "C11c",
            Self::C11d => "C11d",
            Self::C12 => "C12",
        }
    }
}

impl fmt::Display for Cutpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Cutpoint {
    type Err = CutpointError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "C0" => Ok(Self::C0),
            "C1" => Ok(Self::C1),
            "C2" => Ok(Self::C2),
            "C3" => Ok(Self::C3),
            "C4" => Ok(Self::C4),
            "C5" => Ok(Self::C5),
            "C6" => Ok(Self::C6),
            "C7" => Ok(Self::C7),
            "C8" => Ok(Self::C8),
            "C9" => Ok(Self::C9),
            "C10" => Ok(Self::C10),
            "C10a" => Ok(Self::C10a),
            "C10b" => Ok(Self::C10b),
            "C10c" => Ok(Self::C10c),
            "C10d" => Ok(Self::C10d),
            "C11" => Ok(Self::C11),
            "C11a" => Ok(Self::C11a),
            "C11b" => Ok(Self::C11b),
            "C11c" => Ok(Self::C11c),
            "C11d" => Ok(Self::C11d),
            "C12" => Ok(Self::C12),
            other => Err(CutpointError::UnknownCutpoint(other.to_owned())),
        }
    }
}

impl TryFrom<&str> for Cutpoint {
    type Error = CutpointError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

/// Erreur produite par le contrôleur de points de coupure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CutpointError {
    /// Identifiant de point de coupure inconnu.
    #[error("Point de coupure inconnu : '{0}'")]
    UnknownCutpoint(String),

    /// Numéro d'occurrence (`hit_index`) invalide : l'indexation commence à `1`.
    #[error("Index d'occurrence invalide ({hit_index}) : une occurrence doit être >= 1")]
    InvalidHitIndex {
        /// Index fourni.
        hit_index: u64,
    },

    /// Panne contrôlée injectée au point de coupure ciblé.
    #[error("Panne injectée au point {cutpoint} : {message}")]
    InjectedFailure {
        /// Point de coupure déclencheur.
        cutpoint: Cutpoint,
        /// Motif configuré.
        message: &'static str,
    },

    /// Expiration bornée de la barrière avant sa libération.
    #[error("Délai expiré ({timeout:?}) sur la barrière du point {cutpoint}")]
    BarrierTimeout {
        /// Point de coupure bloqué.
        cutpoint: Cutpoint,
        /// Durée maximale configurée.
        timeout: Duration,
    },

    /// La barrière a été annulée explicitement avant libération.
    #[error("Barrière annulée au point {cutpoint}")]
    BarrierAborted {
        /// Point de coupure concerné.
        cutpoint: Cutpoint,
    },

    /// Attente d'un nombre d'arrivées sur la barrière expirée.
    #[error("Délai expiré ({timeout:?}) en attendant {expected} arrivée(s) sur la barrière")]
    ArrivalWaitTimeout {
        /// Nombre d'arrivées attendu.
        expected: usize,
        /// Durée d'attente maximale.
        timeout: Duration,
    },

    /// Attente d'une occurrence sur le contrôleur expirée.
    #[error("Délai expiré ({timeout:?}) en attendant {min_count} hit(s) sur {cutpoint}")]
    HitWaitTimeout {
        /// Point de coupure attendu.
        cutpoint: Cutpoint,
        /// Seuil attendu.
        min_count: u64,
        /// Durée maximale.
        timeout: Duration,
    },
}

struct BarrierState {
    arrived: usize,
    released: bool,
    aborted: bool,
}

/// Barrière déterministe synchronisée par `Mutex` et `Condvar` avec délai maximal borné.
///
/// Permet d'ordonnancer des courses critiques (`C10b`, `C11a`, etc.) sans aucun `thread::sleep`.
pub struct CutpointBarrier {
    state: Mutex<BarrierState>,
    cv: Condvar,
    timeout: Duration,
}

impl fmt::Debug for CutpointBarrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("CutpointBarrier")
            .field("arrived", &guard.arrived)
            .field("released", &guard.released)
            .field("aborted", &guard.aborted)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl CutpointBarrier {
    /// Crée une barrière bornée par `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(BarrierState {
                arrived: 0,
                released: false,
                aborted: false,
            }),
            cv: Condvar::new(),
            timeout,
        })
    }

    /// Renvoie le délai maximal configuré pour cette barrière.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Nombre d'exécutions déjà arrivées sur la barrière.
    #[must_use]
    pub fn arrived_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .arrived
    }

    /// Indique si la barrière a été libérée.
    #[must_use]
    pub fn is_released(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .released
    }

    /// Indique si la barrière a été annulée.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .aborted
    }

    /// Libère tous les threads bloqués ou futurs arrivant sur cette barrière.
    pub fn release(&self) {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.released = true;
        self.cv.notify_all();
    }

    /// Annule cette barrière et réveille immédiatement les threads bloqués avec [`CutpointError::BarrierAborted`].
    pub fn abort(&self) {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.aborted = true;
        self.cv.notify_all();
    }

    /// Attend sans `sleep` qu'au moins `expected_count` threads aient atteint la barrière.
    ///
    /// # Errors
    /// - [`CutpointError::BarrierAborted`] si la barrière est annulée pendant l'attente.
    /// - [`CutpointError::ArrivalWaitTimeout`] si `wait_timeout` expire avant `expected_count`.
    pub fn wait_arrived(
        &self,
        expected_count: usize,
        wait_timeout: Duration,
    ) -> Result<(), CutpointError> {
        let deadline = Instant::now() + wait_timeout;
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        while guard.arrived < expected_count && !guard.aborted {
            let now = Instant::now();
            if now >= deadline {
                return Err(CutpointError::ArrivalWaitTimeout {
                    expected: expected_count,
                    timeout: wait_timeout,
                });
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && guard.arrived < expected_count && !guard.aborted {
                return Err(CutpointError::ArrivalWaitTimeout {
                    expected: expected_count,
                    timeout: wait_timeout,
                });
            }
        }
        if guard.aborted && guard.arrived < expected_count {
            return Err(CutpointError::BarrierAborted {
                cutpoint: Cutpoint::C0,
            });
        }
        Ok(())
    }

    fn wait_for_release(&self, cutpoint: Cutpoint) -> Result<(), CutpointError> {
        let deadline = Instant::now() + self.timeout;
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.arrived += 1;
        self.cv.notify_all();

        while !guard.released && !guard.aborted {
            let now = Instant::now();
            if now >= deadline {
                return Err(CutpointError::BarrierTimeout {
                    cutpoint,
                    timeout: self.timeout,
                });
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && !guard.released && !guard.aborted {
                return Err(CutpointError::BarrierTimeout {
                    cutpoint,
                    timeout: self.timeout,
                });
            }
        }

        if guard.aborted {
            Err(CutpointError::BarrierAborted { cutpoint })
        } else {
            Ok(())
        }
    }
}

/// Action exécutée lorsqu'un point de coupure armé est atteint.
#[derive(Debug, Clone)]
pub enum CutpointAction {
    /// Poursuit l'exécution normalement sans effet.
    Continue,
    /// Retourne une erreur typée [`CutpointError::InjectedFailure`] avec le motif donné.
    Fail(&'static str),
    /// Termine brutalement le processus via [`std::process::abort`] (sans exécuter les destructeurs).
    AbortProcess,
    /// Suspend l'exécution sur la barrière `Condvar`/`Mutex` bornée jusqu'à libération ou expiration.
    Barrier(Arc<CutpointBarrier>),
}

#[derive(Default)]
struct ControllerState {
    always_actions: BTreeMap<Cutpoint, CutpointAction>,
    on_hit_actions: BTreeMap<(Cutpoint, u64), CutpointAction>,
    counts: BTreeMap<Cutpoint, u64>,
    history: Vec<Cutpoint>,
}

struct ControllerInner {
    state: Mutex<ControllerState>,
    hit_cv: Condvar,
}

/// Contrôleur thread-safe et partageable ([`Clone`]) des points d'injection de pannes `C0`–`C12`.
#[derive(Clone)]
pub struct CutpointController {
    inner: Arc<ControllerInner>,
}

impl Default for CutpointController {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CutpointController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("CutpointController")
            .field("armed_always", &guard.always_actions.len())
            .field("armed_on_hit", &guard.on_hit_actions.len())
            .field("total_hits", &guard.history.len())
            .finish()
    }
}

impl CutpointController {
    /// Crée un contrôleur vierge où tous les points de coupure exécutent [`CutpointAction::Continue`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ControllerInner {
                state: Mutex::new(ControllerState::default()),
                hit_cv: Condvar::new(),
            }),
        }
    }

    /// Configure `action` pour toute occurrence future du point `cutpoint`.
    pub fn set_action(&self, cutpoint: Cutpoint, action: CutpointAction) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.always_actions.insert(cutpoint, action);
    }

    /// Configure `action` uniquement pour l'occurrence `hit_index` (indexée à partir de `1`) de `cutpoint`.
    ///
    /// # Errors
    /// - [`CutpointError::InvalidHitIndex`] si `hit_index == 0`.
    pub fn set_action_on_hit(
        &self,
        cutpoint: Cutpoint,
        hit_index: u64,
        action: CutpointAction,
    ) -> Result<(), CutpointError> {
        if hit_index == 0 {
            return Err(CutpointError::InvalidHitIndex { hit_index });
        }
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.on_hit_actions.insert((cutpoint, hit_index), action);
        Ok(())
    }

    /// Crée et arme une [`CutpointBarrier`] bornée par `timeout` sur toutes les occurrences de `cutpoint`.
    #[must_use]
    pub fn arm_barrier(&self, cutpoint: Cutpoint, timeout: Duration) -> Arc<CutpointBarrier> {
        let barrier = CutpointBarrier::new(timeout);
        self.set_action(cutpoint, CutpointAction::Barrier(Arc::clone(&barrier)));
        barrier
    }

    /// Crée et arme une [`CutpointBarrier`] bornée par `timeout` pour l'occurrence `hit_index` (1-based) de `cutpoint`.
    ///
    /// # Errors
    /// - [`CutpointError::InvalidHitIndex`] si `hit_index == 0`.
    pub fn arm_barrier_on_hit(
        &self,
        cutpoint: Cutpoint,
        hit_index: u64,
        timeout: Duration,
    ) -> Result<Arc<CutpointBarrier>, CutpointError> {
        let barrier = CutpointBarrier::new(timeout);
        self.set_action_on_hit(
            cutpoint,
            hit_index,
            CutpointAction::Barrier(Arc::clone(&barrier)),
        )?;
        Ok(barrier)
    }

    /// Retire les actions configurées pour `cutpoint`.
    pub fn clear(&self, cutpoint: Cutpoint) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.always_actions.remove(&cutpoint);
        guard.on_hit_actions.retain(|(cp, _), _| *cp != cutpoint);
    }

    /// Supprime toutes les actions configurées sur ce contrôleur.
    pub fn clear_all(&self) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.always_actions.clear();
        guard.on_hit_actions.clear();
    }

    /// Enregistre le passage au point `cutpoint` et exécute l'action configurée après avoir
    /// libéré le verrou interne du contrôleur.
    ///
    /// # Errors
    /// - [`CutpointError::InjectedFailure`] si l'action est [`CutpointAction::Fail`].
    /// - [`CutpointError::BarrierTimeout`] ou [`CutpointError::BarrierAborted`] si une barrière expire ou est annulée.
    pub fn hit(&self, cutpoint: Cutpoint) -> Result<(), CutpointError> {
        let action = {
            let mut guard = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let next_count = guard
                .counts
                .get(&cutpoint)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            guard.counts.insert(cutpoint, next_count);
            guard.history.push(cutpoint);

            let selected = guard
                .on_hit_actions
                .get(&(cutpoint, next_count))
                .or_else(|| guard.always_actions.get(&cutpoint))
                .cloned()
                .unwrap_or(CutpointAction::Continue);
            self.inner.hit_cv.notify_all();
            selected
        };

        match action {
            CutpointAction::Continue => Ok(()),
            CutpointAction::Fail(message) => {
                Err(CutpointError::InjectedFailure { cutpoint, message })
            }
            CutpointAction::AbortProcess => std::process::abort(),
            CutpointAction::Barrier(barrier) => barrier.wait_for_release(cutpoint),
        }
    }

    /// Nombre d'occurrences enregistrées pour `cutpoint`.
    #[must_use]
    pub fn hit_count(&self, cutpoint: Cutpoint) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .counts
            .get(&cutpoint)
            .copied()
            .unwrap_or(0)
    }

    /// Nombre total de passages enregistrés tous points confondus.
    #[must_use]
    pub fn total_hits(&self) -> u64 {
        u64::try_from(
            self.inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .history
                .len(),
        )
        .unwrap_or(u64::MAX)
    }

    /// Historique ordonné des points de coupure rencontrés.
    #[must_use]
    pub fn history(&self) -> Vec<Cutpoint> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .history
            .clone()
    }

    /// Attend sans `sleep` que `cutpoint` ait été atteint au moins `min_count` fois.
    ///
    /// # Errors
    /// - [`CutpointError::HitWaitTimeout`] si `timeout` expire avant d'atteindre `min_count`.
    pub fn wait_for_hit(
        &self,
        cutpoint: Cutpoint,
        min_count: u64,
        timeout: Duration,
    ) -> Result<(), CutpointError> {
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while guard.counts.get(&cutpoint).copied().unwrap_or(0) < min_count {
            let now = Instant::now();
            if now >= deadline {
                return Err(CutpointError::HitWaitTimeout {
                    cutpoint,
                    min_count,
                    timeout,
                });
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .inner
                .hit_cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && guard.counts.get(&cutpoint).copied().unwrap_or(0) < min_count
            {
                return Err(CutpointError::HitWaitTimeout {
                    cutpoint,
                    min_count,
                    timeout,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cutpoint_roundtrip_all_21_variants() {
        assert_eq!(Cutpoint::ALL.len(), 21);
        for cp in Cutpoint::ALL {
            let parsed: Cutpoint = cp.as_str().parse().unwrap();
            assert_eq!(parsed, cp);
            assert_eq!(cp.to_string(), cp.as_str());
        }
        assert!(matches!(
            "C99".parse::<Cutpoint>(),
            Err(CutpointError::UnknownCutpoint(_))
        ));
    }

    #[test]
    fn test_cutpoint_on_nth_hit_and_fail() {
        let ctrl = CutpointController::new();
        ctrl.set_action_on_hit(Cutpoint::C1, 2, CutpointAction::Fail("ioerr"))
            .unwrap();

        assert!(ctrl.hit(Cutpoint::C1).is_ok());
        assert_eq!(
            ctrl.hit(Cutpoint::C1),
            Err(CutpointError::InjectedFailure {
                cutpoint: Cutpoint::C1,
                message: "ioerr",
            })
        );
        assert_eq!(ctrl.hit_count(Cutpoint::C1), 2);
        assert_eq!(ctrl.history(), vec![Cutpoint::C1, Cutpoint::C1]);
    }
}
