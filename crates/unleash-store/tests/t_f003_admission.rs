//! T-F003-003/004/005/006/009: admission `SQLite`, sans réseau ni modèle.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Fixtures et assertions synthétiques de contrat, jamais du code produit"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use unleash_domain::{
    InvocationId, ProviderProbeId, ReplayWorkerId, Revision, Sha256Digest, WaitReason,
};
use unleash_domain::{PoolId, ReservationId};
use unleash_store::{
    AdmissionCommit, AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome,
    AdmissionUnit, AdmissionUrgency, ExecutionCheckpointObservation, PrepareObjectRequest,
    PublishObjectReferenceRequest, StoreError,
};
use unleash_store::{ExecutionPoolParams, StoreConfig, UnleashStore, WriteCriticality};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

fn next_evidence() -> Uuid {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(10000);
    Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ))
}

#[path = "common/provider_probe.rs"]
mod probe_bench;

const POOL: PoolId = PoolId::from_u128(1);
const PROJECT: Uuid = Uuid::from_u128(100);
const OTHER_PROJECT: Uuid = Uuid::from_u128(101);

fn seed(store: &UnleashStore, include_control: bool) -> Sha256Digest {
    // The fixture observer explicitly reports a synthetic, model-free executor.
    // It does not claim to qualify process-host or a real model connection.
    let bytes = b"synthetic admission bench observation v1; no network or OS executor";
    let object = Sha256Digest::from_bytes(bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: object,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "test:admission-bench".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: object,
            object_generation: handle.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "test:admission-bench".to_owned(),
            source_field: "fixture".to_owned(),
        })
        .unwrap();
    store
        .register_principal("admission-bench", "system", "test:admission-bench")
        .unwrap();
    store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id: POOL,
            ..ExecutionPoolParams::default()
        })
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,move |tx,now|{
        for project in [PROJECT,OTHER_PROJECT] {tx.execute("INSERT INTO projects(project_id,name,weight,state,revision,created_at_ms) VALUES(?1,'fixture',1,'active',0,?2)",rusqlite::params![project.to_string(),now.as_i64()])?;}
        tx.execute("INSERT INTO missions(mission_id,project_id,owner_kind,owner_user_id,completion_policy,state,revision) VALUES('mission',?1,'user','admission-bench','mechanical','active',0)",[PROJECT.to_string()])?;
        tx.execute("INSERT INTO workflow_versions(workflow_version_id,schema_version,content_hash,created_at_ms) VALUES('workflow',1,?1,?2)",rusqlite::params![object.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO workflow_runs(run_id,mission_id,workflow_version_id,selection_reason,effective_bundle_hash,state,revision,started_at_ms) VALUES('00000000-0000-0000-0000-0000000000c8','mission','workflow','fixture',?1,'running',0,?2)",rusqlite::params![object.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO command_receipts(actor_id,command_id,authority_generation,receipt_id,request_id,command_type,command_digest,aggregate_id,state,response_state,response_json,response_retained_until_ms,committed_at_ms) VALUES('test:admission-bench','fixture',?1,'fixture-receipt','fixture','TestProviderProfile','fixture','fixture','accepted','full','{}',?2,?3)",rusqlite::params![daemon.authority_generation.to_string(),now.as_i64()+86_400_000,now.as_i64()])?;
        tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,source_kind,created_at_ms) VALUES('profile','fixture','local_only','zero','user_declared',?1)",[now.as_i64()])?;
        for n in [1_u128,2,8,9] {
            seed_invocation(tx,n,PROJECT,false,object,&daemon,now)?;
        }
        if include_control {seed_invocation(tx,7,OTHER_PROJECT,true,object,&daemon,now)?;}
        for n in [3_u128,4] {
            let id=Uuid::from_u128(n).to_string();
            tx.execute("INSERT INTO step_instances(step_id,run_id,definition_node_id,iteration_vector,step_attempt,state,revision) VALUES(?1,'00000000-0000-0000-0000-0000000000c8',?1,'[]',1,'ready',0)",[&id])?;
            tx.execute("INSERT INTO replay_workers(replay_worker_id,run_id,step_id,attempt_ordinal,authority_generation,daemon_epoch,fence,state,admission_class,revision,queued_at_ms) VALUES(?1,'00000000-0000-0000-0000-0000000000c8',?1,1,?2,?3,0,'queued','work',0,?4)",rusqlite::params![id,daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        }
        Ok(())
    }).unwrap();
    for n in [5_u128, 6] {
        probe_bench::seed_admission_probe(
            store,
            ProviderProbeId::from_u128(n),
            unleash_domain::ProjectId::from_uuid(PROJECT),
            object,
        );
    }
    object
}

fn seed_invocation(
    tx: &rusqlite::Transaction<'_>,
    n: u128,
    project: Uuid,
    control: bool,
    object: Sha256Digest,
    daemon: &unleash_store::DaemonStateRow,
    now: unleash_domain::TimestampMs,
) -> Result<(), StoreError> {
    let id = Uuid::from_u128(n).to_string();
    let agent = Uuid::from_u128(1000 + n).to_string();
    let session = Uuid::from_u128(2000 + n).to_string();
    let manifest = Uuid::from_u128(3000 + n).to_string();
    let event = Uuid::from_u128(4000 + n).to_string();
    tx.execute("INSERT INTO agents(agent_id,level,project_id,state,revision) VALUES(?1,?2,?3,'eligible',0)",rusqlite::params![agent,if control{"N+1"}else{"N"},project.to_string()])?;
    tx.execute("INSERT INTO sessions(session_id,agent_id,revision,state,created_at_ms) VALUES(?1,?2,0,'open',?3)",rusqlite::params![session,agent,now.as_i64()])?;
    tx.execute("INSERT INTO context_manifests(manifest_id,session_id,base_session_revision,content_hash,schema_version,created_at_ms) VALUES(?1,?2,0,?3,1,?4)",rusqlite::params![manifest,session,object.to_string(),now.as_i64()])?;
    tx.execute("INSERT INTO effective_bundles(bundle_hash,schema_version,role_version_id,requested_model_profile_version_id,allowed_model_policy_hash,initial_context_manifest_id,scope_id,scope_revision,content_object_hash) VALUES(?1,1,'role','model',?2,?3,'fixture',0,?2)",rusqlite::params![id,object.to_string(),manifest])?;
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,correlation_id) VALUES(?1,?2,'invocation',?3,1,0,'InvocationQueued',1,?4,'test:admission-bench','test:admission-bench',?5,?5,'{}','full',?6,'restricted','external',?1,?1)",rusqlite::params![event,daemon.stream_id.to_string(),id,project.to_string(),now.as_i64(),object.to_string()])?;
    tx.execute("INSERT INTO invocations(invocation_id,agent_id,session_id,trigger_event_id,effective_bundle_hash,context_manifest_id,state,admission_class,revision,created_at_ms) VALUES(?1,?2,?3,?4,?1,?5,'queued',?6,0,?7)",rusqlite::params![id,agent,session,event,manifest,if control{"control"}else{"work"},now.as_i64()])?;
    Ok(())
}

fn unit(n: u128) -> AdmissionUnit {
    match n {
        3 | 4 => AdmissionUnit::Replay(ReplayWorkerId::from_u128(n)),
        5 | 6 => AdmissionUnit::Probe(ProviderProbeId::from_u128(n)),
        _ => AdmissionUnit::Invocation(InvocationId::from_u128(n)),
    }
}

fn observe(store: &UnleashStore, object: Sha256Digest) {
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    for candidate in snapshot.queued {
        store
            .record_admission_eligibility(AdmissionEligibilityObservation {
                evidence_id: next_evidence(),
                observer_principal_id: "admission-bench".to_owned(),
                object_hash: object,
                unit: candidate.unit,
                unit_revision: candidate.revision,
                urgency: if candidate.class == unleash_domain::AdmissionClass::Control {
                    AdmissionUrgency::UrgentControl
                } else {
                    AdmissionUrgency::Ordinary
                },
                source_revision: snapshot.source_revision,
                authority_generation: snapshot.authority_generation,
                daemon_epoch: snapshot.daemon_epoch,
                valid_until: unleash_domain::TimestampMs::new(i64::MAX),
            })
            .unwrap();
    }
}

fn proposal(unit: AdmissionUnit) -> AdmissionDecision {
    AdmissionDecision::Admit {
        unit,
        deficit_deltas: BTreeMap::from([(Some(unleash_domain::ProjectId::from_uuid(PROJECT)), 0)]),
        consecutive_controls: 0,
        budgets: vec![],
    }
}

fn admit(store: &UnleashStore, object: Sha256Digest, n: u128) -> AdmissionCommit {
    observe(store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    match store
        .commit_execution_admission(POOL, snapshot.token, move |_| Ok(proposal(unit(n))))
        .unwrap()
    {
        AdmissionOutcome::Admitted(commit) => *commit,
        other @ AdmissionOutcome::Unchanged => panic!("expected admission: {other:?}"),
    }
}

#[test]
fn t_f003_003_five_mixed_units_share_the_sqlite_pool() {
    let dir = local_test_dir("f003-mixed").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    for n in 1..=5 {
        let commit = admit(&store, object, n);
        assert_eq!(commit.unit, unit(n));
        assert_eq!(
            store.execution_pool_state(POOL).unwrap().current_occupation,
            i64::try_from(n).unwrap()
        );
    }
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(snapshot.active.len(), 5);
    observe(&store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    assert!(matches!(
        store.commit_execution_admission(POOL, snapshot.token, |_| Ok(proposal(unit(6)))),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        5
    );
    let count:i64=store.read_snapshot(|conn,_|Ok(conn.query_row("SELECT COUNT(*) FROM events WHERE event_type IN ('InvocationAdmitted','ReplayWorkerAdmitted','ProviderProbeAdmitted')",[],|row|row.get(0))?)).unwrap();
    assert_eq!(count, 5);
}

#[test]
#[expect(
    clippy::print_stderr,
    reason = "Mesures locales du cas volumineux demandées pour la remise, visibles avec --nocapture"
)]
fn t_f003_005_control_after_more_than_4096_ineligible_candidates_is_admitted() {
    let dir = local_test_dir("f003-large-queue").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let daemon = store.daemon_state().unwrap();
    let seeded_at = std::time::Instant::now();
    let control = unit(20_000);
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            // Equal timestamps force the durable ID tie-break. All 4096 work
            // entries sort before the control, and none has eligibility proof.
            for number in 10_000..14_096 {
                seed_invocation(tx, number, PROJECT, false, object, &daemon, now)?;
            }
            seed_invocation(tx, 20_000, OTHER_PROJECT, true, object, &daemon, now)
        })
        .unwrap();
    let seed_elapsed = seeded_at.elapsed();
    let read_at = std::time::Instant::now();
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let snapshot_elapsed = read_at.elapsed();
    assert_eq!(snapshot.queued.len(), 4105);
    let control_index = snapshot
        .queued
        .iter()
        .position(|candidate| candidate.unit == control)
        .unwrap();
    assert!(control_index > 4096);
    assert!(
        snapshot
            .queued
            .iter()
            .all(|candidate| candidate.eligibility_evidence.is_none())
    );
    let candidate = snapshot.queued.get(control_index).unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: next_evidence(),
            observer_principal_id: "admission-bench".to_owned(),
            object_hash: object,
            unit: control,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: unleash_domain::TimestampMs::new(i64::MAX),
            urgency: AdmissionUrgency::UrgentControl,
        })
        .unwrap();
    let eligible = store.execution_admission_snapshot(POOL).unwrap();
    let commit_at = std::time::Instant::now();
    let outcome = store
        .commit_execution_admission(POOL, eligible.token, move |current| {
            assert_eq!(current.queued.len(), 4105);
            let admissible: Vec<_> = current
                .queued
                .iter()
                .filter(|candidate| candidate.eligibility_evidence.is_some())
                .map(|candidate| candidate.unit)
                .collect();
            assert_eq!(admissible, vec![control]);
            Ok(AdmissionDecision::Admit {
                unit: control,
                deficit_deltas: BTreeMap::from([(
                    Some(unleash_domain::ProjectId::from_uuid(OTHER_PROJECT)),
                    0,
                )]),
                consecutive_controls: 0,
                budgets: vec![],
            })
        })
        .unwrap();
    let commit_elapsed = commit_at.elapsed();
    assert!(matches!(outcome, AdmissionOutcome::Admitted(ref commit) if commit.unit == control));
    let after = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(after.queued.len(), 4104);
    assert_eq!(after.pool.current_occupation, 1);
    assert_eq!(after.pool.effective_limit, 5);
    assert!(
        after
            .queued
            .iter()
            .all(|candidate| candidate.unit != control)
    );
    eprintln!(
        "large_queue candidates=4105 control_index={control_index} seed_ms={} snapshot_ms={} commit_ms={} occupation=1 effective_limit=5",
        seed_elapsed.as_millis(),
        snapshot_elapsed.as_millis(),
        commit_elapsed.as_millis()
    );
}

#[test]
fn t_f003_003_same_last_slot_proposal_commits_once() {
    let dir = local_test_dir("f003-race").unwrap();
    let store = Arc::new(UnleashStore::open(StoreConfig::new(dir.path())).unwrap());
    let object = seed(&store, false);
    for n in 1..=4 {
        admit(&store, object, n);
    }
    // A second project in the work queue makes the deficit update observable.
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute(
                "UPDATE agents SET project_id=?1 WHERE agent_id=?2",
                rusqlite::params![OTHER_PROJECT.to_string(), Uuid::from_u128(1008).to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    observe(&store, object);
    let revision = store.execution_admission_snapshot(POOL).unwrap().token;
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.commit_execution_admission(POOL, revision, |_| {
                    Ok(AdmissionDecision::Admit {
                        unit: unit(5),
                        deficit_deltas: BTreeMap::from([
                            (Some(unleash_domain::ProjectId::from_uuid(PROJECT)), -1),
                            (Some(unleash_domain::ProjectId::from_uuid(OTHER_PROJECT)), 1),
                        ]),
                        consecutive_controls: 0,
                        budgets: vec![],
                    })
                })
            })
        })
        .collect();
    let outcomes: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(StoreError::RevisionConflict { .. })))
            .count(),
        1
    );
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(snapshot.pool.current_occupation, 5);
    assert_eq!(
        snapshot
            .fairness
            .get(&Some(unleash_domain::ProjectId::from_uuid(PROJECT)))
            .unwrap()
            .1,
        -1
    );
    assert_eq!(
        snapshot
            .fairness
            .get(&Some(unleash_domain::ProjectId::from_uuid(OTHER_PROJECT)))
            .unwrap()
            .1,
        1
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Oracle complet demande, absence de preuves, checkpoint, libération et nouvelle incarnation"
)]
fn t_f003_004_checkpoint_observations_precede_release_and_new_attempt() {
    let dir = local_test_dir("f003-checkpoint").unwrap();
    let config = StoreConfig::new(dir.path());
    let clock_uncertain = Arc::clone(&config.clock_uncertain);
    let store = UnleashStore::open(config).unwrap();
    let object = seed(&store, true);
    let target = admit(&store, object, 1);
    for n in 2..=5 {
        admit(&store, object, n);
    }
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            tx.execute(
                "UPDATE invocations SET state='running',revision=revision+1 WHERE invocation_id=?1",
                [unit(1).id().to_string()],
            )?;
            tx.execute(
                "UPDATE invocation_attempts SET state='running' WHERE attempt_id=?1",
                [target.attempt_id.unwrap().to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    let observation = ExecutionCheckpointObservation {
        reservation_id: target.reservation_id,
        unit: unit(1),
        unit_revision: Revision::new(2),
        lease_id: target.lease.lease_id,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        fence: target.lease.fence,
        boot_identity: daemon.boot_identity,
        checkpoint_published: next_evidence(),
        local_model_quiescent: next_evidence(),
        continuation_neutralized: next_evidence(),
        resources_disposed: next_evidence(),
    };
    observe(&store, object);
    clock_uncertain.store(true, std::sync::atomic::Ordering::Release);
    assert!(
        store
            .request_execution_checkpoint(InvocationId::from_u128(7), observation.continuation())
            .is_err()
    );
    clock_uncertain.store(false, std::sync::atomic::Ordering::Release);
    store
        .request_execution_checkpoint(InvocationId::from_u128(7), observation.continuation())
        .unwrap();
    assert!(
        store
            .request_execution_checkpoint(InvocationId::from_u128(7), observation.continuation())
            .is_err()
    );
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        5
    );
    assert!(
        store
            .complete_execution_checkpoint(target.reservation_id, WaitReason::WaitingChildren)
            .is_err()
    );
    assert!(
        store
            .record_execution_checkpoint(observation.clone())
            .is_err()
    );
    // The bench is the named synthetic observer. Each fact has separate durable
    // provenance; the Store does not turn these fixtures into OS qualification.
    let proofs = [
        ("checkpoint-published", observation.checkpoint_published),
        ("local-model-quiescent", observation.local_model_quiescent),
        (
            "continuation-neutralized",
            observation.continuation_neutralized,
        ),
        ("resources-disposed", observation.resources_disposed),
    ];
    let insert = observation.clone();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{for(role,id)in proofs{tx.execute("INSERT INTO evidence(evidence_id,kind,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,'admission-bench',?4,1,'available')",rusqlite::params![id.to_string(),insert.evidence_binding(role),object.to_string(),now.as_i64()])?;}Ok(())}).unwrap();
    store
        .record_execution_checkpoint(observation.clone())
        .unwrap();
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        5
    );
    store
        .complete_execution_checkpoint(target.reservation_id, WaitReason::WaitingChildren)
        .unwrap();
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        4
    );
    assert!(
        store
            .record_execution_checkpoint(observation.clone())
            .is_err()
    );
    store
        .requeue_checkpointed_execution(unit(1), Revision::new(3))
        .unwrap();
    let resumed = admit(&store, object, 1);
    assert_ne!(resumed.reservation_id, target.reservation_id);
    assert_ne!(resumed.attempt_id, target.attempt_id);
    assert!(resumed.lease.fence > target.lease.fence);
    assert!(matches!(
        store.record_execution_checkpoint(observation),
        Err(StoreError::StaleFence { .. })
    ));
}

#[test]
fn t_f003_006_lease_from_another_lineage_is_never_a_checkpoint_capability() {
    let dir = local_test_dir("f003-lease-lineage").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let target = admit(&store, object, 1);
    let daemon = store.daemon_state().unwrap();
    let observation = ExecutionCheckpointObservation {
        reservation_id: target.reservation_id,
        unit: unit(1),
        unit_revision: target.revision,
        lease_id: target.lease.lease_id,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        fence: target.lease.fence,
        boot_identity: daemon.boot_identity,
        checkpoint_published: next_evidence(),
        local_model_quiescent: next_evidence(),
        continuation_neutralized: next_evidence(),
        resources_disposed: next_evidence(),
    };
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,_|{
        tx.execute("INSERT INTO resource_fences(authority_generation,resource_kind,resource_key,last_fence) SELECT 'foreign-lineage',resource_kind,resource_key,fence FROM resource_leases WHERE lease_id=?1",[target.lease.lease_id.to_string()])?;
        tx.execute("UPDATE resource_leases SET authority_generation='foreign-lineage' WHERE lease_id=?1",[target.lease.lease_id.to_string()])?;Ok(())}).unwrap();
    assert!(matches!(
        store.record_execution_checkpoint(observation),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
}

#[test]
fn t_f003_006_current_lease_cannot_substitute_for_an_older_replay_activation() {
    let dir = local_test_dir("f003-activation-fence").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let target = admit(&store, object, 3);
    let daemon = store.daemon_state().unwrap();
    let replacement = store
        .revoke_and_reassign_lease(
            target.lease.lease_id,
            unleash_store::AcquireLeaseRequest {
                resource_kind: target.lease.resource_kind,
                resource_key: target.lease.resource_key,
                owner_type: target.lease.owner_type,
                owner_id: target.lease.owner_id,
                ttl_ms: 60_000,
            },
        )
        .unwrap();
    let observation = ExecutionCheckpointObservation {
        reservation_id: target.reservation_id,
        unit: unit(3),
        unit_revision: target.revision,
        lease_id: replacement.lease_id,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        fence: replacement.fence,
        boot_identity: daemon.boot_identity,
        checkpoint_published: next_evidence(),
        local_model_quiescent: next_evidence(),
        continuation_neutralized: next_evidence(),
        resources_disposed: next_evidence(),
    };
    assert!(matches!(
        store.record_execution_checkpoint(observation),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
}

#[test]
fn t_f003_003_v007_migration_keeps_unknown_owners_held_without_fabricating_proofs() {
    use unleash_store::{BackupRetentionTier, PrepareBackupRequest};
    let dir = local_test_dir("f003-v007-upgrade").unwrap();
    let mut legacy = StoreConfig::new(dir.path());
    legacy.max_supported_schema_version = 7;
    let store = UnleashStore::open(legacy).unwrap();
    store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    let reservation = ReservationId::from_u128(9000);
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{tx.execute("INSERT INTO capacity_reservations(reservation_id,pool_id,owner_type,owner_id,admission_class,amount,state,authority_generation,daemon_epoch,created_at_ms) VALUES(?1,?2,'replay_worker','historical-unknown','work',1,'reconciling',?3,?4,?5)",rusqlite::params![reservation.to_string(),POOL.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;Ok(())}).unwrap();
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("migration-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let config = StoreConfig::new(dir.path());
    assert!(matches!(
        UnleashStore::open(config.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let store = UnleashStore::open_with_migration_backup(config, Some(backup.backup_id)).unwrap();
    assert_eq!(
        store.daemon_state().unwrap().schema_version,
        unleash_store::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        1
    );
    assert!(store.execution_admission_snapshot(POOL).is_err());
    assert!(store.release_capacity_reservation(reservation).is_err());
    let proofs: (i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT COUNT(*) FROM admission_eligibility_observations",
                    [],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM execution_checkpoint_observations",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(proofs, (0, 0));
}

#[test]
fn t_f003_006_replay_checkpoint_keeps_cursor_and_increments_activation_ordinal() {
    let dir = local_test_dir("f003-replay-checkpoint").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let target = admit(&store, object, 3);
    let daemon = store.daemon_state().unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{tx.execute("UPDATE replay_workers SET state='running',revision=revision+1 WHERE replay_worker_id=?1",[unit(3).id().to_string()])?;Ok(())}).unwrap();
    let observation = ExecutionCheckpointObservation {
        reservation_id: target.reservation_id,
        unit: unit(3),
        unit_revision: Revision::new(2),
        lease_id: target.lease.lease_id,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        fence: target.lease.fence,
        boot_identity: daemon.boot_identity,
        checkpoint_published: next_evidence(),
        local_model_quiescent: next_evidence(),
        continuation_neutralized: next_evidence(),
        resources_disposed: next_evidence(),
    };
    let insert = observation.clone();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{for(role,id)in[("checkpoint-published",insert.checkpoint_published),("local-model-quiescent",insert.local_model_quiescent),("continuation-neutralized",insert.continuation_neutralized),("resources-disposed",insert.resources_disposed)]{tx.execute("INSERT INTO evidence(evidence_id,kind,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,'admission-bench',?4,1,'available')",rusqlite::params![id.to_string(),insert.evidence_binding(role),object.to_string(),now.as_i64()])?;}Ok(())}).unwrap();
    store.record_execution_checkpoint(observation).unwrap();
    store
        .complete_execution_checkpoint(target.reservation_id, WaitReason::Timer)
        .unwrap();
    store
        .requeue_checkpointed_execution(unit(3), Revision::new(3))
        .unwrap();
    let resumed = admit(&store, object, 3);
    assert_ne!(resumed.reservation_id, target.reservation_id);
    assert!(resumed.lease.fence > target.lease.fence);
    let (ordinal, checkpoint, events): (i64, String, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT attempt_ordinal FROM replay_workers WHERE replay_worker_id=?1",
                    [unit(3).id().to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT checkpoint_hash FROM replay_workers WHERE replay_worker_id=?1",
                    [unit(3).id().to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type='ReplayWorkerCheckpointed'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(ordinal, 2);
    assert_eq!(checkpoint, object.to_string());
    assert_eq!(events, 1);
}

#[test]
fn t_f003_004_execution_release_requires_checkpoint_and_neutralization() {
    let dir = local_test_dir("f003-release-proof").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let pool_id = PoolId::from_u128(1);
    store
        .configure_execution_pool(ExecutionPoolParams {
            pool_id,
            ..ExecutionPoolParams::default()
        })
        .unwrap();
    let reservation_id = ReservationId::from_u128(10);
    let daemon = store.daemon_state().unwrap();
    // Intentionally incomplete historical row: neither an owner nor a checkpoint
    // is evidence that the former executor has ceased to act.
    store.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
        tx.execute("INSERT INTO capacity_reservations(reservation_id,pool_id,owner_type,owner_id,admission_class,amount,state,authority_generation,daemon_epoch,created_at_ms) VALUES (?1,?2,'provider_probe','unproven','work',1,'reconciling',?3,?4,?5)", rusqlite::params![reservation_id.to_string(), pool_id.to_string(), daemon.authority_generation.to_string(), i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(), now.as_i64()]).unwrap();
        Ok(())
    }).unwrap();
    assert!(
        store.release_capacity_reservation(reservation_id).is_err(),
        "a naked release cannot attest neutralization"
    );
    assert_eq!(
        store
            .execution_pool_state(pool_id)
            .unwrap()
            .current_occupation,
        1
    );
}

#[test]
fn t_f003_005_observation_changes_invalidate_the_complete_snapshot_token() {
    let dir = local_test_dir("f003-observation-cas").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let before = store.execution_admission_snapshot(POOL).unwrap();
    assert!(
        store
            .commit_execution_admission(POOL, before.token, |_| Ok(proposal(unit(1))))
            .is_err()
    );
    observe(&store, object);
    let after = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(before.source_revision, after.source_revision);
    assert_ne!(before.token, after.token);
    assert!(matches!(
        store.commit_execution_admission(POOL, before.token, |_| Ok(proposal(unit(1)))),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(after.pool.current_occupation, 0);
}

#[test]
fn t_f003_006_stale_status_and_epoch_never_reserve() {
    let dir = local_test_dir("f003-stale-inputs").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    observe(&store, object);
    let before = store.execution_admission_snapshot(POOL).unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{tx.execute("UPDATE invocations SET state='cancelled',revision=revision+1 WHERE invocation_id=?1",[unit(1).id().to_string()])?;Ok(())}).unwrap();
    assert!(matches!(
        store.commit_execution_admission(POOL, before.token, |_| Ok(proposal(unit(1)))),
        Err(StoreError::RevisionConflict { .. })
    ));
    let observation = AdmissionEligibilityObservation {
        evidence_id: next_evidence(),
        observer_principal_id: "admission-bench".to_owned(),
        object_hash: object,
        unit: unit(2),
        unit_revision: Revision::INITIAL,
        source_revision: store
            .execution_admission_snapshot(POOL)
            .unwrap()
            .source_revision,
        authority_generation: before.authority_generation,
        daemon_epoch: before.daemon_epoch,
        valid_until: unleash_domain::TimestampMs::new(i64::MAX),
        urgency: AdmissionUrgency::Ordinary,
    };
    let store = store.restart_same_lineage().unwrap();
    assert!(matches!(
        store.commit_execution_admission(POOL, before.token, |_| Ok(proposal(unit(2)))),
        Err(StoreError::StaleFence { .. })
    ));
    assert!(matches!(
        store.record_admission_eligibility(observation),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(
        store.execution_pool_state(POOL).unwrap().current_occupation,
        0
    );
}

#[test]
fn t_f003_005_expired_or_user_claims_do_not_become_eligibility() {
    let dir = local_test_dir("f003-expired-evidence").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    store
        .register_principal("client", "user", "test:client")
        .unwrap();
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let observation = AdmissionEligibilityObservation {
        evidence_id: next_evidence(),
        observer_principal_id: "client".to_owned(),
        object_hash: object,
        unit: unit(1),
        unit_revision: Revision::INITIAL,
        source_revision: snapshot.source_revision,
        authority_generation: snapshot.authority_generation,
        daemon_epoch: snapshot.daemon_epoch,
        valid_until: unleash_domain::TimestampMs::new(i64::MAX),
        urgency: AdmissionUrgency::Ordinary,
    };
    assert!(
        store
            .record_admission_eligibility(observation.clone())
            .is_err()
    );
    let mut expired = observation;
    expired.observer_principal_id = "admission-bench".to_owned();
    expired.valid_until = unleash_domain::TimestampMs::new(0);
    assert!(store.record_admission_eligibility(expired).is_err());
    assert!(
        store
            .execution_admission_snapshot(POOL)
            .unwrap()
            .queued
            .iter()
            .all(|c| c.eligibility_evidence.is_none())
    );
}

#[test]
fn t_f003_005_snapshot_expires_without_a_write_and_old_observation_never_revives() {
    let dir = local_test_dir("f003-timed-snapshot").unwrap();
    let clock = Arc::new(unleash_testkit::ManualClock::new(1_700_000_000_000));
    let mut config = StoreConfig::new(dir.path());
    config.clock = clock.clone();
    let store = UnleashStore::open(config).unwrap();
    let object = seed(&store, false);
    observe(&store, object);
    let before = store.execution_admission_snapshot(POOL).unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            // Lower than the first observation ID, at the SAME clock instant:
            // insertion order must win over UUID order.
            evidence_id: Uuid::from_u128(1),
            observer_principal_id: "admission-bench".to_owned(),
            object_hash: object,
            unit: unit(1),
            unit_revision: Revision::INITIAL,
            source_revision: before.source_revision,
            authority_generation: before.authority_generation,
            daemon_epoch: before.daemon_epoch,
            valid_until: unleash_domain::TimestampMs::new(1_700_000_000_001),
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    let expiring = store.execution_admission_snapshot(POOL).unwrap();
    clock.advance_ms(1);
    let after = store.execution_admission_snapshot(POOL).unwrap();
    assert_eq!(expiring.source_revision, after.source_revision);
    assert_ne!(expiring.token, after.token);
    assert!(
        after
            .queued
            .iter()
            .find(|c| c.unit == unit(1))
            .unwrap()
            .eligibility_evidence
            .is_none()
    );
    assert!(matches!(
        store.commit_execution_admission(POOL, expiring.token, |_| Ok(proposal(unit(2)))),
        Err(StoreError::RevisionConflict { .. })
    ));
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Oracle atomique : coût insuffisant, action étrangère puis réservation positive sans effet partiel"
)]
fn t_f003_009_budget_failure_rolls_back_attempt_lease_capacity_and_deficit() {
    use unleash_domain::{AccountId, ActionId, MicrosAmount};
    use unleash_store::{
        BudgetAccountCharge, BudgetReservationRequest, BudgetScopeType, PricingCheckStatus,
    };
    let dir = local_test_dir("f003-budget-rollback").unwrap();
    let store = UnleashStore::open(StoreConfig::new(dir.path())).unwrap();
    let object = seed(&store, false);
    let account = AccountId::from_u128(999);
    let action = ActionId::from_u128(999);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1),
        )
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO actions(action_id,origin_kind,replay_worker_id,run_id,step_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms) VALUES(?1,'replay',?2,'00000000-0000-0000-0000-0000000000c8',?2,'fixture_cost',?3,0,?4,?5,0,'prepared','observe_then_decide',0,?6)",rusqlite::params![action.to_string(),unit(3).id().to_string(),object.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;Ok(())}).unwrap();
    observe(&store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let result = store.commit_execution_admission(POOL, snapshot.token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit: unit(3),
            deficit_deltas: BTreeMap::from([(
                Some(unleash_domain::ProjectId::from_uuid(PROJECT)),
                0,
            )]),
            consecutive_controls: 0,
            budgets: vec![BudgetReservationRequest {
                action_id: action,
                model_id: "fixture".to_owned(),
                pricing_version: "zero".to_owned(),
                pricing_status: PricingCheckStatus::Valid,
                max_request_hash: object,
                charges: vec![BudgetAccountCharge {
                    account_id: account,
                    max_micros: MicrosAmount::new(2),
                }],
            }],
        })
    });
    assert!(matches!(result, Err(StoreError::BudgetExhausted { .. })));
    let counts: (i64, i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM capacity_reservations", [], |row| {
                    row.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM invocation_attempts", [], |row| {
                    row.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM resource_leases", [], |row| row.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM execution_fairness", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    assert_eq!(
        store.budget_account(account).unwrap().reserved_micros,
        MicrosAmount::ZERO
    );
    // An invocation may not charge an action belonging to the replay worker;
    // its tentatively allocated attempt must also roll back.
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let foreign = store.commit_execution_admission(POOL, snapshot.token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit: unit(1),
            deficit_deltas: BTreeMap::from([(
                Some(unleash_domain::ProjectId::from_uuid(PROJECT)),
                0,
            )]),
            consecutive_controls: 0,
            budgets: vec![BudgetReservationRequest {
                action_id: action,
                model_id: "fixture".to_owned(),
                pricing_version: "zero".to_owned(),
                pricing_status: PricingCheckStatus::Valid,
                max_request_hash: object,
                charges: vec![BudgetAccountCharge {
                    account_id: account,
                    max_micros: MicrosAmount::new(1),
                }],
            }],
        })
    });
    assert!(matches!(foreign, Err(StoreError::InvalidArgument { .. })));
    let attempts: i64 = store
        .read_snapshot(|conn, _| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM invocation_attempts", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(attempts, 0);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(5),
        )
        .unwrap();
    observe(&store, object);
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let funded = store
        .commit_execution_admission(POOL, snapshot.token, move |_| {
            Ok(AdmissionDecision::Admit {
                unit: unit(3),
                deficit_deltas: BTreeMap::from([(
                    Some(unleash_domain::ProjectId::from_uuid(PROJECT)),
                    0,
                )]),
                consecutive_controls: 0,
                budgets: vec![BudgetReservationRequest {
                    action_id: action,
                    model_id: "fixture".to_owned(),
                    pricing_version: "zero".to_owned(),
                    pricing_status: PricingCheckStatus::Valid,
                    max_request_hash: object,
                    charges: vec![BudgetAccountCharge {
                        account_id: account,
                        max_micros: MicrosAmount::new(2),
                    }],
                }],
            })
        })
        .unwrap();
    assert!(matches!(funded, AdmissionOutcome::Admitted(_)));
    assert_eq!(
        store.budget_account(account).unwrap().reserved_micros,
        MicrosAmount::new(2)
    );
}
