//! # unleash-domain
//!
//! Responsabilité exclusive : Cœur fonctionnel du domaine d'orchestration Unleash.
//! Contient uniquement des structures de données en mémoire, des identifiants newtypes
//! fortement typés, des compteurs et barrières ordonnées, des ports abstraits (traits),
//! ainsi que des machines à états exhaustives et leurs transitions pures.
//!
//! ## Invariants architecturaux
//! - **Zéro I/O :** Aucune dépendance vers `tokio`, `rusqlite`, `reqwest`, `std::fs`,
//!   ni aucune I/O réseau ou système.
//! - **Déterminisme pur :** Pas d'appel direct à l'horloge murale système (`std::time::SystemTime::now`)
//!   ni d'aléa implicite (`thread_rng`). Les horloges et générateurs d'identifiants
//!   sont exclusivement injectés via les traits du module [`ports`].
//! - **Intégrité et sécurité :** `#![forbid(unsafe_code)]` strict.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod errors;
pub mod fences;
pub mod ids;
pub mod ports;
pub mod states;

pub use errors::{DomainTransitionError, FenceError, IdValidationError};
pub use fences::{
    DaemonEpoch, Fence, LineageFence, LineageToken, MicrosAmount, Revision, Sequence, TimestampMs,
};
pub use ids::{
    AccountId, ActionId, ActorId, AgentId, ApprovalScopeId, AttemptId, AuthorityGeneration,
    BackupId, CommandId, DaemonInstanceId, DiscoveryReceiptId, DispatchId, DraftId, EvidenceId,
    GoalId, HostId, InvocationId, IterationId, LeaseId, ManifestId, MissionId, ModelCallId,
    OperationId, PoolId, ProcessId, ProcessRunUuid, ProjectId, ProviderProbeId, ReceiptId,
    RegistryVersionId, ReplayWorkerId, RepoId, RequestId, ReservationId, RestoreId, RunId,
    SecretRef, SessionId, Sha256Digest, StepId, StreamId, TaskId, TerminalId, WorktreeId,
};
pub use ports::{ClockPort, IdGeneratorPort};
pub use states::*;
