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
    InventoryObservation, InventoryObservationError, InventoryObservationPort, InventoryOutcome,
    OllamaInventory, execute_ollama_inventory, execute_ollama_inventory_with_observation,
};
use crate::transport::{BrokerDispatch, ProfileRef};
use crate::transport_tests::TlsFixture;

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
        Self::with_authority(authority())
    }

    fn with_authority(authority: EffectiveAuthority) -> Self {
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
            authority,
            secrets: SecretResolutionSpy::in_memory(),
            dispatches,
        }
    }

    async fn authorize(
        &self,
        request: &FrozenOllamaInventoryRequest,
    ) -> Result<(ActionId, AuthorizationDecision), Box<dyn Error>> {
        self.authorize_effect(
            INVENTORY_VERB,
            EffectsClass::NetworkEgress,
            vec![request.resource()],
            request.payload_digest(),
        )
        .await
    }

    async fn authorize_effect(
        &self,
        verb: &str,
        effects_class: EffectsClass,
        resource_refs: Vec<unleash_action_broker::NormalizedResourceRef>,
        payload_digest: Sha256Digest,
    ) -> Result<(ActionId, AuthorizationDecision), Box<dyn Error>> {
        let action = ActionIntent {
            action_id: ActionId::from_u128(303),
            subject: ActionSubject::new_control_plane(
                ActorId::new("inventory-fixture")?,
                "synthetic-fixture-cause".to_owned(),
            )?,
            project_id: None,
            verb: verb.to_owned(),
            resource_refs,
            payload_digest,
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class,
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
        Ok((action.action_id, decision))
    }

    async fn run(
        &self,
        request: FrozenOllamaInventoryRequest,
    ) -> Result<InventoryExecution, Box<dyn Error>> {
        let (action, decision) = self.authorize(&request).await?;
        Ok(execute_ollama_inventory(
            request,
            BrokerDispatch {
                broker: &self.broker,
                action,
                decision: &decision.decision_ref,
                fence: self.fence,
                authority: &self.authority,
                clock: self.clock.clone(),
                secrets: &self.secrets,
            },
        )
        .await?)
    }

    async fn run_observed(
        &self,
        request: FrozenOllamaInventoryRequest,
        observation: Arc<dyn InventoryObservationPort>,
    ) -> Result<InventoryExecution, Box<dyn Error>> {
        let (action, decision) = self.authorize(&request).await?;
        Ok(execute_ollama_inventory_with_observation(
            request,
            BrokerDispatch {
                broker: &self.broker,
                action,
                decision: &decision.decision_ref,
                fence: self.fence,
                authority: &self.authority,
                clock: self.clock.clone(),
                secrets: &self.secrets,
            },
            observation,
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
    authority_for(
        EffectsClass::NetworkEgress,
        INVENTORY_VERB,
        ModelPolicy::Forbidden,
    )
}

fn authority_for(
    effect: EffectsClass,
    verb: &str,
    model_policy: ModelPolicy,
) -> EffectiveAuthority {
    compute_effective_authority(
        None,
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: "synthetic-inventory-fixture".to_owned(),
            allowed_effects: Some(BTreeSet::from([effect])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([verb.to_owned()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from(["127.0.0.1".to_owned()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy,
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
        inventory_profile()?,
        timeout_ms,
        max_bytes,
    )
}

fn inventory_profile() -> Result<ProfileRef, InventoryError> {
    ProfileRef::new(
        "synthetic-inventory-profile",
        2,
        Sha256Digest::from_bytes(b"inventory-profile"),
    )
    .map_err(|_| InventoryError::InvalidRequest)
}

struct Capture {
    request: Vec<u8>,
    extra_connection: bool,
}

enum Response {
    Bytes(Vec<u8>),
    DelayedBytes(Vec<u8>, Duration),
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
        Self::with_close_signal(response, dispatches, allow_response_reset, None).await
    }

    async fn with_close_signal(
        response: Response,
        dispatches: Arc<AtomicUsize>,
        allow_response_reset: bool,
        close_signal: Option<Arc<tokio::sync::Notify>>,
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
            let response = match response {
                Response::Bytes(bytes) => Some(bytes),
                Response::DelayedBytes(bytes, delay) => {
                    tokio::time::sleep(delay).await;
                    Some(bytes)
                }
                Response::HoldUntilClosed => None,
            };
            let response_sent = if let Some(response) = response {
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
            if let Some(signal) = close_signal {
                signal.notify_one();
            }
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
    Response::Bytes(json_reply(body))
}

#[cfg(target_os = "macos")]
mod chat_tests {
    use super::*;
    use crate::connect_transport::OwnedLoopbackPeer;
    use crate::ollama::OllamaOutcome;
    use crate::ollama_transport::{
        FrozenOllamaChatRequest, OLLAMA_CHAT_VERB, OllamaChatError, OllamaChatExecution,
        OllamaChatLimits, OllamaChatObservation, OllamaChatObservationPort, execute,
    };
    use std::collections::BTreeMap;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use unleash_platform::OsProcessIdentity;
    use unleash_protocol::provider_probe::OllamaChatPlacement;

    const PROGRESS: &str = concat!(
        r#"{"model":"synthetic-model","created_at":"2026-11-20T00:00:00Z","message":{"role":"assistant","content":"Answer"},"done":false}"#,
        "\n",
    );
    const END: &str = concat!(
        r#"{"model":"synthetic-model","created_at":"2026-11-20T00:00:01Z","message":{"role":"assistant","content":"!"},"done":true,"done_reason":"stop","eval_count":2}"#,
        "\n",
    );
    const MODEL: &str = "synthetic-model";
    const LIMITS: OllamaChatLimits = OllamaChatLimits {
        connect_ms: 1_000,
        total_ms: 2_000,
        output_tokens: 8,
    };

    fn published_fixture_body() -> Result<Vec<u8>, unleash_protocol::ProtocolError> {
        unleash_protocol::canonicalize_to_vec(&serde_json::json!({
            "model":MODEL,"messages":[{"role":"user","content":"Fixture prompt"}],
            "stream":true,"options":{"num_predict":8},"tools":[]
        }))
    }

    fn admitted_placement(
        endpoint: SocketAddr,
        process: &OsProcessIdentity,
        valid_until_ms: TimestampMs,
    ) -> OllamaChatPlacement {
        OllamaChatPlacement {
            endpoint: format!("http://{endpoint}/api/chat"),
            boot_identity: process.boot_identity.as_str().to_owned(),
            process_id: process.pid,
            process_start_identity: process.start_identity.as_str().to_owned(),
            evidence_digest: Sha256Digest::from_bytes(b"placement fixture"),
            admission_digest: Sha256Digest::from_bytes(b"admission fixture"),
            valid_until_ms,
        }
    }

    #[test]
    fn t_f012_ollama_chat_admitted_transport_binds_exact_process_and_evidence() -> TestResult {
        let endpoint = SocketAddr::from(([127, 0, 0, 1], 11434));
        let process = unleash_platform::inspect_process_identity(std::process::id())?;
        let placement = admitted_placement(endpoint, &process, TimestampMs::new(1_767_225_601_000));
        let body = published_fixture_body()?;
        let hash = Sha256Digest::from_bytes(&body);
        let prepare = |selected: &OllamaChatPlacement| {
            Ok::<_, Box<dyn Error>>(FrozenOllamaChatRequest::from_admitted_body(
                selected,
                inventory_profile()?,
                OwnedLoopbackPeer::new(endpoint, process.clone())?,
                MODEL,
                LIMITS,
                body.clone(),
                hash,
            ))
        };
        let exact = prepare(&placement)??;
        for mismatch in [
            OllamaChatPlacement {
                process_id: 0,
                ..placement.clone()
            },
            OllamaChatPlacement {
                boot_identity: "another-fixture-boot".into(),
                ..placement.clone()
            },
            OllamaChatPlacement {
                process_start_identity: "another-fixture-start".into(),
                ..placement.clone()
            },
            OllamaChatPlacement {
                endpoint: "http://127.0.0.1:11435/api/chat".into(),
                ..placement.clone()
            },
        ] {
            assert!(matches!(
                prepare(&mismatch)?,
                Err(OllamaChatError::InvalidRequest)
            ));
        }
        for other in [
            OllamaChatPlacement {
                evidence_digest: Sha256Digest::from_bytes(b"other evidence"),
                ..placement.clone()
            },
            OllamaChatPlacement {
                admission_digest: Sha256Digest::from_bytes(b"other admission"),
                ..placement.clone()
            },
            OllamaChatPlacement {
                valid_until_ms: TimestampMs::new(1_767_225_601_001),
                ..placement
            },
        ] {
            assert_ne!(exact.payload_digest(), prepare(&other)??.payload_digest());
        }
        Ok(())
    }

    #[tokio::test]
    async fn t_f012_ollama_chat_admitted_expiry_leaves_no_dispatch() -> TestResult {
        let harness = Harness::with_authority(authority_for(
            EffectsClass::ModelDispatch,
            OLLAMA_CHAT_VERB,
            ModelPolicy::AllowedBounded,
        ));
        let endpoint = SocketAddr::from(([127, 0, 0, 1], 11434));
        let process = unleash_platform::inspect_process_identity(std::process::id())?;
        let placement = admitted_placement(endpoint, &process, TimestampMs::new(1_767_225_600_001));
        let body = published_fixture_body()?;
        let hash = Sha256Digest::from_bytes(&body);
        let request = FrozenOllamaChatRequest::from_admitted_body(
            &placement,
            inventory_profile()?,
            OwnedLoopbackPeer::new(endpoint, process)?,
            MODEL,
            LIMITS,
            body,
            hash,
        )?;
        let (action, decision) = harness
            .authorize_effect(
                OLLAMA_CHAT_VERB,
                EffectsClass::ModelDispatch,
                request.resources(),
                request.payload_digest(),
            )
            .await?;
        harness.clock.advance_ms(1);
        let observer = Observer {
            adopted: true,
            observations: Mutex::new(Vec::new()),
        };
        let result = execute(
            request,
            BrokerDispatch {
                broker: &harness.broker,
                action,
                decision: &decision.decision_ref,
                fence: harness.fence,
                authority: &harness.authority,
                clock: harness.clock.clone(),
                secrets: &harness.secrets,
            },
            &observer,
        )
        .await;
        assert!(matches!(result, Err(OllamaChatError::InvalidRequest)));
        assert_eq!(harness.dispatches.load(Ordering::SeqCst), 0);
        assert!(
            observer
                .observations
                .lock()
                .map_err(|_| "observation lock")?
                .is_empty()
        );
        Ok(())
    }

    struct Observer {
        adopted: bool,
        observations: Mutex<Vec<OllamaChatObservation>>,
    }

    impl OllamaChatObservationPort for Observer {
        fn persist<'a>(
            &'a self,
            observation: &'a OllamaChatObservation,
            _outcome: &'a OllamaOutcome,
        ) -> BoxFuture<'a, Result<(), OllamaChatError>> {
            Box::pin(async move {
                self.observations
                    .lock()
                    .map_err(|_| OllamaChatError::Unavailable)?
                    .push(observation.clone());
                if self.adopted {
                    Ok(())
                } else {
                    Err(OllamaChatError::Unavailable)
                }
            })
        }
    }

    async fn exchange(
        body: &str,
        observer: &Observer,
    ) -> Result<(OllamaChatExecution, ActionState), Box<dyn Error>> {
        let harness = Harness::with_authority(authority_for(
            EffectsClass::ModelDispatch,
            OLLAMA_CHAT_VERB,
            ModelPolicy::AllowedBounded,
        ));
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for part in body.as_bytes().chunks(41) {
            response.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            response.extend_from_slice(part);
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"0\r\n\r\n");
        let mut fixture =
            Fixture::new(Response::Bytes(response), Arc::clone(&harness.dispatches)).await?;
        let peer = std::net::SocketAddr::from(([127, 0, 0, 1], fixture.port));
        let process = unleash_platform::inspect_process_identity(std::process::id())?;
        let expected = published_fixture_body()?;
        let placement = admitted_placement(peer, &process, TimestampMs::new(1_767_225_620_000));
        let request = FrozenOllamaChatRequest::from_admitted_body(
            &placement,
            inventory_profile()?,
            OwnedLoopbackPeer::new(peer, process)?,
            MODEL,
            LIMITS,
            expected.clone(),
            Sha256Digest::from_bytes(&expected),
        )?;
        let digest = request.payload_digest();
        let (action, decision) = harness
            .authorize_effect(
                OLLAMA_CHAT_VERB,
                EffectsClass::ModelDispatch,
                request.resources(),
                digest,
            )
            .await?;
        let execution = execute(
            request,
            BrokerDispatch {
                broker: &harness.broker,
                action,
                decision: &decision.decision_ref,
                fence: harness.fence,
                authority: &harness.authority,
                clock: harness.clock.clone(),
                secrets: &harness.secrets,
            },
            observer,
        )
        .await?;
        harness.assert_one_dispatch_no_secret();
        let capture = fixture.capture().await?;
        assert!(!capture.extra_connection);
        let request = std::str::from_utf8(&capture.request)?;
        let (headers, payload) = request
            .split_once("\r\n\r\n")
            .ok_or("request boundary missing")?;
        assert!(headers.starts_with("POST /api/chat HTTP/1.1\r\n"));
        assert!(headers.contains("Accept: application/x-ndjson\r\n"));
        assert_eq!(payload.as_bytes(), expected.as_slice());
        if let Some(result) = &execution.result {
            assert_eq!(result.observation.request_digest, digest);
            assert_eq!(
                result.observation.response_digest,
                Sha256Digest::from_bytes(body.as_bytes())
            );
        }
        Ok((
            execution,
            harness
                .broker
                .action_state(action)
                .ok_or("broker action missing")?,
        ))
    }

    #[tokio::test]
    async fn t_f012_ollama_chat_releases_only_the_acknowledged_observation() -> TestResult {
        let observer = Observer {
            adopted: true,
            observations: Mutex::new(Vec::new()),
        };
        let (execution, state) = exchange(&format!("{PROGRESS}{END}"), &observer).await?;
        assert_eq!(state, ActionState::ObservedSuccess);
        let receipt = execution.receipt?;
        let result = execution.result.ok_or("observed result unavailable")?;
        assert_eq!(receipt.dispatch_id, result.observation.dispatch_id);
        assert_eq!(
            receipt.output_digest,
            Some(result.observation.outcome_digest)
        );
        assert!(
            matches!(result.outcome, OllamaOutcome::Completed { text, .. } if text == "Answer!")
        );
        assert_eq!(
            observer
                .observations
                .lock()
                .map_err(|_| "observation lock")?
                .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn t_f012_ollama_chat_published_body_keeps_the_exact_wire_and_peer_digest() -> TestResult {
        let endpoint = std::net::SocketAddr::from(([127, 0, 0, 1], 11434));
        let url = format!("http://{endpoint}/api/chat");
        let identity = unleash_platform::inspect_process_identity(std::process::id())?;
        let body = published_fixture_body()?;
        let body_hash = Sha256Digest::from_bytes(&body);
        let peer = || OwnedLoopbackPeer::new(endpoint, identity.clone());
        let published = FrozenOllamaChatRequest::from_published_body(
            &url,
            inventory_profile()?,
            peer()?,
            MODEL,
            LIMITS,
            body.clone(),
            body_hash,
        )?;
        let built = FrozenOllamaChatRequest::new(
            &url,
            inventory_profile()?,
            peer()?,
            MODEL,
            "Fixture prompt",
            LIMITS,
            BTreeMap::new(),
        )?;
        assert_eq!(published.payload_digest(), built.payload_digest());
        assert_eq!(published.resources(), built.resources());

        let mut different_identity = identity.clone();
        different_identity.pid = identity.pid.checked_add(1).ok_or("fixture pid overflow")?;
        let other = FrozenOllamaChatRequest::from_published_body(
            &url,
            inventory_profile()?,
            OwnedLoopbackPeer::new(endpoint, different_identity)?,
            MODEL,
            LIMITS,
            body.clone(),
            body_hash,
        )?;
        assert_ne!(published.payload_digest(), other.payload_digest());

        let mut non_canonical = body.clone();
        non_canonical.push(b'\n');
        let non_canonical_hash = Sha256Digest::from_bytes(&non_canonical);
        for (input, digest, limits) in [
            (
                body.clone(),
                Sha256Digest::from_bytes(b"different fixture"),
                LIMITS,
            ),
            (
                body,
                body_hash,
                OllamaChatLimits {
                    output_tokens: 7,
                    ..LIMITS
                },
            ),
            (non_canonical, non_canonical_hash, LIMITS),
        ] {
            assert!(matches!(
                FrozenOllamaChatRequest::from_published_body(
                    &url,
                    inventory_profile()?,
                    peer()?,
                    MODEL,
                    limits,
                    input,
                    digest,
                ),
                Err(OllamaChatError::InvalidRequest)
            ));
        }
        Ok(())
    }

    #[tokio::test]
    async fn t_f012_ollama_chat_keeps_an_interrupted_generation_unknown() -> TestResult {
        let observer = Observer {
            adopted: true,
            observations: Mutex::new(Vec::new()),
        };
        let (execution, state) = exchange(PROGRESS, &observer).await?;
        assert_eq!(state, ActionState::OutcomeUnknown);
        assert!(execution.result.is_none());
        assert!(
            observer
                .observations
                .lock()
                .map_err(|_| "observation lock")?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_f012_ollama_chat_keeps_an_unacknowledged_observation_unknown() -> TestResult {
        let observer = Observer {
            adopted: false,
            observations: Mutex::new(Vec::new()),
        };
        let (execution, state) = exchange(&format!("{PROGRESS}{END}"), &observer).await?;
        assert_eq!(state, ActionState::OutcomeUnknown);
        assert!(execution.result.is_none());
        assert_eq!(
            observer
                .observations
                .lock()
                .map_err(|_| "observation lock")?
                .len(),
            1
        );
        Ok(())
    }
}

fn json_reply(body: &[u8]) -> Vec<u8> {
    http(
        "200 OK",
        &format!("Content-Length: {}\r\n", body.len()),
        body,
    )
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

struct ObservationProbe {
    closed: Arc<tokio::sync::Notify>,
    ready_to_ack: tokio::sync::Notify,
    acknowledge: tokio::sync::Notify,
    attempts: AtomicUsize,
    fail: bool,
}

impl ObservationProbe {
    fn new(fail: bool) -> Self {
        Self {
            closed: Arc::new(tokio::sync::Notify::new()),
            ready_to_ack: tokio::sync::Notify::new(),
            acknowledge: tokio::sync::Notify::new(),
            attempts: AtomicUsize::new(0),
            fail,
        }
    }
}

impl InventoryObservationPort for ObservationProbe {
    fn persist<'a>(
        &'a self,
        observation: &'a InventoryObservation,
        inventory: &'a OllamaInventory,
    ) -> BoxFuture<'a, Result<(), InventoryObservationError>> {
        Box::pin(async move {
            // Count entry even if a repeated, invalid publication later times out.
            self.attempts.fetch_add(1, Ordering::SeqCst);
            tokio::time::timeout(Duration::from_secs(1), self.closed.notified())
                .await
                .map_err(|_| InventoryObservationError::Unavailable)?;
            let inventory_digest = unleash_protocol::canonical_sha256(inventory)
                .map_err(|_| InventoryObservationError::Unavailable)?;
            let value = serde_json::to_value(observation)
                .map_err(|_| InventoryObservationError::Unavailable)?;
            if observation.action_id() != ActionId::from_u128(303)
                || value.get("inventory_digest") != Some(&serde_json::json!(inventory_digest))
            {
                return Err(InventoryObservationError::Unavailable);
            }
            self.ready_to_ack.notify_one();
            if self.fail {
                return Err(InventoryObservationError::Unavailable);
            }
            self.acknowledge.notified().await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn inventory_durable_observation_waits_for_closed_channel_and_ack() -> TestResult {
    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(false));
    let mut fixture = Fixture::with_close_signal(
        // Once the GET arrives, delay its response beyond the connection bound
        // while keeping the whole exchange inside the independent total bound.
        Response::DelayedBytes(json_reply(br#"{"models":[]}"#), Duration::from_millis(450)),
        Arc::clone(&harness.dispatches),
        false,
        Some(Arc::clone(&observer.closed)),
    )
    .await?;
    let check = async {
        tokio::time::timeout(Duration::from_secs(2), observer.ready_to_ack.notified()).await?;
        assert_eq!(observer.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            harness.broker.action_state(ActionId::from_u128(303)),
            Some(ActionState::Dispatching)
        );
        harness.assert_one_dispatch_no_secret();
        observer.acknowledge.notify_one();
        Ok::<(), Box<dyn Error>>(())
    };
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &format!("http://127.0.0.1:{}/api/tags", fixture.port),
        inventory_profile()?,
        300,
        2500,
        128,
    )?;
    let execution = harness.run_observed(request, observer.clone());
    let (execution, witness) = tokio::join!(execution, check);
    witness?;
    assert!(listed(execution?)?.models.is_empty());
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 1);
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[tokio::test]
async fn inventory_v2_ends_a_stalled_tls_handshake_within_the_connection_window() -> TestResult {
    let tls = TlsFixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("https://{}/api/tags", listener.local_addr()?);
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &endpoint,
        inventory_profile()?,
        300,
        3000,
        128,
    )?
    .with_fixture_root(&tls.ca_der)?;
    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(false));
    let witness = async {
        let (socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
        let mut bytes = Vec::new();
        // The peer never responds. Observe EOF before the whole-exchange deadline
        // and require an actual ClientHello to rule out a pre-connect rejection.
        tokio::time::timeout(
            Duration::from_secs(1),
            socket.take(8192).read_to_end(&mut bytes),
        )
        .await??;
        assert_eq!(bytes.first(), Some(&0x16));
        assert!(bytes.len() < 8192, "read must reach peer EOF");
        Ok::<(), Box<dyn Error>>(())
    };
    let (execution, witness) =
        tokio::join!(harness.run_observed(request, observer.clone()), witness);
    witness?;
    let execution = execution?;
    assert_unknown(&execution);
    assert!(matches!(
        execution.outcome,
        InventoryOutcome::Failed(InventoryError::DeadlineExpired)
    ));
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 0);
    harness.assert_one_dispatch_no_secret();
    Ok(())
}

#[tokio::test]
async fn inventory_v2_requires_the_observation_port_before_dispatch() -> TestResult {
    let harness = Harness::new();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}/api/tags", listener.local_addr()?);
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &endpoint,
        inventory_profile()?,
        300,
        2500,
        128,
    )?;
    let outcome = harness.run(request).await;
    assert!(matches!(
        outcome
            .as_ref()
            .err()
            .and_then(|error| error.downcast_ref::<InventoryError>()),
        Some(InventoryError::ObservationUnavailable)
    ));
    assert_eq!(harness.dispatches.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn inventory_durable_observation_failure_is_unknown_without_second_get() -> TestResult {
    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(true));
    let mut fixture = Fixture::with_close_signal(
        json_response(br#"{"models":[]}"#),
        Arc::clone(&harness.dispatches),
        false,
        Some(Arc::clone(&observer.closed)),
    )
    .await?;
    let execution = harness
        .run_observed(request(fixture.port, 2000, 128)?, observer.clone())
        .await?;
    assert_unknown(&execution);
    assert!(matches!(
        execution.outcome,
        InventoryOutcome::Failed(InventoryError::ObservationUnavailable)
    ));
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 1);
    let decision = harness
        .broker
        .action_audit(ActionId::from_u128(303))
        .and_then(|audit| audit.decision_ref)
        .ok_or("missing decision after unknown")?;
    let repeated = execute_ollama_inventory_with_observation(
        request(fixture.port, 2000, 128)?,
        BrokerDispatch {
            broker: &harness.broker,
            action: ActionId::from_u128(303),
            decision: &decision,
            fence: harness.fence,
            authority: &harness.authority,
            clock: harness.clock.clone(),
            secrets: &harness.secrets,
        },
        observer.clone(),
    )
    .await?;
    assert!(repeated.receipt.is_err());
    assert_eq!(repeated.state, ActionState::OutcomeUnknown);
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 1);
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn t_f012_015_owned_inventory_waits_for_accept_before_http() -> TestResult {
    use crate::connect_transport::OwnedLoopbackPeer;
    use unleash_platform::inspect_process_identity;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let peer = listener.local_addr()?;
    let process = inspect_process_identity(std::process::id())?;
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &format!("http://{peer}/api/tags"),
        inventory_profile()?,
        250,
        500,
        1024,
    )?
    .with_owned_peer(OwnedLoopbackPeer::new(peer, process)?)?;
    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(false));
    let execution = harness.run_observed(request, observer.clone()).await?;

    assert_unknown(&execution);
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 0);
    harness.assert_one_dispatch_no_secret();
    let (mut client, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
    let bytes = tokio::time::timeout(Duration::from_secs(2), client.read(&mut [0_u8; 1])).await??;
    assert_eq!(
        bytes, 0,
        "GET reached a socket before native accept could be observed"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn t_f012_015_owned_inventory_gets_only_from_verified_peer() -> TestResult {
    use crate::connect_transport::OwnedLoopbackPeer;
    use unleash_platform::inspect_process_identity;

    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(false));
    let mut fixture = Fixture::with_close_signal(
        json_response(br#"{"models":[]}"#),
        Arc::clone(&harness.dispatches),
        false,
        Some(observer.closed.clone()),
    )
    .await?;
    let peer = std::net::SocketAddr::from(([127, 0, 0, 1], fixture.port));
    let process = inspect_process_identity(std::process::id())?;
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &format!("http://{peer}/api/tags"),
        inventory_profile()?,
        1_000,
        2_000,
        1024,
    )?
    .with_owned_peer(OwnedLoopbackPeer::new(peer, process)?)?;
    observer.acknowledge.notify_one();
    let execution = harness.run_observed(request, observer.clone()).await?;
    assert!(execution.observation.is_some());
    assert!(listed(execution)?.models.is_empty());
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 1);
    harness.assert_one_dispatch_no_secret();
    assert_get_only(&fixture.capture().await?, fixture.port)
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn t_f012_015_owned_inventory_refuses_changed_identity_without_connecting() -> TestResult {
    use crate::connect_transport::OwnedLoopbackPeer;
    use unleash_platform::inspect_process_identity;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let peer = listener.local_addr()?;
    let mut process = inspect_process_identity(std::process::id())?;
    process.owner_uid ^= 1;
    let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
        &format!("http://{peer}/api/tags"),
        inventory_profile()?,
        1_000,
        2_000,
        1024,
    )?
    .with_owned_peer(OwnedLoopbackPeer::new(peer, process)?)?;
    let harness = Harness::new();
    let observer = Arc::new(ObservationProbe::new(false));
    let execution = harness.run_observed(request, observer.clone()).await?;
    assert_unknown(&execution);
    assert!(matches!(
        execution.outcome,
        InventoryOutcome::Failed(InventoryError::OwnershipUnavailable)
    ));
    assert_eq!(observer.attempts.load(Ordering::SeqCst), 0);
    harness.assert_one_dispatch_no_secret();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}
