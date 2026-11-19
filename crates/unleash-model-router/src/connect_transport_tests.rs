//! T-F012-007/013/015: real loopback sockets, synthetic resolver/CA and memory broker.
//! This bench does not qualify `SQLite` admission, mandate issuance or a provider.

use crate::connect_transport::{
    CONNECT_VERB, CertificateValidation, ConnectError, FixtureOptions, FrozenConnectRequest,
    NameResolver, execute_fixture,
};
use crate::transport::{BrokerDispatch, ProfileRef};
use crate::transport_tests::TlsFixture;
use secrecy::SecretString;
use std::{
    collections::BTreeSet,
    error::Error,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::{TlsAcceptor, rustls::ServerConfig};
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationDecision, AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry,
    BrokerJournalError, BrokerJournalPort, EffectiveAuthority, EffectsClass, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, ScopeKind, SecretStore,
    compute_effective_authority,
};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    LineageFence, ModelPolicy, ReconcileStrategy, Revision, SecretRef, Sha256Digest, TimestampMs,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe,
};

type TestResult = Result<(), Box<dyn Error>>;

struct Resolver {
    addresses: Vec<SocketAddr>,
    calls: AtomicUsize,
    rebind: bool,
}
impl Resolver {
    fn pinned(port: u16) -> Arc<Self> {
        Arc::new(Self {
            addresses: vec![SocketAddr::from(([127, 0, 0, 1], port))],
            calls: AtomicUsize::new(0),
            rebind: true,
        })
    }
}
impl NameResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, ConnectError>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.rebind && call > 0 {
                Ok(vec![SocketAddr::from(([192, 0, 2, 1], port))])
            } else {
                Ok(self.addresses.clone())
            }
        })
    }
}

struct Journal {
    dispatches: Arc<AtomicUsize>,
    observations: Arc<AtomicUsize>,
}
impl BrokerJournalPort for Journal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            if matches!(entry, BrokerJournalEntry::Dispatch { .. }) {
                self.dispatches.fetch_add(1, Ordering::SeqCst);
            }
            if matches!(entry, BrokerJournalEntry::Observation { .. }) {
                self.observations.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        })
    }
}

struct ConnectHarness {
    broker: ActionBroker,
    clock: Arc<ManualClock>,
    fence: LineageFence,
    secrets: SecretResolutionSpy<unleash_testkit::InMemoryTestSecretStore>,
    dispatches: Arc<AtomicUsize>,
}
impl ConnectHarness {
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
            Arc::new(DeterministicIdGenerator::new(202)),
            Arc::new(TestIsolationProbe::standard_available()),
            Arc::new(BrokerExecutionSpy::new()),
            Arc::new(LocalAuthorityFallbackPolicy::default()),
            BrokerConfig::for_test(fence),
            Arc::new(Journal {
                dispatches: Arc::clone(&dispatches),
                observations: Arc::new(AtomicUsize::new(0)),
            }),
        );
        Self {
            broker,
            clock,
            fence,
            secrets: SecretResolutionSpy::in_memory(),
            dispatches,
        }
    }
    fn dispatch<'a>(
        &'a self,
        action: &ActionIntent,
        decision: &'a AuthorizationDecision,
        authority: &'a EffectiveAuthority,
    ) -> BrokerDispatch<'a> {
        BrokerDispatch {
            broker: &self.broker,
            action: action.action_id,
            decision: &decision.decision_ref,
            fence: self.fence,
            authority,
            clock: self.clock.clone(),
            secrets: &self.secrets,
        }
    }
}

fn authority() -> EffectiveAuthority {
    compute_effective_authority(
        None,
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: "synthetic-connect-fixture".to_owned(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([CONNECT_VERB.to_owned()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from([
                "localhost".to_owned(),
                "127.0.0.1".to_owned(),
            ])),
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
    scheme: &str,
    host: &str,
    port: u16,
    timeout: u64,
) -> Result<FrozenConnectRequest, Box<dyn Error>> {
    Ok(FrozenConnectRequest::new_loopback(
        &format!("{scheme}://{host}:{port}/unchanged/path"),
        ProfileRef::new(
            "synthetic-profile",
            1,
            Sha256Digest::from_bytes(b"profile fixture"),
        )?,
        timeout,
    )?)
}

fn intent(
    request: &FrozenConnectRequest,
    broker: &ConnectHarness,
    authority: &EffectiveAuthority,
) -> Result<ActionIntent, Box<dyn Error>> {
    Ok(ActionIntent {
        action_id: ActionId::from_u128(100),
        subject: ActionSubject::new_control_plane(
            ActorId::new("connect-fixture")?,
            "synthetic-fixture-cause".to_owned(),
        )?,
        project_id: None,
        verb: CONNECT_VERB.to_owned(),
        resource_refs: vec![request.resource()],
        payload_digest: request.payload_digest()?,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::NetworkEgress,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: broker.fence,
        action_deadline: TimestampMs::new(1_767_225_660_000),
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    })
}

async fn authorize(
    broker: &ConnectHarness,
    action: ActionIntent,
    authority: &EffectiveAuthority,
) -> Result<AuthorizationDecision, Box<dyn Error>> {
    match broker.broker.authorize_action(action, authority).await? {
        AuthorizationOutcome::Authorized(decision) => Ok(decision),
        AuthorizationOutcome::ApprovalRequired { .. } => Err("unexpected fixture approval".into()),
    }
}

struct Capture {
    tls: bool,
    server_name: Option<String>,
    bytes: Vec<u8>,
    extra_connection: bool,
}

fn serve(
    listener: TcpListener,
    tls: Option<Arc<ServerConfig>>,
) -> JoinHandle<Result<Capture, Box<dyn Error + Send + Sync>>> {
    tokio::spawn(async move {
        let (mut socket, _) =
            tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
        let mut bytes = Vec::new();
        let (tls_ok, server_name) = if let Some(tls) = tls {
            match tokio::time::timeout(
                Duration::from_secs(2),
                TlsAcceptor::from(tls).accept(socket),
            )
            .await?
            {
                Ok(mut stream) => {
                    let name = stream.get_ref().1.server_name().map(str::to_owned);
                    let result = tokio::time::timeout(
                        Duration::from_secs(2),
                        stream.read_to_end(&mut bytes),
                    )
                    .await?;
                    if let Err(error) = result
                        && error.kind() != ErrorKind::UnexpectedEof
                    {
                        return Err(error.into());
                    }
                    (true, name)
                }
                Err(_) => (false, None),
            }
        } else {
            tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes)).await??;
            (false, None)
        };
        let extra_connection = tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_ok();
        Ok(Capture {
            tls: tls_ok,
            server_name,
            bytes,
            extra_connection,
        })
    })
}

#[tokio::test]
async fn t_f012_013_connect_rejects_secret_ref_before_broker_resolution() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let broker = ConnectHarness::new();
    let secret = SecretRef::new("synthetic-connect-credential-ref")?;
    broker.secrets.store(
        &secret,
        SecretString::from("synthetic-connect-not-a-live-secret".to_owned()),
    )?;
    let mut authority = authority();
    authority
        .allowed_secret_refs
        .insert(secret.as_opaque_str().to_owned());
    let request = request("http", "127.0.0.1", port, 1000)?;
    let mut action = intent(&request, &broker, &authority)?;
    action.secret_refs.push(secret);
    let decision = authorize(&broker, action.clone(), &authority).await?;
    let resolver = Resolver::pinned(port);
    let result = execute_fixture(
        request,
        broker.dispatch(&action, &decision, &authority),
        FixtureOptions {
            resolver: resolver.clone(),
            ca_der: None,
        },
    )
    .await;
    assert_eq!(
        (
            broker.secrets.resolution_attempts(),
            broker.secrets.successful_resolutions()
        ),
        (0, 0),
        "connect ne doit jamais entrer dans le coffre, même pour une ref autorisée et résoluble"
    );
    assert!(matches!(result, Err(ConnectError::InvalidRequest)));
    assert_eq!(broker.dispatches.load(Ordering::SeqCst), 0);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        broker.broker.action_state(action.action_id),
        Some(ActionState::Authorized)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_013_connect_tcp_pins_one_resolution_and_sends_no_http() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let peer = listener.local_addr()?;
    let worker = serve(listener, None);
    let resolver = Resolver::pinned(peer.port());
    let broker = ConnectHarness::new();
    let authority = authority();
    let request = request("http", "127.0.0.1", peer.port(), 1000)?;
    let action = intent(&request, &broker, &authority)?;
    let decision = authorize(&broker, action.clone(), &authority).await?;
    let execution = execute_fixture(
        request,
        broker.dispatch(&action, &decision, &authority),
        FixtureOptions {
            resolver: resolver.clone(),
            ca_der: None,
        },
    )
    .await?;
    assert_eq!(execution.receipt?.state, ActionState::ObservedSuccess);
    let observed = execution.observation?.ok_or("missing TCP observation")?;
    assert_eq!(observed.peer, peer);
    assert_eq!(observed.http_application_bytes_sent, 0);
    assert_eq!(
        observed.certificate_validation,
        CertificateValidation::NotTls
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(broker.dispatches.load(Ordering::SeqCst), 1);
    broker.secrets.assert_zero_secret_resolutions();
    let capture = worker.await?.map_err(|error| error.to_string())?;
    assert!(capture.bytes.is_empty());
    assert!(!capture.extra_connection);
    Ok(())
}

#[tokio::test]
async fn t_f012_013_connect_tls_uses_profile_name_with_fixture_ca_and_no_http() -> TestResult {
    let tls = TlsFixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let peer = listener.local_addr()?;
    let worker = serve(listener, Some(tls.server.clone()));
    let resolver = Resolver::pinned(peer.port());
    let broker = ConnectHarness::new();
    let authority = authority();
    let request = request("https", "localhost", peer.port(), 1000)?;
    let action = intent(&request, &broker, &authority)?;
    let decision = authorize(&broker, action.clone(), &authority).await?;
    let execution = execute_fixture(
        request,
        broker.dispatch(&action, &decision, &authority),
        FixtureOptions {
            resolver: resolver.clone(),
            ca_der: Some(tls.ca_der.clone()),
        },
    )
    .await?;
    let receipt = execution.receipt?;
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    let observed = execution.observation?.ok_or("missing TLS observation")?;
    assert_eq!(
        observed.certificate_validation,
        CertificateValidation::FixtureRoot
    );
    assert!(!observed.revocation_checked);
    assert_eq!(observed.http_application_bytes_sent, 0);
    assert_eq!(
        receipt.output_digest,
        Some(unleash_protocol::canonical_sha256(&observed)?)
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    broker.secrets.assert_zero_secret_resolutions();
    let capture = worker.await?.map_err(|error| error.to_string())?;
    assert!(capture.tls);
    assert_eq!(capture.server_name.as_deref(), Some("localhost"));
    assert!(capture.bytes.is_empty());
    assert!(!capture.extra_connection);
    Ok(())
}

#[tokio::test]
async fn t_f012_015_connect_checks_every_address_before_first_socket() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let resolver = Arc::new(Resolver {
        addresses: vec![
            SocketAddr::from(([127, 0, 0, 1], port)),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), port),
        ],
        calls: AtomicUsize::new(0),
        rebind: false,
    });
    let broker = ConnectHarness::new();
    let authority = authority();
    let request = request("http", "127.0.0.1", port, 1000)?;
    let action = intent(&request, &broker, &authority)?;
    let decision = authorize(&broker, action.clone(), &authority).await?;
    let execution = execute_fixture(
        request,
        broker.dispatch(&action, &decision, &authority),
        FixtureOptions {
            resolver: resolver.clone(),
            ca_der: None,
        },
    )
    .await?;
    assert!(
        !execution
            .receipt
            .is_ok_and(|receipt| receipt.state == ActionState::ObservedSuccess)
    );
    assert!(execution.observation?.is_none());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    broker.secrets.assert_zero_secret_resolutions();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn t_f012_013_connect_rejects_unknown_fixture_ca_and_wrong_name_without_http() -> TestResult {
    let tls = TlsFixture::new()?;
    let other = TlsFixture::new()?;
    for wrong_name in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let worker = serve(listener, Some(tls.server.clone()));
        let resolver = Resolver::pinned(port);
        let broker = ConnectHarness::new();
        let authority = authority();
        let request = request(
            "https",
            if wrong_name { "127.0.0.1" } else { "localhost" },
            port,
            1000,
        )?;
        let action = intent(&request, &broker, &authority)?;
        let decision = authorize(&broker, action.clone(), &authority).await?;
        let ca = if wrong_name {
            tls.ca_der.clone()
        } else {
            other.ca_der.clone()
        };
        let execution = execute_fixture(
            request,
            broker.dispatch(&action, &decision, &authority),
            FixtureOptions {
                resolver,
                ca_der: Some(ca),
            },
        )
        .await?;
        assert!(
            !execution
                .receipt
                .is_ok_and(|receipt| receipt.state == ActionState::ObservedSuccess)
        );
        assert!(execution.observation?.is_none());
        broker.secrets.assert_zero_secret_resolutions();
        let capture = worker.await?.map_err(|error| error.to_string())?;
        assert!(!capture.tls);
        assert!(capture.bytes.is_empty());
        assert!(!capture.extra_connection);
    }
    Ok(())
}

#[tokio::test]
async fn t_f012_007_connect_tls_deadline_closes_without_http_or_retry() -> TestResult {
    let tls = TlsFixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let worker = tokio::spawn(async move {
        let (mut socket, _) =
            tokio::time::timeout(Duration::from_secs(2), listener.accept()).await??;
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes)).await??;
        let extra = tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_ok();
        Ok::<_, Box<dyn Error + Send + Sync>>((bytes, extra))
    });
    let resolver = Resolver::pinned(port);
    let broker = ConnectHarness::new();
    let authority = authority();
    let request = request("https", "localhost", port, 100)?;
    let action = intent(&request, &broker, &authority)?;
    let decision = authorize(&broker, action.clone(), &authority).await?;
    let started = tokio::time::Instant::now();
    let execution = execute_fixture(
        request,
        broker.dispatch(&action, &decision, &authority),
        FixtureOptions {
            resolver,
            ca_der: Some(tls.ca_der.clone()),
        },
    )
    .await?;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        !execution
            .receipt
            .is_ok_and(|receipt| receipt.state == ActionState::ObservedSuccess)
    );
    assert!(execution.observation?.is_none());
    let (bytes, extra) = worker.await?.map_err(|error| error.to_string())?;
    broker.secrets.assert_zero_secret_resolutions();
    assert!(
        only_handshake_records(&bytes),
        "only TLS handshake was attempted"
    );
    assert!(!extra);
    Ok(())
}

fn only_handshake_records(mut bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    while !bytes.is_empty() {
        let Some([0x16, 0x03, _, high, low]) = bytes.get(..5) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([*high, *low]));
        if length == 0 || bytes.get(5..5 + length).is_none() {
            return false;
        }
        let Some(remaining) = bytes.get(5 + length..) else {
            return false;
        };
        bytes = remaining;
    }
    true
}
