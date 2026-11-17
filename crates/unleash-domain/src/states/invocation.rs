//! Invocations durables et tentatives d'exécution (`session-model.md`).
//!
//! Spécifie la machine d'état de l'invocation, la règle stricte de possession de slot
//! d'admission (`owns_slot`), la classe d'admission, et l'irréversibilité des tentatives (`AttemptState`).

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// Raison d'attente durable ou de mise en file d'une invocation (`invocations.wait_reason`).
///
/// **Contrat canonique :** Ce sont des valeurs de raison, JAMAIS des états d'invocation (`session-model.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    /// Agent N attendant une décision ou un retour de son agent propriétaire/leader.
    WaitingOwner,
    /// Agent parent attendant l'achèvement de ses sous-agents enfants.
    WaitingChildren,
    /// En attente d'une validation d'authentification ou d'un secret externe.
    Authentication,
    /// En attente d'un événement sur un document ou artefact supervisé.
    DocumentEvent,
    /// En attente de déblocage ou réapprovisionnement budgétaire.
    Budget,
    /// En attente de l'échéance d'un minuteur durable (`workflow_timers`).
    Timer,
}

impl fmt::Display for WaitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WaitingOwner => write!(f, "waiting_owner"),
            Self::WaitingChildren => write!(f, "waiting_children"),
            Self::Authentication => write!(f, "authentication"),
            Self::DocumentEvent => write!(f, "document_event"),
            Self::Budget => write!(f, "budget"),
            Self::Timer => write!(f, "timer"),
        }
    }
}

/// Classe de priorité d'admission globale (`invocations.admission_class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionClass {
    /// Classe réservée aux arbitres et managers organisationnels (N+1 / N+2).
    Control,
    /// Classe ordinaire des travailleurs de calcul (agents N, workers de replay, probes).
    Work,
}

impl fmt::Display for AdmissionClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control => write!(f, "control"),
            Self::Work => write!(f, "work"),
        }
    }
}

/// États d'une invocation logique d'agent (`invocations.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationState {
    /// En file d'attente d'admission ou de dépendances (ne possède aucun slot).
    Queued,
    /// Réservations atomiques acquises dans le pool global, tentative imminente (possède un slot).
    Admitted,
    /// Worker actif et échange en cours avec le modèle ou un outil (possède un slot).
    Running,
    /// Préparation atomique d'un point de reprise durable (possède un slot jusqu'au commit).
    Checkpointing,
    /// En attente durable, canal modèle clos, worker arrêté ou désarmé (aucun slot).
    WaitingDurable,
    /// En cours de réconciliation après crash ou coupure (possède un slot tant que l'ancien exécuteur n'est pas neutralisé).
    Reconciling,
    /// Clôturée avec succès, sortie et preuves persistées (terminal, aucun slot).
    Completed,
    /// Clôturée en échec connu (terminal, aucun slot).
    Failed,
    /// Annulée (terminal, aucun slot).
    Cancelled,
}

impl fmt::Display for InvocationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Admitted => write!(f, "admitted"),
            Self::Running => write!(f, "running"),
            Self::Checkpointing => write!(f, "checkpointing"),
            Self::WaitingDurable => write!(f, "waiting_durable"),
            Self::Reconciling => write!(f, "reconciling"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl InvocationState {
    /// Indique si l'état est terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Détermine rigoureusement si l'invocation détient un permis/slot du pool d'admission global (INV-001, session-model.md).
    ///
    /// # Paramètres
    /// - `old_executor_neutralized` : si `true`, l'ancien exécuteur a été vérifié arrêté et neutralisé lors d'une réconciliation.
    #[must_use]
    pub fn owns_slot(&self, old_executor_neutralized: bool) -> bool {
        match self {
            Self::Admitted | Self::Running | Self::Checkpointing => true,
            Self::Reconciling => !old_executor_neutralized,
            Self::Queued
            | Self::WaitingDurable
            | Self::Completed
            | Self::Failed
            | Self::Cancelled => false,
        }
    }
}

/// États d'une tentative d'exécution physique d'une invocation (`invocation_attempts.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    /// Tentative admise, prête à démarrer.
    Admitted,
    /// Tentative active en cours d'exécution.
    Running,
    /// Tentative réalisant un checkpoint durable.
    Checkpointing,
    /// Tentative en cours d'inventaire ou de réconciliation.
    Reconciling,
    /// Tentative achevée normalement (terminal).
    Ended,
    /// Tentative terminée sur erreur de tentative (terminal).
    Failed,
    /// Tentative clôturée par clôture d'autorité ou fence supérieur (terminal).
    Fenced,
}

impl fmt::Display for AttemptState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admitted => write!(f, "admitted"),
            Self::Running => write!(f, "running"),
            Self::Checkpointing => write!(f, "checkpointing"),
            Self::Reconciling => write!(f, "reconciling"),
            Self::Ended => write!(f, "ended"),
            Self::Failed => write!(f, "failed"),
            Self::Fenced => write!(f, "fenced"),
        }
    }
}

impl AttemptState {
    /// Indique si la tentative est actuellement active (admitted, running, checkpointing, reconciling).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Admitted | Self::Running | Self::Checkpointing | Self::Reconciling
        )
    }

    /// Indique si la tentative est dans un état terminal (`ended`, `failed`, `fenced`).
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Ended | Self::Failed | Self::Fenced)
    }
}

/// Effectue une transition pure d'état d'une tentative physique d'exécution.
///
/// **Invariant critique (session-model.md) :**
/// Une tentative terminale (`ended`, `failed`, `fenced`) ne redevient JAMAIS active.
///
/// # Errors
/// - Retourne `DomainTransitionError::AttemptIrreversible` si la tentative courante est déjà terminale.
/// - Retourne `DomainTransitionError::InvalidTransition` si le changement est illégal.
pub fn transition_attempt(
    current: AttemptState,
    target: AttemptState,
) -> Result<AttemptState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::AttemptIrreversible {
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            AttemptState::Admitted,
            AttemptState::Running | AttemptState::Failed | AttemptState::Fenced,
        ) | (
            AttemptState::Running | AttemptState::Checkpointing,
            AttemptState::Running
                | AttemptState::Checkpointing
                | AttemptState::Reconciling
                | AttemptState::Ended
                | AttemptState::Failed
                | AttemptState::Fenced,
        ) | (
            AttemptState::Reconciling,
            AttemptState::Ended | AttemptState::Failed | AttemptState::Fenced,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Attempt",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état de tentative".into(),
        })
    }
}

/// Effectue une transition pure d'état d'une invocation logique selon `session-model.md`.
///
/// # Errors
/// - Retourne `DomainTransitionError::TerminalImmutable` si l'invocation est déjà dans un état terminal.
/// - Retourne `DomainTransitionError::InvalidTransition` si le changement est illégal.
pub fn transition_invocation_state(
    current: InvocationState,
    target: InvocationState,
) -> Result<InvocationState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Invocation",
            state: current.to_string(),
        });
    }

    let allowed = matches!(
        (current, target),
        (
            InvocationState::Queued,
            InvocationState::Admitted | InvocationState::Cancelled
        ) | (
            InvocationState::Admitted,
            InvocationState::Running | InvocationState::Cancelled | InvocationState::Reconciling,
        ) | (
            InvocationState::Running | InvocationState::Checkpointing,
            InvocationState::Checkpointing
                | InvocationState::WaitingDurable
                | InvocationState::Running
                | InvocationState::Reconciling
                | InvocationState::Completed
                | InvocationState::Failed
                | InvocationState::Cancelled,
        ) | (
            InvocationState::WaitingDurable,
            InvocationState::Queued | InvocationState::Admitted | InvocationState::Cancelled,
        ) | (
            InvocationState::Reconciling,
            InvocationState::WaitingDurable
                | InvocationState::Queued
                | InvocationState::Admitted
                | InvocationState::Failed
                | InvocationState::Cancelled,
        )
    );

    if allowed {
        Ok(target)
    } else {
        Err(DomainTransitionError::InvalidTransition {
            entity: "Invocation",
            from: current.to_string(),
            to: target.to_string(),
            reason: "Transition interdite par la machine d'état d'invocation".into(),
        })
    }
}
