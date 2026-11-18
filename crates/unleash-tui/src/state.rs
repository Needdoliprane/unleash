// SPDX-License-Identifier: Apache-2.0
//! État de présentation de la TUI OVPT Unleash (`F015`, 10 espaces, charge M0 virtualisée).

use std::collections::VecDeque;

use unleash_domain::Revision;
use unleash_protocol::{EventCursor, OperationName};

use crate::editor::TuiEditor;
use crate::pty_view::PtyPanel;
use crate::sanitize::sanitize_single_line;
use crate::terminal::TerminalCapabilities;

/// Taille maximale d'une page d'événements conservée côté client (`T-F015-015`).
pub const EVENT_PAGE_LIMIT: usize = 200;
/// Capacité maximale de la file de notifications de présentation (`T-F015-015`).
pub const MAX_NOTIFICATION_QUEUE: usize = 1024;
/// Nombre total d'événements historiques exposés en pagination dans la tranche M0.
pub const M0_TOTAL_EVENTS: usize = 100_000;
/// Nombre total d'agents logiques dans l'arbre M0.
pub const M0_TOTAL_AGENTS: usize = 1_000;
/// Nombre total de lignes du diff M0.
pub const M0_TOTAL_DIFF_LINES: usize = 10_000;

/// Les dix espaces normatifs de la matrice de parité GUI/TUI (`F014`, `F015`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Space {
    /// Vue d'ensemble mission & démarrage guidé.
    Overview,
    /// Lots, worktrees, diffs et intégrations.
    Workspaces,
    /// Arbre N+2/N+1/N des agents logiques et invocations.
    Agents,
    /// Fils humains N+2/N+1.
    Conversations,
    /// Workflows et procédures F019.
    Workflows,
    /// Studio skills et rôles.
    Skills,
    /// Connaissance, discovery et provenance.
    Knowledge,
    /// Activité, timeline, processus et PTY.
    Activity,
    /// Rapports et preuves.
    Reports,
    /// Paramètres, modèles locaux, AI-Guard et maintenance.
    Settings,
}

impl Space {
    /// Liste ordonnée des 10 espaces.
    pub const ALL: [Self; 10] = [
        Self::Overview,
        Self::Workspaces,
        Self::Agents,
        Self::Conversations,
        Self::Workflows,
        Self::Skills,
        Self::Knowledge,
        Self::Activity,
        Self::Reports,
        Self::Settings,
    ];

    /// Libellé court affiché dans la barre d'onglets.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Workspaces => "Workspaces",
            Self::Agents => "Agents",
            Self::Conversations => "Conversations",
            Self::Workflows => "Workflows",
            Self::Skills => "Skills",
            Self::Knowledge => "Knowledge",
            Self::Activity => "Activity",
            Self::Reports => "Reports",
            Self::Settings => "Settings",
        }
    }

    /// Passe à l'espace suivant.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Overview => Self::Workspaces,
            Self::Workspaces => Self::Agents,
            Self::Agents => Self::Conversations,
            Self::Conversations => Self::Workflows,
            Self::Workflows => Self::Skills,
            Self::Skills => Self::Knowledge,
            Self::Knowledge => Self::Activity,
            Self::Activity => Self::Reports,
            Self::Reports => Self::Settings,
            Self::Settings => Self::Overview,
        }
    }

    /// Passe à l'espace précédent.
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::Overview => Self::Settings,
            Self::Workspaces => Self::Overview,
            Self::Agents => Self::Workspaces,
            Self::Conversations => Self::Agents,
            Self::Workflows => Self::Conversations,
            Self::Skills => Self::Workflows,
            Self::Knowledge => Self::Skills,
            Self::Activity => Self::Knowledge,
            Self::Reports => Self::Activity,
            Self::Settings => Self::Reports,
        }
    }
}

/// Panneau principal actuellement focalisé pour la navigation clavier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    /// Arbre des agents (1 000 agents virtualisés).
    AgentTree,
    /// Timeline paginée des événements (100 000 total, pages de 200).
    Timeline,
    /// Diff virtualisé de 10 000 lignes.
    DiffViewer,
    /// Éditeur multiligne intégré (SKILL/WORKFLOW/message).
    Editor,
    /// Inspecteur PTY isolé.
    Pty,
}

impl FocusPane {
    /// Cycle vers le panneau suivant avec `Tab`.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::AgentTree => Self::Timeline,
            Self::Timeline => Self::DiffViewer,
            Self::DiffViewer => Self::Editor,
            Self::Editor => Self::Pty,
            Self::Pty => Self::AgentTree,
        }
    }

    /// Cycle vers le panneau précédent avec `Shift+Tab`.
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::AgentTree => Self::Pty,
            Self::Timeline => Self::AgentTree,
            Self::DiffViewer => Self::Timeline,
            Self::Editor => Self::DiffViewer,
            Self::Pty => Self::Editor,
        }
    }
}

/// Mode d'interaction actif dans le shell (`cli-tui-design.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Navigation globale par raccourcis clavier.
    Navigation,
    /// Filtrage incrémental (`/`).
    Search,
    /// Palette de commandes (`Ctrl+P`).
    Palette,
    /// Saisie dans l'éditeur multiligne (`e`).
    Editing,
    /// Saisie capturée par le PTY attaché (`a`, sortie par `Ctrl+]`).
    PtyAttached,
    /// Aide contextuelle (`?`).
    Help,
}

/// Ligne d'agent logique dans l'arbre M0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    /// Identifiant stable de l'agent.
    pub id: String,
    /// Identifiant du parent hiérarchique.
    pub parent_id: Option<String>,
    /// Niveau hiérarchique (`N+2`, `N+1`, `N`).
    pub level: &'static str,
    /// Nom affiché (assaini).
    pub name: String,
    /// État logique distinct de l'invocation.
    pub logical_state: &'static str,
    /// État de l'invocation active éventuelle.
    pub invocation_state: &'static str,
    /// Modèle configuré.
    pub model: &'static str,
    /// Profondeur d'indentation.
    pub depth: u8,
    /// Indique si ce nœud possède des enfants repliables.
    pub has_children: bool,
}

/// Entrée d'événement dans la page active de la timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineItem {
    /// Numéro de séquence monotone dans le journal.
    pub sequence: u64,
    /// Type d'événement.
    pub event_type: &'static str,
    /// Résumé assaini.
    pub summary: String,
    /// Causalité.
    pub causation: String,
    /// Sensibilité.
    pub sensitivity: &'static str,
}

/// Entrée de la palette de commandes raccordée aux opérations du catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteCommand {
    /// Identifiant d'opération canonique du protocole.
    pub operation: OperationName,
    /// Libellé affiché.
    pub label: &'static str,
    /// Domaine / espace cible.
    pub space: Space,
    /// Raccourci indicatif.
    pub shortcut: &'static str,
}

/// État de connexion IPC affiché dans la barre de contexte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionBanner {
    /// Connexion initiale en cours.
    Connecting,
    /// Connecté au daemon avec curseur courant.
    Connected {
        /// Curseur courant synchronisé.
        cursor: Option<EventCursor>,
    },
    /// Déconnecté (brouillons conservés sans soumission silencieuse).
    Disconnected {
        /// Motif affiché.
        reason: String,
    },
    /// Débordement de file locale exigeant une resynchronisation snapshot.
    ResyncRequired {
        /// Motif affiché.
        reason: String,
    },
}

/// État de l'application TUI.
#[derive(Debug)]
pub struct TuiState {
    /// Espace actif parmi les 10 espaces.
    pub active_space: Space,
    /// Panneau focalisé.
    pub focus: FocusPane,
    /// Mode d'interaction courant.
    pub mode: InputMode,
    /// Capacités terminal actives (`NO_COLOR`, ASCII, mode linéaire).
    pub capabilities: TerminalCapabilities,
    /// Indique si l'inspecteur latéral est ouvert (commutable en 80x24).
    pub inspector_open: bool,
    /// État de connexion daemon.
    pub connection: ConnectionBanner,
    /// Arbre complet des 1 000 agents logiques.
    pub agents: Vec<AgentEntry>,
    /// Indices visibles après pliage et filtre.
    pub visible_agent_indices: Vec<usize>,
    /// Identifiants des nœuds repliés.
    pub collapsed_agent_ids: std::collections::HashSet<String>,
    /// Sélection dans `visible_agent_indices`.
    pub selected_agent_pos: usize,
    /// Décalage de scroll vertical de l'arbre agents.
    pub agent_scroll: usize,
    /// Filtre texte courant (`/`).
    pub filter_query: String,
    /// Page active (<= 200 entrées) issue des 100 000 événements.
    pub event_page: Vec<TimelineItem>,
    /// Décalage de la page d'événements dans le total de 100 000.
    pub event_page_offset: usize,
    /// Sélection dans `event_page`.
    pub selected_event_pos: usize,
    /// Nombre total d'événements côté backend.
    pub total_events: usize,
    /// Nombre total de lignes du diff virtualisé.
    pub total_diff_lines: usize,
    /// Ligne courante dans le diff de 10 000 lignes.
    pub diff_scroll_line: usize,
    /// Cache fenêtré des lignes visibles du diff.
    pub diff_window_cache: Vec<String>,
    /// Éditeur multiligne intégré (`ropey` + `unicode-segmentation`).
    pub editor: TuiEditor,
    /// Panneau PTY isolé (`vt100` + `tui-term`).
    pub pty: PtyPanel,
    /// Entrées de la palette de commandes (`Ctrl+P`).
    pub palette_commands: Vec<PaletteCommand>,
    /// Requête de filtre dans la palette.
    pub palette_query: String,
    /// Sélection dans la palette filtrée.
    pub palette_selected: usize,
    /// File bornée des notifications reçues (`<= MAX_NOTIFICATION_QUEUE`).
    pub notification_queue: VecDeque<String>,
    /// Nombre de notifications perdues par backpressure.
    pub dropped_notifications: u64,
    /// Révision d'agrégat affichée pour les brouillons locaux.
    pub draft_base_revision: Revision,
    /// Message de statut de pied de page.
    pub status_line: String,
    /// Demande d'arrêt propre du client (n'arrête jamais le daemon).
    pub should_quit: bool,
    /// Les données et leases simulés sont réservés au banc M0.
    pub(crate) fixture: bool,
}

impl TuiState {
    /// Construit l'état initial du client avant la connexion au daemon.
    #[must_use]
    pub fn new(capabilities: TerminalCapabilities) -> Self {
        Self {
            active_space: Space::Overview,
            focus: FocusPane::AgentTree,
            mode: InputMode::Navigation,
            capabilities,
            inspector_open: true,
            connection: ConnectionBanner::Connecting,
            agents: Vec::new(),
            visible_agent_indices: Vec::new(),
            collapsed_agent_ids: std::collections::HashSet::new(),
            selected_agent_pos: 0,
            agent_scroll: 0,
            filter_query: String::new(),
            event_page: Vec::new(),
            event_page_offset: 0,
            selected_event_pos: 0,
            total_events: 0,
            total_diff_lines: 0,
            diff_scroll_line: 0,
            diff_window_cache: Vec::new(),
            editor: TuiEditor::new(""),
            pty: PtyPanel::new("", 18, 80),
            palette_commands: build_palette_commands(),
            palette_query: String::new(),
            palette_selected: 0,
            notification_queue: VecDeque::with_capacity(64),
            dropped_notifications: 0,
            draft_base_revision: Revision::INITIAL,
            status_line: String::from(
                "Prêt · Tab: panneau · /: filtre · Ctrl+P: palette · ?: aide · q: quitter",
            ),
            should_quit: false,
            fixture: false,
        }
    }

    /// Construit la charge de test M0 avec 1 000 agents, 100 000 événements
    /// paginés et un diff de 10 000 lignes.
    #[must_use]
    pub fn with_m0_fixture(capabilities: TerminalCapabilities) -> Self {
        let mut state = Self::new(capabilities);
        state.fixture = true;
        state.connection = ConnectionBanner::Connected { cursor: None };
        state.agents = build_m0_agents(M0_TOTAL_AGENTS);
        state.total_events = M0_TOTAL_EVENTS;
        state.total_diff_lines = M0_TOTAL_DIFF_LINES;
        state.editor = TuiEditor::new(
            "name: reconcile-csv\nmode: supervised\n# Éditeur intégré TUI (ropey + unicode)\n",
        );
        state.pty = PtyPanel::new("proc-run-9b4a2c10", 18, 80);
        state
            .pty
            .process_output(b"[unleash-pty] fixture du banc M0\r\n");
        state.draft_base_revision = Revision::new(413);
        state.recompute_visible_agents();
        state.load_event_page(0);
        state.load_diff_window(0, 40);
        state
    }

    /// Recalcule les indices visibles de l'arbre des 1 000 agents selon pliage et filtre.
    pub fn recompute_visible_agents(&mut self) {
        let needle = self.filter_query.trim().to_lowercase();
        let mut visible = Vec::new();
        for (idx, agent) in self.agents.iter().enumerate() {
            if !needle.is_empty() {
                let matches = agent.name.to_lowercase().contains(&needle)
                    || agent.logical_state.to_lowercase().contains(&needle)
                    || agent.model.to_lowercase().contains(&needle);
                if matches {
                    visible.push(idx);
                }
                continue;
            }
            if let Some(parent) = &agent.parent_id
                && self.collapsed_agent_ids.contains(parent)
            {
                continue;
            }
            visible.push(idx);
        }
        self.visible_agent_indices = visible;
        if self.selected_agent_pos >= self.visible_agent_indices.len() {
            self.selected_agent_pos = self.visible_agent_indices.len().saturating_sub(1);
        }
    }

    /// Charge une page bornée du jeu de données de test.
    pub fn load_event_page(&mut self, offset: usize) {
        if !self.fixture {
            return;
        }
        let clamped_offset = offset.min(self.total_events.saturating_sub(1));
        let needle = self.filter_query.trim().to_lowercase();
        let mut items = Vec::with_capacity(EVENT_PAGE_LIMIT);
        let mut seq_idx = clamped_offset;
        let max_scan = EVENT_PAGE_LIMIT * 10;
        let mut scanned = 0usize;

        while items.len() < EVENT_PAGE_LIMIT && seq_idx < self.total_events && scanned < max_scan {
            let seq = (seq_idx + 1) as u64;
            let event_type = match seq % 4 {
                0 => "IntegrationBlocked",
                1 => "InvocationAdmitted",
                2 => "ObservationRecorded",
                _ => "DocumentStaleDetected",
            };
            let summary = sanitize_single_line(&format!(
                "#{seq} {event_type} lot-{} worker-{:04}",
                (seq % 12) + 1,
                (seq % 977) + 1
            ));
            if needle.is_empty() || summary.to_lowercase().contains(&needle) {
                items.push(TimelineItem {
                    sequence: seq,
                    event_type,
                    summary,
                    causation: format!("command:rcpt-{seq}"),
                    sensitivity: "project",
                });
            }
            seq_idx += 1;
            scanned += 1;
        }
        self.event_page_offset = clamped_offset;
        self.event_page = items;
        if self.selected_event_pos >= self.event_page.len() {
            self.selected_event_pos = self.event_page.len().saturating_sub(1);
        }
    }

    /// Charge une fenêtre bornée du diff de test.
    pub fn load_diff_window(&mut self, start_line: usize, count: usize) {
        if !self.fixture {
            return;
        }
        let safe_start = start_line.min(self.total_diff_lines.saturating_sub(1));
        let end = (safe_start + count.max(1)).min(self.total_diff_lines);
        let mut lines = Vec::with_capacity(end.saturating_sub(safe_start));
        for idx in safe_start..end {
            let line_no = idx + 1;
            if line_no % 50 == 0 {
                lines.push(format!(
                    "- {line_no:>5} | state_{line_no} = Transition::Pending;\n+ {line_no:>5} | state_{line_no} = Transition::Verified;"
                ));
            } else {
                lines.push(format!(
                    "  {line_no:>5} | state_{line_no} = Transition::Pending;"
                ));
            }
        }
        self.diff_scroll_line = safe_start;
        self.diff_window_cache = lines;
    }

    /// Injecte un message/log externe assaini dans la file bornée (avec backpressure explicite).
    pub fn push_notification(&mut self, raw: &str) {
        let clean = sanitize_single_line(raw);
        if self.notification_queue.len() >= MAX_NOTIFICATION_QUEUE {
            self.dropped_notifications = self.dropped_notifications.saturating_add(1);
            let _ = self.notification_queue.pop_front();
            self.connection = ConnectionBanner::ResyncRequired {
                reason: format!(
                    "File locale saturée ({} notifications écartées)",
                    self.dropped_notifications
                ),
            };
        }
        self.notification_queue.push_back(clean);
    }

    /// Retourne l'agent actuellement sélectionné dans l'arbre visible.
    #[must_use]
    pub fn selected_agent(&self) -> Option<&AgentEntry> {
        let idx = *self.visible_agent_indices.get(self.selected_agent_pos)?;
        self.agents.get(idx)
    }

    /// Retourne l'événement actuellement sélectionné dans la page courante.
    #[must_use]
    pub fn selected_event(&self) -> Option<&TimelineItem> {
        self.event_page.get(self.selected_event_pos)
    }

    /// Retourne les commandes de palette correspondant au filtre saisi.
    #[must_use]
    pub fn filtered_palette_commands(&self) -> Vec<&PaletteCommand> {
        let needle = self.palette_query.trim().to_lowercase();
        self.palette_commands
            .iter()
            .filter(|cmd| {
                needle.is_empty()
                    || cmd.label.to_lowercase().contains(&needle)
                    || cmd.operation.as_str().to_lowercase().contains(&needle)
            })
            .collect()
    }
}

fn build_m0_agents(total: usize) -> Vec<AgentEntry> {
    let mut out = Vec::with_capacity(total);
    let teams = [
        ("team-core", "N+2 Core Platform", "dormant"),
        ("team-connectors", "N+2 Connecteurs & Docs", "blocked"),
        ("team-qa", "N+2 Audit & Contrats", "dormant"),
    ];
    for (id, name, st) in teams {
        if out.len() >= total {
            break;
        }
        out.push(AgentEntry {
            id: id.to_owned(),
            parent_id: None,
            level: "N+2",
            name: sanitize_single_line(name),
            logical_state: st,
            invocation_state: "none",
            model: "claude-sonnet-4-5",
            depth: 0,
            has_children: true,
        });
    }

    let leader_count = 20.min(total.saturating_sub(out.len()));
    for i in 0..leader_count {
        let (team_id, _, _) =
            teams
                .get(i % teams.len())
                .copied()
                .unwrap_or(("team-core", "", "dormant"));
        let (logical_state, invocation_state) = match i.cmp(&2) {
            std::cmp::Ordering::Less => ("active", "running"),
            std::cmp::Ordering::Equal => ("blocked", "idle"),
            std::cmp::Ordering::Greater => ("dormant", "idle"),
        };
        out.push(AgentEntry {
            id: format!("leader-{:03}", i + 1),
            parent_id: Some(team_id.to_owned()),
            level: "N+1",
            name: format!("Leader-{:02}", i + 1),
            logical_state,
            invocation_state,
            model: if i % 2 == 0 {
                "claude-haiku-5-5"
            } else {
                "ollama:qwen3-coder"
            },
            depth: 1,
            has_children: true,
        });
    }

    let mut worker_idx = 1usize;
    while out.len() < total {
        let leader_no = ((worker_idx - 1) % leader_count.max(1)) + 1;
        let logical_state = match worker_idx % 6 {
            0 => "blocked",
            1 if worker_idx == 1 => "active",
            2 => "waiting_tool",
            3 => "queued",
            _ => "dormant",
        };
        let invocation_state = if logical_state == "active" {
            "running"
        } else if logical_state == "waiting_tool" {
            "waiting"
        } else {
            "none"
        };
        out.push(AgentEntry {
            id: format!("worker-{worker_idx:04}"),
            parent_id: Some(format!("leader-{leader_no:03}")),
            level: "N",
            name: format!("Worker-{worker_idx:04} [lot-{}]", (worker_idx % 12) + 1),
            logical_state,
            invocation_state,
            model: if worker_idx.is_multiple_of(4) {
                "gpt-6-luna"
            } else {
                "claude-haiku-5-5"
            },
            depth: 2,
            has_children: false,
        });
        worker_idx += 1;
    }

    out
}

fn build_palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            operation: OperationName::GetOverview,
            label: "Rafraîchir Overview",
            space: Space::Overview,
            shortcut: "1",
        },
        PaletteCommand {
            operation: OperationName::ListGuidedTemplates,
            label: "Démarrage guidé — lister templates",
            space: Space::Overview,
            shortcut: "g",
        },
        PaletteCommand {
            operation: OperationName::ListAgents,
            label: "Inspecter arbre des agents",
            space: Space::Agents,
            shortcut: "3",
        },
        PaletteCommand {
            operation: OperationName::SendHumanMessage,
            label: "Envoyer un message N+1",
            space: Space::Conversations,
            shortcut: "4",
        },
        PaletteCommand {
            operation: OperationName::GetWorkspaceDiff,
            label: "Ouvrir diff de l'espace de travail",
            space: Space::Workspaces,
            shortcut: "2",
        },
        PaletteCommand {
            operation: OperationName::ValidateWorkflowDraft,
            label: "Valider brouillon workflow/procédure",
            space: Space::Workflows,
            shortcut: "5",
        },
        PaletteCommand {
            operation: OperationName::SaveSkillDraft,
            label: "Sauvegarder brouillon SKILL",
            space: Space::Skills,
            shortcut: "6",
        },
        PaletteCommand {
            operation: OperationName::SearchKnowledge,
            label: "Explorer connaissances et documents",
            space: Space::Knowledge,
            shortcut: "7",
        },
        PaletteCommand {
            operation: OperationName::AcquireTerminal,
            label: "Acquérir contrôle PTY",
            space: Space::Activity,
            shortcut: "a",
        },
        PaletteCommand {
            operation: OperationName::ExportReport,
            label: "Exporter rapport et preuves",
            space: Space::Reports,
            shortcut: "9",
        },
        PaletteCommand {
            operation: OperationName::ReadSettings,
            label: "Ouvrir paramètres locaux et AI-Guard",
            space: Space::Settings,
            shortcut: "0",
        },
    ]
}
