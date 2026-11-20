//! Exercise the daemon scheduler against the real `SQLite` writer with synthetic executors.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Synthetic test fixtures and direct policy assertions"
)]

use std::sync::Arc;

use unleash_daemon::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use unleash_daemon::recovery::{classify_and_finish_recovery, enter_recovering_and_emit_event};
use unleash_daemon::worker_launch::{WorkerLaunchError, prepare_invocation_worker};
use unleash_domain::{DaemonInstanceId, DaemonMode, InvocationId, ReservationId};
use unleash_platform::{ProcessVerification, verify_process_identity};
use unleash_scheduler::{CapacityWait, CheckpointEligibility, CheckpointRequest, Decision};
use unleash_store::{
    AdmissionCommit, AdmissionOutcome, StoreConfig, StoreError, UnleashStore,
    WorkerProcessObservation, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

#[path = "../../unleash-store/tests/common/admission_fixture.rs"]
mod admission_fixture;
use admission_fixture::{OTHER_PROJECT, POOL, observe, seed, seed_invocation, unit};

#[path = "support/worker_admission_fixture.rs"]
mod worker_admission_fixture;
use worker_admission_fixture::{admit_worker_fixture, set_fixture_order, worker_test_executable};

fn durable_counts(store: &UnleashStore) -> (i64, i64, i64) {
    store
        .read_snapshot(|connection, _| {
            Ok(connection.query_row(
                "SELECT
                  (SELECT count(*) FROM capacity_reservations WHERE pool_id=?1),
                  (SELECT count(*) FROM resource_leases WHERE resource_kind='execution'),
                  (SELECT count(*) FROM events WHERE event_type IN
                      ('InvocationAdmitted','ReplayWorkerAdmitted','ProviderProbeAdmitted'))",
                [POOL.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .unwrap()
}

fn assert_checkpoint_proposal(
    store: &UnleashStore,
    object: unleash_domain::Sha256Digest,
    target: ReservationId,
) {
    let daemon = store.daemon_state().unwrap();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            seed_invocation(tx, 7, OTHER_PROJECT, true, object, &daemon, now)
        })
        .unwrap();
    observe(store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let (outcome, decision) = schedule_execution(
        store,
        POOL,
        snapshot.token,
        ExecutionSchedulingInputs::default(),
    )
    .unwrap();
    assert!(matches!(outcome, AdmissionOutcome::Unchanged));
    assert!(matches!(
        decision,
        Decision::Wait {
            reason: CapacityWait::GlobalCapacity,
            checkpoint: None,
            ..
        }
    ));
    let mut inputs = ExecutionSchedulingInputs::default();
    inputs
        .checkpoint_eligibility
        .insert(target, CheckpointEligibility::SafeBoundary);
    let (outcome, decision) = schedule_execution(store, POOL, snapshot.token, inputs).unwrap();
    assert!(matches!(outcome, AdmissionOutcome::Unchanged));
    let Decision::Wait {
        reason: CapacityWait::GlobalCapacity,
        checkpoint,
        ..
    } = decision
    else {
        panic!("Le contrôle doit attendre le checkpoint à capacité pleine");
    };
    assert_eq!(
        checkpoint,
        Some(CheckpointRequest {
            waiting_control: InvocationId::from_u128(7),
            target
        })
    );
    assert_eq!(durable_counts(store), (5, 5, 5));
    assert!(
        store
            .execution_admission_snapshot(POOL)
            .unwrap()
            .checkpoints
            .is_empty()
    );
}

#[test]
fn t_f003_003_q15_and_writer_enforce_one_pool_with_twenty_logical_leaders() {
    let directory = local_test_dir("f003-q15-writer").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let object = seed(&store, false);
    set_fixture_order(&store);
    let leader_count = store
        .read_snapshot(|connection, _| {
            Ok(connection.query_row(
                "SELECT count(*) FROM agents WHERE level='N+1'",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(leader_count, 20);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let (outcome, decision) = schedule_execution(
        &store,
        POOL,
        snapshot.token,
        ExecutionSchedulingInputs::default(),
    )
    .unwrap();
    assert!(matches!(outcome, AdmissionOutcome::Unchanged));
    assert_eq!(decision, Decision::Idle);

    observe(&store, object);
    let first_token = store.execution_admission_snapshot(POOL).unwrap().token;
    assert!(matches!(
        schedule_execution(&store, POOL, first_token, ExecutionSchedulingInputs::default()),
        Err(StoreError::InvariantViolation(message)) if message == "Plan budgétaire de l’admission absent"
    ));
    assert_eq!(durable_counts(&store), (0, 0, 0));

    let mut first_reservation = None;
    for index in 1_u128..=5 {
        if index > 1 {
            observe(&store, object);
        }
        let snapshot = store.execution_admission_snapshot(POOL).unwrap();
        let mut inputs = ExecutionSchedulingInputs::default();
        inputs.budgets.extend(
            snapshot
                .queued
                .iter()
                .map(|candidate| (candidate.unit, vec![])),
        );
        let (outcome, decision) = schedule_execution(&store, POOL, snapshot.token, inputs).unwrap();
        let AdmissionOutcome::Admitted(commit) = outcome else {
            panic!("Admission attendue pour {}", unit(index).id());
        };
        assert!(matches!(decision, Decision::Admit(_)));
        assert_eq!(commit.unit, unit(index));
        let count = i64::try_from(index).unwrap();
        assert_eq!(durable_counts(&store), (count, count, count));
        if index == 1 {
            first_reservation = Some(commit.reservation_id);
            assert!(matches!(
                schedule_execution(
                    &store,
                    POOL,
                    first_token,
                    ExecutionSchedulingInputs::default()
                ),
                Err(StoreError::RevisionConflict { .. })
            ));
            assert_eq!(durable_counts(&store), (1, 1, 1));
        }
    }

    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(snapshot.pool.current_occupation, 5);
    observe(&store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let (outcome, decision) = schedule_execution(
        &store,
        POOL,
        snapshot.token,
        ExecutionSchedulingInputs::default(),
    )
    .unwrap();
    assert!(matches!(outcome, AdmissionOutcome::Unchanged));
    assert!(matches!(
        decision,
        Decision::Wait {
            reason: CapacityWait::GlobalCapacity,
            ..
        }
    ));
    assert_eq!(durable_counts(&store), (5, 5, 5));
    assert_checkpoint_proposal(&store, object, first_reservation.unwrap());
}

fn worker_preparation_state(
    store: &UnleashStore,
    grant: &AdmissionCommit,
) -> (String, String, i64, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT i.state,t.state,i.revision,(SELECT count(*) FROM events e
                  WHERE e.aggregate_id=i.invocation_id AND e.event_type='InvocationWorkerLaunchPrepared')
                 FROM invocations i JOIN invocation_attempts t ON t.invocation_id=i.invocation_id
                 WHERE i.invocation_id=?1 AND t.attempt_id=?2",
                rusqlite::params![grant.unit.id().to_string(), grant.attempt_id.unwrap().to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?)
        })
        .unwrap()
}

fn worker_test_store(root: &std::path::Path) -> Arc<UnleashStore> {
    let observed_boot = unleash_platform::current_boot_identity()
        .unwrap()
        .to_string();
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    // The standalone Store fixture needs the boot observation normally installed by daemon recovery.
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE daemon_state SET boot_identity=?1 WHERE singleton=1",
                    [&observed_boot],
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    Arc::new(store)
}

fn worker_test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_001_admission_prepares_one_owned_worker_without_granting_execution() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-native-worker").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, manifest_digest) = admit_worker_fixture(&store);
    let original_revision = i64::try_from(grant.revision.as_u64()).unwrap();
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "admitted".to_owned(),
            "admitted".to_owned(),
            original_revision,
            0
        )
    );
    let mut stale = grant.clone();
    stale.lease.fence = stale.lease.fence.next();
    assert!(matches!(
        runtime.block_on(prepare_invocation_worker(
            Arc::clone(&store),
            stale,
            &executable
        )),
        Err(WorkerLaunchError::Admission(StoreError::StaleFence { .. }))
    ));
    assert_eq!(worker_preparation_state(&store, &grant).3, 0);
    let (first, second) = runtime.block_on(async {
        tokio::join!(
            prepare_invocation_worker(Arc::clone(&store), grant.clone(), &executable),
            prepare_invocation_worker(Arc::clone(&store), grant.clone(), &executable),
        )
    });
    let pending = match (first, second) {
        (Ok(worker), Err(WorkerLaunchError::Admission(StoreError::RevisionConflict { .. })))
        | (Err(WorkerLaunchError::Admission(StoreError::RevisionConflict { .. })), Ok(worker)) => {
            worker
        }
        (Err(first), Err(second)) => {
            panic!("expected one prepared worker: {first:?}, {second:?}");
        }
        _ => panic!("one admission must never prepare two children"),
    };
    let ready = pending.ready();
    assert_eq!(ready.kind, unleash_agent_worker::WorkerReplyKind::Ready);
    assert_eq!(ready.binding.attempt_id, grant.attempt_id.unwrap());
    assert_eq!(ready.binding.reservation_id, grant.reservation_id);
    assert_eq!(ready.binding.fence, grant.lease.fence);
    assert_eq!(ready.binding.context_manifest_digest, manifest_digest);
    assert_eq!(ready.pid, pending.identity().pid);
    assert_eq!(
        pending.preparation().prepared_revision,
        grant.revision.next()
    );
    assert_eq!(
        pending.preparation().event_cursor.stream_id,
        grant.event_cursor.stream_id
    );
    assert!(pending.preparation().event_cursor.sequence > grant.event_cursor.sequence);
    assert_eq!(
        verify_process_identity(pending.identity()).unwrap(),
        ProcessVerification::Match
    );
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "admitted".to_owned(),
            "admitted".to_owned(),
            original_revision + 1,
            1
        )
    );
    let identity = pending.identity().clone();
    runtime.block_on(pending.close()).unwrap();
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    assert_eq!(worker_preparation_state(&store, &grant).3, 1);
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_worker_manifest_loss_keeps_claim_and_slot() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-worker-manifest-loss").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, digest) = admit_worker_fixture(&store);
    let object = store.object_record(digest).unwrap();
    let artifact = data.path().join("artifacts").join(object.relative_path);
    assert!(artifact.is_file() && !artifact.is_symlink());
    std::fs::write(artifact, br#"{"fixture":"corrupted"}"#).unwrap();
    let first = runtime.block_on(prepare_invocation_worker(
        Arc::clone(&store),
        grant.clone(),
        &executable,
    ));
    if !matches!(
        first,
        Err(WorkerLaunchError::Admission(
            StoreError::EvidenceUnavailable(_)
        ))
    ) {
        if let Err(error) = first {
            panic!("expected unverified bytes after claim, got {error:?}");
        }
        panic!("a published record with incorrect bytes must fail before native startup");
    }
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "admitted".to_owned(),
            "admitted".to_owned(),
            i64::try_from(grant.revision.as_u64()).unwrap() + 1,
            1
        )
    );
    assert!(matches!(
        runtime.block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable
        )),
        Err(WorkerLaunchError::Admission(
            StoreError::RevisionConflict { .. }
        ))
    ));
    assert_eq!(worker_preparation_state(&store, &grant).3, 1);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_001_worker_permit_requires_running_commit() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-worker-activation").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let identity = pending.identity().clone();
    let binding = pending.ready().binding.clone();
    let preparation = pending.preparation().event_cursor;
    let rev = i64::try_from(pending.preparation().prepared_revision.as_u64()).unwrap();
    let active = runtime
        .block_on(pending.activate(Arc::clone(&store)))
        .unwrap();
    let reply = active.acknowledgement();
    assert_eq!(reply.kind, unleash_agent_worker::WorkerReplyKind::Activated);
    assert_eq!(reply.binding, binding);
    assert_eq!(reply.pid, identity.pid);
    let start = active.start_cursor();
    assert_eq!(start.stream_id, preparation.stream_id);
    assert!(start.sequence > preparation.sequence);
    assert_eq!(
        worker_preparation_state(&store, &grant),
        ("running".to_owned(), "running".to_owned(), rev + 1, 1)
    );
    let payload: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT payload_json FROM events
                 WHERE aggregate_id=?1 AND sequence=?2 AND event_type='InvocationStarted'",
                rusqlite::params![
                    grant.unit.id().to_string(),
                    i64::try_from(start.sequence.as_u64()).unwrap()
                ],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(
        payload.get("attempt_id"),
        Some(&serde_json::json!(binding.attempt_id))
    );
    assert_eq!(
        payload.get("reservation_id"),
        Some(&serde_json::json!(binding.reservation_id))
    );
    assert_eq!(
        payload.get("prepared_sequence"),
        Some(&serde_json::json!(preparation.sequence))
    );
    assert_eq!(
        payload.pointer("/process_identity/pid"),
        Some(&serde_json::json!(identity.pid))
    );
    assert_eq!(
        payload.pointer("/process_identity/start_identity"),
        Some(&serde_json::json!(identity.start_identity))
    );
    let start = active.start_cursor();
    let before_close = worker_preparation_state(&store, &grant);
    let resources = invocation_worker_resources(&store, &grant);
    runtime.block_on(active.close()).unwrap();
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "reconciling".to_owned(),
            "reconciling".to_owned(),
            before_close.2 + 1,
            1,
        ),
        "closing an activated worker must leave a durable reconciliation"
    );
    assert_worker_reconciliation(&store, &grant, start, "worker_control_channel_closed");
    assert_eq!(invocation_worker_resources(&store, &grant), resources);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_worker_activation_refuses_a_stale_preparation() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-revoked-worker").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let id = grant.unit.id().to_string();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE invocations SET revision=revision+1 WHERE invocation_id=?1",
                    [id]
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        runtime.block_on(pending.activate(Arc::clone(&store))),
        Err(WorkerLaunchError::Admission(
            StoreError::RevisionConflict { .. }
        ))
    ));
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "admitted".to_owned(),
            "admitted".to_owned(),
            i64::try_from(grant.revision.as_u64()).unwrap() + 2,
            1,
        )
    );
    let started: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT count(*) FROM events WHERE event_type='InvocationStarted'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(started, 0);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_worker_start_event_failure_rolls_back_both_states() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-worker-start-rollback").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let identity = pending.identity().clone();
    let before = worker_preparation_state(&store, &grant);
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute_batch(
                "CREATE TRIGGER fixture_refuse_worker_start BEFORE INSERT ON events
             WHEN NEW.event_type='InvocationStarted'
             BEGIN SELECT RAISE(ABORT, 'fixture_worker_start_aborted'); END;",
            )?;
            Ok(())
        })
        .unwrap();
    let rejected = runtime.block_on(pending.activate(Arc::clone(&store)));
    assert!(matches!(
        rejected,
        Err(WorkerLaunchError::Admission(error)) if error.to_string().contains("fixture_worker_start_aborted")
    ));
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(worker_preparation_state(&store, &grant), before);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the matching packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_worker_start_rejects_out_of_range_event_sequence() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-worker-sequence-range").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let before = worker_preparation_state(&store, &grant);
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE sqlite_sequence SET seq=9007199254740991 WHERE name='events'",
                    [],
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    match runtime.block_on(pending.activate(Arc::clone(&store))) {
        Err(WorkerLaunchError::Admission(StoreError::InvariantViolation(message))) => {
            assert_eq!(message, "séquence de lancement hors plage");
        }
        Err(error) => panic!("unexpected worker start error: {error:?}"),
        Ok(_) => panic!("worker started despite the out-of-range event sequence"),
    }
    assert_eq!(worker_preparation_state(&store, &grant), before);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

#[test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_shutdown_after_worker_start_preserves_the_single_attempt() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-running-before-permit").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let identity = pending.identity().clone();
    let prepared = pending.preparation().event_cursor;
    let revision = i64::try_from(pending.preparation().prepared_revision.as_u64()).unwrap() + 2;
    let durable_start = || {
        store
            .read_snapshot(|conn, _| {
                Ok(conn.query_row(
                    "SELECT sequence,payload_json FROM events
                 WHERE aggregate_id=?1 AND event_type='InvocationStarted'",
                    [grant.unit.id().to_string()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )?)
            })
            .unwrap()
    };
    let shutdown = async {
        let (sequence, _) = durable_start();
        assert!(u64::try_from(sequence).unwrap() > prepared.sequence.as_u64());
    };
    let start =
        match runtime.block_on(pending.activate_until_shutdown(Arc::clone(&store), shutdown)) {
            Err(WorkerLaunchError::ContinuationUnknown { start_cursor }) => start_cursor,
            Err(error) => panic!("unexpected shutdown result: {error:?}"),
            Ok(worker) => {
                runtime.block_on(worker.close()).unwrap();
                panic!("worker received a permit despite the controller shutdown");
            }
        };
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(start.stream_id, prepared.stream_id);
    let (sequence, raw) = durable_start();
    assert_eq!(start.sequence.as_u64(), u64::try_from(sequence).unwrap());
    let payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        payload.get("attempt_id"),
        Some(&serde_json::json!(grant.attempt_id.unwrap()))
    );
    assert_eq!(
        payload.get("reservation_id"),
        Some(&serde_json::json!(grant.reservation_id))
    );
    assert_eq!(
        payload.get("fence"),
        Some(&serde_json::json!(grant.lease.fence))
    );
    assert_eq!(
        payload.pointer("/process_identity/pid"),
        Some(&serde_json::json!(identity.pid))
    );
    assert_eq!(
        payload.pointer("/process_identity/start_identity"),
        Some(&serde_json::json!(identity.start_identity))
    );
    assert!(
        runtime
            .block_on(prepare_invocation_worker(
                Arc::clone(&store),
                grant.clone(),
                &executable,
            ))
            .is_err()
    );
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "reconciling".to_owned(),
            "reconciling".to_owned(),
            revision,
            1
        )
    );
    assert_worker_reconciliation(&store, &grant, start, "worker_continuation_unconfirmed");
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

const UNCONFIRMED: &str = "worker_continuation_unconfirmed";

fn assert_worker_reconciliation(
    store: &UnleashStore,
    grant: &unleash_store::AdmissionCommit,
    start: unleash_protocol::EventCursor,
    expected_reason: &str,
) {
    let counts: (i64, i64) = store.read_snapshot(|conn, _| {
        Ok(conn.query_row(
            "SELECT (SELECT count(*) FROM invocation_attempts WHERE invocation_id=?1),
             (SELECT count(*) FROM events WHERE aggregate_id=?1 AND event_type='InvocationStarted')",
            [grant.unit.id().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }).unwrap();
    assert_eq!(counts, (1, 1));
    let (count, sequence, exit_observed, outbox, reason): (i64, i64, bool, i64, String) = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*), json_extract(payload_json,'$.start_sequence'),
                        json_extract(payload_json,'$.process_exit_observed'),
                        (SELECT COUNT(*) FROM outbox WHERE event_id=events.event_id),
                        json_extract(payload_json,'$.reason')
                 FROM events WHERE aggregate_id=?1 AND event_type='InvocationReconciling'",
                [grant.unit.id().to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(u64::try_from(sequence).unwrap(), start.sequence.as_u64());
    assert!(exit_observed);
    assert_eq!(outbox, 2);
    assert_eq!(reason, expected_reason);
}

#[test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_worker_close_reports_failed_reconciliation_and_preserves_the_attempt() {
    let data = local_test_dir("f003-worker-close-rollback").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &worker_test_executable(),
        ))
        .unwrap();
    let identity = pending.identity().clone();
    let preparation = pending.preparation().clone();
    let running = runtime
        .block_on(pending.activate(Arc::clone(&store)))
        .unwrap();
    let start = running.start_cursor();
    let before = worker_preparation_state(&store, &grant);
    let resources = invocation_worker_resources(&store, &grant);
    let outbox = reconciliation_outbox_count(&store);
    reject_reconciliation_delivery(&store);

    assert!(matches!(
        runtime.block_on(running.close()),
        Err(WorkerLaunchError::ContinuationNotRecorded { start_cursor }) if start_cursor == start
    ));
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(worker_preparation_state(&store, &grant), before);
    assert_eq!(invocation_worker_resources(&store, &grant), resources);
    assert_eq!(reconciliation_outbox_count(&store), outbox);

    allow_reconciliation_during_drain(&store);
    let observation = WorkerProcessObservation {
        pid: identity.pid,
        owner_uid: identity.owner_uid,
        boot_identity: identity.boot_identity.as_str().to_owned(),
        start_identity: identity.start_identity.as_str().to_owned(),
    };
    let record = || {
        store.record_invocation_worker_channel_closed(
            preparation.clone(),
            start,
            observation.clone(),
            true,
        )
    };
    let cursor = record().unwrap();
    assert_eq!(record().unwrap(), cursor);
    assert!(matches!(
        store.record_invocation_worker_continuation_lost(preparation, start, observation, true),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "reconciling".to_owned(),
            "reconciling".to_owned(),
            before.2 + 1,
            1,
        )
    );
    assert_worker_reconciliation(&store, &grant, start, "worker_control_channel_closed");
    assert_eq!(invocation_worker_resources(&store, &grant), resources);
    assert_eq!(reconciliation_outbox_count(&store), outbox + 2);
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

fn reject_reconciliation_delivery(store: &UnleashStore) {
    store.execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
        tx.execute_batch(
            "CREATE TRIGGER fixture_refuse_reconciliation_delivery BEFORE INSERT ON outbox
             WHEN NEW.destination='harness' AND EXISTS(
                SELECT 1 FROM events WHERE event_id=NEW.event_id AND event_type='InvocationReconciling')
             BEGIN SELECT RAISE(ABORT, 'fixture_reconciliation_delivery_aborted'); END;",
        )?;
        Ok(())
    }).unwrap();
}

fn allow_reconciliation_during_drain(store: &UnleashStore) {
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute_batch(
                "DROP TRIGGER fixture_refuse_reconciliation_delivery;
             UPDATE daemon_state SET mode='draining' WHERE singleton=1;",
            )?;
            Ok(())
        })
        .unwrap();
}

fn invocation_worker_resources(
    store: &UnleashStore,
    grant: &AdmissionCommit,
) -> (String, String, i64, i64, String, String, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT l.state,l.owner_id,l.fence,l.expires_at_ms,c.state,c.owner_id,c.amount
             FROM resource_leases l, capacity_reservations c
             WHERE l.lease_id=?1 AND c.reservation_id=?2",
                rusqlite::params![
                    grant.lease.lease_id.to_string(),
                    grant.reservation_id.to_string()
                ],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )?)
        })
        .unwrap()
}

fn reconciliation_outbox_count(store: &UnleashStore) -> i64 {
    store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?)
        })
        .unwrap()
}

#[test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_reconciliation_outbox_rollback_then_exact_replay_during_drain() {
    let executable = worker_test_executable();
    let data = local_test_dir("f003-reconciliation-rollback").unwrap();
    let store = worker_test_store(data.path());
    let runtime = worker_test_runtime();
    let (grant, _) = admit_worker_fixture(&store);
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &executable,
        ))
        .unwrap();
    let preparation = pending.preparation().clone();
    let identity = pending.identity().clone();
    let observation = WorkerProcessObservation {
        pid: identity.pid,
        owner_uid: identity.owner_uid,
        boot_identity: identity.boot_identity.as_str().to_owned(),
        start_identity: identity.start_identity.as_str().to_owned(),
    };
    let resources = invocation_worker_resources(&store, &grant);
    let outbox_count = || reconciliation_outbox_count(&store);
    let initial_outbox = outbox_count();
    reject_reconciliation_delivery(&store);
    let result = runtime
        .block_on(pending.activate_until_shutdown(Arc::clone(&store), std::future::ready(())));
    let Err(WorkerLaunchError::ContinuationNotRecorded { start_cursor }) = result else {
        panic!("outbox refusal must expose the original start cursor");
    };
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    let running = i64::try_from(preparation.prepared_revision.as_u64()).unwrap() + 1;
    assert_eq!(
        worker_preparation_state(&store, &grant),
        ("running".to_owned(), "running".to_owned(), running, 1)
    );
    assert_eq!(outbox_count(), initial_outbox + 2);
    assert_eq!(invocation_worker_resources(&store, &grant), resources);
    allow_reconciliation_during_drain(&store);
    let mut other_process = observation.clone();
    other_process.start_identity.push_str("-different");
    for (cursor, identity) in [
        (preparation.event_cursor, observation.clone()),
        (start_cursor, other_process),
    ] {
        assert!(matches!(
            store.record_invocation_worker_continuation_lost(
                preparation.clone(),
                cursor,
                identity,
                true,
            ),
            Err(StoreError::StaleFence { .. })
        ));
    }
    assert_eq!(outbox_count(), initial_outbox + 2);
    let record = || {
        store
            .record_invocation_worker_continuation_lost(
                preparation.clone(),
                start_cursor,
                observation.clone(),
                true,
            )
            .unwrap()
    };
    std::thread::scope(|scope| {
        let first = scope.spawn(record);
        let second = scope.spawn(record);
        assert_eq!(first.join().unwrap(), second.join().unwrap());
    });
    assert!(matches!(
        store.record_invocation_worker_continuation_lost(
            preparation,
            start_cursor,
            observation,
            false,
        ),
        Err(StoreError::StaleFence { .. })
    ));
    assert_worker_reconciliation(&store, &grant, start_cursor, UNCONFIRMED);
    assert_eq!(
        worker_preparation_state(&store, &grant),
        (
            "reconciling".to_owned(),
            "reconciling".to_owned(),
            running + 1,
            1
        )
    );
    assert_eq!(outbox_count(), initial_outbox + 4);
    assert_eq!(invocation_worker_resources(&store, &grant), resources);
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}

fn enter_worker_recovery(store: &UnleashStore) -> (DaemonMode, Uuid) {
    let boot = unleash_platform::current_boot_identity().unwrap();
    let instance = DaemonInstanceId::from_uuid(store.config().id_generator.new_uuid());
    let (_, previous, started) =
        enter_recovering_and_emit_event(store, boot.as_str(), instance).unwrap();
    (previous, started.event_id)
}

#[test]
#[ignore = "requires the packaged Rust worker via UNLEASH_TEST_AGENT_WORKER_BIN"]
fn t_f003_006_restart_reconciles_a_live_worker_without_freeing_its_capacity() {
    let data = local_test_dir("f003-worker-after-restart").unwrap();
    let store = worker_test_store(data.path());
    let (previous, started) = enter_worker_recovery(&store);
    classify_and_finish_recovery(&store, previous, started).unwrap();
    let (grant, _) = admit_worker_fixture(&store);
    let runtime = worker_test_runtime();
    let pending = runtime
        .block_on(prepare_invocation_worker(
            Arc::clone(&store),
            grant.clone(),
            &worker_test_executable(),
        ))
        .unwrap();
    let identity = pending.identity().clone();
    let old_preparation = pending.preparation().clone();
    let running = runtime
        .block_on(pending.activate(Arc::clone(&store)))
        .unwrap();
    let start_cursor = running.start_cursor();
    let revision = worker_preparation_state(&store, &grant).2;
    let mut resources = invocation_worker_resources(&store, &grant);
    assert_eq!(resources.0, "held");
    assert_eq!(resources.4, "held");
    resources.0 = "suspect".to_owned();
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();

    let recovered = UnleashStore::open(StoreConfig::new(data.path())).unwrap();
    let (previous, trigger) = enter_worker_recovery(&recovered);
    assert_worker_recovery_rolls_back_on_outbox_error(&recovered, &grant, previous, trigger);
    let (daemon, events) = classify_and_finish_recovery(&recovered, previous, trigger).unwrap();
    assert_eq!(daemon.mode, DaemonMode::Ready);
    assert!(daemon.daemon_epoch > grant.lease.daemon_epoch);
    assert_eq!(
        worker_preparation_state(&recovered, &grant),
        (
            "reconciling".to_owned(),
            "reconciling".to_owned(),
            revision + 1,
            1
        )
    );
    assert_eq!(invocation_worker_resources(&recovered, &grant), resources);
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Match
    );
    assert_historical_worker_event(&recovered, &grant, start_cursor, trigger, &events);
    let old_observation = WorkerProcessObservation {
        pid: identity.pid,
        owner_uid: identity.owner_uid,
        boot_identity: identity.boot_identity.as_str().to_owned(),
        start_identity: identity.start_identity.as_str().to_owned(),
    };
    assert!(matches!(
        recovered.record_invocation_worker_continuation_lost(
            old_preparation,
            start_cursor,
            old_observation,
            false,
        ),
        Err(StoreError::StaleFence { .. })
    ));

    let (previous, next_trigger) = enter_worker_recovery(&recovered);
    let (_, replay) = classify_and_finish_recovery(&recovered, previous, next_trigger).unwrap();
    assert!(
        !replay
            .iter()
            .any(|event| event.event_type == "InvocationReconciling")
    );
    assert_eq!(worker_preparation_state(&recovered, &grant).2, revision + 1);
    assert_eq!(invocation_worker_resources(&recovered, &grant), resources);
    assert_eq!(
        recovered
            .execution_pool_state(POOL)
            .unwrap()
            .current_occupation,
        1
    );
    assert!(matches!(
        runtime.block_on(running.close()),
        Err(WorkerLaunchError::ContinuationNotRecorded { start_cursor: original })
            if original == start_cursor
    ));
    assert_eq!(
        verify_process_identity(&identity).unwrap(),
        ProcessVerification::Exited
    );
    assert_eq!(worker_preparation_state(&recovered, &grant).2, revision + 1);
    assert_eq!(invocation_worker_resources(&recovered, &grant), resources);
    recovered.shutdown().unwrap();
}

fn assert_worker_recovery_rolls_back_on_outbox_error(
    store: &UnleashStore,
    grant: &AdmissionCommit,
    previous: DaemonMode,
    trigger: Uuid,
) {
    let attempts_before = worker_preparation_state(store, grant);
    let resources_before = invocation_worker_resources(store, grant);
    let outbox_before = reconciliation_outbox_count(store);
    reject_reconciliation_delivery(store);
    let error = classify_and_finish_recovery(store, previous, trigger).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("fixture_reconciliation_delivery_aborted")
    );
    assert_eq!(store.daemon_state().unwrap().mode, DaemonMode::Recovering);
    assert_eq!(worker_preparation_state(store, grant), attempts_before);
    assert_eq!(invocation_worker_resources(store, grant), resources_before);
    assert_eq!(reconciliation_outbox_count(store), outbox_before);
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute_batch("DROP TRIGGER fixture_refuse_reconciliation_delivery")?;
            Ok(())
        })
        .unwrap();
}

fn assert_historical_worker_event(
    store: &UnleashStore,
    grant: &AdmissionCommit,
    start: unleash_protocol::EventCursor,
    trigger: Uuid,
    events: &[unleash_protocol::EventEnvelope],
) {
    let mut reconciliation = events
        .iter()
        .filter(|event| event.event_type == "InvocationReconciling");
    let event = reconciliation.next().unwrap();
    assert!(reconciliation.next().is_none());
    assert_eq!(event.aggregate_id, grant.unit.id());
    assert_eq!(event.causation.id, trigger.to_string());
    assert_eq!(
        event.payload.get("start_sequence"),
        Some(&serde_json::json!(start.sequence))
    );
    assert_eq!(
        event.payload.get("attempt_id"),
        Some(&serde_json::json!(grant.attempt_id.unwrap()))
    );
    assert_eq!(
        event.payload.get("reservation_id"),
        Some(&serde_json::json!(grant.reservation_id))
    );
    assert!(!event.payload.contains_key("process_exit_observed"));
    let destinations = store
        .read_snapshot(|conn, _| {
            let mut statement = conn
                .prepare("SELECT destination FROM outbox WHERE event_id=?1 ORDER BY destination")?;
            let rows =
                statement.query_map([event.event_id.to_string()], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap();
    assert_eq!(destinations, ["clients", "harness"]);
}

#[test]
fn t_f003_006_restart_quarantines_unattributed_invocations() {
    let data = local_test_dir("f003-legacy-invocation-recovery").unwrap();
    let store = worker_test_store(data.path());
    seed(&store, false);
    let id = unit(1).id().to_string();
    let foreign = unit(2).id().to_string();
    store.execute_in_writer_tx(WriteCriticality::Critical, {
        let id = id.clone();
        let foreign = foreign.clone();
        move |tx, _| {
            assert_eq!(tx.execute(
                "UPDATE invocations SET state='running' WHERE invocation_id IN (?1,?2)",
                rusqlite::params![id, foreign],
            )?, 2);
            tx.execute(
                "INSERT INTO invocation_attempts(attempt_id,invocation_id,ordinal,authority_generation,
                    daemon_epoch,fence,state,started_at_ms) VALUES(?1,?2,1,?3,1,1,'running',0)",
                rusqlite::params![Uuid::from_u128(990_001).to_string(), foreign, Uuid::from_u128(990_002).to_string()],
            )?;
            Ok(())
        }
    }).unwrap();
    let (previous, trigger) = enter_worker_recovery(&store);
    let (_, events) = classify_and_finish_recovery(&store, previous, trigger).unwrap();
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "InvocationReconciling")
    );
    let (state, count): (String, i64) = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT state,(SELECT COUNT(*) FROM invocation_attempts WHERE invocation_id=?1)
         FROM invocations WHERE invocation_id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(state, "reconciling");
    assert_eq!(count, 0);
    let foreign_states: (String, String) = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
        "SELECT i.state,t.state FROM invocations i JOIN invocation_attempts t USING(invocation_id)
         WHERE i.invocation_id=?1",
        [foreign],
        |row| Ok((row.get(0)?,row.get(1)?)),
    )?)
        })
        .unwrap();
    assert_eq!(
        foreign_states,
        ("reconciling".to_owned(), "running".to_owned())
    );
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        0
    );
    Arc::try_unwrap(store).ok().unwrap().shutdown().unwrap();
}
