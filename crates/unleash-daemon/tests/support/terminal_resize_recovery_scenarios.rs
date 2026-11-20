//! Resize takes effect on the real PTY, then only the daemon loses its ACK.
//! Instrumented host/daemon library subprocesses are distinct from packaged binaries.
use super::super::super::resize_scenarios::{
    acquire_resize_worker, resize_request, stored_dimensions, wait_for_native_size,
};
use super::*;

#[derive(Debug, PartialEq, Eq)]
struct PersistedControl {
    lease: String,
    expires: i64,
    revision: i64,
    rows: u16,
    cols: u16,
    leases: i64,
    processes: i64,
    hosts: i64,
}

fn persisted(fixture: &LiveFixture, terminal: Uuid) -> PersistedControl {
    let connection = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT s.lease_id,l.expires_at_ms,s.control_revision,s.rows,s.cols,
         (SELECT COUNT(*) FROM resource_leases WHERE resource_kind='terminal'),
         (SELECT COUNT(*) FROM processes),(SELECT COUNT(DISTINCT host_id) FROM processes)
         FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id
         WHERE s.terminal_id=?1",
            [terminal.to_string()],
            |row| {
                Ok(PersistedControl {
                    lease: row.get(0)?,
                    expires: row.get(1)?,
                    revision: row.get(2)?,
                    rows: row.get(3)?,
                    cols: row.get(4)?,
                    leases: row.get(5)?,
                    processes: row.get(6)?,
                    hosts: row.get(7)?,
                })
            },
        )
        .unwrap()
}

fn resize_count(paths: &Paths) -> u64 {
    match std::fs::read_to_string(paths.resize_counter()) {
        Ok(value) => value.parse().unwrap(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("native resize counter unavailable: {error}"),
    }
}

fn host_identity(paths: &Paths) -> unleash_platform::OsProcessIdentity {
    serde_json::from_slice(&std::fs::read(paths.host_identity()).unwrap()).unwrap()
}

fn same_live_processes(
    fixture: &LiveFixture,
    paths: &Paths,
    host: &unleash_platform::OsProcessIdentity,
    child: &unleash_platform::OsProcessIdentity,
) {
    assert_eq!(host_identity(paths), *host, "no second native host startup");
    for expected in [host, child] {
        assert_eq!(
            unleash_platform::verify_process_identity(expected).unwrap(),
            unleash_platform::ProcessVerification::Match,
            "the original host and child must still be alive",
        );
    }
    assert_eq!(fixture.host_manifest().pid, Some(child.pid));
}

async fn wait_for_resize_eof(paths: &Paths) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match std::fs::read(paths.resize_eof()) {
            Ok(bytes) => {
                assert_eq!(bytes, RESIZE_EOF);
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("native EOF observation unavailable: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "native response must observe controller EOF"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn t_f011_003_resize_c4_lost_ack_keeps_unknown_without_second_ioctl() {
    let mut harness = harness(false).await;
    let fixture = &mut harness.fixture;
    let acquired = acquire_resize_worker(fixture).await;
    let terminal = acquired.terminal_id.as_uuid();
    let initial = persisted(fixture, terminal);
    assert_eq!((initial.rows, initial.cols), (24, 80));
    assert_eq!(initial.expires, acquired.lease_expires_at.as_i64());
    let native = fixture.host_manifest();
    let child = unleash_platform::inspect_process_identity(native.pid.unwrap()).unwrap();
    let host = host_identity(&harness.paths);
    same_live_processes(fixture, &harness.paths, &host, &child);
    let baseline = resize_count(&harness.paths);
    assert_eq!(baseline, 0, "initial PTY creation is not a resize request");

    unleash_platform::write_private_file_atomic(&harness.paths.resize_arm(), b"armed").unwrap();
    let request = resize_request(fixture, &acquired, 0xf011_7540, 37, 112);
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let retained_ack = receive(harness.listener.as_ref().unwrap(), RESIZE).await;
    assert_eq!(resize_count(&harness.paths), baseline + 1);
    assert!(!harness.paths.resize_eof().try_exists().unwrap());
    // The child runs stty itself. Reading its real output while the ACK is held
    // also proves the callback is not retaining the native operation gate.
    wait_for_native_size(fixture, terminal, "37 112").await;
    assert_eq!(stored_dimensions(fixture, terminal), (24, 80));
    assert_eq!(persisted(fixture, terminal), initial);
    let pending = state(fixture, &accepted);
    assert_eq!(
        (pending.intent.as_str(), pending.action.as_str()),
        ("handshake_resolved", "dispatching")
    );
    assert_eq!(
        (pending.dispatches, pending.proofs, pending.expiry),
        (1, 0, initial.expires)
    );
    physical(&native, &fixture.host_manifest());

    // SIGKILL targets only the known owned daemon Child, never a PID from SQL.
    fixture.daemon.kill_for_restart();
    assert_eq!(persisted(fixture, terminal), initial);
    same_live_processes(fixture, &harness.paths, &host, &child);
    std::fs::remove_file(harness.paths.resize_arm()).unwrap();
    drop(retained_ack);
    wait_for_resize_eof(&harness.paths).await;
    fixture.start_successor().await;
    physical(&native, &fixture.host_manifest());
    same_live_processes(fixture, &harness.paths, &host, &child);
    unchanged_unknown(fixture, &accepted, initial.expires);
    assert_eq!(persisted(fixture, terminal), initial);
    assert_eq!(resize_count(&harness.paths), baseline + 1);

    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_7541), request.command_id)
            .await
            .unwrap()
            .receipt,
        accepted,
    );
    assert_eq!(
        fixture.client.send_command(&request).await.unwrap(),
        accepted
    );
    unchanged_unknown(fixture, &accepted, initial.expires);
    assert_eq!(persisted(fixture, terminal), initial);
    assert_eq!(resize_count(&harness.paths), baseline + 1);

    // A completed, reaped successor drain is the late-task boundary. A delay
    // alone cannot prove that replay did not enqueue a second native operation.
    drain(fixture).await;
    unchanged_unknown(fixture, &accepted, initial.expires);
    assert_eq!(persisted(fixture, terminal), initial);
    assert_eq!(resize_count(&harness.paths), baseline + 1);
    same_live_processes(fixture, &harness.paths, &host, &child);
}
