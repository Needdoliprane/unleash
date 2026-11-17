//! Matrice V1 fermée des locators navigateur et contrôle strict de cardinalité/identité.
//!
//! Réalise `REQ-AUTO-009` :
//! - Admet exclusivement `RoleName`, `Label`, `TestId` et `Css` justifié.
//! - Rejette explicitement `XPath`, les coordonnées d'écran et tout `execute_script` fourni par un modèle.
//! - Impose une cardinalité stricte (`0` → `TargetMissing`, `>1` → `AmbiguousTarget`),
//!   sans sélection opportuniste (`first()` interdit).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::BrowserError;
use crate::policy::ScriptOrigin;

/// Stratégie de ciblage V1 admise par la matrice de référence (`specs/browser-automation.md` §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum LocatorStrategy {
    /// Résolution sémantique par rôle ARIA calculé et nom accessible calculé.
    RoleName {
        /// Rôle ARIA attendu (ex. `"button"`, `"textbox"`, `"link"`).
        role: String,
        /// Nom accessible attendu.
        name: String,
        /// Correspondance exacte ou insensible à la casse après trim.
        exact: bool,
    },
    /// Résolution par libellé calculé ou associé à un contrôle de formulaire.
    Label {
        /// Libellé attendu.
        text: String,
        /// Correspondance exacte ou insensible à la casse après trim.
        exact: bool,
    },
    /// Résolution par attribut de test explicite (`data-testid` par défaut).
    TestId {
        /// Nom de l'attribut HTML (`data-testid` si `None`).
        attribute: Option<String>,
        /// Valeur exacte attendue.
        value: String,
    },
    /// Sélecteur CSS justifié explicitement dans la version publiée.
    Css {
        /// Sélecteur CSS non vide.
        selector: String,
        /// Justification documentaire dans l'artefact publié.
        rationale: String,
    },
}

impl LocatorStrategy {
    /// Construit un sélecteur CSS sûr équivalent lorsqu'applicable (`TestId` ou `Css`).
    #[must_use]
    pub fn as_direct_css(&self) -> Option<String> {
        match self {
            Self::TestId { attribute, value } => {
                let attr = attribute.as_deref().unwrap_or("data-testid");
                Some(format!("[{attr}=\"{}\"]", escape_css_attr_value(value)))
            }
            Self::Css { selector, .. } => Some(selector.clone()),
            Self::RoleName { .. } | Self::Label { .. } => None,
        }
    }
}

impl fmt::Display for LocatorStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RoleName { role, name, .. } => write!(f, "role={role}, name={name}"),
            Self::Label { text, .. } => write!(f, "label={text}"),
            Self::TestId { attribute, value } => {
                let attr = attribute.as_deref().unwrap_or("data-testid");
                write!(f, "{attr}={value}")
            }
            Self::Css { selector, .. } => write!(f, "css={selector}"),
        }
    }
}

/// Entrée brute potentiellement non fiable soumise pour validation contre la matrice V1.
///
/// Permet de rejeter explicitement `XPath`, les coordonnées et les scripts fournis par un modèle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum RawLocatorRequest {
    /// Rôle + nom accessible.
    RoleName {
        /// Rôle ARIA attendu.
        role: String,
        /// Nom accessible attendu.
        name: String,
        /// Correspondance exacte.
        exact: bool,
    },
    /// Libellé accessible.
    Label {
        /// Texte du libellé.
        text: String,
        /// Correspondance exacte.
        exact: bool,
    },
    /// Attribut de test (`data-testid` par défaut).
    TestId {
        /// Attribut optionnel.
        attribute: Option<String>,
        /// Valeur attendue.
        value: String,
    },
    /// Sélecteur CSS justifié.
    Css {
        /// Sélecteur CSS.
        selector: String,
        /// Justification.
        rationale: String,
    },
    /// `XPath` : explicitement exclu de la matrice V1.
    XPath {
        /// Expression `XPath` rejetée.
        expr: String,
    },
    /// Coordonnées d'écran : explicitement exclues de la matrice V1.
    Coordinates {
        /// Abscisse.
        x: f64,
        /// Ordonnée.
        y: f64,
    },
    /// Script arbitraire : rejeté s'il est fourni par un modèle (et jamais admis comme locator).
    ExecuteScript {
        /// Contenu du script.
        script: String,
        /// Provenance déclarée.
        origin: ScriptOrigin,
    },
}

/// Identité métier attendue sur l'élément résolu pour prévenir toute dérive (`REQ-AUTO-009`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpectedTargetIdentity {
    /// Texte visible attendu (exact après trim) si spécifié.
    pub expected_text: Option<String>,
    /// Identifiant SKU produit attendu (`data-sku`) si spécifié.
    pub expected_sku: Option<String>,
    /// Identifiant de compte attendu (`data-account`) si spécifié.
    pub expected_account: Option<String>,
    /// Autres paires attribut → valeur attendues.
    pub expected_attributes: BTreeMap<String, String>,
}

/// Spécification complète d'une cible déclarative dans une procédure publiée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSpec {
    /// Stratégie de résolution V1 validée.
    pub strategy: LocatorStrategy,
    /// Portée de frame optionnelle (`Some(...)` rejeté explicitement avec
    /// [`BrowserError::CapabilityMissing`] dans ce pilote ; utiliser [`crate::BrowserActionKind::SwitchFrame`]).
    pub frame_scope: Option<String>,
    /// Contraintes d'identité métier à vérifier avant toute action.
    pub expected_identity: Option<ExpectedTargetIdentity>,
}

impl TargetSpec {
    /// Valide et construit un [`TargetSpec`] depuis une [`RawLocatorRequest`] selon la matrice V1.
    ///
    /// # Errors
    /// - [`BrowserError::UnsupportedLocatorOrScript`] pour `XPath`, `Coordinates`, ou `ExecuteScript`.
    /// - [`BrowserError::InvalidConfiguration`] si un champ requis de stratégie valide ou `frame_scope` est vide/dangereux.
    pub fn try_from_raw(
        raw: RawLocatorRequest,
        frame_scope: Option<String>,
        expected_identity: Option<ExpectedTargetIdentity>,
    ) -> Result<Self, BrowserError> {
        if let Some(ref scope) = frame_scope
            && scope.trim().is_empty()
        {
            return Err(BrowserError::InvalidConfiguration {
                reason: "frame_scope ne peut pas être vide".into(),
            });
        }
        let strategy = match raw {
            RawLocatorRequest::RoleName { role, name, exact } => {
                if role.trim().is_empty() || name.trim().is_empty() {
                    return Err(BrowserError::InvalidConfiguration {
                        reason: "RoleName exige un rôle et un nom non vides".into(),
                    });
                }
                LocatorStrategy::RoleName { role, name, exact }
            }
            RawLocatorRequest::Label { text, exact } => {
                if text.trim().is_empty() {
                    return Err(BrowserError::InvalidConfiguration {
                        reason: "Label exige un texte non vide".into(),
                    });
                }
                LocatorStrategy::Label { text, exact }
            }
            RawLocatorRequest::TestId { attribute, value } => {
                if value.trim().is_empty() {
                    return Err(BrowserError::InvalidConfiguration {
                        reason: "TestId exige une valeur non vide".into(),
                    });
                }
                if let Some(ref attr) = attribute
                    && !is_safe_attr_name(attr)
                {
                    return Err(BrowserError::InvalidConfiguration {
                        reason: format!("Nom d'attribut TestId invalide : '{attr}'"),
                    });
                }
                LocatorStrategy::TestId { attribute, value }
            }
            RawLocatorRequest::Css {
                selector,
                rationale,
            } => {
                if selector.trim().is_empty() || rationale.trim().is_empty() {
                    return Err(BrowserError::InvalidConfiguration {
                        reason: "Css exige un sélecteur et une justification non vides".into(),
                    });
                }
                if selector.to_ascii_lowercase().contains("javascript:") {
                    return Err(BrowserError::UnsupportedLocatorOrScript {
                        reason: "Pseudo-protocole javascript: interdit dans un sélecteur CSS"
                            .into(),
                    });
                }
                LocatorStrategy::Css {
                    selector,
                    rationale,
                }
            }
            RawLocatorRequest::XPath { expr } => {
                return Err(BrowserError::UnsupportedLocatorOrScript {
                    reason: format!("Stratégie XPath exclue du profil mediated V1 : '{expr}'"),
                });
            }
            RawLocatorRequest::Coordinates { x, y } => {
                return Err(BrowserError::UnsupportedLocatorOrScript {
                    reason: format!("Coordonnées d'écran ({x}, {y}) exclues du profil mediated V1"),
                });
            }
            RawLocatorRequest::ExecuteScript { origin, .. } => {
                let msg = match origin {
                    ScriptOrigin::ModelSupplied => {
                        "execute_script fourni par un modèle strictement interdit en V1"
                    }
                    ScriptOrigin::AdapterVersioned => {
                        "execute_script ne constitue pas une stratégie de locator déclarative"
                    }
                };
                return Err(BrowserError::UnsupportedLocatorOrScript { reason: msg.into() });
            }
        };

        Ok(Self {
            strategy,
            frame_scope,
            expected_identity,
        })
    }
}

/// Cible résolue et vérifiée, valable uniquement pour la génération courante du contexte et de la page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTarget {
    /// Référence éphémère d'élément `WebDriver` (jamais persistée dans le plan de replay).
    pub element_id: String,
    /// Stratégie utilisée pour la résolution.
    pub strategy: LocatorStrategy,
    /// Génération de contexte propriétaire.
    pub context_generation: u64,
    /// Génération de document au moment de la résolution.
    pub page_generation: u64,
    /// Rôle ARIA calculé si interrogé.
    pub computed_role: Option<String>,
    /// Nom accessible calculé si interrogé.
    pub computed_label: Option<String>,
    /// Texte observé sur l'élément.
    pub observed_text: String,
    /// Attributs observés pertinents pour l'identité métier.
    pub observed_attributes: BTreeMap<String, String>,
    /// Cardinalité observée (toujours `1`).
    pub candidate_count: usize,
}

/// Échappe une valeur de chaîne pour insertion sûre dans un sélecteur CSS `[attr="..."]`.
#[must_use]
pub fn escape_css_attr_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Vérifie la stricte unicité (`cardinality == 1`) de l'ensemble de candidats résolus.
///
/// # Errors
/// - [`BrowserError::TargetMissing`] si `candidates` est vide.
/// - [`BrowserError::AmbiguousTarget`] si `candidates.len() > 1`.
pub fn enforce_unique_candidate<T>(
    strategy: &LocatorStrategy,
    candidates: Vec<T>,
) -> Result<T, BrowserError> {
    let count = candidates.len();
    match count {
        0 => Err(BrowserError::TargetMissing {
            locator: strategy.to_string(),
        }),
        1 => candidates
            .into_iter()
            .next()
            .ok_or_else(|| BrowserError::TargetMissing {
                locator: strategy.to_string(),
            }),
        _ => Err(BrowserError::AmbiguousTarget {
            locator: strategy.to_string(),
            candidate_count: count,
        }),
    }
}

/// Contrôle les contraintes métier attendues sur l'élément unique résolu (`REQ-AUTO-009`).
///
/// # Errors
/// - [`BrowserError::TargetDrift`] si le texte, `data-sku`, `data-account` ou un attribut
///   déclaré diverge de l'attendu.
pub fn verify_target_identity(
    expected: &ExpectedTargetIdentity,
    observed_text: &str,
    observed_attributes: &BTreeMap<String, String>,
) -> Result<(), BrowserError> {
    if let Some(exp_text) = &expected.expected_text {
        let obs_trimmed = observed_text.trim();
        if obs_trimmed != exp_text.trim() {
            return Err(BrowserError::TargetDrift {
                expected: format!("text='{exp_text}'"),
                observed: format!("text='{obs_trimmed}'"),
            });
        }
    }

    if let Some(exp_sku) = &expected.expected_sku {
        let obs_sku = observed_attributes
            .get("data-sku")
            .map_or("<absent>", String::as_str);
        if obs_sku != exp_sku {
            return Err(BrowserError::TargetDrift {
                expected: format!("data-sku='{exp_sku}'"),
                observed: format!("data-sku='{obs_sku}'"),
            });
        }
    }

    if let Some(exp_account) = &expected.expected_account {
        let obs_acc = observed_attributes
            .get("data-account")
            .map_or("<absent>", String::as_str);
        if obs_acc != exp_account {
            return Err(BrowserError::TargetDrift {
                expected: format!("data-account='{exp_account}'"),
                observed: format!("data-account='{obs_acc}'"),
            });
        }
    }

    for (attr, exp_val) in &expected.expected_attributes {
        let obs_val = observed_attributes
            .get(attr)
            .map_or("<absent>", String::as_str);
        if obs_val != exp_val {
            return Err(BrowserError::TargetDrift {
                expected: format!("{attr}='{exp_val}'"),
                observed: format!("{attr}='{obs_val}'"),
            });
        }
    }

    Ok(())
}

/// Vérifie la correspondance d'un texte observé face au critère `exact` ou normalisé.
#[must_use]
pub fn text_matches(expected: &str, observed: &str, exact: bool) -> bool {
    if exact {
        observed.trim() == expected.trim()
    } else {
        observed.trim().eq_ignore_ascii_case(expected.trim())
    }
}

fn is_safe_attr_name(attr: &str) -> bool {
    !attr.is_empty()
        && attr
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_locator_rejects_xpath_coordinates_and_model_script() {
        let xpath = RawLocatorRequest::XPath {
            expr: "//button[1]".into(),
        };
        assert!(matches!(
            TargetSpec::try_from_raw(xpath, None, None),
            Err(BrowserError::UnsupportedLocatorOrScript { .. })
        ));

        let coords = RawLocatorRequest::Coordinates { x: 100.0, y: 200.0 };
        assert!(matches!(
            TargetSpec::try_from_raw(coords, None, None),
            Err(BrowserError::UnsupportedLocatorOrScript { .. })
        ));

        let model_js = RawLocatorRequest::ExecuteScript {
            script: "document.querySelector('button').click()".into(),
            origin: ScriptOrigin::ModelSupplied,
        };
        assert!(matches!(
            TargetSpec::try_from_raw(model_js, None, None),
            Err(BrowserError::UnsupportedLocatorOrScript { .. })
        ));
    }

    #[test]
    fn cardinality_and_identity_checks_block_missing_ambiguous_and_drift() {
        let strategy = LocatorStrategy::TestId {
            attribute: None,
            value: "add-apple".into(),
        };

        assert!(matches!(
            enforce_unique_candidate(&strategy, Vec::<&str>::new()),
            Err(BrowserError::TargetMissing { .. })
        ));
        assert!(matches!(
            enforce_unique_candidate(&strategy, vec!["e1", "e2"]),
            Err(BrowserError::AmbiguousTarget {
                candidate_count: 2,
                ..
            })
        ));
        assert_eq!(
            enforce_unique_candidate(&strategy, vec!["e1"]).unwrap(),
            "e1"
        );

        let expected = ExpectedTargetIdentity {
            expected_sku: Some("SYNTHETIC-APPLE-1KG".into()),
            ..Default::default()
        };
        let mut attrs = BTreeMap::new();
        attrs.insert("data-sku".into(), "SYNTHETIC-OTHER-SKU".into());
        let drift = verify_target_identity(&expected, "Add", &attrs);
        assert!(matches!(drift, Err(BrowserError::TargetDrift { .. })));
    }
}
