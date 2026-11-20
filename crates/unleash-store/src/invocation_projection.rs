//! Lecture paginée d'une invocation et de ses tentatives persistées.

use std::collections::HashSet;
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use serde::Serialize;
use unleash_domain::{
    AgentId, AttemptId, AuthorityGeneration, InvocationId, ProjectId, SessionId, Sha256Digest,
};
use unleash_protocol::ReasonCode;
use uuid::Uuid;

use crate::StoreError;

const MAX_ATTEMPTS_PER_PAGE: u16 = 100;
const MAX_EXACT_WIRE_INTEGER: i64 = 9_007_199_254_740_991;

/// Identité et état d'une tentative de l'invocation visible.
#[derive(Debug, Serialize)]
pub struct InvocationAttemptProjection {
    /// Identité durable de la tentative.
    pub attempt_id: AttemptId,
    /// Position de la tentative dans cette invocation, à partir de un.
    pub ordinal: i64,
    /// État de la tentative, indépendant de celui de l'invocation.
    pub state: String,
    /// Génération d'autorité d'origine, y compris pour une tentative historique.
    pub authority_generation: AuthorityGeneration,
    /// Époque du daemon qui a engagé la tentative.
    pub daemon_epoch: i64,
    /// Numéro de fence de la tentative, sans capacité ni secret d'exécution.
    pub fence: i64,
}

/// Identités, références et état d'une invocation dans un snapshot.
#[derive(Debug, Serialize)]
pub struct InvocationProjection {
    /// Identité de l'invocation.
    pub invocation_id: InvocationId,
    /// Agent logique à l'origine de l'invocation.
    pub agent_id: AgentId,
    /// Session exacte engagée pour l'invocation, même si elle est fermée depuis.
    pub session_id: SessionId,
    /// Projet de rattachement de l'agent. Les agents globaux ont une valeur nulle.
    pub project_id: Option<ProjectId>,
    /// État courant de l'invocation.
    pub state: String,
    /// Motif d'attente durable, s'il existe.
    pub wait_reason: Option<String>,
    /// Révision courante de l'invocation.
    pub revision: i64,
    /// Empreinte du bundle engagé.
    pub effective_bundle_hash: Sha256Digest,
    /// Référence du manifeste dans la session, sans lecture de son contenu.
    pub context_manifest_id: Uuid,
    /// Fenêtre ordonnée de tentatives, vide si aucune ne correspond.
    pub attempts: Vec<InvocationAttemptProjection>,
    /// Dernier ordinal lu si une autre fenêtre est disponible.
    pub next_after_attempt_ordinal: Option<i64>,
}

pub(crate) fn get_invocation_conn(
    conn: &Connection,
    invocation_id: InvocationId,
    project_id: Option<ProjectId>,
    allowed_projects: Option<&HashSet<ProjectId>>,
    after_attempt_ordinal: i64,
    limit: u16,
) -> Result<InvocationProjection, StoreError> {
    if allowed_projects.is_some_and(|allowed| !project_id.is_some_and(|id| allowed.contains(&id))) {
        return Err(StoreError::ScopeDenied {
            reason: Some(ReasonCode::AccessDenied),
            message: "Projet hors du périmètre autorisé pour cette lecture d’invocation".to_owned(),
        });
    }
    if !(0..=MAX_EXACT_WIRE_INTEGER).contains(&after_attempt_ordinal)
        || !(1..=MAX_ATTEMPTS_PER_PAGE).contains(&limit)
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "Fenêtre des tentatives invalide".to_owned(),
        });
    }

    let invocation_key = invocation_id.to_string();
    let mut result = conn
        .query_row(
            "SELECT i.agent_id, i.session_id, i.state, i.wait_reason, i.revision,
                    i.effective_bundle_hash, i.context_manifest_id
             FROM invocations i
             JOIN agents a ON a.agent_id = i.agent_id
             JOIN sessions s ON s.session_id = i.session_id AND s.agent_id = a.agent_id
             JOIN context_manifests m ON m.manifest_id = i.context_manifest_id
                 AND m.session_id = s.session_id
             JOIN effective_bundles b ON b.bundle_hash = i.effective_bundle_hash
             JOIN context_manifests origin ON origin.manifest_id = b.initial_context_manifest_id
                 AND origin.session_id = s.session_id
             WHERE i.invocation_id = ?1 AND a.project_id IS ?2",
            params![invocation_key, project_id.map(|id| id.to_string())],
            |row| {
                Ok(InvocationProjection {
                    invocation_id,
                    agent_id: parse_column(row, 0)?,
                    session_id: parse_column(row, 1)?,
                    project_id,
                    state: row.get(2)?,
                    wait_reason: row.get(3)?,
                    revision: wire_integer(row, 4)?,
                    effective_bundle_hash: parse_column(row, 5)?,
                    context_manifest_id: parse_column(row, 6)?,
                    attempts: Vec::new(),
                    next_after_attempt_ordinal: None,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound("Invocation introuvable dans la portée demandée".into())
        })?;
    let mut attempts = read_attempts(conn, &invocation_key, after_attempt_ordinal, limit)?;
    if attempts.len() > usize::from(limit) {
        attempts.pop();
        result.next_after_attempt_ordinal = attempts.last().map(|attempt| attempt.ordinal);
    }
    result.attempts = attempts;
    Ok(result)
}

fn read_attempts(
    conn: &Connection,
    invocation_id: &str,
    after_ordinal: i64,
    limit: u16,
) -> Result<Vec<InvocationAttemptProjection>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT attempt_id, ordinal, state, authority_generation, daemon_epoch, fence
         FROM invocation_attempts
         WHERE invocation_id = ?1 AND ordinal > ?2
         ORDER BY ordinal
         LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![invocation_id, after_ordinal, i64::from(limit) + 1],
        |row| {
            Ok(InvocationAttemptProjection {
                attempt_id: parse_column(row, 0)?,
                ordinal: wire_integer(row, 1)?,
                state: row.get(2)?,
                authority_generation: parse_column(row, 3)?,
                daemon_epoch: wire_integer(row, 4)?,
                fence: wire_integer(row, 5)?,
            })
        },
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from_rusqlite)
}

fn parse_column<T: FromStr>(row: &Row<'_>, index: usize) -> rusqlite::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    row.get::<_, String>(index)?.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn wire_integer(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    let value = row.get(index)?;
    if !(0..=MAX_EXACT_WIRE_INTEGER).contains(&value) {
        return Err(rusqlite::Error::IntegralValueOutOfRange(index, value));
    }
    Ok(value)
}
