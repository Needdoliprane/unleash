//! Preparation of an admitted worker, held on its private channel.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use unleash_agent_worker::{
    WorkerBinding, WorkerBootstrap, WorkerReply, WorkerReplyKind, WorkerWireVersion,
};
use unleash_platform::{OsProcessIdentity, effective_uid, inspect_process_identity};
use unleash_protocol::{MAX_FRAME_BYTES, write_frame};
use unleash_store::{AdmissionCommit, StoreError, UnleashStore, WorkerLaunchPreparation};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Preparation could not establish an owned, waiting worker.
#[derive(Debug, thiserror::Error)]
pub enum WorkerLaunchError {
    /// The exact durable admission could not be claimed.
    #[error("worker admission unavailable")]
    Admission(#[from] StoreError),
    /// The configured packaged worker or its native process was unavailable.
    #[error("owned worker process unavailable")]
    NativeProcess,
    /// Native identity differs from the current daemon incarnation or UID.
    #[error("owned worker identity differs from its admission")]
    Identity,
    /// The stored manifest cannot be passed on the closed bootstrap channel.
    #[error("worker context manifest unavailable")]
    Manifest,
    /// A readiness frame did not describe this owned child.
    #[error("worker readiness unavailable")]
    Readiness,
    /// Startup or closure did not finish inside the controller's deadline.
    #[error("worker deadline exceeded")]
    Deadline,
}

/// A child waiting for the separate running transition.
pub struct PendingInvocationWorker {
    child: Child,
    identity: OsProcessIdentity,
    ready: WorkerReply,
    preparation: WorkerLaunchPreparation,
}

impl PendingInvocationWorker {
    /// Identity observed independently from the child's reply.
    pub const fn identity(&self) -> &OsProcessIdentity {
        &self.identity
    }

    /// Exact readiness observation.
    pub const fn ready(&self) -> &WorkerReply {
        &self.ready
    }

    /// Durable revision and event preceding native launch.
    pub const fn preparation(&self) -> &WorkerLaunchPreparation {
        &self.preparation
    }

    /// Close the controller and reap the still-gated worker.
    ///
    /// # Errors
    /// Returns an error if the exit cannot be established.
    pub async fn close(mut self) -> Result<(), WorkerLaunchError> {
        drop(self.child.stdin.take());
        let status = tokio::time::timeout(STARTUP_TIMEOUT, self.child.wait())
            .await
            .map_err(|_| WorkerLaunchError::Deadline)?
            .map_err(|_| WorkerLaunchError::NativeProcess)?;
        if status.success() {
            Ok(())
        } else {
            Err(WorkerLaunchError::NativeProcess)
        }
    }
}

/// Claim one admission and start a worker without granting continuation.
/// The executable is selected by the host packaging, outside the worker's request.
/// An error preserves the reservation and the exclusive claim for reconciliation.
///
/// # Errors
/// Refuses absent or obsolete admission, unavailable manifest and native startup failure.
pub async fn prepare_invocation_worker(
    store: Arc<UnleashStore>,
    admission: AdmissionCommit,
    executable: &Path,
) -> Result<PendingInvocationWorker, WorkerLaunchError> {
    if !executable.is_absolute()
        || !tokio::fs::metadata(executable)
            .await
            .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(WorkerLaunchError::NativeProcess);
    }
    let boot = admission.lease.boot_identity.clone();
    let (preparation, binding, frame) =
        tokio::task::spawn_blocking(move || prepare_frame(&store, admission))
            .await
            .map_err(|_| WorkerLaunchError::NativeProcess)??;
    let mut child = Command::new(executable)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| WorkerLaunchError::NativeProcess)?;
    let pid = child.id().ok_or(WorkerLaunchError::NativeProcess)?;
    let ready = tokio::time::timeout(STARTUP_TIMEOUT, exchange_ready(&mut child, &frame))
        .await
        .map_err(|_| WorkerLaunchError::Deadline)??;
    if ready.wire_version != WorkerWireVersion::V1
        || ready.kind != WorkerReplyKind::Ready
        || ready.pid != pid
        || ready.binding != binding
    {
        return Err(WorkerLaunchError::Readiness);
    }
    let identity = inspect_process_identity(pid).map_err(|_| WorkerLaunchError::NativeProcess)?;
    if identity.owner_uid != effective_uid()
        || identity.boot_identity.as_str() != boot
        || child
            .try_wait()
            .map_err(|_| WorkerLaunchError::NativeProcess)?
            .is_some()
    {
        return Err(WorkerLaunchError::Identity);
    }
    Ok(PendingInvocationWorker {
        child,
        identity,
        ready,
        preparation,
    })
}

fn prepare_frame(
    store: &UnleashStore,
    admission: AdmissionCommit,
) -> Result<(WorkerLaunchPreparation, WorkerBinding, Vec<u8>), WorkerLaunchError> {
    let lease = admission.lease.clone();
    let reservation = admission.reservation_id;
    let (preparation, manifest) = store.claim_invocation_worker(admission)?;
    let digest = preparation.context_manifest_digest;
    let manifest_json = String::from_utf8(manifest).map_err(|_| WorkerLaunchError::Manifest)?;
    let binding = WorkerBinding {
        invocation_id: preparation.invocation_id,
        attempt_id: preparation.attempt_id,
        agent_id: preparation.agent_id,
        session_id: preparation.session_id,
        reservation_id: reservation,
        lease_id: lease.lease_id,
        authority_generation: lease.authority_generation,
        daemon_epoch: lease.daemon_epoch,
        fence: lease.fence,
        context_manifest_id: preparation.context_manifest_id,
        context_manifest_digest: digest,
        scope_id: preparation.scope_id.clone(),
        scope_revision: preparation.scope_revision,
    };
    let bootstrap = WorkerBootstrap {
        wire_version: WorkerWireVersion::V1,
        binding: binding.clone(),
        manifest_json,
    };
    bootstrap
        .verify()
        .map_err(|_| WorkerLaunchError::Manifest)?;
    let payload = serde_json::to_vec(&bootstrap).map_err(|_| WorkerLaunchError::Manifest)?;
    let mut frame = Vec::new();
    write_frame(&mut frame, &payload).map_err(|_| WorkerLaunchError::Manifest)?;
    Ok((preparation, binding, frame))
}

async fn exchange_ready(child: &mut Child, frame: &[u8]) -> Result<WorkerReply, WorkerLaunchError> {
    let stdin = child.stdin.as_mut().ok_or(WorkerLaunchError::Readiness)?;
    stdin
        .write_all(frame)
        .await
        .map_err(|_| WorkerLaunchError::Readiness)?;
    stdin
        .flush()
        .await
        .map_err(|_| WorkerLaunchError::Readiness)?;
    let stdout = child.stdout.as_mut().ok_or(WorkerLaunchError::Readiness)?;
    let length = stdout
        .read_u32()
        .await
        .map_err(|_| WorkerLaunchError::Readiness)?;
    let length = usize::try_from(length).map_err(|_| WorkerLaunchError::Readiness)?;
    if length > MAX_FRAME_BYTES {
        return Err(WorkerLaunchError::Readiness);
    }
    let mut payload = vec![0; length];
    stdout
        .read_exact(&mut payload)
        .await
        .map_err(|_| WorkerLaunchError::Readiness)?;
    serde_json::from_slice(&payload).map_err(|_| WorkerLaunchError::Readiness)
}
