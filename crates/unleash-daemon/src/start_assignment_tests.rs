//! IPC qualification of the first task's published context and assignment.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "Assertions on isolated workflow fixtures"
)]

use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use serde_json::{Value, json};
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{
    ActorId, AgentId, CommandId, InvocationId, MissionId, RequestId, Revision, Sha256Digest,
};
use unleash_platform::{current_boot_identity, effective_uid};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState, canonical_sha256,
};
use unleash_store::{
    CallerAuthority, InitialContextPlan, PreparedTaskAssignment, StoreConfig, StoreError,
    UnleashStore,
};
use uuid::Uuid;

use crate::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};

#[path = "../../unleash-store/tests/support/workflow_run.rs"]
mod workflow_run;

const WORKER: AgentId = AgentId::from_u128(0x5980);
const MISSION: MissionId = MissionId::from_u128(0x5981);
const PROFILE: Uuid = Uuid::from_u128(0x5982);
const MANIFEST: Uuid = Uuid::from_u128(0x5983);

fn seed(data: &Path) -> (CommandEnvelope, CallerAuthority) {
    let mut config = StoreConfig::new(data);
    config.owner_uid = effective_uid();
    current_boot_identity()
        .unwrap()
        .as_str()
        .clone_into(&mut config.boot_identity);
    let actor = ActorId::new(format!("uid:{}", effective_uid())).unwrap();
    let (fixture, mut command, caller) = workflow_run::prepare(config, actor);
    let definition: String = fixture
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT content_hash FROM workflow_versions WHERE workflow_version_id=?1",
                [workflow_run::PUBLISHED],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    command.payload = serde_json::from_value(json!({
        "mission_id": MISSION, "source": {
            "kind":"workflow_version", "id":"workflow.merge-only", "version":1, "digest":definition
        },
        "purpose":"operational","selection_reason":"first assignment fixture",
    }))
    .unwrap();
    fixture.store.shutdown().unwrap();
    (command, caller)
}

fn command(
    base: &CommandEnvelope,
    id: u128,
    verb: &str,
    target: Uuid,
    revision: u64,
    payload: Value,
) -> CommandEnvelope {
    CommandEnvelope {
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        command_type: verb.to_owned(),
        aggregate_id: target,
        expected_revision: Revision::new(revision),
        payload: serde_json::from_value(payload).unwrap(),
        ..base.clone()
    }
}

async fn start(config: DaemonConfig) -> (DaemonHandle, UnleashClient) {
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(config).await.unwrap() else {
        panic!("isolated authority must start");
    };
    let client = UnleashClient::connect(ClientConfig::new(
        daemon.socket_path().to_owned(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    (daemon, client)
}

async fn prepare_task(client: &mut UnleashClient, create: &mut CommandEnvelope) {
    let mandate = client.send_command(&command(
        create, 0x5984, "AssignMission", MISSION.as_uuid(), 0,
        json!({"completion_policy":"mechanical","scopes":{"implementation":{"kind":"project","revision":1}}}),
    )).await.unwrap();
    let scope = mandate.result_ref.unwrap();
    create.payload.insert(
        "scope_ref".to_owned(),
        json!(scope.strip_prefix("mission-scope:").unwrap()),
    );
    client.send_command(create).await.unwrap();
    client
        .send_command(&command(
            create,
            0x5985,
            "StartRun",
            workflow_run::RUN.as_uuid(),
            1,
            json!({}),
        ))
        .await
        .unwrap();
}

async fn configure_profile(client: &mut UnleashClient, create: &CommandEnvelope, current: u64) {
    client.send_command(&command(create, 0x5994 + u128::from(current), "SetProviderProfile", PROFILE, current, json!({
        "provider_kind":"lmstudio", "dialect":{"kind":"openai_chat_completions","version":1},
        "endpoint":"http://127.0.0.1:9/v1","model":format!("workflow-fixture-local-{}", current + 1),"locality":"local_only",
        "server_management":"user_managed","auth":{"kind":"none"},"pricing":{"kind":"none_local"},
        "limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,
            "load_timeout_ms":180000,"first_token_timeout_ms":60000,"generation_timeout_ms":300000},
        "declared_capabilities":["text"]
    }))).await.unwrap();
}

struct Route {
    _data: tempfile::TempDir,
    _rendezvous: tempfile::TempDir,
    config: DaemonConfig,
    daemon: DaemonHandle,
    client: UnleashClient,
    assign: CommandEnvelope,
    caller: CallerAuthority,
}

async fn route() -> Route {
    let data = unleash_testkit::local_test_dir("f003-bundle-data").unwrap();
    let rendezvous = unleash_testkit::local_test_dir("f003-bundle-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), PermissionsExt::from_mode(0o700)).unwrap();
    let config = DaemonConfig {
        data_root: Some(data.path().to_owned()),
        rendezvous_dir: Some(rendezvous.path().to_owned()),
        ..DaemonConfig::default()
    };
    let seed_dir = data.path().to_owned();
    let (mut create, caller) = tokio::task::spawn_blocking(move || seed(&seed_dir))
        .await
        .unwrap();
    let (daemon, mut client) = start(config.clone()).await;
    prepare_task(&mut client, &mut create).await;
    configure_profile(&mut client, &create, 0).await;
    let task: Uuid = daemon
        .test_runtime_state()
        .store
        .read_snapshot(|db, _| {
            let id: String = db.query_row(
                "SELECT task_id FROM tasks WHERE run_id=?1",
                [workflow_run::RUN.to_string()],
                |row| row.get(0),
            )?;
            Ok(id.parse().unwrap())
        })
        .unwrap();
    client
        .send_command(&command(
            &create,
            0x5987,
            "CreateAgent",
            WORKER.as_uuid(),
            0,
            json!({"mission_id":MISSION,"mission_revision":1,"task_id":task,"task_revision":1}),
        ))
        .await
        .unwrap();
    let assign = command(
        &create,
        0x5988,
        "AssignTask",
        task,
        1,
        json!({"agent_id":WORKER}),
    );
    Route {
        _data: data,
        _rendezvous: rendezvous,
        config,
        daemon,
        client,
        assign,
        caller,
    }
}

struct Facts {
    assignment: Uuid,
    session: String,
    step: String,
    role: String,
    contract: String,
    scope: String,
    workflow: String,
    trigger: Uuid,
}

fn facts(store: &UnleashStore, command: &CommandEnvelope) -> Facts {
    store
        .read_snapshot(|db, _| {
            let (assignment, session, step, role, contract, scope, workflow, trigger): (
            String,String,String,String,String,String,String,String,
        ) = db.query_row(
            "SELECT a.assignment_id,s.session_id,t.step_id,a.role_version_id,t.contract_version_id,
                    t.scope_id,r.workflow_version_id,e.event_id
               FROM tasks t JOIN workflow_runs r USING(run_id)
               JOIN mission_assignments a ON a.mission_id=t.mission_id AND a.agent_id=?2
               JOIN sessions s ON s.agent_id=a.agent_id
               JOIN events e ON e.aggregate_id=r.run_id AND e.event_type='NodeActivated'
                 AND json_extract(e.payload_json,'$.task_id')=t.task_id
              WHERE t.task_id=?1",
            rusqlite::params![command.aggregate_id.to_string(), WORKER.to_string()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,
                      row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        )?;
            Ok(Facts {
                assignment: assignment.parse().unwrap(),
                session,
                step,
                role,
                contract,
                scope,
                workflow,
                trigger: trigger.parse().unwrap(),
            })
        })
        .unwrap()
}

fn publish_bundle(
    store: &UnleashStore,
    assign: CommandEnvelope,
    caller: CallerAuthority,
    available_profile: bool,
) -> Result<Sha256Digest, StoreError> {
    let facts = facts(store, &assign);
    let daemon = store.daemon_state()?;
    let profile = store
        .resolve_provider_profile_snapshot(PROFILE, 1)?
        .value
        .unwrap();
    let profile_digest = profile.configuration_digest;
    let policy =
        json!({"model_policy":"allowed_bounded","locality":"local_only","fallback_allowed":false});
    let context = json!({
        "schema_version":1,"session_id":facts.session,"base_session_revision":0,
        "parent_manifest_id":null,"entries":[],"trigger_event_id":facts.trigger,
    });
    store.publish_initial_context(
        InitialContextPlan {
            manifest_id: MANIFEST,
            authority_generation: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
        },
        &context,
        vec![],
    )?;
    let presented_digest = if available_profile {
        profile_digest
    } else {
        Sha256Digest::from_bytes(b"other fixture version")
    };
    let bundle = json!({
        "schema_version":1,"agent_id":WORKER,"session_id":facts.session,
        "trigger_event_id":facts.trigger,"mission_id":MISSION,"run_id":workflow_run::RUN,
        "task_id":assign.aggregate_id,"step_id":facts.step,"role_version_id":facts.role,
        "contract_version_ids":[facts.contract],"workflow_version_id":facts.workflow,
        "requested_model_profile_version_id":profile.profile_version_ref,
        "requested_model_profile_digest":presented_digest,
        "allowed_model_policy_hash":canonical_sha256(&policy)?,
        "scope_id":facts.scope,"scope_revision":1,"initial_context_manifest_id":MANIFEST,
    });
    store.publish_effective_task_bundle(
        assign,
        caller,
        PreparedTaskAssignment {
            daemon_epoch: daemon.daemon_epoch,
            assignment_id: facts.assignment,
            assignment_epoch: 1,
            assignment_revision: Revision::new(1),
            trigger_event_id: facts.trigger,
            effective_bundle_hash: canonical_sha256(&bundle)?,
            invocation_id: InvocationId::from_u128(0x5989),
        },
        &bundle,
    )
}

async fn publish(route: &Route, available_profile: bool) -> Result<Sha256Digest, StoreError> {
    let store = Arc::clone(&route.daemon.test_runtime_state().store);
    let assign = route.assign.clone();
    let caller = route.caller.clone();
    tokio::task::spawn_blocking(move || publish_bundle(&store, assign, caller, available_profile))
        .await
        .unwrap()
}

#[tokio::test]
async fn queues_the_first_task_from_start_run_and_recovers_its_receipt() {
    let mut route = route().await;
    let hash = publish(&route, true).await.unwrap();
    configure_profile(&mut route.client, &route.assign, 1).await;
    let receipt = route.client.send_command(&route.assign).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let result = receipt
        .result_ref
        .as_deref()
        .unwrap()
        .strip_prefix("invocation:")
        .unwrap();
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x5990),
        query_type: "GetInvocation".to_owned(),
        project_id: route.assign.project_id,
        payload: serde_json::from_value(json!({"invocation_id":result})).unwrap(),
    };
    let queued = route.client.query(&query).await.unwrap().result;
    assert_eq!(queued.get("state"), Some(&json!("queued")));
    assert_eq!(queued.get("effective_bundle_hash"), Some(&json!(hash)));
    let first_epoch = route.daemon.daemon_epoch();
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("qualify assignment restart")
        .await
        .unwrap();
    drop(route.daemon);
    let (daemon, mut client) = start(route.config).await;
    assert!(daemon.daemon_epoch().as_u64() > first_epoch.as_u64());
    route.assign.request_id = RequestId::from_u128(0x5991);
    assert_eq!(client.send_command(&route.assign).await.unwrap(), receipt);
    assert_eq!(client.query(&query).await.unwrap().result, queued);
    let store = Arc::clone(&daemon.test_runtime_state().store);
    let invocation = result.to_owned();
    tokio::task::spawn_blocking(move || {
        assert_queue_and_profile(&store, &route.assign, &invocation);
    })
    .await
    .unwrap();
    drop(client);
    daemon
        .shutdown_graceful("finish first assignment fixture")
        .await
        .unwrap();
}

fn assert_queue_and_profile(store: &UnleashStore, command: &CommandEnvelope, invocation: &str) {
    use unleash_store::{AdmissionOutcome, AdmissionUnit};

    let profile = store
        .resolve_provider_profile_snapshot(PROFILE, 1)
        .unwrap()
        .value
        .unwrap();
    let latest = store
        .resolve_provider_profile_snapshot(PROFILE, 2)
        .unwrap()
        .value
        .unwrap();
    assert_ne!(profile.configuration_digest, latest.configuration_digest);
    let counts = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT (SELECT COUNT(*) FROM invocations WHERE task_id=?1 AND state='queued'),
                    (SELECT COUNT(*) FROM object_references WHERE owner_kind='invocation'
                       AND owner_id=?2 AND source_field='requested_model_profile_version_id'
                       AND object_hash=?3 AND state='live')",
                rusqlite::params![
                    command.aggregate_id.to_string(),
                    invocation,
                    profile.configuration_digest.to_string()
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(counts, (1, 1));

    let id: InvocationId = invocation.parse().unwrap();
    let pool = store.global_execution_pool_id().unwrap();
    let snapshot = store.execution_admission_snapshot(pool).unwrap();
    assert!(snapshot.queued.iter().any(|candidate| {
        candidate.unit == AdmissionUnit::Invocation(id) && candidate.eligibility_evidence.is_none()
    }));
    let (outcome, _) = crate::execution_scheduler::schedule_execution(
        store,
        pool,
        snapshot.token,
        crate::execution_scheduler::ExecutionSchedulingInputs::default(),
    )
    .unwrap();
    assert!(matches!(outcome, AdmissionOutcome::Unchanged));
    assert_eq!(
        store.execution_pool_state(pool).unwrap().current_occupation,
        0
    );
    admit_with_test_observation(store, pool, id);
}

// The fixture supplies eligibility on the real queued invocation. The
// scheduler and reservation below are the product's normal writer path.
fn admit_with_test_observation(
    store: &UnleashStore,
    pool: unleash_domain::PoolId,
    id: InvocationId,
) {
    use unleash_store::{
        AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit, AdmissionUrgency,
        PrepareObjectRequest, PublishObjectReferenceRequest, WriteCriticality,
    };
    const OBSERVER: &str = "start-assignment-fixture-observer";
    let unit = AdmissionUnit::Invocation(id);
    let evidence_id = Uuid::from_u128(0x5996);
    let owner = evidence_id.to_string();
    let bytes = br#"{"schema_version":1,"provenance":"start-assignment-fixture"}"#;
    let object_hash = Sha256Digest::from_bytes(bytes);
    store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, now| {
            tx.execute(
                "INSERT INTO principals(principal_id,principal_kind,actor_id,state,created_at_ms)
             VALUES(?1,'system',?1,'active',?2)",
                rusqlite::params![OBSERVER, now.as_i64()],
            )?;
            Ok(())
        })
        .unwrap();
    let object = store
        .prepare_object(PrepareObjectRequest {
            object_hash,
            byte_length: bytes.len().try_into().unwrap(),
            media_type: "application/json".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: owner.clone(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&object, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash,
            object_generation: object.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: owner,
            source_field: "observation".to_owned(),
        })
        .unwrap();
    let source = store.execution_admission_snapshot(pool).unwrap();
    let candidate = source
        .queued
        .iter()
        .find(|candidate| candidate.unit == unit)
        .unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id,
            observer_principal_id: OBSERVER.to_owned(),
            object_hash,
            unit,
            unit_revision: candidate.revision,
            source_revision: source.source_revision,
            authority_generation: source.authority_generation,
            daemon_epoch: source.daemon_epoch,
            valid_until: unleash_domain::TimestampMs::new(
                store.config().clock.now_ms().as_i64() + 60_000,
            ),
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    let observed = store.execution_admission_snapshot(pool).unwrap();
    let mut inputs = crate::execution_scheduler::ExecutionSchedulingInputs::default();
    inputs.budgets.insert(unit, Vec::new());
    let (admitted, _) =
        crate::execution_scheduler::schedule_execution(store, pool, observed.token, inputs)
            .unwrap();
    assert!(matches!(admitted, AdmissionOutcome::Admitted(commit) if commit.unit==unit));
    let after = store.execution_admission_snapshot(pool).unwrap();
    assert_eq!(after.pool.current_occupation, 1);
    assert_eq!(
        after
            .active
            .iter()
            .filter(|active| active.unit == unit)
            .count(),
        1
    );
    let (repeated, _) = crate::execution_scheduler::schedule_execution(
        store,
        pool,
        after.token,
        crate::execution_scheduler::ExecutionSchedulingInputs::default(),
    )
    .unwrap();
    assert!(matches!(repeated, AdmissionOutcome::Unchanged));
    assert_eq!(
        store.execution_pool_state(pool).unwrap().current_occupation,
        1
    );
}

#[tokio::test]
async fn refuses_a_bundle_profile_digest_that_differs_from_its_configured_version() {
    let route = route().await;
    let result = publish(&route, false).await;
    assert!(
        matches!(result, Err(StoreError::EvidenceUnavailable(message))
        if message == "configured task profile digest differs from its selected version")
    );
    let counts = route
        .daemon
        .test_runtime_state()
        .store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
        "SELECT (SELECT COUNT(*) FROM effective_bundles),(SELECT COUNT(*) FROM invocations)",
        [],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))
    )?)
        })
        .unwrap();
    assert_eq!(counts, (0, 0));
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("finish rejected bundle fixture")
        .await
        .unwrap();
}
