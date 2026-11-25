//! Registre des allocations de worktrees (`worktrees`) : lectures de l'inventaire Git.
//!
//! Une ligne rattache un worktree alloué par Unleash à son projet, son dépôt et son chemin
//! canonique. L'inventaire observé par Git peut contredire ce registre ; la propriété exposée
//! n'est donc qu'une correspondance de chemin, jamais une preuve (`REQ-GIT-001`).

use rusqlite::Connection;
use unleash_domain::ProjectId;

use crate::error::StoreError;

/// Chemins canoniques des worktrees qu'Unleash a alloués pour un dépôt rattaché, hors
/// allocations retirées.
pub(crate) fn managed_paths_conn(
    conn: &Connection,
    project_id: ProjectId,
    repo_id: &str,
) -> Result<Vec<String>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT canonical_path FROM worktrees
             WHERE project_id = ?1 AND repo_id = ?2 AND state <> 'retired'
             ORDER BY canonical_path",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = statement
        .query_map(rusqlite::params![project_id.to_string(), repo_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(StoreError::from_rusqlite)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from_rusqlite)
}
