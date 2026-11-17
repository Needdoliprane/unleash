//! Restauration offline conservatrice, neutralisation, manifeste privé `0600` et capsule delta (`REQ-PERSIST-010`).

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, AuthorityGeneration, BackupId, CommandId, DaemonEpoch, MicrosAmount, ReceiptId,
    RestoreId, Sequence, Sha256Digest, StreamId, TimestampMs,
};

use crate::backup::validate_backup_dir;
use crate::config::{StoreConfig, StoreCutpoint};
use crate::error::StoreError;
use crate::fs_sync::{set_dir_permissions_0700, set_path_permissions, write_atomic_durable};
use crate::objects::{object_relative_path, safe_artifact_path};
use crate::qualification::{apply_and_verify_writer_pragmas, inspect_and_verify_build};

/// État d'une demande de restauration dans `restore_requests` ou le manifeste privé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreRequestState {
    /// Demande enregistrée, daemon entré/maintenu en `read_only_recovery`.
    Requested,
    /// Inventaire et neutralisation des anciens exécuteurs en cours.
    Neutralizing,
    /// Capsule durable du delta préparée et vérifiée.
    CapsuleReady,
    /// Restauration bloquée (`RESTORATION_BLOCKED`) sans lignée cible ni capsule.
    Blocked,
    /// Bascule vers la base restaurée effectuée.
    HandedOver,
    /// Annulée.
    Aborted,
}

impl RestoreRequestState {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Neutralizing => "neutralizing",
            Self::CapsuleReady => "capsule_ready",
            Self::Blocked => "blocked",
            Self::HandedOver => "handed_over",
            Self::Aborted => "aborted",
        }
    }
}

/// Couverture du delta collecté dans la capsule (`restore_imports.coverage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapsuleCoverage {
    /// Delta complet collecté depuis la base source et les observations locales.
    Complete,
    /// Delta partiel affectant des scopes identifiés.
    Partial,
    /// Étendue inconnue (base source perdue/illisible) : gel global des effets.
    Unknown,
}

impl CapsuleCoverage {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
        }
    }
}

/// Entrée d'exécuteur ou host inventorié dans la lignée source avant bascule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorInventoryItem {
    /// Identifiant du host ou processus.
    pub executor_id: String,
    /// Lignée observée sur cet exécuteur.
    pub authority_generation: AuthorityGeneration,
    /// Époque observée.
    pub daemon_epoch: DaemonEpoch,
    /// Identité de boot associée.
    pub boot_identity: String,
    /// Vrai si cet exécuteur a été formellement arrêté et neutralisé.
    pub neutralized: bool,
}

/// Adaptateur vérifiant l'inventaire et la neutralisation des exécuteurs de l'ancienne lignée.
pub trait ExecutorNeutralizer: Send + Sync {
    /// Tente de neutraliser tous les exécuteurs vivants de l'ancienne lignée et renvoie
    /// l'inventaire résultant ou `Err` / des entrées non neutralisées imposant `RestorationBlocked`.
    ///
    /// # Errors
    /// Retourne [`crate::StoreError::RestorationBlocked`] si l'inventaire est incomplet ou un host refuse l'arrêt.
    fn inventory_and_neutralize(
        &self,
        source_authority_generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, crate::StoreError>;
}

/// Neutraliseur par défaut indiquant un inventaire local vide (aucun host vivant).
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyExecutorNeutralizer;

impl ExecutorNeutralizer for EmptyExecutorNeutralizer {
    fn inventory_and_neutralize(
        &self,
        _source_authority_generation: AuthorityGeneration,
    ) -> Result<Vec<ExecutorInventoryItem>, crate::StoreError> {
        Ok(Vec::new())
    }
}

/// Résumé d'une action ou effet postérieur au watermark du backup inclus dans la capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleActionDelta {
    /// Identifiant de l'action source.
    pub action_id: ActionId,
    /// Scope métier affecté.
    pub scope_id: String,
    /// État observé post-watermark (`observed_success`, `observed_failure`, `outcome_unknown`, etc.).
    pub state: String,
    /// Stratégie de réconciliation.
    pub reconcile_strategy: String,
    /// Coût connu observé en micro-unités (jamais remis à zéro par rollback).
    pub settled_micros: Option<MicrosAmount>,
    /// Réserve financière toujours inconnue/retenue.
    pub reserved_unknown_micros: Option<MicrosAmount>,
    /// Compte budgétaire affecté éventuel.
    pub account_id: Option<String>,
}

/// Consommation ou receipt post-watermark inclus dans la capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleReceiptDelta {
    /// Identifiant du reçu source.
    pub receipt_id: ReceiptId,
    /// Acteur source.
    pub actor_id: String,
    /// Commande source.
    pub command_id: CommandId,
    /// Scope touché.
    pub scope_id: String,
    /// Digest de commande.
    pub command_digest: Sha256Digest,
    /// État du reçu.
    pub state: String,
}

/// Réservation budgétaire post-watermark incluse dans la capsule durable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleBudgetDelta {
    /// Identifiant de réservation source.
    pub reservation_id: String,
    /// Compte budgétaire visé.
    pub account_id: String,
    /// Action source associée.
    pub action_id: String,
    /// Montant initialement réservé.
    pub reserved_micros: MicrosAmount,
    /// Montant réellement réglé si connu.
    pub settled_micros: Option<MicrosAmount>,
    /// État (`held`, `unknown`, `settled`, `released`).
    pub state: String,
    /// Version tarifaire associée.
    pub pricing_version: String,
}

/// Capsule durable du delta post-snapshot importée avant activation de la nouvelle lignée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreCapsule {
    /// Identifiant de la demande de restauration.
    pub restore_id: RestoreId,
    /// Reçu de la commande `RestoreBackup` conservé à travers la bascule.
    pub command_receipt_id: ReceiptId,
    /// Sauvegarde restaurée.
    pub backup_id: BackupId,
    /// Lignée source.
    pub source_authority_generation: AuthorityGeneration,
    /// Flux du snapshot restauré.
    pub snapshot_stream_id: StreamId,
    /// Séquence high-water mark du snapshot restauré.
    pub snapshot_sequence: Sequence,
    /// Couverture du delta.
    pub coverage: CapsuleCoverage,
    /// Scopes touchés par des actions/dispatchs/receipts/consommations inbox post-snapshot.
    pub affected_scopes: Vec<String>,
    /// Actions post-watermark (y compris déjà terminales ou nées d'un consommateur outbox).
    pub actions: Vec<CapsuleActionDelta>,
    /// Réservations budgétaires post-watermark ou non résolues (`invocation`/`replay`/`control_plane`).
    pub budget_reservations: Vec<CapsuleBudgetDelta>,
    /// Receipts post-watermark.
    pub receipts: Vec<CapsuleReceiptDelta>,
    /// Consommations inbox post-watermark (`(consumer_id, event_id, scope_id)`).
    pub inbox_consumptions: Vec<(String, String, String)>,
    /// Horodatage de scellement de la capsule.
    pub sealed_at_ms: TimestampMs,
}

/// Manifeste privé de maintenance hors base (`0600`) utilisé quand la base source est illisible/corrompue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceRestoreManifest {
    /// Identifiant de restauration.
    pub restore_id: RestoreId,
    /// Reçu attribué à `RestoreBackup`.
    pub command_receipt_id: ReceiptId,
    /// Acteur et commande pour recherche idempotente tant que la base source est hors-service.
    pub actor_id: String,
    /// Identifiant de commande client.
    pub command_id: CommandId,
    /// Digest canonique de la commande `RestoreBackup`.
    pub command_digest: Sha256Digest,
    /// Sauvegarde visée.
    pub backup_id: BackupId,
    /// Lignée source.
    pub source_authority_generation: AuthorityGeneration,
    /// État courant persisté.
    pub state: RestoreRequestState,
    /// Raison de blocage éventuelle (`RESTORATION_BLOCKED`).
    pub blocked_reason: Option<String>,
    /// Horodatage de mise à jour.
    pub updated_at_ms: TimestampMs,
}

/// Demande d'exécution du protocole de restauration offline conservateur (`RestoreBackup`).
#[derive(Clone)]
pub struct RestoreBackupRequest {
    /// Répertoire de la sauvegarde validée à restaurer.
    pub backup_dir: PathBuf,
    /// Répertoire cible de la nouvelle base restaurée (distinct du backup).
    pub target_data_root: PathBuf,
    /// Identifiant de la commande `RestoreBackup` (pour le receipt / manifeste privé).
    pub command_id: CommandId,
    /// Acteur demandant la restauration.
    pub actor_id: String,
    /// Digest de la commande.
    pub command_digest: Sha256Digest,
    /// Lignée portée par la commande `RestoreBackup` (vérifiée contre source).
    pub source_authority_generation: AuthorityGeneration,
    /// Indique si la base source est corrompue/illisible (déclenche le manifeste privé `0600` et couverture `Unknown` si delta perdu).
    pub source_db_unreadable: bool,
    /// Chemin explicite de la base source si intacte (pour collecter le delta et créer la sauvegarde pré-restore obligatoire).
    pub source_db_path: Option<PathBuf>,
    /// Répertoire d'artefacts source.
    pub source_artifacts_dir: Option<PathBuf>,
    /// Neutraliseur des exécuteurs de la lignée source.
    pub neutralizer: std::sync::Arc<dyn ExecutorNeutralizer>,
}

/// Résultat d'une restauration activée (`RestoreBackup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreActivationResult {
    /// Identifiant de restauration importé dans `restore_imports`.
    pub restore_id: RestoreId,
    /// Reçu conservé de `RestoreBackup`.
    pub command_receipt_id: ReceiptId,
    /// Ancienne lignée neutralisée.
    pub source_authority_generation: AuthorityGeneration,
    /// Nouvelle lignée générée (jamais ordonnée par rapport à l'ancienne).
    pub target_authority_generation: AuthorityGeneration,
    /// Nouveau flux d'événements généré.
    pub target_stream_id: StreamId,
    /// Époque initiale de la nouvelle lignée (`1`).
    pub target_daemon_epoch: DaemonEpoch,
    /// Couverture importée.
    pub coverage: CapsuleCoverage,
    /// Vrai si un gel global des effets a été activé (`coverage == Unknown`).
    pub global_freeze_active: bool,
    /// Scopes individuellement gelés dans `recovery_scope_freezes`.
    pub frozen_scopes: Vec<String>,
    /// Hash de l'objet capsule durablement épinglé.
    pub capsule_object_hash: Sha256Digest,
}

fn write_maintenance_manifest(
    maintenance_dir: &Path,
    manifest: &MaintenanceRestoreManifest,
) -> Result<(), StoreError> {
    fs::create_dir_all(maintenance_dir)?;
    set_dir_permissions_0700(maintenance_dir)?;
    let path = maintenance_dir.join("restore-manifest.json");
    let bytes = serde_json::to_vec_pretty(manifest)?;
    write_atomic_durable(&path, &bytes, 0o600)?;
    Ok(())
}

/// Lit le manifeste privé de maintenance (`0600`) lorsqu'il existe (`InspectRecovery`).
pub(crate) fn read_maintenance_manifest(
    maintenance_dir: &Path,
) -> Result<Option<MaintenanceRestoreManifest>, StoreError> {
    let path = maintenance_dir.join("restore-manifest.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)?;
    let parsed = serde_json::from_slice(&bytes)?;
    Ok(Some(parsed))
}

#[derive(Debug, Clone)]
struct SourceBudgetRow {
    account_id: String,
    scope_type: String,
    scope_id: String,
    currency: String,
    limit_micros: i64,
    spent_micros: i64,
    reserved_micros: i64,
    revision: i64,
}

#[derive(Debug, Clone)]
struct SourceActionRow {
    action_id: String,
    origin_kind: String,
    control_principal_id: Option<String>,
    run_id: Option<String>,
    step_id: Option<String>,
    invocation_id: Option<String>,
    attempt_id: Option<String>,
    replay_worker_id: Option<String>,
    action_type: String,
    payload_hash: String,
    authorization_ref: Option<String>,
    policy_revision: i64,
    resource_lease_id: Option<String>,
    authority_generation: String,
    daemon_epoch: i64,
    fence: i64,
    state: String,
    reconcile_strategy: String,
    provider_idempotency_key: Option<String>,
    revision: i64,
    created_at_ms: i64,
}

#[derive(Debug, Clone)]
struct SourceReservationRow {
    reservation_id: String,
    account_id: String,
    action_id: String,
    reserved_micros: i64,
    settled_micros: Option<i64>,
    state: String,
    pricing_version: String,
    max_request_hash: String,
}

#[derive(Debug, Clone)]
struct SourceInboxRow {
    consumer_id: String,
    event_id: String,
    scope_id: String,
    processed_at_ms: i64,
    result_hash: String,
}

#[derive(Debug, Default)]
struct CollectedSourceDelta {
    actions: Vec<SourceActionRow>,
    budget_accounts: Vec<SourceBudgetRow>,
    budget_reservations: Vec<SourceReservationRow>,
    capsule_actions: Vec<CapsuleActionDelta>,
    capsule_budget_reservations: Vec<CapsuleBudgetDelta>,
    capsule_receipts: Vec<CapsuleReceiptDelta>,
    inbox_consumptions: Vec<SourceInboxRow>,
    principals: Vec<(String, String, String, String, i64)>,
    affected_scopes: BTreeSet<String>,
}

fn ensure_source_restore_tracking(
    src_db: &Path,
    req: &RestoreBackupRequest,
    backup_id: BackupId,
    restore_id: RestoreId,
    receipt_id: ReceiptId,
    now_ms: TimestampMs,
) -> Result<(), StoreError> {
    let mut conn = Connection::open_with_flags(
        src_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;
    conn.busy_timeout(std::time::Duration::from_millis(5_000))
        .map_err(StoreError::from_rusqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::from_rusqlite)?;

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreError::from_rusqlite)?;

    let (src_auth_str, src_rev): (String, i64) = tx
        .query_row(
            "SELECT authority_generation, revision FROM daemon_state WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(StoreError::from_rusqlite)?;
    let src_auth: AuthorityGeneration = src_auth_str.parse()?;
    if src_auth != req.source_authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: src_auth,
            actual: req.source_authority_generation,
            command_id: Some(req.command_id),
        });
    }

    let existing_rcpt: Option<String> = tx
        .query_row(
            "SELECT receipt_id FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![req.actor_id, req.command_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let effective_rcpt = if let Some(r) = existing_rcpt {
        r
    } else {
        let r_str = receipt_id.to_string();
        tx.execute(
            "INSERT INTO command_receipts
             (actor_id, command_id, authority_generation, receipt_id, request_id, command_type,
              command_digest, aggregate_id, state, response_state, response_json,
              response_retained_until_ms, first_event_seq, last_event_seq, committed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'RestoreBackup', ?6, ?7, 'accepted', 'full', '{}', ?8, NULL, NULL, ?9)",
            rusqlite::params![
                req.actor_id,
                req.command_id.to_string(),
                src_auth_str,
                r_str,
                restore_id.to_string(),
                req.command_digest.as_prefixed_str(),
                restore_id.to_string(),
                now_ms.as_i64().saturating_add(30 * 86_400_000),
                now_ms.as_i64()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
        r_str
    };

    tx.execute(
        "UPDATE daemon_state SET mode = 'read_only_recovery', revision = ?1 WHERE singleton = 1",
        [src_rev.saturating_add(1)],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.execute(
        "INSERT INTO restore_requests
         (restore_id, command_receipt_id, backup_id, source_authority_generation, state, blocked_reason, capsule_object_hash, revision)
         VALUES (?1, ?2, ?3, ?4, 'neutralizing', NULL, NULL, 0)",
        rusqlite::params![
            restore_id.to_string(),
            effective_rcpt,
            backup_id.to_string(),
            src_auth_str
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.commit().map_err(StoreError::from_rusqlite)?;
    Ok(())
}

fn mark_source_restore_blocked(src_db: &Path, restore_id: RestoreId, reason: &str) {
    if let Ok(mut conn) = Connection::open_with_flags(
        src_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5_000));
        if let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) {
            let _ = tx.execute(
                "UPDATE restore_requests
                 SET state = 'blocked', blocked_reason = ?1, capsule_object_hash = NULL, revision = revision + 1
                 WHERE restore_id = ?2",
                rusqlite::params![reason, restore_id.to_string()],
            );
            let _ = tx.commit();
        }
    }
}

fn mark_source_restore_handed_over(src_db: &Path, restore_id: RestoreId) {
    if let Ok(mut conn) = Connection::open_with_flags(
        src_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5_000));
        if let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) {
            let _ = tx.execute(
                "UPDATE restore_requests
                 SET state = 'handed_over', blocked_reason = NULL, revision = revision + 1
                 WHERE restore_id = ?1",
                [restore_id.to_string()],
            );
            let _ = tx.commit();
        }
    }
}

struct SnapshotBaseline {
    outbox_scopes: BTreeSet<String>,
    actions: HashSet<String>,
    receipts: HashSet<(String, String)>,
    inbox: HashSet<(String, String)>,
}

fn collect_snapshot_outbox_scopes(snapshot_db: &Path) -> Result<SnapshotBaseline, StoreError> {
    let conn = Connection::open_with_flags(
        snapshot_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;

    let mut outbox_scopes = BTreeSet::new();
    let mut s_out = conn
        .prepare(
            "SELECT e.project_id, e.aggregate_id
             FROM outbox o
             JOIN events e ON e.event_id = o.event_id
             WHERE o.state IN ('pending', 'leased', 'dead_letter')",
        )
        .map_err(StoreError::from_rusqlite)?;
    let out_rows = s_out
        .query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(StoreError::from_rusqlite)?;
    for row in out_rows {
        let (proj, agg) = row.map_err(StoreError::from_rusqlite)?;
        if let Some(p) = proj {
            outbox_scopes.insert(p);
        } else {
            outbox_scopes.insert(agg);
        }
    }

    let mut snap_actions = HashSet::new();
    let mut s_act = conn
        .prepare("SELECT action_id FROM actions")
        .map_err(StoreError::from_rusqlite)?;
    let act_rows = s_act
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(StoreError::from_rusqlite)?;
    for r in act_rows {
        snap_actions.insert(r.map_err(StoreError::from_rusqlite)?);
    }

    let mut snap_rcpts = HashSet::new();
    let mut s_rcpt = conn
        .prepare("SELECT actor_id, command_id FROM command_receipts")
        .map_err(StoreError::from_rusqlite)?;
    let rcpt_rows = s_rcpt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(StoreError::from_rusqlite)?;
    for r in rcpt_rows {
        snap_rcpts.insert(r.map_err(StoreError::from_rusqlite)?);
    }

    let mut snap_inbox = HashSet::new();
    let mut s_inbox = conn
        .prepare("SELECT consumer_id, event_id FROM inbox")
        .map_err(StoreError::from_rusqlite)?;
    let inbox_rows = s_inbox
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(StoreError::from_rusqlite)?;
    for r in inbox_rows {
        snap_inbox.insert(r.map_err(StoreError::from_rusqlite)?);
    }

    Ok(SnapshotBaseline {
        outbox_scopes,
        actions: snap_actions,
        receipts: snap_rcpts,
        inbox: snap_inbox,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "Collecte atomique pré-restore + delta complet (principals, événements, budgets, actions, receipts, inbox)"
)]
fn collect_source_delta(
    src_db: &Path,
    pre_restore_backup_path: &Path,
    snapshot_stream_id: StreamId,
    snapshot_sequence: Sequence,
    baseline: &SnapshotBaseline,
) -> Result<CollectedSourceDelta, StoreError> {
    let src_conn = Connection::open_with_flags(
        src_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;
    src_conn
        .busy_timeout(std::time::Duration::from_millis(5_000))
        .map_err(StoreError::from_rusqlite)?;

    // Sauvegarde pré-restore obligatoire si la base source est lisible
    if pre_restore_backup_path.exists() {
        let _ = fs::remove_file(pre_restore_backup_path);
    }
    {
        let mut pre_dst = Connection::open_with_flags(
            pre_restore_backup_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(StoreError::from_rusqlite)?;
        let bkp = Backup::new(&src_conn, &mut pre_dst).map_err(StoreError::from_rusqlite)?;
        bkp.step(-1).map_err(StoreError::from_rusqlite)?;
    }
    set_path_permissions(pre_restore_backup_path, 0o600)?;

    let mut delta = CollectedSourceDelta::default();

    // Principals
    {
        let mut stmt = src_conn
            .prepare("SELECT principal_id, principal_kind, actor_id, state, created_at_ms FROM principals")
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            delta.principals.push(r.map_err(StoreError::from_rusqlite)?);
        }
    }

    // Scopes touchés par événements postérieurs au watermark
    {
        let seq_i64 = i64::try_from(snapshot_sequence.as_u64()).unwrap_or(0);
        let mut stmt = src_conn
            .prepare(
                "SELECT project_id, aggregate_id FROM events
                 WHERE stream_id != ?1 OR sequence > ?2",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map(
                rusqlite::params![snapshot_stream_id.to_string(), seq_i64],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let (proj, agg) = r.map_err(StoreError::from_rusqlite)?;
            delta.affected_scopes.insert(proj.unwrap_or(agg));
        }
    }

    // Budget accounts et reservations complets (non régression absolue)
    {
        let mut stmt = src_conn
            .prepare(
                "SELECT account_id, scope_type, scope_id, currency, limit_micros, spent_micros, reserved_micros, revision
                 FROM budget_accounts",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceBudgetRow {
                    account_id: r.get(0)?,
                    scope_type: r.get(1)?,
                    scope_id: r.get(2)?,
                    currency: r.get(3)?,
                    limit_micros: r.get(4)?,
                    spent_micros: r.get(5)?,
                    reserved_micros: r.get(6)?,
                    revision: r.get(7)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let acc = r.map_err(StoreError::from_rusqlite)?;
            if acc.scope_type == "project" && (acc.spent_micros > 0 || acc.reserved_micros > 0) {
                delta.affected_scopes.insert(acc.scope_id.clone());
            }
            delta.budget_accounts.push(acc);
        }
    }

    {
        let mut stmt = src_conn
            .prepare(
                "SELECT reservation_id, account_id, action_id, reserved_micros, settled_micros, state, pricing_version, max_request_hash
                 FROM budget_reservations",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceReservationRow {
                    reservation_id: r.get(0)?,
                    account_id: r.get(1)?,
                    action_id: r.get(2)?,
                    reserved_micros: r.get(3)?,
                    settled_micros: r.get(4)?,
                    state: r.get(5)?,
                    pricing_version: r.get(6)?,
                    max_request_hash: r.get(7)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let br = r.map_err(StoreError::from_rusqlite)?;
            if !baseline.actions.contains(&br.action_id)
                || matches!(br.state.as_str(), "held" | "unknown" | "settled")
            {
                delta.capsule_budget_reservations.push(CapsuleBudgetDelta {
                    reservation_id: br.reservation_id.clone(),
                    account_id: br.account_id.clone(),
                    action_id: br.action_id.clone(),
                    reserved_micros: MicrosAmount::new(
                        u64::try_from(br.reserved_micros).unwrap_or(0),
                    ),
                    settled_micros: br
                        .settled_micros
                        .map(|m| MicrosAmount::new(u64::try_from(m).unwrap_or(0))),
                    state: br.state.clone(),
                    pricing_version: br.pricing_version.clone(),
                });
            }
            delta.budget_reservations.push(br);
        }
    }

    // Actions postérieures au snapshot (toutes origines : invocation, replay, control_plane)
    {
        let mut stmt = src_conn
            .prepare(
                "SELECT action_id, origin_kind, control_principal_id, run_id, step_id,
                        invocation_id, attempt_id, replay_worker_id, action_type, payload_hash,
                        authorization_ref, policy_revision, resource_lease_id,
                        authority_generation, daemon_epoch, fence, state, reconcile_strategy,
                        provider_idempotency_key, revision, created_at_ms
                 FROM actions",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceActionRow {
                    action_id: r.get(0)?,
                    origin_kind: r.get(1)?,
                    control_principal_id: r.get(2)?,
                    run_id: r.get(3)?,
                    step_id: r.get(4)?,
                    invocation_id: r.get(5)?,
                    attempt_id: r.get(6)?,
                    replay_worker_id: r.get(7)?,
                    action_type: r.get(8)?,
                    payload_hash: r.get(9)?,
                    authorization_ref: r.get(10)?,
                    policy_revision: r.get(11)?,
                    resource_lease_id: r.get(12)?,
                    authority_generation: r.get(13)?,
                    daemon_epoch: r.get(14)?,
                    fence: r.get(15)?,
                    state: r.get(16)?,
                    reconcile_strategy: r.get(17)?,
                    provider_idempotency_key: r.get(18)?,
                    revision: r.get(19)?,
                    created_at_ms: r.get(20)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let act = r.map_err(StoreError::from_rusqlite)?;
            if baseline.actions.contains(&act.action_id) {
                continue;
            }
            let matching_res = delta
                .budget_reservations
                .iter()
                .find(|br| br.action_id == act.action_id);
            let run_project_scope = act.run_id.as_deref().and_then(|rid| {
                src_conn
                    .query_row(
                        "SELECT m.project_id
                         FROM workflow_runs wr
                         JOIN missions m ON m.mission_id = wr.mission_id
                         WHERE wr.run_id = ?1",
                        [rid],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
            });
            let scope_id = matching_res
                .and_then(|br| {
                    delta
                        .budget_accounts
                        .iter()
                        .find(|a| a.account_id == br.account_id)
                        .map(|a| a.scope_id.clone())
                })
                .or(run_project_scope)
                .unwrap_or_else(|| act.action_id.clone());
            delta.affected_scopes.insert(scope_id.clone());

            if let Ok(parsed_action_id) = act.action_id.parse::<ActionId>() {
                delta.capsule_actions.push(CapsuleActionDelta {
                    action_id: parsed_action_id,
                    scope_id,
                    state: act.state.clone(),
                    reconcile_strategy: act.reconcile_strategy.clone(),
                    settled_micros: matching_res
                        .and_then(|br| br.settled_micros)
                        .map(|m| MicrosAmount::new(u64::try_from(m).unwrap_or(0))),
                    reserved_unknown_micros: matching_res
                        .filter(|br| br.state == "unknown" || br.state == "held")
                        .map(|br| {
                            MicrosAmount::new(u64::try_from(br.reserved_micros).unwrap_or(0))
                        }),
                    account_id: matching_res.map(|br| br.account_id.clone()),
                });
            }

            delta.actions.push(act);
        }
    }

    // Receipts postérieurs au snapshot
    {
        let mut stmt = src_conn
            .prepare(
                "SELECT receipt_id, actor_id, command_id, aggregate_id, command_digest, state
                 FROM command_receipts",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let (rcpt_id_s, actor_id, cmd_id_s, agg_id_s, digest_s, state) =
                r.map_err(StoreError::from_rusqlite)?;
            if baseline
                .receipts
                .contains(&(actor_id.clone(), cmd_id_s.clone()))
            {
                continue;
            }
            if let (Ok(receipt_id), Ok(command_id), Ok(command_digest)) = (
                rcpt_id_s.parse::<ReceiptId>(),
                cmd_id_s.parse::<CommandId>(),
                digest_s.parse::<Sha256Digest>(),
            ) {
                delta.capsule_receipts.push(CapsuleReceiptDelta {
                    receipt_id,
                    actor_id,
                    command_id,
                    scope_id: agg_id_s,
                    command_digest,
                    state,
                });
            }
        }
    }

    // Consommations inbox postérieures au snapshot (sqlite-design.md:103, technical-design.md:31)
    {
        let mut stmt = src_conn
            .prepare(
                "SELECT i.consumer_id, i.event_id, i.processed_at_ms, i.result_hash, e.project_id, e.aggregate_id
                 FROM inbox i
                 JOIN events e ON e.event_id = i.event_id",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let (consumer_id, event_id, processed_at_ms, result_hash, proj_id, agg_id) =
                r.map_err(StoreError::from_rusqlite)?;
            if baseline
                .inbox
                .contains(&(consumer_id.clone(), event_id.clone()))
            {
                continue;
            }
            let scope_id = proj_id.unwrap_or(agg_id);
            delta.affected_scopes.insert(scope_id.clone());
            delta.inbox_consumptions.push(SourceInboxRow {
                consumer_id,
                event_id,
                scope_id,
                processed_at_ms,
                result_hash,
            });
        }
    }

    Ok(delta)
}

fn fk_row_exists(tx: &rusqlite::Transaction<'_>, sql: &str, id: &str) -> Result<bool, StoreError> {
    let cnt: i64 = tx
        .query_row(sql, [id], |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    Ok(cnt > 0)
}

fn action_parents_present_in_target(
    tx: &rusqlite::Transaction<'_>,
    act: &SourceActionRow,
) -> Result<bool, StoreError> {
    if let Some(ref rid) = act.run_id
        && !fk_row_exists(
            tx,
            "SELECT COUNT(*) FROM workflow_runs WHERE run_id = ?1",
            rid,
        )?
    {
        return Ok(false);
    }
    if let Some(ref sid) = act.step_id
        && !fk_row_exists(
            tx,
            "SELECT COUNT(*) FROM step_instances WHERE step_id = ?1",
            sid,
        )?
    {
        return Ok(false);
    }
    match act.origin_kind.as_str() {
        "control_plane" => {
            let Some(ref pid) = act.control_principal_id else {
                return Ok(false);
            };
            fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM principals WHERE principal_id = ?1",
                pid,
            )
        }
        "invocation" => {
            let (Some(inv_id), Some(att_id)) = (&act.invocation_id, &act.attempt_id) else {
                return Ok(false);
            };
            Ok(fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM invocations WHERE invocation_id = ?1",
                inv_id,
            )? && fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM invocation_attempts WHERE attempt_id = ?1",
                att_id,
            )?)
        }
        "replay" => {
            let (Some(rw_id), Some(rid), Some(sid)) =
                (&act.replay_worker_id, &act.run_id, &act.step_id)
            else {
                return Ok(false);
            };
            Ok(fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM replay_workers WHERE replay_worker_id = ?1",
                rw_id,
            )? && fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM workflow_runs WHERE run_id = ?1",
                rid,
            )? && fk_row_exists(
                tx,
                "SELECT COUNT(*) FROM step_instances WHERE step_id = ?1",
                sid,
            )?)
        }
        _ => Ok(false),
    }
}

/// Exécute le protocole offline conservateur de restauration (`RestoreBackup`, `REQ-PERSIST-010`).
#[expect(
    clippy::too_many_lines,
    reason = "Protocole complet offline : validation, manifeste 0600, neutralisation, delta/capsule, import et bascule"
)]
pub(crate) fn execute_restore_backup(
    req: &RestoreBackupRequest,
    target_config: &StoreConfig,
) -> Result<RestoreActivationResult, StoreError> {
    let val = validate_backup_dir(&req.backup_dir, target_config)?;
    if req.target_data_root == req.backup_dir {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "target_data_root ne peut pas être identique à backup_dir".to_owned(),
        });
    }
    if req
        .source_db_path
        .as_ref()
        .is_some_and(|p| p == &target_config.db_path)
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "target_config.db_path ne peut pas écraser source_db_path en place".to_owned(),
        });
    }

    let now_ms = target_config.clock.now_ms();
    let restore_id = RestoreId::from_uuid(target_config.id_generator.new_uuid());
    let receipt_id = ReceiptId::from_uuid(target_config.id_generator.new_uuid());
    let _span = tracing::info_span!(
        "restore_backup",
        restore_id = %restore_id,
        backup_id = %val.backup_id,
        source_authority = %req.source_authority_generation,
        source_db_unreadable = req.source_db_unreadable
    )
    .entered();

    let maint_manifest = if req.source_db_unreadable {
        let m = MaintenanceRestoreManifest {
            restore_id,
            command_receipt_id: receipt_id,
            actor_id: req.actor_id.clone(),
            command_id: req.command_id,
            command_digest: req.command_digest,
            backup_id: val.backup_id,
            source_authority_generation: req.source_authority_generation,
            state: RestoreRequestState::Neutralizing,
            blocked_reason: None,
            updated_at_ms: now_ms,
        };
        write_maintenance_manifest(&target_config.maintenance_dir, &m)?;
        Some(m)
    } else {
        let Some(src_db) = req.source_db_path.as_ref() else {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "source_db_path requis quand source_db_unreadable = false".to_owned(),
            });
        };
        ensure_source_restore_tracking(src_db, req, val.backup_id, restore_id, receipt_id, now_ms)?;
        None
    };

    // Inventaire & neutralisation obligatoires avant toute importation dans une cible
    let neutralization_res = req
        .neutralizer
        .inventory_and_neutralize(req.source_authority_generation);
    let block_reason_opt = match neutralization_res {
        Err(ref e) => Some(e.to_string()),
        Ok(ref items) => items
            .iter()
            .find(|it| {
                !it.neutralized || it.authority_generation != req.source_authority_generation
            })
            .map(|it| {
                format!(
                    "exécuteur '{}' non neutralisé ou lignée divergente",
                    it.executor_id
                )
            }),
    };

    if let Some(reason) = block_reason_opt {
        tracing::warn!(
            restore_id = %restore_id,
            reason = %reason,
            source_db_unreadable = req.source_db_unreadable,
            "restauration bloquée avant génération capsule"
        );
        if let Some(mut m) = maint_manifest {
            m.state = RestoreRequestState::Blocked;
            m.blocked_reason = Some(reason.clone());
            m.updated_at_ms = target_config.clock.now_ms();
            let _ = write_maintenance_manifest(&target_config.maintenance_dir, &m);
        } else if let Some(src_db) = req.source_db_path.as_ref() {
            mark_source_restore_blocked(src_db, restore_id, &reason);
        }
        return Err(StoreError::RestorationBlocked(reason));
    }

    let snapshot_db = req.backup_dir.join("db.sqlite");
    let baseline = collect_snapshot_outbox_scopes(&snapshot_db)?;
    let mut frozen_scopes = baseline.outbox_scopes.clone();

    fs::create_dir_all(&req.target_data_root)?;
    set_dir_permissions_0700(&req.target_data_root)?;

    let (coverage, src_delta) = if req.source_db_unreadable {
        (CapsuleCoverage::Unknown, CollectedSourceDelta::default())
    } else {
        let Some(src_db) = req.source_db_path.as_ref() else {
            return Err(StoreError::RestoreIncomplete(
                "source_db_path absent pour collecte delta".to_owned(),
            ));
        };
        let pre_restore_path = req
            .target_data_root
            .join(format!("pre-restore-{restore_id}.sqlite"));
        let delta = collect_source_delta(
            src_db,
            &pre_restore_path,
            val.manifest.snapshot_stream_id,
            val.manifest.snapshot_sequence,
            &baseline,
        )?;
        for s in &delta.affected_scopes {
            frozen_scopes.insert(s.clone());
        }
        (CapsuleCoverage::Complete, delta)
    };

    let capsule_inbox: Vec<(String, String, String)> = src_delta
        .inbox_consumptions
        .iter()
        .map(|row| {
            (
                row.consumer_id.clone(),
                row.event_id.clone(),
                row.scope_id.clone(),
            )
        })
        .collect();
    let frozen_scopes_vec: Vec<String> = frozen_scopes.iter().cloned().collect();
    let capsule = RestoreCapsule {
        restore_id,
        command_receipt_id: receipt_id,
        backup_id: val.backup_id,
        source_authority_generation: req.source_authority_generation,
        snapshot_stream_id: val.manifest.snapshot_stream_id,
        snapshot_sequence: val.manifest.snapshot_sequence,
        coverage,
        affected_scopes: frozen_scopes_vec.clone(),
        actions: src_delta.capsule_actions,
        budget_reservations: src_delta.capsule_budget_reservations,
        receipts: src_delta.capsule_receipts,
        inbox_consumptions: capsule_inbox,
        sealed_at_ms: now_ms,
    };

    let capsule_bytes = serde_json::to_vec_pretty(&capsule)?;
    let capsule_hash = Sha256Digest::from_bytes(&capsule_bytes);
    let capsule_rel = object_relative_path(&capsule_hash, 1);
    let capsule_len_i64 = i64::try_from(capsule_bytes.len()).unwrap_or(i64::MAX);

    // Copier la base et les objets du backup dans la racine cible
    if let Some(target_db_parent) = target_config.db_path.parent() {
        fs::create_dir_all(target_db_parent)?;
        set_dir_permissions_0700(target_db_parent)?;
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", target_config.db_path.display()));
        if sidecar.exists() {
            let _ = fs::remove_file(sidecar);
        }
    }
    fs::copy(&snapshot_db, &target_config.db_path)?;
    set_path_permissions(&target_config.db_path, 0o600)?;
    fs::File::open(&target_config.db_path)?.sync_all()?;

    fs::create_dir_all(&target_config.artifacts_dir)?;
    set_dir_permissions_0700(&target_config.artifacts_dir)?;
    let bkp_objs_dir = req.backup_dir.join("objects");
    for entry in &val.manifest.included_objects {
        let src_obj = safe_artifact_path(&bkp_objs_dir, &entry.relative_path)?;
        let dst_obj = safe_artifact_path(&target_config.artifacts_dir, &entry.relative_path)?;
        let obj_bytes = fs::read(&src_obj)?;
        write_atomic_durable(&dst_obj, &obj_bytes, 0o600)?;
    }

    let capsule_dst = safe_artifact_path(&target_config.artifacts_dir, &capsule_rel)?;
    write_atomic_durable(&capsule_dst, &capsule_bytes, 0o600)?;

    // Ouvrir la cible et activer nouvelle lignée / stream / read_only_recovery / gels
    let mut target_authority_generation =
        AuthorityGeneration::from_uuid(target_config.id_generator.new_uuid());
    if target_authority_generation == req.source_authority_generation {
        target_authority_generation =
            AuthorityGeneration::from_uuid(target_config.id_generator.new_uuid());
    }
    let mut target_stream_id = StreamId::from_uuid(target_config.id_generator.new_uuid());
    if target_stream_id == val.manifest.snapshot_stream_id {
        target_stream_id = StreamId::from_uuid(target_config.id_generator.new_uuid());
    }
    let target_daemon_epoch = DaemonEpoch::new(1);

    let mut target_conn = Connection::open_with_flags(
        &target_config.db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;
    inspect_and_verify_build(&target_conn, &target_config.build_policy)?;
    let _ = apply_and_verify_writer_pragmas(&target_conn, target_config.busy_timeout_ms)?;

    let tx = target_conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreError::from_rusqlite)?;

    let new_inst = target_config.id_generator.new_uuid().to_string();
    let new_boot = target_config.id_generator.new_uuid().to_string();
    let target_auth_s = target_authority_generation.to_string();
    let src_auth_s = req.source_authority_generation.to_string();
    let target_stream_s = target_stream_id.to_string();

    tx.execute(
        "UPDATE daemon_state
         SET authority_generation = ?1, daemon_epoch = 1, daemon_instance_id = ?2,
             boot_identity = ?3, stream_id = ?4, mode = 'read_only_recovery', started_at_ms = ?5
         WHERE singleton = 1",
        rusqlite::params![
            target_auth_s,
            new_inst,
            new_boot,
            target_stream_s,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    // Enregistrer la capsule comme objet live épinglé
    tx.execute(
        "INSERT INTO object_records
         (object_hash, object_generation, relative_path, byte_length, media_type, state, delete_operation_id, created_at_ms)
         VALUES (?1, 1, ?2, ?3, 'application/json', 'live', NULL, ?4)
         ON CONFLICT(object_hash) DO UPDATE SET state = 'live', relative_path = excluded.relative_path",
        rusqlite::params![capsule_hash.as_prefixed_str(), capsule_rel, capsule_len_i64, now_ms.as_i64()],
    )
    .map_err(StoreError::from_rusqlite)?;

    let capsule_pin_id = target_config.id_generator.new_uuid().to_string();
    tx.execute(
        "INSERT INTO object_pins
         (pin_id, pin_kind, object_hash, object_generation, owner_kind, owner_id, authority_generation, state, created_at_ms, released_at_ms)
         VALUES (?1, 'read', ?2, 1, 'restore_import', ?3, ?4, 'held', ?5, NULL)",
        rusqlite::params![
            capsule_pin_id,
            capsule_hash.as_prefixed_str(),
            restore_id.to_string(),
            target_auth_s,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    // Conserver un reçu command_receipts pour RestoreBackup
    let existing_target_rcpt: Option<String> = tx
        .query_row(
            "SELECT receipt_id FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![req.actor_id, req.command_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let target_rcpt_str = if let Some(r) = existing_target_rcpt {
        r
    } else {
        let r_str = receipt_id.to_string();
        tx.execute(
            "INSERT INTO command_receipts
             (actor_id, command_id, authority_generation, receipt_id, request_id, command_type,
              command_digest, aggregate_id, state, response_state, response_json,
              response_retained_until_ms, first_event_seq, last_event_seq, committed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'RestoreBackup', ?6, ?7, 'completed', 'full', '{}', ?8, NULL, NULL, ?9)",
            rusqlite::params![
                req.actor_id,
                req.command_id.to_string(),
                target_auth_s,
                r_str,
                restore_id.to_string(),
                req.command_digest.as_prefixed_str(),
                restore_id.to_string(),
                now_ms.as_i64().saturating_add(30 * 86_400_000),
                now_ms.as_i64()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
        r_str
    };

    // Appliquer non-régression budgétaire et deltas d'actions source
    for (p_id, p_kind, a_id, st, cr_ms) in &src_delta.principals {
        tx.execute(
            "INSERT OR IGNORE INTO principals (principal_id, principal_kind, actor_id, state, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![p_id, p_kind, a_id, st, cr_ms],
        )
        .map_err(StoreError::from_rusqlite)?;
    }
    for acc in &src_delta.budget_accounts {
        tx.execute(
            "INSERT INTO budget_accounts
             (account_id, scope_type, scope_id, currency, limit_micros, spent_micros, reserved_micros, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(account_id) DO UPDATE SET
               spent_micros = MAX(budget_accounts.spent_micros, excluded.spent_micros),
               reserved_micros = MAX(budget_accounts.reserved_micros, excluded.reserved_micros),
               limit_micros = MAX(budget_accounts.limit_micros, excluded.limit_micros),
               revision = MAX(budget_accounts.revision, excluded.revision)",
            rusqlite::params![
                acc.account_id,
                acc.scope_type,
                acc.scope_id,
                acc.currency,
                acc.limit_micros,
                acc.spent_micros,
                acc.reserved_micros,
                acc.revision
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    for ib in &src_delta.inbox_consumptions {
        let ev_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM events WHERE event_id = ?1",
                [ib.event_id.as_str()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if ev_exists > 0 {
            tx.execute(
                "INSERT OR IGNORE INTO inbox (consumer_id, event_id, processed_at_ms, result_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    ib.consumer_id,
                    ib.event_id,
                    ib.processed_at_ms,
                    ib.result_hash
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    for act in &src_delta.actions {
        if !action_parents_present_in_target(&tx, act)? {
            continue;
        }
        let valid_lease_id = match act.resource_lease_id.as_deref() {
            Some(lid)
                if fk_row_exists(
                    &tx,
                    "SELECT COUNT(*) FROM resource_leases WHERE lease_id = ?1",
                    lid,
                )? =>
            {
                Some(lid.to_owned())
            }
            _ => None,
        };
        tx.execute(
            "INSERT OR IGNORE INTO actions
             (action_id, origin_kind, control_principal_id, run_id, step_id, invocation_id, attempt_id,
              replay_worker_id, action_type, payload_hash, authorization_ref, policy_revision, resource_lease_id,
              authority_generation, daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key, revision, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            rusqlite::params![
                act.action_id,
                act.origin_kind,
                act.control_principal_id,
                act.run_id,
                act.step_id,
                act.invocation_id,
                act.attempt_id,
                act.replay_worker_id,
                act.action_type,
                act.payload_hash,
                act.authorization_ref,
                act.policy_revision,
                valid_lease_id,
                act.authority_generation,
                act.daemon_epoch,
                act.fence,
                act.state,
                act.reconcile_strategy,
                act.provider_idempotency_key,
                act.revision,
                act.created_at_ms
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    for res in &src_delta.budget_reservations {
        let act_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM actions WHERE action_id = ?1",
                [res.action_id.as_str()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if act_exists > 0 {
            tx.execute(
                "INSERT INTO budget_reservations
                 (reservation_id, account_id, action_id, reserved_micros, settled_micros, state, pricing_version, max_request_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_id, action_id) DO UPDATE SET
                   reserved_micros = MAX(budget_reservations.reserved_micros, excluded.reserved_micros),
                   settled_micros = COALESCE(excluded.settled_micros, budget_reservations.settled_micros),
                   state = excluded.state",
                rusqlite::params![
                    res.reservation_id,
                    res.account_id,
                    res.action_id,
                    res.reserved_micros,
                    res.settled_micros,
                    res.state,
                    res.pricing_version,
                    res.max_request_hash
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    let frozen_json = serde_json::to_string(&frozen_scopes_vec)?;
    tx.execute(
        "INSERT INTO restore_imports
         (restore_id, command_receipt_id, backup_id, source_authority_generation, target_authority_generation,
          capsule_object_hash, coverage, state, frozen_scopes_json, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'prepared', ?8, 0)",
        rusqlite::params![
            restore_id.to_string(),
            target_rcpt_str,
            val.backup_id.to_string(),
            src_auth_s,
            target_auth_s,
            capsule_hash.as_prefixed_str(),
            coverage.as_str(),
            frozen_json
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    target_config
        .cutpoints
        .hit(StoreCutpoint::C11RestoreBeforeActivate)?;

    tx.execute(
        "UPDATE restore_imports SET state = 'activated', revision = revision + 1 WHERE restore_id = ?1",
        [restore_id.to_string()],
    )
    .map_err(StoreError::from_rusqlite)?;

    let global_freeze = coverage == CapsuleCoverage::Unknown;
    let gf_i64 = i64::from(global_freeze);
    if global_freeze && frozen_scopes_vec.is_empty() {
        tx.execute(
            "INSERT INTO recovery_scope_freezes
             (scope_id, restore_id, reason, global_freeze, state, created_at_ms, disposed_at_ms, disposition_receipt_id)
             VALUES ('__global__', ?1, 'source_unreadable_unknown_coverage', 1, 'frozen', ?2, NULL, NULL)
             ON CONFLICT(scope_id) DO UPDATE SET
               restore_id = excluded.restore_id,
               global_freeze = 1,
               state = 'frozen',
               disposed_at_ms = NULL,
               disposition_receipt_id = NULL",
            rusqlite::params![restore_id.to_string(), now_ms.as_i64()],
        )
        .map_err(StoreError::from_rusqlite)?;
    } else {
        for scope_id in &frozen_scopes_vec {
            tx.execute(
                "INSERT INTO recovery_scope_freezes
                 (scope_id, restore_id, reason, global_freeze, state, created_at_ms, disposed_at_ms, disposition_receipt_id)
                 VALUES (?1, ?2, ?3, ?4, 'frozen', ?5, NULL, NULL)
                 ON CONFLICT(scope_id) DO UPDATE SET
                   restore_id = excluded.restore_id,
                   reason = excluded.reason,
                   global_freeze = MAX(recovery_scope_freezes.global_freeze, excluded.global_freeze),
                   state = 'frozen',
                   disposed_at_ms = NULL,
                   disposition_receipt_id = NULL",
                rusqlite::params![
                    scope_id,
                    restore_id.to_string(),
                    if global_freeze { "unknown_coverage_freeze" } else { "post_snapshot_or_outbox_scope" },
                    gf_i64,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    tx.commit().map_err(StoreError::from_rusqlite)?;
    drop(target_conn);

    tracing::info!(
        restore_id = %restore_id,
        target_authority_generation = %target_authority_generation,
        target_stream_id = %target_stream_id,
        coverage = coverage.as_str(),
        frozen_scopes_count = frozen_scopes_vec.len(),
        "restauration activée sur nouvelle lignée"
    );

    if let Some(mut m) = maint_manifest {
        m.state = RestoreRequestState::HandedOver;
        m.updated_at_ms = target_config.clock.now_ms();
        write_maintenance_manifest(&target_config.maintenance_dir, &m)?;
    } else if let Some(src_db) = req.source_db_path.as_ref() {
        mark_source_restore_handed_over(src_db, restore_id);
    }

    Ok(RestoreActivationResult {
        restore_id,
        command_receipt_id: receipt_id,
        source_authority_generation: req.source_authority_generation,
        target_authority_generation,
        target_stream_id,
        target_daemon_epoch,
        coverage,
        global_freeze_active: global_freeze,
        frozen_scopes: frozen_scopes_vec,
        capsule_object_hash: capsule_hash,
    })
}
