//! Native connected-FD observations only; no HTTP/TLS, proxy or confinement claim.
#![cfg(target_os = "macos")]

use std::error::Error;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use unleash_platform::{
    OsProcessIdentity, StartIdentity, TcpListenerError, inspect_process_identity,
    observe_owned_tcp_connection, observe_owned_tcp_listener, verify_owned_tcp_connection,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const CHILD_ENV: &str = "UNLEASH_TCP_CONNECTION_FIXTURE";
const PREFIX: &str = "connection-fixture:";
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);

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
                "connection_fixture_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(CHILD_ENV, "1")
            .env("UNLEASH_TCP_CONNECTION_IP", ip.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take().ok_or("child stdin unavailable")?;
        let output = child.stdout.take().ok_or("child stdout unavailable")?;
        let (sender, events) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
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
        assert_eq!(fixture.endpoint.ip(), ip);
        assert_ne!(fixture.endpoint.port(), 0);
        Ok(fixture)
    }

    fn event(&self, kind: &str) -> TestResult<String> {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        let prefix = format!("{PREFIX}{kind} ");
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or("fixture event deadline expired")?;
            let line = self.events.recv_timeout(remaining)?;
            if let Some(value) = line.split_once(&prefix).map(|(_, value)| value) {
                return Ok(value.to_owned());
            }
        }
    }

    fn command(&mut self, command: &str) -> TestResult<String> {
        writeln!(self.input, "{command}")?;
        self.input.flush()?;
        self.event(command)
    }

    fn identity(&self) -> TestResult<OsProcessIdentity> {
        Ok(inspect_process_identity(self.child.id())?)
    }

    fn connect(&self) -> TestResult<TcpStream> {
        Ok(TcpStream::connect_timeout(&self.endpoint, FIXTURE_TIMEOUT)?)
    }

    fn accept(&mut self, client: &TcpStream) -> TestResult {
        let tuple = self.command("accept")?;
        let mut endpoints = tuple.split_whitespace();
        let local: SocketAddr = endpoints.next().ok_or("accepted local missing")?.parse()?;
        let peer: SocketAddr = endpoints.next().ok_or("accepted peer missing")?.parse()?;
        assert_eq!(local, client.peer_addr()?);
        assert_eq!(peer, client.local_addr()?);
        assert!(endpoints.next().is_none());
        Ok(())
    }

    fn assert_quiet(&mut self, accepted: usize) -> TestResult {
        assert_eq!(self.command("inspect")?, format!("{accepted} quiet"));
        Ok(())
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        // Only this fixture's owned direct child is killed, reaped and joined.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn announce(kind: &str, value: impl std::fmt::Display) -> TestResult {
    writeln!(std::io::stdout().lock(), "{PREFIX}{kind} {value}")?;
    std::io::stdout().flush()?;
    Ok(())
}

fn quiet(streams: &[TcpStream]) -> TestResult {
    for stream in streams {
        let mut byte = [0_u8; 1];
        match stream.peek(&mut byte) {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            _ => return Err("application bytes or premature closure detected".into()),
        }
    }
    Ok(())
}

#[test]
#[ignore = "subprocess fixture entry; invoked only by native owner tests"]
fn connection_fixture_child() -> TestResult {
    if std::env::var(CHILD_ENV).as_deref() != Ok("1") {
        return Err("fixture entry requires its parent's explicit environment".into());
    }
    let ip: IpAddr = std::env::var("UNLEASH_TCP_CONNECTION_IP")?.parse()?;
    if !ip.is_loopback() {
        return Err("fixture binds only loopback".into());
    }
    let mut listener = Some(TcpListener::bind(SocketAddr::new(ip, 0))?);
    let endpoint = listener.as_ref().ok_or("listener missing")?.local_addr()?;
    let mut streams = Vec::new();
    announce("ready", endpoint)?;
    for command in std::io::stdin().lock().lines() {
        match command?.as_str() {
            "accept" => {
                let (stream, peer) = listener.as_ref().ok_or("listener missing")?.accept()?;
                let local = stream.local_addr()?;
                stream.set_nonblocking(true)?;
                streams.push(stream);
                announce("accept", format!("{local} {peer}"))?;
            }
            "replace" => {
                drop(listener.take());
                listener = Some(TcpListener::bind(endpoint)?);
                announce("replace", endpoint)?;
            }
            "inspect" => {
                quiet(&streams)?;
                announce("inspect", format!("{} quiet", streams.len()))?;
            }
            _ => return Err("unexpected fixture command".into()),
        }
    }
    Ok(())
}

fn observed_connection(ip: IpAddr) -> TestResult {
    let mut child = FixtureChild::start(ip)?;
    let identity = child.identity()?;
    let listener = observe_owned_tcp_listener(&identity, child.endpoint)?;
    let client = child.connect()?;
    child.accept(&client)?;
    child.assert_quiet(1)?;
    let observation = observe_owned_tcp_connection(&identity, &listener, client.as_fd())?;
    assert_eq!(observation.process_identity(), &identity);
    assert_eq!(observation.client_local_addr(), client.local_addr()?);
    assert_eq!(observation.client_peer_addr(), client.peer_addr()?);
    assert_ne!(observation.server_socket_generation(), 0);
    verify_owned_tcp_connection(&observation, client.as_fd())?;
    child.assert_quiet(1)?;
    Ok(())
}

#[test]
fn owned_connection_observes_actual_ipv4_fd_before_data() -> TestResult {
    observed_connection(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

#[test]
fn owned_connection_observes_actual_ipv6_fd_before_data() -> TestResult {
    observed_connection(IpAddr::V6(Ipv6Addr::LOCALHOST))
}

#[test]
fn owned_connection_requires_an_accepted_server_fd() -> TestResult {
    let mut child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    let listener = observe_owned_tcp_listener(&identity, child.endpoint)?;
    let client = child.connect()?;
    child.assert_quiet(0)?;
    assert!(matches!(
        observe_owned_tcp_connection(&identity, &listener, client.as_fd()),
        Err(TcpListenerError::ConnectionNotFound)
    ));
    child.accept(&client)?;
    observe_owned_tcp_connection(&identity, &listener, client.as_fd())?;
    child.assert_quiet(1)?;
    Ok(())
}

#[test]
fn owned_connection_rejects_other_child_and_false_identity() -> TestResult {
    let mut child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let other = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    let other_identity = other.identity()?;
    let listener = observe_owned_tcp_listener(&identity, child.endpoint)?;
    let other_listener = observe_owned_tcp_listener(&other_identity, other.endpoint)?;
    let client = child.connect()?;
    child.accept(&client)?;
    assert!(matches!(
        observe_owned_tcp_connection(&other_identity, &other_listener, client.as_fd()),
        Err(TcpListenerError::ConnectionMismatch)
    ));
    let mut false_identity = identity.clone();
    false_identity.start_identity = StartIdentity::new("synthetic-other-incarnation")?;
    assert!(matches!(
        observe_owned_tcp_connection(&false_identity, &listener, client.as_fd()),
        Err(TcpListenerError::IdentityChanged)
    ));
    child.assert_quiet(1)?;
    Ok(())
}

#[test]
fn owned_connection_rejects_listener_replaced_at_the_same_port() -> TestResult {
    let mut child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    let listener = observe_owned_tcp_listener(&identity, child.endpoint)?;
    assert_eq!(
        child.command("replace")?.parse::<SocketAddr>()?,
        child.endpoint
    );
    let client = child.connect()?;
    child.accept(&client)?;
    assert!(matches!(
        observe_owned_tcp_connection(&identity, &listener, client.as_fd()),
        Err(TcpListenerError::ListenerChanged)
    ));
    let replacement = observe_owned_tcp_listener(&identity, child.endpoint)?;
    observe_owned_tcp_connection(&identity, &replacement, client.as_fd())?;
    child.assert_quiet(1)?;
    Ok(())
}

#[test]
fn owned_connection_cannot_reuse_an_observation_for_another_client_fd() -> TestResult {
    let mut child = FixtureChild::start(IpAddr::V4(Ipv4Addr::LOCALHOST))?;
    let identity = child.identity()?;
    let listener = observe_owned_tcp_listener(&identity, child.endpoint)?;
    let first = child.connect()?;
    child.accept(&first)?;
    let observation = observe_owned_tcp_connection(&identity, &listener, first.as_fd())?;
    let second = child.connect()?;
    child.accept(&second)?;
    assert_ne!(first.local_addr()?, second.local_addr()?);
    observe_owned_tcp_connection(&identity, &listener, second.as_fd())?;
    assert!(matches!(
        verify_owned_tcp_connection(&observation, second.as_fd()),
        Err(TcpListenerError::ConnectionChanged)
    ));
    child.assert_quiet(2)?;
    Ok(())
}
