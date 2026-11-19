//! One non-rotating native observation for an existing same-daemon controller.
//! This path never launches, allocates, transfers, rotates or retries a host call.
use crate::ipc::DaemonRuntimeState;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use unleash_action_broker::{
    ActionBroker, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort, ActionIntent,
    ActionProvenance, ActionSubject, AuthorityLayer, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture, BrokerConfig, EffectiveAuthority, EffectsClass, HostIsolationProbe, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef,
    PlatformKeyringSecretStore, ResolvedSecret, ScopeKind, StrictNoneAcceptedObligationChecker,
    SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{
    DataLocality, DispatchId, Fence, LineageFence, ModelPolicy, ReconcileStrategy, Sha256Digest,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME, HostClient,
    HostManifest, HostRequest, HostResponse, effective_environment_digest_v1,
    explicit_environment_v1, load_control_secret,
};
use unleash_store::{
    CallerAuthority, CallerRole, PrepareObjectRequest, PublishObjectReferenceRequest,
    terminal_allocations::{
        TerminalIdentityBackend, TerminalProcessIdentity,
        terminal_renewal_types::{
            ResolveTerminalRenewalRequest, TERMINAL_RENEWAL_VERB, TerminalRenewalContext,
            TerminalRenewalObservation, TerminalRenewalRecord,
        },
    },
};

type ObservationSlot = Arc<Mutex<Option<TerminalRenewalObservation>>>;
type RenewalError = Box<dyn std::error::Error + Send + Sync>;

fn authority(c: &TerminalRenewalContext) -> EffectiveAuthority {
    compute_effective_authority(
        Some(c.project_id),
        c.policy_revision,
        c.scope_revision,
        &[AuthorityLayer {
            layer_name: "authenticated_human_terminal_renewal".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([TERMINAL_RENEWAL_VERB.into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Project,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    )
}

fn renewal_intent(
    record: &TerminalRenewalRecord,
    authority: &EffectiveAuthority,
    fence: LineageFence,
) -> Result<ActionIntent, ()> {
    let c = &record.context;
    Ok(ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: Some(c.project_id),
        verb: TERMINAL_RENEWAL_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.context_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.previous_expires_at_ms,
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    })
}

pub(super) async fn run(
    state: Arc<DaemonRuntimeState>,
    record: TerminalRenewalRecord,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
) -> Result<(), ()> {
    let c = &record.context;
    if c.authority_generation != state.authority_generation || c.daemon_epoch != state.daemon_epoch
    {
        return Err(());
    }
    let store = Arc::clone(&state.store);
    let project = c.project_id;
    let policy =
        tokio::task::spawn_blocking(move || store.read_policy_summary_snapshot(Some(project)))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?
            .value;
    if policy.effective_isolation_mode != unleash_store::PolicyIsolationMode::Standard
        || policy
            .guard_config
            .as_ref()
            .is_some_and(|guard| guard.guard_mode != unleash_store::GuardPolicyMode::BrokerOnly)
    {
        return Err(());
    }
    let authority = authority(c);
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let intent = renewal_intent(&record, &authority, fence)?;
    let slot = Arc::new(Mutex::new(None));
    let timer = Arc::new(SystemClockAndTimer::new());
    let broker = ActionBroker::new(
        Arc::clone(&state.store.config().clock),
        timer.clone(),
        timer,
        Arc::clone(&state.store.config().id_generator),
        Arc::new(HostIsolationProbe),
        Arc::new(StrictNoneAcceptedObligationChecker),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig {
            current_fence: fence,
            evaluate_timeout: std::time::Duration::from_secs(2),
            health_check_interval: std::time::Duration::from_secs(30),
            cache_max_entries: 1,
            cache_ttl: std::time::Duration::ZERO,
            canaries_to_redact: vec![],
        },
        Arc::new(
            crate::broker_journal::SqliteBrokerJournal::with_terminal_renewal_observation(
                &state,
                Arc::clone(&slot),
            ),
        ),
    );
    broker.bump_scope_revision(Some(project), c.scope_revision);
    let AuthorizationOutcome::Authorized(decision) = broker
        .authorize_action(intent, &authority)
        .await
        .map_err(|_| ())?
    else {
        return Err(());
    };
    let action = c.action_id;
    let executor = RenewalExecutor {
        state: Arc::clone(&state),
        record,
        slot,
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("No renewal SecretRefs in this path"),
    ));
    let receipt = broker
        .execute_authorized(
            action,
            &decision.decision_ref,
            fence,
            &authority,
            &executor,
            &secrets,
        )
        .await
        .map_err(|_| ())?;
    if receipt.state != unleash_domain::ActionState::ObservedSuccess {
        return Err(());
    }
    Ok(())
}

struct RenewalExecutor {
    state: Arc<DaemonRuntimeState>,
    record: TerminalRenewalRecord,
    slot: ObservationSlot,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
}

impl ActionExecutorPort for RenewalExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(async move {
            let unknown = || ActionExecutionFailure::OutcomeUnknown {
                dispatch_id: dispatch,
                reason: "Terminal renewal requires reconciliation".into(),
            };
            if !secrets.is_empty()
                || intent.action_id != self.record.context.action_id
                || decision.action_id != intent.action_id
                || intent.payload_digest != self.record.context_digest
            {
                return Err(unknown());
            }
            let state = Arc::clone(&self.state);
            let record = self.record.clone();
            let activity = Arc::clone(&self.activity);
            let observation = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                observe_once(&state, &record, dispatch)
            })
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            let digest = unleash_protocol::canonical_sha256(&observation).map_err(|_| unknown())?;
            let captured = observation.observed_at_ms;
            *self.slot.lock().map_err(|_| unknown())? = Some(observation);
            Ok(ActionExecutionResult {
                dispatch_id: dispatch,
                provider_request_id: None,
                output_digest: digest,
                observed_at_ms: captured,
            })
        })
    }
}

fn identity(
    value: &unleash_platform::OsProcessIdentity,
) -> Result<TerminalProcessIdentity, RenewalError> {
    Ok(TerminalProcessIdentity {
        pid: value.pid,
        pgid: value.pgid,
        owner_uid: value.owner_uid,
        boot_identity: value.boot_identity.as_str().into(),
        start_identity: value.start_identity.as_str().into(),
        backend: match value.backend {
            unleash_platform::IdentityBackend::MacosProcPidinfo => {
                TerminalIdentityBackend::MacosProcPidinfo
            }
            unleash_platform::IdentityBackend::LinuxProcfs => TerminalIdentityBackend::LinuxProcfs,
            _ => return Err("terminal renewal identity backend is not qualified".into()),
        },
    })
}

fn validate_manifest(m: &HostManifest, c: &TerminalRenewalContext) -> Result<(), RenewalError> {
    let expected = &c.expected_os_identity;
    if m.host_id != c.host_id
        || m.process_id != c.process_id
        || m.action_id != c.initial_action_id
        || m.process_run_uuid != c.process_run_uuid
        || m.authority_generation != c.authority_generation
        || m.daemon_epoch != c.daemon_epoch
        || m.highest_fence != c.resource_fence
        || m.allocation_context_digest != Some(c.initial_context_digest)
        || m.pid != Some(expected.pid)
        || m.pgid != Some(expected.pgid)
        || m.owner_uid != expected.owner_uid
        || m.boot_identity
            .as_ref()
            .map(unleash_platform::BootIdentity::as_str)
            != Some(expected.boot_identity.as_str())
        || m.start_identity
            .as_ref()
            .map(unleash_platform::StartIdentity::as_str)
            != Some(expected.start_identity.as_str())
    {
        return Err("terminal renewal manifest differs".into());
    }
    Ok(())
}

fn rendezvous(
    state: &DaemonRuntimeState,
    record: &TerminalRenewalRecord,
) -> Result<(PathBuf, Sha256Digest), RenewalError> {
    let c = &record.context;
    let allocation = state
        .store
        .inspect_terminal_allocation_snapshot(c.terminal_id)?
        .value
        .ok_or("terminal launch missing")?;
    if allocation.context_digest != c.initial_context_digest
        || allocation.context.action_id != c.initial_action_id
    {
        return Err("terminal initial context changed".into());
    }
    let runtime = state
        .store
        .config()
        .data_root
        .canonicalize()?
        .join("process-hosts")
        .join(c.host_id.to_string());
    if runtime.canonicalize()? != runtime {
        return Err("terminal renewal rendezvous changed".into());
    }
    unleash_platform::validate_rendezvous_dir(&runtime)?;
    let before = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&before, c)?;
    if identity(&unleash_platform::inspect_process_identity(
        c.expected_os_identity.pid,
    )?)? != c.expected_os_identity
    {
        return Err("terminal renewal fresh OS identity differs".into());
    }
    let spec = &allocation.context.preparation.process_spec;
    let cwd = Path::new(&spec.cwd_canonical);
    if cwd.canonicalize()? != cwd || before.cwd_canonical != cwd {
        return Err("terminal renewal working directory changed".into());
    }
    let profile = state
        .store
        .terminal_runtime_settings()?
        .profiles
        .into_iter()
        .find(|profile| {
            profile.profile_id == spec.environment.profile_id
                && profile.version == spec.environment.version
        })
        .ok_or("terminal renewal environment missing")?;
    if profile.digest()? != spec.environment.digest {
        return Err("terminal renewal environment changed".into());
    }
    let expected = effective_environment_digest_v1(&explicit_environment_v1(
        &profile.variables,
        &runtime.join("child-tmp"),
    )?)?;
    if before.effective_environment_digest != Some(expected) {
        return Err("terminal renewal applied environment changed".into());
    }
    Ok((runtime, expected))
}

fn before_effect(
    state: &DaemonRuntimeState,
    c: &TerminalRenewalContext,
) -> Result<(), RenewalError> {
    if !state.is_accepting()
        || state
            .store
            .config()
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
        || state.store.config().clock.now_ms() >= c.previous_expires_at_ms
    {
        return Err("terminal renewal permission elapsed".into());
    }
    Ok(())
}

fn observe_once(
    state: &DaemonRuntimeState,
    record: &TerminalRenewalRecord,
    dispatch: DispatchId,
) -> Result<TerminalRenewalObservation, RenewalError> {
    let c = &record.context;
    let (runtime, environment) = rendezvous(state, record)?;
    before_effect(state, c)?;
    state
        .store
        .resolve_terminal_renewal(ResolveTerminalRenewalRequest {
            action_id: c.action_id,
            dispatch_id: dispatch,
            context_digest: record.context_digest,
            expected_revision: record.revision,
            caller: CallerAuthority {
                actor_id: c.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
        })?;
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    before_effect(state, c)?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        c.process_run_uuid,
        c.authority_generation,
        c.daemon_epoch,
        c.resource_fence,
    )?;
    before_effect(state, c)?;
    // Handshake reconciles and writes the manifest: it occurs only after the
    // unique durable dispatch and consumed permit, and is never retried here.
    let response = client.call(&HostRequest::Handshake {
        header: client.make_header(c.command_id, c.action_id, record.context_digest),
        current_secret: secret.expose_secret().into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?;
    let observation = capture_response(state, record, dispatch, &runtime, environment, &response)?;
    publish(state, &observation)?;
    Ok(observation)
}

fn capture_response(
    state: &DaemonRuntimeState,
    record: &TerminalRenewalRecord,
    dispatch: DispatchId,
    runtime: &Path,
    environment: Sha256Digest,
    response: &HostResponse,
) -> Result<TerminalRenewalObservation, RenewalError> {
    let c = &record.context;
    let HostResponse::HandshakeAck {
        allocation_context_digest,
        effective_environment_digest,
        process_run_uuid,
        authority_generation,
        daemon_epoch,
        highest_fence,
        os_identity,
        durable_log_seq,
        last_confirmed_input_seq,
        unknown_input_seqs,
        secret_rotated,
    } = response
    else {
        return Err("terminal renewal handshake refused".into());
    };
    if *allocation_context_digest != Some(c.initial_context_digest)
        || *effective_environment_digest != Some(environment)
        || *process_run_uuid != c.process_run_uuid
        || *authority_generation != c.authority_generation
        || *daemon_epoch != c.daemon_epoch
        || *highest_fence != c.resource_fence
        || *secret_rotated
        || identity(os_identity)? != c.expected_os_identity
        || !unknown_input_seqs.is_empty()
        || identity(&unleash_platform::inspect_process_identity(
            c.expected_os_identity.pid,
        )?)? != c.expected_os_identity
    {
        return Err("terminal renewal native acknowledgement differs".into());
    }
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&manifest, c)?;
    if manifest.effective_environment_digest != Some(environment) {
        return Err("terminal renewal native environment differs".into());
    }
    Ok(TerminalRenewalObservation {
        action_id: c.action_id,
        dispatch_id: dispatch,
        context_digest: record.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        os_identity: identity(os_identity)?,
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.resource_fence,
        handshake_response_digest: unleash_protocol::canonical_sha256(response)?,
        manifest_digest: unleash_protocol::canonical_sha256(&manifest)?,
        effective_environment_digest: environment,
        durable_log_seq: *durable_log_seq,
        last_confirmed_input_seq: *last_confirmed_input_seq,
        unknown_input_seqs: unknown_input_seqs.clone(),
        observed_at_ms: state.store.config().clock.now_ms(),
    })
}

fn publish(
    state: &DaemonRuntimeState,
    observation: &TerminalRenewalObservation,
) -> Result<(), RenewalError> {
    let bytes = unleash_protocol::canonicalize_to_vec(observation)?;
    let hash = unleash_protocol::canonical_sha256(observation)?;
    let object = state.store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
        owner_kind: "terminal_renewal_observation".into(),
        owner_id: observation.action_id.to_string(),
    })?;
    state.store.write_prepared_object_bytes(&object, &bytes)?;
    state
        .store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: hash,
            owner_kind: "terminal_renewal_observation".into(),
            owner_id: observation.action_id.to_string(),
            object_generation: object.object_generation,
            source_field: observation.dispatch_id.to_string(),
        })?;
    Ok(())
}
