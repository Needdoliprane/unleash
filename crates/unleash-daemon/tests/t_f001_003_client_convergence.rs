// T-F001-003 / AC-F001-003 / REQ-CORE-003
//! GUI, CLI et TUI utilisent les mêmes commandes, révisions et projections ;
//! leurs snapshots normalisés au même watermark ont un diff sémantique vide.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use serde_json::{Map, Value, json};
use unleash_client::{
    ClientConfig, EventFollowerConfig, EventSyncUpdate, EventSynchronizer, UnleashClient,
};
use unleash_daemon::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, MissionId, ProjectId, RequestId, Revision};
use unleash_protocol::{ClientKind, CommandEnvelope, ProtocolVersion};
use unleash_testkit::local_test_dir;

#[tokio::test]
async fn t_f001_003_cli_gui_tui_converge_to_identical_snapshot_and_events() {
    let rdv = local_test_dir("f001-03-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-03-dat").unwrap();
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };
    let sock = handle.socket_path().to_path_buf();

    let mut cli = UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
        .await
        .unwrap();
    let mut gui = UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Gui, "0.1.0"))
        .await
        .unwrap();
    let mut tui = UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Tui, "0.1.0"))
        .await
        .unwrap();

    let auth = cli.hello().authority_generation;
    let project_id = ProjectId::from_u128(0x2100);
    let mission_id = MissionId::from_u128(0x3100);

    // 1. CLI crée le projet et assigne la mission
    cli.send_command(&CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x101),
        command_id: CommandId::from_u128(0xC101),
        authority_generation: auth,
        actor_ref: None,
        project_id: Some(project_id),
        aggregate_id: project_id.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: String::from("CreateProject"),
        payload: match json!({ "name": "Convergence", "weight": 1 }) {
            Value::Object(m) => m,
            _ => Map::new(),
        },
    })
    .await
    .unwrap();

    cli.send_command(&CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x102),
        command_id: CommandId::from_u128(0xC102),
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

    // TUI s'abonne au flux à partir du snapshot initial
    let mut tui_sync = EventSynchronizer::new(
        ClientConfig::new(sock.clone(), ClientKind::Tui, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (_, mut tui_stream) = tui_sync
        .sync_initial(tui.connection_mut(), RequestId::from_u128(0x103))
        .await
        .unwrap();

    // 2. CLI pause (rev 1 -> 2), GUI reprend (rev 2 -> 3), TUI annule (rev 3 -> 4)
    cli.send_command(&CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x104),
        command_id: CommandId::from_u128(0xC104),
        authority_generation: auth,
        actor_ref: None,
        project_id: Some(project_id),
        aggregate_id: mission_id.as_uuid(),
        expected_revision: Revision::new(1),
        command_type: String::from("PauseMission"),
        payload: match json!({ "reason": "pause CLI" }) {
            Value::Object(m) => m,
            _ => Map::new(),
        },
    })
    .await
    .unwrap();

    gui.send_command(&CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(0x105),
        command_id: CommandId::from_u128(0xC105),
        authority_generation: auth,
        actor_ref: None,
        project_id: Some(project_id),
        aggregate_id: mission_id.as_uuid(),
        expected_revision: Revision::new(2),
        command_type: String::from("ResumeMission"),
        payload: match json!({ "reason": "reprise GUI" }) {
            Value::Object(m) => m,
            _ => Map::new(),
        },
    })
    .await
    .unwrap();

    let final_rcpt = tui
        .send_command(&CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0x106),
            command_id: CommandId::from_u128(0xC106),
            authority_generation: auth,
            actor_ref: None,
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::new(3),
            command_type: String::from("CancelMission"),
            payload: match json!({ "reason": "annulation TUI" }) {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        })
        .await
        .unwrap();

    // 3. Snapshots de CLI, GUI et TUI au même watermark -> strictement égaux
    let (snap_cli, wm_cli) = cli
        .get_state_snapshot(RequestId::from_u128(0x201), Some(project_id), Map::new())
        .await
        .unwrap();
    let (snap_gui, wm_gui) = gui
        .get_state_snapshot(RequestId::from_u128(0x202), Some(project_id), Map::new())
        .await
        .unwrap();
    let (snap_tui, wm_tui) = tui
        .get_state_snapshot(RequestId::from_u128(0x203), Some(project_id), Map::new())
        .await
        .unwrap();

    assert_eq!(wm_cli, final_rcpt.event_cursor);
    assert_eq!(wm_cli, wm_gui);
    assert_eq!(wm_gui, wm_tui);
    assert_eq!(snap_cli, snap_gui);
    assert_eq!(snap_gui, snap_tui);

    // 4. Flux TUI converge vers final_rcpt.event_cursor sans trou
    let mut seen_types = Vec::new();
    while tui_sync
        .current_cursor()
        .is_none_or(|c| c.sequence < final_rcpt.event_cursor.sequence)
    {
        let (update, maybe_new_reader) = tui_sync
            .poll_next(&mut tui_stream, tui.connection_mut(), || {
                RequestId::from_u128(0x999)
            })
            .await
            .unwrap();
        if let Some(nr) = maybe_new_reader {
            tui_stream = nr;
        }
        if let EventSyncUpdate::Events { events, .. } = update {
            for ev in events {
                seen_types.push(ev.event_type);
            }
        }
    }
    assert!(seen_types.iter().any(|t| t == "MissionPaused"));
    assert!(seen_types.iter().any(|t| t == "MissionResumed"));
    assert!(seen_types.iter().any(|t| t == "MissionCancelled"));

    handle.shutdown_graceful("done").await.unwrap();
}
