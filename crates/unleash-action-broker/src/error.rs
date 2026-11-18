// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées de `unleash-action-broker`, du port [`crate::GuardPort`] et du port [`crate::SecretStore`],
//! alignées sur le catalogue wire F017 (`ipc-and-apis.md` et `F017/interfaces.md`).

use thiserror::Error;
use unleash_domain::{
    ActionId, ApprovalScopeId, CommandId, DomainTransitionError, FenceError, IdValidationError,
    Revision,
};
use unleash_platform::{KeyringError, PlatformError};
use unleash_protocol::{ErrorCode, IpcError, ProtocolError, ReasonCode, RetryClass};

/// Erreur produite par les opérations de [`crate::SecretStore`].
///
/// Aucune variante ne contient jamais de valeur secrète en clair (`REQ-SEC-003`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SecretStoreError {
    /// Coffre de secrets verrouillé (`CapabilityDenied/SECRET_STORE_LOCKED`).
    #[error("Coffre de secrets verrouillé")]
    Locked,

    /// Coffre de secrets indisponible (`InternalFailure/SECRET_STORE_UNAVAILABLE`).
    #[error("Coffre de secrets indisponible : {reason}")]
    Unavailable {
        /// Motif diagnostique sans valeur secrète.
        reason: String,
    },

    /// Accès au secret refusé par ACL (`CapabilityDenied/SECRET_ACCESS_DENIED`).
    #[error("Accès au secret refusé : {reason}")]
    AccessDenied {
        /// Motif diagnostique sans valeur secrète.
        reason: String,
    },

    /// Référence de secret introuvable (`NotFoundOrHidden/SECRET_REF_NOT_FOUND`).
    #[error("Référence de secret introuvable")]
    NotFound,

    /// Référence de secret invalide ou malformée.
    #[error("Référence de secret invalide : {reason}")]
    InvalidReference {
        /// Détail de validation (sans secret).
        reason: String,
    },
}

impl From<KeyringError> for SecretStoreError {
    fn from(err: KeyringError) -> Self {
        match err {
            KeyringError::Locked => Self::Locked,
            KeyringError::Unavailable { reason } => Self::Unavailable { reason },
            KeyringError::AccessDenied { reason } => Self::AccessDenied { reason },
            KeyringError::NotFound => Self::NotFound,
            KeyringError::InvalidReference(reason) => Self::InvalidReference { reason },
            KeyringError::Io(io_err) => Self::Unavailable {
                reason: io_err.to_string(),
            },
            other => Self::Unavailable {
                reason: other.to_string(),
            },
        }
    }
}

/// Erreur retournée par une implémentation de [`crate::GuardPort`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GuardPortError {
    /// Service AI-Guard indisponible.
    #[error("Garde AI-Guard indisponible : {reason}")]
    Unavailable {
        /// Motif technique d'indisponibilité.
        reason: String,
    },

    /// Délai d'évaluation ou d'appel dépassé.
    #[error("Délai AI-Guard expiré après {timeout_ms} ms")]
    Timeout {
        /// Délai configuré en millisecondes.
        timeout_ms: u64,
    },

    /// Réponse malformée ou syntaxiquement invalide (jamais interprétée comme Allow).
    #[error("Réponse AI-Guard malformée : {detail}")]
    MalformedResponse {
        /// Détail de l'anomalie de format.
        detail: String,
    },

    /// Capacité non prise en charge ou non négociée.
    #[error("Capacité AI-Guard non prise en charge '{capability}'")]
    UnsupportedCapability {
        /// Capacité demandée absente.
        capability: String,
        /// Version de protocole éventuelle.
        protocol_version: Option<String>,
    },

    /// Version de protocole AI-Guard incompatible.
    #[error("Protocole AI-Guard incompatible : attendu '{expected}', observé '{observed}'")]
    IncompatibleProtocol {
        /// Version attendue.
        expected: String,
        /// Version observée.
        observed: String,
    },

    /// Authentification/TLS refusée par AI-Guard.
    #[error("Authentification AI-Guard refusée : {reason}")]
    Unauthorized {
        /// Motif sans secret.
        reason: String,
    },

    /// Erreur de transport sous-jacente.
    #[error("Erreur de transport AI-Guard : {detail}")]
    Transport {
        /// Détail transport.
        detail: String,
    },
}

/// Erreur métier et de sécurité émise par [`crate::ActionBroker`] et ses validateurs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ActionBrokerError {
    /// Coffre de secrets verrouillé (`CapabilityDenied/SECRET_STORE_LOCKED`).
    #[error("Coffre de secrets verrouillé : {reason}")]
    SecretStoreLocked {
        /// Motif diagnostique sans secret.
        reason: String,
    },

    /// Coffre de secrets indisponible (`InternalFailure/SECRET_STORE_UNAVAILABLE`).
    #[error("Coffre de secrets indisponible : {reason}")]
    SecretStoreUnavailable {
        /// Motif diagnostique sans secret.
        reason: String,
    },

    /// Accès au secret refusé (`CapabilityDenied/SECRET_ACCESS_DENIED`).
    #[error("Accès au secret refusé : {reason}")]
    SecretAccessDenied {
        /// Motif du refus sans secret.
        reason: String,
    },

    /// Référence de secret introuvable (`NotFoundOrHidden/SECRET_REF_NOT_FOUND`).
    #[error("Référence de secret introuvable : {reason}")]
    SecretRefNotFound {
        /// Détail sans secret.
        reason: String,
    },

    /// Refus déterministe par la politique locale (`CapabilityDenied/POLICY_DENIED`).
    #[error("Action refusée par la politique locale : {reason}")]
    PolicyDenied {
        /// Motif explicite du refus.
        reason: String,
        /// Autorités limitantes identifiées lors de l'intersection.
        limiting_authorities: Vec<String>,
    },

    /// Classe d'effet inconnue refusée avant `GuardPort` (`InvalidArgument/EFFECTS_CLASS_UNKNOWN`).
    #[error("Classe d'effet inconnue '{raw_class}'")]
    EffectsClassUnknown {
        /// Valeur brute reçue.
        raw_class: String,
    },

    /// Refus formel retourné par `GuardPort` (`CapabilityDenied/GUARD_DENIED`).
    #[error("Action refusée par AI-Guard ({reason_code}) : {detail}")]
    GuardDenied {
        /// Code de refus fourni par le garde.
        reason_code: String,
        /// Explication du refus.
        detail: String,
        /// Référence de décision associée.
        decision_ref: Option<String>,
    },

    /// Réponse `Allow` porteuse d'une obligation inconnue, contradictoire, non acceptée (`none_accepted`)
    /// ou impossible à imposer (`CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED`).
    #[error("Obligation de décision non satisfaite : {reason}")]
    GuardObligationUnsatisfied {
        /// Motif précis de l'échec d'obligation.
        reason: String,
    },

    /// Approbation humaine requise sur le scope et digest exacts (`CapabilityDenied/GUARD_APPROVAL_REQUIRED`).
    #[error("Approbation humaine requise sur le périmètre {approval_scope} : {reason}")]
    GuardApprovalRequired {
        /// Identifiant du périmètre d'approbation.
        approval_scope: ApprovalScopeId,
        /// Référence de décision du garde.
        decision_ref: String,
        /// Explication de la demande.
        reason: String,
    },

    /// Demande d'approbation non prise en charge par le profil actif (`CapabilityDenied/GUARD_APPROVAL_UNSUPPORTED`).
    #[error("Approbation AI-Guard non prise en charge : {reason}")]
    GuardApprovalUnsupported {
        /// Détail de l'absence de support.
        reason: String,
    },

    /// Service AI-Guard indisponible, en timeout ou réponse malformée (`CapabilityDenied/GUARD_UNAVAILABLE`).
    #[error("AI-Guard indisponible : {reason}")]
    GuardUnavailable {
        /// Motif de l'indisponibilité.
        reason: String,
    },

    /// Protocole ou capacité AI-Guard incompatible (`ProtocolIncompatible/GUARD_PROTOCOL_UNSUPPORTED`).
    #[error("Protocole AI-Guard non supporté : {reason}")]
    GuardProtocolUnsupported {
        /// Détail d'incompatibilité.
        reason: String,
    },

    /// Sujet suspendu localement ou via garde (`CapabilityDenied/SUBJECT_SUSPENDED`).
    #[error("Sujet '{subject}' suspendu : {reason}")]
    SubjectSuspended {
        /// Clé du sujet suspendu.
        subject: String,
        /// Motif de la suspension.
        reason: String,
    },

    /// Sujet définitivement révoqué (`CapabilityDenied/SUBJECT_REVOKED`).
    #[error("Sujet '{subject}' révoqué : {reason}")]
    SubjectRevoked {
        /// Clé du sujet révoqué.
        subject: String,
        /// Motif de la révocation.
        reason: String,
    },

    /// Backend d'isolation OS absent en mode strict (`IsolationUnavailable/ISOLATION_BACKEND_MISSING`).
    #[error("Backend d'isolation '{backend}' absent : {remediation}")]
    IsolationBackendMissing {
        /// Nom du backend attendu.
        backend: String,
        /// Remédiation explicite.
        remediation: String,
    },

    /// Obligation d'isolation non imposable (`IsolationUnavailable/ISOLATION_OBLIGATION_UNENFORCEABLE`).
    #[error("Obligation d'isolation '{backend}' non imposable : {reason} ({remediation})")]
    IsolationObligationUnenforceable {
        /// Nom du backend concerné.
        backend: String,
        /// Cause technique précise.
        reason: String,
        /// Remédiation explicite.
        remediation: String,
    },

    /// Tentative d'auto-élévation de permissions ou d'élargissement par source externe (`CapabilityDenied/PERMISSION_EXPANSION_DENIED`).
    #[error("Élargissement de permissions refusé ({limiting_authority}) : {reason}")]
    PermissionExpansionDenied {
        /// Motif du refus d'élargissement.
        reason: String,
        /// Autorité plafonnante.
        limiting_authority: String,
    },

    /// Ressource ou chemin hors périmètre autorisé (`ScopeDenied/ACCESS_DENIED`).
    #[error("Accès hors scope refusé : {reason}")]
    ScopeDenied {
        /// Motif du rejet de périmètre.
        reason: String,
    },

    /// Opération administrative interdite pour l'appelant (`CapabilityDenied/OPERATION_NOT_AUTHORIZED`).
    #[error("Opération '{operation}' non autorisée pour '{actor}'")]
    OperationNotAuthorized {
        /// Nom de l'opération demandée.
        operation: String,
        /// Identifiant de l'acteur appelant.
        actor: String,
    },

    /// Fence, époque ou lignée obsolète (`StaleFence`).
    #[error("Fence ou lignée obsolète : {reason}")]
    StaleFence {
        /// Détail du rejet de fence.
        reason: String,
        /// Vrai si la lignée entière diverge (`STALE_AUTHORITY_GENERATION`).
        lineage_mismatch: bool,
    },

    /// Conflit de révision optimiste (`RevisionConflict`).
    #[error("Conflit de révision : attendue {expected_revision}, courante {current_revision}")]
    RevisionConflict {
        /// Révision actuelle de l'agrégat.
        current_revision: Revision,
        /// Révision attendue par la commande.
        expected_revision: Revision,
    },

    /// Argument invalide (`InvalidArgument`).
    #[error("Argument invalide : {reason}")]
    InvalidArgument {
        /// Détail de validation.
        reason: String,
    },

    /// Issue d'effet impossible à exclure ou inconnue (`OutcomeUnknown/UNRESOLVED_EFFECT`).
    #[error("Issue d'effet inconnue pour l'action {action_id} : {reason}")]
    OutcomeUnknown {
        /// Identifiant de l'action incertaine.
        action_id: ActionId,
        /// Raison de l'incertitude.
        reason: String,
    },

    /// Violation de contrat ou mutation sur état terminal (`ContractViolation`).
    #[error("Violation contractuelle : {reason}")]
    ContractViolation {
        /// Motif du rejet contractuel.
        reason: String,
        /// Vrai si l'agrégat était déjà terminal (`TERMINAL_IMMUTABLE`).
        terminal_immutable: bool,
    },

    /// Ressource introuvable ou masquée (`NotFoundOrHidden`).
    #[error("Ressource introuvable ou masquée : {resource}")]
    NotFound {
        /// Description de la ressource absente.
        resource: String,
    },

    /// Appel modèle interdit sous `model_policy=forbidden` (`CapabilityDenied/MODEL_CALLS_FORBIDDEN`).
    #[error("Appel modèle interdit sous la politique active : {reason}")]
    ModelCallsForbidden {
        /// Motif du refus.
        reason: String,
    },

    /// Violation de localité des données (`CapabilityDenied/DATA_LOCALITY_VIOLATION`).
    #[error("Localité des données violée : {reason}")]
    DataLocalityViolation {
        /// Détail du refus de placement.
        reason: String,
    },
}

impl ActionBrokerError {
    /// Retourne la classe d'erreur commune [`ErrorCode`] selon la projection F017.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        if let Some(reason) = self.reason_code() {
            return reason.error_code();
        }
        match self {
            Self::RevisionConflict { .. } => ErrorCode::RevisionConflict,
            Self::InvalidArgument { .. } => ErrorCode::InvalidArgument,
            Self::NotFound { .. } => ErrorCode::NotFoundOrHidden,
            _ => ErrorCode::InternalFailure,
        }
    }

    /// Retourne le [`ReasonCode`] canonique F017 s'il existe.
    #[must_use]
    pub const fn reason_code(&self) -> Option<ReasonCode> {
        match self {
            Self::SecretStoreLocked { .. } => Some(ReasonCode::SecretStoreLocked),
            Self::SecretStoreUnavailable { .. } => Some(ReasonCode::SecretStoreUnavailable),
            Self::SecretAccessDenied { .. } => Some(ReasonCode::SecretAccessDenied),
            Self::SecretRefNotFound { .. } => Some(ReasonCode::SecretRefNotFound),
            Self::PolicyDenied { .. } => Some(ReasonCode::PolicyDenied),
            Self::EffectsClassUnknown { .. } => Some(ReasonCode::EffectsClassUnknown),
            Self::GuardDenied { .. } => Some(ReasonCode::GuardDenied),
            Self::GuardObligationUnsatisfied { .. } => Some(ReasonCode::GuardObligationUnsatisfied),
            Self::GuardApprovalRequired { .. } => Some(ReasonCode::GuardApprovalRequired),
            Self::GuardApprovalUnsupported { .. } => Some(ReasonCode::GuardApprovalUnsupported),
            Self::GuardUnavailable { .. } => Some(ReasonCode::GuardUnavailable),
            Self::GuardProtocolUnsupported { .. } => Some(ReasonCode::GuardProtocolUnsupported),
            Self::SubjectSuspended { .. } => Some(ReasonCode::SubjectSuspended),
            Self::SubjectRevoked { .. } => Some(ReasonCode::SubjectRevoked),
            Self::IsolationBackendMissing { .. } => Some(ReasonCode::IsolationBackendMissing),
            Self::IsolationObligationUnenforceable { .. } => {
                Some(ReasonCode::IsolationObligationUnenforceable)
            }
            Self::PermissionExpansionDenied { .. } => Some(ReasonCode::PermissionExpansionDenied),
            Self::ScopeDenied { .. } => Some(ReasonCode::AccessDenied),
            Self::OperationNotAuthorized { .. } => Some(ReasonCode::OperationNotAuthorized),
            Self::StaleFence {
                lineage_mismatch: true,
                ..
            } => Some(ReasonCode::StaleAuthorityGeneration),
            Self::StaleFence {
                lineage_mismatch: false,
                ..
            } => Some(ReasonCode::StaleEpoch),
            Self::OutcomeUnknown { .. } => Some(ReasonCode::UnresolvedEffect),
            Self::ContractViolation {
                terminal_immutable: true,
                ..
            } => Some(ReasonCode::TerminalImmutable),
            Self::ContractViolation {
                terminal_immutable: false,
                ..
            } => Some(ReasonCode::ContractRejected),
            Self::ModelCallsForbidden { .. } => Some(ReasonCode::ModelCallsForbidden),
            Self::DataLocalityViolation { .. } => Some(ReasonCode::DataLocalityViolation),
            Self::RevisionConflict { .. }
            | Self::InvalidArgument { .. }
            | Self::NotFound { .. } => None,
        }
    }

    /// Retourne la classe de reprise [`RetryClass`] imposée par le catalogue.
    #[must_use]
    pub const fn retry_class(&self) -> RetryClass {
        if let Some(reason) = self.reason_code() {
            return reason.default_retry_class();
        }
        match self {
            Self::RevisionConflict { .. } => RetryClass::AfterRefresh,
            _ => RetryClass::Never,
        }
    }

    /// Projette cette erreur vers le DTO wire [`IpcError`] conforme à `ipc-error.schema.json`.
    #[must_use]
    pub fn to_ipc_error(&self, command_id: Option<CommandId>) -> IpcError {
        let mut ipc = match self.reason_code() {
            Some(reason) => IpcError::from_reason(reason, self.to_string()),
            None => IpcError::new(
                self.error_code(),
                None,
                self.to_string(),
                self.retry_class(),
            ),
        };
        if let Self::RevisionConflict {
            current_revision, ..
        } = self
        {
            ipc = ipc.with_current_revision(*current_revision);
        }
        if let Some(cid) = command_id {
            ipc = ipc.with_command_id(cid);
        }
        ipc
    }
}

impl From<SecretStoreError> for ActionBrokerError {
    fn from(err: SecretStoreError) -> Self {
        match err {
            SecretStoreError::Locked => Self::SecretStoreLocked {
                reason: String::from("Coffre OS verrouillé"),
            },
            SecretStoreError::Unavailable { reason } => Self::SecretStoreUnavailable { reason },
            SecretStoreError::AccessDenied { reason } => Self::SecretAccessDenied { reason },
            SecretStoreError::NotFound => Self::SecretRefNotFound {
                reason: String::from("Référence de secret absente du coffre"),
            },
            SecretStoreError::InvalidReference { reason } => Self::InvalidArgument { reason },
        }
    }
}

impl From<DomainTransitionError> for ActionBrokerError {
    fn from(err: DomainTransitionError) -> Self {
        match err {
            DomainTransitionError::TerminalImmutable { entity, state } => Self::ContractViolation {
                reason: format!("Entité '{entity}' déjà terminale dans l'état '{state}'"),
                terminal_immutable: true,
            },
            DomainTransitionError::CannotRetryOutcomeUnknown => Self::ContractViolation {
                reason: err.to_string(),
                terminal_immutable: false,
            },
            DomainTransitionError::PolicyViolation { reason } => Self::PolicyDenied {
                reason,
                limiting_authorities: Vec::new(),
            },
            DomainTransitionError::GuardUnavailableFailClosed => Self::GuardUnavailable {
                reason: err.to_string(),
            },
            DomainTransitionError::StaleFence { reason } => Self::StaleFence {
                reason,
                lineage_mismatch: false,
            },
            DomainTransitionError::IncomparableLineage => Self::StaleFence {
                reason: err.to_string(),
                lineage_mismatch: true,
            },
            other => Self::ContractViolation {
                reason: other.to_string(),
                terminal_immutable: false,
            },
        }
    }
}

impl From<PlatformError> for ActionBrokerError {
    fn from(err: PlatformError) -> Self {
        match err {
            PlatformError::ForbiddenDocumentationPath { path } => Self::ScopeDenied {
                reason: format!("Écriture interdite sous le dossier documentaire '{path}'"),
            },
            PlatformError::SymlinkNotAllowed { path } => Self::ScopeDenied {
                reason: format!("Lien symbolique refusé sur '{path}'"),
            },
            PlatformError::InvalidPath { path, reason } => Self::ScopeDenied {
                reason: format!("Chemin invalide '{path}': {reason}"),
            },
            PlatformError::PeerUidMismatch {
                expected_uid,
                actual_uid,
            } => Self::OperationNotAuthorized {
                operation: String::from("peer_verification"),
                actor: format!("uid:{actual_uid} (attendu uid:{expected_uid})"),
            },
            other => Self::InvalidArgument {
                reason: other.to_string(),
            },
        }
    }
}

impl From<FenceError> for ActionBrokerError {
    fn from(err: FenceError) -> Self {
        match err {
            FenceError::LineageMismatch { .. } | FenceError::IncomparableLineage => {
                Self::StaleFence {
                    reason: err.to_string(),
                    lineage_mismatch: true,
                }
            }
            FenceError::StaleFence { .. } => Self::StaleFence {
                reason: err.to_string(),
                lineage_mismatch: false,
            },
        }
    }
}

impl From<ProtocolError> for ActionBrokerError {
    fn from(err: ProtocolError) -> Self {
        Self::InvalidArgument {
            reason: err.to_string(),
        }
    }
}

impl From<IdValidationError> for ActionBrokerError {
    fn from(err: IdValidationError) -> Self {
        Self::InvalidArgument {
            reason: err.to_string(),
        }
    }
}
