//! Input admission owns no PTY bytes or host I/O. The accepted receipt and
//! digest-only intention are committed together, before broker authorization.

use super::terminal_input_types::{
    MAX_TERMINAL_INPUT_BYTES, MAX_TERMINAL_INPUT_WIRE_INTEGER, TERMINAL_INPUT_VERB,
    TerminalInputContext, TerminalInputRecord, TerminalInputRequest, TerminalInputState,
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
    verify_native_input_state(tx, config, &command, &allocation, revision)?;
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

fn verify_native_input_state(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    command: &InputCommand<'_>,
    allocation: &super::TerminalAllocationRecord,
    revision: Revision,
) -> Result<(), StoreError> {
    let c = &allocation.context;
    let input = command.request;
    let (source_command, reference): (String, String) = if input.lease_id == c.lease_id
        && revision == Revision::INITIAL
    {
        let source: String = tx.query_row(
            "SELECT command_id FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2",
            params![c.receipt_id.to_string(), command.actor_id.as_str()],
            |row| row.get(0),
        )?;
        (source, format!("terminal-allocation:{}", c.terminal_id))
    } else {
        let (source, action): (String,String) = tx.query_row(
            "SELECT r.command_id,t.action_id FROM terminal_control_intents t JOIN command_receipts r ON r.receipt_id=t.receipt_id WHERE t.terminal_id=?1 AND t.next_lease_id=?2 AND t.state='observed' AND t.authority_generation=?3 AND t.actor_id=?4 AND json_extract(t.context_json,'$.previous_control_revision')+1=?5",
            params![c.terminal_id.to_string(),input.lease_id.to_string(),c.authority_generation.to_string(),command.actor_id.as_str(),integer(revision.as_u64())?],
            |row|Ok((row.get(0)?,row.get(1)?)),
        ).optional()?.ok_or_else(|| StoreError::EvidenceUnavailable("current terminal control has no qualified native observation".into()))?;
        (source, format!("terminal-control:{action}"))
    };
    let super::TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
        c.preparation.scope;
    let observed = super::terminal_acquisition_query::read_result(
        tx,
        config,
        command.actor_id,
        source_command.parse()?,
        &reference,
        project_id,
    )?;
    if observed.terminal_id != input.terminal_id
        || observed.process_run_uuid != input.process_run_uuid
        || observed.lease_id != input.lease_id
        || observed.fence != input.fence
        || observed.control_revision != revision
        || observed.input_seq.checked_add(1) != Some(input.input_seq)
    {
        return Err(StoreError::EvidenceUnavailable(
            "native terminal input state differs from the requested sequence or controller".into(),
        ));
    }
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
        || config.clock.now_ms() >= observed.lease_expires_at
    {
        return Err(stale(
            "terminal lease became unavailable during input attestation",
        ));
    }
    Ok(())
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
    if read_daemon_state_conn(conn)?.schema_version < 14 {
        return Ok(None);
    }
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

pub(crate) fn validate_incarnation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    action: ActionId,
) -> Result<(), StoreError> {
    if let Some(record) = read_input_conn(tx, action)? {
        let c = record.context;
        if c.authority_generation != daemon.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: daemon.authority_generation,
                actual: c.authority_generation,
                command_id: None,
            });
        }
        if c.daemon_epoch != daemon.daemon_epoch || c.daemon_boot_identity != daemon.boot_identity {
            return Err(stale("terminal input belongs to an old incarnation"));
        }
    }
    Ok(())
}

pub(crate) fn validate_intent(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    intent: &crate::BrokerIntent,
) -> Result<(), StoreError> {
    let record = read_input_conn(tx, intent.action_id)?;
    let cause_type: Option<String> =
        if let crate::BrokerCause::CommandReceipt { receipt_id } = intent.cause {
            tx.query_row(
                "SELECT command_type FROM command_receipts WHERE receipt_id=?1",
                [receipt_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
        } else {
            None
        };
    if record.is_none()
        && cause_type.as_deref() != Some("SendTerminalInput")
        && intent.manifest.get("verb") != Some(&json!(TERMINAL_INPUT_VERB))
    {
        return Ok(());
    }
    let record = record.ok_or_else(|| invalid("terminal input preparation missing"))?;
    let c = &record.context;
    if record.state != TerminalInputState::Prepared
        || intent.principal_id != c.principal_id
        || intent.cause
            != (crate::BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.project_id != Some(c.project_id)
        || intent.scope_revision != c.scope_revision
        || intent.policy_revision != c.policy_revision
        || intent.payload_digest != record.context_digest
        || intent.action_deadline != c.previous_lease_expires_at
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_INPUT_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs")
            != Some(&json!([{"kind":"terminal","value":c.terminal_id}]))
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "terminal input intent differs from accepted request",
        ));
    }
    validate_live_context(tx, daemon, now, &record)
}

pub(crate) fn validate_live_context(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &TerminalInputRecord,
) -> Result<(), StoreError> {
    let c = &record.context;
    validate_incarnation(tx, daemon, c.action_id)?;
    if daemon.mode != DaemonMode::Ready
        || now >= c.previous_lease_expires_at
        || principal(tx, &c.actor_id)? != c.principal_id
    {
        return Err(stale("terminal input controller expired or unavailable"));
    }
    let allocation =
        read_conn(tx, c.terminal_id)?.ok_or_else(|| invalid("terminal input launch missing"))?;
    let original = &allocation.context;
    if original.action_id != c.initial_action_id
        || allocation.context_digest != c.initial_context_digest
        || original.process_id != c.process_id
        || original.host_id != c.host_id
        || original.process_run_uuid != c.process_run_uuid
        || original.actor_id != c.actor_id
        || original.principal_id != c.principal_id
        || original.authority_generation != c.authority_generation
    {
        return Err(invalid("terminal input initial registration differs"));
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    if canonical_sha256(&runtime)? != c.runtime_settings_digest
        || policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest)
    {
        return Err(invalid("terminal input settings or policy changed"));
    }
    let mut scoped = original.preparation.clone();
    scoped.scope = super::TerminalAllocationScope::ProjectConfiguration {
        project_id: c.project_id,
        revision: c.scope_revision,
    };
    scoped.mission_revision = c.mission_revision;
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, &c.actor_id, &c.principal_id)?;
    control_holds(tx, daemon, &allocation)?;
    control_identity(tx, daemon, &allocation)?;
    let exact: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN processes p ON p.process_id=s.process_id JOIN resource_leases l ON l.lease_id=s.lease_id WHERE s.terminal_id=?1 AND s.process_id=?2 AND s.lease_id=?3 AND s.control_revision=?4 AND s.input_seq=?5 AND s.state IN ('ready','busy','detached') AND p.state='live' AND l.boot_identity=?6 AND l.resource_kind='terminal' AND l.resource_key=?7 AND l.owner_type='principal' AND l.owner_id=?8 AND l.expires_at_ms>=?9 AND l.authority_generation=?10)",
        params![c.terminal_id.to_string(),c.process_id.to_string(),c.lease_id.to_string(),integer(c.control_revision.as_u64())?,integer(c.input_seq.checked_sub(1).ok_or_else(||invalid("terminal input sequence invalid"))?)?,c.daemon_boot_identity,key(&original.preparation)?,c.principal_id.as_str(),c.previous_lease_expires_at.as_i64(),c.authority_generation.to_string()],
        |row| row.get(0),
    )?;
    let receipt: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3 AND command_type='SendTerminalInput' AND aggregate_id=?4 AND authority_generation=?5 AND state='accepted' AND response_state='full')",
        params![c.receipt_id.to_string(),c.actor_id.as_str(),c.command_id.to_string(),c.terminal_id.to_string(),c.authority_generation.to_string()], |row| row.get(0),
    )?;
    if !exact || !receipt {
        return Err(stale("terminal input session or accepted receipt differs"));
    }
    tx_validate_lease_fence(
        tx,
        daemon.authority_generation,
        daemon.daemon_epoch,
        &FenceAssertion {
            lease_id: c.lease_id,
            authority_generation: c.authority_generation,
            daemon_epoch: c.daemon_epoch,
            fence: c.resource_fence,
            owner_id: c.principal_id.to_string(),
        },
    )
}

pub(crate) fn guard_observation(
    tx: &Transaction<'_>,
    phase: &crate::BrokerPhase,
    proof: Option<&super::TerminalObservationProof>,
) -> Result<(), StoreError> {
    let crate::BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase
    else {
        return Ok(());
    };
    let Some(record) = read_input_conn(tx, *action_id)? else {
        return Ok(());
    };
    if receipt.action_id != record.context.action_id
        || record.broker_dispatch_id != Some(receipt.dispatch_id)
        || record
            .dispatch_id
            .is_some_and(|id| id != receipt.dispatch_id)
    {
        return Err(invalid("terminal input observation dispatch differs"));
    }
    if receipt.state == unleash_domain::ActionState::OutcomeUnknown {
        match record.state {
            TerminalInputState::WriteResolved => {
                let changed = tx.execute(
                    "UPDATE terminal_input_intents SET state='outcome_unknown',revision=revision+1 WHERE action_id=?1 AND state='write_resolved' AND revision=?2 AND dispatch_id=?3",
                    params![action_id.to_string(),integer(record.revision.as_u64())?,receipt.dispatch_id.to_string()],
                )?;
                if changed != 1 {
                    return Err(invalid("terminal input unknown projection CAS lost"));
                }
            }
            // C1 can roll back resolution after the dispatch is already durable.
            // Preserve the absent permit; the broker marker still quarantines it.
            TerminalInputState::Prepared | TerminalInputState::OutcomeUnknown => {}
            _ => {
                return Err(invalid(
                    "terminal input unknown observation has no pending intention",
                ));
            }
        }
    } else if !proof
        .is_some_and(|proof| canonical_sha256(phase).is_ok_and(|hash| hash == proof.phase_digest))
    {
        return Err(invalid(
            "terminal input native observation producer not qualified",
        ));
    }
    Ok(())
}

pub(crate) fn guard_causal_assertions(
    conn: &Connection,
    allocation: &super::TerminalAllocationRecord,
) -> Result<(), StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 14 {
        return Ok(());
    }
    let c = &allocation.context;
    let pending: Option<String> = conn.query_row(
        "SELECT action_id FROM terminal_input_intents WHERE terminal_id=?1 AND authority_generation=?2 AND json_extract(context_json,'$.process_run_uuid')=?3 AND json_extract(context_json,'$.initial_context_digest')=?4 AND state IN ('prepared','write_resolved','outcome_unknown')",
        params![c.terminal_id.to_string(),c.authority_generation.to_string(),c.process_run_uuid.to_string(),allocation.context_digest.to_string()],
        |row| row.get(0),
    ).optional()?;
    if let Some(action) = pending {
        let record = read_input_conn(conn, action.parse()?)?
            .ok_or_else(|| invalid("terminal pending input disappeared in snapshot"))?;
        if record.state != TerminalInputState::Prepared
            || record.dispatch_id.is_some()
            || record.broker_dispatch_id.is_some()
        {
            return Err(StoreError::ActionOutcomeUnknown {
                action_id: record.context.action_id,
                detail:
                    "terminal input requires conclusive native observation or qualified disposition"
                        .into(),
            });
        }
    }
    Ok(())
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
