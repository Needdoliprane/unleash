// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration de sécurité `WebView`, capabilities Tauri 2, CSP, AST Markdown
//! et survie du daemon après fermeture/crash de l'hôte GUI (`AC-F014-013`, `AC-F014-014`).
//!
//! Utilise `tauri::test::mock_builder()` afin de valider la vraie configuration
//! `tauri.conf.json`, les vraies capabilities et le vrai routeur `generate_handler!`
//! sans ouvrir de fenêtre bloquante en CI headless.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use tauri::Manager;
use tauri::ipc::{CallbackFn, Channel, InvokeBody, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{
    AuthorityGeneration, CommandId, DaemonEpoch, DaemonInstanceId, OperationId, ReceiptId,
    RequestId, Revision, Sequence, Sha256Digest, StreamId,
};
use unleash_gui::{
    FORBIDDEN_GUI_OPERATIONS, GuiAppState, GuiSubscribeInput, MAX_GUI_PAYLOAD_BYTES,
    configure_builder, exposed_gui_operations, parse_markdown_ast, tauri_command_name,
    ts_export::{generate_gui_ts_files, write_gui_ts_bindings},
    validate_exposed_operation, validate_webview_payload,
};
use unleash_platform::{bind_private_unix_listener, verify_same_uid_peer};
use unleash_protocol::{
    ClientKind, CommandEnvelope, CommandReceipt, EventCausation, EventCursor, EventEnvelope,
    HelloRequest, HelloResponse, MAX_FRAME_BYTES, OperationName, ProtocolVersion, QueryEnvelope,
    QueryResponse, ReceiptState, Sensitivity, SubscribeEventsFrame, SubscribeEventsRequest,
    from_json_str_strict,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

fn test_url() -> tauri::Url {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
    .parse()
    .expect("URL Tauri locale valide")
}

fn build_invoke_request(cmd: &str, body_json: Value) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.to_owned(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: test_url(),
        body: InvokeBody::Json(body_json),
        headers: tauri::http::HeaderMap::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_owned(),
    }
}

#[derive(Default)]
struct FixtureDaemonState {
    recorded_queries: Vec<(String, String)>,
    recorded_commands: Vec<(String, String, Option<String>)>,
    receipts: std::collections::HashMap<CommandId, CommandReceipt>,
    job_ticks: u64,
}

struct FixtureDaemon {
    socket_path: PathBuf,
    state: Arc<Mutex<FixtureDaemonState>>,
    accept_task: JoinHandle<()>,
    _sandbox: tempfile::TempDir,
}

impl FixtureDaemon {
    fn start() -> Self {
        let sandbox = local_test_dir("gui-daemon").expect("répertoire temporaire local");
        let rdv = sandbox.path().join("rdv");
        unleash_platform::ensure_private_rendezvous_dir(&rdv).expect("rendez-vous privé");
        let socket_path = rdv.join("daemon.sock");
        let std_listener = bind_private_unix_listener(&socket_path).expect("socket Unix privé");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let listener = UnixListener::from_std(std_listener).expect("UnixListener Tokio");

        let state = Arc::new(Mutex::new(FixtureDaemonState::default()));
        let state_bg = Arc::clone(&state);
        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let st = Arc::clone(&state_bg);
                tokio::spawn(async move {
                    let _ = handle_fixture_conn(stream, st).await;
                });
            }
        });

        Self {
            socket_path,
            state,
            accept_task,
            _sandbox: sandbox,
        }
    }

    fn gui_config(&self) -> ClientConfig {
        ClientConfig::new(self.socket_path.clone(), ClientKind::Gui, "0.1.0")
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(3))
    }

    fn cli_config(&self) -> ClientConfig {
        ClientConfig::new(self.socket_path.clone(), ClientKind::Cli, "0.1.0")
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(3))
    }
}

impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn read_frame(stream: &mut UnixStream) -> std::io::Result<Option<String>> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(header) as usize;
    assert!(len <= MAX_FRAME_BYTES);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Some(String::from_utf8(body).expect("UTF-8 valide")))
}

async fn write_json<T: serde::Serialize>(stream: &mut UnixStream, val: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(val).expect("sérialisation JSON");
    let len = u32::try_from(bytes.len()).expect("longueur trame <= u32::MAX");
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

async fn handle_fixture_conn(
    mut stream: UnixStream,
    state: Arc<Mutex<FixtureDaemonState>>,
) -> std::io::Result<()> {
    verify_same_uid_peer(&stream)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
    let peer_actor_ref = format!("uid:{}", unleash_platform::effective_uid());

    let Some(hello_raw) = read_frame(&mut stream).await? else {
        return Ok(());
    };
    let _hello: HelloRequest = from_json_str_strict(&hello_raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let stream_id = StreamId::from_u128(0x0101_0000_0000_0000_0000_0000_0000_0001);
    let authority_generation =
        AuthorityGeneration::from_u128(0x2000_0000_0000_0000_0000_0000_0000_0001);
    let hello_resp = HelloResponse {
        protocol_version: ProtocolVersion,
        authority_generation,
        daemon_epoch: DaemonEpoch::new(1),
        daemon_instance_id: DaemonInstanceId::from_u128(0xD000_0007),
        stream_id,
        max_frame_bytes: u32::try_from(MAX_FRAME_BYTES).expect("u32"),
        suggested_segment_bytes: 32 * 1024,
        idempotency_retention_seconds: 604_800,
        capabilities: vec![String::from("core_v2"), String::from("events")],
    };
    write_json(&mut stream, &hello_resp).await?;

    while let Some(raw) = read_frame(&mut stream).await? {
        if let Ok(query) = from_json_str_strict::<QueryEnvelope>(&raw) {
            let resp = {
                let mut st = state.lock().expect("lock state");
                st.recorded_queries
                    .push((query.query_type.clone(), peer_actor_ref.clone()));

                if query.query_type == "GetCommandReceipt" {
                    let cmd_id_str = query
                        .payload
                        .get("command_id")
                        .and_then(Value::as_str)
                        .expect("command_id");
                    let cmd_id: CommandId = cmd_id_str.parse().expect("CommandId valide");
                    let receipt = st.receipts.get(&cmd_id).cloned().expect("reçu connu");
                    let inspection = unleash_protocol::CommandReceiptInspection {
                        receipt,
                        authority_generation,
                        lineage_status: unleash_protocol::LineageStatus::Current,
                        provenance_ref: Some(String::from("journal:seq:1")),
                    };
                    QueryResponse {
                        request_id: query.request_id,
                        watermark: Some(EventCursor::new(stream_id, Sequence::new(st.job_ticks))),
                        result: serde_json::to_value(inspection).expect("serialize inspection"),
                    }
                } else if query.query_type == "GetStateSnapshot" {
                    QueryResponse {
                        request_id: query.request_id,
                        watermark: Some(EventCursor::new(stream_id, Sequence::new(1))),
                        result: json!({
                            "snapshot_ready": true,
                            "agents": 1000,
                            "resolved_actor": peer_actor_ref,
                        }),
                    }
                } else {
                    st.job_ticks += 1;
                    let ticks = st.job_ticks;
                    QueryResponse {
                        request_id: query.request_id,
                        watermark: Some(EventCursor::new(stream_id, Sequence::new(ticks))),
                        result: json!({
                            "mission_status": "active",
                            "job_ticks": ticks,
                            "resolved_actor": peer_actor_ref,
                        }),
                    }
                }
            };
            write_json(&mut stream, &resp).await?;
        } else if let Ok(sub_req) = from_json_str_strict::<SubscribeEventsRequest>(&raw) {
            let start_seq = sub_req.after_sequence.as_u64().saturating_add(1);
            let make_ev = |seq: u64, etype: &str| EventEnvelope {
                event_id: Uuid::from_u128(0xE000_0000 + u128::from(seq)),
                stream_id,
                sequence: Sequence::new(seq),
                event_type: String::from(etype),
                schema_version: 1,
                occurred_at: "2026-11-17T09:30:00Z".parse().expect("ts"),
                recorded_at: "2026-11-17T09:30:00Z".parse().expect("ts"),
                project_id: sub_req.project_id,
                aggregate_id: Uuid::from_u128(1),
                aggregate_revision: Revision::new(seq),
                actor_ref: peer_actor_ref.clone(),
                correlation_id: Uuid::from_u128(0xC000_0000 + u128::from(seq)),
                causation: EventCausation::from_command_receipt(ReceiptId::from_u128(1)),
                command_id: None,
                payload_hash: Sha256Digest::from_bytes(b"stream_event"),
                payload: Map::new(),
                sensitivity: Sensitivity::Public,
            };
            let frame1 = SubscribeEventsFrame::Events {
                events: vec![make_ev(start_seq, "ActionDispatched")],
                cursor: EventCursor::new(stream_id, Sequence::new(start_seq)),
            };
            write_json(&mut stream, &frame1).await?;
            tokio::task::yield_now().await;
            let frame2 = SubscribeEventsFrame::Events {
                events: vec![make_ev(start_seq + 1, "ObservationRecorded")],
                cursor: EventCursor::new(stream_id, Sequence::new(start_seq + 1)),
            };
            write_json(&mut stream, &frame2).await?;
            // Maintenir la connexion de flux ouverte tant que le client est connecté
            let _ = read_frame(&mut stream).await;
            break;
        } else if let Ok(cmd) = from_json_str_strict::<CommandEnvelope>(&raw) {
            let receipt = {
                let mut st = state.lock().expect("lock state");
                st.recorded_commands.push((
                    cmd.command_type.clone(),
                    peer_actor_ref.clone(),
                    cmd.actor_ref.clone(),
                ));
                st.job_ticks += 1;
                let ts: DateTime<Utc> = "2026-11-17T09:30:00Z".parse().expect("timestamp");
                st.receipts
                    .get(&cmd.command_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        let r = CommandReceipt {
                            receipt_id: ReceiptId::from_u128(
                                0x9000_0000 + u128::from(st.job_ticks),
                            ),
                            command_id: cmd.command_id,
                            accepted_at: ts,
                            aggregate_id: cmd.aggregate_id,
                            new_revision: cmd.expected_revision.next(),
                            operation_id: Some(OperationId::from_u128(0x8000_0001)),
                            state: ReceiptState::Accepted,
                            event_cursor: EventCursor::new(stream_id, Sequence::new(st.job_ticks)),
                            result_ref: None,
                        };
                        st.receipts.insert(cmd.command_id, r.clone());
                        r
                    })
            };
            write_json(&mut stream, &receipt).await?;
        } else {
            break;
        }
    }
    Ok(())
}

#[test]
fn t_gui_registry_matches_200_exposed_catalog_operations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog_path = root.join(".claude/catalog/operations.json");
    let raw = fs::read_to_string(&catalog_path).expect("operations.json");
    let catalog: Value = serde_json::from_str(&raw).expect("JSON catalogue valide");
    let ops = catalog["operations"].as_array().expect("liste operations");

    let expected_exposed: BTreeSet<String> = ops
        .iter()
        .filter(|o| {
            o["parity_group_ids"]
                .as_array()
                .is_some_and(|pgs| !pgs.is_empty())
        })
        .map(|o| o["name"].as_str().expect("nom").to_owned())
        .collect();
    assert_eq!(expected_exposed.len(), 200);

    let rust_exposed: BTreeSet<String> = exposed_gui_operations()
        .into_iter()
        .map(|op| op.as_str().to_owned())
        .collect();
    assert_eq!(rust_exposed, expected_exposed);

    for forbidden in FORBIDDEN_GUI_OPERATIONS {
        assert!(!rust_exposed.contains(forbidden.as_str()));
        assert!(validate_exposed_operation(forbidden.as_str()).is_err());
    }
    assert!(validate_exposed_operation("UnknownAdminProxy").is_err());
    assert_eq!(
        tauri_command_name(OperationName::GetOverview),
        "op_get_overview"
    );
}

fn build_test_app(state: GuiAppState) -> tauri::App<tauri::test::MockRuntime> {
    configure_builder(tauri::test::mock_builder(), state)
        .build(tauri::generate_context!())
        .expect("build Tauri MockRuntime avec tauri.conf.json réel")
}

#[test]
fn t_f014_014_tauri_capabilities_csp_markdown_ast_and_actor_spoofing() {
    // T-F014-014 / AC-F014-014 / REQ-UI-014
    let app = build_test_app(GuiAppState::new(None));

    // 1. Vérifier la CSP de production stricte réellement embarquée
    let csp_str = app
        .config()
        .app
        .security
        .csp
        .as_ref()
        .expect("CSP configurée")
        .to_string();
    assert!(csp_str.contains("default-src 'self'"));
    assert!(csp_str.contains("script-src 'self'"));
    assert!(csp_str.contains("connect-src 'none'"));
    assert!(csp_str.contains("object-src 'none'"));
    assert!(!csp_str.contains("unsafe-eval"));
    assert!(!csp_str.contains("wasm-unsafe-eval"));
    assert!(!csp_str.contains("https://"));

    // 2. Fenêtre autorisée `main` vs fenêtre non autorisée `untrusted`
    let main_window = app.get_webview_window("main").unwrap_or_else(|| {
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
            .build()
            .expect("fenêtre main")
    });
    let untrusted_window =
        tauri::WebviewWindowBuilder::new(&app, "untrusted", tauri::WebviewUrl::default())
            .build()
            .expect("fenêtre untrusted");

    // `main` peut appeler `gui_list_exposed_operations` et `gui_render_markdown_ast`
    let list_res = tauri::test::get_ipc_response(
        &main_window,
        build_invoke_request("gui_list_exposed_operations", json!({})),
    )
    .expect("fenêtre main autorisée pour gui_list_exposed_operations");
    let listed_ops: Vec<String> = list_res.deserialize().expect("désérialisation Vec<String>");
    assert_eq!(listed_ops.len(), 200);
    assert!(!listed_ops.contains(&String::from("RecordClaim")));
    assert!(!listed_ops.contains(&String::from("RecordObservation")));
    assert!(!listed_ops.contains(&String::from("RecordJudgment")));
    assert!(!listed_ops.contains(&String::from("Hello")));

    // `untrusted` est rejetée par les capabilities Tauri sur commande applicative
    let untrusted_res = tauri::test::get_ipc_response(
        &untrusted_window,
        build_invoke_request("gui_list_exposed_operations", json!({})),
    );
    assert!(
        untrusted_res.is_err(),
        "une fenêtre hors capability `main` doit être rejetée par Tauri"
    );

    // Commande interdite ou inconnue rejetée même depuis `main`
    for forbidden_cmd in [
        "op_hello",
        "op_record_claim",
        "op_record_observation",
        "op_record_judgment",
        "generic_proxy",
    ] {
        let res = tauri::test::get_ipc_response(
            &main_window,
            build_invoke_request(forbidden_cmd, json!({})),
        );
        assert!(
            res.is_err(),
            "la commande {forbidden_cmd} ne doit pas être enregistrée"
        );
    }

    // 3. Neutralisation Markdown AST côté Rust (HTML/SVG/URL hostiles)
    let md_res = tauri::test::get_ipc_response(
        &main_window,
        build_invoke_request(
            "gui_render_markdown_ast",
            json!({
                "input": {
                    "markdown": "# Audit\n\n<script>alert(1)</script>\n\n<svg onload=\"alert(2)\"></svg>\n\n[Piege](javascript:alert(3))\n\n[Valide](https://example.org)"
                }
            }),
        ),
    )
    .expect("gui_render_markdown_ast réussi sur main");
    let ast: unleash_gui::MarkdownDocumentAst = md_res.deserialize().expect("AST valide");
    assert!(ast.blocked_raw_html_count >= 1);
    assert_eq!(ast.blocked_url_count, 1);

    let oversized_markdown = "a".repeat(unleash_gui::MAX_MARKDOWN_INPUT_BYTES + 10);
    assert!(parse_markdown_ast(&oversized_markdown).is_err());

    // 4. Rejet payload hostile (usurpation actor_ref & dépassement taille)
    let mut forged = Map::new();
    forged.insert(String::from("actor_ref"), json!("admin:root"));
    assert!(validate_webview_payload(&forged).is_err());

    let mut oversized_payload = Map::new();
    oversized_payload.insert(
        String::from("blob"),
        Value::String("x".repeat(MAX_GUI_PAYLOAD_BYTES + 128)),
    );
    assert!(validate_webview_payload(&oversized_payload).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_f014_013_daemon_survives_gui_close_and_actor_ref_is_kernel_derived() {
    // T-F014-013 / AC-F014-013 / REQ-UI-013
    let daemon = FixtureDaemon::start();
    let command_id = CommandId::from_u128(0x7777_0000_0000_0000_0000_0000_0000_0001);
    let aggregate_id = Uuid::from_u128(0xAAAA_0000_0000_0000_0000_0000_0000_0001);

    {
        let gui_state = GuiAppState::new(Some(daemon.gui_config()));
        let app = build_test_app(gui_state);
        let main_win = app.get_webview_window("main").unwrap_or_else(|| {
            tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
                .build()
                .expect("fenêtre main")
        });

        // Requête `op_get_overview` via l'adaptateur Tauri -> `unleash-client` -> daemon socket Unix
        let overview_res = tauri::test::get_ipc_response(
            &main_win,
            build_invoke_request(
                "op_get_overview",
                json!({
                    "input": {
                        "project_id": null,
                        "payload": {}
                    }
                }),
            ),
        )
        .expect("op_get_overview réussit");
        let query_resp: QueryResponse = overview_res.deserialize().expect("QueryResponse");
        assert_eq!(query_resp.result["mission_status"], "active");

        // Tentative de soumission sur une VRAIE commande mutante enregistrée (`op_pause_mission`)
        // avec champ inconnu `actor_ref` au sommet de `GuiCommandInput` (`#[serde(deny_unknown_fields)]`).
        let spoof_res = tauri::test::get_ipc_response(
            &main_win,
            build_invoke_request(
                "op_pause_mission",
                json!({
                    "input": {
                        "project_id": null,
                        "aggregate_id": aggregate_id,
                        "expected_revision": 0,
                        "command_id": command_id,
                        "actor_ref": "forged:superuser",
                        "payload": {}
                    }
                }),
            ),
        );
        let spoof_err = spoof_res.expect_err(
            "un actor_ref forgé dans GuiCommandInput doit être rejeté par deny_unknown_fields",
        );
        let spoof_err_text = spoof_err.to_string();
        assert!(
            spoof_err_text.contains("actor_ref") || spoof_err_text.contains("unknown field"),
            "le rejet doit provenir de deny_unknown_fields sur actor_ref (pas d'une commande inconnue) : {spoof_err_text}"
        );

        // Tentative de soumission avec `actor_ref` imbriqué dans `payload` sur `op_pause_mission`
        let spoof_payload_res = tauri::test::get_ipc_response(
            &main_win,
            build_invoke_request(
                "op_pause_mission",
                json!({
                    "input": {
                        "project_id": null,
                        "aggregate_id": aggregate_id,
                        "expected_revision": 0,
                        "command_id": command_id,
                        "payload": { "actor_ref": "forged:payload" }
                    }
                }),
            ),
        );
        let spoof_payload_err =
            spoof_payload_res.expect_err("un actor_ref dans payload doit être rejeté avant envoi");
        let spoof_payload_text = spoof_payload_err.to_string();
        assert!(
            spoof_payload_text.contains("InvalidArgument")
                && spoof_payload_text.contains("actor_ref"),
            "le rejet payload doit être InvalidArgument sur actor_ref : {spoof_payload_text}"
        );
        assert!(
            daemon
                .state
                .lock()
                .expect("lock")
                .recorded_commands
                .is_empty(),
            "aucune commande usurpée ne doit atteindre le daemon"
        );

        // Soumission valide `op_pause_mission`
        let cmd_res = tauri::test::get_ipc_response(
            &main_win,
            build_invoke_request(
                "op_pause_mission",
                json!({
                    "input": {
                        "project_id": null,
                        "aggregate_id": aggregate_id,
                        "expected_revision": 0,
                        "command_id": command_id,
                        "payload": { "reason": "inspection_m0" }
                    }
                }),
            ),
        )
        .expect("op_pause_mission réussit");
        let receipt: CommandReceipt = cmd_res.deserialize().expect("CommandReceipt");
        assert_eq!(receipt.command_id, command_id);
        assert_eq!(receipt.state, ReceiptState::Accepted);

        // Fermeture / destruction complète de la fenêtre et de l'hôte GUI (drop)
        let _ = main_win.close();
        drop(app);
    }

    // Vérifier auprès du daemon qu'aucune commande d'arrêt implicite n'a été émise
    {
        let st = daemon.state.lock().expect("lock");
        assert_eq!(st.recorded_commands.len(), 1);
        assert_eq!(st.recorded_commands[0].0, "PauseMission");
        assert!(
            st.recorded_commands[0].2.is_none(),
            "l'hôte GUI ne doit jamais propager un actor_ref forgé"
        );
        assert!(
            !st.recorded_commands
                .iter()
                .any(|(c, _, _)| c == "CancelMission" || c == "StopProcess" || c == "StopDaemon")
        );
    }

    // Reconnexion d'un client indépendant (CLI/TUI) prouvant la survie du daemon et la reprise du reçu
    let mut cli = UnleashClient::connect(daemon.cli_config())
        .await
        .expect("le daemon reste joignable après fermeture/crash de l'hôte GUI");
    let inspection = cli
        .get_command_receipt(RequestId::from_u128(0x9999), command_id)
        .await
        .expect("reçu retrouvé par command_id");
    assert_eq!(inspection.receipt.command_id, command_id);
    assert_eq!(inspection.receipt.state, ReceiptState::Accepted);
}

#[test]
fn t_gui_typescript_bindings_in_sync_with_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generated_gui_dir = root.join("apps/gui-renderer/src/generated-gui");

    if std::env::var_os("UPDATE_TS_BINDINGS").is_some_and(|v| !v.is_empty() && v != "0") {
        write_gui_ts_bindings(&generated_gui_dir).expect("écriture bindings TS unleash-gui");
    }

    assert!(
        generated_gui_dir.is_dir(),
        "Le dossier {} doit exister (exécuter avec UPDATE_TS_BINDINGS=1 pour le générer)",
        generated_gui_dir.display()
    );

    let expected_files = generate_gui_ts_files();
    for (filename, expected_content) in &expected_files {
        assert!(
            !expected_content.contains("bigint"),
            "Aucun type 'bigint' autorisé dans {filename}"
        );
        assert!(
            !expected_content.contains(": any") && !expected_content.contains("<any>"),
            "Aucun type 'any' autorisé dans {filename}"
        );
        let repo_file = generated_gui_dir.join(filename);
        let repo_content = fs::read_to_string(&repo_file)
            .unwrap_or_else(|e| panic!("Impossible de lire {} : {e}", repo_file.display()));
        assert_eq!(
            repo_content, *expected_content,
            "Le binding GUI {filename} diverge de ts-rs !"
        );
    }

    for entry in fs::read_dir(&generated_gui_dir).expect("read_dir generated-gui") {
        let name = entry
            .expect("dirent")
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            expected_files.contains_key(&name),
            "Fichier orphelin inattendu dans generated-gui/ : {name}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t_gui_subscribe_events_streams_continuously() {
    let daemon = FixtureDaemon::start();
    let gui_state = GuiAppState::new(Some(daemon.gui_config()));

    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let channel: Channel<unleash_gui::GuiStreamNotification> = Channel::new(move |body| {
        if let InvokeResponseBody::Json(raw) = body {
            let parsed: Value = serde_json::from_str(&raw).expect("JSON Channel valide");
            let _ = tx.send(parsed);
        }
        Ok(())
    });

    let watermark = gui_state
        .subscribe_events(
            GuiSubscribeInput {
                project_id: None,
                aggregate_id: None,
                batch_limit: Some(50),
            },
            channel,
        )
        .await
        .expect("subscribe_events initialise le snapshot et démarre le flux continu");
    assert_eq!(watermark.sequence.as_u64(), 1);

    let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout snapshot")
        .expect("notification snapshot");
    assert_eq!(first["kind"], "snapshot_loaded");
    assert_eq!(first["snapshot"]["snapshot_ready"], true);

    // Deux lots continus reçus après retour de subscribe_events (preuve que _reader n'est pas droppé)
    let second = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout lot 1 continu")
        .expect("notification events lot 1");
    assert_eq!(second["kind"], "events");
    assert_eq!(second["events"][0]["event_type"], "ActionDispatched");
    assert_eq!(second["cursor"]["sequence"], 2);

    let third = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout lot 2 continu")
        .expect("notification events lot 2");
    assert_eq!(third["kind"], "events");
    assert_eq!(third["events"][0]["event_type"], "ObservationRecorded");
    assert_eq!(third["cursor"]["sequence"], 3);

    // La destruction de GuiAppState annule proprement la tâche de lecture continue
    drop(gui_state);
}
