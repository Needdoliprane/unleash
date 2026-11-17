//! Configuration de `unleash-store`, horloges, sondes disque et hooks déterministes d'injection.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use unleash_domain::{ClockPort, IdGeneratorPort, TimestampMs};
use uuid::Uuid;

use crate::error::StoreError;
use crate::qualification::SqliteBuildPolicy;

/// Capacité maximale par défaut de la file d'attente du writer unique (ADR-01).
pub const DEFAULT_WRITER_QUEUE_CAPACITY: usize = 8;

/// Durée maximale par défaut d'une transaction de lecture (5 secondes, ADR-01).
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Seuil de déclenchement du checkpoint `PASSIVE` WAL (16 MiB, ADR-01).
pub const DEFAULT_WAL_PRESSURE_BYTES: u64 = 16 * 1024 * 1024;

/// Seuil d'arrêt des écritures non critiques WAL (32 MiB, ADR-01).
pub const DEFAULT_WAL_STOP_BYTES: u64 = 32 * 1024 * 1024;

/// Délai d'inactivité du writer avant checkpoint `PASSIVE` (30 s, ADR-01).
pub const DEFAULT_WAL_IDLE_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30);

/// Croissance WAL maximale autorisée pour une seule transaction (1 MiB, ADR-01).
pub const DEFAULT_MAX_TX_WAL_GROWTH_BYTES: u64 = 1024 * 1024;

/// Quota maximal d'export par défaut (8 MiB, tests F002 / R6).
pub const DEFAULT_EXPORT_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Taille maximale d'un fragment d'export en continu (256 KiB, tests F002 / R6).
pub const DEFAULT_EXPORT_CHUNK_BYTES: usize = 256 * 1024;

/// Seuil critique d'espace disque libre sous lequel les nouvelles admissions sont refusées (1 GiB, Q-12).
pub const DEFAULT_MIN_FREE_SPACE_BYTES: u64 = 1024 * 1024 * 1024;

/// Délai par défaut de conservation complète des réponses d'idempotence (7 jours, Q-12).
pub const DEFAULT_RECEIPT_RETENTION_DAYS: u32 = 7;

/// Durée minimale de conservation de l'inbox après `acked` et des événements pour `SubscribeEvents` (7 jours, ADR-02).
pub const EVENT_AND_INBOX_MIN_RETENTION_MS: i64 = 7 * 86_400_000;

/// Port d'horloge monotone injectable pour mesurer les délais (lecteurs 5 s, inactivité 30 s) sans `sleep`.
pub trait MonotonicClockPort: Send + Sync {
    /// Renvoie le temps monotone écoulé depuis un point de référence arbitraire.
    fn monotonic_now(&self) -> Duration;
}

/// Implémentation système de [`MonotonicClockPort`] fondée sur `std::time::Instant`.
#[derive(Debug)]
pub struct SystemMonotonicClock {
    origin: Instant,
}

impl SystemMonotonicClock {
    /// Crée une horloge monotone ancrée à l'instant courant.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemMonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClockPort for SystemMonotonicClock {
    fn monotonic_now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Implémentation système par défaut de [`ClockPort`] (UTC murale).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemUtcClock;

impl ClockPort for SystemUtcClock {
    fn now_ms(&self) -> TimestampMs {
        TimestampMs::new(chrono::Utc::now().timestamp_millis())
    }
}

/// Générateur d'identifiants UUID v4/aléatoires sans dépendance externe non autorisée.
#[derive(Debug)]
pub struct SequentialUuidGenerator {
    prefix: u64,
    counter: AtomicU64,
}

impl SequentialUuidGenerator {
    /// Initialise le générateur avec un préfixe.
    #[must_use]
    pub fn new(prefix: u64) -> Self {
        Self {
            prefix,
            counter: AtomicU64::new(1),
        }
    }
}

impl Default for SequentialUuidGenerator {
    fn default() -> Self {
        let nanos =
            u64::try_from(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(1)).unwrap_or(1);
        Self::new(nanos)
    }
}

impl IdGeneratorPort for SequentialUuidGenerator {
    fn new_uuid(&self) -> Uuid {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&self.prefix.to_be_bytes());
        bytes[8..16].copy_from_slice(&seq.to_be_bytes());
        uuid::Builder::from_random_bytes(bytes).into_uuid()
    }
}

/// Sonde d'espace disque libre disponible sur la partition accueillant la base.
pub trait DiskSpaceProbe: Send + Sync {
    /// Renvoie le nombre d'octets libres estimés pour `path` (`None` si indisponible).
    fn available_bytes(&self, path: &Path) -> Option<u64>;
}

/// Sonde système utilisant `df -kP` (sans `unsafe`), exécutée hors transaction SQL.
#[derive(Debug, Default, Clone, Copy)]
pub struct DfDiskSpaceProbe;

impl DiskSpaceProbe for DfDiskSpaceProbe {
    fn available_bytes(&self, path: &Path) -> Option<u64> {
        let target = if path.exists() { path } else { path.parent()? };
        let output = Command::new("df").arg("-kP").arg(target).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let data_line = text.lines().nth(1)?;
        let cols: Vec<&str> = data_line.split_whitespace().collect();
        let avail_kb = cols.get(3)?.parse::<u64>().ok()?;
        avail_kb.checked_mul(1024)
    }
}

/// Sonde d'espace disque contrôlable en test.
#[derive(Debug)]
pub struct FixedDiskSpaceProbe {
    bytes: AtomicU64,
}

impl FixedDiskSpaceProbe {
    /// Crée une sonde renvoyant `initial_bytes`.
    #[must_use]
    pub const fn new(initial_bytes: u64) -> Self {
        Self {
            bytes: AtomicU64::new(initial_bytes),
        }
    }

    /// Met à jour l'espace disponible simulé.
    pub fn set_bytes(&self, bytes: u64) {
        self.bytes.store(bytes, Ordering::SeqCst);
    }
}

impl DiskSpaceProbe for FixedDiskSpaceProbe {
    fn available_bytes(&self, _path: &Path) -> Option<u64> {
        Some(self.bytes.load(Ordering::SeqCst))
    }
}

/// Points de coupure observables dans `unleash-store`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreCutpoint {
    /// Avant d'ouvrir la transaction de commande (`C0`).
    C0BeforeCommandTx,
    /// Dans la transaction de commande, juste avant `COMMIT` (`C1`).
    C1BeforeCommandCommit,
    /// Juste après `COMMIT` de commande, avant retour au client (`C2`).
    C2AfterCommandCommit,
    /// Juste après `COMMIT` du pin de préparation objet (`C10a`).
    C10aAfterPrepPin,
    /// Lors de la sélection/réservation de suppression objet (`C10b`).
    C10bBeforeDeletingReserve,
    /// Après intention durable `deleting/prepared`, avant `unlink` (`C10c`).
    C10cAfterDeletingIntent,
    /// Après `unlink` et `sync_dir`, avant `tombstone/completed` (`C10d`).
    C10dAfterUnlinkSyncBeforeTombstone,
    /// Pendant une migration de schéma (`C11`).
    C11MigrationInProgress,
    /// Après acquisition de `backup_barrier` durable (`C11a`).
    C11aAfterBackupBarrier,
    /// Après snapshot DB cohérent, avant acquisition des pins d'objets (`C11b`).
    C11bAfterDbSnapshotBeforePins,
    /// Après acquisition des pins d'objets, avant/pendant copie (`C11c`).
    C11cAfterPinsDuringCopy,
    /// Après copie et synchronisation du manifeste, avant `complete` (`C11d`).
    C11dAfterManifestBeforeComplete,
    /// Pendant la restauration entre import capsule et activation finale (`C11`).
    C11RestoreBeforeActivate,
    /// Juste après `ApplyInboxEvent` avant acquittement outbox (`C7`).
    C7AfterInboxBeforeOutboxAck,
}

/// Observateur injectable des points de coupure (implémenté en test avec `CutpointController`).
pub trait CutpointObserver: Send + Sync {
    /// Notifie le passage au point de coupure ; peut injecter une erreur ou bloquer sur une barrière.
    ///
    /// # Errors
    /// Retourne [`StoreError::CutpointInjected`] si le point de coupure injecte une panne.
    fn hit(&self, cutpoint: StoreCutpoint) -> Result<(), StoreError>;
}

/// Observateur neutre sans effet.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopCutpointObserver;

impl CutpointObserver for NoopCutpointObserver {
    fn hit(&self, _cutpoint: StoreCutpoint) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Type de panne `SQLite` basse couche injectable en test (`T-F002-007`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedStorageFault {
    /// Simule `SQLITE_BUSY`.
    Busy,
    /// Simule `SQLITE_FULL`.
    Full,
    /// Simule `SQLITE_IOERR`.
    IoErr,
    /// Simule `SQLITE_CORRUPT`.
    Corrupt,
}

/// Endroit où une panne I/O simulée peut frapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultTarget {
    /// Pendant le commit ou l'écriture d'une commande.
    CommandWrite,
    /// Pendant la purge ou checkpoint WAL.
    Maintenance,
    /// Pendant l'unlink d'un objet.
    ObjectUnlink,
}

/// Injecteur de pannes de stockage pour les tests de robustesse.
pub trait StorageFaultInjector: Send + Sync {
    /// Retourne `Some(fault)` si une panne doit être injectée au point `target`.
    fn next_fault(&self, target: FaultTarget) -> Option<InjectedStorageFault>;
}

/// Injecteur par défaut qui n'injecte aucune panne.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopFaultInjector;

impl StorageFaultInjector for NoopFaultInjector {
    fn next_fault(&self, _target: FaultTarget) -> Option<InjectedStorageFault> {
        None
    }
}

/// Évalue l'injecteur de pannes pour la cible donnée et convertit une panne éventuelle en [`StoreError`].
pub(crate) fn check_injected_fault(
    injector: &dyn StorageFaultInjector,
    target: FaultTarget,
) -> Result<(), StoreError> {
    if let Some(fault) = injector.next_fault(target) {
        return Err(match fault {
            InjectedStorageFault::Busy => {
                StoreError::StorageBusy("panne injectée SQLITE_BUSY".to_owned())
            }
            InjectedStorageFault::Full => {
                StoreError::StorageFull("panne injectée SQLITE_FULL".to_owned())
            }
            InjectedStorageFault::IoErr => {
                StoreError::StorageUnavailable("panne injectée SQLITE_IOERR".to_owned())
            }
            InjectedStorageFault::Corrupt => {
                StoreError::StorageCorrupt("panne injectée SQLITE_CORRUPT".to_owned())
            }
        });
    }
    Ok(())
}

/// Détermine un `owner_uid` par défaut sans bloc `unsafe` depuis l'exécutable courant sur Unix.
fn detect_default_owner_uid() -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::env::current_exe()
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .map_or(501, |m| m.uid())
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// Criticité d'une écriture soumise au writer unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCriticality {
    /// Écriture métier ordinaire (refusée à partir de 32 MiB de WAL).
    Normal,
    /// Écriture critique (récupération, libération, checkpoint, purge/tombstone) admise même sous pression WAL.
    Critical,
}

/// Configuration complète d'une instance `UnleashStore`.
#[derive(Clone)]
pub struct StoreConfig {
    /// Racine de données locale (contient la base `SQLite`, `artifacts/`, `backups/`, `maintenance/`).
    pub data_root: PathBuf,
    /// Chemin exact du fichier `SQLite` d'autorité.
    pub db_path: PathBuf,
    /// Répertoire des objets adressés par contenu (`artifacts/`).
    pub artifacts_dir: PathBuf,
    /// Répertoire des sauvegardes gérées (`backups/`).
    pub backups_dir: PathBuf,
    /// Répertoire des manifests privés de maintenance (`maintenance/`).
    pub maintenance_dir: PathBuf,
    /// Identifiant du propriétaire OS (UID effectif).
    pub owner_uid: u32,
    /// Identité de boot observée.
    pub boot_identity: String,
    /// Version applicative du consommateur outbox (pour réarmement `dead_letter`).
    pub consumer_version: String,
    /// Délai `busy_timeout` en ms (250 par défaut).
    pub busy_timeout_ms: u32,
    /// Capacité de la file d'attente du writer unique (8 par défaut).
    pub writer_queue_capacity: usize,
    /// Durée maximale d'une transaction de lecture (5 s par défaut).
    pub read_timeout: Duration,
    /// Seuil WAL de déclenchement du checkpoint `PASSIVE` (16 MiB par défaut).
    pub wal_pressure_bytes: u64,
    /// Seuil WAL de refus des écritures non critiques (32 MiB par défaut).
    pub wal_stop_bytes: u64,
    /// Intervalle d'inactivité déclenchant un checkpoint `PASSIVE` (30 s par défaut).
    pub wal_idle_checkpoint_interval: Duration,
    /// Croissance WAL maximale par transaction (1 MiB par défaut).
    pub max_tx_wal_growth_bytes: u64,
    /// Quota d'octets d'export (8 MiB par défaut).
    pub export_max_bytes: u64,
    /// Taille de fragment d'export (256 KiB par défaut).
    pub export_chunk_bytes: usize,
    /// Espace libre minimum requis pour admettre de nouvelles intentions (1 GiB par défaut).
    pub min_free_space_bytes: u64,
    /// Durée par défaut de conservation des réponses complètes de commandes (1..=90 jours, défaut 7).
    pub default_receipt_retention_days: u32,
    /// Horloge incertaine suspendant les purges automatiques.
    pub clock_uncertain: Arc<AtomicBool>,
    /// Version maximale de schéma supportée par ce binaire (pour tester les anciens binaires).
    pub max_supported_schema_version: i64,
    /// Politique de qualification du build `SQLite`.
    pub build_policy: SqliteBuildPolicy,
    /// Horloge UTC injectée.
    pub clock: Arc<dyn ClockPort>,
    /// Horloge monotone injectée.
    pub monotonic_clock: Arc<dyn MonotonicClockPort>,
    /// Générateur d'identifiants injecté.
    pub id_generator: Arc<dyn IdGeneratorPort>,
    /// Sonde d'espace disque injectée.
    pub disk_space_probe: Arc<dyn DiskSpaceProbe>,
    /// Observateur de cutpoints injecté.
    pub cutpoints: Arc<dyn CutpointObserver>,
    /// Injecteur de pannes de stockage injecté.
    pub fault_injector: Arc<dyn StorageFaultInjector>,
}

impl std::fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreConfig")
            .field("data_root", &self.data_root)
            .field("db_path", &self.db_path)
            .field("writer_queue_capacity", &self.writer_queue_capacity)
            .field("wal_pressure_bytes", &self.wal_pressure_bytes)
            .field("wal_stop_bytes", &self.wal_stop_bytes)
            .field("max_tx_wal_growth_bytes", &self.max_tx_wal_growth_bytes)
            .field("export_max_bytes", &self.export_max_bytes)
            .field(
                "max_supported_schema_version",
                &self.max_supported_schema_version,
            )
            .finish_non_exhaustive()
    }
}

impl StoreConfig {
    /// Construit une configuration avec les valeurs par défaut d'ADR-01/ADR-02/Q-12 sous `data_root`.
    #[must_use]
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        let root = data_root.as_ref().to_path_buf();
        Self {
            db_path: root.join("unleash-authority.db"),
            artifacts_dir: root.join("artifacts"),
            backups_dir: root.join("backups"),
            maintenance_dir: root.join("maintenance"),
            data_root: root,
            owner_uid: detect_default_owner_uid(),
            boot_identity: "boot-default".to_owned(),
            consumer_version: "0.1.0".to_owned(),
            busy_timeout_ms: 250,
            writer_queue_capacity: DEFAULT_WRITER_QUEUE_CAPACITY,
            read_timeout: DEFAULT_READ_TIMEOUT,
            wal_pressure_bytes: DEFAULT_WAL_PRESSURE_BYTES,
            wal_stop_bytes: DEFAULT_WAL_STOP_BYTES,
            wal_idle_checkpoint_interval: DEFAULT_WAL_IDLE_CHECKPOINT_INTERVAL,
            max_tx_wal_growth_bytes: DEFAULT_MAX_TX_WAL_GROWTH_BYTES,
            export_max_bytes: DEFAULT_EXPORT_MAX_BYTES,
            export_chunk_bytes: DEFAULT_EXPORT_CHUNK_BYTES,
            min_free_space_bytes: DEFAULT_MIN_FREE_SPACE_BYTES,
            default_receipt_retention_days: DEFAULT_RECEIPT_RETENTION_DAYS,
            clock_uncertain: Arc::new(AtomicBool::new(false)),
            max_supported_schema_version: 2,
            build_policy: SqliteBuildPolicy::default(),
            clock: Arc::new(SystemUtcClock),
            monotonic_clock: Arc::new(SystemMonotonicClock::new()),
            id_generator: Arc::new(SequentialUuidGenerator::default()),
            disk_space_probe: Arc::new(DfDiskSpaceProbe),
            cutpoints: Arc::new(NoopCutpointObserver),
            fault_injector: Arc::new(NoopFaultInjector),
        }
    }
}
