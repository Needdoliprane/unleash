//! Private, correlated retirement and reactivation of terminal control.
//! These messages never stop, spawn or replace the owned process or PTY.
use serde::{Deserialize, Serialize};
use unleash_domain::{DispatchId, HostId, LeaseId, ProcessId, Revision, Sha256Digest, TerminalId};
use unleash_platform::OsProcessIdentity;
use unleash_protocol::IpcError;
use zeroize::Zeroize;

use crate::{ControlTransferAck, ControlTransferRequest, HostCommandHeader};

/// Immutable coordinates of one writer-authorized native retirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReleaseKey {
    /// Includes the digest of the complete immutable Store release context.
    pub header: HostCommandHeader,
    /// Exact durable broker marker, never replaced on an uncertain ACK.
    pub dispatch_id: DispatchId,
    /// Original allocation acknowledged by the same host.
    pub initial_context_digest: Sha256Digest,
    /// Source lease verified by the writer, not independently read by the host.
    pub lease_id: LeaseId,
    /// Source effective control revision verified by the writer.
    pub control_revision: Revision,
}

/// One authenticated retirement. Credentials only cross the private local socket.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReleaseRequest {
    /// Exact source and durable effect identity.
    pub key: ControlReleaseKey,
    /// Currently effective private credential.
    pub current_secret: String,
    /// Replacement private credential; it does not grant terminal input control.
    pub next_secret: String,
}

impl std::fmt::Debug for ControlReleaseRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlReleaseRequest")
            .field("key", &self.key)
            .field("current_secret", &"[REDACTED]")
            .field("next_secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for ControlReleaseRequest {
    fn drop(&mut self) {
        self.current_secret.zeroize();
        self.next_secret.zeroize();
    }
}

/// Actual non-secret facts captured after native control was retired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReleaseAck {
    /// Exact original request, including its action/command/context and marker.
    pub key: ControlReleaseKey,
    /// Actual native host whose control was retired.
    pub host_id: HostId,
    /// Actual registered logical process retained by that host.
    pub process_id: ProcessId,
    /// Actual retained PTY resource.
    pub terminal_id: TerminalId,
    /// Freshly verified identity of the same anchored, live child.
    pub os_identity: OsProcessIdentity,
    /// Durable native output watermark at retirement.
    pub durable_log_seq: u64,
    /// Last native input confirmed before retirement.
    pub last_confirmed_input_seq: u64,
    /// Ambiguous inputs are preserved and never represented as confirmed.
    pub unknown_input_seqs: Vec<u64>,
}

/// Explicit activation of an already released physical resource.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReacquisitionRequest {
    /// New command/action/dispatch and strictly higher resource fence.
    pub transfer: ControlTransferRequest,
    /// Exact native retirement whose no-controller state is being consumed.
    pub released_by: DispatchId,
}

impl std::fmt::Debug for ControlReacquisitionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlReacquisitionRequest")
            .field("transfer", &self.transfer)
            .field("released_by", &self.released_by)
            .finish()
    }
}

impl Drop for ControlReacquisitionRequest {
    fn drop(&mut self) {
        self.transfer.current_secret.zeroize();
        self.transfer.next_secret.zeroize();
    }
}

/// Actual activation facts, separate from the historical retirement ACK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReacquisitionAck {
    /// Correlated new activation and the unchanged physical resource.
    pub transfer: ControlTransferAck,
    /// Native retirement consumed by this activation.
    pub released_by: DispatchId,
}

/// Native ownership state; an absent field in an old manifest is unqualified
/// for retirement, rather than evidence that retirement has occurred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostControlState {
    /// The host may accept inputs under its current capability and fencing.
    Active {
        /// Exact last explicit reactivation, if any; no new permission on replay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_reacquisition: Option<Box<ControlReacquisitionAck>>,
    },
    /// A retirement has begun; no terminal mutation may regain permission.
    Retiring {
        /// Exact effect identity, without any secret.
        key: ControlReleaseKey,
    },
    /// No terminal controller remains. Reads and separately authorized stop
    /// retain their own contracts under the current private credential.
    Released {
        /// Exact non-secret retirement acknowledgement retained for inspection.
        ack: Box<ControlReleaseAck>,
    },
    /// An explicit new activation is ambiguous; old control stays retired.
    Reactivating {
        /// Previous retirement, unchanged.
        released_by: DispatchId,
        /// Exact new activation request header.
        header: HostCommandHeader,
        /// Unique new activation marker.
        dispatch_id: DispatchId,
    },
}

impl HostControlState {
    pub(crate) const fn permits_input(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

/// A response that carries either qualified facts or a safe refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlReleaseResponse {
    /// Retirement completed; the source key is preserved at replay.
    Ack {
        /// Actual immutable retirement facts.
        ack: Box<ControlReleaseAck>,
    },
    /// Retirement has no qualified completion, including uncertain outcomes.
    Refused {
        /// Typed error with no capability content.
        error: IpcError,
    },
}

/// A response for a new activation of a released terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlReacquisitionResponse {
    /// Actual activation of the same physical resource.
    Ack {
        /// New activation facts, never a retirement acknowledgement.
        ack: Box<ControlReacquisitionAck>,
    },
    /// No qualified activation; a retry never invents a new marker.
    Refused {
        /// Typed error without private credentials.
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
pub(crate) enum ReleaseEnvelope {
    ReleaseControlV1(ControlReleaseRequest),
    AcquireReleasedControlV1(ControlReacquisitionRequest),
}
