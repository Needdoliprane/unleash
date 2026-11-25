//! Durable outcome of the daemon's owned service launch.

use super::{
    ActionState, BrokerAuthorizationDecision, BrokerEvaluation, BrokerPhase, BrokerPhaseCommit,
    BrokerPhaseRequest, DispatchId, OptionalExtension, Ordering, Revision, Sha256Digest,
    StoreError, TimestampMs, UnleashStore, Uuid, WriteCriticality, canonical_sha256, invalid,
    params, read_context, read_daemon_state_conn,
};
use crate::{BrokerActionReceipt, BrokerObservationCause};
use serde::{Deserialize, Serialize};

/// Native process identity supplied by the daemon after its owned-host handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderServiceProcessIdentity {
    /// Process number.
    pub pid: u32,
    /// Process group.
    pub pgid: u32,
    /// Owner of the process.
    pub owner_uid: u32,
    /// Boot identity at observation.
    pub boot_identity: String,
    /// Incarnation within that boot.
    pub start_identity: String,
}

impl ProviderServiceProcessIdentity {
    fn valid_for(&self, boot_identity: &str) -> bool {
        self.pid != 0
            && self.pgid != 0
            && self.boot_identity == boot_identity
            && !self.boot_identity.is_empty()
            && self.boot_identity.len() <= 256
            && !self.boot_identity.chars().any(char::is_control)
            && !self.start_identity.is_empty()
            && self.start_identity.len() <= 256
            && !self.start_identity.chars().any(char::is_control)
    }
}

/// Evidence of the owned host and its authenticated service child, before network placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderServiceLaunchAttestation {
    /// Exact durable preparation covered by the host handshake.
    pub context_digest: Sha256Digest,
    /// Effective environment confirmed by the host.
    pub environment_digest: Sha256Digest,
    /// Effective confinement confirmed by the host.
    pub confinement_digest: Sha256Digest,
    /// Direct process-host started by the daemon.
    pub host: ProviderServiceProcessIdentity,
    /// Direct child attested by that owned host.
    pub child: ProviderServiceProcessIdentity,
}

/// Report from the launch controller. An unconfirmed launch remains reserved for reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderServiceLaunchOutcome {
    /// Authenticated process-host observation.
    Attested(Box<ProviderServiceLaunchAttestation>),
    /// No affirmative evidence of launch.
    Unconfirmed,
    /// No process was started: a certain refusal, with nothing to reconcile (REQ-MODEL-015).
    NotStarted {
        /// Closed cause, recorded by the writer.
        cause: ProviderServiceLaunchRefusal,
    },
}

/// Closed cause of a service launch refused before any child process was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderServiceLaunchRefusal {
    /// Permit expired, clock uncertain or preparation refused by the writer.
    StalePermit,
    /// The retained bootstrap is not the prepared native plan.
    BootstrapMismatch,
    /// The rendezvous directory is occupied or rejected.
    RuntimeInUse,
    /// The system source of randomness is unavailable for the control secret.
    RandomUnavailable,
    /// Preparing the runtime directory or spawning the host failed.
    LaunchUnavailable,
}

/// Durable record of a launch refused before any child process was started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderServiceLaunchRefusalRecord {
    /// Document version.
    pub schema_version: u32,
    /// Durable preparation covered by the refused permit.
    pub context_digest: Sha256Digest,
    /// Closed cause.
    pub cause: ProviderServiceLaunchRefusal,
}

impl UnleashStore {
    /// Record a result against the exact permanent dispatch marker in the current incarnation.
    ///
    /// The daemon owns the runtime verification. This writer binds its attestation to the
    /// immutable preparation and appends it with the broker phase and outbox in one transaction.
    /// A launch refused before any process was started is recorded as an observed failure
    /// without effect; an unconfirmed launch remains an unknown outcome.
    ///
    /// # Errors
    /// Rejects foreign markers, diverging observations, and observations from a stale incarnation.
    pub fn commit_provider_service_launch_observation(
        &self,
        launch_id: Uuid,
        dispatch_id: DispatchId,
        outcome: ProviderServiceLaunchOutcome,
    ) -> Result<BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            let context = read_context(tx, launch_id)?
                .ok_or_else(|| invalid("service launch preparation unavailable"))?;
            if context.lineage_fence.authority_generation != daemon.authority_generation
                || context.lineage_fence.epoch != daemon.daemon_epoch
            {
                return Err(invalid(
                    "service launch observation has a stale incarnation",
                ));
            }
            let action = crate::broker_writer::inspect_action_conn(tx, context.action_id)?
                .filter(|action| {
                    action.dispatch_id == Some(dispatch_id) && action.fence == context.lineage_fence
                })
                .ok_or_else(|| invalid("service launch dispatch does not match"))?;
            let intent = context.broker_intent()?;
            if action.intent_digest != intent.intent_digest {
                return Err(invalid("service launch intent does not match"));
            }
            let marker: Option<(String, Option<i64>)> = tx
                .query_row(
                    "SELECT b.decision_json, d.ended_at_ms
                     FROM broker_dispatches b JOIN action_dispatches d USING(dispatch_id)
                     WHERE b.dispatch_id=?1 AND b.action_id=?2 AND b.evaluation_id=?3",
                    params![
                        dispatch_id.to_string(),
                        action.action_id.to_string(),
                        action.evaluation_id.to_string()
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (decision, recorded_at) =
                marker.ok_or_else(|| invalid("durable service launch marker unavailable"))?;
            let decision: BrokerAuthorizationDecision = serde_json::from_str(&decision)?;
            let mut refusal_record = None;
            let (state, digest, evidence) = match &outcome {
                ProviderServiceLaunchOutcome::Attested(proof) => {
                    if proof.context_digest != intent.payload_digest
                        || proof.environment_digest != context.plan.environment_digest
                        || proof.confinement_digest != context.plan.confinement_digest
                        || !proof.host.valid_for(&daemon.boot_identity)
                        || !proof.child.valid_for(&daemon.boot_identity)
                        || proof.host.pid == proof.child.pid
                        || proof.host.pgid != proof.host.pid
                        || proof.host.owner_uid != daemon.owner_uid
                        || proof.host.owner_uid != proof.child.owner_uid
                        || (recorded_at.is_none() && config.clock_uncertain.load(Ordering::Acquire))
                    {
                        return Err(invalid("service launch attestation diverges"));
                    }
                    (
                        ActionState::ObservedSuccess,
                        Some(canonical_sha256(proof)?),
                        Some(crate::broker_writer::ServiceLaunchEvidence::Attested(proof)),
                    )
                }
                ProviderServiceLaunchOutcome::NotStarted { cause } => {
                    refusal_record = Some(ProviderServiceLaunchRefusalRecord {
                        schema_version: 1,
                        context_digest: intent.payload_digest,
                        cause: *cause,
                    });
                    let record = refusal_record.as_ref();
                    (
                        ActionState::ObservedFailure,
                        record.map(canonical_sha256).transpose()?,
                        record.map(crate::broker_writer::ServiceLaunchEvidence::Refused),
                    )
                }
                ProviderServiceLaunchOutcome::Unconfirmed => {
                    (ActionState::OutcomeUnknown, None, None)
                }
            };
            let request = BrokerPhaseRequest {
                authority_generation: daemon.authority_generation,
                expected_revision: Revision::new(3),
                evaluation: BrokerEvaluation {
                    evaluation_id: action.evaluation_id,
                    previous_evaluation_id: None,
                },
                phase: BrokerPhase::Observation {
                    action_id: action.action_id,
                    intent_digest: intent.intent_digest,
                    observation_revision: Revision::new(1),
                    cause: BrokerObservationCause::Execution,
                    reason: None,
                    receipt: BrokerActionReceipt {
                        action_id: action.action_id,
                        evaluation_id: action.evaluation_id,
                        dispatch_id,
                        decision_ref: decision.decision_ref,
                        state,
                        output_digest: digest,
                        enforcement_refs: Vec::new(),
                        remote_audit_ref: None,
                        remote_audit_unavailable: true,
                        observed_at_ms: recorded_at.map_or(now, TimestampMs::new),
                    },
                },
            };
            crate::broker_writer::commit_service_launch_observation_tx(
                tx,
                &config,
                now,
                &request,
                &super::ServiceLaunchPhaseAccess {
                    intent_digest: intent.intent_digest,
                },
                evidence,
            )
        })
    }
}
