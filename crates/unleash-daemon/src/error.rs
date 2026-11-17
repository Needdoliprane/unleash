// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées du daemon Unleash et projections normatives vers [`IpcError`].

use std::path::PathBuf;

use thiserror::Error;
use unleash_domain::{CommandId, DomainTransitionError};
use unleash_platform::PlatformError;
use unleash_protocol::{ErrorCode, IpcError, ProtocolError, ReasonCode, RetryClass};
use unleash_store::StoreError;

/// Erreur produite par le démarrage, la boucle IPC ou l'arrêt de `unleash-daemon`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// Erreur plateforme OS (verrou, socket, permissions, identité).
    #[error("Erreur plateforme daemon : {0}")]
    Platform(#[from] PlatformError),

    /// Erreur de stockage `SQLite` WAL (`unleash-store`).
    #[error("Erreur de stockage daemon : {0}")]
    Store(#[from] StoreError),

    /// Erreur protocolaire `unleash-protocol`.
    #[error("Erreur protocolaire daemon : {0}")]
    Protocol(#[from] ProtocolError),

    /// Erreur de transition métier `unleash-domain`.
    #[error("Erreur domaine daemon : {0}")]
    Domain(#[from] DomainTransitionError),

    /// Erreur d'identifiant ou de digest.
    #[error("Identifiant invalide : {0}")]
    IdValidation(#[from] unleash_domain::IdValidationError),

    /// Une autre autorité daemon est déjà active sur une autre racine de données.
    #[error(
        "Autorité déjà active sur '{}' (demandé '{}', socket '{}')",
        active_data_root.display(),
        requested_data_root.display(),
        socket_path.display()
    )]
    AuthorityAlreadyActive {
        /// Racine détenue par l'autorité active.
        active_data_root: PathBuf,
        /// Racine demandée.
        requested_data_root: PathBuf,
        /// Socket de l'autorité active.
        socket_path: PathBuf,
    },

    /// Une instance daemon est déjà active sur la même racine de données.
    #[error(
        "Daemon déjà actif sur '{}' (socket '{}')",
        active_data_root.display(),
        socket_path.display()
    )]
    DaemonAlreadyRunning {
        /// Racine active.
        active_data_root: PathBuf,
        /// Socket actif.
        socket_path: PathBuf,
    },

    /// Le changement de racine est bloqué par des exécuteurs non neutralisés de l'ancienne racine.
    #[error(
        "Changement de racine bloqué de '{}' vers '{}' : exécuteurs encore actifs ({})",
        active_data_root.display(),
        requested_data_root.display(),
        active_hosts.join(", ")
    )]
    AuthoritySwitchBlocked {
        /// Ancienne racine propriétaire des exécuteurs.
        active_data_root: PathBuf,
        /// Nouvelle racine demandée.
        requested_data_root: PathBuf,
        /// Identifiants des exécuteurs non neutralisés.
        active_hosts: Vec<String>,
    },

    /// Configuration daemon invalide.
    #[error("Configuration daemon invalide : {0}")]
    InvalidConfiguration(String),

    /// Erreur de sérialisation JSON interne.
    #[error("Erreur JSON daemon : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur I/O système.
    #[error("Erreur I/O daemon : {0}")]
    Io(#[from] std::io::Error),
}

/// Convertit une [`StoreError`] en [`IpcError`] en conservant le `command_id` si fourni.
#[must_use]
pub fn store_error_to_ipc_error(err: &StoreError, command_id: Option<CommandId>) -> IpcError {
    let mut ipc = err.to_ipc_error();
    if ipc.command_id.is_none()
        && let Some(cid) = command_id
    {
        ipc = ipc.with_command_id(cid);
    }
    ipc
}

/// Convertit une [`PlatformError`] en [`IpcError`].
#[must_use]
pub fn platform_error_to_ipc_error(err: &PlatformError) -> IpcError {
    match err {
        PlatformError::PeerUidMismatch { .. } => IpcError::new(
            ErrorCode::AuthenticationFailed,
            None,
            err.to_string(),
            RetryClass::Never,
        ),
        PlatformError::AuthorityAlreadyActive { .. } => {
            IpcError::from_reason(ReasonCode::AuthorityAlreadyActive, err.to_string())
        }
        PlatformError::DaemonAlreadyRunning { .. } => {
            IpcError::from_reason(ReasonCode::DaemonAlreadyRunning, err.to_string())
        }
        PlatformError::ForbiddenDocumentationPath { .. }
        | PlatformError::InvalidPath { .. }
        | PlatformError::SocketPathTooLong { .. } => IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            err.to_string(),
            RetryClass::Never,
        ),
        PlatformError::PermissionDenied { .. }
        | PlatformError::OwnershipMismatch { .. }
        | PlatformError::InsecurePermissions { .. }
        | PlatformError::SymlinkNotAllowed { .. } => {
            IpcError::from_reason(ReasonCode::OperationNotAuthorized, err.to_string())
        }
        _ => IpcError::new(
            ErrorCode::InternalFailure,
            None,
            err.to_string(),
            RetryClass::Never,
        ),
    }
}

/// Convertit une [`DomainTransitionError`] en [`IpcError`].
#[must_use]
pub fn domain_error_to_ipc_error(
    err: &DomainTransitionError,
    command_id: Option<CommandId>,
) -> IpcError {
    let mut ipc = match err {
        DomainTransitionError::TerminalImmutable { .. } => {
            IpcError::from_reason(ReasonCode::TerminalImmutable, err.to_string())
        }
        DomainTransitionError::PredecessorNotTerminal { .. } => {
            IpcError::from_reason(ReasonCode::PredecessorNotTerminal, err.to_string())
        }
        DomainTransitionError::CannotRetryOutcomeUnknown => {
            IpcError::from_reason(ReasonCode::UnresolvedEffect, err.to_string())
        }
        DomainTransitionError::PolicyViolation { .. } => {
            IpcError::from_reason(ReasonCode::PolicyDenied, err.to_string())
        }
        DomainTransitionError::GuardUnavailableFailClosed => {
            IpcError::from_reason(ReasonCode::GuardUnavailable, err.to_string())
        }
        DomainTransitionError::StaleFence { .. } | DomainTransitionError::IncomparableLineage => {
            IpcError::from_reason(ReasonCode::StaleEpoch, err.to_string())
        }
        DomainTransitionError::InvalidTransition { .. }
        | DomainTransitionError::AttemptIrreversible { .. } => {
            IpcError::from_reason(ReasonCode::ContractRejected, err.to_string())
        }
    };
    if let Some(cid) = command_id {
        ipc = ipc.with_command_id(cid);
    }
    ipc
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use unleash_domain::Revision;

    use super::*;

    #[test]
    fn platform_error_to_ipc_error_maps_security_and_path_variants() {
        let auth = platform_error_to_ipc_error(&PlatformError::PeerUidMismatch {
            expected_uid: 501,
            actual_uid: 0,
        });
        assert_eq!(auth.code, ErrorCode::AuthenticationFailed);

        let forbidden = platform_error_to_ipc_error(&PlatformError::ForbiddenDocumentationPath {
            path: String::from("/tmp/.claude"),
        });
        assert_eq!(forbidden.code, ErrorCode::InvalidArgument);

        let perm = platform_error_to_ipc_error(&PlatformError::PermissionDenied {
            context: String::from("0777"),
        });
        assert_eq!(perm.code, ErrorCode::CapabilityDenied);
        assert_eq!(perm.reason_code, Some(ReasonCode::OperationNotAuthorized));
    }

    #[test]
    fn domain_and_store_errors_preserve_command_id_and_map_codes() {
        let cid = CommandId::from_u128(0xABCD);
        let dom = domain_error_to_ipc_error(
            &DomainTransitionError::InvalidTransition {
                entity: "Daemon",
                from: String::from("stopped"),
                to: String::from("ready"),
                reason: String::from("interdit"),
            },
            Some(cid),
        );
        assert_eq!(dom.command_id, Some(cid));
        assert_eq!(dom.reason_code, Some(ReasonCode::ContractRejected));

        let st = store_error_to_ipc_error(
            &StoreError::RevisionConflict {
                current_revision: Revision::new(4),
                expected_revision: Revision::new(2),
                command_id: None,
            },
            Some(cid),
        );
        assert_eq!(st.code, ErrorCode::RevisionConflict);
        assert_eq!(st.command_id, Some(cid));
    }
}
