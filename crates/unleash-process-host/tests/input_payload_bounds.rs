//! Native defense before the durable input marker. These spies prove byte and
//! journal non-mutation; they do not claim an operating system PTY or daemon.

#![expect(
    clippy::unwrap_used,
    reason = "Assertions in exclusively owned local input journal fixtures"
)]

use unleash_domain::{ActionId, CommandId, TimestampMs};
use unleash_process_host::{INPUT_JOURNAL_FILENAME, ProcessHostError, PtyInputTracker};
use unleash_testkit::local_test_dir;

fn assert_rejected_payload(label: &str, payload: &[u8]) {
    let directory = local_test_dir(label).unwrap();
    let runtime = directory.path().join("private");
    unleash_platform::ensure_private_rendezvous_dir(&runtime).unwrap();
    let tracker = PtyInputTracker::open(&runtime, None).unwrap();
    let before = std::fs::read(runtime.join(INPUT_JOURNAL_FILENAME)).unwrap();
    let mut writes = Vec::new();
    let result = tracker.dispatch_input(
        &mut writes,
        1,
        CommandId::from_u128(0xf011_5601),
        ActionId::from_u128(0xf011_5602),
        payload,
        TimestampMs::new(1),
    );
    assert!(
        matches!(result, Err(ProcessHostError::InvalidArgument { .. })),
        "{label}: invalid length rejected before the input marker"
    );
    assert!(writes.is_empty(), "{label}: no effect bytes");
    assert_eq!(
        tracker.summary(),
        (0, vec![]),
        "{label}: no invented input receipt"
    );
    assert_eq!(
        std::fs::read(runtime.join(INPUT_JOURNAL_FILENAME)).unwrap(),
        before,
        "{label}: no durable input marker"
    );
}

#[test]
fn t_f011_005_empty_payload_has_no_marker_or_write() {
    assert_rejected_payload("f011-empty-input", &[]);
}

#[test]
fn t_f011_005_oversized_payload_has_no_marker_or_write() {
    assert_rejected_payload("f011-oversized-input", &vec![0xff; 65_537]);
}

#[test]
fn t_f011_005_exact_payload_bounds_accept_arbitrary_bytes_once() {
    for payload in [vec![0], (0_u8..=255).cycle().take(65_536).collect()] {
        let directory = local_test_dir("f011-binary-input-bounds").unwrap();
        let runtime = directory.path().join("private");
        unleash_platform::ensure_private_rendezvous_dir(&runtime).unwrap();
        let tracker = PtyInputTracker::open(&runtime, None).unwrap();
        let mut writes = Vec::new();
        let command = CommandId::from_u128(0xf011_5611);
        let action = ActionId::from_u128(0xf011_5612);
        let first = tracker
            .dispatch_input(
                &mut writes,
                1,
                command,
                action,
                &payload,
                TimestampMs::new(1),
            )
            .unwrap();
        let second = tracker
            .dispatch_input(
                &mut writes,
                1,
                command,
                action,
                &payload,
                TimestampMs::new(2),
            )
            .unwrap();
        assert_eq!(writes, payload);
        assert_eq!(first.receipt_id, second.receipt_id);
        assert!(!first.deduplicated);
        assert!(second.deduplicated);
        assert_eq!(tracker.summary(), (1, vec![]));
    }
}
