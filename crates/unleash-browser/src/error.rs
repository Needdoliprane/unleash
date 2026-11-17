//! Modèle d'erreurs typé de `unleash-browser` et projection vers [`IpcError`].
//!
//! Chaque variante de [`BrowserError`] correspond à un code de domaine du contrat
//! navigateur (`specs/browser-automation.md` §7 et `features/F019/technical-design.md`).

use thiserror::Error;
use unleash_domain::{ActionId, CommandId};
use unleash_protocol::{ErrorCode, IpcError, ReasonCode, RetryClass};

/// Erreur typée retournée par les opérations du port [`crate::BrowserPort`] et du proxy d'egress.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BrowserError {
    /// Le binaire navigateur ou driver épinglé est absent ou diverge en version ou empreinte `SHA-256`.
    #[error("Runtime navigateur incompatible ({details_ref}) : {message}")]
    RuntimeMismatch {
        /// Référence technique identifiant le binaire ou le composant divergent.
        details_ref: String,
        /// Description diagnostique de l'écart observé.
        message: String,
    },

    /// Une capacité requise ou un navigateur non pris en charge (ex. Safari en `mediated`) est demandé.
    #[error("Capacité navigateur manquante ({details_ref}) : {message}")]
    CapabilityMissing {
        /// Référence vers la capacité absente.
        details_ref: String,
        /// Explication du refus.
        message: String,
    },

    /// Stratégie de locator interdite en V1 (`XPath`, coordonnées) ou `execute_script` fourni par un modèle.
    #[error("Locator ou script non autorisé par la matrice V1 : {reason}")]
    UnsupportedLocatorOrScript {
        /// Motif exact du rejet.
        reason: String,
    },

    /// Le profil d'isolation requis (mode `strict` ou `project_script` sous garantie sans modèle) n'est pas prouvé.
    #[error("Isolation requise indisponible : {reason}")]
    IsolationUnavailable {
        /// Détail sur la garantie de confinement manquante.
        reason: String,
    },

    /// Tentative directe ou indirecte d'invocation d'un modèle sous `model_policy = forbidden`.
    #[error("Appel modèle interdit sous model_policy=forbidden : {reason}")]
    ModelUseForbidden {
        /// Nature ou origine de la tentative bloquée.
        reason: String,
    },

    /// Aucun élément correspondant au locator déclaré n'a été trouvé (`cardinalité = 0`).
    #[error("Cible introuvable pour le locator '{locator}'")]
    TargetMissing {
        /// Représentation synthétique du locator évalué.
        locator: String,
    },

    /// Plusieurs éléments correspondent au locator (`cardinalité > 1`) ; sélection opportuniste interdite.
    #[error("Cible ambiguë ({candidate_count} candidats) pour le locator '{locator}'")]
    AmbiguousTarget {
        /// Représentation synthétique du locator évalué.
        locator: String,
        /// Nombre réel de candidats observés.
        candidate_count: usize,
    },

    /// L'unique cible résolue porte une identité métier divergente de celle attendue par le contrat.
    #[error("Dérive de cible métier : attendu '{expected}', observé '{observed}'")]
    TargetDrift {
        /// Identité métier attendue par le contrat.
        expected: String,
        /// Identité effectivement observée dans le DOM.
        observed: String,
    },

    /// Précondition métier violée avant tout dispatch d'effet.
    #[error("Contrainte métier violée avant effet : {message}")]
    BusinessConstraintViolated {
        /// Détail de la contrainte non respectée.
        message: String,
    },

    /// Postcondition métier en échec après un dispatch techniquement accepté.
    #[error("Postcondition métier non satisfaite après effet : {message}")]
    BusinessPostconditionFailed {
        /// Détail de l'écart constaté entre l'attendu et l'observé.
        message: String,
        /// Référence optionnelle vers la preuve ou l'observation associée.
        evidence_ref: Option<String>,
    },

    /// L'état externe observé contredit le checkpoint ou révèle une modification concurrente.
    #[error("État externe modifié de manière concurrente : {message}")]
    ExternalStateChanged {
        /// Description de l'écart d'état.
        message: String,
    },

    /// Le compte observé dans le contexte navigateur ne correspond pas au mandat attendu.
    #[error("Incompatibilité de compte : attendu '{expected}', observé '{observed}'")]
    AccountMismatch {
        /// Compte attendu.
        expected: String,
        /// Compte observé.
        observed: String,
    },

    /// Une intervention humaine (authentification expirée, MFA, challenge) est requise.
    #[error("Intervention humaine requise sur '{origin}' : {reason}")]
    AuthenticationRequired {
        /// Origine concernée.
        origin: String,
        /// Motif du blocage.
        reason: String,
    },

    /// Délai d'actionnabilité dépassé avant le dispatch de l'effet.
    #[error("Délai d'actionnabilité expiré avant dispatch : {message}")]
    ActionabilityTimeout {
        /// Détail du délai expiré.
        message: String,
    },

    /// Délai d'assertion dépassé avant conclusion déterministe.
    #[error("Délai d'assertion expiré : {message}")]
    AssertionTimeout {
        /// Détail de l'assertion expirée.
        message: String,
    },

    /// Coupure ou timeout après dispatch d'une mutation : l'effet peut avoir eu lieu, aucun retry permis.
    #[error("Issue d'effet inconnue pour l'action {action_id} (clé '{effect_key}') : {reason}")]
    OutcomeUnknown {
        /// Identifiant de l'action dont le transport s'est rompu.
        action_id: ActionId,
        /// Clé logique métier/effet interdisant toute répétition sans réconciliation.
        effect_key: String,
        /// Cause technique de l'incertitude.
        reason: String,
    },

    /// Accès réseau ou redirection vers une origine non couverte par le mandat actif.
    #[error("Sortie réseau bloquée vers '{origin}' : {reason}")]
    EgressBlocked {
        /// Origine refusée.
        origin: String,
        /// Raison du blocage par le proxy d'egress.
        reason: String,
    },

    /// Barrière d'autorité (`LineageFence`), contexte ou génération de page périmé.
    #[error("Fence ou génération de contexte périmée : {reason}")]
    StaleFenceOrContext {
        /// Motif du rejet de fence/génération.
        reason: String,
    },

    /// Paramètre de configuration ou URL invalide avant exécution.
    #[error("Configuration navigateur invalide : {reason}")]
    InvalidConfiguration {
        /// Détail de l'argument invalide.
        reason: String,
    },
}

impl BrowserError {
    /// Retourne la classe d'erreur IPC canonique ([`ErrorCode`]) correspondante.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        self.reason_code().error_code()
    }

    /// Retourne la raison fermée ([`ReasonCode`]) correspondante selon `F019/technical-design.md`.
    #[must_use]
    pub const fn reason_code(&self) -> ReasonCode {
        match self {
            Self::RuntimeMismatch { .. }
            | Self::CapabilityMissing { .. }
            | Self::UnsupportedLocatorOrScript { .. } => ReasonCode::CapabilityUnavailable,
            Self::IsolationUnavailable { .. } => ReasonCode::AutomationProfileUnavailable,
            Self::ModelUseForbidden { .. } => ReasonCode::ModelCallsForbidden,
            Self::TargetMissing { .. } => ReasonCode::AutomationTargetMissing,
            Self::AmbiguousTarget { .. } => ReasonCode::AutomationTargetAmbiguous,
            Self::TargetDrift { .. } => ReasonCode::AutomationTargetDrift,
            Self::BusinessConstraintViolated { .. } => ReasonCode::BusinessConstraintViolated,
            Self::BusinessPostconditionFailed { .. } => ReasonCode::BusinessPostconditionFailed,
            Self::ExternalStateChanged { .. } => ReasonCode::ExternalStateChanged,
            Self::AccountMismatch { .. } => ReasonCode::AutomationAccountMismatch,
            Self::AuthenticationRequired { .. } => ReasonCode::AutomationHumanStepRequired,
            Self::ActionabilityTimeout { .. } => ReasonCode::ActionabilityTimeout,
            Self::AssertionTimeout { .. } => ReasonCode::AssertionTimeout,
            Self::OutcomeUnknown { .. } => ReasonCode::UnresolvedEffect,
            Self::EgressBlocked { .. } => ReasonCode::AccessDenied,
            Self::StaleFenceOrContext { .. } => ReasonCode::StaleEpoch,
            Self::InvalidConfiguration { .. } => ReasonCode::AutomationInputInvalid,
        }
    }

    /// Retourne la politique de reprise canonique ([`RetryClass`]) associée à cette erreur.
    #[must_use]
    pub const fn retry_class(&self) -> RetryClass {
        match self {
            Self::UnsupportedLocatorOrScript { .. }
            | Self::ModelUseForbidden { .. }
            | Self::EgressBlocked { .. }
            | Self::InvalidConfiguration { .. } => RetryClass::Never,
            Self::OutcomeUnknown { .. }
            | Self::ExternalStateChanged { .. }
            | Self::StaleFenceOrContext { .. } => RetryClass::AfterReconciliation,
            Self::ActionabilityTimeout { .. } | Self::AssertionTimeout { .. } => {
                RetryClass::AfterBackoff
            }
            Self::RuntimeMismatch { .. }
            | Self::CapabilityMissing { .. }
            | Self::IsolationUnavailable { .. }
            | Self::TargetMissing { .. }
            | Self::AmbiguousTarget { .. }
            | Self::TargetDrift { .. }
            | Self::BusinessConstraintViolated { .. }
            | Self::BusinessPostconditionFailed { .. }
            | Self::AccountMismatch { .. }
            | Self::AuthenticationRequired { .. } => RetryClass::AfterHumanDecision,
        }
    }

    /// Convertit cette erreur en DTO wire [`IpcError`] conforme à `ipc-error.schema.json`.
    #[must_use]
    pub fn to_ipc_error(&self, command_id: Option<CommandId>) -> IpcError {
        let mut ipc = IpcError::new(
            self.error_code(),
            Some(self.reason_code()),
            self.to_string(),
            self.retry_class(),
        );
        match self {
            Self::RuntimeMismatch { details_ref, .. }
            | Self::CapabilityMissing { details_ref, .. } => {
                ipc = ipc.with_details_ref(details_ref.clone());
            }
            Self::BusinessPostconditionFailed {
                evidence_ref: Some(ev),
                ..
            } => {
                ipc = ipc.with_details_ref(ev.clone());
            }
            Self::OutcomeUnknown { effect_key, .. } => {
                ipc = ipc.with_details_ref(effect_key.clone());
            }
            _ => {}
        }
        if let Some(cid) = command_id {
            ipc = ipc.with_command_id(cid);
        }
        ipc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_wire_mappings_match_f019_table() {
        let mismatch = BrowserError::RuntimeMismatch {
            details_ref: "chromedriver".into(),
            message: "missing".into(),
        };
        let ipc_mismatch = mismatch.to_ipc_error(None);
        assert_eq!(ipc_mismatch.code, ErrorCode::CapabilityDenied);
        assert_eq!(
            ipc_mismatch.reason_code,
            Some(ReasonCode::CapabilityUnavailable)
        );
        assert_eq!(ipc_mismatch.retry_class, RetryClass::AfterHumanDecision);
        assert_eq!(ipc_mismatch.details_ref.as_deref(), Some("chromedriver"));

        let unknown = BrowserError::OutcomeUnknown {
            action_id: ActionId::from_u128(1),
            effect_key: "cart.add.apple".into(),
            reason: "tcp reset".into(),
        };
        let ipc_unknown = unknown.to_ipc_error(Some(CommandId::from_u128(2)));
        assert_eq!(ipc_unknown.code, ErrorCode::OutcomeUnknown);
        assert_eq!(ipc_unknown.reason_code, Some(ReasonCode::UnresolvedEffect));
        assert_eq!(ipc_unknown.retry_class, RetryClass::AfterReconciliation);
        assert_eq!(ipc_unknown.command_id, Some(CommandId::from_u128(2)));

        let forbidden = BrowserError::ModelUseForbidden {
            reason: "embedding".into(),
        };
        let ipc_forbidden = forbidden.to_ipc_error(None);
        assert_eq!(ipc_forbidden.code, ErrorCode::CapabilityDenied);
        assert_eq!(
            ipc_forbidden.reason_code,
            Some(ReasonCode::ModelCallsForbidden)
        );
        assert_eq!(ipc_forbidden.retry_class, RetryClass::Never);

        let iso = BrowserError::IsolationUnavailable {
            reason: "unconfined project_script".into(),
        };
        assert_eq!(iso.error_code(), ErrorCode::IsolationUnavailable);
        assert_eq!(iso.reason_code(), ReasonCode::AutomationProfileUnavailable);
    }
}
