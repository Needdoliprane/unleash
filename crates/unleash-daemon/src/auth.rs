// SPDX-License-Identifier: Apache-2.0
//! Authentification des pairs Unix (`LOCAL_PEERCRED` / `SO_PEERCRED`), résolution serveur
//! d'`actor_ref`, rejet d'usurpation et matrice de capacités (`REQ-CORE-004`).

use std::collections::HashSet;
use std::sync::Arc;

use unleash_domain::{ActorId, ProjectId};
use unleash_platform::{PeerCredentials, effective_uid, verify_peer_uid};
use unleash_protocol::{
    AdminProof, ClientKind, CommandEnvelope, ErrorCode, HelloRequest, IpcError, QueryEnvelope,
    ReasonCode, RetryClass, SubscribeEventsRequest,
};

/// Capacité atomique vérifiée côté daemon pour chaque opération IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Capacité octroyée après vérification de la preuve de cette incarnation.
    Administer,
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

/// Enveloppe serveur : retire les droits privés sans preuve du daemon courant.
pub(crate) struct AdministrativeAuthVerifier {
    pub(crate) inner: Arc<dyn AuthVerifierPort>,
    pub(crate) expected: AdminProof,
}

impl AuthVerifierPort for AdministrativeAuthVerifier {
    fn authenticate_peer(
        &self,
        peer: PeerCredentials,
        hello: &HelloRequest,
    ) -> Result<AuthenticatedSession, IpcError> {
        let mut session = self.inner.authenticate_peer(peer, hello)?;
        let permitted_kind = matches!(
            hello.client_kind,
            ClientKind::Cli | ClientKind::Tui | ClientKind::Gui
        );
        let valid = permitted_kind
            && hello
                .admin_proof
                .as_ref()
                .is_some_and(|p| p.matches(&self.expected));
        // Une capacité ajoutée par un port injecté ne remplace jamais la preuve de transport.
        session.capabilities.remove(&Capability::Administer);
        if valid {
            session.capabilities.insert(Capability::Administer);
        } else {
            session
                .capabilities
                .retain(|cap| *cap == Capability::QueryDaemonDiagnostic);
        }
        Ok(session)
    }
}

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
    if !session.capabilities.contains(&Capability::Administer) {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            "Preuve administrative requise",
        )
        .with_command_id(cmd.command_id));
    }
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
        "CreateProject" | "ReviseProject" => Capability::ManageProjects,
        "SetSettings"
        | "SetPolicy"
        | "ConfigureGuard"
        | "PurgeHistory"
        | "RequestBackup"
        | "SetProviderProfile"
        | "TestProviderProfile" => Capability::Administer,
        "AssignMission" | "PauseMission" | "ResumeMission" | "CancelMission"
        | "AcquireTerminal" | "SendTerminalInput" | "ResizeTerminal" | "ReleaseTerminal" => {
            Capability::ManageMissions
        }
        "StopDaemon" => Capability::StopDaemon,
        "RecordClaim" | "RecordObservation" | "RecordJudgment" => Capability::WorkerReport,
        _ => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Type de commande inconnu ou non supporté",
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
    if query.query_type != "GetDaemonStatus"
        && !session.capabilities.contains(&Capability::Administer)
    {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            "Preuve administrative requise",
        ));
    }
    let required = match query.query_type.as_str() {
        "GetDaemonStatus" => Capability::QueryDaemonDiagnostic,
        "GetHealth" | "GetStateSnapshot" | "GetCommandReceipt" | "ListProjects" | "GetProject"
        | "ReadSettings" | "ReadPolicySummary" | "ReadGuardHealth" | "InspectRecovery"
        | "GetBackupManifest" | "ReadTerminalLog" | "GetProviderProfile" | "GetAgent"
        | "GetInvocation" => Capability::QueryState,
        _ => {
            return Err(IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Type de requête inconnu ou non supporté",
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

/// Vérifie la capacité `QueryState` et le périmètre projet d'un abonnement `SubscribeEvents`.
///
/// # Errors
/// Retourne [`IpcError`] (`CapabilityDenied` ou `ScopeDenied`) en cas de rejet.
pub fn authorize_subscription(
    session: &AuthenticatedSession,
    req: &SubscribeEventsRequest,
) -> Result<(), IpcError> {
    if !session.capabilities.contains(&Capability::Administer) {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            "Preuve administrative requise",
        ));
    }
    if !session.capabilities.contains(&Capability::QueryState) {
        return Err(IpcError::from_reason(
            ReasonCode::OperationNotAuthorized,
            "Capacité QueryState requise pour SubscribeEvents",
        ));
    }

    if let Some(ref allowed) = session.allowed_projects
        && let Some(pid) = req.project_id
        && !allowed.contains(&pid)
    {
        return Err(IpcError::from_reason(
            ReasonCode::AccessDenied,
            "Projet hors du périmètre autorisé pour SubscribeEvents",
        ));
    }

    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use serde_json::Map;
    use unleash_domain::{AuthorityGeneration, CommandId, RequestId, Revision, Sequence, StreamId};
    use unleash_protocol::{ProtocolVersion, SubscribeEventsRequest};

    use super::*;

    fn local_peer() -> PeerCredentials {
        PeerCredentials {
            uid: effective_uid(),
            gid: 0,
            pid: Some(std::process::id()),
        }
    }

    #[test]
    fn peer_uid_verifier_resolves_roles_and_rejects_bad_uid_or_revoked_caps() {
        let v = PeerUidAuthVerifier;
        let cli = v
            .authenticate_peer(
                local_peer(),
                &HelloRequest {
                    protocol_version: ProtocolVersion,
                    client_kind: ClientKind::Cli,
                    client_version: String::from("0.1.0"),
                    capabilities: Vec::new(),
                    admin_proof: None,
                },
            )
            .unwrap();
        assert_eq!(cli.actor_ref, format!("uid:{}", effective_uid()));
        assert!(cli.capabilities.contains(&Capability::StopDaemon));
        assert!(cli.allowed_projects.is_none());

        let worker = v
            .authenticate_peer(
                local_peer(),
                &HelloRequest {
                    protocol_version: ProtocolVersion,
                    client_kind: ClientKind::Worker,
                    client_version: String::from("0.1.0"),
                    capabilities: Vec::new(),
                    admin_proof: None,
                },
            )
            .unwrap();
        assert!(worker.actor_ref.starts_with("worker:uid:"));
        assert!(worker.capabilities.contains(&Capability::WorkerReport));
        assert!(!worker.capabilities.contains(&Capability::StopDaemon));

        let wrong_uid = v
            .authenticate_peer(
                PeerCredentials {
                    uid: effective_uid().saturating_add(99),
                    gid: 0,
                    pid: None,
                },
                &HelloRequest {
                    protocol_version: ProtocolVersion,
                    client_kind: ClientKind::Cli,
                    client_version: String::from("0.1.0"),
                    capabilities: Vec::new(),
                    admin_proof: None,
                },
            )
            .unwrap_err();
        assert_eq!(wrong_uid.code, ErrorCode::AuthenticationFailed);

        let revoked = v
            .authenticate_peer(
                local_peer(),
                &HelloRequest {
                    protocol_version: ProtocolVersion,
                    client_kind: ClientKind::Cli,
                    client_version: String::from("0.1.0"),
                    capabilities: vec![String::from("cap:revoked")],
                    admin_proof: None,
                },
            )
            .unwrap_err();
        assert_eq!(revoked.code, ErrorCode::CapabilityDenied);
        assert_eq!(
            revoked.reason_code,
            Some(ReasonCode::OperationNotAuthorized)
        );
    }

    #[test]
    fn authorize_command_query_and_subscription_enforce_identity_caps_and_scope() {
        let p_ok = ProjectId::from_u128(0x10);
        let p_bad = ProjectId::from_u128(0x20);
        let mut caps = HashSet::new();
        caps.insert(Capability::Administer);
        caps.insert(Capability::QueryState);
        caps.insert(Capability::ManageMissions);
        let mut allowed = HashSet::new();
        allowed.insert(p_ok);

        let session = AuthenticatedSession {
            peer_creds: local_peer(),
            client_kind: ClientKind::Cli,
            actor_ref: String::from("scoped:uid:1"),
            actor_id: ActorId::new("scoped:uid:1").unwrap(),
            capabilities: caps,
            allowed_projects: Some(allowed),
        };

        let forged = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: CommandId::from_u128(2),
            authority_generation: AuthorityGeneration::from_u128(3),
            actor_ref: Some(String::from("root")),
            project_id: Some(p_ok),
            aggregate_id: uuid::Uuid::from_u128(4),
            expected_revision: Revision::INITIAL,
            command_type: String::from("PauseMission"),
            payload: Map::new(),
        };
        assert_eq!(
            authorize_command(&session, &forged).unwrap_err().code,
            ErrorCode::AuthenticationFailed
        );

        let mut scoped_cmd = forged.clone();
        scoped_cmd.actor_ref = None;
        scoped_cmd.project_id = Some(p_bad);
        assert_eq!(
            authorize_command(&session, &scoped_cmd).unwrap_err().code,
            ErrorCode::ScopeDenied
        );
        scoped_cmd.project_id = Some(p_ok);
        assert!(authorize_command(&session, &scoped_cmd).is_ok());
        let mut unknown_command = scoped_cmd.clone();
        unknown_command.command_type = String::from("DO_NOT_ECHO");
        assert!(
            !authorize_command(&session, &unknown_command)
                .unwrap_err()
                .message
                .contains("DO_NOT_ECHO")
        );

        let q_bad = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(5),
            query_type: String::from("GetStateSnapshot"),
            project_id: Some(p_bad),
            payload: Map::new(),
        };
        assert_eq!(
            authorize_query(&session, &q_bad).unwrap_err().code,
            ErrorCode::ScopeDenied
        );
        let mut unknown_query = q_bad.clone();
        unknown_query.query_type = String::from("DO_NOT_ECHO");
        assert!(
            !authorize_query(&session, &unknown_query)
                .unwrap_err()
                .message
                .contains("DO_NOT_ECHO")
        );

        let sub_bad = SubscribeEventsRequest {
            stream_id: StreamId::from_u128(6),
            after_sequence: Sequence::new(0),
            project_id: Some(p_bad),
            aggregate_id: None,
            batch_limit: 16,
        };
        let sub_err = authorize_subscription(&session, &sub_bad).unwrap_err();
        assert_eq!(sub_err.code, ErrorCode::ScopeDenied);
        assert_eq!(sub_err.reason_code, Some(ReasonCode::AccessDenied));

        let sub_none = SubscribeEventsRequest {
            project_id: None,
            ..sub_bad
        };
        assert!(authorize_subscription(&session, &sub_none).is_ok());
    }
}
