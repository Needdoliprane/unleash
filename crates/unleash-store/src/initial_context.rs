//! Publication of a resolved, immutable initial session context (REQ-AGENT-002).

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;
use unleash_domain::{
    AuthorityGeneration, DaemonEpoch, DaemonMode, Revision, SessionId, Sha256Digest,
};
use unleash_protocol::canonicalize_to_vec;
use uuid::Uuid;

use crate::{
    DaemonStateRow, PrepareObjectRequest, PublishObjectReferenceRequest, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
    commands::read_daemon_state_conn,
    objects::{
        PublishedObjectReference, read_verified_object_bytes_conn, tx_prepare_object,
        tx_publish_owned_reference, tx_release_pin, tx_require_owned_reference,
        verify_object_file_before_publish,
    },
};

const MANIFEST_KIND: &str = "context_manifest";
const CONTENT_FIELD: &str = "content_hash";

/// Coordinates allocated by the harness for one context manifest publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialContextPlan {
    /// Manifest ID allocated for the exact context.
    pub manifest_id: Uuid,
    /// Current database lineage when the context was resolved.
    pub authority_generation: AuthorityGeneration,
    /// Daemon epoch that resolved the context.
    pub daemon_epoch: DaemonEpoch,
}

#[derive(Deserialize)]
struct Coordinates {
    schema_version: u32,
    session_id: SessionId,
    base_session_revision: Revision,
    parent_manifest_id: Option<Uuid>,
    entries: Vec<ContextEntry>,
}

#[derive(Deserialize)]
struct ContextEntry {
    object_hash: Sha256Digest,
}

impl UnleashStore {
    /// Publish the harness's resolved document and retain its ordered input objects.
    ///
    /// The harness is responsible for context selection, effective policy and
    /// provenance. This internal port checks the selected object identities and
    /// the exact session revision. It leaves the mutable session head intact.
    /// Reusing the manifest ID with the same document returns the existing digest.
    ///
    /// # Errors
    /// Refuses stale authority, a changed session, unavailable inputs or an
    /// already published ID with different content.
    pub fn publish_initial_context(
        &self,
        plan: InitialContextPlan,
        document: &Value,
        inputs: Vec<PublishedObjectReference>,
    ) -> Result<Sha256Digest, StoreError> {
        let coordinates: Coordinates = serde_json::from_value(document.clone())?;
        check_ordered_inputs(plan, &coordinates, &inputs)?;
        let bytes = canonicalize_to_vec(document)?;
        let hash = Sha256Digest::from_bytes(&bytes);
        let already_published = self.read_snapshot(|conn, _| {
            let published = check_publication(conn, plan, &coordinates, hash)?;
            if published {
                read_verified_object_bytes_conn(conn, self.config(), hash)?;
            }
            Ok(published)
        })?;
        if already_published {
            return Ok(hash);
        }

        let owner = plan.manifest_id.to_string();
        let staging = PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len())
                .map_err(|_| invalid("context length exceeds storage range"))?,
            media_type: "application/json".to_owned(),
            owner_kind: MANIFEST_KIND.to_owned(),
            owner_id: owner.clone(),
        };
        let config = self.config().clone();
        let handle = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = require_lineage(tx, plan)?;
            require_ready(daemon.mode)?;
            tx_prepare_object(tx, &config, plan.authority_generation, now, &staging)
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C10aAfterPrepPin)?;
        let pin = handle.pin_id.clone();
        let result = (|| {
            self.write_prepared_object_bytes(&handle, &bytes)?;
            verify_object_file_before_publish(self.config(), &handle.relative_path, hash)?;
            let reference = PublishObjectReferenceRequest {
                pin_id: handle.pin_id,
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: MANIFEST_KIND.to_owned(),
                owner_id: owner,
                source_field: CONTENT_FIELD.to_owned(),
            };
            let ids = Arc::clone(&self.config().id_generator);
            self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
                if check_publication(tx, plan, &coordinates, hash)? {
                    return tx_release_pin(tx, now, &reference.pin_id);
                }
                for (index, input) in inputs.iter().enumerate() {
                    tx_require_owned_reference(tx, input, plan.authority_generation)?;
                    tx.execute(
                        "INSERT INTO object_references
                            (reference_id,object_hash,owner_kind,owner_id,source_field,state)
                         VALUES (?1,?2,?3,?4,?5,'live')",
                        params![
                            ids.new_uuid().to_string(),
                            input.object_hash().as_prefixed_str(),
                            MANIFEST_KIND,
                            reference.owner_id,
                            format!("entries/{index}"),
                        ],
                    )?;
                }
                tx_publish_owned_reference(
                    tx,
                    ids.as_ref(),
                    now,
                    &reference,
                    plan.authority_generation,
                )?;
                tx.execute(
                    "INSERT INTO context_manifests
                        (manifest_id,session_id,parent_manifest_id,base_session_revision,
                         content_hash,schema_version,created_at_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        reference.owner_id,
                        coordinates.session_id.to_string(),
                        coordinates.parent_manifest_id.map(|id| id.to_string()),
                        stored_revision(coordinates.base_session_revision)?,
                        hash.as_prefixed_str(),
                        coordinates.schema_version,
                        now.as_i64(),
                    ],
                )?;
                Ok(())
            })
        })();
        if result.is_err() {
            self.release_initial_context_pin(plan, pin);
        }
        result.map(|()| hash)
    }

    fn release_initial_context_pin(&self, plan: InitialContextPlan, pin: String) {
        let cleanup = self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
            if read_daemon_state_conn(tx)?.authority_generation == plan.authority_generation {
                tx_release_pin(tx, now, &pin)?;
            }
            Ok(())
        });
        if let Err(error) = cleanup {
            tracing::warn!(manifest_id = %plan.manifest_id, %error, "initial context pin release pending");
        }
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn check_ordered_inputs(
    plan: InitialContextPlan,
    coordinates: &Coordinates,
    inputs: &[PublishedObjectReference],
) -> Result<(), StoreError> {
    if coordinates.schema_version != 1
        || plan.manifest_id.is_nil()
        || coordinates.session_id.as_uuid().is_nil()
        || coordinates.parent_manifest_id == Some(plan.manifest_id)
    {
        return Err(invalid("invalid initial context coordinates"));
    }
    if coordinates.entries.len() != inputs.len()
        || !coordinates
            .entries
            .iter()
            .zip(inputs)
            .all(|(entry, input)| entry.object_hash == input.object_hash())
    {
        return Err(StoreError::EvidenceUnavailable(
            "ordered context entries differ from the retained inputs".to_owned(),
        ));
    }
    Ok(())
}

fn stored_revision(revision: Revision) -> Result<i64, StoreError> {
    i64::try_from(revision.as_u64()).map_err(|_| invalid("context revision exceeds storage range"))
}

// Returns true only for an existing publication of this exact document.
// A fresh publication must compare the same session state in the writer.
fn check_publication(
    conn: &Connection,
    plan: InitialContextPlan,
    coordinates: &Coordinates,
    hash: Sha256Digest,
) -> Result<bool, StoreError> {
    let daemon = require_lineage(conn, plan)?;
    let manifest = plan.manifest_id.to_string();
    let session = coordinates.session_id.to_string();
    let parent = coordinates.parent_manifest_id.map(|id| id.to_string());
    let expected_revision = stored_revision(coordinates.base_session_revision)?;

    let existing = conn
        .query_row(
            "SELECT c.content_hash,
             c.session_id=?2 AND c.base_session_revision=?3 AND c.parent_manifest_id IS ?4
                AND c.schema_version=?5
                AND EXISTS (SELECT 1 FROM object_references ref
                    JOIN object_records o USING(object_hash)
                    WHERE ref.owner_kind=?6 AND ref.owner_id=c.manifest_id
                      AND ref.source_field=?7 AND ref.object_hash=c.content_hash
                      AND ref.state='live' AND o.state='live')
         FROM context_manifests c WHERE c.manifest_id=?1",
            params![
                manifest,
                session,
                expected_revision,
                parent,
                coordinates.schema_version,
                MANIFEST_KIND,
                CONTENT_FIELD
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?;
    if let Some((recorded, retained)) = existing {
        if recorded != hash.as_prefixed_str() {
            return Err(invalid(
                "context manifest ID already names a different document",
            ));
        }
        if !retained {
            return Err(StoreError::EvidenceUnavailable(
                "published context manifest is unavailable".to_owned(),
            ));
        }
        return Ok(true);
    }

    require_ready(daemon.mode)?;
    check_session(conn, coordinates, &session, parent.as_deref())?;
    Ok(false)
}

fn check_session(
    conn: &Connection,
    coordinates: &Coordinates,
    session: &str,
    parent: Option<&str>,
) -> Result<(), StoreError> {
    let (revision, head): (i64, Option<String>) = conn
        .query_row(
            "SELECT revision,head_manifest_id FROM sessions WHERE session_id=?1 AND state='open'",
            [session],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound("open session for initial context".to_owned()))?;
    let revision = Revision::new(u64::try_from(revision).map_err(|_| {
        StoreError::InvariantViolation("invalid stored session revision".to_owned())
    })?);
    if revision != coordinates.base_session_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: coordinates.base_session_revision,
            command_id: None,
        });
    }
    if head.as_deref() != parent {
        return Err(StoreError::EvidenceUnavailable(
            "initial context's parent is no longer the session head".to_owned(),
        ));
    }
    Ok(())
}

fn require_lineage(
    conn: &Connection,
    plan: InitialContextPlan,
) -> Result<DaemonStateRow, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.authority_generation != plan.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: plan.authority_generation,
            command_id: None,
        });
    }
    if daemon.daemon_epoch != plan.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "context was resolved in another daemon epoch".to_owned(),
        });
    }
    Ok(daemon)
}
fn require_ready(mode: DaemonMode) -> Result<(), StoreError> {
    if mode == DaemonMode::Ready {
        return Ok(());
    }
    Err(StoreError::RecoveryInProgress(
        "context publication requires a ready daemon".to_owned(),
    ))
}
