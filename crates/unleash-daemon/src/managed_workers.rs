//! Owns invocation workers admitted by the daemon until their controller is closed.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, oneshot, watch};
use tokio::task::{JoinError, JoinSet};
use unleash_domain::PoolId;
use unleash_platform::OsProcessIdentity;
use unleash_protocol::EventCursor;
use unleash_store::{AdmissionCommit, AdmissionOutcome, AdmissionSnapshotToken, UnleashStore};

use crate::execution_scheduler::{ExecutionSchedulingInputs, schedule_invocation_execution};
use crate::worker_launch::{InvocationWorkerStep, WorkerLaunchError, prepare_invocation_worker};

#[derive(Default)]
pub(crate) struct ManagedWorkers(Mutex<WorkerTasks>);

#[derive(Default)]
struct WorkerTasks {
    closed: bool,
    cleanup_unconfirmed: bool,
    tasks: JoinSet<Result<(), WorkerLaunchError>>,
}

impl WorkerTasks {
    fn record(&mut self, result: Result<Result<(), WorkerLaunchError>, JoinError>) {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.cleanup_unconfirmed = true;
                tracing::warn!(%error, "Failed to close a daemon-owned worker");
            }
            Err(error) => {
                self.cleanup_unconfirmed = true;
                tracing::warn!(%error, "Daemon-owned worker task did not finish normally");
            }
        }
    }
}

type Started = (EventCursor, OsProcessIdentity);

impl ManagedWorkers {
    pub(crate) async fn start(
        &self,
        store: Arc<UnleashStore>,
        admission: AdmissionCommit,
        executable: &Path,
        draining: watch::Receiver<bool>,
    ) -> Result<Started, WorkerLaunchError> {
        let gate = draining.clone();
        let executable = executable.to_path_buf();
        self.track(&gate, move |reply| {
            run_worker(store, admission, executable, draining, move |start| {
                let _ = reply.send(start);
            })
        })
        .await?
    }

    pub(crate) async fn schedule(
        &self,
        store: Arc<UnleashStore>,
        pool: PoolId,
        token: AdmissionSnapshotToken,
        inputs: ExecutionSchedulingInputs,
        executable: &Path,
        draining: watch::Receiver<bool>,
    ) -> Result<InvocationWorkerStep, WorkerLaunchError> {
        let gate = draining.clone();
        let executable = executable.to_path_buf();
        self.track(&gate, move |reply| async move {
            if *draining.borrow() {
                let _ = reply.send(Err(WorkerLaunchError::Draining));
                return Ok(());
            }
            let scheduler_store = Arc::clone(&store);
            let scheduled = tokio::task::spawn_blocking(move || {
                schedule_invocation_execution(&scheduler_store, pool, token, inputs)
            })
            .await;
            let admission = match scheduled {
                Ok(Ok((AdmissionOutcome::Admitted(admission), _))) => admission,
                Ok(Ok((AdmissionOutcome::Unchanged, decision))) => {
                    let _ = reply.send(Ok(InvocationWorkerStep::Deferred(decision)));
                    return Ok(());
                }
                Ok(Err(error)) => {
                    let _ = reply.send(Err(error.into()));
                    return Ok(());
                }
                Err(_) => {
                    let _ = reply.send(Err(WorkerLaunchError::NativeProcess));
                    return Ok(());
                }
            };
            run_worker(
                store,
                (*admission).clone(),
                executable,
                draining,
                move |start| {
                    let _ = reply.send(Ok(InvocationWorkerStep::Admitted { admission, start }));
                },
            )
            .await
        })
        .await?
    }

    async fn track<T, F>(
        &self,
        draining: &watch::Receiver<bool>,
        task: impl FnOnce(oneshot::Sender<T>) -> F,
    ) -> Result<T, WorkerLaunchError>
    where
        F: Future<Output = Result<(), WorkerLaunchError>> + Send + 'static,
        T: Send + 'static,
    {
        let mut workers = self.0.lock().await;
        if workers.closed || *draining.borrow() {
            return Err(WorkerLaunchError::Draining);
        }
        while let Some(result) = workers.tasks.try_join_next() {
            workers.record(result);
        }
        if workers.cleanup_unconfirmed {
            return Err(WorkerLaunchError::ProcessExitUnconfirmed);
        }
        let (reply, started) = oneshot::channel();
        workers.tasks.spawn(task(reply));
        drop(workers);
        started.await.map_err(|_| WorkerLaunchError::NativeProcess)
    }

    /// The coordinator can time this out and then continue waiting on the same tasks.
    pub(crate) async fn close_and_wait(&self) -> bool {
        let mut workers = self.0.lock().await;
        workers.closed = true;
        while let Some(result) = workers.tasks.join_next().await {
            workers.record(result);
        }
        !workers.cleanup_unconfirmed
    }
}

async fn run_worker(
    store: Arc<UnleashStore>,
    admission: AdmissionCommit,
    executable: PathBuf,
    mut draining: watch::Receiver<bool>,
    report: impl FnOnce(Result<Started, WorkerLaunchError>),
) -> Result<(), WorkerLaunchError> {
    if *draining.borrow() {
        report(Err(WorkerLaunchError::Draining));
        return Ok(());
    }
    let pending = match prepare_invocation_worker(Arc::clone(&store), admission, &executable).await
    {
        Ok(pending) => pending,
        Err(error) => return report_start_error(report, error),
    };
    if *draining.borrow() {
        report(Err(WorkerLaunchError::Draining));
        return pending.close().await;
    }
    let identity = pending.identity().clone();
    let worker = match pending
        .activate_until_shutdown(Arc::clone(&store), wait_for_drain(&mut draining))
        .await
    {
        Ok(worker) => worker,
        Err(error) => return report_start_error(report, error),
    };
    report(Ok((worker.start_cursor(), identity)));
    wait_for_drain(&mut draining).await;
    let closed = worker.close().await;
    drop(store);
    closed
}

async fn wait_for_drain(draining: &mut watch::Receiver<bool>) {
    let _ = draining.wait_for(|&started| started).await;
}

fn report_start_error(
    report: impl FnOnce(Result<Started, WorkerLaunchError>),
    error: WorkerLaunchError,
) -> Result<(), WorkerLaunchError> {
    let incomplete = match error {
        WorkerLaunchError::ProcessExitUnconfirmed => {
            Some(WorkerLaunchError::ProcessExitUnconfirmed)
        }
        WorkerLaunchError::ContinuationNotRecorded { start_cursor } => {
            Some(WorkerLaunchError::ContinuationNotRecorded { start_cursor })
        }
        _ => None,
    };
    report(Err(error));
    incomplete.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn timing_out_drain_does_not_discard_an_owned_task() {
        let workers = ManagedWorkers::default();
        let (release, wait) = oneshot::channel();
        workers.0.lock().await.tasks.spawn(async move {
            wait.await.unwrap();
            Ok(())
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), workers.close_and_wait())
                .await
                .is_err()
        );
        {
            let state = workers.0.lock().await;
            assert!(state.closed);
            assert_eq!(state.tasks.len(), 1);
        }
        release.send(()).unwrap();
        assert!(workers.close_and_wait().await);
    }

    #[tokio::test]
    async fn unconfirmed_worker_closure_is_retained_on_repeat_drain() {
        let workers = ManagedWorkers::default();
        workers
            .0
            .lock()
            .await
            .tasks
            .spawn(async { Err(WorkerLaunchError::ProcessExitUnconfirmed) });
        assert!(!workers.close_and_wait().await);
        assert!(!workers.close_and_wait().await);
    }
}
