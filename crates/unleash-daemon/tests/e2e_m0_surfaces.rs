// SPDX-License-Identifier: Apache-2.0
//! E2E transversal M0 multi-surfaces :
//! `unleashd` (binaire réel) + `unleash` CLI `--json` (`unleash tui` inclus) + `unleash-client`
//! + adaptateur commandes `unleash-gui` (`GuiAppState` + `Channel`) + `unleash-tui::TestBackend`.
//!
//! Zéro `sleep` de synchronisation : coordination déterministe via `--ready-json`, canaux bornés
//! et notification de rendu `TestBackend`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::indexing_slicing
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command, Output, Stdio};
use std::task::{Context, Poll};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures_core::Stream;
use ratatui::Terminal;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use serde_json::{Map, Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc;
use unleash_client::{
    ClientConfig, EventFollowerConfig, EventSyncUpdate, EventSynchronizer, UnleashClient,
};
use unleash_domain::{CommandId, MissionId, ProjectId, RequestId, Revision};
use unleash_gui::{
    GuiAppState, GuiCommandInput, GuiQueryInput, GuiStreamNotification, GuiSubscribeInput,
    ensure_operation_kind, tauri_command_name, validate_exposed_operation,
};
use unleash_protocol::{ClientKind, OperationKind, OperationName, ReceiptState};
use unleash_testkit::local_test_dir;
use unleash_tui::{
    BackendCommand, BackendUpdate, ConnectionBanner, MAX_NOTIFICATION_QUEUE, Space,
    TerminalCapabilities, TuiState, apply_backend_update, run_backend_io_worker, run_tui_loop,
};
use uuid::Uuid;

const WAIT_BOUND: Duration = Duration::from_secs(10);

fn unleash_cli_bin() -> PathBuf {
    let daemon_exe = PathBuf::from(env!("CARGO_BIN_EXE_unleashd"));
    let cli_name = format!("unleash{}", std::env::consts::EXE_SUFFIX);
    let candidate = daemon_exe.with_file_name(cli_name);
    assert!(
        candidate.is_file(),
        "Binaire unleash introuvable à '{}' (lancer cargo test --workspace)",
        candidate.display()
    );
    candidate
}

fn spawn_unleashd(data_root: &Path, rdv_dir: &Path) -> (Child, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_unleashd"))
        .arg("--data-root")
        .arg(data_root)
        .arg("--rendezvous-dir")
        .arg(rdv_dir)
        .arg("--foreground")
        .arg("--ready-json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Échec lancement binaire unleashd");

    let stdout = child.stdout.take().expect("stdout pipe requis");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .expect("Lecture ligne --ready-json échouée");
    assert!(
        n > 0,
        "unleashd s'est terminé sans émettre la ligne JSON ready"
    );
    let ready: Value = serde_json::from_str(line.trim()).expect("JSON ready invalide");
    assert_eq!(ready.get("status").and_then(Value::as_str), Some("ready"));
    (child, ready)
}

fn run_cli_raw(socket: &Path, args: &[&str]) -> Output {
    let cli = unleash_cli_bin();
    Command::new(cli)
        .arg("--socket")
        .arg(socket)
        .arg("--json")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("Échec exécution binaire unleash")
}

fn run_cli_json(socket: &Path, args: &[&str]) -> Value {
    let output = run_cli_raw(socket, args);
    assert!(
        output.status.success(),
        "Commande `unleash {}` a échoué (status={}): stderr={}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Sortie JSON CLI invalide")
}

struct ChannelEventStream {
    rx: mpsc::UnboundedReceiver<std::io::Result<Event>>,
}

impl Stream for ChannelEventStream {
    type Item = std::io::Result<Event>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

struct SignalTestBackend {
    inner: TestBackend,
    draw_tx: mpsc::UnboundedSender<String>,
}

impl SignalTestBackend {
    fn snapshot_text(&self) -> String {
        let buf = self.inner.buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }
}

impl Backend for SignalTestBackend {
    type Error = <TestBackend as Backend>::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)?;
        let _ = self.draw_tx.send(self.snapshot_text());
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

async fn wait_for_screen_containing(
    draw_rx: &mut mpsc::UnboundedReceiver<String>,
    needles: &[&str],
) -> String {
    tokio::time::timeout(WAIT_BOUND, async {
        while let Some(frame_text) = draw_rx.recv().await {
            if needles.iter().all(|n| frame_text.contains(n)) {
                return frame_text;
            }
        }
        panic!("Le flux de rendu TestBackend s'est fermé avant d'observer {needles:?}");
    })
    .await
    .expect("Délai dépassé en attente d'une frame TestBackend")
}

async fn recv_gui_event_of_type(
    gui_rx: &mut mpsc::UnboundedReceiver<Value>,
    expected_event_type: &str,
) -> Value {
    tokio::time::timeout(WAIT_BOUND, async {
        while let Some(msg) = gui_rx.recv().await {
            if msg.get("kind").and_then(Value::as_str) == Some("events")
                && let Some(events) = msg.get("events").and_then(Value::as_array)
                && let Some(found) = events.iter().find(|ev| {
                    ev.get("event_type").and_then(Value::as_str) == Some(expected_event_type)
                })
            {
                return found.clone();
            }
        }
        panic!("Canal GUI fermé avant réception de l'événement {expected_event_type}");
    })
    .await
    .expect("Délai dépassé en attente d'un événement sur Channel GUI")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_m0_cross_surface_cli_gui_tui_and_client_convergence() {
    let rdv = local_test_dir("e2e-surf-rdv").unwrap();
    std::fs::set_permissions(
        rdv.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let data = local_test_dir("e2e-surf-dat").unwrap();
    let sock = rdv.path().join("daemon.sock");
    assert!(
        sock.as_os_str().len() <= 103,
        "Socket E2E doit respecter SUN_LEN < 104 octets"
    );

    // 1. Démarrer le vrai binaire `unleashd`
    let (mut daemon_child, ready) = spawn_unleashd(data.path(), rdv.path());
    let auth_str = ready
        .get("authority_generation")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    assert_eq!(ready.get("daemon_epoch").and_then(Value::as_u64), Some(1));

    // 2. CLI `--json` : status + création projet + assignation mission + garde non-TTY `unleash tui`
    let status = run_cli_json(&sock, &["status"]);
    assert_eq!(status.get("mode").and_then(Value::as_str), Some("ready"));
    assert_eq!(
        status.get("authority_generation").and_then(Value::as_str),
        Some(auth_str.as_str())
    );

    let tui_non_tty = run_cli_raw(&sock, &["tui", "--no-color"]);
    assert!(
        !tui_non_tty.status.success(),
        "`unleash tui` lancé sans TTY doit refuser proprement le raw mode"
    );
    let tui_err: Value =
        serde_json::from_slice(&tui_non_tty.stderr).expect("Erreur JSON attendue sur stderr");
    let tui_err_msg = tui_err.get("error").and_then(Value::as_str).unwrap_or("");
    assert!(
        tui_err_msg.contains("Terminal interactif requis"),
        "Message d'erreur inattendu pour `unleash tui` non-TTY : {tui_err_msg}"
    );

    let project_id = ProjectId::from_u128(0x50F0_0001);
    let mission_id = MissionId::from_u128(0x50F0_0002);
    let proj_str = project_id.to_string();
    let mission_str = mission_id.to_string();

    let proj_rcpt = run_cli_json(
        &sock,
        &[
            "project",
            "create",
            "--name",
            "E2E-Surfaces",
            "--project-id",
            &proj_str,
        ],
    );
    assert_eq!(
        proj_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(1)
    );

    let assign_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "assign",
            "--project-id",
            &proj_str,
            "--mission-id",
            &mission_str,
        ],
    );
    assert_eq!(
        assign_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(1)
    );

    // 3. `unleash-client` + `EventSynchronizer` : capture du snapshot initial post-CLI
    let client_cfg = ClientConfig::new(sock.clone(), ClientKind::Tui, "0.1.0")
        .with_timeouts(Duration::from_secs(5), Duration::from_secs(5));
    let mut sync_client = UnleashClient::connect(client_cfg.clone()).await.unwrap();
    let mut synchronizer =
        EventSynchronizer::new(client_cfg.clone(), EventFollowerConfig::default());
    let (initial_sync, mut sync_stream) = synchronizer
        .sync_initial(sync_client.connection_mut(), RequestId::from_u128(0x5100))
        .await
        .unwrap();
    let EventSyncUpdate::SnapshotLoaded {
        snapshot: sync_init_snap,
        ..
    } = initial_sync
    else {
        panic!("SnapshotLoaded attendu lors de sync_initial");
    };
    let init_missions = sync_init_snap
        .get("missions")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(init_missions.len(), 1);
    assert_eq!(
        init_missions[0].get("state").and_then(Value::as_str),
        Some("active")
    );
    assert_eq!(
        init_missions[0].get("revision").and_then(Value::as_u64),
        Some(1)
    );

    // 4. Adaptateur commandes `unleash-gui` (`GuiAppState` + `tauri::ipc::Channel`)
    assert_eq!(
        tauri_command_name(OperationName::PauseMission),
        "op_pause_mission"
    );
    ensure_operation_kind(
        validate_exposed_operation("PauseMission").unwrap(),
        OperationKind::Command,
    )
    .unwrap();
    ensure_operation_kind(
        validate_exposed_operation("GetStateSnapshot").unwrap(),
        OperationKind::Query,
    )
    .unwrap();
    assert!(validate_exposed_operation("RecordClaim").is_err());

    let gui_state = GuiAppState::new(Some(
        ClientConfig::new(sock.clone(), ClientKind::Gui, "0.1.0")
            .with_timeouts(Duration::from_secs(5), Duration::from_secs(5)),
    ));
    let (gui_tx, mut gui_rx) = mpsc::unbounded_channel::<Value>();
    let gui_channel: Channel<GuiStreamNotification> = Channel::new(move |body| {
        if let InvokeResponseBody::Json(raw) = body {
            let parsed: Value = serde_json::from_str(&raw).expect("JSON Channel valide");
            let _ = gui_tx.send(parsed);
        }
        Ok(())
    });
    let _sub_cursor = gui_state
        .subscribe_events(
            GuiSubscribeInput {
                project_id: Some(project_id),
                aggregate_id: None,
                batch_limit: Some(50),
            },
            gui_channel,
        )
        .await
        .unwrap();

    let first_gui_notif = tokio::time::timeout(WAIT_BOUND, gui_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_gui_notif["kind"], "snapshot_loaded");
    assert_eq!(
        first_gui_notif["snapshot"]["missions"][0]["state"],
        "active"
    );

    let gui_snap_before = gui_state
        .execute_query(
            OperationName::GetStateSnapshot,
            GuiQueryInput {
                project_id: Some(project_id),
                payload: Map::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(gui_snap_before.result["missions"][0]["revision"], json!(1));

    // Pause de la mission via l'adaptateur GUI (rev 1 -> 2)
    let gui_pause_cmd_id = CommandId::from_u128(0x50F0_C001);
    let mut pause_payload = Map::new();
    pause_payload.insert(String::from("reason"), json!("pause depuis adaptateur GUI"));
    let gui_pause_rcpt = gui_state
        .submit_command(
            OperationName::PauseMission,
            GuiCommandInput {
                project_id: Some(project_id),
                aggregate_id: mission_id.as_uuid(),
                expected_revision: 1,
                command_id: gui_pause_cmd_id,
                payload: pause_payload,
            },
        )
        .await
        .unwrap();
    assert_eq!(gui_pause_rcpt.state, ReceiptState::Accepted);
    assert_eq!(gui_pause_rcpt.new_revision, Revision::new(2));

    let paused_ev = recv_gui_event_of_type(&mut gui_rx, "MissionPaused").await;
    assert_eq!(paused_ev["aggregate_revision"], json!(2));

    // 5. `unleash-tui::TestBackend` + `run_tui_loop` + `run_backend_io_worker` sur le vrai daemon
    let (draw_tx, mut draw_rx) = mpsc::unbounded_channel::<String>();
    let backend = SignalTestBackend {
        inner: TestBackend::new(120, 40),
        draw_tx,
    };
    let mut terminal = Terminal::new(backend).unwrap();
    let mut tui_state = TuiState::with_m0_fixture(TerminalCapabilities::default());
    let (cmd_tx, cmd_rx) = mpsc::channel::<BackendCommand>(16);
    let (upd_tx, upd_rx) = mpsc::channel::<BackendUpdate>(MAX_NOTIFICATION_QUEUE);
    let io_task = tokio::spawn(run_backend_io_worker(
        Some(client_cfg.clone()),
        cmd_rx,
        upd_tx,
    ));

    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let tui_loop_fut = run_tui_loop(
        &mut terminal,
        &mut tui_state,
        ChannelEventStream { rx: ev_rx },
        Some(&cmd_tx),
        Some(upd_rx),
    );
    let cmd_tx_driver = cmd_tx.clone();
    let tui_driver_fut = async move {
        wait_for_screen_containing(&mut draw_rx, &["IPC v2: seq"]).await;
        cmd_tx_driver
            .send(BackendCommand::ExecuteQuery {
                operation: OperationName::GetStateSnapshot,
            })
            .await
            .unwrap();
        wait_for_screen_containing(&mut draw_rx, &["GetStateSnapshot:", "watermark="]).await;

        let _ = ev_tx.send(Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('3'),
            KeyModifiers::NONE,
        ))));
        wait_for_screen_containing(&mut draw_rx, &["Espace : Agents"]).await;

        let _ = ev_tx.send(Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        ))));
    };

    let (loop_res, ()) = tokio::join!(tui_loop_fut, tui_driver_fut);
    loop_res.unwrap();
    drop(cmd_tx);
    io_task.await.unwrap();

    assert!(tui_state.should_quit);
    assert_eq!(tui_state.active_space, Space::Agents);
    let final_tui_screen = terminal.backend().snapshot_text();
    assert!(final_tui_screen.contains("IPC v2: seq"));
    assert!(final_tui_screen.contains("GetStateSnapshot:"));
    assert!(final_tui_screen.contains("Espace : Agents"));

    // 6. Reprise de la mission via CLI `--json` (rev 2 -> 3) et vérification de convergence multi-surfaces
    let cli_resume_cmd_id = Uuid::from_u128(0x50F0_C002);
    let cli_resume_cmd_str = cli_resume_cmd_id.to_string();
    let resume_rcpt = run_cli_json(
        &sock,
        &[
            "mission",
            "resume",
            &mission_str,
            "--project-id",
            &proj_str,
            "--expected-revision",
            "2",
            "--reason",
            "reprise depuis CLI",
            "--command-id",
            &cli_resume_cmd_str,
        ],
    );
    assert_eq!(
        resume_rcpt.get("new_revision").and_then(Value::as_u64),
        Some(3)
    );
    let resume_seq = resume_rcpt["event_cursor"]["sequence"].as_u64().unwrap();

    // Le flux GUI reçoit MissionResumed en temps réel
    let resumed_ev = recv_gui_event_of_type(&mut gui_rx, "MissionResumed").await;
    assert_eq!(resumed_ev["aggregate_revision"], json!(3));

    // Inspection croisée des reçus : CLI lit le reçu GUI, GUI lit le reçu CLI
    let cli_reads_gui_rcpt = run_cli_json(&sock, &["receipt", &gui_pause_cmd_id.to_string()]);
    assert_eq!(cli_reads_gui_rcpt["lineage_status"], "current");
    assert_eq!(cli_reads_gui_rcpt["receipt"]["new_revision"], json!(2));

    let mut rcpt_payload = Map::new();
    rcpt_payload.insert(String::from("command_id"), json!(cli_resume_cmd_str));
    let gui_reads_cli_rcpt = gui_state
        .execute_query(
            OperationName::GetCommandReceipt,
            GuiQueryInput {
                project_id: None,
                payload: rcpt_payload,
            },
        )
        .await
        .unwrap();
    assert_eq!(gui_reads_cli_rcpt.result["lineage_status"], "current");
    assert_eq!(
        gui_reads_cli_rcpt.result["receipt"]["new_revision"],
        json!(3)
    );

    // Convergence de EventSynchronizer jusqu'au curseur de reprise sans trou
    let mut synced_events = Vec::new();
    tokio::time::timeout(WAIT_BOUND, async {
        while synchronizer
            .current_cursor()
            .is_none_or(|c| c.sequence.as_u64() < resume_seq)
        {
            let (upd, maybe_reader) = synchronizer
                .poll_next(&mut sync_stream, sync_client.connection_mut(), || {
                    RequestId::from_u128(0x5200)
                })
                .await
                .unwrap();
            if let Some(nr) = maybe_reader {
                sync_stream = nr;
            }
            if let EventSyncUpdate::Events { events, .. } = upd {
                for ev in events {
                    synced_events.push((ev.sequence.as_u64(), ev.event_type));
                }
            }
        }
    })
    .await
    .expect("Timeout convergence EventSynchronizer");

    assert!(synced_events.iter().any(|(_, t)| t == "MissionPaused"));
    assert!(synced_events.iter().any(|(_, t)| t == "MissionResumed"));

    // Convergence stricte des snapshots entre CLI, GUI et unleash-client au même watermark
    let cli_snap_wrap = run_cli_json(&sock, &["snapshot", "--project-id", &proj_str]);
    let gui_snap = gui_state
        .execute_query(
            OperationName::GetStateSnapshot,
            GuiQueryInput {
                project_id: Some(project_id),
                payload: Map::new(),
            },
        )
        .await
        .unwrap();
    let (client_snap, client_wm) = sync_client
        .get_state_snapshot(RequestId::from_u128(0x5300), Some(project_id), Map::new())
        .await
        .unwrap();

    assert_eq!(
        cli_snap_wrap["watermark"],
        serde_json::to_value(client_wm).unwrap()
    );
    assert_eq!(gui_snap.watermark, Some(client_wm));
    assert_eq!(cli_snap_wrap["snapshot"], client_snap);
    assert_eq!(gui_snap.result, client_snap);
    assert_eq!(client_snap["missions"][0]["state"], "active");
    assert_eq!(client_snap["missions"][0]["revision"], json!(3));

    // 7. Arrêt propre du daemon par CLI + résilience déconnexion TUI (brouillons préservés)
    drop(gui_state);
    drop(sync_stream);
    drop(sync_client);

    let stop_out = run_cli_json(&sock, &["daemon", "stop"]);
    assert_eq!(
        stop_out.get("stop_outcome").and_then(Value::as_str),
        Some("complete")
    );
    let daemon_exit = daemon_child.wait().expect("wait unleashd après stop");
    assert!(
        daemon_exit.success(),
        "unleashd doit sortir avec code 0 après `daemon stop`"
    );

    let (cmd_tx2, cmd_rx2) = mpsc::channel::<BackendCommand>(4);
    let (upd_tx2, mut upd_rx2) = mpsc::channel::<BackendUpdate>(8);
    let io_after_stop = tokio::spawn(run_backend_io_worker(Some(client_cfg), cmd_rx2, upd_tx2));
    let upd = tokio::time::timeout(WAIT_BOUND, upd_rx2.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(upd, BackendUpdate::Disconnected { .. }));
    let draft_before = tui_state.editor.text();
    apply_backend_update(&mut tui_state, upd);
    assert!(matches!(
        tui_state.connection,
        ConnectionBanner::Disconnected { .. }
    ));
    assert_eq!(tui_state.editor.text(), draft_before);
    drop(cmd_tx2);
    io_after_stop.await.unwrap();
}
