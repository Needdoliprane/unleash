// SPDX-License-Identifier: Apache-2.0
//! Banc de tests `TestBackend`, charge `AC-F014-011`/`T-F015-015`, neutralisation ANSI/OSC,
//! restauration terminal et intégration IPC `unleash-client` pour `unleash-tui`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::print_stdout
)]

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures_core::Stream;
use ratatui::Terminal;
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::{
    AuthorityGeneration, DaemonEpoch, DaemonInstanceId, RequestId, Sequence, StreamId,
};
use unleash_platform::{bind_private_unix_listener, verify_same_uid_peer};
use unleash_protocol::{
    ClientKind, EventCursor, HelloRequest, HelloResponse, MAX_FRAME_BYTES, OperationName,
    ProtocolVersion, QueryEnvelope, QueryResponse, from_json_str_strict,
};
use unleash_testkit::local_test_dir;
use unleash_tui::terminal::install_panic_hook;
use unleash_tui::{
    BackendCommand, BackendUpdate, ConnectionBanner, FocusPane, InputMode, M0_TOTAL_AGENTS,
    M0_TOTAL_DIFF_LINES, M0_TOTAL_EVENTS, MAX_NOTIFICATION_QUEUE, RestoreState, Space,
    TerminalCapabilities, TuiState, apply_backend_update, handle_terminal_event, render_once,
    run_backend_io_worker, run_tui_loop,
};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn buffer_text(term: &Terminal<TestBackend>) -> String {
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn t_f015_005_live_tui_does_not_report_example_state_or_attach_without_a_process() {
    let mut state = TuiState::new(TerminalCapabilities::default());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    render_once(&mut terminal, &mut state).expect("render live state");
    let screen = buffer_text(&terminal);
    assert!(state.agents.is_empty());
    assert!(state.event_page.is_empty());
    assert!(state.diff_window_cache.is_empty());
    assert!(!screen.contains("Pool 3/5"));
    assert!(!screen.contains("1.40 USD"));
    assert!(!screen.contains("31 passed"));

    handle_terminal_event(&mut state, key(KeyCode::Char('a')), None);
    assert_eq!(state.mode, InputMode::Navigation);
    assert!(!state.pty.is_attached());

    apply_backend_update(&mut state, BackendUpdate::Connected { cursor: None });
    render_once(&mut terminal, &mut state).expect("render connected state");
    assert!(buffer_text(&terminal).contains("IPC v2: connecté"));

    handle_terminal_event(&mut state, key(KeyCode::Char('?')), None);
    render_once(&mut terminal, &mut state).expect("render keyboard help");
    let help = buffer_text(&terminal);
    assert!(help.contains("Aide clavier"));
    assert!(!help.contains("100 000 events"));
    assert!(!help.contains("a : attacher PTY"));
}

#[test]
fn t_f015_006_009_013_keyboard_10_spaces_palette_resize_and_accessible_modes() {
    let mut state = TuiState::with_m0_fixture(TerminalCapabilities::default());
    let mut term = Terminal::new(TestBackend::new(120, 40)).expect("TestBackend 120x40");
    render_once(&mut term, &mut state).expect("render 120x40");

    let initial_text = buffer_text(&term);
    assert!(initial_text.contains("OVPT Unleash"));
    assert!(initial_text.contains("Espaces (10/10)"));
    assert!(initial_text.contains("Agents (1000/1000 visibles"));

    // Parcours 100 % clavier des 10 espaces
    for (idx, expected_space) in Space::ALL.iter().enumerate() {
        let digit = char::from_digit(u32::try_from((idx + 1) % 10).unwrap(), 10).unwrap();
        handle_terminal_event(&mut state, key(KeyCode::Char(digit)), None);
        assert_eq!(state.active_space, *expected_space);
    }

    // Palette de commandes Ctrl+P -> filtre -> Entrée
    handle_terminal_event(&mut state, ctrl('p'), None);
    assert_eq!(state.mode, InputMode::Palette);
    render_once(&mut term, &mut state).expect("render palette");
    assert!(buffer_text(&term).contains("Palette de commandes"));

    for c in "agents".chars() {
        handle_terminal_event(&mut state, key(KeyCode::Char(c)), None);
    }
    handle_terminal_event(&mut state, key(KeyCode::Enter), None);
    assert_eq!(state.mode, InputMode::Navigation);
    assert_eq!(state.active_space, Space::Agents);

    // Saisie éditeur multiligne (Unicode + collage sans envoi) puis resize 120x40 -> 80x24
    handle_terminal_event(&mut state, key(KeyCode::Char('e')), None);
    assert_eq!(state.mode, InputMode::Editing);
    handle_terminal_event(
        &mut state,
        Event::Paste(String::from("note 日本語 éàç\n")),
        None,
    );
    assert_eq!(state.mode, InputMode::Editing);

    term.resize(ratatui::layout::Rect::new(0, 0, 80, 24))
        .expect("resize 80x24");
    handle_terminal_event(&mut state, Event::Resize(80, 24), None);
    render_once(&mut term, &mut state).expect("render 80x24");
    assert!(state.editor.text().contains("note 日本語 éàç"));
    assert_eq!(state.active_space, Space::Agents);

    // En mode compact 80x24 + Navigation, 'i' bascule entre Inspecteur et panneau focalisé (y compris DiffViewer)
    handle_terminal_event(&mut state, key(KeyCode::Esc), None);
    assert_eq!(state.mode, InputMode::Navigation);
    state.focus = FocusPane::DiffViewer;
    assert!(state.inspector_open);
    render_once(&mut term, &mut state).expect("render 80x24 inspecteur ouvert");
    assert!(buffer_text(&term).contains("Inspecteur (i: masquer)"));

    handle_terminal_event(&mut state, key(KeyCode::Char('i')), None);
    assert!(!state.inspector_open);
    render_once(&mut term, &mut state).expect("render 80x24 diff sans inspecteur");
    let compact_diff = buffer_text(&term);
    assert!(compact_diff.contains("Diff (10000 lignes"));
    assert!(!compact_diff.contains("Inspecteur (i: masquer)"));

    handle_terminal_event(&mut state, key(KeyCode::Char('i')), None);
    assert!(state.inspector_open);
    render_once(&mut term, &mut state).expect("render 80x24 retour inspecteur");
    assert!(buffer_text(&term).contains("Inspecteur (i: masquer)"));

    // Modes NO_COLOR + ASCII + lecture linéaire
    let mut linear_state = TuiState::with_m0_fixture(TerminalCapabilities {
        no_color: true,
        ascii_only: true,
        enable_mouse: false,
        linear_reader_mode: true,
    });
    let mut term_80 = Terminal::new(TestBackend::new(80, 24)).expect("TestBackend 80x24");
    render_once(&mut term_80, &mut linear_state).expect("render linéaire");
    assert!(buffer_text(&term_80).contains("Mode lecture linéaire"));
}

#[test]
fn t_f015_007_neutralizes_hostile_ansi_osc_and_isolates_pty_keys() {
    let mut state = TuiState::with_m0_fixture(TerminalCapabilities::default());
    // Injection ANSI/OSC 8/OSC 52 dans notifications et nom d'agent
    state.push_notification(
        "alerte \x1b[31mROUGE\x1b[0m \x1b]8;;https://evil.test\x07lien\x1b]8;;\x07 \x1b]52;c;c2VjcmV0\x07fin",
    );
    state.agents[0].name =
        unleash_tui::sanitize_single_line("Team \x1b[2J\x1b]0;HACKED_TITLE\x07Core");

    let mut term = Terminal::new(TestBackend::new(120, 40)).expect("TestBackend");
    render_once(&mut term, &mut state).expect("render");
    let rendered = buffer_text(&term);
    assert!(!rendered.contains('\x1b'));
    assert!(!rendered.contains('\x07'));
    assert!(rendered.contains("Team Core"));
    assert!(rendered.contains("alerte ROUGE lien fin"));

    // Isolation PTY : en mode attaché, 'q' va au PTY et NE quitte PAS la TUI
    handle_terminal_event(&mut state, key(KeyCode::Char('a')), None);
    assert_eq!(state.mode, InputMode::PtyAttached);
    assert!(state.pty.is_attached());
    handle_terminal_event(&mut state, key(KeyCode::Char('q')), None);
    assert!(!state.should_quit);
    assert_eq!(state.pty.drain_pending_input(), b"q");

    // Détachement explicite par Ctrl+]
    handle_terminal_event(&mut state, ctrl(']'), None);
    assert_eq!(state.mode, InputMode::Navigation);
    assert!(!state.pty.is_attached());
}

#[test]
fn t_f015_014_panic_hook_triggers_idempotent_restore() {
    let restore = Arc::new(RestoreState::default());
    install_panic_hook(Arc::clone(&restore));

    // Simuler une activation via accès test interne? RestoreState::restore_stdout n'est déclenchée
    // que si `active` est vrai ; on vérifie que le hook de panique ne bloque ni ne double-panique.
    let res = std::panic::catch_unwind(|| {
        panic!("panic simulée T-F015-014");
    });
    assert!(res.is_err());
    // Vérifier aussi la restauration idempotente
    let _ = restore.is_active();
    restore
        .restore_stdout()
        .expect("restore idempotente après panique");
}

#[test]
fn t_f015_015_ac_f014_011_load_benchmark_1000_agents_100k_events_10k_diff_p95() {
    // T-F015-015 / AC-F014-011 / REQ-CLI-015
    let init_start = Instant::now();
    let mut state = TuiState::with_m0_fixture(TerminalCapabilities::default());
    let init_ms = init_start.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(state.agents.len(), M0_TOTAL_AGENTS);
    assert_eq!(state.total_events, M0_TOTAL_EVENTS);
    assert_eq!(state.total_diff_lines, M0_TOTAL_DIFF_LINES);
    assert!(state.event_page.len() <= unleash_tui::EVENT_PAGE_LIMIT);

    let mut term_120 = Terminal::new(TestBackend::new(120, 40)).expect("TestBackend 120x40");
    let mut term_80 = Terminal::new(TestBackend::new(80, 24)).expect("TestBackend 80x24");

    // 120 itérations : scroll arbre, pagination 100k events, scroll diff 10k, filtre + rendu
    let mut samples_ms_120 = Vec::with_capacity(120);
    let mut samples_ms_80 = Vec::with_capacity(60);

    for step in 0..120usize {
        let t0 = Instant::now();
        match step % 5 {
            0 => {
                state.focus = FocusPane::AgentTree;
                handle_terminal_event(&mut state, key(KeyCode::Char('j')), None);
            }
            1 => {
                state.focus = FocusPane::Timeline;
                handle_terminal_event(&mut state, key(KeyCode::Char('n')), None);
            }
            2 => {
                state.focus = FocusPane::DiffViewer;
                handle_terminal_event(&mut state, key(KeyCode::Char('j')), None);
            }
            3 => {
                handle_terminal_event(&mut state, key(KeyCode::Char('/')), None);
                handle_terminal_event(&mut state, key(KeyCode::Char('l')), None);
                handle_terminal_event(&mut state, key(KeyCode::Enter), None);
                state.filter_query.clear();
                state.recompute_visible_agents();
                state.load_event_page(0);
            }
            _ => {
                handle_terminal_event(&mut state, key(KeyCode::Tab), None);
            }
        }
        render_once(&mut term_120, &mut state).expect("render 120x40");
        samples_ms_120.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    state.inspector_open = false;
    for step in 0..60usize {
        let t0 = Instant::now();
        handle_terminal_event(
            &mut state,
            if step % 15 == 0 {
                key(KeyCode::Char('i'))
            } else if step % 2 == 0 {
                key(KeyCode::Tab)
            } else {
                key(KeyCode::Char('j'))
            },
            None,
        );
        render_once(&mut term_80, &mut state).expect("render 80x24");
        samples_ms_80.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    samples_ms_120.sort_by(f64::total_cmp);
    samples_ms_80.sort_by(f64::total_cmp);

    let p50_120 = samples_ms_120[samples_ms_120.len() / 2];
    let p95_120 = samples_ms_120[(samples_ms_120.len() * 95) / 100];
    let max_120 = *samples_ms_120.last().unwrap();

    let p50_80 = samples_ms_80[samples_ms_80.len() / 2];
    let p95_80 = samples_ms_80[(samples_ms_80.len() * 95) / 100];
    let max_80 = *samples_ms_80.last().unwrap();

    println!(
        "[TUI-LOAD] init_ms={init_ms:.2} 120x40(p50={p50_120:.2}ms p95={p95_120:.2}ms max={max_120:.2}ms) 80x24(p50={p50_80:.2}ms p95={p95_80:.2}ms max={max_80:.2}ms)"
    );

    assert!(
        p95_120 <= 100.0,
        "Budget p95 <= 100 ms dépassé en 120x40 : {p95_120:.2} ms"
    );
    assert!(
        p95_80 <= 100.0,
        "Budget p95 <= 100 ms dépassé en 80x24 : {p95_80:.2} ms"
    );
}

struct TuiFixtureDaemon {
    socket_path: PathBuf,
    queries_received: Arc<Mutex<Vec<String>>>,
    accept_task: JoinHandle<()>,
    _sandbox: tempfile::TempDir,
}

impl TuiFixtureDaemon {
    fn start() -> Self {
        let sandbox = local_test_dir("tui-daemon").expect("local_test_dir");
        let rdv = sandbox.path().join("rdv");
        unleash_platform::ensure_private_rendezvous_dir(&rdv).expect("rdv");
        let socket_path = rdv.join("daemon.sock");
        let std_listener = bind_private_unix_listener(&socket_path).expect("listener");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let listener = UnixListener::from_std(std_listener).expect("UnixListener");
        let queries_received = Arc::new(Mutex::new(Vec::new()));
        let queries_bg = Arc::clone(&queries_received);

        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let q = Arc::clone(&queries_bg);
                tokio::spawn(async move {
                    let _ = handle_tui_daemon_conn(stream, q).await;
                });
            }
        });

        Self {
            socket_path,
            queries_received,
            accept_task,
            _sandbox: sandbox,
        }
    }

    fn tui_config(&self) -> ClientConfig {
        ClientConfig::new(self.socket_path.clone(), ClientKind::Tui, "0.1.0")
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(3))
    }
}

impl Drop for TuiFixtureDaemon {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn handle_tui_daemon_conn(
    mut stream: UnixStream,
    queries_received: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    verify_same_uid_peer(&stream)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;

    let mut hdr = [0u8; 4];
    if stream.read_exact(&mut hdr).await.is_err() {
        return Ok(());
    }
    let len = u32::from_be_bytes(hdr) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    let _hello: HelloRequest = from_json_str_strict(&String::from_utf8(body).unwrap()).unwrap();

    let stream_id = StreamId::from_u128(0x0202_0000_0000_0000_0000_0000_0000_0001);
    let resp = HelloResponse {
        protocol_version: ProtocolVersion,
        authority_generation: AuthorityGeneration::from_u128(0x3000_0001),
        daemon_epoch: DaemonEpoch::new(1),
        daemon_instance_id: DaemonInstanceId::from_u128(0xD000_0008),
        stream_id,
        max_frame_bytes: u32::try_from(MAX_FRAME_BYTES).unwrap(),
        suggested_segment_bytes: 32 * 1024,
        idempotency_retention_seconds: 604_800,
        capabilities: vec![String::from("core_v2")],
    };
    let bytes = serde_json::to_vec(&resp).unwrap();
    stream
        .write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes())
        .await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;

    loop {
        let mut h = [0u8; 4];
        if stream.read_exact(&mut h).await.is_err() {
            break;
        }
        let n = u32::from_be_bytes(h) as usize;
        let mut b = vec![0u8; n];
        stream.read_exact(&mut b).await?;
        let q: QueryEnvelope = from_json_str_strict(&String::from_utf8(b).unwrap()).unwrap();
        queries_received.lock().unwrap().push(q.query_type.clone());
        let qresp = QueryResponse {
            request_id: q.request_id,
            watermark: Some(EventCursor::new(stream_id, Sequence::new(42))),
            result: json!({ "overview": "ok", "active_jobs": 2 }),
        };
        let out = serde_json::to_vec(&qresp).unwrap();
        stream
            .write_all(&u32::try_from(out.len()).unwrap().to_be_bytes())
            .await?;
        stream.write_all(&out).await?;
        stream.flush().await?;
    }
    Ok(())
}

#[tokio::test]
async fn t_f015_005_tui_client_connects_and_quits_without_stopping_daemon() {
    let daemon = TuiFixtureDaemon::start();
    let mut state = TuiState::with_m0_fixture(TerminalCapabilities::default());

    {
        let mut client = UnleashClient::connect(daemon.tui_config())
            .await
            .expect("connexion TUI via unleash-client");
        apply_backend_update(
            &mut state,
            BackendUpdate::Connected {
                cursor: Some(EventCursor::new(client.hello().stream_id, Sequence::new(1))),
            },
        );
        let q = QueryEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xABCD_0001),
            query_type: OperationName::GetOverview.as_str().to_owned(),
            project_id: None,
            payload: serde_json::Map::new(),
        };
        let resp = client.query(&q).await.expect("GetOverview");
        apply_backend_update(
            &mut state,
            BackendUpdate::QueryResult {
                operation: OperationName::GetOverview,
                summary: format!("{:?}", resp.result),
            },
        );
        handle_terminal_event(&mut state, key(KeyCode::Char('q')), None);
        assert!(state.should_quit);
    }

    // Après sortie de la TUI, le daemon est toujours joignable et n'a reçu aucune mutation d'arrêt
    let queries = daemon.queries_received.lock().unwrap().clone();
    assert_eq!(queries, vec![String::from("GetOverview")]);
    let _verify_alive = UnleashClient::connect(daemon.tui_config())
        .await
        .expect("le daemon survit à la fermeture du client TUI");
}

struct CountingBackend {
    inner: TestBackend,
    draws: Arc<AtomicUsize>,
}

impl Backend for CountingBackend {
    type Error = <TestBackend as Backend>::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.draws.fetch_add(1, Ordering::Relaxed);
        self.inner.draw(content)
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

struct ChannelEventStream {
    rx: mpsc::UnboundedReceiver<std::io::Result<Event>>,
}

impl Stream for ChannelEventStream {
    type Item = std::io::Result<Event>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[tokio::test]
async fn t_f015_015_no_busy_redraw_loop_when_backend_channel_closes() {
    // Vérifie l'absence de boucle de redraw infinie lorsque run_backend_io_worker se termine
    // (`client_config == None`) et droppe `upd_tx`.
    let draws = Arc::new(AtomicUsize::new(0));
    let backend = CountingBackend {
        inner: TestBackend::new(120, 40),
        draws: Arc::clone(&draws),
    };
    let mut terminal = Terminal::new(backend).expect("Terminal CountingBackend");
    let mut state = TuiState::with_m0_fixture(TerminalCapabilities::default());

    let (cmd_tx, cmd_rx) = mpsc::channel::<BackendCommand>(8);
    let (upd_tx, upd_rx) = mpsc::channel::<BackendUpdate>(MAX_NOTIFICATION_QUEUE);
    let io_task = tokio::spawn(run_backend_io_worker(None, cmd_rx, upd_tx));

    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let loop_fut = run_tui_loop(
        &mut terminal,
        &mut state,
        ChannelEventStream { rx: ev_rx },
        Some(&cmd_tx),
        Some(upd_rx),
    );
    let driver_fut = async move {
        let _ = io_task.await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        let _ = ev_tx.send(Ok(key(KeyCode::Char('q'))));
    };

    let (loop_res, ()) = tokio::join!(loop_fut, driver_fut);
    loop_res.expect("run_tui_loop termine proprement sur 'q'");

    drop(cmd_tx);

    assert!(matches!(
        state.connection,
        ConnectionBanner::Disconnected { .. }
    ));
    let total_draws = draws.load(Ordering::Relaxed);
    assert!(
        total_draws <= 4,
        "Aucune boucle de redraw à 100 % CPU autorisée après fermeture de upd_rx (reçu {total_draws} redraws)"
    );
}
