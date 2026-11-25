//! Read model for a prepared or retained workflow run, and the pages that list the runs.

use std::collections::HashSet;
use std::fmt::Display;
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use unleash_domain::{
    AuthorityGeneration, ProjectId, Revision, RunId, Sequence, Sha256Digest, StreamId,
};
use unleash_protocol::{
    EventCursor, ListRunsPayload, ReasonCode, RunCoverageStatus, RunListEntry, RunPage,
    RunPageCoverage, RunPurpose,
};
use uuid::Uuid;

use crate::{DaemonStateRow, StoreConfig, StoreError, commands, workflow_run_plan};

/// Durable run coordinates and immutable preparation, read in one snapshot.
#[derive(Debug, Serialize)]
pub struct WorkflowRunProjection {
    /// Run identity.
    pub run_id: RunId,
    /// Project of the owning mission.
    pub project_id: ProjectId,
    /// Owning mission.
    pub mission_id: String,
    /// Exact published workflow version key.
    pub workflow_version_id: String,
    /// Digest of the pinned definition.
    pub definition_hash: Sha256Digest,
    /// Digest of the immutable effective plan.
    pub effective_bundle_hash: Sha256Digest,
    /// Durable state.
    pub state: String,
    /// Current revision.
    pub revision: i64,
    /// Operational or evaluation purpose.
    pub purpose: String,
    /// Retained plan from the preparation writer, when present on the run.
    pub plan: Option<Value>,
}

pub(crate) fn get_run_conn(
    conn: &Connection,
    config: &StoreConfig,
    run: RunId,
    project: ProjectId,
    allowed_projects: Option<&HashSet<ProjectId>>,
) -> Result<WorkflowRunProjection, StoreError> {
    if allowed_projects.is_some_and(|allowed| !allowed.contains(&project)) {
        return Err(StoreError::ScopeDenied {
            reason: Some(ReasonCode::AccessDenied),
            message: "Projet hors du périmètre autorisé pour GetRun".to_owned(),
        });
    }
    let found = conn
        .query_row(
            "SELECT r.mission_id,r.workflow_version_id,v.content_hash,r.effective_bundle_hash,
                r.state,r.revision,r.purpose
         FROM workflow_runs r
         JOIN missions m ON m.mission_id=r.mission_id
         JOIN workflow_versions v ON v.workflow_version_id=r.workflow_version_id
         WHERE r.run_id=?1 AND m.project_id=?2",
            params![run.to_string(), project.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((mission_id, workflow_version_id, definition, bundle, state, revision, purpose)) =
        found
    else {
        return Err(StoreError::NotFound(
            "Run introuvable dans le projet demandé".to_owned(),
        ));
    };
    let definition_hash = workflow_run_plan::parse_hash(&definition)?;
    let effective_bundle_hash = workflow_run_plan::parse_hash(&bundle)?;
    let plan = workflow_run_plan::read_run_manifest(
        conn,
        config,
        &workflow_run_plan::RunManifestTarget {
            run,
            mission: &mission_id,
            version: &workflow_version_id,
            definition_hash,
            bundle_hash: effective_bundle_hash,
        },
    )?;
    Ok(WorkflowRunProjection {
        run_id: run,
        project_id: project,
        mission_id,
        workflow_version_id,
        definition_hash,
        effective_bundle_hash,
        state,
        revision,
        purpose,
        plan,
    })
}

/// Length, in bytes, of a run page cursor before its hexadecimal rendering.
const RUN_CURSOR_BYTES: usize = 50;

/// Format mark of the run page cursor.
const RUN_CURSOR_FORMAT: u8 = 1;

/// Operation mark bound into a run page cursor, so that no other operation resumes from it.
const RUN_CURSOR_OPERATION: u8 = 1;

/// Resume position bonded to its project and to the authority lineage that issued it.
struct RunCursor {
    lineage: AuthorityGeneration,
    project: ProjectId,
    after: RunId,
}

/// Reason a run page cursor was not issued for this listing.
enum RunCursorRefusal {
    /// The cursor is not in the format this store issues.
    Malformed,
    /// The cursor was issued for another operation.
    Foreign,
}

/// Renders a resume position as an opaque cursor.
fn encode_run_cursor(cursor: &RunCursor) -> String {
    let mut bytes = [0_u8; RUN_CURSOR_BYTES];
    bytes[0..2].copy_from_slice(&[RUN_CURSOR_FORMAT, RUN_CURSOR_OPERATION]);
    bytes[2..18].copy_from_slice(&cursor.lineage.as_uuid().as_u128().to_be_bytes());
    bytes[18..34].copy_from_slice(&cursor.project.as_uuid().as_u128().to_be_bytes());
    bytes[34..50].copy_from_slice(&cursor.after.as_uuid().as_u128().to_be_bytes());
    hex::encode(bytes)
}

/// Reads an opaque cursor without trusting it.
fn decode_run_cursor(opaque: &str) -> Result<RunCursor, RunCursorRefusal> {
    let mut bytes = [0_u8; RUN_CURSOR_BYTES];
    if opaque.len() != RUN_CURSOR_BYTES * 2 || hex::decode_to_slice(opaque, &mut bytes).is_err() {
        return Err(RunCursorRefusal::Malformed);
    }
    if bytes[0..1] != [RUN_CURSOR_FORMAT] {
        return Err(RunCursorRefusal::Malformed);
    }
    if bytes[1..2] != [RUN_CURSOR_OPERATION] {
        return Err(RunCursorRefusal::Foreign);
    }
    let mut lineage = [0_u8; 16];
    let mut project = [0_u8; 16];
    let mut after = [0_u8; 16];
    lineage.copy_from_slice(&bytes[2..18]);
    project.copy_from_slice(&bytes[18..34]);
    after.copy_from_slice(&bytes[34..50]);
    Ok(RunCursor {
        lineage: AuthorityGeneration::from_uuid(Uuid::from_u128(u128::from_be_bytes(lineage))),
        project: ProjectId::from_u128(u128::from_be_bytes(project)),
        after: RunId::from_u128(u128::from_be_bytes(after)),
    })
}

/// Watermark of the active lineage stream, read in the transaction of the caller.
fn current_watermark(conn: &Connection, stream: StreamId) -> Result<EventCursor, StoreError> {
    let high: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE stream_id = ?1",
            [stream.to_string()],
            |row| row.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(EventCursor::new(
        stream,
        Sequence::new(u64::try_from(high).unwrap_or(0)),
    ))
}

/// Resume position requested by the page, or `None` for the first page.
///
/// The cursor is fully read again: its shape, its operation, its project and its lineage.
fn resume_after(
    conn: &Connection,
    daemon: &DaemonStateRow,
    page: &ListRunsPayload,
    project: ProjectId,
) -> Result<Option<RunId>, StoreError> {
    let Some(opaque) = page.opaque_cursor() else {
        return Ok(None);
    };
    let refused = |detail: &str| StoreError::InvalidArgument {
        reason: None,
        message: format!("Curseur de liste de runs {detail}"),
    };
    let cursor = match decode_run_cursor(opaque) {
        Ok(cursor) => cursor,
        Err(RunCursorRefusal::Malformed) => return Err(refused("mal formé")),
        Err(RunCursorRefusal::Foreign) => {
            return Err(refused("délivré pour une autre opération"));
        }
    };
    if cursor.project != project {
        return Err(refused("délivré pour un autre projet"));
    }
    if cursor.lineage != daemon.authority_generation {
        return Err(StoreError::ResyncRequired {
            stream_id: daemon.stream_id,
            watermark: current_watermark(conn, daemon.stream_id)?,
            reason: "la lignée d'autorité qui a délivré ce curseur n'est plus servie".to_owned(),
        });
    }
    Ok(Some(cursor.after))
}

/// Relit un identifiant de run stocké sous sa forme canonique.
fn stored<Id: FromStr>(value: &str, field: &str) -> Result<Id, StoreError>
where
    Id::Err: Display,
{
    value.parse().map_err(|error| {
        StoreError::InvariantViolation(format!(
            "Identifiant de run stocké invalide ({field}) : {error}"
        ))
    })
}

/// Relit la finalité d'un run stockée.
fn run_purpose(purpose: &str) -> Result<RunPurpose, StoreError> {
    match purpose {
        "operational" => Ok(RunPurpose::Operational),
        "evaluation" => Ok(RunPurpose::Evaluation),
        other => Err(StoreError::InvariantViolation(format!(
            "finalité de run inconnue '{other}'"
        ))),
    }
}

fn run_list_entry(row: &rusqlite::Row<'_>) -> Result<RunListEntry, StoreError> {
    let run_id: String = row.get(0)?;
    let mission_id: String = row.get(1)?;
    let workflow_version_id: String = row.get(2)?;
    let state: String = row.get(3)?;
    let revision: i64 = row.get(4)?;
    let purpose: String = row.get(5)?;
    let revision = u64::try_from(revision).map_err(|_| {
        StoreError::InvariantViolation("révision de run stockée négative".to_owned())
    })?;
    Ok(RunListEntry {
        run_id: stored(&run_id, "run_id")?,
        mission_id: stored(&mission_id, "mission_id")?,
        workflow_version_id,
        state: commands::parse_run_state(&state)?,
        revision: Revision::new(revision),
        purpose: run_purpose(&purpose)?,
    })
}

/// Liste une page des runs durables d'un projet, triés par identifiant de run croissant.
///
/// Chaque entrée garde la version engagée par le run : une publication postérieure ne la
/// change pas (REQ-WF-001).
///
/// # Errors
/// Refuse une portée invisible, un projet absent, un curseur mal formé ou étranger et un
/// curseur d'une lignée d'autorité qui n'est plus servie.
pub(crate) fn list_runs_conn(
    conn: &Connection,
    page: &ListRunsPayload,
    project: ProjectId,
    allowed_projects: Option<&HashSet<ProjectId>>,
) -> Result<RunPage, StoreError> {
    if allowed_projects.is_some_and(|allowed| !allowed.contains(&project)) {
        return Err(StoreError::ScopeDenied {
            reason: Some(ReasonCode::AccessDenied),
            message: "Projet hors du périmètre autorisé pour ListRuns".to_owned(),
        });
    }
    // Le périmètre se décide avant l'existence du projet : un refus de portée ne révèle rien.
    let present: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
        [project.to_string()],
        |row| row.get(0),
    )?;
    if present == 0 {
        return Err(StoreError::NotFound(
            "Projet introuvable ou masqué pour ListRuns".to_owned(),
        ));
    }
    let daemon = commands::read_daemon_state_conn(conn)?;
    let after = resume_after(conn, &daemon, page, project)?;
    let page_size = usize::from(page.page_size());
    let mut statement = conn.prepare(
        "SELECT r.run_id,r.mission_id,r.workflow_version_id,r.state,r.revision,r.purpose
         FROM workflow_runs r
         JOIN missions m ON m.mission_id=r.mission_id
         WHERE m.project_id=?1 AND r.run_id>?2
         ORDER BY r.run_id
         LIMIT ?3",
    )?;
    let mut rows = statement.query(params![
        project.to_string(),
        after.map_or_else(String::new, ToString::to_string),
        i64::from(page.page_size()) + 1,
    ])?;
    let mut items = Vec::with_capacity(page_size);
    let mut next_cursor = None;
    while let Some(row) = rows.next()? {
        if items.len() == page_size {
            // Une ligne de plus que la page : une autre entrée autorisée suit.
            if let Some(last) = items.last() {
                next_cursor = Some(encode_run_cursor(&RunCursor {
                    lineage: daemon.authority_generation,
                    project,
                    after: last.run_id,
                }));
            }
            break;
        }
        items.push(run_list_entry(row)?);
    }
    Ok(RunPage {
        project_id: project,
        items,
        next_cursor,
        coverage: RunPageCoverage {
            status: RunCoverageStatus::Complete,
        },
    })
}
