//! One original claim, one broker and one native execution; never restart I/O.

use super::owner::TransportOwner;
use crate::broker_activity::BrokerActivityGuard;
use crate::ipc::DaemonRuntimeState;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BrokerConfig, EffectiveAuthority, EffectsClass, GuardMode,
    HostIsolationProbe, InitialEffectivePolicy, IsolationMode, IsolationRequirements,
    LocalAuthorityFallbackPolicy, NormalizedResourceRef, PlatformKeyringSecretStore, ScopeKind,
    StrictNoneAcceptedObligationChecker, SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{Fence, LineageFence, ModelPolicy, ReceiptId, ReconcileStrategy};
use unleash_model_router::{
    connect_transport::{self, FrozenConnectRequest},
    inventory_transport::{self, FrozenOllamaInventoryRequest},
    transport::{BrokerDispatch, ProfileRef},
};
use unleash_protocol::CommandEnvelope;
use unleash_store::CallerAuthority;

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
            TransportOwner::claim(&store, command, caller, receipt_id)
        })
        .await;
        let Ok(Ok(Some(claim))) = claimed else {
            return;
        };
        if run(&state, Arc::new(claim), activity).await.is_err() {
            tracing::warn!("Transport fournisseur non terminé ; consulter son état durable");
        }
    });
}

async fn run(
    state: &Arc<DaemonRuntimeState>,
    claim: Arc<TransportOwner>,
    activity: Arc<BrokerActivityGuard>,
) -> Result<(), ()> {
    let c = claim.context();
    let frozen = freeze_request(&claim)?;
    let store = Arc::clone(&state.store);
    let policy_claim = Arc::clone(&claim);
    let policy = tokio::task::spawn_blocking(move || policy_claim.policy(&store))
        .await
        .map_err(|_| failed("policy_reader_task"))?
        .map_err(|error| {tracing::warn!(stage="policy_reader",code=%error.error_code(),reason=?error.reason_code(),"Connect preparation refused");})?;
    let initial_policy = initial_policy(c, policy)?;
    let authority = claim_authority(&claim);
    let control_fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::ZERO);
    let intent = claim_intent(&claim, &authority, control_fence)?;
    let journal = Arc::new(super::admission::ConnectJournal::new(
        state,
        Arc::clone(&claim),
        Arc::clone(&activity),
    ));
    let outcome = async {
    let timer = Arc::new(SystemClockAndTimer::new());
    let broker = ActionBroker::new_with_policy_snapshot(
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
        initial_policy,
    ).map_err(|error| {tracing::warn!(stage="policy_initialization",code=%error.error_code(),reason=?error.reason_code(),"Connect preparation refused");})?;
    let decision = match broker
        .authorize_action(intent, &authority)
        .await
        .map_err(|error| {tracing::warn!(stage="authorization",code=%error.error_code(),reason=?error.reason_code(),"Connect authorization refused");})?
    {
        AuthorizationOutcome::Authorized(decision) => decision,
        AuthorizationOutcome::ApprovalRequired { .. } => {
            failed("human_approval_required");
            return Err(());
        },
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
    let dispatch = BrokerDispatch {
        broker: &broker,
        action: c.action_id,
        decision: &decision.decision_ref,
        fence: control_fence,
        authority: &authority,
        clock: state.store.config().clock.clone(),
        secrets: &secrets,
    };
    let receipt = match frozen {
        FrozenTransport::Connect(frozen) => connect_transport::execute(frozen, dispatch, observation)
            .await.map_err(|_| ())?.receipt,
        FrozenTransport::Inventory(frozen) => inventory_transport::execute_ollama_inventory_with_observation(
            frozen, dispatch, observation,
        ).await.map_err(|_| ())?.receipt,
    };
    receipt.map_err(|_| ())?;
        Ok(())
    }.await;
    if outcome.is_err() {
        journal.dispose_known_refusal(state).await;
    }
    outcome
}

enum FrozenTransport {
    Connect(FrozenConnectRequest),
    Inventory(FrozenOllamaInventoryRequest),
}

fn freeze_request(claim: &TransportOwner) -> Result<FrozenTransport, ()> {
    let context = claim.context();
    let profile = ProfileRef::new(
        &context.profile_id.to_string(),
        context.profile_version,
        context.configuration_digest,
    )
    .map_err(|_| failed("profile_reference"))?;
    let (frozen, digest) = match claim {
        TransportOwner::Connect(_) => {
            let request = FrozenConnectRequest::new_loopback(
                &context.destination,
                profile,
                context.limits.timeout_ms(),
            )
            .map_err(|_| failed("frozen_connect_request"))?;
            let digest = request
                .payload_digest()
                .map_err(|_| failed("frozen_digest"))?;
            (FrozenTransport::Connect(request), digest)
        }
        TransportOwner::Inventory(_) => {
            let limits = context
                .inventory_limits
                .as_ref()
                .ok_or_else(|| failed("inventory_bounds"))?;
            let request = FrozenOllamaInventoryRequest::new_loopback_with_limits(
                &format!("{}api/tags", context.destination),
                profile,
                limits.connect_timeout_ms,
                limits.timeout_ms,
                usize::try_from(limits.max_response_bytes)
                    .map_err(|_| failed("inventory_body_bound"))?,
            )
            .map_err(|_| failed("frozen_inventory_request"))?;
            let digest = request.payload_digest();
            (FrozenTransport::Inventory(request), digest)
        }
    };
    if digest != context.request_digest {
        failed("request_digest_binding");
        return Err(());
    }
    Ok(frozen)
}

fn claim_authority(claim: &TransportOwner) -> EffectiveAuthority {
    let context = claim.context();
    compute_effective_authority(
        context.project_id,
        context.policy_revision,
        context.scope_revision,
        &[AuthorityLayer {
            layer_name: match claim {
                TransportOwner::Connect(_) => "provider_probe_connect_mandate",
                TransportOwner::Inventory(_) => "provider_probe_inventory_mandate",
            }
            .into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([claim.verb().into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from([claim.destination().host().to_owned()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: context.locality,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: if context.project_id.is_some() {
                ScopeKind::Project
            } else {
                ScopeKind::Global
            },
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    )
}

fn claim_intent(
    claim: &TransportOwner,
    authority: &EffectiveAuthority,
    control_fence: LineageFence,
) -> Result<ActionIntent, ()> {
    let context = claim.context();
    let endpoint = claim.destination();
    Ok(ActionIntent {
        action_id: context.action_id,
        subject: ActionSubject::new_control_plane(
            context.principal_id.clone(),
            format!("receipt:{}", context.receipt_id),
        )
        .map_err(|_| failed("subject"))?,
        project_id: context.project_id,
        verb: claim.verb().into(),
        resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
            host: endpoint.host().into(),
            port: Some(endpoint.port()),
            is_private_or_loopback: true,
        }],
        payload_digest: context.request_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::NetworkEgress,
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: control_fence,
        action_deadline: context.deadline,
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    })
}

fn failed(stage: &'static str) {
    tracing::warn!(stage, "Connect preparation refused");
}

fn initial_policy(
    context: &unleash_store::ProviderProbeConnectContext,
    policy: unleash_store::ProviderConnectPolicySnapshot,
) -> Result<InitialEffectivePolicy, ()> {
    let control_fence = LineageFence::new(
        context.authority_generation,
        context.daemon_epoch,
        Fence::ZERO,
    );
    if policy.project_id != context.project_id
        || policy.policy_revision != context.policy_revision
        || policy.scope_revision != context.scope_revision
        || policy.policy_digest != context.policy_digest
        || policy.control_fence != control_fence
    {
        failed("policy_snapshot_binding");
        return Err(());
    }
    if policy.isolation_mode != unleash_store::PolicyIsolationMode::Standard {
        failed("policy_executor_profile");
        return Err(());
    }
    if let Some(guard) = policy.guard_config
        && (guard.guard_mode != unleash_store::GuardPolicyMode::BrokerOnly
            || guard.endpoint.is_some()
            || guard.transport != unleash_store::GuardTransport::None
            || guard.secret_ref_id.is_some()
            || guard.fallback_policy_ref.is_some()
            || guard.guard_mapping_profile != unleash_action_broker::NONE_ACCEPTED_PROFILE_ID)
    {
        failed("guard_profile_unqualified");
        return Err(());
    }
    // Absence is the canonical BrokerOnly default. NetworkEgress still requires
    // Evaluate whenever Guard is configured; no such port is qualified here.
    Ok(InitialEffectivePolicy {
        project_id: policy.project_id,
        policy_revision: policy.policy_revision,
        scope_revision: policy.scope_revision,
        policy_digest: policy.policy_digest,
        isolation_mode: IsolationMode::Standard,
        guard_mode: GuardMode::BrokerOnly,
        lineage_fence: policy.control_fence,
    })
}
