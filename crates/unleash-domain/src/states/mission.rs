//! Machines d'état pour les objectifs (`Goal`) et les missions (`Mission`).
//!
//! Spécifie les états d'engagement métier, les politiques de complétion et leurs règles de transition.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;
use crate::states::policy::ModelPolicy;

/// États d'un objectif d'organisation (`Goal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    /// Objectif en cours de définition / cadrage initial.
    Draft,
    /// Objectif actif dont les missions sont en cours d'exécution.
    Active,
    /// Objectif bloqué sur incident, décision ou dépendance non résolue.
    Blocked,
    /// Objectif atteint avec succès selon ses critères de réussite explicites (terminal).
    Achieved,
    /// Objectif formellement annulé (terminal).
    Cancelled,
}

impl fmt::Display for GoalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Active => write!(f, "active"),
            Self::Blocked => write!(f, "blocked"),
            Self::Achieved => write!(f, "achieved"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl GoalState {
    /// Indique si l'objectif est dans un état terminal irréversible.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Achieved | Self::Cancelled)
    }
}

/// États d'une mission métier (`Mission`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    /// Mission proposée, en attente de validation de scope ou de cadrage.
    Proposed,
    /// Mission prête à être engagée.
    Ready,
    /// Mission active exécutant un ou plusieurs runs de workflow.
    Active,
    /// Mission mise en pause explicite.
    Paused,
    /// Mission bloquée sur une dépendance ou un blocage de run non résolu.
    Blocked,
    /// Mission terminée avec succès (terminal).
    Succeeded,
    /// Mission en échec après réconciliation (terminal).
    Failed,
    /// Mission annulée formellement (terminal).
    Cancelled,
}

impl fmt::Display for MissionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proposed => write!(f, "proposed"),
            Self::Ready => write!(f, "ready"),
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Blocked => write!(f, "blocked"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl MissionState {
    /// Indique si la mission est dans un état terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Politique de jugement et de clôture d'une mission ou d'un objectif (`completion_policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPolicy {
    /// Clôture mécanique basée sur les conditions du workflow ou du test sans modèle.
    Mechanical,
    /// Clôture validée par une décision humaine explicite.
    Human,
    /// Clôture jugée par un agent propriétaire N+1 / N+2 (strictement interdit sous `ModelPolicy::Forbidden`).
    OwnerAgent,
}

impl fmt::Display for CompletionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mechanical => write!(f, "mechanical"),
            Self::Human => write!(f, "human"),
            Self::OwnerAgent => write!(f, "owner_agent"),
        }
    }
}

impl CompletionPolicy {
    /// Valide la compatibilité de la politique de complétion avec la politique modèle du contexte.
    ///
    /// # Errors
    /// Retourne `DomainTransitionError::PolicyViolation` si `OwnerAgent` est configuré sous `ModelPolicy::Forbidden`.
    pub fn validate_with_model_policy(
        &self,
        model_policy: ModelPolicy,
    ) -> Result<(), DomainTransitionError> {
        if *self == Self::OwnerAgent && model_policy == ModelPolicy::Forbidden {
            return Err(DomainTransitionError::PolicyViolation {
                reason: "La politique de complétion 'owner_agent' est strictement interdite sous 'model_policy=forbidden'".into(),
            });
        }
        Ok(())
    }
}

/// Effectue une transition pure d'état de Goal.
///
/// # Errors
/// Retourne `DomainTransitionError::TerminalImmutable` si l'état actuel est déjà terminal.
/// Retourne `DomainTransitionError::InvalidTransition` si la transition demandée est invalide.
pub fn transition_goal_state(
    current: GoalState,
    target: GoalState,
) -> Result<GoalState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Goal",
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            GoalState::Draft | GoalState::Blocked,
            GoalState::Active | GoalState::Cancelled,
        ) | (
            GoalState::Active,
            GoalState::Blocked | GoalState::Achieved | GoalState::Cancelled,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Goal",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état Goal (state-machines.md)".into(),
        })
    }
}

/// Effectue une transition pure d'état de Mission.
///
/// # Errors
/// Retourne `DomainTransitionError::TerminalImmutable` si l'état actuel est déjà terminal.
/// Retourne `DomainTransitionError::InvalidTransition` si la transition demandée est invalide.
pub fn transition_mission_state(
    current: MissionState,
    target: MissionState,
) -> Result<MissionState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Mission",
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            MissionState::Proposed,
            MissionState::Ready | MissionState::Cancelled
        ) | (
            MissionState::Ready | MissionState::Paused,
            MissionState::Active | MissionState::Cancelled,
        ) | (
            MissionState::Active,
            MissionState::Paused
                | MissionState::Blocked
                | MissionState::Succeeded
                | MissionState::Failed
                | MissionState::Cancelled,
        ) | (
            MissionState::Blocked,
            MissionState::Active | MissionState::Failed | MissionState::Cancelled,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Mission",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état Mission (state-machines.md)".into(),
        })
    }
}
