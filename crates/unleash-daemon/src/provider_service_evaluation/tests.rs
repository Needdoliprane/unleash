//! Real broker evaluation and `SQLite` journal on inert, local fixtures.

use super::*;
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, CommandId, DaemonEpoch, Fence, HostId,
    ProcessId, ProcessRunUuid, RequestId, Revision, Sha256Digest,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, EnvironmentMode, IsolationMode as HostIsolationMode,
    IsolationProfileKind, ResourceKind, StrictIsolationSpec, effective_environment_digest_v1,
    explicit_environment_v1, inspect_strict_service_launch_plan,
};
use unleash_protocol::{ProtocolVersion, ReceiptState, canonical_sha256, canonicalize_to_vec};
use unleash_store::{
    AggregateKind, CallerRole, CommandMutation, CommitCommandRequest, PrepareObjectRequest,
    ProviderProfileConfiguration, ProviderProfilePreparation, PublishObjectReferenceRequest,
    StoreConfig,
};
use unleash_testkit::{TestIsolationProbe, local_test_dir};
use uuid::Uuid;

mod generation_ipc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PROFILE_ID: Uuid = Uuid::from_u128(0xf012_0519);
const LAUNCH_ID: Uuid = Uuid::from_u128(0xf012_0520);

struct Fixture {
    store: Arc<UnleashStore>,
    directory: tempfile::TempDir,
    command: CommandEnvelope,
    caller: CallerAuthority,
    action: ActionId,
    bootstrap: HostBootstrapSpec,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        Self::with_bootstrap(|_| {})
    }

    fn with_bootstrap(adapt: impl FnOnce(&mut HostBootstrapSpec)) -> TestResult<Self> {
        Self::with_profile("http://127.0.0.1:11434", |_, _| Ok(()), adapt)
    }

    fn with_profile(
        endpoint: &str,
        before_launch: impl FnOnce(&UnleashStore, &CallerAuthority) -> TestResult,
        adapt: impl FnOnce(&mut HostBootstrapSpec),
    ) -> TestResult<Self> {
        let directory = local_test_dir("f012-service-evaluation")?;
        let mut config = StoreConfig::new(directory.path());
        config.boot_identity = unleash_platform::current_boot_identity()?.as_str().into();
        config.owner_uid = unleash_platform::effective_uid();
        let store = Arc::new(UnleashStore::open(config)?);
        let instance =
            unleash_domain::DaemonInstanceId::from_uuid(store.config().id_generator.new_uuid());
        let (_, previous, event) = crate::recovery::enter_recovering_and_emit_event(
            &store,
            &store.config().boot_identity,
            instance,
        )?;
        crate::recovery::classify_and_finish_recovery(&store, previous, event.event_id)?;
        let caller = CallerAuthority {
            actor_id: ActorId::new("user:service-evaluation-fixture")?,
            role: CallerRole::HumanOperator,
        };
        store.register_principal(
            "service-evaluation-fixture",
            "user",
            caller.actor_id.as_str(),
        )?;
        configure_profile(&store, &caller, endpoint)?;
        before_launch(&store, &caller)?;
        let (command, bootstrap) = prepare_service_fixture(&store, &caller, directory.path(), adapt)?;
        Ok(Self {
            store,
            directory,
            command,
            caller,
            action: bootstrap.action_id,
            bootstrap,
        })
    }

    fn claim_permit(&self) -> TestResult<ProviderServiceLaunchPermit> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let AuthorizationOutcome::Authorized(decision) =
            runtime.block_on(self.evaluate(Arc::new(TestIsolationProbe::standard_available())))?
        else {
            return Err("fixture evaluation should authorize".into());
        };
        Ok(runtime.block_on(claim_prepared_provider_service_launch(
            Arc::clone(&self.store),
            self.command.clone(),
            self.caller.clone(),
            self.bootstrap.clone(),
            decision,
        ))?)
    }

    async fn evaluate(
        &self,
        probe: Arc<dyn IsolationProbePort>,
    ) -> Result<AuthorizationOutcome, ProviderServiceEvaluationError> {
        self.evaluate_bootstrap(self.bootstrap.clone(), probe).await
    }

    async fn evaluate_bootstrap(
        &self,
        bootstrap: HostBootstrapSpec,
        probe: Arc<dyn IsolationProbePort>,
    ) -> Result<AuthorizationOutcome, ProviderServiceEvaluationError> {
        evaluate_with_probe(
            Arc::clone(&self.store),
            self.command.clone(),
            self.caller.clone(),
            bootstrap,
            Some(probe),
        )
        .await
    }

    fn counts(&self) -> Result<(i64, i64, i64), StoreError> {
        self.store.read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT (SELECT COUNT(*) FROM broker_phase_records),
                    (SELECT COUNT(*) FROM action_dispatches), (SELECT COUNT(*) FROM processes)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
    }
}

fn prepare_service_fixture(
    store: &UnleashStore,
    caller: &CallerAuthority,
    directory: &std::path::Path,
    adapt: impl FnOnce(&mut HostBootstrapSpec),
) -> TestResult<(CommandEnvelope, HostBootstrapSpec)> {
    let mut bootstrap = bootstrap_fixture(&directory.canonicalize()?)?;
    adapt(&mut bootstrap);
    let digests = inspect_strict_service_launch_plan(&bootstrap)?;
    let command = envelope(
        store,
        caller,
        2,
        LAUNCH_ID,
        "PrepareProviderServiceLaunch",
        json!({
            "profile_id": PROFILE_ID, "profile_version": 1,
            "backend": "macos_seatbelt_strict_service_v1",
            "executable": bootstrap.program,
            "executable_digest": digests.executable_digest,
            "command_digest": digests.command_digest,
            "environment_digest": digests.environment_digest,
            "confinement_digest": digests.confinement_digest,
            "deadline": store.config().clock.now_ms().as_i64() + 60_000,
        }),
    )?;
    store.prepare_provider_service_launch(caller.clone(), command.clone())?;
    let context = store
        .read_provider_service_launch(LAUNCH_ID)?
        .ok_or("preparation missing")?;
    bootstrap.allocation_context_digest = Some(canonical_sha256(&context)?);
    bootstrap.host_id = context.host_id;
    bootstrap.process_id = context.process_id;
    bootstrap.process_run_uuid = context.process_run_uuid;
    bootstrap.action_id = context.action_id;
    bootstrap.authority_generation = context.lineage_fence.authority_generation;
    bootstrap.daemon_epoch = context.lineage_fence.epoch;
    Ok((command, bootstrap))
}

fn bootstrap_fixture(root: &std::path::Path) -> TestResult<HostBootstrapSpec> {
    let runtime_dir = root.join("host");
    let cwd = root.join("child");
    let data_root = root.join("protected-data");
    let socket_dir = root.join("protected-sockets");
    for path in [&runtime_dir, &cwd, &data_root, &socket_dir] {
        unleash_platform::ensure_private_rendezvous_dir(path)?;
    }
    let executable = root.join("inert-service-fixture");
    std::fs::write(&executable, b"inert fixture")?;
    let explicit_env = std::collections::BTreeMap::from([("LANG".into(), "C".into())]);
    let effective = explicit_environment_v1(&explicit_env, &cwd.join("child-tmp"))?;
    Ok(HostBootstrapSpec {
        environment_mode: Some(EnvironmentMode::StrictServiceV1),
        expected_effective_environment_digest: Some(effective_environment_digest_v1(&effective)?),
        allocation_context_digest: Some(Sha256Digest::from_bytes(b"unassigned fixture")),
        runtime_dir: runtime_dir.clone(),
        host_id: HostId::from_u128(1),
        process_id: ProcessId::from_u128(2),
        terminal_id: None,
        process_run_uuid: ProcessRunUuid::from_u128(3),
        authority_generation: AuthorityGeneration::from_u128(4),
        daemon_epoch: DaemonEpoch::new(1),
        resource_fence: Fence::new(1),
        action_id: ActionId::from_u128(5),
        mission_id: None,
        owner_agent_id: None,
        worktree_id: None,
        cwd: cwd.clone(),
        program: executable
            .to_str()
            .ok_or("fixture path must be UTF-8")?
            .to_owned(),
        arguments: vec!["fixture".into()],
        explicit_env,
        resource_kind: ResourceKind::Service,
        isolation_mode: HostIsolationMode::Strict,
        strict_spec: Some(StrictIsolationSpec {
            profile_kind: IsolationProfileKind::WorkspaceCommand,
            assigned_write_roots: vec![cwd],
            data_root,
            socket_dir,
            keychain_paths: vec![],
            declared_credential_paths: vec![],
            egress_proxy_addr: None,
            extra_loopback_control_ports: vec![],
            profile_output_dir: runtime_dir.clone(),
        }),
        isolation_profile_kind: Some(IsolationProfileKind::WorkspaceCommand),
        control_secret_path: runtime_dir.join(CONTROL_SECRET_FILENAME),
        canaries_secret_path: None,
        max_segment_bytes: 1024,
        max_total_log_bytes: 65536,
        max_volatile_bytes: 4096,
    })
}

fn configure_profile(store: &UnleashStore, caller: &CallerAuthority, endpoint: &str) -> TestResult {
    let payload = serde_json::from_value::<unleash_protocol::SetProviderProfilePayload>(json!({
        "provider_kind": "ollama", "dialect": {"kind":"ollama_chat","version":1},
        "endpoint": endpoint, "model": "inert-fixture", "locality": "local_only",
        "server_management": "unleash_managed", "auth": {"kind":"none"}, "declared_capabilities": ["text"],
        "limits": {"max_inflight":1,"connect_timeout_ms":1000,"queue_timeout_ms":1000,
            "load_timeout_ms":5000,"first_token_timeout_ms":2000,"generation_timeout_ms":2000},
        "pricing": {"kind":"none_local"}
    }))?.with_normalized_endpoint()?;
    let command = envelope(
        store,
        caller,
        1,
        PROFILE_ID,
        "SetProviderProfile",
        serde_json::to_value(&payload)?,
    )?;
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: PROFILE_ID,
        profile_version: 1,
        project_id: None,
        payload,
        secret_binding: None,
        catalog_binding: None,
    };
    let data = canonicalize_to_vec(&configuration)?;
    let object_hash = Sha256Digest::from_bytes(&data);
    let owner_id = PROFILE_ID.to_string();
    let prepared = store.prepare_object(PrepareObjectRequest {
        object_hash,
        byte_length: u64::try_from(data.len())?,
        media_type: "application/json".into(),
        owner_kind: "service_evaluation_fixture".into(),
        owner_id: owner_id.clone(),
    })?;
    store.write_prepared_object_bytes(&prepared, &data)?;
    store.publish_owned_object_reference(PublishObjectReferenceRequest {
        pin_id: prepared.pin_id,
        object_hash,
        object_generation: prepared.object_generation,
        owner_kind: "service_evaluation_fixture".into(),
        owner_id,
        source_field: "configuration".into(),
    })?;
    store.commit_command(CommitCommandRequest {
        actor_id: caller.actor_id.clone(),
        envelope: command,
        target_aggregate: AggregateKind::ProviderProfile,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
        mutation: CommandMutation::SetProviderProfile {
            caller: caller.clone(),
            preparation: Box::new(ProviderProfilePreparation {
                configuration,
                content_object_hash: object_hash,
            }),
        },
    })?;
    Ok(())
}

fn envelope(
    store: &UnleashStore,
    caller: &CallerAuthority,
    id: u128,
    aggregate_id: Uuid,
    command_type: &str,
    payload: Value,
) -> TestResult<CommandEnvelope> {
    let Value::Object(payload) = payload else {
        return Err("fixture payload is not an object".into());
    };
    Ok(CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: store.active_authority_generation()?,
        actor_ref: Some(caller.actor_id.to_string()),
        project_id: None,
        aggregate_id,
        expected_revision: Revision::INITIAL,
        command_type: command_type.into(),
        payload,
    })
}

#[test]
fn broker_decision_yields_one_durable_launch_permit() -> TestResult {
    let fixture = Fixture::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let AuthorizationOutcome::Authorized(decision) =
        runtime.block_on(fixture.evaluate(Arc::new(TestIsolationProbe::standard_available())))?
    else {
        return Err("fixture evaluation should authorize".into());
    };
    let claim = |decision| {
        claim_prepared_provider_service_launch(
            Arc::clone(&fixture.store),
            fixture.command.clone(),
            fixture.caller.clone(),
            fixture.bootstrap.clone(),
            decision,
        )
    };
    let permit = runtime.block_on(claim(decision.clone()))?;
    let durable = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("action missing")?;
    assert_eq!(permit.context().action_id, fixture.action);
    assert_eq!(durable.state, ActionState::Dispatching);
    assert_eq!(durable.dispatch_id, Some(permit.dispatch_id()));
    assert_eq!(durable.last_event_cursor, permit.commit().event_cursor);
    assert!(runtime.block_on(claim(decision)).is_err());
    assert_eq!(fixture.counts()?, (3, 1, 0));
    Ok(())
}

#[test]
fn launch_permit_rejects_changed_bootstrap_before_start() -> TestResult {
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    let mut bootstrap = fixture.bootstrap.clone();
    bootstrap.arguments.push("changed".into());
    let result = spawn_provider_service_host(
        &fixture.store,
        &fixture.command,
        &fixture.caller,
        bootstrap,
        permit,
        &fixture.directory.path().join("unused-host"),
    );
    assert!(matches!(
        result,
        Err(ProviderServiceLaunchError::Bootstrap(_))
    ));
    assert_eq!(
        std::fs::read_dir(&fixture.bootstrap.runtime_dir)?.count(),
        0
    );
    assert_eq!(fixture.counts()?, (3, 1, 0));
    Ok(())
}

#[test]
fn launch_permit_refuses_uncertain_clock_without_preparing_files() -> TestResult {
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    fixture
        .store
        .config()
        .clock_uncertain
        .store(true, std::sync::atomic::Ordering::Release);
    let result = spawn_provider_service_host(
        &fixture.store,
        &fixture.command,
        &fixture.caller,
        fixture.bootstrap.clone(),
        permit,
        &fixture.directory.path().join("unused-host"),
    );
    assert!(matches!(
        result,
        Err(ProviderServiceLaunchError::StalePermit)
    ));
    assert_eq!(
        std::fs::read_dir(&fixture.bootstrap.runtime_dir)?.count(),
        0
    );
    assert_eq!(fixture.counts()?, (3, 1, 0));
    Ok(())
}

#[test]
fn launch_outcome_unknown_requires_the_dedicated_port_and_replays_exactly() -> TestResult {
    use unleash_store::{
        BrokerActionReceipt, BrokerObservationCause, ProviderServiceLaunchOutcome,
    };

    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    let launch_id = permit.context().launch_id;
    let dispatch_id = permit.dispatch_id();
    let current = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("missing dispatch")?;
    let decision_ref = fixture.store.read_snapshot(move |conn, _| {
        let decision: String = conn.query_row(
            "SELECT decision_json FROM broker_dispatches WHERE dispatch_id=?1",
            [dispatch_id.to_string()],
            |row| row.get(0),
        )?;
        let decision: unleash_store::BrokerAuthorizationDecision = serde_json::from_str(&decision)?;
        Ok(decision.decision_ref)
    })?;
    let generic = BrokerPhaseRequest {
        authority_generation: current.fence.authority_generation,
        expected_revision: current.revision,
        evaluation: BrokerEvaluation {
            evaluation_id: current.evaluation_id,
            previous_evaluation_id: None,
        },
        phase: BrokerPhase::Observation {
            action_id: fixture.action,
            intent_digest: current.intent_digest,
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
            receipt: BrokerActionReceipt {
                action_id: fixture.action,
                evaluation_id: current.evaluation_id,
                dispatch_id,
                decision_ref,
                state: ActionState::OutcomeUnknown,
                output_digest: None,
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: true,
                observed_at_ms: fixture.store.config().clock.now_ms(),
            },
        },
    };
    assert!(fixture.store.commit_broker_phase(generic).is_err());
    assert_eq!(fixture.counts()?, (3, 1, 0));
    let commit = fixture.store.commit_provider_service_launch_observation(
        launch_id,
        dispatch_id,
        ProviderServiceLaunchOutcome::Unconfirmed,
    )?;
    assert!(!commit.replayed);
    let replay = fixture.store.commit_provider_service_launch_observation(
        launch_id,
        dispatch_id,
        ProviderServiceLaunchOutcome::Unconfirmed,
    )?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, commit.event_cursor);
    let outcome = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("missing outcome")?;
    assert_eq!(outcome.state, ActionState::OutcomeUnknown);
    assert_eq!(outcome.dispatch_id, Some(dispatch_id));
    assert_eq!(fixture.counts()?, (4, 1, 0));
    Ok(())
}

#[test]
fn launch_outcome_wrong_dispatch_leaves_preparation_unchanged() -> TestResult {
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    assert!(
        fixture
            .store
            .commit_provider_service_launch_observation(
                permit.context().launch_id,
                DispatchId::from_uuid(Uuid::from_u128(0xf012_0521)),
                unleash_store::ProviderServiceLaunchOutcome::Unconfirmed,
            )
            .is_err()
    );
    assert_eq!(fixture.counts()?, (3, 1, 0));
    assert_eq!(
        fixture
            .store
            .inspect_broker_action(fixture.action)?
            .ok_or("missing action")?
            .state,
        ActionState::Dispatching
    );
    Ok(())
}

#[test]
fn launch_outcome_wrong_attestation_does_not_complete_dispatch() -> TestResult {
    use unleash_store::{
        ProviderServiceLaunchAttestation, ProviderServiceLaunchOutcome,
        ProviderServiceProcessIdentity,
    };
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    let context = permit.context();
    let host = ProviderServiceProcessIdentity {
        pid: 10,
        pgid: 10,
        owner_uid: fixture.store.config().owner_uid,
        boot_identity: fixture.store.config().boot_identity.clone(),
        start_identity: "fixture-host-start".into(),
    };
    let child = ProviderServiceProcessIdentity {
        pid: 20,
        start_identity: "fixture-child-start".into(),
        ..host.clone()
    };
    let proof = ProviderServiceLaunchAttestation {
        context_digest: canonical_sha256(context)?,
        environment_digest: context.plan.environment_digest,
        confinement_digest: Sha256Digest::from_bytes(b"different confinement"),
        host,
        child,
    };
    assert!(
        fixture
            .store
            .commit_provider_service_launch_observation(
                context.launch_id,
                permit.dispatch_id(),
                ProviderServiceLaunchOutcome::Attested(Box::new(proof)),
            )
            .is_err()
    );
    assert_eq!(fixture.counts()?, (3, 1, 0));
    Ok(())
}

#[test]
fn launch_placement_requires_an_attested_dispatch() -> TestResult {
    use unleash_store::ProviderServiceLaunchOutcome;
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    let launch = permit.context().launch_id;
    let dispatch = permit.dispatch_id();
    assert!(
        fixture
            .store
            .read_attested_provider_service_launch(dispatch)
            .is_err()
    );
    fixture.store.commit_provider_service_launch_observation(
        launch,
        dispatch,
        ProviderServiceLaunchOutcome::Unconfirmed,
    )?;
    assert!(
        fixture
            .store
            .read_attested_provider_service_launch(dispatch)
            .is_err()
    );
    assert_eq!(fixture.counts()?, (4, 1, 0));
    Ok(())
}

#[test]
fn launch_placement_recovers_the_exact_attested_child() -> TestResult {
    use unleash_store::{
        ProviderServiceLaunchAttestation, ProviderServiceLaunchOutcome,
        ProviderServiceProcessIdentity,
    };
    let fixture = Fixture::new()?;
    let permit = fixture.claim_permit()?;
    let context = permit.context();
    let host = ProviderServiceProcessIdentity {
        pid: 10,
        pgid: 10,
        owner_uid: fixture.store.config().owner_uid,
        boot_identity: fixture.store.config().boot_identity.clone(),
        start_identity: "fixture-host".into(),
    };
    let child = ProviderServiceProcessIdentity {
        pid: 20,
        start_identity: "fixture-child".into(),
        ..host.clone()
    };
    let proof = ProviderServiceLaunchAttestation {
        context_digest: canonical_sha256(context)?,
        environment_digest: context.plan.environment_digest,
        confinement_digest: context.plan.confinement_digest,
        host,
        child,
    };
    let dispatch = permit.dispatch_id();
    let commit = fixture.store.commit_provider_service_launch_observation(
        context.launch_id,
        dispatch,
        ProviderServiceLaunchOutcome::Attested(Box::new(proof.clone())),
    )?;
    let restored = fixture
        .store
        .read_attested_provider_service_launch(dispatch)?;
    assert_eq!(restored.context, *context);
    assert_eq!(restored.attestation, proof);
    assert_eq!(restored.attestation_digest, canonical_sha256(&proof)?);
    assert_eq!(restored.event_cursor, commit.event_cursor);
    assert_eq!(fixture.counts()?, (4, 1, 0));
    Ok(())
}

#[test]
#[ignore = "owned loopback listener fixture, run only by its exact test parent"]
fn nominal_generation_listener_fixture() -> TestResult {
    let port: u16 = std::fs::read_to_string("listener-port")?.parse()?;
    let _listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    std::fs::write("listener-ready", b"ready")?;
    std::thread::sleep(Duration::from_secs(8));
    Ok(())
}

#[test]
#[ignore = "requires the packaged process-host built from the same sources"]
fn nominal_generation_adopts_owned_listener() -> TestResult {
    use crate::provider_probe_generation_service::{adopt_placement_if_ready, authorizing};
    let host_binary = std::path::PathBuf::from(
        std::env::var_os("UNLEASH_TEST_PROCESS_HOST_BIN").ok_or("missing fixture host binary")?,
    );
    let reserved = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let port = reserved.local_addr()?.port();
    let binary = std::env::current_exe()?.canonicalize()?;
    let program = binary
        .to_str()
        .ok_or("fixture executable must be UTF-8")?
        .to_owned();
    let mut request = None;
    let fixture = Fixture::with_profile(
        &format!("http://127.0.0.1:{port}"),
        |store, caller| {
            request = Some(prepare_nominal_generation(
                store,
                caller,
                ActorId::new("service-evaluation-fixture")?,
                1,
            )?);
            Ok(())
        },
        |spec| {
            spec.program = program;
            spec.arguments = vec![
                "provider_service_evaluation::tests::nominal_generation_listener_fixture".into(),
                "--exact".into(),
                "--include-ignored".into(),
            ];
        },
    )?;
    let request = request.ok_or("missing fixture probe")?;
    std::fs::write(
        fixture.bootstrap.cwd.join("listener-port"),
        port.to_string(),
    )?;
    let permit = fixture.claim_permit()?;
    let dispatch = permit.dispatch_id();
    let rendezvous = fixture.directory.path().join("registry");
    unleash_platform::ensure_private_rendezvous_dir(&rendezvous)?;
    let controller = ManagedProviderServiceHosts::new(rendezvous);
    drop(reserved);
    let owner = NominalTestHost {
        controller: &controller,
        dispatch,
    };
    let outcome = controller.launch_with_binary(
        &fixture.store,
        &fixture.command,
        &fixture.caller,
        fixture.bootstrap.clone(),
        permit,
        &host_binary,
    )?;
    assert_eq!(outcome.state, ActionState::ObservedSuccess);
    let ready = fixture.bootstrap.cwd.join("listener-ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !ready.try_exists()? {
        if std::time::Instant::now() >= deadline {
            return Err("fixture listener did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let store = &fixture.store;
    store.commit_provider_generation_preparation(&request, &fixture.caller)?;
    store
        .commit_provider_generation_authorizing(&request, &fixture.caller, |record, isolation| {
            authorizing(store, record, isolation)
        })?
        .ok_or("missing generation intent")?;
    let evidence = adopt_placement_if_ready(store, &controller, &request, &fixture.caller)?
        .ok_or("owned listener was not adopted")?;
    assert!(adopt_placement_if_ready(store, &controller, &request, &fixture.caller)?.is_none());
    let pool = store.global_execution_pool_id()?;
    let snapshot = store.execution_admission_snapshot(pool)?;
    assert!(snapshot.queued.is_empty());
    assert!(snapshot.active.is_empty());
    assert_eq!(fixture.counts()?, (5, 1, 0));
    verify_staged_placement(store, evidence, dispatch, request.aggregate_id)?;
    verify_owned_generation_admission(store, &request, &fixture.caller, evidence, dispatch, port)?;
    assert_eq!(fixture.counts()?, (5, 1, 0));
    owner.finish()
}

fn verify_owned_generation_admission(
    store: &UnleashStore,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
    evidence: Uuid,
    dispatch: DispatchId,
    port: u16,
) -> TestResult {
    let (handoff, decision) =
        crate::execution_scheduler::schedule_managed_generation(store, command, caller, evidence)?;
    let handoff = handoff.ok_or("managed transport did not receive its admission")?;
    assert!(matches!(decision, unleash_scheduler::Decision::Admit(_)));
    let probe = unleash_domain::ProviderProbeId::from_uuid(command.aggregate_id);
    assert_eq!(
        handoff.commit.unit,
        unleash_store::AdmissionUnit::Probe(probe)
    );
    let snapshot = store.execution_admission_snapshot(store.global_execution_pool_id()?)?;
    assert_eq!(snapshot.active.len(), 1);
    assert!(snapshot.queued.is_empty());
    assert_eq!(
        Sha256Digest::from_bytes(&handoff.inputs.body),
        handoff.inputs.probe.payload_digest(),
    );
    verify_native_generation_binding(store, handoff.inputs, dispatch, port)?;
    Ok(())
}

fn verify_native_generation_binding(
    store: &UnleashStore,
    inputs: unleash_store::AdmittedOllamaChatInputs,
    dispatch: DispatchId,
    port: u16,
) -> TestResult {
    use unleash_model_router::{
        connect_transport::OwnedLoopbackPeer,
        ollama_transport::{FrozenOllamaChatRequest, OllamaChatLimits},
        transport::ProfileRef,
    };
    let launch = store.read_attested_provider_service_launch(dispatch)?;
    let child = unleash_platform::inspect_process_identity(launch.attestation.child.pid)?;
    let endpoint = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let peer = OwnedLoopbackPeer::new(endpoint, child)?;
    let expected = inputs
        .managed_transport
        .ok_or("missing writer transport binding")?;
    let preparation = &inputs.probe.context.preparation;
    let profile = ProfileRef::new(
        &preparation.requested_profile_id,
        preparation.requested_profile_version,
        expected.profile_digest,
    )?;
    let request = FrozenOllamaChatRequest::from_admitted_body(
        &inputs.transport_placement,
        profile,
        peer,
        &preparation.resolved_model_id,
        OllamaChatLimits {
            connect_ms: expected.connect_ms,
            total_ms: expected.total_ms,
            output_tokens: preparation.limits.max_output_tokens,
        },
        inputs.body,
        inputs.probe.payload_digest(),
    )?;
    assert_eq!(request.payload_digest(), expected.request_digest);
    Ok(())
}

fn verify_staged_placement(
    store: &UnleashStore,
    evidence: Uuid,
    dispatch: DispatchId,
    probe: Uuid,
) -> TestResult {
    let hash: String = store.read_snapshot(move |db, _| {
        Ok(db.query_row(
            "SELECT e.object_hash FROM evidence e
             JOIN object_references r ON r.object_hash=e.object_hash
             WHERE e.evidence_id=?1 AND e.kind='observation'
               AND r.owner_id=?2 AND r.source_field='placement' AND r.state='live'",
            [evidence.to_string(), probe.to_string()],
            |row| row.get(0),
        )?)
    })?;
    let fact: Value = serde_json::from_slice(&store.read_verified_object_bytes(hash.parse()?)?)?;
    assert_eq!(
        fact.pointer("/managed_launch/dispatch_id"),
        Some(&json!(dispatch))
    );
    Ok(())
}

struct NominalTestHost<'a> {
    controller: &'a ManagedProviderServiceHosts,
    dispatch: DispatchId,
}

impl NominalTestHost<'_> {
    fn reap(mut host: OwnedProviderServiceHost) -> TestResult {
        // The fixture child exits on its own. The retained host remains
        // available to document departure, then the test reaps it.
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while host.try_wait()?.is_none() {
            if std::time::Instant::now() >= deadline {
                host.child.kill()?;
                host.child.wait()?;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    fn finish(self) -> TestResult {
        let host = self
            .controller
            .take_fixture_host(self.dispatch)
            .ok_or("host not retained")?;
        Self::reap(host)
    }
}

impl Drop for NominalTestHost<'_> {
    fn drop(&mut self) {
        if let Some(host) = self.controller.take_fixture_host(self.dispatch) {
            let _ = Self::reap(host);
        }
    }
}

fn prepare_nominal_generation(
    store: &UnleashStore,
    caller: &CallerAuthority,
    principal_id: ActorId,
    max_output_tokens: u64,
) -> TestResult<CommandEnvelope> {
    use unleash_domain::{MicrosAmount, TimestampMs};
    use unleash_protocol::{ProviderTestLimits, ProviderTestPhase};
    use unleash_store::{
        ExecutionPoolSettings, PolicyScopeKind, ProviderProbeMandateChange,
        ProviderProbeMandateIssue, SettingsPatch, SettingsScopeKind,
    };
    let authority = store.active_authority_generation()?;
    let pool = ExecutionPoolSettings::default();
    let settings = envelope(
        store,
        caller,
        21,
        authority.as_uuid(),
        "SetSettings",
        json!({
            "scope_kind":"user","scope_id":"default","patch":{"execution_pool":pool}
        }),
    )?;
    fixture_commit(
        store,
        caller,
        settings,
        AggregateKind::Settings,
        CommandMutation::SetSettings {
            scope_kind: SettingsScopeKind::User,
            scope_id: "default".into(),
            caller: caller.clone(),
            patch: SettingsPatch {
                execution_pool: Some(pool),
                retention_policy: None,
                confirm_retention_shortening: false,
                analyzer_paths: None,
                extra_typed: Some(serde_json::Map::new()),
            },
        },
    )?;
    store.register_principal(
        unleash_store::PLACEMENT_OBSERVER_PRINCIPAL,
        "system",
        unleash_store::PLACEMENT_OBSERVER_PRINCIPAL,
    )?;
    let limits = ProviderTestLimits::new(5_000, 1, max_output_tokens, MicrosAmount::ZERO);
    let grant = ProviderProbeMandateChange::Issue(ProviderProbeMandateIssue {
        principal_id,
        profile_id: PROFILE_ID,
        profile_version: 1,
        phase: ProviderTestPhase::GenerationProbe,
        locality: DataLocality::LocalOnly,
        capability_set: vec!["text".into()],
        limits: limits.clone(),
        expires_at_ms: TimestampMs::new(store.config().clock.now_ms().as_i64() + 60_000),
    });
    let policy = envelope(
        store,
        caller,
        22,
        authority.as_uuid(),
        "SetPolicy",
        json!({
            "scope_kind":"global", "scope_id":"global", "isolation_mode":"standard",
            "policy_version":"nominal-fixture", "reason":"inert local listener", "provider_probe_mandate":grant,
        }),
    )?;
    let receipt = fixture_commit(
        store,
        caller,
        policy,
        AggregateKind::SecurityPolicy,
        CommandMutation::SetPolicyWithProviderProbeMandate {
            scope_kind: PolicyScopeKind::Global,
            scope_id: "global".into(),
            isolation_mode: PolicyIsolationMode::Standard,
            policy_version: "nominal-fixture".into(),
            reason: "inert local listener".into(),
            caller: caller.clone(),
            mandate: grant,
        },
    )?;
    envelope(
        store,
        caller,
        23,
        Uuid::from_u128(23),
        "TestProviderProfile",
        json!({
            "profile_id":PROFILE_ID,"profile_version":1,"phase":"generation_probe",
            "locality":"local_only","capability_set":["text"],"limits":limits,"mandate_ref":receipt,
        }),
    )
}

fn fixture_commit(
    store: &UnleashStore,
    caller: &CallerAuthority,
    envelope: CommandEnvelope,
    aggregate: AggregateKind,
    mutation: CommandMutation,
) -> Result<unleash_domain::ReceiptId, StoreError> {
    Ok(store
        .commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: aggregate,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
            mutation,
        })?
        .receipt()
        .receipt_id)
}

#[test]
#[ignore = "requires the packaged process-host built from the same sources"]
fn launch_permit_starts_packaged_host() -> TestResult {
    use crate::executors::{ExecutorDrainPort, RendezvousExecutorController};
    let executable = std::path::PathBuf::from(
        std::env::var_os("UNLEASH_TEST_PROCESS_HOST_BIN").ok_or("missing fixture host binary")?,
    );
    let fixture = Fixture::with_bootstrap(|spec| {
        spec.program = "/bin/sleep".into();
        spec.arguments = vec!["4".into()];
    })?;
    let permit = fixture.claim_permit()?;
    let dispatch_id = permit.dispatch_id();
    let launch_id = permit.context().launch_id;
    let host_id = permit.context().host_id.to_string();
    assert_eq!(fixture.counts()?, (3, 1, 0));
    let rendezvous = fixture.directory.path().join("registry");
    unleash_platform::ensure_private_rendezvous_dir(&rendezvous)?;
    let controller = ManagedProviderServiceHosts::new(rendezvous.clone());
    let outcome = controller.launch_with_binary(
        &fixture.store,
        &fixture.command,
        &fixture.caller,
        fixture.bootstrap.clone(),
        permit,
        &executable,
    );
    let listener = controller.observe_listener(&fixture.store, dispatch_id);
    let host = controller
        .take_fixture_host(dispatch_id)
        .map(|host| TestOwnedServiceHost {
            host,
            reaped: false,
        });
    let mut host = host.ok_or("controller did not retain the owned host")?;
    let outcome = outcome?;
    assert_eq!(outcome.state, ActionState::ObservedSuccess);
    assert!(matches!(
        listener,
        Err(ProviderServiceLaunchError::ListenerUnverified)
    ));
    let registry = RendezvousExecutorController;
    let different_root = fixture.directory.path().join("another-fixture-root");
    let (_, live) = registry
        .active_hosts_for_other_root(&rendezvous, &different_root)?
        .ok_or("owned host missing from executor registry")?;
    assert_eq!(live, vec![host_id.clone()]);
    assert_eq!(
        registry
            .drain_on_stop(&rendezvous, Duration::ZERO)
            .retained_services,
        vec![host_id]
    );
    let sequence = i64::try_from(outcome.commit.event_cursor.sequence.as_u64())?;
    let payload: Value = fixture.store.read_snapshot(move |conn, _| {
        let json: String = conn.query_row(
            "SELECT e.payload_json FROM events e JOIN outbox o ON o.event_id=e.event_id
             WHERE e.sequence=?1 AND e.event_type='ActionObserved' AND o.destination='clients'",
            [sequence],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&json)?)
    })?;
    let proof: unleash_store::ProviderServiceLaunchAttestation = serde_json::from_value(
        payload
            .get("provider_service_launch")
            .ok_or("missing attestation")?
            .clone(),
    )?;
    assert_eq!(proof.host.pid, host.host.child.id());
    assert_ne!(proof.child.pid, proof.host.pid);
    assert_eq!(
        proof.context_digest,
        fixture
            .bootstrap
            .allocation_context_digest
            .ok_or("context digest absent")?
    );
    assert_eq!(
        payload.get("output_digest"),
        Some(&json!(canonical_sha256(&proof)?))
    );
    let child = unleash_platform::inspect_process_identity(proof.child.pid)?;
    assert_eq!(proof.child.start_identity, child.start_identity.as_str());
    let replay = fixture.store.commit_provider_service_launch_observation(
        launch_id,
        dispatch_id,
        unleash_store::ProviderServiceLaunchOutcome::Attested(Box::new(proof)),
    )?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, outcome.commit.event_cursor);
    assert_eq!(fixture.counts()?, (4, 1, 0));
    host.reap()?;
    assert!(matches!(
        unleash_platform::verify_process_identity(&child)?,
        unleash_platform::ProcessVerification::Exited
    ));
    assert!(
        registry
            .active_hosts_for_other_root(&rendezvous, &different_root)?
            .is_none()
    );
    Ok(())
}

struct TestOwnedServiceHost {
    host: OwnedProviderServiceHost,
    reaped: bool,
}

impl TestOwnedServiceHost {
    fn reap(&mut self) -> std::io::Result<()> {
        // The service is a finite sleep. Give it time to exit before reaping
        // the test's retained process-host, including on assertion failures.
        std::thread::sleep(Duration::from_secs(4));
        if self.host.try_wait()?.is_none() {
            self.host.child.kill()?;
            self.host.child.wait()?;
        }
        self.reaped = true;
        Ok(())
    }
}

impl Drop for TestOwnedServiceHost {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.reap();
        }
    }
}

#[test]
#[ignore = "requires the native macOS Seatbelt probe and /usr/bin/true"]
fn native_profile_loads_and_records_a_decision() -> TestResult {
    let fixture = Fixture::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(evaluate_provider_service_launch(
        Arc::clone(&fixture.store),
        fixture.command.clone(),
        fixture.caller.clone(),
        fixture.bootstrap.clone(),
    ))?;
    let AuthorizationOutcome::Authorized(decision) = result else {
        return Err("native profile probe should authorize fixture evaluation".into());
    };
    let durable = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("action missing")?;
    assert_eq!(durable.evaluation_id, decision.evaluation_id);
    assert_eq!(durable.state, ActionState::Authorized);
    assert_eq!(fixture.counts()?, (2, 0, 0));
    assert!(
        std::fs::read_dir(&fixture.bootstrap.runtime_dir)?
            .next()
            .is_none()
    );
    Ok(())
}

#[test]
fn broker_rejects_changed_bootstrap_material_before_journaling() -> TestResult {
    let fixture = Fixture::new()?;
    let this = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let assert_rejected = |bootstrap| -> TestResult {
        assert!(matches!(
            this.block_on(fixture.evaluate_bootstrap(
                bootstrap,
                Arc::new(TestIsolationProbe::standard_available())
            )),
            Err(ProviderServiceEvaluationError::Store(
                StoreError::EvidenceUnavailable(_)
            ))
        ));
        assert_eq!(fixture.counts()?, (0, 0, 0));
        Ok(())
    };
    let mut changed_host = fixture.bootstrap.clone();
    changed_host.host_id = HostId::from_u128(99);
    assert_rejected(changed_host)?;
    let mut changed_args = fixture.bootstrap.clone();
    changed_args.arguments.push("different-fixture".into());
    assert_rejected(changed_args)?;
    let mut changed_env = fixture.bootstrap.clone();
    changed_env
        .explicit_env
        .insert("LANG".into(), "en_US.UTF-8".into());
    let env = explicit_environment_v1(
        &changed_env.explicit_env,
        &changed_env.cwd.join("child-tmp"),
    )?;
    changed_env.expected_effective_environment_digest =
        Some(effective_environment_digest_v1(&env)?);
    assert_rejected(changed_env)?;
    let mut changed_profile = fixture.bootstrap.clone();
    let protected = fixture
        .directory
        .path()
        .canonicalize()?
        .join("protected-fixture");
    std::fs::create_dir(&protected)?;
    changed_profile
        .strict_spec
        .as_mut()
        .ok_or("fixture confinement missing")?
        .declared_credential_paths
        .push(protected);
    assert_rejected(changed_profile)?;
    std::fs::write(&fixture.bootstrap.program, b"changed bytes")?;
    assert_rejected(fixture.bootstrap.clone())?;
    Ok(())
}

#[test]
fn broker_decision_is_bound_to_its_durable_evaluation() -> TestResult {
    let fixture = Fixture::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let probe = || Arc::new(TestIsolationProbe::standard_available());
    let AuthorizationOutcome::Authorized(decision) = runtime.block_on(fixture.evaluate(probe()))?
    else {
        return Err("fixture evaluation should authorize".into());
    };
    let stored = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("action missing")?;
    assert_eq!(stored.state, ActionState::Authorized);
    assert_eq!(stored.evaluation_id, decision.evaluation_id);
    assert_eq!(stored.action_id, decision.action_id);
    assert_eq!(fixture.counts()?, (2, 0, 0));
    assert!(matches!(
        runtime.block_on(fixture.evaluate(probe())),
        Err(ProviderServiceEvaluationError::Store(
            StoreError::EvidenceUnavailable(_)
        ))
    ));
    assert_eq!(fixture.counts()?, (2, 0, 0));
    assert_eq!(
        fixture
            .store
            .inspect_broker_action(fixture.action)?
            .ok_or("action missing")?
            .last_event_cursor,
        stored.last_event_cursor
    );
    Ok(())
}

#[test]
fn broker_persists_unavailable_confinement_without_an_allow() -> TestResult {
    let fixture = Fixture::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let probe = Arc::new(TestIsolationProbe::standard_available());
    probe.set_missing_backend("fixture", "fixture deliberately has no backend");
    assert!(matches!(
        runtime.block_on(fixture.evaluate(probe)),
        Err(ProviderServiceEvaluationError::Broker(
            ActionBrokerError::IsolationBackendMissing { .. }
        ))
    ));
    let stored = fixture
        .store
        .inspect_broker_action(fixture.action)?
        .ok_or("action missing")?;
    assert_ne!(stored.state, ActionState::Authorized);
    assert_eq!(fixture.counts()?, (2, 0, 0));
    Ok(())
}
