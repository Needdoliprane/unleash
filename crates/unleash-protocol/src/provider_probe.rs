//! Forme IPC de `TestProviderProfile` (F012), sans autorisation ni admission implicite.

use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{DataLocality, MicrosAmount, Sha256Digest, TimestampMs};

use crate::{ProtocolError, canonical_sha256};

// IPC/JCS : au-delà, les clients JavaScript ne préservent plus chaque entier.
const MAX_EXACT_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Coordonnées internes du placement retenu par le writer pour une admission Ollama.
/// Ce type n'est ni un DTO IPC ni une autorisation de transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaChatPlacement {
    /// URL canonique vérifiée dans le profil et la preuve.
    pub endpoint: String,
    /// Boot dans lequel le writer a admis le processus.
    pub boot_identity: String,
    /// PID issu de la preuve de placement.
    pub process_id: u32,
    /// Identité OS de démarrage du processus.
    pub process_start_identity: String,
    /// Empreinte de l'objet de preuve de placement.
    pub evidence_digest: Sha256Digest,
    /// Empreinte du fait durable d'admission, bail et réservation compris.
    pub admission_digest: Sha256Digest,
    /// Première échéance du contexte, du placement et du bail.
    pub valid_until_ms: TimestampMs,
}

/// Maximum decoded response size for the Ollama v1 chat transport.
pub const OLLAMA_CHAT_MAX_RESPONSE_BYTES: usize = 1_048_576;

/// Exact native request fields shared by the writer and the transport.
pub struct OllamaChatRequestBinding<'a> {
    /// Canonical native URL.
    pub endpoint: &'a str,
    /// Logical provider profile identifier.
    pub profile_id: &'a str,
    /// Exact version of the selected profile.
    pub profile_version: u64,
    /// Digest of the immutable profile configuration.
    pub profile_digest: Sha256Digest,
    /// Connection timeout in milliseconds.
    pub connect_ms: u64,
    /// Exchange timeout in milliseconds.
    pub total_ms: u64,
    /// Digest of the exact native request body.
    pub body_digest: Sha256Digest,
}

impl OllamaChatRequestBinding<'_> {
    /// Digest of the native POST before binding it to the owned peer and admission.
    ///
    /// # Errors
    /// Fails if the binding cannot be canonicalized.
    pub fn digest(&self) -> Result<Sha256Digest, ProtocolError> {
        canonical_sha256(&json!({
            "schema":"unleash.ollama_chat.v1","method":"POST","endpoint":self.endpoint,
            "profile_id":self.profile_id,"profile_version":self.profile_version,"profile_digest":self.profile_digest,
            "connect_timeout_ms":self.connect_ms,"timeout_ms":self.total_ms,
            "body_digest":self.body_digest,"response_max_bytes":OLLAMA_CHAT_MAX_RESPONSE_BYTES,
            "accept":"application/x-ndjson","retries":0,
        }))
    }
}

/// Bind a native request to the numeric loopback socket and its process identity.
///
/// # Errors
/// Fails if the process identity or request binding cannot be canonicalized.
pub fn owned_loopback_request_digest<T: Serialize>(
    request: Sha256Digest,
    endpoint: SocketAddr,
    identity: &T,
) -> Result<Sha256Digest, ProtocolError> {
    let identity = serde_json::to_value(identity)?;
    canonical_sha256(&json!({
        "schema": "unleash.owned_loopback_request.v1",
        "transport_request_digest": request,
        "endpoint": endpoint,
        "process_identity": identity,
    }))
}

/// Bind the exact owned Ollama request to its durable admission and placement.
///
/// # Errors
/// Fails if the admission binding cannot be canonicalized.
pub fn admitted_ollama_chat_digest(
    owned_request: Sha256Digest,
    placement: &OllamaChatPlacement,
) -> Result<Sha256Digest, ProtocolError> {
    canonical_sha256(&json!({
        "schema": "unleash.admitted_ollama_chat.v1",
        "transport_request_digest": owned_request,
        "admission_digest": placement.admission_digest,
        "placement_digest": placement.evidence_digest,
        "valid_until_ms": placement.valid_until_ms,
    }))
}

/// Construit le corps natif de chat Ollama v1 pour un tour utilisateur.
///
/// La préparation persistée et le transport canonisent ce même document
/// avant calcul de leur empreinte. L'appelant résout et vérifie les entrées,
/// leur autorité et les bornes applicables.
#[must_use]
pub fn ollama_chat_body(
    model: &str,
    prompt: &str,
    output_tokens: u64,
    tool_schemas: &BTreeMap<String, Value>,
) -> Value {
    let tools: Vec<_> = tool_schemas
        .iter()
        .map(|(name, schema)| {
            json!({"type":"function","function":{"name":name,"parameters":schema}})
        })
        .collect();
    json!({
        "model":model,"messages":[{"role":"user","content":prompt}],
        "stream":true,"options":{"num_predict":output_tokens},"tools":tools
    })
}

/// Erreur de forme ou de borne statique, sans décision d'autorité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderTestPayloadError {
    /// Référence de profil ou de mandat invalide.
    #[error("invalid profile or mandate reference")]
    InvalidReference,
    /// Capacité invalide ou répétée.
    #[error("invalid or duplicate capability")]
    InvalidCapability,
    /// Délai nul ou limites hors domaine exact JSON ou profil de fixture.
    #[error("invalid test limits")]
    InvalidLimits,
}

/// Phase demandée ; les phases passives ne lancent aucune génération.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderTestPhase {
    /// Vérifier la configuration sans génération.
    ValidateConfig,
    /// Vérifier la connexion sans génération ni chargement implicite.
    Connect,
    /// Observer l’inventaire sans chargement implicite de modèle.
    Inventory,
    /// Exécuter le probe de génération autorisé.
    GenerationProbe,
}

/// Limites demandées par le client, susceptibles d'être abaissées par le serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTestLimits {
    timeout_ms: u64,
    max_calls: u64,
    max_output_tokens: u64,
    max_cost_usd_micros: MicrosAmount,
}

impl ProviderTestLimits {
    /// Prépare des limites demandées ; elles sont validées avec la phase du payload.
    #[must_use]
    pub const fn new(
        timeout_ms: u64,
        max_calls: u64,
        max_output_tokens: u64,
        max_cost_usd_micros: MicrosAmount,
    ) -> Self {
        Self {
            timeout_ms,
            max_calls,
            max_output_tokens,
            max_cost_usd_micros,
        }
    }

    /// Durée maximale demandée, en millisecondes.
    #[must_use]
    pub const fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
    /// Nombre maximal d'appels demandés.
    #[must_use]
    pub const fn max_calls(&self) -> u64 {
        self.max_calls
    }
    /// Nombre maximal de tokens de sortie cumulés.
    #[must_use]
    pub const fn max_output_tokens(&self) -> u64 {
        self.max_output_tokens
    }
    /// Coût maximal demandé en millionièmes de dollar américain.
    #[must_use]
    pub const fn max_cost_usd_micros(&self) -> MicrosAmount {
        self.max_cost_usd_micros
    }
}

/// Demande typée ; les références et limites ne constituent pas une autorisation.
///
/// La politique effective, l'origine, l'acteur, le mandat et la localité réelle
/// sont vérifiés par le serveur. Le décodage applique les contrôles statiques.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawProviderTest")]
pub struct TestProviderProfilePayload {
    profile_id: String,
    profile_version: u64,
    phase: ProviderTestPhase,
    capability_set: Vec<String>,
    limits: ProviderTestLimits,
    locality: DataLocality,
    mandate_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProviderTest {
    profile_id: String,
    profile_version: u64,
    phase: ProviderTestPhase,
    capability_set: Vec<String>,
    limits: ProviderTestLimits,
    locality: DataLocality,
    mandate_ref: String,
}

impl TryFrom<RawProviderTest> for TestProviderProfilePayload {
    type Error = ProviderTestPayloadError;

    fn try_from(raw: RawProviderTest) -> Result<Self, Self::Error> {
        fn valid_ref(value: &str) -> bool {
            !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        }
        if !valid_ref(&raw.profile_id)
            || !(1..=MAX_EXACT_JSON_INTEGER).contains(&raw.profile_version)
            || !valid_ref(&raw.mandate_ref)
        {
            return Err(ProviderTestPayloadError::InvalidReference);
        }
        let mut seen = HashSet::new();
        if raw.capability_set.len() > 64
            || raw
                .capability_set
                .iter()
                .any(|v| !valid_ref(v) || !seen.insert(v))
        {
            return Err(ProviderTestPayloadError::InvalidCapability);
        }
        if raw.limits.timeout_ms == 0
            || raw.limits.timeout_ms > MAX_EXACT_JSON_INTEGER
            || raw.limits.max_calls > MAX_EXACT_JSON_INTEGER
            || raw.limits.max_output_tokens > MAX_EXACT_JSON_INTEGER
            || raw.limits.max_cost_usd_micros.as_u64() > MAX_EXACT_JSON_INTEGER
        {
            return Err(ProviderTestPayloadError::InvalidLimits);
        }
        if raw.phase == ProviderTestPhase::GenerationProbe
            && (raw.limits.timeout_ms > 60_000
                || !(1..=3).contains(&raw.limits.max_calls)
                || !(1..=512).contains(&raw.limits.max_output_tokens)
                || raw.limits.max_cost_usd_micros.as_u64() > 100_000)
        {
            return Err(ProviderTestPayloadError::InvalidLimits);
        }
        Ok(Self {
            profile_id: raw.profile_id,
            profile_version: raw.profile_version,
            phase: raw.phase,
            capability_set: raw.capability_set,
            limits: raw.limits,
            locality: raw.locality,
            mandate_ref: raw.mandate_ref,
        })
    }
}

impl TestProviderProfilePayload {
    /// Construit une demande avec les mêmes vérifications que le décodage.
    ///
    /// # Errors
    /// Retourne une erreur si les références, capacités ou limites sont invalides.
    pub fn try_new(
        profile_id: String,
        profile_version: u64,
        phase: ProviderTestPhase,
        capability_set: Vec<String>,
        limits: ProviderTestLimits,
        locality: DataLocality,
        mandate_ref: String,
    ) -> Result<Self, ProviderTestPayloadError> {
        RawProviderTest {
            profile_id,
            profile_version,
            phase,
            capability_set,
            limits,
            locality,
            mandate_ref,
        }
        .try_into()
    }

    /// Identité exacte du profil demandé.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }
    /// Version exacte du profil demandé.
    #[must_use]
    pub const fn profile_version(&self) -> u64 {
        self.profile_version
    }
    /// Phase demandée.
    #[must_use]
    pub const fn phase(&self) -> ProviderTestPhase {
        self.phase
    }
    /// Capacités demandées sans doublons ; leur vocabulaire appartient au profil.
    #[must_use]
    pub fn capability_set(&self) -> &[String] {
        &self.capability_set
    }
    /// Limites demandées, avant intersection avec les politiques effectives.
    #[must_use]
    pub const fn limits(&self) -> &ProviderTestLimits {
        &self.limits
    }
    /// Localité demandée, non une preuve du placement réel.
    #[must_use]
    pub const fn locality(&self) -> DataLocality {
        self.locality
    }
    /// Référence opaque à vérifier auprès de l'autorité.
    #[must_use]
    pub fn mandate_ref(&self) -> &str {
        &self.mandate_ref
    }
}
