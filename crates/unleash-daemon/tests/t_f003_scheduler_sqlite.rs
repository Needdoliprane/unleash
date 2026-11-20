//! Exercise the daemon scheduler against the real `SQLite` writer with synthetic executors.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Synthetic test fixtures and direct policy assertions"
)]

use std::path::PathBuf;
use std::sync::Arc;

use unleash_daemon::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use unleash_daemon::worker_launch::{WorkerLaunchError, prepare_invocation_worker};
use unleash_domain::{InvocationId, ReservationId, Sha256Digest};
use unleash_platform::{ProcessVerification, verify_process_identity};
use unleash_scheduler::{CapacityWait, CheckpointEligibility, CheckpointRequest, Decision};
use unleash_store::{
    AdmissionCommit, AdmissionOutcome, AdmissionUnit, PrepareObjectRequest,
    PublishObjectReferenceRequest, StoreConfig, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

#[path = "../../unleash-store/tests/common/admission_fixture.rs"]
mod admission_fixture;
use admission_fixture::{OTHER_PROJECT, POOL, observe, seed, seed_invocation, unit};

fn set_fixture_order(store: &UnleashStore) {
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            for index in 10_000_u128..10_020 {
                tx.execute(
                    "INSERT INTO agents(agent_id,level,project_id,state,revision)
                     VALUES(?1,'N+1',?2,'eligible',0)",
                    rusqlite::params![
                        Uuid::from_u128(index).to_string(),
                        OTHER_PROJECT.to_string()
                    ],
                )?;
            }
            for index in [1_u128, 2, 3, 4, 5, 6, 8, 9] {
                let unit = unit(index);
                let statement = match unit {
                    AdmissionUnit::Invocation(_) => {
                        "UPDATE invocations SET queued_at_ms=?1 WHERE invocation_id=?2"
                    }
                    AdmissionUnit::Replay(_) => {
                        "UPDATE replay_workers SET queued_at_ms=?1 WHERE replay_worker_id=?2"
                    }
                    AdmissionUnit::Probe(_) => {
                        "UPDATE provider_probes SET queued_at_ms=?1 WHERE probe_id=?2"
                    }
                };
                assert_eq!(
                    tx.execute(
                        statement,
                        rusqlite::params![i64::try_from(index).unwrap(), unit.id().to_string()]
                    )?,
                    1
                );
            }
            Ok(())
        })
        .unwrap();
}

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

fn admit_worker_fixture(store: &UnleashStore) -> (AdmissionCommit, Sha256Digest) {
    let eligibility = seed(store, false);
    set_fixture_order(store);
    let bytes = br#"{"schema_version":1,"entries":[],"fixture":"owned-worker"}"#;
    let digest = Sha256Digest::from_bytes(bytes);
    let object = store
        .prepare_object(PrepareObjectRequest {
            object_hash: digest,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "fixture:worker-manifest".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&object, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            object_hash: digest,
            pin_id: object.pin_id,
            object_generation: object.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "fixture:worker-manifest".to_owned(),
            source_field: "manifest".to_owned(),
        })
        .unwrap();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE context_manifests SET content_hash=?1 WHERE manifest_id=
                     (SELECT context_manifest_id FROM invocations WHERE invocation_id=?2)",
                    rusqlite::params![digest.to_string(), unit(1).id().to_string()],
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    observe(store, eligibility);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let mut plan = ExecutionSchedulingInputs::default();
    plan.budgets.extend(
        snapshot
            .queued
            .iter()
            .map(|candidate| (candidate.unit, vec![])),
    );
    let (outcome, _) = schedule_execution(store, POOL, snapshot.token, plan).unwrap();
    let AdmissionOutcome::Admitted(grant) = outcome else {
        panic!("expected a committed invocation admission");
    };
    assert_eq!(grant.unit, unit(1));
    (*grant, digest)
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

fn worker_test_executable() -> PathBuf {
    let executable = PathBuf::from(std::env::var_os("UNLEASH_TEST_AGENT_WORKER_BIN").unwrap());
    assert!(executable.is_absolute() && executable.is_file());
    executable
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
