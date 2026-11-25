// SPDX-License-Identifier: Apache-2.0
//! Configuration du daemon Unleash et points d'injection déterministes pour les tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use unleash_browser::LocalInventorySearchPaths;
use unleash_domain::{ClockPort, IdGeneratorPort};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    CutpointObserver, DEFAULT_WRITER_QUEUE_CAPACITY, DiskSpaceProbe, MonotonicClockPort,
    SqliteBuildPolicy, StorageFaultInjector,
};

use crate::auth::{AuthVerifierPort, PeerUidAuthVerifier};
use crate::executors::{ExecutorDrainPort, RendezvousExecutorController};
use crate::recovery::RecoveryGate;
use crate::terminal_host_launcher::{PackagedTerminalHostLauncher, TerminalHostLauncher};

/// Action déclenchée juste après le commit durable d'une commande et avant l'envoi de la réponse IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostCommitAction {
    /// Poursuit normalement l'envoi du reçu sur le socket.
    Continue,
    /// Ferme immédiatement la connexion sans envoyer le reçu (`T-F001-005`).
    DropConnection,
}

/// Hook injectable après commit pour tester la perte de transport après commit (`T-F001-005`).
pub trait PostCommitHook: Send + Sync {
    /// Évalue l'action à exécuter après commit de `envelope`.
    fn after_commit(&self, envelope: &CommandEnvelope) -> PostCommitAction;
}

/// Surcharges optionnelles passées à `StoreConfig` (tests et qualification).
#[derive(Clone, Default)]
pub struct StoreOverrides {
    /// Politique de qualification `SQLite` (`source_id`).
    pub build_policy: Option<SqliteBuildPolicy>,
    /// Capacité de la file du writer (`8` par défaut).
    pub writer_queue_capacity: Option<usize>,
    /// Horloge UTC injectée.
    pub clock: Option<Arc<dyn ClockPort>>,
    /// Horloge monotone injectée.
    pub monotonic_clock: Option<Arc<dyn MonotonicClockPort>>,
    /// Générateur d'identifiants injecté.
    pub id_generator: Option<Arc<dyn IdGeneratorPort>>,
    /// Observateur de cutpoints injecté.
    pub cutpoints: Option<Arc<dyn CutpointObserver>>,
    /// Injecteur de pannes de stockage.
    pub fault_injector: Option<Arc<dyn StorageFaultInjector>>,
    /// Sonde d'espace disque injectée.
    pub disk_space_probe: Option<Arc<dyn DiskSpaceProbe>>,
}

impl std::fmt::Debug for StoreOverrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreOverrides")
            .field("build_policy", &self.build_policy)
            .field("writer_queue_capacity", &self.writer_queue_capacity)
            .finish_non_exhaustive()
    }
}

/// Configuration complète d'une instance [`crate::UnleashDaemon`].
#[derive(Clone)]
pub struct DaemonConfig {
    /// Racine de données explicite (défaut : `PlatformPaths::resolve()?.data_dir`).
    pub data_root: Option<PathBuf>,
    /// Répertoire de rendez-vous explicite, créateur (tests et isolement). Par défaut, le
    /// rendez-vous canonique de l'UID effectif, préparé par `prepare_canonical_rendezvous_dir()` :
    /// sous Linux, sa racine runtime doit préexister, sinon le démarrage échoue avant toute création.
    pub rendezvous_dir: Option<PathBuf>,
    /// Version du consommateur outbox (`dead_letter` rearm).
    pub consumer_version: String,
    /// Nombre maximal de connexions IPC simultanées.
    pub max_connections: usize,
    /// Nombre maximal de mutations en vol simultanées côté serveur IPC.
    pub max_in_flight_mutations: usize,
    /// Capacité de la file bornée de chaque abonné `SubscribeEvents`.
    pub subscriber_queue_capacity: usize,
    /// Délai maximal d'écriture d'une trame vers un abonné lent.
    pub subscriber_write_timeout: Duration,
    /// Délai maximal accordé au drainage des exécuteurs lors de l'arrêt.
    pub shutdown_drain_timeout: Duration,
    /// Si vrai, un démarrage sur la même racine rejoint l'instance active via `Hello`.
    pub attach_if_running: bool,
    /// Surcharges de stockage (horloge, cutpoints, politique `SQLite`).
    pub store_overrides: StoreOverrides,
    /// Contrôleur des exécuteurs persistants et du drainage.
    pub executor_controller: Arc<dyn ExecutorDrainPort>,
    /// Trusted Rust launch port, invoked only after the durable effect permit.
    /// It has no IPC or environment-controlled selection path.
    pub terminal_host_launcher: Arc<dyn TerminalHostLauncher>,
    /// Vérificateur d'authentification des pairs.
    pub auth_verifier: Arc<dyn AuthVerifierPort>,
    /// Barrière optionnelle de test pour observer le mode `recovering`.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Hook optionnel post-commit pour tester la coupure réseau après commit.
    pub post_commit_hook: Option<Arc<dyn PostCommitHook>>,
    /// Chemins passifs d'inventaire des runtimes navigateur pour `GetHealth`.
    pub browser_search_paths: Option<LocalInventorySearchPaths>,
}

impl std::fmt::Debug for DaemonConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonConfig")
            .field("data_root", &self.data_root)
            .field("rendezvous_dir", &self.rendezvous_dir)
            .field("consumer_version", &self.consumer_version)
            .field("max_connections", &self.max_connections)
            .field("max_in_flight_mutations", &self.max_in_flight_mutations)
            .field("subscriber_queue_capacity", &self.subscriber_queue_capacity)
            .finish_non_exhaustive()
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            data_root: None,
            rendezvous_dir: None,
            consumer_version: env!("CARGO_PKG_VERSION").to_owned(),
            max_connections: 64,
            max_in_flight_mutations: DEFAULT_WRITER_QUEUE_CAPACITY * 2,
            subscriber_queue_capacity: 32,
            subscriber_write_timeout: Duration::from_secs(2),
            shutdown_drain_timeout: Duration::from_secs(5),
            attach_if_running: false,
            store_overrides: StoreOverrides::default(),
            executor_controller: Arc::new(RendezvousExecutorController),
            terminal_host_launcher: Arc::new(PackagedTerminalHostLauncher),
            auth_verifier: Arc::new(PeerUidAuthVerifier),
            recovery_gate: None,
            post_commit_hook: None,
            browser_search_paths: None,
        }
    }
}
