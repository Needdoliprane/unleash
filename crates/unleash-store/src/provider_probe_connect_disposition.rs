//! Verified disposal before any durable permission to invoke the transport.
//! This path never handles an uncertain dispatch or a native observation.

use crate::provider_probe_connect::{
    ConnectPhasePermission, ProviderProbeConnectClaim, fact_context,
};
use crate::{BrokerPhase, ExecutionContinuation, StoreError, UnleashStore, WriteCriticality};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use unleash_domain::{ActionId, ActionState, Revision, Sequence, Sha256Digest, TimestampMs};
use unleash_protocol::{EventCursor, RetryClass, canonical_sha256};

const PREFIX: &str = "provider-connect-pre-dispatch:v1:";
const EVENT: &str = "ProviderProbeDispatchRefused";

/// One durable refusal and its exact outbox cursor; never an execution permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConnectDispositionCommit {
    /// Original action, never a substitute dispatch identity.
    pub action_id: ActionId,
    /// Action revision committed with disposal.
    pub revision: Revision,
    /// Actual event committed with lease and capacity release.
    pub event_cursor: EventCursor,
    /// An identical disposition was already durable.
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RefusalReason {
    PolicyChanged,
    AuthorizationDenied,
}

struct RefusalInput {
    record: crate::ProviderProbeConnectRecord,
    action: crate::BrokerActionSnapshot,
    intent: crate::BrokerIntent,
    authorization: BrokerPhase,
    binding: Sha256Digest,
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.to_owned())
}

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("entier de disposition hors plage SQLite"))
}

fn no_dispatch(tx: &Transaction<'_>, action: ActionId) -> Result<(), StoreError> {
    let absent: bool = tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=?1) AND NOT EXISTS(SELECT 1 FROM broker_dispatches WHERE action_id=?1) AND NOT EXISTS(SELECT 1 FROM broker_phase_records WHERE action_id=?1 AND (phase_key='dispatch' OR phase_key LIKE 'observation:%'))",
        [action.to_string()], |row| row.get(0),
    )?;
    if !absent {
        return Err(invalid(
            "disposition interdite après marqueur ou observation",
        ));
    }
    Ok(())
}

fn read_input(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    permission: &ConnectPhasePermission,
    continuation: &ExecutionContinuation,
) -> Result<RefusalInput, StoreError> {
    let record = fact_context(tx, daemon, permission)?;
    let context = &record.context;
    let action = crate::broker_writer::inspect_action_conn(tx, context.action_id)?
        .ok_or_else(|| invalid("action de disposition absente"))?;
    no_dispatch(tx, context.action_id)?;
    let encoded: String = tx.query_row(
        "SELECT intent_json FROM broker_intents WHERE action_id=?1",
        [context.action_id.to_string()],
        |row| row.get(0),
    )?;
    let intent: crate::BrokerIntent = serde_json::from_str(&encoded)?;
    crate::provider_probe_connect::validate_fact_intent(permission, &intent)?;
    if action.fence != intent.lineage_fence || action.intent_digest != intent.intent_digest {
        return Err(invalid("action de disposition d'un autre faisceau"));
    }
    let (encoded, digest): (String, String) = tx.query_row(
        "SELECT phase_json,phase_digest FROM broker_phase_records WHERE action_id=?1 AND evaluation_id=?2 AND phase_key='authorization'",
        params![context.action_id.to_string(), action.evaluation_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?)),
    )?;
    let exact: serde_json::Value = serde_json::from_str(&encoded)?;
    let authorization: BrokerPhase = serde_json::from_value(exact["phase"].clone())?;
    let BrokerPhase::Authorization { intent_digest, .. } = &authorization else {
        return Err(invalid("résultat d'autorisation absent de sa phase"));
    };
    if canonical_sha256(&exact)?.as_prefixed_str() != digest
        || *intent_digest != intent.intent_digest
        || authorization.action_id() != context.action_id
        || exact["evaluation"]["evaluation_id"] != serde_json::json!(action.evaluation_id)
    {
        return Err(invalid("autorisation de disposition incohérente"));
    }
    let binding = canonical_sha256(&serde_json::json!({
        "schema_version":1,"probe_id":context.probe_id,"receipt_id":context.receipt_id,
        "actor_id":context.actor_id,"principal_id":context.principal_id,
        "context_digest":record.context_digest,
        "claim_id":record.claim.as_ref().ok_or_else(||invalid("claim de disposition absente"))?.claim_id,
        "claim_revision":record.claim.as_ref().ok_or_else(||invalid("claim de disposition absente"))?.revision,
        "continuation":continuation,
        "intent_digest":intent.intent_digest,"authorization_digest":digest,
    }))?;
    Ok(RefusalInput {
        record,
        action,
        intent,
        authorization,
        binding,
    })
}

fn replay(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    input: &RefusalInput,
) -> Result<Option<ProviderConnectDispositionCommit>, StoreError> {
    let prior: Option<(String,String,i64,i64)> = tx.query_row(
        "SELECT e.snapshot_ref,v.stream_id,v.sequence,v.aggregate_revision FROM evidence e JOIN events v ON e.snapshot_ref LIKE ?2||'%:'||v.event_id AND v.aggregate_type='action' AND v.aggregate_id=e.action_id AND v.event_type=?3 WHERE e.action_id=?1 AND e.kind='observation' AND e.snapshot_ref LIKE ?2||'%' AND e.producer_id=?4 AND e.availability='available' AND e.object_hash IS NULL",
        params![input.intent.action_id.to_string(),PREFIX,EVENT,crate::CONNECT_OBSERVER_PRINCIPAL], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    prior
        .map(|(reference, stream, sequence, revision)| {
            if !reference.starts_with(&format!("{PREFIX}{}:", input.binding))
                || stream != daemon.stream_id.to_string()
                || input.action.state != ActionState::Denied
            {
                return Err(invalid("rejeu divergent de disposition sans dispatch"));
            }
            Ok(ProviderConnectDispositionCommit {
                action_id: input.intent.action_id,
                revision: Revision::new(
                    u64::try_from(revision).map_err(|_| invalid("révision négative"))?,
                ),
                event_cursor: EventCursor::new(
                    daemon.stream_id,
                    Sequence::new(
                        u64::try_from(sequence).map_err(|_| invalid("séquence négative"))?,
                    ),
                ),
                replayed: true,
            })
        })
        .transpose()
}

fn refusal_reason(tx: &Transaction<'_>, input: &RefusalInput) -> Result<RefusalReason, StoreError> {
    let BrokerPhase::Authorization { outcome, .. } = &input.authorization else {
        return Err(invalid("phase de refus préalable non qualifiée"));
    };
    match (input.action.state, outcome) {
        (ActionState::Denied, crate::BrokerAuthorizationOutcome::Failed { failure })
            if failure.retry_class == RetryClass::Never =>
        {
            Ok(RefusalReason::AuthorizationDenied)
        }
        (ActionState::Authorized, crate::BrokerAuthorizationOutcome::Authorized { .. })
            if crate::provider_probe_connect::policy(tx, input.record.context.project_id)?.1
                != input.record.context.policy_digest =>
        {
            Ok(RefusalReason::PolicyChanged)
        }
        _ => Err(invalid(
            "action encore autorisable, réessayable, ambiguë ou en attente",
        )),
    }
}

fn commit_refusal(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    input: &RefusalInput,
    continuation: &ExecutionContinuation,
    reason: RefusalReason,
) -> Result<ProviderConnectDispositionCommit, StoreError> {
    let revision = Revision::new(
        input
            .action
            .revision
            .as_u64()
            .checked_add(1)
            .ok_or_else(|| invalid("révision de disposition débordée"))?,
    );
    let changed=tx.execute("UPDATE actions SET state='denied',revision=?2 WHERE action_id=?1 AND revision=?3 AND state IN ('authorized','denied')",params![input.intent.action_id.to_string(),integer(revision.as_u64())?,integer(input.action.revision.as_u64())?])?;
    if changed != 1 {
        return Err(invalid("CAS de disposition refusé"));
    }
    let payload = serde_json::json!({"schema_version":1,"probe_id":input.record.context.probe_id,
        "action_id":input.intent.action_id,"evaluation_id":input.action.evaluation_id,
        "intent_digest":input.intent.intent_digest,"reason":reason,"disposition_digest":input.binding,
        "reservation_id":continuation.reservation_id,"lease_id":continuation.lease_id,"fence":continuation.fence});
    let (event, sequence) = crate::broker_writer::append_phase_event_tx(
        tx,
        config,
        daemon,
        now,
        &input.intent,
        revision,
        EVENT,
        &payload,
    )?;
    tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,NULL,?4,?5,1,'available')",params![config.id_generator.new_uuid().to_string(),input.intent.action_id.to_string(),format!("{PREFIX}{}:{event}",input.binding),crate::CONNECT_OBSERVER_PRINCIPAL,now.as_i64()])?;
    crate::provider_probe_connect_observation::transition_probe(
        tx,
        &input.record.context,
        continuation,
        "ended",
    )?;
    crate::leases::tx_release_lease(tx, continuation.lease_id)?;
    crate::budgets::tx_release_capacity_reservation_verified(tx, now, continuation.reservation_id)?;
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeCommandCommit)?;
    Ok(ProviderConnectDispositionCommit {
        action_id: input.intent.action_id,
        revision,
        event_cursor: EventCursor::new(
            daemon.stream_id,
            Sequence::new(u64::try_from(sequence).map_err(|_| invalid("séquence négative"))?),
        ),
        replayed: false,
    })
}

impl UnleashStore {
    /// Disposes only an exactly owned, definitively refused, undispatched connect.
    /// The writer derives the refusal; no caller-provided boolean proves no effect.
    /// # Errors
    /// Rejects stale ownership, any marker, unknown outcome, retry or human wait.
    pub fn dispose_provider_probe_connect_without_dispatch(
        &self,
        claim: &ProviderProbeConnectClaim,
        continuation: &ExecutionContinuation,
    ) -> Result<ProviderConnectDispositionCommit, StoreError> {
        let permission = claim.permission();
        let continuation = continuation.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = crate::commands::read_daemon_state_conn(tx)?;
            let input = read_input(tx, &daemon, &permission, &continuation)?;
            if let Some(prior) = replay(tx, &daemon, &input)? {
                return Ok(prior);
            }
            if config
                .clock_uncertain
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(invalid("horloge incertaine avant disposition"));
            }
            crate::provider_probe_connect_observation::validate_continuation(
                tx,
                &daemon,
                now,
                &input.record,
                &continuation,
                true,
            )?;
            let reason = refusal_reason(tx, &input)?;
            commit_refusal(tx, &config, &daemon, now, &input, &continuation, reason)
        })
    }
}
