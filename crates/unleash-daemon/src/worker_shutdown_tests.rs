//! Local StopDaemon lifecycle test using the packaged invocation worker.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{CommandId, RequestId, Revision, StopOutcome};
use unleash_platform::{ProcessVerification, verify_process_identity};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_testkit::local_test_dir;

use crate::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use crate::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};

#[allow(
    dead_code,
    reason = "Shared admission fixture also covers other executor kinds"
)]
#[path = "../../unleash-store/tests/common/admission_fixture.rs"]
mod admission_fixture;
#[path = "../tests/support/worker_admission_fixture.rs"]
mod worker_admission_fixture;

#[tokio::test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
async fn stop_daemon_reaps_the_admitted_worker_before_reporting_completion() {
    let directory = local_test_dir("stop-daemon-worker").unwrap();
    let configuration = DaemonConfig {
        data_root: Some(directory.path().join("data")),
        rendezvous_dir: Some(directory.path().join("rdv")),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(configuration).await.unwrap()
    else {
        panic!("fixture requires a new daemon");
    };
    let runtime = handle.test_runtime_state();
    let store = Arc::clone(&runtime.store);
    drop(runtime);
    let fixture_store = Arc::clone(&store);
    let (admission, _) = tokio::task::spawn_blocking(move || {
        worker_admission_fixture::admit_worker_fixture(&fixture_store)
    })
    .await
    .unwrap();
    let executable = worker_admission_fixture::worker_test_executable();
    let (start, identity) = handle
        .start_invocation_worker(admission.clone(), &executable)
        .await
        .unwrap();
    assert_eq!(start.stream_id, handle.stream_id());
    assert!(start.sequence > admission.event_cursor.sequence);
    let database = store.config().db_path.clone();
    drop(store);
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Match
    );

    request_stop(&handle).await;
    let report = tokio::time::timeout(Duration::from_secs(15), handle.wait_stopped())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.outcome, StopOutcome::Complete);
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_reconciliation(database, admission, start).await;
}

async fn request_stop(handle: &DaemonHandle) {
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let authority = client.hello().authority_generation;
    let status = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            query_type: "GetDaemonStatus".into(),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let revision = Revision::new(
        status
            .result
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap(),
    );
    let receipt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(2),
            command_id: CommandId::from_u128(2),
            authority_generation: authority,
            actor_ref: None,
            project_id: None,
            aggregate_id: authority.as_uuid(),
            expected_revision: revision,
            command_type: "StopDaemon".into(),
            payload: Map::new(),
        })
        .await
        .unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
}

async fn assert_reconciliation(
    database: std::path::PathBuf,
    admission: unleash_store::AdmissionCommit,
    start: unleash_protocol::EventCursor,
) {
    let observed = tokio::task::spawn_blocking(move || {
        let reader = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        reader
            .query_row(
                "SELECT i.state,t.state,c.state,
               (SELECT count(*) FROM invocation_attempts WHERE invocation_id=i.invocation_id),
               (SELECT count(*) FROM events
                WHERE aggregate_id=i.invocation_id AND event_type='InvocationReconciling'),
               EXISTS(SELECT 1 FROM events e
                JOIN events stop ON stop.stream_id=e.stream_id AND stop.event_type='DaemonStopped'
                WHERE e.aggregate_id=i.invocation_id AND e.event_type='InvocationReconciling'
                  AND e.sequence<stop.sequence AND e.stream_id=?4
                  AND json_extract(e.payload_json,'$.reason')='worker_control_channel_closed'
                  AND json_extract(e.payload_json,'$.start_sequence')=?5
                  AND json_extract(e.payload_json,'$.process_exit_observed')=1)
             FROM invocations i
             JOIN invocation_attempts t ON t.invocation_id=i.invocation_id
             JOIN capacity_reservations c ON c.reservation_id=?2
             WHERE i.invocation_id=?1 AND t.attempt_id=?3",
                rusqlite::params![
                    admission.unit.id().to_string(),
                    admission.reservation_id.to_string(),
                    admission.attempt_id.unwrap().to_string(),
                    start.stream_id.to_string(),
                    i64::try_from(start.sequence.as_u64()).unwrap(),
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        observed,
        (
            "reconciling".into(),
            "reconciling".into(),
            "held".into(),
            1,
            1,
            true
        )
    );
}
