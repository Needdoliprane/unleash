//! T-F012-001/003/004/005/007/008/009/011: real TLS, synthetic keys, fixture journal.
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::{TlsAcceptor, rustls::ServerConfig, server::TlsStream};
use unleash_action_broker::*;
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    LineageFence, ModelPolicy, ReconcileStrategy, Revision, SecretRef, Sha256Digest, TimestampMs,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe,
};

use crate::anthropic::{AnthropicOutcome, AnthropicUsage};
use crate::anthropic_transport::{
    ANTHROPIC_VERSION, MESSAGES_VERB, MessagesRequest, execute_fixture, output_value,
};
use crate::transport::{BrokerDispatch, ProfileRef, RetryAfter};
use crate::transport_tests::TlsFixture;

type TestResult = Result<(), Box<dyn Error>>;
const TOKEN: &str = "messages-fixture-token";
const TEXT: &str = include_str!("../fixtures/anthropic-messages-2026-11-18-text.sse");

#[derive(Clone)]
enum Reply {
    Complete(String),
    Rate(&'static str),
    Cut(String),
    Stall(String),
    Redirect(String),
}
struct Server {
    port: u16,
    done: watch::Sender<bool>,
    captures: Arc<Mutex<Vec<Vec<u8>>>>,
    task: JoinHandle<Result<(), std::io::Error>>,
}
impl Server {
    async fn new(tls: Arc<ServerConfig>, reply: Reply) -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let (done, mut stop) = watch::channel(false);
        let captures = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&captures);
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result?;
                        let tls = Arc::clone(&tls);
                        let reply = reply.clone();
                        let captures = Arc::clone(&observed);
                        let stop = stop.clone();
                        clients.spawn(async move { connection(stream, tls, reply, captures, stop).await });
                    }
                    _ = stop.changed() => break,
                }
            }
            drop(listener);
            while let Some(result) = clients.join_next().await {
                result.map_err(std::io::Error::other)??;
            }
            Ok(())
        });
        Ok(Self {
            port,
            done,
            captures,
            task,
        })
    }
    fn post_count(&self) -> usize {
        self.captures.lock().map_or(usize::MAX, |requests| {
            requests
                .iter()
                .filter(|request| request.starts_with(b"POST "))
                .count()
        })
    }
    async fn finish(self) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
        self.done.send_replace(true);
        tokio::time::timeout(Duration::from_secs(5), self.task).await???;
        Ok(self.captures.lock().map_err(|_| "capture lock")?.clone())
    }
}

async fn request(
    stream: &mut TlsStream<TcpStream>,
    pending: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    loop {
        if let Some(end) = pending.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let header =
                String::from_utf8_lossy(pending.get(..end).ok_or(std::io::ErrorKind::InvalidData)?);
            let length = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let complete = end
                .checked_add(4)
                .and_then(|n| n.checked_add(length))
                .ok_or(std::io::ErrorKind::InvalidData)?;
            if complete > 131_072 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            if pending.len() >= complete {
                return Ok(Some(pending.drain(..complete).collect()));
            }
        }
        let mut bytes = [0; 4096];
        let size = stream.read(&mut bytes).await?;
        if size == 0 {
            return Ok(None);
        }
        pending.extend_from_slice(bytes.get(..size).ok_or(std::io::ErrorKind::InvalidData)?);
        if pending.len() > 131_072 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
}

async fn connection(
    stream: TcpStream,
    tls: Arc<ServerConfig>,
    reply: Reply,
    captures: Arc<Mutex<Vec<Vec<u8>>>>,
    mut done: watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    let Ok(Ok(mut stream)) = tokio::time::timeout(
        Duration::from_secs(3),
        TlsAcceptor::from(tls).accept(stream),
    )
    .await
    else {
        return Ok(());
    };
    let mut pending = Vec::new();
    loop {
        let read = tokio::select! {
            read = request(&mut stream, &mut pending) => read,
            _ = done.changed() => return Ok(()),
        };
        let Some(bytes) = read? else {
            return Ok(());
        };
        captures
            .lock()
            .map_err(|_| std::io::Error::other("capture lock"))?
            .push(bytes);
        let (status, extra, body, complete) = match &reply {
            Reply::Complete(body) => (
                "200 OK",
                "Content-Type: text/event-stream\r\n".to_owned(),
                body.clone(),
                true,
            ),
            Reply::Rate(delay) => (
                "429 Too Many Requests",
                format!("Retry-After: {delay}\r\n"),
                String::new(),
                true,
            ),
            Reply::Cut(body) | Reply::Stall(body) => (
                "200 OK",
                "Content-Type: text/event-stream\r\n".to_owned(),
                body.clone(),
                false,
            ),
            Reply::Redirect(url) => (
                "307 Temporary Redirect",
                format!("Location: {url}\r\n"),
                String::new(),
                true,
            ),
        };
        // Keep-alive lets the oracle count a second POST on the SAME connection as well.
        let length = body.len() + usize::from(!complete) * 100;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: keep-alive\r\n{extra}\r\n{body}"
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
        if matches!(reply, Reply::Cut(_)) {
            stream.shutdown().await?;
            return Ok(());
        }
        if matches!(reply, Reply::Stall(_)) {
            let _ = done.changed().await;
            return Ok(());
        }
    }
}

#[derive(Clone, Copy)]
enum JournalMode {
    Accept,
    RejectObservation,
    DelayObservation,
}
struct Journal {
    mode: JournalMode,
    entries: Mutex<Vec<BrokerJournalEntry>>,
    entered: Notify,
    release: Notify,
}
impl BrokerJournalPort for Journal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry.clone());
        Box::pin(async move {
            if matches!(entry, BrokerJournalEntry::Observation { .. }) {
                match self.mode {
                    JournalMode::RejectObservation => return Err(BrokerJournalError),
                    JournalMode::DelayObservation => {
                        self.entered.notify_one();
                        self.release.notified().await;
                    }
                    JournalMode::Accept => {}
                }
            }
            Ok(())
        })
    }
}
struct Fixture {
    broker: ActionBroker,
    clock: Arc<ManualClock>,
    fence: LineageFence,
    secrets: SecretResolutionSpy<unleash_testkit::InMemoryTestSecretStore>,
    secret: SecretRef,
    journal: Arc<Journal>,
}
impl Fixture {
    fn new(mode: JournalMode) -> Result<Self, Box<dyn Error>> {
        let clock = Arc::new(ManualClock::new(1_767_225_600_000));
        let fence = LineageFence::new(
            AuthorityGeneration::from_u128(22),
            DaemonEpoch::INITIAL,
            Fence::new(1),
        );
        let journal = Arc::new(Journal {
            mode,
            entries: Mutex::new(Vec::new()),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let broker = ActionBroker::new(
            clock.clone(),
            clock.clone(),
            Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
            Arc::new(DeterministicIdGenerator::new(2022)),
            Arc::new(TestIsolationProbe::standard_available()),
            Arc::new(BrokerExecutionSpy::new()),
            Arc::new(LocalAuthorityFallbackPolicy::default()),
            BrokerConfig::for_test(fence),
            journal.clone(),
        );
        let secrets = SecretResolutionSpy::in_memory();
        let secret = SecretRef::new("messages-fixture-key")?;
        secrets.store(&secret, SecretString::from(TOKEN.to_owned()))?;
        Ok(Self {
            broker,
            clock,
            fence,
            secrets,
            secret,
            journal,
        })
    }
    fn profile(version: u64) -> Result<ProfileRef, crate::transport::TransportError> {
        ProfileRef::new(
            "messages-fixture",
            version,
            Sha256Digest::from_bytes(b"messages-profile"),
        )
    }
    fn frozen(&self, port: u16) -> Result<MessagesRequest, crate::transport::TransportError> {
        MessagesRequest::new(
            &format!("https://localhost:{port}/v1/messages"),
            "fixture-model",
            "hello",
            Self::profile(1)?,
            self.secret.clone(),
        )
    }
    fn authority(secret: &SecretRef) -> EffectiveAuthority {
        compute_effective_authority(
            None,
            Revision::INITIAL,
            Revision::INITIAL,
            &[AuthorityLayer {
                layer_name: "messages-fixture".into(),
                allowed_effects: Some(BTreeSet::from([EffectsClass::ModelDispatch])),
                denied_effects: BTreeSet::new(),
                allowed_verbs: Some(BTreeSet::from([
                    MESSAGES_VERB.into(),
                    "model.responses.stream".into(),
                ])),
                denied_verbs: BTreeSet::new(),
                allowed_roots: None,
                allowed_network_hosts: Some(BTreeSet::from([
                    "localhost".into(),
                    "127.0.0.1".into(),
                ])),
                allowed_secret_refs: Some(BTreeSet::from([secret.as_opaque_str().to_owned()])),
                model_policy: ModelPolicy::AllowedBounded,
                data_locality: DataLocality::ApprovedRemote,
                isolation_mode: IsolationMode::Standard,
                require_action_isolation: false,
                max_scope_kind: ScopeKind::Global,
                allow_delegation: false,
                allow_cross_repository: false,
            }],
        )
    }
    fn intent(
        &self,
        request: &MessagesRequest,
        port: u16,
        authority: &EffectiveAuthority,
    ) -> Result<ActionIntent, Box<dyn Error>> {
        Ok(ActionIntent {
            action_id: ActionId::from_u128(71),
            subject: ActionSubject::new_control_plane(
                ActorId::new("messages-fixture")?,
                "receipt:fixture".into(),
            )?,
            project_id: None,
            verb: MESSAGES_VERB.into(),
            resource_refs: vec![
                NormalizedResourceRef::ModelTarget {
                    provider: "anthropic".into(),
                    model_id: "fixture-model".into(),
                    locality: DataLocality::ApprovedRemote,
                },
                NormalizedResourceRef::NetworkEndpoint {
                    host: "localhost".into(),
                    port: Some(port),
                    is_private_or_loopback: true,
                },
            ],
            payload_digest: request.payload_digest()?,
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::ModelDispatch,
            scope_revision: Revision::INITIAL,
            policy_revision: Revision::INITIAL,
            lineage_fence: self.fence,
            action_deadline: TimestampMs::new(self.clock.utc_ms() + 30_000),
            isolation_requirements: IsolationRequirements::from_authority(authority),
            secret_refs: vec![self.secret.clone()],
            provenance: ActionProvenance::EngineInternal,
        })
    }
    async fn authorize(
        &self,
        intent: ActionIntent,
        authority: &EffectiveAuthority,
    ) -> Result<AuthorizationDecision, Box<dyn Error>> {
        match self.broker.authorize_action(intent, authority).await? {
            AuthorizationOutcome::Authorized(decision) => Ok(decision),
            AuthorizationOutcome::ApprovalRequired { .. } => Err("unexpected approval".into()),
        }
    }
    fn dispatch<'a>(
        &'a self,
        decision: &'a AuthorizationDecision,
        authority: &'a EffectiveAuthority,
    ) -> BrokerDispatch<'a> {
        BrokerDispatch {
            broker: &self.broker,
            action: decision.action_id,
            decision: &decision.decision_ref,
            fence: self.fence,
            authority,
            clock: self.clock.clone(),
            secrets: &self.secrets,
        }
    }
}

fn parse_request(bytes: &[u8]) -> Result<(String, Value), Box<dyn Error>> {
    let text = std::str::from_utf8(bytes)?;
    let (header, body) = text.split_once("\r\n\r\n").ok_or("missing headers")?;
    Ok((header.to_lowercase(), serde_json::from_str(body)?))
}
fn partial() -> String {
    TEXT.split("event: message_stop")
        .next()
        .unwrap_or("")
        .replace("\"input_tokens\":0", "\"input_tokens\":10")
        .replace(",\"cache_read_input_tokens\":0", "")
}

#[tokio::test]
async fn t_f012_001_messages_tls_native_request_and_correlated_receipt() -> TestResult {
    let tls = TlsFixture::new()?;
    let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
    let fixture = Fixture::new(JournalMode::Accept)?;
    let request = fixture.frozen(server.port)?;
    let auth = Fixture::authority(&fixture.secret);
    let intent = fixture.intent(&request, server.port, &auth)?;
    let decision = fixture.authorize(intent.clone(), &auth).await?;
    let execution = execute_fixture(
        request,
        fixture.dispatch(&decision, &auth),
        Some(tls.ca.clone()),
    )
    .await?;
    let receipt = execution.receipt?;
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    assert_eq!(receipt.action_id, intent.action_id);
    assert_eq!(receipt.evaluation_id, decision.evaluation_id);
    let usage = execution.observed_usage?.ok_or("missing usage")?;
    assert_eq!(usage.action_id, receipt.action_id);
    assert_eq!(usage.dispatch_id, receipt.dispatch_id);
    let outcome = execution.outcome?.ok_or("missing outcome")?;
    assert_eq!(
        receipt.output_digest,
        Some(unleash_protocol::canonical_sha256(&output_value(&outcome))?)
    );
    assert!(matches!(outcome,AnthropicOutcome::Success { ref text, .. } if text=="Héllo"));
    assert_eq!(server.post_count(), 1);
    let requests = server.finish().await?;
    assert_eq!(requests.len(), 1);
    let (header, body) = parse_request(requests.first().ok_or("no POST")?)?;
    assert!(header.starts_with("post /v1/messages http/1.1\r\n"));
    assert!(header.contains(&format!("x-api-key: {TOKEN}")));
    assert!(header.contains(&format!("anthropic-version: {ANTHROPIC_VERSION}")));
    assert!(!header.contains("authorization:"));
    assert_eq!(
        body,
        json!({"model":"fixture-model","messages":[{"role":"user","content":"hello"}],"max_tokens":256,"stream":true})
    );
    assert!(!body.to_string().contains(TOKEN));
    assert!(fixture.journal.entries.lock().map_err(|_|"journal")?.iter().any(|entry|matches!(entry,BrokerJournalEntry::Observation {receipt:recorded,..} if recorded==&receipt)));
    Ok(())
}

#[tokio::test]
async fn t_f012_007_messages_rate_limit_is_typed_and_never_retried() -> TestResult {
    for (header, expected) in [
        ("2", RetryAfter::Seconds(2)),
        ("301", RetryAfter::BeyondLimit),
        ("n/a", RetryAfter::Unavailable),
    ] {
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Rate(header)).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let request = fixture.frozen(server.port)?;
        let auth = Fixture::authority(&fixture.secret);
        let decision = fixture
            .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
            .await?;
        let execution = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(matches!(
            execution.receipt,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        assert!(execution.outcome?.is_none());
        let limited = execution.rate_limited?.ok_or("missing rate limit")?;
        assert_eq!(limited.action_id, decision.action_id);
        assert_eq!(limited.retry_after, expected);
        assert_eq!(server.post_count(), 1);
        assert_eq!(server.finish().await?.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_007_messages_cut_and_timeout_keep_partial_usage_without_second_post() -> TestResult
{
    for stall in [false, true] {
        let tls = TlsFixture::new()?;
        let reply = if stall {
            Reply::Stall(partial())
        } else {
            Reply::Cut(partial())
        };
        let server = Server::new(tls.server.clone(), reply).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let request = fixture.frozen(server.port)?;
        let auth = Fixture::authority(&fixture.secret);
        let mut intent = fixture.intent(&request, server.port, &auth)?;
        if stall {
            intent.action_deadline = TimestampMs::new(fixture.clock.utc_ms() + 2_000);
        }
        let decision = fixture.authorize(intent, &auth).await?;
        let execution = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(matches!(
            execution.receipt,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        assert!(execution.outcome?.is_none());
        let observed = execution.observed_usage?.ok_or("missing partial usage")?;
        assert_eq!(observed.action_id, decision.action_id);
        assert_eq!(
            observed.usage,
            AnthropicUsage {
                input: Some(10),
                output: Some(3),
                cache_read: None,
                cache_creation: None
            }
        );
        assert_eq!(server.post_count(), 1);
        // Recreating a transport object cannot turn this uncertain action into another POST.
        let repeated = execute_fixture(
            fixture.frozen(server.port)?,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(repeated.receipt.is_err());
        assert!(repeated.outcome?.is_none());
        assert_eq!(server.post_count(), 1);
        assert_eq!(server.finish().await?.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_011_messages_invalid_usage_event_keeps_last_valid_snapshot() -> TestResult {
    let body = format!(
        "{}event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{}},\"usage\":{{\"input_tokens\":20,\"output_tokens\":2}}}}\n\n",
        partial()
    );
    let tls = TlsFixture::new()?;
    let server = Server::new(tls.server.clone(), Reply::Complete(body)).await?;
    let fixture = Fixture::new(JournalMode::Accept)?;
    let request = fixture.frozen(server.port)?;
    let auth = Fixture::authority(&fixture.secret);
    let decision = fixture
        .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
        .await?;
    let execution = execute_fixture(
        request,
        fixture.dispatch(&decision, &auth),
        Some(tls.ca.clone()),
    )
    .await?;
    assert!(matches!(
        execution.receipt,
        Err(ActionBrokerError::OutcomeUnknown { .. })
    ));
    assert!(execution.outcome?.is_none());
    assert_eq!(
        execution.observed_usage?.ok_or("missing usage")?.usage,
        AnthropicUsage {
            input: Some(10),
            output: Some(3),
            cache_read: None,
            cache_creation: None
        }
    );
    assert_eq!(server.finish().await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn t_f012_004_messages_rejected_or_late_receipt_controls_publication() -> TestResult {
    for mode in [
        JournalMode::RejectObservation,
        JournalMode::DelayObservation,
    ] {
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
        let fixture = Fixture::new(mode)?;
        let request = fixture.frozen(server.port)?;
        let auth = Fixture::authority(&fixture.secret);
        let decision = fixture
            .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
            .await?;
        let future = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        );
        tokio::pin!(future);
        if matches!(mode, JournalMode::DelayObservation) {
            tokio::select! {
                value=&mut future => return Err(format!("result before journal acknowledgement: {value:?}").into()),
                ()=fixture.journal.entered.notified() => {},
                ()=tokio::time::sleep(Duration::from_secs(3)) => return Err("journal gate not reached".into()),
            }
            assert_eq!(server.post_count(), 1);
            fixture.journal.release.notify_one();
        }
        let execution = future.await?;
        if matches!(mode, JournalMode::RejectObservation) {
            assert!(execution.receipt.is_err());
            assert!(execution.outcome?.is_none());
        } else {
            assert_eq!(execution.receipt?.state, ActionState::ObservedSuccess);
            assert!(execution.outcome?.is_some());
        }
        assert_eq!(server.finish().await?.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_004_messages_cancelled_observation_never_reposts() -> TestResult {
    let tls = TlsFixture::new()?;
    let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
    let fixture = Fixture::new(JournalMode::DelayObservation)?;
    let request = fixture.frozen(server.port)?;
    let auth = Fixture::authority(&fixture.secret);
    let decision = fixture
        .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
        .await?;
    // Dropping the actual future, rather than merely a Pin reference, loses the ACK.
    {
        let future = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        );
        tokio::pin!(future);
        tokio::select! {
            value=&mut future => return Err(format!("result before journal acknowledgement: {value:?}").into()),
            ()=fixture.journal.entered.notified() => {},
            ()=tokio::time::sleep(Duration::from_secs(3)) => return Err("journal gate not reached".into()),
        }
        assert_eq!(server.post_count(), 1);
    }
    assert_eq!(
        fixture.broker.action_state(decision.action_id),
        Some(ActionState::OutcomeUnknown)
    );
    let repeated = execute_fixture(
        fixture.frozen(server.port)?,
        fixture.dispatch(&decision, &auth),
        Some(tls.ca.clone()),
    )
    .await?;
    assert!(repeated.receipt.is_err());
    assert!(repeated.outcome?.is_none());
    assert_eq!(server.finish().await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn t_f012_008_messages_rejects_responses_intent_and_wrong_credential() -> TestResult {
    for responses in [true, false] {
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let wrong = SecretRef::new("other-fixture-key")?;
        fixture
            .secrets
            .store(&wrong, SecretString::from("other-fixture-token".to_owned()))?;
        let request = fixture.frozen(server.port)?;
        let key = if responses { &fixture.secret } else { &wrong };
        let auth = Fixture::authority(key);
        let mut intent = fixture.intent(&request, server.port, &auth)?;
        if responses {
            intent.verb = "model.responses.stream".into();
            if let Some(NormalizedResourceRef::ModelTarget { provider, .. }) =
                intent.resource_refs.first_mut()
            {
                *provider = "openai".into();
            }
            intent.payload_digest = crate::transport::FrozenRequest::new(
                &format!("https://localhost:{}/v1/responses", server.port),
                "fixture-model",
                "hello",
                Fixture::profile(1)?,
                fixture.secret.clone(),
            )?
            .payload_digest()?;
        } else {
            intent.secret_refs = vec![wrong];
        }
        let decision = fixture.authorize(intent, &auth).await?;
        let execution = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(execution.receipt.is_err());
        assert!(execution.outcome?.is_none());
        assert_eq!(server.post_count(), 0);
        assert!(server.finish().await?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_003_messages_changed_profile_and_input_are_rejected_before_post() -> TestResult {
    for variant in [0, 1] {
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let original = fixture.frozen(server.port)?;
        let changed = MessagesRequest::new(
            &format!("https://localhost:{}/v1/messages", server.port),
            "fixture-model",
            if variant == 0 { "hello" } else { "changed" },
            Fixture::profile(if variant == 0 { 2 } else { 1 })?,
            fixture.secret.clone(),
        )?;
        assert_ne!(original.payload_digest()?, changed.payload_digest()?);
        let auth = Fixture::authority(&fixture.secret);
        let decision = fixture
            .authorize(fixture.intent(&original, server.port, &auth)?, &auth)
            .await?;
        let execution = execute_fixture(
            changed,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(execution.receipt.is_err());
        assert!(execution.outcome?.is_none());
        assert!(server.finish().await?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_008_messages_redirect_never_forwards_key() -> TestResult {
    let tls = TlsFixture::new()?;
    let destination = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
    let source = Server::new(
        tls.server.clone(),
        Reply::Redirect(format!(
            "https://localhost:{}/v1/messages",
            destination.port
        )),
    )
    .await?;
    let fixture = Fixture::new(JournalMode::Accept)?;
    let request = fixture.frozen(source.port)?;
    let auth = Fixture::authority(&fixture.secret);
    let decision = fixture
        .authorize(fixture.intent(&request, source.port, &auth)?, &auth)
        .await?;
    let execution = execute_fixture(
        request,
        fixture.dispatch(&decision, &auth),
        Some(tls.ca.clone()),
    )
    .await?;
    assert!(matches!(
        execution.receipt,
        Err(ActionBrokerError::OutcomeUnknown { .. })
    ));
    assert!(execution.outcome?.is_none());
    assert_eq!(source.finish().await?.len(), 1);
    assert!(destination.finish().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn t_f012_005_messages_semantic_outcomes_and_unknown_usage_stay_distinct() -> TestResult {
    for (reason, expected) in [("end_turn", 0), ("refusal", 1), ("max_tokens", 2)] {
        let body = TEXT
            .replace("end_turn", reason)
            .replace("\"input_tokens\":0,", "")
            .replace(",\"cache_read_input_tokens\":0", "");
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Complete(body)).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let request = fixture.frozen(server.port)?;
        let auth = Fixture::authority(&fixture.secret);
        let decision = fixture
            .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
            .await?;
        let execution = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            Some(tls.ca.clone()),
        )
        .await?;
        assert_eq!(execution.receipt?.state, ActionState::ObservedSuccess);
        assert!(matches!(
            (expected, execution.outcome?),
            (0, Some(AnthropicOutcome::Success { .. }))
                | (1, Some(AnthropicOutcome::Refusal { .. }))
                | (2, Some(AnthropicOutcome::Truncated { .. }))
        ));
        let usage = execution.observed_usage?.ok_or("missing usage")?.usage;
        assert_eq!(usage.input, None);
        assert_eq!(usage.cache_read, None);
        assert_eq!(usage.output, Some(3));
        assert_eq!(server.finish().await?.len(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_011_messages_absent_usage_remains_unknown() -> TestResult {
    let mut body = String::new();
    for event in TEXT.trim_end().split("\n\n") {
        let (name, data) = event.split_once("\ndata: ").ok_or("fixture framing")?;
        let mut data: Value = serde_json::from_str(data)?;
        if let Some(message) = data.get_mut("message").and_then(Value::as_object_mut) {
            message.remove("usage");
        }
        data.as_object_mut()
            .ok_or("fixture object")?
            .remove("usage");
        write!(body, "{name}\ndata: {data}\n\n")?;
    }
    let tls = TlsFixture::new()?;
    let server = Server::new(tls.server.clone(), Reply::Complete(body)).await?;
    let fixture = Fixture::new(JournalMode::Accept)?;
    let request = fixture.frozen(server.port)?;
    let auth = Fixture::authority(&fixture.secret);
    let decision = fixture
        .authorize(fixture.intent(&request, server.port, &auth)?, &auth)
        .await?;
    let execution = execute_fixture(
        request,
        fixture.dispatch(&decision, &auth),
        Some(tls.ca.clone()),
    )
    .await?;
    assert_eq!(execution.receipt?.state, ActionState::ObservedSuccess);
    assert!(
        matches!(execution.outcome?, Some(AnthropicOutcome::Success { usage, .. }) if usage==AnthropicUsage::default())
    );
    assert_eq!(
        execution
            .observed_usage?
            .ok_or("missing observation")?
            .usage,
        AnthropicUsage::default()
    );
    assert_eq!(server.finish().await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn t_f012_007_messages_untrusted_ca_and_wrong_name_have_no_post() -> TestResult {
    for trusted_wrong_name in [false, true] {
        let tls = TlsFixture::new()?;
        let server = Server::new(tls.server.clone(), Reply::Complete(TEXT.into())).await?;
        let fixture = Fixture::new(JournalMode::Accept)?;
        let request = MessagesRequest::new(
            &format!(
                "https://{}:{}/v1/messages",
                if trusted_wrong_name {
                    "127.0.0.1"
                } else {
                    "localhost"
                },
                server.port
            ),
            "fixture-model",
            "hello",
            Fixture::profile(1)?,
            fixture.secret.clone(),
        )?;
        let auth = Fixture::authority(&fixture.secret);
        let mut intent = fixture.intent(&request, server.port, &auth)?;
        if trusted_wrong_name
            && let Some(NormalizedResourceRef::NetworkEndpoint { host, .. }) =
                intent.resource_refs.get_mut(1)
        {
            *host = "127.0.0.1".into();
        }
        let decision = fixture.authorize(intent, &auth).await?;
        let execution = execute_fixture(
            request,
            fixture.dispatch(&decision, &auth),
            trusted_wrong_name.then(|| tls.ca.clone()),
        )
        .await?;
        assert!(matches!(
            execution.receipt,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        assert!(execution.outcome?.is_none());
        assert!(execution.observed_usage?.is_none());
        assert_eq!(server.post_count(), 0);
        assert!(server.finish().await?.is_empty());
    }
    Ok(())
}
