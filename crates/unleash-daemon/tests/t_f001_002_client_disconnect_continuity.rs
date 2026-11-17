// T-F001-002 / AC-F001-002 / REQ-CORE-002
//! Fermer tous les clients laisse le daemon et les missions actives continuer ;
//! la réouverture reconstitue exactement la même identité et tout l'historique.

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

fn make_cmd(
    auth: unleash_domain::AuthorityGeneration,
    (cmd_id, req_id): (u128, u128),
    project_id: Option<ProjectId>,
    aggregate_id: uuid::Uuid,
    expected_revision: u64,
    command_type: &str,
    payload: Value,
) -> CommandEnvelope {
    let map = match payload {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(req_id),
        command_id: CommandId::from_u128(cmd_id),
        authority_generation: auth,
        actor_ref: None,
        project_id,
        aggregate_id,
        expected_revision: Revision::new(expected_revision),
        command_type: command_type.to_owned(),
        payload: map,
    }
}

#[tokio::test]
async fn t_f001_002_missions_survive_all_clients_disconnect_and_reopen() {
    let rdv = local_test_dir("f001-02-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("f001-02-dat").unwrap();

    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("Daemon attendu démarré");
    };

    let sock = handle.socket_path().to_path_buf();
    let project_id = ProjectId::from_u128(0x2001);
    let mission_id = MissionId::from_u128(0x3001);

    let (auth_before, inst_before, epoch_before, seq_before) = {
        let mut gui =
            UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Gui, "0.1.0"))
                .await
                .unwrap();
        let tui = UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Tui, "0.1.0"))
            .await
            .unwrap();

        let auth = gui.hello().authority_generation;
        let inst = gui.hello().daemon_instance_id;
        let epoch = gui.hello().daemon_epoch;
        assert_eq!(tui.hello().authority_generation, auth);

        gui.send_command(&make_cmd(
            auth,
            (0xC001, 0xA001),
            Some(project_id),
            project_id.as_uuid(),
            0,
            "CreateProject",
            json!({ "name": "Projet Continuité", "weight": 2 }),
        ))
        .await
        .unwrap();

        let assign_rcpt = gui
            .send_command(&make_cmd(
                auth,
                (0xC002, 0xA002),
                Some(project_id),
                mission_id.as_uuid(),
                0,
                "AssignMission",
                json!({ "completion_policy": "mechanical", "state": "active" }),
            ))
            .await
            .unwrap();

        (auth, inst, epoch, assign_rcpt.event_cursor.sequence)
    };
    // Tous les clients GUI et TUI sont maintenant fermés (droppés).

    // Le daemon conserve exactement son identité et poursuit les transitions backend
    assert_eq!(handle.authority_generation(), auth_before);
    assert_eq!(handle.daemon_instance_id(), inst_before);
    assert_eq!(handle.daemon_epoch(), epoch_before);

    // Un client CLI ouvre une nouvelle connexion et fait avancer la mission, puis ferme aussi
    {
        let mut cli =
            UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Cli, "0.1.0"))
                .await
                .unwrap();
        cli.send_command(&make_cmd(
            auth_before,
            (0xC003, 0xA003),
            Some(project_id),
            mission_id.as_uuid(),
            1,
            "PauseMission",
            json!({ "reason": "progression sans fenêtre GUI" }),
        ))
        .await
        .unwrap();
    }

    // Réouverture de la GUI : même identité + état et historique complets retrouvés
    let mut gui_reopened =
        UnleashClient::connect(ClientConfig::new(sock.clone(), ClientKind::Gui, "0.1.0"))
            .await
            .unwrap();
    assert_eq!(gui_reopened.hello().authority_generation, auth_before);
    assert_eq!(gui_reopened.hello().daemon_instance_id, inst_before);
    assert_eq!(gui_reopened.hello().daemon_epoch, epoch_before);

    let (snapshot, watermark) = gui_reopened
        .get_state_snapshot(RequestId::from_u128(0xA004), Some(project_id), Map::new())
        .await
        .unwrap();
    assert!(watermark.sequence > seq_before);

    let missions = snapshot
        .get("missions")
        .and_then(Value::as_array)
        .expect("missions doit être un tableau");
    let target = missions
        .iter()
        .find(|m| m.get("mission_id").and_then(Value::as_str) == Some(&mission_id.to_string()))
        .expect("mission doit survivre à la fermeture de tous les clients");
    assert_eq!(target.get("state").and_then(Value::as_str), Some("paused"));
    assert_eq!(target.get("revision").and_then(Value::as_u64), Some(2));

    let mut sync = EventSynchronizer::new(
        ClientConfig::new(sock, ClientKind::Gui, "0.1.0"),
        EventFollowerConfig::default(),
    );
    let (init_update, _reader) = sync
        .sync_initial(gui_reopened.connection_mut(), RequestId::from_u128(0xA005))
        .await
        .unwrap();
    assert!(matches!(
        init_update,
        EventSyncUpdate::SnapshotLoaded { watermark: w, .. } if w == watermark
    ));

    handle.shutdown_graceful("test done").await.unwrap();
}
