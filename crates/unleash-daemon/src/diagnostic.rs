//! Read-only diagnostic authority used only after a proven corrupt store open.

use crate::auth::{AdministrativeAuthVerifier, AuthVerifierPort, Capability};
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
    DiagnosticHelloResponse, DiagnosticMode, DiagnosticReason, ErrorCode, HelloRequest, IpcError,
    MAX_FRAME_BYTES, ProtocolVersion, QueryEnvelope, QueryResponse, READ_ONLY_RECOVERY_CAPABILITY,
    ReasonCode, RetryClass, from_json_str_strict,
};
use unleash_store::StoreConfig;

const TIMEOUT: Duration = Duration::from_secs(5);

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
    let (lock, listener, socket_path, proof) = tokio::task::spawn_blocking(move || {
        let proof = install_admin_proof(&proof_dir)?;
        let result = (|| -> Result<_, DaemonError> {
            lock.remove_stale_socket()?;
            let socket_path = lock.socket_path().to_path_buf();
            let listener = bind_private_unix_listener(&socket_path)?;
            if let Err(error) = listener.set_nonblocking(true) {
                let _ = std::fs::remove_file(&socket_path);
                return Err(DaemonError::Io(error));
            }
            Ok((listener, socket_path))
        })();
        match result {
            Ok((listener, socket_path)) => Ok((lock, listener, socket_path, proof)),
            Err(error) => {
                remove_owned_admin_proof(&proof_dir, &proof);
                Err(error)
            }
        }
    })
    .await
    .map_err(|_| DaemonError::InvalidConfiguration("Création du socket interrompue".into()))??;
    let listener = match UnixListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            let path = socket_path.clone();
            let dir = rendezvous_dir.clone();
            let proof_for_cleanup = proof.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = std::fs::remove_file(path);
                remove_owned_admin_proof(&dir, &proof_for_cleanup);
                drop(lock);
            })
            .await;
            return Err(DaemonError::Io(error));
        }
    };
    let (stop, mut stop_rx) = watch::channel(false);
    let (stopped_tx, stopped) = watch::channel(false);
    let socket_for_task = socket_path.clone();
    let verifier: Arc<dyn AuthVerifierPort> = Arc::new(AdministrativeAuthVerifier {
        inner: inner_verifier,
        expected: proof.clone(),
    });
    tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(16));
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                changed = stop_rx.changed() => { if changed.is_err() || *stop_rx.borrow() { break; } }
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { break; };
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else { continue; };
                    let verifier = Arc::clone(&verifier);
                    let cfg = store_config.clone();
                    clients.spawn(async move { let _permit = permit; serve(stream, verifier, cfg).await; });
                }
                Some(_) = clients.join_next(), if !clients.is_empty() => {}
            }
        }
        clients.abort_all();
        while clients.join_next().await.is_some() {}
        drop(listener);
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_file(&socket_for_task);
            remove_owned_admin_proof(&rendezvous_dir, &proof);
            drop(lock);
        })
        .await;
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
async fn serve(mut stream: UnixStream, verifier: Arc<dyn AuthVerifierPort>, _config: StoreConfig) {
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
        } else {
            // InspectRecovery remains closed until the offline manifest reader is qualified.
            if write_json_frame_timeout(&mut stream, &corrupt(), TIMEOUT)
                .await
                .is_err()
            {
                return;
            }
        }
    }
}
