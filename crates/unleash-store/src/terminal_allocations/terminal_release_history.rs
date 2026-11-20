//! Read-only attribution of a consumed Release before native inspection.

use rusqlite::{Connection, params};
use serde_json::json;
use unleash_domain::{
    ActionId, ActionOrigin, ActionState, AuthorizationEvaluationId, DispatchId, Fence,
    ProcessRunUuid, ProjectId, Sha256Digest, TerminalId, TimestampMs,
};
use unleash_protocol::{LineageStatus, ReceiptState, canonical_sha256};

use super::terminal_control_history::{HistoricalTransferHeader, validate_historical_dispatch};
use super::terminal_release::read_conn_release;
use super::terminal_release_types::{
    TERMINAL_RELEASE_VERB, TerminalReleaseContext, TerminalReleaseNativeKey, TerminalReleaseRecord,
    TerminalReleaseState,
};
use super::{
    TerminalAllocationScope, control_identity, control_lease, invalid, key, principal, read_conn,
};
use crate::commands::read_daemon_state_conn;
use crate::{
    BrokerActionSnapshot, BrokerCause, BrokerIntent, CallerAuthority, DaemonStateRow, StoreError,
};

pub(crate) mod route;
mod seal;
pub use route::{
    TERMINAL_RELEASE_HISTORY_PREFIX, TerminalReleaseReconcilePlan,
    TerminalReleaseReconciliationResult,
};
pub use seal::{
    SealTerminalReleaseHistoryRequest, TERMINAL_RELEASE_HISTORY_EVENT,
    TerminalReleaseHistoryCommit, TerminalReleaseHistoryFact,
};

/// Exact target selected for historical terminal retirement inspection.
#[derive(Debug, Clone, Copy)]
pub struct TerminalReleaseHistoryTarget {
    /// Project selected by the authorized caller.
    pub project_id: ProjectId,
    /// Logical terminal containing the original physical process.
    pub terminal_id: TerminalId,
    /// Exact retained physical generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Original retirement action.
    pub action_id: ActionId,
    /// Unique original dispatch marker.
    pub dispatch_id: DispatchId,
}

/// Facts reconstituted from a single Store snapshot for inspection of the host.
#[derive(Debug)]
pub struct TerminalReleaseHistorySource {
    /// Immutable prior retirement intent with the expected native identity.
    pub context: TerminalReleaseContext,
    /// Exact old key matching the native acknowledgement.
    pub key: TerminalReleaseNativeKey,
    /// The authorization evaluation correlated with the original marker.
    pub evaluation_id: AuthorizationEvaluationId,
    /// Digest of the originally committed broker intent.
    pub intent_digest: Sha256Digest,
    /// Start of the original dispatch, distinct from any effect observation.
    pub dispatched_at_ms: TimestampMs,
    action_revision: unleash_domain::Revision,
}

/// Qualify the durable source of an ambiguous retirement inside a read
/// snapshot or the writer transaction. The returned key grants no control.
///
/// # Errors
/// Rejects an unbound source, foreign scope, or a missing prior receipt or dispatch.
pub fn qualify_terminal_release_history_source(
    conn: &Connection,
    caller: &CallerAuthority,
    target: TerminalReleaseHistoryTarget,
) -> Result<TerminalReleaseHistorySource, StoreError> {
    crate::policies_secrets::require_reconciler(caller, "ReconcileProcess")?;
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.schema_version < 18 {
        return Err(hidden());
    }
    let record = visible_release(conn, caller, target)?;
    let c = &record.context;
    if c.schema_version != 1 || c.resource_fence == Fence::new(0) {
        return Err(invalid(
            "historical release context schema or fence differs",
        ));
    }
    if c.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: c.authority_generation,
            command_id: Some(c.command_id),
        });
    }
    if c.daemon_epoch > daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "historical release is from a future daemon epoch".into(),
        });
    }
    let broker =
        crate::broker_writer::inspect_action_conn(conn, target.action_id)?.ok_or_else(hidden)?;
    if broker.state != ActionState::OutcomeUnknown
        || !matches!(
            record.state,
            TerminalReleaseState::RetireResolved | TerminalReleaseState::OutcomeUnknown
        )
        || record.dispatch_id != Some(target.dispatch_id)
        || broker.dispatch_id != Some(target.dispatch_id)
    {
        return Err(invalid(
            "historical release has no consumed unknown dispatch",
        ));
    }
    validate_receipt(conn, c)?;
    validate_registration(conn, &daemon, c)?;
    let encoded: String = conn.query_row(
        "SELECT intent_json FROM broker_intents WHERE action_id=?1",
        [target.action_id.to_string()],
        |row| row.get(0),
    )?;
    let intent: BrokerIntent = serde_json::from_str(&encoded)?;
    validate_intent(&record, &broker, &intent)?;
    let (evaluation, dispatched_at_ms) =
        validate_historical_dispatch(conn, &broker, &intent, target.dispatch_id)?;
    let key = TerminalReleaseNativeKey {
        header: HistoricalTransferHeader {
            protocol_version: 1,
            process_run_uuid: c.process_run_uuid,
            authority_generation: c.authority_generation,
            daemon_epoch: c.daemon_epoch,
            resource_fence: c.resource_fence,
            command_id: c.command_id,
            action_id: c.action_id,
            payload_hash: record.context_digest,
        },
        dispatch_id: target.dispatch_id,
        initial_context_digest: c.initial_context_digest,
        lease_id: c.lease_id,
        control_revision: c.control_revision,
    };
    Ok(TerminalReleaseHistorySource {
        context: record.context,
        key,
        evaluation_id: evaluation.evaluation_id,
        intent_digest: intent.intent_digest,
        dispatched_at_ms,
        action_revision: broker.revision,
    })
}

fn visible_release(
    conn: &Connection,
    caller: &CallerAuthority,
    target: TerminalReleaseHistoryTarget,
) -> Result<TerminalReleaseRecord, StoreError> {
    let active_principal = match principal(conn, &caller.actor_id) {
        Ok(id) => id,
        Err(StoreError::InvalidArgument { .. } | StoreError::NotFound(_)) => return Err(hidden()),
        Err(error) => return Err(error),
    };
    let visible: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_release_intents
         WHERE action_id=?1 AND actor_id=?2 AND principal_id=?3 AND terminal_id=?4
           AND json_extract(context_json,'$.project_id')=?5
           AND json_extract(context_json,'$.process_run_uuid')=?6)",
        params![
            target.action_id.to_string(),
            caller.actor_id.as_str(),
            active_principal.as_str(),
            target.terminal_id.to_string(),
            target.project_id.to_string(),
            target.process_run_uuid.to_string()
        ],
        |row| row.get(0),
    )?;
    if !visible {
        return Err(hidden());
    }
    let record = read_conn_release(conn, target.action_id)?.ok_or_else(hidden)?;
    let c = &record.context;
    if c.actor_id != caller.actor_id
        || c.principal_id != active_principal
        || c.project_id != target.project_id
        || c.terminal_id != target.terminal_id
        || c.process_run_uuid != target.process_run_uuid
    {
        return Err(hidden());
    }
    Ok(record)
}

fn hidden() -> StoreError {
    StoreError::NotFound("historical terminal release is not visible".into())
}

fn validate_receipt(conn: &Connection, c: &TerminalReleaseContext) -> Result<(), StoreError> {
    let crate::ReceiptInspectionOutcome::Full(receipt) =
        crate::commands::read_command_receipt_inspection(conn, &c.actor_id, c.command_id)?
    else {
        return Err(StoreError::EvidenceUnavailable(
            "historical release receipt is tombstoned".into(),
        ));
    };
    let exact: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1
         AND actor_id=?2 AND command_id=?3 AND command_type='ReleaseTerminal'
         AND aggregate_id=?4 AND authority_generation=?5)",
        params![
            c.receipt_id.to_string(),
            c.actor_id.as_str(),
            c.command_id.to_string(),
            c.terminal_id.to_string(),
            c.authority_generation.to_string()
        ],
        |row| row.get(0),
    )?;
    if !exact
        || receipt.lineage_status != LineageStatus::Current
        || receipt.receipt.receipt_id != c.receipt_id
        || receipt.receipt.command_id != c.command_id
        || receipt.receipt.aggregate_id != c.terminal_id.as_uuid()
        || receipt.receipt.state != ReceiptState::Accepted
        || receipt.receipt.result_ref.as_deref()
            != Some(format!("terminal-release:{}", c.action_id).as_str())
    {
        return Err(invalid("historical release acceptance receipt differs"));
    }
    Ok(())
}

fn validate_registration(
    conn: &Connection,
    daemon: &DaemonStateRow,
    c: &TerminalReleaseContext,
) -> Result<(), StoreError> {
    let allocation = read_conn(conn, c.terminal_id)?
        .ok_or_else(|| invalid("historical release launch absent"))?;
    let initial = &allocation.context;
    if allocation.context_digest != c.initial_context_digest
        || initial.action_id != c.initial_action_id
        || initial.host_id != c.host_id
        || initial.process_id != c.process_id
        || initial.process_run_uuid != c.process_run_uuid
        || initial.actor_id != c.actor_id
        || initial.principal_id != c.principal_id
        || initial.authority_generation != c.authority_generation
        || initial.daemon_boot_identity != c.daemon_boot_identity
        || c.daemon_boot_identity != daemon.boot_identity
        || initial.preparation.mission_id != c.mission_id
        || !matches!(initial.preparation.scope, TerminalAllocationScope::ProjectConfiguration { project_id, .. } if project_id == c.project_id)
        || control_identity(conn, daemon, &allocation)? != c.expected_os_identity
    {
        return Err(StoreError::IdentityUnverified(
            "historical release registration differs".into(),
        ));
    }
    let lease = control_lease(conn, c.lease_id)?;
    if lease.authority_generation != c.authority_generation
        || lease.daemon_epoch != c.daemon_epoch
        || lease.boot_identity != c.daemon_boot_identity
        || lease.resource_kind != "terminal"
        || lease.resource_key != key(&initial.preparation)?
        || lease.owner_type != "principal"
        || lease.owner_id != c.principal_id.as_str()
        || lease.fence != c.resource_fence
    {
        return Err(invalid("historical release lease coordinates differ"));
    }
    Ok(())
}

fn validate_intent(
    record: &TerminalReleaseRecord,
    broker: &BrokerActionSnapshot,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    let c = &record.context;
    if intent.action_id != c.action_id
        || intent.payload_digest != record.context_digest
        || intent.intent_digest != broker.intent_digest
        || canonical_sha256(&intent.manifest)? != intent.intent_digest
        || intent.principal_id != c.principal_id
        || intent.project_id != Some(c.project_id)
        || intent.scope_revision != c.scope_revision
        || intent.policy_revision != c.policy_revision
        || intent.lineage_fence != broker.fence
        || intent.lineage_fence.authority_generation != c.authority_generation
        || intent.lineage_fence.epoch != c.daemon_epoch
        || intent.lineage_fence.fence != Fence::new(0)
        || intent.action_deadline != c.source_expires_at_ms
        || intent.origin
            != (ActionOrigin::ControlPlane {
                control_principal_id: c.principal_id.clone(),
            })
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_RELEASE_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs")
            != Some(&json!([{"kind":"terminal","value":c.terminal_id}]))
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid("historical release broker intent differs"));
    }
    Ok(())
}
