//! One Messages attempt through the caller's broker, never an admission or retry authority.
//! Results require the matching broker receipt. Usage/rate-limit hints are local observations,
//! not durable billing or permission to repeat a request.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture, EffectsClass, ExecutorKind,
    NormalizedResourceRef, ResolvedSecret,
};
use unleash_domain::{
    ActionId, ActionState, AuthorizationEvaluationId, ClockPort, DataLocality, DispatchId,
    SecretRef, Sha256Digest,
};
use unleash_protocol::canonical_sha256;
use zeroize::Zeroizing;

use crate::anthropic::{AnthropicOutcome, AnthropicStream, AnthropicUsage};
use crate::transport::{
    BrokerDispatch, ProfileRef, RateLimited, RetryAfter, TransportError, https_client_builder,
};

/// Distinct authority binding; a Responses authorization does not grant Messages.
pub const MESSAGES_VERB: &str = "model.messages.stream";
/// Explicit version of the supported native Messages request dialect.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_RESPONSE: usize = 1_048_576;

/// Immutable, bounded Messages request. It stores only an opaque credential reference.
pub struct MessagesRequest {
    endpoint: reqwest::Url,
    model: String,
    input: String,
    profile: ProfileRef,
    secret_ref: SecretRef,
}
impl fmt::Debug for MessagesRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessagesRequest").finish_non_exhaustive()
    }
}
impl MessagesRequest {
    /// Pin an HTTPS `/v1/messages` endpoint, native profile and expected credential.
    ///
    /// # Errors
    /// Rejects URL credentials, query/fragment, another path and excessive input/model size.
    pub fn new(
        endpoint: &str,
        model: &str,
        input: &str,
        profile: ProfileRef,
        secret_ref: SecretRef,
    ) -> Result<Self, TransportError> {
        let endpoint = reqwest::Url::parse(endpoint).map_err(|_| TransportError::InvalidRequest)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != "/v1/messages"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || model.trim().is_empty()
            || model.len() > 256
            || input.len() > 65_536
        {
            return Err(TransportError::InvalidRequest);
        }
        Ok(Self {
            endpoint,
            model: model.into(),
            input: input.into(),
            profile,
            secret_ref,
        })
    }
    fn body(&self) -> Value {
        json!({"model":self.model,"messages":[{"role":"user","content":self.input}],"max_tokens":256,"stream":true})
    }
    fn manifest(&self) -> Value {
        json!({"schema":"unleash.messages_transport.v1","provider":"anthropic","dialect":"messages","anthropic_version":ANTHROPIC_VERSION,"verb":MESSAGES_VERB,"endpoint":self.endpoint.as_str(),"profile_id":self.profile.id,"profile_version":self.profile.version,"profile_digest":self.profile.digest,"secret_ref_digest":Sha256Digest::from_bytes(self.secret_ref.as_opaque_str().as_bytes()),"auth":"x-api-key","connect_timeout_ms":2000,"read_timeout_ms":10000,"total_timeout_ms":60000,"max_response_bytes":MAX_RESPONSE,"body":self.body()})
    }
    /// Exact native manifest digest to authorize; it contains no resolved credential.
    ///
    /// # Errors
    /// Returns `Digest` if canonical serialization fails.
    pub fn payload_digest(&self) -> Result<Sha256Digest, TransportError> {
        canonical_sha256(&self.manifest()).map_err(|_| TransportError::Digest)
    }
}

/// Last entirely valid usage event, attributed to the actual dispatch; not final billing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageObservation {
    /// Owning action.
    pub action_id: ActionId,
    /// Physical dispatch.
    pub dispatch_id: DispatchId,
    /// Cumulative counters; missing values remain unknown.
    pub usage: AnthropicUsage,
}

/// Broker result and separate non-durable transport observations.
pub struct MessagesExecution {
    /// Actual broker result, including an uncertain observation commit.
    pub receipt: Result<ActionReceipt, ActionBrokerError>,
    /// Native result released only after the corresponding successful receipt.
    pub outcome: Result<Option<AnthropicOutcome>, TransportError>,
    /// Last valid usage, possibly partial even when the stream/receipt failed.
    pub observed_usage: Result<Option<UsageObservation>, TransportError>,
    /// Local 429 observation, not a durable retry grant.
    pub rate_limited: Result<Option<RateLimited>, TransportError>,
}
impl fmt::Debug for MessagesExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessagesExecution").finish_non_exhaustive()
    }
}

struct Staged {
    action: ActionId,
    evaluation: AuthorizationEvaluationId,
    dispatch: DispatchId,
    decision: String,
    digest: Sha256Digest,
    outcome: AnthropicOutcome,
}
#[derive(Default)]
struct Observations {
    staged: Option<Staged>,
    usage: Option<UsageObservation>,
    rate_limit: Option<RateLimited>,
}
impl Observations {
    fn take(
        &mut self,
        receipt: &ActionReceipt,
    ) -> Result<Option<AnthropicOutcome>, TransportError> {
        let value = self
            .staged
            .as_ref()
            .ok_or(TransportError::StagingUnavailable)?;
        if receipt.state != ActionState::ObservedSuccess
            || receipt.action_id != value.action
            || receipt.evaluation_id != value.evaluation
            || receipt.dispatch_id != value.dispatch
            || receipt.decision_ref.as_str() != value.decision
            || receipt.output_digest != Some(value.digest)
        {
            return Err(TransportError::ReceiptMismatch);
        }
        Ok(self.staged.take().map(|value| value.outcome))
    }
}

struct MessagesExecutor {
    request: MessagesRequest,
    action: ActionId,
    digest: Sha256Digest,
    clock: Arc<dyn ClockPort>,
    observations: Arc<Mutex<Observations>>,
    consumed: AtomicBool,
    #[cfg(test)]
    fixture_ca: Option<reqwest::Certificate>,
}

/// Execute one attempt through the existing broker and its journal.
/// The caller owns durable admission, profile validation and persistence; a fixture
/// journal cannot establish those production preconditions.
///
/// # Errors
/// Returns a local digest error; broker failures are kept in `receipt`.
///
/// # Cancellation safety
/// Cancellation after dispatch requires reconciliation, never another automatic POST.
pub async fn execute_messages(
    request: MessagesRequest,
    dispatch: BrokerDispatch<'_>,
) -> Result<MessagesExecution, TransportError> {
    execute_inner(
        request,
        dispatch,
        #[cfg(test)]
        None,
    )
    .await
}

async fn execute_inner(
    request: MessagesRequest,
    dispatch: BrokerDispatch<'_>,
    #[cfg(test)] fixture_ca: Option<reqwest::Certificate>,
) -> Result<MessagesExecution, TransportError> {
    let observations = Arc::new(Mutex::new(Observations::default()));
    let executor = MessagesExecutor {
        digest: request.payload_digest()?,
        request,
        action: dispatch.action,
        clock: dispatch.clock,
        observations: Arc::clone(&observations),
        consumed: AtomicBool::new(false),
        #[cfg(test)]
        fixture_ca,
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
    let mut observations = observations
        .lock()
        .map_err(|_| TransportError::StagingUnavailable)?;
    let outcome = match &receipt {
        Ok(receipt) => observations.take(receipt),
        Err(_) => Ok(None),
    };
    Ok(MessagesExecution {
        receipt,
        outcome,
        observed_usage: Ok(observations.usage.clone()),
        rate_limited: Ok(observations.rate_limit),
    })
}

#[cfg(test)]
pub(crate) async fn execute_fixture(
    request: MessagesRequest,
    dispatch: BrokerDispatch<'_>,
    ca: Option<reqwest::Certificate>,
) -> Result<MessagesExecution, TransportError> {
    execute_inner(request, dispatch, ca).await
}

pub(crate) fn output_value(outcome: &AnthropicOutcome) -> Value {
    let usage = |u: &AnthropicUsage| json!({"input":u.input,"output":u.output,"cache_read":u.cache_read,"cache_creation":u.cache_creation});
    match outcome {
        AnthropicOutcome::Success {
            text,
            tools,
            stop,
            usage: u,
        } => {
            json!({"kind":"complete","text":text,"tools":tools.iter().map(|t|json!({"id":t.id,"name":t.name,"arguments":t.arguments})).collect::<Vec<_>>(),"stop":match stop {crate::anthropic::StopReason::EndTurn=>json!({"kind":"end_turn"}),crate::anthropic::StopReason::StopSequence(value)=>json!({"kind":"stop_sequence","sequence":value}),crate::anthropic::StopReason::ToolUse=>json!({"kind":"tool_use"})},"usage":usage(u)})
        }
        AnthropicOutcome::Refusal {
            explanation,
            usage: u,
        } => json!({"kind":"refusal","explanation":explanation,"usage":usage(u)}),
        AnthropicOutcome::Truncated { reason, usage: u } => {
            json!({"kind":"truncated","reason":reason,"usage":usage(u)})
        }
        AnthropicOutcome::InvalidSchema { usage: u } => {
            json!({"kind":"invalid_contract","usage":usage(u)})
        }
    }
}

impl MessagesExecutor {
    fn matches(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> bool {
        let Some(host) = self.request.endpoint.host_str() else {
            return false;
        };
        let port = self.request.endpoint.port_or_known_default();
        intent.action_id == self.action && decision.action_id == self.action
            && intent.payload_digest == self.digest && decision.payload_digest == self.digest
            && intent.scope_revision == decision.scope_revision && intent.policy_revision == decision.policy_revision
            && intent.effects_class == EffectsClass::ModelDispatch && intent.verb == MESSAGES_VERB
            && decision.executor == ExecutorKind::Unleash && intent.resource_refs.len() == 2
            && intent.resource_refs.iter().filter(|r| matches!(r, NormalizedResourceRef::ModelTarget {provider,model_id,locality:DataLocality::ApprovedRemote} if provider=="anthropic" && model_id==&self.request.model)).count()==1
            && intent.resource_refs.iter().filter(|r| matches!(r,NormalizedResourceRef::NetworkEndpoint {host:name,port:p,..} if name==host && *p==port)).count()==1
            && secrets.len()==1 && intent.secret_refs.as_slice()==std::slice::from_ref(&self.request.secret_ref)
            && self.clock.now_ms()<intent.action_deadline && decision.expiry.is_none_or(|expiry| self.clock.now_ms()<expiry)
    }
    #[expect(
        clippy::too_many_lines,
        reason = "Linear single-dispatch protocol and staging keep the effect boundary auditable"
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
            reason: "messages_request_binding_rejected".into(),
        };
        let unknown = || ActionExecutionFailure::OutcomeUnknown {
            dispatch_id,
            reason: "messages_transport_outcome_unknown".into(),
        };
        if !self.matches(intent, decision, secrets) {
            return Err(denied());
        }
        if self.consumed.swap(true, Ordering::SeqCst) {
            return Err(unknown());
        }
        let secret = secrets.first().ok_or_else(denied)?;
        if secret.expose_for_transport().is_empty() {
            return Err(denied());
        }
        let value = Zeroizing::new(secret.expose_for_transport().to_owned());
        let mut key =
            reqwest::header::HeaderValue::from_str(value.as_str()).map_err(|_| denied())?;
        key.set_sensitive(true);
        let deadline = decision
            .expiry
            .map_or(intent.action_deadline.as_i64(), |expiry| {
                expiry.as_i64().min(intent.action_deadline.as_i64())
            });
        let remaining = deadline
            .checked_sub(self.clock.now_ms().as_i64())
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(denied)?;
        let builder = https_client_builder(remaining);
        #[cfg(test)]
        let builder = if let Some(ca) = &self.fixture_ca {
            builder.tls_certs_only([ca.clone()])
        } else {
            builder
        };
        let client = builder.build().map_err(|_| denied())?;
        let mut response = client
            .post(self.request.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("x-api-key", key)
            .body(self.request.body().to_string())
            .send()
            .await
            .map_err(|_| unknown())?;
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
            self.observations.lock().map_err(|_| unknown())?.rate_limit = Some(RateLimited {
                action_id: self.action,
                dispatch_id,
                retry_after,
            });
            return Err(ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "messages_provider_rate_limited_unmapped".into(),
            });
        }
        if !response.status().is_success() {
            return Err(unknown());
        }
        let sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
        if !sse
            || response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE as u64)
        {
            return Err(unknown());
        }
        let provider_request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 256
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
                    && !value.contains(secret.expose_for_transport())
            })
            .map(str::to_owned);
        let mut parser = AnthropicStream::new(BTreeMap::new()).map_err(|_| unknown())?;
        let mut size = 0_usize;
        while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
            size = size.saturating_add(chunk.len());
            if size > MAX_RESPONSE {
                return Err(unknown());
            }
            let accepted = parser.push(&chunk);
            // Even when a later event in this chunk is rejected, the parser exposes
            // only the last entirely valid usage event, never an invalid partial update.
            if let Some(usage) = parser.observed_usage() {
                self.observations.lock().map_err(|_| unknown())?.usage = Some(UsageObservation {
                    action_id: self.action,
                    dispatch_id,
                    usage,
                });
            }
            accepted.map_err(|_| unknown())?;
        }
        let outcome = parser
            .finish()
            .map_err(|_| unknown())?
            .ok_or_else(unknown)?;
        let digest = canonical_sha256(&output_value(&outcome)).map_err(|_| unknown())?;
        let mut observations = self.observations.lock().map_err(|_| unknown())?;
        if observations.staged.is_some() {
            return Err(unknown());
        }
        observations.staged = Some(Staged {
            action: self.action,
            evaluation: decision.evaluation_id,
            dispatch: dispatch_id,
            decision: decision.decision_ref.as_str().to_owned(),
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
impl ActionExecutorPort for MessagesExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(self.run(dispatch_id, intent, decision, secrets))
    }
}
