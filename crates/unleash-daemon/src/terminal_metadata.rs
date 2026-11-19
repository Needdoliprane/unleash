//! Pure bounded `SQLite` projection in the existing daemon snapshot.
use std::collections::HashSet;
use unleash_domain::{ActorId, ProjectId};
use unleash_store::{StoreError, terminal_allocations::TerminalMetadataPage};

pub(crate) fn read_terminal_metadata(
    conn: &rusqlite::Connection,
    actor: &ActorId,
    project_filter: Option<ProjectId>,
    allowed_projects: Option<&HashSet<ProjectId>>,
) -> Result<TerminalMetadataPage, StoreError> {
    unleash_store::terminal_allocations::read_terminal_metadata(
        conn,
        actor,
        project_filter,
        allowed_projects,
    )
}
