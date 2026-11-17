// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées de `unleash-process-host` et projection vers [`IpcError`].
//!
//! Aucune variante ne doit jamais contenir de secret de contrôle ou de canari en clair.

use thiserror::Error;
use unleash_domain::{ActionId, CommandId, FenceError};
use unleash_platform::PlatformError;
use unleash_protocol::{ErrorCode, IpcError, ProtocolError, ReasonCode, RetryClass};

/// Erreur typée produite par `unleash-process-host`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProcessHostError {
    /// L'identité du processus cible ou l'ancrage direct non récolté est absent ou invalide (`REQ-TERM-002`).
    #[error("Identité de processus non vérifiable (IDENTITY_UNVERIFIED) : {reason}")]
    IdentityUnverified {
        /// Motif vérifiable (jamais de secret).
        reason: String,
    },

    /// La commande provient d'une autre lignée d'autorité (`authority_generation`) (`REQ-TERM-003`).
    #[error(
        "Lignée d'autorité obsolète (STALE_AUTHORITY_GENERATION) : attendue '{expected}', reçue '{actual}'"
    )]
    StaleAuthorityGeneration {
        /// Lignée active du host.
        expected: String,
        /// Lignée reçue.
        actual: String,
    },

    /// L'époque daemon ou le fence est inférieur au dernier état accepté dans la même lignée (`REQ-TERM-003`).
    #[error("Époque ou fence obsolète (STALE_EPOCH) : {reason}")]
    StaleEpochOrFence {
        /// Détail de comparaison dans la lignée.
        reason: String,
    },

    /// Échec d'authentification du pair Unix ou du secret de contrôle.
    #[error("Authentification refusée sur le canal privé process-host : {reason}")]
    AuthenticationFailed {
        /// Motif sans révéler le secret attendu ni le secret reçu.
        reason: String,
    },

    /// Coupure après écriture au PTY avant reçu durable : répétition automatique interdite (`REQ-TERM-005`).
    #[error(
        "Issue d'entrée terminal inconnue pour input_seq={input_seq} (action={action_id}) : {reason}"
    )]
    TerminalInputOutcomeUnknown {
        /// Numéro de séquence de l'entrée ambiguë.
        input_seq: u64,
        /// Action associée.
        action_id: ActionId,
        /// Motif technique de l'incertitude.
        reason: String,
    },

    /// Issue du lancement non confirmable après reprise (`LaunchOutcomeUnknown`).
    #[error("Issue de lancement du processus inconnue (LAUNCH_OUTCOME_UNKNOWN) : {reason}")]
    LaunchOutcomeUnknown {
        /// Détail diagnostique.
        reason: String,
    },

    /// Backend d'isolation OS requis absent en mode strict (`Q-05`).
    #[error(
        "Backend d'isolation OS '{backend}' absent (ISOLATION_BACKEND_MISSING) : {remediation}"
    )]
    IsolationBackendMissing {
        /// Backend requis (`macos-seatbelt` ou `linux-bubblewrap-seccomp`).
        backend: String,
        /// Remédiation explicite pour l'utilisateur.
        remediation: String,
    },

    /// Obligation d'isolation requise non imposable par le backend OS actif (`Q-05`).
    #[error(
        "Obligation d'isolation '{backend}' impossible à imposer (ISOLATION_OBLIGATION_UNENFORCEABLE) : {reason} ({remediation})"
    )]
    IsolationObligationUnenforceable {
        /// Backend concerné.
        backend: String,
        /// Cause technique précise.
        reason: String,
        /// Remédiation explicite.
        remediation: String,
    },

    /// Tentative d'invocation modèle depuis un process-host strictement interdite (`REQ-TERM-004`).
    #[error("Appel modèle autonome interdit dans process-host (MODEL_CALLS_FORBIDDEN) : {reason}")]
    ModelCallsForbidden {
        /// Détail de la tentative refusée.
        reason: String,
    },

    /// Terminal ou host indisponible (`TERMINAL_UNAVAILABLE`).
    #[error("Terminal ou host indisponible (TERMINAL_UNAVAILABLE) : {reason}")]
    TerminalUnavailable {
        /// Motif d'indisponibilité.
        reason: String,
    },

    /// Curseur de lecture de logs expiré suite à une saturation/rotation (`CURSOR_EXPIRED`).
    #[error(
        "Curseur de logs expiré : séquence demandée {requested_seq}, plus ancienne disponible {oldest_available_seq}"
    )]
    LogCursorExpired {
        /// Séquence demandée par le lecteur.
        requested_seq: u64,
        /// Plus ancienne séquence encore disponible dans le spool.
        oldest_available_seq: u64,
    },

    /// Conflit d'idempotence sur une même séquence ou `command_id`.
    #[error("Conflit d'idempotence : {reason}")]
    IdempotencyConflict {
        /// Détail du conflit.
        reason: String,
    },

    /// Argument invalide ou hors bornes.
    #[error("Argument process-host invalide : {reason}")]
    InvalidArgument {
        /// Détail de validation.
        reason: String,
    },

    /// Erreur de la couche plateforme OS.
    #[error("Erreur plateforme OS : {0}")]
    Platform(#[from] PlatformError),

    /// Erreur du codec ou protocole IPC.
    #[error("Erreur protocole IPC : {0}")]
    Protocol(#[from] ProtocolError),

    /// Erreur de validation de fence du domaine.
    #[error("Erreur fence domaine : {0}")]
    Fence(#[from] FenceError),

    /// Erreur de sérialisation/désérialisation JSON.
    #[error("Erreur JSON process-host : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur I/O système.
    #[error("Erreur I/O process-host : {0}")]
    Io(#[from] std::io::Error),
}

impl ProcessHostError {
    /// Projette cette erreur vers le format filaire canonique [`IpcError`].
    #[must_use]
    pub fn to_ipc_error(&self, command_id: Option<CommandId>) -> IpcError {
        let mut ipc = match self {
            Self::IdentityUnverified { .. } => {
                IpcError::from_reason(ReasonCode::IdentityUnverified, self.to_string())
            }
            Self::StaleAuthorityGeneration { .. }
            | Self::Fence(FenceError::LineageMismatch { .. }) => {
                IpcError::from_reason(ReasonCode::StaleAuthorityGeneration, self.to_string())
            }
            Self::StaleEpochOrFence { .. }
            | Self::Fence(FenceError::StaleFence { .. } | FenceError::IncomparableLineage) => {
                IpcError::from_reason(ReasonCode::StaleEpoch, self.to_string())
            }
            Self::AuthenticationFailed { .. } => IpcError::new(
                ErrorCode::AuthenticationFailed,
                None,
                self.to_string(),
                RetryClass::Never,
            ),
            Self::TerminalInputOutcomeUnknown { action_id, .. } => {
                IpcError::from_reason(ReasonCode::TerminalInputUnknown, self.to_string())
                    .with_details_ref(action_id.to_string())
            }
            Self::LaunchOutcomeUnknown { .. } => {
                IpcError::from_reason(ReasonCode::LaunchOutcomeUnknown, self.to_string())
            }
            Self::IsolationBackendMissing { backend, .. } => {
                IpcError::from_reason(ReasonCode::IsolationBackendMissing, self.to_string())
                    .with_details_ref(backend.clone())
            }
            Self::IsolationObligationUnenforceable { backend, .. } => IpcError::from_reason(
                ReasonCode::IsolationObligationUnenforceable,
                self.to_string(),
            )
            .with_details_ref(backend.clone()),
            Self::ModelCallsForbidden { .. } => {
                IpcError::from_reason(ReasonCode::ModelCallsForbidden, self.to_string())
            }
            Self::TerminalUnavailable { .. } => {
                IpcError::from_reason(ReasonCode::TerminalUnavailable, self.to_string())
            }
            Self::LogCursorExpired { .. } => {
                IpcError::from_reason(ReasonCode::CursorExpired, self.to_string())
            }
            Self::IdempotencyConflict { .. } => IpcError::new(
                ErrorCode::IdempotencyConflict,
                None,
                self.to_string(),
                RetryClass::Never,
            ),
            Self::Platform(err) => platform_to_ipc(err, self.to_string()),
            Self::Protocol(ProtocolError::ProtocolIncompatible { .. }) => IpcError::new(
                ErrorCode::ProtocolIncompatible,
                None,
                self.to_string(),
                RetryClass::Never,
            ),
            Self::InvalidArgument { .. } | Self::Protocol(_) => IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                self.to_string(),
                RetryClass::Never,
            ),
            Self::Json(_) | Self::Io(_) => IpcError::new(
                ErrorCode::InternalFailure,
                None,
                self.to_string(),
                RetryClass::AfterReconciliation,
            ),
        };
        if let Some(cid) = command_id {
            ipc = ipc.with_command_id(cid);
        }
        ipc
    }
}

fn platform_to_ipc(err: &PlatformError, msg: String) -> IpcError {
    match err {
        PlatformError::PeerUidMismatch { .. } => IpcError::new(
            ErrorCode::AuthenticationFailed,
            None,
            msg,
            RetryClass::Never,
        ),
        PlatformError::ForbiddenDocumentationPath { .. }
        | PlatformError::InvalidPath { .. }
        | PlatformError::SocketPathTooLong { .. }
        | PlatformError::SymlinkNotAllowed { .. } => {
            IpcError::new(ErrorCode::InvalidArgument, None, msg, RetryClass::Never)
        }
        _ => IpcError::new(
            ErrorCode::InternalFailure,
            None,
            msg,
            RetryClass::AfterReconciliation,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_ipc_projections_match_canonical_catalog() {
        let e1 = ProcessHostError::IdentityUnverified {
            reason: "pid reused".into(),
        };
        let ipc1 = e1.to_ipc_error(None);
        assert_eq!(ipc1.code, ErrorCode::CapabilityDenied);
        assert_eq!(ipc1.reason_code, Some(ReasonCode::IdentityUnverified));

        let e2 = ProcessHostError::StaleAuthorityGeneration {
            expected: "gen-a".into(),
            actual: "gen-b".into(),
        };
        let ipc2 = e2.to_ipc_error(None);
        assert_eq!(ipc2.code, ErrorCode::StaleFence);
        assert_eq!(ipc2.reason_code, Some(ReasonCode::StaleAuthorityGeneration));
        assert_eq!(ipc2.retry_class, RetryClass::Never);

        let e3 = ProcessHostError::TerminalInputOutcomeUnknown {
            input_seq: 4,
            action_id: ActionId::from_u128(99),
            reason: "host killed before receipt".into(),
        };
        let ipc3 = e3.to_ipc_error(Some(CommandId::from_u128(12)));
        assert_eq!(ipc3.code, ErrorCode::OutcomeUnknown);
        assert_eq!(ipc3.reason_code, Some(ReasonCode::TerminalInputUnknown));
        assert_eq!(ipc3.retry_class, RetryClass::AfterReconciliation);
        assert_eq!(ipc3.command_id, Some(CommandId::from_u128(12)));

        let e4 = ProcessHostError::IsolationBackendMissing {
            backend: "macos-seatbelt".into(),
            remediation: "repasser explicitement en standard".into(),
        };
        let ipc4 = e4.to_ipc_error(None);
        assert_eq!(ipc4.code, ErrorCode::IsolationUnavailable);
        assert_eq!(ipc4.reason_code, Some(ReasonCode::IsolationBackendMissing));
        assert_eq!(ipc4.retry_class, RetryClass::AfterHumanDecision);

        let e5 = ProcessHostError::IsolationObligationUnenforceable {
            backend: "macos-seatbelt".into(),
            reason: "sandbox_apply failed".into(),
            remediation: "repasser en standard".into(),
        };
        let ipc5 = e5.to_ipc_error(None);
        assert_eq!(ipc5.code, ErrorCode::IsolationUnavailable);
        assert_eq!(
            ipc5.reason_code,
            Some(ReasonCode::IsolationObligationUnenforceable)
        );
    }
}
