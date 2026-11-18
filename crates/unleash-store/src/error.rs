//! Erreurs typées de persistance et projection vers le catalogue d'erreurs IPC.

use thiserror::Error;
use unleash_domain::{
    ActionId, AuthorityGeneration, CommandId, ReceiptId, Revision, Sha256Digest, StreamId,
};
use unleash_protocol::{ErrorCode, EventCursor, IpcError, ReasonCode, RetryClass};

/// Erreur produite par les opérations de stockage `unleash-store`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// Le build `SQLite` natif ne satisfait pas la matrice de qualification ou diverge de la valeur compilée.
    #[error("Build SQLite non qualifié : {detail}")]
    BuildUnqualified {
        /// Détail technique du refus.
        detail: String,
    },

    /// La file du writer (8 demandes max) est pleine (`ResourceBusy/BACKPRESSURE`).
    #[error("File du writer saturée ({queue_depth}/{max_depth})")]
    Backpressure {
        /// Profondeur observée lors du refus.
        queue_depth: usize,
        /// Capacité maximale configurée.
        max_depth: usize,
    },

    /// Contention `SQLITE_BUSY` après expiration de `busy_timeout`.
    #[error("Base de données occupée (SQLITE_BUSY) : {0}")]
    StorageBusy(String),

    /// Espace disque libre inférieur au seuil critique (1 GiB par défaut).
    #[error(
        "Espace disque critique insuffisant : {available_bytes} octets libres (< {required_bytes})"
    )]
    StorageLowSpace {
        /// Octets disponibles observés.
        available_bytes: u64,
        /// Seuil minimum exigé.
        required_bytes: u64,
    },

    /// Disque plein ou quota pages atteint (`SQLITE_FULL`).
    #[error("Stockage plein (SQLITE_FULL) : {0}")]
    StorageFull(String),

    /// Erreur d'I/O stockage (`SQLITE_IOERR`) ou canal writer fermé.
    #[error("Stockage indisponible (SQLITE_IOERR) : {0}")]
    StorageUnavailable(String),

    /// Corruption détectée (`SQLITE_CORRUPT`, `SQLITE_NOTADB`, échec `integrity_check`).
    #[error("Corruption de stockage détectée : {0}")]
    StorageCorrupt(String),

    /// Violation d'un invariant interne ou d'une contrainte domaine en base.
    #[error("Violation d'invariant de stockage : {0}")]
    InvariantViolation(String),

    /// Lignée d'autorité (`authority_generation`) différente de la lignée active du daemon.
    #[error("Lignée d'autorité obsolète : attendue '{expected}', reçue '{actual}'")]
    StaleAuthorityGeneration {
        /// Lignée active en base.
        expected: AuthorityGeneration,
        /// Lignée portée par la commande ou le jeton.
        actual: AuthorityGeneration,
        /// Identifiant de commande associé si connu.
        command_id: Option<CommandId>,
    },

    /// Époque ou fence monotone obsolète dans la même lignée.
    #[error("Fence ou époque obsolète : {message}")]
    StaleFence {
        /// Détail du refus de fence.
        message: String,
    },

    /// Même `(actor_id, command_id)` avec un digest canonique différent.
    #[error("Conflit d'idempotence pour la commande {command_id}")]
    IdempotencyConflict {
        /// Identifiant de commande en conflit.
        command_id: CommandId,
    },

    /// Réponse purgée dans le tombstone : la fenêtre garantie d'idempotence a expiré.
    #[error("Fenêtre d'idempotence expirée pour la commande {command_id} (reçu {receipt_id})")]
    IdempotencyWindowExpired {
        /// Commande expirée.
        command_id: CommandId,
        /// Reçu historique conservé pour inspection causale.
        receipt_id: ReceiptId,
    },

    /// Révision attendue différente de la révision courante de l'agrégat.
    #[error("Conflit de révision : attendue {expected_revision}, courante {current_revision}")]
    RevisionConflict {
        /// Révision courante persistée.
        current_revision: Revision,
        /// Révision fournie par la commande.
        expected_revision: Revision,
        /// Identifiant de commande associé.
        command_id: Option<CommandId>,
    },

    /// Version de protocole IPC refusée (ex: `unleash.ipc.v1`).
    #[error("Protocole incompatible : '{version}'")]
    ProtocolIncompatible {
        /// Chaîne de version fournie.
        version: String,
    },

    /// Argument ou contrat d'entrée invalide.
    #[error("Argument invalide : {message}")]
    InvalidArgument {
        /// Code de raison associé éventuel.
        reason: Option<ReasonCode>,
        /// Description de l'erreur.
        message: String,
    },

    /// Publication ou prise de pin visant un objet en cours de suppression (`deleting`).
    #[error("Objet {object_hash} en cours de suppression (OBJECT_DELETING)")]
    ObjectDeleting {
        /// Empreinte de l'objet visé.
        object_hash: Sha256Digest,
    },

    /// Tentative de suppression d'un objet encore protégé par un pin.
    #[error("Objet {object_hash} protégé par un pin actif")]
    ObjectPinned {
        /// Empreinte de l'objet protégé.
        object_hash: Sha256Digest,
    },

    /// Tentative de suppression d'un objet encore référencé en état `live`.
    #[error("Objet {object_hash} encore référencé")]
    ObjectReferenced {
        /// Empreinte de l'objet référencé.
        object_hash: Sha256Digest,
    },

    /// Une barrière globale de sauvegarde bloque la suppression d'objets.
    #[error("Barrière globale de sauvegarde active")]
    BackupBarrierActive,

    /// Dépassement d'un compte budgétaire atomique.
    #[error("Budget épuisé sur le compte '{account_id}'")]
    BudgetExhausted {
        /// Compte bloquant.
        account_id: String,
    },

    /// Tarif absent pour l'action demandée.
    #[error("Tarif inconnu pour le modèle '{model_id}'")]
    PricingUnknown {
        /// Modèle concerné.
        model_id: String,
    },

    /// Tarif périmé sous politique `pricing_stale_policy=block`.
    #[error("Catalogue tarifaire périmé '{pricing_version}'")]
    PricingStale {
        /// Version tarifaire concernée.
        pricing_version: String,
    },

    /// Capacité d'exécution saturée ou gelée.
    #[error("Capacité d'exécution indisponible : {0}")]
    CapacityBlocked(String),

    /// Configuration incohérente de la réserve de contrôle.
    #[error("Réserve de contrôle incohérente : {0}")]
    ControlReserveIncoherent(String),

    /// Le prédécesseur d'un run n'est pas terminal.
    #[error("Prédécesseur non terminal (état '{state}')")]
    PredecessorNotTerminal {
        /// État actuel du prédécesseur.
        state: String,
    },

    /// Un run possède déjà un successeur direct.
    #[error("Un successeur direct existe déjà")]
    SuccessorExists,

    /// Restauration bloquée par un inventaire incomplet ou un exécuteur non neutralisé.
    #[error("Restauration bloquée : {0}")]
    RestorationBlocked(String),

    /// Sauvegarde, capsule ou prérequis de restauration incomplets.
    #[error("Sauvegarde ou restauration incomplète : {0}")]
    RestoreIncomplete(String),

    /// Preuve ou objet manquant ou indisponible.
    #[error("Preuve ou objet indisponible : {0}")]
    EvidenceUnavailable(String),

    /// Flux d'événements ou curseur obsolète nécessitant une resynchronisation par snapshot.
    #[error("Resynchronisation requise sur le flux {stream_id} : {reason}")]
    ResyncRequired {
        /// Identifiant du flux actif.
        stream_id: StreamId,
        /// Watermark courant.
        watermark: EventCursor,
        /// Motif du resync.
        reason: String,
    },

    /// Seuil d'arrêt WAL (32 MiB par défaut) atteint : écritures non critiques refusées.
    #[error("Seuil d'arrêt WAL atteint ({wal_bytes} >= {limit_bytes} octets)")]
    WalPressureStop {
        /// Taille WAL observée.
        wal_bytes: u64,
        /// Plafond d'arrêt configuré.
        limit_bytes: u64,
    },

    /// Une transaction dépasse la borne de croissance WAL (1 MiB par défaut).
    #[error("Croissance WAL par transaction dépassée ({growth_bytes} > {max_bytes} octets)")]
    TxWalGrowthExceeded {
        /// Croissance mesurée.
        growth_bytes: u64,
        /// Plafond autorisé par transaction.
        max_bytes: u64,
    },

    /// Un export dépasse son quota d'octets autorisé (8 MiB par défaut).
    #[error("Quota d'export dépassé ({bytes_written} > {max_bytes} octets)")]
    ExportQuotaExceeded {
        /// Octets émis avant arrêt.
        bytes_written: u64,
        /// Quota configuré.
        max_bytes: u64,
    },

    /// Transaction de lecture dépassant la borne de durée (5 s par défaut).
    #[error("Transaction de lecture expirée ({elapsed_ms}ms > {max_ms}ms)")]
    ReadTimeout {
        /// Durée écoulée en ms.
        elapsed_ms: u64,
        /// Durée maximale en ms.
        max_ms: u64,
    },

    /// Version de schéma supérieure au maximum supporté par ce binaire.
    #[error("Version de schéma {found} non supportée (maximum supporté : {supported_max})")]
    UnsupportedSchemaVersion {
        /// Version trouvée en base.
        found: i64,
        /// Version maximale supportée.
        supported_max: i64,
    },

    /// Migration interrompue détectée dans `schema_migrations`.
    #[error("Migration v{version} interrompue : {detail}")]
    MigrationInterrupted {
        /// Version concernée.
        version: i64,
        /// Détail d'état.
        detail: String,
    },

    /// Somme de contrôle d'une migration divergente du binaire.
    #[error(
        "Checksum divergent pour la migration v{version} (attendu {expected}, observé {actual})"
    )]
    MigrationChecksumMismatch {
        /// Version concernée.
        version: i64,
        /// Checksum attendu.
        expected: String,
        /// Checksum observé en base.
        actual: String,
    },

    /// Sauvegarde préalable vérifiée obligatoire avant migration.
    #[error("Sauvegarde préalable vérifiée obligatoire avant migration")]
    MigrationBackupRequired,

    /// Entité introuvable.
    #[error("Entité introuvable : {0}")]
    NotFound(String),

    /// Panne ou arrêt déclenché par un point de coupure de test.
    #[error("Panne injectée au point de coupure {cutpoint} : {message}")]
    CutpointInjected {
        /// Nom du point de coupure.
        cutpoint: String,
        /// Motif configuré.
        message: String,
    },

    /// Erreur d'I/O système.
    #[error("Erreur I/O : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de sérialisation JSON.
    #[error("Erreur JSON : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur protocolaire.
    #[error("Erreur protocolaire : {0}")]
    Protocol(#[from] unleash_protocol::ProtocolError),

    /// Erreur de transition de domaine.
    #[error("Erreur de domaine : {0}")]
    Domain(#[from] unleash_domain::DomainTransitionError),

    /// Opération non autorisée pour l'appelant (`CapabilityDenied/OPERATION_NOT_AUTHORIZED`).
    #[error("Opération non autorisée : {0}")]
    Unauthorized(String),

    /// Identité du pair ou du décideur non vérifiée (`CapabilityDenied/IDENTITY_UNVERIFIED`).
    #[error("Identité non vérifiée : {0}")]
    IdentityUnverified(String),

    /// Capacité refusée avec code de raison explicite (`SubjectSuspended`, `SubjectRevoked`, etc.).
    #[error("Capacité refusée : {message}")]
    CapabilityDenied {
        /// Code de raison associé.
        reason: ReasonCode,
        /// Détail du refus.
        message: String,
    },

    /// Accès refusé au scope demandé (`ScopeDenied`).
    #[error("Scope refusé : {message}")]
    ScopeDenied {
        /// Raison éventuelle (`ACCESS_DENIED`, `ACCESS_REVOKED`, etc.).
        reason: Option<ReasonCode>,
        /// Description du refus.
        message: String,
    },

    /// Violation contractuelle explicite (`ContractViolation`).
    #[error("Violation de contrat : {message}")]
    ContractViolation {
        /// Raison éventuelle.
        reason: Option<ReasonCode>,
        /// Description de l'erreur contractuelle.
        message: String,
    },

    /// Procédure de récupération en cours ou scope gelé (`ResourceBusy/RECOVERY_IN_PROGRESS`).
    #[error("Récupération en cours ou scope gelé : {0}")]
    RecoveryInProgress(String),

    /// Issue d'effet externe inconnue (`OutcomeUnknown/UNRESOLVED_EFFECT`).
    #[error("Issue d'action {action_id} inconnue : {detail}")]
    ActionOutcomeUnknown {
        /// Identifiant de l'action incertaine.
        action_id: ActionId,
        /// Détail de l'incertitude.
        detail: String,
    },

    /// Incohérence d'inventaire ou de comptabilité budgétaire (`ResourceBusy/ACCOUNTING_MISMATCH`).
    #[error("Incohérence comptable ou d'inventaire : {0}")]
    AccountingMismatch(String),

    /// Erreur de validation d'identifiant ou de digest.
    #[error("Identifiant ou digest invalide : {0}")]
    IdValidation(#[from] unleash_domain::IdValidationError),
}

impl StoreError {
    /// Convertit l'erreur de stockage en classe wire [`ErrorCode`].
    #[must_use]
    pub const fn error_code(&self) -> ErrorCode {
        match self {
            Self::BuildUnqualified { .. }
            | Self::StorageFull(_)
            | Self::StorageUnavailable(_)
            | Self::StorageCorrupt(_)
            | Self::InvariantViolation(_)
            | Self::UnsupportedSchemaVersion { .. }
            | Self::MigrationInterrupted { .. }
            | Self::MigrationChecksumMismatch { .. }
            | Self::Io(_)
            | Self::Json(_)
            | Self::CutpointInjected { .. } => ErrorCode::InternalFailure,

            Self::Backpressure { .. }
            | Self::StorageBusy(_)
            | Self::StorageLowSpace { .. }
            | Self::ObjectDeleting { .. }
            | Self::ObjectPinned { .. }
            | Self::BackupBarrierActive
            | Self::CapacityBlocked(_)
            | Self::PredecessorNotTerminal { .. }
            | Self::RestorationBlocked(_)
            | Self::RecoveryInProgress(_)
            | Self::AccountingMismatch(_)
            | Self::WalPressureStop { .. }
            | Self::ReadTimeout { .. } => ErrorCode::ResourceBusy,

            Self::StaleAuthorityGeneration { .. } | Self::StaleFence { .. } => {
                ErrorCode::StaleFence
            }

            Self::IdempotencyConflict { .. } => ErrorCode::IdempotencyConflict,

            Self::IdempotencyWindowExpired { .. }
            | Self::ObjectReferenced { .. }
            | Self::RestoreIncomplete(_)
            | Self::EvidenceUnavailable(_)
            | Self::MigrationBackupRequired
            | Self::ContractViolation { reason: None, .. }
            | Self::Domain(_) => ErrorCode::ContractViolation,

            Self::ContractViolation {
                reason: Some(r), ..
            }
            | Self::ScopeDenied {
                reason: Some(r), ..
            }
            | Self::InvalidArgument {
                reason: Some(r), ..
            } => r.error_code(),

            Self::ActionOutcomeUnknown { .. } => ErrorCode::OutcomeUnknown,

            Self::Unauthorized(_) | Self::IdentityUnverified(_) => ErrorCode::CapabilityDenied,
            Self::CapabilityDenied { reason, .. } => reason.error_code(),
            Self::ScopeDenied { reason: None, .. } => ErrorCode::ScopeDenied,

            Self::RevisionConflict { .. } => ErrorCode::RevisionConflict,
            Self::ProtocolIncompatible { .. } => ErrorCode::ProtocolIncompatible,

            Self::InvalidArgument { reason: None, .. }
            | Self::ControlReserveIncoherent(_)
            | Self::SuccessorExists
            | Self::TxWalGrowthExceeded { .. }
            | Self::ExportQuotaExceeded { .. }
            | Self::Protocol(_)
            | Self::IdValidation(_) => ErrorCode::InvalidArgument,

            Self::BudgetExhausted { .. }
            | Self::PricingUnknown { .. }
            | Self::PricingStale { .. } => ErrorCode::BudgetExceeded,

            Self::ResyncRequired { .. } => ErrorCode::ResyncRequired,
            Self::NotFound(_) => ErrorCode::NotFoundOrHidden,
        }
    }

    /// Renvoie le [`ReasonCode`] canonique associé lorsqu'il existe.
    #[must_use]
    pub const fn reason_code(&self) -> Option<ReasonCode> {
        match self {
            Self::BuildUnqualified { .. } => Some(ReasonCode::StorageBuildUnqualified),
            Self::Backpressure { .. } | Self::WalPressureStop { .. } => {
                Some(ReasonCode::Backpressure)
            }
            Self::StorageBusy(_) | Self::ReadTimeout { .. } => Some(ReasonCode::StorageBusy),
            Self::StorageLowSpace { .. } => Some(ReasonCode::StorageLowSpace),
            Self::StorageFull(_) => Some(ReasonCode::StorageFull),
            Self::StorageUnavailable(_) => Some(ReasonCode::StorageUnavailable),
            Self::StorageCorrupt(_) => Some(ReasonCode::StorageCorrupt),
            Self::InvariantViolation(_)
            | Self::UnsupportedSchemaVersion { .. }
            | Self::MigrationInterrupted { .. }
            | Self::MigrationChecksumMismatch { .. } => Some(ReasonCode::InvariantViolation),
            Self::StaleAuthorityGeneration { .. } => Some(ReasonCode::StaleAuthorityGeneration),
            Self::StaleFence { .. } => Some(ReasonCode::StaleEpoch),
            Self::IdempotencyWindowExpired { .. } => Some(ReasonCode::IdempotencyWindowExpired),
            Self::ObjectDeleting { .. } => Some(ReasonCode::ObjectDeleting),
            Self::BudgetExhausted { .. } => Some(ReasonCode::BudgetExhausted),
            Self::PricingUnknown { .. } => Some(ReasonCode::PricingUnknown),
            Self::PricingStale { .. } => Some(ReasonCode::PricingStale),
            Self::ControlReserveIncoherent(_) => Some(ReasonCode::ControlReserveIncoherent),
            Self::PredecessorNotTerminal { .. } => Some(ReasonCode::PredecessorNotTerminal),
            Self::SuccessorExists => Some(ReasonCode::SuccessorExists),
            Self::RestorationBlocked(_) => Some(ReasonCode::RestorationBlocked),
            Self::RestoreIncomplete(_) | Self::MigrationBackupRequired => {
                Some(ReasonCode::RestoreIncomplete)
            }
            Self::EvidenceUnavailable(_) => Some(ReasonCode::EvidenceUnavailable),
            Self::Unauthorized(_) => Some(ReasonCode::OperationNotAuthorized),
            Self::IdentityUnverified(_) => Some(ReasonCode::IdentityUnverified),
            Self::CapabilityDenied { reason, .. } => Some(*reason),
            Self::RecoveryInProgress(_) => Some(ReasonCode::RecoveryInProgress),
            Self::ActionOutcomeUnknown { .. } => Some(ReasonCode::UnresolvedEffect),
            Self::AccountingMismatch(_) => Some(ReasonCode::AccountingMismatch),
            Self::ContractViolation { reason, .. }
            | Self::ScopeDenied { reason, .. }
            | Self::InvalidArgument { reason, .. } => *reason,
            Self::ResyncRequired { .. } => Some(ReasonCode::CursorExpired),
            Self::TxWalGrowthExceeded { .. } | Self::ExportQuotaExceeded { .. } => {
                Some(ReasonCode::SchemaLimitExceeded)
            }
            _ => None,
        }
    }

    /// Renvoie la politique de reprise [`RetryClass`] conforme au contrat canonique.
    #[must_use]
    pub const fn retry_class(&self) -> RetryClass {
        if let Some(reason) = self.reason_code() {
            return reason.default_retry_class();
        }
        match self {
            Self::RevisionConflict { .. } | Self::ObjectPinned { .. } => RetryClass::AfterRefresh,
            Self::BackupBarrierActive | Self::CapacityBlocked(_) => RetryClass::AfterBackoff,
            Self::Io(_) | Self::Json(_) | Self::CutpointInjected { .. } => {
                RetryClass::AfterReconciliation
            }
            _ => RetryClass::Never,
        }
    }

    /// Projette cette erreur vers le DTO wire normatif [`IpcError`].
    #[must_use]
    pub fn to_ipc_error(&self) -> IpcError {
        let mut err = IpcError::new(
            self.error_code(),
            self.reason_code(),
            self.to_string(),
            self.retry_class(),
        );
        match self {
            Self::StaleAuthorityGeneration {
                command_id: Some(cmd),
                ..
            } => {
                err = err.with_command_id(*cmd);
            }
            Self::IdempotencyConflict { command_id }
            | Self::IdempotencyWindowExpired { command_id, .. } => {
                err = err.with_command_id(*command_id);
            }
            Self::RevisionConflict {
                current_revision,
                command_id,
                ..
            } => {
                err = err.with_current_revision(*current_revision);
                if let Some(cmd) = command_id {
                    err = err.with_command_id(*cmd);
                }
            }
            _ => {}
        }
        err
    }

    /// Convertit une erreur `rusqlite` vers la classe de stockage exacte (`SQLITE_BUSY`, `FULL`, `IOERR`, `CORRUPT`).
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Permet l'usage direct `.map_err(StoreError::from_rusqlite)` sur rusqlite::Result"
    )]
    pub fn from_rusqlite(err: rusqlite::Error) -> Self {
        match &err {
            rusqlite::Error::SqliteFailure(ffi_err, msg) => {
                let detail = msg
                    .as_deref()
                    .map_or_else(|| err.to_string(), ToOwned::to_owned);
                match ffi_err.code {
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                        Self::StorageBusy(detail)
                    }
                    rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::TooBig => {
                        Self::StorageFull(detail)
                    }
                    rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::CannotOpen => {
                        Self::StorageUnavailable(detail)
                    }
                    rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                        Self::StorageCorrupt(detail)
                    }
                    rusqlite::ErrorCode::ConstraintViolation => Self::InvariantViolation(detail),
                    _ => Self::StorageUnavailable(detail),
                }
            }
            rusqlite::Error::QueryReturnedNoRows => {
                Self::NotFound("aucune ligne retournée".to_owned())
            }
            other => Self::InvariantViolation(other.to_string()),
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::from_rusqlite(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unleash_domain::CommandId;

    #[expect(
        clippy::too_many_lines,
        reason = "Catalogue complet des variantes StoreError vérifiées en projection IPC"
    )]
    fn build_error_samples() -> Vec<StoreError> {
        let auth = AuthorityGeneration::from_u128(1);
        let cmd = CommandId::from_u128(2);
        let rcpt = ReceiptId::from_u128(3);
        let digest = Sha256Digest::from_bytes(b"digest");
        let stream = StreamId::from_u128(4);
        let cursor = EventCursor::new(stream, unleash_domain::Sequence::new(1));

        vec![
            StoreError::BuildUnqualified {
                detail: "test".to_owned(),
            },
            StoreError::Backpressure {
                queue_depth: 8,
                max_depth: 8,
            },
            StoreError::WalPressureStop {
                wal_bytes: 33,
                limit_bytes: 32,
            },
            StoreError::StorageBusy("busy".to_owned()),
            StoreError::ReadTimeout {
                elapsed_ms: 5001,
                max_ms: 5000,
            },
            StoreError::StorageLowSpace {
                available_bytes: 1,
                required_bytes: 2,
            },
            StoreError::StorageFull("full".to_owned()),
            StoreError::StorageUnavailable("io".to_owned()),
            StoreError::StorageCorrupt("corrupt".to_owned()),
            StoreError::InvariantViolation("inv".to_owned()),
            StoreError::UnsupportedSchemaVersion {
                found: 99,
                supported_max: 2,
            },
            StoreError::MigrationInterrupted {
                version: 1,
                detail: "interrupted".to_owned(),
            },
            StoreError::MigrationChecksumMismatch {
                version: 1,
                expected: "a".to_owned(),
                actual: "b".to_owned(),
            },
            StoreError::StaleAuthorityGeneration {
                expected: auth,
                actual: AuthorityGeneration::from_u128(9),
                command_id: Some(cmd),
            },
            StoreError::StaleFence {
                message: "stale".to_owned(),
            },
            StoreError::IdempotencyWindowExpired {
                command_id: cmd,
                receipt_id: rcpt,
            },
            StoreError::ObjectDeleting {
                object_hash: digest,
            },
            StoreError::BudgetExhausted {
                account_id: "acc".to_owned(),
            },
            StoreError::PricingUnknown {
                model_id: "m".to_owned(),
            },
            StoreError::PricingStale {
                pricing_version: "pv".to_owned(),
            },
            StoreError::ControlReserveIncoherent("bad".to_owned()),
            StoreError::PredecessorNotTerminal {
                state: "running".to_owned(),
            },
            StoreError::SuccessorExists,
            StoreError::RestorationBlocked("blocked".to_owned()),
            StoreError::RestoreIncomplete("incomplete".to_owned()),
            StoreError::MigrationBackupRequired,
            StoreError::EvidenceUnavailable("ev".to_owned()),
            StoreError::ResyncRequired {
                stream_id: stream,
                watermark: cursor,
                reason: "expired".to_owned(),
            },
            StoreError::TxWalGrowthExceeded {
                growth_bytes: 2,
                max_bytes: 1,
            },
            StoreError::ExportQuotaExceeded {
                bytes_written: 9,
                max_bytes: 8,
            },
            StoreError::InvalidArgument {
                reason: Some(ReasonCode::OperationNotAuthorized),
                message: "denied".to_owned(),
            },
            StoreError::Unauthorized("denied".to_owned()),
            StoreError::IdentityUnverified("unverified".to_owned()),
            StoreError::CapabilityDenied {
                reason: ReasonCode::SubjectSuspended,
                message: "suspended".to_owned(),
            },
            StoreError::ScopeDenied {
                reason: Some(ReasonCode::AccessRevoked),
                message: "revoked".to_owned(),
            },
            StoreError::ContractViolation {
                reason: Some(ReasonCode::TerminalImmutable),
                message: "immutable".to_owned(),
            },
            StoreError::RecoveryInProgress("recovery".to_owned()),
            StoreError::ActionOutcomeUnknown {
                action_id: ActionId::from_u128(5),
                detail: "unknown".to_owned(),
            },
            StoreError::AccountingMismatch("mismatch".to_owned()),
        ]
    }

    #[test]
    fn test_reason_codes_align_with_error_codes_and_ipc_projection() {
        for err in build_error_samples() {
            if let Some(rc) = err.reason_code() {
                assert_eq!(
                    err.error_code(),
                    rc.error_code(),
                    "error_code diverge pour {err:?} ({rc:?})"
                );
                let ipc = err.to_ipc_error();
                assert_eq!(ipc.code, rc.error_code());
                assert_eq!(ipc.reason_code, Some(rc));
            }
        }
    }
}
