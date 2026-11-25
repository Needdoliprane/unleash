//! Contrats du lancement managé qui n'exigent aucun plan natif (toutes plateformes).
//!
//! Ces tests obtiennent le permis de lancement par les seuls ports du writer (`Authorizing`,
//! `Authorization`, `Dispatch`) sur une préparation au plan synthétique, sans inspecter de
//! binaire ni de profil de confinement. Ils tournent donc aussi sous Linux, dans la CI.
//! L'évaluation par le broker du démon et le lancement d'un hôte exigent macOS : voir `native`
//! (REQ-MODEL-015).

use super::*;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, AuthorizationEvaluationId, DaemonEpoch,
    DispatchId, HostId, ProcessId, ProcessRunUuid, Revision, Sha256Digest,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, EnvironmentMode, IsolationMode as HostIsolationMode,
    IsolationProfileKind, ResourceKind, StrictIsolationSpec, effective_environment_digest_v1,
    explicit_environment_v1,
};
use unleash_protocol::{
    CommandId, ProtocolVersion, ReceiptState, RequestId, canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, BrokerAuthorizationCause, BrokerAuthorizationDecision,
    BrokerAuthorizationOutcome, BrokerExecutor, BrokerEvaluation, BrokerPhase,
    BrokerPhaseRequest, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    PrepareObjectRequest, ProviderProfileConfiguration, ProviderProfilePreparation,
    ProviderServiceLaunchFingerprint, PublishObjectReferenceRequest, StoreConfig, StoreError,
    UnleashStore,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

// macOS, et seulement macOS : le plan de service `strict_service_v1` y est refusé ailleurs par
// `validate_launch` (`IsolationObligationUnenforceable`, avant toute phase du broker), et
// l'observation native du listener loopback possédé n'existe que sur macOS. Aucun double de
// Seatbelt ne les remplace.
#[cfg(target_os = "macos")]
mod native;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PROFILE_ID: Uuid = Uuid::from_u128(0xf012_0519);
const LAUNCH_ID: Uuid = Uuid::from_u128(0xf012_0520);

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_permit_refuses_uncertain_clock_without_preparing_files() -> TestResult {
    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
    let bootstrap = bootstrap_fixture(&fixture.directory.path().canonicalize()?)?;
    fixture
        .store
        .config()
        .clock_uncertain
        .store(true, Ordering::Release);
    let result = spawn_provider_service_host(
        &fixture.store,
        &fixture.command,
        &fixture.caller,
        bootstrap.clone(),
        permit,
        &fixture.directory.path().join("unused-host"),
    );
    assert!(matches!(
        result,
        Err(ProviderServiceLaunchError::StalePermit)
    ));
    assert_eq!(
        std::fs::read_dir(&bootstrap.runtime_dir)?.count(),
        0
    );
    assert_eq!(fixture.counts()?, (3, 1, 0));
    Ok(())
}

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_outcome_unknown_requires_the_dedicated_port_and_replays_exactly()
-> TestResult {
    use unleash_store::{
        BrokerActionReceipt, BrokerObservationCause, ProviderServiceLaunchOutcome,
    };

    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
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
        let decision: unleash_store::BrokerAuthorizationDecision =
            serde_json::from_str(&decision)?;
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

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_outcome_wrong_dispatch_leaves_preparation_unchanged() -> TestResult {
    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
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

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_outcome_wrong_attestation_does_not_complete_dispatch() -> TestResult {
    use unleash_store::{
        ProviderServiceLaunchAttestation, ProviderServiceLaunchOutcome,
        ProviderServiceProcessIdentity,
    };
    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
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

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_placement_requires_an_attested_dispatch() -> TestResult {
    use unleash_store::ProviderServiceLaunchOutcome;
    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
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

// T-F012-015 / AC-F012-015 / REQ-MODEL-015
#[test]
fn t_f012_015_launch_placement_recovers_the_exact_attested_child() -> TestResult {
    use unleash_store::{
        ProviderServiceLaunchAttestation, ProviderServiceLaunchOutcome,
        ProviderServiceProcessIdentity,
    };
    let fixture = WriterFixture::new()?;
    let permit = fixture.claim_permit_through_writer()?;
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

/// Préparation au plan synthétique : le démon n'a pas encore de planificateur de lancement.
struct WriterFixture {
    store: Arc<UnleashStore>,
    directory: tempfile::TempDir,
    command: CommandEnvelope,
    caller: CallerAuthority,
    action: ActionId,
}

impl WriterFixture {
    fn new() -> TestResult<Self> {
        let directory = local_test_dir("f012-service-writer")?;
        let store = ready_store(directory.path())?;
        let caller = operator()?;
        configure_profile(&store, &caller, "http://127.0.0.1:11434")?;
        let command = envelope(
            &store,
            &caller,
            2,
            LAUNCH_ID,
            "PrepareProviderServiceLaunch",
            json!({
                "profile_id": PROFILE_ID, "profile_version": 1,
                "backend": "macos_seatbelt_strict_service_v1",
                "executable": "/fixture/inert-service",
                "executable_digest": Sha256Digest::from_bytes(b"fixture executable"),
                "command_digest": Sha256Digest::from_bytes(b"fixture command"),
                "environment_digest": Sha256Digest::from_bytes(b"fixture environment"),
                "confinement_digest": Sha256Digest::from_bytes(b"fixture confinement"),
                "deadline": store.config().clock.now_ms().as_i64() + 60_000,
            }),
        )?;
        store.prepare_provider_service_launch(caller.clone(), command.clone())?;
        let action = store
            .read_provider_service_launch(LAUNCH_ID)?
            .ok_or("preparation missing")?
            .action_id;
        Ok(Self {
            store,
            directory,
            command,
            caller,
            action,
        })
    }

    /// Permis obtenu par les seuls ports du writer, sans évaluation ni inspection native.
    fn claim_permit_through_writer(&self) -> TestResult<ProviderServiceLaunchPermit> {
        let store = &self.store;
        let caller = &self.caller;
        let context = store.read_provider_service_launch_for_authorization(&self.command, caller)?;
        let intent = context.broker_intent()?;
        let intent_digest = intent.intent_digest;
        let payload_digest = intent.payload_digest;
        let evaluation = BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_u128(0xf012_0530),
            previous_evaluation_id: None,
        };
        let decision = BrokerAuthorizationDecision {
            action_id: context.action_id,
            evaluation_id: evaluation.evaluation_id,
            source_evaluation_id: None,
            decision_ref: "inert-writer-fixture".into(),
            policy_revision: context.policy_revision,
            scope_revision: context.scope_revision,
            payload_digest,
            expiry: Some(context.plan.deadline),
            applied_obligations: vec![],
            fallback_policy_ref: None,
            from_cache: false,
            executor: BrokerExecutor::Unleash,
        };
        let authorization = BrokerPhaseRequest {
            authority_generation: context.lineage_fence.authority_generation,
            expected_revision: Revision::new(1),
            evaluation,
            phase: BrokerPhase::Authorization {
                action_id: context.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        };
        let first = BrokerPhaseRequest {
            expected_revision: Revision::INITIAL,
            phase: BrokerPhase::Authorizing { intent },
            ..authorization.clone()
        };
        store.commit_provider_service_launch_broker_phase(&self.command, caller, first)?;
        store.commit_provider_service_launch_broker_phase(&self.command, caller, authorization.clone())?;
        Ok(store.commit_provider_service_launch_dispatch(
            &self.command,
            caller,
            BrokerPhaseRequest {
                expected_revision: Revision::new(2),
                phase: BrokerPhase::Dispatch {
                    action_id: context.action_id,
                    intent_digest,
                    decision,
                    dispatch_id: DispatchId::from_u128(0xf012_0531),
                    fence: context.lineage_fence,
                },
                ..authorization
            },
            ProviderServiceLaunchFingerprint {
                context_digest: payload_digest,
                executable_digest: context.plan.executable_digest,
                command_digest: context.plan.command_digest,
                environment_digest: context.plan.environment_digest,
                confinement_digest: context.plan.confinement_digest,
            },
        )?)
    }

    fn counts(&self) -> Result<(i64, i64, i64), StoreError> {
        phase_counts(&self.store)
    }
}

fn ready_store(directory: &std::path::Path) -> TestResult<Arc<UnleashStore>> {
    let mut config = StoreConfig::new(directory);
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
    Ok(store)
}

fn operator() -> TestResult<CallerAuthority> {
    Ok(CallerAuthority {
        actor_id: ActorId::new("user:service-evaluation-fixture")?,
        role: CallerRole::HumanOperator,
    })
}

/// Phases broker, marqueurs de dispatch et lignes de processus engagés.
fn phase_counts(store: &UnleashStore) -> Result<(i64, i64, i64), StoreError> {
    store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT (SELECT COUNT(*) FROM broker_phase_records),
                    (SELECT COUNT(*) FROM action_dispatches), (SELECT COUNT(*) FROM processes)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    })
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

fn configure_profile(
    store: &UnleashStore,
    caller: &CallerAuthority,
    endpoint: &str,
) -> TestResult<()> {
    let payload = serde_json::from_value::<unleash_protocol::SetProviderProfilePayload>(json!({
        "provider_kind": "ollama", "dialect": {"kind":"ollama_chat","version":1},
        "endpoint": endpoint, "model": "inert-fixture", "locality": "local_only",
        "server_management": "unleash_managed", "auth": {"kind":"none"}, "declared_capabilities": ["text"],
        "limits": {"max_inflight":1,"connect_timeout_ms":1000,"queue_timeout_ms":1000,
            "load_timeout_ms":5000,"first_token_timeout_ms":2000,"generation_timeout_ms":2000},
        "pricing": {"kind":"none_local"}
    }))?
    .with_normalized_endpoint()?;
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
