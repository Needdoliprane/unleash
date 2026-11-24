//! Single authorized native Ollama Chat exchange. The caller retains ownership
//! of admission, placement evidence and final resource disposition.

use crate::MAX_TOOLS;
use crate::connect_transport::{
    FrozenConnectRequest, OwnedLoopbackPeer, SystemResolver, resolve_loopback_peer,
};
use crate::inventory_transport::http::read_ndjson_body;
use crate::ollama::{OllamaOutcome, OllamaStream};
use crate::transport::{BrokerDispatch, ProfileRef};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time::Instant;
use unleash_action_broker::{
    ActionBrokerError, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort,
    ActionIntent, ActionReceipt, AuthorizationDecision, BoxFuture, EffectsClass, ExecutorKind,
    NormalizedResourceRef, ResolvedSecret,
};
use unleash_domain::{
    ActionId, ActionState, ClockPort, DataLocality, DispatchId, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    canonical_sha256, canonicalize_to_vec, parse_json_value_strict,
    provider_probe::{
        OLLAMA_CHAT_MAX_RESPONSE_BYTES, OllamaChatPlacement, OllamaChatRequestBinding,
        admitted_ollama_chat_digest, ollama_chat_body,
    },
};

const MAX_BODY: usize = 65_536;

/// Broker verb for a native chat request.
pub const OLLAMA_CHAT_VERB: &str = "model.ollama.chat";

/// Caller-provided limits for one generation probe.
#[derive(Debug, Clone, Copy)]
pub struct OllamaChatLimits {
    /// Maximum connection time in milliseconds.
    pub connect_ms: u64,
    /// Maximum exchange and observation time in milliseconds.
    pub total_ms: u64,
    /// Maximum generated tokens.
    pub output_tokens: u64,
}

/// Failure detected before a usable broker result was obtained.
#[derive(Debug, thiserror::Error)]
pub enum OllamaChatError {
    /// Invalid request or action binding.
    #[error("invalid Ollama Chat request")]
    InvalidRequest,
    /// Unable to build the canonical identity.
    #[error("Ollama Chat digest unavailable")]
    DigestUnavailable,
    /// The result could not be obtained.
    #[error("Ollama Chat result unavailable")]
    Unavailable,
}

/// Immutable native chat request bound to an expected loopback process.
pub struct FrozenOllamaChatRequest {
    target: FrozenConnectRequest,
    model: String,
    body: Vec<u8>,
    connect_ms: u64,
    digest: Sha256Digest,
    placement_expiry: Option<TimestampMs>,
    decoder: Mutex<Option<OllamaStream>>,
}

impl fmt::Debug for FrozenOllamaChatRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrozenOllamaChatRequest")
            .finish_non_exhaustive()
    }
}

impl FrozenOllamaChatRequest {
    /// Freeze a request for the placement selected by a durable admission.
    /// The caller obtains the peer from the managed process owner and the
    /// placement from the Store. The binding does not grant a dispatch permit.
    /// # Errors
    /// Rejects incompatible request, endpoint or process inputs.
    pub fn from_admitted_body(
        placement: &OllamaChatPlacement,
        profile: ProfileRef,
        peer: OwnedLoopbackPeer,
        model: &str,
        limits: OllamaChatLimits,
        body: Vec<u8>,
        expected_body_hash: Sha256Digest,
    ) -> Result<Self, OllamaChatError> {
        let native = peer.process_identity();
        if placement.process_id != native.pid
            || placement.boot_identity != native.boot_identity.as_str()
            || placement.process_start_identity != native.start_identity.as_str()
        {
            return Err(OllamaChatError::InvalidRequest);
        }
        let mut request = Self::from_published_body(
            &placement.endpoint,
            profile,
            peer,
            model,
            limits,
            body,
            expected_body_hash,
        )?;
        request.digest = admitted_ollama_chat_digest(request.digest, placement)
            .map_err(|_| OllamaChatError::DigestUnavailable)?;
        request.placement_expiry = Some(placement.valid_until_ms);
        Ok(request)
    }

    /// Use a canonical body previously published by the Store for a text probe.
    /// The caller supplies the expected hash from the durable probe context.
    ///
    /// # Errors
    /// Rejects unsupported input or a body divergent from its frozen identity.
    pub fn from_published_body(
        endpoint: &str,
        profile: ProfileRef,
        peer: OwnedLoopbackPeer,
        model: &str,
        limits: OllamaChatLimits,
        body: Vec<u8>,
        expected_body_hash: Sha256Digest,
    ) -> Result<Self, OllamaChatError> {
        if body.is_empty()
            || body.len() > MAX_BODY
            || Sha256Digest::from_bytes(&body) != expected_body_hash
        {
            return Err(OllamaChatError::InvalidRequest);
        }
        let encoded = std::str::from_utf8(&body).map_err(|_| OllamaChatError::InvalidRequest)?;
        let document =
            parse_json_value_strict(encoded).map_err(|_| OllamaChatError::InvalidRequest)?;
        let prompt = document
            .pointer("/messages/0/content")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .ok_or(OllamaChatError::InvalidRequest)?;
        let contracts = BTreeMap::new();
        let canonical = canonicalize_to_vec(&ollama_chat_body(
            model,
            prompt,
            limits.output_tokens,
            &contracts,
        ))
        .map_err(|_| OllamaChatError::DigestUnavailable)?;
        if canonical != body {
            return Err(OllamaChatError::InvalidRequest);
        }
        Self::freeze(endpoint, profile, peer, model, limits, body, contracts)
    }

    /// Freeze one local native chat request.
    ///
    /// # Errors
    /// Rejects invalid endpoints, bounds and input.
    pub fn new(
        endpoint: &str,
        profile: ProfileRef,
        peer: OwnedLoopbackPeer,
        model: &str,
        prompt: &str,
        limits: OllamaChatLimits,
        contracts: BTreeMap<String, Value>,
    ) -> Result<Self, OllamaChatError> {
        if contracts.len() > MAX_TOOLS || prompt.trim().is_empty() || prompt.len() > MAX_BODY {
            return Err(OllamaChatError::InvalidRequest);
        }
        let body = canonicalize_to_vec(&ollama_chat_body(
            model,
            prompt,
            limits.output_tokens,
            &contracts,
        ))
        .map_err(|_| OllamaChatError::DigestUnavailable)?;
        Self::freeze(endpoint, profile, peer, model, limits, body, contracts)
    }

    fn freeze(
        endpoint: &str,
        profile: ProfileRef,
        peer: OwnedLoopbackPeer,
        model: &str,
        limits: OllamaChatLimits,
        body: Vec<u8>,
        contracts: BTreeMap<String, Value>,
    ) -> Result<Self, OllamaChatError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| OllamaChatError::InvalidRequest)?;
        if url.scheme() != "http"
            || url.path() != "/api/chat"
            || !(1..=512).contains(&limits.output_tokens)
            || !(1..=limits.total_ms).contains(&limits.connect_ms)
            || body.len() > MAX_BODY
        {
            return Err(OllamaChatError::InvalidRequest);
        }
        let request_digest = OllamaChatRequestBinding {
            endpoint: url.as_str(),
            profile_id: &profile.id,
            profile_version: profile.version,
            profile_digest: profile.digest,
            connect_ms: limits.connect_ms,
            total_ms: limits.total_ms,
            body_digest: Sha256Digest::from_bytes(&body),
        }
        .digest()
        .map_err(|_| OllamaChatError::DigestUnavailable)?;
        let digest = peer
            .bind_digest(request_digest)
            .map_err(|_| OllamaChatError::DigestUnavailable)?;
        let target = FrozenConnectRequest::new_loopback(endpoint, profile, limits.total_ms)
            .and_then(|target| target.with_owned_peer(peer))
            .map_err(|_| OllamaChatError::InvalidRequest)?;
        let decoder =
            OllamaStream::new(model, contracts).map_err(|_| OllamaChatError::InvalidRequest)?;
        Ok(Self {
            target,
            model: model.to_owned(),
            body,
            connect_ms: limits.connect_ms,
            digest,
            placement_expiry: None,
            decoder: Mutex::new(Some(decoder)),
        })
    }

    /// Canonical digest for the broker intent.
    pub const fn payload_digest(&self) -> Sha256Digest {
        self.digest
    }

    /// Model and endpoint resources for the broker intent.
    pub fn resources(&self) -> Vec<NormalizedResourceRef> {
        vec![
            NormalizedResourceRef::ModelTarget {
                provider: "ollama".into(),
                model_id: self.model.clone(),
                locality: DataLocality::LocalOnly,
            },
            self.target.resource(),
        ]
    }
}

/// Exact attribution of a completed HTTP body.
#[derive(Debug, Clone)]
pub struct OllamaChatObservation {
    /// Action owning the exchange.
    pub action_id: ActionId,
    /// Physical dispatch.
    pub dispatch_id: DispatchId,
    /// Digest of the authorized request.
    pub request_digest: Sha256Digest,
    /// Digest of the bounded HTTP body.
    pub response_digest: Sha256Digest,
    /// Canonical digest of the observed outcome.
    pub outcome_digest: Sha256Digest,
    /// Local time at which the response was observed.
    pub observed_at_ms: TimestampMs,
}

/// Adopts the observation and outcome before broker success is published.
pub trait OllamaChatObservationPort: Send + Sync {
    /// Acknowledge exact adoption or preserve an unknown outcome.
    fn persist<'a>(
        &'a self,
        observation: &'a OllamaChatObservation,
        outcome: &'a OllamaOutcome,
    ) -> BoxFuture<'a, Result<(), OllamaChatError>>;
}

/// Output admitted for release by the exact broker receipt.
pub struct OllamaChatResult {
    /// Bound observation.
    pub observation: OllamaChatObservation,
    /// Decoded native result.
    pub outcome: OllamaOutcome,
}

/// Broker receipt and optionally released output.
pub struct OllamaChatExecution {
    /// Broker response to this attempt.
    pub receipt: Result<ActionReceipt, ActionBrokerError>,
    /// Output after successful adoption and receipt verification.
    pub result: Option<OllamaChatResult>,
}

struct Executor<'a> {
    request: FrozenOllamaChatRequest,
    action: ActionId,
    clock: Arc<dyn ClockPort>,
    port: &'a dyn OllamaChatObservationPort,
    stage: Mutex<Option<OllamaChatResult>>,
}

impl Executor<'_> {
    fn expiry(&self, intent: &ActionIntent, decision: &AuthorizationDecision) -> TimestampMs {
        let action_expiry = decision.expiry.map_or(intent.action_deadline, |expiry| {
            expiry.min(intent.action_deadline)
        });
        self.request
            .placement_expiry
            .map_or(action_expiry, |expiry| expiry.min(action_expiry))
    }

    fn live(&self, intent: &ActionIntent, decision: &AuthorizationDecision) -> bool {
        self.clock.now_ms() < self.expiry(intent, decision)
    }

    fn remaining_ms(&self, intent: &ActionIntent, decision: &AuthorizationDecision) -> Option<u64> {
        self.expiry(intent, decision)
            .as_i64()
            .checked_sub(self.clock.now_ms().as_i64())
            .and_then(|remaining| u64::try_from(remaining).ok())
            .filter(|remaining| *remaining > 0)
            .map(|remaining| remaining.min(self.request.target.timeout_ms()))
    }

    fn bound(
        &self,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        secrets: &[ResolvedSecret],
    ) -> bool {
        intent.action_id == self.action
            && decision.action_id == self.action
            && intent.verb == OLLAMA_CHAT_VERB
            && intent.effects_class == EffectsClass::ModelDispatch
            && intent.payload_digest == self.request.digest
            && decision.payload_digest == self.request.digest
            && intent.scope_revision == decision.scope_revision
            && intent.policy_revision == decision.policy_revision
            && decision.executor == ExecutorKind::Unleash
            && intent.resource_refs == self.request.resources()
            && intent.secret_refs.is_empty()
            && secrets.is_empty()
            && self.live(intent, decision)
    }

    async fn exchange(
        &self,
        dispatch_id: DispatchId,
        intent: &ActionIntent,
        decision: &AuthorizationDecision,
        deadline: Instant,
        mut decoder: OllamaStream,
    ) -> Result<OllamaChatResult, OllamaChatError> {
        let live = || self.live(intent, decision) && Instant::now() < deadline;
        let connect_deadline =
            deadline.min(Instant::now() + Duration::from_millis(self.request.connect_ms));
        let target = &self.request.target;
        let mut channel = tokio::time::timeout_at(connect_deadline, async {
            let peer = resolve_loopback_peer(target, &SystemResolver).await?;
            target.open_socket(peer, connect_deadline, live).await
        })
        .await
        .map_err(|_| OllamaChatError::Unavailable)?
        .map_err(|_| OllamaChatError::Unavailable)?;
        if !live() || Instant::now() >= connect_deadline {
            return Err(OllamaChatError::Unavailable);
        }
        let head = format!(
            "POST /api/chat HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/x-ndjson\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            target.http_authority(),
            self.request.body.len()
        );
        let mut wire = head.into_bytes();
        wire.extend_from_slice(&self.request.body);
        channel
            .write_all(&wire)
            .await
            .map_err(|_| OllamaChatError::Unavailable)?;
        let bytes = read_ndjson_body(&mut channel, OLLAMA_CHAT_MAX_RESPONSE_BYTES)
            .await
            .map_err(|_| OllamaChatError::Unavailable)?;
        channel
            .shutdown()
            .await
            .map_err(|_| OllamaChatError::Unavailable)?;
        drop(channel);
        decoder
            .push(&bytes)
            .map_err(|_| OllamaChatError::Unavailable)?;
        let outcome = decoder.finish().map_err(|_| OllamaChatError::Unavailable)?;
        if matches!(outcome, OllamaOutcome::Failed { .. }) || !live() {
            return Err(OllamaChatError::Unavailable);
        }
        let response_digest = Sha256Digest::from_bytes(&bytes);
        let outcome_digest = canonical_sha256(&json!({
            "schema":"unleash.ollama_chat_observation.v1","action_id":self.action,
            "dispatch_id":dispatch_id,"request_digest":self.request.digest,
            "response_digest":response_digest,"outcome":outcome
        }))
        .map_err(|_| OllamaChatError::DigestUnavailable)?;
        let observation = OllamaChatObservation {
            action_id: self.action,
            dispatch_id,
            request_digest: self.request.digest,
            response_digest,
            outcome_digest,
            observed_at_ms: self.clock.now_ms(),
        };
        self.port.persist(&observation, &outcome).await?;
        Ok(OllamaChatResult {
            observation,
            outcome,
        })
    }
}

impl ActionExecutorPort for Executor<'_> {
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
                    reason: "ollama_chat_binding_rejected".into(),
                });
            }
            let unknown = || ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "ollama_chat_outcome_unknown".into(),
            };
            let decoder = self
                .request
                .decoder
                .lock()
                .map_err(|_| unknown())?
                .take()
                .ok_or_else(unknown)?;
            let window = self.remaining_ms(intent, decision).ok_or_else(unknown)?;
            let deadline = Instant::now() + Duration::from_millis(window);
            let observed = tokio::time::timeout_at(
                deadline,
                self.exchange(dispatch_id, intent, decision, deadline, decoder),
            )
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            if !self.live(intent, decision) || Instant::now() >= deadline {
                return Err(unknown());
            }
            let execution = ActionExecutionResult {
                dispatch_id,
                provider_request_id: None,
                output_digest: observed.observation.outcome_digest,
                observed_at_ms: observed.observation.observed_at_ms,
            };
            *self.stage.lock().map_err(|_| unknown())? = Some(observed);
            Ok(execution)
        })
    }
}

/// Execute a frozen request through the caller's broker.
///
/// # Errors
/// Returns an error for a mismatched intent or unavailable staging.
pub async fn execute(
    request: FrozenOllamaChatRequest,
    dispatch: BrokerDispatch<'_>,
    port: &dyn OllamaChatObservationPort,
) -> Result<OllamaChatExecution, OllamaChatError> {
    if request
        .placement_expiry
        .is_some_and(|expiry| dispatch.clock.now_ms() >= expiry)
        || dispatch.broker.action_has_secret_refs(dispatch.action) != Some(false)
        || !dispatch
            .broker
            .action_audit(dispatch.action)
            .is_some_and(|audit| {
                audit.payload_digest == request.digest
                    && audit.decision_ref.as_ref() == Some(dispatch.decision)
            })
    {
        return Err(OllamaChatError::InvalidRequest);
    }
    let executor = Executor {
        request,
        action: dispatch.action,
        clock: dispatch.clock,
        port,
        stage: Mutex::new(None),
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
    let state = dispatch.broker.action_state(dispatch.action);
    let result = executor
        .stage
        .into_inner()
        .map_err(|_| OllamaChatError::Unavailable)?
        .filter(|observed| {
            let observation = &observed.observation;
            state == Some(ActionState::ObservedSuccess)
                && receipt.as_ref().is_ok_and(|receipt| {
                    receipt.state == ActionState::ObservedSuccess
                        && receipt.action_id == observation.action_id
                        && receipt.dispatch_id == observation.dispatch_id
                        && receipt.output_digest == Some(observation.outcome_digest)
                })
        });
    Ok(OllamaChatExecution { receipt, result })
}
