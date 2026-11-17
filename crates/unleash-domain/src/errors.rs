//! Types d'erreurs de domaine pour `unleash-domain`.
//!
//! Définit les erreurs typées pour les validations d'identifiants, les contrôles de fences
//! et les transitions d'états du domaine.

use thiserror::Error;

/// Erreur survenant lors d'une transition invalide ou rejetée dans une machine à états de domaine.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainTransitionError {
    /// La transition demandée entre deux états n'est pas autorisée par le contrat.
    #[error("Transition invalide pour '{entity}': de '{from}' vers '{to}' ({reason})")]
    InvalidTransition {
        /// Entité concernée (ex: "Daemon", "Run", "Action", etc.).
        entity: &'static str,
        /// État de départ.
        from: String,
        /// État cible rejeté.
        to: String,
        /// Motif du refus.
        reason: String,
    },

    /// Tentative de mutation sur une entité déjà dans un état terminal (INV-018, Q-03).
    #[error(
        "Mutation interdite sur l'entité '{entity}' dans l'état terminal '{state}' (TERMINAL_IMMUTABLE)"
    )]
    TerminalImmutable {
        /// Entité concernée.
        entity: &'static str,
        /// État terminal observé.
        state: String,
    },

    /// Le run prédécesseur spécifié n'est pas dans un état terminal (`PREDECESSOR_NOT_TERMINAL`).
    #[error("Le run prédécesseur n'est pas terminal (état actuel: '{state}')")]
    PredecessorNotTerminal {
        /// État non terminal du run prédécesseur.
        state: String,
    },

    /// Interdiction absolue de retry automatique ou direct depuis `OutcomeUnknown` (INV-012, C4/C5).
    #[error(
        "Interdiction absolue de retry ou redépart direct depuis 'OutcomeUnknown' sans réconciliation"
    )]
    CannotRetryOutcomeUnknown,

    /// Violation d'une politique déclarée (par exemple `CompletionPolicy::OwnerAgent` sous `ModelPolicy::Forbidden`).
    #[error("Politique refusée: {reason}")]
    PolicyViolation {
        /// Motif de la violation.
        reason: String,
    },

    /// Garde de sécurité indisponible sous politique `guard_required` (fail-closed, jamais d'allow implicite).
    #[error("Garde de sécurité indisponible sous politique 'guard_required' (fail-closed)")]
    GuardUnavailableFailClosed,

    /// Une tentative terminale (`ended`, `failed`, `fenced`) ne peut plus jamais redevenir active.
    #[error("Tentative terminale irréversible (état: '{state}')")]
    AttemptIrreversible {
        /// État terminal de la tentative.
        state: String,
    },

    /// Lignée ou fence périmée lors d'une vérification.
    #[error("Fence périmée: {reason}")]
    StaleFence {
        /// Motif de rejet.
        reason: String,
    },

    /// Comparaison d'ordre impossible entre deux lignées d'autorité distinctes.
    #[error("Lignées d'autorité distinctes et incomparables")]
    IncomparableLineage,
}

/// Erreur de validation d'identifiant newtype ou digest.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdValidationError {
    /// Chaîne non conforme à la spécification UUID.
    #[error("UUID invalide: {0}")]
    InvalidUuid(String),

    /// Chaîne non conforme à un digest SHA-256 (64 caractères hex minuscules requis).
    #[error("Digest SHA-256 invalide: {0}")]
    InvalidSha256(String),

    /// Identifiant vide non autorisé pour ce type.
    #[error("L'identifiant '{0}' ne peut pas être vide")]
    EmptyIdentifier(&'static str),
}

/// Erreur lors de la vérification de monotonie de fences ou de lignées.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FenceError {
    /// Les deux fences proviennent de générations d'autorité différentes et sont incomparables.
    #[error("Incohérence de lignée d'autorité: attendue '{expected}', reçue '{actual}'")]
    LineageMismatch {
        /// Lignée attendue.
        expected: String,
        /// Lignée reçue.
        actual: String,
    },

    /// Le compteur d'époque ou de fence est inférieur au minimum requis dans la même lignée.
    #[error("Fence périmée: reçu {actual}, minimum requis {expected_min}")]
    StaleFence {
        /// Minimum attendu.
        expected_min: u64,
        /// Valeur reçue.
        actual: u64,
    },

    /// Tentative de comparaison d'ordre entre lignées distinctes.
    #[error("Comparaison d'ordre interdite entre lignées d'autorité distinctes")]
    IncomparableLineage,
}
