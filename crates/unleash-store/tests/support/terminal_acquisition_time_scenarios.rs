//! Deterministic UTC cutover during a query over synthetic SQLite observations.
//! No sleep, host, PTY or operating system identity claim is involved.
use super::*;

struct CutoverClock {
    clock: Arc<ManualClock>,
    armed: AtomicBool,
    reads: std::sync::atomic::AtomicUsize,
    query_thread: std::thread::ThreadId,
    expiry: Option<TimestampMs>,
    uncertain: Arc<AtomicBool>,
}

impl unleash_domain::ClockPort for CutoverClock {
    fn now_ms(&self) -> TimestampMs {
        let before = unleash_domain::ClockPort::now_ms(self.clock.as_ref());
        if std::thread::current().id() != self.query_thread {
            return before;
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.armed.swap(false, Ordering::SeqCst) {
            if let Some(expiry) = self.expiry {
                self.clock.set_utc_ms(expiry.as_i64());
            } else {
                self.uncertain.store(true, Ordering::SeqCst);
            }
        }
        before
    }
}

fn assert_query_cutover_refused(expire: bool) {
    let (store, clock, _fault, allocation, _directory) = observed(false);
    let actor = allocation.context.actor_id.clone();
    let command: unleash_domain::CommandId = store
        .read_snapshot(|conn, _| {
            let id: String = conn.query_row(
                "SELECT command_id FROM command_receipts WHERE receipt_id=?1",
                [allocation.context.receipt_id.to_string()],
                |row| row.get(0),
            )?;
            Ok(id.parse()?)
        })
        .unwrap();
    let reference = format!("terminal-allocation:{}", allocation.context.terminal_id);
    let ordinary = store.inspect_command_receipt(&actor, command).unwrap();
    let mut config = store.config().clone();
    let cutover = Arc::new(CutoverClock {
        clock,
        armed: AtomicBool::new(false),
        reads: std::sync::atomic::AtomicUsize::new(0),
        query_thread: std::thread::current().id(),
        expiry: expire.then_some(allocation.context.lease_expires_at),
        uncertain: config.clock_uncertain.clone(),
    });
    config.clock = cutover.clone();
    drop(store);
    let store = UnleashStore::open(config).unwrap();
    assert_eq!(
        store
            .inspect_terminal_acquisition_result(&actor, command, &reference, PROJECT)
            .unwrap()
            .value
            .terminal_id,
        allocation.context.terminal_id,
        "the identical observation and lease qualify before arming the clock cutover"
    );
    cutover.reads.store(0, Ordering::SeqCst);
    cutover.armed.store(true, Ordering::SeqCst);
    let refused = store
        .inspect_terminal_acquisition_result(&actor, command, &reference, PROJECT)
        .unwrap_err();
    let expected = if expire {
        "expired during result inspection"
    } else {
        "uncertain during result inspection"
    };
    assert!(
        matches!(refused, StoreError::StaleFence { ref message } if message.contains(expected)),
        "the refusal must identify the final temporal guard: {refused}"
    );
    assert!(
        cutover.reads.load(Ordering::SeqCst) >= 2,
        "UTC must be sampled again after inspecting the proof"
    );
    assert_eq!(
        store.inspect_command_receipt(&actor, command).unwrap(),
        ordinary
    );
    let expiry: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                [allocation.context.lease_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        expiry,
        allocation.context.lease_expires_at.as_i64(),
        "inspection never renews the deadline"
    );
}

#[test]
fn t_f011_005_acquisition_result_rechecks_expiry_after_proof_inspection() {
    assert_query_cutover_refused(true);
}

#[test]
fn t_f011_005_acquisition_result_rechecks_clock_reliability_after_proof_inspection() {
    assert_query_cutover_refused(false);
}
