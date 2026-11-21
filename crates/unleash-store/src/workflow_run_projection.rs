//! Read model for a prepared or retained workflow run.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use unleash_domain::{ProjectId, RunId, Sha256Digest};
use unleash_protocol::ReasonCode;

use crate::{StoreConfig, StoreError, workflow_run_plan};

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
