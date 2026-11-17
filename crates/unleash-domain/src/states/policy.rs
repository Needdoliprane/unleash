//! Politiques d'exécution, de modèles et états de publication.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// Politique d'autorisation des modèles d'intelligence artificielle (`model_policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPolicy {
    /// Interdiction totale d'appels modèles transitifs, résumés, compactions ou embeddings (INV-018).
    Forbidden,
    /// Appels modèles autorisés sous budgets stricts et checkpoints bornés.
    AllowedBounded,
}

impl fmt::Display for ModelPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Forbidden => write!(f, "forbidden"),
            Self::AllowedBounded => write!(f, "allowed_bounded"),
        }
    }
}

/// Contrainte de localité des données pour les appels modèles (`data_locality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataLocality {
    /// Les données doivent être traitées exclusivement sur la machine locale de l'utilisateur (INV-018).
    LocalOnly,
    /// Les destinations distantes expressément qualifiées et autorisées sont acceptées.
    ApprovedRemote,
}

impl fmt::Display for DataLocality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalOnly => write!(f, "local_only"),
            Self::ApprovedRemote => write!(f, "approved_remote"),
        }
    }
}

/// État de publication d'une procédure, d'un skill ou d'un workflow dans le registre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    /// Brouillon modifiable.
    Draft,
    /// Validé par l'analyse sémantique et prêt pour publication.
    Validated,
    /// Publié et disponible pour de nouveaux engagements/runs.
    Published,
    /// Déprécié : les runs existants continuent mais aucun nouveau run ne doit l'engager.
    Deprecated,
    /// Révoqué pour motif de sécurité : application immédiate aux futures actions.
    Revoked,
}

impl fmt::Display for PublicationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Validated => write!(f, "validated"),
            Self::Published => write!(f, "published"),
            Self::Deprecated => write!(f, "deprecated"),
            Self::Revoked => write!(f, "revoked"),
        }
    }
}

impl PublicationState {
    /// Indique si la ressource est accessible pour de nouveaux runs.
    #[must_use]
    pub const fn is_usable_for_new_runs(&self) -> bool {
        matches!(self, Self::Published)
    }

    /// Indique si la ressource a été révoquée.
    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        matches!(self, Self::Revoked)
    }
}

/// Effectue une transition pure d'état de publication selon `state-machines.md`.
///
/// # Errors
/// Retourne `DomainTransitionError::InvalidTransition` si le changement est illégal.
pub fn transition_publication_state(
    current: PublicationState,
    target: PublicationState,
) -> Result<PublicationState, DomainTransitionError> {
    let allowed = matches!(
        (current, target),
        (
            PublicationState::Draft,
            PublicationState::Validated | PublicationState::Draft
        ) | (
            PublicationState::Validated,
            PublicationState::Published | PublicationState::Draft
        ) | (
            PublicationState::Published,
            PublicationState::Deprecated | PublicationState::Revoked
        ) | (PublicationState::Deprecated, PublicationState::Revoked)
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Publication",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition de publication non autorisée".into(),
        })
    }
}
