//! # unleash-testkit
//!
//! Responsabilité exclusive : Harnais de test déterministe pour le workspace OVPT Unleash.
//! Fournit des adaptateurs injectables sans I/O non maîtrisée, une matrice complète de points
//! d'injection de pannes (`C0`–`C12`), un oracle d'effets externes sur disque local, un harnais
//! de crash par processus enfant sans `sleep` de synchronisation, une garde stricte pour les
//! tests payants en développement et la création de répertoires temporaires compatibles `SUN_LEN`.
//!
//! ## Dépendances admises
//! - `unleash-domain` (ports `ClockPort`, `IdGeneratorPort` et types temps/identifiants).
//! - Bibliothèques standard et utilitaires de test (`tempfile`, `dotenvy`, `secrecy`, `zeroize`,
//!   `serde`, `serde_json`, `thiserror`, `uuid`).
//! - Aucun appel réseau ni effet réel par défaut.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod crash_harness;
pub mod cutpoints;
pub mod guard_double;
pub mod ids;
pub mod live_guard;
pub mod temp;

pub use clock::{ClockWakeup, ManualClock};
pub use crash_harness::{
    ChildCrashBridge, ChildCrashContext, ChildCrashHarness, ChildCrashOutcome, ChildCrashSpec,
    CrashHarnessError, CrashKillMode, ExternalEffectOracle, ExternalEffectRecord,
    NewExternalEffect,
};
pub use cutpoints::{Cutpoint, CutpointAction, CutpointBarrier, CutpointController, CutpointError};
pub use guard_double::{
    BrokerExecutionSpy, DeterministicGuardDouble, DispatchedActionRecord, InMemoryTestSecretStore,
    ManualClockTimer, ScriptedGuardOutcome, SecretResolutionSpy, SimulatedExecutionFailure,
    TestIsolationProbe, assert_refused_with_zero_side_effects,
};
pub use ids::{DeterministicIdGenerator, IdGeneratorError, SeedDiagnosticGuard};
pub use live_guard::{
    LIVE_MODEL_ALLOWLIST, LiveCallPermit, LiveGuardError, LiveModel, LiveProvider, LiveTestGuard,
};
pub use temp::{
    MACOS_SUN_LEN, MAX_TEMP_DIR_PATH_BYTES, MAX_UNIX_SOCKET_PATH_BYTES, TempDirError,
    local_test_dir, private_test_dir, socket_path_in,
};
