//! Lecture de l'identité et de la session durables d'un agent.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use unleash_domain::{AgentId, ProjectId};
use unleash_protocol::ReasonCode;

use crate::StoreError;

/// Faits d'identité visibles dans un snapshot du Store.
#[derive(Debug, Serialize)]
pub struct AgentProjection {
    /// Identité de l'agent logique.
    pub agent_id: AgentId,
    /// Projet de rattachement, absent pour un agent global.
    pub project_id: Option<ProjectId>,
    /// Niveau organisationnel sous sa forme `N`, `N+1` ou `N+2`.
    pub level: String,
    /// État de l'identité logique, indépendant de toute invocation.
    pub state: String,
    /// Révision courante de l'agent.
    pub revision: i64,
    /// Session primaire ouverte, si elle existe.
    pub session_id: Option<String>,
    /// Parent organisationnel visible dans le périmètre du lecteur.
    pub organization_parent_id: Option<String>,
    /// Créateur agent visible dans le périmètre du lecteur.
    pub created_by_agent_id: Option<String>,
}

pub(crate) fn get_agent_conn(
    conn: &Connection,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    allowed_projects: Option<&HashSet<ProjectId>>,
) -> Result<AgentProjection, StoreError> {
    if allowed_projects.is_some_and(|allowed| !project_id.is_some_and(|id| allowed.contains(&id))) {
        return Err(StoreError::ScopeDenied {
            reason: Some(ReasonCode::AccessDenied),
            message: "Projet hors du périmètre autorisé pour cette lecture d’agent".to_owned(),
        });
    }
    let visible = allowed_projects.map(|projects| {
        projects
            .iter()
            .map(ToString::to_string)
            .collect::<HashSet<_>>()
    });
    conn.query_row(
        "SELECT a.level, a.state, a.revision, s.session_id,
                parent.agent_id AS parent_id, parent.project_id AS parent_project,
                creator.agent_id AS creator_id, creator.project_id AS creator_project
         FROM agents a
         LEFT JOIN sessions s ON s.agent_id = a.agent_id AND s.state = 'open'
         LEFT JOIN agents parent ON parent.agent_id = a.organization_parent_id
         LEFT JOIN agents creator ON creator.agent_id = a.created_by_agent_id
         WHERE a.agent_id = ?1 AND a.project_id IS ?2",
        params![agent_id.to_string(), project_id.map(|id| id.to_string())],
        |row| {
            Ok(AgentProjection {
                agent_id,
                project_id,
                level: row.get(0)?,
                state: row.get(1)?,
                revision: row.get(2)?,
                session_id: row.get(3)?,
                organization_parent_id: visible_link(
                    row,
                    "parent_id",
                    "parent_project",
                    visible.as_ref(),
                )?,
                created_by_agent_id: visible_link(
                    row,
                    "creator_id",
                    "creator_project",
                    visible.as_ref(),
                )?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from_rusqlite)?
    .ok_or_else(|| StoreError::NotFound("Agent introuvable dans la portée demandée".to_owned()))
}

fn visible_link(
    row: &Row<'_>,
    id_column: &str,
    project_column: &str,
    allowed: Option<&HashSet<String>>,
) -> rusqlite::Result<Option<String>> {
    if let Some(allowed) = allowed {
        let project: Option<String> = row.get(project_column)?;
        if !project.is_some_and(|id| allowed.contains(&id)) {
            return Ok(None);
        }
    }
    row.get(id_column)
}
