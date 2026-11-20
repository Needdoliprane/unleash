//! Immutable retirement intent and actual native retirement facts.
use super::TerminalProcessIdentity;
use super::terminal_control_history::HistoricalTransferHeader;
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, CommandId, DaemonEpoch, DispatchId, Fence,
    HostId, LeaseId, MissionId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, Revision,
    Sha256Digest, TerminalId, TimestampMs,
};

/// Closed broker verb for retiring control without stopping a process.
pub const TERMINAL_RELEASE_VERB: &str = "terminal.release";

/// Closed public target. Authority and evidence never come from this payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseRequest {
    /// Logical terminal also named by the command aggregate.
    pub terminal_id: TerminalId,
    /// Exact physical process generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Explicit currently held controller lease.
    pub lease_id: LeaseId,
    /// Explicit current resource fence.
    pub fence: Fence,
}

impl TerminalReleaseRequest {
    pub(crate) fn is_valid(&self) -> bool {
        !self.terminal_id.as_uuid().is_nil()
            && !self.process_run_uuid.as_uuid().is_nil()
            && !self.lease_id.as_uuid().is_nil()
            && (1..=9_007_199_254_740_991).contains(&self.fence.as_u64())
    }
}

/// Complete accepted retirement. No proposed extension or successor lease exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseContext {
    /// Closed context format, exactly one.
    pub schema_version: u32,
    /// Original exact wire command.
    pub command_id: CommandId,
    /// Immutable acceptance receipt.
    pub receipt_id: ReceiptId,
    /// Distinct retirement action.
    pub action_id: ActionId,
    /// Existing logical terminal.
    pub terminal_id: TerminalId,
    /// Existing initial process-launch action, never repeated.
    pub initial_action_id: ActionId,
    /// Immutable V010 launch context digest.
    pub initial_context_digest: Sha256Digest,
    /// Existing native host.
    pub host_id: HostId,
    /// Existing logical process.
    pub process_id: ProcessId,
    /// Existing physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Authenticated actor, not a client-supplied principal.
    pub actor_id: ActorId,
    /// Resolved active principal of that actor.
    pub principal_id: ActorId,
    /// Same authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Current source incarnation.
    pub daemon_epoch: DaemonEpoch,
    /// Current physical boot.
    pub daemon_boot_identity: String,
    /// Exact owning project.
    pub project_id: ProjectId,
    /// Accepted project revision.
    pub scope_revision: Revision,
    /// Existing owned mission.
    pub mission_id: MissionId,
    /// Accepted mission revision.
    pub mission_revision: Revision,
    /// Effective security revision at acceptance.
    pub policy_revision: Revision,
    /// Complete policy snapshot digest, not its scalar revision alone.
    pub security_policy_digest: Sha256Digest,
    /// Exact configured runtime/pools; never an environment value.
    pub runtime_settings_digest: Sha256Digest,
    /// Existing lease being retired.
    pub lease_id: LeaseId,
    /// Existing resource fence, distinct from the broker lineage fence.
    pub resource_fence: Fence,
    /// Effective source control revision.
    pub control_revision: Revision,
    /// Source deadline; retirement never extends it.
    pub source_expires_at_ms: TimestampMs,
    /// Freshly revalidated registered OS identity is required before native I/O.
    pub expected_os_identity: TerminalProcessIdentity,
}

/// Intent state; an unknown native effect grants no retry or lease disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalReleaseState {
    /// Accepted, no native permission consumed.
    Prepared,
    /// One exact broker dispatch consumed.
    RetireResolved,
    /// Native retirement and the lease disposition committed atomically.
    Observed,
    /// Native retirement cannot yet be qualified; concurrency stays blocked.
    OutcomeUnknown,
    /// Definite terminal refusal before any marker.
    NotDispatched,
    /// Historical qualified retirement disposed the barrier without rewriting Unknown.
    Disposed,
}

/// Store-owned view of one intent and its immutable broker marker.
#[derive(Debug, Clone)]
pub struct TerminalReleaseRecord {
    /// Original accepted intent.
    pub context: TerminalReleaseContext,
    /// RFC8785/SHA256 digest of that complete intent.
    pub context_digest: Sha256Digest,
    /// Durable intent lifecycle.
    pub state: TerminalReleaseState,
    /// Intent CAS revision, distinct from effective control revision.
    pub revision: Revision,
    /// Consumed native permission, if any.
    pub dispatch_id: Option<DispatchId>,
    /// Published native observation, if any.
    pub observation_object_hash: Option<Sha256Digest>,
    /// Ledger state observed in the same snapshot.
    pub broker_action_state: Option<ActionState>,
    /// Ledger marker even when its permit consumption rolled back.
    pub broker_dispatch_id: Option<DispatchId>,
}

/// Serialized coordinates echoed by the native host; the header is facts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseNativeKey {
    /// Native v1 header; payload_hash is this accepted ReleaseContext digest.
    pub header: HistoricalTransferHeader,
    /// Original durable dispatch.
    pub dispatch_id: DispatchId,
    /// Original V010 binding actually checked by the host.
    pub initial_context_digest: Sha256Digest,
    /// Source lease verified by the writer and echoed by the host.
    pub lease_id: LeaseId,
    /// Source control revision verified by the writer and echoed by the host.
    pub control_revision: Revision,
}

/// Complete actual non-secret ACK; a digest alone cannot replace this body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseNativeAck {
    /// Original immutable source and dispatch coordinates.
    pub key: TerminalReleaseNativeKey,
    /// Actual native host.
    pub host_id: HostId,
    /// Actual retained logical process.
    pub process_id: ProcessId,
    /// Actual retained terminal.
    pub terminal_id: TerminalId,
    /// Actual same anchored child observed by that host.
    pub os_identity: TerminalProcessIdentity,
    /// Durable log watermark observed at retirement.
    pub durable_log_seq: u64,
    /// Last fully confirmed native input.
    pub last_confirmed_input_seq: u64,
    /// Native ambiguity preserved without replay or invented confirmation.
    pub unknown_input_seqs: Vec<u64>,
}

/// Evidence published by the native producer before any successful projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseObservation {
    /// Closed object format, exactly one.
    pub schema_version: u32,
    /// Complete native retirement facts.
    pub ack: TerminalReleaseNativeAck,
    /// Canonical digest of the actual post-retirement private manifest.
    pub manifest_digest: Sha256Digest,
    /// Actual capture time, never an invented time of the retirement effect.
    pub captured_at_ms: TimestampMs,
}

/// Consume the source permission only after native identity preflight.
#[derive(Debug, Clone)]
pub struct ResolveTerminalReleaseRequest {
    /// Authenticated, resolved internal caller.
    pub caller: crate::CallerAuthority,
    /// Original uniquely accepted retirement.
    pub action_id: ActionId,
    /// Exact immutable context.
    pub context_digest: Sha256Digest,
    /// Exact still-prepared intent revision.
    pub expected_revision: Revision,
    /// Already durable marker, never generated by this resolver.
    pub dispatch_id: DispatchId,
}

/// Public causal facts. The returned lease/fence have been retired, not granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleaseResult {
    /// Original accepted command.
    pub command_id: CommandId,
    /// Original accepted receipt.
    pub receipt_id: ReceiptId,
    /// Retained terminal.
    pub terminal_id: TerminalId,
    /// Retained physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Retired lease identity.
    pub lease_id: LeaseId,
    /// Retired resource fence.
    pub fence: Fence,
    /// Effective revision for a distinct explicit acquisition.
    pub control_revision: Revision,
    /// Closed disposition, always released.
    pub disposition: TerminalReleaseDisposition,
}

/// Closed disposition for the initial non-stopping release path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalReleaseDisposition {
    /// The native controller was retired; the physical resource lives on.
    Released,
}

/// Explicit source of a new activation, never a current capability by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReleasedControlSource {
    /// Prior retirement action.
    pub action_id: ActionId,
    /// Its unique native marker.
    pub dispatch_id: DispatchId,
    /// Digest of the immutable prior ReleaseContext.
    pub context_digest: Sha256Digest,
    /// Published full native retirement observation object.
    pub observation_object_hash: Sha256Digest,
}

/// Complete native response for the distinct explicit reactivation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalReacquisitionNativeAck {
    /// Exact prior native retirement, not a newly allocated identifier.
    pub released_by: DispatchId,
    /// Actual new rotation facts; they never certify the original Release action.
    pub transfer: super::terminal_control_history::HistoricalTransferAck,
}
