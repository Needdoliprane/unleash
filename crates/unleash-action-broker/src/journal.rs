//! Durable boundary implemented by the daemon/store adapter; this crate never owns `SQLite`.

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActorId, ApprovalScopeId, AuthorizationEvaluationId, DispatchId, LineageFence, ReceiptId,
    Revision, Sha256Digest, TimestampMs,
};

use crate::{
    ActionBrokerError, ActionIntent, ActionReceipt, AuthorizationDecision, AuthorizationOutcome,
    BoxFuture,
};

/// Stable identity of one authorization evaluation, allocated before its first durable append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationEvaluation {
    /// Durable local identity allocated before the first journal append.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Previous evaluation of this action, only after its durable retryable result and backoff.
    pub previous_evaluation_id: Option<AuthorizationEvaluationId>,
}

/// Cause of an authorization record; a human resolution never overwrites the evaluation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthorizationCause {
    /// Result of this local/Guard/cache evaluation.
    Evaluation,
    /// A committed human decision, to be verified against the exact receipt by the adapter.
    HumanApproval {
        /// Resolved deciding actor.
        actor_id: ActorId,
        /// Existing durable decision receipt; the broker never fabricates it.
        decision_receipt_id: ReceiptId,
        /// Canonical command digest recorded with that receipt, not a provider hash.
        decision_command_digest: Sha256Digest,
        /// Exact approved scope.
        approval_scope: ApprovalScopeId,
    },
}

/// Source of an observation; optional references are supplied evidence, not an assertion of verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationCause {
    /// Executor observation; no separate evidence reference was supplied.
    Execution,
    /// Explicit reconciliation; `None` states that no evidence reference was supplied.
    Reconciliation {
        /// Actual evidence reference checked by the adapter, if present.
        evidence_ref: Option<String>,
    },
    /// External receipt bound to an existing dispatch and fence.
    ExternalReceipt {
        /// Actual evidence reference checked by the adapter, if present.
        evidence_ref: Option<String>,
    },
}

/// Durable phase, correlated to a stable evaluation and immutable exact replay payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerJournalEntry {
    /// Persist the exact intention before authorization starts.
    Authorizing {
        /// Identity and retry predecessor.
        evaluation: AuthorizationEvaluation,
        /// Proposed action and lineage.
        intent: ActionIntent,
    },
    /// Persist a decision, refusal or temporary unavailability; Allow never means success.
    Authorization {
        /// Identity and retry predecessor.
        evaluation: AuthorizationEvaluation,
        /// Evaluation or separately attributed human decision.
        cause: AuthorizationCause,
        /// Exact intention.
        intent: ActionIntent,
        /// Actual authorization result, including its retry semantics.
        outcome: Result<AuthorizationOutcome, ActionBrokerError>,
        /// Stable UTC lower bound for a retryable unavailable result, allocated before append.
        /// `None` for every other result; monotonic backoff is also enforced after acknowledgement.
        retry_not_before_ms: Option<TimestampMs>,
    },
    /// Commit the dispatch marker before secrets or external I/O.
    Dispatch {
        /// Evaluation whose successful authorization is being used.
        evaluation: AuthorizationEvaluation,
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
        /// Evaluation used by the existing dispatch.
        evaluation: AuthorizationEvaluation,
        /// Stable ordinal of this action's observations, allocated before append; distinct from SQL action revision.
        observation_revision: Revision,
        /// Explicit source, without raw executor/Guard text.
        cause: ObservationCause,
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
/// never silently replace a different payload. Evaluation results and receipt-bound human
/// resolutions are distinct causes; observations are keyed by evaluation, dispatch and ordinal.
/// The adapter verifies human receipt actor, command digest and scope, and any supplied evidence
/// reference before accepting a resolution. A cache source must have a durable authorization.
/// A new evaluation may follow only the latest known durable retryable result after backoff;
/// its predecessor belongs to the same action. In-memory absence after restart does not prove
/// absence in the ledger: recovery must load the durable evaluation or refuse a fresh one.
/// **Dispatch is an exception:** `Ok` grants the
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
