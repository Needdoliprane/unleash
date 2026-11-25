//! IPC qualification of the first task's published context and assignment.

// This module is compiled under `#[cfg(test)]`, so `clippy.toml` already permits
// `unwrap`, `expect` and `panic` here; expecting those lints would never be fulfilled.
#![expect(
    clippy::indexing_slicing,
    reason = "Assertions on isolated workflow fixtures read JSON answers by key"
)]

use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use serde_json::{Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_domain::{
    ActorId, AgentId, CommandId, InvocationId, MissionId, RequestId, Revision, Sha256Digest,
};
use unleash_platform::{current_boot_identity, effective_uid};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, ErrorCode, ProtocolVersion, QueryEnvelope,
    ReasonCode, ReceiptState, canonical_sha256,
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
    let (fixture, mut command, caller) =
        workflow_run::prepare(config, actor, unleash_domain::AgentLevel::N, |_, _, _| {});
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

async fn configure_profile(
    client: &mut UnleashClient,
    create: &CommandEnvelope,
    profile_id: Uuid,
    current: u64,
) {
    client.send_command(&command(create, profile_id.as_u128() + 0x10 + u128::from(current), "SetProviderProfile", profile_id, current, json!({
        "provider_kind":"lmstudio", "dialect":{"kind":"openai_chat_completions","version":1},
        "endpoint":"http://127.0.0.1:9/v1","model":format!("workflow-fixture-local-{}", current + 1),"locality":"local_only",
        "server_management":"user_managed","auth":{"kind":"none"},"pricing":{"kind":"none_local"},
        "limits":{"max_inflight":1,"connect_timeout_ms":2000,"queue_timeout_ms":1000,
            "load_timeout_ms":180_000,"first_token_timeout_ms":60_000,"generation_timeout_ms":300_000},
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
    configure_profile(&mut client, &create, PROFILE, 0).await;
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

async fn configured_profile_reference(
    client: &mut UnleashClient,
    command: &CommandEnvelope,
    profile_id: Uuid,
) -> Value {
    let result = client
        .query(&QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x59a5),
            query_type: "GetProviderProfile".to_owned(),
            project_id: command.project_id,
            payload: serde_json::from_value(json!({"profile_id":profile_id,"profile_version":1}))
                .unwrap(),
        })
        .await
        .unwrap()
        .result;
    assert_eq!(result["profile_id"], json!(profile_id));
    assert_eq!(result["profile_version"], 1);
    json!({"profile_id":profile_id,"profile_version":1,"digest":result["configuration_digest"]})
}

// T-F012-002 — Un binding de configuration garde la référence exacte du profil.
#[expect(
    clippy::too_many_lines,
    reason = "One binding lifecycle through IPC and restart"
)]
#[tokio::test]
async fn persists_and_removes_a_versioned_skill_model_preference() {
    let mut route = route().await;
    let project = route.assign.project_id.unwrap();
    let scope = json!({"kind":"project","owner_id":project});
    let skill = json!({"skill_id":"skill.fixture","skill_version":1,"skill_scope":scope});
    let preferred = configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let read = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x59a0),
        query_type: "ReadSettings".to_owned(),
        project_id: Some(project),
        payload: serde_json::from_value(json!({"scope_kind":"project","scope_id":project}))
            .unwrap(),
    };
    let upsert = command(
        &route.assign,
        0x59a1,
        "SetSettings",
        project.as_uuid(),
        0,
        json!({"scope_kind":"project","scope_id":project,"patch":{
            "model_selection_binding":{
                "operation":"upsert","configuration_scope":scope,"skill_ref":skill,
                "preferred_profile_ref":preferred
            }
        }}),
    );
    let unrelated_project = Uuid::from_u128(0x59b0);
    for (id, path, value) in [
        (
            0x59b1,
            "/preferred_profile_ref/digest",
            json!(Sha256Digest::from_bytes(b"not the configured version")),
        ),
        (
            0x59b2,
            "/configuration_scope/owner_id",
            json!(unrelated_project),
        ),
        (
            0x59b3,
            "/skill_ref/skill_scope/owner_id",
            json!(unrelated_project),
        ),
        (0x59b4, "/skill_ref/skill_version", json!(2)),
    ] {
        let mut invalid = upsert.clone();
        invalid.command_id = CommandId::from_u128(id);
        *invalid.payload.get_mut("patch").unwrap()["model_selection_binding"]
            .pointer_mut(path)
            .unwrap() = value;
        assert!(
            matches!(
                route.client.send_command(&invalid).await,
                Err(ClientError::Remote(error))
                    if error.reason_code == Some(ReasonCode::ModelConfigurationConflict)
            ),
            "invalid reference at {path}"
        );
    }
    assert_eq!(
        route.client.query(&read).await.unwrap().result["revision"],
        0
    );

    let receipt = route.client.send_command(&upsert).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(1));
    configure_profile(&mut route.client, &route.assign, PROFILE, 1).await;
    let settings = route.client.query(&read).await.unwrap().result;
    assert_eq!(settings["revision"], 1);
    let [active] = settings["model_selection_bindings"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected one binding after upsert");
    };
    assert_eq!(active["configuration_scope"], scope);
    assert_eq!(active["skill_ref"], skill);
    assert_eq!(active["preferred_profile_ref"], preferred);
    assert_eq!(active["binding_version"], 1);
    assert_eq!(active["state"], "active");
    assert_eq!(active["command_receipt_id"], json!(receipt.receipt_id));
    assert_eq!(active["principal_id"], json!(route.caller.actor_id));
    let mut document = active.clone();
    let digest = document.as_object_mut().unwrap().remove("digest").unwrap();
    assert_eq!(digest, json!(canonical_sha256(&document).unwrap()));
    let expected_ref = format!(
        "model-selection-binding:{}:1:{}",
        active["binding_id"].as_str().unwrap(),
        digest.as_str().unwrap()
    );
    assert_eq!(receipt.result_ref.as_deref(), Some(expected_ref.as_str()));

    let mut remove = command(
        &route.assign,
        0x59a3,
        "SetSettings",
        project.as_uuid(),
        0,
        json!({"scope_kind":"project","scope_id":project,"patch":{
            "model_selection_binding":{
                "operation":"remove","configuration_scope":scope,"skill_ref":skill
            }
        }}),
    );
    assert!(matches!(
        route.client.send_command(&remove).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::RevisionConflict
    ));
    remove.command_id = CommandId::from_u128(0x59a4);
    remove.expected_revision = Revision::new(1);
    let removal = route.client.send_command(&remove).await.unwrap();
    assert_eq!(removal.state, ReceiptState::Accepted);

    drop(route.client);
    route
        .daemon
        .shutdown_graceful("restart settings fixture")
        .await
        .unwrap();
    drop(route.daemon);
    let (daemon, mut client) = start(route.config).await;
    assert_eq!(client.send_command(&upsert).await.unwrap(), receipt);
    let settings = client.query(&read).await.unwrap().result;
    assert_eq!(settings["revision"], 2);
    let [original, removed] = settings["model_selection_bindings"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected the original version and a tombstone");
    };
    assert_eq!(original, active);
    assert_eq!(removed["binding_id"], active["binding_id"]);
    assert_eq!(removed["binding_version"], 2);
    assert_eq!(removed["command_receipt_id"], json!(removal.receipt_id));
    assert_eq!(removed["state"], "removed");
    assert_eq!(removed["preferred_profile_ref"], Value::Null);
    drop(client);
    daemon
        .shutdown_graceful("finish settings fixture")
        .await
        .unwrap();
}

// T-F012-002 — Le vrai writer engage une première politique modèle et son reçu.
#[expect(
    clippy::too_many_lines,
    reason = "One model policy lifecycle through IPC and restart"
)]
#[tokio::test]
async fn publishes_a_role_model_policy_through_settings() {
    let mut route = route().await;
    let project = route.assign.project_id.unwrap();
    let profile = configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let policy = json!({
        "operation": "publish",
        "policy_id": "policy.fixture",
        "allowed_profiles": [profile],
        "allowed_capabilities": ["text"],
        "fallback_profiles": [],
        "limits": {
            "max_llm_calls": 1,
            "max_output_tokens": 128,
            "max_cost_microusd": 10000,
            "max_duration_seconds": 60
        }
    });
    let publish = command(
        &route.assign,
        0x59d0,
        "SetSettings",
        project.as_uuid(),
        0,
        json!({"scope_kind":"project","scope_id":project,"patch":{"model_policy_version":policy}}),
    );
    let read = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x59d1),
        query_type: "ReadSettings".to_owned(),
        project_id: Some(project),
        payload: serde_json::from_value(json!({"scope_kind":"project","scope_id":project}))
            .unwrap(),
    };
    let receipt = route.client.send_command(&publish).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(1));
    let published = route.client.query(&read).await.unwrap().result;
    assert_eq!(published.get("scope_kind"), Some(&json!("project")));
    assert_eq!(published.get("scope_id"), Some(&json!(project)));
    assert_eq!(published.get("revision"), Some(&json!(1)));
    let [version] = published
        .get("model_policy_versions")
        .and_then(Value::as_array)
        .unwrap()
        .as_slice()
    else {
        panic!("expected the single published policy version");
    };
    for field in [
        "policy_id",
        "allowed_profiles",
        "allowed_capabilities",
        "fallback_profiles",
        "limits",
    ] {
        assert_eq!(
            version.get(field),
            Some(policy.get(field).unwrap()),
            "{field}"
        );
    }
    assert_eq!(version.get("version"), Some(&json!(1)));
    assert_eq!(
        version.get("principal_id"),
        Some(&json!(route.caller.actor_id))
    );
    assert_eq!(
        version.get("command_receipt_id"),
        Some(&json!(receipt.receipt_id))
    );
    let digest: Sha256Digest = version
        .get("digest")
        .and_then(Value::as_str)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(version.get("digest"), Some(&json!(digest)));
    let reference = format!("model-policy:policy.fixture:1:{digest}");
    assert_eq!(
        version.get("policy_version_ref").and_then(Value::as_str),
        Some(reference.as_str())
    );
    assert_eq!(version.get("revocation"), Some(&Value::Null));
    assert_eq!(receipt.result_ref.as_deref(), Some(reference.as_str()));

    drop(route.client);
    route
        .daemon
        .shutdown_graceful("restart model policy fixture")
        .await
        .unwrap();
    drop(route.daemon);
    let (daemon, mut client) = start(route.config).await;
    let mut retry = publish.clone();
    retry.request_id = RequestId::from_u128(0x59d2);
    assert_eq!(client.send_command(&retry).await.unwrap(), receipt);
    let mut stale = publish;
    stale.request_id = RequestId::from_u128(0x59d3);
    stale.command_id = CommandId::from_u128(0x59d3);
    assert!(matches!(
        client.send_command(&stale).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::RevisionConflict
    ));
    assert_eq!(client.query(&read).await.unwrap().result, published);
    drop(client);
    daemon
        .shutdown_graceful("finish model policy fixture")
        .await
        .unwrap();
}

fn model_policy(profiles: &Value, limits: &Value) -> Value {
    json!({"operation":"publish","policy_id":"policy.fixture","allowed_profiles":profiles,
        "allowed_capabilities":["text"],"fallback_profiles":[],"limits":limits})
}

fn model_policy_bounds(calls: u64, tokens: u64, seconds: u64) -> Value {
    json!({"max_llm_calls":calls,"max_output_tokens":tokens,"max_cost_microusd":10000,
        "max_duration_seconds":seconds})
}

fn model_policy_revocation(version: u64) -> Value {
    json!({"operation":"revoke","policy_id":"policy.fixture","version":version})
}

/// Envoie un `SetSettings` de politique de modèle sur le socket, à la révision attendue.
async fn set_model_policy(
    client: &mut UnleashClient,
    base: &CommandEnvelope,
    id: u128,
    project: Uuid,
    revision: u64,
    policy: &Value,
) -> Result<CommandReceipt, ClientError> {
    client
        .send_command(&command(
            base,
            id,
            "SetSettings",
            project.as_uuid(),
            revision,
            json!({"scope_kind":"project","scope_id":project,"patch":{"model_policy_version":policy}}),
        ))
        .await
}

fn settings_query(id: u128, project: Uuid) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        query_type: "ReadSettings".to_owned(),
        project_id: Some(project),
        payload: serde_json::from_value(json!({"scope_kind":"project","scope_id":project}))
            .unwrap(),
    }
}

fn project_settings(project: Uuid, policy: &Value) -> Value {
    json!({"scope_kind":"project","scope_id":project,"patch":{"model_policy_version":policy}})
}

fn is_model_conflict(result: Result<CommandReceipt, ClientError>) -> bool {
    matches!(
        result,
        Err(ClientError::Remote(error))
            if error.reason_code == Some(ReasonCode::ModelConfigurationConflict)
    )
}

// T-F012-002 — La révocation par les réglages est un fait distinct : la version garde son
// document, son digest et son reçu de publication ; l'ensemble survit au redémarrage et au rejeu.
#[expect(
    clippy::too_many_lines,
    reason = "One revocation lifecycle through IPC and restart"
)]
#[tokio::test]
async fn revokes_a_published_model_policy_version_through_settings() {
    let mut route = route().await;
    let project = route.assign.project_id.unwrap();
    let profile = configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let read = settings_query(0x59d1, project);
    let allowed = json!([profile]);
    let first = set_model_policy(
        &mut route.client,
        &route.assign,
        0x59d0,
        project,
        0,
        &model_policy(&allowed, &model_policy_bounds(1, 128, 60)),
    )
    .await
    .unwrap();
    let second = set_model_policy(
        &mut route.client,
        &route.assign,
        0x59d4,
        project,
        1,
        &model_policy(&allowed, &model_policy_bounds(2, 256, 120)),
    )
    .await
    .unwrap();
    let revoked = set_model_policy(
        &mut route.client,
        &route.assign,
        0x59d5,
        project,
        2,
        &model_policy_revocation(1),
    )
    .await
    .unwrap();

    let published = route.client.query(&read).await.unwrap().result;
    let [retained, active] = published["model_policy_versions"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected the two published policy versions");
    };
    assert_eq!(published["revision"], json!(3));
    let fields: Vec<&str> = retained
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        fields,
        [
            "policy_id",
            "version",
            "digest",
            "policy_version_ref",
            "allowed_profiles",
            "allowed_capabilities",
            "fallback_profiles",
            "limits",
            "principal_id",
            "command_receipt_id",
            "revocation",
        ]
    );
    assert_eq!(retained["version"], json!(1));
    assert_eq!(retained["command_receipt_id"], json!(first.receipt_id));
    assert_eq!(
        retained["revocation"],
        json!({"principal_id":route.caller.actor_id,
            "command_receipt_id":revoked.receipt_id})
    );
    assert_eq!(active["version"], json!(2));
    assert_eq!(active["command_receipt_id"], json!(second.receipt_id));
    assert_eq!(active["revocation"], Value::Null);
    assert_eq!(
        second.result_ref,
        active["policy_version_ref"].as_str().map(ToOwned::to_owned)
    );
    assert_eq!(
        revoked.result_ref,
        retained["policy_version_ref"]
            .as_str()
            .map(|reference| format!("{reference}:revoked"))
    );

    // La révocation répétée est refusée sans rien engager.
    let repeated = set_model_policy(
        &mut route.client,
        &route.assign,
        0x59d6,
        project,
        3,
        &model_policy_revocation(1),
    )
    .await;
    assert!(is_model_conflict(repeated));
    assert_eq!(route.client.query(&read).await.unwrap().result, published);

    drop(route.client);
    route
        .daemon
        .shutdown_graceful("restart model policy revocation fixture")
        .await
        .unwrap();
    drop(route.daemon);
    let (daemon, mut client) = start(route.config).await;
    let mut replay = command(
        &route.assign,
        0x59d5,
        "SetSettings",
        project.as_uuid(),
        2,
        project_settings(project, &model_policy_revocation(1)),
    );
    replay.request_id = RequestId::from_u128(0x59d7);
    assert_eq!(client.send_command(&replay).await.unwrap(), revoked);
    assert_eq!(client.query(&read).await.unwrap().result, published);
    drop(client);
    daemon
        .shutdown_graceful("finish model policy revocation fixture")
        .await
        .unwrap();
}

/// Une route dont la politique de modèle `policy.fixture` n'a encore rien publié.
async fn unpublished_model_policy() -> (Route, Uuid, Value, QueryEnvelope, CommandEnvelope) {
    let mut route = route().await;
    let project = route.assign.project_id.unwrap();
    let profile = configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let read = settings_query(0x59e9, project);
    let publish = command(
        &route.assign,
        0x59e0,
        "SetSettings",
        project.as_uuid(),
        0,
        project_settings(
            project,
            &model_policy(&json!([profile]), &model_policy_bounds(1, 128, 60)),
        ),
    );
    (route, project, profile, read, publish)
}

// T-F012-002 — Un acteur sans preuve administrative ne publie pas, et la portée du patch doit
// être celle de l'enveloppe.
#[tokio::test]
async fn refuses_a_model_policy_without_edition_right_or_outside_the_envelope_scope() {
    let (mut route, project, _profile, read, publish) = unpublished_model_policy().await;

    // Acteur sans droit d'édition : la preuve administrative manque.
    let mut unprivileged = UnleashClient::connect(
        ClientConfig::new(
            route.daemon.socket_path().to_owned(),
            ClientKind::Cli,
            "0.1.0",
        )
        .without_admin_credential(),
    )
    .await
    .unwrap();
    assert!(matches!(
        unprivileged.send_command(&publish).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied
    ));
    drop(unprivileged);

    // Portée du patch incompatible avec l'enveloppe : réglages utilisateur sous un projet.
    let mut crossed = publish.clone();
    crossed.command_id = CommandId::from_u128(0x59e1);
    crossed.payload = json!({"scope_kind":"user","scope_id":"default",
        "patch":{"model_policy_version":publish.payload["patch"]["model_policy_version"]}})
    .as_object()
    .unwrap()
    .clone();
    assert!(matches!(
        route.client.send_command(&crossed).await,
        Err(ClientError::Remote(error)) if error.code == ErrorCode::InvalidArgument
    ));

    let settings = route.client.query(&read).await.unwrap().result;
    assert_eq!(settings["revision"], json!(0));
    assert_eq!(settings["model_policy_versions"], json!([]));
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("finish refused model policy fixture")
        .await
        .unwrap();
}

// T-F012-002 — Un patch fermé hors contrat est refusé sans écho de la valeur ; une référence
// incohérente avec l'état est un conflit de configuration ; rien n'est engagé.
#[tokio::test]
async fn refuses_a_model_policy_patch_outside_the_closed_contract_or_incoherent_with_state() {
    let (mut route, project, profile, read, publish) = unpublished_model_policy().await;

    for (id, path, value) in [
        (0x59e2, "/version", json!(1)),
        (0x59e3, "/principal_id", json!("DO_NOT_ECHO")),
        (0x59e4, "/limits/max_cost_microusd", json!(-1)),
        (0x59e5, "/allowed_profiles/0/profile_version", json!(0)),
        (0x59e7, "/policy_id", json!("DO_NOT_ECHO policy")),
    ] {
        let mut invalid = publish.clone();
        invalid.command_id = CommandId::from_u128(id);
        *invalid.payload["patch"]["model_policy_version"]
            .pointer_mut(path)
            .unwrap() = value;
        assert!(
            matches!(
                route.client.send_command(&invalid).await,
                Err(ClientError::Remote(error))
                    if error.code == ErrorCode::InvalidArgument
                        && !error.message.contains("DO_NOT_ECHO")
            ),
            "accepted {path}"
        );
    }

    // Références incohérentes avec l'état : digest divergent, repli hors liste, version absente.
    let mut divergent = publish.clone();
    divergent.command_id = CommandId::from_u128(0x59e8);
    divergent.payload["patch"]["model_policy_version"]["allowed_profiles"][0]["digest"] =
        json!(Sha256Digest::from_bytes(b"other fixture version"));
    assert!(is_model_conflict(
        route.client.send_command(&divergent).await
    ));
    let mut widened = publish.clone();
    widened.command_id = CommandId::from_u128(0x59ea);
    widened.payload["patch"]["model_policy_version"]["fallback_profiles"] =
        json!([{"profile_id":PROFILE,"profile_version":2,"digest":profile["digest"]}]);
    assert!(is_model_conflict(route.client.send_command(&widened).await));
    let missing = set_model_policy(
        &mut route.client,
        &route.assign,
        0x59eb,
        project,
        0,
        &model_policy_revocation(1),
    )
    .await;
    assert!(is_model_conflict(missing));

    let settings = route.client.query(&read).await.unwrap().result;
    assert_eq!(settings["revision"], json!(0));
    assert_eq!(settings["model_policy_versions"], json!([]));
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("finish refused model policy fixture")
        .await
        .unwrap();
}

// T-F012-002 — Une même commande engage une préférence de skill et une politique de modèle sous
// une seule révision et un seul reçu.
#[tokio::test]
async fn commits_a_skill_preference_and_a_model_policy_in_one_settings_command() {
    let mut route = route().await;
    let project = route.assign.project_id.unwrap();
    let scope = json!({"kind":"project","owner_id":project});
    let skill = json!({"skill_id":"skill.fixture","skill_version":1,"skill_scope":scope});
    let profile = configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let composed = command(
        &route.assign,
        0x59f0,
        "SetSettings",
        project.as_uuid(),
        0,
        json!({"scope_kind":"project","scope_id":project,"patch":{
            "model_selection_binding":{"operation":"upsert","configuration_scope":scope,
                "skill_ref":skill,"preferred_profile_ref":profile},
            "model_policy_version":model_policy(
                &json!([profile]),
                &model_policy_bounds(1, 128, 60),
            )
        }}),
    );
    let read = settings_query(0x59f1, project);

    let receipt = route.client.send_command(&composed).await.unwrap();

    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::new(1));
    assert!(
        receipt
            .result_ref
            .as_deref()
            .unwrap()
            .starts_with("model-selection-binding:")
    );
    let settings = route.client.query(&read).await.unwrap().result;
    let [binding] = settings["model_selection_bindings"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected one binding in the composed command");
    };
    assert_eq!(binding["state"], "active");
    assert_eq!(binding["command_receipt_id"], json!(receipt.receipt_id));
    let [policy] = settings["model_policy_versions"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected one model policy version in the composed command");
    };
    assert_eq!(policy["version"], json!(1));
    assert_eq!(policy["command_receipt_id"], json!(receipt.receipt_id));
    assert_eq!(policy["allowed_profiles"], json!([profile]));
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("finish composed settings fixture")
        .await
        .unwrap();
}

// T-F012-002 — Le niveau utilisateur peut choisir un profil utilisateur pour un skill de projet.
#[tokio::test]
async fn user_preferences_resolve_exact_skill_and_profile_namespaces_independently() {
    const USER_PROFILE: Uuid = Uuid::from_u128(0x59c0);

    let mut route = route().await;
    let mut user = route.assign.clone();
    user.project_id = None;
    configure_profile(&mut route.client, &user, USER_PROFILE, 0).await;
    let project_profile =
        configured_profile_reference(&mut route.client, &route.assign, PROFILE).await;
    let user_profile = configured_profile_reference(&mut route.client, &user, USER_PROFILE).await;
    let read = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x59c1),
        query_type: "ReadSettings".to_owned(),
        project_id: None,
        payload: serde_json::from_value(json!({"scope_kind":"user","scope_id":"default"})).unwrap(),
    };
    let configure = |id, reference| {
        command(
            &user,
            id,
            "SetSettings",
            user.authority_generation.as_uuid(),
            0,
            json!({"scope_kind":"user","scope_id":"default","patch":{
                "model_selection_binding":{
                    "operation":"upsert",
                    "configuration_scope":{"kind":"global","owner_id":null},
                    "skill_ref":{"skill_id":"skill.fixture","skill_version":1,
                        "skill_scope":{"kind":"project","owner_id":route.assign.project_id}},
                    "preferred_profile_ref":reference
                }
            }}),
        )
    };
    let refused = configure(0x59c2, project_profile);
    assert!(matches!(
        route.client.send_command(&refused).await,
        Err(ClientError::Remote(error))
            if error.reason_code == Some(ReasonCode::ModelConfigurationConflict)
    ));
    assert_eq!(
        route.client.query(&read).await.unwrap().result["revision"],
        0
    );
    let permitted = configure(0x59c3, user_profile.clone());
    let receipt = route.client.send_command(&permitted).await.unwrap();
    assert_eq!(receipt.new_revision, Revision::new(1));
    let result = route.client.query(&read).await.unwrap().result;
    let [binding] = result["model_selection_bindings"]
        .as_array()
        .unwrap()
        .as_slice()
    else {
        panic!("expected one binding after upsert");
    };
    assert_eq!(
        binding["configuration_scope"],
        json!({"kind":"global","owner_id":null})
    );
    assert_eq!(
        binding["skill_ref"]["skill_scope"]["owner_id"],
        json!(route.assign.project_id)
    );
    assert_eq!(binding["preferred_profile_ref"], user_profile);
    drop(route.client);
    route
        .daemon
        .shutdown_graceful("finish user settings fixture")
        .await
        .unwrap();
}

// T-F003-002 — Bundle épinglé et mise en file. L'observation d'admission est synthétique.
#[tokio::test]
async fn queues_the_first_task_from_start_run_and_recovers_its_receipt() {
    let mut route = route().await;
    let hash = publish(&route, true).await.unwrap();
    configure_profile(&mut route.client, &route.assign, PROFILE, 1).await;
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

// T-F003-002 — Refus d'une empreinte divergente pour le profil exact.
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
