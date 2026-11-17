// SPDX-License-Identifier: Apache-2.0
//! Erreurs typées de la crate client IPC `unleash-client`.

use thiserror::Error;
use unleash_domain::{AuthorityGeneration, CommandId};
use unleash_platform::PlatformError;
use unleash_protocol::{IpcError, ProtocolError};

/// Erreur produite par le transport, la négociation, la corrélation ou la synchronisation
/// dans `unleash-client`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClientError {
    /// Erreur de la couche OS (`unleash-platform`) lors du contrôle des permissions ou du socket.
    #[error("Erreur plateforme : {0}")]
    Platform(#[from] PlatformError),

    /// Erreur de codec, de canonisation ou de décodage strict `unleash-protocol`.
    #[error("Erreur protocolaire : {0}")]
    Protocol(#[from] ProtocolError),

    /// Erreur métier ou technique renvoyée sur le fil par le daemon ([`IpcError`]).
    #[error("Erreur distante daemon ({}) : {}", .0.code, .0.message)]
    Remote(IpcError),

    /// Réponse inattendue ou violation de contrat de cadrage de la part du serveur.
    #[error("Réponse IPC inattendue (attendu {expected}) : {detail}")]
    UnexpectedResponse {
        /// Type de trame attendu.
        expected: &'static str,
        /// Détail diagnostic sans secret.
        detail: String,
    },

    /// Incohérence d'identifiant de corrélation (`request_id` ou `command_id`) dans la réponse.
    #[error("Incohérence de corrélation IPC : attendu '{expected}', reçu '{actual}'")]
    CorrelationMismatch {
        /// Identifiant attendu.
        expected: String,
        /// Identifiant reçu.
        actual: String,
    },

    /// Commande définitivement verrouillée après refus `STALE_AUTHORITY_GENERATION`.
    ///
    /// Le client ne réécrit jamais automatiquement `authority_generation` ou `command_id`.
    #[error(
        "Commande {command_id} verrouillée après StaleFence/STALE_AUTHORITY_GENERATION (lignée épinglée {pinned_generation})"
    )]
    StaleLineageLocked {
        /// Identifiant de la commande rejetée.
        command_id: CommandId,
        /// Lignée d'origine épinglée lors de sa création durable.
        pinned_generation: AuthorityGeneration,
    },

    /// Tentative de réémission d'une commande déjà dans un état terminal (`Confirmed` / `Failed`).
    #[error("Commande {command_id} déjà dans un état terminal ; aucune réémission possible")]
    AlreadyTerminal {
        /// Identifiant de commande concerné.
        command_id: CommandId,
    },

    /// Commande introuvable dans la file locale de corrélation.
    #[error("Commande {command_id} absente de la file locale de corrélation")]
    UnknownCommand {
        /// Identifiant de commande absent.
        command_id: CommandId,
    },

    /// Conflit d'idempotence détecté localement dans la file de corrélation.
    #[error("Conflit d'idempotence local pour {command_id} : digest divergent")]
    LocalIdempotencyConflict {
        /// Identifiant de commande en conflit.
        command_id: CommandId,
    },

    /// Charge utile `client_upload` dépassant le plafond strict de 10 MiB (`MAX_CLIENT_UPLOAD_BYTES`).
    #[error("Import client_upload trop volumineux : {length} octets (maximum {max} octets)")]
    UploadTooLarge {
        /// Taille totale soumise en octets.
        length: u64,
        /// Plafond maximal autorisé.
        max: u64,
    },

    /// Réponse `GetStateSnapshot` dépourvue du watermark transactionnel requis.
    #[error("Réponse GetStateSnapshot invalide : watermark transactionnel absent")]
    MissingSnapshotWatermark,

    /// Délai dépassé sur une attente IPC (connexion ou requête).
    #[error("Délai dépassé sur l'opération IPC '{operation}' après {timeout_ms} ms")]
    Timeout {
        /// Nom de l'opération.
        operation: &'static str,
        /// Délai configuré en millisecondes.
        timeout_ms: u64,
    },

    /// Connexion Unix fermée de façon inattendue avant la réponse complète.
    #[error("Connexion IPC interrompue pendant '{operation}'")]
    Disconnected {
        /// Opération en cours lors de la coupure.
        operation: &'static str,
    },

    /// Fichier de file de corrélation locale invalide ou corrompu.
    #[error("File locale de corrélation invalide : {reason}")]
    InvalidCorrelationStore {
        /// Motif du rejet.
        reason: String,
    },

    /// Erreur d'I/O système.
    #[error("Erreur I/O client IPC : {0}")]
    Io(#[from] std::io::Error),
}
