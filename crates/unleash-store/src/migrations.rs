//! Migrations versionnées `STRICT`, sommes de contrôle SHA-256 et vérifications d'ouverture (`REQ-PERSIST-005`).

use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::{BackupId, TimestampMs};

use crate::config::{CutpointObserver, StoreCutpoint};
use crate::error::StoreError;

/// Version courante maximale du schéma gérée par cette version de `unleash-store`.
pub const CURRENT_SCHEMA_VERSION: i64 = 2;

/// État persisté d'une migration dans `schema_migrations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    /// Migration en cours (doit bloquer une ouverture ordinaire si retrouvée au démarrage).
    Applying,
    /// Migration terminée et vérifiée.
    Applied,
    /// Migration en échec.
    Failed,
}

impl MigrationState {
    fn from_sql(s: &str) -> Option<Self> {
        match s {
            "applying" => Some(Self::Applying),
            "applied" => Some(Self::Applied),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Enregistrement d'une migration appliquée ou interrompue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// Numéro de version croissant (>= 1).
    pub version: i64,
    /// Nom symbolique de la migration.
    pub name: String,
    /// Somme de contrôle SHA-256 (`sha256:<hex>`) du DDL exact.
    pub checksum_sha256: String,
    /// État de la migration.
    pub state: MigrationState,
    /// Sauvegarde préalable vérifiée obligatoire lors d'une montée de version.
    pub pre_migration_backup_id: Option<BackupId>,
    /// Horodatage de début.
    pub started_at_ms: i64,
    /// Horodatage de fin si terminée.
    pub completed_at_ms: Option<i64>,
}

/// Résultat d'inspection ou d'exécution des migrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Version de schéma atteinte après migration.
    pub current_version: i64,
    /// Migrations exécutées lors de cette passe.
    pub newly_applied: Vec<i64>,
}

struct MigrationDef {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[MigrationDef] = &[
    MigrationDef {
        version: 1,
        name: "v001_core_authority_events_objects_execution",
        sql: V001_SQL,
    },
    MigrationDef {
        version: 2,
        name: "v002_workflow_automation_registries_and_probes",
        sql: V002_SQL,
    },
];

fn migration_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    format!("sha256:{}", hex::encode(digest))
}

/// Contrôle en lecture seule la compatibilité du schéma avant toute écriture (`REQ-PERSIST-005`).
///
/// # Errors
/// Retourne [`StoreError::UnsupportedSchemaVersion`], [`StoreError::MigrationInterrupted`],
/// [`StoreError::MigrationChecksumMismatch`], [`StoreError::MigrationBackupRequired`] ou [`StoreError::StorageCorrupt`].
#[expect(
    clippy::too_many_lines,
    reason = "Validation séquentielle stricte quick_check, user_version, états, checksums et pré-sauvegarde"
)]
pub(crate) fn preflight_check_existing_db(
    ro_conn: &Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
) -> Result<i64, StoreError> {
    let quick: String = ro_conn
        .pragma_query_value(None, "quick_check", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    if !quick.eq_ignore_ascii_case("ok") {
        return Err(StoreError::StorageCorrupt(format!(
            "PRAGMA quick_check a retourné : {quick}"
        )));
    }

    let user_version: i64 = ro_conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;

    if user_version > max_supported_version {
        return Err(StoreError::UnsupportedSchemaVersion {
            found: user_version,
            supported_max: max_supported_version,
        });
    }

    let has_table: i64 = ro_conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    if has_table == 0 {
        if user_version > 0 {
            return Err(StoreError::MigrationInterrupted {
                version: user_version,
                detail: "table schema_migrations absente pour user_version > 0".to_owned(),
            });
        }
        return Ok(0);
    }

    let mut stmt = ro_conn
        .prepare(
            "SELECT version, checksum_sha256, state FROM schema_migrations ORDER BY version ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut collected = Vec::new();
    for row_res in rows {
        collected.push(row_res.map_err(StoreError::from_rusqlite)?);
    }

    for (ver, _, _) in &collected {
        if *ver > max_supported_version {
            return Err(StoreError::UnsupportedSchemaVersion {
                found: *ver,
                supported_max: max_supported_version,
            });
        }
    }

    for (ver, _, state_str) in &collected {
        let state = MigrationState::from_sql(state_str).ok_or_else(|| {
            StoreError::MigrationInterrupted {
                version: *ver,
                detail: format!("état de migration inconnu '{state_str}'"),
            }
        })?;
        if state != MigrationState::Applied {
            return Err(StoreError::MigrationInterrupted {
                version: *ver,
                detail: format!("migration laissée dans l'état '{state_str}'"),
            });
        }
    }

    let mut max_applied = 0_i64;
    for (ver, checksum, _) in collected {
        let def = MIGRATIONS.iter().find(|m| m.version == ver).ok_or(
            StoreError::UnsupportedSchemaVersion {
                found: ver,
                supported_max: max_supported_version,
            },
        )?;
        let expected = migration_checksum(def.sql);
        if checksum != expected {
            return Err(StoreError::MigrationChecksumMismatch {
                version: ver,
                expected,
                actual: checksum,
            });
        }
        if ver > max_applied {
            max_applied = ver;
        }
    }

    if max_applied != user_version {
        tracing::warn!(
            user_version,
            max_applied,
            "migration interrompue : discordance user_version vs schema_migrations"
        );
        return Err(StoreError::MigrationInterrupted {
            version: max_applied.max(user_version),
            detail: format!(
                "discordance PRAGMA user_version ({user_version}) vs schema_migrations ({max_applied})"
            ),
        });
    }

    let target_version = max_supported_version.min(CURRENT_SCHEMA_VERSION);
    if max_applied >= 1 && target_version > max_applied {
        let Some(bkp_id) = pre_migration_backup_id else {
            tracing::warn!(
                max_applied,
                target_version,
                "montée de version refusée sans sauvegarde pré-migration"
            );
            return Err(StoreError::MigrationBackupRequired);
        };
        let valid_backup: i64 = ro_conn
            .query_row(
                "SELECT COUNT(*) FROM backup_jobs WHERE backup_id = ?1 AND state = 'complete'",
                [bkp_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if valid_backup == 0 {
            tracing::warn!(backup_id = %bkp_id, "sauvegarde pré-migration absente ou incomplète");
            return Err(StoreError::MigrationBackupRequired);
        }
    }

    Ok(max_applied)
}

/// Applique les migrations manquantes jusqu'à `max_supported_version.min(CURRENT_SCHEMA_VERSION)`.
///
/// # Errors
/// Retourne [`StoreError`] en cas d'incohérence, absence de sauvegarde pré-migration ou panne `C11`.
pub(crate) fn apply_pending_migrations(
    conn: &mut Connection,
    max_supported_version: i64,
    pre_migration_backup_id: Option<BackupId>,
    now_ms: TimestampMs,
    cutpoints: &dyn CutpointObserver,
) -> Result<MigrationReport, StoreError> {
    if max_supported_version < 1 {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("max_supported_schema_version invalide : {max_supported_version}"),
        });
    }

    let current =
        preflight_check_existing_db(conn, max_supported_version, pre_migration_backup_id)?;
    let target = max_supported_version.min(CURRENT_SCHEMA_VERSION);
    let mut newly_applied = Vec::new();

    for def in MIGRATIONS {
        if def.version <= current || def.version > target {
            continue;
        }
        let checksum = migration_checksum(def.sql);
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from_rusqlite)?;

        tx.execute_batch(def.sql)
            .map_err(StoreError::from_rusqlite)?;

        let bkp_str = pre_migration_backup_id.map(|id| id.to_string());
        tx.execute(
            "INSERT INTO schema_migrations (version, name, checksum_sha256, state, pre_migration_backup_id, started_at_ms, completed_at_ms)
             VALUES (?1, ?2, ?3, 'applying', ?4, ?5, NULL)",
            rusqlite::params![def.version, def.name, checksum, bkp_str, now_ms.as_i64()],
        )
        .map_err(StoreError::from_rusqlite)?;

        cutpoints.hit(StoreCutpoint::C11MigrationInProgress)?;

        tx.execute(
            "UPDATE schema_migrations SET state = 'applied', completed_at_ms = ?1 WHERE version = ?2",
            rusqlite::params![now_ms.as_i64(), def.version],
        )
        .map_err(StoreError::from_rusqlite)?;

        tx.pragma_update(None, "user_version", def.version)
            .map_err(StoreError::from_rusqlite)?;

        let daemon_exists: i64 = tx
            .query_row("SELECT COUNT(*) FROM daemon_state", [], |r| r.get(0))
            .map_err(StoreError::from_rusqlite)?;
        if daemon_exists > 0 {
            tx.execute(
                "UPDATE daemon_state SET schema_version = ?1 WHERE singleton = 1",
                [def.version],
            )
            .map_err(StoreError::from_rusqlite)?;
        }

        tx.commit().map_err(StoreError::from_rusqlite)?;
        tracing::info!(
            migration_version = def.version,
            migration_name = def.name,
            "migration SQLite appliquée"
        );
        newly_applied.push(def.version);
    }

    Ok(MigrationReport {
        current_version: current.max(target),
        newly_applied,
    })
}

const V001_SQL: &str = r"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version >= 1),
    name TEXT NOT NULL,
    checksum_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('applying', 'applied', 'failed')),
    pre_migration_backup_id TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
) STRICT;

CREATE TABLE IF NOT EXISTS store_runtime_manifests (
    manifest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    sqlite_version TEXT NOT NULL,
    sqlite_source_id TEXT NOT NULL,
    expected_source_id TEXT NOT NULL,
    compile_options_json TEXT NOT NULL,
    effective_pragmas_json TEXT NOT NULL,
    authority_generation TEXT,
    daemon_epoch INTEGER,
    qualified_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS daemon_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    owner_uid INTEGER NOT NULL CHECK (owner_uid >= 0),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    daemon_instance_id TEXT NOT NULL,
    boot_identity TEXT NOT NULL,
    stream_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    mode TEXT NOT NULL CHECK (mode IN ('starting', 'recovering', 'ready', 'degraded', 'read_only_recovery', 'draining', 'stopped')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    started_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS command_receipts (
    actor_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    authority_generation TEXT NOT NULL,
    receipt_id TEXT NOT NULL UNIQUE,
    request_id TEXT NOT NULL,
    command_type TEXT NOT NULL,
    command_digest TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('accepted', 'completed', 'blocked', 'rejected')),
    response_state TEXT NOT NULL CHECK (response_state IN ('full', 'tombstone')),
    response_json TEXT,
    response_retained_until_ms INTEGER NOT NULL,
    first_event_seq INTEGER,
    last_event_seq INTEGER,
    committed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (actor_id, command_id),
    CHECK ((response_state = 'full' AND response_json IS NOT NULL) OR (response_state = 'tombstone' AND response_json IS NULL))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_command_receipts_lineage ON command_receipts(authority_generation, committed_at_ms);
CREATE INDEX IF NOT EXISTS idx_command_receipts_retention ON command_receipts(response_state, response_retained_until_ms);

CREATE TRIGGER IF NOT EXISTS trg_command_receipts_immutable_fields
BEFORE UPDATE ON command_receipts
FOR EACH ROW
WHEN NEW.actor_id != OLD.actor_id
  OR NEW.command_id != OLD.command_id
  OR NEW.authority_generation != OLD.authority_generation
  OR NEW.receipt_id != OLD.receipt_id
  OR NEW.command_digest != OLD.command_digest
  OR NEW.state != OLD.state
  OR NEW.response_retained_until_ms < OLD.response_retained_until_ms
  OR (OLD.response_state = 'tombstone' AND NEW.response_state = 'full')
BEGIN
    SELECT RAISE(ABORT, 'command_receipts immutable key/digest/lineage/retention violation');
END;

CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    stream_id TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    aggregate_revision INTEGER NOT NULL CHECK (aggregate_revision >= 1),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_type TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    project_id TEXT,
    actor_ref TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    payload_state TEXT NOT NULL CHECK (payload_state IN ('full', 'tombstone')),
    payload_hash TEXT NOT NULL,
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('public', 'project', 'confidential', 'restricted')),
    command_actor_id TEXT,
    command_id TEXT,
    causation_kind TEXT NOT NULL CHECK (causation_kind IN ('command', 'event', 'action', 'timer', 'external')),
    causation_id TEXT NOT NULL,
    causation_receipt_id TEXT,
    correlation_id TEXT NOT NULL,
    UNIQUE (aggregate_type, aggregate_id, aggregate_revision, event_ordinal),
    CHECK ((command_actor_id IS NULL AND command_id IS NULL) OR (command_actor_id IS NOT NULL AND command_id IS NOT NULL)),
    CHECK ((causation_kind = 'command' AND causation_receipt_id IS NOT NULL AND causation_id = causation_receipt_id) OR (causation_kind != 'command' AND causation_receipt_id IS NULL)),
    FOREIGN KEY (command_actor_id, command_id) REFERENCES command_receipts(actor_id, command_id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (causation_receipt_id) REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE INDEX IF NOT EXISTS idx_events_stream_seq ON events(stream_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_aggregate ON events(aggregate_type, aggregate_id, aggregate_revision);
CREATE INDEX IF NOT EXISTS idx_events_recorded ON events(recorded_at_ms);

CREATE TRIGGER IF NOT EXISTS trg_events_no_physical_delete
BEFORE DELETE ON events
BEGIN
    SELECT RAISE(ABORT, 'events rows must never be physically deleted');
END;

CREATE TABLE IF NOT EXISTS outbox (
    event_id TEXT NOT NULL REFERENCES events(event_id),
    destination TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'leased', 'acked', 'dead_letter')),
    delivery_attempt INTEGER NOT NULL DEFAULT 0 CHECK (delivery_attempt >= 0),
    next_attempt_at_ms INTEGER NOT NULL,
    authority_generation TEXT,
    lease_epoch INTEGER,
    consumer_version TEXT,
    delivery_key TEXT NOT NULL UNIQUE,
    acked_at_ms INTEGER,
    last_error TEXT,
    rearm_count INTEGER NOT NULL DEFAULT 0 CHECK (rearm_count IN (0, 1)),
    PRIMARY KEY (event_id, destination)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_outbox_state_next ON outbox(state, next_attempt_at_ms);

CREATE TABLE IF NOT EXISTS inbox (
    consumer_id TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES events(event_id),
    processed_at_ms INTEGER NOT NULL,
    result_hash TEXT NOT NULL,
    PRIMARY KEY (consumer_id, event_id)
) STRICT;

CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    weight INTEGER NOT NULL DEFAULT 1 CHECK (weight BETWEEN 1 AND 10),
    state TEXT NOT NULL DEFAULT 'active',
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS principals (
    principal_id TEXT PRIMARY KEY,
    principal_kind TEXT NOT NULL CHECK (principal_kind IN ('user', 'service', 'agent', 'system')),
    actor_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL DEFAULT 'active',
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS agents (
    agent_id TEXT PRIMARY KEY,
    level TEXT NOT NULL CHECK (level IN ('N', 'N+1', 'N+2')),
    organization_parent_id TEXT REFERENCES agents(agent_id),
    created_by_agent_id TEXT REFERENCES agents(agent_id),
    project_id TEXT REFERENCES projects(project_id),
    state TEXT NOT NULL CHECK (state IN ('dormant', 'eligible', 'active', 'blocked', 'paused', 'retired')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TRIGGER IF NOT EXISTS trg_agents_immutable_creator
BEFORE UPDATE ON agents
FOR EACH ROW
WHEN NEW.created_by_agent_id IS NOT OLD.created_by_agent_id
BEGIN
    SELECT RAISE(ABORT, 'agents.created_by_agent_id is immutable');
END;

CREATE TABLE IF NOT EXISTS worktrees (
    worktree_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    repo_id TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS goals (
    goal_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('user', 'agent')),
    owner_user_id TEXT REFERENCES principals(principal_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    owner_epoch INTEGER NOT NULL DEFAULT 0 CHECK (owner_epoch >= 0),
    completion_policy TEXT NOT NULL CHECK (completion_policy IN ('mechanical', 'human', 'owner_agent')),
    state TEXT NOT NULL CHECK (state IN ('draft', 'active', 'blocked', 'achieved', 'cancelled')),
    definition_ref TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    CHECK ((owner_kind = 'user' AND owner_user_id IS NOT NULL AND owner_agent_id IS NULL)
        OR (owner_kind = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS missions (
    mission_id TEXT PRIMARY KEY,
    goal_id TEXT REFERENCES goals(goal_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('user', 'agent')),
    owner_user_id TEXT REFERENCES principals(principal_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    completion_policy TEXT NOT NULL CHECK (completion_policy IN ('mechanical', 'human', 'owner_agent')),
    leader_agent_id TEXT REFERENCES agents(agent_id),
    worktree_id TEXT REFERENCES worktrees(worktree_id),
    state TEXT NOT NULL CHECK (state IN ('proposed', 'ready', 'active', 'paused', 'blocked', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((owner_kind = 'user' AND owner_user_id IS NOT NULL AND owner_agent_id IS NULL)
        OR (owner_kind = 'agent' AND owner_agent_id IS NOT NULL AND owner_user_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS object_records (
    object_hash TEXT PRIMARY KEY,
    object_generation INTEGER NOT NULL CHECK (object_generation >= 1),
    relative_path TEXT NOT NULL UNIQUE,
    byte_length INTEGER NOT NULL CHECK (byte_length >= 0),
    media_type TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'live', 'deleting', 'tombstone', 'quarantined')),
    delete_operation_id TEXT,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS object_references (
    reference_id TEXT PRIMARY KEY,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    source_field TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('live', 'tombstone')),
    tombstone_reason TEXT,
    UNIQUE (owner_kind, owner_id, source_field, object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS object_pins (
    pin_id TEXT PRIMARY KEY,
    pin_kind TEXT NOT NULL CHECK (pin_kind IN ('preparation', 'read', 'backup', 'backup_barrier')),
    object_hash TEXT REFERENCES object_records(object_hash),
    object_generation INTEGER CHECK (object_generation IS NULL OR object_generation >= 1),
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    authority_generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held', 'reconciling', 'released')),
    created_at_ms INTEGER NOT NULL,
    released_at_ms INTEGER,
    CHECK ((pin_kind = 'backup_barrier' AND object_hash IS NULL AND object_generation IS NULL)
        OR (pin_kind != 'backup_barrier' AND object_hash IS NOT NULL AND object_generation IS NOT NULL))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_object_pins_active ON object_pins(state, pin_kind, object_hash);

CREATE TABLE IF NOT EXISTS object_deletions (
    delete_operation_id TEXT PRIMARY KEY,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    object_generation INTEGER NOT NULL CHECK (object_generation >= 1),
    relative_path TEXT NOT NULL,
    reason TEXT NOT NULL,
    decision_receipt_id TEXT REFERENCES command_receipts(receipt_id),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'unlinked', 'completed')),
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE (object_hash, object_generation)
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_versions (
    workflow_version_id TEXT PRIMARY KEY,
    registry_version_id TEXT,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    visibility TEXT NOT NULL DEFAULT 'published' CHECK (visibility IN ('published', 'qualification_only', 'deprecated', 'revoked')),
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_runs (
    run_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    workflow_version_id TEXT NOT NULL REFERENCES workflow_versions(workflow_version_id),
    purpose TEXT NOT NULL DEFAULT 'operational' CHECK (purpose IN ('operational', 'evaluation')),
    candidate_ref TEXT,
    predecessor_run_id TEXT REFERENCES workflow_runs(run_id),
    selection_reason TEXT NOT NULL,
    selected_via_alias TEXT,
    effective_bundle_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared', 'running', 'paused', 'blocked', 'cancelling', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    CHECK (ended_at_ms IS NULL OR ended_at_ms >= started_at_ms)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_runs_one_successor
ON workflow_runs(predecessor_run_id) WHERE predecessor_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS workflow_iterations (
    iteration_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    loop_node_id TEXT NOT NULL,
    iteration_vector TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (run_id, loop_node_id, iteration_vector)
) STRICT;

CREATE TABLE IF NOT EXISTS step_instances (
    step_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    iteration_id TEXT REFERENCES workflow_iterations(iteration_id),
    definition_node_id TEXT NOT NULL,
    iteration_vector TEXT NOT NULL,
    step_attempt INTEGER NOT NULL CHECK (step_attempt >= 1),
    state TEXT NOT NULL CHECK (state IN ('ready', 'queued', 'running', 'waiting_evidence', 'awaiting_decision', 'blocked', 'succeeded', 'failed', 'cancelled')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (run_id, definition_node_id, iteration_vector, step_attempt)
) STRICT;

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    contract_version_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    head_manifest_id TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    state TEXT NOT NULL CHECK (state IN ('open', 'closed', 'retired')),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_one_open_per_agent
ON sessions(agent_id) WHERE state = 'open';

CREATE TABLE IF NOT EXISTS session_entries (
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    entry_seq INTEGER NOT NULL CHECK (entry_seq >= 1),
    entry_kind TEXT NOT NULL,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    causation_event_id TEXT REFERENCES events(event_id),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, entry_seq)
) STRICT;

CREATE TABLE IF NOT EXISTS context_manifests (
    manifest_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    parent_manifest_id TEXT REFERENCES context_manifests(manifest_id),
    base_session_revision INTEGER NOT NULL CHECK (base_session_revision >= 0),
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS effective_bundles (
    bundle_hash TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    role_version_id TEXT NOT NULL,
    workflow_version_id TEXT REFERENCES workflow_versions(workflow_version_id),
    requested_model_profile_version_id TEXT NOT NULL,
    allowed_model_policy_hash TEXT NOT NULL,
    initial_context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    content_object_hash TEXT NOT NULL REFERENCES object_records(object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS invocations (
    invocation_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    session_id TEXT NOT NULL REFERENCES sessions(session_id),
    parent_invocation_id TEXT REFERENCES invocations(invocation_id),
    trigger_event_id TEXT NOT NULL REFERENCES events(event_id),
    task_id TEXT REFERENCES tasks(task_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    effective_bundle_hash TEXT NOT NULL REFERENCES effective_bundles(bundle_hash),
    context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'checkpointing', 'waiting_durable', 'reconciling', 'completed', 'failed', 'cancelled')),
    wait_reason TEXT CHECK (wait_reason IS NULL OR wait_reason IN ('waiting_owner', 'waiting_children', 'authentication', 'document_event', 'budget', 'timer')),
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class IN ('control', 'work')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    terminal_at_ms INTEGER,
    CHECK (wait_reason IS NULL OR state IN ('queued', 'waiting_durable')),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'checkpointing', 'reconciling') OR admission_class IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invocations_one_active_per_agent
ON invocations(agent_id) WHERE state NOT IN ('completed', 'failed', 'cancelled');

CREATE TABLE IF NOT EXISTS resource_fences (
    authority_generation TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    last_fence INTEGER NOT NULL CHECK (last_fence >= 0),
    PRIMARY KEY (authority_generation, resource_kind, resource_key)
) STRICT;

CREATE TABLE IF NOT EXISTS resource_leases (
    lease_id TEXT PRIMARY KEY,
    authority_generation TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    boot_identity TEXT NOT NULL,
    ttl_ms INTEGER NOT NULL CHECK (ttl_ms > 0),
    last_heartbeat_seq INTEGER NOT NULL CHECK (last_heartbeat_seq >= 0),
    expires_at_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held', 'suspect', 'reconciling', 'released', 'revoked')),
    FOREIGN KEY (authority_generation, resource_kind, resource_key)
        REFERENCES resource_fences(authority_generation, resource_kind, resource_key)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_resource_leases_one_active
ON resource_leases(authority_generation, resource_kind, resource_key)
WHERE state IN ('held', 'suspect', 'reconciling');

CREATE TABLE IF NOT EXISTS capacity_pools (
    pool_id TEXT PRIMARY KEY,
    pool_kind TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    unit TEXT NOT NULL,
    requested_limit INTEGER NOT NULL CHECK (requested_limit >= 0),
    effective_limit INTEGER NOT NULL CHECK (effective_limit >= 0),
    admissions_frozen INTEGER NOT NULL CHECK (admissions_frozen IN (0, 1)),
    hard_limit INTEGER NOT NULL CHECK (hard_limit >= 1),
    control_reserve_mode TEXT CHECK (control_reserve_mode IS NULL OR control_reserve_mode IN ('soft', 'hard')),
    control_reserved_units INTEGER,
    control_burst_limit INTEGER,
    checkpoint_escalation_s INTEGER,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (pool_kind, scope_key, unit),
    CHECK (requested_limit <= hard_limit AND effective_limit <= hard_limit),
    CHECK ((pool_kind = 'execution' AND hard_limit = 50 AND requested_limit BETWEEN 1 AND 50 AND effective_limit BETWEEN 1 AND 50 AND control_reserve_mode IS NOT NULL AND control_reserved_units IS NOT NULL AND control_burst_limit >= 1 AND checkpoint_escalation_s BETWEEN 10 AND 600)
        OR (pool_kind != 'execution' AND control_reserve_mode IS NULL AND control_reserved_units IS NULL AND control_burst_limit IS NULL AND checkpoint_escalation_s IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS capacity_reservations (
    reservation_id TEXT PRIMARY KEY,
    pool_id TEXT NOT NULL REFERENCES capacity_pools(pool_id),
    owner_type TEXT NOT NULL CHECK (owner_type IN ('agent_invocation', 'replay_worker', 'provider_probe', 'run', 'process', 'terminal', 'browser_context')),
    owner_id TEXT NOT NULL,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class IN ('control', 'work')),
    amount INTEGER NOT NULL CHECK (amount > 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'reconciling', 'released')),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    created_at_ms INTEGER NOT NULL,
    released_at_ms INTEGER
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_capacity_reservations_active_owner
ON capacity_reservations(pool_id, owner_type, owner_id)
WHERE state IN ('held', 'reconciling');

CREATE TABLE IF NOT EXISTS budget_accounts (
    account_id TEXT PRIMARY KEY,
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global', 'project', 'provider')),
    scope_id TEXT NOT NULL,
    currency TEXT NOT NULL,
    limit_micros INTEGER NOT NULL CHECK (limit_micros >= 0),
    spent_micros INTEGER NOT NULL CHECK (spent_micros >= 0),
    reserved_micros INTEGER NOT NULL CHECK (reserved_micros >= 0),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (scope_type, scope_id, currency)
) STRICT;

CREATE TABLE IF NOT EXISTS invocation_attempts (
    attempt_id TEXT PRIMARY KEY,
    invocation_id TEXT NOT NULL REFERENCES invocations(invocation_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    process_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('admitted', 'running', 'checkpointing', 'reconciling', 'ended', 'failed', 'fenced')),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    error_code TEXT,
    UNIQUE (invocation_id, ordinal)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invocation_attempts_one_active
ON invocation_attempts(invocation_id)
WHERE state IN ('admitted', 'running', 'checkpointing', 'reconciling');

CREATE TABLE IF NOT EXISTS actions (
    action_id TEXT PRIMARY KEY,
    origin_kind TEXT NOT NULL CHECK (origin_kind IN ('invocation', 'replay', 'control_plane')),
    control_principal_id TEXT REFERENCES principals(principal_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    attempt_id TEXT REFERENCES invocation_attempts(attempt_id),
    replay_worker_id TEXT,
    action_type TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    authorization_ref TEXT,
    policy_revision INTEGER NOT NULL CHECK (policy_revision >= 0),
    resource_lease_id TEXT REFERENCES resource_leases(lease_id),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'authorizing', 'approval_required', 'authorized', 'denied', 'dispatching', 'in_flight', 'observed_success', 'observed_failure', 'outcome_unknown', 'cancel_requested', 'cancelled')),
    reconcile_strategy TEXT NOT NULL CHECK (reconcile_strategy IN ('read_only_retryable', 'local_compare_and_set', 'provider_idempotency', 'observe_then_decide', 'manual_reconciliation')),
    provider_idempotency_key TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL,
    CHECK ((origin_kind = 'invocation' AND invocation_id IS NOT NULL AND attempt_id IS NOT NULL AND replay_worker_id IS NULL AND control_principal_id IS NULL)
        OR (origin_kind = 'replay' AND replay_worker_id IS NOT NULL AND run_id IS NOT NULL AND step_id IS NOT NULL AND invocation_id IS NULL AND attempt_id IS NULL AND control_principal_id IS NULL)
        OR (origin_kind = 'control_plane' AND control_principal_id IS NOT NULL AND invocation_id IS NULL AND attempt_id IS NULL AND replay_worker_id IS NULL)),
    CHECK (state NOT IN ('prepared', 'authorizing', 'approval_required') OR authorization_ref IS NULL)
) STRICT;

CREATE TABLE IF NOT EXISTS action_dispatches (
    dispatch_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    adapter_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    provider_request_id TEXT,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    transport_state TEXT NOT NULL,
    receipt_hash TEXT,
    UNIQUE (action_id, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS budget_reservations (
    reservation_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES budget_accounts(account_id),
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    reserved_micros INTEGER NOT NULL CHECK (reserved_micros >= 0),
    settled_micros INTEGER CHECK (settled_micros IS NULL OR settled_micros >= 0),
    state TEXT NOT NULL CHECK (state IN ('held', 'unknown', 'settled', 'released')),
    pricing_version TEXT NOT NULL,
    max_request_hash TEXT NOT NULL,
    UNIQUE (account_id, action_id)
) STRICT;

CREATE TABLE IF NOT EXISTS processes (
    process_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id),
    process_run_uuid TEXT NOT NULL UNIQUE,
    authority_generation TEXT NOT NULL,
    host_id TEXT,
    pid INTEGER CHECK (pid IS NULL OR pid > 0),
    pgid INTEGER CHECK (pgid IS NULL OR pgid > 0),
    boot_identity TEXT,
    start_identity TEXT,
    identity_backend TEXT NOT NULL,
    owner_uid INTEGER NOT NULL CHECK (owner_uid >= 0),
    cwd_ref TEXT NOT NULL,
    worktree_id TEXT REFERENCES worktrees(worktree_id),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'live', 'exited', 'lost', 'suspect', 'identity_mismatch', 'unknown')),
    last_observed_at_ms INTEGER NOT NULL,
    exit_code INTEGER,
    exit_signal INTEGER
) STRICT;

CREATE TABLE IF NOT EXISTS terminal_sessions (
    terminal_id TEXT PRIMARY KEY,
    process_id TEXT NOT NULL REFERENCES processes(process_id),
    owner_agent_id TEXT REFERENCES agents(agent_id),
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    purpose_key TEXT NOT NULL,
    rows INTEGER NOT NULL CHECK (rows BETWEEN 1 AND 500),
    cols INTEGER NOT NULL CHECK (cols BETWEEN 1 AND 1000),
    lease_id TEXT NOT NULL REFERENCES resource_leases(lease_id),
    input_seq INTEGER NOT NULL CHECK (input_seq >= 0),
    durable_log_seq INTEGER NOT NULL CHECK (durable_log_seq >= 0),
    state TEXT NOT NULL CHECK (state IN ('preparing', 'ready', 'busy', 'detached', 'stopping', 'exited', 'suspect', 'unavailable'))
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_terminal_sessions_reusable
ON terminal_sessions(mission_id, purpose_key)
WHERE state IN ('preparing', 'ready', 'busy', 'detached');

CREATE TABLE IF NOT EXISTS backup_jobs (
    backup_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'copying', 'verifying', 'complete', 'aborted')),
    source_authority_generation TEXT NOT NULL,
    snapshot_stream_id TEXT,
    snapshot_sequence INTEGER,
    manifest_object_hash TEXT REFERENCES object_records(object_hash),
    coverage TEXT NOT NULL,
    destination_path TEXT,
    retention_tier TEXT NOT NULL DEFAULT 'manual' CHECK (retention_tier IN ('daily', 'weekly', 'manual')),
    created_at_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS restore_requests (
    restore_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id),
    backup_id TEXT NOT NULL,
    source_authority_generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('requested', 'neutralizing', 'capsule_ready', 'blocked', 'handed_over', 'aborted')),
    blocked_reason TEXT,
    capsule_object_hash TEXT REFERENCES object_records(object_hash),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((state = 'blocked' AND blocked_reason IS NOT NULL AND capsule_object_hash IS NULL) OR (state != 'blocked'))
) STRICT;

CREATE TABLE IF NOT EXISTS restore_imports (
    restore_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL,
    backup_id TEXT NOT NULL,
    source_authority_generation TEXT NOT NULL,
    target_authority_generation TEXT NOT NULL,
    capsule_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    coverage TEXT NOT NULL CHECK (coverage IN ('complete', 'partial', 'unknown')),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'activated')),
    frozen_scopes_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (target_authority_generation != source_authority_generation)
) STRICT;

CREATE TABLE IF NOT EXISTS recovery_scope_freezes (
    scope_id TEXT PRIMARY KEY,
    restore_id TEXT NOT NULL REFERENCES restore_imports(restore_id),
    reason TEXT NOT NULL,
    global_freeze INTEGER NOT NULL CHECK (global_freeze IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN ('frozen', 'disposed')),
    created_at_ms INTEGER NOT NULL,
    disposed_at_ms INTEGER,
    disposition_receipt_id TEXT REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS evidence (
    evidence_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('observation', 'claim', 'judgment')),
    action_id TEXT REFERENCES actions(action_id),
    run_id TEXT REFERENCES workflow_runs(run_id),
    snapshot_ref TEXT,
    object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    producer_id TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    supersedes_id TEXT REFERENCES evidence(evidence_id),
    availability TEXT NOT NULL CHECK (availability IN ('available', 'purged', 'unavailable'))
) STRICT;

CREATE TRIGGER IF NOT EXISTS trg_evidence_no_claim_to_observation
BEFORE UPDATE ON evidence
FOR EACH ROW
WHEN OLD.kind = 'claim' AND NEW.kind = 'observation'
BEGIN
    SELECT RAISE(ABORT, 'cannot convert claim to observation by update');
END;
";

const V002_SQL: &str = r"
CREATE TABLE IF NOT EXISTS replay_workers (
    replay_worker_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT NOT NULL REFERENCES step_instances(step_id),
    attempt_ordinal INTEGER NOT NULL CHECK (attempt_ordinal >= 1),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    process_id TEXT REFERENCES processes(process_id),
    checkpoint_hash TEXT,
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'checkpointing', 'waiting_durable', 'reconciling', 'ended', 'failed', 'fenced')),
    wait_reason TEXT,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class = 'work'),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'checkpointing', 'reconciling') OR admission_class = 'work')
) STRICT;

CREATE TABLE IF NOT EXISTS provider_profiles (
    profile_version_ref TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    locality TEXT NOT NULL CHECK (locality IN ('local_only', 'approved_remote')),
    pricing_version TEXT NOT NULL,
    price_valid_until_ms INTEGER,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('ovpt_release', 'user_import', 'user_declared')),
    pricing_stale_policy TEXT NOT NULL DEFAULT 'block' CHECK (pricing_stale_policy IN ('block', 'warn')),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS provider_probes (
    probe_id TEXT PRIMARY KEY,
    command_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id),
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    source_run_id TEXT REFERENCES workflow_runs(run_id),
    profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
    context_hash TEXT NOT NULL,
    phase TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'admitted', 'running', 'reconciling', 'ended', 'failed', 'cancelled')),
    wait_reason TEXT,
    admission_class TEXT CHECK (admission_class IS NULL OR admission_class = 'work'),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (state NOT IN ('queued', 'admitted', 'running', 'reconciling') OR admission_class = 'work')
) STRICT;

CREATE TABLE IF NOT EXISTS execution_reservation_owners (
    reservation_id TEXT PRIMARY KEY REFERENCES capacity_reservations(reservation_id),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    attempt_id TEXT REFERENCES invocation_attempts(attempt_id),
    replay_worker_id TEXT REFERENCES replay_workers(replay_worker_id),
    provider_probe_id TEXT REFERENCES provider_probes(probe_id),
    CHECK (((invocation_id IS NOT NULL) + (replay_worker_id IS NOT NULL) + (provider_probe_id IS NOT NULL)) = 1),
    CHECK (attempt_id IS NULL OR invocation_id IS NOT NULL)
) STRICT;

CREATE TABLE IF NOT EXISTS model_calls (
    model_call_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL UNIQUE REFERENCES actions(action_id),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('invocation', 'provider_probe')),
    invocation_id TEXT REFERENCES invocations(invocation_id),
    provider_probe_id TEXT REFERENCES provider_probes(probe_id),
    parent_model_call_id TEXT REFERENCES model_calls(model_call_id),
    trigger_event_id TEXT NOT NULL REFERENCES events(event_id),
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK ((owner_kind = 'invocation' AND invocation_id IS NOT NULL AND provider_probe_id IS NULL)
        OR (owner_kind = 'provider_probe' AND provider_probe_id IS NOT NULL AND invocation_id IS NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS model_call_manifests (
    manifest_id TEXT PRIMARY KEY,
    model_call_id TEXT NOT NULL REFERENCES model_calls(model_call_id),
    manifest_kind TEXT NOT NULL CHECK (manifest_kind IN ('request', 'result')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    parent_manifest_id TEXT REFERENCES model_call_manifests(manifest_id),
    context_manifest_id TEXT NOT NULL REFERENCES context_manifests(manifest_id),
    provider_id TEXT NOT NULL,
    resolved_model_id TEXT NOT NULL,
    capabilities_hash TEXT NOT NULL,
    allowed_model_policy_hash TEXT NOT NULL,
    request_payload_hash TEXT NOT NULL,
    usage_object_hash TEXT,
    result_object_hash TEXT,
    content_object_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (model_call_id, manifest_kind, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS workflow_timers (
    timer_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    step_id TEXT REFERENCES step_instances(step_id),
    timer_kind TEXT NOT NULL,
    due_at_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('armed', 'fired', 'cancelled')),
    fired_event_id TEXT REFERENCES events(event_id),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE TABLE IF NOT EXISTS mission_assignments (
    assignment_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(mission_id),
    agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    role_version_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    assignment_epoch INTEGER NOT NULL CHECK (assignment_epoch >= 0),
    state TEXT NOT NULL CHECK (state IN ('active', 'superseded', 'released', 'revoked')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mission_assignments_one_active
ON mission_assignments(mission_id, agent_id) WHERE state = 'active';

CREATE TABLE IF NOT EXISTS mailbox_items (
    mailbox_item_id TEXT PRIMARY KEY,
    recipient_agent_id TEXT NOT NULL REFERENCES agents(agent_id),
    event_id TEXT NOT NULL REFERENCES events(event_id),
    visibility TEXT NOT NULL,
    wake_rule TEXT NOT NULL,
    coalescing_key TEXT,
    state TEXT NOT NULL CHECK (state IN ('pending', 'delivered', 'processed')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    UNIQUE (recipient_agent_id, event_id)
) STRICT;

CREATE TABLE IF NOT EXISTS for_each_cursors (
    step_id TEXT PRIMARY KEY REFERENCES step_instances(step_id),
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    data_digest TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    next_index INTEGER NOT NULL CHECK (next_index >= 0),
    batch_start INTEGER NOT NULL CHECK (batch_start >= 0),
    phase TEXT NOT NULL CHECK (phase IN ('body', 'after_batch')),
    item_results_ref TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    CHECK (0 <= batch_start AND batch_start <= next_index AND next_index <= item_count)
) STRICT;

CREATE TABLE IF NOT EXISTS decision_chains (
    decision_chain_id TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    category TEXT NOT NULL,
    parent_chain_id TEXT REFERENCES decision_chains(decision_chain_id),
    policy_version TEXT NOT NULL,
    consecutive_turns INTEGER NOT NULL CHECK (consecutive_turns >= 0),
    cumulative_turns INTEGER NOT NULL CHECK (cumulative_turns >= 0),
    reserved_turns INTEGER NOT NULL CHECK (reserved_turns >= 0),
    last_progress_event_id TEXT REFERENCES events(event_id),
    state TEXT NOT NULL CHECK (state IN ('open', 'stalled', 'closed')),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_decision_chains_one_open
ON decision_chains(subject_kind, subject_id, category, coalesce(parent_chain_id, ''))
WHERE state IN ('open', 'stalled');

CREATE TABLE IF NOT EXISTS evaluation_reports (
    evaluation_report_id TEXT PRIMARY KEY,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    required_checks TEXT NOT NULL,
    judge_policy TEXT NOT NULL,
    result TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS discovery_receipts (
    discovery_receipt_id TEXT PRIMARY KEY,
    authority_generation TEXT NOT NULL,
    decision TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS registry_entries (
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('skill', 'role', 'contract', 'automation', 'technical_resource')),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('global', 'project', 'repository')),
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    PRIMARY KEY (entry_kind, scope_kind, scope_owner_key, entry_id),
    CHECK ((scope_kind = 'global' AND scope_owner_key = 'global') OR (scope_kind != 'global' AND length(scope_owner_key) > 0))
) STRICT;

CREATE TABLE IF NOT EXISTS registry_versions (
    registry_version_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    content_hash TEXT NOT NULL,
    publication_state TEXT NOT NULL CHECK (publication_state IN ('published', 'deprecated', 'revoked')),
    author_principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    discovery_receipt_id TEXT NOT NULL REFERENCES discovery_receipts(discovery_receipt_id),
    evaluation_report_id TEXT NOT NULL REFERENCES evaluation_reports(evaluation_report_id),
    published_at_ms INTEGER NOT NULL,
    UNIQUE (entry_kind, scope_kind, scope_owner_key, entry_id, version),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_drafts (
    draft_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    validation_report_id TEXT REFERENCES evaluation_reports(evaluation_report_id),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_aliases (
    alias_id TEXT PRIMARY KEY,
    entry_kind TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    scope_owner_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('version', 'draft')),
    target_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    FOREIGN KEY (entry_kind, scope_kind, scope_owner_key, entry_id)
        REFERENCES registry_entries(entry_kind, scope_kind, scope_owner_key, entry_id)
) STRICT;

CREATE TABLE IF NOT EXISTS registry_bindings (
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    slot_id TEXT NOT NULL,
    bound_versions_hash TEXT NOT NULL,
    bound_event_id TEXT NOT NULL REFERENCES events(event_id),
    state TEXT NOT NULL,
    PRIMARY KEY (run_id, slot_id)
) STRICT;

CREATE TABLE IF NOT EXISTS source_bindings (
    source_binding_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    kind TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS automation_versions (
    automation_version_id TEXT PRIMARY KEY,
    registry_version_id TEXT NOT NULL REFERENCES registry_versions(registry_version_id),
    parameter_schema_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash)
) STRICT;

CREATE TABLE IF NOT EXISTS automation_version_modes (
    automation_version_id TEXT NOT NULL REFERENCES automation_versions(automation_version_id),
    automation_mode TEXT NOT NULL CHECK (automation_mode IN ('replay', 'supervised')),
    workflow_version_id TEXT NOT NULL REFERENCES workflow_versions(workflow_version_id),
    model_policy TEXT NOT NULL CHECK (model_policy IN ('forbidden', 'allowed_bounded')),
    driver_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    contract_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    PRIMARY KEY (automation_version_id, automation_mode),
    CHECK ((automation_mode = 'replay' AND model_policy = 'forbidden')
        OR (automation_mode = 'supervised' AND model_policy = 'allowed_bounded'))
) STRICT;

CREATE TABLE IF NOT EXISTS automation_data_revisions (
    data_revision_id TEXT PRIMARY KEY,
    source_binding_id TEXT NOT NULL REFERENCES source_bindings(source_binding_id),
    schema_hash TEXT NOT NULL,
    parser_profile_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    row_count INTEGER NOT NULL CHECK (row_count >= 0),
    validation_receipt_id TEXT NOT NULL REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS automation_runs (
    run_id TEXT PRIMARY KEY REFERENCES workflow_runs(run_id),
    automation_version_id TEXT NOT NULL,
    data_revision_id TEXT NOT NULL REFERENCES automation_data_revisions(data_revision_id),
    automation_mode TEXT NOT NULL CHECK (automation_mode IN ('replay', 'supervised')),
    model_policy TEXT NOT NULL CHECK (model_policy IN ('forbidden', 'allowed_bounded')),
    driver_manifest_hash TEXT NOT NULL REFERENCES object_records(object_hash),
    FOREIGN KEY (automation_version_id, automation_mode)
        REFERENCES automation_version_modes(automation_version_id, automation_mode),
    CHECK ((automation_mode = 'replay' AND model_policy = 'forbidden')
        OR (automation_mode = 'supervised' AND model_policy = 'allowed_bounded'))
) STRICT;

CREATE TABLE IF NOT EXISTS guided_setup_drafts (
    draft_id TEXT PRIMARY KEY,
    template_version_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    validation_receipt_id TEXT REFERENCES command_receipts(receipt_id),
    preview_hash TEXT,
    applied_receipt_id TEXT REFERENCES command_receipts(receipt_id)
) STRICT;

CREATE TABLE IF NOT EXISTS model_catalog_snapshots (
    catalog_id TEXT NOT NULL,
    catalog_version TEXT NOT NULL,
    digest TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('ovpt_release', 'user_import', 'user_declared')),
    published_at_ms INTEGER NOT NULL,
    valid_until_ms INTEGER NOT NULL,
    imported_at_ms INTEGER NOT NULL,
    content_ref TEXT NOT NULL,
    PRIMARY KEY (catalog_id, catalog_version)
) STRICT;
";
