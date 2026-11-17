//! Connexion writer unique sur thread dédié, file bornée à 8 demandes, pression WAL et lecteurs bornés.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{Connection, InterruptHandle, OpenFlags, Transaction, TransactionBehavior};
use tokio::sync::oneshot;
use unleash_domain::TimestampMs;

use crate::config::{FaultTarget, StoreConfig, WriteCriticality, check_injected_fault};
use crate::error::StoreError;
use crate::qualification::apply_and_verify_reader_pragmas;

/// Télémétrie instantanée de la file writer et du journal WAL (`ADR-01`, R6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WalTelemetry {
    /// Profondeur courante de la file du writer.
    pub queue_depth: usize,
    /// Profondeur maximale observée depuis l'ouverture.
    pub max_queue_depth_observed: usize,
    /// Nombre total de requêtes soumises.
    pub total_submitted: u64,
    /// Nombre total de requêtes rejetées par backpressure (> 8).
    pub total_rejected_backpressure: u64,
    /// Nombre total d'écritures non critiques rejetées au seuil d'arrêt WAL (32 MiB).
    pub total_rejected_wal_stop: u64,
    /// Taille actuelle du fichier `-wal` en octets.
    pub wal_bytes: u64,
    /// Taille maximale du fichier `-wal` observée.
    pub max_wal_bytes_observed: u64,
    /// Nombre total de checkpoints `PASSIVE` exécutés.
    pub passive_checkpoints: u64,
    /// Nombre total de checkpoints `TRUNCATE` exécutés.
    pub truncate_checkpoints: u64,
    /// Nombre de frames totales / checkpointées lors du dernier checkpoint `(n_log, n_ckpt)`.
    pub last_checkpoint_frames: Option<(i32, i32)>,
    /// Nombre de lecteurs actifs en cours.
    pub active_readers: usize,
    /// Âge du plus vieux lecteur actif en millisecondes selon l'horloge monotone injectée.
    pub oldest_reader_age_ms: Option<u64>,
    /// Nombre de lecteurs expirés neutralisés de force.
    pub expired_readers_terminated: u64,
    /// Vrai si au moins un lecteur expiré n'a pas pu être neutralisé.
    pub reader_neutralization_blocked: bool,
}

/// Jeton de contrôle permettant au registry d'interrompre et de libérer un lecteur expiré.
pub struct ReaderControl {
    pub(crate) reader_id: u64,
    pub(crate) started_at: Duration,
    pub(crate) deadline_at: Duration,
    pub(crate) interrupt_handle: InterruptHandle,
    pub(crate) aborted: AtomicBool,
    pub(crate) block_neutralization: AtomicBool,
    pub(crate) released: Arc<AtomicBool>,
    pub(crate) wake_signal: Option<Arc<(Mutex<bool>, Condvar)>>,
    pub(crate) released_cv: Arc<(Mutex<bool>, Condvar)>,
}

impl std::fmt::Debug for ReaderControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderControl")
            .field("reader_id", &self.reader_id)
            .field("started_at", &self.started_at)
            .field("deadline_at", &self.deadline_at)
            .field("aborted", &self.is_aborted())
            .finish_non_exhaustive()
    }
}

impl ReaderControl {
    /// Identifiant unique du lecteur enregistré.
    #[must_use]
    pub const fn reader_id(&self) -> u64 {
        self.reader_id
    }

    /// Indique si le lecteur a été interrompu pour dépassement de délai.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Active ou désactive le blocage de neutralisation (fixture de test R6).
    pub fn set_block_neutralization(&self, blocked: bool) {
        self.block_neutralization.store(blocked, Ordering::Release);
    }
}

/// Jeton de contrôle d'une session lectrice maintenue ouverte pour les tests R6 (`query_only=ON`).
pub struct HeldReaderSession {
    pub(crate) reader_id: u64,
    pub(crate) control: Arc<ReaderControl>,
    pub(crate) released: Arc<AtomicBool>,
    pub(crate) wake_signal: Arc<(Mutex<bool>, Condvar)>,
    pub(crate) join_handle: Option<JoinHandle<Result<(), StoreError>>>,
}

impl std::fmt::Debug for HeldReaderSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldReaderSession")
            .field("reader_id", &self.reader_id)
            .field("aborted", &self.is_aborted())
            .field("released", &self.is_released())
            .finish_non_exhaustive()
    }
}

impl HeldReaderSession {
    /// Identifiant du lecteur.
    #[must_use]
    pub const fn reader_id(&self) -> u64 {
        self.reader_id
    }

    /// Indique si le lecteur a été interrompu pour dépassement de délai.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.control.is_aborted()
    }

    /// Indique si la transaction `SQLite` sous-jacente a réellement été libérée/fermée.
    #[must_use]
    pub fn is_released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    /// Empêche ou autorise la libération coopérative de la transaction (fixture R6 neutralisation bloquée).
    pub fn set_block_neutralization(&self, blocked: bool) {
        self.control.set_block_neutralization(blocked);
    }

    /// Libère explicitement la session lectrice et attend la fermeture effective de sa connexion.
    ///
    /// # Errors
    /// Retourne [`StoreError`] si le thread lecteur a remonté une erreur inattendue.
    pub fn release(mut self) -> Result<(), StoreError> {
        self.control.set_block_neutralization(false);
        {
            let (lock, cv) = &*self.wake_signal;
            let mut flag = lock.lock().unwrap_or_else(PoisonError::into_inner);
            *flag = true;
            cv.notify_all();
        }
        if let Some(handle) = self.join_handle.take() {
            match handle.join() {
                Ok(res) => res?,
                Err(_) => {
                    return Err(StoreError::InvariantViolation(
                        "thread lecteur paniqué".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Drop for HeldReaderSession {
    fn drop(&mut self) {
        self.control.set_block_neutralization(false);
        let (lock, cv) = &*self.wake_signal;
        let mut flag = lock.lock().unwrap_or_else(PoisonError::into_inner);
        *flag = true;
        cv.notify_all();
        drop(flag);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Jeton fourni à une fermeture lectrice pour observer les interruptions de dépassement.
#[derive(Debug, Clone)]
pub struct ReaderContext {
    pub(crate) control: Arc<ReaderControl>,
}

impl ReaderContext {
    /// Identifiant du lecteur.
    #[must_use]
    pub fn reader_id(&self) -> u64 {
        self.control.reader_id()
    }

    /// Indique si le lecteur a été annulé pour expiration de sa fenêtre de 5 s.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.control.is_aborted()
    }

    /// Simule un blocage de neutralisation sur ce lecteur (test R6).
    pub fn set_block_neutralization(&self, blocked: bool) {
        self.control.set_block_neutralization(blocked);
    }
}

/// Registre thread-safe des lecteurs actifs permettant d'expirer et interrompre toute lecture > 5 s.
#[derive(Debug)]
pub(crate) struct ReaderRegistry {
    next_id: AtomicU64,
    active: Mutex<BTreeMap<u64, Arc<ReaderControl>>>,
}

impl ReaderRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicU64::new(1),
            active: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn register(
        &self,
        started_at: Duration,
        timeout: Duration,
        interrupt_handle: InterruptHandle,
        wake_signal: Option<Arc<(Mutex<bool>, Condvar)>>,
    ) -> Arc<ReaderControl> {
        let reader_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let control = Arc::new(ReaderControl {
            reader_id,
            started_at,
            deadline_at: started_at.saturating_add(timeout),
            interrupt_handle,
            aborted: AtomicBool::new(false),
            block_neutralization: AtomicBool::new(false),
            released: Arc::new(AtomicBool::new(false)),
            wake_signal,
            released_cv: Arc::new((Mutex::new(false), Condvar::new())),
        });
        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        guard.insert(reader_id, Arc::clone(&control));
        control
    }

    pub(crate) fn unregister(&self, reader_id: u64) {
        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ctrl) = guard.remove(&reader_id) {
            ctrl.released.store(true, Ordering::Release);
            let (lock, cv) = &*ctrl.released_cv;
            let mut done = lock.lock().unwrap_or_else(PoisonError::into_inner);
            *done = true;
            cv.notify_all();
        }
    }

    pub(crate) fn reap_expired(&self, now_monotonic: Duration) -> (u64, bool, usize, Option<u64>) {
        let expired: Vec<Arc<ReaderControl>> = {
            let guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            guard
                .values()
                .filter(|c| now_monotonic >= c.deadline_at)
                .cloned()
                .collect()
        };

        let mut terminated = 0_u64;
        let mut blocked = false;

        for ctrl in expired {
            ctrl.aborted.store(true, Ordering::Release);
            ctrl.interrupt_handle.interrupt();

            if ctrl.block_neutralization.load(Ordering::Acquire) {
                blocked = true;
                continue;
            }

            if let Some(ref wake) = ctrl.wake_signal {
                {
                    let (wlock, wcv) = &**wake;
                    let mut flag = wlock.lock().unwrap_or_else(PoisonError::into_inner);
                    *flag = true;
                    wcv.notify_all();
                }
                let (lock, cv) = &*ctrl.released_cv;
                let wait_deadline = Instant::now() + Duration::from_millis(500);
                let mut done = lock.lock().unwrap_or_else(PoisonError::into_inner);
                while !*done && Instant::now() < wait_deadline {
                    let rem = wait_deadline.saturating_duration_since(Instant::now());
                    let (next_done, _) = cv
                        .wait_timeout(done, rem)
                        .unwrap_or_else(PoisonError::into_inner);
                    done = next_done;
                }
                if *done {
                    terminated += 1;
                } else {
                    blocked = true;
                }
            } else {
                terminated += 1;
            }
        }

        let guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let active_count = guard.len();
        let oldest_age = guard
            .values()
            .map(|c| {
                u64::try_from(now_monotonic.saturating_sub(c.started_at).as_millis())
                    .unwrap_or(u64::MAX)
            })
            .max();
        (terminated, blocked, active_count, oldest_age)
    }
}

/// Calcule la taille actuelle du fichier `-wal` associé à `db_path`.
#[must_use]
pub(crate) fn wal_file_bytes(db_path: &Path) -> u64 {
    let mut wal_os = db_path.as_os_str().to_os_string();
    wal_os.push("-wal");
    let wal_path = PathBuf::from(wal_os);
    std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0)
}

type WriterClosure = Box<dyn FnOnce(Result<(&mut Connection, TimestampMs), StoreError>) + Send>;

enum WriterCommand {
    RunTx {
        criticality: WriteCriticality,
        task: WriterClosure,
    },
    MaintenanceTick {
        truncate: bool,
        reply: oneshot::Sender<Result<WalTelemetry, StoreError>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

struct QueueInner {
    items: VecDeque<WriterCommand>,
    closed: bool,
    telemetry: WalTelemetry,
}

/// Handle vers le thread writer unique propriétaire de la connexion d'écriture `SQLite`.
pub(crate) struct StoreWriterHandle {
    config: StoreConfig,
    readers: Arc<ReaderRegistry>,
    queue: Arc<(Mutex<QueueInner>, Condvar)>,
    join_handle: Option<JoinHandle<()>>,
}

impl StoreWriterHandle {
    pub(crate) fn start(
        config: StoreConfig,
        conn: Connection,
        readers: Arc<ReaderRegistry>,
    ) -> Result<Self, StoreError> {
        let initial_wal = wal_file_bytes(&config.db_path);
        let queue = Arc::new((
            Mutex::new(QueueInner {
                items: VecDeque::with_capacity(config.writer_queue_capacity),
                closed: false,
                telemetry: WalTelemetry {
                    wal_bytes: initial_wal,
                    max_wal_bytes_observed: initial_wal,
                    ..WalTelemetry::default()
                },
            }),
            Condvar::new(),
        ));

        let q_worker = Arc::clone(&queue);
        let cfg_worker = config.clone();
        let readers_worker = Arc::clone(&readers);

        let handle = std::thread::Builder::new()
            .name("unleash-store-writer".to_owned())
            .spawn(move || {
                writer_thread_loop(conn, &cfg_worker, &readers_worker, &q_worker);
            })
            .map_err(StoreError::Io)?;

        Ok(Self {
            config,
            readers,
            queue,
            join_handle: Some(handle),
        })
    }

    pub(crate) fn submit_blocking<T, F>(
        &self,
        criticality: WriteCriticality,
        f: F,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let rx = self.enqueue_tx(criticality, f)?;
        rx.blocking_recv()
            .map_err(|_| StoreError::StorageUnavailable("thread writer arrêté".to_owned()))?
    }

    pub(crate) fn submit_immediate_blocking<T, F>(
        &self,
        criticality: WriteCriticality,
        enforce_tx_wal_cap: bool,
        f: F,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let cfg = self.config.clone();
        self.submit_blocking(criticality, move |conn, now| {
            run_guarded_immediate_tx(conn, &cfg, criticality, enforce_tx_wal_cap, now, f)
        })
    }

    pub(crate) async fn submit_async<T, F>(
        &self,
        criticality: WriteCriticality,
        f: F,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let rx = self.enqueue_tx(criticality, f)?;
        rx.await
            .map_err(|_| StoreError::StorageUnavailable("thread writer arrêté".to_owned()))?
    }

    pub(crate) async fn submit_immediate_async<T, F>(
        &self,
        criticality: WriteCriticality,
        enforce_tx_wal_cap: bool,
        f: F,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let cfg = self.config.clone();
        self.submit_async(criticality, move |conn, now| {
            run_guarded_immediate_tx(conn, &cfg, criticality, enforce_tx_wal_cap, now, f)
        })
        .await
    }

    fn enqueue_tx<T, F>(
        &self,
        criticality: WriteCriticality,
        f: F,
    ) -> Result<oneshot::Receiver<Result<T, StoreError>>, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection, TimestampMs) -> Result<T, StoreError> + Send + 'static,
    {
        let (res_tx, res_rx) = oneshot::channel();
        let wrapped: WriterClosure = Box::new(move |outcome| match outcome {
            Ok((conn, now)) => {
                let _ = res_tx.send(f(conn, now));
            }
            Err(err) => {
                let _ = res_tx.send(Err(err));
            }
        });

        let (lock, cv) = &*self.queue;
        {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.closed {
                return Err(StoreError::StorageUnavailable(
                    "writer store déjà fermé".to_owned(),
                ));
            }
            guard.telemetry.total_submitted += 1;
            let depth = guard.items.len();
            if depth >= self.config.writer_queue_capacity {
                guard.telemetry.total_rejected_backpressure += 1;
                tracing::warn!(
                    queue_depth = depth,
                    max_depth = self.config.writer_queue_capacity,
                    "file du writer saturée : BACKPRESSURE"
                );
                return Err(StoreError::Backpressure {
                    queue_depth: depth,
                    max_depth: self.config.writer_queue_capacity,
                });
            }
            guard.items.push_back(WriterCommand::RunTx {
                criticality,
                task: wrapped,
            });
            let new_depth = guard.items.len();
            guard.telemetry.queue_depth = new_depth;
            if new_depth > guard.telemetry.max_queue_depth_observed {
                guard.telemetry.max_queue_depth_observed = new_depth;
            }
            cv.notify_one();
        }

        Ok(res_rx)
    }

    pub(crate) fn maintenance_tick(&self, truncate: bool) -> Result<WalTelemetry, StoreError> {
        let (tx, rx) = oneshot::channel();
        let (lock, cv) = &*self.queue;
        {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.closed {
                return Err(StoreError::StorageUnavailable(
                    "writer store fermé".to_owned(),
                ));
            }
            guard.items.push_back(WriterCommand::MaintenanceTick {
                truncate,
                reply: tx,
            });
            cv.notify_one();
        }
        rx.blocking_recv()
            .map_err(|_| StoreError::StorageUnavailable("thread writer arrêté".to_owned()))?
    }

    pub(crate) fn telemetry(&self) -> WalTelemetry {
        let now_mono = self.config.monotonic_clock.monotonic_now();
        let (_, _, active_readers, oldest_age) = self.readers.reap_expired_read_only(now_mono);
        let wal = wal_file_bytes(&self.config.db_path);
        let (lock, _) = &*self.queue;
        let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        guard.telemetry.wal_bytes = wal;
        if wal > guard.telemetry.max_wal_bytes_observed {
            guard.telemetry.max_wal_bytes_observed = wal;
        }
        guard.telemetry.active_readers = active_readers;
        guard.telemetry.oldest_reader_age_ms = oldest_age;
        guard.telemetry.clone()
    }

    pub(crate) fn shutdown(&mut self) {
        let (tx, rx) = oneshot::channel();
        let (lock, cv) = &*self.queue;
        {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.closed {
                return;
            }
            guard.closed = true;
            guard.items.push_back(WriterCommand::Shutdown { reply: tx });
            cv.notify_all();
        }
        let _ = rx.blocking_recv();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for StoreWriterHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl ReaderRegistry {
    fn reap_expired_read_only(&self, now_monotonic: Duration) -> (u64, bool, usize, Option<u64>) {
        let guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let active_count = guard.len();
        let oldest_age = guard
            .values()
            .map(|c| {
                u64::try_from(now_monotonic.saturating_sub(c.started_at).as_millis())
                    .unwrap_or(u64::MAX)
            })
            .max();
        (0, false, active_count, oldest_age)
    }
}

fn execute_writer_run_tx(
    conn: &mut Connection,
    config: &StoreConfig,
    readers: &Arc<ReaderRegistry>,
    queue: &Arc<(Mutex<QueueInner>, Condvar)>,
    last_write_mono: &mut Duration,
    criticality: WriteCriticality,
    task: WriterClosure,
) {
    let (lock, _) = &**queue;
    let now_mono = config.monotonic_clock.monotonic_now();
    let mut wal_before =
        effective_wal_pressure_bytes(conn, &config.db_path, config.wal_pressure_bytes);

    // Vérifier pression WAL et inactivité 30 s avant toute écriture normale
    if wal_before >= config.wal_pressure_bytes
        || now_mono.saturating_sub(*last_write_mono) >= config.wal_idle_checkpoint_interval
    {
        run_checkpoint_and_reap(conn, config, readers, queue, false);
        wal_before = lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .telemetry
            .wal_bytes;
    }

    if criticality == WriteCriticality::Normal && wal_before >= config.wal_stop_bytes {
        let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        guard.telemetry.total_rejected_wal_stop += 1;
        guard.telemetry.wal_bytes = wal_before;
        drop(guard);
        tracing::warn!(
            wal_bytes = wal_before,
            limit_bytes = config.wal_stop_bytes,
            "seuil d'arrêt WAL atteint : écriture normale refusée"
        );
        task(Err(StoreError::WalPressureStop {
            wal_bytes: wal_before,
            limit_bytes: config.wal_stop_bytes,
        }));
        return;
    }

    let now_utc = config.clock.now_ms();
    task(Ok((conn, now_utc)));
    *last_write_mono = config.monotonic_clock.monotonic_now();

    let wal_after_raw = wal_file_bytes(&config.db_path);
    let wal_after = if wal_after_raw < config.wal_pressure_bytes {
        wal_after_raw
    } else {
        effective_wal_pressure_bytes(conn, &config.db_path, config.wal_pressure_bytes)
    };
    {
        let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        guard.telemetry.wal_bytes = wal_after;
        if wal_after > guard.telemetry.max_wal_bytes_observed {
            guard.telemetry.max_wal_bytes_observed = wal_after;
        }
    }
    if wal_after >= config.wal_pressure_bytes {
        run_checkpoint_and_reap(conn, config, readers, queue, false);
    }
}

fn writer_thread_loop(
    mut conn: Connection,
    config: &StoreConfig,
    readers: &Arc<ReaderRegistry>,
    queue: &Arc<(Mutex<QueueInner>, Condvar)>,
) {
    let mut last_write_mono = config.monotonic_clock.monotonic_now();
    let (lock, cv) = &**queue;

    loop {
        let cmd = {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            while guard.items.is_empty() {
                if guard.closed {
                    return;
                }
                let idle_elapsed = config
                    .monotonic_clock
                    .monotonic_now()
                    .saturating_sub(last_write_mono);
                if idle_elapsed >= config.wal_idle_checkpoint_interval
                    && wal_file_bytes(&config.db_path) > 0
                {
                    drop(guard);
                    run_checkpoint_and_reap(&conn, config, readers, queue, false);
                    last_write_mono = config.monotonic_clock.monotonic_now();
                    guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                    continue;
                }
                let (next_g, _) = cv
                    .wait_timeout(guard, Duration::from_millis(250))
                    .unwrap_or_else(PoisonError::into_inner);
                guard = next_g;
            }
            let next_cmd = guard.items.pop_front();
            guard.telemetry.queue_depth = guard.items.len();
            next_cmd
        };

        let Some(cmd) = cmd else {
            continue;
        };

        match cmd {
            WriterCommand::Shutdown { reply } => {
                let _ = reply.send(());
                return;
            }
            WriterCommand::MaintenanceTick { truncate, reply } => {
                if let Err(err) =
                    check_injected_fault(config.fault_injector.as_ref(), FaultTarget::Maintenance)
                {
                    tracing::warn!(
                        truncate,
                        error = %err,
                        "panne injectée pendant MaintenanceTick"
                    );
                    let _ = reply.send(Err(err));
                    continue;
                }
                run_checkpoint_and_reap(&conn, config, readers, queue, truncate);
                let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                let _ = reply.send(Ok(guard.telemetry.clone()));
            }
            WriterCommand::RunTx { criticality, task } => execute_writer_run_tx(
                &mut conn,
                config,
                readers,
                queue,
                &mut last_write_mono,
                criticality,
                task,
            ),
        }
    }
}

fn run_wal_checkpoint(conn: &Connection, truncate: bool) -> Option<(i32, i32)> {
    let sql = if truncate {
        "PRAGMA wal_checkpoint(TRUNCATE)"
    } else {
        "PRAGMA wal_checkpoint(PASSIVE)"
    };
    conn.query_row(sql, [], |r| Ok((r.get::<_, i32>(1)?, r.get::<_, i32>(2)?)))
        .ok()
}

fn wal_bytes_from_checkpoint(physical: u64, ckpt_res: Option<(i32, i32)>) -> u64 {
    match ckpt_res {
        Some((n_log, n_ckpt)) if n_ckpt >= n_log => 0,
        _ => physical,
    }
}

fn effective_wal_pressure_bytes(conn: &Connection, db_path: &Path, threshold: u64) -> u64 {
    let physical = wal_file_bytes(db_path);
    if physical < threshold {
        return physical;
    }
    let ckpt = run_wal_checkpoint(conn, false);
    wal_bytes_from_checkpoint(physical, ckpt)
}

fn run_checkpoint_and_reap(
    conn: &Connection,
    config: &StoreConfig,
    readers: &Arc<ReaderRegistry>,
    queue: &Arc<(Mutex<QueueInner>, Condvar)>,
    truncate: bool,
) {
    let now_mono = config.monotonic_clock.monotonic_now();
    let (term, blocked, active_cnt, oldest_age) = readers.reap_expired(now_mono);
    if term > 0 || blocked {
        tracing::warn!(
            expired_readers_terminated = term,
            reader_neutralization_blocked = blocked,
            active_readers = active_cnt,
            "récolte des lecteurs SQLite expirés"
        );
    }

    let ckpt_res = run_wal_checkpoint(conn, truncate);
    let physical = wal_file_bytes(&config.db_path);
    let wal_after = if truncate {
        physical
    } else {
        wal_bytes_from_checkpoint(physical, ckpt_res)
    };

    let (lock, _) = &**queue;
    let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
    guard.telemetry.expired_readers_terminated += term;
    guard.telemetry.reader_neutralization_blocked = blocked;
    guard.telemetry.active_readers = active_cnt;
    guard.telemetry.oldest_reader_age_ms = oldest_age;
    guard.telemetry.wal_bytes = wal_after;
    if wal_after > guard.telemetry.max_wal_bytes_observed {
        guard.telemetry.max_wal_bytes_observed = wal_after;
    }
    if truncate {
        guard.telemetry.truncate_checkpoints += 1;
    } else {
        guard.telemetry.passive_checkpoints += 1;
    }
    if let Some((n_log, n_ckpt)) = ckpt_res {
        guard.telemetry.last_checkpoint_frames = Some((n_log, n_ckpt));
    }
}

/// Exécute une transaction `BEGIN IMMEDIATE` en appliquant la garde de croissance WAL (1 MiB) et les pré-checks.
pub(crate) fn run_guarded_immediate_tx<T, F>(
    conn: &mut Connection,
    config: &StoreConfig,
    criticality: WriteCriticality,
    enforce_tx_wal_cap: bool,
    now_utc: TimestampMs,
    f: F,
) -> Result<T, StoreError>
where
    F: FnOnce(&Transaction<'_>, TimestampMs) -> Result<T, StoreError>,
{
    let wal_pressure = effective_wal_pressure_bytes(conn, &config.db_path, config.wal_stop_bytes);
    if criticality == WriteCriticality::Normal && wal_pressure >= config.wal_stop_bytes {
        return Err(StoreError::WalPressureStop {
            wal_bytes: wal_pressure,
            limit_bytes: config.wal_stop_bytes,
        });
    }
    let wal_before = wal_file_bytes(&config.db_path);

    let page_size: u64 = conn
        .pragma_query_value(None, "page_size", |r| r.get::<_, i64>(0))
        .map(|v| u64::try_from(v).unwrap_or(4096))
        .unwrap_or(4096);
    let page_count_before: u64 = conn
        .pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))
        .map(|v| u64::try_from(v).unwrap_or(0))
        .unwrap_or(0);

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreError::from_rusqlite)?;

    let out = f(&tx, now_utc)?;

    if enforce_tx_wal_cap {
        let page_count_after: u64 = tx
            .pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))
            .map(|v| u64::try_from(v).unwrap_or(0))
            .unwrap_or(page_count_before);
        let page_growth_bytes = page_count_after
            .saturating_sub(page_count_before)
            .saturating_mul(page_size);
        let wal_now = wal_file_bytes(&config.db_path);
        let wal_spill_growth = wal_now.saturating_sub(wal_before);
        let effective_growth = page_growth_bytes.max(wal_spill_growth);
        if effective_growth > config.max_tx_wal_growth_bytes {
            // Rollback automatique en droppant `tx` sans appeler `commit()`
            drop(tx);
            return Err(StoreError::TxWalGrowthExceeded {
                growth_bytes: effective_growth,
                max_bytes: config.max_tx_wal_growth_bytes,
            });
        }
    }

    tx.commit().map_err(StoreError::from_rusqlite)?;
    Ok(out)
}

/// Ouvre une connexion lectrice `query_only=ON` enregistrée auprès du `ReaderRegistry`.
pub(crate) fn open_reader_connection(config: &StoreConfig) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        &config.db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;
    apply_and_verify_reader_pragmas(&conn, config.busy_timeout_ms)?;
    Ok(conn)
}

/// Démarre une session lectrice maintenue ouverte sur un thread dédié (pour tests R6).
pub(crate) fn spawn_held_reader_session(
    config: &StoreConfig,
    readers: &Arc<ReaderRegistry>,
) -> Result<HeldReaderSession, StoreError> {
    let conn = open_reader_connection(config)?;
    let interrupt_handle = conn.get_interrupt_handle();
    let wake_signal = Arc::new((Mutex::new(false), Condvar::new()));
    let started_at = config.monotonic_clock.monotonic_now();
    let control = readers.register(
        started_at,
        config.read_timeout,
        interrupt_handle,
        Some(Arc::clone(&wake_signal)),
    );
    let reader_id = control.reader_id();
    let released_flag = Arc::clone(&control.released);

    // Ouvrir la transaction de lecture AVANT de retourner, afin que le read-mark WAL soit réel immédiatement.
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), StoreError>>(1);
    let readers_for_thread = Arc::clone(readers);
    let ctrl_for_thread = Arc::clone(&control);
    let wake_for_thread = Arc::clone(&wake_signal);

    let join_handle = std::thread::Builder::new()
        .name(format!("unleash-held-reader-{reader_id}"))
        .spawn(move || {
            let start_res = (|| -> Result<(), StoreError> {
                conn.execute_batch("BEGIN DEFERRED;")
                    .map_err(StoreError::from_rusqlite)?;
                let _: i64 = conn
                    .query_row("SELECT COUNT(*) FROM daemon_state", [], |r| r.get(0))
                    .map_err(StoreError::from_rusqlite)?;
                Ok(())
            })();
            match start_res {
                Ok(()) => {
                    let _ = ready_tx.send(Ok(()));
                }
                Err(e) => {
                    readers_for_thread.unregister(reader_id);
                    let _ = ready_tx.send(Err(e));
                    return Ok(());
                }
            }

            {
                let (wlock, wcv) = &*wake_for_thread;
                let mut flag = wlock.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    while !*flag {
                        flag = wcv.wait(flag).unwrap_or_else(PoisonError::into_inner);
                    }
                    if !ctrl_for_thread.block_neutralization.load(Ordering::Acquire) {
                        break;
                    }
                    *flag = false;
                }
            }

            let _ = conn.execute_batch("ROLLBACK;");
            drop(conn);
            readers_for_thread.unregister(reader_id);
            Ok(())
        })
        .map_err(StoreError::Io)?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(HeldReaderSession {
            reader_id,
            control,
            released: released_flag,
            wake_signal,
            join_handle: Some(join_handle),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(StoreError::StorageUnavailable(
            "échec démarrage thread lecteur".to_owned(),
        )),
    }
}
