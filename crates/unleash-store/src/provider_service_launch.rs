//! Explicit, durable preparation of an Unleash-managed provider service.
//! The Store records intent here. It does not give out an execution permit.

use std::{path::PathBuf, sync::atomic::Ordering};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActorId, DaemonMode, Fence, HostId, LineageFence, ProcessId, ProcessRunUuid,
    ProjectId, ReceiptId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProviderKind, ProviderServerManagement, ReceiptState,
    Sensitivity, canonical_sha256,
};
use uuid::Uuid;

use crate::{
    AggregateKind, BrokerCause, BrokerIntent, CallerAuthority, CommandMutation,
    CommitCommandRequest, CommitOutcome, DaemonStateRow, EventAppendSpec, StoreConfig, StoreError,
    UnleashStore,
    commands::ensure_scope_not_frozen_tx,
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_security_admin},
};

pub(crate) const SCHEMA_VERSION: i64 = 19;
const COMMAND: &str = "PrepareProviderServiceLaunch";
const VERB: &str = "model.provider.service.launch";

/// Backend requested by the daemon's launch planner. This value is not backend qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderServiceBackend {
    /// Strict service bootstrap with the pinned macOS Seatbelt profile.
    MacosSeatbeltStrictServiceV1,
}

/// Closed plan supplied by the explicit server-side launch producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderServiceLaunchPlan {
    /// Logical provider profile.
    pub profile_id: Uuid,
    /// Exact immutable profile revision.
    pub profile_version: u64,
    /// Requested native backend.
    pub backend: ProviderServiceBackend,
    /// Absolute path of the intended executable.
    pub executable: PathBuf,
    /// Digest of the executable bytes to verify before execution.
    pub executable_digest: Sha256Digest,
    /// Digest of the complete native command.
    pub command_digest: Sha256Digest,
    /// Digest of the explicit environment plan.
    pub environment_digest: Sha256Digest,
    /// Digest of the confinement plan.
    pub confinement_digest: Sha256Digest,
    /// Latest time at which the action may be dispatched.
    pub deadline: TimestampMs,
}

/// Immutable preparation facts. Reading them never grants the right to spawn a process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderServiceLaunchContext {
    /// Version of the stored context document.
    pub schema_version: u32,
    /// Aggregate created by the explicit preparation command.
    pub launch_id: Uuid,
    /// Receipt of that command, distinct from profile test receipts.
    pub receipt_id: ReceiptId,
    /// Exact action reserved for a subsequent broker operation.
    pub action_id: ActionId,
    /// Logical process identity.
    pub process_id: ProcessId,
    /// Host identity.
    pub host_id: HostId,
    /// Process incarnation reserved for this preparation.
    pub process_run_uuid: ProcessRunUuid,
    /// Authenticated actor who submitted the explicit command.
    pub actor_id: ActorId,
    /// Active principal resolved by the writer.
    pub principal_id: ActorId,
    /// Scope of both the profile and the command.
    pub project_id: Option<ProjectId>,
    /// Exact immutable provider profile key.
    pub profile_version_ref: String,
    /// Digest of the stored profile configuration.
    pub configuration_digest: Sha256Digest,
    /// Canonical loopback endpoint derived from the profile.
    pub endpoint: String,
    /// Control plane authority and epoch at preparation.
    pub lineage_fence: LineageFence,
    /// Effective scope revision.
    pub scope_revision: Revision,
    /// Effective security policy revision.
    pub policy_revision: Revision,
    /// Digest of both policy layers and Guard configuration.
    pub policy_digest: Sha256Digest,
    /// Exact plan covered by the receipt and context digest.
    pub plan: ProviderServiceLaunchPlan,
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        message: message.to_owned(),
        reason: None,
    }
}

pub(crate) fn validate_command(request: &CommitCommandRequest) -> Result<(), StoreError> {
    let envelope = &request.envelope;
    let CommandMutation::PrepareProviderServiceLaunch { caller } = &request.mutation else {
        if envelope.command_type == COMMAND
            || request.target_aggregate == AggregateKind::ProviderServiceLaunch
        {
            return Err(invalid("service launch requires its dedicated preparation"));
        }
        return Ok(());
    };
    require_security_admin(caller, COMMAND)?;
    if envelope.command_type != COMMAND
        || envelope.aggregate_id.is_nil()
        || envelope.expected_revision != Revision::INITIAL
        || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        || request.actor_id != caller.actor_id
        || request.target_aggregate != AggregateKind::ProviderServiceLaunch
        || request.receipt_state != ReceiptState::Accepted
        || request.operation_id.is_some()
        || request.result_ref.is_some()
        || request.fence_assertion.is_some()
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
    {
        return Err(invalid("inconsistent service launch command"));
    }
    Ok(())
}

impl UnleashStore {
    /// Prepare an explicit service launch. This operation creates no action, process or permit.
    ///
    /// # Errors
    /// Rejects invalid authority, profile, or plan, and propagates transaction failures.
    pub fn prepare_provider_service_launch(
        &self,
        caller: CallerAuthority,
        envelope: CommandEnvelope,
    ) -> Result<CommitOutcome, StoreError> {
        self.commit_command(CommitCommandRequest {
            actor_id: caller.actor_id.clone(),
            envelope,
            target_aggregate: AggregateKind::ProviderServiceLaunch,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::PrepareProviderServiceLaunch { caller },
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        })
    }

    /// Read the preparation and verify its digest. This is a historical read, never a permit.
    ///
    /// # Errors
    /// Returns an error on an inconsistent document or storage failure.
    pub fn read_provider_service_launch(
        &self,
        launch_id: Uuid,
    ) -> Result<Option<ProviderServiceLaunchContext>, StoreError> {
        self.read_snapshot(|conn, _snapshot| {
            if crate::commands::read_daemon_state_conn(conn)?.schema_version < SCHEMA_VERSION {
                return Ok(None);
            }
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT context_json,context_digest FROM provider_service_launches WHERE launch_id=?1",
                    [launch_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((document, digest)) = row else {
                return Ok(None);
            };
            let context: ProviderServiceLaunchContext = serde_json::from_str(&document)?;
            if context.launch_id != launch_id || canonical_sha256(&context)?.to_string() != digest {
                return Err(invalid("inconsistent service launch context"));
            }
            Ok(Some(context))
        })
    }
}

fn resolve_launch_profile(
    tx: &Transaction<'_>,
    now: TimestampMs,
    envelope: &CommandEnvelope,
) -> Result<
    (
        ProviderServiceLaunchPlan,
        crate::ConfiguredProviderProfile,
        String,
    ),
    StoreError,
> {
    let plan: ProviderServiceLaunchPlan =
        serde_json::from_value(Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("invalid service launch plan"))?;
    if !plan.executable.is_absolute()
        || !plan.executable.components().all(|component| {
            matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        || plan.executable.file_name().is_none()
        || plan.profile_id.is_nil()
        || plan.profile_version == 0
        || plan.deadline <= now
    {
        return Err(invalid(
            "invalid service launch executable, profile or deadline",
        ));
    }
    let profile = crate::provider_profiles::resolve(tx, plan.profile_id, plan.profile_version)?
        .filter(|profile| profile.configuration.project_id == envelope.project_id)
        .ok_or_else(|| StoreError::NotFound("provider profile unavailable in scope".to_owned()))?;
    let payload = &profile.configuration.payload;
    let destination = payload
        .endpoint_destination()
        .map_err(|_| invalid("invalid service endpoint"))?;
    if payload.provider_kind() != ProviderKind::Ollama
        || payload.server_management() != ProviderServerManagement::UnleashManaged
        || payload.locality() != unleash_domain::DataLocality::LocalOnly
        || !destination.is_loopback_literal()
    {
        return Err(invalid(
            "service preparation requires a managed local Ollama profile",
        ));
    }
    Ok((plan, profile, destination.normalized_endpoint().to_owned()))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Inputs belong to the enclosing command transaction"
)]
pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    receipt: ReceiptId,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.schema_version < SCHEMA_VERSION
        || daemon.mode != DaemonMode::Ready
        || config.clock_uncertain.load(Ordering::Acquire)
    {
        return Err(StoreError::RecoveryInProgress(
            "service launch preparation requires a ready daemon and reliable clock".to_owned(),
        ));
    }
    let (plan, profile, endpoint) = resolve_launch_profile(tx, now, envelope)?;
    ensure_scope_not_frozen_tx(tx, envelope.project_id, None, None)?;
    let principal: String = tx.query_row(
        "SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind IN ('user','service','system')",
        [caller.actor_id.as_str()],
        |row| row.get(0),
    ).optional()?.ok_or_else(|| invalid("no active service launch principal"))?;
    for subject in [principal.as_str(), caller.actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, envelope.project_id)?;
    }
    let launch_id = envelope.aggregate_id;
    let occupied: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM provider_service_launches WHERE launch_id=?1)",
        [launch_id.to_string()],
        |row| row.get(0),
    )?;
    let revision = Revision::new(1);
    if occupied {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: Revision::INITIAL,
            command_id: Some(envelope.command_id),
        });
    }
    let scope_revision = crate::provider_probe_connect::scope_revision(tx, envelope.project_id)?;
    let (policy_revision, policy_digest) =
        crate::provider_probe_connect::policy(tx, envelope.project_id)?;
    let context = ProviderServiceLaunchContext {
        schema_version: 1,
        launch_id,
        receipt_id: receipt,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        process_id: ProcessId::from_uuid(config.id_generator.new_uuid()),
        host_id: HostId::from_uuid(config.id_generator.new_uuid()),
        process_run_uuid: ProcessRunUuid::from_uuid(config.id_generator.new_uuid()),
        actor_id: caller.actor_id.clone(),
        principal_id: ActorId::new(principal)?,
        project_id: envelope.project_id,
        profile_version_ref: profile.profile_version_ref,
        configuration_digest: profile.configuration_digest,
        endpoint,
        lineage_fence: LineageFence {
            authority_generation: daemon.authority_generation,
            epoch: daemon.daemon_epoch,
            fence: Fence::new(0),
        },
        scope_revision,
        policy_revision,
        policy_digest,
        plan,
    };
    let digest = canonical_sha256(&context)?;
    tx.execute(
        "INSERT INTO provider_service_launches(launch_id,receipt_id,action_id,profile_version_ref,context_digest,context_json) VALUES(?1,?2,?3,?4,?5,?6)",
        params![launch_id.to_string(), receipt.to_string(), context.action_id.to_string(),
            context.profile_version_ref, digest.to_string(), serde_json::to_string(&context)?],
    )?;
    let payload =
        json!({"receipt_id": receipt, "action_id": context.action_id, "context_digest": digest});
    events.push(EventAppendSpec {
        event_type: "ProviderServiceLaunchPrepared".to_owned(),
        aggregate_type: AggregateKind::ProviderServiceLaunch,
        aggregate_id: launch_id,
        aggregate_revision: revision,
        project_id: envelope.project_id,
        correlation_id: envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: payload
            .as_object()
            .cloned()
            .ok_or_else(|| invalid("invalid service launch event"))?,
        sensitivity: Sensitivity::Restricted,
        outbox_destinations: vec!["clients".to_owned(), "harness".to_owned()],
    });
    Ok(revision)
}

pub(crate) fn reject_unqualified_intent(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    if intent.manifest.get("verb").and_then(Value::as_str) == Some(VERB) {
        return Err(invalid("service launch execution is not qualified"));
    }
    if daemon.schema_version < SCHEMA_VERSION {
        return Ok(());
    }
    let (receipt, event) = match intent.cause {
        BrokerCause::CommandReceipt { receipt_id } => (Some(receipt_id.to_string()), None),
        BrokerCause::Event { event_id } => (None, Some(event_id.to_string())),
        BrokerCause::Timer { .. } => (None, None),
    };
    let reserved: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM provider_service_launches
            WHERE action_id=?1 OR receipt_id=?2
               OR receipt_id=(SELECT causation_receipt_id FROM events WHERE event_id=?3)
         )",
        params![intent.action_id.to_string(), receipt, event],
        |row| row.get(0),
    )?;
    if reserved {
        return Err(invalid(
            "service launch preparation cannot authorize another effect",
        ));
    }
    Ok(())
}
