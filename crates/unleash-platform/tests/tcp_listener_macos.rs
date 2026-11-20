//! Native macOS listener ownership only. No Ollama, model, external service,
//! connected-peer qualification or confinement proof is exercised here.
#![cfg(target_os = "macos")]

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use unleash_platform::{
    OsProcessIdentity, StartIdentity, TcpListenerError, inspect_process_identity,
    observe_owned_tcp_listener, verify_owned_tcp_listener,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const CHILD_ENV: &str = "UNLEASH_TCP_LISTENER_FIXTURE";
const PREFIX: &str = "listener-fixture:";

struct FixtureChild {
    child: Child,
    input: ChildStdin,
    events: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    endpoint: SocketAddr,
}

impl FixtureChild {
    fn start(ip: IpAddr) -> TestResult<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "listener_fixture_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(CHILD_ENV, "1")
            .env("UNLEASH_TCP_LISTENER_IP", ip.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take().ok_or("child stdin unavailable")?;
        let output = child.stdout.take().ok_or("child stdout unavailable")?;
        let (sender, events) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut fixture = Self {
            child,
            input,
            events,
            reader: Some(reader),
            endpoint: SocketAddr::new(ip, 0),
        };
        fixture.endpoint = fixture.event("ready")?.parse()?;
        assert!(fixture.endpoint.ip().is_loopback());
        assert_ne!(fixture.endpoint.port(), 0);
        Ok(fixture)
    }

    fn event(&self, kind: &str) -> TestResult<String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let prefix = format!("{PREFIX}{kind} ");
        loop {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or("fixture event deadline expired")?;
            let line = self.events.recv_timeout(remaining)?;
            if let Some(value) = line.split_once(&prefix).map(|(_, value)| value) {
                return Ok(value.to_owned());
            }
        }
    }

    fn identity(&self) -> TestResult<OsProcessIdentity> {
        Ok(inspect_process_identity(self.child.id())?)
    }

    fn replace_listener(&mut self) -> TestResult {
        writeln!(self.input, "replace")?;
        self.input.flush()?;
        let endpoint: SocketAddr = self.event("replaced")?.parse()?;
        assert_eq!(endpoint, self.endpoint);
        Ok(())
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        // Only this fixture's direct child is signalled and reaped.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
#[ignore = "subprocess fixture entry; invoked explicitly by native owner tests"]
fn listener_fixture_child() -> TestResult {
    if std::env::var(CHILD_ENV).as_deref() != Ok("1") {
        return Err("fixture entry requires its parent's explicit environment".into());
    }
    let ip: IpAddr = std::env::var("UNLEASH_TCP_LISTENER_IP")?.parse()?;
    if !ip.is_loopback() {
        return Err("fixture may bind only loopback".into());
    }
    let mut listener = Some(TcpListener::bind(SocketAddr::new(ip, 0))?);
    let endpoint = listener.as_ref().ok_or("listener missing")?.local_addr()?;
    writeln!(std::io::stdout().lock(), "{PREFIX}ready {endpoint}")?;
    std::io::stdout().flush()?;
    for command in std::io::stdin().lock().lines() {
        match command?.as_str() {
            "replace" => {
                drop(listener.take());
                listener = Some(TcpListener::bind(endpoint)?);
                writeln!(std::io::stdout().lock(), "{PREFIX}replaced {endpoint}")?;
                std::io::stdout().flush()?;
            }
            _ => return Err("unexpected fixture command".into()),
        }
    }
    drop(listener);
    Ok(())
}

fn observed_listener(ip: IpAddr) -> TestResult {
    let child = FixtureChild::start(ip)?;
    let identity = child.identity()?;
    let first = observe_owned_tcp_listener(&identity, child.endpoint)?;
    assert_eq!(first.process_identity(), &identity);
    assert_eq!(first.endpoint(), child.endpoint);
    assert_ne!(first.socket_generation(), 0);
    verify_owned_tcp_listener(&first)?;
    let second = observe_owned_tcp_listener(&identity, child.endpoint)?;
    assert_eq!(first, second, "a stable listener keeps its native instance");
    Ok(())
}

#[test]
fn owned_loopback_listener_observes_real_ipv4_child() -> TestResult {
    observed_listener(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

#[test]
fn owned_loopback_listener_observes_real_ipv6_child() -> TestResult {
    observed_listener(IpAddr::V6(Ipv6Addr::LOCALHOST))
}

#[test]
fn owned_loopback_listener_rejects_other_child_and_full_identity() -> TestResult {
    let first = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let other = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = first.identity()?;
    assert_ne!(first.endpoint, other.endpoint);
    assert!(matches!(
        observe_owned_tcp_listener(&other.identity()?, first.endpoint),
        Err(TcpListenerError::ListenerNotFound)
    ));
    assert!(matches!(
        observe_owned_tcp_listener(&identity, other.endpoint),
        Err(TcpListenerError::ListenerNotFound)
    ));
    let mut wrong = identity.clone();
    wrong.start_identity = StartIdentity::new("synthetic-other-instance")?;
    assert!(matches!(
        observe_owned_tcp_listener(&wrong, first.endpoint),
        Err(TcpListenerError::IdentityChanged)
    ));
    let mut wrong = identity.clone();
    wrong.owner_uid = wrong.owner_uid.checked_add(1).ok_or("uid overflow")?;
    assert!(matches!(
        observe_owned_tcp_listener(&wrong, first.endpoint),
        Err(TcpListenerError::IdentityChanged)
    ));
    let mut wrong = identity;
    wrong.pgid = wrong.pgid.checked_add(1).ok_or("pgid overflow")?;
    assert!(matches!(
        observe_owned_tcp_listener(&wrong, first.endpoint),
        Err(TcpListenerError::IdentityChanged)
    ));
    Ok(())
}

#[test]
fn owned_loopback_listener_rejects_replacement_at_the_same_port() -> TestResult {
    let mut child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    let original = observe_owned_tcp_listener(&identity, child.endpoint)?;
    child.replace_listener()?;
    assert_eq!(
        child.identity()?,
        identity,
        "the process itself did not restart"
    );
    assert!(matches!(
        verify_owned_tcp_listener(&original),
        Err(TcpListenerError::ListenerChanged)
    ));
    let replacement = observe_owned_tcp_listener(&identity, child.endpoint)?;
    assert_ne!(
        replacement.socket_generation(),
        original.socket_generation()
    );
    verify_owned_tcp_listener(&replacement)?;
    Ok(())
}

#[test]
fn owned_loopback_listener_rejects_unqualified_endpoint_without_io() -> TestResult {
    let child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    for endpoint in [
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), child.endpoint.port()),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), child.endpoint.port()),
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            child.endpoint.port(),
        ),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    ] {
        assert!(matches!(
            observe_owned_tcp_listener(&identity, endpoint),
            Err(TcpListenerError::InvalidEndpoint)
        ));
    }
    Ok(())
}
