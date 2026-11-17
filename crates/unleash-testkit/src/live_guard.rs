//! Garde de sécurité pour les tests live payants pendant le développement.
//!
//! Applique strictement la politique `.claude/rules/testing.md` :
//! - ne lit `.env` via `dotenvy` **que** lorsque `UNLEASH_LIVE_TESTS=1` est actif dans l'environnement ;
//! - impose l'allowlist fermée `["claude-haiku-5-5", "gpt-6-luna"]` (refuse tout modèle intelligent
//!   tel que `gpt-6.1-sol`, `claude-opus-5-5`, `claude-fable-5-2`, `claude-sonnet-5-5`, `gpt-6-astra`) ;
//! - contrôle strictement le plafond de dépense `UNLEASH_LIVE_TEST_BUDGET_USD` en micro-dollars ;
//! - encapsule les clés `UNLEASH_DEV_ANTHROPIC_API_KEY` et `UNLEASH_DEV_OPENAI_API_KEY` dans
//!   [`secrecy::SecretString`] et efface les tampons temporaires via [`zeroize::Zeroize`] (`Debug` masqué).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use secrecy::SecretString;
use thiserror::Error;
use zeroize::Zeroize;

/// Allowlist fermée des seuls modèles autorisés pour les tests live payants en développement.
pub const LIVE_MODEL_ALLOWLIST: [&str; 2] = ["claude-haiku-5-5", "gpt-6-luna"];

const ENV_LIVE_TESTS: &str = "UNLEASH_LIVE_TESTS";
const ENV_ANTHROPIC_API_KEY: &str = "UNLEASH_DEV_ANTHROPIC_API_KEY";
const ENV_OPENAI_API_KEY: &str = "UNLEASH_DEV_OPENAI_API_KEY";
const ENV_MODEL_ALLOWLIST: &str = "UNLEASH_LIVE_TEST_MODEL_ALLOWLIST";
const ENV_ANTHROPIC_MODEL: &str = "UNLEASH_LIVE_TEST_ANTHROPIC_MODEL";
const ENV_OPENAI_MODEL: &str = "UNLEASH_LIVE_TEST_OPENAI_MODEL";
const ENV_BUDGET_USD: &str = "UNLEASH_LIVE_TEST_BUDGET_USD";

const MICROS_PER_USD: u64 = 1_000_000;

/// Fournisseur associé à un modèle de test live autorisé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveProvider {
    /// Fournisseur Anthropic (`UNLEASH_DEV_ANTHROPIC_API_KEY`).
    Anthropic,
    /// Fournisseur `OpenAI` (`UNLEASH_DEV_OPENAI_API_KEY`).
    OpenAi,
}

impl fmt::Display for LiveProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anthropic => f.write_str("anthropic"),
            Self::OpenAi => f.write_str("openai"),
        }
    }
}

/// Énumération fermée des modèles autorisés pour les tests live payants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveModel {
    /// Modèle léger Anthropic (`claude-haiku-5-5`).
    ClaudeHaiku55,
    /// Modèle léger `OpenAI` (`gpt-6-luna`).
    Gpt6Luna,
}

impl LiveModel {
    /// Renvoie l'identifiant canonique exact du modèle.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeHaiku55 => "claude-haiku-5-5",
            Self::Gpt6Luna => "gpt-6-luna",
        }
    }

    /// Renvoie le fournisseur associé à ce modèle.
    #[must_use]
    pub const fn provider(self) -> LiveProvider {
        match self {
            Self::ClaudeHaiku55 => LiveProvider::Anthropic,
            Self::Gpt6Luna => LiveProvider::OpenAi,
        }
    }

    /// Résout un nom de modèle contre l'allowlist fermée [`LIVE_MODEL_ALLOWLIST`].
    ///
    /// # Errors
    /// - [`LiveGuardError::ModelNotAllowed`] pour tout modèle absent de l'allowlist fermée.
    pub fn parse_allowed(name: &str) -> Result<Self, LiveGuardError> {
        match name {
            "claude-haiku-5-5" => Ok(Self::ClaudeHaiku55),
            "gpt-6-luna" => Ok(Self::Gpt6Luna),
            other => Err(LiveGuardError::ModelNotAllowed {
                model: other.to_owned(),
            }),
        }
    }
}

impl fmt::Display for LiveModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur émise par [`LiveTestGuard`] sans jamais exposer de valeur de secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveGuardError {
    /// Les tests live sont désactivés (`UNLEASH_LIVE_TESTS` absent ou différent de `1`).
    #[error(
        "Tests live désactivés : définir UNLEASH_LIVE_TESTS=1 pour autoriser le chargement de .env"
    )]
    LiveTestsDisabled,

    /// Erreur lors de la lecture ou de l'analyse du fichier `.env` par `dotenvy`.
    #[error("Erreur de lecture/analyse du fichier .env : {message}")]
    Dotenv {
        /// Description de l'erreur (sans contenu de secret).
        message: String,
    },

    /// Modèle refusé car absent de l'allowlist fermée `["claude-haiku-5-5", "gpt-6-luna"]`.
    #[error(
        "Modèle '{model}' refusé par l'allowlist fermée des tests live (autorisés : claude-haiku-5-5, gpt-6-luna)"
    )]
    ModelNotAllowed {
        /// Nom du modèle refusé.
        model: String,
    },

    /// Modèle configuré incohérent avec le fournisseur attendu.
    #[error("Modèle '{model}' incompatible avec le fournisseur '{expected_provider}'")]
    ProviderModelMismatch {
        /// Modèle configuré.
        model: String,
        /// Fournisseur attendu.
        expected_provider: LiveProvider,
    },

    /// Plafond `UNLEASH_LIVE_TEST_BUDGET_USD` manquant.
    #[error("Plafond budgétaire {ENV_BUDGET_USD} absent de la configuration live")]
    MissingBudget,

    /// Plafond `UNLEASH_LIVE_TEST_BUDGET_USD` invalide.
    #[error("Plafond budgétaire '{raw}' invalide dans {ENV_BUDGET_USD}")]
    InvalidBudget {
        /// Chaîne brute fournie.
        raw: String,
    },

    /// Coût estimé d'un appel invalide (négatif, NaN, infini ou précision illégale).
    #[error("Coût d'appel live invalide : {reason}")]
    InvalidCost {
        /// Motif du rejet.
        reason: String,
    },

    /// Dépassement du plafond budgétaire `UNLEASH_LIVE_TEST_BUDGET_USD`.
    #[error(
        "Budget tests live dépassé : demandé {requested_micros} µUSD, restant {remaining_micros} µUSD (plafond {limit_micros} µUSD)"
    )]
    BudgetExceeded {
        /// Montant demandé pour l'appel en micro-dollars.
        requested_micros: u64,
        /// Solde restant avant refus en micro-dollars.
        remaining_micros: u64,
        /// Plafond configuré en micro-dollars.
        limit_micros: u64,
    },

    /// Clé API développeur absente ou vide pour le fournisseur requis.
    #[error("Clé API développeur absente ou vide pour le fournisseur '{provider}'")]
    MissingApiKey {
        /// Fournisseur dont la clé est requise.
        provider: LiveProvider,
    },
}

/// Autorisation accordée pour exécuter un appel live sur un modèle autorisé et un budget réservé.
pub struct LiveCallPermit<'a> {
    model: LiveModel,
    reserved_micros: u64,
    api_key: &'a SecretString,
}

impl fmt::Debug for LiveCallPermit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveCallPermit")
            .field("model", &self.model)
            .field("reserved_micros", &self.reserved_micros)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl<'a> LiveCallPermit<'a> {
    /// Modèle autorisé pour cet appel.
    #[must_use]
    pub const fn model(&self) -> LiveModel {
        self.model
    }

    /// Montant réservé sur le budget de la campagne en micro-dollars.
    #[must_use]
    pub const fn reserved_micros(&self) -> u64 {
        self.reserved_micros
    }

    /// Référence protégée vers la clé API correspondante (`SecretString`).
    #[must_use]
    pub const fn api_key(&self) -> &'a SecretString {
        self.api_key
    }
}

/// Garde stricte régissant les tests live payants en développement.
///
/// L'implémentation de [`fmt::Debug`] masque systématiquement les clés API (`[REDACTED]`).
pub struct LiveTestGuard {
    allowed_models: Vec<LiveModel>,
    budget_limit_micros: u64,
    spent_micros: Mutex<u64>,
    anthropic_api_key: Option<SecretString>,
    openai_api_key: Option<SecretString>,
}

impl fmt::Debug for LiveTestGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let anthropic_dbg = if self.anthropic_api_key.is_some() {
            Some("[REDACTED]")
        } else {
            None
        };
        let openai_dbg = if self.openai_api_key.is_some() {
            Some("[REDACTED]")
        } else {
            None
        };
        f.debug_struct("LiveTestGuard")
            .field("allowed_models", &self.allowed_models)
            .field("budget_limit_micros", &self.budget_limit_micros)
            .field("spent_micros", &self.spent_micros())
            .field("anthropic_api_key", &anthropic_dbg)
            .field("openai_api_key", &openai_dbg)
            .finish()
    }
}

impl LiveTestGuard {
    /// Charge la garde depuis l'environnement du processus et le fichier `.env` découvert
    /// par `dotenvy` **uniquement** si `UNLEASH_LIVE_TESTS=1` est actif dans l'environnement.
    ///
    /// Si `.env` est absent dans le répertoire courant ou parent, poursuit uniquement avec les
    /// variables déjà présentes dans l'environnement.
    ///
    /// # Errors
    /// - [`LiveGuardError::LiveTestsDisabled`] si `UNLEASH_LIVE_TESTS != "1"`.
    /// - [`LiveGuardError::Dotenv`] si un fichier `.env` trouvé échoue au parsing.
    /// - [`LiveGuardError::ModelNotAllowed`], [`LiveGuardError::MissingBudget`], [`LiveGuardError::InvalidBudget`].
    pub fn from_env() -> Result<Self, LiveGuardError> {
        Self::from_env_lookup_and_dotenv(|k| std::env::var(k).ok(), None, false)
    }

    /// Charge la garde depuis l'environnement du processus et le fichier `.env` explicite `dotenv_path`
    /// **uniquement** si `UNLEASH_LIVE_TESTS=1` est actif dans l'environnement.
    ///
    /// # Errors
    /// - [`LiveGuardError::LiveTestsDisabled`] si `UNLEASH_LIVE_TESTS != "1"` (sans lire `dotenv_path`).
    /// - [`LiveGuardError::Dotenv`] si `dotenv_path` est illisible ou invalide.
    /// - [`LiveGuardError::ModelNotAllowed`], [`LiveGuardError::MissingBudget`], [`LiveGuardError::InvalidBudget`].
    pub fn from_dotenv_path(dotenv_path: &Path) -> Result<Self, LiveGuardError> {
        Self::from_env_lookup_and_dotenv(|k| std::env::var(k).ok(), Some(dotenv_path), true)
    }

    /// Constructeur pur avec fonction de lecture d'environnement injectable et chemin `.env` optionnel.
    ///
    /// Permet des tests déterministes et parallèles sans mutation globale de `std::env`
    /// (interdite par `#![forbid(unsafe_code)]` en édition Rust 2024).
    ///
    /// # Errors
    /// - [`LiveGuardError::LiveTestsDisabled`] dès que `env_lookup("UNLEASH_LIVE_TESTS")` diffère de `"1"`,
    ///   avant toute ouverture ou analyse de `dotenv_path`.
    /// - [`LiveGuardError::Dotenv`] si `dotenvy` rencontre une erreur I/O ou syntaxique.
    /// - [`LiveGuardError::ModelNotAllowed`], [`LiveGuardError::ProviderModelMismatch`],
    ///   [`LiveGuardError::MissingBudget`], [`LiveGuardError::InvalidBudget`].
    pub fn from_env_and_dotenv_with<F>(
        env_lookup: F,
        dotenv_path: Option<&Path>,
    ) -> Result<Self, LiveGuardError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_env_lookup_and_dotenv(env_lookup, dotenv_path, dotenv_path.is_some())
    }

    fn from_env_lookup_and_dotenv<F>(
        env_lookup: F,
        dotenv_path: Option<&Path>,
        require_dotenv_file: bool,
    ) -> Result<Self, LiveGuardError>
    where
        F: Fn(&str) -> Option<String>,
    {
        // Garde impérative : ne jamais ouvrir ni parser .env si UNLEASH_LIVE_TESTS != "1".
        let live_flag = env_lookup(ENV_LIVE_TESTS);
        if live_flag.as_deref().map(str::trim) != Some("1") {
            return Err(LiveGuardError::LiveTestsDisabled);
        }

        let mut dotenv_non_secret: BTreeMap<String, String> = BTreeMap::new();
        let mut dotenv_anthropic: Option<SecretString> = None;
        let mut dotenv_openai: Option<SecretString> = None;

        if let Some(path) = dotenv_path {
            if path.exists() || require_dotenv_file {
                load_dotenv_path(
                    path,
                    &mut dotenv_non_secret,
                    &mut dotenv_anthropic,
                    &mut dotenv_openai,
                )?;
            }
        } else {
            load_dotenv_default(
                &mut dotenv_non_secret,
                &mut dotenv_anthropic,
                &mut dotenv_openai,
            )?;
        }

        let resolve_non_secret = |key: &str| -> Option<String> {
            env_lookup(key)
                .filter(|v| !v.trim().is_empty())
                .or_else(|| dotenv_non_secret.get(key).cloned())
        };

        let allowed_models = resolve_allowed_models(resolve_non_secret(ENV_MODEL_ALLOWLIST))?;

        if let Some(anthropic_model) = resolve_non_secret(ENV_ANTHROPIC_MODEL) {
            let parsed = validate_model_against(&anthropic_model, &allowed_models)?;
            if parsed.provider() != LiveProvider::Anthropic {
                return Err(LiveGuardError::ProviderModelMismatch {
                    model: anthropic_model,
                    expected_provider: LiveProvider::Anthropic,
                });
            }
        }

        if let Some(openai_model) = resolve_non_secret(ENV_OPENAI_MODEL) {
            let parsed = validate_model_against(&openai_model, &allowed_models)?;
            if parsed.provider() != LiveProvider::OpenAi {
                return Err(LiveGuardError::ProviderModelMismatch {
                    model: openai_model,
                    expected_provider: LiveProvider::OpenAi,
                });
            }
        }

        let raw_budget = resolve_non_secret(ENV_BUDGET_USD).ok_or(LiveGuardError::MissingBudget)?;
        let budget_limit_micros = parse_usd_to_micros(&raw_budget)?;

        let anthropic_api_key =
            resolve_secret_key(env_lookup(ENV_ANTHROPIC_API_KEY), dotenv_anthropic);
        let openai_api_key = resolve_secret_key(env_lookup(ENV_OPENAI_API_KEY), dotenv_openai);

        Ok(Self {
            allowed_models,
            budget_limit_micros,
            spent_micros: Mutex::new(0),
            anthropic_api_key,
            openai_api_key,
        })
    }

    /// Renvoie la liste des modèles autorisés pour cette instance (toujours incluse dans [`LIVE_MODEL_ALLOWLIST`]).
    #[must_use]
    pub fn allowed_models(&self) -> &[LiveModel] {
        &self.allowed_models
    }

    /// Plafond budgétaire configuré en micro-dollars (`1 USD = 1_000_000 µUSD`).
    #[must_use]
    pub const fn budget_limit_micros(&self) -> u64 {
        self.budget_limit_micros
    }

    /// Montant déjà consommé en micro-dollars.
    #[must_use]
    pub fn spent_micros(&self) -> u64 {
        *self
            .spent_micros
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Solde budgétaire restant en micro-dollars.
    #[must_use]
    pub fn remaining_micros(&self) -> u64 {
        self.budget_limit_micros.saturating_sub(self.spent_micros())
    }

    /// Indique si la clé Anthropic de développement est chargée.
    #[must_use]
    pub const fn has_anthropic_key(&self) -> bool {
        self.anthropic_api_key.is_some()
    }

    /// Indique si la clé `OpenAI` de développement est chargée.
    #[must_use]
    pub const fn has_openai_key(&self) -> bool {
        self.openai_api_key.is_some()
    }

    /// Vérifie qu'un nom de modèle appartient à l'allowlist fermée active.
    ///
    /// # Errors
    /// - [`LiveGuardError::ModelNotAllowed`] pour tout modèle hors allowlist fermée.
    pub fn authorize_model(&self, model: &str) -> Result<LiveModel, LiveGuardError> {
        validate_model_against(model, &self.allowed_models)
    }

    /// Renvoie la clé API protégée par [`SecretString`] pour `model`.
    ///
    /// # Errors
    /// - [`LiveGuardError::MissingApiKey`] si la clé du fournisseur correspondant est absente ou vide.
    pub fn key_for_model(&self, model: LiveModel) -> Result<&SecretString, LiveGuardError> {
        match model.provider() {
            LiveProvider::Anthropic => {
                self.anthropic_api_key
                    .as_ref()
                    .ok_or(LiveGuardError::MissingApiKey {
                        provider: LiveProvider::Anthropic,
                    })
            }
            LiveProvider::OpenAi => {
                self.openai_api_key
                    .as_ref()
                    .ok_or(LiveGuardError::MissingApiKey {
                        provider: LiveProvider::OpenAi,
                    })
            }
        }
    }

    /// Vérifie le modèle contre l'allowlist fermée, la présence de sa clé API, puis réserve
    /// atomiquement `estimated_cost_micros` sur le budget de la campagne.
    ///
    /// # Errors
    /// - [`LiveGuardError::ModelNotAllowed`] si le modèle est refusé.
    /// - [`LiveGuardError::MissingApiKey`] si la clé API requise est absente.
    /// - [`LiveGuardError::BudgetExceeded`] si la dépense ferait dépasser `UNLEASH_LIVE_TEST_BUDGET_USD`.
    pub fn authorize_call_micros(
        &self,
        model: &str,
        estimated_cost_micros: u64,
    ) -> Result<LiveCallPermit<'_>, LiveGuardError> {
        let live_model = self.authorize_model(model)?;
        let api_key = self.key_for_model(live_model)?;

        let mut spent = self
            .spent_micros
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remaining = self.budget_limit_micros.saturating_sub(*spent);
        if estimated_cost_micros > remaining {
            return Err(LiveGuardError::BudgetExceeded {
                requested_micros: estimated_cost_micros,
                remaining_micros: remaining,
                limit_micros: self.budget_limit_micros,
            });
        }
        *spent = spent.saturating_add(estimated_cost_micros);

        Ok(LiveCallPermit {
            model: live_model,
            reserved_micros: estimated_cost_micros,
            api_key,
        })
    }

    /// Variante acceptant un coût estimé en USD flottant (validé fini et non négatif).
    ///
    /// # Errors
    /// - [`LiveGuardError::InvalidCost`] si `estimated_cost_usd` est NaN, infini, négatif ou hors borne.
    /// - Propage les erreurs de [`Self::authorize_call_micros`].
    pub fn authorize_call_usd(
        &self,
        model: &str,
        estimated_cost_usd: f64,
    ) -> Result<LiveCallPermit<'_>, LiveGuardError> {
        if !estimated_cost_usd.is_finite() || estimated_cost_usd < 0.0 {
            return Err(LiveGuardError::InvalidCost {
                reason: format!("montant USD invalide ({estimated_cost_usd})"),
            });
        }
        let max_usd_f64 = 10_000_000_000.0_f64;
        if estimated_cost_usd > max_usd_f64 {
            return Err(LiveGuardError::InvalidCost {
                reason: String::from("montant USD hors plage"),
            });
        }
        let formatted = format!("{estimated_cost_usd:.6}");
        let micros = parse_usd_to_micros(&formatted).map_err(|_| LiveGuardError::InvalidCost {
            reason: format!("impossible de convertir '{formatted}' en micro-USD"),
        })?;
        self.authorize_call_micros(model, micros)
    }
}

fn load_dotenv_path(
    path: &Path,
    non_secret: &mut BTreeMap<String, String>,
    anthropic_key: &mut Option<SecretString>,
    openai_key: &mut Option<SecretString>,
) -> Result<(), LiveGuardError> {
    let iter = dotenvy::from_path_iter(path).map_err(|err| LiveGuardError::Dotenv {
        message: err.to_string(),
    })?;
    consume_dotenv_iter(iter, non_secret, anthropic_key, openai_key)
}

fn load_dotenv_default(
    non_secret: &mut BTreeMap<String, String>,
    anthropic_key: &mut Option<SecretString>,
    openai_key: &mut Option<SecretString>,
) -> Result<(), LiveGuardError> {
    match dotenvy::from_filename_iter(".env") {
        Ok(iter) => consume_dotenv_iter(iter, non_secret, anthropic_key, openai_key),
        Err(dotenvy::Error::Io(io_err)) if io_err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(LiveGuardError::Dotenv {
            message: err.to_string(),
        }),
    }
}

fn consume_dotenv_iter<I>(
    iter: I,
    non_secret: &mut BTreeMap<String, String>,
    anthropic_key: &mut Option<SecretString>,
    openai_key: &mut Option<SecretString>,
) -> Result<(), LiveGuardError>
where
    I: Iterator<Item = Result<(String, String), dotenvy::Error>>,
{
    for item in iter {
        let (key, mut value) = item.map_err(|err| LiveGuardError::Dotenv {
            message: err.to_string(),
        })?;
        match key.as_str() {
            ENV_ANTHROPIC_API_KEY => {
                *anthropic_key = into_optional_secret(value);
            }
            ENV_OPENAI_API_KEY => {
                *openai_key = into_optional_secret(value);
            }
            ENV_MODEL_ALLOWLIST | ENV_ANTHROPIC_MODEL | ENV_OPENAI_MODEL | ENV_BUDGET_USD => {
                if !value.trim().is_empty() {
                    non_secret.insert(key, std::mem::take(&mut value));
                }
                value.zeroize();
            }
            _ => {
                value.zeroize();
            }
        }
    }
    Ok(())
}

fn into_optional_secret(mut raw: String) -> Option<SecretString> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        raw.zeroize();
        return None;
    }
    if trimmed.len() == raw.len() {
        return Some(SecretString::from(raw));
    }
    let cleaned = trimmed.to_owned();
    raw.zeroize();
    Some(SecretString::from(cleaned))
}

fn resolve_secret_key(
    env_val: Option<String>,
    dotenv_val: Option<SecretString>,
) -> Option<SecretString> {
    match env_val {
        Some(raw) => match into_optional_secret(raw) {
            Some(secret) => Some(secret),
            None => dotenv_val,
        },
        None => dotenv_val,
    }
}

fn resolve_allowed_models(raw_allowlist: Option<String>) -> Result<Vec<LiveModel>, LiveGuardError> {
    let Some(raw) = raw_allowlist else {
        return Ok(vec![LiveModel::ClaudeHaiku55, LiveModel::Gpt6Luna]);
    };

    let mut models = Vec::new();
    for entry in raw.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = LiveModel::parse_allowed(trimmed)?;
        if !models.contains(&parsed) {
            models.push(parsed);
        }
    }

    if models.is_empty() {
        return Err(LiveGuardError::ModelNotAllowed {
            model: String::from("<allowlist vide>"),
        });
    }
    Ok(models)
}

fn validate_model_against(
    model: &str,
    allowed_models: &[LiveModel],
) -> Result<LiveModel, LiveGuardError> {
    let parsed = LiveModel::parse_allowed(model)?;
    if !allowed_models.contains(&parsed) {
        return Err(LiveGuardError::ModelNotAllowed {
            model: model.to_owned(),
        });
    }
    Ok(parsed)
}

fn parse_usd_to_micros(raw: &str) -> Result<u64, LiveGuardError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with('+') {
        return Err(LiveGuardError::InvalidBudget {
            raw: raw.to_owned(),
        });
    }

    let mut parts = trimmed.split('.');
    let whole_str = parts.next().unwrap_or("");
    let frac_str = parts.next();
    if parts.next().is_some() {
        return Err(LiveGuardError::InvalidBudget {
            raw: raw.to_owned(),
        });
    }

    if whole_str.is_empty() && frac_str.is_none_or(str::is_empty) {
        return Err(LiveGuardError::InvalidBudget {
            raw: raw.to_owned(),
        });
    }

    let whole: u64 = if whole_str.is_empty() {
        0
    } else {
        if !whole_str.chars().all(|c| c.is_ascii_digit()) {
            return Err(LiveGuardError::InvalidBudget {
                raw: raw.to_owned(),
            });
        }
        whole_str
            .parse::<u64>()
            .map_err(|_| LiveGuardError::InvalidBudget {
                raw: raw.to_owned(),
            })?
    };

    let frac_micros: u64 = match frac_str {
        None => 0,
        Some("") => {
            return Err(LiveGuardError::InvalidBudget {
                raw: raw.to_owned(),
            });
        }
        Some(f) => {
            if f.len() > 6 || !f.chars().all(|c| c.is_ascii_digit()) {
                return Err(LiveGuardError::InvalidBudget {
                    raw: raw.to_owned(),
                });
            }
            let mut padded = String::with_capacity(6);
            padded.push_str(f);
            while padded.len() < 6 {
                padded.push('0');
            }
            padded
                .parse::<u64>()
                .map_err(|_| LiveGuardError::InvalidBudget {
                    raw: raw.to_owned(),
                })?
        }
    };

    whole
        .checked_mul(MICROS_PER_USD)
        .and_then(|w| w.checked_add(frac_micros))
        .ok_or_else(|| LiveGuardError::InvalidBudget {
            raw: raw.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_usd_to_micros_exact() {
        assert_eq!(parse_usd_to_micros("5").unwrap(), 5_000_000);
        assert_eq!(parse_usd_to_micros("0.25").unwrap(), 250_000);
        assert_eq!(parse_usd_to_micros("1.234567").unwrap(), 1_234_567);
        assert_eq!(parse_usd_to_micros("0").unwrap(), 0);
        assert!(parse_usd_to_micros("-1").is_err());
        assert!(parse_usd_to_micros("1.1234567").is_err());
        assert!(parse_usd_to_micros("nan").is_err());
    }

    #[test]
    fn test_closed_allowlist_constant() {
        assert_eq!(LIVE_MODEL_ALLOWLIST, ["claude-haiku-5-5", "gpt-6-luna"]);
        assert!(LiveModel::parse_allowed("claude-haiku-5-5").is_ok());
        assert!(LiveModel::parse_allowed("gpt-6-luna").is_ok());
        assert!(LiveModel::parse_allowed("gpt-6.1-sol").is_err());
        assert!(LiveModel::parse_allowed("claude-opus-5-5").is_err());
        assert!(LiveModel::parse_allowed("claude-fable-5-2").is_err());
        assert!(LiveModel::parse_allowed("claude-sonnet-5-5").is_err());
        assert!(LiveModel::parse_allowed("gpt-6-astra").is_err());
    }
}
