//! Hiérarchie organisationnelle et états des agents logiques (`session-model.md`).

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// Niveau d'un agent logique dans la hiérarchie N+2 / N+1 / N (INV-007, ADR-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLevel {
    /// Agent d'exécution opérationnelle (niveau N).
    N,
    /// Leader de mission / manager opérationnel (niveau N+1).
    NPlus1,
    /// Arbitre organisationnel d'objectif / leader stratégique (niveau N+2).
    NPlus2,
}

impl fmt::Display for AgentLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::N => write!(f, "N"),
            Self::NPlus1 => write!(f, "N+1"),
            Self::NPlus2 => write!(f, "N+2"),
        }
    }
}

/// États d'un agent logique (`agents.state`).
///
/// **Attention :** Ces états ne suffisent pas à compter les slots d'admission ;
/// seul le registre d'admission fait foi (`session-model.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Dormant : agent inactif en mémoire/stockage, ne consomme aucun slot.
    Dormant,
    /// Éligible : prêt à être activé sur une mission ou une tâche.
    Eligible,
    /// Actif : possède ou attend une invocation active.
    Active,
    /// Bloqué : en attente d'une décision supérieure ou d'un incident.
    Blocked,
    /// En pause.
    Paused,
    /// Retiré : agent archivé, plus aucune nouvelle invocation autorisée (terminal).
    Retired,
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dormant => write!(f, "dormant"),
            Self::Eligible => write!(f, "eligible"),
            Self::Active => write!(f, "active"),
            Self::Blocked => write!(f, "blocked"),
            Self::Paused => write!(f, "paused"),
            Self::Retired => write!(f, "retired"),
        }
    }
}

impl AgentState {
    /// Indique si l'agent est retiré de manière définitive.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        matches!(self, Self::Retired)
    }
}

/// Effectue une transition pure d'état d'un agent logique.
///
/// # Errors
/// - Retourne `DomainTransitionError::TerminalImmutable` si l'agent est déjà dans l'état `Retired`.
/// - Retourne `DomainTransitionError::InvalidTransition` si le changement est illégal.
pub fn transition_agent_state(
    current: AgentState,
    target: AgentState,
) -> Result<AgentState, DomainTransitionError> {
    if current.is_retired() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Agent",
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            AgentState::Dormant,
            AgentState::Eligible | AgentState::Active | AgentState::Retired,
        ) | (
            AgentState::Eligible | AgentState::Blocked | AgentState::Paused,
            AgentState::Active | AgentState::Dormant | AgentState::Retired,
        ) | (
            AgentState::Active,
            AgentState::Dormant | AgentState::Blocked | AgentState::Paused | AgentState::Retired,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Agent",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état Agent".into(),
        })
    }
}
