//! T-F003-001 / T-F004-002: inspect durable agent execution identities over actual UDS.
#![expect(
    clippy::unwrap_used,
    reason = "Setup and assertions in an isolated real-daemon fixture"
)]

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{
    AuthVerifierPort, AuthenticatedSession, DaemonConfig, DaemonStartOutcome, PeerUidAuthVerifier,
    UnleashDaemon,
};
use unleash_domain::{
    ActorId, AgentId, AgentLevel, AgentState, CommandId, InvocationId, ProjectId, RequestId,
    Revision, Sha256Digest,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, PrepareObjectRequest,
    PublishObjectReferenceRequest, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

const PROJECT: ProjectId = ProjectId::from_u128(0xf003_0101);
const OTHER_PROJECT: ProjectId = ProjectId::from_u128(0xf003_0102);
const AGENT: AgentId = AgentId::from_u128(0xf003_0103);
const GLOBAL_PARENT: AgentId = AgentId::from_u128(0xf003_0104);
const INVOCATION: InvocationId = InvocationId::from_u128(0xf003_0105);
const MANIFEST: Uuid = Uuid::from_u128(0xf003_0106);
const FIRST_ATTEMPT: Uuid = Uuid::from_u128(0xf003_0107);
const SECOND_ATTEMPT: Uuid = Uuid::from_u128(0xf003_0108);

struct ProjectScopedVerifier;

impl AuthVerifierPort for ProjectScopedVerifier {
    fn authenticate_peer(
        &self,
        peer: unleash_platform::PeerCredentials,
        hello: &unleash_protocol::HelloRequest,
    ) -> Result<AuthenticatedSession, unleash_protocol::IpcError> {
        let mut session = PeerUidAuthVerifier.authenticate_peer(peer, hello)?;
        if hello.client_kind == ClientKind::Tui {
            session.allowed_projects = Some([PROJECT].into_iter().collect());
        }
        Ok(session)
    }
}

fn seed_dormant_agent(root: &Path) -> String {
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    let authority_generation = store.active_authority_generation().unwrap();
    let commit = |verb: &str, kind, project_id, aggregate_id, command, mutation| {
        store
            .commit_command(CommitCommandRequest {
                actor_id: ActorId::new("user:fixture").unwrap(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(command),
                    command_id: CommandId::from_u128(command),
                    authority_generation,
                    actor_ref: None,
                    project_id,
                    aggregate_id,
                    expected_revision: Revision::INITIAL,
                    command_type: verb.to_owned(),
                    payload: Map::new(),
                },
                target_aggregate: kind,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation,
                events: Vec::new(),
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap()
    };
    for (n, project) in [PROJECT, OTHER_PROJECT].into_iter().enumerate() {
        commit(
            "CreateProject",
            AggregateKind::Project,
            Some(project),
            project.as_uuid(),
            n as u128 + 1,
            CommandMutation::CreateProject {
                project_id: project,
                name: "IPC fixture".to_owned(),
                weight: 1,
            },
        );
    }
    for (n, agent_id, project_id, parent, level) in [
        (3, GLOBAL_PARENT, None, None, AgentLevel::NPlus2),
        (4, AGENT, Some(PROJECT), Some(GLOBAL_PARENT), AgentLevel::N),
    ] {
        commit(
            "CreateAgent",
            AggregateKind::Agent,
            project_id,
            agent_id.as_uuid(),
            n,
            CommandMutation::CreateAgent {
                agent_id,
                level,
                organization_parent_id: parent,
                created_by_agent_id: parent,
                project_id,
                state: AgentState::Dormant,
            },
        );
    }
    let session_id = store
        .read_snapshot(|db, _| {
            Ok(db.query_row(
                "SELECT session_id FROM sessions WHERE agent_id = ?1 AND state = 'open'",
                [AGENT.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    store.shutdown().unwrap();
    session_id
}

fn query(name: &str, project_id: Option<ProjectId>, payload: &Value) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(10),
        query_type: name.to_owned(),
        project_id,
        payload: payload.as_object().unwrap().clone(),
    }
}

fn seed_invocation_history(root: &Path) -> (String, Sha256Digest) {
    let session = seed_dormant_agent(root);
    let store = UnleashStore::open(StoreConfig::new(root)).unwrap();
    let bytes = b"{\"fixture\":\"historical invocation inspection, no execution\"}";
    let hash = Sha256Digest::from_bytes(bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "fixture:invocation-history".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            object_hash: hash,
            pin_id: handle.pin_id,
            object_generation: handle.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "fixture:invocation-history".to_owned(),
            source_field: "history".to_owned(),
        })
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    let session_for_tx = session.clone();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            let event = Uuid::from_u128(0xf003_0109).to_string();
            let invocation = INVOCATION.to_string();
            let manifest = MANIFEST.to_string();
            let digest = hash.to_string();
            let timestamp = now.as_i64();
            tx.execute(
                "INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,
                    event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,
                    occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,
                    causation_kind,causation_id,correlation_id)
                 VALUES (?1,?2,'invocation',?3,1,0,'InvocationQueued',1,?4,'fixture','fixture',
                    ?5,?5,'{}','full',?6,'project','external',?1,?1)",
                rusqlite::params![event, daemon.stream_id.to_string(), invocation, PROJECT.to_string(),
                                  timestamp, Sha256Digest::from_bytes(b"{}").to_string()],
            )?;
            tx.execute(
                "INSERT INTO context_manifests (manifest_id,session_id,base_session_revision,
                    content_hash,schema_version,created_at_ms) VALUES (?1,?2,0,?3,1,?4)",
                rusqlite::params![manifest, session_for_tx, digest, timestamp],
            )?;
            tx.execute(
                "INSERT INTO effective_bundles (bundle_hash,schema_version,role_version_id,
                    requested_model_profile_version_id,allowed_model_policy_hash,
                    initial_context_manifest_id,scope_id,scope_revision,content_object_hash)
                 VALUES (?1,1,'fixture-role','fixture-model',?1,?2,'fixture-scope',0,?1)",
                rusqlite::params![digest, manifest],
            )?;
            tx.execute(
                "INSERT INTO invocations (invocation_id,agent_id,session_id,trigger_event_id,
                    effective_bundle_hash,context_manifest_id,state,admission_class,revision,
                    created_at_ms,terminal_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,'completed','work',4,?7,?7)",
                rusqlite::params![invocation, AGENT.to_string(), session_for_tx, event, digest,
                                  manifest, timestamp],
            )?;
            for (id, ordinal, state) in [(FIRST_ATTEMPT, 1, "fenced"), (SECOND_ATTEMPT, 2, "ended")] {
                tx.execute(
                    "INSERT INTO invocation_attempts (attempt_id,invocation_id,ordinal,
                        authority_generation,daemon_epoch,fence,state,started_at_ms,ended_at_ms)
                     VALUES (?1,?2,?3,?4,?3,?3,?5,?6,?6)",
                    rusqlite::params![id.to_string(), invocation, ordinal,
                                      daemon.authority_generation.to_string(), state, timestamp],
                )?;
            }
            Ok(())
        })
        .unwrap();
    store.shutdown().unwrap();
    (session, hash)
}

#[tokio::test]
async fn t_f003_001_get_invocation_inspects_history_with_a_bounded_attempt_window() {
    use std::os::unix::fs::PermissionsExt;

    let rendezvous = local_test_dir("f003-invocation-read-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f003-invocation-read-data").unwrap();
    let root = data.path().to_path_buf();
    let (session, bundle) = tokio::task::spawn_blocking(move || seed_invocation_history(&root))
        .await
        .unwrap();
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        auth_verifier: Arc::new(ProjectScopedVerifier),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("fixture daemon must own its private authority");
    };
    let socket = daemon.socket_path().to_path_buf();
    let config = |kind| ClientConfig::new(socket.clone(), kind, "invocation-read-fixture");
    let mut admin = UnleashClient::connect(config(ClientKind::Cli))
        .await
        .unwrap();
    let mut scoped = UnleashClient::connect(config(ClientKind::Tui))
        .await
        .unwrap();
    let mut diagnostic = UnleashClient::connect(config(ClientKind::Cli).without_admin_credential())
        .await
        .unwrap();
    let first = json!({"invocation_id": INVOCATION, "limit": 1});
    let read = |scope, payload: &Value| query("GetInvocation", scope, payload);
    let baseline = admin
        .query(&query("GetStateSnapshot", Some(PROJECT), &json!({})))
        .await
        .unwrap();
    let page = scoped.query(&read(Some(PROJECT), &first)).await.unwrap();
    let value = page.result;
    assert_eq!(page.watermark, baseline.watermark);
    assert_eq!(value.get("invocation_id"), Some(&json!(INVOCATION)));
    assert_eq!(value.get("agent_id"), Some(&json!(AGENT)));
    assert_eq!(value.get("session_id"), Some(&json!(session)));
    assert_eq!(value.get("project_id"), Some(&json!(PROJECT)));
    assert_eq!(value.get("state"), Some(&json!("completed")));
    assert_eq!(value.get("wait_reason"), Some(&Value::Null));
    assert_eq!(value.get("effective_bundle_hash"), Some(&json!(bundle)));
    assert_eq!(value.get("context_manifest_id"), Some(&json!(MANIFEST)));
    assert_eq!(
        value
            .get("attempts")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        value.pointer("/attempts/0/attempt_id"),
        Some(&json!(FIRST_ATTEMPT))
    );
    assert_eq!(value.pointer("/attempts/0/state"), Some(&json!("fenced")));
    assert_eq!(value.pointer("/attempts/0/fence"), Some(&json!(1)));
    assert_eq!(value.get("next_after_attempt_ordinal"), Some(&json!(1)));
    let next = json!({"invocation_id": INVOCATION, "after_attempt_ordinal": 1, "limit": 1});
    let next_page = scoped.query(&read(Some(PROJECT), &next)).await.unwrap();
    assert_eq!(next_page.watermark, page.watermark);
    let value = next_page.result;
    assert_eq!(
        value
            .get("attempts")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        value.pointer("/attempts/0/attempt_id"),
        Some(&json!(SECOND_ATTEMPT))
    );
    assert_eq!(value.pointer("/attempts/0/state"), Some(&json!("ended")));
    assert_eq!(value.pointer("/attempts/0/fence"), Some(&json!(2)));
    assert_eq!(value.get("next_after_attempt_ordinal"), Some(&Value::Null));
    assert_invocation_read_refusals(&mut admin, &mut scoped, &mut diagnostic, &first).await;
    drop((admin, scoped, diagnostic));
    daemon
        .shutdown_graceful("invocation-read-fixture-complete")
        .await
        .unwrap();
}

async fn assert_invocation_read_refusals(
    admin: &mut UnleashClient,
    scoped: &mut UnleashClient,
    diagnostic: &mut UnleashClient,
    first_page: &Value,
) {
    let read = |scope, payload: &Value| query("GetInvocation", scope, payload);
    assert!(
        matches!(admin.query(&read(Some(OTHER_PROJECT), first_page)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden)
    );
    assert!(
        matches!(scoped.query(&read(None, first_page)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::ScopeDenied)
    );
    assert!(
        matches!(diagnostic.query(&read(Some(PROJECT), first_page)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    for invalid in [
        json!({"invocation_id": INVOCATION, "unexpected": "DO_NOT_ECHO"}),
        json!({"invocation_id": INVOCATION, "limit": 0}),
        json!({"invocation_id": INVOCATION, "limit": 101}),
        json!({"invocation_id": INVOCATION, "after_attempt_ordinal": -1}),
        json!({"invocation_id": INVOCATION, "after_attempt_ordinal": 9_007_199_254_740_992_i64}),
        json!({"invocation_id": INVOCATION, "limit": null}),
    ] {
        assert!(
            matches!(admin.query(&read(Some(PROJECT), &invalid)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::InvalidArgument && !error.message.contains("DO_NOT_ECHO"))
        );
    }
}

#[tokio::test]
async fn t_f003_001_get_agent_reads_durable_session_and_filters_agent_relations() {
    use std::os::unix::fs::PermissionsExt;

    let rendezvous = local_test_dir("f003-agent-read-rdv").unwrap();
    std::fs::set_permissions(rendezvous.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("f003-agent-read-data").unwrap();
    let root = data.path().to_path_buf();
    let session_id = tokio::task::spawn_blocking(move || seed_dormant_agent(&root))
        .await
        .unwrap();
    let DaemonStartOutcome::Started(daemon) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rendezvous.path().to_path_buf()),
        auth_verifier: Arc::new(ProjectScopedVerifier),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("fixture daemon must own its private authority");
    };
    let socket = daemon.socket_path().to_path_buf();
    let config = |kind| ClientConfig::new(socket.clone(), kind, "agent-read-fixture");
    let mut admin = UnleashClient::connect(config(ClientKind::Cli))
        .await
        .unwrap();
    let mut scoped = UnleashClient::connect(config(ClientKind::Tui))
        .await
        .unwrap();
    let mut diagnostic = UnleashClient::connect(config(ClientKind::Cli).without_admin_credential())
        .await
        .unwrap();
    let read = |scope, agent_id| query("GetAgent", scope, &json!({"agent_id": agent_id}));

    let baseline = admin
        .query(&query("GetStateSnapshot", Some(PROJECT), &json!({})))
        .await
        .unwrap();
    let result = admin.query(&read(Some(PROJECT), AGENT)).await.unwrap();
    assert_eq!(
        result.result,
        json!({
            "agent_id": AGENT, "project_id": PROJECT, "level": "N", "state": "dormant",
            "revision": 1, "session_id": session_id,
            "organization_parent_id": GLOBAL_PARENT, "created_by_agent_id": GLOBAL_PARENT
        })
    );
    // With no intervening writes, an agent read returns the current watermark.
    assert!(result.watermark.is_some());
    assert_eq!(result.watermark, baseline.watermark);
    let restricted = scoped.query(&read(Some(PROJECT), AGENT)).await.unwrap();
    assert_eq!(
        restricted.result.get("session_id").and_then(Value::as_str),
        Some(session_id.as_str())
    );
    assert_eq!(
        restricted.result.get("organization_parent_id"),
        Some(&Value::Null)
    );
    assert_eq!(
        restricted.result.get("created_by_agent_id"),
        Some(&Value::Null)
    );
    assert!(
        matches!(scoped.query(&read(None, GLOBAL_PARENT)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::ScopeDenied)
    );
    assert!(
        matches!(scoped.query(&read(Some(PROJECT), GLOBAL_PARENT)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden)
    );
    assert!(
        matches!(admin.query(&read(Some(OTHER_PROJECT), AGENT)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::NotFoundOrHidden)
    );
    assert!(
        matches!(diagnostic.query(&read(Some(PROJECT), AGENT)).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::CapabilityDenied)
    );
    assert!(
        matches!(admin.query(&query("GetAgent", Some(PROJECT), &json!({"agent_id": AGENT, "unexpected": "DO_NOT_ECHO"}))).await, Err(ClientError::Remote(error)) if error.code == ErrorCode::InvalidArgument && !error.message.contains("DO_NOT_ECHO"))
    );
    drop((admin, scoped, diagnostic));
    daemon
        .shutdown_graceful("agent-read-fixture-complete")
        .await
        .unwrap();
}
