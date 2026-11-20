//! Authenticated terminal configuration and allocation orchestration.

use crate::ipc::DaemonRuntimeState;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, EffectiveAuthority, EffectsClass, HostIsolationProbe,
    IsolationMode as BrokerIsolationMode, IsolationRequirements, LocalAuthorityFallbackPolicy,
    NormalizedResourceRef, PlatformKeyringSecretStore, ScopeKind,
    StrictNoneAcceptedObligationChecker, SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{DataLocality, Fence, LineageFence, ModelPolicy, ReconcileStrategy};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, EnvironmentMode, HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME,
    HostBootstrapSpec, HostClient, HostControlSecret, HostManifest, HostRequest, HostResponse,
    IsolationMode as HostIsolationMode, ResourceKind, effective_environment_digest_v1,
    explicit_environment_v1, load_control_secret, store_control_secret,
};
use unleash_store::{
    CallerAuthority, CallerRole, FenceAssertion, PrepareObjectRequest,
    PublishObjectReferenceRequest, ResolveTerminalLaunchRequest,
};

#[path = "terminal_metadata.rs"]
mod metadata;
pub(crate) use metadata::read_terminal_metadata;
#[path = "terminal_control.rs"]
mod control;
#[path = "terminal_input.rs"]
mod input;
#[path = "terminal_release.rs"]
mod release;
#[path = "terminal_renewal.rs"]
mod renewal;
pub(crate) use input::{decode as decode_input, prepare as prepare_input, start as start_input};
pub(crate) use renewal::{prepare_resize, start_resize};

pub(crate) use release::{
    prepare as prepare_release, reconcile as reconcile_release, start as start_release,
};

pub(crate) fn start_registered(
    state: &Arc<DaemonRuntimeState>,
    actor: unleash_domain::ActorId,
    command: unleash_domain::CommandId,
) {
    control::start(state, actor, command);
}

type ObservationSlot =
    Arc<std::sync::Mutex<Option<unleash_store::terminal_allocations::TerminalLaunchObservation>>>;

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    terminal_id: unleash_domain::TerminalId,
    after_offset: u64,
    max_bytes: usize,
}

pub(crate) async fn read_log(
    state: &Arc<DaemonRuntimeState>,
    session: &crate::auth::AuthenticatedSession,
    query: unleash_protocol::QueryEnvelope,
) -> Result<unleash_protocol::QueryResponse, IpcError> {
    let hidden = || {
        IpcError::new(
            ErrorCode::NotFoundOrHidden,
            None,
            "Terminal observation is not available in this scope",
            RetryClass::Never,
        )
    };
    let raw: LogQuery =
        serde_json::from_value(serde_json::Value::Object(query.payload)).map_err(|_| hidden())?;
    if raw.max_bytes == 0 || raw.max_bytes > 65536 {
        return Err(hidden());
    }
    let store = state.store.clone();
    let actor = session.actor_id.clone();
    let project = query.project_id;
    let inspected = tokio::task::spawn_blocking(move || {
        let snapshot = store.inspect_terminal_allocation_snapshot(raw.terminal_id)?;
        let record = snapshot
            .value
            .ok_or_else(|| unleash_store::StoreError::NotFound("terminal missing".into()))?;
        let unleash_store::TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
            record.context.preparation.scope;
        if record.context.actor_id != actor || project != Some(project_id) {
            return Err(unleash_store::StoreError::InvalidArgument {
                reason: None,
                message: "terminal scope differs".into(),
            });
        }
        let observed = store.read_snapshot(|conn, _| read_log_binding(conn, &record))?;
        Ok::<_, unleash_store::StoreError>((record, observed, snapshot.cursor))
    })
    .await
    .map_err(|_| hidden())?
    .map_err(|_| hidden())?;
    let (record, pid, cursor) = inspected;
    let Some(binding) = pid else {
        if record.action_state == Some(unleash_domain::ActionState::OutcomeUnknown) {
            return Err(IpcError::new(
                ErrorCode::OutcomeUnknown,
                None,
                "Terminal launch requires reconciliation",
                RetryClass::AfterReconciliation,
            ));
        }
        return Err(IpcError::new(
            ErrorCode::ResourceBusy,
            None,
            "Terminal launch observation is pending",
            RetryClass::AfterRefresh,
        ));
    };
    let pid = binding.pid;
    // A read does not acquire/renew a lease or write input. Output is released
    // only after the broker observation and actual process identity committed.
    if record.context.authority_generation != state.authority_generation
        || binding.epoch != state.daemon_epoch
    {
        return Err(hidden());
    }
    let root = state.store.config().data_root.clone();
    let ids = state.store.config().id_generator.clone();
    let run = record.context.process_run_uuid;
    let outcome = tokio::task::spawn_blocking(move || {
        read_host_log(&root, ids.as_ref(), &record, &binding, &raw)
    })
    .await
    .map_err(|_| hidden())?
    .map_err(|_| hidden())?;
    let result = match outcome {
        HostLogRead::Page(result) => result,
        HostLogRead::CursorExpired => {
            return Err(IpcError::from_reason(
                ReasonCode::CursorExpired,
                "Le curseur de lecture du terminal a expiré",
            ));
        }
    };
    let text = result
        .segments
        .iter()
        .flat_map(|segment| segment.bytes.iter().copied())
        .collect::<Vec<_>>();
    Ok(unleash_protocol::QueryResponse {
        request_id: query.request_id,
        watermark: Some(cursor),
        result: serde_json::json!({"terminal_id":raw.terminal_id,"process_run_uuid":run,"pid":pid,"text":String::from_utf8_lossy(&text),"durable_log_seq":result.durable_watermark_seq,"log":result}),
    })
}

struct LogReadBinding {
    pid: u32,
    epoch: unleash_domain::DaemonEpoch,
    fence: unleash_domain::Fence,
}

fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn read_log_binding(
    conn: &rusqlite::Connection,
    record: &unleash_store::TerminalAllocationRecord,
) -> Result<Option<LogReadBinding>, unleash_store::StoreError> {
    use rusqlite::OptionalExtension;
    let c = &record.context;
    let invalid =
        || unleash_store::StoreError::InvariantViolation("terminal epoch or fence invalid".into());
    conn.query_row("SELECT p.pid,l.daemon_epoch,l.fence FROM processes p JOIN terminal_sessions s ON s.process_id=p.process_id JOIN actions a ON a.action_id=p.action_id JOIN resource_leases l ON l.lease_id=s.lease_id JOIN daemon_state d ON d.singleton=1 JOIN principals owner ON owner.principal_id=l.owner_id AND owner.principal_kind='user' AND owner.state='active' WHERE s.terminal_id=?1 AND p.process_run_uuid=?2 AND p.action_id=?3 AND a.state='observed_success' AND p.authority_generation=d.authority_generation AND l.authority_generation=d.authority_generation AND l.boot_identity=d.boot_identity AND l.resource_kind='terminal' AND l.owner_type='principal' AND l.owner_id=?4 AND l.daemon_epoch=d.daemon_epoch AND ((l.lease_id=?5 AND l.fence=?6 AND l.daemon_epoch=?7) OR EXISTS(SELECT 1 FROM terminal_control_intents control JOIN actions transfer ON transfer.action_id=control.action_id WHERE control.terminal_id=s.terminal_id AND control.next_lease_id=l.lease_id AND control.authority_generation=d.authority_generation AND control.daemon_epoch=l.daemon_epoch AND control.next_fence=l.fence AND control.state='observed' AND control.observation_object_hash IS NOT NULL AND transfer.state='observed_success' AND json_extract(control.context_json,'$.initial_context_digest')=?8))",rusqlite::params![c.terminal_id.to_string(),c.process_run_uuid.to_string(),c.action_id.to_string(),c.principal_id.as_str(),c.lease_id.to_string(),i64::try_from(c.resource_fence.as_u64()).map_err(|_|invalid())?,i64::try_from(c.daemon_epoch.as_u64()).map_err(|_|invalid())?,record.context_digest.to_string()],|row|Ok(LogReadBinding {pid:row.get(0)?,epoch:unleash_domain::DaemonEpoch::new(sql_u64(row,1)?),fence:unleash_domain::Fence::new(sql_u64(row,2)?)})).optional().map_err(unleash_store::StoreError::from)
}

enum HostLogRead {
    Page(unleash_process_host::LogReadResult),
    CursorExpired,
}

fn read_host_log(
    root: &std::path::Path,
    ids: &dyn unleash_domain::IdGeneratorPort,
    record: &unleash_store::TerminalAllocationRecord,
    binding: &LogReadBinding,
    raw: &LogQuery,
) -> Result<HostLogRead, TerminalError> {
    let c = &record.context;
    let runtime = root.join("process-hosts").join(c.host_id.to_string());
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    if manifest.allocation_context_digest != Some(record.context_digest)
        || manifest.process_run_uuid != c.process_run_uuid
        || manifest.host_id != c.host_id
        || manifest.action_id != c.action_id
    {
        return Err("terminal manifest binding differs".into());
    }
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        c.process_run_uuid,
        c.authority_generation,
        binding.epoch,
        binding.fence,
    )?;
    let header = client.make_header(ids.new_command_id(), c.action_id, record.payload_digest());
    let HostResponse::HandshakeAck {
        os_identity,
        allocation_context_digest,
        effective_environment_digest,
        ..
    } = client.call(&HostRequest::Handshake {
        header,
        current_secret: secret.expose_secret().into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?
    else {
        return Err("terminal handshake refused".into());
    };
    if os_identity.pid != binding.pid
        || allocation_context_digest != Some(record.context_digest)
        || effective_environment_digest != manifest.effective_environment_digest
    {
        return Err("terminal live identity differs".into());
    }
    let header = client.make_header(ids.new_command_id(), c.action_id, record.payload_digest());
    match client.call(&HostRequest::ReadLog {
        header,
        secret: secret.expose_secret().into(),
        after_offset: raw.after_offset,
        max_bytes: raw.max_bytes,
        strict_cursor: true,
    })? {
        HostResponse::LogChunk { result } => Ok(HostLogRead::Page(result)),
        HostResponse::Error { error }
            if error.code == ErrorCode::ResyncRequired
                && error.reason_code == Some(ReasonCode::CursorExpired)
                && error.retry_class == RetryClass::AfterRefresh =>
        {
            Ok(HostLogRead::CursorExpired)
        }
        _ => Err("terminal log unavailable".into()),
    }
}

pub(crate) fn start(state: &Arc<DaemonRuntimeState>, terminal: unleash_domain::TerminalId) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let state = state.clone();
    tokio::spawn(async move {
        let activity = Arc::new(activity);
        let store = state.store.clone();
        let record = tokio::task::spawn_blocking(move || {
            store.inspect_terminal_allocation_snapshot(terminal)
        })
        .await;
        let Ok(Ok(snapshot)) = record else {
            return;
        };
        let Some(record) = snapshot.value else {
            return;
        };
        if record.state != unleash_store::TerminalAllocationState::Allocated {
            return;
        }
        if run_allocation(state, record, activity).await.is_err() {
            tracing::warn!("Terminal acquisition has no confirmed observation; inspect recovery");
        }
    });
}

async fn run_allocation(
    state: Arc<DaemonRuntimeState>,
    record: unleash_store::TerminalAllocationRecord,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
) -> Result<(), ()> {
    let c = &record.context;
    let unleash_store::TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    if c.authority_generation != state.authority_generation || c.daemon_epoch != state.daemon_epoch
    {
        return Err(());
    }
    let store = state.store.clone();
    let policy =
        tokio::task::spawn_blocking(move || store.read_policy_summary_snapshot(Some(project_id)))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?
            .value;
    if policy.effective_isolation_mode != unleash_store::PolicyIsolationMode::Standard
        || policy
            .guard_config
            .as_ref()
            .is_some_and(|g| g.guard_mode != unleash_store::GuardPolicyMode::BrokerOnly)
    {
        return Err(());
    }
    let AllocationAuthorization {
        authority,
        fence,
        intent,
    } = allocation_authorization(&record)?;
    let slot: ObservationSlot = Arc::new(std::sync::Mutex::new(None));
    let broker = allocation_broker(&state, slot.clone(), fence);
    broker.bump_scope_revision(Some(project_id), revision);
    let decision = match broker
        .authorize_action(intent, &authority)
        .await
        .map_err(|error| {tracing::warn!(terminal_stage="authorization",code=%error.error_code(),reason=?error.reason_code());})?
    {
        AuthorizationOutcome::Authorized(d) => d,
        AuthorizationOutcome::ApprovalRequired { .. } => return Err(()),
    };
    let action = c.action_id;
    let executor = TerminalExecutor {
        state: state.clone(),
        record,
        slot,
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("No terminal secrets in this qualified path"),
    ));
    let receipt=broker
        .execute_authorized(
            action,
            &decision.decision_ref,
            fence,
            &authority,
            &executor,
            &secrets,
        )
        .await
        .map_err(|error| {tracing::warn!(terminal_stage="execution_or_observation",code=%error.error_code(),reason=?error.reason_code());})?;
    if receipt.state != unleash_domain::ActionState::ObservedSuccess {
        return Err(());
    }
    Ok(())
}

struct AllocationAuthorization {
    authority: EffectiveAuthority,
    fence: LineageFence,
    intent: ActionIntent,
}

fn allocation_authorization(
    record: &unleash_store::TerminalAllocationRecord,
) -> Result<AllocationAuthorization, ()> {
    let c = &record.context;
    let unleash_store::TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    let authority = compute_effective_authority(
        Some(project_id),
        c.policy_revision,
        revision,
        &[AuthorityLayer {
            layer_name: "authenticated_human_terminal".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([
                unleash_store::TERMINAL_ALLOCATION_VERB.into()
            ])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![std::path::PathBuf::from(
                &c.preparation.process_spec.cwd_canonical,
            )]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: BrokerIsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Project,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let intent = ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: Some(project_id),
        verb: unleash_store::TERMINAL_ALLOCATION_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.payload_digest(),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.lease_expires_at,
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    };
    Ok(AllocationAuthorization {
        authority,
        fence,
        intent,
    })
}

fn allocation_broker(
    state: &Arc<DaemonRuntimeState>,
    slot: ObservationSlot,
    fence: LineageFence,
) -> ActionBroker {
    let timer = Arc::new(SystemClockAndTimer::new());
    ActionBroker::new(
        state.store.config().clock.clone(),
        timer.clone(),
        timer,
        state.store.config().id_generator.clone(),
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
            crate::broker_journal::SqliteBrokerJournal::with_terminal_observation(state, slot),
        ),
    )
}

struct TerminalExecutor {
    state: Arc<DaemonRuntimeState>,
    record: unleash_store::TerminalAllocationRecord,
    slot: ObservationSlot,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
}
impl unleash_action_broker::ActionExecutorPort for TerminalExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch: unleash_domain::DispatchId,
        intent: &'a unleash_action_broker::ActionIntent,
        decision: &'a unleash_action_broker::AuthorizationDecision,
        secrets: &'a [unleash_action_broker::ResolvedSecret],
    ) -> unleash_action_broker::BoxFuture<
        'a,
        Result<
            unleash_action_broker::ActionExecutionResult,
            unleash_action_broker::ActionExecutionFailure,
        >,
    > {
        Box::pin(async move {
            let unknown = || unleash_action_broker::ActionExecutionFailure::OutcomeUnknown {
                dispatch_id: dispatch,
                reason: "Terminal launch requires reconciliation".into(),
            };
            if !secrets.is_empty()
                || intent.action_id != self.record.context.action_id
                || decision.action_id != intent.action_id
                || intent.payload_digest != self.record.payload_digest()
            {
                return Err(unknown());
            }
            let state = self.state.clone();
            let record = self.record.clone();
            let activity = self.activity.clone();
            let observation = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                spawn_and_observe(&state, &record, dispatch)
            })
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            let digest = unleash_protocol::canonical_sha256(&observation).map_err(|_| unknown())?;
            let observed_at_ms = observation.observed_at_ms;
            *self.slot.lock().map_err(|_| unknown())? = Some(observation);
            Ok(unleash_action_broker::ActionExecutionResult {
                dispatch_id: dispatch,
                provider_request_id: None,
                output_digest: digest,
                observed_at_ms,
            })
        })
    }
}

type TerminalError = Box<dyn std::error::Error + Send + Sync>;

struct HostLaunch {
    spec: HostBootstrapSpec,
    secret: HostControlSecret,
    expected: unleash_domain::Sha256Digest,
}

fn spawn_and_observe(
    state: &Arc<DaemonRuntimeState>,
    record: &unleash_store::TerminalAllocationRecord,
    dispatch: unleash_domain::DispatchId,
) -> Result<unleash_store::terminal_allocations::TerminalLaunchObservation, TerminalError> {
    let profile = resolve_launch_permit(state, record, dispatch)?;
    let launch = prepare_host_launch(state, record, profile)?;
    let expected_host = spawn_registered_host(state, &launch)?;
    let observation = observe_spawned_host(state, record, dispatch, &launch, &expected_host)?;
    publish_launch_observation(state, &observation)?;
    Ok(observation)
}

fn resolve_launch_permit(
    state: &Arc<DaemonRuntimeState>,
    record: &unleash_store::TerminalAllocationRecord,
    dispatch: unleash_domain::DispatchId,
) -> Result<unleash_store::terminal_allocations::TerminalEnvironmentProfile, TerminalError> {
    let c = &record.context;
    let profile = state
        .store
        .terminal_runtime_settings()?
        .profiles
        .into_iter()
        .find(|p| {
            p.profile_id == c.preparation.process_spec.environment.profile_id
                && p.version == c.preparation.process_spec.environment.version
        })
        .ok_or("terminal environment changed")?;
    if profile.digest()? != c.preparation.process_spec.environment.digest {
        return Err("terminal environment digest changed".into());
    }
    let process = &c.preparation.process_spec;
    if std::fs::canonicalize(&process.cwd_canonical)?
        != std::path::Path::new(&process.cwd_canonical)
    {
        return Err("terminal cwd changed".into());
    }
    // This ACK is a one-use permit. Any error or cancellation after it is inspected,
    // never followed by an automatic second resolution or another physical launch.
    state
        .store
        .resolve_terminal_launch(ResolveTerminalLaunchRequest {
            terminal_id: c.terminal_id,
            process_run_uuid: c.process_run_uuid,
            action_id: c.action_id,
            dispatch_id: dispatch,
            authority_generation: c.authority_generation,
            expected_revision: record.revision,
            context_digest: record.context_digest,
            payload_digest: record.payload_digest(),
            caller: CallerAuthority {
                actor_id: c.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
            lease: FenceAssertion {
                lease_id: c.lease_id,
                owner_id: c.principal_id.to_string(),
                authority_generation: c.authority_generation,
                daemon_epoch: c.daemon_epoch,
                fence: c.resource_fence,
            },
        })?;
    Ok(profile)
}

fn prepare_host_launch(
    state: &Arc<DaemonRuntimeState>,
    record: &unleash_store::TerminalAllocationRecord,
    profile: unleash_store::terminal_allocations::TerminalEnvironmentProfile,
) -> Result<HostLaunch, TerminalError> {
    let c = &record.context;
    let process = &c.preparation.process_spec;
    let base = state.store.config().data_root.join("process-hosts");
    unleash_platform::ensure_private_rendezvous_dir(&base)?;
    let runtime = base.join(c.host_id.to_string());
    unleash_platform::ensure_private_rendezvous_dir(&runtime)?;
    let runtime = runtime.canonicalize()?;
    let secret = HostControlSecret::new(format!(
        "terminal-control:{}",
        state.store.config().id_generator.new_uuid()
    ))?;
    let secret_path = runtime.join(CONTROL_SECRET_FILENAME);
    store_control_secret(&secret_path, &secret)?;
    let effective = explicit_environment_v1(&profile.variables, &runtime.join("child-tmp"))?;
    let expected = effective_environment_digest_v1(&effective)?;
    let spec = HostBootstrapSpec {
        environment_mode: Some(EnvironmentMode::ExplicitV1),
        expected_effective_environment_digest: Some(expected),
        allocation_context_digest: Some(record.context_digest),
        runtime_dir: runtime.clone(),
        host_id: c.host_id,
        process_id: c.process_id,
        terminal_id: Some(c.terminal_id),
        process_run_uuid: c.process_run_uuid,
        authority_generation: c.authority_generation,
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.resource_fence,
        action_id: c.action_id,
        mission_id: Some(c.preparation.mission_id),
        owner_agent_id: None,
        worktree_id: None,
        cwd: process.cwd_canonical.clone().into(),
        program: process.program.clone(),
        arguments: process.arguments.clone(),
        explicit_env: profile.variables,
        resource_kind: ResourceKind::Pty {
            rows: process.rows,
            cols: process.cols,
        },
        isolation_mode: HostIsolationMode::Standard,
        strict_spec: None,
        isolation_profile_kind: None,
        control_secret_path: secret_path.clone(),
        canaries_secret_path: None,
        max_segment_bytes: 65536,
        max_total_log_bytes: 1_048_576,
        max_volatile_bytes: 65536,
    };
    let bootstrap = runtime.join("bootstrap.json");
    unleash_platform::write_private_file_atomic(&bootstrap, &serde_json::to_vec(&spec)?)?;
    Ok(HostLaunch {
        spec,
        secret,
        expected,
    })
}

fn spawn_registered_host(
    state: &Arc<DaemonRuntimeState>,
    launch: &HostLaunch,
) -> Result<unleash_platform::OsProcessIdentity, TerminalError> {
    let spec = &launch.spec;
    let c = spec;
    let bootstrap = spec.runtime_dir.join("bootstrap.json");
    let executable = std::env::current_exe()?
        .parent()
        .ok_or("daemon executable has no directory")?
        .join("unleash-process-host");
    let child = state
        .terminal_host_launcher
        .spawn(&executable, &bootstrap)?;
    let host_identity = unleash_platform::inspect_process_identity(child.id())?;
    {
        let mut children = state
            .terminal_children
            .lock()
            .map_err(|_| "terminal child registry poisoned")?;
        children.retain_mut(|child| child.try_wait().ok().flatten().is_none());
        children.push(child);
        crate::executors::RendezvousExecutorController::upsert_registration(
            &state.rendezvous_dir,
            crate::executors::ExecutorRegistration {
                executor_id: c.host_id.to_string(),
                data_root: state.store.config().data_root.clone(),
                pid: Some(host_identity.pid),
                boot_identity: Some(host_identity.boot_identity.as_str().into()),
                start_identity: Some(host_identity.start_identity.as_str().into()),
                os_identity: Some(host_identity.clone()),
                state: crate::executors::ExecutorState::Active,
            },
        )?;
    }
    Ok(host_identity)
}

fn connect_spawned_host(
    record: &unleash_store::TerminalAllocationRecord,
    launch: &HostLaunch,
    expected_host: &unleash_platform::OsProcessIdentity,
) -> Result<HostClient, TerminalError> {
    let c = &record.context;
    let process = &c.preparation.process_spec;
    let runtime = &launch.spec.runtime_dir;
    let spec = &launch.spec;
    let expected = launch.expected;
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(process.readiness_timeout_ms);
    let client = loop {
        match HostClient::connect(
            &runtime.join(HOST_SOCKET_FILENAME),
            c.process_run_uuid,
            c.authority_generation,
            c.daemon_epoch,
            c.resource_fence,
        ) {
            Ok(client) => break client,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    };
    client.verify_owned_peer(expected_host)?;
    let before = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    if before.allocation_context_digest != Some(record.context_digest)
        || before.effective_environment_digest != Some(expected)
        || before.action_id != c.action_id
        || before.host_id != c.host_id
        || before.process_id != c.process_id
        || before.process_run_uuid != c.process_run_uuid
        || before.command_digest != spec.compute_command_digest()?
    {
        return Err("host manifest does not match launch".into());
    }
    Ok(client)
}

fn acknowledge_spawned_host(
    state: &Arc<DaemonRuntimeState>,
    record: &unleash_store::TerminalAllocationRecord,
    launch: &HostLaunch,
    client: &mut HostClient,
) -> Result<unleash_platform::OsProcessIdentity, TerminalError> {
    let c = &record.context;
    let secret = &launch.secret;
    let expected = launch.expected;
    let header = client.make_header(
        state.store.config().id_generator.new_command_id(),
        c.action_id,
        record.payload_digest(),
    );
    let HostResponse::HandshakeAck {
        os_identity,
        effective_environment_digest,
        allocation_context_digest,
        ..
    } = client.call(&HostRequest::Handshake {
        header,
        current_secret: secret.expose_secret().into(),
        rotate_to_secret: None,
        require_higher_epoch: false,
    })?
    else {
        return Err("host did not acknowledge launch".into());
    };
    if effective_environment_digest != Some(expected)
        || allocation_context_digest != Some(record.context_digest)
        || os_identity.owner_uid != unleash_platform::effective_uid()
    {
        return Err("host applied proof differs".into());
    }
    Ok(os_identity)
}

fn observe_spawned_host(
    state: &Arc<DaemonRuntimeState>,
    record: &unleash_store::TerminalAllocationRecord,
    dispatch: unleash_domain::DispatchId,
    launch: &HostLaunch,
    expected_host: &unleash_platform::OsProcessIdentity,
) -> Result<unleash_store::terminal_allocations::TerminalLaunchObservation, TerminalError> {
    let c = &record.context;
    let runtime = &launch.spec.runtime_dir;
    let secret = &launch.secret;
    let expected = launch.expected;
    let mut client = connect_spawned_host(record, launch, expected_host)?;
    let os_identity = acknowledge_spawned_host(state, record, launch, &mut client)?;
    client.verify_owned_peer(expected_host)?;
    let header = client.make_header(
        state.store.config().id_generator.new_command_id(),
        c.action_id,
        record.payload_digest(),
    );
    let HostResponse::ReconcileState {
        exit_status,
        durable_log_seq,
        last_confirmed_input_seq,
        unknown_input_seqs,
        ..
    } = client.call(&HostRequest::Reconcile {
        header,
        secret: secret.expose_secret().into(),
    })?
    else {
        return Err("host launch observation unavailable".into());
    };
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    if last_confirmed_input_seq != 0
        || !unknown_input_seqs.is_empty()
        || manifest.last_confirmed_input_seq != last_confirmed_input_seq
        || manifest.in_flight_input_seq.is_some()
    {
        return Err("new terminal has no qualified initial native input state".into());
    }
    let observation = unleash_store::terminal_allocations::TerminalLaunchObservation {
        terminal_id: c.terminal_id,
        action_id: c.action_id,
        dispatch_id: dispatch,
        context_digest: record.context_digest,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        pid: os_identity.pid,
        pgid: os_identity.pgid,
        owner_uid: os_identity.owner_uid,
        boot_identity: os_identity.boot_identity.as_str().into(),
        start_identity: os_identity.start_identity.as_str().into(),
        identity_backend: match os_identity.backend {
            unleash_platform::IdentityBackend::MacosProcPidinfo => "macos_proc_pidinfo",
            unleash_platform::IdentityBackend::LinuxProcfs => "linux_procfs",
            _ => return Err("terminal OS identity backend is not qualified".into()),
        }
        .into(),
        manifest_digest: unleash_protocol::canonical_sha256(&manifest)?,
        effective_environment_digest: expected,
        durable_log_seq,
        input_state: Some(unleash_store::terminal_allocations::terminal_acquisition_query::TerminalObservedInputState {
            last_confirmed_input_seq,
            unknown_input_seqs,
        }),
        observed_at_ms: state.store.config().clock.now_ms(),
        exit_code: exit_status.and_then(|s| s.exit_code),
        exit_signal: exit_status.and_then(|s| s.exit_signal),
        exited: exit_status.is_some(),
    };
    Ok(observation)
}

fn publish_launch_observation(
    state: &Arc<DaemonRuntimeState>,
    observation: &unleash_store::terminal_allocations::TerminalLaunchObservation,
) -> Result<(), TerminalError> {
    let terminal = observation.terminal_id;
    let dispatch = observation.dispatch_id;
    let bytes = unleash_protocol::canonicalize_to_vec(observation)?;
    let digest = unleash_protocol::canonical_sha256(observation)?;
    let object = state.store.prepare_object(PrepareObjectRequest {
        object_hash: digest,
        byte_length: bytes.len() as u64,
        media_type: "application/json".into(),
        owner_kind: "terminal_launch_observation".into(),
        owner_id: terminal.to_string(),
    })?;
    state.store.write_prepared_object_bytes(&object, &bytes)?;
    state
        .store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: digest,
            object_generation: object.object_generation,
            owner_kind: "terminal_launch_observation".into(),
            owner_id: terminal.to_string(),
            source_field: dispatch.to_string(),
        })?;
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PtyDimensions {
    rows: u16,
    cols: u16,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessReadiness {
    kind: String,
    timeout_ms: u64,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTerminalPayload {
    mission_id: unleash_domain::MissionId,
    invocation_id: Option<unleash_domain::InvocationId>,
    purpose_key: String,
    reuse_policy: String,
    scope_id: String,
    scope_revision: unleash_domain::Revision,
    worktree_id: Option<unleash_domain::WorktreeId>,
    cwd: String,
    program: String,
    arguments: Vec<String>,
    environment_profile_id: String,
    pty: PtyDimensions,
    readiness: ProcessReadiness,
}

pub(crate) async fn prepare_acquisition(
    state: &Arc<DaemonRuntimeState>,
    session: &crate::auth::AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<unleash_store::CommitCommandRequest, IpcError> {
    use unleash_store::terminal_allocations::{
        NormalizedTerminalProcessSpec, TerminalAllocationScope, TerminalEnvironmentRef,
    };
    use unleash_store::{
        CallerAuthority, CallerRole, CommandMutation, TerminalAllocationPreparation,
    };
    let bad = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Terminal request or configured scope is inadmissible",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    if !matches!(
        session.client_kind,
        unleash_protocol::ClientKind::Cli
            | unleash_protocol::ClientKind::Tui
            | unleash_protocol::ClientKind::Gui
    ) {
        return Err(bad());
    }
    let project = command.project_id.ok_or_else(bad)?;
    if ["terminal_id", "process_run_uuid", "control_mode"]
        .iter()
        .any(|key| command.payload.contains_key(*key))
    {
        return prepare_registered_acquisition(session, command);
    }
    let terminal = unleash_domain::TerminalId::from_uuid(command.aggregate_id);
    let store = state.store.clone();
    let actor = session.actor_id.clone();
    let envelope = command.clone();
    let preparation=tokio::task::spawn_blocking(move||->Result<TerminalAllocationPreparation,unleash_store::StoreError> {
        let invalid=||unleash_store::StoreError::InvalidArgument {reason:None,message:"Terminal request or configuration invalid".into()};
        if store.inspect_command_receipt(&actor,envelope.command_id).is_ok() {
            return store.inspect_terminal_allocation_snapshot(terminal)?.value.map(|r|r.context.preparation).ok_or_else(invalid);
        }
        let raw:NewTerminalPayload=serde_json::from_value(serde_json::Value::Object(envelope.payload.clone())).map_err(|_|invalid())?;
        if raw.invocation_id.is_some() || raw.worktree_id.is_some() || raw.reuse_policy!="reuse_compatible"
            || raw.scope_id!=project.to_string() || raw.readiness.kind!="process_started"
            || raw.arguments.len()>128 || raw.arguments.iter().map(String::len).sum::<usize>()>65536
            || raw.arguments.iter().any(|a|a.contains('\0')) {return Err(invalid());}
        let configured=store.terminal_runtime_settings()?;
        let environment=configured.profiles.iter().find(|p|p.profile_id==raw.environment_profile_id).ok_or_else(invalid)?;
        let scope=store.get_project(project)?;
        if scope.revision!=raw.scope_revision {return Err(invalid());}
        let mission_revision=store.read_snapshot(|conn,_| {
            use rusqlite::OptionalExtension;
            conn.query_row("SELECT m.revision FROM missions m JOIN principals p ON p.principal_id=m.owner_user_id WHERE m.mission_id=?1 AND m.project_id=?2 AND m.owner_kind='user' AND m.state IN ('ready','active') AND p.actor_id=?3 AND p.principal_kind='user' AND p.state='active'",rusqlite::params![raw.mission_id.to_string(),project.to_string(),actor.as_str()],|r|r.get::<_,i64>(0)).optional()?.ok_or_else(invalid)
        })?;
        let root=std::path::PathBuf::from(scope.primary_root_path.ok_or_else(invalid)?);
        let relative=std::path::Path::new(&raw.cwd);
        if relative.as_os_str().is_empty() || relative.components().any(|c|!matches!(c,std::path::Component::Normal(_)|std::path::Component::CurDir)) {return Err(invalid());}
        let canonical_root=root.canonicalize().map_err(|_|invalid())?;
        let cwd=root.join(relative).canonicalize().map_err(|_|invalid())?;
        if root!=canonical_root || !cwd.starts_with(&canonical_root) || cwd!=root.join(relative) || !cwd.is_dir() {return Err(invalid());}
        let program=std::path::Path::new(&raw.program);
        let metadata=std::fs::metadata(program).map_err(|_|invalid())?;
        if !program.is_absolute() || !metadata.is_file() || metadata.permissions().mode()&0o111==0 || metadata.permissions().mode()&0o6000!=0 {return Err(invalid());}
        Ok(TerminalAllocationPreparation {mission_id:raw.mission_id,mission_revision:unleash_domain::Revision::new(u64::try_from(mission_revision).map_err(|_|invalid())?),scope:TerminalAllocationScope::ProjectConfiguration {project_id:project,revision:scope.revision},purpose_key:raw.purpose_key,
            process_spec:NormalizedTerminalProcessSpec {program:raw.program,arguments:raw.arguments,requested_cwd:raw.cwd,cwd_canonical:cwd.to_str().ok_or_else(invalid)?.into(),environment:TerminalEnvironmentRef {profile_id:environment.profile_id.clone(),version:environment.version,digest:environment.digest()?},rows:raw.pty.rows,cols:raw.pty.cols,readiness_timeout_ms:raw.readiness.timeout_ms},auxiliary:configured.pools(),lease_ttl_ms:configured.lease_ttl_ms})
    }).await.map_err(|_|bad())?.map_err(|e|crate::error::store_error_to_ipc_error(&e,Some(command.command_id)))?;
    Ok(unleash_store::CommitCommandRequest {
        actor_id: session.actor_id.clone(),
        envelope: command,
        target_aggregate: unleash_store::AggregateKind::TerminalAllocation,
        operation_id: None,
        receipt_state: unleash_protocol::ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalAllocation {
            terminal_id: terminal,
            caller: CallerAuthority {
                actor_id: session.actor_id.clone(),
                role: CallerRole::PolicyAdmin,
            },
            preparation,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

fn prepare_registered_acquisition(
    session: &crate::auth::AuthenticatedSession,
    command: CommandEnvelope,
) -> Result<unleash_store::CommitCommandRequest, IpcError> {
    use unleash_store::CommandMutation;
    let bad = || {
        IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Terminal request or configured scope is inadmissible",
            RetryClass::Never,
        )
        .with_command_id(command.command_id)
    };
    let request: unleash_store::terminal_allocations::RegisteredTerminalControl =
        serde_json::from_value(serde_json::Value::Object(command.payload.clone()))
            .map_err(|_| bad())?;
    if request.terminal_id.as_uuid() != command.aggregate_id {
        return Err(bad());
    }
    Ok(unleash_store::CommitCommandRequest {
        actor_id: session.actor_id.clone(),
        envelope: command,
        target_aggregate: unleash_store::AggregateKind::TerminalControl,
        operation_id: None,
        receipt_state: unleash_protocol::ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalControl {
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

use std::sync::Arc;
use unleash_protocol::{CommandEnvelope, ErrorCode, IpcError, ReasonCode, RetryClass};
use unleash_store::terminal_allocations::{PreparedTerminalEnvironment, TerminalRuntimeSettings};

fn invalid(command: &CommandEnvelope) -> IpcError {
    IpcError::new(
        ErrorCode::InvalidArgument,
        None,
        "Invalid terminal runtime configuration",
        RetryClass::Never,
    )
    .with_command_id(command.command_id)
}

pub(crate) async fn prepare_configuration_objects(
    state: &Arc<DaemonRuntimeState>,
    actor: &unleash_domain::ActorId,
    command: &CommandEnvelope,
) -> Result<Option<Vec<PreparedTerminalEnvironment>>, IpcError> {
    if command.command_type != "SetSettings" {
        return Ok(None);
    }
    let Some(raw) = command
        .payload
        .get("patch")
        .and_then(|v| v.get("terminal_runtime"))
    else {
        return Ok(None);
    };
    if command.project_id.is_some()
        || command.aggregate_id != state.authority_generation.as_uuid()
        || command
            .payload
            .get("scope_kind")
            .and_then(serde_json::Value::as_str)
            != Some("user")
        || command
            .payload
            .get("scope_id")
            .and_then(serde_json::Value::as_str)
            != Some("default")
    {
        return Err(invalid(command));
    }
    let config: TerminalRuntimeSettings =
        serde_json::from_value(raw.clone()).map_err(|_| invalid(command))?;
    config.validate().map_err(|_| invalid(command))?;
    let store = state.store.clone();
    let expected = command.expected_revision;
    let id = command.command_id;
    let actor = actor.clone();
    tokio::task::spawn_blocking(move || {
        // A known command is resolved by CommitCommand's exact digest/idempotence check.
        // Neither a stale CAS nor a changed payload may publish objects before that check.
        if store.inspect_command_receipt(&actor, id).is_ok() {
            return Ok(None);
        }
        let current = store.read_settings(unleash_store::SettingsScopeKind::User, "default")?;
        if current.revision != expected {
            return Err(unleash_store::StoreError::RevisionConflict {
                current_revision: current.revision,
                expected_revision: expected,
                command_id: Some(id),
            });
        }
        if let Some(old) = current.extra_typed.get("terminal_runtime") {
            let old: TerminalRuntimeSettings = serde_json::from_value(old.clone())?;
            for profile in &config.profiles {
                if let Some(prior) = old
                    .profiles
                    .iter()
                    .find(|p| p.profile_id == profile.profile_id)
                    && (profile.version < prior.version
                        || (profile.version == prior.version && profile != prior))
                {
                    return Err(unleash_store::StoreError::InvalidArgument {
                        reason: None,
                        message: "terminal profile version is immutable".into(),
                    });
                }
            }
        }
        let prepared = config
            .profiles
            .iter()
            .map(|profile| store.prepare_terminal_environment(profile, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok::<_, unleash_store::StoreError>(Some(prepared))
    })
    .await
    .map_err(|_| invalid(command))?
    .map_err(|e| crate::error::store_error_to_ipc_error(&e, Some(id)))
}
