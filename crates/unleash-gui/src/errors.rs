// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées de la frontière `WebView` <-> hôte Tauri `unleash-gui`.

use serde::Serialize;
use thiserror::Error;
use ts_rs::TS;
use unleash_client::ClientError;
use unleash_protocol::{ErrorCode, ReasonCode, RetryClass};

/// Erreur de la façade Tauri exposée au renderer sous forme structurée, sans jamais
/// divulguer de secret, de chemin socket interne ni de trace système sensible.
#[derive(Debug, Error)]
pub enum GuiError {
    /// L'opération demandée n'appartient pas au catalogue exposé au renderer GUI.
    #[error("Opération non autorisée pour le renderer GUI : {operation}")]
    ForbiddenOperation {
        /// Nom de l'opération refusée.
        operation: String,
    },

    /// La requête fournie par la `WebView` dépasse les bornes de taille ou viole la forme stricte.
    #[error("Entrée WebView invalide : {detail}")]
    InvalidInput {
        /// Description bornée du motif de rejet.
        detail: String,
    },

    /// Erreur provenant du client IPC commun ou du daemon.
    #[error("Erreur IPC backend : {0}")]
    Client(#[from] ClientError),

    /// Aucun client daemon configuré dans l'état de l'hôte.
    #[error("Client daemon non initialisé")]
    ClientNotConnected,

    /// Erreur du moteur Tauri.
    #[error("Erreur hôte Tauri : {0}")]
    Tauri(String),
}

/// Projection JSON sûre d'une [`GuiError`] retournée à la `WebView` par les commandes Tauri.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GuiErrorPayload {
    /// Classe d'erreur canonique alignée sur le protocole Unleash.
    pub code: ErrorCode,
    /// Raison typée si disponible.
    pub reason_code: Option<ReasonCode>,
    /// Stratégie de reprise client.
    pub retry_class: RetryClass,
    /// Message borné sans secret ni chemin système sensible.
    pub message: String,
}

impl From<&GuiError> for GuiErrorPayload {
    fn from(err: &GuiError) -> Self {
        match err {
            GuiError::ForbiddenOperation { .. } => Self {
                code: ErrorCode::CapabilityDenied,
                reason_code: Some(ReasonCode::UnauthorizedAction),
                retry_class: RetryClass::Never,
                message: err.to_string(),
            },
            GuiError::InvalidInput { .. } => Self {
                code: ErrorCode::InvalidArgument,
                reason_code: None,
                retry_class: RetryClass::Never,
                message: err.to_string(),
            },
            GuiError::Client(ClientError::Remote(ipc)) => Self {
                code: ipc.code,
                reason_code: ipc.reason_code,
                retry_class: ipc.retry_class,
                message: ipc.message.clone(),
            },
            GuiError::Client(other) => Self {
                code: ErrorCode::InternalFailure,
                reason_code: None,
                retry_class: RetryClass::AfterHumanDecision,
                message: other.to_string(),
            },
            GuiError::ClientNotConnected => Self {
                code: ErrorCode::InternalFailure,
                reason_code: None,
                retry_class: RetryClass::AfterBackoff,
                message: err.to_string(),
            },
            GuiError::Tauri(msg) => Self {
                code: ErrorCode::InternalFailure,
                reason_code: None,
                retry_class: RetryClass::AfterHumanDecision,
                message: msg.clone(),
            },
        }
    }
}

impl serde::Serialize for GuiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        GuiErrorPayload::from(self).serialize(serializer)
    }
}
