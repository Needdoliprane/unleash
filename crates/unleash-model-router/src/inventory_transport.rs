//! Passive Ollama inventory boundary. No admission, provider qualification or
//! product route is owned here. The first implementation remains fail-closed.

use std::fmt;

use serde::{Deserialize, Serialize};
use unleash_action_broker::{ActionReceipt, NormalizedResourceRef};
use unleash_domain::Sha256Digest;
use unleash_protocol::canonical_sha256;

use crate::connect_transport::FrozenConnectRequest;
use crate::transport::{BrokerDispatch, ProfileRef};

/// This is metadata retrieval, not model generation or a connection-only proof.
pub const INVENTORY_VERB: &str = "model.provider.inventory";

/// Closed local failures contain no response, model name or credential value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InventoryError {
    /// An immutable request or caller-provided bound is invalid.
    #[error("invalid inventory request")]
    InvalidRequest,
    /// A canonical request identity is unavailable.
    #[error("inventory request digest unavailable")]
    DigestUnavailable,
    /// The effect path has not supplied a usable result.
    #[error("inventory transport unavailable")]
    Unavailable,
    /// A response exceeds the exact frozen body bound.
    #[error("inventory response too large")]
    ResponseTooLarge,
    /// A native response does not satisfy the bounded inventory dialect.
    #[error("invalid inventory response")]
    InvalidResponse,
    /// Redirects cannot enlarge the authorized destination.
    #[error("inventory redirect rejected")]
    RedirectRejected,
    /// The total request deadline expired.
    #[error("inventory deadline expired")]
    DeadlineExpired,
}

/// Frozen GET identity. Bounds belong to the caller's authorized request; they
/// are not provider capabilities or a new admission policy.
pub struct FrozenOllamaInventoryRequest {
    target: FrozenConnectRequest,
    digest: Sha256Digest,
}

impl fmt::Debug for FrozenOllamaInventoryRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenOllamaInventoryRequest")
            .finish_non_exhaustive()
    }
}

impl FrozenOllamaInventoryRequest {
    /// Freeze the exact native `/api/tags` route on a loopback target. This does
    /// not perform resolution, I/O or authorization.
    ///
    /// # Errors
    /// Rejects a different route, zero body bound or unsupported target/timeout.
    pub fn new_loopback(
        endpoint: &str,
        profile: ProfileRef,
        timeout_ms: u64,
        max_response_bytes: usize,
    ) -> Result<Self, InventoryError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| InventoryError::InvalidRequest)?;
        if url.path() != "/api/tags" || max_response_bytes == 0 {
            return Err(InventoryError::InvalidRequest);
        }
        let digest = canonical_sha256(&serde_json::json!({
            "schema":"unleash.ollama_inventory.v1", "method":"GET", "endpoint":url.as_str(),
            "profile_id":profile.id, "profile_version":profile.version, "profile_digest":profile.digest,
            "timeout_ms":timeout_ms, "max_response_bytes":max_response_bytes,
            "request_body_bytes":0, "redirects":false, "retries":0,
        }))
        .map_err(|_| InventoryError::DigestUnavailable)?;
        let target = FrozenConnectRequest::new_loopback(endpoint, profile, timeout_ms)
            .map_err(|_| InventoryError::InvalidRequest)?;
        Ok(Self { target, digest })
    }

    /// Identity that the actual broker intent and decision must both bind.
    #[must_use]
    pub const fn payload_digest(&self) -> Sha256Digest {
        self.digest
    }

    /// The existing broker must independently authorize this target.
    #[must_use]
    pub fn resource(&self) -> NormalizedResourceRef {
        self.target.resource()
    }
}

/// Native list, including an explicitly empty list. It asserts no capability,
/// placement, model readiness or identity of weights beyond the observed fields.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaInventory {
    /// Models actually returned by the list endpoint.
    pub models: Vec<OllamaListedModel>,
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

/// The local result is not a durable inventory observation.
pub enum InventoryOutcome {
    /// The exact native inventory was observed.
    Listed(OllamaInventory),
    /// No successful list can be attributed to this execution.
    Failed(InventoryError),
}

/// The broker receipt remains authoritative for an uncertain dispatched effect.
pub struct InventoryExecution {
    /// Actual result of the caller's broker.
    pub receipt: ActionReceipt,
    /// Correlated native list or closed local failure.
    pub outcome: InventoryOutcome,
}

/// Closed boundary for the test-first transport. No socket, secret access or
/// generic-broker bypass is available while its implementation is unqualified.
///
/// # Errors
/// Returns `Unavailable` while the effect path is closed.
pub async fn execute_ollama_inventory(
    _request: FrozenOllamaInventoryRequest,
    _dispatch: BrokerDispatch<'_>,
) -> Result<InventoryExecution, InventoryError> {
    Err(InventoryError::Unavailable)
}
