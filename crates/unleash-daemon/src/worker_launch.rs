//! Preparation of an admitted worker, held on its private channel.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use unleash_agent_worker::{
    WorkerBinding, WorkerBootstrap, WorkerPermit, WorkerReply, WorkerReplyKind, WorkerWireVersion,
};
use unleash_platform::{
    OsProcessIdentity, ProcessVerification, effective_uid, inspect_process_identity,
    verify_process_identity,
};
use unleash_protocol::{EventCursor, MAX_FRAME_BYTES, write_frame};
use unleash_store::{
    AdmissionCommit, StoreError, UnleashStore, WorkerLaunchPreparation, WorkerProcessObservation,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Preparation could not establish an owned, waiting worker.
#[derive(Debug, thiserror::Error)]
pub enum WorkerLaunchError {
    /// The daemon has closed worker admission for shutdown.
    #[error("daemon is draining invocation workers")]
    Draining,
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
    /// The owned child's exit could not be confirmed.
    #[error("owned worker exit unconfirmed")]
    ProcessExitUnconfirmed,
    /// Start was committed, but permit consumption was not established.
    #[error("worker start committed with unconfirmed continuation")]
    ContinuationUnknown {
        /// Exact durable start event to reconcile.
        start_cursor: EventCursor,
    },
    /// The start committed and recording reconciliation itself failed.
    #[error("worker start committed but reconciliation could not be recorded")]
    ContinuationNotRecorded {
        /// Exact durable start event to reconcile after the writer is available.
        start_cursor: EventCursor,
    },
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

    /// Engage the running transition and send the permit to this owned child.
    ///
    /// # Errors
    /// Returns an error if the transition or permit cannot be established.
    pub async fn activate(
        self,
        store: Arc<UnleashStore>,
    ) -> Result<RunningInvocationWorker, WorkerLaunchError> {
        self.activate_until_shutdown(store, std::future::pending())
            .await
    }

    /// Commit the start, then observe controller shutdown during permit exchange.
    /// A ready shutdown signal is polled before the permit exchange.
    /// On an interrupted exchange the reservation stays held for reconciliation.
    ///
    /// # Errors
    /// Returns the exact durable start cursor if the continuation was not established.
    pub async fn activate_until_shutdown(
        mut self,
        store: Arc<UnleashStore>,
        shutdown: impl Future<Output = ()>,
    ) -> Result<RunningInvocationWorker, WorkerLaunchError> {
        let permit = match encode_frame(&WorkerPermit {
            wire_version: WorkerWireVersion::V1,
            binding: self.ready.binding.clone(),
        }) {
            Ok(permit) => permit,
            Err(error) => return Err(self.close_and_report_failure(error).await),
        };
        let identity = self.identity.clone();
        let preparation = self.preparation.clone();
        let observation = self.process_observation();
        let start_observation = observation.clone();
        let start_store = Arc::clone(&store);
        let start = tokio::task::spawn_blocking(move || {
            if !matches!(
                verify_process_identity(&identity),
                Ok(ProcessVerification::Match)
            ) {
                return Err(WorkerLaunchError::Identity);
            }
            Ok(start_store.commit_invocation_worker_running(preparation, start_observation)?)
        })
        .await
        .unwrap_or(Err(WorkerLaunchError::NativeProcess));
        let start_cursor = match start {
            Ok(cursor) => cursor,
            Err(error) => return Err(self.close_and_report_failure(error).await),
        };

        let reply = if matches!(self.child.try_wait(), Ok(None)) {
            tokio::select! {
                biased;
                () = shutdown => None,
                result = tokio::time::timeout(STARTUP_TIMEOUT, exchange_worker(&mut self.child, &permit)) => {
                    result.ok().and_then(Result::ok)
                }
            }
        } else {
            None
        };
        let Some(acknowledgement) = reply.filter(|reply| {
            reply.kind == WorkerReplyKind::Activated
                && reply.wire_version == self.ready.wire_version
                && reply.pid == self.ready.pid
                && reply.binding == self.ready.binding
        }) else {
            let preparation = self.preparation.clone();
            let process_exit_observed = self.close_after_failed_activation().await;
            let written = tokio::task::spawn_blocking(move || {
                let result = store.record_invocation_worker_continuation_lost(
                    preparation, start_cursor, observation, process_exit_observed,
                );
                if let Err(error) = &result {
                    tracing::warn!(code=%error.error_code(), "Recording worker reconciliation failed");
                }
                result
            }).await;
            if !matches!(written, Ok(Ok(_))) {
                return Err(WorkerLaunchError::ContinuationNotRecorded { start_cursor });
            }
            if !process_exit_observed {
                return Err(WorkerLaunchError::ProcessExitUnconfirmed);
            }
            return Err(WorkerLaunchError::ContinuationUnknown { start_cursor });
        };
        Ok(RunningInvocationWorker {
            pending: self,
            acknowledgement,
            start_cursor,
            store: Arc::downgrade(&store),
        })
    }

    fn process_observation(&self) -> WorkerProcessObservation {
        WorkerProcessObservation {
            pid: self.identity.pid,
            owner_uid: self.identity.owner_uid,
            boot_identity: self.identity.boot_identity.as_str().to_owned(),
            start_identity: self.identity.start_identity.as_str().to_owned(),
        }
    }

    async fn close_after_failed_activation(mut self) -> bool {
        match self.reap().await {
            Ok(status) => {
                if !status.success() {
                    tracing::warn!(%status, "Worker exited abnormally during activation");
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, "Worker activation cleanup did not finish normally");
                false
            }
        }
    }

    async fn close_and_report_failure(self, error: WorkerLaunchError) -> WorkerLaunchError {
        if self.close_after_failed_activation().await {
            error
        } else {
            WorkerLaunchError::ProcessExitUnconfirmed
        }
    }

    /// Close the controller and reap the still-gated worker.
    ///
    /// # Errors
    /// Returns an error if the exit cannot be established.
    pub async fn close(mut self) -> Result<(), WorkerLaunchError> {
        if self.reap().await?.success() {
            Ok(())
        } else {
            Err(WorkerLaunchError::NativeProcess)
        }
    }

    async fn reap(&mut self) -> Result<std::process::ExitStatus, WorkerLaunchError> {
        reap_child(&mut self.child).await
    }
}

/// The owned child has acknowledged its permit after a durable start.
pub struct RunningInvocationWorker {
    pending: PendingInvocationWorker,
    acknowledgement: WorkerReply,
    start_cursor: EventCursor,
    store: Weak<UnleashStore>,
}

impl RunningInvocationWorker {
    /// Reply from the exact child on the private channel.
    pub const fn acknowledgement(&self) -> &WorkerReply {
        &self.acknowledgement
    }

    /// Durable start event, before the permit was sent.
    pub const fn start_cursor(&self) -> EventCursor {
        self.start_cursor
    }

    /// Close the current worker's controller and journal reconciliation for the
    /// exact running attempt. The lease and capacity remain held.
    ///
    /// # Errors
    /// Returns an error if the exit or the reconciliation write cannot be established.
    pub async fn close(self) -> Result<(), WorkerLaunchError> {
        let Self {
            mut pending,
            start_cursor,
            store,
            ..
        } = self;
        let identity = pending.process_observation();
        let exit = pending.reap().await;
        let preparation = pending.preparation;
        drop(pending.child);

        let store = store
            .upgrade()
            .ok_or(WorkerLaunchError::ContinuationNotRecorded { start_cursor })?;
        let process_exit_observed = exit.is_ok();
        let written = tokio::task::spawn_blocking(move || {
            let result = store.record_invocation_worker_channel_closed(
                preparation,
                start_cursor,
                identity,
                process_exit_observed,
            );
            if let Err(error) = &result {
                tracing::warn!(code=%error.error_code(), "Recording closed worker channel failed");
            }
            result
        })
        .await;
        if !matches!(written, Ok(Ok(_))) {
            return Err(WorkerLaunchError::ContinuationNotRecorded { start_cursor });
        }
        if exit?.success() {
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
    let (identity, ready) = match observe_ready(&mut child, &frame, &binding, &boot).await {
        Ok(observation) => observation,
        Err(error) => {
            // This is the child we own; close it before releasing the preparation task.
            let _ = child.start_kill();
            return Err(if reap_child(&mut child).await.is_ok() {
                error
            } else {
                WorkerLaunchError::ProcessExitUnconfirmed
            });
        }
    };
    Ok(PendingInvocationWorker {
        child,
        identity,
        ready,
        preparation,
    })
}

async fn observe_ready(
    child: &mut Child,
    frame: &[u8],
    binding: &WorkerBinding,
    boot: &str,
) -> Result<(OsProcessIdentity, WorkerReply), WorkerLaunchError> {
    let pid = child.id().ok_or(WorkerLaunchError::NativeProcess)?;
    let ready = tokio::time::timeout(STARTUP_TIMEOUT, exchange_worker(child, frame))
        .await
        .map_err(|_| WorkerLaunchError::Deadline)??;
    if ready.wire_version != WorkerWireVersion::V1
        || ready.kind != WorkerReplyKind::Ready
        || ready.pid != pid
        || &ready.binding != binding
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
    Ok((identity, ready))
}

async fn reap_child(child: &mut Child) -> Result<std::process::ExitStatus, WorkerLaunchError> {
    drop(child.stdin.take());
    tokio::time::timeout(STARTUP_TIMEOUT, child.wait())
        .await
        .map_err(|_| WorkerLaunchError::Deadline)?
        .map_err(|_| WorkerLaunchError::NativeProcess)
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
    let frame = encode_frame(&bootstrap).map_err(|_| WorkerLaunchError::Manifest)?;
    Ok((preparation, binding, frame))
}

fn encode_frame(message: &impl serde::Serialize) -> Result<Vec<u8>, WorkerLaunchError> {
    let payload = serde_json::to_vec(message).map_err(|_| WorkerLaunchError::Readiness)?;
    let mut frame = Vec::new();
    write_frame(&mut frame, &payload).map_err(|_| WorkerLaunchError::Readiness)?;
    Ok(frame)
}

async fn exchange_worker(
    child: &mut Child,
    frame: &[u8],
) -> Result<WorkerReply, WorkerLaunchError> {
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
