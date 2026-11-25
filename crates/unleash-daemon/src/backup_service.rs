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

/// Engage par le writer unique un blocage avant tout marqueur d'effet, puis publie son événement.
///
/// Rend vrai si le magasin a attesté le motif, ou l'avait déjà engagé ; faux s'il le refuse,
/// notamment dès qu'un marqueur de dispatch existe : l'issue exige alors une inspection.
async fn block_before_dispatch(
    state: &Arc<DaemonRuntimeState>,
    record: &BackupRequestRecord,
    activity: &Arc<BrokerActivityGuard>,
    reason: BackupBlockReason,
) -> bool {
    let store = Arc::clone(&state.store);
    let backup_id = record.backup_id;
    let generation = record.authority_generation;
    let task_activity = Arc::clone(activity);
    let outcome = tokio::task::spawn_blocking(move || {
        let _activity = task_activity;
        store.block_claimed_backup(backup_id, generation, reason)
    })
    .await;
    let committed = matches!(outcome, Ok(Ok(_)));
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
    committed
}

/// Refus du courtier avant tout marqueur d'effet.
///
/// Le harnais ne connaît pas toujours la cause du refus : le magasin n'engage que le motif qu'il
/// atteste lui-même, du plus précis au plus général. `preflight_failed` n'est attesté que par
/// l'absence de tout marqueur : une demande réclamée sans dispatch ne reste jamais sans issue.
async fn block_after_broker_refusal(
    state: &Arc<DaemonRuntimeState>,
    record: &BackupRequestRecord,
    activity: &Arc<BrokerActivityGuard>,
) {
    for reason in [
        BackupBlockReason::AuthorizationDenied,
        BackupBlockReason::PolicyChanged,
        BackupBlockReason::PreflightFailed,
    ] {
        if block_before_dispatch(state, record, activity, reason).await {
            return;
        }
    }
}

/// Point de contrôle du harnais, atteint hors du thread writer ; inerte avec l'observateur de production.
async fn harness_checkpoint(
    state: &Arc<DaemonRuntimeState>,
    activity: &Arc<BrokerActivityGuard>,
    checkpoint: StoreCutpoint,
) -> bool {
    let observer = Arc::clone(&state.store.config().cutpoints);
    let task_activity = Arc::clone(activity);
    tokio::task::spawn_blocking(move || {
        let _activity = task_activity;
        observer.hit(checkpoint).is_ok()
    })
    .await
    .unwrap_or(false)
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
            tracing::warn!(
                backup_id = %backup_id,
                "Sauvegarde non terminée ; consulter son état durable"
            );
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
        || !matches!(
            canonical_sha256(&record.action_payload()),
            Ok(digest) if digest == record.payload_digest
        )
    {
        // Incohérence interne (invariant défensif) : aucun effet n'est possible. Le magasin
        // n'engage le blocage que sous l'autorité qui est celle de la demande.
        block_before_dispatch(
            &state,
            &record,
            &activity,
            BackupBlockReason::PreflightFailed,
        )
        .await;
        return Err(());
    }
    let store = Arc::clone(&state.store);
    let read_activity = Arc::clone(&activity);
    // Résumé de politique illisible ou harnais interrompu : la revalidation n'a pas eu lieu.
    let policy = match tokio::task::spawn_blocking(move || {
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
    {
        Ok(Ok(read)) => Ok(read.value),
        Ok(Err(_)) | Err(_) => Err(()),
    };
    let policy = match policy {
        Ok(policy) => policy,
        Err(()) => {
            // Le blocage n'est engagé que si le magasin peut encore, dans sa propre transaction,
            // lire l'état et prouver l'absence de tout marqueur d'effet.
            block_before_dispatch(
                &state,
                &record,
                &activity,
                BackupBlockReason::PreflightFailed,
            )
            .await;
            return Err(());
        }
    };
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
    if !harness_checkpoint(
        &state,
        &activity,
        StoreCutpoint::C11iAfterBackupRevalidation,
    )
    .await
    {
        block_before_dispatch(
            &state,
            &record,
            &activity,
            BackupBlockReason::PreflightFailed,
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
    // Contrôles internes du harnais : aucun n'engage le moindre marqueur d'effet.
    let preflight = match (
        ActionSubject::new_control_plane(
            record.principal_id.clone(),
            format!("receipt:{}", record.receipt_id),
        ),
        engaged_policy_digest(&policy),
    ) {
        (Ok(subject), Ok(policy_digest)) => Ok((subject, policy_digest)),
        (Err(_), _) | (_, Err(_)) => Err(()),
    };
    let (subject, policy_digest) = match preflight {
        Ok(preflight) => preflight,
        Err(()) => {
            block_before_dispatch(
                &state,
                &record,
                &activity,
                BackupBlockReason::PreflightFailed,
            )
            .await;
            return Err(());
        }
    };
    let intent = ActionIntent {
        action_id: record.action_id,
        subject,
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
            policy_digest,
            isolation_mode: IsolationMode::Standard,
            guard_mode: GuardMode::BrokerOnly,
            lineage_fence: record.lineage_fence,
        },
    );
    let broker = match broker {
        Ok(broker) => broker,
        Err(_) => {
            block_before_dispatch(
                &state,
                &record,
                &activity,
                BackupBlockReason::PreflightFailed,
            )
            .await;
            return Err(());
        }
    };
    let decision = match broker.authorize_action(intent, &authority).await {
        Ok(AuthorizationOutcome::Authorized(decision)) => decision,
        Ok(AuthorizationOutcome::ApprovalRequired { .. }) => {
            // Le producteur interne ne prévoit aucune approbation : l'action reste non dispatchée.
            block_before_dispatch(
                &state,
                &record,
                &activity,
                BackupBlockReason::PreflightFailed,
            )
            .await;
            return Err(());
        }
        Err(_) => {
            // Le writer refuse un blocage sans motif attesté : le magasin n'engage que
            // celui qu'il peut prouver.
            block_after_broker_refusal(&state, &record, &activity).await;
            return Err(());
        }
    };
    if !harness_checkpoint(
        &state,
        &activity,
        StoreCutpoint::C11jAfterBackupAuthorization,
    )
    .await
    {
        block_after_broker_refusal(&state, &record, &activity).await;
        return Err(());
    }
    let executor = BackupExecutor {
        store: Arc::clone(&state.store),
        record: record.clone(),
        activity,
    };
    let secrets = PlatformKeyringSecretStore::new(Arc::new(UnavailableOsKeyring::new(
        "Références secrètes interdites pour sauvegarde",
    )));
    if broker
        .execute_authorized(
            record.action_id,
            &decision.decision_ref,
            record.lineage_fence,
            &authority,
            &executor,
            &secrets,
        )
        .await
        .is_err()
    {
        // Sans marqueur de dispatch, la demande ne reste jamais `claimed` : blocage attesté.
        // Marqueur engagé : le magasin refuse le blocage, l'issue inconnue reste à réconcilier.
        block_after_broker_refusal(&state, &executor.record, &executor.activity).await;
        return Err(());
    }
    Ok(())
}
