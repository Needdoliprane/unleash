//! Machine d'état des actions auprès du broker (`failure-recovery.md` et `sqlite-design.md`).
//!
//! Spécifie les 12 états exacts d'action, les origines d'action (XOR strict),
//! les stratégies de réconciliation et l'interdiction absolue de retry automatique depuis `OutcomeUnknown` (INV-012, C4/C5).

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::errors::DomainTransitionError;
use crate::ids::{ActorId, AttemptId, InvocationId, ReplayWorkerId};

/// Origine obligatoire d'une action durable (XOR strict entre `Invocation`, `Replay` et `ControlPlane`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOrigin {
    /// Action initiée par une tentative d'invocation d'agent logique.
    Invocation {
        /// Invocation parente.
        invocation_id: InvocationId,
        /// Tentative courante de l'invocation.
        attempt_id: AttemptId,
    },
    /// Action initiée par un worker mécanique de replay sans modèle.
    Replay {
        /// Identifiant du worker de replay.
        replay_worker_id: ReplayWorkerId,
    },
    /// Action déclenchée directement par le plan de contrôle (commande système ou administrative).
    ControlPlane {
        /// Principal acteur émetteur.
        control_principal_id: ActorId,
    },
}

/// Les 12 états exacts d'une action durable (`actions.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    /// Intention durable préparée et persistée avant soumission.
    Prepared,
    /// En cours d'évaluation par le broker de politique ou le garde de sécurité.
    Authorizing,
    /// Décision humaine requise sur le périmètre exact d'approbation.
    ApprovalRequired,
    /// Action autorisée ; prête pour engagement de dispatch.
    Authorized,
    /// Refusée formellement (terminal, aucun secret résolu ni dispatch).
    Denied,
    /// Marqueur durable de départ engagé avant toute opération d'entrée-sortie.
    Dispatching,
    /// Départ confirmé par l'adaptateur ou le transport.
    InFlight,
    /// Effet externe observé avec succès et attribué (terminal).
    ObservedSuccess,
    /// Effet externe observé en échec sans effet ou attribué (terminal, sauf réarmement explicite).
    ObservedFailure,
    /// Départ ou issue impossible à déterminer avec certitude (C4/C5).
    OutcomeUnknown,
    /// Annulation demandée pour une action en vol ou en dispatch.
    CancelRequested,
    /// Action annulée sans effet externe réalisé (terminal).
    Cancelled,
}

impl fmt::Display for ActionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepared => write!(f, "prepared"),
            Self::Authorizing => write!(f, "authorizing"),
            Self::ApprovalRequired => write!(f, "approval_required"),
            Self::Authorized => write!(f, "authorized"),
            Self::Denied => write!(f, "denied"),
            Self::Dispatching => write!(f, "dispatching"),
            Self::InFlight => write!(f, "in_flight"),
            Self::ObservedSuccess => write!(f, "observed_success"),
            Self::ObservedFailure => write!(f, "observed_failure"),
            Self::OutcomeUnknown => write!(f, "outcome_unknown"),
            Self::CancelRequested => write!(f, "cancel_requested"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl ActionState {
    /// Indique si l'état de l'action est terminal.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::ObservedSuccess | Self::Denied | Self::Cancelled)
    }
}

/// Stratégie de tolérance aux pannes et réconciliation d'un type d'action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileStrategy {
    /// Lecture pure pouvant être répétée sans effet de bord.
    ReadOnlyRetryable,
    /// Écriture locale atomique vérifiant l'état attendu (Compare-And-Set).
    LocalCompareAndSet,
    /// API distante avec clé d'idempotence formellement qualifiée par le fournisseur.
    ProviderIdempotency,
    /// Observation de l'état réel distant avant toute décision de reprise.
    ObserveThenDecide,
    /// Réconciliation manuelle ou humaine obligatoire.
    ManualReconciliation,
}

impl fmt::Display for ReconcileStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnlyRetryable => write!(f, "read_only_retryable"),
            Self::LocalCompareAndSet => write!(f, "local_compare_and_set"),
            Self::ProviderIdempotency => write!(f, "provider_idempotency"),
            Self::ObserveThenDecide => write!(f, "observe_then_decide"),
            Self::ManualReconciliation => write!(f, "manual_reconciliation"),
        }
    }
}

/// Résultat produit par une procédure de réconciliation d'action (`ReconcileAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileOutcome {
    /// Succès certain et prouvé de l'effet.
    ConfirmedSuccess,
    /// Échec certain sans aucun effet externe accompli.
    ConfirmedFailureNoEffect,
    /// L'opération distante est toujours en cours d'exécution active.
    StillRunning,
    /// Conflit détecté avec un autre acteur ou une modification concurrente.
    Conflict,
    /// Issue toujours indéterminée après examen des preuves.
    Unknown,
}

impl fmt::Display for ReconcileOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfirmedSuccess => write!(f, "confirmed_success"),
            Self::ConfirmedFailureNoEffect => write!(f, "confirmed_failure_no_effect"),
            Self::StillRunning => write!(f, "still_running"),
            Self::Conflict => write!(f, "conflict"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Classification d'un élément d'inventaire lors de la récupération après panne (`InspectRecovery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClassification {
    /// Intention préparée mais aucun marqueur de départ n'a été émis (C3).
    NotDispatched,
    /// Action en cours de traitement actif confirmée par réconciliation.
    InFlight,
    /// Succès confirmé avec preuves attribuables.
    ConfirmedSuccess,
    /// Échec confirmé sans effet résiduel.
    ConfirmedFailureNoEffect,
    /// Conflit d'état ou divergence détectée.
    Conflict,
    /// Effet impossible à affirmer ou exclure.
    Unknown,
}

impl fmt::Display for RecoveryClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDispatched => write!(f, "not_dispatched"),
            Self::InFlight => write!(f, "in_flight"),
            Self::ConfirmedSuccess => write!(f, "confirmed_success"),
            Self::ConfirmedFailureNoEffect => write!(f, "confirmed_failure_no_effect"),
            Self::Conflict => write!(f, "conflict"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Mode d'évaluation de la garde de sécurité AI-Guard (`guard_required` vs `guard_optional`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardMode {
    /// Garde strictement obligatoire (fail-closed, indisponibilité = refus).
    GuardRequired,
    /// Garde optionnelle (indisponibilité = bascule sur politique locale de repli).
    GuardOptional,
}

/// Événements causant des transitions dans la machine d'état d'action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionEvent {
    /// Soumission de l'intention préparée au broker d'action.
    SubmitToBroker,

    /// Autorisation accordée par le garde / politique sans obligation insatisfaite.
    GuardAllow,

    /// Refus formel par le garde / politique ou obligation insatisfaite.
    GuardDeny {
        /// Motif du refus.
        reason: String,
    },

    /// Approbation humaine requise par le protocole ou la politique.
    GuardRequireApproval,

    /// Garde indisponible ou en timeout.
    GuardUnavailable {
        /// Mode de configuration de la garde.
        mode: GuardMode,
        /// Si `GuardOptional`, décision locale pré-autorisée de repli (`Some(true)` pour autoriser, `Some(false)` pour refuser).
        fallback_decision: Option<bool>,
    },

    /// Décision rendue suite à une demande d'approbation humaine.
    HumanApprovalDecision {
        /// Approbation accordée ou refusée.
        approved: bool,
    },

    /// Réévaluation de l'autorisation suite à expiration, révocation ou modification de mandat.
    ReevaluateAuthorized {
        /// Si `true`, le droit d'agir est définitivement révoqué.
        revoked: bool,
    },

    /// Marqueur durable de dispatch persisté avant toute émission I/O.
    CommitDispatchMarker,

    /// Départ de la requête confirmé par l'adaptateur de transport.
    ConfirmDispatchDeparture,

    /// Résultat externe direct observé et attribué.
    ObserveResult {
        /// Succès ou échec de l'opération.
        success: bool,
    },

    /// Départ ou effet impossible à exclure (C4/C5 : timeout réseau, crash en vol).
    ObserveDepartureImpossibleToExclude,

    /// Application du résultat formel d'une réconciliation.
    ApplyReconciliation(ReconcileOutcome),

    /// Réarmement d'un nouveau dispatch après un échec sans effet prouvé (`ObservedFailure`).
    RetryAfterObservedFailure {
        /// La stratégie d'action autorise-t-elle explicitement un nouveau dispatch ?
        strategy_allows_retry: bool,
        /// L'autorisation initiale est-elle toujours valide sans réévaluation ?
        authorization_still_valid: bool,
    },

    /// Demande formelle d'annulation de l'action.
    CancelAction,
}

/// Effectue une transition pure dans la machine d'état d'action selon `failure-recovery.md`.
///
/// **Invariant critique (INV-012, C4/C5) :**
/// Aucune transition vers un nouveau dispatch ni aucun retry automatique n'est possible depuis `OutcomeUnknown`.
///
/// # Errors
/// - `DomainTransitionError::CannotRetryOutcomeUnknown` si une relance directe est tentée depuis `OutcomeUnknown`.
/// - `DomainTransitionError::TerminalImmutable` si l'action est déjà dans un état terminal.
/// - `DomainTransitionError::GuardUnavailableFailClosed` si le garde est indisponible sous `guard_required`.
/// - `DomainTransitionError::InvalidTransition` si la transition est illégale.
#[expect(
    clippy::too_many_lines,
    reason = "Couvre exhaustivement l'ensemble des transitions et règles d'action de failure-recovery.md"
)]
pub fn transition_action(
    current: ActionState,
    event: ActionEvent,
) -> Result<ActionState, DomainTransitionError> {
    if current.is_terminal() {
        return Err(DomainTransitionError::TerminalImmutable {
            entity: "Action",
            state: current.to_string(),
        });
    }

    match (current, event) {
        // prepared -> authorizing
        (ActionState::Prepared, ActionEvent::SubmitToBroker) => Ok(ActionState::Authorizing),

        // authorizing + GuardAllow -> authorized
        (ActionState::Authorizing, ActionEvent::GuardAllow) => Ok(ActionState::Authorized),

        // authorizing + GuardDeny -> denied
        (ActionState::Authorizing, ActionEvent::GuardDeny { .. }) => Ok(ActionState::Denied),

        // authorizing + GuardRequireApproval -> approval_required
        (ActionState::Authorizing, ActionEvent::GuardRequireApproval) => {
            Ok(ActionState::ApprovalRequired)
        }

        // authorizing + GuardUnavailable:
        // sous guard_required : reste authorizing, fail-closed
        (
            ActionState::Authorizing,
            ActionEvent::GuardUnavailable {
                mode: GuardMode::GuardRequired,
                ..
            },
        ) => Err(DomainTransitionError::GuardUnavailableFailClosed),

        // sous guard_optional : bascule sur fallback_decision
        (
            ActionState::Authorizing,
            ActionEvent::GuardUnavailable {
                mode: GuardMode::GuardOptional,
                fallback_decision,
            },
        ) => match fallback_decision {
            Some(true) => Ok(ActionState::Authorized),
            Some(false) => Ok(ActionState::Denied),
            None => Err(DomainTransitionError::GuardUnavailableFailClosed),
        },

        // approval_required + HumanApprovalDecision
        (ActionState::ApprovalRequired, ActionEvent::HumanApprovalDecision { approved }) => {
            if approved {
                Ok(ActionState::Authorized)
            } else {
                Ok(ActionState::Denied)
            }
        }

        // authorized + ReevaluateAuthorized
        (ActionState::Authorized, ActionEvent::ReevaluateAuthorized { revoked }) => {
            if revoked {
                Ok(ActionState::Denied)
            } else {
                Ok(ActionState::Authorizing)
            }
        }

        // authorized + CommitDispatchMarker -> dispatching
        (ActionState::Authorized, ActionEvent::CommitDispatchMarker) => {
            Ok(ActionState::Dispatching)
        }

        // dispatching + ConfirmDispatchDeparture -> in_flight
        (ActionState::Dispatching, ActionEvent::ConfirmDispatchDeparture) => {
            Ok(ActionState::InFlight)
        }

        // dispatching / in_flight + ObserveResult
        (
            ActionState::Dispatching | ActionState::InFlight,
            ActionEvent::ObserveResult { success },
        ) => {
            if success {
                Ok(ActionState::ObservedSuccess)
            } else {
                Ok(ActionState::ObservedFailure)
            }
        }

        // dispatching / in_flight + ObserveDepartureImpossibleToExclude -> outcome_unknown
        (
            ActionState::Dispatching | ActionState::InFlight,
            ActionEvent::ObserveDepartureImpossibleToExclude,
        ) => Ok(ActionState::OutcomeUnknown),

        // Réconciliation sur dispatching / in_flight / outcome_unknown
        (
            ActionState::Dispatching | ActionState::InFlight | ActionState::OutcomeUnknown,
            ActionEvent::ApplyReconciliation(outcome),
        ) => match outcome {
            ReconcileOutcome::ConfirmedSuccess => Ok(ActionState::ObservedSuccess),
            ReconcileOutcome::ConfirmedFailureNoEffect => Ok(ActionState::ObservedFailure),
            ReconcileOutcome::StillRunning => Ok(ActionState::InFlight),
            ReconcileOutcome::Conflict | ReconcileOutcome::Unknown => {
                Ok(ActionState::OutcomeUnknown)
            }
        },

        // cancel_requested + Réconciliation
        (ActionState::CancelRequested, ActionEvent::ApplyReconciliation(outcome)) => {
            match outcome {
                ReconcileOutcome::ConfirmedSuccess => Ok(ActionState::ObservedSuccess),
                ReconcileOutcome::ConfirmedFailureNoEffect => Ok(ActionState::Cancelled),
                ReconcileOutcome::StillRunning => Ok(ActionState::CancelRequested),
                ReconcileOutcome::Conflict | ReconcileOutcome::Unknown => {
                    Ok(ActionState::OutcomeUnknown)
                }
            }
        }

        // observed_failure + RetryAfterObservedFailure
        (
            ActionState::ObservedFailure,
            ActionEvent::RetryAfterObservedFailure {
                strategy_allows_retry,
                authorization_still_valid,
            },
        ) => {
            if !strategy_allows_retry {
                return Err(DomainTransitionError::InvalidTransition {
                    entity: "Action",
                    from: current.to_string(),
                    to: "dispatching".into(),
                    reason: "La stratégie d'action n'autorise pas de retry après échec".into(),
                });
            }

            if authorization_still_valid {
                Ok(ActionState::Dispatching)
            } else {
                Ok(ActionState::Authorizing)
            }
        }

        // Annulation depuis prepared, authorizing, approval_required, authorized -> cancelled
        (
            ActionState::Prepared
            | ActionState::Authorizing
            | ActionState::ApprovalRequired
            | ActionState::Authorized,
            ActionEvent::CancelAction,
        ) => Ok(ActionState::Cancelled),

        // Annulation depuis dispatching, in_flight -> cancel_requested
        (ActionState::Dispatching | ActionState::InFlight, ActionEvent::CancelAction) => {
            Ok(ActionState::CancelRequested)
        }

        // Invariant absolu : tentative de relance / dispatch depuis OutcomeUnknown sans réconciliation
        (
            ActionState::OutcomeUnknown,
            ActionEvent::CommitDispatchMarker
            | ActionEvent::ConfirmDispatchDeparture
            | ActionEvent::RetryAfterObservedFailure { .. },
        ) => Err(DomainTransitionError::CannotRetryOutcomeUnknown),

        (from, evt) => Err(DomainTransitionError::InvalidTransition {
            entity: "Action",
            from: from.to_string(),
            to: format!("{evt:?}"),
            reason: "Événement non permis pour l'état d'action courant".into(),
        }),
    }
}
