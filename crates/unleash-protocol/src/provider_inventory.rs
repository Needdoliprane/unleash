//! Closed, canonical data for passive Ollama inventory.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum size of the canonical native fact.
pub const MAX_INVENTORY_FACT_BYTES: usize = 16_384;
/// Maximum size of the decoded response and canonical list.
pub const MAX_INVENTORY_LIST_BYTES: usize = 1_048_576;

/// Native list, including an explicitly empty list. It asserts no capability,
/// placement, model readiness or identity of weights beyond the observed fields.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaInventory {
    /// Models actually returned by the list endpoint.
    pub models: Vec<OllamaListedModel>,
}

impl fmt::Debug for OllamaInventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OllamaInventory")
            .field("model_count", &self.models.len())
            .finish_non_exhaustive()
    }
}

/// Native declarations are retained as observed; missing metadata stays absent.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaListedModel {
    /// Native model name, not a qualified identity.
    pub name: String,
    /// Native alias if present.
    pub model: Option<String>,
    /// Timestamp supplied by the backend, without fabrication or conversion.
    pub modified_at: Option<String>,
    /// Size supplied by the backend.
    pub size: Option<u64>,
    /// Opaque digest declared by the backend, not a locally verified weights hash.
    pub digest: Option<String>,
    /// Native details if supplied.
    pub details: Option<OllamaModelDetails>,
}

impl fmt::Debug for OllamaListedModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OllamaListedModel").finish_non_exhaustive()
    }
}

/// Details reported by `/api/tags`, distinct from tested capabilities.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaModelDetails {
    /// Declared format.
    pub format: Option<String>,
    /// Declared primary family.
    pub family: Option<String>,
    /// Declared families.
    pub families: Option<Vec<String>>,
    /// Declared parameter-size label.
    pub parameter_size: Option<String>,
    /// Declared quantization label.
    pub quantization_level: Option<String>,
}

impl fmt::Debug for OllamaModelDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OllamaModelDetails").finish_non_exhaustive()
    }
}
