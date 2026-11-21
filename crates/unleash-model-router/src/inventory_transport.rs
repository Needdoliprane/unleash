//! Passive Ollama inventory boundary. No admission, provider qualification or
//! product route is owned here. Only the caller's broker can dispatch the GET.

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tokio_rustls::{TlsConnector, rustls};
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture, EffectsClass, ExecutorKind,
    NormalizedResourceRef, ResolvedSecret,
};
use unleash_domain::{ActionId, ActionState, ClockPort, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::provider_inventory::{MAX_INVENTORY_FACT_BYTES, MAX_INVENTORY_LIST_BYTES};
pub use unleash_protocol::provider_inventory::{
    OllamaInventory, OllamaListedModel, OllamaModelDetails,
};
use unleash_protocol::{canonical_sha256, canonicalize_to_vec};

use crate::connect_transport::{
    CertificateValidation, ConnectError, FrozenConnectRequest, OwnedLoopbackPeer, SystemResolver,
    TlsValidation, resolve_loopback_peer, system_tls_validation,
};
use crate::transport::{BrokerDispatch, ProfileRef};

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
    /// The expected native listener or accepted connection could not be verified.
    #[error("owned inventory loopback socket unavailable")]
    OwnershipUnavailable,
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
    /// The native producer did not receive an acknowledgement for its proof.
    #[error("inventory observation unavailable")]
    ObservationUnavailable,
}

/// Closed acknowledgement failure, without provider bytes or storage details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InventoryObservationError {
    /// The caller must retain uncertainty after the already dispatched GET.
    #[error("inventory observation acknowledgement unavailable")]
    Unavailable,
}

/// Receives only the exact native fact and typed list after channel closure.
/// An acknowledgement does not itself attest final resource disposition.
pub trait InventoryObservationPort: Send + Sync {
    /// Adopt this exact pair or fail without authorizing a retry.
    fn persist<'a>(
        &'a self,
        observation: &'a InventoryObservation,
        inventory: &'a OllamaInventory,
    ) -> BoxFuture<'a, Result<(), InventoryObservationError>>;
}

/// Frozen GET identity. Bounds belong to the caller's authorized request; they
/// are not provider capabilities or a new admission policy.
pub struct FrozenOllamaInventoryRequest {
    target: FrozenConnectRequest,
    digest: Sha256Digest,
    max_response_bytes: usize,
    connect_timeout_ms: Option<u64>,
    #[cfg(test)]
    fixture_tls: Option<TlsValidation>,
}

impl fmt::Debug for FrozenOllamaInventoryRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenOllamaInventoryRequest")
            .finish_non_exhaustive()
    }
}

impl FrozenOllamaInventoryRequest {
    /// Freeze separate connection and whole-exchange bounds for the durable
    /// inventory producer. The older constructor retains its original digest.
    ///
    /// # Errors
    /// Rejects missing, incompatible or unsupported connection and request bounds.
    pub fn new_loopback_with_limits(
        endpoint: &str,
        profile: ProfileRef,
        connect_timeout_ms: u64,
        timeout_ms: u64,
        max_response_bytes: usize,
    ) -> Result<Self, InventoryError> {
        if !(1..=timeout_ms).contains(&connect_timeout_ms) {
            return Err(InventoryError::InvalidRequest);
        }
        Self::freeze(
            endpoint,
            profile,
            Some(connect_timeout_ms),
            timeout_ms,
            max_response_bytes,
        )
    }

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
        Self::freeze(endpoint, profile, None, timeout_ms, max_response_bytes)
    }

    fn freeze(
        endpoint: &str,
        profile: ProfileRef,
        connect_timeout_ms: Option<u64>,
        timeout_ms: u64,
        max_response_bytes: usize,
    ) -> Result<Self, InventoryError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| InventoryError::InvalidRequest)?;
        if url.path() != "/api/tags"
            || !(1..=MAX_INVENTORY_LIST_BYTES).contains(&max_response_bytes)
        {
            return Err(InventoryError::InvalidRequest);
        }
        let schema = if connect_timeout_ms.is_some() {
            "unleash.ollama_inventory.v2"
        } else {
            "unleash.ollama_inventory.v1"
        };
        let mut commitment = serde_json::json!({
            "schema":schema, "method":"GET", "endpoint":url.as_str(),
            "profile_id":profile.id, "profile_version":profile.version, "profile_digest":profile.digest,
            "timeout_ms":timeout_ms, "max_response_bytes":max_response_bytes,
            "request_body_bytes":0, "redirects":false, "retries":0,
        });
        if let Some(bound) = connect_timeout_ms {
            commitment
                .as_object_mut()
                .ok_or(InventoryError::DigestUnavailable)?
                .insert("connect_timeout_ms".into(), serde_json::Value::from(bound));
        }
        let digest =
            canonical_sha256(&commitment).map_err(|_| InventoryError::DigestUnavailable)?;
        let target = FrozenConnectRequest::new_loopback(endpoint, profile, timeout_ms)
            .map_err(|_| InventoryError::InvalidRequest)?;
        Ok(Self {
            target,
            digest,
            max_response_bytes,
            connect_timeout_ms,
            #[cfg(test)]
            fixture_tls: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_fixture_root(
        mut self,
        certificate_der: &[u8],
    ) -> Result<Self, ConnectError> {
        self.fixture_tls = Some(crate::connect_transport::fixture_tls_validation(
            certificate_der,
        )?);
        Ok(self)
    }

    /// Require a pinned native peer for a bounded inventory with an observation port.
    ///
    /// # Errors
    /// Rejects a legacy request, a different endpoint, or a second binding.
    pub fn with_owned_peer(mut self, owned: OwnedLoopbackPeer) -> Result<Self, InventoryError> {
        if self.connect_timeout_ms.is_none() {
            return Err(InventoryError::InvalidRequest);
        }
        let digest = owned
            .bind_digest(self.digest)
            .map_err(|_| InventoryError::DigestUnavailable)?;
        self.target = self
            .target
            .with_owned_peer(owned)
            .map_err(|_| InventoryError::InvalidRequest)?;
        self.digest = digest;
        Ok(self)
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
    observation_port: Option<Arc<dyn InventoryObservationPort>>,
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

    async fn tls(&self) -> Result<TlsValidation, InventoryError> {
        #[cfg(test)]
        if let Some(fixture) = &self.request.fixture_tls {
            return Ok(TlsValidation {
                config: Arc::clone(&fixture.config),
                source: fixture.source,
            });
        }
        tokio::task::spawn_blocking(system_tls_validation)
            .await
            .map_err(|_| InventoryError::Unavailable)?
            .map_err(|_| InventoryError::Unavailable)
    }

    async fn channel(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        total_deadline: Instant,
    ) -> Result<(Box<dyn Channel>, SocketAddr, CertificateValidation), InventoryError> {
        let tls = if self.request.target.endpoint().scheme() == "https" {
            Some(self.tls().await?)
        } else {
            None
        };
        let peer = resolve_loopback_peer(&self.request.target, &SystemResolver)
            .await
            .map_err(|_| InventoryError::InvalidRequest)?;
        if !self.live(intent, decision) {
            return Err(InventoryError::DeadlineExpired);
        }
        let deadline = if let Some(bound) = self.request.connect_timeout_ms {
            let window = bound.min(self.remaining(intent, decision)?);
            (Instant::now() + Duration::from_millis(window)).min(total_deadline)
        } else {
            total_deadline
        };
        let (channel, source) = tokio::time::timeout_at(
            deadline,
            self.connect_channel(peer, tls, deadline, intent, decision),
        )
        .await
        .map_err(|_| InventoryError::DeadlineExpired)??;
        if !self.live(intent, decision) || Instant::now() >= deadline {
            return Err(InventoryError::DeadlineExpired);
        }
        Ok((channel, peer, source))
    }

    async fn connect_channel(
        &self,
        peer: SocketAddr,
        tls: Option<TlsValidation>,
        deadline: Instant,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<(Box<dyn Channel>, CertificateValidation), InventoryError> {
        let socket = self
            .request
            .target
            .open_socket(peer, deadline, || self.live(intent, decision))
            .await
            .map_err(|error| match error {
                ConnectError::OwnershipUnavailable => InventoryError::OwnershipUnavailable,
                ConnectError::DestinationRejected => InventoryError::InvalidRequest,
                ConnectError::DeadlineExpired => InventoryError::DeadlineExpired,
                _ => InventoryError::Unavailable,
            })?;
        if !self.live(intent, decision) || Instant::now() >= deadline {
            return Err(InventoryError::DeadlineExpired);
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
        Ok((channel, source))
    }

    async fn observe(
        &self,
        dispatch: DispatchId,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        deadline: Instant,
    ) -> Result<ObservedInventory, InventoryError> {
        let (mut channel, peer, certificate_validation) =
            self.channel(intent, decision, deadline).await?;
        if !self.live(intent, decision) || Instant::now() >= deadline {
            return Err(InventoryError::DeadlineExpired);
        }
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
            inventory_digest: self.observation_digest(&inventory, MAX_INVENTORY_LIST_BYTES)?,
            observed_at_ms: self.clock.now_ms(),
        };
        let digest = self.observation_digest(&observation, MAX_INVENTORY_FACT_BYTES)?;
        Ok(ObservedInventory {
            inventory,
            observation,
            digest,
        })
    }

    fn observation_digest<T: Serialize>(
        &self,
        value: &T,
        maximum: usize,
    ) -> Result<Sha256Digest, InventoryError> {
        let bytes = canonicalize_to_vec(value).map_err(|_| InventoryError::DigestUnavailable)?;
        if self.observation_port.is_some() && bytes.len() > maximum {
            return Err(InventoryError::ResponseTooLarge);
        }
        Ok(Sha256Digest::from_bytes(&bytes))
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
            let deadline = Instant::now() + Duration::from_millis(remaining);
            let observed = tokio::time::timeout_at(deadline, async {
                let observed = self
                    .observe(dispatch_id, intent, decision, deadline)
                    .await?;
                if let Some(port) = &self.observation_port {
                    if !self.live(intent, decision) || Instant::now() >= deadline {
                        return Err(InventoryError::DeadlineExpired);
                    }
                    port.persist(&observed.observation, &observed.inventory)
                        .await
                        .map_err(|_| InventoryError::ObservationUnavailable)?;
                }
                Ok(observed)
            })
            .await
            .unwrap_or(Err(InventoryError::DeadlineExpired));
            let observed = if self.live(intent, decision) && Instant::now() < deadline {
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
    if request.connect_timeout_ms.is_some() {
        return Err(InventoryError::ObservationUnavailable);
    }
    execute_inventory(request, dispatch, None).await
}

async fn execute_inventory(
    request: FrozenOllamaInventoryRequest,
    dispatch: BrokerDispatch<'_>,
    observation_port: Option<Arc<dyn InventoryObservationPort>>,
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
        observation_port,
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

/// Execute one broker-authorized GET and await adoption of both native objects
/// after the response channel closes, before returning a successful result to
/// the broker. The caller still owns admission and final resource disposition.
///
/// # Errors
/// Rejects a divergent action or secret reference before network access. An
/// adoption failure after dispatch preserves uncertainty and issues no retry.
pub async fn execute_ollama_inventory_with_observation(
    request: FrozenOllamaInventoryRequest,
    dispatch: BrokerDispatch<'_>,
    observation_port: Arc<dyn InventoryObservationPort>,
) -> Result<InventoryExecution, InventoryError> {
    execute_inventory(request, dispatch, Some(observation_port)).await
}

#[cfg(test)]
mod request_tests {
    use super::{
        FrozenOllamaInventoryRequest, InventoryError, MAX_INVENTORY_LIST_BYTES as MAX_RESPONSE,
    };
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

    fn bounded(
        connect_timeout_ms: u64,
        timeout_ms: u64,
    ) -> Result<FrozenOllamaInventoryRequest, InventoryError> {
        let profile = ProfileRef::new(
            "synthetic-inventory-profile",
            1,
            Sha256Digest::from_bytes(b"profile"),
        )
        .map_err(|_| InventoryError::InvalidRequest)?;
        FrozenOllamaInventoryRequest::new_loopback_with_limits(
            "http://127.0.0.1:11434/api/tags",
            profile,
            connect_timeout_ms,
            timeout_ms,
            MAX_RESPONSE,
        )
    }

    #[test]
    fn inventory_v2_binds_connection_and_exchange_limits_separately() -> Result<(), InventoryError>
    {
        let frozen = bounded(250, 1000)?;
        let expected = unleash_protocol::canonical_sha256(&serde_json::json!({
            "schema":"unleash.ollama_inventory.v2","method":"GET","endpoint":"http://127.0.0.1:11434/api/tags",
            "profile_id":"synthetic-inventory-profile","profile_version":1,"profile_digest":Sha256Digest::from_bytes(b"profile"),
            "connect_timeout_ms":250,"timeout_ms":1000,"max_response_bytes":MAX_RESPONSE,
            "request_body_bytes":0,"redirects":false,"retries":0,
        })).map_err(|_| InventoryError::DigestUnavailable)?;
        assert_eq!(frozen.payload_digest(), expected);
        assert_ne!(
            frozen.payload_digest(),
            bounded(251, 1000)?.payload_digest()
        );
        assert_ne!(
            frozen.payload_digest(),
            bounded(250, 1001)?.payload_digest()
        );
        Ok(())
    }

    #[test]
    fn inventory_v2_refuses_absent_or_incompatible_connection_bound() {
        for (connect, exchange) in [(0, 1000), (1001, 1000), (1, 0), (1, 60001)] {
            assert!(matches!(
                bounded(connect, exchange),
                Err(InventoryError::InvalidRequest)
            ));
        }
    }
}
