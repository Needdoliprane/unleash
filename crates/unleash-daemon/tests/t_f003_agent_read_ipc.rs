//! T-F003-001 / T-F004-002: inspect a dormant agent and its primary session over actual UDS.
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
    ActorId, AgentId, AgentLevel, AgentState, CommandId, ProjectId, RequestId, Revision,
};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, QueryEnvelope, ReceiptState,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, StoreConfig, UnleashStore,
};
use unleash_testkit::local_test_dir;

const PROJECT: ProjectId = ProjectId::from_u128(0xf003_0101);
const OTHER_PROJECT: ProjectId = ProjectId::from_u128(0xf003_0102);
const AGENT: AgentId = AgentId::from_u128(0xf003_0103);
const GLOBAL_PARENT: AgentId = AgentId::from_u128(0xf003_0104);

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
