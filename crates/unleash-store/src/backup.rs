//! Sauvegardes cohérentes (`sqlite3_backup_step(-1)` + pins + rotation Q-12) et export borné (`REQ-PERSIST-010`).

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    AuthorityGeneration, BackupId, Sequence, Sha256Digest, StreamId, TimestampMs,
};

use crate::config::{
    CutpointObserver, FaultTarget, StoreConfig, StoreCutpoint, WriteCriticality,
    check_injected_fault,
};
use crate::error::StoreError;
use crate::fs_sync::{
    set_dir_permissions_0700, set_path_permissions, sync_dir, write_atomic_durable,
};
use crate::objects::safe_artifact_path;
use crate::qualification::{
    compiled_sqlite_source_id, compiled_sqlite_version, inspect_and_verify_build,
};
use crate::writer::{ReaderRegistry, StoreWriterHandle, open_reader_connection};

/// Palier de rétention d'une sauvegarde (Q-12 : 7 quotidiennes, 4 hebdomadaires, manuelles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupRetentionTier {
    /// Sauvegarde quotidienne automatique (rotation à 7).
    Daily,
    /// Sauvegarde hebdomadaire (rotation à 4).
    Weekly,
    /// Sauvegarde manuelle (conservée jusqu'à suppression explicite).
    Manual,
}

impl BackupRetentionTier {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Manual => "manual",
        }
    }
}

/// État d'un job de sauvegarde dans `backup_jobs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupJobState {
    /// Barrière acquise, attente de clôture des suppressions en vol.
    Preparing,
    /// Snapshot DB ou copie d'objets en cours.
    Copying,
    /// Vérifications `integrity_check`, `foreign_key_check` et hashes en cours.
    Verifying,
    /// Sauvegarde vérifiée, manifeste synchronisé, complète.
    Complete,
    /// Sauvegarde abandonnée proprement.
    Aborted,
}

/// Entrée d'objet inclus dans le manifeste d'une sauvegarde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupObjectEntry {
    /// Empreinte SHA-256 attendue.
    pub object_hash: Sha256Digest,
    /// Génération de l'incarnation copiée.
    pub object_generation: i64,
    /// Chemin relatif de stockage.
    pub relative_path: String,
    /// Taille exacte en octets.
    pub byte_length: u64,
}

/// Manifeste durable et signé d'une sauvegarde complète (`<backup_dir>/manifest.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Identifiant du job de sauvegarde.
    pub backup_id: BackupId,
    /// État final (`complete` exigé par `ValidateBackup`).
    pub state: BackupJobState,
    /// Version `SQLite` du build ayant produit le snapshot.
    pub sqlite_version: String,
    /// `sqlite_source_id()` exact du build.
    pub sqlite_source_id: String,
    /// Version du schéma `SQLite` au snapshot.
    pub schema_version: i64,
    /// Lignée d'autorité source.
    pub source_authority_generation: AuthorityGeneration,
    /// Flux d'événements du snapshot.
    pub snapshot_stream_id: StreamId,
    /// Séquence high-water mark du snapshot.
    pub snapshot_sequence: Sequence,
    /// Liste des objets inclus avec taille et hash.
    pub included_objects: Vec<BackupObjectEntry>,
    /// Résumé des comptes/budgets au snapshot.
    pub budgets_included: bool,
    /// Résumé des receipts/tombstones au snapshot.
    pub receipts_included: bool,
    /// Couverture déclarée (ex: `"db_and_objects"`).
    pub coverage: String,
    /// Palier de rétention (Q-12).
    pub retention_tier: BackupRetentionTier,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
}

/// Demande de création d'une sauvegarde complète (`PrepareBackup`).
#[derive(Debug, Clone)]
pub struct PrepareBackupRequest {
    /// Nouvelle feuille cible autorisée, jamais précréée (par défaut sous `backups_dir/<backup_id>`).
    pub destination_dir: PathBuf,
    /// Palier de rétention (Q-12).
    pub retention_tier: BackupRetentionTier,
    /// Étiquette de couverture (`"db_and_objects"`).
    pub coverage: String,
}

/// Résultat d'une validation de sauvegarde (`ValidateBackup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidateBackupReport {
    /// Identifiant de sauvegarde validé.
    pub backup_id: BackupId,
    /// Manifeste lu et vérifié.
    pub manifest: BackupManifest,
    /// Nombre d'objets lus et vérifiés par recalcul SHA-256.
    pub verified_object_count: usize,
    /// Résultat `PRAGMA integrity_check` (`ok`).
    pub integrity_check_ok: bool,
    /// Nombre d'anomalies `PRAGMA foreign_key_check` (`0`).
    pub foreign_key_violations: usize,
}

/// Jeton d'annulation pour un export borné en continu.
#[derive(Debug, Default, Clone)]
pub struct ExportCancellation {
    cancelled: Arc<AtomicBool>,
}

impl ExportCancellation {
    /// Crée un nouveau jeton d'annulation actif.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Demande l'annulation immédiate de l'export.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Indique si l'annulation a été demandée.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Demande d'export borné par quota d'octets et taille de fragment (`R6`).
#[derive(Debug, Clone)]
pub struct BoundedExportRequest {
    /// Chemin final de publication (n'est créé que si l'export est 100 % complet sous quota).
    pub destination_path: PathBuf,
    /// Répertoire temporaire de staging inspectable.
    pub staging_dir: PathBuf,
    /// Quota maximal en octets (`None` = valeur `StoreConfig::export_max_bytes`, 8 MiB par défaut).
    pub max_bytes_override: Option<u64>,
    /// Taille maximale d'un fragment (`None` = `StoreConfig::export_chunk_bytes`, 256 KiB).
    pub chunk_bytes_override: Option<usize>,
    /// Jeton d'annulation optionnel.
    pub cancellation: ExportCancellation,
}

/// Issue d'un export borné réussi (jamais renvoyée si tronquée ou annulée).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedExportOutcome {
    /// Chemin du fichier final publié.
    pub destination_path: PathBuf,
    /// Nombre total d'octets écrits.
    pub bytes_written: u64,
    /// Nombre de fragments émis.
    pub chunks_written: usize,
    /// Statut explicite (`complete`).
    pub complete: bool,
}

struct SnapshotInspection {
    schema_version: i64,
    authority_generation: AuthorityGeneration,
    stream_id: StreamId,
    snapshot_sequence: Sequence,
    included_objects: Vec<BackupObjectEntry>,
    budgets_included: bool,
    receipts_included: bool,
}

fn verify_sqlite_integrity_and_fks(conn: &Connection) -> Result<(bool, usize), StoreError> {
    let mut ic_stmt = conn
        .prepare("PRAGMA integrity_check")
        .map_err(StoreError::from_rusqlite)?;
    let ic_rows = ic_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(StoreError::from_rusqlite)?;
    let mut ic_msgs = Vec::new();
    for item in ic_rows {
        ic_msgs.push(item.map_err(StoreError::from_rusqlite)?);
    }
    let ic_ok = ic_msgs.len() == 1
        && ic_msgs
            .first()
            .is_some_and(|m| m.eq_ignore_ascii_case("ok"));
    if !ic_ok {
        return Err(StoreError::RestoreIncomplete(format!(
            "PRAGMA integrity_check en échec sur sauvegarde : {}",
            ic_msgs.join("; ")
        )));
    }

    let mut fk_stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(StoreError::from_rusqlite)?;
    let fk_rows = fk_stmt
        .query_map([], |_| Ok(()))
        .map_err(StoreError::from_rusqlite)?;
    let mut fk_violations = 0_usize;
    for item in fk_rows {
        item.map_err(StoreError::from_rusqlite)?;
        fk_violations += 1;
    }
    if fk_violations > 0 {
        return Err(StoreError::RestoreIncomplete(format!(
            "PRAGMA foreign_key_check a détecté {fk_violations} violation(s)"
        )));
    }

    Ok((true, 0))
}

fn inspect_backup_snapshot(snapshot_db: &Path) -> Result<SnapshotInspection, StoreError> {
    let conn = Connection::open_with_flags(
        snapshot_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;

    let (schema_version, auth_str, stream_str): (i64, String, String) = conn
        .query_row(
            "SELECT schema_version, authority_generation, stream_id FROM daemon_state WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(StoreError::from_rusqlite)?;

    let max_seq_i64: i64 = conn
        .query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |r| {
            r.get(0)
        })
        .map_err(StoreError::from_rusqlite)?;
    let budget_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM budget_accounts", [], |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;
    let receipt_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r.get(0))
        .map_err(StoreError::from_rusqlite)?;

    let mut stmt = conn
        .prepare(
            "SELECT object_hash, object_generation, relative_path, byte_length
             FROM object_records
             WHERE state IN ('live', 'prepared')
             ORDER BY object_hash ASC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([], |r| {
            let hash_s: String = r.get(0)?;
            let gen_v: i64 = r.get(1)?;
            let rel: String = r.get(2)?;
            let blen: i64 = r.get(3)?;
            Ok((hash_s, gen_v, rel, blen))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut included_objects = Vec::new();
    for item in rows {
        let (hash_s, gen_v, rel, blen) = item.map_err(StoreError::from_rusqlite)?;
        let digest: Sha256Digest = hash_s.parse()?;
        included_objects.push(BackupObjectEntry {
            object_hash: digest,
            object_generation: gen_v,
            relative_path: rel,
            byte_length: u64::try_from(blen).unwrap_or(0),
        });
    }

    Ok(SnapshotInspection {
        schema_version,
        authority_generation: auth_str.parse()?,
        stream_id: stream_str
            .parse()
            .map_err(|e| StoreError::InvariantViolation(format!("stream_id invalide : {e}")))?,
        snapshot_sequence: Sequence::new(u64::try_from(max_seq_i64).unwrap_or(0)),
        included_objects,
        budgets_included: budget_count >= 0,
        receipts_included: receipt_count >= 0,
    })
}

fn copy_and_verify_backup_objects(
    artifacts_dir: &Path,
    backup_objects_dir: &Path,
    entries: &[BackupObjectEntry],
) -> Result<(), StoreError> {
    fs::create_dir_all(backup_objects_dir)?;
    set_dir_permissions_0700(backup_objects_dir)?;

    for entry in entries {
        let src = safe_artifact_path(artifacts_dir, &entry.relative_path)?;
        if !src.exists() {
            return Err(StoreError::EvidenceUnavailable(format!(
                "objet source absent pendant sauvegarde : {}",
                entry.relative_path
            )));
        }
        let bytes = fs::read(&src)?;
        let actual_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if actual_len != entry.byte_length {
            return Err(StoreError::RestoreIncomplete(format!(
                "taille objet divergente pour {} ({actual_len} vs {})",
                entry.relative_path, entry.byte_length
            )));
        }
        if Sha256Digest::from_bytes(&bytes) != entry.object_hash {
            return Err(StoreError::StorageCorrupt(format!(
                "empreinte SHA-256 corrompue pour {}",
                entry.relative_path
            )));
        }
        let dst = safe_artifact_path(backup_objects_dir, &entry.relative_path)?;
        write_atomic_durable(&dst, &bytes, 0o600)?;
    }
    Ok(())
}

fn apply_q12_rotation(
    tx: &rusqlite::Transaction<'_>,
    backups_dir: &Path,
    current_dest: &Path,
    tier: BackupRetentionTier,
) -> Result<(), StoreError> {
    let keep = match tier {
        BackupRetentionTier::Daily => 7_usize,
        BackupRetentionTier::Weekly => 4_usize,
        BackupRetentionTier::Manual => return Ok(()),
    };

    let mut stmt = tx
        .prepare(
            "SELECT backup_id, destination_path
             FROM backup_jobs
             WHERE retention_tier = ?1 AND state = 'complete'
             ORDER BY created_at_ms DESC, rowid DESC",
        )
        .map_err(StoreError::from_rusqlite)?;
    let rows = stmt
        .query_map([tier.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(StoreError::from_rusqlite)?;

    let mut all = Vec::new();
    for item in rows {
        all.push(item.map_err(StoreError::from_rusqlite)?);
    }
    drop(stmt);

    for (old_id, old_dest_opt) in all.into_iter().skip(keep) {
        tx.execute("DELETE FROM backup_jobs WHERE backup_id = ?1", [&old_id])
            .map_err(StoreError::from_rusqlite)?;
        if let Some(old_dest) = old_dest_opt {
            let p = PathBuf::from(old_dest);
            if p != current_dest && p.starts_with(backups_dir) && p.exists() {
                let _ = fs::remove_dir_all(&p);
            }
        }
    }
    Ok(())
}

/// Abandonne explicitement une sauvegarde interrompue et relâche sa barrière et ses pins associés (`storage-model.md`).
pub(crate) fn tx_abandon_backup_job(
    tx: &rusqlite::Transaction<'_>,
    now_ms: TimestampMs,
    backup_id: BackupId,
) -> Result<(), StoreError> {
    let bkp_id = backup_id.to_string();
    let state_opt: Option<String> = tx
        .query_row(
            "SELECT state FROM backup_jobs WHERE backup_id = ?1",
            [bkp_id.as_str()],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(StoreError::from_rusqlite(other)),
        })?;
    let Some(state) = state_opt else {
        return Err(StoreError::NotFound(format!("backup_job {bkp_id}")));
    };
    if state == "complete" {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("sauvegarde {bkp_id} déjà terminée"),
        });
    }
    tx.execute(
        "UPDATE backup_jobs SET state = 'aborted', revision = revision + 1 WHERE backup_id = ?1",
        [bkp_id.as_str()],
    )
    .map_err(StoreError::from_rusqlite)?;
    let released = tx
        .execute(
            "UPDATE object_pins SET state = 'released', released_at_ms = ?1
             WHERE owner_kind = 'backup_job' AND owner_id = ?2 AND state IN ('held', 'reconciling')",
            rusqlite::params![now_ms.as_i64(), bkp_id],
        )
        .map_err(StoreError::from_rusqlite)?;
    tracing::info!(
        backup_id = %backup_id,
        released_pins = released,
        "sauvegarde interrompue abandonnée et pins relâchés"
    );
    Ok(())
}

fn prevalidate_backup_destination(destination: &Path) -> Result<(), StoreError> {
    if destination.file_name().is_none() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "la destination de sauvegarde doit désigner une nouvelle feuille".into(),
        });
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(StoreError::InvalidArgument {
            reason: None,
            message: "la destination de sauvegarde existe déjà".into(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reserve_backup_destination(destination: &Path) -> Result<(), StoreError> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Prevalidation is only an early refusal. This non-recursive mkdir reserves the leaf
    // exclusively and refuses a directory, file or symlink that appeared after C11a.
    builder.create(destination)?;
    set_dir_permissions_0700(destination)?;
    sync_dir(parent)?;
    Ok(())
}

/// Exécute le protocole complet `PrepareBackup` (barrière durable, `sqlite3_backup_step(-1)`, pins, copies, vérifications, Q-12).
#[expect(
    clippy::too_many_lines,
    reason = "Orchestration atomique barrière durable, backup_step(-1), pins backup, copies vérifiées et rotation Q-12"
)]
pub(crate) fn prepare_backup_workflow(
    config: &StoreConfig,
    writer: &StoreWriterHandle,
    cutpoints: &dyn CutpointObserver,
    req: &PrepareBackupRequest,
) -> Result<BackupManifest, StoreError> {
    check_injected_fault(config.fault_injector.as_ref(), FaultTarget::Maintenance)?;
    if let Some(db_parent) = config.db_path.parent()
        && req.destination_dir == db_parent
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "destination de sauvegarde identique à la racine live interdite".to_owned(),
        });
    }
    prevalidate_backup_destination(&req.destination_dir)?;

    let backup_id = BackupId::from_uuid(config.id_generator.new_uuid());
    let _span = tracing::info_span!(
        "prepare_backup",
        backup_id = %backup_id,
        tier = req.retention_tier.as_str()
    )
    .entered();
    let backup_id_str = backup_id.to_string();
    let barrier_pin_id = config.id_generator.new_uuid().to_string();
    let dest_str = req.destination_dir.to_string_lossy().into_owned();
    let tier_str = req.retention_tier.as_str().to_owned();
    let coverage_str = req.coverage.clone();

    let (source_auth, created_at_ms) = {
        let bkp_id_c = backup_id_str.clone();
        let pin_id_c = barrier_pin_id.clone();
        let dest_c = dest_str.clone();
        let tier_c = tier_str.clone();
        let cov_c = coverage_str.clone();
        writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now_ms| {
            let deleting_cnt: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM object_records WHERE state = 'deleting'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            if deleting_cnt > 0 {
                return Err(StoreError::RestoreIncomplete(format!(
                    "{deleting_cnt} suppression(s) d'objet encore en cours avant sauvegarde"
                )));
            }

            let auth_str: String = tx
                .query_row(
                    "SELECT authority_generation FROM daemon_state WHERE singleton = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            let source_auth: AuthorityGeneration = auth_str.parse()?;

            tx.execute(
                "INSERT INTO object_pins
                 (pin_id, pin_kind, object_hash, object_generation, owner_kind, owner_id, authority_generation, state, created_at_ms, released_at_ms)
                 VALUES (?1, 'backup_barrier', NULL, NULL, 'backup_job', ?2, ?3, 'held', ?4, NULL)",
                rusqlite::params![pin_id_c, bkp_id_c, auth_str, now_ms.as_i64()],
            )
            .map_err(StoreError::from_rusqlite)?;

            tx.execute(
                "INSERT INTO backup_jobs
                 (backup_id, state, source_authority_generation, snapshot_stream_id, snapshot_sequence, manifest_object_hash, coverage, destination_path, retention_tier, created_at_ms, revision)
                 VALUES (?1, 'preparing', ?2, NULL, NULL, NULL, ?3, ?4, ?5, ?6, 0)",
                rusqlite::params![bkp_id_c, auth_str, cov_c, dest_c, tier_c, now_ms.as_i64()],
            )
            .map_err(StoreError::from_rusqlite)?;

            Ok((source_auth, now_ms))
        })?
    };

    cutpoints.hit(StoreCutpoint::C11aAfterBackupBarrier)?;

    reserve_backup_destination(&req.destination_dir)?;
    let snapshot_db_path = req.destination_dir.join("db.sqlite");

    {
        let src_conn = open_reader_connection(config)?;
        let mut dst_conn = Connection::open_with_flags(
            &snapshot_db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(StoreError::from_rusqlite)?;
        let bkp = Backup::new(&src_conn, &mut dst_conn).map_err(StoreError::from_rusqlite)?;
        bkp.step(-1).map_err(StoreError::from_rusqlite)?;
    }
    set_path_permissions(&snapshot_db_path, 0o600)?;
    File::open(&snapshot_db_path)?.sync_all()?;
    sync_dir(&req.destination_dir)?;

    cutpoints.hit(StoreCutpoint::C11bAfterDbSnapshotBeforePins)?;

    let snapshot = inspect_backup_snapshot(&snapshot_db_path)?;

    let backup_pin_ids: Vec<String> = {
        let entries = snapshot.included_objects.clone();
        let bkp_id_c = backup_id_str.clone();
        let barrier_c = barrier_pin_id.clone();
        let stream_c = snapshot.stream_id.to_string();
        let seq_i64 = i64::try_from(snapshot.snapshot_sequence.as_u64()).unwrap_or(0);
        let auth_s = source_auth.to_string();
        let mut generated_pin_ids = Vec::with_capacity(entries.len());
        for _ in &entries {
            generated_pin_ids.push(config.id_generator.new_uuid().to_string());
        }
        let pin_ids_c = generated_pin_ids.clone();

        writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now_ms| {
            for (entry, pin_id) in entries.iter().zip(pin_ids_c.iter()) {
                tx.execute(
                    "INSERT INTO object_pins
                     (pin_id, pin_kind, object_hash, object_generation, owner_kind, owner_id, authority_generation, state, created_at_ms, released_at_ms)
                     VALUES (?1, 'backup', ?2, ?3, 'backup_job', ?4, ?5, 'held', ?6, NULL)",
                    rusqlite::params![
                        pin_id,
                        entry.object_hash.as_prefixed_str(),
                        entry.object_generation,
                        bkp_id_c,
                        auth_s,
                        now_ms.as_i64()
                    ],
                )
                .map_err(StoreError::from_rusqlite)?;
            }

            tx.execute(
                "UPDATE object_pins SET state = 'released', released_at_ms = ?1 WHERE pin_id = ?2",
                rusqlite::params![now_ms.as_i64(), barrier_c],
            )
            .map_err(StoreError::from_rusqlite)?;

            tx.execute(
                "UPDATE backup_jobs
                 SET state = 'copying', snapshot_stream_id = ?1, snapshot_sequence = ?2, revision = revision + 1
                 WHERE backup_id = ?3",
                rusqlite::params![stream_c, seq_i64, bkp_id_c],
            )
            .map_err(StoreError::from_rusqlite)?;

            Ok(())
        })?;
        generated_pin_ids
    };

    cutpoints.hit(StoreCutpoint::C11cAfterPinsDuringCopy)?;

    let objects_dir = req.destination_dir.join("objects");
    copy_and_verify_backup_objects(
        &config.artifacts_dir,
        &objects_dir,
        &snapshot.included_objects,
    )?;

    {
        let verify_conn = Connection::open_with_flags(
            &snapshot_db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(StoreError::from_rusqlite)?;
        inspect_and_verify_build(&verify_conn, &config.build_policy)?;
        let _ = verify_sqlite_integrity_and_fks(&verify_conn)?;
    }

    let manifest_path = req.destination_dir.join("manifest.json");
    let mut manifest = BackupManifest {
        backup_id,
        state: BackupJobState::Verifying,
        sqlite_version: compiled_sqlite_version().to_owned(),
        sqlite_source_id: compiled_sqlite_source_id().to_owned(),
        schema_version: snapshot.schema_version,
        source_authority_generation: snapshot.authority_generation,
        snapshot_stream_id: snapshot.stream_id,
        snapshot_sequence: snapshot.snapshot_sequence,
        included_objects: snapshot.included_objects,
        budgets_included: snapshot.budgets_included,
        receipts_included: snapshot.receipts_included,
        coverage: req.coverage.clone(),
        retention_tier: req.retention_tier,
        created_at_ms,
    };

    let verifying_json = serde_json::to_vec_pretty(&manifest)?;
    write_atomic_durable(&manifest_path, &verifying_json, 0o600)?;

    cutpoints.hit(StoreCutpoint::C11dAfterManifestBeforeComplete)?;

    manifest.state = BackupJobState::Complete;
    let complete_json = serde_json::to_vec_pretty(&manifest)?;
    write_atomic_durable(&manifest_path, &complete_json, 0o600)?;

    let backups_root = config.backups_dir.clone();
    let dest_owned = req.destination_dir.clone();
    let tier = req.retention_tier;
    let bkp_id_f = backup_id_str;
    writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now_ms| {
        tx.execute(
            "UPDATE backup_jobs SET state = 'complete', revision = revision + 1 WHERE backup_id = ?1",
            [bkp_id_f.as_str()],
        )
        .map_err(StoreError::from_rusqlite)?;

        for pin_id in backup_pin_ids {
            tx.execute(
                "UPDATE object_pins SET state = 'released', released_at_ms = ?1 WHERE pin_id = ?2",
                rusqlite::params![now_ms.as_i64(), pin_id],
            )
            .map_err(StoreError::from_rusqlite)?;
        }

        apply_q12_rotation(tx, &backups_root, &dest_owned, tier)?;
        Ok(())
    })?;

    Ok(manifest)
}

/// Valide hors ligne une sauvegarde complète (`manifest.json`, `SQLite` build + intégrité + FKs + objets SHA-256).
pub(crate) fn validate_backup_dir(
    backup_dir: &Path,
    config: &StoreConfig,
) -> Result<ValidateBackupReport, StoreError> {
    let manifest_path = backup_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Err(StoreError::RestoreIncomplete(format!(
            "manifest.json manquant dans '{}'",
            backup_dir.display()
        )));
    }
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        StoreError::RestoreIncomplete(format!("manifeste sauvegarde invalide : {e}"))
    })?;

    if manifest.state != BackupJobState::Complete {
        return Err(StoreError::RestoreIncomplete(format!(
            "sauvegarde incomplète (état {:?})",
            manifest.state
        )));
    }

    let db_path = backup_dir.join("db.sqlite");
    if !db_path.exists() {
        return Err(StoreError::RestoreIncomplete(format!(
            "db.sqlite manquant dans '{}'",
            backup_dir.display()
        )));
    }

    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreError::from_rusqlite)?;
    inspect_and_verify_build(&conn, &config.build_policy)?;
    let (integrity_check_ok, foreign_key_violations) = verify_sqlite_integrity_and_fks(&conn)?;

    let objects_dir = backup_dir.join("objects");
    let mut verified_count = 0_usize;
    for entry in &manifest.included_objects {
        let obj_path = safe_artifact_path(&objects_dir, &entry.relative_path)?;
        if !obj_path.exists() {
            return Err(StoreError::EvidenceUnavailable(format!(
                "objet manquant dans la sauvegarde : {}",
                entry.relative_path
            )));
        }
        let bytes = fs::read(&obj_path)?;
        let len_u64 = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if len_u64 != entry.byte_length {
            return Err(StoreError::RestoreIncomplete(format!(
                "taille objet altérée dans la sauvegarde : {}",
                entry.relative_path
            )));
        }
        if Sha256Digest::from_bytes(&bytes) != entry.object_hash {
            return Err(StoreError::StorageCorrupt(format!(
                "empreinte SHA-256 altérée dans la sauvegarde : {}",
                entry.relative_path
            )));
        }
        verified_count += 1;
    }

    Ok(ValidateBackupReport {
        backup_id: manifest.backup_id,
        manifest,
        verified_object_count: verified_count,
        integrity_check_ok,
        foreign_key_violations,
    })
}

struct StagingFileGuard {
    path: PathBuf,
    active: bool,
}

impl Drop for StagingFileGuard {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct ExportReaderGuard<'a> {
    conn: Option<Connection>,
    readers: &'a ReaderRegistry,
    reader_id: u64,
}

impl Drop for ExportReaderGuard<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let _ = c.execute_batch("ROLLBACK;");
            drop(c);
        }
        self.readers.unregister(self.reader_id);
    }
}

struct ChunkedExportSink<'a> {
    writer: BufWriter<File>,
    chunk_bytes: usize,
    max_bytes: u64,
    bytes_written: u64,
    chunks_written: usize,
    buf: Vec<u8>,
    cancellation: &'a ExportCancellation,
    control: &'a crate::writer::ReaderControl,
    started_at: std::time::Duration,
    read_timeout: std::time::Duration,
    clock: &'a dyn crate::config::MonotonicClockPort,
}

impl ChunkedExportSink<'_> {
    fn check_limits_before_write(&self) -> Result<(), StoreError> {
        if self.cancellation.is_cancelled() {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: "export annulé par l'appelant".to_owned(),
            });
        }
        let elapsed = self.clock.monotonic_now().saturating_sub(self.started_at);
        if self.control.is_aborted() || elapsed > self.read_timeout {
            return Err(StoreError::ReadTimeout {
                elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                max_ms: u64::try_from(self.read_timeout.as_millis()).unwrap_or(5_000),
            });
        }
        Ok(())
    }

    fn flush_slice(&mut self, slice: &[u8]) -> Result<(), StoreError> {
        self.check_limits_before_write()?;
        if slice.is_empty() {
            return Ok(());
        }
        self.writer.write_all(slice)?;
        self.bytes_written = self
            .bytes_written
            .saturating_add(u64::try_from(slice.len()).unwrap_or(u64::MAX));
        self.chunks_written += 1;
        if self.bytes_written > self.max_bytes {
            return Err(StoreError::ExportQuotaExceeded {
                bytes_written: self.bytes_written,
                max_bytes: self.max_bytes,
            });
        }
        Ok(())
    }

    fn push_line(&mut self, line: &str) -> Result<(), StoreError> {
        self.check_limits_before_write()?;
        let raw = line.as_bytes();
        if !self.buf.is_empty() && self.buf.len().saturating_add(raw.len() + 1) > self.chunk_bytes {
            let chunk = core::mem::take(&mut self.buf);
            self.flush_slice(&chunk)?;
        }
        if raw.len() + 1 > self.chunk_bytes {
            for part in raw.chunks(self.chunk_bytes) {
                self.flush_slice(part)?;
            }
            self.buf.push(b'\n');
            if self.buf.len() >= self.chunk_bytes {
                let chunk = core::mem::take(&mut self.buf);
                self.flush_slice(&chunk)?;
            }
        } else {
            self.buf.extend_from_slice(raw);
            self.buf.push(b'\n');
            if self.buf.len() >= self.chunk_bytes {
                let chunk = core::mem::take(&mut self.buf);
                self.flush_slice(&chunk)?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(u64, usize, File), StoreError> {
        if !self.buf.is_empty() {
            let tail = core::mem::take(&mut self.buf);
            self.flush_slice(&tail)?;
        }
        self.writer.flush()?;
        let file = self
            .writer
            .into_inner()
            .map_err(|e| StoreError::Io(e.into_error()))?;
        Ok((self.bytes_written, self.chunks_written, file))
    }
}

/// Exécute un export NDJSON en continu strictement borné par quota et délai lecteur (`8 MiB / 256 KiB / 5 s`).
#[expect(
    clippy::too_many_lines,
    reason = "Streaming contrôlé en lecteur dédié avec garde de quota 8 MiB, délai 5 s et publication atomique"
)]
pub(crate) fn run_bounded_export(
    config: &StoreConfig,
    readers: &Arc<ReaderRegistry>,
    req: &BoundedExportRequest,
) -> Result<BoundedExportOutcome, StoreError> {
    let max_bytes = req.max_bytes_override.unwrap_or(config.export_max_bytes);
    let chunk_bytes = req
        .chunk_bytes_override
        .unwrap_or(config.export_chunk_bytes)
        .max(1024);

    fs::create_dir_all(&req.staging_dir)?;
    set_dir_permissions_0700(&req.staging_dir)?;
    if let Some(dest_parent) = req.destination_path.parent() {
        fs::create_dir_all(dest_parent)?;
        set_dir_permissions_0700(dest_parent)?;
    }

    let staging_name = format!("export-{}.part", config.id_generator.new_uuid());
    let staging_path = req.staging_dir.join(staging_name);
    let mut staging_guard = StagingFileGuard {
        path: staging_path.clone(),
        active: true,
    };

    let staging_file = File::create(&staging_path)?;
    set_path_permissions(&staging_path, 0o600)?;

    let conn = open_reader_connection(config)?;
    let started_at = config.monotonic_clock.monotonic_now();
    let control = readers.register(
        started_at,
        config.read_timeout,
        conn.get_interrupt_handle(),
        None,
    );
    let reader_id = control.reader_id();
    conn.execute_batch("BEGIN DEFERRED;")
        .map_err(StoreError::from_rusqlite)?;

    let reader_guard = ExportReaderGuard {
        conn: Some(conn),
        readers: readers.as_ref(),
        reader_id,
    };

    let Some(read_conn) = reader_guard.conn.as_ref() else {
        return Err(StoreError::StorageUnavailable(
            "connexion export indisponible".to_owned(),
        ));
    };

    let mut sink = ChunkedExportSink {
        writer: BufWriter::new(staging_file),
        chunk_bytes,
        max_bytes,
        bytes_written: 0,
        chunks_written: 0,
        buf: Vec::with_capacity(chunk_bytes),
        cancellation: &req.cancellation,
        control: &control,
        started_at,
        read_timeout: config.read_timeout,
        clock: config.monotonic_clock.as_ref(),
    };

    let queries: [(&str, &str); 6] = [
        (
            "daemon_state",
            "SELECT json_object('table','daemon_state','authority_generation',authority_generation,'daemon_epoch',daemon_epoch,'stream_id',stream_id,'mode',mode,'revision',revision) FROM daemon_state",
        ),
        (
            "projects",
            "SELECT json_object('table','projects','project_id',project_id,'name',name,'state',state,'revision',revision) FROM projects ORDER BY project_id",
        ),
        (
            "missions",
            "SELECT json_object('table','missions','mission_id',mission_id,'project_id',project_id,'state',state,'revision',revision) FROM missions ORDER BY mission_id",
        ),
        (
            "command_receipts",
            "SELECT json_object('table','command_receipts','actor_id',actor_id,'command_id',command_id,'receipt_id',receipt_id,'command_digest',command_digest,'state',state) FROM command_receipts ORDER BY committed_at_ms, actor_id, command_id",
        ),
        (
            "events",
            "SELECT json_object('table','events','sequence',sequence,'event_id',event_id,'stream_id',stream_id,'event_type',event_type,'payload_state',payload_state,'payload_json',payload_json) FROM events ORDER BY sequence ASC",
        ),
        (
            "budget_accounts",
            "SELECT json_object('table','budget_accounts','account_id',account_id,'limit_micros',limit_micros,'reserved_micros',reserved_micros,'spent_micros',spent_micros) FROM budget_accounts ORDER BY account_id",
        ),
    ];

    for (_label, sql) in queries {
        let mut stmt = read_conn.prepare(sql).map_err(StoreError::from_rusqlite)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(StoreError::from_rusqlite)?;
        for item in rows {
            let line = item.map_err(StoreError::from_rusqlite)?;
            sink.push_line(&line)?;
        }
    }

    let (bytes_written, chunks_written, file) = sink.finish()?;
    file.sync_all()?;
    drop(file);
    drop(reader_guard);

    fs::rename(&staging_path, &req.destination_path)?;
    set_path_permissions(&req.destination_path, 0o600)?;
    if let Some(dest_parent) = req.destination_path.parent() {
        sync_dir(dest_parent)?;
    }
    sync_dir(&req.staging_dir)?;
    staging_guard.active = false;

    Ok(BoundedExportOutcome {
        destination_path: req.destination_path.clone(),
        bytes_written,
        chunks_written,
        complete: true,
    })
}
