//! Publish the initial mission scope manifest with the mission and receipt.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use unleash_domain::{
    ActorId, CommandId, CompletionPolicy, DaemonEpoch, DaemonMode, GoalId, MissionId, MissionState,
    ProjectId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    EventCausationKind, MissionScopeSelector, ReceiptState, Sensitivity, canonicalize_to_vec,
    compute_command_digest_v2,
};

use crate::{
    AggregateKind, CommandMutation, CommitCommandRequest, CommitOutcome, DaemonStateRow,
    PrepareObjectRequest, PublishObjectReferenceRequest, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
    commands::{
        NewMission, classify_command_replay, ensure_scope_not_frozen_tx, insert_mission,
        read_daemon_state_conn,
    },
    objects::{
        PreparedObjectHandle, tx_prepare_object, tx_publish_owned_reference, tx_release_pin,
        verify_object_file_before_publish,
    },
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx},
    workflow_run_plan::{ExecutionScope, MissionScopes, scope_field},
};

const COMMAND: &str = "AssignMission";
const OWNER_KIND: &str = "mission";
const MISSION_REVISION: Revision = Revision::new(1);

#[derive(Debug, Clone)]
struct Assignment {
    mission: MissionId,
    project: ProjectId,
    goal: Option<GoalId>,
    owner: String,
    completion_policy: CompletionPolicy,
    state: MissionState,
}

/// A scope document staged by the Store for one exact mission command.
#[derive(Debug, Clone)]
pub struct PreparedMissionScope {
    assignment: Assignment,
    daemon_epoch: DaemonEpoch,
    project_revision: Revision,
    command_digest: Sha256Digest,
    reference: PublishObjectReferenceRequest,
}

impl UnleashStore {
    /// Commit a human-owned mission with the project scopes named in its command.
    ///
    /// Uses the receipt and event of the existing mission operation. The Store
    /// copies the exact project identity and revision into the immutable scope
    /// document and retains it in the transaction creating the mission.
    ///
    /// # Errors
    /// Refuses an inconsistent assignment, unavailable project, stale revision,
    /// or a scope document that cannot be published.
    pub fn commit_scoped_mission(
        &self,
        mut req: CommitCommandRequest,
    ) -> Result<CommitOutcome, StoreError> {
        let assignment = Assignment::from_request(&req)?;
        let selectors = read_scope_selectors(&req)?;
        let digest = compute_command_digest_v2(&req.envelope)?;
        let replay = || {
            self.read_snapshot(|conn, _| {
                let daemon = read_daemon_state_conn(conn)?;
                classify_command_replay(conn, &daemon, &req.actor_id, &req.envelope, digest)
            })
        };
        if let Some(receipt) = replay()? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }

        let (project_revision, epoch) = match self.read_snapshot(|conn, _| {
            let daemon = read_daemon_state_conn(conn)?;
            check_daemon(
                &daemon,
                req.envelope.authority_generation,
                req.envelope.command_id,
                None,
            )?;
            let revision = project_revision(conn, &assignment, &req.actor_id)?;
            verify_selectors(&selectors, revision, req.envelope.command_id)?;
            Ok((revision, daemon.daemon_epoch))
        }) {
            Ok(bound) => bound,
            Err(error) => {
                return replay()?.map_or(Err(error), |receipt| {
                    Ok(CommitOutcome::IdempotentReplay(receipt))
                });
            }
        };
        let scopes = selectors
            .into_keys()
            .map(|key| {
                (
                    key,
                    ExecutionScope {
                        scope_id: assignment.project.to_string(),
                        scope_revision: project_revision,
                    },
                )
            })
            .collect();
        let manifest = MissionScopes {
            schema_version: 1,
            mission_id: assignment.mission,
            mission_revision: MISSION_REVISION,
            scopes,
        };
        let bytes = canonicalize_to_vec(&manifest)?;
        let hash = Sha256Digest::from_bytes(&bytes);
        let handle = self.stage_mission_scope(&req, epoch, &bytes, hash)?;
        let pin_id = handle.pin_id.clone();
        let authority = req.envelope.authority_generation;
        let result = (|| {
            self.write_prepared_object_bytes(&handle, &bytes)?;
            verify_object_file_before_publish(self.config(), &handle.relative_path, hash)?;
            let [event] = req.events.as_mut_slice() else {
                return Err(invalid("scoped mission must have its assignment event"));
            };
            event.payload.insert("scope_ref".to_owned(), json!(hash));
            req.result_ref = Some(result_ref(hash));
            let reference = PublishObjectReferenceRequest {
                pin_id: handle.pin_id,
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: OWNER_KIND.to_owned(),
                owner_id: assignment.mission.to_string(),
                source_field: scope_field(MISSION_REVISION),
            };
            req.mutation = CommandMutation::CreateScopedMission(Box::new(PreparedMissionScope {
                assignment,
                daemon_epoch: epoch,
                project_revision,
                command_digest: digest,
                reference,
            }));
            self.commit_command(req)
        })();
        if !matches!(result, Ok(CommitOutcome::Committed(_))) {
            let cleanup = self.execute_in_writer_tx(WriteCriticality::Critical, move |tx, now| {
                if read_daemon_state_conn(tx)?.authority_generation == authority {
                    tx_release_pin(tx, now, &pin_id)?;
                }
                Ok(())
            });
            if let Err(error) = cleanup {
                tracing::warn!(%error, "initial mission scope pin release pending");
            }
        }
        result
    }

    fn stage_mission_scope(
        &self,
        req: &CommitCommandRequest,
        epoch: DaemonEpoch,
        bytes: &[u8],
        hash: Sha256Digest,
    ) -> Result<PreparedObjectHandle, StoreError> {
        let cfg = self.config().clone();
        let authority = req.envelope.authority_generation;
        let command = req.envelope.command_id;
        let object = PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len())
                .map_err(|_| invalid("mission manifest exceeds storage range"))?,
            media_type: "application/json".to_owned(),
            owner_kind: OWNER_KIND.to_owned(),
            owner_id: req.envelope.aggregate_id.to_string(),
        };
        let handle = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            check_daemon(&daemon, authority, command, Some(epoch))?;
            tx_prepare_object(tx, &cfg, authority, now, &object)
        })?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C10aAfterPrepPin)?;
        Ok(handle)
    }
}

impl Assignment {
    fn from_request(req: &CommitCommandRequest) -> Result<Self, StoreError> {
        let CommandMutation::CreateMission {
            mission_id,
            goal_id,
            project_id,
            owner_principal_id: Some(owner),
            owner_agent_id: None,
            completion_policy,
            state,
        } = &req.mutation
        else {
            return Err(invalid(
                "initial project scopes require a human-owned mission",
            ));
        };
        let assignment = Self {
            mission: *mission_id,
            project: *project_id,
            goal: *goal_id,
            owner: owner.clone(),
            completion_policy: *completion_policy,
            state: *state,
        };
        assignment.validate_command(req)?;
        if req.result_ref.is_some() {
            return Err(invalid("assignment already carries a result reference"));
        }
        Ok(assignment)
    }

    fn validate_command(&self, req: &CommitCommandRequest) -> Result<(), StoreError> {
        let [event] = req.events.as_slice() else {
            return Err(invalid("scoped mission requires one event"));
        };
        if req.envelope.command_type != COMMAND
            || req.envelope.project_id != Some(self.project)
            || req.envelope.aggregate_id != self.mission.as_uuid()
            || self.mission.as_uuid().is_nil()
            || req
                .envelope
                .actor_ref
                .as_deref()
                .is_some_and(|actor| actor != req.actor_id.as_str())
            || req.target_aggregate != AggregateKind::Mission
            || req.receipt_state != ReceiptState::Accepted
            || req.operation_id.is_some()
            || req.fence_assertion.is_some()
            || req.retention_days_override.is_some()
            || !req.budget_reservations.is_empty()
            || !req.capacity_reservations.is_empty()
            || event.event_type != "MissionAssigned"
            || event.aggregate_type != AggregateKind::Mission
            || event.aggregate_id != self.mission.as_uuid()
            || event.aggregate_revision != MISSION_REVISION
            || event.project_id != Some(self.project)
            || event.causation_kind != EventCausationKind::Command
            || event.causation_id.is_some()
            || event.sensitivity != Sensitivity::Project
            || !event.outbox_destinations.is_empty()
        {
            return Err(invalid("inconsistent scoped mission command"));
        }
        Ok(())
    }
}

fn result_ref(hash: Sha256Digest) -> String {
    format!("mission-scope:{hash}")
}

pub(crate) fn validate_identity(req: &CommitCommandRequest) -> Result<(), StoreError> {
    let scoped = req.envelope.command_type == COMMAND
        && req
            .envelope
            .payload
            .get("scopes")
            .is_some_and(|value| !value.is_null());
    let CommandMutation::CreateScopedMission(publication) = &req.mutation else {
        return if scoped {
            Err(invalid(
                "scoped mission requires its prepared scope manifest",
            ))
        } else {
            Ok(())
        };
    };
    publication.assignment.validate_command(req)?;
    let [event] = req.events.as_slice() else {
        return Err(invalid("missing assignment event"));
    };
    let reference = &publication.reference;
    if !scoped
        || compute_command_digest_v2(&req.envelope)? != publication.command_digest
        || req.result_ref.as_deref() != Some(result_ref(reference.object_hash).as_str())
        || reference.owner_kind != OWNER_KIND
        || reference.owner_id != publication.assignment.mission.to_string()
        || reference.source_field != scope_field(MISSION_REVISION)
        || event.payload.get("scope_ref") != Some(&json!(reference.object_hash))
    {
        return Err(invalid("inconsistent scoped mission manifest"));
    }
    Ok(())
}

fn project_revision(
    conn: &Connection,
    assignment: &Assignment,
    actor: &ActorId,
) -> Result<Revision, StoreError> {
    let rev: i64 = conn
        .query_row(
            "SELECT p.revision FROM projects p JOIN principals u ON u.principal_id=?2
         WHERE p.project_id=?1 AND p.state='active' AND u.actor_id=?3
           AND u.principal_kind='user' AND u.state='active'",
            params![
                assignment.project.to_string(),
                assignment.owner,
                actor.as_str()
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::NotFound("project or owner unavailable for assignment".to_owned())
        })?;
    if let Some(goal) = assignment.goal {
        let in_project: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM goals WHERE goal_id=?1 AND project_id=?2)",
            params![goal.to_string(), assignment.project.to_string()],
            |row| row.get(0),
        )?;
        if !in_project {
            return Err(StoreError::NotFound(
                "goal unavailable in this project".to_owned(),
            ));
        }
    }
    u64::try_from(rev).map(Revision::new).map_err(|_| {
        StoreError::InvariantViolation("invalid persisted project revision".to_owned())
    })
}

fn read_scope_selectors(
    req: &CommitCommandRequest,
) -> Result<BTreeMap<String, MissionScopeSelector>, StoreError> {
    let selectors: BTreeMap<String, MissionScopeSelector> = serde_json::from_value(
        req.envelope
            .payload
            .get("scopes")
            .cloned()
            .ok_or_else(|| invalid("missing mission scopes"))?,
    )
    .map_err(|_| invalid("invalid mission scopes"))?;
    if selectors.is_empty() || selectors.keys().any(|key| key.trim().is_empty()) {
        return Err(invalid("a scoped mission requires nonempty scope names"));
    }
    Ok(selectors)
}

fn verify_selectors(
    selectors: &BTreeMap<String, MissionScopeSelector>,
    revision: Revision,
    command: CommandId,
) -> Result<(), StoreError> {
    for selector in selectors.values() {
        let MissionScopeSelector::Project { revision: expected } = selector;
        require_revision(revision, *expected, command)?;
    }
    Ok(())
}

fn require_revision(
    current: Revision,
    expected: Revision,
    command: CommandId,
) -> Result<(), StoreError> {
    if current == expected {
        Ok(())
    } else {
        Err(StoreError::RevisionConflict {
            current_revision: current,
            expected_revision: expected,
            command_id: Some(command),
        })
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn check_daemon(
    daemon: &DaemonStateRow,
    authority: unleash_domain::AuthorityGeneration,
    command: CommandId,
    epoch: Option<DaemonEpoch>,
) -> Result<(), StoreError> {
    if daemon.authority_generation != authority {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: authority,
            command_id: Some(command),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "scoped assignment requires a ready daemon".to_owned(),
        ));
    }
    if epoch.is_some_and(|expected| expected != daemon.daemon_epoch) {
        return Err(StoreError::StaleFence {
            message: "scoped assignment crosses daemon epochs".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    (config, daemon, now): (&StoreConfig, &DaemonStateRow, TimestampMs),
    envelope: &unleash_protocol::CommandEnvelope,
    actor: &ActorId,
    publication: &PreparedMissionScope,
) -> Result<Revision, StoreError> {
    let assignment = &publication.assignment;
    check_daemon(
        daemon,
        envelope.authority_generation,
        envelope.command_id,
        Some(publication.daemon_epoch),
    )?;
    let current = project_revision(tx, assignment, actor)?;
    require_revision(current, publication.project_revision, envelope.command_id)?;
    ensure_scope_not_frozen_tx(tx, Some(assignment.project), None, None)?;
    ensure_subject_not_blocked_tx(
        tx,
        SubjectKind::Actor,
        actor.as_str(),
        Some(assignment.project),
    )?;
    let revision = insert_mission(
        tx,
        NewMission {
            mission_id: assignment.mission,
            goal_id: assignment.goal,
            project_id: assignment.project,
            owner_principal_id: Some(&assignment.owner),
            owner_agent_id: None,
            completion_policy: assignment.completion_policy,
            state: assignment.state,
            expected_revision: envelope.expected_revision,
            command_id: envelope.command_id,
        },
    )?;
    tx_publish_owned_reference(
        tx,
        config.id_generator.as_ref(),
        now,
        &publication.reference,
        daemon.authority_generation,
    )?;
    Ok(revision)
}
