// SPDX-License-Identifier: Apache-2.0
//! Rendu Ratatui adaptatif de la TUI OVPT Unleash (`120x40` référence, `80x24` compact,
//! `NO_COLOR`, `--ascii`, mode linéaire lecteur d'écran).

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};

use crate::sanitize::sanitize_single_line;
use crate::state::{ConnectionBanner, FocusPane, InputMode, Space, TuiState};

/// Dessine une frame complète de l'application TUI sans aucune allocation non bornée.
pub fn draw(frame: &mut Frame<'_>, state: &mut TuiState) {
    let area = frame.area();
    if area.width < 20 || area.height < 10 {
        let msg = Paragraph::new("Terminal trop étroit (minimum 20x10)");
        frame.render_widget(msg, area);
        return;
    }

    if state.capabilities.linear_reader_mode {
        draw_linear_accessible(frame, area, state);
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    let Some(header_area) = vertical.first().copied() else {
        return;
    };
    let Some(tabs_area) = vertical.get(1).copied() else {
        return;
    };
    let Some(body_area) = vertical.get(2).copied() else {
        return;
    };
    let Some(footer_area) = vertical.get(3).copied() else {
        return;
    };

    draw_context_bar(frame, header_area, state);
    draw_space_tabs(frame, tabs_area, state);

    let compact = area.width < 100 || area.height < 30;
    if compact {
        draw_compact_body(frame, body_area, state);
    } else {
        draw_standard_body(frame, body_area, state);
    }

    draw_footer(frame, footer_area, state);

    if state.mode == InputMode::Palette {
        draw_palette_overlay(frame, area, state);
    } else if state.mode == InputMode::Help {
        draw_help_overlay(frame, area, state);
    }
}

fn border_set(ascii_only: bool) -> ratatui::symbols::border::Set<'static> {
    if ascii_only {
        ratatui::symbols::border::PLAIN
    } else {
        ratatui::symbols::border::ROUNDED
    }
}

fn focus_style(focused: bool, no_color: bool) -> Style {
    if no_color {
        if focused {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
        }
    } else if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn draw_context_bar(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let conn_label = match &state.connection {
        ConnectionBanner::Connected { cursor } => cursor.map_or_else(
            || String::from("IPC v2: connecté"),
            |c| format!("IPC v2: seq {}", c.sequence.as_u64()),
        ),
        ConnectionBanner::Disconnected { reason } => {
            format!("DECONNECTE: {}", sanitize_single_line(reason))
        }
        ConnectionBanner::ResyncRequired { reason } => {
            format!("RESYNC: {}", sanitize_single_line(reason))
        }
    };
    let header_line = Line::from(vec![
        Span::styled(
            " OVPT Unleash ",
            if state.capabilities.no_color {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            },
        ),
        Span::raw(format!(
            " · Mission M0 [mode:supervised] · Pool 3/5 (max 50) · {conn_label} · Rev {}",
            state.draft_base_revision
        )),
    ]);
    let p = Paragraph::new(vec![
        header_line,
        Line::from(
            " Alerte : blocked_target_checkout sur dev · Doc périmé : ADR-09 · Coût est. 1.40 USD (+ inconnu)",
        ),
    ]);
    frame.render_widget(p, area);
}

fn draw_space_tabs(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let titles: Vec<Line<'_>> = Space::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| Line::from(format!("{}:{}", (i + 1) % 10, s.label())))
        .collect();
    let selected = Space::ALL
        .iter()
        .position(|s| *s == state.active_space)
        .unwrap_or(0);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .title(" Espaces (10/10) ");
    let tabs = Tabs::new(titles)
        .block(block)
        .select(selected)
        .highlight_style(if state.capabilities.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        });
    frame.render_widget(tabs, area);
}

fn draw_standard_body(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if state.inspector_open {
            vec![
                Constraint::Percentage(34),
                Constraint::Percentage(38),
                Constraint::Percentage(28),
            ]
        } else {
            vec![Constraint::Percentage(48), Constraint::Percentage(52)]
        })
        .split(area);

    if let Some(left) = cols.first().copied() {
        let left_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(left);
        if let Some(top_l) = left_split.first().copied() {
            draw_agent_tree(frame, top_l, state);
        }
        if let Some(bot_l) = left_split.get(1).copied() {
            draw_timeline(frame, bot_l, state);
        }
    }

    if let Some(center) = cols.get(1).copied() {
        let center_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ])
            .split(center);
        if let Some(top_c) = center_split.first().copied() {
            draw_diff_viewer(frame, top_c, state);
        }
        if let Some(mid_c) = center_split.get(1).copied() {
            draw_editor(frame, mid_c, state);
        }
        if let Some(bot_c) = center_split.get(2).copied() {
            state.pty.render_into(
                bot_c,
                frame.buffer_mut(),
                state.capabilities.ascii_only,
                state.capabilities.no_color,
            );
        }
    }

    if state.inspector_open
        && let Some(right) = cols.get(2).copied()
    {
        draw_inspector(frame, right, state);
    }
}

fn draw_compact_body(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    if state.inspector_open && state.mode == InputMode::Navigation {
        draw_inspector(frame, area, state);
        return;
    }
    match state.focus {
        FocusPane::AgentTree => draw_agent_tree(frame, area, state),
        FocusPane::Timeline => draw_timeline(frame, area, state),
        FocusPane::DiffViewer => draw_diff_viewer(frame, area, state),
        FocusPane::Editor => draw_editor(frame, area, state),
        FocusPane::Pty => state.pty.render_into(
            area,
            frame.buffer_mut(),
            state.capabilities.ascii_only,
            state.capabilities.no_color,
        ),
    }
}

fn draw_linear_accessible(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let agent_line = state.selected_agent().map_or_else(
        || String::from("Aucun agent sélectionné"),
        |a| format!("{} [{} / {}]", a.name, a.logical_state, a.invocation_state),
    );
    let event_line = state.selected_event().map_or_else(
        || String::from("Aucun événement sélectionné"),
        |e| e.summary.clone(),
    );
    let text = format!(
        "OVPT Unleash (Mode lecture linéaire)\nEspace: {}\nAgents: {} visibles sur {}\nSélection agent: {}\nTimeline: page {} ({} total)\nSélection événement: {}\nDiff: ligne {}/{}\nStatut: {}",
        state.active_space.label(),
        state.visible_agent_indices.len(),
        state.agents.len(),
        agent_line,
        state.event_page_offset + 1,
        state.total_events,
        event_line,
        state.diff_scroll_line + 1,
        state.total_diff_lines,
        state.status_line
    );
    let p = Paragraph::new(text).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn draw_agent_tree(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let focused = state.focus == FocusPane::AgentTree;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .border_style(focus_style(focused, state.capabilities.no_color))
        .title(format!(
            " Agents ({}/{} visibles, 3/5 slots) ",
            state.visible_agent_indices.len(),
            state.agents.len()
        ));
    let inner = block.inner(area);
    let height = usize::from(inner.height).max(1);

    if state.selected_agent_pos < state.agent_scroll {
        state.agent_scroll = state.selected_agent_pos;
    } else if state.selected_agent_pos >= state.agent_scroll + height {
        state.agent_scroll = state.selected_agent_pos + 1 - height;
    }

    let items: Vec<ListItem<'_>> = state
        .visible_agent_indices
        .iter()
        .enumerate()
        .skip(state.agent_scroll)
        .take(height)
        .filter_map(|(vis_idx, &agent_idx)| {
            let agent = state.agents.get(agent_idx)?;
            let indent = "  ".repeat(usize::from(agent.depth));
            let marker = if agent.has_children {
                if state.collapsed_agent_ids.contains(&agent.id) {
                    "[+]"
                } else {
                    "[-]"
                }
            } else {
                " · "
            };
            let selected_prefix = if vis_idx == state.selected_agent_pos {
                ">"
            } else {
                " "
            };
            let line = format!(
                "{selected_prefix}{indent}{marker} {} {} [{}|inv:{}]",
                agent.level, agent.name, agent.logical_state, agent.invocation_state
            );
            let style = if vis_idx == state.selected_agent_pos {
                if state.capabilities.no_color {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                }
            } else {
                Style::default()
            };
            Some(ListItem::new(line).style(style))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_timeline(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let focused = state.focus == FocusPane::Timeline;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .border_style(focus_style(focused, state.capabilities.no_color))
        .title(format!(
            " Timeline ({} total · page {}..{}) ",
            state.total_events,
            state.event_page_offset + 1,
            state.event_page_offset + state.event_page.len()
        ));
    let inner = block.inner(area);
    let height = usize::from(inner.height).max(1);
    let start = state.selected_event_pos.saturating_sub(height / 2);

    let items: Vec<ListItem<'_>> = state
        .event_page
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(idx, ev)| {
            let prefix = if idx == state.selected_event_pos {
                ">"
            } else {
                " "
            };
            let line = format!("{prefix} {}", ev.summary);
            let style = if idx == state.selected_event_pos {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_diff_viewer(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let focused = state.focus == FocusPane::DiffViewer;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .border_style(focus_style(focused, state.capabilities.no_color))
        .title(format!(
            " Diff reducer.rs ({} lignes · ligne {}) ",
            state.total_diff_lines,
            state.diff_scroll_line + 1
        ));
    let lines: Vec<Line<'_>> = state
        .diff_window_cache
        .iter()
        .map(|l| Line::from(sanitize_single_line(l)))
        .collect();
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}

fn draw_editor(frame: &mut Frame<'_>, area: Rect, state: &mut TuiState) {
    let focused = state.focus == FocusPane::Editor || state.mode == InputMode::Editing;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .border_style(focus_style(focused, state.capabilities.no_color))
        .title(format!(
            " Editeur integre ({} lignes) [e: editer, Esc: naviguer] ",
            state.editor.line_count()
        ));
    state
        .editor
        .render_into(area, frame.buffer_mut(), block, state.capabilities.no_color);
}

fn draw_inspector(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .title(" Inspecteur (i: masquer) ");

    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Espace : {}",
        state.active_space.label()
    )));
    if let Some(agent) = state.selected_agent() {
        lines.push(Line::from("--- Agent ---"));
        lines.push(Line::from(format!("ID : {}", agent.id)));
        lines.push(Line::from(format!("Nom : {}", agent.name)));
        lines.push(Line::from(format!(
            "Etats : {} / inv:{}",
            agent.logical_state, agent.invocation_state
        )));
        lines.push(Line::from(format!("Modèle : {}", agent.model)));
    }
    if let Some(ev) = state.selected_event() {
        lines.push(Line::from("--- Evénement ---"));
        lines.push(Line::from(format!("Seq : {}", ev.sequence)));
        lines.push(Line::from(format!("Type : {}", ev.event_type)));
        lines.push(Line::from(format!("Cause : {}", ev.causation)));
        lines.push(Line::from(format!("Sensibilité : {}", ev.sensitivity)));
    }
    if !state.notification_queue.is_empty() {
        lines.push(Line::from("--- Notifications bornées ---"));
        for msg in state.notification_queue.iter().rev().take(4) {
            lines.push(Line::from(msg.clone()));
        }
    }

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(p, area);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let mode_label = match state.mode {
        InputMode::Navigation => "NAV",
        InputMode::Search => "FILTRE",
        InputMode::Palette => "PALETTE",
        InputMode::Editing => "EDITION",
        InputMode::PtyAttached => "PTY-ATTACHE",
        InputMode::Help => "AIDE",
    };
    let line1 = format!(
        " Mode:{mode_label} | Filtre:\"{}\" | Notifications:{}/{} (drop:{}) ",
        sanitize_single_line(&state.filter_query),
        state.notification_queue.len(),
        crate::state::MAX_NOTIFICATION_QUEUE,
        state.dropped_notifications
    );
    let line2 = format!(" {} ", sanitize_single_line(&state.status_line));
    let p = Paragraph::new(vec![Line::from(line1), Line::from(line2)]);
    frame.render_widget(p, area);
}

fn draw_palette_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let width = area.width.min(68);
    let height = area.height.min(16);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);

    let filtered = state.filtered_palette_commands();
    let mut items = vec![ListItem::new(format!(
        "Recherche : {}_",
        sanitize_single_line(&state.palette_query)
    ))];
    for (idx, cmd) in filtered.iter().enumerate() {
        let marker = if idx == state.palette_selected {
            ">"
        } else {
            " "
        };
        items.push(ListItem::new(format!(
            "{marker} [{}] {} ({})",
            cmd.shortcut,
            cmd.label,
            cmd.operation.as_str()
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .title(" Palette de commandes (Entree: executer · Esc: fermer) ");
    let list = List::new(items).block(block);
    frame.render_widget(list, popup);
}

fn draw_help_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let width = area.width.min(70);
    let height = area.height.min(14);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set(state.capabilities.ascii_only))
        .title(" Aide clavier OVPT Unleash (Esc pour fermer) ");
    let help = Paragraph::new(
        "1..0 : changer d'espace (10 espaces)\n\
         Tab / Shift+Tab : panneau suivant / precedent\n\
         j / k / Haut / Bas : defiler / selectionner\n\
         n / p : page suivante / precedente timeline (100 000 events)\n\
         / : filtrer agents et timeline · Ctrl+P : palette de commandes\n\
         e : editeur multiligne · a : attacher PTY (Ctrl+] pour detacher)\n\
         i : ouvrir/fermer inspecteur · q : quitter sans arreter le daemon",
    )
    .block(block)
    .wrap(Wrap { trim: false });
    frame.render_widget(help, popup);
}
