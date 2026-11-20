//! Private inherited channel for an ephemeral invocation attempt.
#![forbid(unsafe_code)]

use std::io::{Read, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use unleash_domain::{
    AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, Fence, InvocationId, LeaseId,
    ReservationId, Revision, SessionId, Sha256Digest,
};
use unleash_protocol::{read_frame, write_frame};
use uuid::Uuid;

/// Version of the inherited worker channel, independent of public IPC negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerWireVersion {
    /// Exact attempt and manifest binding.
    #[serde(rename = "unleash.agent-worker.v1")]
    V1,
}

/// Continuation coordinates the parent selects from the committed admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBinding {
    /// Logical invocation.
    pub invocation_id: InvocationId,
    /// Exact newly admitted attempt.
    pub attempt_id: AttemptId,
    /// Logical agent.
    pub agent_id: AgentId,
    /// Durable session bound to the invocation.
    pub session_id: SessionId,
    /// Held global execution reservation.
    pub reservation_id: ReservationId,
    /// Execution lease.
    pub lease_id: LeaseId,
    /// Authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation preparing this child.
    pub daemon_epoch: DaemonEpoch,
    /// Fence of the exact attempt.
    pub fence: Fence,
    /// Initial context manifest identity.
    pub context_manifest_id: Uuid,
    /// SHA-256 of the original manifest bytes.
    pub context_manifest_digest: Sha256Digest,
    /// Frozen scope identity.
    pub scope_id: String,
    /// Frozen scope revision.
    pub scope_revision: Revision,
}

/// First frame on the private descriptor pair. It contains the verified manifest bytes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBootstrap {
    /// Required worker channel version.
    pub wire_version: WorkerWireVersion,
    /// Coordinates for this child only.
    pub binding: WorkerBinding,
    /// Exact UTF-8 JSON of the durable manifest.
    pub manifest_json: String,
}

/// The owning daemon sends this frame after its running transition commits.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPermit {
    /// Required worker channel version.
    pub wire_version: WorkerWireVersion,
    /// Same coordinates as the bootstrap.
    pub binding: WorkerBinding,
}

/// Observed state of the inherited channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReplyKind {
    /// The manifest was verified. The child is waiting for its permit.
    Ready,
    /// The matching permit was consumed.
    Activated,
}

/// Reply on the inherited channel, correlated to the actual child and attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReply {
    /// Negotiated worker version.
    pub wire_version: WorkerWireVersion,
    /// Stage the child reached.
    pub kind: WorkerReplyKind,
    /// Child PID. The parent must verify it against its owned child handle.
    pub pid: u32,
    /// Exact binding acknowledged by the child.
    pub binding: WorkerBinding,
}

/// Errors are deliberately independent of manifest and incoming frame contents.
#[derive(Debug, thiserror::Error)]
pub enum WorkerChannelError {
    /// An inherited descriptor failed or the frame was truncated or oversized.
    #[error("worker channel frame unavailable")]
    Frame,
    /// A frame did not match the expected closed message.
    #[error("invalid worker channel message")]
    Message,
    /// The received manifest failed shape or digest verification.
    #[error("worker context manifest unavailable")]
    Manifest,
    /// The parent frame did not authorize this exact attempt.
    #[error("worker permit does not match its admission")]
    Permit,
}

impl WorkerBootstrap {
    /// Check that the manifest bytes match this binding and contain a JSON object.
    ///
    /// # Errors
    /// Refuses an invalid scope, incarnation or unverified manifest.
    pub fn verify(&self) -> Result<(), WorkerChannelError> {
        let binding = &self.binding;
        if binding.fence == Fence::ZERO
            || binding.daemon_epoch.as_u64() == 0
            || binding.scope_id.trim().is_empty()
        {
            return Err(WorkerChannelError::Permit);
        }
        if Sha256Digest::from_bytes(self.manifest_json.as_bytes())
            != binding.context_manifest_digest
            || !serde_json::from_str::<serde_json::Value>(&self.manifest_json)
                .is_ok_and(|value| value.is_object())
        {
            return Err(WorkerChannelError::Manifest);
        }
        Ok(())
    }
}

/// Handle the child side of the inherited channel. EOF ends its continuation.
///
/// The first reply attests only readiness. A permit from the same descriptor is
/// required for activation. The PID is resolved by the process, never taken from
/// an incoming frame.
///
/// # Errors
/// Refuses invalid frames and permits for a different binding.
pub fn run_inherited_channel(
    mut reader: impl Read,
    mut writer: impl Write,
) -> Result<(), WorkerChannelError> {
    let Some(bootstrap) = receive::<WorkerBootstrap>(&mut reader)? else {
        return Ok(());
    };
    bootstrap.verify()?;
    let mut reply = WorkerReply {
        wire_version: bootstrap.wire_version,
        kind: WorkerReplyKind::Ready,
        pid: std::process::id(),
        binding: bootstrap.binding,
    };
    send(&mut writer, &reply)?;

    let Some(permit) = receive::<WorkerPermit>(&mut reader)? else {
        return Ok(());
    };
    if permit.wire_version != reply.wire_version || permit.binding != reply.binding {
        return Err(WorkerChannelError::Permit);
    }
    reply.kind = WorkerReplyKind::Activated;
    send(&mut writer, &reply)?;

    if read_frame(&mut reader)
        .map_err(|_| WorkerChannelError::Frame)?
        .is_some()
    {
        return Err(WorkerChannelError::Message);
    }
    Ok(())
}

fn receive<T: DeserializeOwned>(reader: &mut impl Read) -> Result<Option<T>, WorkerChannelError> {
    read_frame(reader)
        .map_err(|_| WorkerChannelError::Frame)?
        .map(|frame| serde_json::from_slice(&frame).map_err(|_| WorkerChannelError::Message))
        .transpose()
}

fn send(writer: &mut impl Write, reply: &WorkerReply) -> Result<(), WorkerChannelError> {
    let frame = serde_json::to_vec(reply).map_err(|_| WorkerChannelError::Message)?;
    write_frame(writer, &frame).map_err(|_| WorkerChannelError::Frame)
}
