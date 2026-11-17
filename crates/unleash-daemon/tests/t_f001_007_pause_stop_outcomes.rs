// T-F001-007 / AC-F001-007 / REQ-CORE-007
//! Pause et `StopDaemon` exposent intention, progression et résultat réel
//! (`stop_outcome` complete / partial / unknown) sans contradiction avec les événements.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};
use unleash_client::{
    ClientConfig, ClientError, EventFollowerConfig, EventSyncUpdate, EventSynchronizer,
    UnleashClient,
};
use unleash_daemon::{
    DaemonConfig, DaemonError, DaemonStartOutcome, DrainReport, ExecutorDrainPort, UnleashDaemon,
};
use unleash_domain::{CommandId, MissionId, ProjectId, RequestId, Revision, StopOutcome};
use unleash_protocol::{ClientKind, CommandEnvelope, ErrorCode, ProtocolVersion, ReceiptState};
use unleash_testkit::local_test_dir;

struct FixedDrainController {
    report: DrainReport,
}

impl ExecutorDrainPort for FixedDrainController {
    fn active_hosts_for_other_root(
        &self,
        _rendezvous_dir: &Path,
        _requested_root: &Path,
    ) -> Result<Option<(std::path::PathBuf, Vec<String>)>, DaemonError> {
        Ok(None)
    }

    fn drain_on_stop(&self, _rendezvous_dir: &Path, _timeout: Duration) -> DrainReport {
        self.report.clone()
    }
}

async fn run_stop_scenario(
    expected_outcome: StopOutcome,
    retained: Vec<String>,
    unknown: Vec<String>,
) {
    let rdv = local_test_dir("f001-07-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-07-dat").unwrap();
    let controller = Arc::new(FixedDrainController {
        report: DrainReport {
            outcome: expected_outcome,
            retained_services: retained.clone(),
            unknown_services: unknown.clone(),
        },
    });
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        executor_controller: controller,
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    let mut client =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
            .await
            .unwrap();
    let auth = client.hello().authority_generation;

    // PauseMission sur une mission créée
    let project_id = ProjectId::from_u128(0x2700);
    let mission_id = MissionId::from_u128(0x3700);
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x1),
            command_id: CommandId::from_u128(0xC1),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: project_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: String::from("CreateProject"),
            payload: match json!({ "name": "StopTest", "weight": 1 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x2),
            command_id: CommandId::from_u128(0xC2),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
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
    let pause_rcpt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x3),
            command_id: CommandId::from_u128(0xC3),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(1),
            command_type: String::from("PauseMission"),
            payload: match json!({ "reason": "inspection arrêt" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    assert_eq!(pause_rcpt.new_revision, Revision::new(2));

    // Abonnement avant StopDaemon pour observer DaemonStopRequested et DaemonStopped
    let mut sync = EventSynchronizer::new(
        ClientConfig::new(sock, ClientKind::Cli, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (_, mut stream) = sync
        .sync_initial(client.connection_mut(), RequestId::from_u128(0x4))
        .await
        .unwrap();

    // Lire daemon_state.revision courante
    let status = client
        .query(&unleash_protocol::QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x5),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let daemon_rev = status
        .result
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap();

    // Mauvaise révision sur StopDaemon -> RevisionConflict
    let wrong_rev = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x6),
            command_id: CommandId::from_u128(0xC6),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(daemon_rev + 10),
            command_type: String::from("StopDaemon"),
            payload: Map::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        wrong_rev,
        ClientError::Remote(ref e) if e.code == ErrorCode::RevisionConflict
    ));

    // StopDaemon nominal avec aggregate_id = authority_generation et expected_revision = daemon_state.revision
    let stop_rcpt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x7),
            command_id: CommandId::from_u128(0xC7),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(daemon_rev),
            command_type: String::from("StopDaemon"),
            payload: match json!({ "reason": "arrêt contrôlé" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    assert_eq!(stop_rcpt.aggregate_id, auth.as_uuid());
    assert_eq!(stop_rcpt.state, ReceiptState::Accepted);
    assert!(stop_rcpt.operation_id.is_some());

    let mut saw_requested = false;
    let mut observed_stop_payload: Option<Map<String, Value>> = None;
    while observed_stop_payload.is_none() {
        let (upd, next_stream) = sync
            .poll_next(&mut stream, client.connection_mut(), || {
                RequestId::from_u128(0x99)
            })
            .await
            .unwrap();
        if let Some(ns) = next_stream {
            stream = ns;
        }
        if let EventSyncUpdate::Events { events, .. } = upd {
            for ev in events {
                if ev.event_type == "DaemonStopRequested" {
                    saw_requested = true;
                } else if ev.event_type == "DaemonStopped" {
                    observed_stop_payload = Some(ev.payload);
                }
            }
        }
    }

    assert!(
        saw_requested,
        "DaemonStopRequested doit précéder DaemonStopped"
    );
    let stop_payload = observed_stop_payload.unwrap();
    assert_eq!(
        stop_payload.get("stop_outcome").and_then(Value::as_str),
        Some(expected_outcome.to_string().as_str())
    );
    let final_report = handle.wait_stopped().await.unwrap();
    assert_eq!(final_report.outcome, expected_outcome);
    assert_eq!(final_report.retained_services, retained);
    assert_eq!(final_report.unknown_services, unknown);
}

#[tokio::test]
async fn t_f001_007_stop_distinguishes_complete_partial_and_unknown_outcomes() {
    run_stop_scenario(StopOutcome::Complete, Vec::new(), Vec::new()).await;
    run_stop_scenario(
        StopOutcome::Partial,
        vec![String::from("pty-retained-1")],
        Vec::new(),
    )
    .await;
    run_stop_scenario(
        StopOutcome::Unknown,
        Vec::new(),
        vec![String::from("worker-unobservable-1")],
    )
    .await;
}

struct RecordingDrainController {
    recorded_timeout: Mutex<Option<Duration>>,
}

impl ExecutorDrainPort for RecordingDrainController {
    fn active_hosts_for_other_root(
        &self,
        _rendezvous_dir: &Path,
        _requested_root: &Path,
    ) -> Result<Option<(std::path::PathBuf, Vec<String>)>, DaemonError> {
        Ok(None)
    }

    fn drain_on_stop(&self, _rendezvous_dir: &Path, timeout: Duration) -> DrainReport {
        *self.recorded_timeout.lock().unwrap() = Some(timeout);
        DrainReport {
            outcome: StopOutcome::Complete,
            retained_services: Vec::new(),
            unknown_services: Vec::new(),
        }
    }
}

#[tokio::test]
async fn t_f001_007_stop_daemon_propagates_drain_timeout_ms_override() {
    let rdv = local_test_dir("f001-07t-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-07t-dat").unwrap();
    let spy = Arc::new(RecordingDrainController {
        recorded_timeout: Mutex::new(None),
    });

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        shutdown_drain_timeout: Duration::from_secs(30),
        executor_controller: Arc::clone(&spy) as Arc<dyn ExecutorDrainPort>,
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "0.1.0",
    ))
    .await
    .unwrap();
    let auth = client.hello().authority_generation;

    let status = client
        .query(&unleash_protocol::QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x701),
            query_type: String::from("GetDaemonStatus"),
            project_id: None,
            payload: Map::new(),
        })
        .await
        .unwrap();
    let daemon_rev = status
        .result
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap();

    // drain_timeout_ms invalide -> InvalidArgument sans arrêter le daemon
    let bad_err = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x702),
            command_id: CommandId::from_u128(0xC702),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(daemon_rev),
            command_type: String::from("StopDaemon"),
            payload: match json!({ "drain_timeout_ms": -5 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(
        bad_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::InvalidArgument
    ));

    // drain_timeout_ms valide -> propagé jusqu'à ExecutorDrainPort::drain_on_stop
    let ok_rcpt = client
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x703),
            command_id: CommandId::from_u128(0xC703),
            authority_generation: auth,
            actor_ref: None,
            project_id: None,
            aggregate_id: auth.as_uuid(),
            expected_revision: Revision::new(daemon_rev),
            command_type: String::from("StopDaemon"),
            payload: match json!({ "reason": "custom drain timeout", "drain_timeout_ms": 345 }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();
    assert_eq!(ok_rcpt.state, ReceiptState::Accepted);

    let report = handle.wait_stopped().await.unwrap();
    assert_eq!(report.outcome, StopOutcome::Complete);
    assert_eq!(
        *spy.recorded_timeout.lock().unwrap(),
        Some(Duration::from_millis(345))
    );
}
