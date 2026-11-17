//! Contrats d'entrée/sortie, journal d'effets durables et assertions déterministes pour [`crate::BrowserPort`].

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, LeaseId, LineageFence, ModelPolicy, RunId, SessionId, Sha256Digest,
};
use url::Url;

use crate::egress::EgressPolicy;
use crate::error::BrowserError;
use crate::locator::{LocatorStrategy, TargetSpec};
use crate::policy::{ConfinementProof, ExecutionMode, IsolationLevel};
use crate::runtime::PinnedBrowserRuntime;

/// Requête d'ouverture de contexte navigateur isolé rattaché à un run (`open_context`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenContextRequest {
    /// Identifiant du run propriétaire du contexte.
    pub run_id: RunId,
    /// Identifiant de session logique Unleash.
    pub session_id: SessionId,
    /// Bail sérialisant l'accès au périmètre métier.
    pub lease_id: LeaseId,
    /// Barrière d'autorité active.
    pub fence: LineageFence,
    /// URL `WebDriver` loopback (ex. `http://127.0.0.1:4444`).
    pub webdriver_endpoint: Url,
    /// Runtime navigateur et driver strictement épinglés.
    pub runtime: PinnedBrowserRuntime,
    /// Mode d'exécution (`mediated` ou `project_script`).
    pub execution_mode: ExecutionMode,
    /// Politique d'autorisation modèle dérivée du run.
    pub model_policy: ModelPolicy,
    /// Niveau d'isolation exigé (`standard` ou `strict`).
    pub isolation_level: IsolationLevel,
    /// Preuve de confinement OS fournie par `unleash-platform`/`unleash-process-host`.
    pub confinement_proof: ConfinementProof,
    /// Politique d'egress autorisée pour ce mandat.
    pub egress_policy: EgressPolicy,
    /// URL du proxy d'egress local Unleash à configurer dans la session `WebDriver`.
    pub proxy_url: Option<Url>,
    /// Répertoire de profil isolé dédié (`--user-data-dir`, permissions `0700`), jamais le profil personnel.
    pub user_data_dir: PathBuf,
    /// Exécution headless (`--headless=new`).
    pub headless: bool,
    /// Timeout des scripts internes de l'adaptateur.
    pub script_timeout: Duration,
    /// Timeout de chargement de page `WebDriver`.
    pub page_load_timeout: Duration,
}

/// Handle d'un contexte navigateur ouvert, lié à son run, à sa génération et à son fence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserContextHandle {
    /// Identifiant de session logique Unleash.
    pub session_id: SessionId,
    /// Identifiant de session `WebDriver` attribué par le driver.
    pub webdriver_session_id: String,
    /// Identifiant du run propriétaire.
    pub run_id: RunId,
    /// Bail associé au contexte.
    pub lease_id: LeaseId,
    /// Barrière d'autorité sous laquelle le contexte a été ouvert.
    pub fence: LineageFence,
    /// Génération monotone de ce contexte.
    pub context_generation: u64,
    /// Mode d'exécution effectif.
    pub execution_mode: ExecutionMode,
    /// Politique modèle effective.
    pub model_policy: ModelPolicy,
    /// Répertoire de profil isolé.
    pub user_data_dir: PathBuf,
}

/// Portée d'observation demandée à [`crate::BrowserPort::observe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveScope {
    /// Inclut un extrait textuel borné du DOM courant.
    pub include_dom_summary: bool,
    /// Capture une capture d'écran PNG et calcule son empreinte `SHA-256`.
    pub include_screenshot: bool,
    /// Plafond d'octets conservés dans `text_excerpt`.
    pub max_text_bytes: usize,
}

impl Default for ObserveScope {
    fn default() -> Self {
        Self {
            include_dom_summary: true,
            include_screenshot: false,
            max_text_bytes: 16_384,
        }
    }
}

/// Observation horodatée et attribuable produite par [`crate::BrowserPort::observe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageObservation {
    /// Référence unique d'observation.
    pub observation_id: String,
    /// Génération du contexte.
    pub context_generation: u64,
    /// Génération du document observé.
    pub page_generation: u64,
    /// URL courante de la page.
    pub url: Url,
    /// Origine normalisée observée (`about:blank` avant première navigation).
    pub origin: String,
    /// Titre de la page.
    pub title: String,
    /// Extrait borné de la page (expurgé des secrets).
    pub text_excerpt: String,
    /// Empreinte `SHA-256` de la capture d'écran si demandée.
    pub screenshot_sha256: Option<Sha256Digest>,
    /// Horodatage UTC (ms) fourni par le port d'horloge injecté.
    pub observed_at_ms: i64,
}

/// Catalogue fermé des opérations d'action V1 de [`crate::BrowserPort::perform_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserActionKind {
    /// Navigation vers une URL dont l'origine est vérifiée contre [`EgressPolicy`].
    Navigate {
        /// URL cible.
        url: Url,
    },
    /// Clic `WebDriver` sur la cible résolue.
    Click,
    /// Saisie de texte dans un champ cible.
    Fill {
        /// Texte à saisir.
        value: String,
        /// Efface le champ avant saisie.
        clear_first: bool,
    },
    /// Sélection d'option dans un `<select>`.
    SelectOption {
        /// Valeur de l'option.
        value: String,
    },
    /// Coche ou décoche une case à cocher.
    SetChecked {
        /// État souhaité.
        checked: bool,
    },
    /// Envoi d'une touche clavier sur l'élément cible.
    PressKey {
        /// Touche à envoyer.
        key: String,
    },
    /// Lecture non mutante du texte de l'élément cible.
    ReadText,
    /// Lecture non mutante d'un attribut de l'élément cible.
    ReadAttribute {
        /// Nom de l'attribut HTML.
        name: String,
    },
    /// Prise de capture d'écran de la page courante.
    TakeScreenshot,
    /// Bascule vers une iframe par index ou retour au document parent (`None`).
    SwitchFrame {
        /// Index de frame ou `None` pour remonter au parent.
        index: Option<u16>,
    },
    /// Entrée explicitement interdite représentant un `execute_script` fourni par un modèle.
    ///
    /// Toujours rejetée avec [`BrowserError::ModelUseForbidden`] ou [`BrowserError::UnsupportedLocatorOrScript`].
    ModelSuppliedExecuteScript {
        /// Corps du script rejeté.
        script: String,
    },
}

impl BrowserActionKind {
    /// Indique si cette opération provoque une mutation ou une navigation externe.
    #[must_use]
    pub const fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Navigate { .. }
                | Self::Click
                | Self::Fill { .. }
                | Self::SelectOption { .. }
                | Self::SetChecked { .. }
                | Self::PressKey { .. }
        )
    }

    /// Indique si cette opération requiert une cible résolue préalable.
    #[must_use]
    pub const fn requires_target(&self) -> bool {
        matches!(
            self,
            Self::Click
                | Self::Fill { .. }
                | Self::SelectOption { .. }
                | Self::SetChecked { .. }
                | Self::PressKey { .. }
                | Self::ReadText
                | Self::ReadAttribute { .. }
        )
    }
}

/// Intention d'action typée transmise à [`crate::BrowserPort::perform_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserActionIntent {
    /// Identifiant unique de cette action durable.
    pub action_id: ActionId,
    /// Clé logique métier/effet utilisée pour interdire tout double dispatch d'un effet inconnu.
    pub logical_effect_key: String,
    /// Opération V1 demandée.
    pub kind: BrowserActionKind,
    /// Délai maximal autorisé pour cette opération.
    pub timeout: Duration,
    /// Indique si l'appel demande un recours modèle (bloqué sous `ModelPolicy::Forbidden`).
    pub requires_model_call: bool,
}

/// État connu ou inconnu d'un effet externe navigateur (`REQ-AUTO-011`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    /// Aucune mutation externe envoyée (lecture seule ou refus avant dispatch).
    None,
    /// Effet envoyé et acquitté par le driver (les postconditions métier restent à vérifier séparément).
    Confirmed,
    /// Échec avéré avant mutation (ex. élément non trouvé ou non interactif retourné par `WebDriver`).
    FailedKnown,
    /// Coupure ou timeout après début de dispatch : répétition strictement interdite sans réconciliation.
    OutcomeUnknown,
}

/// Résultat technique retourné par [`crate::BrowserPort::perform_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// Identifiant de l'action exécutée.
    pub action_id: ActionId,
    /// État de l'effet externe.
    pub effect_state: EffectState,
    /// Confirme que la réponse `WebDriver` a été reçue sans coupure.
    pub dispatch_confirmed: bool,
    /// Sortie textuelle éventuelle (ex. pour `ReadText` ou `ReadAttribute`).
    pub text_output: Option<String>,
    /// Empreinte `SHA-256` éventuelle (ex. pour `TakeScreenshot`).
    pub screenshot_sha256: Option<Sha256Digest>,
}

/// Enregistrement durable d'intention avant dispatch et de résolution d'effet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectJournalEntry {
    /// Identifiant de la dernière action pour cette clé logique.
    pub action_id: ActionId,
    /// Clé logique stable d'effet.
    pub effect_key: String,
    /// Barrière d'autorité au moment du pré-dispatch.
    pub fence: LineageFence,
    /// État courant de l'effet.
    pub state: EffectState,
    /// Détail ou référence de réconciliation.
    pub detail: String,
}

/// Port de journalisation durable des intentions d'effets avant dispatch (`REQ-AUTO-011`).
pub trait EffectJournalPort: Send + Sync {
    /// Vérifie s'il existe déjà un effet `OutcomeUnknown` non réconcilié pour `effect_key`.
    ///
    /// # Errors
    /// - [`BrowserError::OutcomeUnknown`] si la clé est bloquée en attente de réconciliation.
    fn check_unresolved_unknown(&self, effect_key: &str) -> Result<(), BrowserError>;

    /// Persiste l'intention d'action **avant** tout envoi `WebDriver`.
    ///
    /// # Errors
    /// - [`BrowserError::OutcomeUnknown`] si la clé est déjà `OutcomeUnknown`.
    fn record_pre_dispatch(
        &self,
        action_id: ActionId,
        effect_key: &str,
        fence: LineageFence,
        detail: &str,
    ) -> Result<(), BrowserError>;

    /// Marque l'effet comme techniquement confirmé par la réponse `WebDriver`.
    fn mark_confirmed(&self, effect_key: &str, detail: &str);

    /// Marque l'effet comme échoué avant mutation certaine.
    fn mark_failed_known(&self, effect_key: &str, detail: &str);

    /// Marque l'effet comme `OutcomeUnknown` après coupure ou timeout post-dispatch.
    fn mark_outcome_unknown(&self, action_id: ActionId, effect_key: &str, reason: &str);

    /// Réconcilie explicitement un effet `OutcomeUnknown` à partir d'une preuve ou disposition.
    ///
    /// # Errors
    /// - [`BrowserError::InvalidConfiguration`] si `new_state == EffectState::OutcomeUnknown` ou
    ///   si l'entrée est absente.
    fn reconcile_outcome(
        &self,
        effect_key: &str,
        new_state: EffectState,
        proof_ref: &str,
    ) -> Result<(), BrowserError>;

    /// Retourne l'instantané de toutes les entrées enregistrées.
    fn snapshot(&self) -> Vec<EffectJournalEntry>;
}

/// Implémentation en mémoire thread-safe de [`EffectJournalPort`] pour M0 et les tests.
#[derive(Debug, Default)]
pub struct InMemoryEffectJournal {
    entries: Mutex<HashMap<String, EffectJournalEntry>>,
}

impl InMemoryEffectJournal {
    /// Crée un journal vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EffectJournalPort for InMemoryEffectJournal {
    fn check_unresolved_unknown(&self, effect_key: &str) -> Result<(), BrowserError> {
        let guard = self
            .entries
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou du journal d'effets corrompu".into(),
            })?;
        if let Some(entry) = guard.get(effect_key)
            && entry.state == EffectState::OutcomeUnknown
        {
            return Err(BrowserError::OutcomeUnknown {
                action_id: entry.action_id,
                effect_key: entry.effect_key.clone(),
                reason: format!(
                    "Répétition interdite : effet précédent non réconcilié ({})",
                    entry.detail
                ),
            });
        }
        Ok(())
    }

    fn record_pre_dispatch(
        &self,
        action_id: ActionId,
        effect_key: &str,
        fence: LineageFence,
        detail: &str,
    ) -> Result<(), BrowserError> {
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou du journal d'effets corrompu".into(),
            })?;
        if let Some(existing) = guard.get(effect_key)
            && existing.state == EffectState::OutcomeUnknown
        {
            return Err(BrowserError::OutcomeUnknown {
                action_id: existing.action_id,
                effect_key: existing.effect_key.clone(),
                reason: "Effet déjà bloqué en OutcomeUnknown avant pré-dispatch".into(),
            });
        }
        guard.insert(
            effect_key.to_owned(),
            EffectJournalEntry {
                action_id,
                effect_key: effect_key.to_owned(),
                fence,
                state: EffectState::None,
                detail: detail.to_owned(),
            },
        );
        Ok(())
    }

    fn mark_confirmed(&self, effect_key: &str, detail: &str) {
        if let Ok(mut guard) = self.entries.lock()
            && let Some(entry) = guard.get_mut(effect_key)
        {
            entry.state = EffectState::Confirmed;
            detail.clone_into(&mut entry.detail);
        }
    }

    fn mark_failed_known(&self, effect_key: &str, detail: &str) {
        if let Ok(mut guard) = self.entries.lock()
            && let Some(entry) = guard.get_mut(effect_key)
        {
            entry.state = EffectState::FailedKnown;
            detail.clone_into(&mut entry.detail);
        }
    }

    fn mark_outcome_unknown(&self, action_id: ActionId, effect_key: &str, reason: &str) {
        if let Ok(mut guard) = self.entries.lock() {
            let entry = guard
                .entry(effect_key.to_owned())
                .or_insert_with(|| EffectJournalEntry {
                    action_id,
                    effect_key: effect_key.to_owned(),
                    fence: LineageFence::new(
                        unleash_domain::AuthorityGeneration::from_u128(0),
                        unleash_domain::DaemonEpoch::INITIAL,
                        unleash_domain::Fence::ZERO,
                    ),
                    state: EffectState::OutcomeUnknown,
                    detail: reason.to_owned(),
                });
            entry.action_id = action_id;
            entry.state = EffectState::OutcomeUnknown;
            reason.clone_into(&mut entry.detail);
        }
    }

    fn reconcile_outcome(
        &self,
        effect_key: &str,
        new_state: EffectState,
        proof_ref: &str,
    ) -> Result<(), BrowserError> {
        if new_state == EffectState::OutcomeUnknown {
            return Err(BrowserError::InvalidConfiguration {
                reason: "La réconciliation doit aboutir à un état déterminé".into(),
            });
        }
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| BrowserError::InvalidConfiguration {
                reason: "Verrou du journal d'effets corrompu".into(),
            })?;
        let entry =
            guard
                .get_mut(effect_key)
                .ok_or_else(|| BrowserError::InvalidConfiguration {
                    reason: format!("Effet '{effect_key}' introuvable pour réconciliation"),
                })?;
        entry.state = new_state;
        entry.detail = format!("reconciled:{proof_ref}");
        Ok(())
    }

    fn snapshot(&self) -> Vec<EffectJournalEntry> {
        self.entries
            .lock()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// Règle déterministe publiée évaluée par [`crate::BrowserPort::evaluate_assertions`] (`REQ-AUTO-010`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum BusinessAssertion {
    /// Vérifie que l'origine observée correspond exactement à l'origine attendue.
    UrlOriginEquals {
        /// Identifiant de la règle dans le rapport.
        rule_id: String,
        /// Origine normalisée attendue.
        expected_origin: String,
    },
    /// Vérifie que le chemin d'URL courant correspond exactement au chemin attendu.
    UrlPathEquals {
        /// Identifiant de la règle.
        rule_id: String,
        /// Chemin attendu (ex. `"/cart"`).
        expected_path: String,
    },
    /// Vérifie que le titre contient une sous-chaîne attendue.
    TitleContains {
        /// Identifiant de la règle.
        rule_id: String,
        /// Sous-chaîne attendue.
        substring: String,
    },
    /// Vérifie le texte exact d'un élément cible unique.
    ElementTextEquals {
        /// Identifiant de la règle.
        rule_id: String,
        /// Cible V1.
        target: TargetSpec,
        /// Texte attendu (après trim).
        expected: String,
    },
    /// Vérifie qu'un élément cible unique contient un texte attendu.
    ElementTextContains {
        /// Identifiant de la règle.
        rule_id: String,
        /// Cible V1.
        target: TargetSpec,
        /// Sous-chaîne attendue.
        substring: String,
    },
    /// Vérifie la valeur exacte d'un attribut HTML sur un élément cible unique.
    ElementAttrEquals {
        /// Identifiant de la règle.
        rule_id: String,
        /// Cible V1.
        target: TargetSpec,
        /// Nom de l'attribut.
        attr: String,
        /// Valeur attendue.
        expected: String,
    },
    /// Vérifie le nombre exact de correspondances d'une stratégie de ciblage.
    ElementCountEquals {
        /// Identifiant de la règle.
        rule_id: String,
        /// Stratégie V1.
        strategy: LocatorStrategy,
        /// Nombre attendu.
        expected: usize,
    },
    /// Vérifie qu'un montant entier en unités mineures lu sur la cible ne dépasse pas `max_minor`.
    NumericMinorAtMost {
        /// Identifiant de la règle.
        rule_id: String,
        /// Cible V1.
        target: TargetSpec,
        /// Plafond inclusif en unités mineures (ex. centimes).
        max_minor: u64,
    },
    /// Vérifie qu'une quantité entière lue sur la cible égale exactement `expected`.
    QuantityEquals {
        /// Identifiant de la règle.
        rule_id: String,
        /// Cible V1.
        target: TargetSpec,
        /// Quantité attendue.
        expected: u64,
    },
}

impl BusinessAssertion {
    /// Retourne l'identifiant stable de la règle.
    #[must_use]
    pub fn rule_id(&self) -> &str {
        match self {
            Self::UrlOriginEquals { rule_id, .. }
            | Self::UrlPathEquals { rule_id, .. }
            | Self::TitleContains { rule_id, .. }
            | Self::ElementTextEquals { rule_id, .. }
            | Self::ElementTextContains { rule_id, .. }
            | Self::ElementAttrEquals { rule_id, .. }
            | Self::ElementCountEquals { rule_id, .. }
            | Self::NumericMinorAtMost { rule_id, .. }
            | Self::QuantityEquals { rule_id, .. } => rule_id,
        }
    }
}

/// Verdict déterministe d'une règle d'assertion métier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionVerdict {
    /// Règle vérifiée conforme.
    Pass,
    /// Règle formellement contredite par l'observation.
    Fail,
    /// Règle non tranchable (ex. valeur illisible ou ambiguë).
    Unknown,
}

/// Résultat détaillé d'une règle évaluée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionRuleResult {
    /// Identifiant de la règle.
    pub rule_id: String,
    /// Verdict obtenu.
    pub verdict: AssertionVerdict,
    /// Explication factuelle de l'attendu vs observé.
    pub detail: String,
}

/// Rapport d'évaluation complet produit par [`crate::BrowserPort::evaluate_assertions`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionEvaluationReport {
    /// Origine contrôlée au moment de l'évaluation.
    pub controlled_origin: String,
    /// Résultats individuels par règle.
    pub results: Vec<AssertionRuleResult>,
    /// `true` uniquement si toutes les règles valent [`AssertionVerdict::Pass`].
    pub all_passed: bool,
}

impl AssertionEvaluationReport {
    /// Convertit un échec ou une incertitude d'assertion en [`BrowserError`].
    ///
    /// # Errors
    /// - [`BrowserError::BusinessPostconditionFailed`] si une règle vaut `Fail` et `after_effect` est `true`.
    /// - [`BrowserError::BusinessConstraintViolated`] si une règle vaut `Fail` et `after_effect` est `false`.
    /// - [`BrowserError::AssertionTimeout`] si aucune règle ne vaut `Fail` mais qu'une règle vaut `Unknown`.
    pub fn ensure_passed(&self, after_effect: bool) -> Result<(), BrowserError> {
        if self.all_passed {
            return Ok(());
        }
        if let Some(failed) = self
            .results
            .iter()
            .find(|r| r.verdict == AssertionVerdict::Fail)
        {
            if after_effect {
                return Err(BrowserError::BusinessPostconditionFailed {
                    message: format!("Règle '{}' en échec : {}", failed.rule_id, failed.detail),
                    evidence_ref: Some(format!("assertion:{}", failed.rule_id)),
                });
            }
            return Err(BrowserError::BusinessConstraintViolated {
                message: format!("Règle '{}' violée : {}", failed.rule_id, failed.detail),
            });
        }
        let unknown_detail = self
            .results
            .iter()
            .find(|r| r.verdict == AssertionVerdict::Unknown)
            .map_or_else(
                || "Assertion indéterminée".to_owned(),
                |r| format!("Règle '{}' indéterminée : {}", r.rule_id, r.detail),
            );
        Err(BrowserError::AssertionTimeout {
            message: unknown_detail,
        })
    }
}

/// Entrée fournie à [`crate::BrowserPort::checkpoint`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointInput {
    /// Référence logique d'étape ou de curseur de lot.
    pub step_ref: String,
    /// Dernière observation connue si disponible.
    pub last_observation_id: Option<String>,
    /// Métadonnées complémentaires typées (ex. numéro de ligne traitée).
    pub metadata: BTreeMap<String, String>,
}

/// Checkpoint durable retourné par [`crate::BrowserPort::checkpoint`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCheckpoint {
    /// Run propriétaire.
    pub run_id: RunId,
    /// Bail associé.
    pub lease_id: LeaseId,
    /// Fence courant.
    pub fence: LineageFence,
    /// Génération du contexte.
    pub context_generation: u64,
    /// Étape atteinte.
    pub step_ref: String,
    /// Dernière observation rattachée.
    pub last_observation_id: Option<String>,
    /// Effets confirmés enregistrés dans le journal.
    pub confirmed_effects: Vec<String>,
    /// Effets demeurant `OutcomeUnknown` et bloquant la reprise aveugle.
    pub unknown_effects: Vec<String>,
    /// Toujours `true` : les références éphémères DOM/WebElement doivent être réobservées à la reprise.
    pub ephemeral_handles_invalidated: bool,
}

/// Raison de fermeture d'un contexte navigateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// Fin normale du run ou de la réservation.
    Completed,
    /// Révocation ou annulation du mandat.
    RevokedOrCancelled,
    /// Nettoyage après échec ou timeout.
    ErrorCleanup,
}

/// Statut observé de la fermeture de session `WebDriver`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseOutcome {
    /// Session `WebDriver` fermée avec confirmation du driver.
    Closed,
    /// Fermeture incertaine (coupure pendant `DELETE /session/{id}`).
    Unknown,
}

/// Résultat retourné par [`crate::BrowserPort::close_context`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseContextResult {
    /// Session logique fermée.
    pub session_id: SessionId,
    /// Statut de fermeture observé.
    pub outcome: CloseOutcome,
    /// Détail diagnostique si la fermeture est incertaine.
    pub detail: String,
}
