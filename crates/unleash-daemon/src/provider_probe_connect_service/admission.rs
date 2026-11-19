//! The original claim gates durable broker phases and the existing V008 scheduler.

use crate::broker_activity::BrokerActivityGuard;
use crate::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use unleash_action_broker::{
    BoxFuture, BrokerJournalEntry, BrokerJournalError, BrokerJournalPort, ObservationCause,
};
use unleash_domain::{ActionState, Revision};
use unleash_store::{
    AdmissionOutcome, AdmissionUnit, AdoptedProviderConnectObservation, BrokerPhaseRequest,
    CommitProviderConnectObservation, ExecutionContinuation, ProviderProbeConnectClaim, StoreError,
    UnleashStore,
};

pub(super) type FactSlot = Mutex<Option<AdoptedProviderConnectObservation>>;

pub(super) struct ConnectJournal {
    runtime: Weak<DaemonRuntimeState>,
    pub(super) claim: Arc<ProviderProbeConnectClaim>,
    pub(super) continuation: Arc<Mutex<Option<ExecutionContinuation>>>,
    pub(super) fact: Arc<FactSlot>,
    pub(super) activity: Arc<BrokerActivityGuard>,
}

impl ConnectJournal {
    pub(super) fn new(
        runtime: &Arc<DaemonRuntimeState>,
        claim: Arc<ProviderProbeConnectClaim>,
        activity: Arc<BrokerActivityGuard>,
    ) -> Self {
        Self {
            runtime: Arc::downgrade(runtime),
            claim,
            continuation: Arc::new(Mutex::new(None)),
            fact: Arc::new(Mutex::new(None)),
            activity,
        }
    }

    async fn await_admission(
        &self,
        state: &Arc<DaemonRuntimeState>,
    ) -> Result<(), BrokerJournalError> {
        let remaining = self
            .claim
            .context()
            .deadline
            .as_i64()
            .checked_sub(state.store.config().clock.now_ms().as_i64())
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or(BrokerJournalError)?;
        // Subscribe before reading durable inputs; a wakeup is never permission.
        let mut changes = state.subscriptions.observe_internal();
        tokio::time::timeout(Duration::from_millis(remaining), async {
            loop {
                if *state.drain_started_tx.borrow() {
                    return Err(BrokerJournalError);
                }
                let store = Arc::clone(&state.store);
                let claim = Arc::clone(&self.claim);
                let slot = Arc::clone(&self.continuation);
                let activity = Arc::clone(&self.activity);
                let result = tokio::task::spawn_blocking(move || {
                    let _activity = activity;
                    if let Some(current) = slot.lock().map_err(|_| unavailable())?.clone() {
                        store.validate_provider_probe_connect_continuation(&claim, current)?;
                        return Ok(None);
                    }
                    observe_eligibility(&store, &claim)?;
                    let pool = store.global_execution_pool_id()?;
                    let snapshot = store.execution_admission_snapshot(pool)?;
                    let mut inputs = ExecutionSchedulingInputs::default();
                    // Connect reserves no monetary amount or model call. Other
                    // producers must supply their own plans; they stay ineligible
                    // here unless their plan is known, rather than fabricated.
                    inputs
                        .budgets
                        .insert(AdmissionUnit::Probe(claim.context().probe_id), vec![]);
                    let (outcome, _) =
                        match schedule_execution(&store, pool, snapshot.token, inputs) {
                            Ok(result) => result,
                            // The unresolved-server bound is inside the same V008
                            // transaction. Wait for a durable change, never dispatch
                            // or synthesize another capacity reservation.
                            Err(StoreError::CapacityBlocked(_)) => return Ok(None),
                            Err(error) => return Err(error),
                        };
                    if let AdmissionOutcome::Admitted(commit) = &outcome {
                        if commit.unit == AdmissionUnit::Probe(claim.context().probe_id) {
                            let continuation = ExecutionContinuation {
                                reservation_id: commit.reservation_id,
                                unit: commit.unit,
                                unit_revision: commit.revision,
                                lease_id: commit.lease.lease_id,
                                authority_generation: commit.lease.authority_generation,
                                daemon_epoch: commit.lease.daemon_epoch,
                                fence: commit.lease.fence,
                                boot_identity: commit.lease.boot_identity.clone(),
                            };
                            store.validate_provider_probe_connect_continuation(
                                &claim,
                                continuation.clone(),
                            )?;
                            *slot.lock().map_err(|_| unavailable())? = Some(continuation);
                        }
                        return Ok(Some(commit.event_cursor));
                    }
                    Ok(None)
                })
                .await
                .map_err(|_| BrokerJournalError)?
                .map_err(|_| BrokerJournalError)?;
                if let Some(cursor) = result {
                    publish_committed_events_ordered(
                        state,
                        cursor.stream_id,
                        cursor.sequence.as_u64(),
                        1,
                        &[],
                    )
                    .await;
                }
                if self
                    .continuation
                    .lock()
                    .map_err(|_| BrokerJournalError)?
                    .is_some()
                {
                    return Ok(());
                }
                changes.changed().await.map_err(|()| BrokerJournalError)?;
            }
        })
        .await
        .map_err(|_| BrokerJournalError)?
    }
}

fn unavailable() -> StoreError {
    StoreError::EvidenceUnavailable("orchestrateur connect indisponible".into())
}

fn observe_eligibility(
    store: &UnleashStore,
    claim: &ProviderProbeConnectClaim,
) -> Result<(), StoreError> {
    let c = claim.context();
    let bytes = store.read_verified_object_bytes(c.configuration_digest)?;
    if unleash_domain::Sha256Digest::from_bytes(&bytes) != c.configuration_digest {
        return Err(unavailable());
    }
    let value = serde_json::json!({"schema_version":1,"producer":"daemon_provider_connect_preflight","probe_id":c.probe_id,
        "receipt_id":c.receipt_id,"action_id":c.action_id,"claim_id":claim.claim_id(),"context_digest":claim.context_digest(),
        "configuration_digest":c.configuration_digest,"mandate_digest":c.mandate_digest,"policy_digest":c.policy_digest,
        "transport":"loopback_only_tcp_tls_no_http_no_secret_no_model","authority_generation":c.authority_generation,"daemon_epoch":c.daemon_epoch});
    let hash = super::observation::publish_document(
        store,
        &value,
        "provider_probe_connect_eligibility",
        "eligibility",
    )?;
    let snapshot = store.execution_admission_snapshot(store.global_execution_pool_id()?)?;
    let candidate = snapshot
        .queued
        .iter()
        .find(|row| row.unit == AdmissionUnit::Probe(c.probe_id))
        .ok_or_else(unavailable)?;
    store.record_admission_eligibility(unleash_store::AdmissionEligibilityObservation {
        evidence_id: store.config().id_generator.new_uuid(),
        observer_principal_id: unleash_store::CONNECT_OBSERVER_PRINCIPAL.into(),
        object_hash: hash,
        unit: candidate.unit,
        unit_revision: candidate.revision,
        source_revision: snapshot.source_revision,
        authority_generation: snapshot.authority_generation,
        daemon_epoch: snapshot.daemon_epoch,
        valid_until: c.deadline,
        urgency: unleash_store::AdmissionUrgency::Ordinary,
    })
}

fn project(
    entry: &BrokerJournalEntry,
) -> Result<
    (
        unleash_store::BrokerEvaluation,
        unleash_store::BrokerIntent,
        unleash_store::BrokerPhase,
    ),
    BrokerJournalError,
> {
    if let BrokerJournalEntry::Observation {
        intent,
        receipt,
        cause: ObservationCause::Execution,
        ..
    } = entry
        && intent.verb == unleash_model_router::connect_transport::CONNECT_VERB
        && receipt.state == ActionState::OutcomeUnknown
        && receipt.output_digest.is_none()
        && receipt.remote_audit_ref.is_none()
    {
        let mut safe = entry.clone();
        if let BrokerJournalEntry::Observation { reason, .. } = &mut safe {
            *reason = None;
        }
        return crate::broker_journal::project(&safe);
    }
    crate::broker_journal::project(entry)
}

impl BrokerJournalPort for ConnectJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            let state = self.runtime.upgrade().ok_or(BrokerJournalError)?;
            let (evaluation, intent, phase) = project(entry)?;
            if intent.action_id != self.claim.context().action_id {
                return Err(BrokerJournalError);
            }
            let authorizing = matches!(&phase, unleash_store::BrokerPhase::Authorizing { .. });
            let store = Arc::clone(&state.store);
            let claim = Arc::clone(&self.claim);
            let continuation = Arc::clone(&self.continuation);
            let fact = Arc::clone(&self.fact);
            let activity = Arc::clone(&self.activity);
            let action = intent.action_id;
            let result = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                let expected_revision = store
                    .inspect_broker_action(action)?
                    .map_or(Revision::INITIAL, |s| s.revision);
                let observation = matches!(&phase, unleash_store::BrokerPhase::Observation { .. });
                let request = BrokerPhaseRequest {
                    authority_generation: intent.lineage_fence.authority_generation,
                    expected_revision,
                    evaluation,
                    phase,
                };
                if observation {
                    let continuation = continuation
                        .lock()
                        .map_err(|_| unavailable())?
                        .clone()
                        .ok_or_else(unavailable)?;
                    let fact = fact.lock().map_err(|_| unavailable())?.clone();
                    store.commit_provider_probe_connect_observation(
                        &claim,
                        CommitProviderConnectObservation {
                            continuation,
                            fact,
                            request,
                        },
                    )
                } else {
                    store.commit_provider_probe_connect_phase(&claim, request)
                }
            })
            .await;
            let cursor = match &result {
                Ok(Ok(commit)) => Some(commit.event_cursor),
                _ => {
                    let store = Arc::clone(&state.store);
                    tokio::task::spawn_blocking(move || store.inspect_broker_action(action))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .flatten()
                        .map(|s| s.last_event_cursor)
                }
            };
            if let Some(cursor) = cursor
                && cursor.stream_id == state.stream_id
            {
                publish_committed_events_ordered(
                    &state,
                    cursor.stream_id,
                    cursor.sequence.as_u64(),
                    1,
                    &[],
                )
                .await;
            }
            result
                .map_err(|_| BrokerJournalError)?
                .map_err(|_| BrokerJournalError)?;
            if authorizing {
                self.await_admission(&state).await?;
            }
            Ok(())
        })
    }
}
