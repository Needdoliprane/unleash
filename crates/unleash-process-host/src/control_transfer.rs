//! Additive private control-transfer protocol. It never spawns a process.
use serde::{Deserialize, Serialize};
use unleash_domain::{DispatchId, Sha256Digest};
use unleash_platform::OsProcessIdentity;
use unleash_protocol::IpcError;

use crate::HostCommandHeader;

/// A transfer already committed and uniquely dispatched by the daemon writer.
/// Secrets travel only on the protected local control socket.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlTransferRequest {
    /// Exact accepted command/action/context and successor coordinates.
    pub header: HostCommandHeader,
    /// Unique durable marker; never replaced after an uncertain result.
    pub dispatch_id: DispatchId,
    /// Immutable launch context acknowledged by this existing host.
    pub initial_context_digest: Sha256Digest,
    /// Current capability, omitted from Debug and all proofs.
    pub current_secret: String,
    /// Planned replacement capability, omitted from Debug and all proofs.
    pub next_secret: String,
}
impl std::fmt::Debug for ControlTransferRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlTransferRequest")
            .field("header", &self.header)
            .field("dispatch_id", &self.dispatch_id)
            .field("initial_context_digest", &self.initial_context_digest)
            .field("current_secret", &"[REDACTED]")
            .field("next_secret", &"[REDACTED]")
            .finish()
    }
}

/// Actual non-secret acknowledgement from the same live host and child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlTransferAck {
    /// Exact accepted transfer, including payload digest and successor fence.
    pub header: HostCommandHeader,
    /// Original unique marker echoed by the host.
    pub dispatch_id: DispatchId,
    /// Initial allocation stays unchanged across transfer.
    pub initial_context_digest: Sha256Digest,
    /// Environment applied at the original spawn, unchanged by transfer.
    pub effective_environment_digest: Sha256Digest,
    /// Actual identity verified while the host retains the child anchor.
    pub os_identity: OsProcessIdentity,
    /// Durable output offset captured for this acknowledgement.
    pub durable_log_seq: u64,
    /// Last native input with a durable receipt.
    pub last_confirmed_input_seq: u64,
    /// Ambiguous inputs preserved without replay.
    pub unknown_input_seqs: Vec<u64>,
}

/// Closed private response. A refusal never becomes a successful transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlTransferResponse {
    /// Correlated facts; exact replay returns these same facts.
    Ack {
        /// Actual immutable acknowledgement.
        ack: Box<ControlTransferAck>,
    },
    /// No usable acknowledgement, including an uncertain intermediate result.
    Refused {
        /// Safe typed error; no capability or raw request is included.
        error: IpcError,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum TransferEnvelope {
    TransferControlV1(ControlTransferRequest),
}
