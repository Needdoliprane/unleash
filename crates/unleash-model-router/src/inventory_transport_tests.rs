//! T-F012-013 / REQ-MODEL-013: native inventory against synthetic loopback peers.
//! Memory authority/journal only: no `SQLite` admission, mandate or real provider.

use std::collections::BTreeSet;
use std::error::Error;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationDecision, AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry,
    BrokerJournalError, BrokerJournalPort, EffectiveAuthority, EffectsClass, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, ScopeKind, compute_effective_authority,
};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    LineageFence, ModelPolicy, ReconcileStrategy, Revision, Sha256Digest, TimestampMs,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe,
};

use crate::inventory_transport::{
    FrozenOllamaInventoryRequest, INVENTORY_VERB, InventoryError, InventoryExecution,
    InventoryOutcome, OllamaInventory, execute_ollama_inventory,
};
use crate::transport::{BrokerDispatch, ProfileRef};

type TestResult = Result<(), Box<dyn Error>>;

struct Journal(Arc<AtomicUsize>);

impl BrokerJournalPort for Journal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            if matches!(entry, BrokerJournalEntry::Dispatch { .. }) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        })
    }
}

struct Harness {
    broker: ActionBroker,
    clock: Arc<ManualClock>,
    fence: LineageFence,
    authority: EffectiveAuthority,
    secrets: SecretResolutionSpy<unleash_testkit::InMemoryTestSecretStore>,
    dispatches: Arc<AtomicUsize>,
}

impl Harness {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(1_767_225_600_000));
        let fence = LineageFence::new(
            AuthorityGeneration::from_u128(2),
            DaemonEpoch::INITIAL,
            Fence::new(1),
        );
        let dispatches = Arc::new(AtomicUsize::new(0));
        let broker = ActionBroker::new(
            clock.clone(),
            clock.clone(),
            Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
            Arc::new(DeterministicIdGenerator::new(303)),
            Arc::new(TestIsolationProbe::standard_available()),
            Arc::new(BrokerExecutionSpy::new()),
            Arc::new(LocalAuthorityFallbackPolicy::default()),
            BrokerConfig::for_test(fence),
            Arc::new(Journal(Arc::clone(&dispatches))),
        );
        Self {
            broker,
            clock,
            fence,
            authority: authority(),
            secrets: SecretResolutionSpy::in_memory(),
            dispatches,
        }
    }

    async fn run(
        &self,
        request: FrozenOllamaInventoryRequest,
    ) -> Result<InventoryExecution, Box<dyn Error>> {
        let action = ActionIntent {
            action_id: ActionId::from_u128(303),
            subject: ActionSubject::new_control_plane(
                ActorId::new("inventory-fixture")?,
                "synthetic-fixture-cause".to_owned(),
            )?,
            project_id: None,
            verb: INVENTORY_VERB.to_owned(),
            resource_refs: vec![request.resource()],
            payload_digest: request.payload_digest(),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::NetworkEgress,
            scope_revision: Revision::new(1),
            policy_revision: Revision::INITIAL,
            lineage_fence: self.fence,
            action_deadline: TimestampMs::new(1_767_225_660_000),
            isolation_requirements: IsolationRequirements::from_authority(&self.authority),
            secret_refs: vec![],
            provenance: ActionProvenance::EngineInternal,
        };
        let decision: AuthorizationDecision = match self
            .broker
            .authorize_action(action.clone(), &self.authority)
            .await?
        {
            AuthorizationOutcome::Authorized(decision) => decision,
            AuthorizationOutcome::ApprovalRequired { .. } => {
                return Err("fixture unexpectedly requires approval".into());
            }
        };
        assert_eq!(self.dispatches.load(Ordering::SeqCst), 0);
        Ok(execute_ollama_inventory(
            request,
            BrokerDispatch {
                broker: &self.broker,
                action: action.action_id,
                decision: &decision.decision_ref,
                fence: self.fence,
                authority: &self.authority,
                clock: self.clock.clone(),
                secrets: &self.secrets,
            },
        )
        .await?)
    }

    fn assert_one_dispatch_no_secret(&self) {
        assert_eq!(self.dispatches.load(Ordering::SeqCst), 1);
        assert_eq!(self.secrets.resolution_attempts(), 0);
        assert_eq!(self.secrets.successful_resolutions(), 0);
    }
}

fn authority() -> EffectiveAuthority {
    compute_effective_authority(
        None,
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: "synthetic-inventory-fixture".to_owned(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([INVENTORY_VERB.to_owned()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from(["127.0.0.1".to_owned()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Global,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    )
}

fn request(
    port: u16,
    timeout_ms: u64,
    max_bytes: usize,
) -> Result<FrozenOllamaInventoryRequest, InventoryError> {
    FrozenOllamaInventoryRequest::new_loopback(
        &format!("http://127.0.0.1:{port}/api/tags"),
        ProfileRef::new(
            "synthetic-inventory-profile",
            2,
            Sha256Digest::from_bytes(b"inventory-profile"),
        )
        .map_err(|_| InventoryError::InvalidRequest)?,
        timeout_ms,
        max_bytes,
    )
}

struct Capture {
    request: Vec<u8>,
    extra_connection: bool,
}

enum Response {
    Bytes(Vec<u8>),
    HoldUntilClosed,
}

struct Fixture {
    port: u16,
    task: Option<JoinHandle<io::Result<Capture>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Fixture {
    async fn new(response: Response, dispatches: Arc<AtomicUsize>) -> io::Result<Self> {
        Self::with_response_reset(response, dispatches, false).await
    }

    async fn with_response_reset(
        response: Response,
        dispatches: Arc<AtomicUsize>,
        allow_response_reset: bool,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(async move {
            let (mut socket, _) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
            if dispatches.load(Ordering::SeqCst) != 1 {
                return Err(io::Error::other(
                    "socket reached before unique broker dispatch",
                ));
            }
            let mut request = Vec::new();
            loop {
                let byte = tokio::time::timeout(Duration::from_secs(2), socket.read_u8()).await??;
                request.push(byte);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                if request.len() > 8192 {
                    return Err(io::Error::other("fixture request header exceeded bound"));
                }
            }
            let response_sent = if let Response::Bytes(response) = response {
                socket.write_all(&response).await?;
                socket.shutdown().await?;
                true
            } else {
                false
            };
            let mut remaining = Vec::new();
            let closed = tokio::time::timeout(
                Duration::from_secs(2),
                socket.take(8192).read_to_end(&mut remaining),
            )
            .await?;
            // A client rejecting an oversized unread response may reset TCP.
            // This only collects its already received request bytes; it is not
            // a successful product observation or permission to drain/retry.
            if let Err(error) = closed
                && !(error.kind() == io::ErrorKind::ConnectionReset
                    && allow_response_reset
                    && response_sent)
            {
                return Err(error);
            }
            request.extend(remaining);
            let extra_connection =
                tokio::time::timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_ok();
            Ok(Capture {
                request,
                extra_connection,
            })
        });
        Ok(Self {
            port,
            task: Some(task),
        })
    }

    async fn capture(&mut self) -> Result<Capture, Box<dyn Error>> {
        Ok(self
            .task
            .take()
            .ok_or("fixture already consumed")?
            .await??)
    }
}

fn http(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: application/json\r\n{headers}\r\n"
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn json_response(body: &[u8]) -> Response {
    Response::Bytes(http(
        "200 OK",
        &format!("Content-Length: {}\r\n", body.len()),
        body,
    ))
}

fn assert_get_only(capture: &Capture, port: u16) -> TestResult {
    let request = std::str::from_utf8(&capture.request)?;
    let (header, body) = request.split_once("\r\n\r\n").ok_or("incomplete GET")?;
    assert!(header.starts_with("GET /api/tags HTTP/1.1\r\n"));
    assert!(
        header
            .to_ascii_lowercase()
            .contains(&format!("\r\nhost: 127.0.0.1:{port}"))
    );
    assert!(!header.to_ascii_lowercase().contains("authorization:"));
    assert!(!header.to_ascii_lowercase().contains("transfer-encoding:"));
    for line in header.lines() {
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            assert_eq!(value.trim(), "0");
        }
    }
    assert!(
        body.is_empty(),
        "inventory GET uploaded an application body"
    );
    assert!(!capture.extra_connection, "inventory retried the socket");
    Ok(())
}

fn listed(execution: InventoryExecution) -> Result<OllamaInventory, Box<dyn Error>> {
    assert_eq!(execution.state, ActionState::ObservedSuccess);
    let receipt = execution.receipt?;
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    let observation = execution
        .observation
        .ok_or("missing native inventory fact")?;
    assert_eq!(receipt.action_id, observation.action_id());
    assert_eq!(receipt.dispatch_id, observation.dispatch_id());
    assert_eq!(
        receipt.output_digest,
        Some(unleash_protocol::canonical_sha256(&observation)?)
    );
    match execution.outcome {
        InventoryOutcome::Listed(list) => Ok(list),
        InventoryOutcome::Failed(error) => Err(error.into()),
    }
}

fn assert_unknown(execution: &InventoryExecution) {
    assert_eq!(execution.state, ActionState::OutcomeUnknown);
    assert!(
        matches!(&execution.receipt, Err(ActionBrokerError::OutcomeUnknown { action_id, .. }) if *action_id == ActionId::from_u128(303))
    );
    assert!(execution.observation.is_none());
}

async fn rejected_response(
    response: Response,
    max_bytes: usize,
    expected: InventoryError,
) -> TestResult {
    let harness = Harness::new();
    let mut fixture = Fixture::with_response_reset(
        response,
        Arc::clone(&harness.dispatches),
        expected == InventoryError::ResponseTooLarge,
    )
    .await?;
    let execution = harness.run(request(fixture.port, 1000, max_bytes)?).await?;
    assert_unknown(&execution);
    assert!(matches!(execution.outcome, InventoryOutcome::Failed(error) if error == expected));
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[tokio::test]
async fn t_f012_013_inventory_gets_one_native_list_without_upload() -> TestResult {
    let harness = Harness::new();
    let body = br#"{"models":[{"name":"synthetic:small","model":"synthetic:small","modified_at":"2026-11-20T00:00:00Z","size":1234,"digest":"backend-declared-digest","details":{"format":"gguf","family":"synthetic","families":["synthetic"],"parameter_size":"fixture","quantization_level":"fixture-q"}},{"name":"metadata-absent"}]}"#;
    let mut fixture = Fixture::new(json_response(body), Arc::clone(&harness.dispatches)).await?;
    let inventory = listed(
        harness
            .run(request(fixture.port, 1000, body.len())?)
            .await?,
    )?;
    assert_eq!(inventory.models.len(), 2);
    let model = inventory.models.first().ok_or("missing native model")?;
    assert_eq!(model.digest.as_deref(), Some("backend-declared-digest"));
    assert_eq!(model.size, Some(1234));
    assert_eq!(model.modified_at.as_deref(), Some("2026-11-20T00:00:00Z"));
    let details = model.details.as_ref().ok_or("missing native details")?;
    assert_eq!(details.format.as_deref(), Some("gguf"));
    assert_eq!(details.quantization_level.as_deref(), Some("fixture-q"));
    assert_eq!(details.parameter_size.as_deref(), Some("fixture"));
    let absent = inventory
        .models
        .get(1)
        .ok_or("missing metadata-absent model")?;
    assert!(absent.digest.is_none() && absent.details.is_none() && absent.size.is_none());
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[tokio::test]
async fn t_f012_013_inventory_empty_is_an_observed_empty_list() -> TestResult {
    let harness = Harness::new();
    let mut fixture = Fixture::new(
        json_response(br#"{"models":[]}"#),
        Arc::clone(&harness.dispatches),
    )
    .await?;
    let inventory = listed(harness.run(request(fixture.port, 1000, 128)?).await?)?;
    assert!(inventory.models.is_empty());
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[tokio::test]
async fn t_f012_013_inventory_rejects_declared_body_over_bound() -> TestResult {
    rejected_response(
        Response::Bytes(http("200 OK", "Content-Length: 4096\r\n", b"{}")),
        128,
        InventoryError::ResponseTooLarge,
    )
    .await
}

#[tokio::test]
async fn t_f012_013_inventory_rejects_chunked_body_over_bound() -> TestResult {
    rejected_response(
        Response::Bytes(http(
            "200 OK",
            "Transfer-Encoding: chunked\r\n",
            b"21\r\n012345678901234567890123456789012\r\n0\r\n\r\n",
        )),
        32,
        InventoryError::ResponseTooLarge,
    )
    .await
}

#[tokio::test]
async fn t_f012_013_inventory_deadline_closes_one_attempt() -> TestResult {
    let harness = Harness::new();
    let mut fixture =
        Fixture::new(Response::HoldUntilClosed, Arc::clone(&harness.dispatches)).await?;
    let started = tokio::time::Instant::now();
    let execution = harness.run(request(fixture.port, 80, 128)?).await?;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_unknown(&execution);
    assert!(matches!(
        execution.outcome,
        InventoryOutcome::Failed(InventoryError::DeadlineExpired)
    ));
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[tokio::test]
async fn t_f012_013_inventory_rejects_invalid_native_json() -> TestResult {
    rejected_response(
        json_response(br#"{"models":"not-a-list"}"#),
        128,
        InventoryError::InvalidResponse,
    )
    .await
}

#[tokio::test]
async fn t_f012_013_inventory_rejects_redirect_without_contacting_second_listener() -> TestResult {
    let second = TcpListener::bind("127.0.0.1:0").await?;
    let location = format!(
        "Location: http://{}/api/tags\r\nContent-Length: 0\r\n",
        second.local_addr()?
    );
    rejected_response(
        Response::Bytes(http("302 Found", &location, b"")),
        128,
        InventoryError::RedirectRejected,
    )
    .await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), second.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_013_inventory_does_not_retry_provider_error() -> TestResult {
    rejected_response(
        Response::Bytes(http(
            "429 Too Many Requests",
            "Content-Length: 0\r\nRetry-After: 0\r\n",
            b"",
        )),
        128,
        InventoryError::InvalidResponse,
    )
    .await
}

#[test]
fn t_f012_013_inventory_constructor_rejects_generation_route_and_remote_http() -> TestResult {
    for endpoint in [
        "http://127.0.0.1:11434/api/generate",
        "http://192.0.2.1:11434/api/tags",
    ] {
        let profile = ProfileRef::new(
            "synthetic-inventory-profile",
            2,
            Sha256Digest::from_bytes(b"profile"),
        )?;
        assert!(matches!(
            FrozenOllamaInventoryRequest::new_loopback(endpoint, profile, 1000, 128),
            Err(InventoryError::InvalidRequest)
        ));
    }
    Ok(())
}
