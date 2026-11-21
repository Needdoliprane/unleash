//! Publication and selection of an effective bundle for a workflow task (REQ-AGENT-002).

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::Value;
use unleash_domain::{AgentId, DaemonMode, InvocationId, Revision, Sha256Digest, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, canonicalize_to_vec, compute_command_digest_v2,
};
use uuid::Uuid;

use crate::{
    CallerAuthority, CommitOutcome, DaemonStateRow, PrepareObjectRequest, PreparedTaskAssignment,
    PublishObjectReferenceRequest, StoreConfig, StoreCutpoint, StoreError, UnleashStore,
    WriteCriticality,
    commands::{classify_command_replay, read_daemon_state_conn},
    objects::{
        tx_prepare_object, tx_publish_owned_reference, tx_release_pin,
        verify_object_file_before_publish,
    },
    task_assignment::{
        AssignmentCommand, read_document, validate_assignment_envelope, verify_staged_bundle,
    },
};

const OBJECT_KIND: &str = "effective_bundle";
const CONTENT_FIELD: &str = "content_object_hash";
const TASK_KIND: &str = "task";

/// Fields projected into `SQLite`. The harness supplies the rest of the resolved
/// document, including its policy, limits and provenance, unchanged.
#[derive(Deserialize)]
struct BundleIndex {
    schema_version: u32,
    role_version_id: String,
    workflow_version_id: String,
    requested_model_profile_version_id: String,
    allowed_model_policy_hash: Sha256Digest,
    initial_context_manifest_id: Uuid,
    scope_id: String,
    scope_revision: Revision,
    trigger_event_id: Uuid,
}

#[derive(PartialEq, Eq)]
struct AssignmentRevision {
    id: Uuid,
    epoch: u64,
    revision: Revision,
    session: String,
    session_revision: Revision,
}

impl AssignmentRevision {
    fn matches(&self, plan: &PreparedTaskAssignment) -> bool {
        self.id == plan.assignment_id
            && self.epoch == plan.assignment_epoch
            && self.revision == plan.assignment_revision
    }
}

struct Publication {
    envelope: CommandEnvelope,
    caller: CallerAuthority,
    assignee: AgentId,
    preparation: PreparedTaskAssignment,
    index: BundleIndex,
    document: Value,
}

impl UnleashStore {
    /// Publish the harness's resolved bundle for one exact task and assignment.
    ///
    /// The harness supplies the effective roles, bindings, policy and limits.
    /// The Store links the immutable document to this revision of the task and
    /// active assignment. It does not reserve execution capacity.
    ///
    /// # Errors
    /// Refuses stale or unavailable context, mismatched coordinates and a
    /// different bundle already selected for the same revisions.
    pub fn publish_effective_task_bundle(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
        preparation: PreparedTaskAssignment,
        document: &Value,
    ) -> Result<Sha256Digest, StoreError> {
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        let assignee = validate_assignment_envelope(&envelope, &caller)?;
        let index: BundleIndex = serde_json::from_value(document.clone())?;
        let bytes = canonicalize_to_vec(document)?;
        let hash = Sha256Digest::from_bytes(&bytes);
        if index.schema_version != 1
            || index.initial_context_manifest_id.is_nil()
            || index.trigger_event_id != preparation.trigger_event_id
            || hash != preparation.effective_bundle_hash
        {
            return Err(invalid("task bundle publication coordinates do not match"));
        }
        let publication = Arc::new(Publication {
            envelope,
            caller,
            assignee,
            preparation,
            index,
            document: document.clone(),
        });
        self.persist_task_bundle(publication, &bytes, hash)
    }

    fn persist_task_bundle(
        &self,
        publication: Arc<Publication>,
        bytes: &[u8],
        hash: Sha256Digest,
    ) -> Result<Sha256Digest, StoreError> {
        let config = self.config().clone();
        let staging = Arc::clone(&publication);
        let byte_length =
            u64::try_from(bytes.len()).map_err(|_| invalid("task bundle exceeds storage range"))?;
        let handle = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let (daemon, slot) = staging.check_target(tx)?;
            if staging.already_selected(tx, &daemon, &slot)? {
                return Ok(None);
            }
            let request = PrepareObjectRequest {
                object_hash: hash,
                byte_length,
                media_type: "application/json".to_owned(),
                owner_kind: OBJECT_KIND.to_owned(),
                owner_id: hash.as_prefixed_str(),
            };
            tx_prepare_object(tx, &config, daemon.authority_generation, now, &request).map(Some)
        })?;
        let Some(handle) = handle else {
            self.read_verified_object_bytes(hash)?;
            return Ok(hash);
        };
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C10aAfterPrepPin)?;
        let pin = handle.pin_id.clone();
        let authority = publication.envelope.authority_generation;
        let result = (|| {
            self.write_prepared_object_bytes(&handle, bytes)?;
            verify_object_file_before_publish(self.config(), &handle.relative_path, hash)?;
            let config = self.config().clone();
            let reference = PublishObjectReferenceRequest {
                pin_id: handle.pin_id,
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: OBJECT_KIND.to_owned(),
                owner_id: hash.as_prefixed_str(),
                source_field: CONTENT_FIELD.to_owned(),
            };
            self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
                publication.commit(tx, &config, now, &reference)
            })
        })();
        if result.is_err() {
            let cleanup = self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
                if read_daemon_state_conn(tx)?.authority_generation == authority {
                    tx_release_pin(tx, now, &pin)?;
                }
                Ok(())
            });
            if let Err(error) = cleanup {
                tracing::warn!(bundle_hash = %hash, %error, "task bundle preparation pin release pending");
            }
        }
        result.map(|()| hash)
    }

    /// Queue a task using the exact bundle selected by the harness.
    ///
    /// The envelope supplies the task revision and agent ID. A replay is
    /// recognized before requiring the task to remain ready for assignment.
    ///
    /// # Errors
    /// Refuses a missing or stale selection, incompatible envelope or failed
    /// writer commit.
    pub fn assign_published_task(
        &self,
        mut envelope: CommandEnvelope,
        caller: CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        envelope
            .actor_ref
            .get_or_insert_with(|| caller.actor_id.to_string());
        let assignee = validate_assignment_envelope(&envelope, &caller)?;
        if let Some(receipt) = lookup_receipt(self, &envelope, &caller)? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        match self.resolve_published_assignment(&envelope, &caller, assignee) {
            Ok(preparation) => self.assign_task(envelope, caller, preparation),
            Err(error) => {
                if let Some(receipt) = lookup_receipt(self, &envelope, &caller)? {
                    return Ok(CommitOutcome::IdempotentReplay(receipt));
                }
                Err(error)
            }
        }
    }

    fn resolve_published_assignment(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        assignee: AgentId,
    ) -> Result<PreparedTaskAssignment, StoreError> {
        self.read_snapshot(|conn, _| {
            let daemon = current_daemon(conn, envelope)?;
            let slot = ready_assignment(conn, envelope, caller, assignee)?;
            let hash = selected_bundle(conn, envelope, &slot)?.ok_or_else(|| {
                StoreError::EvidenceUnavailable(
                    "no bundle selected for the active task assignment".to_owned(),
                )
            })?;
            let document = read_document(conn, self.config(), hash)?;
            let index: BundleIndex = serde_json::from_value(document)?;
            Ok(PreparedTaskAssignment {
                daemon_epoch: daemon.daemon_epoch,
                assignment_id: slot.id,
                assignment_epoch: slot.epoch,
                assignment_revision: slot.revision,
                trigger_event_id: index.trigger_event_id,
                effective_bundle_hash: hash,
                invocation_id: InvocationId::from_uuid(self.config().id_generator.new_uuid()),
            })
        })
    }
}

impl Publication {
    fn command(&self) -> AssignmentCommand<'_> {
        AssignmentCommand {
            envelope: &self.envelope,
            caller: &self.caller,
            preparation: &self.preparation,
        }
    }

    fn check_target(
        &self,
        conn: &Connection,
    ) -> Result<(DaemonStateRow, AssignmentRevision), StoreError> {
        let daemon = current_daemon(conn, &self.envelope)?;
        let slot = ready_assignment(conn, &self.envelope, &self.caller, self.assignee)?;
        if daemon.daemon_epoch != self.preparation.daemon_epoch || !slot.matches(&self.preparation)
        {
            return Err(StoreError::StaleFence {
                message: "task bundle was resolved for a different epoch or assignment revision"
                    .to_owned(),
            });
        }
        Ok((daemon, slot))
    }

    fn already_selected(
        &self,
        conn: &Connection,
        daemon: &DaemonStateRow,
        slot: &AssignmentRevision,
    ) -> Result<bool, StoreError> {
        let Some(existing) = selected_bundle(conn, &self.envelope, slot)? else {
            return Ok(false);
        };
        if existing != self.preparation.effective_bundle_hash {
            return Err(invalid(
                "a different bundle is already selected for these task and assignment revisions",
            ));
        }
        verify_staged_bundle(conn, daemon, &self.command(), self.assignee, &self.document)?;
        Ok(true)
    }

    fn commit(
        &self,
        tx: &Transaction<'_>,
        config: &StoreConfig,
        now: TimestampMs,
        reference: &PublishObjectReferenceRequest,
    ) -> Result<(), StoreError> {
        let (daemon, slot) = self.check_target(tx)?;
        if self.already_selected(tx, &daemon, &slot)? {
            return tx_release_pin(tx, now, &reference.pin_id);
        }
        let hash = reference.object_hash.as_prefixed_str();
        tx_publish_owned_reference(
            tx,
            config.id_generator.as_ref(),
            now,
            reference,
            daemon.authority_generation,
        )?;
        tx.execute(
            "INSERT INTO effective_bundles
                (bundle_hash,schema_version,role_version_id,workflow_version_id,
                 requested_model_profile_version_id,allowed_model_policy_hash,
                 initial_context_manifest_id,scope_id,scope_revision,content_object_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?1)
             ON CONFLICT(bundle_hash) DO NOTHING",
            params![
                hash,
                self.index.schema_version,
                self.index.role_version_id,
                self.index.workflow_version_id,
                self.index.requested_model_profile_version_id,
                self.index.allowed_model_policy_hash.as_prefixed_str(),
                self.index.initial_context_manifest_id.to_string(),
                self.index.scope_id,
                i64::try_from(self.index.scope_revision.as_u64())
                    .map_err(|_| invalid("task bundle scope revision exceeds storage range"))?,
            ],
        )?;
        verify_staged_bundle(tx, &daemon, &self.command(), self.assignee, &self.document)?;
        let field = selection_field(&self.envelope, &slot);
        tx.execute(
            "INSERT INTO object_references
                (reference_id,object_hash,owner_kind,owner_id,source_field,state)
             VALUES (?1,?2,?3,?4,?5,'live')",
            params![
                config.id_generator.new_uuid().to_string(),
                hash,
                TASK_KIND,
                self.envelope.aggregate_id.to_string(),
                field
            ],
        )?;
        Ok(())
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn current_daemon(
    conn: &Connection,
    envelope: &CommandEnvelope,
) -> Result<DaemonStateRow, StoreError> {
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.authority_generation != envelope.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "task bundle resolution requires a ready daemon".to_owned(),
        ));
    }
    Ok(daemon)
}

fn lookup_receipt(
    store: &UnleashStore,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<CommandReceipt>, StoreError> {
    let digest = compute_command_digest_v2(envelope)?;
    store.read_snapshot(|conn, _| {
        let daemon = read_daemon_state_conn(conn)?;
        classify_command_replay(conn, &daemon, &caller.actor_id, envelope, digest)
    })
}

fn ready_assignment(
    conn: &Connection,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    assignee: AgentId,
) -> Result<AssignmentRevision, StoreError> {
    let (state, current_revision, assignment_id, epoch, assignment_revision, session, session_revision): (String, i64, String, i64, i64, String, i64) = conn.query_row(
        "SELECT t.state,t.revision,a.assignment_id,a.assignment_epoch,a.revision,s.session_id,s.revision
         FROM tasks t
         JOIN missions m ON m.mission_id=t.mission_id AND m.owner_kind='user'
         JOIN principals owner ON owner.principal_id=m.owner_user_id
            AND owner.principal_kind='user' AND owner.state='active' AND owner.actor_id=?4
         JOIN mission_assignments a ON a.mission_id=m.mission_id AND a.agent_id=?3
            AND a.state='active' AND a.scope_id=t.scope_id AND a.scope_revision=t.scope_revision
         JOIN agents g ON g.agent_id=a.agent_id AND g.project_id=m.project_id
             AND g.state IN ('eligible','active')
         JOIN sessions s ON s.agent_id=g.agent_id AND s.state='open'
         WHERE t.task_id=?1 AND m.project_id=?2",
        params![envelope.aggregate_id.to_string(), envelope.project_id.map(|id| id.to_string()), assignee.to_string(), caller.actor_id.as_str()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).optional()?.ok_or_else(|| StoreError::NotFound("task assignment unavailable for this caller".to_owned()))?;
    let current_revision = Revision::new(stored_counter(current_revision)?);
    if envelope.expected_revision != current_revision {
        return Err(StoreError::RevisionConflict {
            expected_revision: envelope.expected_revision,
            current_revision,
            command_id: Some(envelope.command_id),
        });
    }
    if state != "ready" {
        return Err(invalid("task is not ready for an assignment"));
    }
    let id = assignment_id.parse().map_err(|_| {
        StoreError::InvariantViolation("invalid persisted assignment ID".to_owned())
    })?;
    Ok(AssignmentRevision {
        id,
        epoch: stored_counter(epoch)?,
        revision: Revision::new(stored_counter(assignment_revision)?),
        session,
        session_revision: Revision::new(stored_counter(session_revision)?),
    })
}

fn stored_counter(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| {
        StoreError::InvariantViolation("invalid persisted assignment counter".to_owned())
    })
}

fn selection_field(envelope: &CommandEnvelope, slot: &AssignmentRevision) -> String {
    format!(
        "effective_bundle/{}/{}/{}/{}/{}/{}/{}",
        envelope.authority_generation,
        envelope.expected_revision.as_u64(),
        slot.id,
        slot.epoch,
        slot.revision.as_u64(),
        slot.session,
        slot.session_revision.as_u64(),
    )
}

fn selected_bundle(
    conn: &Connection,
    envelope: &CommandEnvelope,
    slot: &AssignmentRevision,
) -> Result<Option<Sha256Digest>, StoreError> {
    let mut search = conn.prepare(
        "SELECT object_hash,state FROM object_references
         WHERE owner_kind=?1 AND owner_id=?2 AND source_field=?3 LIMIT 2",
    )?;
    let mut matches = search.query(params![
        TASK_KIND,
        envelope.aggregate_id.to_string(),
        selection_field(envelope, slot)
    ])?;
    let Some(row) = matches.next()? else {
        return Ok(None);
    };
    let hash: String = row.get(0)?;
    let state: String = row.get(1)?;
    if matches.next()?.is_some() {
        return Err(StoreError::InvariantViolation(
            "multiple task bundles selected for the same revisions".to_owned(),
        ));
    }
    if state != "live" {
        return Err(StoreError::EvidenceUnavailable(
            "selected task bundle was retired".to_owned(),
        ));
    }
    hash.parse().map(Some).map_err(|_| {
        StoreError::InvariantViolation("invalid selected task bundle digest".to_owned())
    })
}

pub(crate) fn require_selected_bundle(
    conn: &Connection,
    command: &AssignmentCommand<'_>,
    assignee: AgentId,
) -> Result<(), StoreError> {
    let slot = ready_assignment(conn, command.envelope, command.caller, assignee)?;
    if !slot.matches(command.preparation)
        || selected_bundle(conn, command.envelope, &slot)?
            != Some(command.preparation.effective_bundle_hash)
    {
        return Err(StoreError::EvidenceUnavailable(
            "bundle was not selected for this assignment".to_owned(),
        ));
    }
    Ok(())
}
