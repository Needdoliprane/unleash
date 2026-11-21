//! Wire coordinates for preparing and reading a workflow run.

use serde::{Deserialize, Serialize};
use unleash_domain::{MissionId, RunId, Sha256Digest};

/// Exact published source of an operational run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreateRunSource {
    /// A published workflow definition, selected without an alias.
    WorkflowVersion {
        /// Logical identifier in the definition document.
        id: String,
        /// Version in the definition document.
        version: u32,
        /// Digest of the published definition.
        digest: Sha256Digest,
    },
}

/// Purpose of the exposed operational preparation route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateRunPurpose {
    /// Prepare a normal run from a published definition.
    Operational,
}

/// Closed argument object for the operational workflow-version path of `CreateRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRunPayload {
    /// Mission that owns the run.
    pub mission_id: MissionId,
    /// Exact published source.
    pub source: CreateRunSource,
    /// Operational purpose.
    pub purpose: CreateRunPurpose,
    /// Optional terminal predecessor selected by the caller.
    pub predecessor_run_id: Option<RunId>,
    /// Shareable reason for the selection.
    pub selection_reason: String,
    /// Digest of the scope manifest retained by the current mission revision.
    pub scope_ref: Sha256Digest,
}

/// Closed argument object for `GetRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRunPayload {
    /// Run in the project identified by the query envelope.
    pub run_id: RunId,
}
