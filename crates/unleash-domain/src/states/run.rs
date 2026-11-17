//! Machine d'état du Workflow Run (`state-machines.md`).
//!
//! Exécute les transitions pures d'un run, valide la terminalité irréversible
//! (`TerminalImmutable`), la contrainte de prédécesseur terminal (`PREDECESSOR_NOT_TERMINAL`),
//! et la progression de diagnostic sous `EffectUnknown`.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;

/// États d'un run de workflow (`workflow_runs.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Préparé : définitions et liaisons engagées, budgets alloués, prêt à démarrer.
    Prepared,
    /// En cours d'exécution active.
    Running,
    /// En pause après réconciliation propre des activités en vol.
    Paused,
    /// Bloqué sur preuve manquante, autorisation ou effet externe inconnu (`EffectUnknown`).
    Blocked,
    /// En cours d'annulation et de neutralisation des activités actives.
    Cancelling,
    /// Clôturé avec succès : le body et la clause de fin (`done`) sont vérifiés (terminal).
    Succeeded,
    /// Clôturé en échec connu après arrêt propre des activités (terminal).
    Failed,
    /// Clôturé suite à une annulation réconciliée (terminal).
    Cancelled,
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepared => write!(f, "prepared"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Blocked => write!(f, "blocked"),
            Self::Cancelling => write!(f, "cancelling"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl RunState {
    /// Indique si l'état est terminal. Un état terminal ne peut plus JAMAIS être rouvert ni muté (INV-018, Q-03).
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Valeur de vérité d'une garde de fin ou d'un critère de succès (`success_when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardTruth {
    /// Fait prouvé et vérifié vrai.
    True,
    /// Fait prouvé et vérifié faux.
    False,
    /// Fait requis absent, périmé, masqué ou indisponible.
    Unknown,
}

impl fmt::Display for GuardTruth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::True => write!(f, "true"),
            Self::False => write!(f, "false"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Événements ou commandes causant des transitions dans le cycle de vie d'un run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// Commande `StartRun` : démarre le run préparé.
    StartRun,

    /// Commande `PauseRun` : demande la suspension du run.
    PauseRun {
        /// Indique si toutes les activités en vol ont été réconciliées immédiatement.
        activities_reconciled: bool,
    },

    /// Commande `ResumeRun` : reprend un run en pause.
    ResumeRun,

    /// Événement `NeedEvidence` : une preuve indispensable est manquante.
    NeedEvidence,

    /// Événement `NeedAuthorization` : une approbation ou autorisation est requise.
    NeedAuthorization,

    /// Événement `EffectUnknown` : l'issue d'une action externe est indéterminée.
    EffectUnknown,

    /// Commande `CancelRun` : demande l'arrêt d'urgence et le fencing du run.
    CancelRun,

    /// Événement `CancellationReconciled` : toutes les activités ont été arrêtées.
    CancellationReconciled,

    /// Événement `BodyClosed` : le corps d'exécution s'est achevé normalement.
    BodyClosed {
        /// Résultat d'évaluation de la garde `done` / `success_when`.
        done: GuardTruth,
    },

    /// Commande `ResolveBlock` : résolution d'un blocage par fourniture d'une preuve ou décision.
    ResolveBlock {
        /// Si le blocage portait sur une garde de complétion (`GUARD_UNKNOWN`), sa nouvelle évaluation.
        resolved_guard: Option<GuardTruth>,
        /// Indique si une intention de pause reste active après résolution.
        pause_requested: bool,
    },

    /// Dépassement de la deadline temporelle globale du run.
    DeadlineExceeded {
        /// Indique si les activités résiduelles sont fermées.
        activities_closed: bool,
    },

    /// Épuisement d'un quota budgétaire (appels, jetons ou financier).
    BudgetExhausted {
        /// Indique si les activités résiduelles sont fermées.
        activities_closed: bool,
    },

    /// Échec non traité dans le corps du workflow.
    UnhandledFailure {
        /// Indique si les activités résiduelles sont fermées.
        activities_closed: bool,
    },

    /// Progression diagnostique sous blocage : enregistrement d'une observation pure.
    ObservationRecorded,

    /// Progression diagnostique sous blocage : admission d'une tâche à mandat restreint.
    DiagnosticAdmitted,

    /// Progression diagnostique sous blocage : résolution d'une barrière d'action externe.
    EffectOutcomeResolved {
        /// Indique si toutes les barrières bloquantes de l'incident sont désormais levées.
        all_barriers_cleared: bool,
        /// Indique si une pause avait été demandée.
        pause_requested: bool,
    },
}

/// Valide qu'un run prédécesseur désigné (`predecessor_run_id`) est bien dans un état terminal.
///
/// # Errors
/// Retourne `DomainTransitionError::PredecessorNotTerminal` si le prédécesseur n'est pas encore terminal (Q-03).
pub fn validate_predecessor_run(predecessor_state: RunState) -> Result<(), DomainTransitionError> {
    if !predecessor_state.is_terminal() {
        return Err(DomainTransitionError::PredecessorNotTerminal {
            state: predecessor_state.to_string(),
        });
    }
    Ok(())
}

/// Effectue la transition pure d'état d'un Workflow Run selon la matrice canonique de `state-machines.md`.
///
/// # Errors
/// - Retourne `DomainTransitionError::TerminalImmutable` si le run est déjà dans un état terminal (INV-018).
/// - Retourne `DomainTransitionError::InvalidTransition` si l'événement n'est pas applicable à l'état courant.
#[expect(
    clippy::too_many_lines,
    reason = "Couvre exhaustivement les transitions canoniques du Run selon state-machines.md"
)]
pub fn transition_run(
    current: RunState,
    event: RunEvent,
) -> Result<RunState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Run",
            state: current.to_string(),
        });
    }

    match (current, event) {
        // prepared + StartRun / paused + ResumeRun -> running
        (RunState::Prepared, RunEvent::StartRun) | (RunState::Paused, RunEvent::ResumeRun) => {
            Ok(RunState::Running)
        }

        // running + PauseRun -> paused (si réconcilié) ou blocked (intention pause)
        (
            RunState::Running,
            RunEvent::PauseRun {
                activities_reconciled,
            },
        ) => {
            if activities_reconciled {
                Ok(RunState::Paused)
            } else {
                Ok(RunState::Blocked)
            }
        }

        // running + (NeedEvidence | NeedAuthorization | EffectUnknown) -> blocked
        // running + BodyClosed (done inconnu) -> blocked (GUARD_UNKNOWN avec intention de clôture)
        // blocked + (ObservationRecorded | DiagnosticAdmitted) -> blocked
        (
            RunState::Running,
            RunEvent::NeedEvidence
            | RunEvent::NeedAuthorization
            | RunEvent::EffectUnknown
            | RunEvent::BodyClosed {
                done: GuardTruth::Unknown,
            },
        )
        | (RunState::Blocked, RunEvent::ObservationRecorded | RunEvent::DiagnosticAdmitted) => {
            Ok(RunState::Blocked)
        }

        // blocked + ResolveBlock -> running ou paused (ou terminal si résolution d'un done inconnu)
        (
            RunState::Blocked,
            RunEvent::ResolveBlock {
                resolved_guard,
                pause_requested,
            },
        ) => {
            if let Some(guard) = resolved_guard {
                match guard {
                    GuardTruth::True => Ok(RunState::Succeeded),
                    GuardTruth::False => Ok(RunState::Failed),
                    GuardTruth::Unknown => Ok(RunState::Blocked),
                }
            } else if pause_requested {
                Ok(RunState::Paused)
            } else {
                Ok(RunState::Running)
            }
        }

        // prepared/running/paused/blocked + CancelRun -> cancelling
        (
            RunState::Prepared | RunState::Running | RunState::Paused | RunState::Blocked,
            RunEvent::CancelRun,
        ) => Ok(RunState::Cancelling),

        // cancelling + CancellationReconciled -> cancelled
        (RunState::Cancelling, RunEvent::CancellationReconciled) => Ok(RunState::Cancelled),

        // running + BodyClosed:
        // done vrai -> succeeded
        (
            RunState::Running,
            RunEvent::BodyClosed {
                done: GuardTruth::True,
            },
        ) => Ok(RunState::Succeeded),
        // done faux -> failed (DONE_NOT_MET)
        (
            RunState::Running,
            RunEvent::BodyClosed {
                done: GuardTruth::False,
            },
        ) => Ok(RunState::Failed),

        // running/blocked + DeadlineExceeded / BudgetExhausted / UnhandledFailure
        (
            RunState::Running | RunState::Blocked,
            RunEvent::DeadlineExceeded { activities_closed }
            | RunEvent::BudgetExhausted { activities_closed }
            | RunEvent::UnhandledFailure { activities_closed },
        ) => {
            if activities_closed {
                Ok(RunState::Failed)
            } else {
                Ok(RunState::Blocked)
            }
        }

        // blocked + EffectOutcomeResolved -> running/paused si all_barriers_cleared, sinon reste blocked
        (
            RunState::Blocked,
            RunEvent::EffectOutcomeResolved {
                all_barriers_cleared,
                pause_requested,
            },
        ) => {
            if all_barriers_cleared {
                if pause_requested {
                    Ok(RunState::Paused)
                } else {
                    Ok(RunState::Running)
                }
            } else {
                Ok(RunState::Blocked)
            }
        }

        (from, evt) => Err(DomainTransitionError::InvalidTransition {
            entity: "Run",
            from: from.to_string(),
            to: format!("{evt:?}"),
            reason: "Événement non applicable à l'état courant selon la machine d'état du Run"
                .into(),
        }),
    }
}
