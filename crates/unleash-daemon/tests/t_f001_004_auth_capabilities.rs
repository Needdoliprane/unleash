// T-F001-004 / AC-F001-004 / REQ-CORE-004
//! Vérification d'identité locale et de capacités avant toute opération :
//! refus d'`actor_ref` forgé, UID pair invalide, capacité worker limitée et scope projet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};
use unleash_client::{ClientConfig, ClientError, UnleashClient};
use unleash_daemon::{
    AuthVerifierPort, AuthenticatedSession, Capability, DaemonConfig, DaemonError,
    DaemonStartOutcome, DrainReport, ExecutorDrainPort, PeerUidAuthVerifier, UnleashDaemon,
};
use unleash_domain::{ActorId, CommandId, MissionId, ProjectId, RequestId, Revision, StopOutcome};
use unleash_platform::PeerCredentials;
use unleash_protocol::{
    ClientKind, CommandEnvelope, ErrorCode, HelloRequest, IpcError, ProtocolVersion, ReasonCode,
    RetryClass,
};
use unleash_testkit::local_test_dir;

struct SpyExecutorPort {
    dispatches: AtomicU64,
}

impl ExecutorDrainPort for SpyExecutorPort {
    fn active_hosts_for_other_root(
        &self,
        _rendezvous_dir: &Path,
        _requested_root: &Path,
    ) -> Result<Option<(std::path::PathBuf, Vec<String>)>, DaemonError> {
        Ok(None)
    }

    fn drain_on_stop(&self, _rendezvous_dir: &Path, _timeout: Duration) -> DrainReport {
        DrainReport {
            outcome: StopOutcome::Complete,
            retained_services: Vec::new(),
            unknown_services: Vec::new(),
        }
    }

    fn dispatch_count(&self) -> u64 {
        self.dispatches.load(Ordering::Acquire)
    }
}

struct RejectingForeignUidOrTokenVerifier {
    allowed_project: ProjectId,
}

impl AuthVerifierPort for RejectingForeignUidOrTokenVerifier {
    fn authenticate_peer(
        &self,
        peer: PeerCredentials,
        hello: &HelloRequest,
    ) -> Result<AuthenticatedSession, IpcError> {
        if hello.capabilities.iter().any(|c| c == "simulate:wrong_uid") {
            return Err(IpcError::new(
                ErrorCode::AuthenticationFailed,
                None,
                format!("UID pair {} non autorisé", peer.uid),
                RetryClass::Never,
            ));
        }
        if hello.capabilities.iter().any(|c| c == "scope:restricted") {
            let mut caps = HashSet::new();
            caps.insert(Capability::QueryState);
            caps.insert(Capability::ManageMissions);
            let mut allowed = HashSet::new();
            allowed.insert(self.allowed_project);
            return Ok(AuthenticatedSession {
                peer_creds: peer,
                client_kind: hello.client_kind,
                actor_ref: format!("scoped:uid:{}", peer.uid),
                actor_id: ActorId::new(format!("scoped:uid:{}", peer.uid)).unwrap(),
                capabilities: caps,
                allowed_projects: Some(allowed),
            });
        }
        PeerUidAuthVerifier.authenticate_peer(peer, hello)
    }
}

#[tokio::test]
async fn t_f001_004_rejects_forged_actor_wrong_uid_worker_admin_and_out_of_scope() {
    let rdv = local_test_dir("f001-04-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-04-dat").unwrap();
    let allowed_proj = ProjectId::from_u128(0x2201);
    let forbidden_proj = ProjectId::from_u128(0x2202);
    let mission_id = MissionId::from_u128(0x3201);

    let spy = Arc::new(SpyExecutorPort {
        dispatches: AtomicU64::new(0),
    });
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        executor_controller: spy.clone(),
        auth_verifier: Arc::new(RejectingForeignUidOrTokenVerifier {
            allowed_project: allowed_proj,
        }),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    // Amorçage projet & mission par client autorisé
    let mut admin =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth = admin.hello().authority_generation;

    admin
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x1),
            command_id: CommandId::from_u128(0xC1),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(allowed_proj),
            aggregate_id: allowed_proj.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "AuthScope", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    admin
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x2),
            command_id: CommandId::from_u128(0xC2),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(allowed_proj),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("AssignMission"),
            payload: match json!({ "completion_policy": "mechanical", "state": "active" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    let (snap_before, wm_before) = admin
        .get_state_snapshot(RequestId::from_u128(0x3), None, Map::new())
        .await
        .unwrap();

    // 1. Forged actor_ref dans JSON -> AuthenticationFailed sans mutation
    let forged_err = admin
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x4),
            command_id: CommandId::from_u128(0xC4),
            authority_generation: auth,
            actor_ref: Some(String::from("admin:forged-root")),
            project_id: Some(allowed_proj),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: match json!({ "reason": "usurpation" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        forged_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::AuthenticationFailed
    ));

    // 2. Pair rejeté (simulation autre UID / credential invalide) -> AuthenticationFailed
    let wrong_uid_err = UnleashClient::connect(
        ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0")
            .with_capabilities(vec![String::from("simulate:wrong_uid")]),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        wrong_uid_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::AuthenticationFailed
    ));

    // 3. Capacité expirée au handshake worker -> CapabilityDenied
    let expired_cap_err = UnleashClient::connect(
        ClientConfig::new(sock.clone(), ClientKind::Worker, "0.1.0")
            .with_capabilities(vec![String::from("cap:expired")]),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        expired_cap_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::CapabilityDenied
            && e.reason_code == Some(ReasonCode::OperationNotAuthorized)
    ));

    // 4. Worker authentifié avec capacité limitée tentant PauseMission & StopDaemon -> CapabilityDenied
    let mut worker =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Worker, "0.1.0"))
            .await
            .unwrap();
    let worker_pause_err = worker
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x5),
            command_id: CommandId::from_u128(0xC5),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(allowed_proj),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: Map::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        worker_pause_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::CapabilityDenied
            && e.reason_code == Some(ReasonCode::OperationNotAuthorized)
    ));

    let worker_stop_err = worker
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x6),
            command_id: CommandId::from_u128(0xC6),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(2),
            command_type: String::from("StopDaemon"),
            payload: Map::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        worker_stop_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::CapabilityDenied
    ));

    // 5. Client restreint en portée visant un autre projet -> ScopeDenied
    let mut scoped = UnleashClient::connect(
        ClientConfig::new(sock, ClientKind::Cli, "0.1.0")
            .with_capabilities(vec![String::from("scope:restricted")]),
    )
    .await
    .unwrap();
    let scope_err = scoped
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x7),
            command_id: CommandId::from_u128(0xC7),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(forbidden_proj),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: Map::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        scope_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::ScopeDenied
            && e.reason_code == Some(ReasonCode::AccessDenied)
    ));

    // Aucun changement d'état ni dispatch sur l'adaptateur espion
    let (snap_after, wm_after) = admin
        .get_state_snapshot(RequestId::from_u128(0x8), None, Map::new())
        .await
        .unwrap();
    assert_eq!(wm_before, wm_after);
    assert_eq!(snap_before, snap_after);
    assert_eq!(spy.dispatch_count(), 0);

    handle.shutdown_graceful("done").await.unwrap();
}
