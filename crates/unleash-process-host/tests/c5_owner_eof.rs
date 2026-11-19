//! Regression for loss of the C5 barrier owner. The sink is an independently
//! inspected, fsynced file, not a PTY. This isolates the EOF bug; it does not
//! qualify daemon recovery or the end-to-end T-F011-005 scenario.

#![expect(
    clippy::unwrap_used,
    reason = "Assertions in exclusively owned local subprocess fixtures"
)]

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use unleash_domain::{ActionId, CommandId, TimestampMs};
use unleash_process_host::{
    HostCutpointObserver, ProcessHostError, PtyInputTracker, StdioBarrierCutpointObserver,
};
use unleash_testkit::local_test_dir;

const FIXTURE_ROOT: &str = "UNLEASH_C5_EOF_FIXTURE_ROOT";
const BYTES: &[u8] = b"one-observed-write\n";
const DEADLINE: Duration = Duration::from_secs(20);

struct OwnedFixture(Child);
impl Drop for OwnedFixture {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn t_f011_005_c5_owner_eof_cannot_confirm_a_written_input() {
    let directory = local_test_dir("f011-c5-owner-eof").unwrap();
    let runtime = directory.path().join("private");
    unleash_platform::ensure_private_rendezvous_dir(&runtime).unwrap();
    let mut child = OwnedFixture(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "c5_eof_tracker_fixture",
                "--nocapture",
            ])
            .env_clear()
            .env(FIXTURE_ROOT, &runtime)
            .env("UNLEASH_HOST_CUTPOINT_TARGET", "C5")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, lines) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        let line = lines
            .recv_timeout(DEADLINE)
            .expect("bounded C5 notification")
            .unwrap();
        if line.ends_with("UNLEASH_CRASH_REACHED:C5") {
            break;
        }
    }
    assert_eq!(std::fs::read(runtime.join("effect.bin")).unwrap(), BYTES);
    // This is the only writer of the controller pipe. EOF must cancel C5,
    // rather than converting the already written input into a confirmed receipt.
    drop(child.0.stdin.take());
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "C5 EOF releases the owned fixture"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    reader.join().unwrap();
    let output = child.finished_stderr();
    assert!(
        status.success(),
        "EOF fixture must preserve Unknown: {output}"
    );
    assert_eq!(std::fs::read(runtime.join("effect.bin")).unwrap(), BYTES);
    let tracker = PtyInputTracker::open(&runtime, None).unwrap();
    assert_eq!(tracker.summary(), (0, vec![1]));
}

impl OwnedFixture {
    fn finished_stderr(&mut self) -> String {
        use std::io::Read;
        let mut output = String::new();
        if let Some(mut stderr) = self.0.stderr.take() {
            stderr.read_to_string(&mut output).unwrap();
        }
        output
    }
}

struct DurableSink(File);
impl Write for DurableSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.0.write(bytes)?;
        self.0.sync_all()?;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.sync_all()
    }
}

#[test]
#[ignore = "owned C5 tracker fixture selected by exact argv only"]
fn c5_eof_tracker_fixture() {
    let root = std::env::var_os(FIXTURE_ROOT).expect("explicit owned fixture directory");
    let root = Path::new(&root);
    let observer: Arc<dyn HostCutpointObserver> = Arc::new(
        StdioBarrierCutpointObserver::from_env().expect("explicit synthetic C5 selection"),
    );
    let tracker = PtyInputTracker::open(root, Some(observer)).unwrap();
    let mut sink = DurableSink(
        OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(root.join("effect.bin"))
            .unwrap(),
    );
    let command = CommandId::from_u128(0xf011_5501);
    let action = ActionId::from_u128(0xf011_5502);
    let outcome = tracker.dispatch_input(&mut sink, 1, command, action, BYTES, TimestampMs::new(1));
    assert!(
        outcome.is_err(),
        "EOF after the write must never create a native receipt"
    );
    assert_eq!(tracker.summary(), (0, vec![1]));
    assert_eq!(tracker.reconcile_in_flight().unwrap(), vec![1]);
    let before = std::fs::read(root.join("effect.bin")).unwrap();
    assert!(matches!(
        tracker.dispatch_input(&mut sink, 1, command, action, BYTES, TimestampMs::new(2)),
        Err(ProcessHostError::TerminalInputOutcomeUnknown { input_seq: 1, action_id, .. }) if action_id == action
    ));
    assert_eq!(std::fs::read(root.join("effect.bin")).unwrap(), before);
    assert_eq!(before, BYTES);
}
