//! One native input attempt after an exact durable broker dispatch.
//!
//! Raw bytes stay in this task only. Cancellation, a missing response or a lost
//! commit ACK never starts a second native write.

use crate::ipc::DaemonRuntimeState;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
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
    ActorId, CommandId, DataLocality, DispatchId, Fence, LineageFence, ModelPolicy,
    ReconcileStrategy, Sha256Digest,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME, HostClient,
    HostManifest, HostRequest, HostResponse, effective_environment_digest_v1,
    explicit_environment_v1, load_control_secret,
};
use unleash_protocol::{CommandEnvelope, ErrorCode, IpcError, ReceiptState, RetryClass};
use unleash_store::{
    CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest, PrepareObjectRequest,
    PublishObjectReferenceRequest,
    terminal_allocations::{
        TerminalIdentityBackend, TerminalProcessIdentity,
        terminal_input_types::{
            ResolveTerminalInputRequest, TERMINAL_INPUT_VERB, TerminalInputContext,
            TerminalInputObservation, TerminalInputRecord, TerminalInputRequest,
            TerminalInputState, TerminalNativeInputReceipt,
        },
    },
};

type ObservationSlot = Arc<Mutex<Option<TerminalInputObservation>>>;
type InputError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) fn decode(command: &CommandEnvelope) -> Result<TerminalInputRequest, IpcError> {
    let invalid = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Terminal input shape is inadmissible",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    let request: TerminalInputRequest =
        serde_json::from_value(serde_json::Value::Object(command.payload.clone()))
            .map_err(|_| invalid())?;
    request.validate().map_err(|_| invalid())?;
    if command.project_id.is_none() || request.terminal_id.as_uuid() != command.aggregate_id {
        return Err(invalid());
    }
    Ok(request)
}

pub(crate) fn prepare(
    session: &crate::auth::AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<CommitCommandRequest, IpcError> {
    let request = decode(&command)?;
    if !matches!(
        session.client_kind,
        unleash_protocol::ClientKind::Cli
            | unleash_protocol::ClientKind::Tui
            | unleash_protocol::ClientKind::Gui
    ) {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Terminal input requires an authenticated human client",
            RetryClass::Never,
        )
        .with_command_id(command.command_id));
    }
    Ok(CommitCommandRequest {
        actor_id: session.actor_id.clone(),
        envelope: command,
        target_aggregate: unleash_store::AggregateKind::TerminalControl,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalInput {
            caller: CallerAuthority {
                actor_id: session.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
            request,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

pub(crate) fn start(
    state: &Arc<DaemonRuntimeState>,
    actor: ActorId,
    command: CommandId,
    request: TerminalInputRequest,
) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let activity = Arc::new(activity);
        let store = Arc::clone(&state.store);
        let found = tokio::task::spawn_blocking(move || {
            store.inspect_terminal_input_request(&actor, command)
        })
        .await;
        let Ok(Ok(snapshot)) = found else {
            return;
        };
        let Some(record) = snapshot.value else {
            return;
        };
        if record.state != TerminalInputState::Prepared
            || record.broker_action_state.is_some()
            || record.broker_dispatch_id.is_some()
            || !matches_request(&record.context, &request)
        {
            return;
        }
        if run(state, record, request.payload_bytes, activity)
            .await
            .is_err()
        {
            tracing::warn!("Terminal input has no confirmed observation; inspect recovery");
        }
    });
}

fn matches_request(c: &TerminalInputContext, request: &TerminalInputRequest) -> bool {
    request.validate().is_ok()
        && request.terminal_id == c.terminal_id
        && request.process_run_uuid == c.process_run_uuid
        && request.lease_id == c.lease_id
        && request.fence == c.resource_fence
        && request.input_seq == c.input_seq
        && u64::try_from(request.payload_bytes.len()).ok() == Some(c.payload_byte_length)
        && Sha256Digest::from_bytes(&request.payload_bytes) == c.payload_digest
}

fn authority(c: &TerminalInputContext) -> EffectiveAuthority {
    compute_effective_authority(
        Some(c.project_id),
        c.policy_revision,
        c.scope_revision,
        &[AuthorityLayer {
            layer_name: "authenticated_human_terminal_input".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([TERMINAL_INPUT_VERB.into()])),
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

fn intent(
    record: &TerminalInputRecord,
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
        verb: TERMINAL_INPUT_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.context_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.previous_lease_expires_at,
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    })
}

async fn run(
    state: Arc<DaemonRuntimeState>,
    record: TerminalInputRecord,
    bytes: Vec<u8>,
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
    let intent = intent(&record, &authority, fence)?;
    let slot = Arc::new(Mutex::new(None));
    let timer = Arc::new(SystemClockAndTimer::new());
    let wait_timer = Arc::clone(&timer);
    let broker = ActionBroker::new(
        Arc::clone(&state.store.config().clock),
        wait_timer,
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
            crate::broker_journal::SqliteBrokerJournal::with_terminal_input_observation(
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
    let executor = InputExecutor {
        state,
        record,
        bytes,
        slot,
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("No terminal input SecretRefs in this path"),
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

struct InputExecutor {
    state: Arc<DaemonRuntimeState>,
    record: TerminalInputRecord,
    bytes: Vec<u8>,
    slot: ObservationSlot,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
}

impl ActionExecutorPort for InputExecutor {
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
                reason: "Terminal input requires reconciliation".into(),
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
            let bytes = self.bytes.clone();
            let activity = Arc::clone(&self.activity);
            let observation = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                write_and_observe(&state, &record, bytes, dispatch)
            })
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            let output_digest =
                unleash_protocol::canonical_sha256(&observation).map_err(|_| unknown())?;
            let observed_at_ms = observation.observed_at_ms;
            *self.slot.lock().map_err(|_| unknown())? = Some(observation);
            Ok(ActionExecutionResult {
                dispatch_id: dispatch,
                provider_request_id: None,
                output_digest,
                observed_at_ms,
            })
        })
    }
}

fn identity(
    value: &unleash_platform::OsProcessIdentity,
) -> Result<TerminalProcessIdentity, InputError> {
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
            _ => return Err("terminal input identity backend is not qualified".into()),
        },
    })
}

fn rendezvous(
    state: &DaemonRuntimeState,
    record: &TerminalInputRecord,
) -> Result<(PathBuf, Sha256Digest, TerminalProcessIdentity), InputError> {
    let c = &record.context;
    let allocation = state
        .store
        .inspect_terminal_allocation_snapshot(c.terminal_id)?
        .value
        .ok_or("terminal initial allocation missing")?;
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
        return Err("terminal runtime path changed".into());
    }
    unleash_platform::validate_rendezvous_dir(&runtime)?;
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    // Inspect the OS now, rather than treating the host's cached anchor or a
    // private manifest as a fresh observation. The writer still revalidates
    // the registered tuple atomically when the one-use permit is consumed.
    let observed = identity(&unleash_platform::inspect_process_identity(
        manifest.pid.ok_or("terminal registered PID missing")?,
    )?)?;
    validate_manifest(&manifest, c, &observed)?;
    let spec = &allocation.context.preparation.process_spec;
    let cwd = std::path::Path::new(&spec.cwd_canonical);
    if cwd.canonicalize()? != cwd || manifest.cwd_canonical != cwd {
        return Err("terminal working directory changed".into());
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
        .ok_or("terminal environment missing")?;
    if profile.digest()? != spec.environment.digest {
        return Err("terminal environment changed".into());
    }
    let digest = effective_environment_digest_v1(&explicit_environment_v1(
        &profile.variables,
        &runtime.join("child-tmp"),
    )?)?;
    if manifest.effective_environment_digest != Some(digest) {
        return Err("terminal applied environment changed".into());
    }
    Ok((runtime, digest, observed))
}

fn validate_manifest(
    m: &HostManifest,
    c: &TerminalInputContext,
    expected: &TerminalProcessIdentity,
) -> Result<(), InputError> {
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
        return Err("terminal input manifest differs".into());
    }
    Ok(())
}

fn before_effect(
    state: &DaemonRuntimeState,
    c: &TerminalInputContext,
) -> Result<unleash_domain::TimestampMs, InputError> {
    let now = state.store.config().clock.now_ms();
    if !state.is_accepting()
        || state.store.config().clock_uncertain.load(Ordering::Acquire)
        || now >= c.previous_lease_expires_at
    {
        return Err("terminal input permission elapsed".into());
    }
    Ok(now)
}

fn write_and_observe(
    state: &DaemonRuntimeState,
    record: &TerminalInputRecord,
    bytes: Vec<u8>,
    dispatch: DispatchId,
) -> Result<TerminalInputObservation, InputError> {
    let c = &record.context;
    if u64::try_from(bytes.len())? != c.payload_byte_length
        || Sha256Digest::from_bytes(&bytes) != c.payload_digest
    {
        return Err("terminal input bytes differ from accepted intent".into());
    }
    let (runtime, environment, observed_identity) = rendezvous(state, record)?;
    before_effect(state, c)?;
    let resolved = state
        .store
        .resolve_terminal_input(ResolveTerminalInputRequest {
            action_id: c.action_id,
            dispatch_id: dispatch,
            authority_generation: c.authority_generation,
            expected_revision: record.revision,
            context_digest: record.context_digest,
            caller: CallerAuthority {
                actor_id: c.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
        })?;
    if observed_identity != resolved.expected_os_identity {
        return Err("terminal registered and observed identities differ".into());
    }
    before_effect(state, c)?;
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        c.process_run_uuid,
        c.authority_generation,
        c.daemon_epoch,
        c.resource_fence,
    )?;
    validate_handshake(
        &mut client,
        record,
        secret.expose_secret(),
        environment,
        &resolved.expected_os_identity,
    )?;
    if identity(&unleash_platform::inspect_process_identity(
        observed_identity.pid,
    )?)? != observed_identity
    {
        return Err("terminal live identity changed before input".into());
    }
    let header = client.make_header(c.command_id, c.action_id, record.context_digest);
    // This is the only native SendInput call. Never repeat it after an error.
    let response = client.call(&HostRequest::SendInput {
        header,
        secret: secret.expose_secret().into(),
        input_seq: c.input_seq,
        payload_bytes: bytes,
        now_ms: before_effect(state, c)?,
    })?;
    let observed_at_ms = state.store.config().clock.now_ms();
    let HostResponse::InputReceipt { receipt } = &response else {
        return Err("terminal input has no native receipt".into());
    };
    if receipt.input_seq != c.input_seq
        || receipt.command_id != c.command_id
        || receipt.action_id != c.action_id
        || receipt.payload_hash != c.payload_digest
    {
        return Err("terminal input receipt differs".into());
    }
    let observation = TerminalInputObservation {
        action_id: c.action_id,
        dispatch_id: dispatch,
        context_digest: record.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.resource_fence,
        native_receipt: TerminalNativeInputReceipt {
            input_seq: receipt.input_seq,
            command_id: receipt.command_id,
            action_id: receipt.action_id,
            receipt_id: receipt.receipt_id,
            payload_hash: receipt.payload_hash,
        },
        native_response_digest: unleash_protocol::canonical_sha256(&response)?,
        observed_at_ms,
    };
    publish(state, &observation)?;
    Ok(observation)
}

fn validate_handshake(
    client: &mut HostClient,
    record: &TerminalInputRecord,
    secret: &str,
    environment: Sha256Digest,
    expected: &TerminalProcessIdentity,
) -> Result<(), InputError> {
    let c = &record.context;
    let response = client.call(&HostRequest::Handshake {
        header: client.make_header(c.command_id, c.action_id, record.context_digest),
        current_secret: secret.into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?;
    let HostResponse::HandshakeAck {
        allocation_context_digest,
        effective_environment_digest,
        process_run_uuid,
        authority_generation,
        daemon_epoch,
        highest_fence,
        os_identity,
        last_confirmed_input_seq,
        unknown_input_seqs,
        secret_rotated,
        ..
    } = response
    else {
        return Err("terminal input handshake refused".into());
    };
    if allocation_context_digest != Some(c.initial_context_digest)
        || effective_environment_digest != Some(environment)
        || process_run_uuid != c.process_run_uuid
        || authority_generation != c.authority_generation
        || daemon_epoch != c.daemon_epoch
        || highest_fence != c.resource_fence
        || identity(&os_identity)? != *expected
        || last_confirmed_input_seq.checked_add(1) != Some(c.input_seq)
        || !unknown_input_seqs.is_empty()
        || secret_rotated
    {
        return Err("terminal input live state differs".into());
    }
    Ok(())
}

fn publish(
    state: &DaemonRuntimeState,
    observation: &TerminalInputObservation,
) -> Result<(), InputError> {
    let bytes = unleash_protocol::canonicalize_to_vec(observation)?;
    let hash = unleash_protocol::canonical_sha256(observation)?;
    let object = state.store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
        owner_kind: "terminal_input_observation".into(),
        owner_id: observation.action_id.to_string(),
    })?;
    state.store.write_prepared_object_bytes(&object, &bytes)?;
    state
        .store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: hash,
            object_generation: object.object_generation,
            owner_kind: "terminal_input_observation".into(),
            owner_id: observation.action_id.to_string(),
            source_field: observation.dispatch_id.to_string(),
        })?;
    Ok(())
}
