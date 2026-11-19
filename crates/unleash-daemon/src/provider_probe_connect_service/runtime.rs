//! One original claim, one broker and one native execution; never restart I/O.

use crate::executors::BrokerActivityGuard;
use crate::ipc::DaemonRuntimeState;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, EffectsClass, HostIsolationProbe, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef,
    PlatformKeyringSecretStore, ScopeKind, StrictNoneAcceptedObligationChecker,
    SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{Fence, LineageFence, ModelPolicy, ReceiptId, ReconcileStrategy};
use unleash_model_router::{
    connect_transport::{self, FrozenConnectRequest},
    transport::{BrokerDispatch, ProfileRef},
};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    CallerAuthority, ClaimProviderProbeConnectRequest, ProviderProbeConnectClaim,
    ProviderProbeConnectClaimOutcome,
};

pub(super) fn start(
    state: &Arc<DaemonRuntimeState>,
    command: CommandEnvelope,
    caller: CallerAuthority,
    receipt_id: ReceiptId,
) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let activity = Arc::new(activity);
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let store = Arc::clone(&state.store);
        let worker = Arc::clone(&activity);
        let claimed = tokio::task::spawn_blocking(move || {
            let _activity = worker;
            let probe = unleash_domain::ProviderProbeId::from_uuid(command.aggregate_id);
            let record = store
                .inspect_provider_probe_connect_snapshot(probe)?
                .value
                .ok_or_else(|| unleash_store::StoreError::NotFound("probe connect".into()))?;
            let control_fence = LineageFence::new(
                record.context.authority_generation,
                record.context.daemon_epoch,
                Fence::ZERO,
            );
            store.claim_provider_probe_connect(ClaimProviderProbeConnectRequest {
                envelope: command,
                caller,
                receipt_id,
                context_digest: record.context_digest,
                control_fence,
            })
        })
        .await;
        let Ok(Ok(ProviderProbeConnectClaimOutcome::FreshClaim(claim))) = claimed else {
            return;
        };
        if run(&state, Arc::new(claim), activity).await.is_err() {
            tracing::warn!("Connexion fournisseur non terminée ; consulter son état durable");
        }
    });
}

async fn run(
    state: &Arc<DaemonRuntimeState>,
    claim: Arc<ProviderProbeConnectClaim>,
    activity: Arc<BrokerActivityGuard>,
) -> Result<(), ()> {
    let c = claim.context();
    let endpoint = claim.destination();
    let frozen = FrozenConnectRequest::new_loopback(
        &c.destination,
        ProfileRef::new(
            &c.profile_id.to_string(),
            c.profile_version,
            c.configuration_digest,
        )
        .map_err(|_| ())?,
        c.limits.timeout_ms(),
    )
    .map_err(|_| ())?;
    if frozen.payload_digest().map_err(|_| ())? != c.request_digest {
        return Err(());
    }
    let store = Arc::clone(&state.store);
    let project = c.project_id;
    let policy = tokio::task::spawn_blocking(move || store.read_policy_summary_snapshot(project))
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
        c.project_id,
        c.policy_revision,
        c.scope_revision,
        &[AuthorityLayer {
            layer_name: "provider_probe_connect_mandate".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([connect_transport::CONNECT_VERB.into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from([endpoint.host().to_owned()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: c.locality,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: if c.project_id.is_some() {
                ScopeKind::Project
            } else {
                ScopeKind::Global
            },
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );
    let control_fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::ZERO);
    let intent = ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: c.project_id,
        verb: connect_transport::CONNECT_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
            host: endpoint.host().into(),
            port: endpoint.port(),
            is_private_or_loopback: true,
        }],
        payload_digest: c.request_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::NetworkEgress,
        scope_revision: c.scope_revision,
        policy_revision: c.policy_revision,
        lineage_fence: control_fence,
        action_deadline: c.deadline,
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    };
    let journal = Arc::new(super::admission::ConnectJournal::new(
        state,
        Arc::clone(&claim),
        Arc::clone(&activity),
    ));
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
            current_fence: control_fence,
            evaluate_timeout: Duration::from_secs(2),
            health_check_interval: Duration::from_secs(30),
            cache_max_entries: 1,
            cache_ttl: Duration::ZERO,
            canaries_to_redact: vec![],
        },
        journal.clone(),
    );
    let decision = match broker
        .authorize_action(intent, &authority)
        .await
        .map_err(|_| ())?
    {
        AuthorizationOutcome::Authorized(decision) => decision,
        AuthorizationOutcome::ApprovalRequired { .. } => return Err(()),
    };
    let observation = Arc::new(super::observation::ObservationAdapter {
        store: Arc::clone(&state.store),
        claim: Arc::clone(&claim),
        continuation: Arc::clone(&journal.continuation),
        fact: Arc::clone(&journal.fact),
        activity,
    });
    let secrets = PlatformKeyringSecretStore::new(Arc::new(
        unleash_platform::UnavailableOsKeyring::new("Secrets interdits pour connect"),
    ));
    let result = connect_transport::execute(
        frozen,
        BrokerDispatch {
            broker: &broker,
            action: c.action_id,
            decision: &decision.decision_ref,
            fence: control_fence,
            authority: &authority,
            clock: state.store.config().clock.clone(),
            secrets: &secrets,
        },
        observation,
    )
    .await
    .map_err(|_| ())?;
    result.receipt.map_err(|_| ())?;
    Ok(())
}
