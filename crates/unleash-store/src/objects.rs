//! Objets adressés par contenu hors base, pins et collecte exclusive (`REQ-PERSIST-006`, `C10a..d`).

use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use unleash_domain::{AuthorityGeneration, ReceiptId, Sha256Digest, TimestampMs};

use crate::config::{
    CutpointObserver, FaultTarget, StoreConfig, StoreCutpoint, WriteCriticality,
    check_injected_fault,
};
use crate::error::StoreError;
use crate::fs_sync::{sync_dir, write_atomic_durable};
use crate::writer::StoreWriterHandle;

/// État d'un enregistrement d'objet dans `object_records`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectState {
    /// Préparé sous pin avant écriture/publication complète.
    Prepared,
    /// Octets durables vérifiés et référence publiable.
    Live,
    /// Collecte engagée : tout pin ou publication concurrent doit échouer (`OBJECT_DELETING`).
    Deleting,
    /// Supprimé physiquement et synchronisé.
    Tombstone,
    /// Divergence d'octets ou fichier étranger détecté : mis en quarantaine sans unlink.
    Quarantined,
}

/// Nature d'un pin dans `object_pins`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    /// Pin protégeant la préparation d'octets avant publication.
    Preparation,
    /// Pin de lecture prolongée.
    Read,
    /// Pin d'une copie de sauvegarde sur une incarnation précise.
    Backup,
    /// Barrière globale de sauvegarde empêchant toute nouvelle suppression.
    BackupBarrier,
}

/// État d'un pin d'objet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    /// Détenu activement.
    Held,
    /// En cours de réconciliation après crash (reste strictement protecteur).
    Reconciling,
    /// Libéré explicitement.
    Released,
}

/// Demande de préparation d'un objet avant écriture physique hors transaction (`PrepareObject`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareObjectRequest {
    /// Empreinte SHA-256 attendue du contenu.
    pub object_hash: Sha256Digest,
    /// Taille exacte attendue en octets.
    pub byte_length: u64,
    /// Type MIME / media type.
    pub media_type: String,
    /// Type de propriétaire (`action`, `session`, `evidence`, `capsule`, etc.).
    pub owner_kind: String,
    /// Identifiant du propriétaire.
    pub owner_id: String,
}

/// Poignée durable retournée par `PrepareObject`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedObjectHandle {
    /// Empreinte SHA-256 de l'objet.
    pub object_hash: Sha256Digest,
    /// Génération physique exacte attribuée (>= 1).
    pub object_generation: i64,
    /// Chemin relatif propre à cette incarnation sous `artifacts_dir`.
    pub relative_path: String,
    /// Identifiant du pin de préparation obtenu.
    pub pin_id: String,
    /// Vrai si l'objet était déjà `live` (aucune écriture disque requise).
    pub already_live: bool,
    /// Vrai si ce producteur est le rédacteur physique exclusif (faux si un concurrent prépare déjà).
    pub is_physical_writer: bool,
}

/// Demande de publication atomique d'une référence métier après durabilité physique (`PublishObjectReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishObjectReferenceRequest {
    /// Identifiant du pin de préparation détenu.
    pub pin_id: String,
    /// Empreinte de l'objet.
    pub object_hash: Sha256Digest,
    /// Génération exacte attendue.
    pub object_generation: i64,
    /// Type du propriétaire métier.
    pub owner_kind: String,
    /// Identifiant du propriétaire métier.
    pub owner_id: String,
    /// Champ source portant la référence (ex: `"content_object_hash"`).
    pub source_field: String,
}

/// Poignée de référence exacte rendue au producteur interne après publication.
/// Les champs privés empêchent de fabriquer une poignée depuis un payload client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedObjectReference {
    reference_id: String,
    object_hash: Sha256Digest,
    object_generation: i64,
    owner_kind: String,
    owner_id: String,
    source_field: String,
    authority_generation: AuthorityGeneration,
}

/// Enregistrement lu depuis `object_records`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRecordView {
    /// Empreinte SHA-256.
    pub object_hash: Sha256Digest,
    /// Génération d'incarnation courante.
    pub object_generation: i64,
    /// Chemin relatif sous `artifacts_dir`.
    pub relative_path: String,
    /// Taille en octets.
    pub byte_length: u64,
    /// Type média.
    pub media_type: String,
    /// État courant.
    pub state: ObjectState,
    /// Opération de suppression liée éventuelle.
    pub delete_operation_id: Option<String>,
    /// Horodatage de création.
    pub created_at_ms: TimestampMs,
}

/// Demande d'engagement d'une suppression exclusive (`PrepareObjectDeletion`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareObjectDeletionRequest {
    /// Empreinte ciblée.
    pub object_hash: Sha256Digest,
    /// Génération exacte ciblée par le collecteur.
    pub expected_generation: i64,
    /// Motif auditée de suppression (ex: `"gc_unreferenced"` ou `"retention_purge"`).
    pub reason: String,
    /// Reçu de décision éventuel si purge explicite.
    pub decision_receipt_id: Option<ReceiptId>,
}

/// Intention durable de suppression retournée par `PrepareObjectDeletion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionIntent {
    /// Identifiant d'opération de suppression.
    pub delete_operation_id: String,
    /// Empreinte visée.
    pub object_hash: Sha256Digest,
    /// Génération exacte figée.
    pub object_generation: i64,
    /// Chemin relatif exact d'incarnation figé.
    pub relative_path: String,
    /// État (`prepared`, `unlinked`, `completed`).
    pub state: String,
}

/// Résultat d'une réconciliation d'objets, pins et suppressions après panne.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectReconciliationReport {
    /// Nombre de pins basculés en `reconciling`.
    pub pins_marked_reconciling: usize,
    /// Nombre de suppressions interrompues finalisées en `tombstone`/`completed`.
    pub deletions_finalized: usize,
    /// Nombre d'objets placés en quarantaine suite à une anomalie de chemin/hash.
    pub objects_quarantined: usize,
    /// Nombre d'objets préparés conservés sous protection de pin.
    pub prepared_objects_preserved: usize,
    /// Lignée observée lors de la réconciliation.
    pub authority_generation: Option<AuthorityGeneration>,
}

/// Construit le chemin relatif déterministe propre à une incarnation `(hash, génération)`.
#[must_use]
pub(crate) fn object_relative_path(hash: &Sha256Digest, generation: i64) -> String {
    let hex = hash.hex();
    let prefix = &hex[..2];
    format!("sha256/{prefix}/{hex}.g{generation}.blob")
}

/// Résout un chemin relatif d'artefact en garantissant l'absence de traversée.
pub(crate) fn safe_artifact_path(
    artifacts_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, StoreError> {
    let rel = Path::new(relative_path);
    if rel.is_absolute() {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!("chemin relatif d'artefact interdit (absolu) : '{relative_path}'"),
        });
    }
    let mut joined = artifacts_dir.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(part) => joined.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StoreError::InvalidArgument {
                    reason: None,
                    message: format!("traversée de répertoire interdite dans '{relative_path}'"),
                });
            }
        }
    }
    Ok(joined)
}

fn parse_object_state(raw: &str) -> Result<ObjectState, StoreError> {
    match raw {
        "prepared" => Ok(ObjectState::Prepared),
        "live" => Ok(ObjectState::Live),
        "deleting" => Ok(ObjectState::Deleting),
        "tombstone" => Ok(ObjectState::Tombstone),
        "quarantined" => Ok(ObjectState::Quarantined),
        other => Err(StoreError::InvariantViolation(format!(
            "état object_records inconnu '{other}'"
        ))),
    }
}

/// Prépare atomiquement un enregistrement d'objet et son pin de préparation.
#[expect(
    clippy::too_many_lines,
    reason = "Transaction atomique de préparation/réattachement multi-états d'objet sous pin"
)]
pub(crate) fn tx_prepare_object(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    authority_generation: AuthorityGeneration,
    now_ms: TimestampMs,
    req: &PrepareObjectRequest,
) -> Result<PreparedObjectHandle, StoreError> {
    let hash_str = req.object_hash.as_prefixed_str();
    let pin_id = config.id_generator.new_uuid().to_string();
    let existing: Option<(i64, String, String)> = tx
        .query_row(
            "SELECT object_generation, relative_path, state
             FROM object_records WHERE object_hash = ?1",
            [hash_str.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let auth_str = authority_generation.to_string();
    let byte_len_i64 = i64::try_from(req.byte_length).unwrap_or(i64::MAX);

    let (object_generation, relative_path, already_live, is_physical_writer) = if let Some((
        gen_val,
        rel_path,
        state_str,
    )) = existing
    {
        match parse_object_state(&state_str)? {
            ObjectState::Deleting => {
                return Err(StoreError::ObjectDeleting {
                    object_hash: req.object_hash,
                });
            }
            ObjectState::Quarantined => {
                return Err(StoreError::StorageCorrupt(format!(
                    "objet {hash_str} en quarantaine"
                )));
            }
            ObjectState::Live => (gen_val, rel_path, true, false),
            ObjectState::Prepared => {
                let exists_on_disk = safe_artifact_path(&config.artifacts_dir, &rel_path)
                    .map(|p| p.exists())
                    .unwrap_or(false);
                (gen_val, rel_path, false, !exists_on_disk)
            }
            ObjectState::Tombstone => {
                let next_gen = gen_val.saturating_add(1);
                let new_rel = object_relative_path(&req.object_hash, next_gen);
                tx.execute(
                    "UPDATE object_records
                             SET object_generation = ?1, relative_path = ?2, byte_length = ?3,
                                 media_type = ?4, state = 'prepared', delete_operation_id = NULL,
                                 created_at_ms = ?5
                             WHERE object_hash = ?6",
                    rusqlite::params![
                        next_gen,
                        new_rel,
                        byte_len_i64,
                        req.media_type,
                        now_ms.as_i64(),
                        hash_str
                    ],
                )
                .map_err(StoreError::from_rusqlite)?;
                (next_gen, new_rel, false, true)
            }
        }
    } else {
        let first_gen = 1_i64;
        let rel_path = object_relative_path(&req.object_hash, first_gen);
        tx.execute(
                "INSERT INTO object_records
                 (object_hash, object_generation, relative_path, byte_length, media_type, state, delete_operation_id, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', NULL, ?6)",
                rusqlite::params![
                    hash_str,
                    first_gen,
                    rel_path,
                    byte_len_i64,
                    req.media_type,
                    now_ms.as_i64()
                ],
            )
            .map_err(StoreError::from_rusqlite)?;
        (first_gen, rel_path, false, true)
    };

    tx.execute(
        "INSERT INTO object_pins
         (pin_id, pin_kind, object_hash, object_generation, owner_kind, owner_id, authority_generation, state, created_at_ms, released_at_ms)
         VALUES (?1, 'preparation', ?2, ?3, ?4, ?5, ?6, 'held', ?7, NULL)",
        rusqlite::params![
            pin_id,
            hash_str,
            object_generation,
            req.owner_kind,
            req.owner_id,
            auth_str,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(PreparedObjectHandle {
        object_hash: req.object_hash,
        object_generation,
        relative_path,
        pin_id,
        already_live,
        is_physical_writer,
    })
}

/// Écrit physiquement et durablement les octets d'un objet préparé hors transaction `SQLite`.
pub(crate) fn write_prepared_object_bytes_to_disk(
    config: &StoreConfig,
    handle: &PreparedObjectHandle,
    bytes: &[u8],
) -> Result<(), StoreError> {
    let computed = Sha256Digest::from_bytes(bytes);
    if computed != handle.object_hash {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: format!(
                "empreinte réelle {} != attendue {}",
                computed.as_prefixed_str(),
                handle.object_hash.as_prefixed_str()
            ),
        });
    }
    let dest = safe_artifact_path(&config.artifacts_dir, &handle.relative_path)?;
    if handle.already_live {
        if dest.exists() {
            let disk_bytes = fs::read(&dest)?;
            if Sha256Digest::from_bytes(&disk_bytes) != handle.object_hash {
                return Err(StoreError::StorageCorrupt(format!(
                    "divergence SHA-256 sur objet déjà live {}",
                    handle.relative_path
                )));
            }
            return Ok(());
        }
        return Err(StoreError::EvidenceUnavailable(format!(
            "fichier absent pour objet live {}",
            handle.relative_path
        )));
    }
    if dest.exists() {
        let existing_bytes = fs::read(&dest)?;
        if Sha256Digest::from_bytes(&existing_bytes) != handle.object_hash {
            return Err(StoreError::StorageCorrupt(format!(
                "fichier existant divergent au chemin {}",
                handle.relative_path
            )));
        }
        return Ok(());
    }
    write_atomic_durable(&dest, bytes, 0o600)?;
    Ok(())
}

/// Vérifie les octets présents sur disque pour un objet avant de le publier `live`.
pub(crate) fn verify_object_file_before_publish(
    config: &StoreConfig,
    relative_path: &str,
    expected_hash: Sha256Digest,
) -> Result<(), StoreError> {
    let path = safe_artifact_path(&config.artifacts_dir, relative_path)?;
    if !path.exists() {
        return Err(StoreError::EvidenceUnavailable(format!(
            "octets introuvables avant publication : {relative_path}"
        )));
    }
    let bytes = fs::read(&path)?;
    if Sha256Digest::from_bytes(&bytes) != expected_hash {
        return Err(StoreError::StorageCorrupt(format!(
            "empreinte disque corrompue avant publication : {relative_path}"
        )));
    }
    Ok(())
}

/// Publie atomiquement la référence métier, passe l'objet en `live` et libère le pin de préparation.
pub(crate) fn tx_publish_object_reference(
    tx: &Transaction<'_>,
    id_generator: &dyn unleash_domain::IdGeneratorPort,
    now_ms: TimestampMs,
    req: &PublishObjectReferenceRequest,
) -> Result<(), StoreError> {
    let hash_str = req.object_hash.as_prefixed_str();
    let rec: Option<(i64, String)> = tx
        .query_row(
            "SELECT object_generation, state FROM object_records WHERE object_hash = ?1",
            [hash_str.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((cur_gen, state_str)) = rec else {
        return Err(StoreError::NotFound(format!("object {hash_str}")));
    };
    let state = parse_object_state(&state_str)?;
    if state == ObjectState::Deleting {
        return Err(StoreError::ObjectDeleting {
            object_hash: req.object_hash,
        });
    }
    if cur_gen != req.object_generation {
        return Err(StoreError::InvariantViolation(format!(
            "génération d'objet divergente : attendue {}, observée {cur_gen}",
            req.object_generation
        )));
    }
    let pin_exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM object_pins
             WHERE pin_id = ?1 AND object_hash = ?2 AND object_generation = ?3 AND state IN ('held', 'reconciling')",
            rusqlite::params![req.pin_id, hash_str, req.object_generation],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if pin_exists == 0 {
        return Err(StoreError::InvariantViolation(format!(
            "pin de préparation absent ou déjà libéré : {}",
            req.pin_id
        )));
    }

    if state == ObjectState::Prepared {
        tx.execute(
            "UPDATE object_records SET state = 'live'
             WHERE object_hash = ?1 AND object_generation = ?2 AND state = 'prepared'",
            rusqlite::params![hash_str, req.object_generation],
        )
        .map_err(StoreError::from_rusqlite)?;
    } else if state != ObjectState::Live {
        return Err(StoreError::InvariantViolation(format!(
            "état objet non publiable : {state_str}"
        )));
    }

    let ref_id = id_generator.new_uuid().to_string();
    tx.execute(
        "INSERT INTO object_references (reference_id, object_hash, owner_kind, owner_id, source_field, state, tombstone_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, 'live', NULL)
         ON CONFLICT (owner_kind, owner_id, source_field, object_hash)
         DO UPDATE SET state = 'live', tombstone_reason = NULL",
        rusqlite::params![ref_id, hash_str, req.owner_kind, req.owner_id, req.source_field],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.execute(
        "UPDATE object_pins SET state = 'released', released_at_ms = ?1
         WHERE pin_id = ?2 AND object_hash = ?3 AND object_generation = ?4
           AND state IN ('held', 'reconciling')",
        rusqlite::params![now_ms.as_i64(), req.pin_id, hash_str, req.object_generation],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(())
}

pub(crate) fn tx_publish_owned_reference(
    tx: &Transaction<'_>,
    id_generator: &dyn unleash_domain::IdGeneratorPort,
    now_ms: TimestampMs,
    req: &PublishObjectReferenceRequest,
    authority: AuthorityGeneration,
) -> Result<PublishedObjectReference, StoreError> {
    let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM object_pins WHERE pin_id=?1 AND authority_generation=?2 AND object_hash=?3 AND object_generation=?4 AND state IN ('held','reconciling'))",rusqlite::params![req.pin_id,authority.to_string(),req.object_hash.to_string(),req.object_generation],|row|row.get(0))?;
    if !current {
        return Err(StoreError::InvariantViolation(
            "pin du producteur absent de la lignée active".to_owned(),
        ));
    }
    tx_publish_object_reference(tx, id_generator, now_ms, req)?;
    let reference_id = tx.query_row("SELECT reference_id FROM object_references WHERE object_hash=?1 AND owner_kind=?2 AND owner_id=?3 AND source_field=?4 AND state='live'",rusqlite::params![req.object_hash.to_string(),req.owner_kind,req.owner_id,req.source_field],|row|row.get(0))?;
    Ok(PublishedObjectReference {
        reference_id,
        object_hash: req.object_hash,
        object_generation: req.object_generation,
        owner_kind: req.owner_kind.clone(),
        owner_id: req.owner_id.clone(),
        source_field: req.source_field.clone(),
        authority_generation: authority,
    })
}

pub(crate) fn tx_retire_owned_reference(
    tx: &Transaction<'_>,
    handle: &PublishedObjectReference,
    authority: AuthorityGeneration,
    reason: &str,
) -> Result<usize, StoreError> {
    if authority != handle.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: authority,
            actual: handle.authority_generation,
            command_id: None,
        });
    }
    tx.execute("UPDATE object_references SET state='tombstone',tombstone_reason=?1 WHERE reference_id=?2 AND object_hash=?3 AND owner_kind=?4 AND owner_id=?5 AND source_field=?6 AND state='live' AND EXISTS(SELECT 1 FROM object_records o WHERE o.object_hash=object_references.object_hash AND o.object_generation=?7)",rusqlite::params![reason,handle.reference_id,handle.object_hash.to_string(),handle.owner_kind,handle.owner_id,handle.source_field,handle.object_generation]).map_err(StoreError::from_rusqlite)
}

/// Acquiert un pin de lecture sur un objet `live`.
pub(crate) fn tx_acquire_read_pin(
    tx: &Transaction<'_>,
    id_generator: &dyn unleash_domain::IdGeneratorPort,
    authority_generation: AuthorityGeneration,
    now_ms: TimestampMs,
    object_hash: Sha256Digest,
    owner_kind: &str,
    owner_id: &str,
) -> Result<String, StoreError> {
    let hash_str = object_hash.as_prefixed_str();
    let rec: Option<(i64, String)> = tx
        .query_row(
            "SELECT object_generation, state FROM object_records WHERE object_hash = ?1",
            [hash_str.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((gen_val, state_str)) = rec else {
        return Err(StoreError::NotFound(format!("object {hash_str}")));
    };
    match parse_object_state(&state_str)? {
        ObjectState::Deleting => return Err(StoreError::ObjectDeleting { object_hash }),
        ObjectState::Quarantined => {
            return Err(StoreError::StorageCorrupt(format!(
                "objet {hash_str} en quarantaine"
            )));
        }
        ObjectState::Live => {}
        _ => {
            return Err(StoreError::EvidenceUnavailable(format!(
                "objet {hash_str} non live ({state_str})"
            )));
        }
    }
    let pin_id = id_generator.new_uuid().to_string();
    tx.execute(
        "INSERT INTO object_pins
         (pin_id, pin_kind, object_hash, object_generation, owner_kind, owner_id, authority_generation, state, created_at_ms, released_at_ms)
         VALUES (?1, 'read', ?2, ?3, ?4, ?5, ?6, 'held', ?7, NULL)",
        rusqlite::params![
            pin_id,
            hash_str,
            gen_val,
            owner_kind,
            owner_id,
            authority_generation.to_string(),
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(pin_id)
}

/// Libère un pin actif (`held` ou `reconciling`).
pub(crate) fn tx_release_pin(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    pin_id: &str,
) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE object_pins SET state = 'released', released_at_ms = ?1
         WHERE pin_id = ?2 AND state IN ('held', 'reconciling')",
        rusqlite::params![now_ms.as_i64(), pin_id],
    )
    .map_err(StoreError::from_rusqlite)?;
    Ok(())
}

/// Passe les références d'un objet de `live` à `tombstone`.
pub(crate) fn tx_tombstone_object_references(
    tx: &Transaction<'_>,
    object_hash: Sha256Digest,
    reason: &str,
) -> Result<usize, StoreError> {
    let n = tx
        .execute(
            "UPDATE object_references SET state = 'tombstone', tombstone_reason = ?1
             WHERE object_hash = ?2 AND state = 'live'",
            rusqlite::params![reason, object_hash.as_prefixed_str()],
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(n)
}

/// Réserve exclusivement la suppression d'une incarnation d'objet (`PrepareObjectDeletion`).
#[expect(
    clippy::too_many_lines,
    reason = "Vérification séquentielle barrière backup, état, références, pins et enregistrement deleting atomique"
)]
pub(crate) fn tx_prepare_object_deletion(
    tx: &Transaction<'_>,
    id_generator: &dyn unleash_domain::IdGeneratorPort,
    now_ms: TimestampMs,
    req: &PrepareObjectDeletionRequest,
) -> Result<DeletionIntent, StoreError> {
    let barrier_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM object_pins
             WHERE pin_kind = 'backup_barrier' AND state IN ('held', 'reconciling')",
            [],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if barrier_count > 0 {
        return Err(StoreError::BackupBarrierActive);
    }

    let hash_str = req.object_hash.as_prefixed_str();
    let rec: Option<(i64, String, String)> = tx
        .query_row(
            "SELECT object_generation, relative_path, state FROM object_records WHERE object_hash = ?1",
            [hash_str.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((cur_gen, rel_path, state_str)) = rec else {
        return Err(StoreError::NotFound(format!("object {hash_str}")));
    };
    if cur_gen != req.expected_generation {
        return Err(StoreError::InvariantViolation(format!(
            "collecte refusée : génération courante {cur_gen} != attendue {}",
            req.expected_generation
        )));
    }
    match parse_object_state(&state_str)? {
        ObjectState::Deleting => {
            return Err(StoreError::ObjectDeleting {
                object_hash: req.object_hash,
            });
        }
        ObjectState::Quarantined => {
            return Err(StoreError::StorageCorrupt(format!(
                "objet {hash_str} en quarantaine"
            )));
        }
        ObjectState::Tombstone => {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message: format!("objet {hash_str} déjà en tombstone"),
            });
        }
        ObjectState::Live | ObjectState::Prepared => {}
    }

    let live_refs: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM object_references WHERE object_hash = ?1 AND state = 'live'",
            [hash_str.as_str()],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if live_refs > 0 {
        return Err(StoreError::ObjectReferenced {
            object_hash: req.object_hash,
        });
    }

    let active_pins: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM object_pins
             WHERE object_hash = ?1 AND object_generation = ?2 AND state IN ('held', 'reconciling')",
            rusqlite::params![hash_str, req.expected_generation],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    if active_pins > 0 {
        return Err(StoreError::ObjectPinned {
            object_hash: req.object_hash,
        });
    }

    let del_op_id = id_generator.new_uuid().to_string();
    let rcpt_str = req.decision_receipt_id.map(|r| r.to_string());
    tx.execute(
        "INSERT INTO object_deletions
         (delete_operation_id, object_hash, object_generation, relative_path, reason, decision_receipt_id, state, created_at_ms, completed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'prepared', ?7, NULL)",
        rusqlite::params![
            del_op_id,
            hash_str,
            req.expected_generation,
            rel_path,
            req.reason,
            rcpt_str,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    tx.execute(
        "UPDATE object_records SET state = 'deleting', delete_operation_id = ?1
         WHERE object_hash = ?2 AND object_generation = ?3",
        rusqlite::params![del_op_id, hash_str, req.expected_generation],
    )
    .map_err(StoreError::from_rusqlite)?;

    Ok(DeletionIntent {
        delete_operation_id: del_op_id,
        object_hash: req.object_hash,
        object_generation: req.expected_generation,
        relative_path: rel_path,
        state: "prepared".to_owned(),
    })
}

/// Exécute physiquement `unlink` + `sync_dir` puis finalise `tombstone`/`completed` (avec support `C10c`/`C10d`).
#[expect(
    clippy::too_many_lines,
    reason = "Séquence exacte C10c -> re-vérification -> unlink/quarantaine -> C10d -> finalisation tombstone"
)]
pub(crate) fn execute_unlink_and_finalize_deletion(
    config: &StoreConfig,
    writer: &StoreWriterHandle,
    intent: &DeletionIntent,
    cutpoints: &dyn CutpointObserver,
    fire_c10c: bool,
) -> Result<(), StoreError> {
    if fire_c10c {
        cutpoints.hit(StoreCutpoint::C10cAfterDeletingIntent)?;
    }

    let op_id = intent.delete_operation_id.clone();
    let hash = intent.object_hash;
    let hash_str = hash.as_prefixed_str();
    let exp_gen = intent.object_generation;
    let exp_rel = intent.relative_path.clone();

    let verified_target = writer.submit_immediate_blocking(WriteCriticality::Normal, false, move |tx, _now| {
        let del_state: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT state, object_generation, relative_path
                 FROM object_deletions WHERE delete_operation_id = ?1 AND object_hash = ?2",
                rusqlite::params![op_id, hash_str],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        let Some((d_state, d_gen, d_rel)) = del_state else {
            return Err(StoreError::NotFound(format!("deletion {op_id}")));
        };
        if d_gen != exp_gen || d_rel != exp_rel || !matches!(d_state.as_str(), "prepared" | "unlinked") {
            return Err(StoreError::InvariantViolation(format!(
                "intention de suppression obsolète ou invalide ({op_id}, state={d_state}, gen={d_gen})"
            )));
        }

        let rec: Option<(i64, String, String, Option<String>)> = tx
            .query_row(
                "SELECT object_generation, relative_path, state, delete_operation_id
                 FROM object_records WHERE object_hash = ?1",
                [hash_str.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        let Some((r_gen, r_rel, r_state, r_del_op)) = rec else {
            return Err(StoreError::NotFound(format!("object {hash_str}")));
        };
        if r_gen != exp_gen || r_rel != exp_rel || r_state != "deleting" || r_del_op.as_deref() != Some(&op_id) {
            return Err(StoreError::InvariantViolation(format!(
                "incarnation d'objet protégée contre ancien collecteur (gen={r_gen} vs {exp_gen})"
            )));
        }
        Ok((r_rel, d_state))
    })?;

    let (rel_path, del_state) = verified_target;
    let abs_path = safe_artifact_path(&config.artifacts_dir, &rel_path)?;

    if del_state == "prepared" && abs_path.exists() {
        let on_disk = fs::read(&abs_path)?;
        let actual = Sha256Digest::from_bytes(&on_disk);
        if actual != intent.object_hash {
            let op_id_q = intent.delete_operation_id.clone();
            let h_q = intent.object_hash.as_prefixed_str();
            writer.submit_immediate_blocking(
                WriteCriticality::Critical,
                false,
                move |tx, now_ms| {
                    tx.execute(
                        "UPDATE object_records SET state = 'quarantined'
                     WHERE object_hash = ?1 AND object_generation = ?2",
                        rusqlite::params![h_q, exp_gen],
                    )
                    .map_err(StoreError::from_rusqlite)?;
                    tx.execute(
                        "UPDATE object_deletions SET state = 'completed', completed_at_ms = ?1
                     WHERE delete_operation_id = ?2",
                        rusqlite::params![now_ms.as_i64(), op_id_q],
                    )
                    .map_err(StoreError::from_rusqlite)?;
                    Ok(())
                },
            )?;
            tracing::warn!(
                object_hash = %intent.object_hash,
                object_generation = exp_gen,
                relative_path = %rel_path,
                "fichier étranger ou corrompu placé en quarantaine sans unlink"
            );
            return Err(StoreError::StorageCorrupt(format!(
                "fichier étranger ou corrompu détecté sous '{rel_path}' : placé en quarantaine sans unlink"
            )));
        }
        check_injected_fault(config.fault_injector.as_ref(), FaultTarget::ObjectUnlink)?;
        fs::remove_file(&abs_path)?;
    }
    // Un unlink peut précéder un crash/IOERR de sync_dir. L'absence du chemin ne
    // prouve donc pas la durabilité de sa suppression : resynchroniser à la reprise.
    check_injected_fault(
        config.fault_injector.as_ref(),
        FaultTarget::ObjectDeletionSync,
    )?;
    if let Some(parent) = abs_path.parent() {
        sync_dir(parent)?;
    }
    tracing::info!(
        object_hash = %intent.object_hash, object_generation = exp_gen,
        delete_operation_id = %intent.delete_operation_id,
        "suppression et synchronisation répertoire vérifiées"
    );

    let op_id_u = intent.delete_operation_id.clone();
    writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
        tx.execute(
            "UPDATE object_deletions SET state = 'unlinked'
             WHERE delete_operation_id = ?1 AND state = 'prepared'",
            [op_id_u],
        )
        .map_err(StoreError::from_rusqlite)?;
        Ok(())
    })?;

    cutpoints.hit(StoreCutpoint::C10dAfterUnlinkSyncBeforeTombstone)?;

    let op_id_f = intent.delete_operation_id.clone();
    let h_f = intent.object_hash.as_prefixed_str();
    writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, now_ms| {
        tx.execute(
            "UPDATE object_deletions SET state = 'completed', completed_at_ms = ?1
             WHERE delete_operation_id = ?2 AND state IN ('prepared', 'unlinked')",
            rusqlite::params![now_ms.as_i64(), op_id_f],
        )
        .map_err(StoreError::from_rusqlite)?;
        tx.execute(
            "UPDATE object_records SET state = 'tombstone'
             WHERE object_hash = ?1 AND object_generation = ?2 AND state = 'deleting' AND delete_operation_id = ?3",
            rusqlite::params![h_f, exp_gen, op_id_f],
        )
        .map_err(StoreError::from_rusqlite)?;
        Ok(())
    })?;

    Ok(())
}

/// Réconcilie au redémarrage pins actifs, objets préparés et suppressions interrompues.
pub(crate) fn reconcile_objects_workflow(
    config: &StoreConfig,
    writer: &StoreWriterHandle,
    cutpoints: &dyn CutpointObserver,
) -> Result<ObjectReconciliationReport, StoreError> {
    let (pins_marked, prepared_preserved, quarantined_count, authority_gen, pending_deletions) =
        writer.submit_immediate_blocking(WriteCriticality::Critical, false, move |tx, _now| {
            let pins_marked = tx
                .execute(
                    "UPDATE object_pins SET state = 'reconciling' WHERE state = 'held'",
                    [],
                )
                .map_err(StoreError::from_rusqlite)?;

            let prepared_preserved_i64: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM object_records WHERE state = 'prepared'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;

            let quarantined_i64: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM object_records WHERE state = 'quarantined'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;

            let auth_str: String = tx
                .query_row(
                    "SELECT authority_generation FROM daemon_state WHERE singleton = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            let auth_gen: AuthorityGeneration = auth_str.parse()?;

            let mut stmt = tx
                .prepare(
                    "SELECT d.delete_operation_id, d.object_hash, d.object_generation, d.relative_path, d.state
                     FROM object_deletions d
                     JOIN object_records r ON r.object_hash = d.object_hash
                     WHERE d.state IN ('prepared', 'unlinked')
                       AND r.state = 'deleting'
                       AND r.object_generation = d.object_generation
                       AND r.delete_operation_id = d.delete_operation_id",
                )
                .map_err(StoreError::from_rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    let op_id: String = r.get(0)?;
                    let h_str: String = r.get(1)?;
                    let gen_val: i64 = r.get(2)?;
                    let rel_path: String = r.get(3)?;
                    let st_str: String = r.get(4)?;
                    Ok((op_id, h_str, gen_val, rel_path, st_str))
                })
                .map_err(StoreError::from_rusqlite)?;

            let mut intents = Vec::new();
            for row in rows {
                let (op_id, h_str, gen_val, rel_path, st_str) =
                    row.map_err(StoreError::from_rusqlite)?;
                let hash: Sha256Digest = h_str.parse()?;
                intents.push(DeletionIntent {
                    delete_operation_id: op_id,
                    object_hash: hash,
                    object_generation: gen_val,
                    relative_path: rel_path,
                    state: st_str,
                });
            }
            Ok((
                pins_marked,
                usize::try_from(prepared_preserved_i64).unwrap_or(0),
                usize::try_from(quarantined_i64).unwrap_or(0),
                auth_gen,
                intents,
            ))
        })?;

    let mut deletions_finalized = 0_usize;
    let mut newly_quarantined = 0_usize;
    for intent in pending_deletions {
        match execute_unlink_and_finalize_deletion(config, writer, &intent, cutpoints, false) {
            Ok(()) => deletions_finalized += 1,
            Err(StoreError::StorageCorrupt(_)) => newly_quarantined += 1,
            Err(e) => return Err(e),
        }
    }

    Ok(ObjectReconciliationReport {
        pins_marked_reconciling: pins_marked,
        deletions_finalized,
        objects_quarantined: quarantined_count + newly_quarantined,
        prepared_objects_preserved: prepared_preserved,
        authority_generation: Some(authority_gen),
    })
}

/// Lit un enregistrement `object_records` sur une connexion de lecture.
pub(crate) fn read_object_record_conn(
    conn: &rusqlite::Connection,
    object_hash: Sha256Digest,
) -> Result<ObjectRecordView, StoreError> {
    let hash_str = object_hash.as_prefixed_str();
    let row: Option<(i64, String, i64, String, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT object_generation, relative_path, byte_length, media_type, state, delete_operation_id, created_at_ms
             FROM object_records WHERE object_hash = ?1",
            [hash_str.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((
        object_generation,
        relative_path,
        byte_len_i64,
        media_type,
        st_str,
        delete_operation_id,
        created_ms,
    )) = row
    else {
        return Err(StoreError::NotFound(format!("object {hash_str}")));
    };
    Ok(ObjectRecordView {
        object_hash,
        object_generation,
        relative_path,
        byte_length: u64::try_from(byte_len_i64).unwrap_or(0),
        media_type,
        state: parse_object_state(&st_str)?,
        delete_operation_id,
        created_at_ms: TimestampMs::new(created_ms),
    })
}

/// Lit et vérifie l'empreinte SHA-256 des octets d'un objet `live`.
pub(crate) fn read_verified_object_bytes_conn(
    conn: &rusqlite::Connection,
    config: &StoreConfig,
    object_hash: Sha256Digest,
) -> Result<Vec<u8>, StoreError> {
    let rec = read_object_record_conn(conn, object_hash)?;
    if rec.state != ObjectState::Live {
        return Err(StoreError::EvidenceUnavailable(format!(
            "objet {} non live ({:?})",
            object_hash.as_prefixed_str(),
            rec.state
        )));
    }
    let path = safe_artifact_path(&config.artifacts_dir, &rec.relative_path)?;
    if !path.exists() {
        return Err(StoreError::EvidenceUnavailable(format!(
            "fichier objet absent : {}",
            rec.relative_path
        )));
    }
    let bytes = fs::read(&path)?;
    if Sha256Digest::from_bytes(&bytes) != object_hash {
        return Err(StoreError::EvidenceUnavailable(format!(
            "divergence SHA-256 à la lecture de {}",
            rec.relative_path
        )));
    }
    Ok(bytes)
}
