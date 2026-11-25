//! Prepare the broker intent and observe placement for a managed generation probe.

use crate::auth::AuthenticatedSession;
use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};
use crate::provider_service_evaluation::{ManagedProviderServiceHosts, ProviderServiceLaunchError};
use std::sync::Arc;
use unleash_action_broker::{
    ActionIntent, ActionProvenance, ActionSubject, AuthorizationEvaluation, BrokerJournalEntry,
    EffectsClass, IsolationRequirements,
};
use unleash_domain::{AuthorizationEvaluationId, Fence, LineageFence, ReconcileStrategy, Revision};
use unleash_protocol::CommandEnvelope;
use unleash_store::{
    BrokerPhaseRequest, CallerAuthority, PolicyIsolationMode, PreparedProviderProbe, StoreError,
    UnleashStore,
};

fn unavailable() -> StoreError {
    StoreError::EvidenceUnavailable("intention native de génération indisponible".into())
}

pub(crate) fn authorizing(
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
        resource_refs: serde_json::from_value(record.resource_refs()).map_err(|_| unavailable())?,
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

pub(crate) fn adopt_placement_if_ready(
    store: &UnleashStore,
    services: &ManagedProviderServiceHosts,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<uuid::Uuid>, ProviderServiceLaunchError> {
    let Some(probe) = store.read_waiting_managed_generation(command, caller)? else {
        return Ok(None);
    };
    let Some(listener) = services.observe_probe_listener(store, &probe)? else {
        return Ok(None);
    };
    Ok(Some(store.stage_managed_provider_generation_placement(
        command, caller, listener,
    )?))
}

/// Issue de la recherche de placement d'un probe de génération, pour le journal (REQ-MODEL-015).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PlacementReport {
    /// Placement adopté et mis en scène pour l'ordonnanceur.
    Adopted(uuid::Uuid),
    /// Aucun service managé n'offre encore d'écoute confirmée.
    AwaitingEvidence,
    /// Le writer a refusé la lecture ou l'adoption.
    Refused { code: String },
    /// La tâche du writer n'a pas rendu de réponse exploitable.
    Indeterminate,
}

/// Classe l'issue de la recherche de placement ; le refus garde le code du writer.
pub(crate) fn placement_report(
    result: &Result<Result<Option<uuid::Uuid>, ProviderServiceLaunchError>, tokio::task::JoinError>,
) -> PlacementReport {
    match result {
        Ok(Ok(Some(evidence))) => PlacementReport::Adopted(*evidence),
        Ok(Ok(None)) => PlacementReport::AwaitingEvidence,
        Ok(Err(error)) => PlacementReport::Refused {
            code: error.diagnostic_code().into_owned(),
        },
        Err(_) => PlacementReport::Indeterminate,
    }
}

fn log_placement(report: &PlacementReport, probe: uuid::Uuid) {
    match report {
        PlacementReport::Adopted(evidence) => tracing::info!(
            probe_id = %probe,
            evidence_id = %evidence,
            "Placement du probe de génération adopté"
        ),
        PlacementReport::AwaitingEvidence => tracing::debug!(
            probe_id = %probe,
            code = "placement_evidence_pending",
            "Placement du probe de génération en attente d'une écoute managée"
        ),
        PlacementReport::Refused { code } => tracing::warn!(
            probe_id = %probe,
            code = %code,
            "Placement du probe de génération refusé par le writer"
        ),
        PlacementReport::Indeterminate => tracing::warn!(
            probe_id = %probe,
            code = "placement_writer_task",
            "Issue du writer de placement indéterminée"
        ),
    }
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
        let preparation_command = command.clone();
        let preparation_caller = caller.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _activity = writer_activity;
            store.commit_provider_generation_authorizing(
                &preparation_command,
                &preparation_caller,
                |record, isolation| authorizing(&store, record, isolation),
            )
        })
        .await;
        match result {
            Ok(Ok(commit)) => {
                if let Some(commit) = commit {
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
                if !*state.drain_started_tx.borrow() {
                    let writer_activity = Arc::clone(&activity);
                    let placement = tokio::task::spawn_blocking(move || {
                        let _activity = writer_activity;
                        adopt_placement_if_ready(
                            &state.store,
                            &state.provider_service_hosts,
                            &command,
                            &caller,
                        )
                    })
                    .await;
                    // Seule l'absence de preuve est une attente silencieuse ; un refus ou une
                    // tâche sans réponse est un avertissement, avec son code.
                    log_placement(&placement_report(&placement), command.aggregate_id);
                }
            }
            Ok(Err(error)) => tracing::warn!(
                code=%error.error_code(), reason=?error.reason_code(),
                "Préparation du broker de génération en attente",
            ),
            Err(_) => tracing::warn!(
                code = "generation_writer_task",
                "Issue du writer de génération indéterminée"
            ),
        }
        drop(activity);
    });
}
