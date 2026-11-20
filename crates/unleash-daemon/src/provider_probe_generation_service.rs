//! First durable broker phase for the independently authorized generation probe.

use crate::auth::AuthenticatedSession;
use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};
use std::sync::Arc;
use unleash_action_broker::{
    ActionIntent, ActionProvenance, ActionSubject, AuthorizationEvaluation, BrokerJournalEntry,
    EffectsClass, IsolationRequirements, NormalizedResourceRef,
};
use unleash_domain::{AuthorizationEvaluationId, Fence, LineageFence, ReconcileStrategy, Revision};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    BrokerPhaseRequest, PolicyIsolationMode, PreparedProviderProbe, StoreError, UnleashStore,
};

fn unavailable() -> StoreError {
    StoreError::EvidenceUnavailable("intention native de génération indisponible".into())
}

fn authorizing(
    store: &UnleashStore,
    record: &PreparedProviderProbe,
    isolation: PolicyIsolationMode,
) -> Result<BrokerPhaseRequest, StoreError> {
    let context = &record.context;
    let intent = ActionIntent {
        action_id: context.action_id,
        subject: ActionSubject::new_control_plane(
            context.principal_id.clone(),
            format!("receipt:{}", context.command_receipt_id),
        )
        .map_err(|_| unavailable())?,
        project_id: context.project_id,
        verb: unleash_store::PROVIDER_PROBE_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::ModelTarget {
            provider: context.profile.provider_id.clone(),
            model_id: context.preparation.resolved_model_id.clone(),
            locality: context.profile.declared_locality,
        }],
        payload_digest: record.payload_digest(),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ModelDispatch,
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: LineageFence::new(
            context.authority_generation,
            context.daemon_epoch,
            Fence::ZERO,
        ),
        action_deadline: context.deadline,
        isolation_requirements: match isolation {
            PolicyIsolationMode::Standard => IsolationRequirements::standard_unconfined(),
            PolicyIsolationMode::Strict => IsolationRequirements::strict_default(),
        },
        secret_refs: context.preparation.secret_refs.clone(),
        provenance: ActionProvenance::EngineInternal,
    };
    let entry = BrokerJournalEntry::Authorizing {
        evaluation: AuthorizationEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_uuid(
                store.config().id_generator.new_uuid(),
            ),
            previous_evaluation_id: None,
        },
        intent,
    };
    let (evaluation, _, phase) =
        crate::broker_journal::project(&entry).map_err(|_| unavailable())?;
    Ok(BrokerPhaseRequest {
        authority_generation: context.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation,
        phase,
    })
}

pub(crate) fn start(
    state: &Arc<DaemonRuntimeState>,
    session: &AuthenticatedSession,
    command: &CommandEnvelope,
) {
    let Ok(caller) = crate::provider_profile_service::caller_for(session, command) else {
        return;
    };
    let Some(activity) = state.broker_activity.admit() else {
        return;
    };
    let state = Arc::clone(state);
    let activity = Arc::new(activity);
    let command = command.clone();
    tokio::spawn(async move {
        let store = Arc::clone(&state.store);
        let writer_activity = Arc::clone(&activity);
        let result = tokio::task::spawn_blocking(move || {
            let _activity = writer_activity;
            store.commit_provider_generation_authorizing(&command, &caller, |record, isolation| {
                authorizing(&store, record, isolation)
            })
        })
        .await;
        match result {
            Ok(Ok(Some(commit))) => {
                let cursor = commit.event_cursor;
                publish_committed_events_ordered(
                    &state,
                    cursor.stream_id,
                    cursor.sequence.as_u64(),
                    1,
                    &[],
                )
                .await;
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => tracing::warn!(
                code=%error.error_code(), reason=?error.reason_code(),
                "Préparation du broker de génération en attente",
            ),
            Err(_) => tracing::warn!("Issue du writer de génération indéterminée"),
        }
        drop(activity);
    });
}
