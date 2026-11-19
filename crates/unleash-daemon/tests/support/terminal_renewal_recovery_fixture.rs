//! Owned instrumented daemon/host library subprocesses. These are distinct from
//! the packaged nominal binaries; native server, identity capture and PTY are real.
use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use unleash_process_host::{HostCutpointObserver, HostInputCutpoint, ProcessHostError};
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};

#[path = "terminal_renewal_recovery_scenarios.rs"]
mod scenarios;

const BEFORE: &str =
    "reattach_scenarios::renewal_recovery_fixture::daemon_before_renewal_handshake";
const AFTER: &str = "reattach_scenarios::renewal_recovery_fixture::daemon_after_renewal_handshake";
const HOST: &str = "reattach_scenarios::renewal_recovery_fixture::native_host_fixture";
const READY: &str = "UNLEASH_RENEWAL_RECOVERY_READY:";
const DATA: &str = "UNLEASH_RENEWAL_RECOVERY_DATA";
const RDV: &str = "UNLEASH_RENEWAL_RECOVERY_RDV";
const BOOTSTRAP: &str = "UNLEASH_RENEWAL_RECOVERY_BOOTSTRAP";
const COUNTER: &str = "UNLEASH_RENEWAL_RECOVERY_COUNTER";
const ARM: &str = "UNLEASH_RENEWAL_RECOVERY_ARM";
const BARRIER: &str = "UNLEASH_RENEWAL_RECOVERY_BARRIER";
const HANDSHAKE: &[u8] = b"HANDSHAKE_APPLIED\n";
const DISPATCH: &[u8] = b"DISPATCH_COMMITTED\n";

#[derive(Clone)]
struct Paths {
    counter: PathBuf,
    arm: PathBuf,
    barrier: PathBuf,
}
impl Paths {
    fn from_env() -> Self {
        Self {
            counter: PathBuf::from(std::env::var_os(COUNTER).unwrap()),
            arm: PathBuf::from(std::env::var_os(ARM).unwrap()),
            barrier: PathBuf::from(std::env::var_os(BARRIER).unwrap()),
        }
    }
    fn armed(&self) -> bool {
        self.arm.try_exists().unwrap()
    }
}

fn barrier(path: &Path, message: &[u8]) -> std::io::Result<()> {
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    stream.write_all(message)?;
    stream.flush()?;
    let mut ack = [0];
    stream.read_exact(&mut ack)?;
    if ack != [1] {
        return Err(std::io::Error::other(
            "owned barrier acknowledgement differs",
        ));
    }
    Ok(())
}

struct BeforeNativeHandshake(Paths);
impl CutpointObserver for BeforeNativeHandshake {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C4AfterBrokerDispatchCommit && self.0.armed() {
            barrier(&self.0.barrier, DISPATCH).map_err(|_| StoreError::CutpointInjected {
                cutpoint: "C4AfterBrokerDispatchCommit".into(),
                message: "owned C4 barrier closed".into(),
            })?;
        }
        Ok(())
    }
}

struct NativeCounter {
    paths: Paths,
    count: Mutex<usize>,
}
impl HostCutpointObserver for NativeCounter {
    fn hit(&self, point: HostInputCutpoint) -> Result<(), ProcessHostError> {
        if point != HostInputCutpoint::HandshakeAppliedBeforeAck {
            return Ok(());
        }
        {
            let mut count = self.count.lock().unwrap();
            *count = count.checked_add(1).unwrap();
            assert!(*count <= 1024, "bounded owned handshake counter");
            // Actual callback count, flushed by the host subprocess itself.
            // The counter and controller survive SIGKILL of the daemon owner.
            unleash_platform::write_private_file_atomic(
                &self.paths.counter,
                &HANDSHAKE.repeat(*count),
            )?;
        }
        if self.paths.armed() {
            barrier(&self.paths.barrier, HANDSHAKE)?;
        }
        Ok(())
    }
}

struct InstrumentedHostLauncher(Paths);
impl unleash_daemon::terminal_host_launcher::TerminalHostLauncher for InstrumentedHostLauncher {
    fn spawn(&self, _packaged: &Path, bootstrap: &Path) -> std::io::Result<Child> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--ignored", "--exact", HOST, "--nocapture"])
            .env_clear()
            .env(BOOTSTRAP, bootstrap)
            .env(COUNTER, &self.0.counter)
            .env(ARM, &self.0.arm)
            .env(BARRIER, &self.0.barrier)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        // Same native session boundary as the packaged host, without another dependency.
        unleash_process_host::configure_setsid_pre_exec(&mut command);
        command.spawn()
    }
}

#[test]
#[ignore = "owned host-library crash fixture selected only by exact argv"]
fn native_host_fixture() {
    let path = PathBuf::from(std::env::var_os(BOOTSTRAP).unwrap());
    unleash_platform::validate_rendezvous_dir(path.parent().unwrap()).unwrap();
    let raw = unleash_platform::read_private_file_to_string(&path)
        .unwrap()
        .unwrap();
    let spec: unleash_process_host::HostBootstrapSpec = serde_json::from_str(&raw).unwrap();
    let observer = NativeCounter {
        paths: Paths::from_env(),
        count: Mutex::new(0),
    };
    let server = unleash_process_host::ProcessHostServer::start(
        &spec,
        &unleash_process_host::OsIsolationBackendProbe::new(),
        Box::new(unleash_process_host::OsProcessInspector),
        Some(Arc::new(observer)),
    )
    .unwrap();
    server.serve_until_shutdown().unwrap();
}

async fn daemon_fixture(before: bool) {
    let paths = Paths::from_env();
    let config = unleash_daemon::DaemonConfig {
        data_root: Some(PathBuf::from(std::env::var_os(DATA).unwrap())),
        rendezvous_dir: Some(PathBuf::from(std::env::var_os(RDV).unwrap())),
        terminal_host_launcher: Arc::new(InstrumentedHostLauncher(paths.clone())),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: before
                .then(|| Arc::new(BeforeNativeHandshake(paths)) as Arc<dyn CutpointObserver>),
            ..unleash_daemon::StoreOverrides::default()
        },
        ..unleash_daemon::DaemonConfig::default()
    };
    let unleash_daemon::DaemonStartOutcome::Started(handle) =
        unleash_daemon::UnleashDaemon::start(config).await.unwrap()
    else {
        unreachable!()
    };
    handle.wait_ready().await.unwrap();
    writeln!(
        std::io::stdout().lock(),
        "{READY}{}",
        json!({"status":"ready"})
    )
    .unwrap();
    std::io::stdout().flush().unwrap();
    let _ = handle.wait_stopped().await;
}
#[tokio::test]
#[ignore = "owned daemon-library C4 fixture selected only by exact argv"]
async fn daemon_before_renewal_handshake() {
    daemon_fixture(true).await;
}
#[tokio::test]
#[ignore = "owned daemon-library lost native ACK fixture selected only by exact argv"]
async fn daemon_after_renewal_handshake() {
    daemon_fixture(false).await;
}

async fn receive(listener: &tokio::net::UnixListener, expected: &[u8]) -> tokio::net::UnixStream {
    let (mut stream, _) = tokio::time::timeout(DEADLINE, listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stream.peer_cred().unwrap().uid(),
        unleash_platform::effective_uid()
    );
    let mut message = vec![0; expected.len()];
    tokio::time::timeout(DEADLINE, stream.read_exact(&mut message))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message, expected);
    stream
}

fn count(paths: &Paths) -> usize {
    let bytes = std::fs::read(&paths.counter).unwrap();
    assert_eq!(bytes.len() % HANDSHAKE.len(), 0);
    assert!(
        bytes
            .chunks_exact(HANDSHAKE.len())
            .all(|chunk| chunk == HANDSHAKE)
    );
    bytes.len() / HANDSHAKE.len()
}
