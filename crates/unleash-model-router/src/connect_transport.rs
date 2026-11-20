//! One-attempt loopback TCP/TLS component; no HTTP, model request or secret.
//! The caller still owns the real mandate, admission and durable broker journal.
//! Offline TLS chain/name validation is not native online revocation validation.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, rustls};
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

const MAX_RESOLVED_ADDRESSES: usize = 64;

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
    /// The complete resolved set is empty, oversized or outside the frozen target.
    #[error("provider connect destination rejected")]
    DestinationRejected,
    /// Offline trust anchors or the strict TLS configuration are unavailable.
    #[error("provider connect offline trust unavailable")]
    TrustUnavailable,
    /// The single TCP/TLS attempt did not establish the expected peer.
    #[error("provider connect transport unavailable")]
    TransportUnavailable,
    /// The action/decision or total monotonic timeout has expired.
    #[error("provider connect deadline expired")]
    DeadlineExpired,
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
    /// Fix endpoint/profile and one total timeout. The target must be a loopback
    /// literal, or `localhost` over TLS. TLS retains the name for certificate checks.
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
            || !(host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
                || endpoint.scheme() == "https" && host == "localhost")
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

    pub(crate) const fn endpoint(&self) -> &reqwest::Url {
        &self.endpoint
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) const fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    pub(crate) fn http_authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
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

/// Durable acknowledgement of the actual local transport observation.
/// The implementation is trusted server code, never a provider receipt or client claim.
pub trait ConnectObservationPort: Send + Sync {
    /// Persist these exact facts and their action/dispatch binding after channel
    /// closure and before the broker may commit a successful observation.
    ///
    /// # Errors
    /// Any failure or uncertain acknowledgement keeps the dispatched effect unknown.
    fn persist<'a>(
        &'a self,
        observation: &'a ConnectObservation,
    ) -> BoxFuture<'a, Result<(), ConnectError>>;
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

pub(crate) struct SystemResolver;
impl NameResolver for SystemResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, ConnectError>> {
        Box::pin(async move {
            if let Ok(ip) = host.parse::<IpAddr>()
                && ip.is_loopback()
            {
                return Ok(vec![SocketAddr::new(ip, port)]);
            }
            // The first component has no DNS permission or DNS worker. The
            // special localhost name is resolved locally, retaining its TLS name.
            if host == "localhost" {
                return Ok(vec![
                    SocketAddr::from(([127, 0, 0, 1], port)),
                    SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port),
                ]);
            }
            Err(ConnectError::ResolutionUnavailable)
        })
    }
}

#[cfg(test)]
pub(crate) struct FixtureOptions {
    pub resolver: Arc<dyn NameResolver>,
    pub ca_der: Option<Vec<u8>>,
    pub observation_port: Arc<dyn ConnectObservationPort>,
    pub closed_probe: Option<Arc<AtomicBool>>,
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
    observation_port: Arc<dyn ConnectObservationPort>,
    consumed: AtomicBool,
    staged: Arc<Mutex<Option<Staged>>>,
    #[cfg(test)]
    ca_der: Option<Vec<u8>>,
    #[cfg(test)]
    closed_probe: Option<Arc<AtomicBool>>,
}

pub(crate) struct TlsValidation {
    pub(crate) config: Arc<rustls::ClientConfig>,
    pub(crate) source: CertificateValidation,
}

fn tls_validation(
    roots: rustls::RootCertStore,
    source: CertificateValidation,
) -> Result<TlsValidation, ConnectError> {
    if roots.is_empty() {
        return Err(ConnectError::TrustUnavailable);
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| ConnectError::TrustUnavailable)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    // WebPKI is entirely offline: no AIA/OCSP fetch and no proxy or HTTP client.
    // No CRLs have been qualified, so revocation remains explicitly unobserved.
    Ok(TlsValidation {
        config: Arc::new(config),
        source,
    })
}

pub(crate) fn system_tls_validation() -> Result<TlsValidation, ConnectError> {
    // The loader otherwise honors these overrides. An environment-provided root
    // file must not be described as the operating system's configured trust.
    if std::env::var_os("SSL_CERT_FILE").is_some() || std::env::var_os("SSL_CERT_DIR").is_some() {
        return Err(ConnectError::TrustUnavailable);
    }
    let loaded = rustls_native_certs::load_native_certs();
    if !loaded.errors.is_empty() {
        return Err(ConnectError::TrustUnavailable);
    }
    let mut roots = rustls::RootCertStore::empty();
    for certificate in loaded.certs {
        roots
            .add(certificate)
            .map_err(|_| ConnectError::TrustUnavailable)?;
    }
    tls_validation(roots, CertificateValidation::OfflineSystemRoots)
}

pub(crate) async fn resolve_loopback_peer(
    request: &FrozenConnectRequest,
    resolver: &dyn NameResolver,
) -> Result<SocketAddr, ConnectError> {
    let addresses = resolver.resolve(&request.host, request.port).await?;
    let literal = request.host.parse::<IpAddr>().ok();
    if addresses.is_empty()
        || addresses.len() > MAX_RESOLVED_ADDRESSES
        || addresses.iter().any(|address| {
            address.port() != request.port
                || !address.ip().is_loopback()
                || literal.is_some_and(|ip| address.ip() != ip)
        })
    {
        return Err(ConnectError::DestinationRejected);
    }
    addresses
        .first()
        .copied()
        .ok_or(ConnectError::DestinationRejected)
}

impl ConnectExecutor {
    fn live(&self, intent: &ActionIntent, decision: &AuthorizationDecision) -> bool {
        let now = self.clock.now_ms();
        now < intent.action_deadline && decision.expiry.is_none_or(|deadline| now < deadline)
    }

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
            && self.live(intent, decision)
    }

    fn remaining_ms(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<u64, ConnectError> {
        let deadline = decision.expiry.map_or(intent.action_deadline, |expiry| {
            expiry.min(intent.action_deadline)
        });
        deadline
            .as_i64()
            .checked_sub(self.clock.now_ms().as_i64())
            .and_then(|remaining| u64::try_from(remaining).ok())
            .filter(|remaining| *remaining > 0)
            .map(|remaining| remaining.min(self.request.timeout_ms))
            .ok_or(ConnectError::DeadlineExpired)
    }

    async fn tls(&self) -> Result<TlsValidation, ConnectError> {
        #[cfg(test)]
        if let Some(certificate) = &self.ca_der {
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(rustls::pki_types::CertificateDer::from(certificate.clone()))
                .map_err(|_| ConnectError::TrustUnavailable)?;
            return tls_validation(roots, CertificateValidation::FixtureRoot);
        }
        // Root enumeration can block on the local keychain/filesystem. It cannot
        // contact a peer, and a timed-out waiter never proceeds to DNS or connect.
        tokio::task::spawn_blocking(system_tls_validation)
            .await
            .map_err(|_| ConnectError::TrustUnavailable)?
    }

    async fn peer(&self) -> Result<SocketAddr, ConnectError> {
        resolve_loopback_peer(&self.request, self.resolver.as_ref()).await
    }

    async fn connect(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
    ) -> Result<(SocketAddr, CertificateValidation), ConnectError> {
        let tls = if self.request.endpoint.scheme() == "https" {
            Some(self.tls().await?)
        } else {
            None
        };
        let peer = self.peer().await?;
        if !self.live(intent, decision) {
            return Err(ConnectError::DeadlineExpired);
        }
        // A sockaddr, never the hostname, is passed to TCP: no second resolution,
        // fallback address, proxy, redirect or automatic reconnect exists here.
        let socket = TcpStream::connect(peer)
            .await
            .map_err(|_| ConnectError::TransportUnavailable)?;
        if socket
            .peer_addr()
            .map_err(|_| ConnectError::TransportUnavailable)?
            != peer
        {
            return Err(ConnectError::TransportUnavailable);
        }
        if !self.live(intent, decision) {
            return Err(ConnectError::DeadlineExpired);
        }
        let source = if let Some(tls) = tls {
            let name = rustls::pki_types::ServerName::try_from(self.request.host.clone())
                .map_err(|_| ConnectError::TrustUnavailable)?;
            let mut stream = TlsConnector::from(tls.config)
                .connect(name, socket)
                .await
                .map_err(|_| ConnectError::TransportUnavailable)?;
            // Only TLS close_notify is written, never application data.
            stream
                .shutdown()
                .await
                .map_err(|_| ConnectError::TransportUnavailable)?;
            drop(stream);
            tls.source
        } else {
            drop(socket);
            CertificateValidation::NotTls
        };
        // Instrument the actual drop boundary only in the fixture. No product
        // authorization, durable proof or observation depends on this probe.
        #[cfg(test)]
        if let Some(probe) = &self.closed_probe {
            probe.store(true, Ordering::SeqCst);
        }
        if !self.live(intent, decision) {
            return Err(ConnectError::DeadlineExpired);
        }
        Ok((peer, source))
    }

    async fn observe(
        &self,
        dispatch_id: DispatchId,
        peer: SocketAddr,
        source: CertificateValidation,
    ) -> Result<ActionExecutionResult, ConnectError> {
        let observed_at_ms = self.clock.now_ms();
        let observation = ConnectObservation {
            schema_version: 1,
            action_id: self.action,
            dispatch_id,
            request_digest: self.digest,
            peer,
            certificate_validation: source,
            revocation_checked: false,
            http_application_bytes_sent: 0,
            observed_at_ms,
        };
        let digest = canonical_sha256(&observation).map_err(|_| ConnectError::Digest)?;
        // No execution result or staged success exists until the exact local
        // document and its dispatch binding have been durably acknowledged.
        self.observation_port.persist(&observation).await?;
        let mut stage = self
            .staged
            .lock()
            .map_err(|_| ConnectError::ObservationUnavailable)?;
        if stage.is_some() {
            return Err(ConnectError::ObservationUnavailable);
        }
        *stage = Some(Staged {
            action: self.action,
            dispatch: dispatch_id,
            digest,
            observation,
        });
        Ok(ActionExecutionResult {
            dispatch_id,
            provider_request_id: None,
            output_digest: digest,
            observed_at_ms,
        })
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
            if !self.bound(intent, decision, secrets) {
                return Err(ActionExecutionFailure::ConfirmedFailureNoEffect {
                    dispatch_id,
                    reason: "connect_binding_rejected".to_owned(),
                });
            }
            let unknown = || ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "connect_transport_outcome_unknown".to_owned(),
            };
            if self.consumed.swap(true, Ordering::SeqCst) {
                return Err(unknown());
            }
            let remaining = self.remaining_ms(intent, decision).map_err(|_| unknown())?;
            let result = tokio::time::timeout(Duration::from_millis(remaining), async {
                let (peer, source) = self.connect(intent, decision).await?;
                self.observe(dispatch_id, peer, source).await
            })
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            if !self.live(intent, decision) {
                return Err(unknown());
            }
            Ok(result)
        })
    }
}

/// Execute through the broker and a required durable local observation port.
/// The port is awaited after channel closure and before a successful result.
/// Real journaling and mandate/admission/fence proof remain caller obligations.
///
/// # Errors
/// Reports local digest/staging failures; broker failures remain in `receipt`.
pub async fn execute(
    request: FrozenConnectRequest,
    dispatch: BrokerDispatch<'_>,
    observation_port: Arc<dyn ConnectObservationPort>,
) -> Result<ConnectExecution, ConnectError> {
    execute_inner(
        request,
        dispatch,
        observation_port,
        #[cfg(test)]
        None,
    )
    .await
}

async fn execute_inner(
    request: FrozenConnectRequest,
    dispatch: BrokerDispatch<'_>,
    observation_port: Arc<dyn ConnectObservationPort>,
    #[cfg(test)] fixture: Option<FixtureOptions>,
) -> Result<ConnectExecution, ConnectError> {
    let digest = request.payload_digest()?;
    // The broker resolves references before invoking the executor. Reject them
    // here, using the actual intention on this same broker and ActionId. That
    // intention cannot be replaced by reevaluation; the full binding is checked
    // again by the broker and by ConnectExecutor::bound.
    if dispatch.broker.action_has_secret_refs(dispatch.action) != Some(false)
        || !dispatch
            .broker
            .action_audit(dispatch.action)
            .is_some_and(|audit| {
                audit.payload_digest == digest
                    && audit.decision_ref.as_ref() == Some(dispatch.decision)
            })
    {
        return Err(ConnectError::InvalidRequest);
    }
    let staged = Arc::new(Mutex::new(None));
    #[cfg(test)]
    let (resolver, ca_der, closed_probe) = fixture.map_or_else(
        || {
            (
                Arc::new(SystemResolver) as Arc<dyn NameResolver>,
                None,
                None,
            )
        },
        |options| (options.resolver, options.ca_der, options.closed_probe),
    );
    #[cfg(not(test))]
    let resolver = Arc::new(SystemResolver) as Arc<dyn NameResolver>;
    let executor = ConnectExecutor {
        request,
        action: dispatch.action,
        digest,
        clock: dispatch.clock,
        resolver,
        observation_port,
        consumed: AtomicBool::new(false),
        staged: Arc::clone(&staged),
        #[cfg(test)]
        ca_der,
        #[cfg(test)]
        closed_probe,
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
    let observation_port = Arc::clone(&fixture.observation_port);
    execute_inner(request, dispatch, observation_port, Some(fixture)).await
}
