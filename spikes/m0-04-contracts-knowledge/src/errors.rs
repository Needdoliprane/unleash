//! Types d'erreurs du spike M0 n°4 (contrats et connaissance multilangage).

use thiserror::Error;

/// Erreurs typées produites par le harnais d'expérience contrats & connaissance.
#[derive(Debug, Error)]
pub enum SpikeError {
    /// Erreur d'entrée/sortie système ou d'exécution Git locale.
    #[error("Erreur I/O : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de création de répertoire temporaire local via `unleash-testkit`.
    #[error("Erreur répertoire temporaire : {0}")]
    TempDir(#[from] unleash_testkit::TempDirError),

    /// Échec d'une commande Git locale.
    #[error("Commande Git '{args}' en échec (code={status:?}) : {stderr}")]
    GitCommandFailed {
        /// Arguments passés à `git`.
        args: String,
        /// Code de sortie observé.
        status: Option<i32>,
        /// Sortie d'erreur standard.
        stderr: String,
    },

    /// Erreur de sérialisation ou canonisation protocolaire.
    #[error("Erreur protocolaire : {0}")]
    Protocol(#[from] unleash_protocol::ProtocolError),

    /// Erreur de sérialisation/désérialisation JSON.
    #[error("Erreur JSON : {0}")]
    Json(#[from] serde_json::Error),

    /// Erreur de validation d'identifiant ou de digest du domaine.
    #[error("Erreur identifiant/digest : {0}")]
    IdValidation(#[from] unleash_domain::IdValidationError),

    /// Échec de chargement ou de compilation d'un JSON Schema local.
    #[error("Erreur de schéma '{schema}' : {reason}")]
    SchemaBuild {
        /// Identifiant ou chemin du schéma concerné.
        schema: String,
        /// Raison de l'échec.
        reason: String,
    },

    /// Violation d'un JSON Schema lors de la validation d'une instance produite.
    #[error("Validation JSON Schema '{schema}' échouée : {details}")]
    SchemaValidation {
        /// Identifiant du schéma violé.
        schema: String,
        /// Détails agrégés des erreurs de validation.
        details: String,
    },

    /// Erreur d'horodatage invalide.
    #[error("Horodatage UTC invalide : {0}")]
    InvalidTimestamp(i64),

    /// Violation des règles de construction ou d'autorité d'un objet contractuel.
    #[error("Invariant contractuel violé : {0}")]
    ContractInvariant(String),
}
