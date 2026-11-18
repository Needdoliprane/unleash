//! Horloge manuelle déterministe implémentant [`unleash_domain::ClockPort`].
//!
//! Maintient de manière séparée :
//! - l'horloge murale UTC en millisecondes (`TimestampMs`), pouvant avancer ou reculer de façon
//!   déterministe pour simuler un saut NTP ou une dérive horloge ;
//! - une horloge monotone simulée (`Duration`) qui ne recule jamais ;
//! - des notifications de réveil et attentes bornées via `Condvar` et `Waker`, sans `thread::sleep`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::task::Waker;
use std::time::{Duration, Instant};

use unleash_domain::{ClockPort, TimestampMs};

/// Horodatage initial par défaut (`2026-01-01T00:00:00Z` en millisecondes UTC).
const DEFAULT_INITIAL_UTC_MS: i64 = 1_767_225_600_000;

#[derive(Debug, Clone, Copy)]
enum WakeupTarget {
    Monotonic(Duration),
    UtcMs(i64),
}

struct WakeupInner {
    triggered: AtomicBool,
    wait_lock: Mutex<bool>,
    wait_cv: Condvar,
    waker: Mutex<Option<Waker>>,
}

impl WakeupInner {
    fn new(already_triggered: bool) -> Self {
        Self {
            triggered: AtomicBool::new(already_triggered),
            wait_lock: Mutex::new(already_triggered),
            wait_cv: Condvar::new(),
            waker: Mutex::new(None),
        }
    }

    fn trigger(&self) {
        if self.triggered.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut ready = self
            .wait_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *ready = true;
        self.wait_cv.notify_all();
        drop(ready);

        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// Jeton de notification de réveil enregistré sur une [`ManualClock`].
///
/// Permet d'observer de manière non bloquante ou synchrone bornée (via `Condvar`, sans `sleep`)
/// l'atteinte d'une échéance monotone ou UTC.
#[derive(Clone)]
pub struct ClockWakeup {
    inner: Arc<WakeupInner>,
}

impl fmt::Debug for ClockWakeup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClockWakeup")
            .field("triggered", &self.is_triggered())
            .finish()
    }
}

impl ClockWakeup {
    /// Indique si l'échéance associée a été atteinte.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.inner.triggered.load(Ordering::Acquire)
    }

    /// Attend sur une `Condvar` jusqu'au déclenchement du réveil ou jusqu'à `safety_timeout`.
    ///
    /// Retourne `true` si le réveil s'est déclenché, `false` en cas d'expiration de la garde.
    /// N'utilise jamais `std::thread::sleep`.
    #[must_use]
    pub fn wait_bounded(&self, safety_timeout: Duration) -> bool {
        if self.is_triggered() {
            return true;
        }
        let deadline = Instant::now() + safety_timeout;
        let mut guard = self
            .inner
            .wait_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !*guard {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .inner
                .wait_cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && !*guard {
                return false;
            }
        }
        true
    }

    /// Enregistre un [`Waker`] asynchrone à réveiller lorsque l'échéance est atteinte.
    ///
    /// Si l'échéance est déjà atteinte au moment de l'appel, réveille immédiatement `waker`.
    pub fn set_waker(&self, waker: Waker) {
        if self.is_triggered() {
            waker.wake();
            return;
        }
        let mut slot = self
            .inner
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *slot = Some(waker);
        drop(slot);

        if self.is_triggered()
            && let Some(w) = self
                .inner
                .waker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        {
            w.wake();
        }
    }
}

struct WakeupEntry {
    target: WakeupTarget,
    signal: Arc<WakeupInner>,
}

struct ClockState {
    utc_ms: i64,
    monotonic: Duration,
    waiters: usize,
    wakeups: Vec<WakeupEntry>,
}

impl ClockState {
    fn evaluate_wakeups(&mut self) {
        let utc_ms = self.utc_ms;
        let monotonic = self.monotonic;
        let mut remaining = Vec::with_capacity(self.wakeups.len());
        for entry in self.wakeups.drain(..) {
            let fired = match entry.target {
                WakeupTarget::Monotonic(target) => monotonic >= target,
                WakeupTarget::UtcMs(target) => utc_ms >= target,
            };
            if fired {
                entry.signal.trigger();
            } else {
                remaining.push(entry);
            }
        }
        self.wakeups = remaining;
    }
}

struct ManualClockInner {
    state: Mutex<ClockState>,
    state_cv: Condvar,
}

/// Horloge manuelle déterministe injectable pour les tests Unleash.
///
/// Fournit l'horodatage UTC en millisecondes via [`ClockPort`] et une horloge monotone simulée
/// indépendante des sauts en arrière de l'horloge murale. Partageable entre threads via [`Clone`].
#[derive(Clone)]
pub struct ManualClock {
    inner: Arc<ManualClockInner>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new(DEFAULT_INITIAL_UTC_MS)
    }
}

impl fmt::Debug for ManualClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("ManualClock")
            .field("utc_ms", &guard.utc_ms)
            .field("monotonic", &guard.monotonic)
            .field("pending_wakeups", &guard.wakeups.len())
            .field("waiters", &guard.waiters)
            .finish()
    }
}

impl ManualClock {
    /// Crée une horloge manuelle initialisée à `initial_utc_ms` avec un temps monotone à zéro.
    #[must_use]
    pub fn new(initial_utc_ms: i64) -> Self {
        Self::with_monotonic(initial_utc_ms, Duration::ZERO)
    }

    /// Crée une horloge manuelle avec un temps UTC initial et une horloge monotone initiale donnés.
    #[must_use]
    pub fn with_monotonic(initial_utc_ms: i64, initial_monotonic: Duration) -> Self {
        Self {
            inner: Arc::new(ManualClockInner {
                state: Mutex::new(ClockState {
                    utc_ms: initial_utc_ms,
                    monotonic: initial_monotonic,
                    waiters: 0,
                    wakeups: Vec::new(),
                }),
                state_cv: Condvar::new(),
            }),
        }
    }

    /// Renvoie l'horodatage UTC courant en millisecondes brutes.
    #[must_use]
    pub fn utc_ms(&self) -> i64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .utc_ms
    }

    /// Renvoie la valeur courante de l'horloge monotone simulée.
    #[must_use]
    pub fn monotonic_now(&self) -> Duration {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .monotonic
    }

    /// Renvoie la valeur courante de l'horloge monotone simulée en millisecondes (saturant à `u64::MAX`).
    #[must_use]
    pub fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.monotonic_now().as_millis()).unwrap_or(u64::MAX)
    }

    /// Avance simultanément l'horloge monotone et l'horloge UTC de `delta`,
    /// puis déclenche tous les réveils échus sans utiliser `sleep`.
    pub fn advance(&self, delta: Duration) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.monotonic = guard.monotonic.saturating_add(delta);
        let delta_ms = i64::try_from(delta.as_millis()).unwrap_or(i64::MAX);
        guard.utc_ms = guard.utc_ms.saturating_add(delta_ms);
        guard.evaluate_wakeups();
        self.inner.state_cv.notify_all();
    }

    /// Avance simultanément l'horloge monotone et l'horloge UTC de `delta_ms` millisecondes.
    pub fn advance_ms(&self, delta_ms: u64) {
        self.advance(Duration::from_millis(delta_ms));
    }

    /// Avance uniquement l'horloge monotone de `delta` (simule une horloge murale bloquée ou gelée).
    pub fn advance_monotonic(&self, delta: Duration) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.monotonic = guard.monotonic.saturating_add(delta);
        guard.evaluate_wakeups();
        self.inner.state_cv.notify_all();
    }

    /// Fixe l'horloge murale UTC à `new_utc_ms` (avance ou recul) sans jamais faire reculer
    /// l'horloge monotone simulée.
    pub fn set_utc_ms(&self, new_utc_ms: i64) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.utc_ms = new_utc_ms;
        guard.evaluate_wakeups();
        self.inner.state_cv.notify_all();
    }

    /// Décale l'horloge murale UTC de `delta_ms` (positif ou négatif) sans toucher au temps monotone.
    pub fn shift_utc_ms(&self, delta_ms: i64) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.utc_ms = guard.utc_ms.saturating_add(delta_ms);
        guard.evaluate_wakeups();
        self.inner.state_cv.notify_all();
    }

    /// Recule l'horloge murale UTC de `delta_ms` millisecondes tout en préservant la monotonie
    /// stricte de l'horloge monotone simulée.
    pub fn rewind_utc_ms(&self, delta_ms: u64) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let delta = i64::try_from(delta_ms).unwrap_or(i64::MAX);
        guard.utc_ms = guard.utc_ms.saturating_sub(delta);
        guard.evaluate_wakeups();
        self.inner.state_cv.notify_all();
    }

    /// Enregistre une notification de réveil déclenchée dès que l'horloge monotone aura avancé de `after`.
    #[must_use]
    pub fn register_monotonic_wakeup(&self, after: Duration) -> ClockWakeup {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let target = guard.monotonic.saturating_add(after);
        Self::register_locked(&mut guard, WakeupTarget::Monotonic(target))
    }

    /// Enregistre une notification de réveil déclenchée dès que l'horloge monotone atteint `target`.
    #[must_use]
    pub fn register_monotonic_wakeup_at(&self, target: Duration) -> ClockWakeup {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Self::register_locked(&mut guard, WakeupTarget::Monotonic(target))
    }

    /// Enregistre une notification de réveil déclenchée dès que l'horloge UTC atteint `at_utc_ms`.
    #[must_use]
    pub fn register_utc_wakeup(&self, at_utc_ms: TimestampMs) -> ClockWakeup {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Self::register_locked(&mut guard, WakeupTarget::UtcMs(at_utc_ms.as_i64()))
    }

    fn register_locked(guard: &mut ClockState, target: WakeupTarget) -> ClockWakeup {
        let already_due = match target {
            WakeupTarget::Monotonic(t) => guard.monotonic >= t,
            WakeupTarget::UtcMs(t) => guard.utc_ms >= t,
        };
        if already_due {
            return ClockWakeup {
                inner: Arc::new(WakeupInner::new(true)),
            };
        }
        let signal = Arc::new(WakeupInner::new(false));
        guard.wakeups.push(WakeupEntry {
            target,
            signal: Arc::clone(&signal),
        });
        ClockWakeup { inner: signal }
    }

    /// Bloque le thread appelant sur la `Condvar` interne jusqu'à ce que l'horloge monotone
    /// atteigne `target`, ou jusqu'à expiration de `safety_timeout`.
    ///
    /// N'utilise jamais `std::thread::sleep`.
    #[must_use]
    pub fn wait_until_monotonic(&self, target: Duration, safety_timeout: Duration) -> bool {
        let deadline = Instant::now() + safety_timeout;
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if guard.monotonic >= target {
            return true;
        }
        guard.waiters += 1;
        self.inner.state_cv.notify_all();

        // Libère explicitement `guard` avant le `Drop` de `_waiter_guard` afin que
        // `WaiterDecrementGuard` ne tente jamais de reverrouiller `state` depuis le même thread.
        let _waiter_guard = WaiterDecrementGuard { inner: &self.inner };
        while guard.monotonic < target {
            let now = Instant::now();
            if now >= deadline {
                drop(guard);
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .inner
                .state_cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && guard.monotonic < target {
                drop(guard);
                return false;
            }
        }
        drop(guard);
        true
    }

    /// Bloque le thread appelant jusqu'à ce que l'horloge monotone ait avancé de `delta`
    /// par rapport à sa valeur au moment de l'appel, borné par `safety_timeout`.
    #[must_use]
    pub fn wait_for_monotonic_advance(&self, delta: Duration, safety_timeout: Duration) -> bool {
        let target = self.monotonic_now().saturating_add(delta);
        self.wait_until_monotonic(target, safety_timeout)
    }

    /// Attend qu'au moins `min_waiters` threads soient en attente dans [`Self::wait_until_monotonic`]
    /// ou [`Self::wait_for_monotonic_advance`], borné par `safety_timeout`.
    #[must_use]
    pub fn wait_for_waiters(&self, min_waiters: usize, safety_timeout: Duration) -> bool {
        let deadline = Instant::now() + safety_timeout;
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while guard.waiters < min_waiters {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, wait_res) = self
                .inner
                .state_cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if wait_res.timed_out() && guard.waiters < min_waiters {
                return false;
            }
        }
        true
    }

    /// Renvoie le nombre de réveils actuellement en attente d'échéance.
    #[must_use]
    pub fn pending_wakeup_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wakeups
            .len()
    }

    pub(crate) fn cancel_wakeup(&self, wakeup: &ClockWakeup) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wakeups
            .retain(|entry| !Arc::ptr_eq(&entry.signal, &wakeup.inner));
    }
}

struct WaiterDecrementGuard<'a> {
    inner: &'a ManualClockInner,
}

impl Drop for WaiterDecrementGuard<'_> {
    fn drop(&mut self) {
        let mut guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.waiters = guard.waiters.saturating_sub(1);
    }
}

impl ClockPort for ManualClock {
    fn now_ms(&self) -> TimestampMs {
        TimestampMs::new(self.utc_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manual_clock_advance_and_rewind_utc_preserves_monotonic() {
        let clock = ManualClock::new(1_000);
        assert_eq!(clock.now_ms(), TimestampMs::new(1_000));
        assert_eq!(clock.monotonic_now(), Duration::ZERO);

        clock.advance(Duration::from_millis(500));
        assert_eq!(clock.now_ms(), TimestampMs::new(1_500));
        assert_eq!(clock.monotonic_ms(), 500);

        clock.rewind_utc_ms(800);
        assert_eq!(clock.now_ms(), TimestampMs::new(700));
        assert_eq!(clock.monotonic_ms(), 500);
    }

    #[test]
    fn test_manual_clock_wakeup_without_sleep() {
        let clock = ManualClock::new(10_000);
        let mono_wake = clock.register_monotonic_wakeup(Duration::from_secs(5));
        let utc_wake = clock.register_utc_wakeup(TimestampMs::new(13_000));

        assert!(!mono_wake.is_triggered());
        assert!(!utc_wake.is_triggered());

        clock.advance(Duration::from_secs(3));
        assert!(utc_wake.is_triggered());
        assert!(!mono_wake.is_triggered());

        clock.advance(Duration::from_secs(2));
        assert!(mono_wake.is_triggered());
        assert_eq!(clock.pending_wakeup_count(), 0);
    }
}
