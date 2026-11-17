// SPDX-License-Identifier: Apache-2.0
//! Cycle de vie terminal RAII, détection de capacités (`NO_COLOR`, compact/linéaire)
//! et restauration idempotente sur sortie normale, panique ou suspension (`AC-F015-013`, `AC-F015-014`).

use std::io::{self, IsTerminal, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use thiserror::Error;

/// Erreur liée à l'initialisation ou la restauration du terminal interactif.
#[derive(Debug, Error)]
pub enum TerminalLifecycleError {
    /// Flux stdout/stdin non interactif (refus de forcer le raw mode sur pipe/non-TTY).
    #[error("Terminal interactif requis pour `unleash tui` (stdin/stdout non-TTY)")]
    NotATty,

    /// Erreur d'entrée/sortie lors d'un appel Crossterm/Ratatui.
    #[error("Erreur I/O terminal : {0}")]
    Io(#[from] io::Error),
}

/// Préférences d'affichage et capacités détectées du terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Drapeaux indépendants des capacités terminal détectées (NO_COLOR, ASCII, souris, lecture linéaire)"
)]
pub struct TerminalCapabilities {
    /// Désactive les couleurs lorsque `NO_COLOR` est défini ou demandé.
    pub no_color: bool,
    /// Utilise des bordures ASCII pures au lieu des glyphes Unicode box-drawing.
    pub ascii_only: bool,
    /// Active la capture souris optionnelle (toujours désactivable sans perte de parité).
    pub enable_mouse: bool,
    /// Mode lecture linéaire adapté aux lecteurs d'écran (`VoiceOver` / Orca).
    pub linear_reader_mode: bool,
}

impl Default for TerminalCapabilities {
    fn default() -> Self {
        Self::detect_from_env()
    }
}

impl TerminalCapabilities {
    /// Détecte les préférences passives depuis l'environnement (`NO_COLOR`, `TERM`).
    #[must_use]
    pub fn detect_from_env() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let term = std::env::var("TERM").unwrap_or_default();
        let ascii_only = term == "dumb";
        Self {
            no_color,
            ascii_only,
            enable_mouse: false,
            linear_reader_mode: false,
        }
    }
}

/// État partagé permettant une restauration idempotente y compris depuis un hook de panique.
#[derive(Debug, Default)]
pub struct RestoreState {
    active: AtomicBool,
    mouse_enabled: AtomicBool,
    restore_count: AtomicBool,
}

impl RestoreState {
    /// Indique si le mode terminal actif attend encore une restauration.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Indique si au moins une restauration complète a déjà été exécutée.
    #[must_use]
    pub fn was_restored(&self) -> bool {
        self.restore_count.load(Ordering::SeqCst)
    }

    /// Exécute de façon idempotente la sortie des modes raw/alternate/paste/focus/souris et réaffiche le curseur.
    ///
    /// # Errors
    /// Retourne l'erreur I/O éventuelle sans bloquer la réinitialisation des autres drapeaux.
    pub fn restore_stdout(&self) -> io::Result<()> {
        if !self.active.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        self.restore_count.store(true, Ordering::SeqCst);

        let mut stdout = io::stdout();
        let mouse = self.mouse_enabled.swap(false, Ordering::SeqCst);
        let first_err = disable_raw_mode().err();
        let exec_res = if mouse {
            execute!(
                stdout,
                DisableMouseCapture,
                DisableBracketedPaste,
                DisableFocusChange,
                LeaveAlternateScreen,
                crossterm::cursor::Show
            )
        } else {
            execute!(
                stdout,
                DisableBracketedPaste,
                DisableFocusChange,
                LeaveAlternateScreen,
                crossterm::cursor::Show
            )
        };
        let flush_res = stdout.flush();

        if let Some(err) = first_err {
            return Err(err);
        }
        exec_res?;
        flush_res
    }
}

/// Garde RAII propriétaire du terminal interactif Crossterm + Ratatui.
///
/// Restaure automatiquement tous les modes activés dans [`Drop`] et installe un hook de panique
/// préservant le hook précédent (`S-TUI-04`).
#[derive(Debug)]
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restore_state: Arc<RestoreState>,
}

impl TerminalGuard {
    /// Initialise le terminal interactif après vérification stricte qu'il s'agit bien d'un TTY.
    ///
    /// # Errors
    /// Retourne [`TerminalLifecycleError::NotATty`] si stdin ou stdout n'est pas un TTY,
    /// ou [`TerminalLifecycleError::Io`] en cas d'échec d'initialisation partielle (nettoyée avant retour).
    pub fn enter(caps: &TerminalCapabilities) -> Result<Self, TerminalLifecycleError> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(TerminalLifecycleError::NotATty);
        }

        let restore_state = Arc::new(RestoreState::default());
        install_panic_hook(Arc::clone(&restore_state));

        enable_raw_mode()?;
        restore_state.active.store(true, Ordering::SeqCst);

        let mut stdout = io::stdout();
        let setup_res = if caps.enable_mouse {
            restore_state.mouse_enabled.store(true, Ordering::SeqCst);
            execute!(
                stdout,
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableFocusChange,
                EnableMouseCapture
            )
        } else {
            execute!(
                stdout,
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableFocusChange
            )
        };

        if let Err(err) = setup_res {
            let _ = restore_state.restore_stdout();
            return Err(TerminalLifecycleError::Io(err));
        }

        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(term) => term,
            Err(err) => {
                let _ = restore_state.restore_stdout();
                return Err(TerminalLifecycleError::Io(err));
            }
        };

        Ok(Self {
            terminal,
            restore_state,
        })
    }

    /// Accès mutable au [`Terminal`] Ratatui pour le rendu des frames.
    pub const fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    /// Restaure manuellement et de manière idempotente l'état initial du terminal.
    ///
    /// # Errors
    /// Propage l'erreur I/O de restauration éventuelle.
    pub fn restore(&mut self) -> io::Result<()> {
        self.terminal.show_cursor()?;
        self.restore_state.restore_stdout()
    }

    /// Réactive les modes raw/alternate après reprise (`SIGCONT`) d'une suspension.
    ///
    /// # Errors
    /// Retourne [`TerminalLifecycleError::Io`] si le réarmement échoue.
    pub fn reenter_after_resume(
        &mut self,
        caps: &TerminalCapabilities,
    ) -> Result<(), TerminalLifecycleError> {
        enable_raw_mode()?;
        self.restore_state.active.store(true, Ordering::SeqCst);
        let mut stdout = io::stdout();
        if caps.enable_mouse {
            self.restore_state
                .mouse_enabled
                .store(true, Ordering::SeqCst);
            execute!(
                stdout,
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableFocusChange,
                EnableMouseCapture
            )?;
        } else {
            execute!(
                stdout,
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableFocusChange
            )?;
        }
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

static REGISTERED_RESTORE: Mutex<Option<Arc<RestoreState>>> = Mutex::new(None);
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Enregistre un [`RestoreState`] auprès du hook de panique global et installe le hook une seule fois.
pub fn install_panic_hook(state: Arc<RestoreState>) {
    if let Ok(mut guard) = REGISTERED_RESTORE.lock() {
        *guard = Some(state);
    }
    if !HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            if let Ok(guard) = REGISTERED_RESTORE.lock()
                && let Some(st) = guard.as_ref()
            {
                let _ = st.restore_stdout();
            }
            previous_hook(panic_info);
        }));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use super::{RestoreState, TerminalCapabilities, TerminalGuard, TerminalLifecycleError};

    #[test]
    fn restore_state_is_idempotent_even_without_tty() {
        let state = Arc::new(RestoreState::default());
        assert!(!state.is_active());
        assert!(!state.was_restored());
        // Appel idempotent sur état inactif : ne doit pas paniquer ni basculer was_restored
        state.restore_stdout().expect("no-op sans panique");
        assert!(!state.was_restored());

        // Simuler une activation sans appeler ioctl TTY dans un runner sans TTY :
        state.active.store(true, Ordering::SeqCst);
        let _ = state.restore_stdout();
        assert!(!state.is_active());
        assert!(state.was_restored());
        // Deuxième appel strictement idempotent
        state.restore_stdout().expect("deuxième appel no-op");
        assert!(!state.is_active());
    }

    #[test]
    fn non_tty_guard_refuses_raw_mode_cleanly() {
        if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            let err = TerminalGuard::enter(&TerminalCapabilities::default())
                .expect_err("refus non-TTY attendu en runner CI");
            assert!(matches!(err, TerminalLifecycleError::NotATty));
        }
    }
}
