//! Exercise the daemon scheduler against the real `SQLite` writer with synthetic executors.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Synthetic test fixtures and direct policy assertions"
)]

use unleash_daemon::execution_scheduler::{ExecutionSchedulingInputs, schedule_execution};
use unleash_domain::{InvocationId, ReservationId};
use unleash_scheduler::{CapacityWait, CheckpointEligibility, CheckpointRequest, Decision};
use unleash_store::{
    AdmissionOutcome, AdmissionUnit, StoreConfig, StoreError, UnleashStore, WriteCriticality,
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
