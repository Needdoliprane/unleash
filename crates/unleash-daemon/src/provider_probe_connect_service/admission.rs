//! Pending Authorizing -> V008 admission -> Guard journal assembly.
//! The only reservation authority is the existing Store writer and scheduler.
//! Eligibility production, active-claim reads and the SQL dispatch hook are
//! required integration dependencies; no permissive implementation exists here.

use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::broker_journal::SqliteBrokerJournal;
use crate::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use crate::executors::BrokerActivityGuard;
use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};
use unleash_action_broker::{BoxFuture, BrokerJournalEntry, BrokerJournalError, BrokerJournalPort};
use unleash_domain::{
    ActionId, ActorId, LineageFence, PoolId, ProviderProbeId, ReceiptId, Revision, Sha256Digest,
    TimestampMs,
};
use unleash_protocol::EventCursor;
use unleash_store::{
    AdmissionCommit, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionSnapshotToken,
    AdmissionUnit, ExecutionAdmissionSnapshot, ExecutionContinuation, StoreError, UnleashStore,
};

/// Construct only from a freshly acknowledged Store claim, never a client payload
/// or inspection of a claim whose owner/acknowledgement is unknown.
#[derive(Clone)]
pub(super) struct AdmissionBinding {
    pub(super) probe_id: ProviderProbeId,
    pub(super) receipt_id: ReceiptId,
    pub(super) principal_id: ActorId,
    pub(super) action_id: ActionId,
    pub(super) claim_revision: Revision,
    pub(super) context_digest: Sha256Digest,
    pub(super) request_digest: Sha256Digest,
    pub(super) control_fence: LineageFence,
    pub(super) pool_id: PoolId,
    pub(super) deadline: TimestampMs,
}

/// Trusted, real eligibility producer and Store adapter; no client implements it.
pub(super) trait AdmissionPreparation: Send + Sync {
    /// Read one coherent live claim/continuation, rechecking current identity,
    /// mandate, epoch and lease. A stale or uncertain owner is an error.
    fn current_continuation(
        &self,
        binding: &AdmissionBinding,
    ) -> Result<Option<ExecutionContinuation>, StoreError>;

    /// Publish actual local preflight observations under an active service/system
    /// principal. A queued row alone is not evidence of eligibility.
    fn observe_eligibility(
        &self,
        binding: &AdmissionBinding,
    ) -> Result<AdmissionEligibilityObservation, StoreError>;

    /// Reuse the shared scheduling input plans for the current snapshot. Missing
    /// plans stay errors; another selected unit is never replaced with this probe.
    fn scheduling_inputs(
        &self,
        binding: &AdmissionBinding,
        snapshot: &ExecutionAdmissionSnapshot,
    ) -> Result<ExecutionSchedulingInputs, StoreError>;

    /// Wait on the existing durable input/event mechanism, including expiry.
    /// Register/recheck without losing a change; do not poll or return merely
    /// because this producer recorded its own observation. This is a wake-up,
    /// never a reservation or permission. The journal retains its exact entry.
    fn wait_for_change<'a>(
        &'a self,
        binding: &'a AdmissionBinding,
        token: AdmissionSnapshotToken,
    ) -> BoxFuture<'a, Result<(), StoreError>>;
}

enum AdmissionGateResult {
    Ready {
        cursor: Option<EventCursor>,
    },
    Waiting {
        cursor: Option<EventCursor>,
        token: AdmissionSnapshotToken,
    },
}

fn check_continuation(
    binding: &AdmissionBinding,
    continuation: &ExecutionContinuation,
) -> Result<(), StoreError> {
    if continuation.unit != AdmissionUnit::Probe(binding.probe_id)
        || continuation.authority_generation != binding.control_fence.authority_generation
        || continuation.daemon_epoch != binding.control_fence.epoch
        || continuation.fence.as_u64() == 0
    {
        return Err(StoreError::InvariantViolation(
            "continuation connect incohérente".to_owned(),
        ));
    }
    Ok(())
}

fn continuation(commit: &AdmissionCommit) -> ExecutionContinuation {
    ExecutionContinuation {
        reservation_id: commit.reservation_id,
        unit: commit.unit,
        unit_revision: commit.revision,
        lease_id: commit.lease.lease_id,
        authority_generation: commit.lease.authority_generation,
        daemon_epoch: commit.lease.daemon_epoch,
        fence: commit.lease.fence,
        boot_identity: commit.lease.boot_identity.clone(),
    }
}

fn gate_blocking(
    store: &UnleashStore,
    preparation: &dyn AdmissionPreparation,
    binding: &AdmissionBinding,
) -> Result<AdmissionGateResult, StoreError> {
    if let Some(existing) = preparation.current_continuation(binding)? {
        check_continuation(binding, &existing)?;
        return Ok(AdmissionGateResult::Ready { cursor: None });
    }
    let observed = preparation.observe_eligibility(binding)?;
    if observed.unit != AdmissionUnit::Probe(binding.probe_id)
        || observed.authority_generation != binding.control_fence.authority_generation
        || observed.daemon_epoch != binding.control_fence.epoch
    {
        return Err(StoreError::InvariantViolation(
            "éligibilité connect incohérente".to_owned(),
        ));
    }
    store.record_admission_eligibility(observed)?;
    let snapshot = store.execution_admission_snapshot(binding.pool_id)?;
    let inputs = preparation.scheduling_inputs(binding, &snapshot)?;
    let (outcome, _decision) = schedule_execution(store, binding.pool_id, snapshot.token, inputs)?;
    match outcome {
        AdmissionOutcome::Unchanged => Ok(AdmissionGateResult::Waiting {
            cursor: None,
            token: snapshot.token,
        }),
        AdmissionOutcome::Admitted(commit) => {
            let cursor = Some(commit.event_cursor);
            if commit.unit == AdmissionUnit::Probe(binding.probe_id) {
                let committed = continuation(&commit);
                check_continuation(binding, &committed)?;
                // The future network-context validator must also bind the claim
                // revision/owner and active mandate inside the V008 transaction.
                // This re-read additionally refuses a change already observed
                // after commit, before an Authorizing ACK could start Guard.
                let current = preparation.current_continuation(binding)?.ok_or_else(|| {
                    StoreError::InvariantViolation(
                        "claim connect périmé après admission".to_owned(),
                    )
                })?;
                if current != committed {
                    return Err(StoreError::InvariantViolation(
                        "continuation connect modifiée après admission".to_owned(),
                    ));
                }
                Ok(AdmissionGateResult::Ready { cursor })
            } else {
                // Its real admission remains committed and its existing outbox
                // informs the corresponding owner. This gate runs no other unit.
                Ok(AdmissionGateResult::Waiting {
                    cursor,
                    token: snapshot.token,
                })
            }
        }
    }
}

pub(super) struct ConnectAdmissionJournal {
    inner: SqliteBrokerJournal,
    runtime: Weak<DaemonRuntimeState>,
    preparation: Arc<dyn AdmissionPreparation>,
    binding: AdmissionBinding,
}

impl ConnectAdmissionJournal {
    pub(super) fn new(
        runtime: &Arc<DaemonRuntimeState>,
        inner: SqliteBrokerJournal,
        preparation: Arc<dyn AdmissionPreparation>,
        binding: AdmissionBinding,
    ) -> Self {
        // The caller must supply the qualified connect projection, including its
        // local-proof success guard and closed Unknown mapping. The generic
        // journal currently lacks those hooks and is not a product fallback.
        Self {
            inner,
            runtime: Arc::downgrade(runtime),
            preparation,
            binding,
        }
    }

    async fn await_admission(
        &self,
        state: &Arc<DaemonRuntimeState>,
        activity: Arc<BrokerActivityGuard>,
    ) -> Result<(), BrokerJournalError> {
        let remaining = self
            .binding
            .deadline
            .as_i64()
            .checked_sub(state.store.config().clock.now_ms().as_i64())
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or(BrokerJournalError)?;
        tokio::time::timeout(Duration::from_millis(remaining), async {
            loop {
                let store = Arc::clone(&state.store);
                let preparation = Arc::clone(&self.preparation);
                let binding = self.binding.clone();
                let worker_activity = Arc::clone(&activity);
                let outcome = tokio::task::spawn_blocking(move || {
                    let _activity = worker_activity;
                    gate_blocking(&store, preparation.as_ref(), &binding)
                })
                .await
                .map_err(|_| BrokerJournalError)?
                .map_err(|_| BrokerJournalError)?;
                let cursor = match &outcome {
                    AdmissionGateResult::Ready { cursor }
                    | AdmissionGateResult::Waiting { cursor, .. } => *cursor,
                };
                if let Some(cursor) = cursor {
                    publish_committed_events_ordered(
                        state,
                        cursor.stream_id,
                        cursor.sequence.as_u64(),
                        1,
                        &[],
                    )
                    .await;
                }
                match outcome {
                    AdmissionGateResult::Ready { .. } => {
                        if state.store.config().clock.now_ms() >= self.binding.deadline {
                            return Err(BrokerJournalError);
                        }
                        return Ok(());
                    }
                    AdmissionGateResult::Waiting { token, .. } => {
                        self.preparation
                            .wait_for_change(&self.binding, token)
                            .await
                            .map_err(|_| BrokerJournalError)?;
                    }
                }
            }
        })
        .await
        .map_err(|_| BrokerJournalError)?
    }
}

impl BrokerJournalPort for ConnectAdmissionJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            let state = self.runtime.upgrade().ok_or(BrokerJournalError)?;
            let activity = Arc::new(state.broker_activity.admit().ok_or(BrokerJournalError)?);
            if let BrokerJournalEntry::Authorizing { intent, .. } = entry {
                if intent.action_id != self.binding.action_id
                    || intent.subject.principal_id() != &self.binding.principal_id
                    || intent.subject.cause_ref() != format!("receipt:{}", self.binding.receipt_id)
                    || intent.payload_digest != self.binding.request_digest
                    || intent.lineage_fence != self.binding.control_fence
                    || !intent.secret_refs.is_empty()
                {
                    return Err(BrokerJournalError);
                }
                // The action becomes durable before V008 can inspect it. The
                // Authorizing ACK is held until admission; Guard cannot run yet.
                self.inner.append(entry).await?;
                self.await_admission(&state, activity).await
            } else {
                // The Store dispatch transaction must still revalidate mandate,
                // exact continuation and TTL atomically with its C4 marker.
                self.inner.append(entry).await
            }
        })
    }
}
