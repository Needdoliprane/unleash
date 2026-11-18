//! Single-attempt Responses executor, callable through the existing broker port.
//!
//! The caller owns admission, durable intent/dispatch and result persistence.
//! This module does not provide an authorization, budget or retry mechanism.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionExecutionFailure, ActionExecutionResult,
    ActionExecutorPort, ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture,
    EffectiveAuthority, EffectsClass, ExecutorKind, GuardDecisionRef, NormalizedResourceRef,
    ResolvedSecret, SecretStore,
};
use unleash_domain::{
    ActionId, ActionState, ClockPort, DataLocality, DispatchId, LineageFence, SecretRef,
    Sha256Digest,
};
use unleash_protocol::canonical_sha256;
use zeroize::Zeroizing;

use crate::openai::{OpenAiOutcome, OpenAiStream, OpenAiUsage};

const MAX_INPUT: usize = 65_536;
const MAX_RESPONSE: usize = 1_048_576;
const MAX_MODEL: usize = 256;
const VERB: &str = "model.responses.stream";

/// Invalid immutable request or unavailable response staging.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The endpoint, model, or input is outside this executor's supported subset.
    #[error("invalid model request")]
    InvalidRequest,
    /// The request or result could not be canonically hashed.
    #[error("canonical model digest unavailable")]
    Digest,
    /// The supplied broker receipt does not match the staged result.
    #[error("broker receipt does not match model result")]
    ReceiptMismatch,
    /// Staging cannot be read safely.
    #[error("model result staging unavailable")]
    StagingUnavailable,
}

/// Immutable, bounded input for a single Responses call; no secret is stored here.
pub struct FrozenRequest {
    endpoint: reqwest::Url,
    model: String,
    input: String,
    profile: ProfileRef,
    secret_ref: SecretRef,
}

impl fmt::Debug for FrozenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenRequest").finish_non_exhaustive()
    }
}

/// Immutable identity supplied by the owner of the resolved provider profile.
pub struct ProfileRef {
    id: String,
    version: u64,
    digest: Sha256Digest,
}

impl fmt::Debug for ProfileRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileRef").finish_non_exhaustive()
    }
}

impl ProfileRef {
    /// Construct a versioned profile reference; validation of the durable record
    /// remains the caller's responsibility.
    ///
    /// # Errors
    /// Rejects an empty, oversized or unversioned profile reference.
    pub fn new(id: &str, version: u64, digest: Sha256Digest) -> Result<Self, TransportError> {
        if id.trim().is_empty() || id.len() > 256 || version == 0 {
            return Err(TransportError::InvalidRequest);
        }
        Ok(Self {
            id: id.to_owned(),
            version,
            digest,
        })
    }
}

impl FrozenRequest {
    /// Fix an HTTPS Responses endpoint, profile, input and opaque credential reference.
    ///
    /// # Errors
    /// Rejects credentials in URLs, query/fragment, unsupported paths and oversized input.
    pub fn new(
        endpoint: &str,
        model: &str,
        input: &str,
        profile: ProfileRef,
        secret_ref: SecretRef,
    ) -> Result<Self, TransportError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| TransportError::InvalidRequest)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/v1/responses"
            || url.query().is_some()
            || url.fragment().is_some()
            || model.trim().is_empty()
            || model.len() > MAX_MODEL
            || input.len() > MAX_INPUT
        {
            return Err(TransportError::InvalidRequest);
        }
        Ok(Self {
            endpoint: url,
            model: model.to_owned(),
            input: input.to_owned(),
            profile,
            secret_ref,
        })
    }

    fn body(&self) -> Value {
        json!({"model":self.model,"input":self.input,"stream":true,"store":false,"max_output_tokens":256})
    }

    fn manifest(&self) -> Value {
        json!({"schema":"unleash.responses_transport.v1","endpoint":self.endpoint.as_str(),"provider":"openai","profile_id":self.profile.id,"profile_version":self.profile.version,"profile_digest":self.profile.digest,"dialect":"responses","auth":"bearer","secret_ref_digest":Sha256Digest::from_bytes(self.secret_ref.as_opaque_str().as_bytes()),"connect_timeout_ms":2000,"read_timeout_ms":10000,"total_timeout_ms":60000,"max_response_bytes":MAX_RESPONSE,"body":self.body()})
    }

    /// Digest to bind into the broker's exact `ActionIntent` and decision.
    ///
    /// # Errors
    /// Returns `Digest` if canonical serialization fails.
    pub fn payload_digest(&self) -> Result<Sha256Digest, TransportError> {
        canonical_sha256(&self.manifest()).map_err(|_| TransportError::Digest)
    }
}

struct Staged {
    action: ActionId,
    dispatch: DispatchId,
    digest: Sha256Digest,
    outcome: OpenAiOutcome,
}

/// Bounded interpretation of a provider's `Retry-After` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfter {
    /// An integer delay no greater than 300 seconds.
    Seconds(u64),
    /// The provider requested more than 300 seconds.
    BeyondLimit,
    /// The field was absent, malformed, or uses an unsupported date format.
    Unavailable,
}

/// Attributed local rate-limit observation; it is not a durable receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimited {
    /// Action owning the dispatch.
    pub action_id: ActionId,
    /// Exact physical dispatch.
    pub dispatch_id: DispatchId,
    /// Bounded provider retry hint; no retry is performed here.
    pub retry_after: RetryAfter,
}

struct ResultHandle {
    staged: Arc<Mutex<Option<Staged>>>,
    rate_limit: Arc<Mutex<Option<RateLimited>>>,
}
impl ResultHandle {
    fn rate_limited(&self) -> Result<Option<RateLimited>, TransportError> {
        self.rate_limit
            .lock()
            .map(|value| *value)
            .map_err(|_| TransportError::StagingUnavailable)
    }
    fn take(&self, receipt: &ActionReceipt) -> Result<Option<OpenAiOutcome>, TransportError> {
        let mut slot = self
            .staged
            .lock()
            .map_err(|_| TransportError::StagingUnavailable)?;
        let Some(value) = slot.as_ref() else {
            return Err(TransportError::StagingUnavailable);
        };
        if receipt.state != ActionState::ObservedSuccess
            || receipt.action_id != value.action
            || receipt.dispatch_id != value.dispatch
            || receipt.output_digest != Some(value.digest)
        {
            return Err(TransportError::ReceiptMismatch);
        }
        Ok(slot.take().map(|value| value.outcome))
    }
}

struct ModelExecutor {
    request: FrozenRequest,
    action: ActionId,
    digest: Sha256Digest,
    clock: Arc<dyn ClockPort>,
    staged: Arc<Mutex<Option<Staged>>>,
    rate_limit: Arc<Mutex<Option<RateLimited>>>,
    consumed: AtomicBool,
    #[cfg(test)]
    fixture_ca: Option<reqwest::Certificate>,
}

/// Inputs to the existing broker dispatch; none independently grants authority.
pub struct BrokerDispatch<'a> {
    /// The broker that already owns the action and durable journal.
    pub broker: &'a ActionBroker,
    /// Exact authorized action.
    pub action: ActionId,
    /// Broker-issued decision reference.
    pub decision: &'a GuardDecisionRef,
    /// Current lineage fence revalidated by the broker.
    pub fence: LineageFence,
    /// Effective authority revalidated by the broker.
    pub authority: &'a EffectiveAuthority,
    /// Clock supplied by the runtime.
    pub clock: Arc<dyn ClockPort>,
    /// Secret store used only by the broker after authorization.
    pub secrets: &'a dyn SecretStore,
}

impl fmt::Debug for BrokerDispatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerDispatch")
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

/// Broker result and explicitly separate non-durable transport observation.
pub struct BrokerExecution {
    /// Actual broker response; an error may indicate an uncertain durable observation.
    pub receipt: Result<ActionReceipt, ActionBrokerError>,
    /// Model output, released only with a matching observed-success broker receipt.
    pub outcome: Result<Option<OpenAiOutcome>, TransportError>,
    /// Attributed local 429, if observed; it is not a durable receipt or retry grant.
    pub rate_limited: Result<Option<RateLimited>, TransportError>,
}

impl fmt::Debug for BrokerExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerExecution").finish_non_exhaustive()
    }
}

/// Run one request exclusively through `ActionBroker::execute_authorized`.
/// The caller must provide the runtime broker backed by its durable journal;
/// the broker owns dispatch and authorization checks. Test-memory brokers do
/// not prove this production precondition.
///
/// # Errors
/// Returns a local staging or digest error. Broker failures remain in `receipt`.
///
/// # Cancellation safety
/// Once broker dispatch is committed, cancellation leaves reconciliation to the broker.
pub async fn execute(
    request: FrozenRequest,
    dispatch: BrokerDispatch<'_>,
) -> Result<BrokerExecution, TransportError> {
    execute_inner(
        request,
        dispatch,
        #[cfg(test)]
        None,
    )
    .await
}

async fn execute_inner(
    request: FrozenRequest,
    dispatch: BrokerDispatch<'_>,
    #[cfg(test)] fixture_ca: Option<reqwest::Certificate>,
) -> Result<BrokerExecution, TransportError> {
    let (executor, handle) = build_executor(
        request,
        dispatch.action,
        dispatch.clock,
        #[cfg(test)]
        fixture_ca,
    )?;
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
    let rate_limited = handle.rate_limited();
    let outcome = match &receipt {
        Ok(receipt) => handle.take(receipt),
        Err(_) => Ok(None),
    };
    Ok(BrokerExecution {
        receipt,
        outcome,
        rate_limited,
    })
}

fn build_executor(
    request: FrozenRequest,
    action: ActionId,
    clock: Arc<dyn ClockPort>,
    #[cfg(test)] fixture_ca: Option<reqwest::Certificate>,
) -> Result<(ModelExecutor, ResultHandle), TransportError> {
    let digest = request.payload_digest()?;
    let staged = Arc::new(Mutex::new(None));
    let rate_limit = Arc::new(Mutex::new(None));
    Ok((
        ModelExecutor {
            request,
            action,
            digest,
            clock,
            staged: Arc::clone(&staged),
            rate_limit: Arc::clone(&rate_limit),
            consumed: AtomicBool::new(false),
            #[cfg(test)]
            fixture_ca,
        },
        ResultHandle { staged, rate_limit },
    ))
}

#[cfg(test)]
pub(crate) async fn execute_fixture(
    request: FrozenRequest,
    dispatch: BrokerDispatch<'_>,
    ca: Option<reqwest::Certificate>,
) -> Result<BrokerExecution, TransportError> {
    execute_inner(request, dispatch, ca).await
}

pub(crate) fn output_value(outcome: &OpenAiOutcome) -> Value {
    fn usage(value: Option<&OpenAiUsage>) -> Value {
        value.map_or(Value::Null, |u| json!({"input":u.input,"output":u.output,"total":u.total,"cache":u.cache,"cache_write":u.cache_write,"reasoning":u.reasoning}))
    }
    match outcome {
        OpenAiOutcome::Success {
            text,
            tools,
            usage: u,
        } => {
            json!({"kind":"complete","text":text,"tools":tools.iter().map(|t| json!({"id":t.id,"name":t.name,"arguments":t.arguments})).collect::<Vec<_>>(),"usage":usage(u.as_ref())})
        }
        OpenAiOutcome::Refusal { reason, usage: u } => {
            json!({"kind":"refusal","reason":reason,"usage":usage(u.as_ref())})
        }
        OpenAiOutcome::Truncated { usage: u } => {
            json!({"kind":"truncated","usage":usage(u.as_ref())})
        }
        OpenAiOutcome::InvalidSchema { usage: u } => {
            json!({"kind":"invalid_contract","usage":usage(u.as_ref())})
        }
    }
}

impl ModelExecutor {
    fn matches_intent(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> bool {
        let Some(host) = self.request.endpoint.host_str() else {
            return false;
        };
        let port = self.request.endpoint.port_or_known_default();
        let targets = intent.resource_refs.iter().filter(|r| matches!(r, NormalizedResourceRef::ModelTarget { provider, model_id, locality: DataLocality::ApprovedRemote } if provider == "openai" && model_id == &self.request.model)).count();
        let endpoints = intent.resource_refs.iter().filter(|r| matches!(r, NormalizedResourceRef::NetworkEndpoint { host: name, port: p, .. } if name == host && *p == port)).count();
        intent.action_id == self.action
            && decision.action_id == self.action
            && intent.payload_digest == self.digest
            && decision.payload_digest == self.digest
            && intent.scope_revision == decision.scope_revision
            && intent.policy_revision == decision.policy_revision
            && intent.effects_class == EffectsClass::ModelDispatch
            && intent.verb == VERB
            && decision.executor == ExecutorKind::Unleash
            && targets == 1
            && endpoints == 1
            && intent.resource_refs.len() == 2
            && secrets.len() == 1
            && intent.secret_refs.as_slice() == std::slice::from_ref(&self.request.secret_ref)
            && self.clock.now_ms() < intent.action_deadline
            && decision
                .expiry
                .is_none_or(|deadline| self.clock.now_ms() < deadline)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Linear validation, one dispatch and outcome staging preserve the effect boundary"
    )]
    async fn run(
        &self,
        dispatch_id: DispatchId,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> Result<ActionExecutionResult, ActionExecutionFailure> {
        let denied = || ActionExecutionFailure::ConfirmedFailureNoEffect {
            dispatch_id,
            reason: "model_request_binding_rejected".into(),
        };
        let unknown = || ActionExecutionFailure::OutcomeUnknown {
            dispatch_id,
            reason: "model_transport_outcome_unknown".into(),
        };
        if !self.matches_intent(intent, decision, secrets) {
            return Err(denied());
        }
        if self.consumed.swap(true, Ordering::SeqCst) {
            return Err(unknown());
        }
        let mut authorization = None;
        if let Some(secret) = secrets.first() {
            if secret.expose_for_transport().is_empty() {
                return Err(denied());
            }
            let value = Zeroizing::new(format!("Bearer {}", secret.expose_for_transport()));
            let mut header =
                reqwest::header::HeaderValue::from_str(value.as_str()).map_err(|_| denied())?;
            header.set_sensitive(true);
            authorization = Some(header);
        }
        let now = self.clock.now_ms().as_i64();
        let deadline = decision
            .expiry
            .map_or(intent.action_deadline.as_i64(), |expiry| {
                expiry.as_i64().min(intent.action_deadline.as_i64())
            });
        let remaining = deadline
            .checked_sub(now)
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(denied)?;
        let builder = reqwest::Client::builder()
            .tls_backend_rustls()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(2))
            .read_timeout(Duration::from_secs(10))
            .timeout(Duration::from_millis(remaining.min(60_000)));
        #[cfg(test)]
        let builder = if let Some(ca) = &self.fixture_ca {
            builder.tls_certs_only([ca.clone()])
        } else {
            builder
        };
        let client = builder.build().map_err(|_| denied())?;
        let mut request = client
            .post(self.request.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(self.request.body().to_string());
        if let Some(header) = authorization {
            request = request.header(reqwest::header::AUTHORIZATION, header);
        }
        let mut response = request.send().await.map_err(|_| unknown())?;
        if response.status().as_u16() == 429 {
            let retry_after = match response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
            {
                Some(value) if value <= 300 => RetryAfter::Seconds(value),
                Some(_) => RetryAfter::BeyondLimit,
                None => RetryAfter::Unavailable,
            };
            *self.rate_limit.lock().map_err(|_| unknown())? = Some(RateLimited {
                action_id: self.action,
                dispatch_id,
                retry_after,
            });
            // The current broker executor port cannot carry a typed rate-limit
            // result; retain uncertainty rather than report ContractViolation.
            return Err(ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "provider_rate_limited_unmapped".into(),
            });
        }
        if !response.status().is_success() {
            return Err(unknown());
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"));
        if !content_type
            || response
                .content_length()
                .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            return Err(unknown());
        }
        let provider_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 256
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
                    && !secrets
                        .iter()
                        .any(|secret| v.contains(secret.expose_for_transport()))
            })
            .map(str::to_owned);
        let mut parser = OpenAiStream::new(BTreeMap::new()).map_err(|_| unknown())?;
        let mut size = 0_usize;
        while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
            size = size.saturating_add(chunk.len());
            if size > MAX_RESPONSE {
                return Err(unknown());
            }
            parser.push(&chunk).map_err(|_| unknown())?;
        }
        let outcome = parser
            .finish()
            .map_err(|_| unknown())?
            .ok_or_else(unknown)?;
        let digest = canonical_sha256(&output_value(&outcome)).map_err(|_| unknown())?;
        let mut stage = self.staged.lock().map_err(|_| unknown())?;
        if stage.is_some() {
            return Err(unknown());
        }
        *stage = Some(Staged {
            action: self.action,
            dispatch: dispatch_id,
            digest,
            outcome,
        });
        Ok(ActionExecutionResult {
            dispatch_id,
            provider_request_id,
            output_digest: digest,
            observed_at_ms: self.clock.now_ms(),
        })
    }
}

impl ActionExecutorPort for ModelExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        resolved_secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(self.run(dispatch_id, intent, decision, resolved_secrets))
    }
}
