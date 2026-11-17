//! # Modèle d'erreurs protocolaire et DTO d'erreur IPC
//!
//! Responsabilité exclusive : Définition des 17 classes communes d'erreur [`ErrorCode`],
//! des 5 classes de reprise [`RetryClass`], de l'énumération fermée des [`ReasonCode`],
//! du DTO wire [`IpcError`] (conforme à `ipc-error.schema.json`) et de l'erreur
//! d'implémentation de la crate [`ProtocolError`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use ts_rs::TS;
use unleash_domain::{CommandId, Revision};

/// Erreur interne survenue lors du traitement, décodage ou validation protocolaire.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// Clé dupliquée détectée dans un objet JSON (interdit en décodage strict).
    #[error("Clé dupliquée '{key}' détectée")]
    DuplicateKey {
        /// Nom de la clé JSON dupliquée.
        key: String,
    },

    /// Version de protocole incompatible ou refusée (ex: `unleash.ipc.v1`).
    #[error("Version de protocole incompatible : '{version}' (version attendue : unleash.ipc.v2)")]
    ProtocolIncompatible {
        /// Version fournie dans la requête.
        version: String,
    },

    /// Dépassement du plafond de taille de trame (8 MiB = `8_388_608` octets).
    #[error("Trame trop volumineuse : {length} octets (plafond strict : {max} octets)")]
    FrameTooLarge {
        /// Longueur observée de la trame.
        length: u32,
        /// Plafond maximal autorisé.
        max: u32,
    },

    /// Fin de flux inattendue (EOF prématuré).
    #[error("Fin de flux inattendue lors de la lecture de la trame IPC")]
    UnexpectedEof,

    /// Erreur d'I/O sous-jacente synchrone.
    #[error("Erreur d'I/O protocolaire : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de syntaxe JSON ou de désérialisation Serde.
    #[error("Erreur de décodage JSON : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur survenue lors de la canonisation RFC 8785 (JCS).
    #[error("Erreur de canonisation JCS : {0}")]
    Jcs(String),

    /// Erreur de validation de contrat ou d'invariant.
    #[error("Erreur de validation protocolaire : {0}")]
    Validation(String),
}

/// Les dix-sept classes d'erreur communes canoniques (ADR-02 / `ipc-error.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Argument invalide ou schéma non respecté.
    InvalidArgument,
    /// Échec d'authentification du client ou du pair.
    AuthenticationFailed,
    /// Capacité requise absente, révoquée ou non attribuée.
    CapabilityDenied,
    /// Ressource ou projet hors du scope autorisé.
    ScopeDenied,
    /// Conflit de révision optimiste (`expected_revision` mismatch).
    RevisionConflict,
    /// Même `command_id` avec un payload/digest canonique différent.
    IdempotencyConflict,
    /// Ressource inexistante ou masquée pour préserver la sécurité.
    NotFoundOrHidden,
    /// Ressource momentanément occupée, verrouillée ou saturée (ex: backpressure).
    ResourceBusy,
    /// Dépassement de quota ou de budget d'exécution/modèle.
    BudgetExceeded,
    /// Limite de débit atteinte.
    RateLimited,
    /// Version de protocole incompatible (ex: tentative de soumission v1).
    ProtocolIncompatible,
    /// Barrière ou epoch obsolète (lignée serveur ou bail expiré).
    StaleFence,
    /// Confinement d'isolation ou de profil requis non disponible.
    IsolationUnavailable,
    /// Règle de contrat ou postcondition métier violée.
    ContractViolation,
    /// Issue d'un effet externe inconnue ; réconciliation obligatoire avant toute reprise.
    OutcomeUnknown,
    /// Panne ou indisponibilité interne non recouvrable automatiquement.
    InternalFailure,
    /// Décalage de flux nécessitant rechargement d'un snapshot et reprise de curseur.
    ResyncRequired,
}

impl ErrorCode {
    /// Renvoie le nom textuel wire canonique du code d'erreur.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidArgument => "InvalidArgument",
            Self::AuthenticationFailed => "AuthenticationFailed",
            Self::CapabilityDenied => "CapabilityDenied",
            Self::ScopeDenied => "ScopeDenied",
            Self::RevisionConflict => "RevisionConflict",
            Self::IdempotencyConflict => "IdempotencyConflict",
            Self::NotFoundOrHidden => "NotFoundOrHidden",
            Self::ResourceBusy => "ResourceBusy",
            Self::BudgetExceeded => "BudgetExceeded",
            Self::RateLimited => "RateLimited",
            Self::ProtocolIncompatible => "ProtocolIncompatible",
            Self::StaleFence => "StaleFence",
            Self::IsolationUnavailable => "IsolationUnavailable",
            Self::ContractViolation => "ContractViolation",
            Self::OutcomeUnknown => "OutcomeUnknown",
            Self::InternalFailure => "InternalFailure",
            Self::ResyncRequired => "ResyncRequired",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Les cinq classes de retry canoniques (`ipc-error.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetryClass {
    /// Aucun retry permis : refus définitif.
    Never,
    /// Retry autorisé après délai exponentiel (backoff).
    AfterBackoff,
    /// Retry autorisé après rafraîchissement de l'état ou du snapshot.
    AfterRefresh,
    /// Retry subordonné à une phase explicite de réconciliation.
    AfterReconciliation,
    /// Retry subordonné à une décision humaine explicite.
    AfterHumanDecision,
}

impl RetryClass {
    /// Renvoie la chaîne textuelle normalisée de la classe de reprise.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::AfterBackoff => "after_backoff",
            Self::AfterRefresh => "after_refresh",
            Self::AfterReconciliation => "after_reconciliation",
            Self::AfterHumanDecision => "after_human_decision",
        }
    }
}

impl fmt::Display for RetryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Catalogue fermé et typé des raisons d'erreur canoniques d'Unleash.
///
/// Invariants :
/// - Chaque raison appartient à exactement une classe [`ErrorCode`].
/// - Aucune raison ne porte le nom d'une classe commune.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[non_exhaustive]
pub enum ReasonCode {
    // --- RevisionConflict ---
    /// Conflit lors de la publication d'un artefact.
    #[serde(rename = "PUBLICATION_CONFLICT")]
    PublicationConflict,
    /// Conflit de version lors de l'application d'un contrat.
    #[serde(rename = "CONTRACT_VERSION_CONFLICT")]
    ContractVersionConflict,

    // --- StaleFence ---
    /// Époque d'exécution, de tentative ou de lease obsolète.
    #[serde(rename = "STALE_EPOCH")]
    StaleEpoch,
    /// Affectation de mission ou de goal réattribuée.
    #[serde(rename = "OWNER_EPOCH_CHANGED")]
    OwnerEpochChanged,
    /// Lignée serveur de la commande différente de la lignée active.
    #[serde(rename = "STALE_AUTHORITY_GENERATION")]
    StaleAuthorityGeneration,

    // --- ContractViolation ---
    /// Rôle ou slot déjà engagé dans une liaison immuable.
    #[serde(rename = "BINDING_ALREADY_ENGAGED")]
    BindingAlreadyEngaged,
    /// Référence de preuve devenue périmée ou non vérifiable.
    #[serde(rename = "STALE_EVIDENCE")]
    StaleEvidence,
    /// Critères de fin ou définition of done non satisfaits.
    #[serde(rename = "DONE_NOT_MET")]
    DoneNotMet,
    /// Borne d'itération maximale de boucle atteinte.
    #[serde(rename = "LOOP_EXHAUSTED")]
    LoopExhausted,
    /// Contrat formellement rejeté par le vérificateur.
    #[serde(rename = "CONTRACT_REJECTED")]
    ContractRejected,
    /// Tentative de mutation sur un agrégat terminal immuable.
    #[serde(rename = "TERMINAL_IMMUTABLE")]
    TerminalImmutable,
    /// Fenêtre d'idempotence expirée (tombstone minimal conservé).
    #[serde(rename = "IDEMPOTENCY_WINDOW_EXPIRED")]
    IdempotencyWindowExpired,
    /// Conflit lors de l'application d'un draft guidé.
    #[serde(rename = "APPLIED_DRAFT_CONFLICT")]
    AppliedDraftConflict,
    /// Tentative de modification d'un draft déjà appliqué.
    #[serde(rename = "APPLIED_DRAFT_IMMUTABLE")]
    AppliedDraftImmutable,
    /// Exécution de test attendue non fournie.
    #[serde(rename = "EXPECTED_EXECUTION_MISSING")]
    ExpectedExecutionMissing,
    /// Divergence entre observation d'exécution et attendu.
    #[serde(rename = "EVIDENCE_EXECUTION_MISMATCH")]
    EvidenceExecutionMismatch,
    /// Projection d'observation invalide ou corrompue.
    #[serde(rename = "EVIDENCE_PROJECTION_INVALID")]
    EvidenceProjectionInvalid,
    /// Observation ambiguë ne permettant pas de conclure.
    #[serde(rename = "EVIDENCE_AMBIGUOUS")]
    EvidenceAmbiguous,
    /// Divergence entre l'empreinte de snapshot et la preuve.
    #[serde(rename = "EVIDENCE_SNAPSHOT_MISMATCH")]
    EvidenceSnapshotMismatch,
    /// Affirmation formelle contredite par l'observation vérifiée.
    #[serde(rename = "CLAIM_CONTRADICTED")]
    ClaimContradicted,
    /// Reçu de recherche préalable obligatoire absent avant création.
    #[serde(rename = "DISCOVERY_REQUIRED")]
    DiscoveryRequired,
    /// Trace de recherche préalable manquante à la clôture.
    #[serde(rename = "DISCOVERY_EVIDENCE_MISSING")]
    DiscoveryEvidenceMissing,
    /// Restauration incomplète ou capsules manquantes.
    #[serde(rename = "RESTORE_INCOMPLETE")]
    RestoreIncomplete,
    /// Preuve obligatoire indisponible ou purgée.
    #[serde(rename = "EVIDENCE_UNAVAILABLE")]
    EvidenceUnavailable,
    /// Incompatibilité entre le mode d'automatisation et la politique de modèle.
    #[serde(rename = "AUTOMATION_MODE_POLICY_MISMATCH")]
    AutomationModePolicyMismatch,
    /// Bundle d'automatisation requis indisponible.
    #[serde(rename = "AUTOMATION_BUNDLE_UNAVAILABLE")]
    AutomationBundleUnavailable,
    /// Cible de l'automatisation absente ou introuvable.
    #[serde(rename = "AUTOMATION_TARGET_MISSING")]
    AutomationTargetMissing,
    /// Cible de l'automatisation ambiguë (plusieurs correspondances).
    #[serde(rename = "AUTOMATION_TARGET_AMBIGUOUS")]
    AutomationTargetAmbiguous,
    /// Dérive observée de l'identité de la cible métier.
    #[serde(rename = "AUTOMATION_TARGET_DRIFT")]
    AutomationTargetDrift,
    /// Contrainte métier violée avant dispatch.
    #[serde(rename = "BUSINESS_CONSTRAINT_VIOLATED")]
    BusinessConstraintViolated,
    /// Postcondition métier non vérifiée après dispatch.
    #[serde(rename = "BUSINESS_POSTCONDITION_FAILED")]
    BusinessPostconditionFailed,
    /// Incompatibilité de compte lors de l'exécution de l'automatisation.
    #[serde(rename = "AUTOMATION_ACCOUNT_MISMATCH")]
    AutomationAccountMismatch,
    /// État externe modifié de manière concurrente.
    #[serde(rename = "EXTERNAL_STATE_CHANGED")]
    ExternalStateChanged,
    /// Délai d'actionnabilité dépassé avant dispatch.
    #[serde(rename = "ACTIONABILITY_TIMEOUT")]
    ActionabilityTimeout,
    /// Délai d'assertion dépassé.
    #[serde(rename = "ASSERTION_TIMEOUT")]
    AssertionTimeout,

    // --- OutcomeUnknown ---
    /// Effet externe sans observation probante : statut incertain.
    #[serde(rename = "UNRESOLVED_EFFECT")]
    UnresolvedEffect,

    // --- CapabilityDenied ---
    /// Capacité ou runtime manquant.
    #[serde(rename = "CAPABILITY_UNAVAILABLE")]
    CapabilityUnavailable,
    /// Décision humaine ou autorité habilitée requise.
    #[serde(rename = "DECISION_AUTHORITY_REQUIRED")]
    DecisionAuthorityRequired,
    /// Définition ou version révoquée : nouveaux démarrages interdits.
    #[serde(rename = "DEFINITION_REVOKED")]
    DefinitionRevoked,
    /// Appel modèle strictement interdit sous la politique en vigueur.
    #[serde(rename = "MODEL_CALLS_FORBIDDEN")]
    ModelCallsForbidden,
    /// Opération non autorisée par le mandat ou l'ACL active.
    #[serde(rename = "OPERATION_NOT_AUTHORIZED")]
    OperationNotAuthorized,
    /// Identité du pair ou de l'exécuteur non vérifiable.
    #[serde(rename = "IDENTITY_UNVERIFIED")]
    IdentityUnverified,

    // --- BudgetExceeded ---
    /// Budget monétaire ou de tokens épuisé.
    #[serde(rename = "BUDGET_EXHAUSTED")]
    BudgetExhausted,
    /// Budget alloué à la supervision atteint.
    #[serde(rename = "SUPERVISION_BUDGET_EXHAUSTED")]
    SupervisionBudgetExhausted,

    // --- ResourceBusy ---
    /// Le prédécesseur du Run n'a pas encore atteint un état terminal.
    #[serde(rename = "PREDECESSOR_NOT_TERMINAL")]
    PredecessorNotTerminal,
    /// Publication ou rattachement visant un objet en cours de purge.
    #[serde(rename = "OBJECT_DELETING")]
    ObjectDeleting,
    /// File d'attente du writer saturée (backpressure).
    #[serde(rename = "BACKPRESSURE")]
    Backpressure,
    /// Base de données momentanément verrouillée (`SQLITE_BUSY`).
    #[serde(rename = "STORAGE_BUSY")]
    StorageBusy,
    /// Espace de stockage disponible sous le seuil critique (1 GiB).
    #[serde(rename = "STORAGE_LOW_SPACE")]
    StorageLowSpace,
    /// Procédure de reprise après incident en cours d'exécution.
    #[serde(rename = "RECOVERY_IN_PROGRESS")]
    RecoveryInProgress,
    /// Une autorité active est déjà engagée pour cette instance.
    #[serde(rename = "AUTHORITY_ALREADY_ACTIVE")]
    AuthorityAlreadyActive,
    /// Un daemon actif tourne déjà pour ce compte utilisateur.
    #[serde(rename = "DAEMON_ALREADY_RUNNING")]
    DaemonAlreadyRunning,
    /// Procédure de restauration bloquée par un prérequis manquant.
    #[serde(rename = "RESTORATION_BLOCKED")]
    RestorationBlocked,
    /// Incohérence de comptabilité ou d'inventaire des ressources.
    #[serde(rename = "ACCOUNTING_MISMATCH")]
    AccountingMismatch,
    /// Intervention humaine obligatoire avant de poursuivre l'automatisation.
    #[serde(rename = "AUTOMATION_HUMAN_STEP_REQUIRED")]
    AutomationHumanStepRequired,

    // --- InvalidArgument ---
    /// Le prédécesseur a déjà un successeur direct.
    #[serde(rename = "SUCCESSOR_EXISTS")]
    SuccessorExists,
    /// Incohérence détectée lors de l'upload de segments par le client.
    #[serde(rename = "CLIENT_UPLOAD_MISMATCH")]
    ClientUploadMismatch,
    /// Paramètres de réserve de contrôle incohérents.
    #[serde(rename = "CONTROL_RESERVE_INCOHERENT")]
    ControlReserveIncoherent,
    /// Données d'entrée d'automatisation (CSV/mapping) invalides.
    #[serde(rename = "AUTOMATION_INPUT_INVALID")]
    AutomationInputInvalid,
    /// Validateur de contrat inconnu au registre local.
    #[serde(rename = "UNKNOWN_VALIDATOR")]
    UnknownValidator,
    /// Paramètres de règle contractuelle invalides.
    #[serde(rename = "INVALID_RULE_PARAMETERS")]
    InvalidRuleParameters,
    /// Prédicat de claim non reconnu par le moteur.
    #[serde(rename = "UNKNOWN_CLAIM_PREDICATE")]
    UnknownClaimPredicate,
    /// Taille ou complexité du document dépassant la limite autorisée.
    #[serde(rename = "SCHEMA_LIMIT_EXCEEDED")]
    SchemaLimitExceeded,

    // --- InternalFailure ---
    /// Disque plein lors de l'écriture en base (`SQLITE_FULL`).
    #[serde(rename = "STORAGE_FULL")]
    StorageFull,
    /// Erreur d'I/O système sur le stockage (`SQLITE_IOERR`).
    #[serde(rename = "STORAGE_UNAVAILABLE")]
    StorageUnavailable,
    /// Base de données corrompue (`SQLITE_CORRUPT`).
    #[serde(rename = "STORAGE_CORRUPT")]
    StorageCorrupt,
    /// Build `SQLite` compilé non conforme aux exigences qualifiées.
    #[serde(rename = "STORAGE_BUILD_UNQUALIFIED")]
    StorageBuildUnqualified,
    /// Invariant de domaine critique violé en base.
    #[serde(rename = "INVARIANT_VIOLATION")]
    InvariantViolation,

    // --- NotFoundOrHidden ---
    /// Preuve obligatoire introuvable dans le journal.
    #[serde(rename = "EVIDENCE_MISSING")]
    EvidenceMissing,

    // --- IsolationUnavailable ---
    /// Profil d'isolation navigateur ou système non disponible.
    #[serde(rename = "AUTOMATION_PROFILE_UNAVAILABLE")]
    AutomationProfileUnavailable,

    // --- ResyncRequired ---
    /// Curseur de lecture incrémentale expiré.
    #[serde(rename = "CURSOR_EXPIRED")]
    CursorExpired,
    /// Stream d'événements resynchronisé après bascule ou restauration.
    #[serde(rename = "STREAM_RESYNC_REQUIRED")]
    StreamResyncRequired,
}

impl ReasonCode {
    /// Renvoie la classe d'erreur unique [`ErrorCode`] associée à cette raison.
    #[must_use]
    pub const fn error_code(&self) -> ErrorCode {
        match self {
            Self::PublicationConflict | Self::ContractVersionConflict => {
                ErrorCode::RevisionConflict
            }

            Self::StaleEpoch | Self::OwnerEpochChanged | Self::StaleAuthorityGeneration => {
                ErrorCode::StaleFence
            }

            Self::BindingAlreadyEngaged
            | Self::StaleEvidence
            | Self::DoneNotMet
            | Self::LoopExhausted
            | Self::ContractRejected
            | Self::TerminalImmutable
            | Self::IdempotencyWindowExpired
            | Self::AppliedDraftConflict
            | Self::AppliedDraftImmutable
            | Self::ExpectedExecutionMissing
            | Self::EvidenceExecutionMismatch
            | Self::EvidenceProjectionInvalid
            | Self::EvidenceAmbiguous
            | Self::EvidenceSnapshotMismatch
            | Self::ClaimContradicted
            | Self::DiscoveryRequired
            | Self::DiscoveryEvidenceMissing
            | Self::RestoreIncomplete
            | Self::EvidenceUnavailable
            | Self::AutomationModePolicyMismatch
            | Self::AutomationBundleUnavailable
            | Self::AutomationTargetMissing
            | Self::AutomationTargetAmbiguous
            | Self::AutomationTargetDrift
            | Self::BusinessConstraintViolated
            | Self::BusinessPostconditionFailed
            | Self::AutomationAccountMismatch
            | Self::ExternalStateChanged
            | Self::ActionabilityTimeout
            | Self::AssertionTimeout => ErrorCode::ContractViolation,

            Self::UnresolvedEffect => ErrorCode::OutcomeUnknown,

            Self::CapabilityUnavailable
            | Self::DecisionAuthorityRequired
            | Self::DefinitionRevoked
            | Self::ModelCallsForbidden
            | Self::OperationNotAuthorized
            | Self::IdentityUnverified => ErrorCode::CapabilityDenied,

            Self::BudgetExhausted | Self::SupervisionBudgetExhausted => ErrorCode::BudgetExceeded,

            Self::PredecessorNotTerminal
            | Self::ObjectDeleting
            | Self::Backpressure
            | Self::StorageBusy
            | Self::StorageLowSpace
            | Self::RecoveryInProgress
            | Self::AuthorityAlreadyActive
            | Self::DaemonAlreadyRunning
            | Self::RestorationBlocked
            | Self::AccountingMismatch
            | Self::AutomationHumanStepRequired => ErrorCode::ResourceBusy,

            Self::SuccessorExists
            | Self::ClientUploadMismatch
            | Self::ControlReserveIncoherent
            | Self::AutomationInputInvalid
            | Self::UnknownValidator
            | Self::InvalidRuleParameters
            | Self::UnknownClaimPredicate
            | Self::SchemaLimitExceeded => ErrorCode::InvalidArgument,

            Self::StorageFull
            | Self::StorageUnavailable
            | Self::StorageCorrupt
            | Self::StorageBuildUnqualified
            | Self::InvariantViolation => ErrorCode::InternalFailure,

            Self::EvidenceMissing => ErrorCode::NotFoundOrHidden,

            Self::AutomationProfileUnavailable => ErrorCode::IsolationUnavailable,

            Self::CursorExpired | Self::StreamResyncRequired => ErrorCode::ResyncRequired,
        }
    }

    /// Renvoie la politique de reprise par défaut ([`RetryClass`]) stipulée par le contrat canonique.
    #[must_use]
    pub const fn default_retry_class(&self) -> RetryClass {
        match self {
            Self::PublicationConflict
            | Self::ContractVersionConflict
            | Self::OwnerEpochChanged
            | Self::StaleEvidence
            | Self::EvidenceSnapshotMismatch
            | Self::PredecessorNotTerminal
            | Self::ObjectDeleting
            | Self::CursorExpired
            | Self::StreamResyncRequired => RetryClass::AfterRefresh,

            Self::StaleEpoch
            | Self::UnresolvedEffect
            | Self::IdentityUnverified
            | Self::ExternalStateChanged => RetryClass::AfterReconciliation,

            Self::Backpressure
            | Self::StorageBusy
            | Self::RecoveryInProgress
            | Self::AuthorityAlreadyActive
            | Self::StorageUnavailable
            | Self::ActionabilityTimeout
            | Self::AssertionTimeout => RetryClass::AfterBackoff,

            Self::DoneNotMet
            | Self::LoopExhausted
            | Self::ContractRejected
            | Self::ExpectedExecutionMissing
            | Self::EvidenceAmbiguous
            | Self::RestoreIncomplete
            | Self::AutomationBundleUnavailable
            | Self::AutomationTargetMissing
            | Self::AutomationTargetAmbiguous
            | Self::AutomationTargetDrift
            | Self::BusinessConstraintViolated
            | Self::BusinessPostconditionFailed
            | Self::AutomationAccountMismatch
            | Self::CapabilityUnavailable
            | Self::DecisionAuthorityRequired
            | Self::BudgetExhausted
            | Self::SupervisionBudgetExhausted
            | Self::StorageLowSpace
            | Self::RestorationBlocked
            | Self::AccountingMismatch
            | Self::AutomationHumanStepRequired
            | Self::StorageFull
            | Self::AutomationProfileUnavailable => RetryClass::AfterHumanDecision,

            Self::StaleAuthorityGeneration
            | Self::BindingAlreadyEngaged
            | Self::TerminalImmutable
            | Self::IdempotencyWindowExpired
            | Self::AppliedDraftConflict
            | Self::AppliedDraftImmutable
            | Self::EvidenceExecutionMismatch
            | Self::EvidenceProjectionInvalid
            | Self::ClaimContradicted
            | Self::DiscoveryRequired
            | Self::DiscoveryEvidenceMissing
            | Self::EvidenceUnavailable
            | Self::AutomationModePolicyMismatch
            | Self::DefinitionRevoked
            | Self::ModelCallsForbidden
            | Self::OperationNotAuthorized
            | Self::DaemonAlreadyRunning
            | Self::SuccessorExists
            | Self::ClientUploadMismatch
            | Self::ControlReserveIncoherent
            | Self::AutomationInputInvalid
            | Self::UnknownValidator
            | Self::InvalidRuleParameters
            | Self::UnknownClaimPredicate
            | Self::SchemaLimitExceeded
            | Self::StorageCorrupt
            | Self::StorageBuildUnqualified
            | Self::InvariantViolation
            | Self::EvidenceMissing => RetryClass::Never,
        }
    }

    /// Renvoie la chaîne textuelle wire canonique (`UPPER_SNAKE`).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PublicationConflict => "PUBLICATION_CONFLICT",
            Self::ContractVersionConflict => "CONTRACT_VERSION_CONFLICT",
            Self::StaleEpoch => "STALE_EPOCH",
            Self::OwnerEpochChanged => "OWNER_EPOCH_CHANGED",
            Self::StaleAuthorityGeneration => "STALE_AUTHORITY_GENERATION",
            Self::BindingAlreadyEngaged => "BINDING_ALREADY_ENGAGED",
            Self::StaleEvidence => "STALE_EVIDENCE",
            Self::DoneNotMet => "DONE_NOT_MET",
            Self::LoopExhausted => "LOOP_EXHAUSTED",
            Self::ContractRejected => "CONTRACT_REJECTED",
            Self::TerminalImmutable => "TERMINAL_IMMUTABLE",
            Self::IdempotencyWindowExpired => "IDEMPOTENCY_WINDOW_EXPIRED",
            Self::AppliedDraftConflict => "APPLIED_DRAFT_CONFLICT",
            Self::AppliedDraftImmutable => "APPLIED_DRAFT_IMMUTABLE",
            Self::ExpectedExecutionMissing => "EXPECTED_EXECUTION_MISSING",
            Self::EvidenceExecutionMismatch => "EVIDENCE_EXECUTION_MISMATCH",
            Self::EvidenceProjectionInvalid => "EVIDENCE_PROJECTION_INVALID",
            Self::EvidenceAmbiguous => "EVIDENCE_AMBIGUOUS",
            Self::EvidenceSnapshotMismatch => "EVIDENCE_SNAPSHOT_MISMATCH",
            Self::ClaimContradicted => "CLAIM_CONTRADICTED",
            Self::DiscoveryRequired => "DISCOVERY_REQUIRED",
            Self::DiscoveryEvidenceMissing => "DISCOVERY_EVIDENCE_MISSING",
            Self::RestoreIncomplete => "RESTORE_INCOMPLETE",
            Self::EvidenceUnavailable => "EVIDENCE_UNAVAILABLE",
            Self::AutomationModePolicyMismatch => "AUTOMATION_MODE_POLICY_MISMATCH",
            Self::AutomationBundleUnavailable => "AUTOMATION_BUNDLE_UNAVAILABLE",
            Self::AutomationTargetMissing => "AUTOMATION_TARGET_MISSING",
            Self::AutomationTargetAmbiguous => "AUTOMATION_TARGET_AMBIGUOUS",
            Self::AutomationTargetDrift => "AUTOMATION_TARGET_DRIFT",
            Self::BusinessConstraintViolated => "BUSINESS_CONSTRAINT_VIOLATED",
            Self::BusinessPostconditionFailed => "BUSINESS_POSTCONDITION_FAILED",
            Self::AutomationAccountMismatch => "AUTOMATION_ACCOUNT_MISMATCH",
            Self::ExternalStateChanged => "EXTERNAL_STATE_CHANGED",
            Self::ActionabilityTimeout => "ACTIONABILITY_TIMEOUT",
            Self::AssertionTimeout => "ASSERTION_TIMEOUT",
            Self::UnresolvedEffect => "UNRESOLVED_EFFECT",
            Self::CapabilityUnavailable => "CAPABILITY_UNAVAILABLE",
            Self::DecisionAuthorityRequired => "DECISION_AUTHORITY_REQUIRED",
            Self::DefinitionRevoked => "DEFINITION_REVOKED",
            Self::ModelCallsForbidden => "MODEL_CALLS_FORBIDDEN",
            Self::OperationNotAuthorized => "OPERATION_NOT_AUTHORIZED",
            Self::IdentityUnverified => "IDENTITY_UNVERIFIED",
            Self::BudgetExhausted => "BUDGET_EXHAUSTED",
            Self::SupervisionBudgetExhausted => "SUPERVISION_BUDGET_EXHAUSTED",
            Self::PredecessorNotTerminal => "PREDECESSOR_NOT_TERMINAL",
            Self::ObjectDeleting => "OBJECT_DELETING",
            Self::Backpressure => "BACKPRESSURE",
            Self::StorageBusy => "STORAGE_BUSY",
            Self::StorageLowSpace => "STORAGE_LOW_SPACE",
            Self::RecoveryInProgress => "RECOVERY_IN_PROGRESS",
            Self::AuthorityAlreadyActive => "AUTHORITY_ALREADY_ACTIVE",
            Self::DaemonAlreadyRunning => "DAEMON_ALREADY_RUNNING",
            Self::RestorationBlocked => "RESTORATION_BLOCKED",
            Self::AccountingMismatch => "ACCOUNTING_MISMATCH",
            Self::AutomationHumanStepRequired => "AUTOMATION_HUMAN_STEP_REQUIRED",
            Self::SuccessorExists => "SUCCESSOR_EXISTS",
            Self::ClientUploadMismatch => "CLIENT_UPLOAD_MISMATCH",
            Self::ControlReserveIncoherent => "CONTROL_RESERVE_INCOHERENT",
            Self::AutomationInputInvalid => "AUTOMATION_INPUT_INVALID",
            Self::UnknownValidator => "UNKNOWN_VALIDATOR",
            Self::InvalidRuleParameters => "INVALID_RULE_PARAMETERS",
            Self::UnknownClaimPredicate => "UNKNOWN_CLAIM_PREDICATE",
            Self::SchemaLimitExceeded => "SCHEMA_LIMIT_EXCEEDED",
            Self::StorageFull => "STORAGE_FULL",
            Self::StorageUnavailable => "STORAGE_UNAVAILABLE",
            Self::StorageCorrupt => "STORAGE_CORRUPT",
            Self::StorageBuildUnqualified => "STORAGE_BUILD_UNQUALIFIED",
            Self::InvariantViolation => "INVARIANT_VIOLATION",
            Self::EvidenceMissing => "EVIDENCE_MISSING",
            Self::AutomationProfileUnavailable => "AUTOMATION_PROFILE_UNAVAILABLE",
            Self::CursorExpired => "CURSOR_EXPIRED",
            Self::StreamResyncRequired => "STREAM_RESYNC_REQUIRED",
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur lors de la résolution d'une raison d'erreur non reconnue.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Raison d'erreur inconnue : '{0}'")]
pub struct UnknownReasonCodeError(pub String);

impl FromStr for ReasonCode {
    type Err = UnknownReasonCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PUBLICATION_CONFLICT" => Ok(Self::PublicationConflict),
            "CONTRACT_VERSION_CONFLICT" => Ok(Self::ContractVersionConflict),
            "STALE_EPOCH" => Ok(Self::StaleEpoch),
            "OWNER_EPOCH_CHANGED" => Ok(Self::OwnerEpochChanged),
            "STALE_AUTHORITY_GENERATION" => Ok(Self::StaleAuthorityGeneration),
            "BINDING_ALREADY_ENGAGED" => Ok(Self::BindingAlreadyEngaged),
            "STALE_EVIDENCE" => Ok(Self::StaleEvidence),
            "DONE_NOT_MET" => Ok(Self::DoneNotMet),
            "LOOP_EXHAUSTED" => Ok(Self::LoopExhausted),
            "CONTRACT_REJECTED" => Ok(Self::ContractRejected),
            "TERMINAL_IMMUTABLE" => Ok(Self::TerminalImmutable),
            "IDEMPOTENCY_WINDOW_EXPIRED" => Ok(Self::IdempotencyWindowExpired),
            "APPLIED_DRAFT_CONFLICT" => Ok(Self::AppliedDraftConflict),
            "APPLIED_DRAFT_IMMUTABLE" => Ok(Self::AppliedDraftImmutable),
            "EXPECTED_EXECUTION_MISSING" => Ok(Self::ExpectedExecutionMissing),
            "EVIDENCE_EXECUTION_MISMATCH" => Ok(Self::EvidenceExecutionMismatch),
            "EVIDENCE_PROJECTION_INVALID" => Ok(Self::EvidenceProjectionInvalid),
            "EVIDENCE_AMBIGUOUS" => Ok(Self::EvidenceAmbiguous),
            "EVIDENCE_SNAPSHOT_MISMATCH" => Ok(Self::EvidenceSnapshotMismatch),
            "CLAIM_CONTRADICTED" => Ok(Self::ClaimContradicted),
            "DISCOVERY_REQUIRED" => Ok(Self::DiscoveryRequired),
            "DISCOVERY_EVIDENCE_MISSING" => Ok(Self::DiscoveryEvidenceMissing),
            "RESTORE_INCOMPLETE" => Ok(Self::RestoreIncomplete),
            "EVIDENCE_UNAVAILABLE" => Ok(Self::EvidenceUnavailable),
            "AUTOMATION_MODE_POLICY_MISMATCH" => Ok(Self::AutomationModePolicyMismatch),
            "AUTOMATION_BUNDLE_UNAVAILABLE" => Ok(Self::AutomationBundleUnavailable),
            "AUTOMATION_TARGET_MISSING" => Ok(Self::AutomationTargetMissing),
            "AUTOMATION_TARGET_AMBIGUOUS" => Ok(Self::AutomationTargetAmbiguous),
            "AUTOMATION_TARGET_DRIFT" => Ok(Self::AutomationTargetDrift),
            "BUSINESS_CONSTRAINT_VIOLATED" => Ok(Self::BusinessConstraintViolated),
            "BUSINESS_POSTCONDITION_FAILED" => Ok(Self::BusinessPostconditionFailed),
            "AUTOMATION_ACCOUNT_MISMATCH" => Ok(Self::AutomationAccountMismatch),
            "EXTERNAL_STATE_CHANGED" => Ok(Self::ExternalStateChanged),
            "ACTIONABILITY_TIMEOUT" => Ok(Self::ActionabilityTimeout),
            "ASSERTION_TIMEOUT" => Ok(Self::AssertionTimeout),
            "UNRESOLVED_EFFECT" => Ok(Self::UnresolvedEffect),
            "CAPABILITY_UNAVAILABLE" => Ok(Self::CapabilityUnavailable),
            "DECISION_AUTHORITY_REQUIRED" => Ok(Self::DecisionAuthorityRequired),
            "DEFINITION_REVOKED" => Ok(Self::DefinitionRevoked),
            "MODEL_CALLS_FORBIDDEN" => Ok(Self::ModelCallsForbidden),
            "OPERATION_NOT_AUTHORIZED" => Ok(Self::OperationNotAuthorized),
            "IDENTITY_UNVERIFIED" => Ok(Self::IdentityUnverified),
            "BUDGET_EXHAUSTED" => Ok(Self::BudgetExhausted),
            "SUPERVISION_BUDGET_EXHAUSTED" => Ok(Self::SupervisionBudgetExhausted),
            "PREDECESSOR_NOT_TERMINAL" => Ok(Self::PredecessorNotTerminal),
            "OBJECT_DELETING" => Ok(Self::ObjectDeleting),
            "BACKPRESSURE" => Ok(Self::Backpressure),
            "STORAGE_BUSY" => Ok(Self::StorageBusy),
            "STORAGE_LOW_SPACE" => Ok(Self::StorageLowSpace),
            "RECOVERY_IN_PROGRESS" => Ok(Self::RecoveryInProgress),
            "AUTHORITY_ALREADY_ACTIVE" => Ok(Self::AuthorityAlreadyActive),
            "DAEMON_ALREADY_RUNNING" => Ok(Self::DaemonAlreadyRunning),
            "RESTORATION_BLOCKED" => Ok(Self::RestorationBlocked),
            "ACCOUNTING_MISMATCH" => Ok(Self::AccountingMismatch),
            "AUTOMATION_HUMAN_STEP_REQUIRED" => Ok(Self::AutomationHumanStepRequired),
            "SUCCESSOR_EXISTS" => Ok(Self::SuccessorExists),
            "CLIENT_UPLOAD_MISMATCH" => Ok(Self::ClientUploadMismatch),
            "CONTROL_RESERVE_INCOHERENT" => Ok(Self::ControlReserveIncoherent),
            "AUTOMATION_INPUT_INVALID" => Ok(Self::AutomationInputInvalid),
            "UNKNOWN_VALIDATOR" => Ok(Self::UnknownValidator),
            "INVALID_RULE_PARAMETERS" => Ok(Self::InvalidRuleParameters),
            "UNKNOWN_CLAIM_PREDICATE" => Ok(Self::UnknownClaimPredicate),
            "SCHEMA_LIMIT_EXCEEDED" => Ok(Self::SchemaLimitExceeded),
            "STORAGE_FULL" => Ok(Self::StorageFull),
            "STORAGE_UNAVAILABLE" => Ok(Self::StorageUnavailable),
            "STORAGE_CORRUPT" => Ok(Self::StorageCorrupt),
            "STORAGE_BUILD_UNQUALIFIED" => Ok(Self::StorageBuildUnqualified),
            "INVARIANT_VIOLATION" => Ok(Self::InvariantViolation),
            "EVIDENCE_MISSING" => Ok(Self::EvidenceMissing),
            "AUTOMATION_PROFILE_UNAVAILABLE" => Ok(Self::AutomationProfileUnavailable),
            "CURSOR_EXPIRED" => Ok(Self::CursorExpired),
            "STREAM_RESYNC_REQUIRED" => Ok(Self::StreamResyncRequired),
            _ => Err(UnknownReasonCodeError(s.to_owned())),
        }
    }
}

/// DTO normatif de réponse d'erreur IPC (`ipc-error.schema.json`).
///
/// Invariants :
/// - `code`, `reason_code`, `message`, `retry_class`, `details_ref`, `command_id` sont obligatoires.
/// - `reason_code`, `details_ref` et `command_id` sont toujours sérialisés même s'ils valent `null`.
/// - `current_revision` est omis de la sérialisation si `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IpcError {
    /// Classe commune d'erreur parmi les 17 canoniques.
    pub code: ErrorCode,
    /// Raison typée ou null. Clé toujours présente.
    pub reason_code: Option<ReasonCode>,
    /// Message d'erreur explicatif (borné à 4096 caractères).
    pub message: String,
    /// Stratégie de reprise autorisée pour le client.
    pub retry_class: RetryClass,
    /// Référence opaque vers les détails sensibles protégés ou null. Clé toujours présente.
    pub details_ref: Option<String>,
    /// Identifiant de commande corrélé ou null. Clé toujours présente.
    #[ts(type = "string | null")]
    pub command_id: Option<CommandId>,
    /// Révision courante de l'agrégat (présente uniquement lors de conflits de révision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub current_revision: Option<Revision>,
}

impl IpcError {
    /// Crée une nouvelle erreur IPC avec les champs obligatoires.
    #[must_use]
    pub fn new(
        code: ErrorCode,
        reason_code: Option<ReasonCode>,
        message: impl Into<String>,
        retry_class: RetryClass,
    ) -> Self {
        Self {
            code,
            reason_code,
            message: message.into(),
            retry_class,
            details_ref: None,
            command_id: None,
            current_revision: None,
        }
    }

    /// Crée une erreur standard à partir d'un [`ReasonCode`] typé en dérivant
    /// automatiquement son [`ErrorCode`] et sa [`RetryClass`] par défaut.
    #[must_use]
    pub fn from_reason(reason: ReasonCode, message: impl Into<String>) -> Self {
        Self::new(
            reason.error_code(),
            Some(reason),
            message,
            reason.default_retry_class(),
        )
    }

    /// Associe un identifiant de commande à l'erreur.
    #[must_use]
    pub fn with_command_id(mut self, command_id: CommandId) -> Self {
        self.command_id = Some(command_id);
        self
    }

    /// Associe une référence de détails protégés à l'erreur.
    #[must_use]
    pub fn with_details_ref(mut self, details_ref: impl Into<String>) -> Self {
        self.details_ref = Some(details_ref.into());
        self
    }

    /// Associe la révision courante lors d'un conflit de révision.
    #[must_use]
    pub fn with_current_revision(mut self, revision: Revision) -> Self {
        self.current_revision = Some(revision);
        self
    }
}
