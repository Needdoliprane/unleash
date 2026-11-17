//! # spike-m0-04-contracts-knowledge
//!
//! Expérience M0 n°4 (tâche `t08`, `REQ-CONTRACT-004` / `REQ-KNOW-003`) :
//! validation expérimentale hors workspace des contrats observables (`ExpectedTestExecution`,
//! `ExpectedExecutionBinding`, `TestExecutionObservation`, `ContractClaim`) et de la
//! découverte multilangage avec dégradation explicite sans fabrication de vérité.
//!
//! ## Dépendances admises
//! - `unleash-domain` (ports `ClockPort`, `IdGeneratorPort`, `Sha256Digest`, identifiants).
//! - `unleash-protocol` (`ReasonCode`, canonisation RFC 8785 / `canonical_sha256`).
//! - `unleash-testkit` (`ManualClock`, `DeterministicIdGenerator`, `local_test_dir`).
//! - Bibliothèques standard du workspace (`serde`, `serde_json`, `jsonschema`, `referencing`,
//!   `sha2`, `hex`, `chrono`, `thiserror`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod contracts;
pub mod errors;
pub mod knowledge;
pub mod schema_registry;
pub mod types;

pub use contracts::{
    ClaimReconciliationRecord, ContractVerdict, ObservationRecord, OnUnknownPolicy,
    ParsedTestReport, RuleOutcome, RuleVerdict, SealBindingRequest, SealedActionJournal,
    VerdictStatus, evaluate_contract_verdict, parse_cargo_test_text, parse_junit_xml,
    validate_claim_predicate_matches, validate_git_snapshot_matches,
    validate_test_execution_matches,
};
pub use errors::SpikeError;
pub use knowledge::{
    AnalyzerProbeConfig, AnalyzerQualification, CapturedSnapshot, CodeState, DiscoveredRelation,
    DiscoveredSymbol, LanguageProfile, MultilanguageDiscoveryOutput, OverlayEntry, OverlayManifest,
    ProfileAvailability, RelationConfidence, capture_git_snapshot_with_hook,
    discover_multilanguage_repo, probe_analyzer,
};
pub use schema_registry::{SchemaKind, SchemaRegistry};
pub use types::{
    ActivationRef, AuthorityKind, AuthorityRef, CaptureConsistency, ClaimProposition,
    ClosedPredicate, CommandSpec, ContentDigest, ContractClaim, DiscoveryCause, DiscoveryCauseKind,
    DiscoveryDecision, DiscoveryOrigin, DiscoveryPurpose, DiscoveryReceipt, DiscoveryStrategy,
    EventCursorRef, ExactRef, ExecutionResult, ExpectedExecutionBinding, ExpectedTestExecution,
    Freshness, HexSha256, ParseState, ProcessState, RegistryKind, RegistryNamespace, ResultPolicy,
    SnapshotKind, SnapshotRef, SubjectRef, TemporalRelation, TestCasesSummary,
    TestExecutionObservation, ToolOrParserRef, TypedSourceRef, ValidationBasis,
    timestamp_ms_to_utc,
};
