//! # DTOs et enveloppes normatives du protocole IPC (`unleash.ipc.v2`)
//!
//! Responsabilité exclusive : Définition des structures de données (DTOs) de commandes,
//! reçus, événements, requêtes, poignées de main et segments, avec sérialisation stricte
//! `#[serde(deny_unknown_fields)]` et exposition TypeScript (`ts-rs`).

use crate::codec::MAX_FRAME_BYTES;
use crate::errors::ProtocolError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use ts_rs::TS;
use unleash_domain::{
    AuthorityGeneration, CommandId, DaemonEpoch, DaemonInstanceId, OperationId, ProjectId,
    ReceiptId, RequestId, Revision, Sequence, Sha256Digest, StreamId,
};

/// Constante de version protocolaire pour `unleash.ipc.v2`.
pub const PROTOCOL_VERSION_V2: &str = "unleash.ipc.v2";

/// Constante de version protocolaire obsolète `unleash.ipc.v1`.
pub const PROTOCOL_VERSION_V1: &str = "unleash.ipc.v1";

/// Plafond maximal d'octets pour un import de données `client_upload` (10 MiB = `10_485_760` octets).
pub const MAX_CLIENT_UPLOAD_BYTES: u64 = 10 * 1024 * 1024;

/// Version du protocole IPC Unleash.
///
/// Refuse strictement `unleash.ipc.v1` et exige `unleash.ipc.v2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, TS)]
#[ts(type = "\"unleash.ipc.v2\"")]
pub struct ProtocolVersion;

impl ProtocolVersion {
    /// Chaîne canonique de la version de protocole en vigueur.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        PROTOCOL_VERSION_V2
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(PROTOCOL_VERSION_V2)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == PROTOCOL_VERSION_V2 {
            Ok(Self)
        } else if s == PROTOCOL_VERSION_V1 {
            Err(de::Error::custom(format!(
                "Version de protocole 'unleash.ipc.v1' refusée : mise à niveau vers unleash.ipc.v2 requise [protocol_incompatible:{s}]"
            )))
        } else {
            Err(de::Error::custom(format!(
                "Version de protocole inconnue ou incompatible : '{s}' [protocol_incompatible:{s}]"
            )))
        }
    }
}

/// Enveloppe commune de commande mutante (`ipc-command.schema.json`).
///
/// Conforme à `unleash.ipc.v2` :
/// - `authority_generation` épinglée côté client lors de la création durable de l'intention.
/// - `aggregate_id` et `expected_revision` obligatoires (0 lors d'une création).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    /// Version du protocole (doit être `unleash.ipc.v2`).
    pub protocol_version: ProtocolVersion,
    /// Identifiant de la transmission réseau (peut changer au retry).
    #[ts(type = "string")]
    pub request_id: RequestId,
    /// Identifiant d'idempotence durable de la commande.
    #[ts(type = "string")]
    pub command_id: CommandId,
    /// Lignée serveur épinglée lors de la création durable.
    #[ts(type = "string")]
    pub authority_generation: AuthorityGeneration,
    /// Identifiant de l'acteur authentifié résolu serveur (null si non fourni).
    pub actor_ref: Option<String>,
    /// Identifiant du projet concerné ou null.
    #[ts(type = "string | null")]
    pub project_id: Option<ProjectId>,
    /// Agrégat ciblé (UUID).
    #[ts(type = "string")]
    pub aggregate_id: uuid::Uuid,
    /// Révision attendue de l'agrégat (0 pour une création).
    #[ts(type = "number")]
    pub expected_revision: Revision,
    /// Nom canonique de l'opération (ex: `PauseMission`).
    pub command_type: String,
    /// Charge utile métier de la commande (objet JSON).
    #[ts(type = "Record<string, unknown>")]
    pub payload: serde_json::Map<String, serde_json::Value>,
}

/// État d'avancement d'une commande dans son cycle de vie serveur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ReceiptState {
    /// Commande acceptée et engagée durablement pour traitement.
    Accepted,
    /// Opération entièrement terminée avec succès selon son contrat.
    Completed,
    /// Exécution bloquée par une attente externe, une preuve ou une décision.
    Blocked,
    /// Commande formellement rejetée par le domaine ou les règles métier.
    Rejected,
}

/// Curseur de position dans le flux transactionnel d'événements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EventCursor {
    /// Identifiant du flux d'événements.
    #[ts(type = "string")]
    pub stream_id: StreamId,
    /// Numéro de séquence ordonné dans le flux.
    #[ts(type = "number")]
    pub sequence: Sequence,
}

impl EventCursor {
    /// Crée un nouveau curseur d'événement.
    #[must_use]
    pub const fn new(stream_id: StreamId, sequence: Sequence) -> Self {
        Self {
            stream_id,
            sequence,
        }
    }
}

/// Reçu serveur d'engagement de commande (`ipc-receipt.schema.json`).
///
/// Invariants :
/// - `operation_id` est obligatoirement sérialisé (vaut `null` sans opération longue).
/// - `result_ref` est omis si absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommandReceipt {
    /// Identifiant global unique serveur du reçu.
    #[ts(type = "string")]
    pub receipt_id: ReceiptId,
    /// Identifiant de la commande originale.
    #[ts(type = "string")]
    pub command_id: CommandId,
    /// Horodatage UTC RFC 3339 d'acceptation de la commande.
    pub accepted_at: DateTime<Utc>,
    /// Agrégat ciblé.
    #[ts(type = "string")]
    pub aggregate_id: uuid::Uuid,
    /// Nouvelle révision de l'agrégat suite à l'engagement.
    #[ts(type = "number")]
    pub new_revision: Revision,
    /// Identifiant de l'opération longue initiée ou null. Clé toujours présente.
    #[ts(type = "string | null")]
    pub operation_id: Option<OperationId>,
    /// État d'avancement du traitement.
    pub state: ReceiptState,
    /// Curseur d'événement correspondant dans le journal.
    pub event_cursor: EventCursor,
    /// Référence opaque vers le résultat produit ou null si absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref: Option<String>,
}

/// Statut de filiation d'un reçu lors d'une inspection historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LineageStatus {
    /// Reçu émis au sein de la lignée serveur actuellement active.
    Current,
    /// Reçu historique issu d'une lignée antérieure (avant restauration/recovery).
    Historical,
}

/// Réponse enrichie d'inspection de reçu (`GetCommandReceipt` / inspection historique).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommandReceiptInspection {
    /// Reçu de commande original.
    pub receipt: CommandReceipt,
    /// Lignée d'autorité sous laquelle le reçu a été engagé.
    #[ts(type = "string")]
    pub authority_generation: AuthorityGeneration,
    /// Indication si la lignée est courante ou historique.
    pub lineage_status: LineageStatus,
    /// Référence de provenance pour audit.
    pub provenance_ref: Option<String>,
}

/// Nature causale d'un événement engagé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventCausationKind {
    /// Cause directe : commande cliente acceptée.
    Command,
    /// Cause réactive : événement antérieur du journal.
    Event,
    /// Cause d'effet : retour d'une action exécutée par un worker/broker.
    Action,
    /// Cause temporelle : expiration d'un minuteur ou d'un délai d'attente.
    Timer,
    /// Cause environnementale ou signal externe.
    External,
}

/// Cause typée d'un événement durable (`causation`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EventCausation {
    /// Famille causale.
    pub kind: EventCausationKind,
    /// Identifiant de la cause (ex: `receipt_id` si `kind=command`).
    pub id: String,
}

impl EventCausation {
    /// Crée une causalité issue d'une commande engagée.
    #[must_use]
    pub fn from_command_receipt(receipt_id: ReceiptId) -> Self {
        Self {
            kind: EventCausationKind::Command,
            id: receipt_id.to_string(),
        }
    }
}

/// Niveau de sensibilité de sécurité et de confidentialité d'un événement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Sensitivity {
    /// Information non confidentielle exportable librement.
    Public,
    /// Restreint aux membres du projet.
    Project,
    /// Données sensibles de configuration ou de contenu.
    Confidential,
    /// Données hautement restreintes (droits administratifs stricts).
    Restricted,
}

/// Enveloppe wire d'un événement durable engagé (`event.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    /// Identifiant unique de l'événement (UUID).
    #[ts(type = "string")]
    pub event_id: uuid::Uuid,
    /// Identifiant du flux ordonné auquel appartient l'événement.
    #[ts(type = "string")]
    pub stream_id: StreamId,
    /// Numéro de séquence strictement monotone dans le flux (>= 1).
    #[ts(type = "number")]
    pub sequence: Sequence,
    /// Type de fait intervenu (ex: `InvocationAdmitted`).
    pub event_type: String,
    /// Version de l'enveloppe (vaut 1).
    pub schema_version: u32,
    /// Date-heure UTC de survenance du fait.
    pub occurred_at: DateTime<Utc>,
    /// Date-heure UTC d'enregistrement en base.
    pub recorded_at: DateTime<Utc>,
    /// Projet associé ou null.
    #[ts(type = "string | null")]
    pub project_id: Option<ProjectId>,
    /// Agrégat modifié par cet événement.
    #[ts(type = "string")]
    pub aggregate_id: uuid::Uuid,
    /// Révision résultante de l'agrégat (>= 1).
    #[ts(type = "number")]
    pub aggregate_revision: Revision,
    /// Acteur à l'origine du fait.
    pub actor_ref: String,
    /// Identifiant de corrélation transverse de la requête.
    #[ts(type = "string")]
    pub correlation_id: uuid::Uuid,
    /// Lien causal typé ayant produit cet événement.
    pub causation: EventCausation,
    /// Identifiant de la commande liée (informatif, ou null).
    #[ts(type = "string | null")]
    pub command_id: Option<CommandId>,
    /// Empreinte canonique SHA-256 du payload (`sha256:<hex64>`).
    #[ts(type = "string")]
    pub payload_hash: Sha256Digest,
    /// Charge utile métier de l'événement (objet JSON).
    #[ts(type = "Record<string, unknown>")]
    pub payload: serde_json::Map<String, serde_json::Value>,
    /// Niveau de sensibilité de l'événement.
    pub sensitivity: Sensitivity,
}

/// Enveloppe de requête en lecture (`ipc-query.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QueryEnvelope {
    /// Version du protocole (doit être `unleash.ipc.v2`).
    pub protocol_version: ProtocolVersion,
    /// Identifiant de corrélation de la requête.
    #[ts(type = "string")]
    pub request_id: RequestId,
    /// Nom canonique de la requête (ex: `GetAgent`).
    pub query_type: String,
    /// Périmètre projet optionnel.
    #[ts(type = "string | null")]
    pub project_id: Option<ProjectId>,
    /// Paramètres typés de la lecture (objet JSON).
    #[ts(type = "Record<string, unknown>")]
    pub payload: serde_json::Map<String, serde_json::Value>,
}

/// Réponse à une requête de lecture (`ipc-query-response.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QueryResponse {
    /// Identifiant de corrélation correspondant à la requête.
    #[ts(type = "string")]
    pub request_id: RequestId,
    /// Watermark transactionnel de la vue projetée si applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<EventCursor>,
    /// Résultat de la requête.
    #[ts(type = "unknown")]
    pub result: serde_json::Value,
}

/// Type de client se connectant au daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ClientKind {
    /// Interface en ligne de commande.
    Cli,
    /// Interface terminal texte riche (Ratatui).
    Tui,
    /// Interface graphique native (Tauri/React).
    Gui,
    /// Worker éphémère d'exécution de tâche ou de replay.
    Worker,
    /// Sonde d'évaluation ou de vérification d'environnement.
    Probe,
}

/// Preuve administrative éphémère de 32 octets, transmise uniquement au daemon local.
#[derive(Clone)]
pub struct AdminProof([u8; 32]);

impl AdminProof {
    /// Construit la preuve à partir de 32 octets cryptographiquement aléatoires.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse la forme hexadécimale minuscule exacte sans exposer les entrées invalides.
    #[must_use]
    pub fn from_hex(value: &str) -> Option<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return None;
        }
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(value, &mut bytes).ok()?;
        Some(Self(bytes))
    }

    /// Encode la preuve sous une chaîne effacée à sa destruction.
    #[must_use]
    pub fn to_hex(&self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(hex::encode(self.0))
    }

    /// Compare les preuves en temps constant.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        bool::from(self.0.ct_eq(&other.0))
    }
}
impl std::fmt::Debug for AdminProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdminProof([REDACTED])")
    }
}
impl Drop for AdminProof {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}
impl PartialEq for AdminProof {
    fn eq(&self, other: &Self) -> bool {
        self.matches(other)
    }
}
impl Eq for AdminProof {}
impl Serialize for AdminProof {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}
impl<'de> Deserialize<'de> for AdminProof {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = zeroize::Zeroizing::new(String::deserialize(deserializer)?);
        Self::from_hex(&raw).ok_or_else(|| de::Error::custom("preuve administrative invalide"))
    }
}

/// Poignée de main initiale transmise par le client (`ipc-hello.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HelloRequest {
    /// Version protocolaire annoncée par le client (`unleash.ipc.v2`).
    pub protocol_version: ProtocolVersion,
    /// Nature du client.
    pub client_kind: ClientKind,
    /// Version du binaire client.
    pub client_version: String,
    /// Capacités optionnelles requises ou supportées par le client.
    pub capabilities: Vec<String>,
    /// Preuve réservée au transport Rust ; absente ou nulle pour un accès diagnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub admin_proof: Option<AdminProof>,
}

/// Réponse du daemon à la poignée de main initiale (`ipc-hello.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HelloResponse {
    /// Version protocolaire active du daemon (`unleash.ipc.v2`).
    pub protocol_version: ProtocolVersion,
    /// Lignée d'autorité serveur active.
    #[ts(type = "string")]
    pub authority_generation: AuthorityGeneration,
    /// Époque d'incarnation du daemon au sein de la lignée.
    #[ts(type = "number")]
    pub daemon_epoch: DaemonEpoch,
    /// Identifiant d'instance unique de ce daemon en cours d'exécution.
    #[ts(type = "string")]
    pub daemon_instance_id: DaemonInstanceId,
    /// Identifiant du flux principal d'événements du daemon.
    #[ts(type = "string")]
    pub stream_id: StreamId,
    /// Taille maximale admise d'une trame en octets (8 MiB).
    pub max_frame_bytes: u32,
    /// Taille suggérée des segments pour les uploads découpés (ex: 1 MiB).
    pub suggested_segment_bytes: u32,
    /// Durée de conservation garantie des réponses d'idempotence en secondes (ex: 7 jours).
    #[ts(type = "number")]
    pub idempotency_retention_seconds: u64,
    /// Capacités exposées par le daemon.
    pub capabilities: Vec<String>,
}

/// Capability offered by clients which understand a separate offline diagnostic handshake.
pub const READ_ONLY_RECOVERY_CAPABILITY: &str = "read_only_recovery_v1";

/// Only mode served when durable identity cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticMode {
    /// Read-only recovery without a fabricated authority or event stream.
    ReadOnlyRecovery,
}

/// Cause established before entering the diagnostic service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticReason {
    /// Opening the existing store reported actual corruption.
    StorageCorrupt,
}

/// Diagnostic handshake, intentionally distinct from `HelloResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticHelloResponse {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Bounded offline mode.
    pub mode: DiagnosticMode,
    /// Verified reason for this diagnostic.
    pub reason_code: DiagnosticReason,
    /// Maximum frame size.
    pub max_frame_bytes: u32,
    /// Operations granted by the authenticated session.
    pub capabilities: Vec<String>,
}

/// Demande d'abonnement au flux d'événements (`ipc-subscribe-events.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SubscribeEventsRequest {
    /// Flux d'événements ciblé.
    #[ts(type = "string")]
    pub stream_id: StreamId,
    /// Numéro de séquence à partir duquel reprendre l'écoute (> `after_sequence`).
    #[ts(type = "number")]
    pub after_sequence: Sequence,
    /// Filtre projet optionnel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "string | null")]
    pub project_id: Option<ProjectId>,
    /// Filtre agrégat optionnel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "string | null")]
    pub aggregate_id: Option<uuid::Uuid>,
    /// Nombre maximal d'événements par trame de lot.
    pub batch_limit: u32,
}

/// Trame émise sur le flux d'abonnement aux événements (`ipc-subscribe-events.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum SubscribeEventsFrame {
    /// Lot d'événements ordonnés avec curseur de progression.
    Events {
        /// Liste des événements du lot.
        events: Vec<EventEnvelope>,
        /// Curseur de fin de lot.
        cursor: EventCursor,
    },
    /// Signal exigeant une resynchronisation complète du client (curseur expiré ou bascule de stream).
    ResyncRequired {
        /// Nouveau flux d'événements actif.
        #[ts(type = "string")]
        stream_id: StreamId,
        /// Watermark transactionnel actuel à charger via snapshot.
        current_watermark: EventCursor,
        /// Motif de la demande de resynchronisation.
        reason: String,
    },
}

/// Segment d'upload client pour le transfert de données d'automatisation volumineuses (F019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ClientUploadSegment {
    /// Commande liée à cet import de données.
    #[ts(type = "string")]
    pub command_id: CommandId,
    /// Index du segment (0-based).
    pub segment_index: u32,
    /// Nombre total de segments attendus.
    pub total_segments: u32,
    /// Position de départ en octets dans le fichier source.
    #[ts(type = "number")]
    pub byte_offset: u64,
    /// Longueur de ce segment en octets.
    pub byte_length: u32,
    /// Données brutes du segment encodées en base64.
    pub data_base64: String,
    /// Empreinte canonique SHA-256 du segment.
    #[ts(type = "string")]
    pub sha256: Sha256Digest,
}

impl ClientUploadSegment {
    /// Construit un segment `client_upload` validé à partir d'une tranche d'octets bruts.
    ///
    /// Calcule automatiquement `byte_length`, l'encodage base64 standard RFC 4648 et
    /// l'empreinte canonique SHA-256.
    ///
    /// # Errors
    /// Renvoie [`ProtocolError::Validation`] si `total_segments == 0`,
    /// `segment_index >= total_segments`, si la longueur dépasse la borne de trame,
    /// ou si `byte_offset + bytes.len()` dépasse [`MAX_CLIENT_UPLOAD_BYTES`] (10 MiB).
    pub fn from_bytes(
        command_id: CommandId,
        segment_index: u32,
        total_segments: u32,
        byte_offset: u64,
        bytes: &[u8],
    ) -> Result<Self, ProtocolError> {
        let byte_length = u32::try_from(bytes.len()).map_err(|_| {
            ProtocolError::Validation(format!(
                "Segment client_upload trop volumineux ({} octets)",
                bytes.len()
            ))
        })?;
        let segment = Self {
            command_id,
            segment_index,
            total_segments,
            byte_offset,
            byte_length,
            data_base64: encode_base64_standard(bytes),
            sha256: Sha256Digest::from_bytes(bytes),
        };
        segment.validate()?;
        Ok(segment)
    }

    /// Valide les invariants structurels, les bornes, l'encodage base64 et le digest SHA-256,
    /// puis restitue les octets bruts décodés.
    ///
    /// # Errors
    /// Renvoie [`ProtocolError::Validation`] si :
    /// - `total_segments == 0` ou `segment_index >= total_segments` ;
    /// - `byte_length` ou `byte_offset + byte_length` dépasse les plafonds (`MAX_FRAME_BYTES` / `MAX_CLIENT_UPLOAD_BYTES`) ;
    /// - `data_base64` n'est pas du base64 standard RFC 4648 valide ;
    /// - la longueur décodée diffère de `byte_length` ;
    /// - le SHA-256 calculé diffère de `sha256`.
    pub fn decode_and_validate(&self) -> Result<Vec<u8>, ProtocolError> {
        if self.total_segments == 0 {
            return Err(ProtocolError::Validation(
                "client_upload : total_segments doit être >= 1".to_owned(),
            ));
        }
        if self.segment_index >= self.total_segments {
            return Err(ProtocolError::Validation(format!(
                "client_upload : segment_index ({}) >= total_segments ({})",
                self.segment_index, self.total_segments
            )));
        }
        let expected_len = usize::try_from(self.byte_length).map_err(|_| {
            ProtocolError::Validation(format!(
                "client_upload : byte_length ({}) non représentable",
                self.byte_length
            ))
        })?;
        if expected_len > MAX_FRAME_BYTES {
            return Err(ProtocolError::Validation(format!(
                "client_upload : byte_length ({}) dépasse le plafond de trame ({MAX_FRAME_BYTES})",
                self.byte_length
            )));
        }
        let end_offset = self
            .byte_offset
            .checked_add(u64::from(self.byte_length))
            .ok_or_else(|| {
                ProtocolError::Validation(
                    "client_upload : dépassement arithmétique sur byte_offset + byte_length"
                        .to_owned(),
                )
            })?;
        if end_offset > MAX_CLIENT_UPLOAD_BYTES {
            return Err(ProtocolError::Validation(format!(
                "client_upload : la plage [{}, {end_offset}) dépasse la limite d'import de {MAX_CLIENT_UPLOAD_BYTES} octets",
                self.byte_offset
            )));
        }
        if self.data_base64.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::Validation(format!(
                "client_upload : data_base64 ({} octets) dépasse le plafond de trame ({MAX_FRAME_BYTES})",
                self.data_base64.len()
            )));
        }

        let decoded = decode_base64_strict(&self.data_base64)?;
        if decoded.len() != expected_len {
            return Err(ProtocolError::Validation(format!(
                "client_upload : byte_length annoncé ({}) != octets décodés ({})",
                self.byte_length,
                decoded.len()
            )));
        }

        let computed = Sha256Digest::from_bytes(&decoded);
        if computed != self.sha256 {
            return Err(ProtocolError::Validation(format!(
                "client_upload : digest SHA-256 incompatible (attendu {}, calculé {computed})",
                self.sha256
            )));
        }

        Ok(decoded)
    }

    /// Valide les invariants du segment sans conserver les octets décodés.
    ///
    /// # Errors
    /// Identique à [`Self::decode_and_validate`].
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.decode_and_validate().map(|_| ())
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64_standard(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let &[b0, b1, b2] = chunk else {
            continue;
        };
        let triple = (usize::from(b0) << 16) | (usize::from(b1) << 8) | usize::from(b2);
        push_b64_char(&mut out, (triple >> 18) & 0x3F);
        push_b64_char(&mut out, (triple >> 12) & 0x3F);
        push_b64_char(&mut out, (triple >> 6) & 0x3F);
        push_b64_char(&mut out, triple & 0x3F);
    }

    match chunks.remainder() {
        [b0] => {
            let v = usize::from(*b0) << 16;
            push_b64_char(&mut out, (v >> 18) & 0x3F);
            push_b64_char(&mut out, (v >> 12) & 0x3F);
            out.push_str("==");
        }
        [b0, b1] => {
            let v = (usize::from(*b0) << 16) | (usize::from(*b1) << 8);
            push_b64_char(&mut out, (v >> 18) & 0x3F);
            push_b64_char(&mut out, (v >> 12) & 0x3F);
            push_b64_char(&mut out, (v >> 6) & 0x3F);
            out.push('=');
        }
        _ => {}
    }

    out
}

fn push_b64_char(out: &mut String, idx: usize) {
    if let Some(&ch) = BASE64_ALPHABET.get(idx) {
        out.push(char::from(ch));
    }
}

const fn decode_b64_symbol(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn decode_base64_strict(input: &str) -> Result<Vec<u8>, ProtocolError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let raw = input.as_bytes();
    if !raw.len().is_multiple_of(4) {
        return Err(ProtocolError::Validation(
            "client_upload : longueur base64 invalide (multiple de 4 requis)".to_owned(),
        ));
    }

    let chunk_count = raw.len() / 4;
    let mut out = Vec::with_capacity(chunk_count * 3);

    for (idx, chunk) in raw.chunks_exact(4).enumerate() {
        let is_last = idx + 1 == chunk_count;
        let &[b0, b1, b2, b3] = chunk else {
            continue;
        };

        let v0 = decode_b64_symbol(b0).ok_or_else(|| {
            ProtocolError::Validation("client_upload : caractère base64 invalide".to_owned())
        })?;
        let v1 = decode_b64_symbol(b1).ok_or_else(|| {
            ProtocolError::Validation("client_upload : caractère base64 invalide".to_owned())
        })?;

        if b2 == b'=' && b3 == b'=' {
            if !is_last {
                return Err(ProtocolError::Validation(
                    "client_upload : padding '=' hors dernier bloc base64".to_owned(),
                ));
            }
            if (v1 & 0x0F) != 0 {
                return Err(ProtocolError::Validation(
                    "client_upload : bits de padding base64 non nuls".to_owned(),
                ));
            }
            out.push((v0 << 2) | (v1 >> 4));
        } else if b3 == b'=' {
            if !is_last {
                return Err(ProtocolError::Validation(
                    "client_upload : padding '=' hors dernier bloc base64".to_owned(),
                ));
            }
            let v2 = decode_b64_symbol(b2).ok_or_else(|| {
                ProtocolError::Validation("client_upload : caractère base64 invalide".to_owned())
            })?;
            if (v2 & 0x03) != 0 {
                return Err(ProtocolError::Validation(
                    "client_upload : bits de padding base64 non nuls".to_owned(),
                ));
            }
            out.push((v0 << 2) | (v1 >> 4));
            out.push((v1 << 4) | (v2 >> 2));
        } else {
            let v2 = decode_b64_symbol(b2).ok_or_else(|| {
                ProtocolError::Validation("client_upload : caractère base64 invalide".to_owned())
            })?;
            let v3 = decode_b64_symbol(b3).ok_or_else(|| {
                ProtocolError::Validation("client_upload : caractère base64 invalide".to_owned())
            })?;
            out.push((v0 << 2) | (v1 >> 4));
            out.push((v1 << 4) | (v2 >> 2));
            out.push((v2 << 6) | v3);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_and_strict_rejections() {
        for len in 0..=16 {
            let data: Vec<u8> = (0..len)
                .map(|i| u8::try_from(i * 17).unwrap_or(0))
                .collect();
            let encoded = encode_base64_standard(&data);
            let decoded = decode_base64_strict(&encoded).expect("décodage base64 valide");
            assert_eq!(decoded, data);
        }

        // Longueur non multiple de 4
        assert!(decode_base64_strict("AA").is_err());
        // Caractère hors alphabet
        assert!(decode_base64_strict("AA!A").is_err());
        // Bits de padding non nuls ("AB==" a v1=1 != 0 dans ses 4 bits bas)
        assert!(decode_base64_strict("AB==").is_err());
        // Padding hors dernier bloc
        assert!(decode_base64_strict("AA==AAAA").is_err());
    }
}
