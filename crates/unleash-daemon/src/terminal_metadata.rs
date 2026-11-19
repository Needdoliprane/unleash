//! Pure bounded SQLite projection. No host connection, capability or lease renewal.
use serde::Serialize;
use std::collections::HashSet;
use unleash_domain::{ActorId, ProcessRunUuid, ProjectId, Revision, TerminalId};
use unleash_store::StoreError;

const PAGE_LIMIT: usize = 256;

#[derive(Serialize)]
pub(crate) struct TerminalMetadataPage {
    pub(crate) items: Vec<TerminalMetadata>,
    pub(crate) truncated: bool,
}

#[derive(Serialize)]
pub(crate) struct TerminalMetadata {
    terminal_id: TerminalId,
    process_run_uuid: ProcessRunUuid,
    project_id: ProjectId,
    control_revision: Revision,
    terminal_state: String,
    process_state: String,
    lease_state: String,
}

pub(crate) fn read_terminal_metadata(
    conn: &rusqlite::Connection,
    actor: &ActorId,
    project_filter: Option<ProjectId>,
    allowed_projects: Option<&HashSet<ProjectId>>,
) -> Result<TerminalMetadataPage, StoreError> {
    let allowed = allowed_projects
        .map(|projects| {
            serde_json::to_string(&projects.iter().map(ToString::to_string).collect::<Vec<_>>())
        })
        .transpose()?;
    let project = project_filter.map(|project| project.to_string());
    // Filtering precedes LIMIT so neither rows nor the truncation bit disclose
    // another actor/project. All values are bound parameters, never SQL text.
    let mut statement=conn.prepare("SELECT s.terminal_id,p.process_run_uuid,a.project_id,s.control_revision,s.state,p.state,l.state
        FROM terminal_sessions s
        JOIN processes p ON p.process_id=s.process_id
        JOIN terminal_allocations a ON a.terminal_id=s.terminal_id AND a.process_id=p.process_id AND a.process_run_uuid=p.process_run_uuid AND a.action_id=p.action_id
        JOIN principals owner ON owner.principal_id=a.principal_id
        JOIN resource_leases l ON l.lease_id=s.lease_id
        JOIN daemon_state d ON d.singleton=1
        JOIN capacity_reservations cap ON cap.reservation_id=json_extract(a.context_json,'$.pty_reservation_id') AND cap.owner_type='terminal' AND cap.owner_id=s.terminal_id AND cap.amount=1
        JOIN capacity_pools pool ON pool.pool_id=cap.pool_id AND pool.pool_id=?4 AND pool.pool_kind='pty' AND pool.scope_key='global' AND pool.hard_limit<=50
        WHERE p.authority_generation=d.authority_generation AND p.boot_identity=d.boot_identity AND p.owner_uid=d.owner_uid
        AND p.state IN ('live','suspect') AND s.state IN ('ready','busy','detached','suspect')
        AND cap.authority_generation=d.authority_generation AND cap.state IN ('held','reconciling')
        AND owner.actor_id=?1 AND owner.principal_kind='user' AND owner.state='active' AND json_extract(a.context_json,'$.actor_id')=?1
        AND (?2 IS NULL OR a.project_id=?2) AND (?3 IS NULL OR a.project_id IN (SELECT value FROM json_each(?3)))
        ORDER BY s.terminal_id LIMIT 257")?;
    let rows = statement.query_map(
        rusqlite::params![
            actor.as_str(),
            project,
            allowed,
            unleash_store::terminal_allocations::TERMINAL_PTY_POOL_ID.to_string()
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (terminal, run, project, revision, terminal_state, process_state, lease_state) = row?;
        let invalid = || {
            StoreError::InvariantViolation("invalid terminal metadata identity or revision".into())
        };
        items.push(TerminalMetadata {
            terminal_id: terminal.parse().map_err(|_| invalid())?,
            process_run_uuid: run.parse().map_err(|_| invalid())?,
            project_id: project.parse().map_err(|_| invalid())?,
            control_revision: Revision::new(u64::try_from(revision).map_err(|_| invalid())?),
            terminal_state,
            process_state,
            lease_state,
        });
    }
    let truncated = items.len() > PAGE_LIMIT;
    items.truncate(PAGE_LIMIT);
    Ok(TerminalMetadataPage { items, truncated })
}
