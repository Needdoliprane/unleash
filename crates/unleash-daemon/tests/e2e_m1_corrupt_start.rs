// T-F018-010 / AC-F018-010 / REQ-REC-010; REQ-CORE-001
//! Real daemon startup against synthetic corrupt storage. No user data, API or secrets.
//! Readiness is checked without prescribing the pending diagnostic Hello wire contract.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Assertions and exclusively owned local fixture processes"
)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use unleash_domain::{
    AuthorityGeneration, BackupId, CommandId, ReceiptId, RestoreId, Sha256Digest, TimestampMs,
};
use unleash_platform::{
    AuthorityLock, PlatformError, SOCKET_FILENAME, ensure_private_rendezvous_dir,
    validate_socket_file, write_private_file_atomic,
};
use unleash_store::{MaintenanceRestoreManifest, RestoreRequestState, StoreConfig};
use unleash_testkit::local_test_dir;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

struct FixtureDaemon {
    child: Option<Child>,
    ready: mpsc::Receiver<std::io::Result<String>>,
    stderr: mpsc::Receiver<std::io::Result<String>>,
}

impl FixtureDaemon {
    fn spawn(data: &Path, rendezvous: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
            .arg("--data-root")
            .arg(data)
            .arg("--rendezvous-dir")
            .arg(rendezvous)
            .args(["--foreground", "--ready-json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let (stderr_tx, stderr_rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = ready_tx.send(result);
        });
        std::thread::spawn(move || {
            let mut text = String::new();
            let result = stderr.read_to_string(&mut text).map(|_| text);
            let _ = stderr_tx.send(result);
        });
        Self {
            child: Some(child),
            ready,
            stderr: stderr_rx,
        }
    }

    fn readiness_line(&self) -> String {
        self.ready
            .recv_timeout(PROCESS_TIMEOUT)
            .expect("daemon did not report readiness or close stdout before the deadline")
            .unwrap()
    }

    fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child.take();
                return status;
            }
            assert!(Instant::now() < deadline, "fixture daemon did not exit");
            std::thread::yield_now();
        }
    }

    fn stderr_after_exit(&self) -> String {
        self.stderr
            .recv_timeout(PROCESS_TIMEOUT)
            .expect("fixture stderr reader did not finish")
            .unwrap()
    }

    fn stop_clean(&mut self) {
        let child = self.child.as_mut().unwrap();
        assert!(
            child.try_wait().unwrap().is_none(),
            "recovery daemon exited unexpectedly"
        );
        // This PID belongs to our unreaped Child, so it cannot have been reused.
        let signal = Command::new("/bin/kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status()
            .unwrap();
        assert!(signal.success(), "SIGTERM of the owned fixture failed");
        let status = self.wait_exit();
        let stderr = self.stderr_after_exit();
        assert!(
            status.success(),
            "recovery daemon shutdown failed: {status}; stderr={stderr}"
        );
    }
}

impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn assert_preserved(files: &[(PathBuf, Vec<u8>)], diagnostic: &str) {
    for (path, expected) in files {
        let observed = std::fs::read(path).unwrap_or_else(|error| {
            panic!(
                "fixture storage disappeared: {}; {error}; {diagnostic}",
                path.display()
            )
        });
        assert_eq!(
            &observed,
            expected,
            "fixture storage changed: {}; {diagnostic}",
            path.display()
        );
    }
}

#[test]
fn t_f018_010_corrupt_start_serves_recovery_without_replacing_storage() {
    let fixture = local_test_dir("m1-corrupt-start").unwrap();
    let data = fixture.path().join("data");
    let rendezvous = fixture.path().join("rdv");
    ensure_private_rendezvous_dir(&data).unwrap();
    ensure_private_rendezvous_dir(&rendezvous).unwrap();
    let config = StoreConfig::new(&data);
    let socket = rendezvous.join(SOCKET_FILENAME);
    let maintenance = MaintenanceRestoreManifest {
        restore_id: RestoreId::from_u128(0xF018_1001),
        command_receipt_id: ReceiptId::from_u128(0xF018_1002),
        actor_id: "fixture:offline-recovery".into(),
        command_id: CommandId::from_u128(0xF018_1003),
        command_digest: Sha256Digest::from_bytes(b"synthetic blocked restore metadata"),
        backup_id: BackupId::from_u128(0xF018_1004),
        source_authority_generation: AuthorityGeneration::from_u128(0xF018_1005),
        state: RestoreRequestState::Blocked,
        blocked_reason: Some("RESTORATION_BLOCKED: synthetic fixture inventory".into()),
        updated_at_ms: TimestampMs::new(42),
        history: Vec::new(),
    };
    let preserved = [
        (config.db_path.clone(), vec![0xa7; 4096]),
        (
            config.db_path.with_extension("db-wal"),
            b"synthetic WAL bytes that must not be discarded".repeat(16),
        ),
        (
            config.maintenance_dir.join("restore-manifest.json"),
            serde_json::to_vec(&maintenance).unwrap(),
        ),
    ];
    for (path, bytes) in &preserved {
        write_private_file_atomic(path, bytes).unwrap();
    }

    let mut daemon = FixtureDaemon::spawn(&data, &rendezvous);
    let line = daemon.readiness_line();
    if line.trim().is_empty() {
        let status = daemon.wait_exit();
        let stderr = daemon.stderr_after_exit();
        assert_preserved(&preserved, &stderr);
        assert!(
            !socket.exists(),
            "startup failed after exposing a socket: {stderr}"
        );
        panic!(
            "T-F018-010: daemon exited before recovery readiness; DB/WAL/maintenance preserved byte-for-byte; status={status}; stderr={stderr}"
        );
    }
    let ready: Value = serde_json::from_str(&line).expect("daemon readiness must be JSON");
    let mode = ready.get("mode").or_else(|| ready.get("status"));
    assert_eq!(mode.and_then(Value::as_str), Some("read_only_recovery"));
    // The manifest knows only a historical source. It cannot attest an active
    // authority, epoch or event stream when SQLite is unreadable.
    for field in ["authority_generation", "daemon_epoch", "stream_id"] {
        assert!(
            ready.get(field).is_none_or(Value::is_null),
            "recovery readiness invented an active {field}: {ready}"
        );
    }
    validate_socket_file(&socket).unwrap();
    assert!(matches!(
        AuthorityLock::acquire_in(&rendezvous, &data),
        Err(PlatformError::DaemonAlreadyRunning { .. })
    ));
    assert_preserved(&preserved, "after diagnostic readiness");

    daemon.stop_clean();
    assert!(
        !socket.exists(),
        "socket survived clean diagnostic shutdown"
    );
    assert_preserved(&preserved, "after clean diagnostic shutdown");
}
