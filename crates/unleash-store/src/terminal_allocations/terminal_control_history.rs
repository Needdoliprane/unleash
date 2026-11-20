//! Historical V012 facts. This closed internal seam grants no control and
//! never turns an old rotation into a current broker success.

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, AuthorizationEvaluationId, CommandId, DaemonEpoch,
    DispatchId, Fence, ProcessRunUuid, ReceiptId, Sha256Digest, TimestampMs,
};
use unleash_protocol::EventCursor;

use super::{TerminalControlObservation, TerminalProcessIdentity};
use crate::{CallerAuthority, PreparedObjectHandle, SnapshotProjection, StoreError, UnleashStore};

#[path = "terminal_control_history_validation.rs"]
mod validation;
#[path = "terminal_control_history_writer.rs"]
mod writer;

pub(super) fn observed_launch_environment(
    conn: &rusqlite::Connection,
    config: &crate::StoreConfig,
    record: &super::TerminalControlRecord,
    allocation: &super::TerminalAllocationRecord,
) -> Result<Sha256Digest, StoreError> {
    validation::observed_environment(conn, config, record, allocation)
}

/// Append-only observation event; it is never a broker success phase.
pub const TERMINAL_CONTROL_HISTORY_EVENT: &str = "TerminalControlHistoricalAttested";

/// Non-secret header in the version-one native transfer acknowledgement.
/// Its serialized shape matches the host header; it is not a host command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalTransferHeader {
    /// Native protocol version, exactly one.
    pub protocol_version: u32,
    /// Original physical process generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Original authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Epoch of the rotation, not the inspector's epoch.
    pub daemon_epoch: DaemonEpoch,
    /// Fence of the rotation, never an input permission.
    pub resource_fence: Fence,
    /// Original accepted command.
    pub command_id: CommandId,
    /// Original uniquely dispatched action.
    pub action_id: ActionId,
    /// Original accepted V012 context digest.
    pub payload_hash: Sha256Digest,
}

/// Complete non-secret native ACK, retained as facts rather than authority.
/// OS identity serialization is shared with the existing terminal proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalTransferAck {
    /// Exact native header of the original transfer command.
    pub header: HistoricalTransferHeader,
    /// Original unique transfer marker.
    pub dispatch_id: DispatchId,
    /// Immutable launch context acknowledged by the host.
    pub initial_context_digest: Sha256Digest,
    /// Environment actually applied to the existing child.
    pub effective_environment_digest: Sha256Digest,
    /// Identity reported by the host's retained child anchor.
    pub os_identity: TerminalProcessIdentity,
    /// Durable output sequence in the immutable ACK.
    pub durable_log_seq: u64,
    /// Last natively receipted input in the immutable ACK.
    pub last_confirmed_input_seq: u64,
    /// Native ambiguous inputs; no historical fact permits replaying them.
    pub unknown_input_seqs: Vec<u64>,
}

/// Bounded object prepared by the future internal reconciliation producer.
/// Tests may model this shape; the Store cannot manufacture a native ACK.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalControlHistoryProof {
    /// Closed evidence format, exactly one.
    pub schema_version: u32,
    /// Actual initial acceptance receipt, not an inspection receipt.
    pub source_receipt_id: ReceiptId,
    /// Actual authorization evaluation of the original dispatch.
    pub source_evaluation_id: AuthorizationEvaluationId,
    /// Immutable broker intent of that evaluation.
    pub source_intent_digest: Sha256Digest,
    /// Complete native ACK. No secret or capability value is included.
    pub ack: HistoricalTransferAck,
    /// Fresh attributable inspection of the same host and OS generation.
    /// Its capture time is not the unknown time of the rotation effect.
    pub inspection: TerminalControlObservation,
}

/// Internal request. The writer derives the current authority and epoch;
/// neither comes from this request or from the historical native header.
#[derive(Debug, Clone)]
pub struct SealTerminalControlHistoryRequest {
    /// Authenticated internal caller, revalidated against the source owner.
    pub caller: CallerAuthority,
    /// Original V012 action, which remains unknown after adoption.
    pub source_action_id: ActionId,
    /// Exact consumed dispatch of that action.
    pub dispatch_id: DispatchId,
    /// Exact immutable context, including old and planned control coordinates.
    pub context_digest: Sha256Digest,
    /// Physically prepared evidence; reference and event must seal atomically.
    pub prepared_proof: PreparedObjectHandle,
}

/// A historical observation, deliberately without an exercisable lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalControlHistoryFact {
    /// Closed factual result format.
    pub schema_version: u32,
    /// Original unknown action.
    pub action_id: ActionId,
    /// Original unique marker.
    pub dispatch_id: DispatchId,
    /// Original accepted receipt.
    pub source_receipt_id: ReceiptId,
    /// Original authorization evaluation.
    pub source_evaluation_id: AuthorizationEvaluationId,
    /// Original immutable broker intent.
    pub source_intent_digest: Sha256Digest,
    /// Original accepted control context.
    pub context_digest: Sha256Digest,
    /// Actual canonical native ACK digest, stable across later inspections.
    pub native_ack_digest: Sha256Digest,
    /// First qualified proof object retained by the fact.
    pub observation_object_hash: Sha256Digest,
    /// Rotation epoch attested historically, never promoted to the current epoch.
    pub rotation_epoch: DaemonEpoch,
    /// Rotation fence attested historically, without any grant of control.
    pub rotation_fence: Fence,
    /// Reconciler epoch derived by the writer at the first adoption.
    pub inspected_by_epoch: DaemonEpoch,
    /// Attributed inspector, distinct from the original native command header.
    pub inspected_by: ActorId,
    /// First inspection capture time. No effect timestamp is inferred.
    pub observed_at_ms: TimestampMs,
    /// Original append-only event, returned unchanged on replay.
    pub event_cursor: EventCursor,
}

/// Whether the same fact was already sealed, without another publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalControlHistoryCommit {
    /// Qualified historical observation, carrying no current capability.
    pub fact: TerminalControlHistoryFact,
    /// True for an identical already sealed native fact.
    pub replayed: bool,
}

impl UnleashStore {
    /// Seal a historical fact without changing the original unknown action,
    /// current control, leases, TTLs, reservations or broker phases.
    ///
    /// # Errors
    /// Refuses foreign, malformed, unbound, missing or tombstoned evidence.
    pub fn seal_terminal_control_history(
        &self,
        request: SealTerminalControlHistoryRequest,
    ) -> Result<TerminalControlHistoryCommit, StoreError> {
        writer::seal(self, request)
    }

    /// Read an already sealed fact under its exact source owner's ACL. This
    /// read never contacts a host or infers control from the old receipt.
    ///
    /// # Errors
    /// Refuses foreign, malformed, missing or tombstoned evidence.
    pub fn inspect_terminal_control_history(
        &self,
        actor: &ActorId,
        source_action_id: ActionId,
        dispatch_id: DispatchId,
    ) -> Result<SnapshotProjection<Option<TerminalControlHistoryFact>>, StoreError> {
        writer::inspect(self, actor, source_action_id, dispatch_id)
    }
}
