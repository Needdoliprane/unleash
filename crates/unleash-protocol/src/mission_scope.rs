//! Exact scope selectors for mission preparation.

use serde::{Deserialize, Serialize};
use unleash_domain::Revision;

/// Scope referenced by a mission at creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionScopeSelector {
    /// The project of the mission envelope at an exact observed revision.
    Project {
        /// Project revision approved for this scope.
        revision: Revision,
    },
}
