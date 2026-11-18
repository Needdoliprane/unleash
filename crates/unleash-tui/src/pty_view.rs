// SPDX-License-Identifier: Apache-2.0
//! Rendu PTY isolé basé sur `vt100` (`0.16.2`) et `tui-term` (`0.3.4`), conformément à `ADR-15`
//! règle 5.
//!
//! L'attachement au PTY est explicite et gouverné par `(process_run_uuid, lease_id, fence)` ;
//! en mode attaché, aucune touche ordinaire n'est interprétée comme raccourci métier global.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use tui_term::widget::PseudoTerminal;
use vt100::Parser;

/// État d'attachement explicite au terminal contrôlé (`F011`, `REQ-CLI-007`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyAttachState {
    /// Mode inspecteur en lecture seule : les touches restent gérées par la navigation TUI.
    Detached,
    /// Mode attaché sous lease explicite : les frappes vont au processus jusqu'à `Ctrl+]`.
    Attached {
        /// Identifiant de lease accordé par `AcquireTerminal`.
        lease_id: String,
        /// Numéro de fence courant.
        fence: u64,
    },
}

/// Panneau d'émulation PTY borné et isolé des logs ordinaires.
pub struct PtyPanel {
    parser: Parser,
    process_run_uuid: String,
    attach_state: PtyAttachState,
    pending_tx_bytes: Vec<u8>,
}

impl std::fmt::Debug for PtyPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyPanel")
            .field("process_run_uuid", &self.process_run_uuid)
            .field("attach_state", &self.attach_state)
            .field("pending_tx_bytes_len", &self.pending_tx_bytes.len())
            .finish_non_exhaustive()
    }
}

impl PtyPanel {
    /// Crée un panneau PTY vide. Un identifiant vide signale l'absence de processus.
    #[must_use]
    pub fn new(process_run_uuid: impl Into<String>, rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(rows.max(4), cols.max(20), 2000),
            process_run_uuid: process_run_uuid.into(),
            attach_state: PtyAttachState::Detached,
            pending_tx_bytes: Vec::new(),
        }
    }

    /// Retourne l'état courant d'attachement.
    #[must_use]
    pub const fn attach_state(&self) -> &PtyAttachState {
        &self.attach_state
    }

    /// Indique si le mode attaché capture les frappes clavier.
    #[must_use]
    pub const fn is_attached(&self) -> bool {
        matches!(self.attach_state, PtyAttachState::Attached { .. })
    }

    /// Bascule en mode attaché avec un lease et une fence explicites.
    pub fn attach(&mut self, lease_id: impl Into<String>, fence: u64) {
        self.attach_state = PtyAttachState::Attached {
            lease_id: lease_id.into(),
            fence,
        };
    }

    /// Détache le contrôle interactif sans arrêter le processus backend (`AC-F015-005`).
    pub fn detach(&mut self) {
        self.attach_state = PtyAttachState::Detached;
    }

    /// Alimente le parseur `vt100` avec des octets reçus du flux PTY.
    pub fn process_output(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Redimensionne l'écran `vt100` local sans lancer d'effet backend implicite.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(4), cols.max(20));
    }

    /// Enregistre une entrée utilisateur vers la file d'émission PTY (en mode attaché uniquement).
    /// Retourne `false` si le PTY est détaché.
    pub fn queue_input(&mut self, bytes: &[u8]) -> bool {
        if !self.is_attached() {
            return false;
        }
        self.pending_tx_bytes.extend_from_slice(bytes);
        true
    }

    /// Consomme les octets en attente d'envoi vers `SendTerminalInput`.
    pub fn drain_pending_input(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_tx_bytes)
    }

    /// Rend l'écran `vt100` via [`tui_term::widget::PseudoTerminal`].
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, ascii_only: bool, no_color: bool) {
        let title = match &self.attach_state {
            PtyAttachState::Detached if self.process_run_uuid.is_empty() => {
                String::from(" PTY · aucun processus chargé ")
            }
            PtyAttachState::Detached => {
                format!(" PTY {} [DETACHE — 'a' attacher] ", self.process_run_uuid)
            }
            PtyAttachState::Attached { lease_id, fence } => format!(
                " PTY {} [ATTACHE lease={} fence={} — Ctrl+] detacher] ",
                self.process_run_uuid, lease_id, fence
            ),
        };
        let border_set = if ascii_only {
            ratatui::symbols::border::PLAIN
        } else {
            ratatui::symbols::border::ROUNDED
        };
        let border_style = if no_color {
            Style::default()
        } else if self.is_attached() {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_set(border_set)
            .border_style(border_style);

        let pseudo_term = PseudoTerminal::new(self.parser.screen()).block(block);
        pseudo_term.render(area, buf);
    }
}
