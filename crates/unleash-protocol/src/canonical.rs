//! # Canonisation JSON RFC 8785 (JCS) et digests cryptographiques
//!
//! Responsabilité exclusive : Normalisation canonique d'objets JSON selon la norme RFC 8785
//! (tri lexicographique des clés en unités de code UTF-16, échappements minimaux, formatage strict des nombres ES6)
//! et calcul déterministe de digests SHA-256 (`sha256:<hex64>`).
//!
//! Contrat d'idempotence v2 :
//! - [`compute_command_digest_v2`] inclut : `authority_generation`, `protocol_version`,
//!   `command_type`, `project_id`, `aggregate_id`, `expected_revision` et `payload`.
//! - Exclut rigoureusement : `request_id`, `actor_ref` et les détails de transport.

use crate::envelopes::CommandEnvelope;
use crate::errors::ProtocolError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use unleash_domain::{AuthorityGeneration, ProjectId, Revision, Sha256Digest};

/// Sérialise une structure ou une valeur en octets JSON canoniques conformes à la RFC 8785 (JCS).
///
/// # Errors
/// Renvoie [`ProtocolError::Jcs`] si la sérialisation JCS échoue.
pub fn canonicalize_to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    serde_jcs::to_vec(value).map_err(|e| ProtocolError::Jcs(e.to_string()))
}

/// Sérialise une structure en chaîne UTF-8 JSON canonique RFC 8785 (JCS).
///
/// # Errors
/// Renvoie [`ProtocolError::Jcs`] si la sérialisation JCS échoue.
pub fn canonicalize_to_string<T: Serialize>(value: &T) -> Result<String, ProtocolError> {
    serde_jcs::to_string(value).map_err(|e| ProtocolError::Jcs(e.to_string()))
}

/// Calcule l'empreinte canonique SHA-256 [`Sha256Digest`] d'une structure sérialisable selon la RFC 8785.
///
/// # Errors
/// Renvoie [`ProtocolError::Jcs`] si la canonisation JCS échoue.
pub fn canonical_sha256<T: Serialize>(value: &T) -> Result<Sha256Digest, ProtocolError> {
    let canonical_bytes = canonicalize_to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical_bytes);
    let hash: [u8; 32] = hasher.finalize().into();
    Ok(Sha256Digest::from_raw_bytes(hash))
}

/// Structure intermédiaire projetée pour le calcul de l'empreinte d'idempotence v2.
/// Contient uniquement les 7 champs normatifs exigés par le contrat d'idempotence,
/// et exclut `request_id`, `actor_ref` ainsi que tout champ de transport.
#[derive(Serialize)]
struct CommandDigestV2Payload<'a> {
    aggregate_id: uuid::Uuid,
    authority_generation: &'a AuthorityGeneration,
    command_type: &'a str,
    expected_revision: Revision,
    payload: &'a serde_json::Value,
    project_id: Option<ProjectId>,
    protocol_version: &'a str,
}

/// Calcule le digest canonique SHA-256 d'idempotence d'une commande selon le protocole v2.
///
/// Conforme à l'exigence `REQ-CORE-003` / `INV-041` :
/// - Inclut obligatoirement : `authority_generation`, `protocol_version`, `command_type`,
///   `project_id`, `aggregate_id`, `expected_revision` et `payload` canonisé.
/// - Exclut strictement : `request_id` (pouvant changer à chaque retransmission)
///   et `actor_ref` (identité authentifiée côté serveur).
///
/// # Errors
/// Renvoie [`ProtocolError::Jcs`] en cas d'erreur de canonisation.
pub fn compute_command_digest_v2(
    envelope: &CommandEnvelope,
) -> Result<Sha256Digest, ProtocolError> {
    let projected = CommandDigestV2Payload {
        aggregate_id: envelope.aggregate_id,
        authority_generation: &envelope.authority_generation,
        command_type: &envelope.command_type,
        expected_revision: envelope.expected_revision,
        payload: &envelope.payload,
        project_id: envelope.project_id,
        protocol_version: envelope.protocol_version.as_str(),
    };

    canonical_sha256(&projected)
}
