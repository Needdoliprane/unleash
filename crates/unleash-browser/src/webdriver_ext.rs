//! Commandes `WebDriver` W3C complémentaires absentes du jeu standard de `fantoccini`.
//!
//! Couvre notamment les endpoints W3C `WebDriver` 2 `Get Computed Role` et
//! `Get Computed Label` nécessaires à la stratégie [`crate::LocatorStrategy::RoleName`]
//! et [`crate::LocatorStrategy::Label`].

use fantoccini::wd::WebDriverCompatibleCommand;
use http::Method;
use url::{ParseError, Url};

use crate::error::BrowserError;

/// Identifiant de version du script d'observation interne propre à l'adaptateur.
///
/// Aucun script libre ou fourni par un modèle n'est jamais exécuté via cet adaptateur.
pub const ADAPTER_OBSERVE_SCRIPT_VERSION: &str = "unleash.browser.observe.v1";

/// Commande W3C `WebDriver` 2 : `GET /session/{session id}/element/{element id}/computedrole`.
#[derive(Debug, Clone)]
pub struct GetComputedRole {
    /// Identifiant `WebElement` renvoyé par le driver.
    pub element_id: String,
}

impl WebDriverCompatibleCommand for GetComputedRole {
    fn endpoint(&self, base_url: &Url, session_id: Option<&str>) -> Result<Url, ParseError> {
        let sid = session_id.unwrap_or("");
        base_url.join(&format!(
            "session/{sid}/element/{}/computedrole",
            self.element_id
        ))
    }

    fn method_and_body(&self, _request_url: &Url) -> (Method, Option<String>) {
        (Method::GET, None)
    }
}

/// Commande W3C `WebDriver` 2 : `GET /session/{session id}/element/{element id}/computedlabel`.
#[derive(Debug, Clone)]
pub struct GetComputedLabel {
    /// Identifiant `WebElement` renvoyé par le driver.
    pub element_id: String,
}

impl WebDriverCompatibleCommand for GetComputedLabel {
    fn endpoint(&self, base_url: &Url, session_id: Option<&str>) -> Result<Url, ParseError> {
        let sid = session_id.unwrap_or("");
        base_url.join(&format!(
            "session/{sid}/element/{}/computedlabel",
            self.element_id
        ))
    }

    fn method_and_body(&self, _request_url: &Url) -> (Method, Option<String>) {
        (Method::GET, None)
    }
}

/// Commande W3C `WebDriver` : `GET /status` pour vérifier la disponibilité du driver.
#[derive(Debug, Clone, Copy)]
pub struct WebDriverStatusCommand;

impl WebDriverCompatibleCommand for WebDriverStatusCommand {
    fn endpoint(&self, base_url: &Url, _session_id: Option<&str>) -> Result<Url, ParseError> {
        base_url.join("status")
    }

    fn method_and_body(&self, _request_url: &Url) -> (Method, Option<String>) {
        (Method::GET, None)
    }
}

/// Extrait une chaîne JSON W3C optionnelle depuis la valeur retournée par `issue_cmd`.
///
/// # Errors
/// - [`BrowserError::InvalidConfiguration`] si le driver retourne un type JSON non textuel.
pub fn extract_w3c_optional_string(
    value: serde_json::Value,
    endpoint_name: &str,
) -> Result<Option<String>, BrowserError> {
    match value {
        serde_json::Value::String(s) => Ok(Some(s)),
        serde_json::Value::Null => Ok(None),
        other => Err(BrowserError::InvalidConfiguration {
            reason: format!("Réponse W3C inattendue sur {endpoint_name}: {other}"),
        }),
    }
}
