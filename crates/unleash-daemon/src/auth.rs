// SPDX-License-Identifier: Apache-2.0
//! Authentification des pairs Unix (`LOCAL_PEERCRED` / `SO_PEERCRED`), résolution serveur
//! d'`actor_ref`, rejet d'usurpation et matrice de capacités (`REQ-CORE-004`).

use std::collections::HashSet;

use unleash_domain::{ActorId, ProjectId};
use unleash_platform::{PeerCredentials, effective_uid, verify_peer_uid};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, HelloRequest, IpcError, QueryEnvelope, ReasonCode,
    RetryClass,
};

/// Capacité atomique vérifiée côté daemon pour chaque opération IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Lecture de diagnostic (`GetDaemonStatus`, `GetHealth`).
    QueryDaemonDiagnostic,
    /// Lecture de projections et reçus (`GetStateSnapshot`, `GetCommandReceipt`, `SubscribeEvents`).
    QueryState,
    /// Création et gestion de projets (`CreateProject`).
    ManageProjects,
    /// Création et pilotage de missions (`AssignMission`, `PauseMission`, `ResumeMission`, `CancelMission`).
    ManageMissions,
    /// Arrêt contrôlé du daemon (`StopDaemon`).
    StopDaemon,
    /// Opérations réservées aux workers authentifiés (`RecordClaim`, `RecordObservation`, `RecordJudgment`).
    WorkerReport,
}

/// Session client authentifiée côté serveur à l'issue du handshake `Hello`.
#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    /// Identifiants noyau extraits du socket Unix.
    pub peer_creds: PeerCredentials,
    /// Nature déclarée du client.
    pub client_kind: ClientKind,
    /// Référence d'acteur résolue côté serveur (jamais contrôlée par le client).
    pub actor_ref: String,
    /// Identifiant d'acteur utilisé comme clé d'idempotence en base.
    pub actor_id: ActorId,
    /// Capacités accordées pour cette session.
    pub capabilities: HashSet<Capability>,
    /// Restriction optionnelle aux projets autorisés (`None` = tous projets de l'utilisateur).
    pub allowed_projects: Option<HashSet<ProjectId>>,
}

/// Port d'authentification des pairs et d'attribution de capacités lors du `Hello`.
pub trait AuthVerifierPort: Send + Sync {
    /// Valide les credentials noyau et la requête `HelloRequest` et construit la session.
    ///
    /// # Errors
    /// Retourne [`IpcError`] (`AuthenticationFailed` ou `CapabilityDenied`) en cas de rejet.
    fn authenticate_peer(
        &self,
        peer: PeerCredentials,
        hello: &HelloRequest,
    ) -> Result<AuthenticatedSession, IpcError>;
}

/// Vérificateur par défaut vérifiant l'UID effectif local et les capacités selon `ClientKind`.
#[derive(Debug, Default, Clone, Copy)]
pub struct PeerUidAuthVerifier;

impl AuthVerifierPort for PeerUidAuthVerifier {
    fn authenticate_peer(
        &self,
        peer: PeerCredentials,
        hello: &HelloRequest,
    ) -> Result<AuthenticatedSession, IpcError> {
        verify_peer_uid(peer, effective_uid()).map_err(|e| {
            IpcError::new(
                ErrorCode::AuthenticationFailed,
                None,
                e.to_string(),
                RetryClass::Never,
            )
        })?;

        if hello
            .capabilities
            .iter()
            .any(|cap| cap == "cap:expired" || cap == "cap:revoked")
        {
            return Err(IpcError::from_reason(
                ReasonCode::OperationNotAuthorized,
                "Capacité client expirée ou révoquée",
            ));
        }

        let (actor_ref, capabilities) = match hello.client_kind {
            ClientKind::Cli | ClientKind::Tui | ClientKind::Gui => {
                let mut caps = HashSet::new();
                caps.insert(Capability::QueryDaemonDiagnostic);
                caps.insert(Capability::QueryState);
                caps.insert(Capability::ManageProjects);
                caps.insert(Capability::ManageMissions);
                caps.insert(Capability::StopDaemon);
                (format!("uid:{}", peer.uid), caps)
            }
            ClientKind::Worker => {
                let mut caps = HashSet::new();
                caps.insert(Capability::QueryDaemonDiagnostic);
                caps.insert(Capability::WorkerReport);
                (
                    format!("worker:uid:{}:pid:{}", peer.uid, peer.pid.unwrap_or(0)),
                    caps,
                )
            }
            ClientKind::Probe => {
                let mut caps = HashSet::new();
                caps.insert(Capability::QueryDaemonDiagnostic);
                (
                    format!("probe:uid:{}:pid:{}", peer.uid, peer.pid.unwrap_or(0)),
                    caps,
                )
            }
            _ => {
                return Err(IpcError::new(
                    ErrorCode::AuthenticationFailed,
                    None,
                    "Nature de client inconnue",
                    RetryClass::Never,
                ));
            }
        };

        let actor_id = ActorId::new(&actor_ref).map_err(|e| {
            IpcError::new(
                ErrorCode::AuthenticationFailed,
                None,
                e.to_string(),
                RetryClass::Never,
            )
        })?;

        Ok(AuthenticatedSession {
            peer_creds: peer,
            client_kind: hello.client_kind,
            actor_ref,
            actor_id,
            capabilities,
            allowed_projects: None,
        })
    }
}

/// Vérifie l'identité d'acteur, la capacité et le périmètre projet d'une commande entrante.
///
/// # Errors
/// - [`ErrorCode::AuthenticationFailed`] si `cmd.actor_ref` est fourni et diverge de `session.actor_ref`.
/// - [`ErrorCode::CapabilityDenied`] (`OPERATION_NOT_AUTHORIZED`) si l'opération dépasse les droits.
/// - [`ErrorCode::ScopeDenied`] (`ACCESS_DENIED`) si le projet ciblé est hors du scope autorisé.
/// - [`ErrorCode::InvalidArgument`] si `command_type` est inconnu.
pub fn authorize_command(
    session: &AuthenticatedSession,
    cmd: &CommandEnvelope,
) -> Result<(), IpcError> {
    if let Some(ref claimed) = cmd.actor_ref
        && claimed != &session.actor_ref
    {
        return Err(IpcError::new(
            ErrorCode::AuthenticationFailed,
            None,
            format!(
                "Usurpation d'actor_ref refusée : session authentifiée '{}'",
                session.actor_ref
            ),
            RetryClass::Never,
        )
        .with_command_id(cmd.command_id));
    }

    let required = match cmd.command_type.as_str() {
        "CreateProject" => Capability::ManageProjects,
        "AssignMission" | "PauseMission" | "ResumeMission" | "CancelMission" => {
            Capability::ManageMissions
        }
        "StopDaemon" => Capability::StopDaemon,
        "RecordClaim" | "RecordObservation" | "RecordJudgment" => Capability::WorkerReport,
        other => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("Type de commande inconnu ou non supporté : '{other}'"),
                RetryClass::Never,
            )
            .with_command_id(cmd.command_id));
        }
    };

    if !session.capabilities.contains(&required) {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            format!(
                "Capacité requise absente pour '{}' (client {:?})",
                cmd.command_type, session.client_kind
            ),
        )
        .with_command_id(cmd.command_id));
    }

    if let Some(ref allowed) = session.allowed_projects {
        match cmd.project_id {
            Some(pid) if allowed.contains(&pid) => {}
            _ => {
                return Err(IpcError::from_reason(
                    ReasonCode::AccessDenied,
                    "Projet hors du périmètre autorisé pour cette session",
                )
                .with_command_id(cmd.command_id));
            }
        }
    }

    Ok(())
}

/// Vérifie les capacités et le scope projet pour une requête [`QueryEnvelope`].
///
/// # Errors
/// Retourne [`IpcError`] (`CapabilityDenied`, `ScopeDenied`, `InvalidArgument`).
pub fn authorize_query(
    session: &AuthenticatedSession,
    query: &QueryEnvelope,
) -> Result<(), IpcError> {
    let required = match query.query_type.as_str() {
        "GetDaemonStatus" | "GetHealth" => Capability::QueryDaemonDiagnostic,
        "GetStateSnapshot" | "GetCommandReceipt" => Capability::QueryState,
        other => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                format!("Type de requête inconnu ou non supporté : '{other}'"),
                RetryClass::Never,
            ));
        }
    };

    if !session.capabilities.contains(&required) {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            format!(
                "Capacité requise absente pour la requête '{}'",
                query.query_type
            ),
        ));
    }

    if let Some(ref allowed) = session.allowed_projects
        && let Some(pid) = query.project_id
        && !allowed.contains(&pid)
    {
        return Err(IpcError::from_reason(
            ReasonCode::AccessDenied,
            "Projet hors du périmètre autorisé pour cette requête",
        ));
    }

    Ok(())
}
