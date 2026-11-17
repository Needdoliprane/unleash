//! Machine d'état d'une étape de workflow (`StepState`).
//!
//! Spécifie les états d'une activation d'étape de workflow selon `state-machines.md`.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// États d'une activation d'étape de workflow (`step_instances.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Prête : dépendances amont satisfaites, prête pour admission/file.
    Ready,
    /// En file d'admission.
    Queued,
    /// En cours d'exécution active par un worker ou broker.
    Running,
    /// En attente d'une preuve d'observation ou d'intervention humaine (`AUTOMATION_HUMAN_STEP_REQUIRED`).
    WaitingEvidence,
    /// En attente d'une décision d'arbitrage leader ou d'approbation.
    AwaitingDecision,
    /// Bloquée sur incident ou dépendance.
    Blocked,
    /// Terminée avec succès vérifié par contrat (terminal).
    Succeeded,
    /// Terminée en échec après épuisement des tentatives ou erreur irréversible (terminal).
    Failed,
    /// Annulée (terminal).
    Cancelled,
}

impl fmt::Display for StepState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => write!(f, "ready"),
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::WaitingEvidence => write!(f, "waiting_evidence"),
            Self::AwaitingDecision => write!(f, "awaiting_decision"),
            Self::Blocked => write!(f, "blocked"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl StepState {
    /// Indique si l'étape est dans un état terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Effectue une transition pure d'état d'étape.
///
/// # Errors
/// - Retourne `DomainTransitionError::TerminalImmutable` si l'état actuel est déjà terminal.
/// - Retourne `DomainTransitionError::InvalidTransition` si le changement est illégal.
pub fn transition_step(
    current: StepState,
    target: StepState,
) -> Result<StepState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Step",
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            StepState::Ready,
            StepState::Queued | StepState::Running | StepState::Cancelled
        ) | (
            StepState::Queued | StepState::WaitingEvidence | StepState::AwaitingDecision,
            StepState::Running | StepState::Blocked | StepState::Cancelled,
        ) | (
            StepState::Running,
            StepState::WaitingEvidence
                | StepState::AwaitingDecision
                | StepState::Blocked
                | StepState::Succeeded
                | StepState::Failed
                | StepState::Cancelled,
        ) | (
            StepState::Blocked,
            StepState::Ready
                | StepState::Queued
                | StepState::Running
                | StepState::Failed
                | StepState::Cancelled,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Step",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état Step".into(),
        })
    }
}
