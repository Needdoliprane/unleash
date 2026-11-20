//! Synthetic inputs shared by the daemon worker lifecycle tests.

use std::path::PathBuf;

use unleash_domain::Sha256Digest;
use unleash_store::{
    AdmissionCommit, AdmissionOutcome, AdmissionSnapshotToken, AdmissionUnit, PrepareObjectRequest,
    PublishObjectReferenceRequest, UnleashStore, WriteCriticality,
};
use uuid::Uuid;

use super::admission_fixture::{OTHER_PROJECT, POOL, observe, seed, unit};
use super::{ExecutionSchedulingInputs, schedule_execution};

pub(crate) fn set_fixture_order(store: &UnleashStore) {
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

pub(crate) fn prepare_worker_fixture(
    store: &UnleashStore,
) -> (
    AdmissionSnapshotToken,
    ExecutionSchedulingInputs,
    Sha256Digest,
) {
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
    (snapshot.token, plan, digest)
}

pub(crate) fn admit_worker_fixture(store: &UnleashStore) -> (AdmissionCommit, Sha256Digest) {
    let (token, plan, digest) = prepare_worker_fixture(store);
    let (outcome, _) = schedule_execution(store, POOL, token, plan).unwrap();
    let AdmissionOutcome::Admitted(grant) = outcome else {
        panic!("expected a committed invocation admission");
    };
    assert_eq!(grant.unit, unit(1));
    (*grant, digest)
}

pub(crate) fn worker_test_executable() -> PathBuf {
    let executable = PathBuf::from(std::env::var_os("UNLEASH_TEST_AGENT_WORKER_BIN").unwrap());
    assert!(executable.is_absolute() && executable.is_file());
    executable
}
