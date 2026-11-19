//! T-F012-007/008: transport accepts only explicitly pinned HTTPS targets.

use crate::openai::OpenAiOutcome;
use crate::transport::{BrokerDispatch, RetryAfter, execute_fixture, output_value};
use crate::transport::{FrozenRequest, ProfileRef, TransportError};
use secrecy::SecretString;
use std::{
    collections::BTreeSet, error::Error, fs, path::Path, process::Command, sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        crypto::aws_lc_rs,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    },
};
use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry, BrokerJournalError,
    BrokerJournalPort, EffectsClass, IsolationMode, IsolationRequirements,
    LocalAuthorityFallbackPolicy, NormalizedResourceRef, ScopeKind, SecretStore,
    compute_effective_authority,
};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    LineageFence, ModelPolicy, ReconcileStrategy, Revision, SecretRef, TimestampMs,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe, local_test_dir,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn t_f012_008_rejects_non_tls_target_before_dispatch() -> TestResult {
    assert!(
        FrozenRequest::new(
            "http://example.invalid/v1/responses",
            "fixture-model",
            "hello",
            profile()?,
            fixture_ref()?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn t_f012_008_request_digest_pins_endpoint_profile_and_input() -> TestResult {
    let digest = unleash_domain::Sha256Digest::from_bytes(b"fixture-profile");
    let one = FrozenRequest::new(
        "https://localhost:1234/v1/responses",
        "fixture-model",
        "hello",
        ProfileRef::new("profile", 1, digest)?,
        fixture_ref()?,
    )?
    .payload_digest()?;
    let changed_version = FrozenRequest::new(
        "https://localhost:1234/v1/responses",
        "fixture-model",
        "hello",
        ProfileRef::new("profile", 2, digest)?,
        fixture_ref()?,
    )?
    .payload_digest()?;
    let changed_endpoint = FrozenRequest::new(
        "https://localhost:1235/v1/responses",
        "fixture-model",
        "hello",
        ProfileRef::new("profile", 1, digest)?,
        fixture_ref()?,
    )?
    .payload_digest()?;
    let changed_input = FrozenRequest::new(
        "https://localhost:1234/v1/responses",
        "fixture-model",
        "other",
        ProfileRef::new("profile", 1, digest)?,
        fixture_ref()?,
    )?
    .payload_digest()?;
    let changed_secret = FrozenRequest::new(
        "https://localhost:1234/v1/responses",
        "fixture-model",
        "hello",
        ProfileRef::new("profile", 1, digest)?,
        SecretRef::new("other-fixture-ref")?,
    )?
    .payload_digest()?;
    assert_ne!(one, changed_version);
    assert_ne!(one, changed_endpoint);
    assert_ne!(one, changed_input);
    assert_ne!(one, changed_secret);
    assert!(ProfileRef::new("profile", 0, digest).is_err());
    Ok(())
}

fn profile() -> Result<ProfileRef, TransportError> {
    ProfileRef::new(
        "fixture-profile",
        1,
        unleash_domain::Sha256Digest::from_bytes(b"fixture-profile"),
    )
}

fn fixture_ref() -> Result<SecretRef, Box<dyn Error>> {
    Ok(SecretRef::new("fixture-credential-ref")?)
}

fn openssl(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let result = command.output()?;
    if result.status.success() {
        Ok(())
    } else {
        Err("fixture certificate generation failed".into())
    }
}

pub(crate) struct TlsFixture {
    _dir: tempfile::TempDir,
    pub(crate) ca: reqwest::Certificate,
    pub(crate) ca_der: Vec<u8>,
    pub(crate) server: Arc<ServerConfig>,
}

impl TlsFixture {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        // Ephemeral keys only: never a provider credential or installed trust root.
        let dir = local_test_dir("model-http-tls-fixture")?;
        let root = dir.path();
        let ca_key = root.join("ca.key");
        let ca_pem = root.join("ca.pem");
        let ca_der = root.join("ca.der");
        let server_key = root.join("server.key");
        let server_csr = root.join("server.csr");
        let server_pem = root.join("server.pem");
        let cert_der = root.join("server.der");
        let key_der = root.join("server-key.der");
        let config = root.join("fixture.cnf");
        fs::write(
            &config,
            "[req]\ndistinguished_name=dn\n[dn]\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n[server]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n",
        )?;
        key(&ca_key)?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["req", "-new", "-x509", "-key"])
                .arg(&ca_key)
                .args(["-out"])
                .arg(&ca_pem)
                .args(["-days", "1", "-subj", "/CN=Unleash fixture CA", "-config"])
                .arg(&config)
                .args(["-extensions", "ca"]),
        )?;
        key(&server_key)?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["req", "-new", "-key"])
                .arg(&server_key)
                .args(["-out"])
                .arg(&server_csr)
                .args(["-subj", "/CN=localhost"]),
        )?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["x509", "-req", "-in"])
                .arg(&server_csr)
                .args(["-CA"])
                .arg(&ca_pem)
                .args(["-CAkey"])
                .arg(&ca_key)
                .args(["-CAcreateserial", "-out"])
                .arg(&server_pem)
                .args(["-days", "1", "-sha256", "-extfile"])
                .arg(&config)
                .args(["-extensions", "server"]),
        )?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["x509", "-in"])
                .arg(&server_pem)
                .args(["-outform", "DER", "-out"])
                .arg(&cert_der),
        )?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
                .arg(&server_key)
                .args(["-outform", "DER", "-out"])
                .arg(&key_der),
        )?;
        openssl(
            Command::new("/usr/bin/openssl")
                .args(["x509", "-in"])
                .arg(&ca_pem)
                .args(["-outform", "DER", "-out"])
                .arg(&ca_der),
        )?;
        let ca = reqwest::Certificate::from_pem(&fs::read(ca_pem)?)?;
        let cert = CertificateDer::from(fs::read(cert_der)?);
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(fs::read(key_der)?));
        let server = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)?;
        Ok(Self {
            _dir: dir,
            ca,
            ca_der: fs::read(ca_der)?,
            server: Arc::new(server),
        })
    }
}

fn key(path: &Path) -> Result<(), Box<dyn Error>> {
    openssl(
        Command::new("/usr/bin/openssl")
            .args([
                "ecparam",
                "-name",
                "prime256v1",
                "-genkey",
                "-noout",
                "-out",
            ])
            .arg(path),
    )
}

#[derive(Debug)]
struct Capture {
    request: Vec<u8>,
    second_socket: bool,
}

fn serve(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    response: String,
) -> JoinHandle<Result<Capture, std::io::Error>> {
    tokio::spawn(async move {
        let (socket, _) = tokio::time::timeout(Duration::from_secs(4), listener.accept()).await??;
        let mut bytes = Vec::new();
        if let Ok(Ok(mut stream)) = tokio::time::timeout(
            Duration::from_secs(4),
            TlsAcceptor::from(tls).accept(socket),
        )
        .await
        {
            let mut buffer = [0_u8; 1024];
            while bytes.len() < 131_072 {
                let read =
                    tokio::time::timeout(Duration::from_secs(4), stream.read(&mut buffer)).await;
                let Ok(Ok(n)) = read else {
                    break;
                };
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(buffer.get(..n).ok_or(std::io::ErrorKind::InvalidData)?);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(
                        bytes.get(..end).ok_or(std::io::ErrorKind::InvalidData)?,
                    );
                    let len = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            if !bytes.is_empty() {
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
        }
        let second_socket = tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_ok();
        Ok(Capture {
            request: bytes,
            second_socket,
        })
    })
}

fn response(status: &str, headers: &str, body: &str, declared: usize) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {declared}\r\nConnection: close\r\n{headers}\r\n{body}"
    )
}

struct BrokerFixture {
    broker: ActionBroker,
    clock: Arc<ManualClock>,
    fence: LineageFence,
    secrets: SecretResolutionSpy<unleash_testkit::InMemoryTestSecretStore>,
    secret: SecretRef,
}

#[derive(Clone, Copy)]
enum JournalMode {
    AcceptFixture,
    RejectObservation,
}
struct FixtureJournal(JournalMode);
impl BrokerJournalPort for FixtureJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            if matches!(self.0, JournalMode::RejectObservation)
                && matches!(entry, BrokerJournalEntry::Observation { .. })
            {
                Err(BrokerJournalError)
            } else {
                Ok(())
            }
        })
    }
}

impl BrokerFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_journal(JournalMode::AcceptFixture)
    }
    fn with_journal(mode: JournalMode) -> Result<Self, Box<dyn Error>> {
        let clock = Arc::new(ManualClock::new(1_767_225_600_000));
        let fence = LineageFence::new(
            AuthorityGeneration::from_u128(2),
            DaemonEpoch::INITIAL,
            Fence::new(1),
        );
        let broker = ActionBroker::new(
            clock.clone(),
            clock.clone(),
            Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
            Arc::new(DeterministicIdGenerator::new(202)),
            Arc::new(TestIsolationProbe::standard_available()),
            Arc::new(BrokerExecutionSpy::new()),
            Arc::new(LocalAuthorityFallbackPolicy::default()),
            BrokerConfig::for_test(fence),
            Arc::new(FixtureJournal(mode)),
        );
        let secrets = SecretResolutionSpy::in_memory();
        let secret = SecretRef::new("fixture-credential-ref")?;
        secrets.store(
            &secret,
            SecretString::from("fixture-token-not-a-secret".to_owned()),
        )?;
        Ok(Self {
            broker,
            clock,
            fence,
            secrets,
            secret,
        })
    }
}

fn authority(
    host: &str,
    allow: bool,
    secret: Option<&SecretRef>,
) -> unleash_action_broker::EffectiveAuthority {
    let layer = AuthorityLayer {
        layer_name: "fixture".into(),
        allowed_effects: Some(if allow {
            BTreeSet::from([EffectsClass::ModelDispatch])
        } else {
            BTreeSet::new()
        }),
        denied_effects: BTreeSet::new(),
        allowed_verbs: Some(BTreeSet::from(["model.responses.stream".into()])),
        denied_verbs: BTreeSet::new(),
        allowed_roots: Some(Vec::new()),
        allowed_network_hosts: Some(BTreeSet::from([host.into()])),
        allowed_secret_refs: Some(secret.map_or_else(BTreeSet::new, |s| {
            BTreeSet::from([s.as_opaque_str().to_owned()])
        })),
        model_policy: ModelPolicy::AllowedBounded,
        data_locality: DataLocality::ApprovedRemote,
        isolation_mode: IsolationMode::Standard,
        require_action_isolation: false,
        max_scope_kind: ScopeKind::Global,
        allow_delegation: false,
        allow_cross_repository: false,
    };
    compute_effective_authority(None, Revision::INITIAL, Revision::new(1), &[layer])
}

fn intent(
    request: &FrozenRequest,
    host: &str,
    port: u16,
    fence: LineageFence,
    authority: &unleash_action_broker::EffectiveAuthority,
    secret: Option<SecretRef>,
) -> Result<ActionIntent, Box<dyn Error>> {
    Ok(ActionIntent {
        action_id: ActionId::from_u128(100),
        subject: ActionSubject::new_control_plane(
            ActorId::new("fixture")?,
            "fixture-receipt".into(),
        )?,
        project_id: None,
        verb: "model.responses.stream".into(),
        resource_refs: vec![
            NormalizedResourceRef::ModelTarget {
                provider: "openai".into(),
                model_id: "fixture-model".into(),
                locality: DataLocality::ApprovedRemote,
            },
            NormalizedResourceRef::NetworkEndpoint {
                host: host.into(),
                port: Some(port),
                is_private_or_loopback: true,
            },
        ],
        payload_digest: request.payload_digest()?,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ModelDispatch,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: fence,
        action_deadline: TimestampMs::new(1_767_225_660_000),
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: secret.into_iter().collect(),
        provenance: ActionProvenance::EngineInternal,
    })
}

async fn authorized(
    b: &BrokerFixture,
    i: ActionIntent,
    a: &unleash_action_broker::EffectiveAuthority,
) -> Result<unleash_action_broker::AuthorizationDecision, Box<dyn Error>> {
    match b.broker.authorize_action(i, a).await? {
        AuthorizationOutcome::Authorized(decision) => Ok(decision),
        AuthorizationOutcome::ApprovalRequired { .. } => Err("unexpected approval".into()),
    }
}

#[tokio::test]
async fn t_f012_004_tls_response_observed_only_after_broker_receipt() -> TestResult {
    let tls = TlsFixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let body = include_str!("../fixtures/openai-responses-2026-11-18-text.sse");
    let worker = serve(
        listener,
        tls.server.clone(),
        response(
            "200 OK",
            "Content-Type: text/event-stream\r\nx-request-id: req_fixture\r\n",
            body,
            body.len(),
        ),
    );
    let broker = BrokerFixture::new()?;
    let request = FrozenRequest::new(
        &format!("https://localhost:{port}/v1/responses"),
        "fixture-model",
        "hello",
        profile()?,
        fixture_ref()?,
    )?;
    let auth = authority("localhost", true, Some(&broker.secret));
    let action = intent(
        &request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(broker.secret.clone()),
    )?;
    let decision = authorized(&broker, action.clone(), &auth).await?;
    let result = execute_fixture(
        request,
        BrokerDispatch {
            broker: &broker.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: broker.fence,
            authority: &auth,
            clock: broker.clock.clone(),
            secrets: &broker.secrets,
        },
        Some(tls.ca.clone()),
    )
    .await?;
    let receipt = result.receipt?;
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    assert_eq!(receipt.action_id, action.action_id);
    let outcome = result.outcome?.ok_or("missing observed result")?;
    assert!(
        matches!(&outcome, OpenAiOutcome::Success { text, tools, .. } if text == "Héllo" && tools.is_empty())
    );
    assert_eq!(
        receipt.output_digest,
        Some(unleash_protocol::canonical_sha256(&output_value(&outcome))?)
    );
    assert_eq!(receipt.remote_audit_ref.as_deref(), Some("req_fixture"));
    let seen = worker.await??;
    let request = String::from_utf8(seen.request)?;
    assert!(request.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .ok_or("missing request body")?;
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body)?,
        serde_json::json!({"model":"fixture-model","input":"hello","stream":true,"store":false,"max_output_tokens":256})
    );
    assert!(!seen.second_socket);
    Ok(())
}

#[tokio::test]
async fn t_f012_004_failed_broker_observation_hides_tls_result() -> TestResult {
    let tls = TlsFixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let body = include_str!("../fixtures/openai-responses-2026-11-18-text.sse");
    let worker = serve(
        listener,
        tls.server.clone(),
        response(
            "200 OK",
            "Content-Type: text/event-stream\r\n",
            body,
            body.len(),
        ),
    );
    let broker = BrokerFixture::with_journal(JournalMode::RejectObservation)?;
    let request = FrozenRequest::new(
        &format!("https://localhost:{port}/v1/responses"),
        "fixture-model",
        "hello",
        profile()?,
        fixture_ref()?,
    )?;
    let auth = authority("localhost", true, Some(&broker.secret));
    let action = intent(
        &request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(broker.secret.clone()),
    )?;
    let decision = authorized(&broker, action.clone(), &auth).await?;
    let result = execute_fixture(
        request,
        BrokerDispatch {
            broker: &broker.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: broker.fence,
            authority: &auth,
            clock: broker.clock.clone(),
            secrets: &broker.secrets,
        },
        Some(tls.ca.clone()),
    )
    .await?;
    assert!(result.receipt.is_err());
    assert!(result.outcome?.is_none());
    let seen = worker.await??;
    assert!(seen.request.starts_with(b"POST /v1/responses "));
    assert!(!seen.second_socket);
    Ok(())
}

#[tokio::test]
async fn t_f012_007_unknown_ca_and_wrong_name_never_send_post() -> TestResult {
    let tls = TlsFixture::new()?;
    for (host, ca) in [("localhost", None), ("127.0.0.1", Some(tls.ca.clone()))] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let worker = serve(listener, tls.server.clone(), response("200 OK", "", "", 0));
        let broker = BrokerFixture::new()?;
        let request = FrozenRequest::new(
            &format!("https://{host}:{port}/v1/responses"),
            "fixture-model",
            "hello",
            profile()?,
            fixture_ref()?,
        )?;
        let auth = authority(host, true, Some(&broker.secret));
        let action = intent(
            &request,
            host,
            port,
            broker.fence,
            &auth,
            Some(broker.secret.clone()),
        )?;
        let decision = authorized(&broker, action.clone(), &auth).await?;
        let result = execute_fixture(
            request,
            BrokerDispatch {
                broker: &broker.broker,
                action: action.action_id,
                decision: &decision.decision_ref,
                fence: broker.fence,
                authority: &auth,
                clock: broker.clock.clone(),
                secrets: &broker.secrets,
            },
            ca,
        )
        .await?;
        assert!(matches!(
            result.receipt,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        assert!(result.outcome?.is_none());
        let seen = worker.await??;
        assert!(seen.request.is_empty());
        assert!(!seen.second_socket);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_007_rate_limit_and_interruption_do_not_retry() -> TestResult {
    let tls = TlsFixture::new()?;
    for (status, headers, body, expected_rate) in [
        (
            "429 Too Many Requests",
            "Retry-After: 17\r\n",
            "",
            Some(RetryAfter::Seconds(17)),
        ),
        (
            "429 Too Many Requests",
            "Retry-After: 301\r\n",
            "",
            Some(RetryAfter::BeyondLimit),
        ),
        (
            "429 Too Many Requests",
            "Retry-After: invalid\r\n",
            "",
            Some(RetryAfter::Unavailable),
        ),
        (
            "200 OK",
            "Content-Type: text/event-stream\r\n",
            "event: response.created\ndata: {",
            None,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let worker = serve(
            listener,
            tls.server.clone(),
            response(status, headers, body, body.len()),
        );
        let broker = BrokerFixture::new()?;
        let request = FrozenRequest::new(
            &format!("https://localhost:{port}/v1/responses"),
            "fixture-model",
            "hello",
            profile()?,
            fixture_ref()?,
        )?;
        let auth = authority("localhost", true, Some(&broker.secret));
        let action = intent(
            &request,
            "localhost",
            port,
            broker.fence,
            &auth,
            Some(broker.secret.clone()),
        )?;
        let decision = authorized(&broker, action.clone(), &auth).await?;
        let result = execute_fixture(
            request,
            BrokerDispatch {
                broker: &broker.broker,
                action: action.action_id,
                decision: &decision.decision_ref,
                fence: broker.fence,
                authority: &auth,
                clock: broker.clock.clone(),
                secrets: &broker.secrets,
            },
            Some(tls.ca.clone()),
        )
        .await?;
        assert!(matches!(
            result.receipt,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        assert!(result.outcome?.is_none());
        let rate_limited = result.rate_limited?;
        assert_eq!(rate_limited.map(|value| value.retry_after), expected_rate);
        if let Some(observed) = rate_limited {
            assert_eq!(observed.action_id, action.action_id);
            assert!(broker.broker.audit_events().iter().any(|event| matches!(event, unleash_action_broker::BrokerAuditEvent::ActionDispatchCommitted { action_id, dispatch_id, .. } if *action_id == observed.action_id && *dispatch_id == observed.dispatch_id)));
        }
        let seen = worker.await??;
        assert!(seen.request.starts_with(b"POST /v1/responses "));
        assert!(!seen.second_socket);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_005_tls_model_outcomes_stay_distinct() -> TestResult {
    let tls = TlsFixture::new()?;
    for (name, body) in [
        (
            "refusal",
            include_str!("../fixtures/openai-responses-2026-11-18-refusal.sse"),
        ),
        (
            "truncated",
            include_str!("../fixtures/openai-responses-2026-11-18-incomplete.sse"),
        ),
        (
            "schema",
            include_str!("../fixtures/openai-responses-2026-11-18-function.sse"),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let worker = serve(
            listener,
            tls.server.clone(),
            response(
                "200 OK",
                "Content-Type: text/event-stream\r\n",
                body,
                body.len(),
            ),
        );
        let broker = BrokerFixture::new()?;
        let request = FrozenRequest::new(
            &format!("https://localhost:{port}/v1/responses"),
            "fixture-model",
            "hello",
            profile()?,
            fixture_ref()?,
        )?;
        let auth = authority("localhost", true, Some(&broker.secret));
        let action = intent(
            &request,
            "localhost",
            port,
            broker.fence,
            &auth,
            Some(broker.secret.clone()),
        )?;
        let decision = authorized(&broker, action.clone(), &auth).await?;
        let result = execute_fixture(
            request,
            BrokerDispatch {
                broker: &broker.broker,
                action: action.action_id,
                decision: &decision.decision_ref,
                fence: broker.fence,
                authority: &auth,
                clock: broker.clock.clone(),
                secrets: &broker.secrets,
            },
            Some(tls.ca.clone()),
        )
        .await?;
        assert_eq!(result.receipt?.state, ActionState::ObservedSuccess);
        assert!(matches!(
            (name, result.outcome?),
            ("refusal", Some(OpenAiOutcome::Refusal { .. }))
                | ("truncated", Some(OpenAiOutcome::Truncated { .. }))
                | ("schema", Some(OpenAiOutcome::InvalidSchema { .. }))
        ));
        assert!(!worker.await??.second_socket);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_008_broker_refusal_precedes_socket() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let broker = BrokerFixture::new()?;
    let request = FrozenRequest::new(
        &format!("https://localhost:{port}/v1/responses"),
        "fixture-model",
        "hello",
        profile()?,
        fixture_ref()?,
    )?;
    let auth = authority("localhost", false, Some(&broker.secret));
    let action = intent(
        &request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(broker.secret.clone()),
    )?;
    assert!(broker.broker.authorize_action(action, &auth).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_008_changed_frozen_request_fails_before_socket() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let broker = BrokerFixture::new()?;
    let endpoint = format!("https://localhost:{port}/v1/responses");
    let authorized_request = FrozenRequest::new(
        &endpoint,
        "fixture-model",
        "hello",
        profile()?,
        fixture_ref()?,
    )?;
    let changed_request = FrozenRequest::new(
        &endpoint,
        "fixture-model",
        "changed",
        profile()?,
        fixture_ref()?,
    )?;
    let auth = authority("localhost", true, Some(&broker.secret));
    let action = intent(
        &authorized_request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(broker.secret.clone()),
    )?;
    let decision = authorized(&broker, action.clone(), &auth).await?;
    let result = execute_fixture(
        changed_request,
        BrokerDispatch {
            broker: &broker.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: broker.fence,
            authority: &auth,
            clock: broker.clock.clone(),
            secrets: &broker.secrets,
        },
        None,
    )
    .await?;
    assert!(matches!(
        result.receipt,
        Err(ActionBrokerError::ContractViolation { .. })
    ));
    assert!(result.outcome?.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_008_profile_secret_reference_mismatch_has_no_post() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let broker = BrokerFixture::new()?;
    let wrong = SecretRef::new("other-fixture-ref")?;
    broker
        .secrets
        .store(&wrong, SecretString::from("other-fixture-token".to_owned()))?;
    let request = FrozenRequest::new(
        &format!("https://localhost:{port}/v1/responses"),
        "fixture-model",
        "hello",
        profile()?,
        broker.secret.clone(),
    )?;
    let auth = authority("localhost", true, Some(&wrong));
    let action = intent(
        &request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(wrong),
    )?;
    let decision = authorized(&broker, action.clone(), &auth).await?;
    let result = execute_fixture(
        request,
        BrokerDispatch {
            broker: &broker.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: broker.fence,
            authority: &auth,
            clock: broker.clock.clone(),
            secrets: &broker.secrets,
        },
        None,
    )
    .await?;
    assert!(matches!(
        result.receipt,
        Err(ActionBrokerError::ContractViolation { .. })
    ));
    assert!(result.outcome?.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_008_redirect_does_not_forward_body_or_fixture_credential() -> TestResult {
    let tls = TlsFixture::new()?;
    let source = TcpListener::bind("127.0.0.1:0").await?;
    let destination = TcpListener::bind("127.0.0.1:0").await?;
    let port = source.local_addr()?.port();
    let location = format!(
        "Location: https://localhost:{}/v1/responses\r\n",
        destination.local_addr()?.port()
    );
    let worker = serve(
        source,
        tls.server.clone(),
        response("307 Temporary Redirect", &location, "", 0),
    );
    let broker = BrokerFixture::new()?;
    let request = FrozenRequest::new(
        &format!("https://localhost:{port}/v1/responses"),
        "fixture-model",
        "hello",
        profile()?,
        fixture_ref()?,
    )?;
    let auth = authority("localhost", true, Some(&broker.secret));
    let action = intent(
        &request,
        "localhost",
        port,
        broker.fence,
        &auth,
        Some(broker.secret.clone()),
    )?;
    let decision = authorized(&broker, action.clone(), &auth).await?;
    let result = execute_fixture(
        request,
        BrokerDispatch {
            broker: &broker.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: broker.fence,
            authority: &auth,
            clock: broker.clock.clone(),
            secrets: &broker.secrets,
        },
        Some(tls.ca.clone()),
    )
    .await?;
    assert!(matches!(
        result.receipt,
        Err(ActionBrokerError::OutcomeUnknown { .. })
    ));
    assert!(result.outcome?.is_none());
    let seen = worker.await??;
    assert!(
        String::from_utf8(seen.request)?
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-token-not-a-secret")
    );
    assert!(!seen.second_socket);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), destination.accept())
            .await
            .is_err()
    );
    Ok(())
}
