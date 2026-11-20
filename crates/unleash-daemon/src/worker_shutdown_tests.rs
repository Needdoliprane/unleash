//! Local StopDaemon lifecycle test using the packaged invocation worker.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{CommandId, RequestId, Revision, StopOutcome};
use unleash_platform::{ProcessVerification, verify_process_identity};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState};
use unleash_testkit::local_test_dir;

use crate::execution_scheduler::{
    ExecutionSchedulingInputs, schedule_execution, schedule_invocation_execution,
};
use crate::worker_launch::{InvocationWorkerStep, WorkerLaunchError};
use crate::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};

#[allow(
    dead_code,
    reason = "Shared admission fixture also covers other executor kinds"
)]
#[path = "../../unleash-store/tests/common/admission_fixture.rs"]
mod admission_fixture;
#[path = "../tests/support/worker_admission_fixture.rs"]
#[allow(dead_code, reason = "Shared with the existing integration tests")]
mod worker_admission_fixture;

#[test]
fn invocation_worker_turn_preserves_an_earlier_replay() {
    use unleash_scheduler::{Decision, ExecutionUnit};
    use unleash_store::{AdmissionOutcome, StoreConfig, UnleashStore, WriteCriticality};

    let directory = local_test_dir("worker-turn-replay").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let evidence = admission_fixture::seed(&store, false);
    worker_admission_fixture::set_fixture_order(&store);
    let replay = unleash_domain::ReplayWorkerId::from_u128(3);
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE replay_workers SET queued_at_ms=0 WHERE replay_worker_id=?1",
                    [replay.to_string()],
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    admission_fixture::observe(&store, evidence);
    let pool = admission_fixture::POOL;
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    let candidate = admission_fixture::unit(3);
    let mut inputs = ExecutionSchedulingInputs::default();
    inputs.budgets.insert(candidate, vec![]);
    let (admission, proposed) =
        schedule_invocation_execution(&store, pool, snapshot.token, inputs).unwrap();

    assert!(
        matches!(proposed, Decision::Admit(plan) if plan.unit == ExecutionUnit::ReplayWorker(replay))
    );
    assert!(matches!(admission, AdmissionOutcome::Unchanged));
    assert_eq!(
        store.execution_pool_state(pool).unwrap().current_occupation,
        0
    );
    store.shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
async fn stop_daemon_reaps_the_admitted_worker_before_reporting_completion() {
    let directory = local_test_dir("stop-daemon-worker").unwrap();
    let handle = fixture_daemon(directory.path()).await;
    let store = Arc::clone(&handle.test_runtime_state().store);
    let fixture_store = Arc::clone(&store);
    let (token, inputs, _) = tokio::task::spawn_blocking(move || {
        worker_admission_fixture::prepare_worker_fixture(&fixture_store)
    })
    .await
    .unwrap();
    let executable = worker_admission_fixture::worker_test_executable();
    let competing_inputs = ExecutionSchedulingInputs {
        budgets: inputs.budgets.clone(),
        ..ExecutionSchedulingInputs::default()
    };
    let (first, second) = tokio::join!(
        handle.schedule_invocation_worker(admission_fixture::POOL, token, inputs, &executable),
        handle.schedule_invocation_worker(
            admission_fixture::POOL,
            token,
            competing_inputs,
            &executable,
        ),
    );
    let (admission, start) = match (first, second) {
        (
            Ok(InvocationWorkerStep::Admitted { admission, start }),
            Err(WorkerLaunchError::Admission(unleash_store::StoreError::RevisionConflict {
                ..
            })),
        )
        | (
            Err(WorkerLaunchError::Admission(unleash_store::StoreError::RevisionConflict {
                ..
            })),
            Ok(InvocationWorkerStep::Admitted { admission, start }),
        ) => (admission, start),
        (first, second) => panic!("one worker and one stale turn expected: {first:?}, {second:?}"),
    };
    let (start, identity) = start.unwrap();
    assert_eq!(admission.unit, admission_fixture::unit(1));
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
    assert_reconciliation(database, *admission, start).await;
}

#[tokio::test]
async fn unavailable_worker_returns_the_durable_admission_for_reconciliation() {
    let directory = local_test_dir("schedule-unavailable-worker").unwrap();
    let handle = fixture_daemon(directory.path()).await;
    let store = Arc::clone(&handle.test_runtime_state().store);
    let fixture_store = Arc::clone(&store);
    let (token, inputs, _) = tokio::task::spawn_blocking(move || {
        worker_admission_fixture::prepare_worker_fixture(&fixture_store)
    })
    .await
    .unwrap();
    let InvocationWorkerStep::Admitted { admission, start } = handle
        .schedule_invocation_worker(
            admission_fixture::POOL,
            token,
            inputs,
            &directory.path().join("absent-worker"),
        )
        .await
        .unwrap()
    else {
        panic!("expected a committed admission");
    };
    assert!(matches!(start, Err(WorkerLaunchError::NativeProcess)));
    assert_eq!(admission.unit, admission_fixture::unit(1));
    let state = tokio::task::spawn_blocking(move || {
        store
            .read_snapshot(|connection, _| {
                Ok(connection.query_row(
                    "SELECT i.state,r.state,(SELECT count(*) FROM events
                    WHERE aggregate_id=i.invocation_id AND event_type='InvocationStarted')
                 FROM invocations i JOIN capacity_reservations r ON r.reservation_id=?2
                 WHERE i.invocation_id=?1",
                    rusqlite::params![
                        admission.unit.id().to_string(),
                        admission.reservation_id.to_string()
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?)
            })
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(state, ("admitted".into(), "held".into(), 0));
    assert_eq!(
        handle
            .shutdown_graceful("fixture complete")
            .await
            .unwrap()
            .outcome,
        StopOutcome::Complete
    );
}

async fn fixture_daemon(path: &std::path::Path) -> DaemonHandle {
    let configuration = DaemonConfig {
        data_root: Some(path.join("data")),
        rendezvous_dir: Some(path.join("rdv")),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(configuration).await.unwrap()
    else {
        panic!("fixture requires a new daemon");
    };
    handle
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
