//! Persistance transactionnelle des projets (`ReviseProject`/`ListProjects`/`GetProject`)
//! et des paramètres (`ReadSettings`/`SetSettings` : réserve Q-15 et politique de rétention Q-12).

use std::fs;
use std::path::{Component, Path};

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unleash_domain::{CommandId, PoolId, ProjectId, Revision, TimestampMs};
use unleash_protocol::ReasonCode;

use crate::budgets::{ControlReserveMode, ExecutionPoolParams, tx_configure_execution_pool};
use crate::error::StoreError;
use crate::policies_secrets::{
    AlertSeverity, CallerAuthority, NewAlertSpec, record_alert_tx, require_security_admin,
};

/// Délai minimal de préavis (24 h = 86 400 000 ms) avant prise d'effet d'un raccourcissement Q-12.
pub const RETENTION_SHORTENING_NOTICE_MS: i64 = 86_400_000;

/// Source rattachée à un projet (dépôt, dossier documentaire, source distante).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSourceSpec {
    /// Nature de source (`directory`, `git`, `document_folder`, ...).
    pub kind: String,
    /// Chemin local ou URI validée.
    pub path_or_uri: String,
    /// Libellé affichable.
    pub label: String,
    /// Source en lecture seule.
    pub read_only: bool,
}

/// Dépôt Git rattaché optionnellement à un projet (un projet généraliste n'en requiert pas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRepositorySpec {
    /// Identifiant du dépôt dans le projet.
    pub repo_id: String,
    /// Racine locale validée du dépôt.
    pub root_path: String,
    /// Branche par défaut éventuelle.
    pub default_branch: Option<String>,
}

/// Projection persistée d'un projet (`projects`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    /// Identifiant du projet.
    pub project_id: ProjectId,
    /// Nom du projet.
    pub name: String,
    /// Description métier.
    pub description: String,
    /// Poids d'équité ordonnancement (1..=10).
    pub weight: i64,
    /// État (`active`, `archived`, ...).
    pub state: String,
    /// Racine locale principale optionnelle (aucune requise pour un projet généraliste).
    pub primary_root_path: Option<String>,
    /// Sources validées associées.
    pub sources: Vec<ProjectSourceSpec>,
    /// Dépôts associés éventuels.
    pub repositories: Vec<ProjectRepositorySpec>,
    /// Paramètres JSON typés associés au projet.
    pub settings_json: Value,
    /// Révision optimiste.
    pub revision: Revision,
    /// Date de création.
    pub created_at_ms: TimestampMs,
    /// Date de dernière mise à jour.
    pub updated_at_ms: TimestampMs,
}

/// Portée d'un agrégat `settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsScopeKind {
    /// Paramètres globaux utilisateur.
    User,
    /// Surcharges portées par un projet.
    Project,
    /// Surcharges portées par un dépôt.
    Repository,
    /// Surcharges portées par un workspace.
    Workspace,
}

impl SettingsScopeKind {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Repository => "repository",
            Self::Workspace => "workspace",
        }
    }
}

/// Configuration du pool global d'exécution et de la réserve de contrôle (Q-15 / ADR-13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPoolSettings {
    /// Nombre maximal d'exécutions simultanées C (1..=50, défaut 5).
    pub max_active_executions: u32,
    /// Mode de réserve contrôle (`soft` par défaut, `hard` seulement si C >= 2).
    pub control_reserve_mode: ControlReserveMode,
    /// Nombre d'unités réservées pour le contrôle (`1..=max(1, C/2)`).
    pub control_reserved_units: u32,
    /// Plafond de rafale contrôle (>= 1, défaut 2).
    pub control_burst_limit: u32,
    /// Délai d'escalade checkpoint en secondes (10..=600, défaut 60).
    pub checkpoint_escalation_s: u32,
    /// Poids projet par défaut (1..=10, défaut 1).
    pub project_weight: u32,
}

impl Default for ExecutionPoolSettings {
    fn default() -> Self {
        Self {
            max_active_executions: 5,
            control_reserve_mode: ControlReserveMode::Soft,
            control_reserved_units: 1,
            control_burst_limit: 2,
            checkpoint_escalation_s: 60,
            project_weight: 1,
        }
    }
}

/// Classes d'artefacts gouvernées par la politique de rétention Q-12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    /// Réponses complètes de commandes (`>= 7 j`, jamais raccourcie rétroactivement).
    CommandReceipts,
    /// Payloads d'événements du journal métier (`>= 7 j`, tombstone sans suppression ligne).
    Events,
    /// Payloads et manifestations d'appels modèles (90 j par défaut).
    ModelPayloads,
    /// Sorties brutes d'outils et terminaux (30 j par défaut).
    ToolTerminalLogs,
    /// Captures, DOM et traces navigateur (30 j par défaut).
    BrowserArtifacts,
    /// Captures de travail non committé / dirty captures (30 j par défaut).
    DirtyCaptures,
    /// Révisions et observations d'automatisation (90 j par défaut).
    AutomationData,
    /// Copies locales de documents connecteurs (30 j par défaut).
    DocumentCopies,
    /// Artefacts temporaires d'export remis (7 j par défaut).
    ExportArtifacts,
    /// Journaux de diagnostic et télémétrie locale (30 j par défaut).
    LogsTelemetry,
    /// Classe générique par défaut (30 j).
    General,
}

impl RetentionClass {
    /// Représentation SQL canonique dans `object_records.retention_class`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CommandReceipts => "command_receipts",
            Self::Events => "events",
            Self::ModelPayloads => "model_payloads",
            Self::ToolTerminalLogs => "tool_terminal_logs",
            Self::BrowserArtifacts => "browser_artifacts",
            Self::DirtyCaptures => "dirty_captures",
            Self::AutomationData => "automation_data",
            Self::DocumentCopies => "document_copies",
            Self::ExportArtifacts => "export_artifacts",
            Self::LogsTelemetry => "logs_telemetry",
            Self::General => "general",
        }
    }

    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "command_receipts" => Self::CommandReceipts,
            "events" => Self::Events,
            "model_payloads" => Self::ModelPayloads,
            "tool_terminal_logs" => Self::ToolTerminalLogs,
            "browser_artifacts" => Self::BrowserArtifacts,
            "dirty_captures" => Self::DirtyCaptures,
            "automation_data" => Self::AutomationData,
            "document_copies" => Self::DocumentCopies,
            "export_artifacts" => Self::ExportArtifacts,
            "logs_telemetry" => Self::LogsTelemetry,
            _ => Self::General,
        }
    }
}

/// Politique Q-12 (`unleash.retention.default.v1`) persistée dans `settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicySettings {
    /// Identifiant canonique de politique (`unleash.retention.default.v1`).
    pub policy_version: String,
    /// Fenêtre minimale des réponses de reçus (`>= 7`).
    pub command_receipts_min_days: u32,
    /// Fenêtre minimale avant tombstoning `events` (`>= 7`).
    pub events_min_days: u32,
    /// Rétention des payloads modèles (90 j).
    pub model_payloads_days: u32,
    /// Rétention des logs outils/terminaux (30 j).
    pub tool_terminal_logs_days: u32,
    /// Rétention des artefacts navigateur (30 j).
    pub browser_artifacts_days: u32,
    /// Rétention des captures non committées (30 j).
    pub dirty_captures_days: u32,
    /// Rétention des données d'automatisation (90 j).
    pub automation_data_days: u32,
    /// Rétention des copies locales de documents (30 j).
    pub document_copies_days: u32,
    /// Rétention des artefacts d'export (7 j).
    pub export_artifacts_days: u32,
    /// Rétention des journaux/télémétrie (30 j).
    pub logs_telemetry_days: u32,
    /// Nombre de sauvegardes locales quotidiennes conservées (7).
    pub local_backups_daily: u32,
    /// Nombre de sauvegardes locales hebdomadaires conservées (4).
    pub local_backups_weekly: u32,
    /// Borne maximale de rétention résiduelle dans la rotation des sauvegardes (35 j).
    pub max_backup_rotation_days: u32,
}

impl Default for RetentionPolicySettings {
    fn default() -> Self {
        Self {
            policy_version: "unleash.retention.default.v1".to_owned(),
            command_receipts_min_days: 7,
            events_min_days: 7,
            model_payloads_days: 90,
            tool_terminal_logs_days: 30,
            browser_artifacts_days: 30,
            dirty_captures_days: 30,
            automation_data_days: 90,
            document_copies_days: 30,
            export_artifacts_days: 7,
            logs_telemetry_days: 30,
            local_backups_daily: 7,
            local_backups_weekly: 4,
            max_backup_rotation_days: 35,
        }
    }
}

impl RetentionPolicySettings {
    /// Retourne la durée de rétention en jours pour une classe d'artefact donnée.
    #[must_use]
    pub const fn days_for_class(&self, class: RetentionClass) -> u32 {
        match class {
            RetentionClass::CommandReceipts => self.command_receipts_min_days,
            RetentionClass::Events => self.events_min_days,
            RetentionClass::ModelPayloads => self.model_payloads_days,
            RetentionClass::ToolTerminalLogs => self.tool_terminal_logs_days,
            RetentionClass::BrowserArtifacts => self.browser_artifacts_days,
            RetentionClass::DirtyCaptures => self.dirty_captures_days,
            RetentionClass::AutomationData => self.automation_data_days,
            RetentionClass::DocumentCopies => self.document_copies_days,
            RetentionClass::ExportArtifacts => self.export_artifacts_days,
            RetentionClass::LogsTelemetry | RetentionClass::General => self.logs_telemetry_days,
        }
    }
}

/// Configuration explicite d'un chemin d'analyseur (Q-13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzerPathConfig {
    /// Langage ou identifiant d'analyseur (`rust`, `python`, `typescript`, ...).
    pub language: String,
    /// Chemin explicite validé de l'exécutable.
    pub executable_path: String,
    /// Origine déclarée (`project_explicit`, `toolchain`, `user_declared`).
    pub source: String,
}

/// Patch typé accepté par `SetSettings`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatch {
    /// Nouveau profil de pool d'exécution (Q-15).
    pub execution_pool: Option<ExecutionPoolSettings>,
    /// Nouvelle politique de rétention (Q-12).
    pub retention_policy: Option<RetentionPolicySettings>,
    /// Confirmation explicite de l'utilisateur requise lors d'un raccourcissement Q-12.
    pub confirm_retention_shortening: bool,
    /// Chemins d'analyseurs configurés explicitement (Q-13).
    pub analyzer_paths: Option<Vec<AnalyzerPathConfig>>,
    /// Paramètres typés complémentaires sans secrets.
    pub extra_typed: Option<Map<String, Value>>,
}

/// Raccourcissement Q-12 planifié avec préavis minimal de 24 h.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionShorteningNotice {
    /// Politique cible en attente d'entrée en vigueur.
    pub scheduled_policy: RetentionPolicySettings,
    /// Horodatage effectif (>= date de soumission + 24 h).
    pub effective_at_ms: TimestampMs,
    /// Champs concernés par le raccourcissement.
    pub shortened_fields: Vec<String>,
}

/// État complet retourné par `ReadSettings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsRecord {
    /// Portée lue.
    pub scope_kind: SettingsScopeKind,
    /// Identifiant de portée.
    pub scope_id: String,
    /// Révision optimiste de l'agrégat `settings`.
    pub revision: Revision,
    /// Paramètres effectifs du pool d'exécution (Q-15).
    pub execution_pool: ExecutionPoolSettings,
    /// Politique active de rétention (Q-12).
    pub retention_policy: RetentionPolicySettings,
    /// Raccourcissement planifié en attente de fin de préavis >= 24 h.
    pub pending_retention: Option<RetentionShorteningNotice>,
    /// Chemins d'analyseurs explicites (Q-13).
    pub analyzer_paths: Vec<AnalyzerPathConfig>,
    /// Champs typés additionnels.
    pub extra_typed: Map<String, Value>,
    /// Date de dernière mise à jour.
    pub updated_at_ms: TimestampMs,
    /// Acteur modificateur.
    pub updated_by_actor: String,
}

fn validate_project_directory_path(
    path_str: &str,
    require_writable: bool,
) -> Result<String, StoreError> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "chemin de racine projet vide ou invalide".to_owned(),
        });
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("racine projet '{trimmed}' doit être un chemin absolu"),
        });
    }
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: format!("chemin '{trimmed}' contient '..' interdit"),
                });
            }
            Component::Normal(part) if part == ".claude" => {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: format!("chemin '{trimmed}' pointe dans .claude/ interdit"),
                });
            }
            _ => {}
        }
    }

    let meta = fs::metadata(path).map_err(|e| StoreError::InvalidArgument {
        reason: None,
        message: format!("racine projet '{trimmed}' inaccessible : {e}"),
    })?;
    if !meta.is_dir() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("racine projet '{trimmed}' n'est pas un répertoire"),
        });
    }
    if require_writable && meta.permissions().readonly() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("racine projet '{trimmed}' en lecture seule"),
        });
    }
    let canon = fs::canonicalize(path).map_err(|e| StoreError::InvalidArgument {
        reason: None,
        message: format!("échec de résolution canonique '{trimmed}' : {e}"),
    })?;
    for comp in canon.components() {
        if let Component::Normal(part) = comp
            && part == ".claude"
        {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: format!("chemin canonique '{trimmed}' pointe dans .claude/ interdit"),
            });
        }
    }
    fs::read_dir(&canon).map_err(|e| StoreError::InvalidArgument {
        reason: None,
        message: format!("droits de lecture absents sur '{trimmed}' : {e}"),
    })?;
    Ok(trimmed.to_owned())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Mutation atomique ReviseProject avec validation stricte des chemins et révision"
)]
pub(crate) fn tx_revise_project(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: CommandId,
    project_id: ProjectId,
    name: &str,
    description: &str,
    weight: i64,
    primary_root_path: Option<&str>,
    sources: &[ProjectSourceSpec],
    repositories: &[ProjectRepositorySpec],
    settings_json: &Value,
) -> Result<Revision, StoreError> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "nom de projet vide interdit".to_owned(),
        });
    }
    if !(1..=10).contains(&weight) {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("poids projet {weight} hors [1..=10]"),
        });
    }

    let cur_rev: Option<i64> = tx
        .query_row(
            "SELECT revision FROM projects WHERE project_id = ?1",
            [project_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some(rev) = cur_rev else {
        return Err(StoreError::NotFound(format!("project {project_id}")));
    };
    let exp_i64 = i64::try_from(expected_rev.as_u64()).unwrap_or(i64::MAX);
    if rev != exp_i64 {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let validated_primary = primary_root_path
        .map(|p| validate_project_directory_path(p, true))
        .transpose()?;

    for src in sources {
        if src.kind == "directory" || src.kind == "git" || src.kind == "local" {
            let _ = validate_project_directory_path(&src.path_or_uri, !src.read_only)?;
        }
    }
    for repo in repositories {
        if repo.repo_id.trim().is_empty() {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "repo_id vide interdit".to_owned(),
            });
        }
        let _ = validate_project_directory_path(&repo.root_path, true)?;
    }

    let sources_str = serde_json::to_string(sources)?;
    let repos_str = serde_json::to_string(repositories)?;
    let settings_str = serde_json::to_string(settings_json)?;
    let next_rev = rev.saturating_add(1);

    tx.execute(
        "UPDATE projects
         SET name = ?1,
             description = ?2,
             weight = ?3,
             primary_root_path = ?4,
             sources_json = ?5,
             repositories_json = ?6,
             settings_json = ?7,
             revision = ?8,
             updated_at_ms = ?9
         WHERE project_id = ?10",
        rusqlite::params![
            trimmed_name,
            description,
            weight,
            validated_primary,
            sources_str,
            repos_str,
            settings_str,
            next_rev,
            now_ms.as_i64(),
            project_id.to_string()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(Revision::new(u64::try_from(next_rev).unwrap_or(1)))
}

struct ProjectRow {
    pid_s: String,
    name: String,
    description: String,
    weight: i64,
    state: String,
    primary_root_path: Option<String>,
    sources_s: String,
    repos_s: String,
    settings_s: String,
    rev: i64,
    created_ms: i64,
    updated_ms: i64,
}

fn map_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        pid_s: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        weight: row.get(3)?,
        state: row.get(4)?,
        primary_root_path: row.get(5)?,
        sources_s: row.get(6)?,
        repos_s: row.get(7)?,
        settings_s: row.get(8)?,
        rev: row.get(9)?,
        created_ms: row.get(10)?,
        updated_ms: row.get(11)?,
    })
}

fn build_project_record(row: ProjectRow) -> Result<ProjectRecord, StoreError> {
    let ProjectRow {
        pid_s,
        name,
        description,
        weight,
        state,
        primary_root_path,
        sources_s,
        repos_s,
        settings_s,
        rev,
        created_ms,
        updated_ms,
    } = row;
    let project_id = pid_s.parse::<ProjectId>()?;
    let sources: Vec<ProjectSourceSpec> = serde_json::from_str(&sources_s)?;
    let repositories: Vec<ProjectRepositorySpec> = serde_json::from_str(&repos_s)?;
    let settings_json: Value = serde_json::from_str(&settings_s)?;
    Ok(ProjectRecord {
        project_id,
        name,
        description,
        weight,
        state,
        primary_root_path,
        sources,
        repositories,
        settings_json,
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        created_at_ms: TimestampMs::new(created_ms),
        updated_at_ms: TimestampMs::new(if updated_ms == 0 {
            created_ms
        } else {
            updated_ms
        }),
    })
}

pub(crate) fn list_projects_conn(conn: &Connection) -> Result<Vec<ProjectRecord>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT project_id, name, description, weight, state, primary_root_path,
                    sources_json, repositories_json, settings_json, revision,
                    created_at_ms, updated_at_ms
             FROM projects ORDER BY created_at_ms ASC, project_id ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], map_project_row)
        .map_err(StoreError::from_rusqlite)?;
    let mut out = Vec::new();
    for item in rows {
        out.push(build_project_record(
            item.map_err(StoreError::from_rusqlite)?,
        )?);
    }
    Ok(out)
}

pub(crate) fn get_project_conn(
    conn: &Connection,
    project_id: ProjectId,
) -> Result<ProjectRecord, StoreError> {
    let row = conn
        .query_row(
            "SELECT project_id, name, description, weight, state, primary_root_path,
                    sources_json, repositories_json, settings_json, revision,
                    created_at_ms, updated_at_ms
             FROM projects WHERE project_id = ?1",
            [project_id.to_string()],
            map_project_row,
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some(t) = row else {
        return Err(StoreError::NotFound(format!("project {project_id}")));
    };
    build_project_record(t)
}

fn validate_retention_policy(pol: &RetentionPolicySettings) -> Result<(), StoreError> {
    if pol.policy_version.trim().is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "policy_version de rétention vide interdit".to_owned(),
        });
    }
    if pol.command_receipts_min_days < 7 || pol.events_min_days < 7 {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "les fenêtres minimales command_receipts et events ne peuvent pas descendre sous 7 jours (ADR-02 / Q-12)".to_owned(),
        });
    }
    if pol.model_payloads_days == 0
        || pol.tool_terminal_logs_days == 0
        || pol.browser_artifacts_days == 0
        || pol.dirty_captures_days == 0
        || pol.automation_data_days == 0
        || pol.document_copies_days == 0
        || pol.export_artifacts_days == 0
        || pol.logs_telemetry_days == 0
        || pol.local_backups_daily == 0
        || pol.local_backups_weekly == 0
        || pol.max_backup_rotation_days == 0
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "les durées et compteurs Q-12 doivent être strictement positifs".to_owned(),
        });
    }
    Ok(())
}

fn detect_retention_shortening(
    current: &RetentionPolicySettings,
    next: &RetentionPolicySettings,
) -> Vec<String> {
    let mut shortened = Vec::new();
    let checks = [
        (
            "command_receipts_min_days",
            next.command_receipts_min_days < current.command_receipts_min_days,
        ),
        (
            "events_min_days",
            next.events_min_days < current.events_min_days,
        ),
        (
            "model_payloads_days",
            next.model_payloads_days < current.model_payloads_days,
        ),
        (
            "tool_terminal_logs_days",
            next.tool_terminal_logs_days < current.tool_terminal_logs_days,
        ),
        (
            "browser_artifacts_days",
            next.browser_artifacts_days < current.browser_artifacts_days,
        ),
        (
            "dirty_captures_days",
            next.dirty_captures_days < current.dirty_captures_days,
        ),
        (
            "automation_data_days",
            next.automation_data_days < current.automation_data_days,
        ),
        (
            "document_copies_days",
            next.document_copies_days < current.document_copies_days,
        ),
        (
            "export_artifacts_days",
            next.export_artifacts_days < current.export_artifacts_days,
        ),
        (
            "logs_telemetry_days",
            next.logs_telemetry_days < current.logs_telemetry_days,
        ),
        (
            "local_backups_daily",
            next.local_backups_daily < current.local_backups_daily,
        ),
        (
            "local_backups_weekly",
            next.local_backups_weekly < current.local_backups_weekly,
        ),
        (
            "max_backup_rotation_days",
            next.max_backup_rotation_days < current.max_backup_rotation_days,
        ),
    ];
    for (name, is_shorter) in checks {
        if is_shorter {
            shortened.push(name.to_owned());
        }
    }
    shortened
}

pub(crate) fn validate_execution_pool_q15(pool: &ExecutionPoolSettings) -> Result<(), StoreError> {
    if !(1..=50).contains(&pool.max_active_executions) {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "max_active_executions {} hors [1..=50]",
            pool.max_active_executions
        )));
    }
    let max_reserve = (pool.max_active_executions / 2).max(1);
    if pool.control_reserved_units < 1 || pool.control_reserved_units > max_reserve {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "control_reserved_units={} hors [1..={max_reserve}] pour C={}",
            pool.control_reserved_units, pool.max_active_executions
        )));
    }
    if pool.control_reserve_mode == ControlReserveMode::Hard && pool.max_active_executions < 2 {
        return Err(StoreError::ControlReserveIncoherent(
            "control_reserve_mode=hard exige max_active_executions >= 2".to_owned(),
        ));
    }
    if pool.control_burst_limit < 1 {
        return Err(StoreError::ControlReserveIncoherent(
            "control_burst_limit doit être >= 1".to_owned(),
        ));
    }
    if !(10..=600).contains(&pool.checkpoint_escalation_s) {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "checkpoint_escalation_s {} hors [10..=600]",
            pool.checkpoint_escalation_s
        )));
    }
    if !(1..=10).contains(&pool.project_weight) {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "project_weight {} hors [1..=10]",
            pool.project_weight
        )));
    }
    Ok(())
}

fn validate_analyzer_paths(paths: &[AnalyzerPathConfig]) -> Result<(), StoreError> {
    for entry in paths {
        if entry.language.trim().is_empty() || entry.executable_path.trim().is_empty() {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "configuration analyseur incomplète (langage/chemin requis)".to_owned(),
            });
        }
    }
    Ok(())
}

fn parse_settings_row(
    scope_kind: SettingsScopeKind,
    scope_id: &str,
    now_ms: TimestampMs,
    row: (
        i64,
        String,
        String,
        Option<String>,
        Option<i64>,
        String,
        String,
        i64,
        String,
    ),
) -> Result<SettingsRecord, StoreError> {
    let (
        rev,
        pool_s,
        ret_s,
        pending_ret_s,
        pending_eff_opt,
        analyzers_s,
        extra_s,
        updated_at,
        updated_by_actor,
    ) = row;
    let execution_pool: ExecutionPoolSettings = serde_json::from_str(&pool_s)?;
    let mut retention_policy: RetentionPolicySettings = serde_json::from_str(&ret_s)?;
    let mut pending_retention = None;

    if let (Some(p_str), Some(eff_ms)) = (pending_ret_s, pending_eff_opt) {
        let scheduled: RetentionPolicySettings = serde_json::from_str(&p_str)?;
        if now_ms.as_i64() >= eff_ms {
            retention_policy = scheduled;
        } else {
            let shortened_fields = detect_retention_shortening(&retention_policy, &scheduled);
            pending_retention = Some(RetentionShorteningNotice {
                scheduled_policy: scheduled,
                effective_at_ms: TimestampMs::new(eff_ms),
                shortened_fields,
            });
        }
    }

    let analyzer_paths: Vec<AnalyzerPathConfig> = serde_json::from_str(&analyzers_s)?;
    let extra_typed: Map<String, Value> = serde_json::from_str(&extra_s)?;

    Ok(SettingsRecord {
        scope_kind,
        scope_id: scope_id.to_owned(),
        revision: Revision::new(u64::try_from(rev).unwrap_or(0)),
        execution_pool,
        retention_policy,
        pending_retention,
        analyzer_paths,
        extra_typed,
        updated_at_ms: TimestampMs::new(updated_at),
        updated_by_actor,
    })
}

pub(crate) fn read_settings_conn(
    conn: &Connection,
    now_ms: TimestampMs,
    scope_kind: SettingsScopeKind,
    scope_id: &str,
) -> Result<SettingsRecord, StoreError> {
    #[expect(
        clippy::type_complexity,
        reason = "Tuple SQLite temporaire projeté en SettingsRecord"
    )]
    let row: Option<(
        i64,
        String,
        String,
        Option<String>,
        Option<i64>,
        String,
        String,
        i64,
        String,
    )> = conn
        .query_row(
            "SELECT revision, execution_pool_json, retention_policy_json,
                    pending_retention_policy_json, pending_effective_at_ms,
                    analyzer_paths_json, extra_typed_json, updated_at_ms, updated_by_actor
             FROM settings WHERE scope_kind = ?1 AND scope_id = ?2",
            rusqlite::params![scope_kind.as_str(), scope_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    match row {
        Some(t) => parse_settings_row(scope_kind, scope_id, now_ms, t),
        None => Ok(SettingsRecord {
            scope_kind,
            scope_id: scope_id.to_owned(),
            revision: Revision::INITIAL,
            execution_pool: ExecutionPoolSettings::default(),
            retention_policy: RetentionPolicySettings::default(),
            pending_retention: None,
            analyzer_paths: vec![],
            extra_typed: Map::new(),
            updated_at_ms: TimestampMs::new(0),
            updated_by_actor: "system".to_owned(),
        }),
    }
}

pub(crate) fn resolve_effective_retention_tx(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    scope_kind: &str,
    scope_id: &str,
) -> Result<RetentionPolicySettings, StoreError> {
    let row: Option<(String, Option<String>, Option<i64>)> = tx
        .query_row(
            "SELECT retention_policy_json, pending_retention_policy_json, pending_effective_at_ms
             FROM settings WHERE scope_kind = ?1 AND scope_id = ?2",
            rusqlite::params![scope_kind, scope_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((active_json, pending_opt, eff_opt)) = row else {
        return Ok(RetentionPolicySettings::default());
    };
    let active: RetentionPolicySettings = serde_json::from_str(&active_json)?;
    if let (Some(p_json), Some(eff_ms)) = (pending_opt, eff_opt)
        && now_ms.as_i64() >= eff_ms
    {
        let promoted: RetentionPolicySettings = serde_json::from_str(&p_json)?;
        tx.execute(
            "UPDATE settings
             SET retention_policy_json = ?1,
                 pending_retention_policy_json = NULL,
                 pending_effective_at_ms = NULL,
                 updated_at_ms = ?2
             WHERE scope_kind = ?3 AND scope_id = ?4",
            rusqlite::params![p_json, now_ms.as_i64(), scope_kind, scope_id],
        )
        .map_err(StoreError::from_rusqlite)?;
        return Ok(promoted);
    }
    Ok(active)
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "Mutation atomique SetSettings : Q-15 + Q-12 + synchro capacity_pools + alerte préavis"
)]
pub(crate) fn tx_set_settings(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    expected_rev: Revision,
    cmd_id: CommandId,
    scope_kind: SettingsScopeKind,
    scope_id: &str,
    caller: &CallerAuthority,
    patch: &SettingsPatch,
) -> Result<Revision, StoreError> {
    require_security_admin(caller, "SetSettings")?;
    let trimmed_scope = scope_id.trim();
    if trimmed_scope.is_empty() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "scope_id des paramètres vide interdit".to_owned(),
        });
    }

    if scope_kind == SettingsScopeKind::Project {
        let project_id = trimmed_scope.parse::<ProjectId>()?;
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if exists != 1 {
            return Err(StoreError::NotFound(format!("project {project_id}")));
        }
    }

    let current = read_settings_conn(tx, now_ms, scope_kind, trimmed_scope)?;
    if current.revision != expected_rev {
        return Err(StoreError::RevisionConflict {
            current_revision: current.revision,
            expected_revision: expected_rev,
            command_id: Some(cmd_id),
        });
    }

    let next_pool = match patch.execution_pool.as_ref() {
        Some(p) => {
            validate_execution_pool_q15(p)?;
            p.clone()
        }
        None => current.execution_pool.clone(),
    };

    if patch.execution_pool.is_some() && scope_kind == SettingsScopeKind::User {
        let existing_pool_id: Option<String> = tx
            .query_row(
                "SELECT pool_id FROM capacity_pools
                 WHERE pool_kind = 'execution' AND scope_key = 'global' AND unit = 'slot'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        let pool_id = match existing_pool_id {
            Some(s) => s.parse::<PoolId>()?,
            None => ExecutionPoolParams::default().pool_id,
        };
        tx_configure_execution_pool(
            tx,
            &ExecutionPoolParams {
                pool_id,
                requested_limit: i64::from(next_pool.max_active_executions),
                control_reserve_mode: next_pool.control_reserve_mode,
                control_reserved_units: i64::from(next_pool.control_reserved_units),
                control_burst_limit: i64::from(next_pool.control_burst_limit),
                checkpoint_escalation_s: i64::from(next_pool.checkpoint_escalation_s),
            },
        )?;
    }

    let mut next_active_ret = current.retention_policy.clone();
    let mut next_pending_json: Option<String> = current
        .pending_retention
        .as_ref()
        .map(|n| serde_json::to_string(&n.scheduled_policy))
        .transpose()?;
    let mut next_pending_eff_ms: Option<i64> = current
        .pending_retention
        .as_ref()
        .map(|n| n.effective_at_ms.as_i64());

    if let Some(ref candidate_ret) = patch.retention_policy {
        validate_retention_policy(candidate_ret)?;
        let shortened = detect_retention_shortening(&current.retention_policy, candidate_ret);
        if shortened.is_empty() {
            next_active_ret = candidate_ret.clone();
            next_pending_json = None;
            next_pending_eff_ms = None;
        } else {
            if !patch.confirm_retention_shortening {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: format!(
                        "raccourcissement de rétention ({}) requiert confirmation explicite et préavis >= 24h",
                        shortened.join(", ")
                    ),
                });
            }
            let effective_ms = now_ms
                .as_i64()
                .saturating_add(RETENTION_SHORTENING_NOTICE_MS);
            next_pending_json = Some(serde_json::to_string(candidate_ret)?);
            next_pending_eff_ms = Some(effective_ms);
            record_alert_tx(
                tx,
                now_ms,
                &NewAlertSpec {
                    alert_id: format!("ret-shorten-{}-{}", scope_kind.as_str(), trimmed_scope),
                    kind: "retention_warning".to_owned(),
                    severity: AlertSeverity::Warning,
                    blocking: false,
                    project_id: (scope_kind == SettingsScopeKind::Project)
                        .then(|| trimmed_scope.parse::<ProjectId>().ok())
                        .flatten(),
                    subject_kind: "settings".to_owned(),
                    subject_id: trimmed_scope.to_owned(),
                    title: "Raccourcissement de rétention planifié (préavis 24h)".to_owned(),
                    summary: format!(
                        "Champs raccourcis : {} (entrée en vigueur différée de 24h)",
                        shortened.join(", ")
                    ),
                    reason_code: None,
                    evidence_ref: None,
                    allowed_operations: vec!["ReadSettings".to_owned(), "PurgeHistory".to_owned()],
                },
            )?;
        }
    }

    let next_analyzers = match patch.analyzer_paths.as_ref() {
        Some(paths) => {
            validate_analyzer_paths(paths)?;
            paths.clone()
        }
        None => current.analyzer_paths,
    };
    let next_extra = patch.extra_typed.clone().unwrap_or(current.extra_typed);

    let next_rev = current.revision.as_u64().saturating_add(1);
    let next_rev_i64 = i64::try_from(next_rev).unwrap_or(1);

    let pool_json = serde_json::to_string(&next_pool)?;
    let ret_json = serde_json::to_string(&next_active_ret)?;
    let analyzers_json = serde_json::to_string(&next_analyzers)?;
    let extra_json = serde_json::to_string(&next_extra)?;

    tx.execute(
        "INSERT INTO settings
         (scope_kind, scope_id, revision, execution_pool_json, retention_policy_version,
          retention_policy_json, pending_retention_policy_json, pending_effective_at_ms,
          analyzer_paths_json, model_selection_bindings_json, extra_typed_json,
          updated_at_ms, updated_by_actor)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '[]', ?10, ?11, ?12)
         ON CONFLICT(scope_kind, scope_id) DO UPDATE SET
           revision = excluded.revision,
           execution_pool_json = excluded.execution_pool_json,
           retention_policy_version = excluded.retention_policy_version,
           retention_policy_json = excluded.retention_policy_json,
           pending_retention_policy_json = excluded.pending_retention_policy_json,
           pending_effective_at_ms = excluded.pending_effective_at_ms,
           analyzer_paths_json = excluded.analyzer_paths_json,
           extra_typed_json = excluded.extra_typed_json,
           updated_at_ms = excluded.updated_at_ms,
           updated_by_actor = excluded.updated_by_actor",
        rusqlite::params![
            scope_kind.as_str(),
            trimmed_scope,
            next_rev_i64,
            pool_json,
            next_active_ret.policy_version,
            ret_json,
            next_pending_json,
            next_pending_eff_ms,
            analyzers_json,
            extra_json,
            now_ms.as_i64(),
            caller.actor_id.as_str()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    let _ = ReasonCode::ControlReserveIncoherent;
    Ok(Revision::new(next_rev))
}
