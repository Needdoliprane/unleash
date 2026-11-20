//! Instrumented daemon-library subprocess. This proves transaction chronology;
//! the adjacent nominal scenario separately executes the packaged daemon.
use super::*;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncReadExt;
use unleash_store::{CutpointObserver, StoreCutpoint, StoreError};

const ENTRY: &str = "reattach_scenarios::renewal_fixture::terminal_renewal_daemon_fixture";
const READY: &str = "UNLEASH_RENEWAL_FIXTURE_READY:";
const DATA: &str = "UNLEASH_RENEWAL_FIXTURE_DATA";
const RENDEZVOUS: &str = "UNLEASH_RENEWAL_FIXTURE_RENDEZVOUS";
const BARRIER: &str = "UNLEASH_RENEWAL_FIXTURE_BARRIER";
const ARMED: &str = "UNLEASH_RENEWAL_FIXTURE_ARM";
const INTENT_BARRIER: &str = "UNLEASH_RENEWAL_FIXTURE_INTENT_BARRIER";
const SIGNAL: &[u8] = b"RENEWAL_BEFORE_OBSERVATION_COMMIT\n";
const INTENT_SIGNAL: &[u8] = b"RENEWAL_AUTHORIZING_COMMITTED\n";

struct FixtureCutpoints {
    observation_barrier: Option<PathBuf>,
    intent_barrier: Option<PathBuf>,
    armed: Option<PathBuf>,
    fired: AtomicBool,
}

impl FixtureCutpoints {
    fn take_arm(&self) -> bool {
        self.armed.as_ref().is_some_and(|path| path.is_file())
            && !self.fired.swap(true, Ordering::AcqRel)
    }
}

impl CutpointObserver for FixtureCutpoints {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C2AfterCommandCommit
            && self.intent_barrier.is_none()
            && self.take_arm()
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C2AfterCommandCommit".into(),
                message: "owned resize receipt acknowledgement lost".into(),
            });
        }
        if point == StoreCutpoint::C1BeforeTerminalRenewalObservationCommit
            && let Some(barrier) = &self.observation_barrier
        {
            wait_for_release(barrier, SIGNAL);
        }
        if point == StoreCutpoint::C3AfterBrokerIntentCommit
            && let Some(barrier) = &self.intent_barrier
            && self.take_arm()
        {
            wait_for_release(barrier, INTENT_SIGNAL);
        }
        Ok(())
    }
}

fn wait_for_release(barrier: &Path, signal: &[u8]) {
    let mut stream = std::os::unix::net::UnixStream::connect(barrier).unwrap();
    stream.set_read_timeout(Some(DEADLINE)).unwrap();
    stream.set_write_timeout(Some(DEADLINE)).unwrap();
    stream.write_all(signal).unwrap();
    stream.flush().unwrap();
    let mut released = [0];
    stream.read_exact(&mut released).unwrap();
    assert_eq!(released, [1]);
}

struct ExactPackagedHost(PathBuf);

impl unleash_daemon::terminal_host_launcher::TerminalHostLauncher for ExactPackagedHost {
    fn spawn(&self, _sibling: &Path, bootstrap: &Path) -> std::io::Result<Child> {
        unleash_daemon::terminal_host_launcher::TerminalHostLauncher::spawn(
            &unleash_daemon::terminal_host_launcher::PackagedTerminalHostLauncher,
            &self.0,
            bootstrap,
        )
    }
}

#[tokio::test]
#[ignore = "owned daemon-library renewal fixture selected by exact argv only"]
async fn terminal_renewal_daemon_fixture() {
    let data = PathBuf::from(std::env::var_os(DATA).unwrap());
    let rendezvous = PathBuf::from(std::env::var_os(RENDEZVOUS).unwrap());
    let observation_barrier = std::env::var_os(BARRIER).map(PathBuf::from);
    let intent_barrier = std::env::var_os(INTENT_BARRIER).map(PathBuf::from);
    let armed = std::env::var_os(ARMED).map(PathBuf::from);
    assert_ne!(observation_barrier.is_some(), armed.is_some());
    assert!(intent_barrier.is_none() || armed.is_some());
    let host = Path::new(env!("CARGO_BIN_EXE_unleashd"))
        .parent()
        .unwrap()
        .join("unleash-process-host");
    assert!(host.is_file());
    let config = unleash_daemon::DaemonConfig {
        data_root: Some(data),
        rendezvous_dir: Some(rendezvous),
        store_overrides: unleash_daemon::StoreOverrides {
            cutpoints: Some(Arc::new(FixtureCutpoints {
                observation_barrier,
                intent_barrier,
                armed,
                fired: AtomicBool::new(false),
            })),
            ..unleash_daemon::StoreOverrides::default()
        },
        terminal_host_launcher: Arc::new(ExactPackagedHost(host)),
        ..unleash_daemon::DaemonConfig::default()
    };
    let unleash_daemon::DaemonStartOutcome::Started(handle) =
        unleash_daemon::UnleashDaemon::start(config).await.unwrap()
    else {
        panic!("the owned fixture must start its actual authority");
    };
    handle.wait_ready().await.unwrap();
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{READY}{}", json!({"status":"ready"})).unwrap();
    stdout.flush().unwrap();
    drop(stdout);
    let _ = handle.wait_stopped().await;
}

pub(super) async fn start_with_receipt_cutpoint(armed: &Path) -> LiveFixture {
    LiveFixture::empty_with(|data, rdv| {
        FixtureDaemon::spawn_driver(
            ENTRY,
            data,
            &[(DATA, data), (RENDEZVOUS, rdv), (ARMED, armed)],
            READY,
        )
    })
    .await
}

pub(super) async fn wait_before_commit(
    listener: &tokio::net::UnixListener,
) -> tokio::net::UnixStream {
    wait_for_signal(listener, SIGNAL).await
}

pub(super) async fn start_with_intent_cutpoint(armed: &Path, barrier: &Path) -> LiveFixture {
    LiveFixture::empty_with(|data, rdv| {
        FixtureDaemon::spawn_driver(
            ENTRY,
            data,
            &[
                (DATA, data),
                (RENDEZVOUS, rdv),
                (ARMED, armed),
                (INTENT_BARRIER, barrier),
            ],
            READY,
        )
    })
    .await
}

pub(super) async fn wait_after_intent(
    listener: &tokio::net::UnixListener,
) -> tokio::net::UnixStream {
    wait_for_signal(listener, INTENT_SIGNAL).await
}

async fn wait_for_signal(
    listener: &tokio::net::UnixListener,
    signal: &[u8],
) -> tokio::net::UnixStream {
    let (mut stream, _) = tokio::time::timeout(DEADLINE, listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stream.peer_cred().unwrap().uid(),
        unleash_platform::effective_uid()
    );
    let mut actual = vec![0; signal.len()];
    tokio::time::timeout(DEADLINE, stream.read_exact(&mut actual))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, signal);
    stream
}

#[tokio::test]
async fn t_f011_renewal_deadline_waits_for_exact_native_observation_commit() {
    let barrier_root = local_test_dir("renewal-precommit-barrier").unwrap();
    std::fs::set_permissions(barrier_root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let barrier = barrier_root.path().join("barrier.sock");
    let listener = unleash_platform::bind_private_unix_listener(&barrier).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
    let mut fixture = LiveFixture::empty_with(|data, rdv| {
        FixtureDaemon::spawn_driver(
            ENTRY,
            data,
            &[(DATA, data), (RENDEZVOUS, rdv), (BARRIER, &barrier)],
            READY,
        )
    })
    .await;
    fixture.provision(true).await;
    renewal_scenarios::exercise(&mut fixture, Some(&listener)).await;
}
