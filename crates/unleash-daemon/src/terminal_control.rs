//! Transfer an existing PTY's control; never invoke a process launcher.
use crate::ipc::DaemonRuntimeState;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use unleash_action_broker::{
    ActionBroker, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort, ActionIntent,
    ActionProvenance, ActionSubject, AuthorityLayer, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture, BrokerConfig, EffectsClass, HostIsolationProbe, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef,
    PlatformKeyringSecretStore, ResolvedSecret, ScopeKind, StrictNoneAcceptedObligationChecker,
    SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{
    ActorId, CommandId, DataLocality, DispatchId, Fence, LineageFence, ModelPolicy,
    ReconcileStrategy,
};
use unleash_process_host::{
    CONTROL_SECRET_FILENAME, ControlTransferRequest, ControlTransferResponse,
    HOST_MANIFEST_FILENAME, HOST_SOCKET_FILENAME, HostClient, HostControlSecret, HostManifest,
    effective_environment_digest_v1, explicit_environment_v1, load_control_secret,
    store_control_secret,
};
use unleash_store::{
    CallerAuthority, CallerRole, PrepareObjectRequest, PublishObjectReferenceRequest,
    terminal_allocations::{
        ResolveTerminalControlRequest, TERMINAL_CONTROL_VERB, TerminalControlObservation,
        TerminalControlRecord, TerminalControlState, TerminalIdentityBackend,
        TerminalProcessIdentity,
    },
};

type ObservationSlot = Arc<Mutex<Option<TerminalControlObservation>>>;
type ControlError = Box<dyn std::error::Error + Send + Sync>;

pub(super) fn start(state: &Arc<DaemonRuntimeState>, actor: ActorId, command: CommandId) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let state = state.clone();
    tokio::spawn(async move {
        let activity = Arc::new(activity);
        let store = state.store.clone();
        let found = tokio::task::spawn_blocking(move || {
            store.inspect_terminal_control_request(&actor, command)
        })
        .await;
        let Ok(Ok(snapshot)) = found else {
            return;
        };
        let Some(record) = snapshot.value else {
            return;
        };
        if record.state != TerminalControlState::Prepared
            || record.broker_action_state.is_some()
            || record.broker_dispatch_id.is_some()
        {
            return;
        }
        if run(state, record, activity).await.is_err() {
            tracing::warn!("Terminal control has no confirmed observation; inspect recovery");
        }
    });
}

async fn run(
    state: Arc<DaemonRuntimeState>,
    record: TerminalControlRecord,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
) -> Result<(), ()> {
    let c = &record.context;
    if c.authority_generation != state.authority_generation || c.daemon_epoch != state.daemon_epoch
    {
        return Err(());
    }
    let store = state.store.clone();
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
    let authority = compute_effective_authority(
        Some(project),
        c.policy_revision,
        c.scope_revision,
        &[AuthorityLayer {
            layer_name: "authenticated_human_terminal_control".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([TERMINAL_CONTROL_VERB.into()])),
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
    );
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let intent = ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: Some(project),
        verb: TERMINAL_CONTROL_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.context_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.expires_at_ms,
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    };
    let slot = Arc::new(Mutex::new(None));
    let timer = Arc::new(SystemClockAndTimer::new());
    let broker = ActionBroker::new(
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
            crate::broker_journal::SqliteBrokerJournal::with_terminal_control_observation(
                &state,
                slot.clone(),
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
    let executor = ControlExecutor {
        state: state.clone(),
        record,
        slot,
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("No terminal control SecretRefs in this path"),
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

struct ControlExecutor {
    state: Arc<DaemonRuntimeState>,
    record: TerminalControlRecord,
    slot: ObservationSlot,
    activity: Arc<crate::broker_activity::BrokerActivityGuard>,
}
impl ActionExecutorPort for ControlExecutor {
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
                reason: "Terminal control requires reconciliation".into(),
            };
            if !secrets.is_empty()
                || intent.action_id != self.record.context.action_id
                || decision.action_id != intent.action_id
                || intent.payload_digest != self.record.context_digest
            {
                return Err(unknown());
            }
            let state = self.state.clone();
            let record = self.record.clone();
            let activity = self.activity.clone();
            let observation = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                transfer_and_observe(&state, &record, dispatch)
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
    identity: &unleash_platform::OsProcessIdentity,
) -> Result<TerminalProcessIdentity, ControlError> {
    Ok(TerminalProcessIdentity {
        pid: identity.pid,
        pgid: identity.pgid,
        owner_uid: identity.owner_uid,
        boot_identity: identity.boot_identity.as_str().into(),
        start_identity: identity.start_identity.as_str().into(),
        backend: match identity.backend {
            unleash_platform::IdentityBackend::MacosProcPidinfo => {
                TerminalIdentityBackend::MacosProcPidinfo
            }
            unleash_platform::IdentityBackend::LinuxProcfs => TerminalIdentityBackend::LinuxProcfs,
            _ => return Err("terminal control identity backend is not qualified".into()),
        },
    })
}

fn transfer_and_observe(
    state: &Arc<DaemonRuntimeState>,
    record: &TerminalControlRecord,
    dispatch: DispatchId,
) -> Result<TerminalControlObservation, ControlError> {
    let c = &record.context;
    // One-use permit. A missing ACK never authorizes another call or spawn.
    state
        .store
        .resolve_terminal_control(ResolveTerminalControlRequest {
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
    let allocation = state
        .store
        .inspect_terminal_allocation_snapshot(c.terminal_id)?
        .value
        .ok_or("terminal initial allocation missing")?;
    if allocation.context_digest != c.initial_context_digest {
        return Err("terminal initial context changed".into());
    }
    let runtime = state
        .store
        .config()
        .data_root
        .canonicalize()?
        .join("process-hosts")
        .join(c.host_id.to_string());
    let canonical = runtime.canonicalize()?;
    if canonical != runtime {
        return Err("terminal host runtime path changed".into());
    }
    unleash_platform::validate_rendezvous_dir(&runtime)?;
    let before = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&before, record)?;
    let cwd = std::path::Path::new(&allocation.context.preparation.process_spec.cwd_canonical);
    if cwd.canonicalize()? != cwd || before.cwd_canonical != cwd {
        return Err("terminal configured working directory changed".into());
    }
    if before.daemon_epoch != c.previous_epoch || before.highest_fence != c.previous_fence {
        return Err("terminal host already advanced; inspect without repeating transfer".into());
    }
    let profile = state
        .store
        .terminal_runtime_settings()?
        .profiles
        .into_iter()
        .find(|profile| {
            profile.profile_id
                == allocation
                    .context
                    .preparation
                    .process_spec
                    .environment
                    .profile_id
                && profile.version
                    == allocation
                        .context
                        .preparation
                        .process_spec
                        .environment
                        .version
        })
        .ok_or("terminal control environment changed")?;
    if profile.digest()?
        != allocation
            .context
            .preparation
            .process_spec
            .environment
            .digest
    {
        return Err("terminal control environment differs".into());
    }
    let expected = effective_environment_digest_v1(&explicit_environment_v1(
        &profile.variables,
        &runtime.join("child-tmp"),
    )?)?;
    if before.effective_environment_digest != Some(expected) {
        return Err("terminal host applied environment differs".into());
    }
    let secret = load_control_secret(&runtime.join(CONTROL_SECRET_FILENAME))?;
    let next_path = runtime.join(format!("control-transfer-{}.secret", c.action_id));
    if std::fs::symlink_metadata(&next_path).is_ok() {
        return Err("terminal transfer capability already exists; inspect".into());
    }
    let next = HostControlSecret::new(format!(
        "terminal-control:{}",
        state.store.config().id_generator.new_uuid()
    ))?;
    store_control_secret(&next_path, &next)?;
    let mut client = HostClient::connect(
        &runtime.join(HOST_SOCKET_FILENAME),
        c.process_run_uuid,
        c.authority_generation,
        c.daemon_epoch,
        c.next_fence,
    )?;
    let header = client.make_header(c.command_id, c.action_id, record.context_digest);
    let request = ControlTransferRequest {
        header: header.clone(),
        dispatch_id: dispatch,
        initial_context_digest: c.initial_context_digest,
        current_secret: secret.expose_secret().into(),
        next_secret: next.expose_secret().into(),
    };
    let ControlTransferResponse::Ack { ack } = client.transfer_control(&request)? else {
        return Err("terminal transfer has no correlated acknowledgement".into());
    };
    if ack.header != header
        || ack.dispatch_id != dispatch
        || ack.initial_context_digest != c.initial_context_digest
        || ack.effective_environment_digest != expected
        || identity(&ack.os_identity)? != c.expected_os_identity
    {
        return Err("terminal transfer acknowledgement differs".into());
    }
    let manifest = HostManifest::load_private(&runtime.join(HOST_MANIFEST_FILENAME))?;
    validate_manifest(&manifest, record)?;
    if manifest.daemon_epoch != c.daemon_epoch
        || manifest.highest_fence != c.next_fence
        || manifest.effective_environment_digest != Some(expected)
    {
        return Err("terminal transfer manifest differs".into());
    }
    let observation = TerminalControlObservation {
        action_id: c.action_id,
        dispatch_id: dispatch,
        context_digest: record.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        os_identity: identity(&ack.os_identity)?,
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.next_fence,
        transfer_ack_digest: unleash_protocol::canonical_sha256(&ack)?,
        manifest_digest: unleash_protocol::canonical_sha256(&manifest)?,
        effective_environment_digest: expected,
        durable_log_seq: ack.durable_log_seq,
        last_confirmed_input_seq: ack.last_confirmed_input_seq,
        unknown_input_seqs: ack.unknown_input_seqs,
        observed_at_ms: state.store.config().clock.now_ms(),
    };
    publish(state, &observation)?;
    Ok(observation)
}

fn validate_manifest(
    manifest: &HostManifest,
    record: &TerminalControlRecord,
) -> Result<(), ControlError> {
    let c = &record.context;
    if manifest.host_id != c.host_id
        || manifest.process_id != c.process_id
        || manifest.action_id != c.initial_action_id
        || manifest.process_run_uuid != c.process_run_uuid
        || manifest.authority_generation != c.authority_generation
        || manifest.allocation_context_digest != Some(c.initial_context_digest)
        || manifest.pid != Some(c.expected_os_identity.pid)
        || manifest.pgid != Some(c.expected_os_identity.pgid)
        || manifest.owner_uid != c.expected_os_identity.owner_uid
        || manifest
            .boot_identity
            .as_ref()
            .map(unleash_platform::BootIdentity::as_str)
            != Some(c.expected_os_identity.boot_identity.as_str())
        || manifest
            .start_identity
            .as_ref()
            .map(unleash_platform::StartIdentity::as_str)
            != Some(c.expected_os_identity.start_identity.as_str())
    {
        return Err("terminal existing manifest registration differs".into());
    }
    Ok(())
}

fn publish(
    state: &Arc<DaemonRuntimeState>,
    observation: &TerminalControlObservation,
) -> Result<(), ControlError> {
    let bytes = unleash_protocol::canonicalize_to_vec(observation)?;
    let hash = unleash_protocol::canonical_sha256(observation)?;
    let object = state.store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: bytes.len() as u64,
        media_type: "application/json".into(),
        owner_kind: "terminal_control_observation".into(),
        owner_id: observation.action_id.to_string(),
    })?;
    state.store.write_prepared_object_bytes(&object, &bytes)?;
    state
        .store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: hash,
            object_generation: object.object_generation,
            owner_kind: "terminal_control_observation".into(),
            owner_id: observation.action_id.to_string(),
            source_field: observation.dispatch_id.to_string(),
        })?;
    Ok(())
}
