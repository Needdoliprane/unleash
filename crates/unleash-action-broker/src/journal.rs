//! Durable boundary implemented by the daemon/store adapter; this crate never owns `SQLite`.

use unleash_domain::{DispatchId, LineageFence};

use crate::{
    ActionBrokerError, ActionIntent, ActionReceipt, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture,
};

/// One of the four journal commits required before acknowledging a broker phase.
#[derive(Debug, Clone)]
pub enum BrokerJournalEntry {
    /// Persist the exact intention before authorization starts.
    Authorizing {
        /// Proposed action and lineage.
        intent: ActionIntent,
    },
    /// Persist a decision, refusal or temporary unavailability; Allow never means success.
    Authorization {
        /// Exact intention.
        intent: ActionIntent,
        /// Actual authorization result, including its retry semantics.
        outcome: Result<AuthorizationOutcome, ActionBrokerError>,
    },
    /// Commit the dispatch marker before secrets or external I/O.
    Dispatch {
        /// Exact intention.
        intent: ActionIntent,
        /// Revalidated decision.
        decision: AuthorizationDecision,
        /// Broker-assigned dispatch identity passed to the executor.
        dispatch_id: DispatchId,
        /// Current lineage/epoch/fence checked before this marker.
        fence: LineageFence,
    },
    /// Persist the observation before returning success or confirmed failure.
    Observation {
        /// Exact intention.
        intent: ActionIntent,
        /// Observed state and correlated dispatch.
        receipt: ActionReceipt,
        /// Sanitized failure/uncertainty detail, when present.
        reason: Option<String>,
    },
}

/// Failure to durably commit a broker phase. Details must already exclude secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Broker journal unavailable")]
pub struct BrokerJournalError;

/// Store port. `Ok` means the phase and its causal event are durably committed atomically.
///
/// Implementations must reject stale fences/revisions and conflicting action/dispatch identities.
/// Authorizing, Authorization and Observation are idempotent for an exact repeated phase and
/// never silently replace a different payload. **Dispatch is an exception:** `Ok` grants the
/// caller permission to launch the effect, so it may acknowledge only a newly committed marker.
/// A marker already present, even with the same dispatch ID, must return `Err`; the action must
/// then be inspected/reconciled. An unknown commit result never permits a fresh dispatch ID or
/// another automatic launch.
/// They must not hold a `SQLite` transaction across network I/O. Cancellation may leave a committed
/// entry, so recovery must inspect the ledger before resuming any external effect.
pub trait BrokerJournalPort: Send + Sync {
    /// Atomically persist the phase. An error prevents any following secret resolution/dispatch;
    /// an observation error after dispatch leaves an uncertain action for recovery.
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>>;
}

/// Explicitly non-durable journal for local tests only; cannot establish a persistence guarantee.
#[derive(Debug)]
pub(crate) struct TestMemoryJournal;

impl BrokerJournalPort for TestMemoryJournal {
    fn append<'a>(
        &'a self,
        _entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async { Ok(()) })
    }
}
