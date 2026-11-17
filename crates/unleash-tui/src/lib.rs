// SPDX-License-Identifier: Apache-2.0
//! # unleash-tui
//!
//! Responsabilité exclusive : Interface terminal plein écran Ratatui 0.30 + Crossterm 0.29
//! à parité fonctionnelle avec la GUI (`ADR-15`, `F015`).
//!
//! ## Garanties et frontières
//! - `#![forbid(unsafe_code)]`.
//! - Consomme exclusivement `unleash_client::UnleashClient` (zéro accès `SQLite`, zéro logique
//!   métier locale).
//! - Un seul lecteur d'événements terminal (`crossterm::event::EventStream`) sous Tokio et une
//!   seule tâche de rendu, avec files bornées (`MAX_NOTIFICATION_QUEUE = 1024`).
//! - Module d'édition interne [`editor::TuiEditor`] fondé sur `ropey` + `unicode-segmentation`
//!   (`tui-textarea 0.7.0` étant incompatible avec Ratatui 0.30 / Crossterm 0.29).
//! - Rendu PTY isolé [`pty_view::PtyPanel`] fondé sur `vt100` + `tui-term`, distinct des vues
//!   textuelles qui neutralisent les séquences ANSI/OSC hostiles via [`sanitize::sanitize_terminal_text`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod app;
pub mod editor;
pub mod pty_view;
pub mod sanitize;
pub mod state;
pub mod terminal;
pub mod ui;

pub use app::{
    BackendCommand, BackendUpdate, apply_backend_update, handle_terminal_event, render_once,
    run_tui,
};
pub use editor::TuiEditor;
pub use pty_view::{PtyAttachState, PtyPanel};
pub use sanitize::{sanitize_single_line, sanitize_terminal_text};
pub use state::{
    AgentEntry, ConnectionBanner, EVENT_PAGE_LIMIT, FocusPane, InputMode, M0_TOTAL_AGENTS,
    M0_TOTAL_DIFF_LINES, M0_TOTAL_EVENTS, MAX_NOTIFICATION_QUEUE, PaletteCommand, Space,
    TimelineItem, TuiState,
};
pub use terminal::{RestoreState, TerminalCapabilities, TerminalGuard, TerminalLifecycleError};
pub use ui::draw;
