//! # unleash-browser
//!
//! Responsabilité exclusive : Implémentation du port interne [`BrowserPort`] en `WebDriver` W3C
//! via `fantoccini` pour le pilote de référence V1 (`Chrome for Testing` + `chromedriver`, Q-18),
//! vérification cryptographique `SHA-256` et de version du runtime épinglé, contrôle strict de la
//! matrice V1 des locators (`RoleName`, `Label`, `TestId`, `Css`) et de cardinalité/identité,
//! proxy d'egress HTTP local par mandat ([`EgressProxy`]) et enforcement anti-modèle sous
//! `model_policy = forbidden`.
//!
//! ## Dépendances admises
//! - `unleash-domain` et `unleash-protocol` (identités, fences, politiques, erreurs IPC).
//! - `fantoccini` (`default-features = false` sur canal loopback HTTP sans TLS fictif), `tokio`,
//!   `hyper`, `hyper-util`, `http`, `http-body-util`, `bytes`, `url`, `serde`, `serde_json`,
//!   `thiserror`, `sha2`, `hex`, `tracing`, `uuid`.
//! - Aucun appel à un modèle LLM, aucun téléchargement implicite de binaire, aucune exécution de
//!   script fourni par un modèle.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod adapter;
pub mod egress;
pub mod error;
pub mod locator;
pub mod policy;
pub mod port;
pub mod runtime;
pub mod types;
pub mod webdriver_ext;

pub use adapter::{FantocciniBrowserAdapter, MonotonicIdPort, SystemClockPort};
pub use egress::{EgressAuditEvent, EgressPolicy, EgressProxy, normalize_origin};
pub use error::BrowserError;
pub use locator::{
    ExpectedTargetIdentity, LocatorStrategy, RawLocatorRequest, ResolvedTarget, TargetSpec,
    enforce_unique_candidate, escape_css_attr_value, verify_target_identity,
};
pub use policy::{
    ConfinementProof, DeniedModelAttempt, ExecutionMode, IsolationLevel, ModelAttemptKind,
    ModelPolicyEnforcer, ScriptOrigin, ZeroModelProofReport, check_execution_isolation,
};
pub use port::{BoxFuture, BrowserPort};
pub use runtime::{
    BrowserEngineKind, DetectedBrowserArtifact, LocalInventorySearchPaths, LocalRuntimeInventory,
    PinnedBinary, PinnedBrowserRuntime, RuntimeInspection, TargetPlatform, compute_file_sha256,
    inventory_local_runtimes, verify_pinned_runtime,
};
pub use types::{
    ActionOutcome, AssertionEvaluationReport, AssertionRuleResult, AssertionVerdict,
    BrowserActionIntent, BrowserActionKind, BrowserCheckpoint, BrowserContextHandle,
    BusinessAssertion, CheckpointInput, CloseContextResult, CloseOutcome, CloseReason,
    EffectJournalEntry, EffectJournalPort, EffectState, InMemoryEffectJournal, ObserveScope,
    OpenContextRequest, PageObservation,
};
pub use webdriver_ext::{
    ADAPTER_OBSERVE_SCRIPT_VERSION, GetComputedLabel, GetComputedRole, WebDriverStatusCommand,
};
