//! Restauration offline conservatrice, neutralisation, manifeste privé `0600` et capsule delta (`REQ-PERSIST-010`).

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
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
use crate::fs_sync::{
    set_dir_permissions_0700, set_path_permissions, sync_parent_dir, write_atomic_durable,
};
use crate::migrations::{CURRENT_SCHEMA_VERSION, apply_pending_migrations_for_restore};
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

/// Dispatch post-watermark capturé dans la capsule durable (`action_dispatches`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleDispatchDelta {
    /// Identifiant du dispatch.
    pub dispatch_id: String,
    /// Identifiant de l'action.
    pub action_id: ActionId,
    /// Numéro ordinal.
    pub ordinal: i64,
    /// Adaptateur émetteur.
    pub adapter_id: String,
    /// Hash de la requête transport.
    pub request_hash: Sha256Digest,
    /// Identifiant fournisseur éventuel.
    pub provider_request_id: Option<String>,
    /// Horodatage de début.
    pub started_at_ms: TimestampMs,
    /// Horodatage de fin éventuel.
    pub ended_at_ms: Option<TimestampMs>,
    /// État transport observé.
    pub transport_state: String,
    /// Empreinte de reçu transport éventuelle.
    pub receipt_hash: Option<Sha256Digest>,
}

/// Nature d'un tombstone ou d'une révocation post-snapshot capturé dans la capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapsuleTombstoneKind {
    /// Réponse de reçu de commande expirée/tombstonée.
    CommandReceipt,
    /// Payload d'événement tombstoné.
    EventPayload,
    /// Objet supprimé définitivement (`object_records` / `object_deletions`).
    ObjectDeletion,
    /// Référence d'objet tombstonée.
    ObjectReference,
    /// Révocation de référence secrète (`secret_references`).
    SecretReferenceRevocation,
    /// Révocation définitive de sujet (`subject_suspensions`).
    SubjectRevocation,
}

impl CapsuleTombstoneKind {
    /// Représentation textuelle canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CommandReceipt => "command_receipt",
            Self::EventPayload => "event_payload",
            Self::ObjectDeletion => "object_deletion",
            Self::ObjectReference => "object_reference",
            Self::SecretReferenceRevocation => "secret_reference_revocation",
            Self::SubjectRevocation => "subject_revocation",
        }
    }
}

/// Tombstone ou révocation de sécurité post-snapshot appliqué lors de la bascule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleTombstoneDelta {
    /// Catégorie du tombstone.
    pub kind: CapsuleTombstoneKind,
    /// Identifiant principal de l'entité (ex. `actor_id` ou `object_hash` ou `secret_ref_id`).
    pub entity_id: String,
    /// Sous-identifiant éventuel (ex. `command_id` ou `reference_id`).
    pub sub_id: Option<String>,
    /// Statut cible appliqué (`tombstone`, `revoked_local`, `revoked`, ...).
    pub status: String,
    /// Motif ou état complémentaire éventuel.
    pub reason: Option<String>,
    /// Horodatage d'observation.
    pub updated_at_ms: TimestampMs,
}

/// Entrée historique append-only consignée dans le manifeste privé de maintenance `0600`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceManifestEntry {
    /// Identifiant de restauration.
    pub restore_id: RestoreId,
    /// Reçu attribué à `RestoreBackup`.
    pub command_receipt_id: ReceiptId,
    /// Acteur demandeur.
    pub actor_id: String,
    /// Identifiant de commande client.
    pub command_id: CommandId,
    /// Digest canonique de la commande.
    pub command_digest: Sha256Digest,
    /// Sauvegarde ciblée.
    pub backup_id: BackupId,
    /// Lignée source.
    pub source_authority_generation: AuthorityGeneration,
    /// État persisté pour cette tentative (`neutralizing`, `blocked`, `handed_over`, ...).
    pub state: RestoreRequestState,
    /// Raison éventuelle de blocage (`RESTORATION_BLOCKED`).
    pub blocked_reason: Option<String>,
    /// Horodatage de cette entrée.
    pub updated_at_ms: TimestampMs,
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
    /// Dispatches post-watermark (`action_dispatches`).
    #[serde(default)]
    pub dispatches: Vec<CapsuleDispatchDelta>,
    /// Tombstones et révocations de sécurité post-watermark (`command_receipts`, `events`, objets, secrets, sujets).
    #[serde(default)]
    pub tombstones: Vec<CapsuleTombstoneDelta>,
    /// Historique du manifeste privé importé vers `restore_imports` / `restore_manifest_records`.
    #[serde(default)]
    pub manifest_entries: Vec<MaintenanceManifestEntry>,
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
    /// Journal append-only des tentatives successives avant bascule.
    #[serde(default)]
    pub history: Vec<MaintenanceManifestEntry>,
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

    let log_path = maintenance_dir.join("restore-manifest.jsonl");
    let log_entry = MaintenanceManifestEntry {
        restore_id: manifest.restore_id,
        command_receipt_id: manifest.command_receipt_id,
        actor_id: manifest.actor_id.clone(),
        command_id: manifest.command_id,
        command_digest: manifest.command_digest,
        backup_id: manifest.backup_id,
        source_authority_generation: manifest.source_authority_generation,
        state: manifest.state,
        blocked_reason: manifest.blocked_reason.clone(),
        updated_at_ms: manifest.updated_at_ms,
    };
    let mut line = serde_json::to_vec(&log_entry)?;
    line.push(b'\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    f.write_all(&line)?;
    f.sync_all()?;
    set_path_permissions(&log_path, 0o600)?;
    sync_parent_dir(&log_path)?;
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
    project_id: Option<String>,
    scope_key: Option<String>,
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
    approval_scope: Option<String>,
    subject_kind: Option<String>,
    subject_id: Option<String>,
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
struct SourceDispatchRow {
    dispatch_id: String,
    action_id: String,
    ordinal: i64,
    adapter_id: String,
    request_hash: String,
    provider_request_id: Option<String>,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    transport_state: String,
    receipt_hash: Option<String>,
}

#[derive(Debug, Clone)]
struct SourceSecretRevocationRow {
    secret_ref_id: String,
    opaque_secret_ref: String,
    namespace: String,
    purpose: String,
    secret_kind: String,
    target_kind: String,
    target_id: String,
    scope_kind: String,
    scope_id: String,
    backend_kind: String,
    status: String,
    local_revoked_at_ms: Option<i64>,
    remote_revocation_state: String,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct SourceSubjectRevocationRow {
    suspension_id: String,
    subject_kind: String,
    subject_id: String,
    scope_kind: String,
    scope_id: String,
    status: String,
    reason: String,
    guard_propagation_state: String,
    acted_by_actor: String,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct SourceObjectDeletionRow {
    delete_operation_id: String,
    object_hash: String,
    relative_path: String,
    reason: String,
    state: String,
    created_at_ms: i64,
    completed_at_ms: Option<i64>,
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
    dispatches: Vec<SourceDispatchRow>,
    capsule_dispatches: Vec<CapsuleDispatchDelta>,
    tombstones: Vec<CapsuleTombstoneDelta>,
    tombstoned_receipts: Vec<(String, String)>,
    tombstoned_events: Vec<String>,
    object_deletions: Vec<SourceObjectDeletionRow>,
    secret_revocations: Vec<SourceSecretRevocationRow>,
    subject_revocations: Vec<SourceSubjectRevocationRow>,
    budget_accounts: Vec<SourceBudgetRow>,
    budget_reservations: Vec<SourceReservationRow>,
    capsule_actions: Vec<CapsuleActionDelta>,
    capsule_budget_reservations: Vec<CapsuleBudgetDelta>,
    capsule_receipts: Vec<CapsuleReceiptDelta>,
    inbox_consumptions: Vec<SourceInboxRow>,
    principals: Vec<(String, String, String, String, i64)>,
    affected_scopes: BTreeSet<String>,
}

enum SourceTrackOutcome {
    TrackedInDb,
    FallbackUnwritableManifest,
}

#[expect(
    clippy::too_many_lines,
    reason = "Suivi transactionnel de restore_requests en base source avec repli manifeste 0600"
)]
fn ensure_source_restore_tracking(
    src_db: &Path,
    req: &RestoreBackupRequest,
    backup_id: BackupId,
    restore_id: RestoreId,
    receipt_id: ReceiptId,
    now_ms: TimestampMs,
) -> Result<SourceTrackOutcome, StoreError> {
    let mut conn = match Connection::open_with_flags(
        src_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            let mapped = StoreError::from_rusqlite(err);
            if matches!(
                mapped,
                StoreError::StorageCorrupt(_) | StoreError::StorageUnavailable(_)
            ) && src_db.exists()
            {
                return Ok(SourceTrackOutcome::FallbackUnwritableManifest);
            }
            return Err(mapped);
        }
    };
    if let Err(err) = conn.busy_timeout(std::time::Duration::from_millis(5_000)) {
        let mapped = StoreError::from_rusqlite(err);
        if matches!(mapped, StoreError::StorageCorrupt(_)) {
            return Ok(SourceTrackOutcome::FallbackUnwritableManifest);
        }
        return Err(mapped);
    }
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(StoreError::from_rusqlite)?;

    let tx = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(t) => t,
        Err(err) => {
            let mapped = StoreError::from_rusqlite(err);
            if matches!(
                mapped,
                StoreError::StorageCorrupt(_) | StoreError::StorageUnavailable(_)
            ) {
                return Ok(SourceTrackOutcome::FallbackUnwritableManifest);
            }
            return Err(mapped);
        }
    };

    let (src_auth_str, src_rev, src_mode): (String, i64, String) = tx
        .query_row(
            "SELECT authority_generation, revision, mode FROM daemon_state WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(StoreError::from_rusqlite)?;
    if !matches!(
        src_mode.as_str(),
        "ready" | "degraded" | "read_only_recovery" | "recovering" | "stopped"
    ) {
        return Err(StoreError::CapacityBlocked(format!(
            "RestoreBackup refusé depuis mode daemon '{src_mode}'"
        )));
    }
    let src_auth: AuthorityGeneration = src_auth_str.parse()?;
    if src_auth != req.source_authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: src_auth,
            actual: req.source_authority_generation,
            command_id: Some(req.command_id),
        });
    }

    let existing_rcpt: Option<(String, String)> = tx
        .query_row(
            "SELECT receipt_id, command_digest FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![req.actor_id, req.command_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    if let Some((rcpt_id_s, stored_digest)) = existing_rcpt {
        if stored_digest != req.command_digest.as_prefixed_str() {
            return Err(StoreError::IdempotencyConflict {
                command_id: req.command_id,
            });
        }
        let existing_req: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT state, blocked_reason FROM restore_requests WHERE command_receipt_id = ?1 ORDER BY rowid DESC LIMIT 1",
                [&rcpt_id_s],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        if let Some((st, br)) = existing_req
            && st == "blocked"
        {
            return Err(StoreError::RestorationBlocked(
                br.unwrap_or_else(|| "restauration précédemment bloquée".to_owned()),
            ));
        }
    }

    let effective_rcpt = receipt_id.to_string();
    let existing_check: Option<String> = tx
        .query_row(
            "SELECT receipt_id FROM command_receipts WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![req.actor_id, req.command_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let rcpt_to_use = if let Some(r) = existing_check {
        r
    } else {
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
                effective_rcpt,
                restore_id.to_string(),
                req.command_digest.as_prefixed_str(),
                restore_id.to_string(),
                now_ms.as_i64().saturating_add(30 * 86_400_000),
                now_ms.as_i64()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
        effective_rcpt
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
            rcpt_to_use,
            backup_id.to_string(),
            src_auth_str
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.commit().map_err(StoreError::from_rusqlite)?;
    Ok(SourceTrackOutcome::TrackedInDb)
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
    dispatches: HashSet<String>,
    receipts: HashSet<(String, String)>,
    full_receipts: HashSet<(String, String)>,
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

    let mut snap_dispatches = HashSet::new();
    let mut s_disp = conn
        .prepare("SELECT dispatch_id FROM action_dispatches")
        .map_err(StoreError::from_rusqlite)?;
    let disp_rows = s_disp
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(StoreError::from_rusqlite)?;
    for r in disp_rows {
        snap_dispatches.insert(r.map_err(StoreError::from_rusqlite)?);
    }

    let mut snap_rcpts = HashSet::new();
    let mut snap_full_rcpts = HashSet::new();
    let mut s_rcpt = conn
        .prepare("SELECT actor_id, command_id, response_state FROM command_receipts")
        .map_err(StoreError::from_rusqlite)?;
    let rcpt_rows = s_rcpt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(StoreError::from_rusqlite)?;
    for r in rcpt_rows {
        let (a, c, st) = r.map_err(StoreError::from_rusqlite)?;
        snap_rcpts.insert((a.clone(), c.clone()));
        if st == "full" {
            snap_full_rcpts.insert((a, c));
        }
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
        dispatches: snap_dispatches,
        receipts: snap_rcpts,
        full_receipts: snap_full_rcpts,
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

    // Scopes d'exécution touchés par événements postérieurs au watermark
    {
        let seq_i64 = i64::try_from(snapshot_sequence.as_u64()).unwrap_or(0);
        let mut stmt = src_conn
            .prepare(
                "SELECT project_id, aggregate_type, aggregate_id FROM events
                 WHERE stream_id != ?1 OR sequence > ?2",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map(
                rusqlite::params![snapshot_stream_id.to_string(), seq_i64],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(StoreError::from_rusqlite)?;
        for r in rows {
            let (proj, agg_type, agg) = r.map_err(StoreError::from_rusqlite)?;
            if let Some(p) = proj {
                delta.affected_scopes.insert(p);
            } else if matches!(
                agg_type.as_str(),
                "project" | "mission" | "workflow_run" | "action" | "invocation"
            ) {
                delta.affected_scopes.insert(agg);
            }
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
    let src_schema_ver: i64 = src_conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap_or(2);
    {
        let actions_sql = if src_schema_ver >= 3 {
            "SELECT action_id, origin_kind, control_principal_id, project_id, scope_key,
                    run_id, step_id, invocation_id, attempt_id, replay_worker_id, action_type,
                    payload_hash, authorization_ref, policy_revision, resource_lease_id,
                    approval_scope, subject_kind, subject_id, authority_generation,
                    daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key,
                    revision, created_at_ms
             FROM actions"
        } else {
            "SELECT action_id, origin_kind, control_principal_id, NULL, NULL,
                    run_id, step_id, invocation_id, attempt_id, replay_worker_id, action_type,
                    payload_hash, authorization_ref, policy_revision, resource_lease_id,
                    NULL, NULL, NULL, authority_generation,
                    daemon_epoch, fence, state, reconcile_strategy, provider_idempotency_key,
                    revision, created_at_ms
             FROM actions"
        };
        let mut stmt = src_conn
            .prepare(actions_sql)
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceActionRow {
                    action_id: r.get(0)?,
                    origin_kind: r.get(1)?,
                    control_principal_id: r.get(2)?,
                    project_id: r.get(3)?,
                    scope_key: r.get(4)?,
                    run_id: r.get(5)?,
                    step_id: r.get(6)?,
                    invocation_id: r.get(7)?,
                    attempt_id: r.get(8)?,
                    replay_worker_id: r.get(9)?,
                    action_type: r.get(10)?,
                    payload_hash: r.get(11)?,
                    authorization_ref: r.get(12)?,
                    policy_revision: r.get(13)?,
                    resource_lease_id: r.get(14)?,
                    approval_scope: r.get(15)?,
                    subject_kind: r.get(16)?,
                    subject_id: r.get(17)?,
                    authority_generation: r.get(18)?,
                    daemon_epoch: r.get(19)?,
                    fence: r.get(20)?,
                    state: r.get(21)?,
                    reconcile_strategy: r.get(22)?,
                    provider_idempotency_key: r.get(23)?,
                    revision: r.get(24)?,
                    created_at_ms: r.get(25)?,
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
                .or_else(|| act.project_id.clone())
                .or_else(|| act.scope_key.clone())
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

    // Dispatches postérieurs au snapshot (`action_dispatches`)
    {
        let mut stmt = src_conn
            .prepare(
                "SELECT dispatch_id, action_id, ordinal, adapter_id, request_hash,
                        provider_request_id, started_at_ms, ended_at_ms, transport_state, receipt_hash
                 FROM action_dispatches ORDER BY started_at_ms ASC, ordinal ASC",
            )
            .map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceDispatchRow {
                    dispatch_id: r.get(0)?,
                    action_id: r.get(1)?,
                    ordinal: r.get(2)?,
                    adapter_id: r.get(3)?,
                    request_hash: r.get(4)?,
                    provider_request_id: r.get(5)?,
                    started_at_ms: r.get(6)?,
                    ended_at_ms: r.get(7)?,
                    transport_state: r.get(8)?,
                    receipt_hash: r.get(9)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            let d = item.map_err(StoreError::from_rusqlite)?;
            if baseline.dispatches.contains(&d.dispatch_id) {
                continue;
            }
            if let (Ok(aid), Ok(req_h)) = (
                d.action_id.parse::<ActionId>(),
                d.request_hash.parse::<Sha256Digest>(),
            ) {
                let rcpt_h = d
                    .receipt_hash
                    .as_deref()
                    .map(str::parse::<Sha256Digest>)
                    .transpose()?;
                delta.capsule_dispatches.push(CapsuleDispatchDelta {
                    dispatch_id: d.dispatch_id.clone(),
                    action_id: aid,
                    ordinal: d.ordinal,
                    adapter_id: d.adapter_id.clone(),
                    request_hash: req_h,
                    provider_request_id: d.provider_request_id.clone(),
                    started_at_ms: TimestampMs::new(d.started_at_ms),
                    ended_at_ms: d.ended_at_ms.map(TimestampMs::new),
                    transport_state: d.transport_state.clone(),
                    receipt_hash: rcpt_h,
                });
            }
            delta.dispatches.push(d);
        }
    }

    // Tombstones de reçus et d'événements
    {
        let mut r_stmt = src_conn
            .prepare(
                "SELECT actor_id, command_id, committed_at_ms
                 FROM command_receipts WHERE response_state = 'tombstone'",
            )
            .map_err(StoreError::from_rusqlite)?;
        let r_rows = r_stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in r_rows {
            let (aid, cid, ts) = item.map_err(StoreError::from_rusqlite)?;
            let key = (aid.clone(), cid.clone());
            if baseline.receipts.contains(&key) && !baseline.full_receipts.contains(&key) {
                continue;
            }
            delta.tombstoned_receipts.push(key);
            delta.tombstones.push(CapsuleTombstoneDelta {
                kind: CapsuleTombstoneKind::CommandReceipt,
                entity_id: aid,
                sub_id: Some(cid),
                status: "tombstone".to_owned(),
                reason: Some("retention_receipt_tombstone".to_owned()),
                updated_at_ms: TimestampMs::new(ts),
            });
        }

        let mut ev_stmt = src_conn
            .prepare(
                "SELECT event_id, recorded_at_ms FROM events WHERE payload_state = 'tombstone'",
            )
            .map_err(StoreError::from_rusqlite)?;
        let ev_rows = ev_stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(StoreError::from_rusqlite)?;
        for item in ev_rows {
            let (ev_id, ts) = item.map_err(StoreError::from_rusqlite)?;
            delta.tombstoned_events.push(ev_id.clone());
            delta.tombstones.push(CapsuleTombstoneDelta {
                kind: CapsuleTombstoneKind::EventPayload,
                entity_id: ev_id,
                sub_id: None,
                status: "tombstone".to_owned(),
                reason: Some("event_tombstone".to_owned()),
                updated_at_ms: TimestampMs::new(ts),
            });
        }

        let mut od_stmt = src_conn
            .prepare(
                "SELECT delete_operation_id, object_hash, relative_path, reason,
                        state, created_at_ms, completed_at_ms
                 FROM object_deletions",
            )
            .map_err(StoreError::from_rusqlite)?;
        let od_rows = od_stmt
            .query_map([], |r| {
                Ok(SourceObjectDeletionRow {
                    delete_operation_id: r.get(0)?,
                    object_hash: r.get(1)?,
                    relative_path: r.get(2)?,
                    reason: r.get(3)?,
                    state: r.get(4)?,
                    created_at_ms: r.get(5)?,
                    completed_at_ms: r.get(6)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in od_rows {
            let od = item.map_err(StoreError::from_rusqlite)?;
            delta.tombstones.push(CapsuleTombstoneDelta {
                kind: CapsuleTombstoneKind::ObjectDeletion,
                entity_id: od.object_hash.clone(),
                sub_id: Some(od.delete_operation_id.clone()),
                status: od.state.clone(),
                reason: Some(od.reason.clone()),
                updated_at_ms: TimestampMs::new(od.completed_at_ms.unwrap_or(od.created_at_ms)),
            });
            delta.object_deletions.push(od);
        }
    }

    // Révocations de sécurité (secret_references & subject_suspensions) si schéma source >= V3
    if src_schema_ver >= 3 {
        let mut s_stmt = src_conn
            .prepare(
                "SELECT secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind,
                        target_kind, target_id, scope_kind, scope_id, backend_kind, status,
                        local_revoked_at_ms, remote_revocation_state, revision,
                        created_at_ms, updated_at_ms
                 FROM secret_references
                 WHERE status IN ('revoked_local', 'revoked_remote')",
            )
            .map_err(StoreError::from_rusqlite)?;
        let s_rows = s_stmt
            .query_map([], |r| {
                Ok(SourceSecretRevocationRow {
                    secret_ref_id: r.get(0)?,
                    opaque_secret_ref: r.get(1)?,
                    namespace: r.get(2)?,
                    purpose: r.get(3)?,
                    secret_kind: r.get(4)?,
                    target_kind: r.get(5)?,
                    target_id: r.get(6)?,
                    scope_kind: r.get(7)?,
                    scope_id: r.get(8)?,
                    backend_kind: r.get(9)?,
                    status: r.get(10)?,
                    local_revoked_at_ms: r.get(11)?,
                    remote_revocation_state: r.get(12)?,
                    revision: r.get(13)?,
                    created_at_ms: r.get(14)?,
                    updated_at_ms: r.get(15)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in s_rows {
            let sr = item.map_err(StoreError::from_rusqlite)?;
            delta.tombstones.push(CapsuleTombstoneDelta {
                kind: CapsuleTombstoneKind::SecretReferenceRevocation,
                entity_id: sr.secret_ref_id.clone(),
                sub_id: None,
                status: sr.status.clone(),
                reason: Some(sr.remote_revocation_state.clone()),
                updated_at_ms: TimestampMs::new(sr.updated_at_ms),
            });
            delta.secret_revocations.push(sr);
        }

        let mut subj_stmt = src_conn
            .prepare(
                "SELECT suspension_id, subject_kind, subject_id, scope_kind, scope_id, status,
                        reason, guard_propagation_state, acted_by_actor, revision,
                        created_at_ms, updated_at_ms
                 FROM subject_suspensions WHERE status = 'revoked'",
            )
            .map_err(StoreError::from_rusqlite)?;
        let subj_rows = subj_stmt
            .query_map([], |r| {
                Ok(SourceSubjectRevocationRow {
                    suspension_id: r.get(0)?,
                    subject_kind: r.get(1)?,
                    subject_id: r.get(2)?,
                    scope_kind: r.get(3)?,
                    scope_id: r.get(4)?,
                    status: r.get(5)?,
                    reason: r.get(6)?,
                    guard_propagation_state: r.get(7)?,
                    acted_by_actor: r.get(8)?,
                    revision: r.get(9)?,
                    created_at_ms: r.get(10)?,
                    updated_at_ms: r.get(11)?,
                })
            })
            .map_err(StoreError::from_rusqlite)?;
        for item in subj_rows {
            let sr = item.map_err(StoreError::from_rusqlite)?;
            delta.tombstones.push(CapsuleTombstoneDelta {
                kind: CapsuleTombstoneKind::SubjectRevocation,
                entity_id: format!("{}:{}", sr.subject_kind, sr.subject_id),
                sub_id: Some(sr.scope_id.clone()),
                status: sr.status.clone(),
                reason: Some(sr.reason.clone()),
                updated_at_ms: TimestampMs::new(sr.updated_at_ms),
            });
            delta.subject_revocations.push(sr);
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
    let existing_target_manifest = read_maintenance_manifest(&target_config.maintenance_dir)?;
    let src_maint_dir_opt = req
        .source_db_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join("maintenance"))
        .filter(|d| d != &target_config.maintenance_dir);
    let existing_src_manifest = src_maint_dir_opt
        .as_deref()
        .map(read_maintenance_manifest)
        .transpose()?
        .flatten();

    let mut prior_history: Vec<MaintenanceManifestEntry> = Vec::new();
    for m in [&existing_src_manifest, &existing_target_manifest]
        .into_iter()
        .flatten()
    {
        if !m.history.is_empty() {
            for entry in &m.history {
                if !prior_history
                    .iter()
                    .any(|e| e.restore_id == entry.restore_id)
                {
                    prior_history.push(entry.clone());
                }
            }
        } else if !prior_history.iter().any(|e| e.restore_id == m.restore_id) {
            prior_history.push(MaintenanceManifestEntry {
                restore_id: m.restore_id,
                command_receipt_id: m.command_receipt_id,
                actor_id: m.actor_id.clone(),
                command_id: m.command_id,
                command_digest: m.command_digest,
                backup_id: m.backup_id,
                source_authority_generation: m.source_authority_generation,
                state: m.state,
                blocked_reason: m.blocked_reason.clone(),
                updated_at_ms: m.updated_at_ms,
            });
        }
    }

    // Recherche idempotente dans le manifeste privé 0600 AVANT toute nouvelle allocation
    if let Some(prev) = prior_history
        .iter()
        .rev()
        .find(|e| e.actor_id == req.actor_id && e.command_id == req.command_id)
    {
        if prev.command_digest != req.command_digest {
            return Err(StoreError::IdempotencyConflict {
                command_id: req.command_id,
            });
        }
        if prev.state == RestoreRequestState::Blocked {
            return Err(StoreError::RestorationBlocked(
                prev.blocked_reason
                    .clone()
                    .unwrap_or_else(|| "restauration précédemment bloquée".to_owned()),
            ));
        }
    }

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

    let use_manifest_fallback = if req.source_db_unreadable {
        true
    } else {
        let Some(src_db) = req.source_db_path.as_ref() else {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "source_db_path requis quand source_db_unreadable = false".to_owned(),
            });
        };
        matches!(
            ensure_source_restore_tracking(
                src_db,
                req,
                val.backup_id,
                restore_id,
                receipt_id,
                now_ms,
            )?,
            SourceTrackOutcome::FallbackUnwritableManifest
        )
    };

    let current_entry = MaintenanceManifestEntry {
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

    let maint_manifest = if use_manifest_fallback {
        let mut history = prior_history.clone();
        history.push(current_entry.clone());
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
            history,
        };
        write_maintenance_manifest(&target_config.maintenance_dir, &m)?;
        Some(m)
    } else {
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
            if let Some(last) = m.history.last_mut() {
                last.state = RestoreRequestState::Blocked;
                last.blocked_reason = Some(reason.clone());
                last.updated_at_ms = m.updated_at_ms;
            }
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

    let (coverage, src_delta) = if use_manifest_fallback {
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
    let mut capsule_manifest_entries = prior_history.clone();
    if use_manifest_fallback {
        let mut handed = current_entry;
        handed.state = RestoreRequestState::HandedOver;
        handed.updated_at_ms = now_ms;
        capsule_manifest_entries.push(handed);
    }
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
        dispatches: src_delta.capsule_dispatches,
        tombstones: src_delta.tombstones,
        manifest_entries: capsule_manifest_entries.clone(),
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
    let mig_report = apply_pending_migrations_for_restore(
        &mut target_conn,
        target_config.max_supported_schema_version,
        val.backup_id,
        now_ms,
        target_config.cutpoints.as_ref(),
    )?;
    let target_schema_ver = mig_report.current_version.clamp(1, CURRENT_SCHEMA_VERSION);

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
             boot_identity = ?3, stream_id = ?4, schema_version = ?5,
             mode = 'read_only_recovery', started_at_ms = ?6
         WHERE singleton = 1",
        rusqlite::params![
            target_auth_s,
            new_inst,
            new_boot,
            target_stream_s,
            target_schema_ver,
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
        if target_schema_ver >= 3 {
            let valid_proj = match act.project_id.as_deref() {
                Some(pid)
                    if fk_row_exists(
                        &tx,
                        "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
                        pid,
                    )? =>
                {
                    Some(pid.to_owned())
                }
                _ => None,
            };
            tx.execute(
                "INSERT INTO actions
                 (action_id, origin_kind, control_principal_id, project_id, scope_key, run_id, step_id,
                  invocation_id, attempt_id, replay_worker_id, action_type, payload_hash, authorization_ref,
                  policy_revision, resource_lease_id, approval_scope, subject_kind, subject_id,
                  authority_generation, daemon_epoch, fence, state, reconcile_strategy,
                  provider_idempotency_key, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?26)
                 ON CONFLICT(action_id) DO UPDATE SET
                   state = excluded.state,
                   revision = MAX(actions.revision, excluded.revision)",
                rusqlite::params![
                    act.action_id,
                    act.origin_kind,
                    act.control_principal_id,
                    valid_proj,
                    act.scope_key,
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
                    act.approval_scope,
                    act.subject_kind,
                    act.subject_id,
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
        } else {
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
    }

    for d in &src_delta.dispatches {
        let act_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM actions WHERE action_id = ?1",
                [d.action_id.as_str()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if act_exists > 0 {
            tx.execute(
                "INSERT OR IGNORE INTO action_dispatches
                 (dispatch_id, action_id, ordinal, adapter_id, request_hash, provider_request_id,
                  started_at_ms, ended_at_ms, transport_state, receipt_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    d.dispatch_id,
                    d.action_id,
                    d.ordinal,
                    d.adapter_id,
                    d.request_hash,
                    d.provider_request_id,
                    d.started_at_ms,
                    d.ended_at_ms,
                    d.transport_state,
                    d.receipt_hash
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    // Importer les tombstones & révocations post-snapshot
    for (aid, cid) in &src_delta.tombstoned_receipts {
        tx.execute(
            "UPDATE command_receipts
             SET response_state = 'tombstone', response_json = NULL
             WHERE actor_id = ?1 AND command_id = ?2",
            rusqlite::params![aid, cid],
        )
        .map_err(StoreError::from_rusqlite)?;
    }
    for ev_id in &src_delta.tombstoned_events {
        tx.execute(
            "UPDATE events
             SET payload_state = 'tombstone', payload_json = '{\"tombstoned\":true}'
             WHERE event_id = ?1",
            [ev_id],
        )
        .map_err(StoreError::from_rusqlite)?;
    }
    for od in &src_delta.object_deletions {
        let obj_exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM object_records WHERE object_hash = ?1",
                [od.object_hash.as_str()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if obj_exists > 0 {
            tx.execute(
                "UPDATE object_references
                 SET state = 'tombstone', tombstone_reason = COALESCE(tombstone_reason, ?1)
                 WHERE object_hash = ?2 AND state = 'live'",
                rusqlite::params![od.reason, od.object_hash],
            )
            .map_err(StoreError::from_rusqlite)?;
            tx.execute(
                "UPDATE object_records
                 SET state = 'tombstone', delete_operation_id = ?1
                 WHERE object_hash = ?2",
                rusqlite::params![od.delete_operation_id, od.object_hash],
            )
            .map_err(StoreError::from_rusqlite)?;
            if od.state == "completed"
                && let Ok(dst_obj) =
                    safe_artifact_path(&target_config.artifacts_dir, &od.relative_path)
            {
                let _ = fs::remove_file(dst_obj);
            }
        }
    }
    if target_schema_ver >= 3 {
        for sr in &src_delta.secret_revocations {
            tx.execute(
                "INSERT INTO secret_references
                 (secret_ref_id, opaque_secret_ref, namespace, purpose, secret_kind, target_kind,
                  target_id, scope_kind, scope_id, backend_kind, status, local_revoked_at_ms,
                  remote_revocation_state, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(secret_ref_id) DO UPDATE SET
                   status = excluded.status,
                   local_revoked_at_ms = COALESCE(excluded.local_revoked_at_ms, secret_references.local_revoked_at_ms),
                   remote_revocation_state = excluded.remote_revocation_state,
                   revision = MAX(secret_references.revision, excluded.revision),
                   updated_at_ms = excluded.updated_at_ms",
                rusqlite::params![
                    sr.secret_ref_id,
                    sr.opaque_secret_ref,
                    sr.namespace,
                    sr.purpose,
                    sr.secret_kind,
                    sr.target_kind,
                    sr.target_id,
                    sr.scope_kind,
                    sr.scope_id,
                    sr.backend_kind,
                    sr.status,
                    sr.local_revoked_at_ms,
                    sr.remote_revocation_state,
                    sr.revision,
                    sr.created_at_ms,
                    sr.updated_at_ms
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
        for sr in &src_delta.subject_revocations {
            tx.execute(
                "INSERT INTO subject_suspensions
                 (suspension_id, subject_kind, subject_id, scope_kind, scope_id, status, reason,
                  guard_propagation_state, acted_by_actor, revision, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'revoked', ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(subject_kind, subject_id, scope_kind, scope_id) DO UPDATE SET
                   status = 'revoked',
                   reason = excluded.reason,
                   guard_propagation_state = excluded.guard_propagation_state,
                   acted_by_actor = excluded.acted_by_actor,
                   revision = MAX(subject_suspensions.revision, excluded.revision),
                   updated_at_ms = excluded.updated_at_ms",
                rusqlite::params![
                    sr.suspension_id,
                    sr.subject_kind,
                    sr.subject_id,
                    sr.scope_kind,
                    sr.scope_id,
                    sr.reason,
                    sr.guard_propagation_state,
                    sr.acted_by_actor,
                    sr.revision,
                    sr.created_at_ms,
                    sr.updated_at_ms
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
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
    let imported_manifest_json = if capsule_manifest_entries.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&capsule_manifest_entries)?)
    };
    if target_schema_ver >= 3 {
        tx.execute(
            "INSERT INTO restore_imports
             (restore_id, command_receipt_id, backup_id, source_authority_generation, target_authority_generation,
              capsule_object_hash, coverage, state, frozen_scopes_json, imported_manifest_json, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'prepared', ?8, ?9, 0)",
            rusqlite::params![
                restore_id.to_string(),
                target_rcpt_str,
                val.backup_id.to_string(),
                src_auth_s,
                target_auth_s,
                capsule_hash.as_prefixed_str(),
                coverage.as_str(),
                frozen_json,
                imported_manifest_json
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
        for (idx, me) in capsule_manifest_entries.iter().enumerate() {
            let entry_id = format!("{restore_id}:{idx}");
            tx.execute(
                "INSERT INTO restore_manifest_records
                 (entry_id, imported_by_restore_id, source_restore_id, command_receipt_id,
                  actor_id, command_id, command_digest, backup_id, source_authority_generation,
                  state, blocked_reason, updated_at_ms, imported_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    entry_id,
                    restore_id.to_string(),
                    me.restore_id.to_string(),
                    me.command_receipt_id.to_string(),
                    me.actor_id,
                    me.command_id.to_string(),
                    me.command_digest.as_prefixed_str(),
                    me.backup_id.to_string(),
                    me.source_authority_generation.to_string(),
                    me.state.as_str(),
                    me.blocked_reason,
                    me.updated_at_ms.as_i64(),
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    } else {
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
    }

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
        if let Some(last) = m.history.last_mut() {
            last.state = RestoreRequestState::HandedOver;
            last.updated_at_ms = m.updated_at_ms;
        }
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
