//! Input admission owns no PTY bytes or host I/O. The accepted receipt and
//! digest-only intention are committed together, before broker authorization.

use super::terminal_input_types::{
    MAX_TERMINAL_INPUT_BYTES, MAX_TERMINAL_INPUT_WIRE_INTEGER, TerminalInputContext,
    TerminalInputRecord, TerminalInputRequest, TerminalInputState,
};
use super::{
    control_holds, control_identity, control_lease, control_runtime, integer, invalid, key, policy,
    principal, read_conn, stale, validate_preparation,
};
use crate::commands::read_daemon_state_conn;
use crate::leases::tx_validate_lease_fence;
use crate::{
    AggregateKind, CallerAuthority, CommitCommandRequest, DaemonStateRow, EventAppendSpec,
    FenceAssertion, StoreConfig, StoreError, UnleashStore,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use unleash_domain::{
    ActionId, ActorId, CommandId, DaemonMode, ReceiptId, Revision, Sequence, Sha256Digest,
    TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, EventCursor, Sensitivity, canonical_sha256,
};

pub(crate) fn validate_command(
    command: &CommitCommandRequest,
    caller: &CallerAuthority,
    input: &TerminalInputRequest,
) -> Result<(), StoreError> {
    input.validate()?;
    if command.target_aggregate != AggregateKind::TerminalControl
        || command.envelope.command_type != "SendTerminalInput"
        || command.envelope.aggregate_id != input.terminal_id.as_uuid()
        || command.envelope.project_id.is_none()
        || caller.actor_id != command.actor_id
        || !caller.role.is_human_decision_maker()
        || command
            .envelope
            .actor_ref
            .as_ref()
            .is_some_and(|actor| actor != command.actor_id.as_str())
        || command.operation_id.is_some()
        || command.result_ref.is_some()
        || command.fence_assertion.is_some()
        || !command.events.is_empty()
        || !command.capacity_reservations.is_empty()
        || !command.budget_reservations.is_empty()
        || serde_json::to_value(input)?.as_object() != Some(&command.envelope.payload)
    {
        return Err(invalid(
            "terminal input command identity or closed payload invalid",
        ));
    }
    Ok(())
}

impl TerminalInputRequest {
    /// Check the wire domain only; this never validates controller authority.
    /// # Errors
    /// Refuses empty/oversized input, nil identities and non-exact wire integers.
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.terminal_id.as_uuid().is_nil()
            || self.process_run_uuid.as_uuid().is_nil()
            || self.lease_id.as_uuid().is_nil()
            || !(1..=MAX_TERMINAL_INPUT_WIRE_INTEGER).contains(&self.fence.as_u64())
            || !(1..=MAX_TERMINAL_INPUT_WIRE_INTEGER).contains(&self.input_seq)
            || !(1..=MAX_TERMINAL_INPUT_BYTES).contains(&self.payload_bytes.len())
        {
            return Err(invalid("terminal input shape or bounds invalid"));
        }
        Ok(())
    }
}

pub(crate) struct InputCommand<'a> {
    pub actor_id: &'a ActorId,
    pub envelope: &'a CommandEnvelope,
    pub request: &'a TerminalInputRequest,
    pub receipt_id: ReceiptId,
}

pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: InputCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.schema_version < 14
        || daemon.mode != DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "terminal input requires ready V014 authority and a reliable clock",
        ));
    }
    let input = command.request;
    let allocation = read_conn(tx, input.terminal_id)?
        .ok_or_else(|| invalid("terminal input allocation missing"))?;
    let original = &allocation.context;
    let principal = principal(tx, command.actor_id)?;
    let super::TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
        original.preparation.scope;
    if original.authority_generation != daemon.authority_generation
        || original.daemon_boot_identity != daemon.boot_identity
        || command.actor_id != &original.actor_id
        || principal != original.principal_id
        || command.envelope.project_id != Some(project_id)
        || input.process_run_uuid != original.process_run_uuid
    {
        return Err(invalid("terminal input lineage, actor or scope differs"));
    }
    let (lease_id, revision, sequence): (String, i64, i64) = tx.query_row(
        "SELECT lease_id,control_revision,input_seq FROM terminal_sessions WHERE terminal_id=?1 AND process_id=?2 AND state IN ('ready','busy','detached')",
        params![input.terminal_id.to_string(), original.process_id.to_string()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?.ok_or_else(|| stale("terminal input session is not currently controllable"))?;
    let revision = Revision::new(
        u64::try_from(revision).map_err(|_| invalid("terminal control revision invalid"))?,
    );
    if command.envelope.expected_revision != revision {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: command.envelope.expected_revision,
            command_id: Some(command.envelope.command_id),
        });
    }
    let next = u64::try_from(sequence)
        .ok()
        .and_then(|value| value.checked_add(1))
        .filter(|value| *value <= MAX_TERMINAL_INPUT_WIRE_INTEGER)
        .ok_or_else(|| invalid("terminal input sequence exhausted or invalid"))?;
    if input.input_seq != next {
        return Err(invalid(
            "terminal input sequence is not the next confirmed sequence",
        ));
    }
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_input_intents WHERE terminal_id=?1 AND state IN ('prepared','write_resolved','outcome_unknown')) OR EXISTS(SELECT 1 FROM terminal_control_intents WHERE terminal_id=?1 AND state IN ('prepared','handshake_resolved','outcome_unknown'))",
        [input.terminal_id.to_string()], |row| row.get(0),
    )?;
    if pending {
        return Err(StoreError::CapacityBlocked(
            "terminal input or control requires observation or disposition".into(),
        ));
    }
    if lease_id != input.lease_id.to_string() {
        return Err(stale("terminal input lease is not the current controller"));
    }
    let lease = control_lease(tx, input.lease_id)?;
    tx_validate_lease_fence(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        &FenceAssertion {
            lease_id: input.lease_id,
            authority_generation: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            fence: input.fence,
            owner_id: principal.to_string(),
        },
    )?;
    let expires: i64 = tx.query_row(
        "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
        [input.lease_id.to_string()],
        |row| row.get(0),
    )?;
    if lease.authority_generation != daemon.authority_generation
        || lease.boot_identity != daemon.boot_identity
        || lease.resource_kind != "terminal"
        || lease.resource_key != key(&original.preparation)?
        || lease.owner_type != "principal"
        || expires <= now.as_i64()
    {
        return Err(stale(
            "terminal input controller lineage or deadline differs",
        ));
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    let (scope_revision, mission_revision): (i64, i64) = tx.query_row(
        "SELECT p.revision,m.revision FROM projects p JOIN missions m ON m.project_id=p.project_id WHERE p.project_id=?1 AND m.mission_id=?2",
        params![project_id.to_string(), original.preparation.mission_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?)),
    )?;
    let scope_revision = Revision::new(
        u64::try_from(scope_revision)
            .map_err(|_| invalid("terminal input project revision invalid"))?,
    );
    let mission_revision = Revision::new(
        u64::try_from(mission_revision)
            .map_err(|_| invalid("terminal input mission revision invalid"))?,
    );
    let mut scoped = original.preparation.clone();
    scoped.scope = super::TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision: scope_revision,
    };
    scoped.mission_revision = mission_revision;
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, command.actor_id, &principal)?;
    control_identity(tx, daemon, &allocation)?;
    control_holds(tx, daemon, &allocation)?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalInputContext {
        schema_version: 1,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        command_id: command.envelope.command_id,
        receipt_id: command.receipt_id,
        actor_id: command.actor_id.clone(),
        principal_id: principal,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        daemon_boot_identity: daemon.boot_identity.clone(),
        project_id,
        scope_revision,
        mission_id: original.preparation.mission_id,
        mission_revision,
        policy_revision,
        security_policy_digest,
        runtime_settings_digest: canonical_sha256(&runtime)?,
        terminal_id: input.terminal_id,
        process_id: original.process_id,
        host_id: original.host_id,
        process_run_uuid: original.process_run_uuid,
        initial_action_id: original.action_id,
        initial_context_digest: allocation.context_digest,
        control_revision: revision,
        lease_id: input.lease_id,
        resource_fence: input.fence,
        previous_lease_expires_at: TimestampMs::new(expires),
        proposed_lease_expires_at: TimestampMs::new(
            now.as_i64()
                .checked_add(runtime.lease_ttl_ms)
                .ok_or_else(|| invalid("terminal input renewal deadline overflow"))?,
        ),
        input_seq: input.input_seq,
        payload_digest: Sha256Digest::from_raw_bytes(Sha256::digest(&input.payload_bytes).into()),
        payload_byte_length: u64::try_from(input.payload_bytes.len())
            .map_err(|_| invalid("terminal input length invalid"))?,
    };
    insert_intent(tx, &context)?;
    events.push(EventAppendSpec {
        event_type: "TerminalInputPrepared".into(),
        aggregate_type: AggregateKind::TerminalInputIntent,
        aggregate_id: context.action_id.as_uuid(),
        aggregate_revision: Revision::new(1),
        project_id: Some(project_id),
        correlation_id: input.terminal_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: json!({"terminal_id":input.terminal_id,"action_id":context.action_id,"input_seq":input.input_seq,"payload_digest":context.payload_digest,"payload_byte_length":context.payload_byte_length,"state":"prepared"}).as_object().cloned().ok_or_else(|| invalid("terminal input event invalid"))?,
        sensitivity: Sensitivity::Project,
        outbox_destinations: vec!["clients".into(),"harness".into()],
    });
    Ok(revision)
}

fn insert_intent(tx: &Transaction<'_>, context: &TerminalInputContext) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO terminal_input_intents(action_id,terminal_id,receipt_id,command_id,actor_id,principal_id,authority_generation,daemon_epoch,lease_id,resource_fence,input_seq,payload_digest,payload_byte_length,context_digest,context_json,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'prepared',1)",
        params![context.action_id.to_string(),context.terminal_id.to_string(),context.receipt_id.to_string(),context.command_id.to_string(),context.actor_id.as_str(),context.principal_id.as_str(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.lease_id.to_string(),integer(context.resource_fence.as_u64())?,integer(context.input_seq)?,context.payload_digest.to_string(),integer(context.payload_byte_length)?,canonical_sha256(context)?.to_string(),serde_json::to_string(context)?],
    )?;
    Ok(())
}

#[derive(Deserialize)]
struct InputRow {
    context_json: String,
    context_digest: Sha256Digest,
    state: TerminalInputState,
    revision: Revision,
    dispatch_id: Option<unleash_domain::DispatchId>,
    observation_object_hash: Option<Sha256Digest>,
    renewal: Option<super::terminal_input_types::TerminalInputRenewal>,
    renewed_lease_expires_at: Option<TimestampMs>,
}

pub(crate) fn read_input_conn(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalInputRecord>, StoreError> {
    let row: Option<String> = conn.query_row(
        "SELECT json_object('context_json',context_json,'context_digest',context_digest,'state',state,'revision',revision,'dispatch_id',dispatch_id,'observation_object_hash',observation_object_hash,'renewal',renewal,'renewed_lease_expires_at',renewed_lease_expires_at) FROM terminal_input_intents WHERE action_id=?1",
        [action.to_string()], |row| row.get(0),
    ).optional()?;
    row.map(|encoded| {
        let row: InputRow = serde_json::from_str(&encoded)?;
        let context: TerminalInputContext = serde_json::from_str(&row.context_json)?;
        if context.action_id != action || canonical_sha256(&context)? != row.context_digest {
            return Err(invalid("terminal input context digest differs"));
        }
        let broker = crate::broker_writer::inspect_action_conn(conn, action)?;
        Ok(TerminalInputRecord {
            context,
            context_digest: row.context_digest,
            state: row.state,
            revision: row.revision,
            dispatch_id: row.dispatch_id,
            observation_object_hash: row.observation_object_hash,
            renewal: row.renewal,
            renewed_lease_expires_at: row.renewed_lease_expires_at,
            broker_action_state: broker.as_ref().map(|snapshot| snapshot.state),
            broker_dispatch_id: broker.and_then(|snapshot| snapshot.dispatch_id),
        })
    })
    .transpose()
}

impl UnleashStore {
    /// Inspect this actor's input intention with its broker marker in one snapshot.
    /// This returns historical facts and cannot grant a native write permit.
    /// # Errors
    /// Refuses corrupt durable data; missing or foreign commands stay absent.
    pub fn inspect_terminal_input_request(
        &self,
        actor: &ActorId,
        command: CommandId,
    ) -> Result<crate::SnapshotProjection<Option<TerminalInputRecord>>, StoreError> {
        self.read_snapshot(|conn,_| {
            let daemon = read_daemon_state_conn(conn)?;
            let high: i64 = conn.query_row("SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1", [daemon.stream_id.to_string()], |row| row.get(0))?;
            let value = if daemon.schema_version < 14 { None } else {
                let action: Option<String> = conn.query_row("SELECT action_id FROM terminal_input_intents WHERE actor_id=?1 AND command_id=?2 AND authority_generation=?3", params![actor.as_str(),command.to_string(),daemon.authority_generation.to_string()], |row| row.get(0)).optional()?;
                action.map(|action| read_input_conn(conn,action.parse()?)).transpose()?.flatten()
            };
            Ok(crate::SnapshotProjection { value, cursor:EventCursor::new(daemon.stream_id,Sequence::new(u64::try_from(high).map_err(|_| invalid("terminal input watermark invalid"))?)) })
        })
    }
}
