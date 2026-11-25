//! Broker evaluation for a prepared managed provider service.

use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
    process::{Child, ExitStatus},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionIntent, AuthorityLayer, AuthorizationDecision,
    AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry, BrokerJournalError,
    BrokerJournalPort, EffectsClass, GuardMode, InitialEffectivePolicy, IsolationBackendStatus,
    IsolationMode, IsolationProbePort, IsolationRequirements, LocalAuthorityFallbackPolicy,
    NONE_ACCEPTED_PROFILE_ID, ScopeKind, StrictNoneAcceptedObligationChecker, SystemClockAndTimer,
    compute_effective_authority,
};
use unleash_domain::{CommandId, DataLocality, DispatchId, Fence, ModelPolicy, Revision};
use unleash_platform::{OsProcessIdentity, PlatformError, validate_rendezvous_dir};
use unleash_process_host::{
    HostBootstrapSpec, HostControlSecret, ProcessHostError, StrictServiceLaunchObservation,
    attest_strict_service_launch, inspect_strict_service_launch_plan,
    probe_strict_service_confinement, store_control_secret,
};
use unleash_protocol::{CommandEnvelope, canonical_sha256};
use unleash_store::{
    BrokerEvaluation, BrokerPhase, BrokerPhaseRequest, CallerAuthority, GuardPolicyMode,
    GuardTransport, PolicyIsolationMode, PolicySummary, ProviderServiceLaunchContext,
    ProviderServiceLaunchFingerprint, ProviderServiceLaunchPermit, StoreError, UnleashStore,
};

use crate::terminal_host_launcher::{PackagedTerminalHostLauncher, TerminalHostLauncher as _};

mod controller;
pub(crate) use controller::ManagedProviderServiceHosts;
pub use controller::ManagedProviderServiceLaunch;

/// Failure while binding or evaluating a prepared launch.
#[derive(Debug, thiserror::Error)]
pub enum ProviderServiceEvaluationError {
    /// The durable preparation or writer was unavailable.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The broker refused or could not evaluate the preparation.
    #[error(transparent)]
    Broker(#[from] ActionBrokerError),
    /// An internal evaluation task could not be joined.
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
    /// The durable intent could not be decoded by the broker.
    #[error(transparent)]
    Intent(#[from] serde_json::Error),
    /// The process host rejected the prepared bootstrap.
    #[error(transparent)]
    Bootstrap(#[from] ProcessHostError),
}

/// Evaluate one new preparation through the durable service journal.
/// This operation does not issue a process launch permit.
///
/// # Errors
/// Returns the failure reported by the writer or broker.
pub async fn evaluate_provider_service_launch(
    store: Arc<UnleashStore>,
    envelope: CommandEnvelope,
    caller: CallerAuthority,
    bootstrap: HostBootstrapSpec,
) -> Result<AuthorizationOutcome, ProviderServiceEvaluationError> {
    evaluate_with_probe(store, envelope, caller, bootstrap, None).await
}

/// Obtain the exclusive durable permit for this evaluated service launch.
///
/// # Errors
/// Refuses stale or divergent launch material and any already marked dispatch.
pub async fn claim_prepared_provider_service_launch(
    store: Arc<UnleashStore>,
    envelope: CommandEnvelope,
    caller: CallerAuthority,
    bootstrap: HostBootstrapSpec,
    decision: AuthorizationDecision,
) -> Result<ProviderServiceLaunchPermit, ProviderServiceEvaluationError> {
    tokio::task::spawn_blocking(move || {
        let context = store.read_provider_service_launch_for_authorization(&envelope, &caller)?;
        let fingerprint = verify_bootstrap(&context, &bootstrap)?;
        let decision = crate::broker_journal::decision(&decision).map_err(|_| {
            StoreError::EvidenceUnavailable("service decision cannot be persisted".into())
        })?;
        let request = BrokerPhaseRequest {
            authority_generation: context.lineage_fence.authority_generation,
            expected_revision: Revision::new(2),
            evaluation: BrokerEvaluation {
                evaluation_id: decision.evaluation_id,
                previous_evaluation_id: None,
            },
            phase: BrokerPhase::Dispatch {
                action_id: decision.action_id,
                intent_digest: context.broker_intent()?.intent_digest,
                decision,
                dispatch_id: DispatchId::from_uuid(store.config().id_generator.new_uuid()),
                fence: context.lineage_fence,
            },
        };
        store
            .commit_provider_service_launch_dispatch(&envelope, &caller, request, fingerprint)
            .map_err(ProviderServiceEvaluationError::from)
    })
    .await?
}

/// Failure before launching or while observing a managed provider service.
#[derive(Debug, thiserror::Error)]
pub enum ProviderServiceLaunchError {
    /// The retained bootstrap differs from the broker's preparation.
    #[error(transparent)]
    Bootstrap(#[from] ProviderServiceEvaluationError),
    /// The writer refused the last pre-launch check.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The clock cannot establish that the permit is still live.
    #[error("service launch permit expired or clock uncertain")]
    StalePermit,
    /// The runtime can no longer admit a launch operation.
    #[error("daemon is not accepting provider service launches")]
    Draining,
    /// A fresh service launch needs a dedicated empty runtime directory.
    #[error("service launch runtime already in use")]
    RuntimeInUse,
    /// The owned process-host cannot be authenticated.
    #[error("service host identity unavailable")]
    HostUnverified,
    /// The daemon could not verify the exact child listener.
    #[error("managed service listener unavailable")]
    ListenerUnverified,
    /// Preparing or launching the packaged host failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The runtime path was rejected.
    #[error(transparent)]
    Platform(#[from] PlatformError),
    /// The bootstrap could not be encoded.
    #[error(transparent)]
    Encoding(#[from] serde_json::Error),
    /// A host secret or attestation was rejected.
    #[error(transparent)]
    ProcessHost(#[from] ProcessHostError),
    /// The launch or ordered publication task became unavailable.
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
}

/// Retains the directly spawned host even when its identity or attestation is uncertain.
#[must_use = "retain the launched host for observation and process ownership"]
struct OwnedProviderServiceHost {
    child: Child,
    host_identity: Option<OsProcessIdentity>,
    bootstrap: HostBootstrapSpec,
    secret: HostControlSecret,
    command_id: CommandId,
    dispatch_id: DispatchId,
}

impl OwnedProviderServiceHost {
    /// The durable dispatch for this owned host.
    #[must_use]
    const fn dispatch_id(&self) -> DispatchId {
        self.dispatch_id
    }

    /// Poll and reap the owned host after exit.
    ///
    /// # Errors
    /// Propagates an OS failure to check the child.
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Observe the ready service through its private authenticated control socket.
    ///
    /// # Errors
    /// An absent, exited or inconsistent host leaves the handle in the caller's ownership.
    fn attest(&mut self) -> Result<StrictServiceLaunchObservation, ProviderServiceLaunchError> {
        if self.try_wait()?.is_some() {
            return Err(ProviderServiceLaunchError::HostUnverified);
        }
        let host = self
            .host_identity
            .as_ref()
            .ok_or(ProviderServiceLaunchError::HostUnverified)?;
        Ok(attest_strict_service_launch(
            &self.bootstrap,
            host,
            &self.secret,
            self.command_id,
        )?)
    }
}

/// Secret de contrôle neuf d'un service fournisseur managé : 256 bits de la source d'aléa du
/// système, jamais dérivés du générateur d'identifiants du démon, dont la séquence est
/// reproductible.
pub(crate) fn fresh_provider_service_secret(
) -> Result<HostControlSecret, ProviderServiceLaunchError> {
    Ok(HostControlSecret::generate()?)
}

fn spawn_provider_service_host(
    store: &UnleashStore,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    bootstrap: HostBootstrapSpec,
    permit: ProviderServiceLaunchPermit,
    executable: &Path,
) -> Result<OwnedProviderServiceHost, ProviderServiceLaunchError> {
    let check_clock = || {
        let config = store.config();
        if config.clock_uncertain.load(Ordering::Acquire)
            || config.clock.now_ms() >= permit.expires_at()
        {
            return Err(ProviderServiceLaunchError::StalePermit);
        }
        Ok(())
    };
    check_clock()?;
    verify_bootstrap(permit.context(), &bootstrap)?;
    if bootstrap.resource_fence != Fence::new(1) || bootstrap.canaries_secret_path.is_some() {
        return Err(
            ProviderServiceEvaluationError::Bootstrap(ProcessHostError::InvalidArgument {
                reason: "service launch requires a fresh control fence and private runtime".into(),
            })
            .into(),
        );
    }
    validate_rendezvous_dir(&bootstrap.runtime_dir)?;
    if std::fs::read_dir(&bootstrap.runtime_dir)?.next().is_some() {
        return Err(ProviderServiceLaunchError::RuntimeInUse);
    }
    let bootstrap_path = bootstrap.runtime_dir.join("bootstrap.json");
    let secret = fresh_provider_service_secret()?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&bootstrap_path)?;
    serde_json::to_writer(&file, &bootstrap)?;
    file.sync_all()?;
    store_control_secret(&bootstrap.control_secret_path, &secret)?;
    store.validate_provider_service_launch_permit(envelope, caller, &permit)?;
    check_clock()?;
    let dispatch_id = permit.dispatch_id();
    // No path after the spawn boundary can reuse this permit.
    drop(permit);
    let child = PackagedTerminalHostLauncher.spawn(executable, &bootstrap_path)?;
    let host_identity = unleash_platform::inspect_process_identity(child.id())
        .ok()
        .filter(|identity| {
            identity.pgid == child.id() && identity.owner_uid == unleash_platform::effective_uid()
        });
    Ok(OwnedProviderServiceHost {
        child,
        host_identity,
        bootstrap,
        secret,
        command_id: envelope.command_id,
        dispatch_id,
    })
}

async fn evaluate_with_probe(
    store: Arc<UnleashStore>,
    envelope: CommandEnvelope,
    caller: CallerAuthority,
    bootstrap: HostBootstrapSpec,
    probe_override: Option<Arc<dyn IsolationProbePort>>,
) -> Result<AuthorizationOutcome, ProviderServiceEvaluationError> {
    let reader = Arc::clone(&store);
    let original = envelope.clone();
    let actor = caller.clone();
    let (context, snapshot, probe) =
        tokio::task::spawn_blocking(move || -> Result<_, ProviderServiceEvaluationError> {
            let (context, policy) =
                reader.read_provider_service_launch_evaluation(&original, &actor)?;
            let snapshot = initial_policy(&context, policy)?;
            verify_bootstrap(&context, &bootstrap)?;
            let probe: Arc<dyn IsolationProbePort> = if let Some(probe) = probe_override {
                probe
            } else {
                probe_strict_service_confinement(&bootstrap, context.plan.confinement_digest)?;
                Arc::new(PreparedServiceSeatbeltProbe)
            };
            Ok((context, snapshot, probe))
        })
        .await??;
    let intent: ActionIntent =
        serde_json::from_value(serde_json::Value::Object(context.broker_intent()?.manifest))?;
    let authority = compute_effective_authority(
        context.project_id,
        context.policy_revision,
        context.scope_revision,
        &[AuthorityLayer {
            layer_name: "provider_service_launch_preparation".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([intent.verb.clone()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![context.plan.executable]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Strict,
            require_action_isolation: true,
            max_scope_kind: if context.project_id.is_some() {
                ScopeKind::Project
            } else {
                ScopeKind::Global
            },
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );
    let timer = Arc::new(SystemClockAndTimer::new());
    let broker = ActionBroker::new_with_policy_snapshot(
        store.config().clock.clone(),
        timer.clone(),
        timer,
        store.config().id_generator.clone(),
        probe,
        Arc::new(StrictNoneAcceptedObligationChecker),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig {
            current_fence: context.lineage_fence,
            evaluate_timeout: Duration::from_secs(2),
            health_check_interval: Duration::from_secs(30),
            cache_max_entries: 1,
            cache_ttl: Duration::ZERO,
            canaries_to_redact: vec![],
        },
        Arc::new(ServiceEvaluationJournal {
            store,
            envelope,
            caller,
        }),
        snapshot,
    )?;
    Ok(broker.authorize_action(intent, &authority).await?)
}

// This adapter is created only by the evaluation that loaded the exact profile.
struct PreparedServiceSeatbeltProbe;

impl IsolationProbePort for PreparedServiceSeatbeltProbe {
    fn probe_isolation(&self, requested: &IsolationRequirements) -> IsolationBackendStatus {
        let supported = IsolationRequirements::strict_default();
        if requested == &supported {
            IsolationBackendStatus::Available {
                backend_name: "macos-seatbelt".into(),
                supported_capabilities: supported.required_capabilities,
                abi_version: "seatbelt-sbpl-v1".into(),
            }
        } else {
            IsolationBackendStatus::Unenforceable {
                backend_name: "macos-seatbelt".into(),
                missing_capabilities: requested.required_capabilities.clone(),
                reason: "isolation requirements differ from the managed service profile".into(),
                remediation: "evaluate the prepared Strict service profile".into(),
            }
        }
    }
}

fn verify_bootstrap(
    context: &ProviderServiceLaunchContext,
    bootstrap: &HostBootstrapSpec,
) -> Result<ProviderServiceLaunchFingerprint, ProviderServiceEvaluationError> {
    let unavailable = || {
        StoreError::EvidenceUnavailable(
            "provider service bootstrap differs from the durable preparation".to_owned(),
        )
    };
    let context_digest = canonical_sha256(context).map_err(StoreError::from)?;
    if bootstrap.allocation_context_digest != Some(context_digest)
        || bootstrap.host_id != context.host_id
        || bootstrap.process_id != context.process_id
        || bootstrap.process_run_uuid != context.process_run_uuid
        || bootstrap.action_id != context.action_id
        || bootstrap.authority_generation != context.lineage_fence.authority_generation
        || bootstrap.daemon_epoch != context.lineage_fence.epoch
        || std::path::Path::new(&bootstrap.program) != context.plan.executable.as_path()
    {
        return Err(unavailable().into());
    }
    let observed = inspect_strict_service_launch_plan(bootstrap)?;
    if observed.executable_digest != context.plan.executable_digest
        || observed.command_digest != context.plan.command_digest
        || observed.environment_digest != context.plan.environment_digest
        || observed.confinement_digest != context.plan.confinement_digest
    {
        return Err(unavailable().into());
    }
    Ok(ProviderServiceLaunchFingerprint {
        context_digest,
        executable_digest: observed.executable_digest,
        command_digest: observed.command_digest,
        environment_digest: observed.environment_digest,
        confinement_digest: observed.confinement_digest,
    })
}

fn initial_policy(
    context: &ProviderServiceLaunchContext,
    policy: PolicySummary,
) -> Result<InitialEffectivePolicy, ActionBrokerError> {
    if let Some(guard) = policy.guard_config
        && (guard.guard_mode != GuardPolicyMode::BrokerOnly
            || guard.endpoint.is_some()
            || guard.transport != GuardTransport::None
            || guard.secret_ref_id.is_some()
            || guard.fallback_policy_ref.is_some()
            || guard.guard_mapping_profile != NONE_ACCEPTED_PROFILE_ID)
    {
        return Err(ActionBrokerError::GuardUnavailable {
            reason: "Provider service evaluation requires a qualified Guard port".into(),
        });
    }
    Ok(InitialEffectivePolicy {
        project_id: context.project_id,
        policy_revision: context.policy_revision,
        scope_revision: context.scope_revision,
        policy_digest: context.policy_digest,
        isolation_mode: match policy.effective_isolation_mode {
            PolicyIsolationMode::Standard => IsolationMode::Standard,
            PolicyIsolationMode::Strict => IsolationMode::Strict,
        },
        guard_mode: GuardMode::BrokerOnly,
        lineage_fence: context.lineage_fence,
    })
}

struct ServiceEvaluationJournal {
    store: Arc<UnleashStore>,
    envelope: CommandEnvelope,
    caller: CallerAuthority,
}

impl BrokerJournalPort for ServiceEvaluationJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            let (evaluation, intent, phase) = crate::broker_journal::project(entry)?;
            let expected_revision = match phase {
                BrokerPhase::Authorizing { .. } => Revision::INITIAL,
                BrokerPhase::Authorization { .. } => Revision::new(1),
                _ => return Err(BrokerJournalError),
            };
            let request = BrokerPhaseRequest {
                authority_generation: intent.lineage_fence.authority_generation,
                expected_revision,
                evaluation,
                phase,
            };
            let store = Arc::clone(&self.store);
            let envelope = self.envelope.clone();
            let caller = self.caller.clone();
            tokio::task::spawn_blocking(move || {
                store.commit_provider_service_launch_broker_phase(&envelope, &caller, request)
                    .map_err(|error| {
                        tracing::warn!(code = %error.error_code(), "Provider service evaluation was not persisted");
                        BrokerJournalError
                    })
                    .map(|_| ())
            }).await.map_err(|_| BrokerJournalError)?
        })
    }
}

// Le module de tests tourne sur toutes les plateformes : les contrats du writer y sont exercés
// par les ports du writer sur un plan synthétique. Seule sa partie `native` (évaluation réelle
// par le broker, lancement d'un hôte, écoute possédée) est réservée à macOS, seul système où
// le plan strict est qualifié (REQ-MODEL-015).
#[cfg(test)]
mod tests;
