//! Digest-only input intention and native observation contracts.
//! Deserialization alone never grants control or dispatch permission. Raw input
//! exists only in the request; durable contexts retain its digest and length.

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, CommandId, DaemonEpoch, DispatchId, Fence,
    HostId, LeaseId, MissionId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, Revision,
    Sha256Digest, TerminalId, TimestampMs,
};

/// Closed internal action verb; no public alias is inferred from it.
pub const TERMINAL_INPUT_VERB: &str = "terminal.send_input";
/// Maximum bytes in one new input, independent of the private frame limit.
pub const MAX_TERMINAL_INPUT_BYTES: usize = 65_536;
/// Exact integer range shared by JSON and existing JavaScript consumers.
pub const MAX_TERMINAL_INPUT_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

/// Public input shape, subject to static validation and authoritative writer checks.
/// In particular an existing result is a fact, never an input permit.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputRequest {
    /// Resource also named by the envelope aggregate.
    pub terminal_id: TerminalId,
    /// Exact physical process generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Explicit controller lease; never acquired implicitly.
    pub lease_id: LeaseId,
    /// Explicit resource fence, in the exact wire integer domain.
    pub fence: Fence,
    /// Next sequence requested after the last confirmed native receipt.
    pub input_seq: u64,
    /// Arbitrary bytes, with length from one through 65,536 inclusive.
    pub payload_bytes: Vec<u8>,
}

impl std::fmt::Debug for TerminalInputRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalInputRequest")
            .field("terminal_id", &self.terminal_id)
            .field("process_run_uuid", &self.process_run_uuid)
            .field("lease_id", &self.lease_id)
            .field("fence", &self.fence)
            .field("input_seq", &self.input_seq)
            .field("payload_byte_length", &self.payload_bytes.len())
            .finish_non_exhaustive()
    }
}

/// Immutable writer-built input intent. No bytes or private host secret/path
/// can enter its persisted form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputContext {
    /// Closed durable format version.
    pub schema_version: u32,
    /// Allocated once before broker authorizing, including uncertain commit ACKs.
    pub action_id: ActionId,
    /// Original exact wire command.
    pub command_id: CommandId,
    /// Original accepted receipt, reused before any new effect attempt.
    pub receipt_id: ReceiptId,
    /// Actor resolved by the daemon transport.
    pub actor_id: ActorId,
    /// Existing human principal, resolved in the writer.
    pub principal_id: ActorId,
    /// Current authority lineage at acceptance.
    pub authority_generation: AuthorityGeneration,
    /// Current daemon incarnation at acceptance.
    pub daemon_epoch: DaemonEpoch,
    /// Current physical boot, not a claimed process identity.
    pub daemon_boot_identity: String,
    /// Exact project scope.
    pub project_id: ProjectId,
    /// Current scope revision pinned before Guard.
    pub scope_revision: Revision,
    /// Existing mission, with current human ownership.
    pub mission_id: MissionId,
    /// Current mission revision pinned before Guard.
    pub mission_revision: Revision,
    /// Current policy revision pinned before Guard.
    pub policy_revision: Revision,
    /// Exact policy content pinned before Guard.
    pub security_policy_digest: Sha256Digest,
    /// Exact configured TTL/runtime settings read by the writer.
    pub runtime_settings_digest: Sha256Digest,
    /// Registered resource, with no allocation or spawn semantics.
    pub terminal_id: TerminalId,
    /// Registered process.
    pub process_id: ProcessId,
    /// Already observed native host.
    pub host_id: HostId,
    /// Already observed physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Original launch action, which this intent can never repeat.
    pub initial_action_id: ActionId,
    /// Unchanged launch context.
    pub initial_context_digest: Sha256Digest,
    /// Effective controller CAS, not an input counter.
    pub control_revision: Revision,
    /// Exact lease held at acceptance and revalidated before dispatch.
    pub lease_id: LeaseId,
    /// Exact resource fence held at acceptance.
    pub resource_fence: Fence,
    /// Existing deadline; accepting an intent does not extend it.
    pub previous_lease_expires_at: TimestampMs,
    /// Proposed deadline, fixed once. Activation requires a conclusive native
    /// receipt and the same still-held, unexpired lease in the current incarnation.
    pub proposed_lease_expires_at: TimestampMs,
    /// Requested native sequence. Unknown prevents any new sequence admission.
    pub input_seq: u64,
    /// SHA-256 of the exact bytes; distinct from the context's canonical digest.
    pub payload_digest: Sha256Digest,
    /// Exact byte length, without retaining the bytes.
    pub payload_byte_length: u64,
}

/// Input intent lifecycle, separate from transfer and process allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalInputState {
    /// Accepted command before a one-use effect permit.
    Prepared,
    /// Durable broker dispatch consumed before private host I/O.
    WriteResolved,
    /// Real native receipt durably recorded; renewal can still be refused.
    Observed,
    /// Native effect/receipt cannot be proved; no automatic repetition or renewal.
    OutcomeUnknown,
    /// Durable terminal authorization refusal with no dispatch or effect permit.
    NotDispatched,
    /// Explicit qualified disposition retained historically.
    Disposed,
}

/// Snapshot retains both the consumed permit and the broker marker. A rollback
/// can leave the former absent after the latter is already durable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputRecord {
    /// Immutable accepted intent.
    pub context: TerminalInputContext,
    /// Canonical digest of the complete context, not its raw payload digest.
    pub context_digest: Sha256Digest,
    /// Input intent state.
    pub state: TerminalInputState,
    /// Intent CAS, independent of the terminal controller revision.
    pub revision: Revision,
    /// One-use effect permit already consumed, if any.
    pub dispatch_id: Option<DispatchId>,
    /// Exact published native observation object, if recorded.
    pub observation_object_hash: Option<Sha256Digest>,
    /// Original renewal decision committed with that observation, never
    /// recalculated to extend the lease when its receipt is replayed.
    pub renewal: Option<TerminalInputRenewal>,
    /// Original activated deadline, present only when renewal was committed.
    pub renewed_lease_expires_at: Option<TimestampMs>,
    /// Broker state from the same snapshot.
    pub broker_action_state: Option<ActionState>,
    /// Original broker dispatch, even after a resolution rollback.
    pub broker_dispatch_id: Option<DispatchId>,
}

/// Stable contents of the actual native input receipt. Transport deduplication
/// flags are not part of the persisted native receipt identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalNativeInputReceipt {
    /// Native sequence actually confirmed.
    pub input_seq: u64,
    /// Original command, not a reconciliation command.
    pub command_id: CommandId,
    /// Original effect action.
    pub action_id: ActionId,
    /// Receipt generated and persisted by the native host.
    pub receipt_id: ReceiptId,
    /// Hash of the exact bytes actually written.
    pub payload_hash: Sha256Digest,
}

/// Actual native receipt plus its dispatch/context and capture provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputObservation {
    /// Effect action whose receipt was received.
    pub action_id: ActionId,
    /// Original durable broker marker.
    pub dispatch_id: DispatchId,
    /// Exact immutable input intent.
    pub context_digest: Sha256Digest,
    /// Registered terminal, never a spawn request.
    pub terminal_id: TerminalId,
    /// Native host that returned the receipt.
    pub host_id: HostId,
    /// Exact physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Controller incarnation used for the actual native request.
    pub daemon_epoch: DaemonEpoch,
    /// Resource fence used for that request.
    pub resource_fence: Fence,
    /// Stable receipt read from the real native response.
    pub native_receipt: TerminalNativeInputReceipt,
    /// Digest of the complete actual native response; no value is fabricated
    /// when the response was lost or never received.
    pub native_response_digest: Sha256Digest,
    /// Time the daemon captured the response, before object publication.
    pub observed_at_ms: TimestampMs,
}

/// A known physical effect and authority to renew are separate facts. A late
/// receipt remains evidence even when it cannot reactivate expired control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalInputRenewal {
    /// New deadline engaged atomically with the native receipt.
    Renewed,
    /// Original lease has expired; no reactivation.
    LeaseExpired,
    /// Another controller/lease/fence is now current.
    ControllerChanged,
    /// Authority incarnation changed; no grant for an old controller.
    AuthorityChanged,
    /// A reliable UTC deadline cannot be established.
    ClockUncertain,
    /// Current scope, subject or policy no longer permits a new control grant.
    AuthorityBlocked,
}

/// One-use internal resolution after the exact broker Dispatch ACK. An error
/// never permits host I/O or an automatic retry with another dispatch identity.
#[derive(Debug, Clone)]
pub struct ResolveTerminalInputRequest {
    /// Original input action, distinct from process launch or control transfer.
    pub action_id: ActionId,
    /// Exact marker already durably committed by the broker.
    pub dispatch_id: DispatchId,
    /// Accepted authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Input-intention revision, independently of the control revision.
    pub expected_revision: Revision,
    /// Accepted digest-only context.
    pub context_digest: Sha256Digest,
    /// Authenticated daemon caller, never decoded from the public payload.
    pub caller: crate::CallerAuthority,
}

/// A newly consumed permit. This is neither a native receipt nor reusable
/// authority: the executor must still validate the actual host before writing.
#[derive(Debug, Clone)]
pub struct ResolvedTerminalInput {
    /// Exact consumed intention and durable dispatch.
    pub input: TerminalInputRecord,
    /// Physical identity already observed and recorded for this process.
    pub expected_os_identity: super::TerminalProcessIdentity,
}
