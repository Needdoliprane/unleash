//! Internal `RequestBackup` producer; no client supplies an action or destination.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use unleash_action_broker::{
    ActionBroker, ActionExecutionFailure, ActionExecutionResult, ActionExecutorPort, ActionIntent,
    ActionProvenance, ActionSubject, AuthorityLayer, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture, BrokerConfig, EffectsClass, GuardMode, HostIsolationProbe, InitialEffectivePolicy,
    IsolationMode, IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef,
    PlatformKeyringSecretStore, ResolvedSecret, ScopeKind, StrictNoneAcceptedObligationChecker,
    SystemClockAndTimer, compute_effective_authority,
};
use unleash_domain::{
    BackupId, DataLocality, DispatchId, ModelPolicy, ReconcileStrategy, TimestampMs,
};
use unleash_platform::UnavailableOsKeyring;
use unleash_protocol::canonical_sha256;
use unleash_store::{
    BackupBlockReason, BackupRequestRecord, ExecuteReservedBackupRequest, GuardPolicyMode,
    PolicyIsolationMode, REQUEST_BACKUP_VERB, StoreCutpoint,
    policies_secrets::engaged_policy_digest,
};

use crate::broker_activity::BrokerActivityGuard;
use crate::broker_journal::SqliteBrokerJournal;
use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};

async fn block_before_dispatch(
    state: &Arc<DaemonRuntimeState>,
    record: &BackupRequestRecord,
    activity: &Arc<BrokerActivityGuard>,
    reason: BackupBlockReason,
) {
    let store = Arc::clone(&state.store);
    let backup_id = record.backup_id;
    let generation = record.authority_generation;
    let task_activity = Arc::clone(activity);
    let outcome = tokio::task::spawn_blocking(move || {
        let _activity = task_activity;
        store.block_claimed_backup(backup_id, generation, reason)
    })
    .await;
    let cursor = if let Ok(Ok(record)) = outcome {
        Some(record.last_event_cursor)
    } else {
        let store = Arc::clone(&state.store);
        let task_activity = Arc::clone(activity);
        tokio::task::spawn_blocking(move || {
            let _activity = task_activity;
            store
                .inspect_backup_request_snapshot(backup_id)
                .ok()?
                .value
                .map(|r| r.last_event_cursor)
        })
        .await
        .ok()
        .flatten()
    };
    if let Some(cursor) = cursor
        && cursor.stream_id == state.stream_id
    {
        publish_committed_events_ordered(state, cursor.stream_id, cursor.sequence.as_u64(), 1, &[])
            .await;
    }
}

/// Admit a detached orchestration only after the command receipt exists.
pub(crate) fn start(state: &Arc<DaemonRuntimeState>, backup_id: BackupId) {
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let activity = Arc::new(activity);
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let store = Arc::clone(&state.store);
        let authority = state.authority_generation;
        let claim_activity = Arc::clone(&activity);
        let claimed = tokio::task::spawn_blocking(move || {
            let _activity = claim_activity;
            store.claim_backup_request(backup_id, authority)
        })
        .await;
        let Ok(Ok(record)) = claimed else {
            // A concurrent or uncertain claim is inspected, never reissued here.
            return;
        };
        if run(state, record, activity).await.is_err() {
            tracing::warn!("Sauvegarde non terminée ; consulter son état durable");
        }
    });
}

struct BackupExecutor {
    store: Arc<unleash_store::UnleashStore>,
    record: BackupRequestRecord,
    activity: Arc<BrokerActivityGuard>,
}

impl ActionExecutorPort for BackupExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        decision: &'a AuthorizationDecision,
        secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(async move {
            let unknown = || ActionExecutionFailure::OutcomeUnknown {
                dispatch_id,
                reason: "Issue de sauvegarde à réconcilier".into(),
            };
            if !secrets.is_empty()
                || intent.action_id != self.record.action_id
                || intent.payload_digest != self.record.payload_digest
                || decision.action_id != self.record.action_id
                || intent.verb != REQUEST_BACKUP_VERB
            {
                return Err(unknown());
            }
            let request = ExecuteReservedBackupRequest {
                backup_id: self.record.backup_id,
                action_id: self.record.action_id,
                dispatch_id,
                authority_generation: self.record.authority_generation,
            };
            let store = Arc::clone(&self.store);
            let activity = Arc::clone(&self.activity);
            let result = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                let result = store.execute_reserved_backup(request)?;
                let observed_at_ms = store.config().clock.now_ms();
                Ok::<_, unleash_store::StoreError>((result, observed_at_ms))
            })
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?;
            Ok(ActionExecutionResult {
                dispatch_id,
                provider_request_id: None,
                output_digest: result.0.output_digest,
                observed_at_ms: result.1,
            })
        })
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Construction complète de l'autorité, de l'intention et du broker réservé"
)]
async fn run(
    state: Arc<DaemonRuntimeState>,
    record: BackupRequestRecord,
    activity: Arc<BrokerActivityGuard>,
) -> Result<(), ()> {
    if record.authority_generation != state.authority_generation
        || record.lineage_fence.epoch != state.daemon_epoch
        || record.destination_dir
            != state
                .store
                .config()
                .backups_dir
                .join(record.backup_id.to_string())
        || canonical_sha256(&record.action_payload()).map_err(|_| ())? != record.payload_digest
    {
        return Err(());
    }
    let store = Arc::clone(&state.store);
    let read_activity = Arc::clone(&activity);
    let policy = tokio::task::spawn_blocking(move || {
        let _activity = read_activity;
        // Point de contrôle de la revalidation (REQ-SEC-007) : atteint hors du thread writer,
        // inerte avec l'observateur de production.
        store
            .config()
            .cutpoints
            .hit(StoreCutpoint::C11hAfterBackupClaim)?;
        store.read_policy_summary_snapshot(None)
    })
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?
    .value;
    if policy.effective_isolation_mode != PolicyIsolationMode::Standard {
        block_before_dispatch(
            &state,
            &record,
            &activity,
            BackupBlockReason::IsolationProfileUnavailable,
        )
        .await;
        return Err(());
    }
    if policy
        .guard_config
        .as_ref()
        .is_some_and(|guard| guard.guard_mode != GuardPolicyMode::BrokerOnly)
    {
        block_before_dispatch(
            &state,
            &record,
            &activity,
            BackupBlockReason::GuardProfileUnavailable,
        )
        .await;
        return Err(());
    }
    if policy
        .global_policy
        .as_ref()
        .map_or(unleash_domain::Revision::INITIAL, |policy| policy.revision)
        != record.policy_revision
    {
        // REQ-SEC-007 : la demande n'est pas exécutée sous une politique qu'elle n'a pas engagée.
        block_before_dispatch(
            &state,
            &record,
            &activity,
            BackupBlockReason::PolicyChanged,
        )
        .await;
        return Err(());
    }
    let authority = compute_effective_authority(
        None,
        record.policy_revision,
        record.scope_revision,
        &[AuthorityLayer {
            layer_name: "managed_backup".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::LocalWrite])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([REQUEST_BACKUP_VERB.into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![state.store.config().backups_dir.clone()]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Global,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );
    let intent = ActionIntent {
        action_id: record.action_id,
        subject: ActionSubject::new_control_plane(
            record.principal_id.clone(),
            format!("receipt:{}", record.receipt_id),
        )
        .map_err(|_| ())?,
        project_id: None,
        verb: REQUEST_BACKUP_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Path(record.destination_dir.clone())],
        payload_digest: record.payload_digest,
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::LocalWrite,
        scope_revision: record.scope_revision,
        policy_revision: record.policy_revision,
        lineage_fence: record.lineage_fence,
        action_deadline: TimestampMs::new(
            state
                .store
                .config()
                .clock
                .now_ms()
                .as_i64()
                .saturating_add(300_000),
        ),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    let timer = Arc::new(SystemClockAndTimer::new());
    // Le courtier réservé est lié à la politique engagée par la demande (REQ-SEC-007) :
    // à la révision initiale, toute politique posée vaudrait `PolicyDenied`.
    let broker = ActionBroker::new_with_policy_snapshot(
        state.store.config().clock.clone(),
        timer.clone(),
        timer,
        state.store.config().id_generator.clone(),
        Arc::new(HostIsolationProbe),
        Arc::new(StrictNoneAcceptedObligationChecker),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig {
            current_fence: record.lineage_fence,
            evaluate_timeout: Duration::from_millis(2_000),
            health_check_interval: Duration::from_secs(30),
            cache_max_entries: 1,
            cache_ttl: Duration::ZERO,
            canaries_to_redact: Vec::new(),
        },
        Arc::new(SqliteBrokerJournal::new(&state)),
        InitialEffectivePolicy {
            project_id: None,
            policy_revision: record.policy_revision,
            scope_revision: record.scope_revision,
            policy_digest: engaged_policy_digest(&policy).map_err(|_| ())?,
            isolation_mode: IsolationMode::Standard,
            guard_mode: GuardMode::BrokerOnly,
            lineage_fence: record.lineage_fence,
        },
    )
    .map_err(|_| ())?;
    let decision = match broker.authorize_action(intent, &authority).await {
        Ok(AuthorizationOutcome::Authorized(decision)) => decision,
        Ok(AuthorizationOutcome::ApprovalRequired { .. }) => return Err(()),
        Err(_) => {
            // Le writer refuse ce blocage si aucun refus terminal n'est engagé,
            // notamment si l'issue de l'append est seulement incertaine.
            block_before_dispatch(
                &state,
                &record,
                &activity,
                BackupBlockReason::AuthorizationDenied,
            )
            .await;
            return Err(());
        }
    };
    let executor = BackupExecutor {
        store: Arc::clone(&state.store),
        record: record.clone(),
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(UnavailableOsKeyring::new(
        "Références secrètes interdites pour sauvegarde",
    )));
    broker
        .execute_authorized(
            record.action_id,
            &decision.decision_ref,
            record.lineage_fence,
            &authority,
            &executor,
            &secrets,
        )
        .await
        .map_err(|_| ())?;
    Ok(())
}
