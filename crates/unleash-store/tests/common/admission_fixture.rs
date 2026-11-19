//! Synthetic durable inputs shared by the Store and daemon admission tests.

use unleash_domain::{InvocationId, PoolId, ProviderProbeId, ReplayWorkerId, Sha256Digest};
use unleash_store::{
    AdmissionEligibilityObservation, AdmissionUnit, AdmissionUrgency, ExecutionPoolParams,
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreError, UnleashStore,
    WriteCriticality,
};
use uuid::Uuid;

pub(crate) fn next_evidence() -> Uuid {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(10000);
    Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ))
}

#[path = "provider_probe.rs"]
mod probe_bench;

pub(crate) const POOL: PoolId = PoolId::from_u128(1);
pub(crate) const PROJECT: Uuid = Uuid::from_u128(100);
pub(crate) const OTHER_PROJECT: Uuid = Uuid::from_u128(101);

pub(crate) fn seed(store: &UnleashStore, include_control: bool) -> Sha256Digest {
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

pub(crate) fn seed_invocation(
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

pub(crate) fn unit(n: u128) -> AdmissionUnit {
    match n {
        3 | 4 => AdmissionUnit::Replay(ReplayWorkerId::from_u128(n)),
        5 | 6 => AdmissionUnit::Probe(ProviderProbeId::from_u128(n)),
        _ => AdmissionUnit::Invocation(InvocationId::from_u128(n)),
    }
}

pub(crate) fn observe(store: &UnleashStore, object: Sha256Digest) {
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
