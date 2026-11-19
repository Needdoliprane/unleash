//! Same-incarnation lease renewal facts, distinct from successor transfer.
use super::TerminalProcessIdentity;
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DispatchId, Fence, HostId,
    LeaseId, MissionId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, Revision, Sha256Digest,
    TerminalId, TimestampMs,
};

/// Internal broker verb; a renewal never launches or transfers control.
pub const TERMINAL_RENEWAL_VERB: &str = "terminal.renew_registered";

/// Immutable writer-built intention. Its proposed deadline is stable across
/// replay, but grants no time until its native observation is committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRenewalContext {
    /// Closed durable format.
    pub schema_version: u32,
    /// Accepted exact wire command.
    pub command_id: unleash_domain::CommandId,
    /// Durable accepted cause, never a capability on its own.
    pub receipt_id: ReceiptId,
    /// Distinct renewal action, never the initial launch.
    pub action_id: ActionId,
    /// Existing registered terminal.
    pub terminal_id: TerminalId,
    /// Original launch action.
    pub initial_action_id: ActionId,
    /// Exact immutable launch context.
    pub initial_context_digest: Sha256Digest,
    /// Existing native host.
    pub host_id: HostId,
    /// Existing logical process.
    pub process_id: ProcessId,
    /// Existing physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Transport-authenticated actor.
    pub actor_id: ActorId,
    /// Writer-resolved human principal.
    pub principal_id: ActorId,
    /// Unchanged authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Current incarnation, identical to the lease incarnation.
    pub daemon_epoch: DaemonEpoch,
    /// Current physical boot.
    pub daemon_boot_identity: String,
    /// Exact authorized project.
    pub project_id: ProjectId,
    /// Pinned project scope revision.
    pub scope_revision: Revision,
    /// Existing mission.
    pub mission_id: MissionId,
    /// Pinned human-owned mission revision.
    pub mission_revision: Revision,
    /// Pinned effective security policy revision.
    pub policy_revision: Revision,
    /// Exact effective policy snapshot.
    pub security_policy_digest: Sha256Digest,
    /// Exact runtime settings, including configured TTL and pools.
    pub runtime_settings_digest: Sha256Digest,
    /// The existing held lease; no successor ID is allocated.
    pub lease_id: LeaseId,
    /// Unchanged current resource fence.
    pub resource_fence: Fence,
    /// Unchanged effective control revision.
    pub control_revision: Revision,
    /// Deadline observed at acceptance, used in the activation CAS.
    pub previous_expires_at_ms: TimestampMs,
    /// UTC plus configured TTL, proposed once before broker authorization.
    pub proposed_expires_at_ms: TimestampMs,
    /// Actual registered identity, reobserved before the native handshake.
    pub expected_os_identity: TerminalProcessIdentity,
}

/// Closed durable states; unknown never reopens a native permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalRenewalState {
    /// Accepted with no consumed dispatch.
    Prepared,
    /// One durable dispatch has been consumed.
    HandshakeResolved,
    /// Qualified proof and the deadline CAS committed atomically.
    Observed,
    /// Reconciliation is required, with no retry permission.
    OutcomeUnknown,
    /// Old control was neutralized by a separately qualified disposition;
    /// the historical unknown action and its deadline remain unchanged.
    Disposed,
}

/// Snapshot of an exact intention and its durable broker state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRenewalRecord {
    /// Immutable accepted context.
    pub context: TerminalRenewalContext,
    /// Canonical hash of that context.
    pub context_digest: Sha256Digest,
    /// Durable intention state.
    pub state: TerminalRenewalState,
    /// CAS revision of the intention, distinct from control revision.
    pub revision: Revision,
    /// Consumed dispatch, if any.
    pub dispatch_id: Option<DispatchId>,
    /// Published native observation hash, if committed.
    pub observation_object_hash: Option<Sha256Digest>,
    /// Current broker action state, including marker-only uncertainty.
    pub broker_action_state: Option<ActionState>,
    /// Exact durable broker dispatch, even before permit consumption.
    pub broker_dispatch_id: Option<DispatchId>,
}

/// Native facts captured after one authenticated, non-rotating handshake.
/// Digests cover actual non-secret response/manifest bytes, never secret input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRenewalObservation {
    /// Same renewal action.
    pub action_id: ActionId,
    /// Same one-use durable dispatch.
    pub dispatch_id: DispatchId,
    /// Exact renewal intention.
    pub context_digest: Sha256Digest,
    /// Existing registered terminal.
    pub terminal_id: TerminalId,
    /// Existing native host.
    pub host_id: HostId,
    /// Existing physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Freshly reobserved registered OS identity.
    pub os_identity: TerminalProcessIdentity,
    /// Same incarnation, without rotation.
    pub daemon_epoch: DaemonEpoch,
    /// Same resource fence, without transfer.
    pub resource_fence: Fence,
    /// Canonical digest of the actual handshake response.
    pub handshake_response_digest: Sha256Digest,
    /// Canonical digest of the actual post-handshake manifest.
    pub manifest_digest: Sha256Digest,
    /// Native applied environment digest, including host-managed temporary path.
    pub effective_environment_digest: Sha256Digest,
    /// Native durable log watermark.
    pub durable_log_seq: u64,
    /// Last fully confirmed native input sequence.
    pub last_confirmed_input_seq: u64,
    /// Native ambiguous inputs; a nonempty set cannot grant renewal.
    pub unknown_input_seqs: Vec<u64>,
    /// Actual capture UTC, between dispatch and broker receipt.
    pub observed_at_ms: TimestampMs,
}

/// Consume a renewal dispatch once, after identity preflight and before host I/O.
#[derive(Debug, Clone)]
pub struct ResolveTerminalRenewalRequest {
    /// Exact renewal action.
    pub action_id: ActionId,
    /// Exact durable dispatch.
    pub dispatch_id: DispatchId,
    /// Original immutable context.
    pub context_digest: Sha256Digest,
    /// Current intention CAS, distinct from session revision.
    pub expected_revision: Revision,
    /// Authenticated control authority, never inferred from a query result.
    pub caller: crate::CallerAuthority,
}

/// Exact durable kind of a registered acquisition; never inferred by a client.
#[derive(Debug, Clone)]
pub enum RegisteredTerminalIntent {
    /// Successor-incarnation transfer with a higher fence.
    Transfer(super::TerminalControlRecord),
    /// Same-incarnation observation with an unchanged controller.
    Renewal(TerminalRenewalRecord),
}
