//! Read-only diagnostic authority used only after a proven corrupt store open.

use crate::auth::{AdministrativeAuthVerifier, AuthVerifierPort, Capability};
use crate::broker_activity::BrokerActivity;
use crate::daemon::{install_admin_proof, remove_owned_admin_proof};
use crate::error::{DaemonError, platform_error_to_ipc_error};
use crate::ipc::{read_raw_frame_timeout, write_json_frame_timeout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use unleash_platform::{AuthorityLock, bind_private_unix_listener, verify_same_uid_peer};
use unleash_protocol::{
    AdminProof, DiagnosticHelloResponse, DiagnosticMode, DiagnosticReason, ErrorCode, HelloRequest,
    IpcError, MAX_FRAME_BYTES, ProtocolVersion, QueryEnvelope, QueryResponse,
    READ_ONLY_RECOVERY_CAPABILITY, ReasonCode, RetryClass, from_json_str_strict,
};
use unleash_store::{StoreConfig, UnleashStore};

const TIMEOUT: Duration = Duration::from_secs(5);

struct PendingSocket {
    lock: Option<AuthorityLock>,
    proof: AdminProof,
    rendezvous_dir: PathBuf,
    socket_path: Option<PathBuf>,
    listener: Option<std::os::unix::net::UnixListener>,
}

impl Drop for PendingSocket {
    fn drop(&mut self) {
        drop(self.listener.take());
        if let Some(socket_path) = self.socket_path.take() {
            let _ = std::fs::remove_file(socket_path);
        }
        remove_owned_admin_proof(&self.rendezvous_dir, &self.proof);
        drop(self.lock.take());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use unleash_platform::{ADMIN_PROOF_FILENAME, SOCKET_FILENAME};
    use unleash_testkit::local_test_dir;

    #[tokio::test]
    async fn cancelled_start_cleans_socket_and_proof_before_releasing_lock() {
        let data = local_test_dir("m1-diagnostic-cancel-data").unwrap();
        let rdv = local_test_dir("m1-diagnostic-cancel-rdv").unwrap();
        std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let data_path = data.path().to_path_buf();
        let rdv_path = rdv.path().to_path_buf();
        let lock = AuthorityLock::acquire_in(&rdv_path, &data_path).unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let prepare_dir = rdv_path.clone();
        let task = tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let pending = prepare_diagnostic(lock, prepare_dir).unwrap();
                let _ = ready_tx.send(());
                release_rx.recv().unwrap();
                pending
            })
            .await;
        });
        ready_rx.await.unwrap();
        assert!(rdv_path.join(SOCKET_FILENAME).exists());
        assert!(rdv_path.join(ADMIN_PROOF_FILENAME).exists());
        task.abort();
        let _ = task.await;
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(lock) = AuthorityLock::acquire_in(&rdv_path, &data_path) {
                    assert!(!rdv_path.join(SOCKET_FILENAME).exists());
                    assert!(!rdv_path.join(ADMIN_PROOF_FILENAME).exists());
                    drop(lock);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn shutdown_waits_for_cancelled_blocking_reader() {
        let reads = BrokerActivity::new();
        let guard = reads.admit().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let reader = tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                let _ = started_tx.send(());
                release_rx.recv().unwrap();
            })
            .await;
        });
        started_rx.await.unwrap();
        reader.abort();
        let _ = reader.await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), reads.close_and_wait())
                .await
                .is_err()
        );
        assert!(reads.admit().is_none());
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), reads.close_and_wait())
            .await
            .unwrap();
    }
}

fn prepare_diagnostic(
    lock: AuthorityLock,
    rendezvous_dir: PathBuf,
) -> Result<PendingSocket, DaemonError> {
    let proof = install_admin_proof(&rendezvous_dir)?;
    let mut pending = PendingSocket {
        lock: Some(lock),
        proof,
        rendezvous_dir,
        socket_path: None,
        listener: None,
    };
    let lock = pending
        .lock
        .as_ref()
        .ok_or_else(|| DaemonError::InvalidConfiguration("Verrou absent".into()))?;
    lock.remove_stale_socket()?;
    let socket_path = lock.socket_path().to_path_buf();
    let listener = bind_private_unix_listener(&socket_path)?;
    pending.socket_path = Some(socket_path);
    listener.set_nonblocking(true)?;
    pending.listener = Some(listener);
    Ok(pending)
}

/// A diagnostic authority without a durable lineage or event stream.
#[derive(Debug)]
pub struct DiagnosticHandle {
    socket_path: PathBuf,
    stop: watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
}

impl DiagnosticHandle {
    /// Socket of the read-only diagnostic authority.
    #[must_use]
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Stop the diagnostic authority without opening the store for writing.
    pub async fn shutdown_graceful(&self) {
        self.stop.send_replace(true);
        self.wait_stopped().await;
    }

    /// Wait until the socket and authority lock have been released.
    pub async fn wait_stopped(&self) {
        let mut stopped = self.stopped.clone();
        while !*stopped.borrow_and_update() {
            if stopped.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for DiagnosticHandle {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

pub(crate) async fn start(
    lock: AuthorityLock,
    rendezvous_dir: PathBuf,
    store_config: StoreConfig,
    inner_verifier: Arc<dyn AuthVerifierPort>,
) -> Result<DiagnosticHandle, DaemonError> {
    let proof_dir = rendezvous_dir.clone();
    let mut pending = tokio::task::spawn_blocking(move || prepare_diagnostic(lock, proof_dir))
        .await
        .map_err(|_| {
            DaemonError::InvalidConfiguration("Création du socket interrompue".into())
        })??;
    let listener = UnixListener::from_std(
        pending
            .listener
            .take()
            .ok_or_else(|| DaemonError::InvalidConfiguration("Socket absent".into()))?,
    )?;
    let socket_path = pending
        .socket_path
        .as_ref()
        .ok_or_else(|| DaemonError::InvalidConfiguration("Chemin socket absent".into()))?
        .clone();
    let (stop, mut stop_rx) = watch::channel(false);
    let (stopped_tx, stopped) = watch::channel(false);
    let verifier: Arc<dyn AuthVerifierPort> = Arc::new(AdministrativeAuthVerifier {
        inner: inner_verifier,
        expected: pending.proof.clone(),
    });
    tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(16));
        let reads = BrokerActivity::new();
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                changed = stop_rx.changed() => { if changed.is_err() || *stop_rx.borrow() { break; } }
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { break; };
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else { continue; };
                    let verifier = Arc::clone(&verifier);
                    let cfg = store_config.clone();
                    let reads = Arc::clone(&reads);
                    clients.spawn(async move { let _permit = permit; serve(stream, verifier, cfg, reads).await; });
                }
                Some(_) = clients.join_next(), if !clients.is_empty() => {}
            }
        }
        clients.abort_all();
        while clients.join_next().await.is_some() {}
        // Une lecture bloquante peut survivre à l'annulation de son client.
        // Conserver le verrou d'autorité jusqu'à sa fin effective.
        reads.close_and_wait().await;
        drop(listener);
        let _ = tokio::task::spawn_blocking(move || drop(pending)).await;
        stopped_tx.send_replace(true);
    });
    Ok(DiagnosticHandle {
        socket_path,
        stop,
        stopped,
    })
}

fn corrupt() -> IpcError {
    IpcError::from_reason(
        ReasonCode::StorageCorrupt,
        "Stockage corrompu ; diagnostic en lecture seule",
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "Handshake et service diagnostic bornés sur un unique socket"
)]
async fn serve(
    mut stream: UnixStream,
    verifier: Arc<dyn AuthVerifierPort>,
    config: StoreConfig,
    reads: Arc<BrokerActivity>,
) {
    let peer = match verify_same_uid_peer(&stream) {
        Ok(peer) => peer,
        Err(error) => {
            let _ = write_json_frame_timeout(
                &mut stream,
                &platform_error_to_ipc_error(&error),
                TIMEOUT,
            )
            .await;
            return;
        }
    };
    let raw = match read_raw_frame_timeout(&mut stream, MAX_FRAME_BYTES, TIMEOUT).await {
        Ok(Some(raw)) => zeroize::Zeroizing::new(raw),
        _ => return,
    };
    let Some(hello) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|s| from_json_str_strict::<HelloRequest>(s).ok())
    else {
        let _ = write_json_frame_timeout(
            &mut stream,
            &IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Handshake Hello invalide",
                RetryClass::Never,
            ),
            TIMEOUT,
        )
        .await;
        return;
    };
    let session = match verifier.authenticate_peer(peer, &hello) {
        Ok(session) => session,
        Err(error) => {
            let _ = write_json_frame_timeout(&mut stream, &error, TIMEOUT).await;
            return;
        }
    };
    if !hello
        .capabilities
        .iter()
        .any(|cap| cap == READ_ONLY_RECOVERY_CAPABILITY)
    {
        let _ = write_json_frame_timeout(&mut stream, &corrupt(), TIMEOUT).await;
        return;
    }
    let mut capabilities = Vec::new();
    if session
        .capabilities
        .contains(&Capability::QueryDaemonDiagnostic)
    {
        capabilities.push("query_daemon_diagnostic".to_owned());
    }
    if session.capabilities.contains(&Capability::Administer)
        && session.capabilities.contains(&Capability::QueryState)
        && session.allowed_projects.is_none()
    {
        capabilities.push("inspect_recovery".to_owned());
    }
    let response = DiagnosticHelloResponse {
        protocol_version: ProtocolVersion,
        mode: DiagnosticMode::ReadOnlyRecovery,
        reason_code: DiagnosticReason::StorageCorrupt,
        max_frame_bytes: u32::try_from(MAX_FRAME_BYTES).unwrap_or(u32::MAX),
        capabilities,
    };
    if write_json_frame_timeout(&mut stream, &response, TIMEOUT)
        .await
        .is_err()
    {
        return;
    }
    while let Ok(Some(raw)) = read_raw_frame_timeout(&mut stream, MAX_FRAME_BYTES, TIMEOUT).await {
        let query = std::str::from_utf8(&raw)
            .ok()
            .and_then(|s| from_json_str_strict::<QueryEnvelope>(s).ok());
        let Some(query) = query else {
            let _ = write_json_frame_timeout(&mut stream, &corrupt(), TIMEOUT).await;
            continue;
        };
        if matches!(
            query.query_type.as_str(),
            "GetDaemonStatus" | "InspectRecovery"
        ) && (query.project_id.is_some() || !query.payload.is_empty())
        {
            let error = IpcError::new(
                ErrorCode::InvalidArgument,
                None,
                "Paramètres de diagnostic invalides",
                RetryClass::Never,
            );
            if write_json_frame_timeout(&mut stream, &error, TIMEOUT)
                .await
                .is_err()
            {
                return;
            }
            continue;
        }
        if query.query_type == "GetDaemonStatus"
            && query.project_id.is_none()
            && query.payload.is_empty()
            && session
                .capabilities
                .contains(&Capability::QueryDaemonDiagnostic)
        {
            let response = QueryResponse {
                request_id: query.request_id,
                watermark: None,
                result: serde_json::json!({"mode":"read_only_recovery", "storage_corrupt":true}),
            };
            if write_json_frame_timeout(&mut stream, &response, TIMEOUT)
                .await
                .is_err()
            {
                return;
            }
        } else if query.query_type == "InspectRecovery"
            && (!session.capabilities.contains(&Capability::Administer)
                || !session.capabilities.contains(&Capability::QueryState)
                || session.allowed_projects.is_some())
        {
            let error = IpcError::from_reason(
                ReasonCode::OperationNotAuthorized,
                "Preuve administrative globale requise",
            );
            if write_json_frame_timeout(&mut stream, &error, TIMEOUT)
                .await
                .is_err()
            {
                return;
            }
        } else if query.query_type == "InspectRecovery" {
            let cfg = config.clone();
            let Some(read_guard) = reads.admit() else {
                return;
            };
            let report = tokio::task::spawn_blocking(move || {
                let _read_guard = read_guard;
                UnleashStore::inspect_recovery_offline(&cfg)
            })
            .await;
            match report {
                Ok(Ok(report))
                    if report.storage_corrupt
                        && report.active_authority_generation.is_none()
                        && report.active_stream_id.is_none() =>
                {
                    let response = QueryResponse {
                        request_id: query.request_id,
                        watermark: None,
                        result: serde_json::json!({"report": report, "inventory_coverage": "unknown"}),
                    };
                    if write_json_frame_timeout(&mut stream, &response, TIMEOUT)
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                _ => {
                    let error = IpcError::new(
                        ErrorCode::InternalFailure,
                        None,
                        "Inspection hors ligne indisponible ou non qualifiée",
                        RetryClass::AfterReconciliation,
                    );
                    if write_json_frame_timeout(&mut stream, &error, TIMEOUT)
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        } else if write_json_frame_timeout(&mut stream, &corrupt(), TIMEOUT)
            .await
            .is_err()
        {
            return;
        }
    }
}
