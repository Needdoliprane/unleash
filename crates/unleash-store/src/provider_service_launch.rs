//! Explicit, durable preparation of an Unleash-managed provider service.
//! The Store records intent here. It does not give out an execution permit.

use std::{path::PathBuf, sync::atomic::Ordering};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, ActorId, AuthorizationEvaluationId, DaemonMode,
    DispatchId, Fence, HostId, LineageFence, ProcessId, ProcessRunUuid, ProjectId, ReceiptId,
    ReconcileStrategy, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProviderKind, ProviderServerManagement, ReceiptState,
    Sensitivity, canonical_sha256, compute_command_digest_v2,
};
use uuid::Uuid;

use crate::{
    AggregateKind, BrokerAuthorizationDecision, BrokerCause, BrokerEvaluation, BrokerExecutor,
    BrokerIntent, BrokerPhase, BrokerPhaseCommit, BrokerPhaseRequest, CallerAuthority,
    CommandMutation, CommitCommandRequest, CommitOutcome, DaemonStateRow, EventAppendSpec,
    GuardPolicyMode, GuardTransport, StoreConfig, StoreCutpoint, StoreError, UnleashStore,
    WriteCriticality,
    commands::{classify_command_replay, ensure_scope_not_frozen_tx, read_daemon_state_conn},
    policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_security_admin},
};

pub(crate) mod attested;
mod observation;
pub use attested::AttestedProviderServiceLaunch;
pub use observation::{
    ProviderServiceLaunchAttestation, ProviderServiceLaunchOutcome, ProviderServiceProcessIdentity,
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
    /// Commit one prepared service launch and return its exclusive launch permit.
    /// The caller supplies fingerprints observed from the bootstrap before claiming.
    ///
    /// # Errors
    /// Refuses a divergent boot plan, decision, or previously marked launch.
    pub fn commit_provider_service_launch_dispatch(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        request: BrokerPhaseRequest,
        fingerprint: ProviderServiceLaunchFingerprint,
    ) -> Result<ProviderServiceLaunchPermit, StoreError> {
        let permit = self.with_live_service_preparation(
            envelope,
            caller,
            move |tx, config, now, context| {
                let BrokerPhase::Dispatch {
                    action_id,
                    intent_digest,
                    decision,
                    dispatch_id,
                    fence,
                } = &request.phase
                else {
                    return Err(invalid("service permit requires a dispatch request"));
                };
                let intent = context.broker_intent()?;
                let expected = ProviderServiceLaunchFingerprint {
                    context_digest: intent.payload_digest,
                    executable_digest: context.plan.executable_digest,
                    command_digest: context.plan.command_digest,
                    environment_digest: context.plan.environment_digest,
                    confinement_digest: context.plan.confinement_digest,
                };
                if fingerprint != expected
                    || *action_id != context.action_id
                    || *intent_digest != intent.intent_digest
                    || *fence != context.lineage_fence
                    || request.authority_generation != context.lineage_fence.authority_generation
                {
                    return Err(invalid(
                        "service permit differs from the durable preparation",
                    ));
                }
                let expires_at = decision.expiry.map_or(context.plan.deadline, |expiry| {
                    expiry.min(context.plan.deadline)
                });
                let commit = crate::broker_writer::commit_service_launch_dispatch_tx(
                    tx,
                    config,
                    now,
                    &request,
                    &ServiceLaunchPhaseAccess {
                        intent_digest: intent.intent_digest,
                    },
                )?;
                if commit.replayed {
                    return Err(invalid(
                        "a recorded dispatch cannot issue another service permit",
                    ));
                }
                Ok(ProviderServiceLaunchPermit {
                    context,
                    dispatch_id: *dispatch_id,
                    expires_at,
                    commit,
                })
            },
        )?;
        self.config()
            .cutpoints
            .hit(StoreCutpoint::C4AfterBrokerDispatchCommit)?;
        Ok(permit)
    }

    /// Recheck the live preparation and exact dispatch before the owned launcher uses its permit.
    ///
    /// # Errors
    /// Refuses changed authority, policy, profile, dispatch or an expired permit.
    pub fn validate_provider_service_launch_permit(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        permit: &ProviderServiceLaunchPermit,
    ) -> Result<(), StoreError> {
        let expected = permit.context.clone();
        let dispatch_id = permit.dispatch_id;
        let revision = permit.commit.revision;
        let cursor = permit.commit.event_cursor;
        let expires_at = permit.expires_at;
        self.with_live_service_preparation(envelope, caller, move |tx, _, now, context| {
            if now >= expires_at || context != expected {
                return Err(invalid(
                    "service launch permit no longer matches its preparation",
                ));
            }
            let intent = context.broker_intent()?;
            let active = crate::broker_writer::inspect_action_conn(tx, context.action_id)?;
            if !active.is_some_and(|action| {
                action.state == ActionState::Dispatching
                    && action.dispatch_id == Some(dispatch_id)
                    && action.revision == revision
                    && action.last_event_cursor == cursor
                    && action.intent_digest == intent.intent_digest
                    && action.fence == context.lineage_fence
            }) {
                return Err(invalid("service launch dispatch is no longer current"));
            }
            Ok(())
        })
    }

    /// Commit an entry from the daemon's dedicated service evaluation journal.
    /// The writer binds both phases to the preparation. Dispatch remains separate.
    ///
    /// # Errors
    /// Refuses entries outside the two evaluation phases of the current preparation.
    pub fn commit_provider_service_launch_broker_phase(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        request: BrokerPhaseRequest,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        self.with_live_service_preparation(envelope, caller, move |tx, config, now, context| {
            let expected = context.broker_intent()?;
            let matches = match &request.phase {
                BrokerPhase::Authorizing { intent } => *intent == expected,
                BrokerPhase::Authorization {
                    action_id,
                    intent_digest,
                    ..
                } => *action_id == expected.action_id && *intent_digest == expected.intent_digest,
                _ => false,
            };
            if !matches
                || request.authority_generation != context.lineage_fence.authority_generation
            {
                return Err(invalid(
                    "service evaluation phase differs from the preparation",
                ));
            }
            crate::broker_writer::commit_service_launch_phase_tx(
                tx,
                config,
                now,
                &request,
                &ServiceLaunchPhaseAccess {
                    intent_digest: expected.intent_digest,
                },
            )
        })
    }

    /// Commit the first durable broker phase for the prepared service.
    /// A repeat returns the original phase receipt and cannot start a process.
    ///
    /// # Errors
    /// Refuses any preparation that is no longer applicable.
    pub fn commit_provider_service_launch_authorizing(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        self.with_live_service_preparation(envelope, caller, |tx, config, now, context| {
            let intent = context.broker_intent()?;
            let phase_access = ServiceLaunchPhaseAccess {
                intent_digest: intent.intent_digest,
            };
            let prior = crate::broker_writer::inspect_action_conn(tx, context.action_id)?;
            let evaluation_id = match prior {
                None => AuthorizationEvaluationId::from_uuid(config.id_generator.new_uuid()),
                Some(snapshot)
                    if snapshot.state == ActionState::Authorizing
                        && snapshot.revision == Revision::new(1)
                        && snapshot.fence == context.lineage_fence
                        && snapshot.intent_digest == intent.intent_digest
                        && snapshot.dispatch_id.is_none() =>
                {
                    snapshot.evaluation_id
                }
                Some(_) => {
                    return Err(invalid(
                        "service launch already has a different broker state",
                    ));
                }
            };
            let request = BrokerPhaseRequest {
                authority_generation: context.lineage_fence.authority_generation,
                expected_revision: Revision::INITIAL,
                evaluation: BrokerEvaluation {
                    evaluation_id,
                    previous_evaluation_id: None,
                },
                phase: BrokerPhase::Authorizing { intent },
            };
            crate::broker_writer::commit_service_launch_phase_tx(
                tx,
                config,
                now,
                &request,
                &phase_access,
            )
        })
    }

    /// Revalidate the original preparation in one snapshot of the writer.
    /// The result is a candidate for authorization and cannot start a process.
    ///
    /// # Errors
    /// Refuses missing or inapplicable preparations.
    pub fn read_provider_service_launch_for_authorization(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<ProviderServiceLaunchContext, StoreError> {
        self.with_live_service_preparation(envelope, caller, |_, _, _, context| Ok(context))
    }

    /// Read a new service preparation and its effective policy in the same writer snapshot.
    /// Once an evaluation has entered the journal it must be resolved from that journal.
    ///
    /// # Errors
    /// Refuses stale preparations and preparations that already have a broker action.
    pub fn read_provider_service_launch_evaluation(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<(ProviderServiceLaunchContext, crate::PolicySummary), StoreError> {
        self.with_live_service_preparation(envelope, caller, |tx, _, _, context| {
            if crate::broker_writer::inspect_action_conn(tx, context.action_id)?.is_some() {
                return Err(StoreError::EvidenceUnavailable(
                    "service launch already has a durable broker evaluation".to_owned(),
                ));
            }
            let policy = crate::policies_secrets::read_policy_summary_conn(tx, context.project_id)?;
            Ok((context, policy))
        })
    }

    fn with_live_service_preparation<T: Send + 'static>(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
        operation: impl FnOnce(
            &Transaction<'_>,
            &StoreConfig,
            TimestampMs,
            ProviderServiceLaunchContext,
        ) -> Result<T, StoreError>
        + Send
        + 'static,
    ) -> Result<T, StoreError> {
        require_security_admin(caller, COMMAND)?;
        if envelope.command_type != COMMAND
            || envelope.actor_ref.as_deref() != Some(caller.actor_id.as_str())
        {
            return Err(invalid(
                "service launch command identity does not match the caller",
            ));
        }
        let digest = compute_command_digest_v2(envelope)?;
        let envelope = envelope.clone();
        let caller = caller.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            require_ready(&config, &daemon)?;
            let context = read_live_preparation(tx, &daemon, now, &envelope, &caller, digest)?;
            operation(tx, &config, now, context)
        })
    }

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
            if read_daemon_state_conn(conn)?.schema_version < SCHEMA_VERSION {
                return Ok(None);
            }
            read_context(conn, launch_id)
        })
    }
}

impl ProviderServiceLaunchContext {
    /// Build the broker intent corresponding to this preparation.
    ///
    /// # Errors
    /// Fails if the intent or preparation cannot be canonicalized.
    pub fn broker_intent(&self) -> Result<BrokerIntent, StoreError> {
        authorizing_intent(self)
    }
}

/// Native launch material observed by the daemon before requesting a permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderServiceLaunchFingerprint {
    /// Digest of the V019 preparation linked by the bootstrap.
    pub context_digest: Sha256Digest,
    /// Digest of the opened executable's bytes.
    pub executable_digest: Sha256Digest,
    /// Digest of the arguments, program, working directory and environment keys.
    pub command_digest: Sha256Digest,
    /// Digest of the full explicit child environment.
    pub environment_digest: Sha256Digest,
    /// Digest of the exact rendered confinement profile.
    pub confinement_digest: Sha256Digest,
}

/// Unserializable, single-owner handoff from the writer to the launcher.
pub struct ProviderServiceLaunchPermit {
    context: ProviderServiceLaunchContext,
    dispatch_id: DispatchId,
    expires_at: TimestampMs,
    commit: BrokerPhaseCommit,
}

impl ProviderServiceLaunchPermit {
    /// The durable preparation covered by the permit.
    #[must_use]
    pub const fn context(&self) -> &ProviderServiceLaunchContext {
        &self.context
    }

    /// Exact committed dispatch.
    #[must_use]
    pub const fn dispatch_id(&self) -> DispatchId {
        self.dispatch_id
    }

    /// Earliest expiry of the plan or the decision.
    #[must_use]
    pub const fn expires_at(&self) -> TimestampMs {
        self.expires_at
    }

    /// Durable commit to publish before the effect.
    #[must_use]
    pub const fn commit(&self) -> &BrokerPhaseCommit {
        &self.commit
    }
}

pub(crate) struct ServiceLaunchPhaseAccess {
    intent_digest: Sha256Digest,
}

impl ServiceLaunchPhaseAccess {
    pub(crate) fn validate_intent(&self, intent: &BrokerIntent) -> Result<(), StoreError> {
        if intent.intent_digest != self.intent_digest {
            return Err(invalid(
                "service launch intent differs from the prepared plan",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_dispatch_executor(
        &self,
        tx: &Transaction<'_>,
        intent: &BrokerIntent,
        decision: &BrokerAuthorizationDecision,
    ) -> Result<(), StoreError> {
        self.validate_intent(intent)?;
        let policy = crate::policies_secrets::read_policy_summary_conn(tx, intent.project_id)?;
        let external_guard = policy.guard_config.is_some_and(|guard| {
            guard.guard_mode != GuardPolicyMode::BrokerOnly
                || guard.endpoint.is_some()
                || guard.transport != GuardTransport::None
                || guard.secret_ref_id.is_some()
                || guard.fallback_policy_ref.is_some()
        });
        if decision.executor != BrokerExecutor::Unleash
            || !decision.applied_obligations.is_empty()
            || decision.fallback_policy_ref.is_some()
            || external_guard
        {
            return Err(invalid("service launch executor profile is not qualified"));
        }
        Ok(())
    }
}

fn authorizing_intent(context: &ProviderServiceLaunchContext) -> Result<BrokerIntent, StoreError> {
    let payload_digest = canonical_sha256(context)?;
    let reconcile_strategy = ReconcileStrategy::ObserveThenDecide;
    let manifest = json!({
        "action_id": context.action_id,
        "subject": {
            "kind": "control_plane", "principal_id": context.principal_id,
            "cause_ref": format!("receipt:{}", context.receipt_id),
            "invocation_id": null, "attempt_id": null, "replay_worker_id": null,
            "agent_id": null, "mission_id": null, "role_version": null
        },
        "project_id": context.project_id,
        "verb": VERB,
        "resource_refs": [{"kind": "path", "value": context.plan.executable}],
        "payload_digest": payload_digest,
        "reconcile_strategy": reconcile_strategy,
        "effects_class": "process_exec",
        "scope_revision": context.scope_revision,
        "policy_revision": context.policy_revision,
        "lineage_fence": context.lineage_fence,
        "action_deadline": context.plan.deadline,
        "isolation_requirements": {
            "mode": "strict",
            "require_process_confinement": true,
            "required_capabilities": ["filesystem_write_scoping", "forbidden_paths_read_deny",
                "network_egress_proxy_only", "clean_environment", "subprocess_inheritance"]
        },
        "secret_refs": [],
        "provenance": {"origin": "engine_internal"}
    });
    let Value::Object(manifest) = manifest else {
        return Err(invalid("invalid service launch intent document"));
    };
    Ok(BrokerIntent {
        action_id: context.action_id,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: context.principal_id.clone(),
        },
        principal_id: context.principal_id.clone(),
        cause: BrokerCause::CommandReceipt {
            receipt_id: context.receipt_id,
        },
        project_id: context.project_id,
        payload_digest,
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: context.lineage_fence,
        action_deadline: context.plan.deadline,
        reconcile_strategy,
        intent_digest: canonical_sha256(&manifest)?,
        manifest,
    })
}

fn read_context(
    conn: &Connection,
    launch_id: Uuid,
) -> Result<Option<ProviderServiceLaunchContext>, StoreError> {
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
}

fn require_ready(config: &StoreConfig, daemon: &DaemonStateRow) -> Result<(), StoreError> {
    if daemon.schema_version < SCHEMA_VERSION
        || daemon.mode != DaemonMode::Ready
        || config.clock_uncertain.load(Ordering::Acquire)
    {
        return Err(StoreError::RecoveryInProgress(
            "service launch preparation requires a ready daemon and reliable clock".to_owned(),
        ));
    }
    Ok(())
}

fn live_principal(
    tx: &Transaction<'_>,
    caller: &CallerAuthority,
    project: Option<ProjectId>,
) -> Result<ActorId, StoreError> {
    ensure_scope_not_frozen_tx(tx, project, None, None)?;
    let principal: String = tx.query_row(
        "SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind IN ('user','service','system')",
        [caller.actor_id.as_str()],
        |row| row.get(0),
    ).optional()?.ok_or_else(|| invalid("no active service launch principal"))?;
    for subject in [principal.as_str(), caller.actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, project)?;
    }
    Ok(ActorId::new(principal)?)
}

fn read_live_preparation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    digest: Sha256Digest,
) -> Result<ProviderServiceLaunchContext, StoreError> {
    let unavailable =
        || StoreError::EvidenceUnavailable("service launch preparation unavailable".to_owned());
    let receipt = classify_command_replay(tx, daemon, &caller.actor_id, envelope, digest)?
        .filter(|receipt| receipt.state == ReceiptState::Accepted)
        .ok_or_else(unavailable)?;
    let context = read_context(tx, envelope.aggregate_id)?.ok_or_else(unavailable)?;
    let lineage = LineageFence::new(
        daemon.authority_generation,
        daemon.daemon_epoch,
        Fence::ZERO,
    );
    if context.schema_version != 1
        || context.receipt_id != receipt.receipt_id
        || context.actor_id != caller.actor_id
        || context.project_id != envelope.project_id
        || context.lineage_fence != lineage
        || context.plan.deadline <= now
    {
        return Err(unavailable());
    }
    let (plan, profile, endpoint) = resolve_launch_profile(tx, now, envelope)?;
    let principal = live_principal(tx, caller, envelope.project_id)?;
    let scope_revision = crate::provider_probe_connect::scope_revision(tx, envelope.project_id)?;
    let (policy_revision, policy_digest) =
        crate::provider_probe_connect::policy(tx, envelope.project_id)?;
    if context.principal_id != principal
        || context.plan != plan
        || context.profile_version_ref != profile.profile_version_ref
        || context.configuration_digest != profile.configuration_digest
        || context.endpoint != endpoint
        || context.scope_revision != scope_revision
        || context.policy_revision != policy_revision
        || context.policy_digest != policy_digest
    {
        return Err(unavailable());
    }
    Ok(context)
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
    require_ready(config, daemon)?;
    let (plan, profile, endpoint) = resolve_launch_profile(tx, now, envelope)?;
    let principal = live_principal(tx, caller, envelope.project_id)?;
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
        principal_id: principal,
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
