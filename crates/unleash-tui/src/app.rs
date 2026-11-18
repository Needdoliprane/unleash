// SPDX-License-Identifier: Apache-2.0
//! Boucle événementielle unique (`crossterm::event::EventStream` sous Tokio) et pont I/O
//! asynchrone vers `unleash_client::UnleashClient` (`ADR-15` règle 4).

use std::future::poll_fn;
use std::pin::Pin;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_core::Stream;
use ratatui::backend::Backend;
use tokio::sync::mpsc;
use unleash_client::{ClientConfig, UnleashClient};
use unleash_domain::RequestId;
use unleash_protocol::{EventCursor, OperationName, ProtocolVersion, QueryEnvelope};
use uuid::Uuid;

use crate::state::{
    ConnectionBanner, EVENT_PAGE_LIMIT, FocusPane, InputMode, MAX_NOTIFICATION_QUEUE, Space,
    TuiState,
};
use crate::terminal::{TerminalCapabilities, TerminalGuard, TerminalLifecycleError};
use crate::ui::draw;

/// Événement envoyé par la tâche I/O client vers la tâche unique de rendu TUI.
#[derive(Debug, Clone)]
pub enum BackendUpdate {
    /// Connexion `Hello` réussie.
    Connected {
        /// Curseur courant initial si disponible.
        cursor: Option<EventCursor>,
    },
    /// Réponse de requête reçue du daemon.
    QueryResult {
        /// Opération concernée.
        operation: OperationName,
        /// Résumé textuel borné.
        summary: String,
    },
    /// Coupure ou erreur de transport sans arrêter la TUI ni ses brouillons locaux.
    Disconnected {
        /// Raison bornée.
        reason: String,
    },
}

/// Commande envoyée par le contrôleur TUI vers la tâche I/O `unleash-client`.
#[derive(Debug, Clone)]
pub enum BackendCommand {
    /// Demande de lecture typée du catalogue.
    ExecuteQuery {
        /// Opération de lecture.
        operation: OperationName,
    },
}

/// Traite un événement clavier/terminal/redimensionnement sur `TuiState` de manière déterministe
/// (utilisé à la fois par la boucle interactive `run_tui` et par les tests `TestBackend`).
pub fn handle_terminal_event(
    state: &mut TuiState,
    event: Event,
    backend_tx: Option<&mpsc::Sender<BackendCommand>>,
) {
    match event {
        Event::Key(key) => handle_key_event(state, key, backend_tx),
        Event::Paste(pasted) => {
            // Collage contrôlé : jamais interprété comme soumission ni commande globale.
            if state.mode == InputMode::Editing {
                state.editor.insert_str(&pasted);
            } else if state.mode == InputMode::Search {
                state
                    .filter_query
                    .push_str(&pasted.replace(['\r', '\n'], " "));
                state.recompute_visible_agents();
                state.load_event_page(0);
            } else if state.mode == InputMode::PtyAttached {
                let _ = state.pty.queue_input(pasted.as_bytes());
            }
        }
        Event::Resize(cols, rows) => {
            state.pty.resize(rows / 3, cols / 2);
            state.status_line =
                format!("Redimensionné {cols}x{rows} (sélection et brouillon conservés)");
        }
        Event::FocusGained | Event::FocusLost | Event::Mouse(_) => {}
    }
}

fn handle_key_event(
    state: &mut TuiState,
    key: KeyEvent,
    backend_tx: Option<&mpsc::Sender<BackendCommand>>,
) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        state.should_quit = true;
        return;
    }

    match state.mode {
        InputMode::PtyAttached => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(']') {
                state.pty.detach();
                state.mode = InputMode::Navigation;
                state.status_line = String::from("PTY détaché (processus backend toujours actif)");
                return;
            }
            if let KeyCode::Char(c) = key.code {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                let _ = state.pty.queue_input(s.as_bytes());
            } else if key.code == KeyCode::Enter {
                let _ = state.pty.queue_input(b"\n");
            }
        }
        InputMode::Editing => handle_editing_key(state, key),
        InputMode::Search => match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                state.mode = InputMode::Navigation;
            }
            KeyCode::Backspace => {
                state.filter_query.pop();
                state.recompute_visible_agents();
                state.load_event_page(0);
            }
            KeyCode::Char(c) => {
                state.filter_query.push(c);
                state.recompute_visible_agents();
                state.load_event_page(0);
            }
            _ => {}
        },
        InputMode::Palette => handle_palette_key(state, key, backend_tx),
        InputMode::Help => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?' | 'q')) {
                state.mode = InputMode::Navigation;
            }
        }
        InputMode::Navigation => handle_navigation_key(state, key, backend_tx),
    }
}

fn handle_editing_key(state: &mut TuiState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            state.mode = InputMode::Navigation;
            state.status_line = String::from("Retour en navigation (brouillon conservé)");
        }
        KeyCode::Enter => state.editor.insert_char('\n'),
        KeyCode::Backspace => state.editor.backspace(),
        KeyCode::Up => state.editor.move_up(),
        KeyCode::Down => state.editor.move_down(),
        KeyCode::Left => state.editor.move_left(),
        KeyCode::Right => state.editor.move_right(),
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = state.editor.undo();
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = state.editor.redo();
        }
        KeyCode::Char(c) => state.editor.insert_char(c),
        _ => {}
    }
}

fn handle_palette_key(
    state: &mut TuiState,
    key: KeyEvent,
    backend_tx: Option<&mpsc::Sender<BackendCommand>>,
) {
    let filtered_len = state.filtered_palette_commands().len();
    match key.code {
        KeyCode::Esc => state.mode = InputMode::Navigation,
        KeyCode::Backspace => {
            state.palette_query.pop();
            state.palette_selected = 0;
        }
        KeyCode::Up => {
            state.palette_selected = state.palette_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if state.palette_selected + 1 < filtered_len {
                state.palette_selected += 1;
            }
        }
        KeyCode::Enter => {
            let chosen = state
                .filtered_palette_commands()
                .get(state.palette_selected)
                .map(|c| (c.space, c.operation, c.label));
            if let Some((space, op, label)) = chosen {
                state.active_space = space;
                state.mode = InputMode::Navigation;
                state.status_line = format!("Palette : {label} ({})", op.as_str());
                if let Some(tx) = backend_tx {
                    let _ = tx.try_send(BackendCommand::ExecuteQuery { operation: op });
                }
            }
        }
        KeyCode::Char(c) => {
            state.palette_query.push(c);
            state.palette_selected = 0;
        }
        _ => {}
    }
}

fn handle_navigation_key(
    state: &mut TuiState,
    key: KeyEvent,
    backend_tx: Option<&mpsc::Sender<BackendCommand>>,
) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        state.mode = InputMode::Palette;
        state.palette_query.clear();
        state.palette_selected = 0;
        return;
    }

    match key.code {
        KeyCode::Char('q') => state.should_quit = true,
        KeyCode::Tab => state.focus = state.focus.next(),
        KeyCode::BackTab => state.focus = state.focus.prev(),
        KeyCode::Char('/') => state.mode = InputMode::Search,
        KeyCode::Char('?') => state.mode = InputMode::Help,
        KeyCode::Char('i') => state.inspector_open = !state.inspector_open,
        KeyCode::Char('e') => {
            state.focus = FocusPane::Editor;
            state.mode = InputMode::Editing;
        }
        KeyCode::Char('a') => {
            state.focus = FocusPane::Pty;
            if state.fixture {
                state.pty.attach("lease-001", 7);
                state.mode = InputMode::PtyAttached;
            } else {
                state.status_line = String::from("Aucune session PTY disponible");
            }
        }
        KeyCode::Char(']') => {
            state.active_space = state.active_space.next();
            if let Some(tx) = backend_tx {
                let _ = tx.try_send(BackendCommand::ExecuteQuery {
                    operation: OperationName::GetOverview,
                });
            }
        }
        KeyCode::Char('[') => state.active_space = state.active_space.prev(),
        KeyCode::Char(d @ '0'..='9') => {
            let idx = match d {
                '1' => 0,
                '2' => 1,
                '3' => 2,
                '4' => 3,
                '5' => 4,
                '6' => 5,
                '7' => 6,
                '8' => 7,
                '9' => 8,
                _ => 9,
            };
            if let Some(space) = Space::ALL.get(idx).copied() {
                state.active_space = space;
                state.status_line = format!("Espace actif : {}", space.label());
            }
        }
        KeyCode::Down | KeyCode::Char('j') => move_selection_down(state),
        KeyCode::Up | KeyCode::Char('k') => move_selection_up(state),
        KeyCode::Enter | KeyCode::Left | KeyCode::Right => toggle_selected_agent(state),
        KeyCode::Char('n') => {
            let next_offset = (state.event_page_offset + EVENT_PAGE_LIMIT)
                .min(state.total_events.saturating_sub(EVENT_PAGE_LIMIT));
            state.load_event_page(next_offset);
        }
        KeyCode::Char('p') => {
            let prev_offset = state.event_page_offset.saturating_sub(EVENT_PAGE_LIMIT);
            state.load_event_page(prev_offset);
        }
        _ => {}
    }
}

fn move_selection_down(state: &mut TuiState) {
    match state.focus {
        FocusPane::AgentTree => {
            if state.selected_agent_pos + 1 < state.visible_agent_indices.len() {
                state.selected_agent_pos += 1;
            }
        }
        FocusPane::Timeline => {
            if state.selected_event_pos + 1 < state.event_page.len() {
                state.selected_event_pos += 1;
            }
        }
        FocusPane::DiffViewer => {
            let next = (state.diff_scroll_line + 20).min(state.total_diff_lines.saturating_sub(1));
            state.load_diff_window(next, 40);
        }
        FocusPane::Editor => state.editor.move_down(),
        FocusPane::Pty => {}
    }
}

fn move_selection_up(state: &mut TuiState) {
    match state.focus {
        FocusPane::AgentTree => {
            state.selected_agent_pos = state.selected_agent_pos.saturating_sub(1);
        }
        FocusPane::Timeline => {
            state.selected_event_pos = state.selected_event_pos.saturating_sub(1);
        }
        FocusPane::DiffViewer => {
            let prev = state.diff_scroll_line.saturating_sub(20);
            state.load_diff_window(prev, 40);
        }
        FocusPane::Editor => state.editor.move_up(),
        FocusPane::Pty => {}
    }
}

fn toggle_selected_agent(state: &mut TuiState) {
    if state.focus != FocusPane::AgentTree {
        return;
    }
    let Some(agent_id) = state
        .selected_agent()
        .filter(|a| a.has_children)
        .map(|a| a.id.clone())
    else {
        return;
    };
    if !state.collapsed_agent_ids.remove(&agent_id) {
        state.collapsed_agent_ids.insert(agent_id);
    }
    state.recompute_visible_agents();
}

/// Applique une mise à jour asynchrone reçue depuis `unleash-client`.
pub fn apply_backend_update(state: &mut TuiState, update: BackendUpdate) {
    match update {
        BackendUpdate::Connected { cursor } => {
            state.connection = ConnectionBanner::Connected { cursor };
            state.push_notification("Connexion IPC v2 établie avec le daemon");
        }
        BackendUpdate::QueryResult { operation, summary } => {
            state.push_notification(&format!("{}: {summary}", operation.as_str()));
        }
        BackendUpdate::Disconnected { reason } => {
            state.connection = ConnectionBanner::Disconnected { reason };
            state.push_notification("Transport daemon interrompu (brouillons préservés)");
        }
    }
}

/// Reçoit la prochaine mise à jour du worker I/O ou bascule en attente passive dès que le
/// canal se ferme, afin d'éviter toute boucle de redraw à 100 % CPU lorsque le daemon est absent.
async fn recv_update(upd_rx: &mut Option<mpsc::Receiver<BackendUpdate>>) -> BackendUpdate {
    if let Some(rx) = upd_rx.as_mut() {
        if let Some(upd) = rx.recv().await {
            return upd;
        }
        *upd_rx = None;
    }
    std::future::pending().await
}

/// Boucle événementielle TUI réutilisable sur tout [`Backend`] Ratatui et flux [`Stream`].
///
/// # Sûreté à l'annulation
/// Ne conserve aucun verrou synchrone à travers `.await` et termine dès la fermeture du flux
/// d'événements terminal ou lorsque `state.should_quit` devient vrai.
///
/// # Errors
/// Propage toute erreur émise par [`ratatui::Terminal::draw`].
pub async fn run_tui_loop<B, S>(
    terminal: &mut ratatui::Terminal<B>,
    state: &mut TuiState,
    mut event_stream: S,
    cmd_tx: Option<&mpsc::Sender<BackendCommand>>,
    mut upd_rx: Option<mpsc::Receiver<BackendUpdate>>,
) -> Result<(), B::Error>
where
    B: Backend,
    S: Stream<Item = std::io::Result<Event>> + Unpin,
{
    loop {
        terminal.draw(|frame| draw(frame, state))?;
        if state.should_quit {
            break;
        }

        let mut pinned_stream = Pin::new(&mut event_stream);
        tokio::select! {
            maybe_event = poll_fn(|cx| pinned_stream.as_mut().poll_next(cx)) => {
                match maybe_event {
                    Some(Ok(ev)) => handle_terminal_event(state, ev, cmd_tx),
                    Some(Err(_)) | None => break,
                }
            }
            upd = recv_update(&mut upd_rx) => {
                apply_backend_update(state, upd);
            }
        }
    }
    Ok(())
}

/// Lance la TUI interactive plein écran avec garde RAII terminal, lecteur d'événements unique et
/// worker asynchrone `unleash-client`.
///
/// # Sûreté à l'annulation
/// À la sortie normale, erreur ou panique, `TerminalGuard` restaure le mode terminal avant retour.
///
/// # Errors
/// Retourne [`TerminalLifecycleError::NotATty`] hors terminal interactif, ou [`TerminalLifecycleError::Io`].
pub async fn run_tui(
    client_config: Option<ClientConfig>,
    capabilities: TerminalCapabilities,
) -> Result<(), TerminalLifecycleError> {
    let mut guard = TerminalGuard::enter(&capabilities)?;
    let mut state = TuiState::new(capabilities);

    let (cmd_tx, cmd_rx) = mpsc::channel::<BackendCommand>(64);
    let (upd_tx, upd_rx) = mpsc::channel::<BackendUpdate>(MAX_NOTIFICATION_QUEUE);

    let io_task = tokio::spawn(run_backend_io_worker(client_config, cmd_rx, upd_tx));
    let event_stream = EventStream::new();

    let loop_res = run_tui_loop(
        guard.terminal_mut(),
        &mut state,
        event_stream,
        Some(&cmd_tx),
        Some(upd_rx),
    )
    .await;

    drop(cmd_tx);
    let _ = io_task.await;
    guard.restore()?;
    loop_res.map_err(TerminalLifecycleError::Io)
}

/// Effectue un rendu unique déterministe sur n'importe quel backend Ratatui (notamment `TestBackend`).
///
/// # Errors
/// Propage toute erreur du backend de rendu.
pub fn render_once<B: Backend>(
    terminal: &mut ratatui::Terminal<B>,
    state: &mut TuiState,
) -> Result<(), B::Error> {
    terminal.draw(|frame| draw(frame, state))?;
    Ok(())
}

/// Worker I/O borné reliant la TUI à `UnleashClient` hors thread de rendu.
pub async fn run_backend_io_worker(
    client_config: Option<ClientConfig>,
    mut cmd_rx: mpsc::Receiver<BackendCommand>,
    upd_tx: mpsc::Sender<BackendUpdate>,
) {
    let Some(cfg) = client_config else {
        tracing::debug!("Worker TUI démarré sans socket daemon configurée");
        let _ = upd_tx
            .send(BackendUpdate::Disconnected {
                reason: String::from("Socket daemon non configuré"),
            })
            .await;
        return;
    };

    let mut client = match UnleashClient::connect(cfg).await {
        Ok(c) => {
            tracing::info!(stream_id = %c.hello().stream_id, "Connexion IPC TUI établie");
            let _ = upd_tx.send(BackendUpdate::Connected { cursor: None }).await;
            c
        }
        Err(err) => {
            tracing::warn!(error = %err, "Échec connexion IPC TUI");
            let _ = upd_tx
                .send(BackendUpdate::Disconnected {
                    reason: err.to_string(),
                })
                .await;
            return;
        }
    };

    let mut seq = 1u128;
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            BackendCommand::ExecuteQuery { operation } => {
                seq = seq.saturating_add(1);
                let q = QueryEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_uuid(Uuid::from_u128(0x7000_0000 + seq)),
                    query_type: operation.as_str().to_owned(),
                    project_id: None,
                    payload: serde_json::Map::new(),
                };
                match client.query(&q).await {
                    Ok(resp) => {
                        tracing::debug!(operation = operation.as_str(), "Requête TUI exécutée");
                        let _ = upd_tx
                            .send(BackendUpdate::QueryResult {
                                operation,
                                summary: format!("watermark={:?}", resp.watermark),
                            })
                            .await;
                    }
                    Err(err) => {
                        tracing::warn!(
                            operation = operation.as_str(),
                            error = %err,
                            "Erreur requête IPC TUI"
                        );
                        let _ = upd_tx
                            .send(BackendUpdate::Disconnected {
                                reason: err.to_string(),
                            })
                            .await;
                    }
                }
            }
        }
    }
}
