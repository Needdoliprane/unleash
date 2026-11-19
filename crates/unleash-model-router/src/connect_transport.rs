//! One-attempt loopback TCP/TLS component; no HTTP, model request or secret.
//! The caller still owns the real mandate, admission and durable broker journal.
//! Offline TLS chain/name validation is not native online revocation validation.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::json;
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture, EffectsClass, ExecutorKind,
    NormalizedResourceRef, ResolvedSecret,
};
use unleash_domain::{ActionId, ActionState, ClockPort, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::canonical_sha256;

use crate::transport::{BrokerDispatch, ProfileRef};

/// Exact verb of the passive transport component, not a model dispatch.
pub const CONNECT_VERB: &str = "model.provider.connect";

/// Closed local failure; no provider-controlled text is included.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// Invalid or unsupported immutable endpoint/bounds.
    #[error("invalid provider connect request")]
    InvalidRequest,
    /// Canonical metadata cannot be hashed.
    #[error("provider connect digest unavailable")]
    Digest,
    /// Observation staging is unavailable or mismatched.
    #[error("provider connect observation unavailable")]
    ObservationUnavailable,
    /// The local resolver could not return a result.
    #[error("provider connect resolution unavailable")]
    ResolutionUnavailable,
}

/// Immutable input for the first, explicitly loopback-only connect component.
pub struct FrozenConnectRequest {
    endpoint: reqwest::Url,
    host: String,
    port: u16,
    profile: ProfileRef,
    timeout_ms: u64,
}

impl fmt::Debug for FrozenConnectRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenConnectRequest")
            .finish_non_exhaustive()
    }
}

impl FrozenConnectRequest {
    /// Fix endpoint/profile and one total timeout. HTTP requires a loopback literal;
    /// TLS keeps the original host for strict certificate name validation.
    /// No DNS, socket, credential or qualification is obtained by this constructor.
    ///
    /// # Errors
    /// Rejects userinfo, query/fragment, unsupported schemes, port zero or bounds.
    pub fn new_loopback(
        endpoint: &str,
        profile: ProfileRef,
        timeout_ms: u64,
    ) -> Result<Self, ConnectError> {
        let endpoint = reqwest::Url::parse(endpoint).map_err(|_| ConnectError::InvalidRequest)?;
        let host = endpoint
            .host_str()
            .ok_or(ConnectError::InvalidRequest)?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = endpoint
            .port_or_known_default()
            .ok_or(ConnectError::InvalidRequest)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || port == 0
            || !(1..=60_000).contains(&timeout_ms)
            || endpoint.scheme() == "http"
                && !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        {
            return Err(ConnectError::InvalidRequest);
        }
        Ok(Self {
            endpoint,
            host,
            port,
            profile,
            timeout_ms,
        })
    }

    /// Exact digest required by the action intent and authorization decision.
    ///
    /// # Errors
    /// Returns `Digest` when canonical serialization is unavailable.
    pub fn payload_digest(&self) -> Result<Sha256Digest, ConnectError> {
        canonical_sha256(&json!({"schema":"unleash.provider_connect.v1","endpoint":self.endpoint.as_str(),
            "profile_id":self.profile.id,"profile_version":self.profile.version,"profile_digest":self.profile.digest,
            "destination_scope":"loopback_only","timeout_ms":self.timeout_ms,"http_application_bytes":0,
            "tls_validation":"offline_system_roots_chain_name","revocation":"unavailable"}))
            .map_err(|_|ConnectError::Digest)
    }

    /// Resource that the outer broker must authorize. The hint is not DNS proof.
    #[must_use]
    pub fn resource(&self) -> NormalizedResourceRef {
        NormalizedResourceRef::NetworkEndpoint {
            host: self.host.clone(),
            port: Some(self.port),
            is_private_or_loopback: self.host == "localhost"
                || self.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
        }
    }
}

/// What actually verified the TLS peer; never a claim of provider/model readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateValidation {
    /// Plain loopback TCP, no certificate.
    NotTls,
    /// Offline chain/date/name checking against locally loaded system roots.
    OfflineSystemRoots,
    /// Synthetic fixture trust anchor, unavailable in production builds.
    #[cfg(test)]
    FixtureRoot,
}

/// Local observation only; no provider receipt, model identity or usage is invented.
#[derive(Clone, Serialize)]
pub struct ConnectObservation {
    /// Version of this closed observation.
    pub schema_version: u32,
    /// Owning action.
    pub action_id: ActionId,
    /// Exact physical dispatch.
    pub dispatch_id: DispatchId,
    /// Frozen input identity.
    pub request_digest: Sha256Digest,
    /// Actual peer socket address.
    pub peer: SocketAddr,
    /// Certificate validation actually performed.
    pub certificate_validation: CertificateValidation,
    /// No local CRL source has been qualified by this component.
    pub revocation_checked: bool,
    /// Application bytes are never written by this component.
    pub http_application_bytes_sent: u64,
    /// Runtime observation timestamp.
    pub observed_at_ms: TimestampMs,
}

impl fmt::Debug for ConnectObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectObservation")
            .field("action_id", &self.action_id)
            .field("dispatch_id", &self.dispatch_id)
            .field("certificate_validation", &self.certificate_validation)
            .finish_non_exhaustive()
    }
}

/// Broker result and separately correlated, non-durable local observation.
pub struct ConnectExecution {
    /// The actual broker result, including uncertain commit outcomes.
    pub receipt: Result<ActionReceipt, ActionBrokerError>,
    /// Released only after a matching observed-success broker receipt.
    pub observation: Result<Option<ConnectObservation>, ConnectError>,
}

impl fmt::Debug for ConnectExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectExecution").finish_non_exhaustive()
    }
}

pub(crate) trait NameResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, ConnectError>>;
}

struct SystemResolver;
impl NameResolver for SystemResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, ConnectError>> {
        Box::pin(async move {
            if let Ok(ip) = host.parse::<IpAddr>() {
                return Ok(vec![SocketAddr::new(ip, port)]);
            }
            tokio::net::lookup_host((host, port))
                .await
                .map(|addresses| addresses.collect())
                .map_err(|_| ConnectError::ResolutionUnavailable)
        })
    }
}

#[cfg(test)]
pub(crate) struct FixtureOptions {
    pub resolver: Arc<dyn NameResolver>,
    pub ca_der: Option<Vec<u8>>,
}

struct Staged {
    action: ActionId,
    dispatch: DispatchId,
    digest: Sha256Digest,
    observation: ConnectObservation,
}

struct ConnectExecutor {
    request: FrozenConnectRequest,
    action: ActionId,
    digest: Sha256Digest,
    clock: Arc<dyn ClockPort>,
    resolver: Arc<dyn NameResolver>,
    consumed: AtomicBool,
    staged: Arc<Mutex<Option<Staged>>>,
    #[cfg(test)]
    ca_der: Option<Vec<u8>>,
}

impl ConnectExecutor {
    fn bound(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> bool {
        let resource = self.request.resource();
        intent.action_id == self.action
            && decision.action_id == self.action
            && intent.payload_digest == self.digest
            && decision.payload_digest == self.digest
            && intent.scope_revision == decision.scope_revision
            && intent.policy_revision == decision.policy_revision
            && intent.verb == CONNECT_VERB
            && intent.effects_class == EffectsClass::NetworkEgress
            && decision.executor == ExecutorKind::Unleash
            && intent.resource_refs.as_slice() == [resource]
            && intent.secret_refs.is_empty()
            && secrets.is_empty()
            && self.clock.now_ms() < intent.action_deadline
            && decision
                .expiry
                .is_none_or(|deadline| self.clock.now_ms() < deadline)
    }
}

impl ActionExecutorPort for ConnectExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(async move {
            if !self.bound(intent, decision, secrets) || self.consumed.swap(true, Ordering::SeqCst)
            {
                return Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                    dispatch_id,
                    reason: "connect_binding_rejected".to_owned(),
                });
            }
            Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                dispatch_id,
                reason: "connect_transport_not_implemented".to_owned(),
            })
        })
    }
}

/// Execute exclusively through the existing broker. Real durable journaling,
/// mandate/admission/fence proof and result persistence remain caller obligations.
///
/// # Errors
/// Reports local digest/staging failures; broker failures remain in `receipt`.
pub async fn execute(
    request: FrozenConnectRequest,
    dispatch: BrokerDispatch<'_>,
) -> Result<ConnectExecution, ConnectError> {
    execute_inner(
        request,
        dispatch,
        #[cfg(test)]
        None,
    )
    .await
}

async fn execute_inner(
    request: FrozenConnectRequest,
    dispatch: BrokerDispatch<'_>,
    #[cfg(test)] fixture: Option<FixtureOptions>,
) -> Result<ConnectExecution, ConnectError> {
    let digest = request.payload_digest()?;
    let staged = Arc::new(Mutex::new(None));
    #[cfg(test)]
    let (resolver, ca_der) = fixture.map_or_else(
        || (Arc::new(SystemResolver) as Arc<dyn NameResolver>, None),
        |options| (options.resolver, options.ca_der),
    );
    #[cfg(not(test))]
    let resolver = Arc::new(SystemResolver) as Arc<dyn NameResolver>;
    let executor = ConnectExecutor {
        request,
        action: dispatch.action,
        digest,
        clock: dispatch.clock,
        resolver,
        consumed: AtomicBool::new(false),
        staged: Arc::clone(&staged),
        #[cfg(test)]
        ca_der,
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
    let observation = if let Ok(receipt) = &receipt
        && receipt.state == ActionState::ObservedSuccess
    {
        let mut slot = staged
            .lock()
            .map_err(|_| ConnectError::ObservationUnavailable)?;
        match slot.take() {
            Some(staged)
                if staged.action == receipt.action_id
                    && staged.dispatch == receipt.dispatch_id
                    && receipt.output_digest == Some(staged.digest) =>
            {
                Ok(Some(staged.observation))
            }
            _ => Err(ConnectError::ObservationUnavailable),
        }
    } else {
        Ok(None)
    };
    Ok(ConnectExecution {
        receipt,
        observation,
    })
}

#[cfg(test)]
pub(crate) async fn execute_fixture(
    request: FrozenConnectRequest,
    dispatch: BrokerDispatch<'_>,
    fixture: FixtureOptions,
) -> Result<ConnectExecution, ConnectError> {
    execute_inner(request, dispatch, Some(fixture)).await
}
