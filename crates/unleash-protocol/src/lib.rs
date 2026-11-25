//! # unleash-protocol
//!
//! Responsabilité exclusive : Types de données, DTOs normatifs versionnés (`unleash.ipc.v2`),
//! canonisation RFC 8785 (JCS), codec de trames Unix et types TypeScript pour l'API IPC locale d'Unleash.
//!
//! ## Invariants architecturaux
//! - `#![forbid(unsafe_code)]` strict.
//! - Zéro I/O réseau ou base de données (`tokio`, `rusqlite`, `reqwest` interdits).
//! - Décodage JSON strict rejetant toute clé dupliquée (à tout niveau d'imbrication) avant Serde.
//! - Tous les DTOs ont `#[serde(deny_unknown_fields)]`.
//! - Canonisation RFC 8785 déterministe avec calcul de digest SHA-256 (`sha256:<hex64>`).
//! - Codec de trames Unix (big-endian `u32` + JSON UTF-8) borné au plafond strict de 8 MiB (`8_388_608` octets).
//! - Catalogue exhaustif des 204 opérations en bijection exacte avec la conception.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod canonical;
pub mod codec;
pub mod envelopes;
pub mod errors;
pub mod mission_scope;
pub mod operations;
pub mod provider_inventory;
pub mod provider_probe;
pub mod provider_profile;
pub mod strict_json;
pub mod ts_export;
pub mod workflow_run;
pub mod worktrees;

pub use canonical::{
    canonical_sha256, canonicalize_to_string, canonicalize_to_vec, compute_command_digest_v2,
};
pub use codec::{FrameBuffer, MAX_FRAME_BYTES, read_frame, write_frame};
pub use envelopes::{
    AdminProof, AssignTaskPayload, ClientKind, ClientUploadSegment, CommandEnvelope,
    CommandReceipt, CommandReceiptInspection, DiagnosticHelloResponse, DiagnosticMode,
    DiagnosticReason, EventCausation, EventCausationKind, EventCursor, EventEnvelope, HelloRequest,
    HelloResponse, LineageStatus, MAX_CLIENT_UPLOAD_BYTES, PROTOCOL_VERSION_V1,
    PROTOCOL_VERSION_V2, ProtocolVersion, QueryEnvelope, QueryResponse,
    READ_ONLY_RECOVERY_CAPABILITY, ReceiptState, Sensitivity, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
pub use errors::{
    ErrorCode, IpcError, ProtocolError, ReasonCode, RetryClass, UnknownReasonCodeError,
};
pub use mission_scope::MissionScopeSelector;
pub use operations::{OperationKind, OperationName, UnknownOperationError};
pub use provider_probe::{
    ProviderTestLimits, ProviderTestPayloadError, ProviderTestPhase, TestProviderProfilePayload,
};
pub use provider_profile::{
    InlineProviderPrice, ProviderAuthentication, ProviderDialect, ProviderDialectKind,
    ProviderKind, ProviderPricing, ProviderPricingStalePolicy, ProviderProfileLimits,
    ProviderProfilePayloadError, ProviderServerManagement, SetProviderProfilePayload,
};
pub use strict_json::{from_json_str_strict, parse_json_value_strict};
pub use ts_export::{generate_all_ts_files, write_ts_bindings};
pub use workflow_run::{CreateRunPayload, CreateRunPurpose, CreateRunSource, GetRunPayload};
pub use worktrees::{
    DEFAULT_WORKTREE_PAGE_SIZE, GitObjectFormat, GitObjectId, GitOperation, InventoryCoverage,
    InventoryCoverageStatus, ListWorktreesPayload, ListWorktreesResult,
    MAX_WORKTREE_CURSOR_CHARS, MAX_WORKTREE_PAGE_SIZE, OperationBranch,
    OperationBranchProvenance, ReservedBranchOperation, WorktreeContractViolation, WorktreeDirty,
    WorktreeItem, WorktreeOwnership, WorktreeQueryError, is_worktree_branch_ref,
    is_well_formed_cursor,
};
