//! Forme IPC de `TestProviderProfile` (F012), sans autorisation ni admission implicite.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use unleash_domain::{DataLocality, MicrosAmount};

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
    /// Délai nul ou limites de génération hors profil de fixture.
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
        if !valid_ref(&raw.profile_id) || raw.profile_version == 0 || !valid_ref(&raw.mandate_ref) {
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
        if raw.limits.timeout_ms == 0 {
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
