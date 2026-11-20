//! Passive Ollama inventory boundary. No admission, provider qualification or
//! product route is owned here. Only the caller's broker can dispatch the GET.

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, rustls};
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture, EffectsClass, ExecutorKind,
    NormalizedResourceRef, ResolvedSecret,
};
use unleash_domain::{ActionId, ActionState, ClockPort, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::canonical_sha256;

use crate::connect_transport::{
    CertificateValidation, FrozenConnectRequest, SystemResolver, resolve_loopback_peer,
    system_tls_validation,
};
use crate::transport::{BrokerDispatch, MAX_RESPONSE, ProfileRef};

mod http;

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
    max_response_bytes: usize,
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
    /// Rejects a different route, zero body bound, a bound above the crate's
    /// 1 MiB response ceiling or an unsupported target/timeout.
    pub fn new_loopback(
        endpoint: &str,
        profile: ProfileRef,
        timeout_ms: u64,
        max_response_bytes: usize,
    ) -> Result<Self, InventoryError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| InventoryError::InvalidRequest)?;
        if url.path() != "/api/tags" || !(1..=MAX_RESPONSE).contains(&max_response_bytes) {
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
        Ok(Self {
            target,
            digest,
            max_response_bytes,
        })
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

/// Native, non-durable fact produced only after the complete bounded response
/// was read and the channel was closed. It is not server/model qualification.
#[derive(Serialize)]
pub struct InventoryObservation {
    schema_version: u32,
    action_id: ActionId,
    dispatch_id: DispatchId,
    request_digest: Sha256Digest,
    peer: SocketAddr,
    certificate_validation: CertificateValidation,
    revocation_checked: bool,
    status_code: u16,
    response_digest: Sha256Digest,
    response_body_bytes: usize,
    inventory_digest: Sha256Digest,
    observed_at_ms: TimestampMs,
}

impl fmt::Debug for InventoryObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InventoryObservation")
            .field("action_id", &self.action_id)
            .field("dispatch_id", &self.dispatch_id)
            .finish_non_exhaustive()
    }
}

impl InventoryObservation {
    /// Native owner; this grants no execution or durable adoption authority.
    #[must_use]
    pub const fn action_id(&self) -> ActionId {
        self.action_id
    }

    /// Native dispatch associated with the exact response.
    #[must_use]
    pub const fn dispatch_id(&self) -> DispatchId {
        self.dispatch_id
    }
}

/// The local result is not a durable inventory observation.
#[derive(Debug)]
pub enum InventoryOutcome {
    /// The exact native inventory was observed.
    Listed(OllamaInventory),
    /// No successful list can be attributed to this execution.
    Failed(InventoryError),
}

/// The broker receipt remains authoritative for an uncertain dispatched effect.
#[derive(Debug)]
pub struct InventoryExecution {
    /// Actual result of the caller's broker.
    pub receipt: Result<ActionReceipt, ActionBrokerError>,
    /// Current state read on that same action after the broker returned. In
    /// particular, `Dispatching` is not a committed `OutcomeUnknown` observation.
    pub state: ActionState,
    /// Correlated native list or closed local failure.
    pub outcome: InventoryOutcome,
    /// The exact local fact matching the broker's output digest, if successful.
    /// The caller still owns durable adoption and verified resource disposition.
    pub observation: Option<InventoryObservation>,
}

trait Channel: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Channel for T {}

struct ObservedInventory {
    inventory: OllamaInventory,
    observation: InventoryObservation,
    digest: Sha256Digest,
}

type Stage = Arc<Mutex<Option<Result<ObservedInventory, InventoryError>>>>;

struct InventoryExecutor {
    request: FrozenOllamaInventoryRequest,
    action: ActionId,
    clock: Arc<dyn ClockPort>,
    consumed: AtomicBool,
    stage: Stage,
}

impl InventoryExecutor {
    fn live(&self, intent: &ActionIntent, decision: &AuthorizationDecision) -> bool {
        let now = self.clock.now_ms();
        now < intent.action_deadline && decision.expiry.is_none_or(|expiry| now < expiry)
    }

    fn bound(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> bool {
        intent.action_id == self.action
            && decision.action_id == self.action
            && intent.verb == INVENTORY_VERB
            && intent.effects_class == EffectsClass::NetworkEgress
            && intent.payload_digest == self.request.digest
            && decision.payload_digest == self.request.digest
            && intent.scope_revision == decision.scope_revision
            && intent.policy_revision == decision.policy_revision
            && decision.executor == ExecutorKind::Unleash
            && intent.resource_refs.as_slice() == [self.request.resource()]
            && intent.secret_refs.is_empty()
            && secrets.is_empty()
            && self.live(intent, decision)
    }

    fn remaining(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<u64, InventoryError> {
        decision
            .expiry
            .map_or(intent.action_deadline, |expiry| {
                expiry.min(intent.action_deadline)
            })
            .as_i64()
            .checked_sub(self.clock.now_ms().as_i64())
            .and_then(|remaining| u64::try_from(remaining).ok())
            .filter(|remaining| *remaining > 0)
            .map(|remaining| remaining.min(self.request.target.timeout_ms()))
            .ok_or(InventoryError::DeadlineExpired)
    }

    async fn channel(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<(Box<dyn Channel>, SocketAddr, CertificateValidation), InventoryError> {
        let tls = if self.request.target.endpoint().scheme() == "https" {
            Some(
                tokio::task::spawn_blocking(system_tls_validation)
                    .await
                    .map_err(|_| InventoryError::Unavailable)?
                    .map_err(|_| InventoryError::Unavailable)?,
            )
        } else {
            None
        };
        let peer = resolve_loopback_peer(&self.request.target, &SystemResolver)
            .await
            .map_err(|_| InventoryError::InvalidRequest)?;
        if !self.live(intent, decision) {
            return Err(InventoryError::DeadlineExpired);
        }
        // The already validated sockaddr, never a hostname, selects this single
        // attempt. Recouple the actual peer BEFORE any HTTP application bytes.
        let socket = TcpStream::connect(peer)
            .await
            .map_err(|_| InventoryError::Unavailable)?;
        if socket
            .peer_addr()
            .map_err(|_| InventoryError::Unavailable)?
            != peer
        {
            return Err(InventoryError::InvalidRequest);
        }
        let (channel, source): (Box<dyn Channel>, _) = if let Some(tls) = tls {
            let name =
                rustls::pki_types::ServerName::try_from(self.request.target.host().to_owned())
                    .map_err(|_| InventoryError::InvalidRequest)?;
            let stream = TlsConnector::from(tls.config)
                .connect(name, socket)
                .await
                .map_err(|_| InventoryError::Unavailable)?;
            (Box::new(stream), tls.source)
        } else {
            (Box::new(socket), CertificateValidation::NotTls)
        };
        if !self.live(intent, decision) {
            return Err(InventoryError::DeadlineExpired);
        }
        Ok((channel, peer, source))
    }

    async fn observe(
        &self,
        dispatch: DispatchId,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<ObservedInventory, InventoryError> {
        let (mut channel, peer, certificate_validation) = self.channel(intent, decision).await?;
        let authority = self.request.target.http_authority();
        let request = format!(
            "GET /api/tags HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
        channel
            .write_all(request.as_bytes())
            .await
            .map_err(|_| InventoryError::Unavailable)?;
        let response =
            http::read_response(channel.as_mut(), self.request.max_response_bytes).await?;
        channel
            .shutdown()
            .await
            .map_err(|_| InventoryError::Unavailable)?;
        drop(channel);
        let inventory: OllamaInventory =
            serde_json::from_slice(&response.body).map_err(|_| InventoryError::InvalidResponse)?;
        let observation = InventoryObservation {
            schema_version: 1,
            action_id: self.action,
            dispatch_id: dispatch,
            request_digest: self.request.digest,
            peer,
            certificate_validation,
            revocation_checked: false,
            status_code: response.status,
            response_digest: Sha256Digest::from_bytes(&response.body),
            response_body_bytes: response.body.len(),
            inventory_digest: canonical_sha256(&inventory)
                .map_err(|_| InventoryError::DigestUnavailable)?,
            observed_at_ms: self.clock.now_ms(),
        };
        let digest =
            canonical_sha256(&observation).map_err(|_| InventoryError::DigestUnavailable)?;
        Ok(ObservedInventory {
            inventory,
            observation,
            digest,
        })
    }
}

impl ActionExecutorPort for InventoryExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(async move {
            if !self.bound(intent, decision, secrets) {
                return Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                    dispatch_id,
                    reason: "inventory_binding_rejected".to_owned(),
                });
            }
            let unknown = || ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "inventory_transport_outcome_unknown".to_owned(),
            };
            if self.consumed.swap(true, Ordering::SeqCst) {
                return Err(unknown());
            }
            let remaining = self.remaining(intent, decision).map_err(|_| unknown())?;
            let started = tokio::time::Instant::now();
            let duration = Duration::from_millis(remaining);
            let observed =
                tokio::time::timeout(duration, self.observe(dispatch_id, intent, decision))
                    .await
                    .unwrap_or(Err(InventoryError::DeadlineExpired));
            let observed = if self.live(intent, decision) && started.elapsed() < duration {
                observed
            } else {
                Err(InventoryError::DeadlineExpired)
            };
            let mut stage = self.stage.lock().map_err(|_| unknown())?;
            match observed {
                Ok(observed) => {
                    let result = ActionExecutionResult {
                        dispatch_id,
                        provider_request_id: None,
                        output_digest: observed.digest,
                        observed_at_ms: observed.observation.observed_at_ms,
                    };
                    *stage = Some(Ok(observed));
                    Ok(result)
                }
                Err(error) => {
                    *stage = Some(Err(error));
                    Err(unknown())
                }
            }
        })
    }
}

/// Execute one explicit GET through the caller's broker. Only a complete native
/// response with a matching local fact can yield a listed result. This component
/// does not persist that fact or authorize admission/resource release.
///
/// # Errors
/// Rejects an absent/divergent action or secret reference before resolution.
/// The first transport does not support credential-bearing inventory.
///
/// # Cancellation
/// Once the broker has dispatched, cancellation may leave an effect unknown.
/// The owner must reconcile that same action; this function never retries it.
pub async fn execute_ollama_inventory(
    request: FrozenOllamaInventoryRequest,
    dispatch: BrokerDispatch<'_>,
) -> Result<InventoryExecution, InventoryError> {
    if dispatch.broker.action_has_secret_refs(dispatch.action) != Some(false)
        || !dispatch
            .broker
            .action_audit(dispatch.action)
            .is_some_and(|audit| {
                audit.payload_digest == request.digest
                    && audit.decision_ref.as_ref() == Some(dispatch.decision)
            })
    {
        return Err(InventoryError::InvalidRequest);
    }
    let stage = Arc::new(Mutex::new(None));
    let executor = InventoryExecutor {
        request,
        action: dispatch.action,
        clock: dispatch.clock,
        consumed: AtomicBool::new(false),
        stage: Arc::clone(&stage),
    };
    let receipt = dispatch
        .broker
        .execute_authorized(
            dispatch.action,
            dispatch.decision,
            dispatch.fence,
            dispatch.authority,
            &executor,
            dispatch.secrets,
        )
        .await;
    let broker_state = dispatch
        .broker
        .action_state(dispatch.action)
        .ok_or(InventoryError::Unavailable)?;
    let result = stage
        .lock()
        .map_err(|_| InventoryError::Unavailable)?
        .take();
    match result {
        Some(Ok(observed))
            if broker_state == ActionState::ObservedSuccess
                && receipt.as_ref().is_ok_and(|receipt| {
                    receipt.state == ActionState::ObservedSuccess
                        && receipt.action_id == observed.observation.action_id
                        && receipt.dispatch_id == observed.observation.dispatch_id
                        && receipt.output_digest == Some(observed.digest)
                }) =>
        {
            Ok(InventoryExecution {
                receipt,
                state: broker_state,
                outcome: InventoryOutcome::Listed(observed.inventory),
                observation: Some(observed.observation),
            })
        }
        other => {
            let error = match other {
                Some(Err(error)) => error,
                _ => InventoryError::Unavailable,
            };
            Ok(InventoryExecution {
                receipt,
                state: broker_state,
                outcome: InventoryOutcome::Failed(error),
                observation: None,
            })
        }
    }
}

#[cfg(test)]
mod request_tests {
    use super::{FrozenOllamaInventoryRequest, InventoryError, MAX_RESPONSE};
    use crate::transport::ProfileRef;
    use unleash_domain::Sha256Digest;

    fn request(
        endpoint: &str,
        bound: usize,
    ) -> Result<FrozenOllamaInventoryRequest, InventoryError> {
        let profile = ProfileRef::new(
            "synthetic-inventory-profile",
            1,
            Sha256Digest::from_bytes(b"profile"),
        )
        .map_err(|_| InventoryError::InvalidRequest)?;
        FrozenOllamaInventoryRequest::new_loopback(endpoint, profile, 1000, bound)
    }

    #[test]
    fn t_f012_013_inventory_body_bound_cannot_exceed_transport_ceiling()
    -> Result<(), InventoryError> {
        for bound in [0, MAX_RESPONSE + 1, usize::MAX] {
            assert!(matches!(
                request("http://127.0.0.1:11434/api/tags", bound),
                Err(InventoryError::InvalidRequest)
            ));
        }
        let ceiling = request("http://127.0.0.1:11434/api/tags", MAX_RESPONSE)?;
        let smaller = request("http://127.0.0.1:11434/api/tags", MAX_RESPONSE - 1)?;
        assert_ne!(ceiling.payload_digest(), smaller.payload_digest());
        Ok(())
    }

    #[test]
    fn t_f012_013_inventory_ipv6_host_has_one_pair_of_brackets() -> Result<(), InventoryError> {
        let ipv6 = request("http://[::1]:11434/api/tags", 128)?;
        assert_eq!(ipv6.target.http_authority(), "[::1]:11434");
        let ipv4 = request("http://127.0.0.1:11434/api/tags", 128)?;
        assert_eq!(ipv4.target.http_authority(), "127.0.0.1:11434");
        Ok(())
    }
}
