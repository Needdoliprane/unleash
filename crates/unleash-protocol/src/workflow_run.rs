//! Wire coordinates for preparing, reading and listing workflow runs.

use serde::{Deserialize, Deserializer, Serialize};
use unleash_domain::{MissionId, ProjectId, Revision, RunId, RunState, Sha256Digest};

/// Page size retained when `ListRuns` omits `page_size`.
pub const DEFAULT_RUN_PAGE_SIZE: u16 = 50;

/// Largest page size accepted by `ListRuns`.
pub const MAX_RUN_PAGE_SIZE: u16 = 100;

/// Largest length, in bytes, of an `opaque_cursor` of `ListRuns`.
pub const MAX_OPAQUE_CURSOR_LEN: usize = 512;

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

/// Shape or bound defect of a `ListRuns` payload, before any authority decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ListRunsPayloadError {
    /// `page_size` lies outside the closed interval from 1 to 100.
    #[error("page_size must be an integer from 1 to 100")]
    InvalidPageSize,
    /// `opaque_cursor` is empty, longer than 512 characters, or holds a character
    /// outside ASCII letters, digits, dashes and underscores.
    #[error("opaque_cursor must hold 1 to 512 ASCII letters, digits, dashes or underscores")]
    InvalidCursor,
}

/// Closed argument object of `ListRuns`.
///
/// Decoding applies the contract bounds: `page_size` from 1 to 100, 50 by default, and an
/// `opaque_cursor` omitted or null for the first page, otherwise 1 to 512 ASCII letters,
/// digits, dashes or underscores. The cursor stays opaque: only the daemon issues and reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawListRuns")]
pub struct ListRunsPayload {
    page_size: u16,
    opaque_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawListRuns {
    #[serde(default, deserialize_with = "present")]
    page_size: Option<u16>,
    #[serde(default)]
    opaque_cursor: Option<String>,
}

/// Reads a present field and refuses a null one, as the closed schema does.
fn present<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: Deserializer<'de>,
{
    u16::deserialize(deserializer).map(Some)
}

/// Tells whether a cursor respects the wire charset of `opaque_cursor`.
fn is_well_formed_cursor(cursor: &str) -> bool {
    !cursor.is_empty()
        && cursor.len() <= MAX_OPAQUE_CURSOR_LEN
        && cursor
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

impl TryFrom<RawListRuns> for ListRunsPayload {
    type Error = ListRunsPayloadError;

    fn try_from(raw: RawListRuns) -> Result<Self, Self::Error> {
        let page_size = raw.page_size.unwrap_or(DEFAULT_RUN_PAGE_SIZE);
        if !(1..=MAX_RUN_PAGE_SIZE).contains(&page_size) {
            return Err(ListRunsPayloadError::InvalidPageSize);
        }
        if let Some(cursor) = raw.opaque_cursor.as_deref()
            && !is_well_formed_cursor(cursor)
        {
            return Err(ListRunsPayloadError::InvalidCursor);
        }
        Ok(Self {
            page_size,
            opaque_cursor: raw.opaque_cursor,
        })
    }
}

impl ListRunsPayload {
    /// Requested page size, from 1 to 100.
    #[must_use]
    pub const fn page_size(&self) -> u16 {
        self.page_size
    }

    /// Opaque cursor of the previous page; `None` for the first page.
    #[must_use]
    pub fn opaque_cursor(&self) -> Option<&str> {
        self.opaque_cursor.as_deref()
    }
}

/// Durable purpose of a run, as retained at its creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPurpose {
    /// Operational execution of a published version.
    Operational,
    /// Evaluation of a candidate, without operational effect.
    Evaluation,
}

/// Durable run summarized in a `ListRuns` page.
///
/// The version is the one engaged by the run: it never follows an alias and does not change
/// when a later version is published (REQ-WF-001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunListEntry {
    /// Run identity.
    pub run_id: RunId,
    /// Mission owning the run.
    pub mission_id: MissionId,
    /// Exact key of the workflow version engaged by the run.
    pub workflow_version_id: String,
    /// Durable state of the run.
    pub state: RunState,
    /// Current revision of the run.
    pub revision: Revision,
    /// Purpose of the run.
    pub purpose: RunPurpose,
}

/// Coverage status of a run page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCoverageStatus {
    /// The page covers the runs visible at this read, with the indicated pagination.
    Complete,
}

/// Coverage of a run page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPageCoverage {
    /// Coverage status.
    pub status: RunCoverageStatus,
}

/// Page of the runs of one project returned by `ListRuns`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPage {
    /// Project of the read.
    pub project_id: ProjectId,
    /// Visible runs of this page, sorted by ascending run identity.
    pub items: Vec<RunListEntry>,
    /// Opaque cursor of the next page, `None` when no other authorized entry follows.
    pub next_cursor: Option<String>,
    /// Coverage of the page.
    pub coverage: RunPageCoverage,
}
