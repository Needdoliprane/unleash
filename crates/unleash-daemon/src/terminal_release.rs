//! Explicit native controller retirement. It never stops the host, child or PTY.
use crate::ipc::DaemonRuntimeState;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};
use unleash_action_broker::{
    ActionBroker, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort, ActionIntent,
    ActionProvenance, ActionSubject, AuthorityLayer, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture, BrokerConfig, EffectiveAuthority, EffectsClass, GuardMode, HostIsolationProbe,
    InitialEffectivePolicy, IsolationMode, IsolationRequirements, LocalAuthorityFallbackPolicy,
    NormalizedResourceRef, PlatformKeyringSecretStore, ResolvedSecret, ScopeKind,
    StrictNoneAcceptedObligationChecker, SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{
    ActorId, CommandId, DataLocality, DispatchId, Fence, LineageFence, ModelPolicy,
    ReconcileStrategy,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, ControlReleaseKey, ControlReleaseRequest, ControlReleaseResponse,
    HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME, HostClient, HostControlSecret, HostControlState,
    HostManifest, load_control_secret,
};
use unleash_protocol::{CommandEnvelope, ErrorCode, IpcError, ReceiptState, RetryClass};
use unleash_store::{
    CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest, PrepareObjectRequest,
    PublishObjectReferenceRequest,
    terminal_allocations::{
        TerminalIdentityBackend, TerminalProcessIdentity,
        terminal_release_types::{
            ResolveTerminalReleaseRequest, TERMINAL_RELEASE_VERB, TerminalReleaseContext,
            TerminalReleaseObservation, TerminalReleaseRecord, TerminalReleaseRequest,
            TerminalReleaseState,
        },
    },
};
type Slot = Arc<Mutex<Option<TerminalReleaseObservation>>>;
type ReleaseError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) fn prepare(
    session: &crate::auth::AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<CommitCommandRequest, IpcError> {
    let bad = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Terminal release target is inadmissible",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    let request: TerminalReleaseRequest =
        serde_json::from_value(serde_json::Value::Object(command.payload.clone()))
            .map_err(|_| bad())?;
    if command.project_id.is_none()
        || request.terminal_id.as_uuid() != command.aggregate_id
        || request.terminal_id.as_uuid().is_nil()
        || request.process_run_uuid.as_uuid().is_nil()
        || request.lease_id.as_uuid().is_nil()
        || !(1..=9_007_199_254_740_991).contains(&request.fence.as_u64())
        || !matches!(
            session.client_kind,
            unleash_protocol::ClientKind::Cli
                | unleash_protocol::ClientKind::Tui
                | unleash_protocol::ClientKind::Gui
        )
    {
        return Err(bad());
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
        mutation: CommandMutation::PrepareTerminalRelease {
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

pub(crate) fn start(state: &Arc<DaemonRuntimeState>, actor: ActorId, command: CommandId) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let activity = Arc::new(activity);
        let store = Arc::clone(&state.store);
        let Ok(Ok(snapshot)) = tokio::task::spawn_blocking(move || {
            store.inspect_terminal_release_request(&actor, command)
        })
        .await
        else {
            return;
        };
        let Some(record) = snapshot.value else {
            return;
        };
        if record.state != TerminalReleaseState::Prepared
            || record.broker_action_state.is_some()
            || record.broker_dispatch_id.is_some()
        {
            return;
        }
        if run(state, record, activity).await.is_err() {
            tracing::warn!("Terminal release has no confirmed disposition; inspect recovery");
        }
    });
}

fn authority(c: &TerminalReleaseContext) -> EffectiveAuthority {
    compute_effective_authority(
        Some(c.project_id),
        c.policy_revision,
        c.scope_revision,
        &[AuthorityLayer {
            layer_name: "authenticated_human_terminal_release".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([TERMINAL_RELEASE_VERB.into()])),
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

async fn run(
    state: Arc<DaemonRuntimeState>,
    record: TerminalReleaseRecord,
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
    let slot = Arc::new(Mutex::new(None));
    let (broker, intent) = build_broker(&state, &record, &slot, &authority, fence)?;
    let AuthorizationOutcome::Authorized(decision) = broker
        .authorize_action(intent, &authority)
        .await
        .map_err(|_| ())?
    else {
        return Err(());
    };
    let action = c.action_id;
    let executor = ReleaseExecutor {
        state: Arc::clone(&state),
        record,
        slot,
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("No release SecretRefs in this path"),
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

fn build_broker(
    state: &Arc<DaemonRuntimeState>,
    record: &TerminalReleaseRecord,
    slot: &Slot,
    authority: &EffectiveAuthority,
    fence: LineageFence,
) -> Result<(ActionBroker, ActionIntent), ()> {
    let c = &record.context;
    let project = c.project_id;
    let intent = ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: Some(project),
        verb: TERMINAL_RELEASE_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.context_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.source_expires_at_ms,
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    };
    let timer = Arc::new(SystemClockAndTimer::new());
    let broker = ActionBroker::new_with_policy_snapshot(
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
            crate::broker_journal::SqliteBrokerJournal::with_terminal_release_observation(
                state,
                Arc::clone(slot),
            ),
        ),
        InitialEffectivePolicy {
            project_id: Some(project),
            policy_revision: c.policy_revision,
            scope_revision: c.scope_revision,
            policy_digest: c.security_policy_digest,
            isolation_mode: IsolationMode::Standard,
            guard_mode: GuardMode::BrokerOnly,
            lineage_fence: fence,
        },
    )
    .map_err(|_| ())?;
    Ok((broker, intent))
}

struct ReleaseExecutor {
    state: Arc<DaemonRuntimeState>,
    record: TerminalReleaseRecord,
    slot: Slot,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
}
impl ActionExecutorPort for ReleaseExecutor {
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
                reason: "Terminal release requires reconciliation".into(),
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
            let captured = observation.captured_at_ms;
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
) -> Result<TerminalProcessIdentity, ReleaseError> {
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
            _ => return Err("terminal release identity backend unqualified".into()),
        },
    })
}

fn validate_manifest(m: &HostManifest, c: &TerminalReleaseContext) -> Result<(), ReleaseError> {
    let i = &c.expected_os_identity;
    if m.host_id != c.host_id
        || m.process_id != c.process_id
        || m.terminal_id != Some(c.terminal_id)
        || m.action_id != c.initial_action_id
        || m.process_run_uuid != c.process_run_uuid
        || m.authority_generation != c.authority_generation
        || m.daemon_epoch != c.daemon_epoch
        || m.highest_fence != c.resource_fence
        || m.allocation_context_digest != Some(c.initial_context_digest)
        || m.pid != Some(i.pid)
        || m.pgid != Some(i.pgid)
        || m.owner_uid != i.owner_uid
        || m.boot_identity
            .as_ref()
            .map(unleash_platform::BootIdentity::as_str)
            != Some(i.boot_identity.as_str())
        || m.start_identity
            .as_ref()
            .map(unleash_platform::StartIdentity::as_str)
            != Some(i.start_identity.as_str())
    {
        return Err("terminal release native manifest differs".into());
    }
    Ok(())
}

fn rendezvous(
    state: &DaemonRuntimeState,
    record: &TerminalReleaseRecord,
) -> Result<PathBuf, ReleaseError> {
    let c = &record.context;
    let allocation = state
        .store
        .inspect_terminal_allocation_snapshot(c.terminal_id)?
        .value
        .ok_or("terminal launch missing")?;
    if allocation.context_digest != c.initial_context_digest
        || allocation.context.action_id != c.initial_action_id
    {
        return Err("terminal release launch binding differs".into());
    }
    let runtime = state
        .store
        .config()
        .data_root
        .canonicalize()?
        .join("process-hosts")
        .join(c.host_id.to_string());
    if runtime.canonicalize()? != runtime {
        return Err("terminal release rendezvous changed".into());
    }
    unleash_platform::validate_rendezvous_dir(&runtime)?;
    let m = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&m, c)?;
    if !matches!(m.control_state, Some(HostControlState::Active { .. })) {
        return Err("native host does not attest active retirement support".into());
    }
    if identity(&unleash_platform::inspect_process_identity(
        c.expected_os_identity.pid,
    )?)? != c.expected_os_identity
    {
        return Err("terminal release fresh OS identity differs".into());
    }
    Ok(runtime)
}

fn before_effect(
    state: &DaemonRuntimeState,
    c: &TerminalReleaseContext,
) -> Result<(), ReleaseError> {
    if !state.is_accepting()
        || state.store.config().clock_uncertain.load(Ordering::Acquire)
        || state.store.config().clock.now_ms() >= c.source_expires_at_ms
    {
        return Err("terminal release permission elapsed".into());
    }
    Ok(())
}

fn observe_once(
    state: &DaemonRuntimeState,
    record: &TerminalReleaseRecord,
    dispatch: DispatchId,
) -> Result<TerminalReleaseObservation, ReleaseError> {
    let c = &record.context;
    let runtime = rendezvous(state, record)?;
    before_effect(state, c)?;
    state
        .store
        .resolve_terminal_release(ResolveTerminalReleaseRequest {
            caller: CallerAuthority {
                actor_id: c.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
            action_id: c.action_id,
            context_digest: record.context_digest,
            expected_revision: record.revision,
            dispatch_id: dispatch,
        })?;
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    let next = HostControlSecret::generate();
    before_effect(state, c)?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        c.process_run_uuid,
        c.authority_generation,
        c.daemon_epoch,
        c.resource_fence,
    )?;
    before_effect(state, c)?;
    let key = ControlReleaseKey {
        header: client.make_header(c.command_id, c.action_id, record.context_digest),
        dispatch_id: dispatch,
        initial_context_digest: c.initial_context_digest,
        lease_id: c.lease_id,
        control_revision: c.control_revision,
    };
    let response = client.release_control(&ControlReleaseRequest {
        key: key.clone(),
        current_secret: secret.expose_secret().into(),
        next_secret: next.expose_secret().into(),
    })?;
    let ControlReleaseResponse::Ack { ack } = response else {
        return Err("terminal release native refusal".into());
    };
    if ack.key != key
        || ack.host_id != c.host_id
        || ack.process_id != c.process_id
        || ack.terminal_id != c.terminal_id
        || identity(&ack.os_identity)? != c.expected_os_identity
        || !ack.unknown_input_seqs.is_empty()
    {
        return Err("terminal release native acknowledgement differs".into());
    }
    let m = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&m, c)?;
    if !matches!(&m.control_state,Some(HostControlState::Released {ack:published}) if published.as_ref()==ack.as_ref())
        || identity(&unleash_platform::inspect_process_identity(
            c.expected_os_identity.pid,
        )?)? != c.expected_os_identity
    {
        return Err("terminal release published native disposition differs".into());
    }
    let observation = TerminalReleaseObservation {
        schema_version: 1,
        ack: serde_json::from_value(serde_json::to_value(&ack)?)?,
        manifest_digest: unleash_protocol::canonical_sha256(&m)?,
        captured_at_ms: state.store.config().clock.now_ms(),
    };
    publish(state, &observation)?;
    Ok(observation)
}

fn publish(
    state: &DaemonRuntimeState,
    observation: &TerminalReleaseObservation,
) -> Result<(), ReleaseError> {
    let bytes = unleash_protocol::canonicalize_to_vec(observation)?;
    let hash = unleash_protocol::canonical_sha256(observation)?;
    let action = observation.ack.key.header.action_id;
    let object = state.store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
        owner_kind: "terminal_release_observation".into(),
        owner_id: action.to_string(),
    })?;
    state.store.write_prepared_object_bytes(&object, &bytes)?;
    state
        .store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: hash,
            owner_kind: "terminal_release_observation".into(),
            owner_id: action.to_string(),
            object_generation: object.object_generation,
            source_field: observation.ack.key.dispatch_id.to_string(),
        })?;
    Ok(())
}
