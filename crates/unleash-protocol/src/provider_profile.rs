//! Configuration wire F012. Toute autorité, qualification et résolution restent au serveur.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use unleash_domain::{DataLocality, SecretRef, Sha256Digest};
use url::Url;

const MAX_EXACT_INTEGER: u64 = 9_007_199_254_740_991;

/// Fournisseur déclaré ; aucun choix ne prouve la disponibilité du backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderKind {
    /// Fournisseur `OpenAI`.
    Openai,
    /// Fournisseur Anthropic.
    Anthropic,
    /// Serveur LM Studio.
    Lmstudio,
    /// Serveur Ollama.
    Ollama,
    /// Serveur llama.cpp.
    LlamaCpp,
    /// Serveur compatible distinct des fournisseurs natifs.
    OpenaiCompatible,
}

/// Dialecte déclaré, distinct du fournisseur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderDialectKind {
    /// `OpenAI` Responses.
    OpenaiResponses,
    /// Anthropic Messages.
    AnthropicMessages,
    /// Chat Completions compatible.
    OpenaiChatCompletions,
    /// API native de conversation Ollama.
    OllamaChat,
}

/// Nom et version exacts du dialecte déclaré.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDialect {
    /// Dialecte choisi.
    pub kind: ProviderDialectKind,
    /// Version positive dans le domaine exact JSON.
    pub version: u64,
}

/// Mode de gestion déclaré, non preuve de propriété d'un processus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderServerManagement {
    /// Serveur géré par l'utilisateur.
    UserManaged,
    /// Gestion explicitement demandée à Unleash.
    UnleashManaged,
}

/// Authentification demandée : uniquement une référence opaque, jamais une valeur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ProviderAuthentication {
    /// Aucun secret demandé ; à autoriser explicitement côté serveur.
    None,
    /// Référence à résoudre seulement après autorisation de l'effet.
    SecretRef {
        /// Référence opaque au coffre.
        secret_ref: SecretRef,
    },
}

/// Limites explicitement demandées ; aucune n'autorise seule un effet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileLimits {
    /// Concurrence maximale demandée.
    pub max_inflight: u64,
    /// Délai de connexion en millisecondes.
    pub connect_timeout_ms: u64,
    /// Délai de file en millisecondes.
    pub queue_timeout_ms: u64,
    /// Délai de chargement en millisecondes.
    pub load_timeout_ms: u64,
    /// Délai de premier token en millisecondes.
    pub first_token_timeout_ms: u64,
    /// Délai de génération en millisecondes.
    pub generation_timeout_ms: u64,
}

/// Traitement demandé des tarifs périmés, sous réserve de politique effective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderPricingStalePolicy {
    /// Bloquer si le tarif est périmé.
    Block,
    /// Demander une alerte ; le serveur décide si cette option est permise.
    Warn,
}

fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictDigestObject {
    algorithm: String,
    hex: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StrictDigest {
    String(String),
    Object(StrictDigestObject),
}

fn deserialize_catalog_digest<'de, D>(deserializer: D) -> Result<Sha256Digest, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    match StrictDigest::deserialize(deserializer)? {
        StrictDigest::String(value) => value.parse().map_err(D::Error::custom),
        StrictDigest::Object(value) if value.algorithm == "sha256" => {
            Sha256Digest::parse_hex(&value.hex).map_err(D::Error::custom)
        }
        StrictDigest::Object(_) => Err(D::Error::custom("unsupported digest algorithm")),
    }
}

/// Entrée tarifaire inline bornée, sans paliers ; les champs inconnus sont refusés.
/// Les valeurs nulles restent inconnues et ne valent pas zéro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlineProviderPrice {
    /// Code de devise déclaré sur trois lettres majuscules.
    pub currency: String,
    /// Prix d'entrée en micro-unités par million de tokens, ou inconnu.
    #[serde(deserialize_with = "present_option")]
    pub input_micros_per_million: Option<u64>,
    /// Prix d'entrée en cache, ou inconnu.
    #[serde(deserialize_with = "present_option")]
    pub cached_input_micros_per_million: Option<u64>,
    /// Prix d'écriture de cache, ou inconnu.
    #[serde(deserialize_with = "present_option")]
    pub cache_write_micros_per_million: Option<u64>,
    /// Prix de sortie, ou inconnu.
    #[serde(deserialize_with = "present_option")]
    pub output_micros_per_million: Option<u64>,
    /// Date de constat déclarée, en millisecondes UTC.
    pub observed_at_ms: u64,
    /// Date de validité déclarée, en millisecondes UTC.
    pub valid_until_ms: u64,
    /// URL de provenance déclarée ou absente ; elle n'est jamais contactée par le DTO.
    #[serde(deserialize_with = "present_option")]
    pub source_url: Option<String>,
}

/// Source tarifaire déclarée. La qualification et la version effective sont serveur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ProviderPricing {
    /// Aucun tarif fournisseur pour un calcul local à qualifier.
    NoneLocal,
    /// Entrée exacte d'un catalogue ; elle peut contenir des paliers complets.
    Catalog {
        /// Identité du catalogue.
        catalog_id: String,
        /// Version du catalogue.
        catalog_version: String,
        /// Empreinte du snapshot à vérifier par le service.
        #[serde(deserialize_with = "deserialize_catalog_digest")]
        catalog_digest: Sha256Digest,
        /// Empreinte de l'entrée exacte à vérifier par le service.
        #[serde(deserialize_with = "deserialize_catalog_digest")]
        entry_digest: Sha256Digest,
        /// Traitement demandé si périmé.
        stale_policy: ProviderPricingStalePolicy,
    },
    /// Entrée importée déclarée, à valider et versionner par le service.
    UserImport {
        /// Sous-ensemble tarifaire inline sans paliers.
        entry: InlineProviderPrice,
        /// Traitement demandé si périmé.
        stale_policy: ProviderPricingStalePolicy,
    },
    /// Entrée saisie, à valider et versionner par le service.
    UserDeclared {
        /// Sous-ensemble tarifaire inline sans paliers.
        entry: InlineProviderPrice,
        /// Traitement demandé si périmé.
        stale_policy: ProviderPricingStalePolicy,
    },
}

/// Erreur de forme statique ; aucune évaluation des permissions n'est effectuée ici.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderProfilePayloadError {
    /// Fournisseur et dialecte ne correspondent pas.
    #[error("provider and dialect are incompatible")]
    InvalidDialect,
    /// Une référence ou une URL est mal formée.
    #[error("invalid profile reference or endpoint")]
    InvalidReference,
    /// Une limite ou un prix dépasse le domaine wire.
    #[error("invalid profile limits or price")]
    InvalidLimits,
    /// L'authentification déclarée est incohérente.
    #[error("invalid authentication")]
    InvalidAuthentication,
}

/// Payload de `SetProviderProfile` ; identité et version viennent de l'enveloppe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawProviderProfile")]
pub struct SetProviderProfilePayload {
    provider_kind: ProviderKind,
    dialect: ProviderDialect,
    endpoint: String,
    model: String,
    locality: DataLocality,
    server_management: ProviderServerManagement,
    auth: ProviderAuthentication,
    limits: ProviderProfileLimits,
    declared_capabilities: Vec<String>,
    pricing: ProviderPricing,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProviderProfile {
    provider_kind: ProviderKind,
    dialect: ProviderDialect,
    endpoint: String,
    model: String,
    locality: DataLocality,
    server_management: ProviderServerManagement,
    auth: ProviderAuthentication,
    limits: ProviderProfileLimits,
    declared_capabilities: Vec<String>,
    pricing: ProviderPricing,
}

fn bounded_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

// Vérification uniquement structurelle. Placement, TLS, DNS et autorisation sont serveur.
fn endpoint_shape(s: &str) -> bool {
    if !s.is_ascii()
        || s.len() > 2048
        || s.bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
        || s.contains(['?', '#'])
    {
        return false;
    }
    let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return false;
    };
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let mut path_bytes = path.bytes();
    while let Some(byte) = path_bytes.next() {
        if byte == b'%'
            && !(path_bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && path_bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return false;
        }
    }
    Url::parse(s).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port() != Some(0)
            && (url.scheme() == "https"
                || match url.host() {
                    Some(url::Host::Domain(name)) => name == "localhost",
                    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    None => false,
                })
    })
}

fn validate_pricing(
    pricing: &ProviderPricing,
    provider: ProviderKind,
) -> Result<(), ProviderProfilePayloadError> {
    match pricing {
        ProviderPricing::NoneLocal
            if matches!(provider, ProviderKind::Openai | ProviderKind::Anthropic) =>
        {
            Err(ProviderProfilePayloadError::InvalidLimits)
        }
        ProviderPricing::Catalog {
            catalog_id,
            catalog_version,
            ..
        } if !bounded_text(catalog_id, 256) || !bounded_text(catalog_version, 256) => {
            Err(ProviderProfilePayloadError::InvalidReference)
        }
        ProviderPricing::NoneLocal | ProviderPricing::Catalog { .. } => Ok(()),
        ProviderPricing::UserImport { entry, .. } | ProviderPricing::UserDeclared { entry, .. } => {
            if entry.currency.len() != 3
                || !entry.currency.bytes().all(|b| b.is_ascii_uppercase())
                || entry.observed_at_ms > entry.valid_until_ms
                || entry.valid_until_ms > MAX_EXACT_INTEGER
                || [
                    entry.input_micros_per_million,
                    entry.cached_input_micros_per_million,
                    entry.cache_write_micros_per_million,
                    entry.output_micros_per_million,
                ]
                .into_iter()
                .flatten()
                .any(|n| n > MAX_EXACT_INTEGER)
                || entry
                    .source_url
                    .as_ref()
                    .is_some_and(|s| !endpoint_shape(s))
            {
                Err(ProviderProfilePayloadError::InvalidLimits)
            } else {
                Ok(())
            }
        }
    }
}

impl TryFrom<RawProviderProfile> for SetProviderProfilePayload {
    type Error = ProviderProfilePayloadError;

    fn try_from(raw: RawProviderProfile) -> Result<Self, Self::Error> {
        let compatible = matches!(
            (raw.provider_kind, raw.dialect.kind),
            (ProviderKind::Openai, ProviderDialectKind::OpenaiResponses)
                | (
                    ProviderKind::Anthropic,
                    ProviderDialectKind::AnthropicMessages
                )
                | (
                    ProviderKind::Lmstudio | ProviderKind::LlamaCpp,
                    ProviderDialectKind::OpenaiChatCompletions
                )
                | (ProviderKind::Ollama, ProviderDialectKind::OllamaChat)
                | (
                    ProviderKind::OpenaiCompatible,
                    ProviderDialectKind::OpenaiResponses
                        | ProviderDialectKind::OpenaiChatCompletions
                )
        );
        if !compatible || !(1..=MAX_EXACT_INTEGER).contains(&raw.dialect.version) {
            return Err(ProviderProfilePayloadError::InvalidDialect);
        }
        if !endpoint_shape(&raw.endpoint) || !bounded_text(&raw.model, 256) {
            return Err(ProviderProfilePayloadError::InvalidReference);
        }
        match &raw.auth {
            ProviderAuthentication::SecretRef { secret_ref }
                if !bounded_text(secret_ref.as_opaque_str(), 256) =>
            {
                return Err(ProviderProfilePayloadError::InvalidAuthentication);
            }
            ProviderAuthentication::None
                if matches!(
                    raw.provider_kind,
                    ProviderKind::Openai | ProviderKind::Anthropic
                ) =>
            {
                return Err(ProviderProfilePayloadError::InvalidAuthentication);
            }
            _ => {}
        }
        let limits = &raw.limits;
        if [
            limits.max_inflight,
            limits.connect_timeout_ms,
            limits.queue_timeout_ms,
            limits.load_timeout_ms,
            limits.first_token_timeout_ms,
            limits.generation_timeout_ms,
        ]
        .iter()
        .any(|v| !(1..=MAX_EXACT_INTEGER).contains(v))
        {
            return Err(ProviderProfilePayloadError::InvalidLimits);
        }
        let mut seen = HashSet::new();
        if raw.declared_capabilities.len() > 64
            || raw
                .declared_capabilities
                .iter()
                .any(|c| !bounded_text(c, 256) || !seen.insert(c))
        {
            return Err(ProviderProfilePayloadError::InvalidReference);
        }
        validate_pricing(&raw.pricing, raw.provider_kind)?;
        Ok(Self {
            provider_kind: raw.provider_kind,
            dialect: raw.dialect,
            endpoint: raw.endpoint,
            model: raw.model,
            locality: raw.locality,
            server_management: raw.server_management,
            auth: raw.auth,
            limits: raw.limits,
            declared_capabilities: raw.declared_capabilities,
            pricing: raw.pricing,
        })
    }
}

impl SetProviderProfilePayload {
    /// Produit une copie dont seul l'endpoint est normalisé suivant `Url`.
    /// Le décodage et le digest de l'enveloppe d'origine restent inchangés.
    /// Aucune résolution DNS, connexion ou preuve de placement n'est effectuée.
    /// # Errors
    /// Refuse une URL invalide ; les interdits structurels restent ceux du DTO.
    pub fn with_normalized_endpoint(&self) -> Result<Self, ProviderProfilePayloadError> {
        let endpoint = Url::parse(&self.endpoint)
            .map_err(|_| ProviderProfilePayloadError::InvalidReference)?;
        let mut normalized = self.clone();
        normalized.endpoint = endpoint.to_string();
        Ok(normalized)
    }

    /// Fournisseur déclaré.
    #[must_use]
    pub const fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }
    /// Dialecte déclaré.
    #[must_use]
    pub const fn dialect(&self) -> &ProviderDialect {
        &self.dialect
    }
    /// Endpoint déclaré, non preuve de destination ou de placement.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    /// Identifiant de modèle déclaré.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Localité demandée.
    #[must_use]
    pub const fn locality(&self) -> DataLocality {
        self.locality
    }
    /// Mode de gestion déclaré.
    #[must_use]
    pub const fn server_management(&self) -> ProviderServerManagement {
        self.server_management
    }
    /// Authentification déclarée.
    #[must_use]
    pub const fn auth(&self) -> &ProviderAuthentication {
        &self.auth
    }
    /// Limites demandées.
    #[must_use]
    pub const fn limits(&self) -> &ProviderProfileLimits {
        &self.limits
    }
    /// Capacités déclarées ; aucune n'est observée ici.
    #[must_use]
    pub fn declared_capabilities(&self) -> &[String] {
        &self.declared_capabilities
    }
    /// Source de tarif déclarée.
    #[must_use]
    pub const fn pricing(&self) -> &ProviderPricing {
        &self.pricing
    }
}
