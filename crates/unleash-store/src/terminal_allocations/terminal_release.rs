//! Accepted retirement of control; physical resources and their holds remain.
use super::terminal_release_types::{
    TERMINAL_RELEASE_VERB, TerminalReleaseContext, TerminalReleaseRecord, TerminalReleaseRequest,
    TerminalReleaseState,
};
use super::{
    TerminalAllocationRecord, TerminalAllocationScope, control_holds, control_identity,
    control_lease, control_runtime, integer, invalid, key, policy, principal, read_conn, stale,
    validate_preparation,
};
use crate::commands::read_daemon_state_conn;
use crate::{
    AggregateKind, BrokerCause, BrokerIntent, CallerAuthority, CommitCommandRequest,
    DaemonStateRow, EventAppendSpec, StoreConfig, StoreError, UnleashStore,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use unleash_domain::{ActionId, ActorId, CommandId, DaemonMode, ReceiptId, Revision, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, EventCursor, Sensitivity, canonical_sha256,
};

pub(crate) fn validate_command(
    command: &CommitCommandRequest,
    caller: &CallerAuthority,
    request: &TerminalReleaseRequest,
) -> Result<(), StoreError> {
    if !request.is_valid()
        || command.target_aggregate != AggregateKind::TerminalControl
        || command.envelope.command_type != "ReleaseTerminal"
        || command.envelope.aggregate_id != request.terminal_id.as_uuid()
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
        || serde_json::to_value(request)?.as_object() != Some(&command.envelope.payload)
    {
        return Err(invalid(
            "terminal release command identity or closed payload invalid",
        ));
    }
    Ok(())
}

pub(crate) struct ReleaseCommand<'a> {
    pub actor: &'a ActorId,
    pub envelope: &'a CommandEnvelope,
    pub request: &'a TerminalReleaseRequest,
    pub receipt: ReceiptId,
}

pub(crate) fn guard_pending(
    conn: &Connection,
    d: &DaemonStateRow,
    terminal: unleash_domain::TerminalId,
) -> Result<(), StoreError> {
    if d.schema_version >= 18 && conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_release_intents WHERE terminal_id=?1 AND state IN ('prepared','retire_resolved','outcome_unknown'))",[terminal.to_string()],|row|row.get::<_,bool>(0))? {
        return Err(StoreError::CapacityBlocked("terminal release requires observation or qualified disposition".into()));
    }
    Ok(())
}

fn guard_other_pending(
    conn: &Connection,
    terminal: unleash_domain::TerminalId,
) -> Result<(), StoreError> {
    let pending:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_input_intents WHERE terminal_id=?1 AND state IN ('prepared','write_resolved','outcome_unknown')) OR EXISTS(SELECT 1 FROM terminal_control_intents WHERE terminal_id=?1 AND state IN ('prepared','handshake_resolved','outcome_unknown')) OR EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE terminal_id=?1 AND refusal_event_id IS NULL AND state IN ('prepared','handshake_resolved','outcome_unknown'))",[terminal.to_string()],|row|row.get(0))?;
    if pending {
        return Err(StoreError::CapacityBlocked(
            "terminal effect requires observation before retirement".into(),
        ));
    }
    Ok(())
}

pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    command: &ReleaseCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if d.schema_version < 18
        || d.mode != DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(stale(
            "terminal release requires ready V018 authority and a reliable clock",
        ));
    }
    guard_pending(tx, d, command.request.terminal_id)?;
    guard_other_pending(tx, command.request.terminal_id)?;
    let allocation = read_conn(tx, command.request.terminal_id)?
        .ok_or_else(|| invalid("terminal release allocation missing"))?;
    let base = &allocation.context;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = base.preparation.scope;
    let principal = principal(tx, command.actor)?;
    if base.actor_id != *command.actor
        || base.principal_id != principal
        || command.envelope.project_id != Some(project_id)
        || command.request.process_run_uuid != base.process_run_uuid
    {
        if command.envelope.project_id != Some(project_id)
            || base.actor_id != *command.actor
            || base.principal_id != principal
        {
            return Err(StoreError::NotFound(
                "terminal release target is not available in this scope".into(),
            ));
        }
        return Err(stale("terminal release physical generation differs"));
    }
    let (revision,expires):(i64,i64)=tx.query_row("SELECT s.control_revision,l.expires_at_ms FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id WHERE s.terminal_id=?1",[base.terminal_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
    let revision = Revision::new(
        u64::try_from(revision).map_err(|_| invalid("terminal control revision invalid"))?,
    );
    if revision != command.envelope.expected_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: command.envelope.expected_revision,
            command_id: Some(command.envelope.command_id),
        });
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    let (scope,mission):(i64,i64)=tx.query_row("SELECT p.revision,m.revision FROM projects p JOIN missions m ON m.project_id=p.project_id WHERE p.project_id=?1 AND m.mission_id=?2",params![project_id.to_string(),base.preparation.mission_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalReleaseContext {
        schema_version: 1,
        command_id: command.envelope.command_id,
        receipt_id: command.receipt,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        terminal_id: base.terminal_id,
        initial_action_id: base.action_id,
        initial_context_digest: allocation.context_digest,
        host_id: base.host_id,
        process_id: base.process_id,
        process_run_uuid: base.process_run_uuid,
        actor_id: base.actor_id.clone(),
        principal_id: principal,
        authority_generation: d.authority_generation,
        daemon_epoch: d.daemon_epoch,
        daemon_boot_identity: d.boot_identity.clone(),
        project_id,
        scope_revision: Revision::new(
            u64::try_from(scope).map_err(|_| invalid("terminal scope revision invalid"))?,
        ),
        mission_id: base.preparation.mission_id,
        mission_revision: Revision::new(
            u64::try_from(mission).map_err(|_| invalid("terminal mission revision invalid"))?,
        ),
        policy_revision,
        security_policy_digest,
        runtime_settings_digest: canonical_sha256(&runtime)?,
        lease_id: command.request.lease_id,
        resource_fence: command.request.fence,
        control_revision: revision,
        source_expires_at_ms: TimestampMs::new(expires),
        expected_os_identity: control_identity(tx, d, &allocation)?,
    };
    validate_current(tx, d, now, &context, "held")?;
    super::terminal_renewal::validate_policy_for_held_control(tx, project_id)?;
    let digest = canonical_sha256(&context)?;
    tx.execute("INSERT INTO terminal_release_intents(action_id,terminal_id,receipt_id,command_id,actor_id,principal_id,authority_generation,daemon_epoch,lease_id,resource_fence,control_revision,source_expires_at_ms,context_digest,context_json,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'prepared',1)",params![context.action_id.to_string(),context.terminal_id.to_string(),context.receipt_id.to_string(),context.command_id.to_string(),context.actor_id.as_str(),context.principal_id.as_str(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.lease_id.to_string(),integer(context.resource_fence.as_u64())?,integer(revision.as_u64())?,expires,digest.to_string(),serde_json::to_string(&context)?])?;
    events.push(EventAppendSpec {event_type:"TerminalReleasePrepared".into(),aggregate_type:AggregateKind::TerminalControlIntent,aggregate_id:context.action_id.as_uuid(),aggregate_revision:Revision::new(1),project_id:Some(project_id),correlation_id:context.terminal_id.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,payload:json!({"terminal_id":context.terminal_id,"process_run_uuid":context.process_run_uuid,"control_revision":revision,"state":"preparing"}).as_object().cloned().ok_or_else(||invalid("terminal release event invalid"))?,sensitivity:Sensitivity::Project,outbox_destinations:vec!["clients".into(),"harness".into()]});
    Ok(revision)
}

pub(super) fn validate_current(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalReleaseContext,
    lease_state: &str,
) -> Result<TerminalAllocationRecord, StoreError> {
    if d.mode != DaemonMode::Ready
        || d.authority_generation != c.authority_generation
        || d.daemon_epoch != c.daemon_epoch
        || d.boot_identity != c.daemon_boot_identity
        || now >= c.source_expires_at_ms
        || principal(tx, &c.actor_id)? != c.principal_id
    {
        return Err(stale("terminal release authority or lease expired"));
    }
    let allocation = read_conn(tx, c.terminal_id)?
        .ok_or_else(|| invalid("terminal release initial launch missing"))?;
    let base = &allocation.context;
    if allocation.context_digest != c.initial_context_digest
        || base.action_id != c.initial_action_id
        || base.host_id != c.host_id
        || base.process_id != c.process_id
        || base.process_run_uuid != c.process_run_uuid
        || base.actor_id != c.actor_id
        || base.principal_id != c.principal_id
        || base.authority_generation != c.authority_generation
        || base.daemon_boot_identity != c.daemon_boot_identity
        || control_identity(tx, d, &allocation)? != c.expected_os_identity
    {
        return Err(invalid("terminal release registration differs"));
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    if canonical_sha256(&runtime)? != c.runtime_settings_digest
        || policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest)
    {
        return Err(invalid("terminal release policy or runtime changed"));
    }
    let mut scoped = base.preparation.clone();
    scoped.scope = TerminalAllocationScope::ProjectConfiguration {
        project_id: c.project_id,
        revision: c.scope_revision,
    };
    scoped.mission_revision = c.mission_revision;
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, &c.actor_id, &c.principal_id)?;
    control_holds(tx, d, &allocation)?;
    let lease = control_lease(tx, c.lease_id)?;
    if lease.authority_generation != c.authority_generation
        || lease.daemon_epoch != c.daemon_epoch
        || lease.boot_identity != c.daemon_boot_identity
        || lease.resource_kind != "terminal"
        || lease.resource_key != key(&base.preparation)?
        || lease.owner_type != "principal"
        || lease.owner_id != c.principal_id.as_str()
        || lease.fence != c.resource_fence
        || lease.state != lease_state
    {
        return Err(stale("terminal release is not the exact controller"));
    }
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id JOIN resource_fences f ON f.authority_generation=l.authority_generation AND f.resource_kind=l.resource_kind AND f.resource_key=l.resource_key WHERE s.terminal_id=?1 AND s.process_id=?2 AND s.lease_id=?3 AND s.control_revision=?4 AND s.state IN ('ready','busy','detached') AND l.expires_at_ms=?5 AND f.last_fence=?6)",params![c.terminal_id.to_string(),c.process_id.to_string(),c.lease_id.to_string(),integer(c.control_revision.as_u64())?,c.source_expires_at_ms.as_i64(),integer(c.resource_fence.as_u64())?],|row|row.get(0))?;
    if !exact {
        return Err(stale("terminal release controller CAS differs"));
    }
    Ok(allocation)
}

pub(super) fn read_conn_release(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalReleaseRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 18 {
        return Ok(None);
    }
    let row:Option<(String,String,String,i64,Option<String>,Option<String>)>=conn.query_row("SELECT context_json,context_digest,state,revision,dispatch_id,observation_object_hash FROM terminal_release_intents WHERE action_id=?1",[action.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    row.map(
        |(encoded, digest, state, revision, dispatch, observation)| {
            let context: TerminalReleaseContext = serde_json::from_str(&encoded)?;
            let context_digest = digest.parse()?;
            if context.action_id != action || canonical_sha256(&context)? != context_digest {
                return Err(invalid("terminal release context integrity differs"));
            }
            let state = match state.as_str() {
                "prepared" => TerminalReleaseState::Prepared,
                "retire_resolved" => TerminalReleaseState::RetireResolved,
                "observed" => TerminalReleaseState::Observed,
                "outcome_unknown" => TerminalReleaseState::OutcomeUnknown,
                "not_dispatched" => TerminalReleaseState::NotDispatched,
                "disposed" => TerminalReleaseState::Disposed,
                _ => return Err(invalid("terminal release state invalid")),
            };
            let broker = crate::broker_writer::inspect_action_conn(conn, action)?;
            Ok(TerminalReleaseRecord {
                context,
                context_digest,
                state,
                revision: Revision::new(
                    u64::try_from(revision)
                        .map_err(|_| invalid("terminal release revision invalid"))?,
                ),
                dispatch_id: dispatch.map(|v| v.parse()).transpose()?,
                observation_object_hash: observation.map(|v| v.parse()).transpose()?,
                broker_action_state: broker.as_ref().map(|v| v.state),
                broker_dispatch_id: broker.and_then(|v| v.dispatch_id),
            })
        },
    )
    .transpose()
}

pub(super) fn validate_live(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    record: &TerminalReleaseRecord,
) -> Result<(), StoreError> {
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(stale("terminal release clock uncertain"));
    }
    let state = match record.state {
        TerminalReleaseState::Prepared => "held",
        TerminalReleaseState::RetireResolved => "reconciling",
        _ => return Err(stale("terminal release is not pending")),
    };
    validate_current(tx, d, now, &record.context, state)?;
    super::validate_incarnation(tx, d, record.context.action_id)?;
    let c = &record.context;
    let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND command_id=?2 AND actor_id=?3 AND aggregate_id=?4 AND authority_generation=?5 AND command_type='ReleaseTerminal' AND state='accepted' AND response_state='full')",params![c.receipt_id.to_string(),c.command_id.to_string(),c.actor_id.as_str(),c.terminal_id.to_string(),c.authority_generation.to_string()],|row|row.get(0))?;
    if !receipt {
        return Err(invalid("terminal release acceptance receipt differs"));
    }
    guard_other_pending(tx, record.context.terminal_id)
}

pub(super) fn validate_intent(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
) -> Result<bool, StoreError> {
    let Some(record) = read_conn_release(tx, intent.action_id)? else {
        return Ok(false);
    };
    let c = &record.context;
    let state = match record.state {
        TerminalReleaseState::Prepared => "held",
        TerminalReleaseState::RetireResolved => "reconciling",
        _ => return Err(stale("terminal release permit not pending")),
    };
    validate_current(tx, d, now, c, state)?;
    guard_other_pending(tx, c.terminal_id)?;
    if intent.principal_id != c.principal_id
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.project_id != Some(c.project_id)
        || intent.scope_revision != c.scope_revision
        || intent.policy_revision != c.policy_revision
        || intent.payload_digest != record.context_digest
        || intent.action_deadline != c.source_expires_at_ms
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_RELEASE_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs")
            != Some(&json!([{"kind":"terminal","value":c.terminal_id}]))
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "terminal release broker intent differs from accepted command",
        ));
    }
    Ok(true)
}

impl UnleashStore {
    /// Inspect accepted control retirement without any native effect.
    /// # Errors
    /// Returns storage/integrity failures; another actor's command remains hidden.
    pub fn inspect_terminal_release_request(
        &self,
        actor: &ActorId,
        command: CommandId,
    ) -> Result<crate::SnapshotProjection<Option<TerminalReleaseRecord>>, StoreError> {
        self.read_snapshot(|conn,_| {
            let d=read_daemon_state_conn(conn)?;
            let value=if d.schema_version<18 {None} else {let action:Option<String>=conn.query_row("SELECT action_id FROM terminal_release_intents WHERE actor_id=?1 AND command_id=?2 AND authority_generation=?3",params![actor.as_str(),command.to_string(),d.authority_generation.to_string()],|row|row.get(0)).optional()?;action.map(|id|read_conn_release(conn,id.parse()?)).transpose()?.flatten()};
            let high:i64=conn.query_row("SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",[d.stream_id.to_string()],|row|row.get(0))?;
            Ok(crate::SnapshotProjection {value,cursor:EventCursor::new(d.stream_id,unleash_domain::Sequence::new(u64::try_from(high).map_err(|_|invalid("terminal release watermark invalid"))?))})
        })
    }
}

pub(super) fn qualified_source(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    allocation: &TerminalAllocationRecord,
    lease: unleash_domain::LeaseId,
) -> Result<Option<super::terminal_release_types::TerminalReleasedControlSource>, StoreError> {
    if d.schema_version < 18 {
        return Ok(None);
    }
    let action:Option<String>=tx.query_row("SELECT action_id FROM terminal_release_intents WHERE terminal_id=?1 AND lease_id=?2 AND state='observed'",params![allocation.context.terminal_id.to_string(),lease.to_string()],|row|row.get(0)).optional()?;
    let Some(action) = action else {
        return Ok(None);
    };
    let record = read_conn_release(tx, action.parse()?)?
        .ok_or_else(|| invalid("terminal release source missing"))?;
    let c = &record.context;
    if c.actor_id != allocation.context.actor_id
        || c.principal_id != allocation.context.principal_id
        || principal(tx, &c.actor_id)? != c.principal_id
        || c.authority_generation != d.authority_generation
        || c.daemon_boot_identity != d.boot_identity
        || c.daemon_epoch > d.daemon_epoch
        || c.initial_action_id != allocation.context.action_id
        || c.initial_context_digest != allocation.context_digest
        || c.process_run_uuid != allocation.context.process_run_uuid
        || record.broker_action_state != Some(unleash_domain::ActionState::ObservedSuccess)
        || control_lease(tx, lease)?.state != "released"
    {
        return Err(stale("terminal reactivation retirement source differs"));
    }
    let hash = record
        .observation_object_hash
        .ok_or_else(|| invalid("terminal retirement source has no proof"))?;
    super::terminal_release_observation::read_published(tx, config, &record, hash)?;
    Ok(Some(
        super::terminal_release_types::TerminalReleasedControlSource {
            action_id: c.action_id,
            dispatch_id: record
                .dispatch_id
                .ok_or_else(|| invalid("terminal retirement source has no dispatch"))?,
            context_digest: record.context_digest,
            observation_object_hash: hash,
        },
    ))
}

pub(super) fn validate_reactivation_source(
    conn: &Connection,
    c: &super::TerminalControlContext,
) -> Result<(), StoreError> {
    let Some(source) = &c.release_source else {
        return Ok(());
    };
    let record = read_conn_release(conn, source.action_id)?
        .ok_or_else(|| invalid("reactivation release source missing"))?;
    let r = &record.context;
    if record.state != TerminalReleaseState::Observed
        || record.broker_action_state != Some(unleash_domain::ActionState::ObservedSuccess)
        || record.context_digest != source.context_digest
        || record.dispatch_id != Some(source.dispatch_id)
        || record.observation_object_hash != Some(source.observation_object_hash)
        || r.terminal_id != c.terminal_id
        || r.initial_action_id != c.initial_action_id
        || r.initial_context_digest != c.initial_context_digest
        || r.host_id != c.host_id
        || r.process_id != c.process_id
        || r.process_run_uuid != c.process_run_uuid
        || r.actor_id != c.actor_id
        || r.principal_id != c.principal_id
        || r.authority_generation != c.authority_generation
        || r.daemon_boot_identity != c.daemon_boot_identity
        || r.daemon_epoch != c.previous_epoch
        || r.lease_id != c.previous_lease_id
        || r.resource_fence != c.previous_fence
        || r.control_revision.as_u64().checked_add(1) != Some(c.previous_control_revision.as_u64())
        || r.expected_os_identity != c.expected_os_identity
        || control_lease(conn, c.previous_lease_id)?.state != "released"
    {
        return Err(stale(
            "reactivation is not linked to the exact retired controller",
        ));
    }
    Ok(())
}

pub(super) fn validate_reactivation_ack(
    conn: &Connection,
    config: &StoreConfig,
    record: &super::TerminalControlRecord,
    observation: &super::TerminalControlObservation,
) -> Result<(), StoreError> {
    let c = &record.context;
    let Some(source) = &c.release_source else {
        return Ok(());
    };
    validate_reactivation_source(conn, c)?;
    let source_record = read_conn_release(conn, source.action_id)?
        .ok_or_else(|| invalid("retirement proof source missing"))?;
    let retired = super::terminal_release_observation::read_published(
        conn,
        config,
        &source_record,
        source.observation_object_hash,
    )?;
    let session_seq: i64 = conn.query_row(
        "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1 AND process_id=?2",
        params![c.terminal_id.to_string(), c.process_id.to_string()],
        |row| row.get(0),
    )?;
    if u64::try_from(session_seq).ok() != Some(retired.ack.last_confirmed_input_seq)
        || observation.last_confirmed_input_seq != retired.ack.last_confirmed_input_seq
        || observation.durable_log_seq < retired.ack.durable_log_seq
    {
        return Err(invalid(
            "reactivation input sequence or log watermark differs from retirement",
        ));
    }
    let allocation = read_conn(conn, c.terminal_id)?
        .ok_or_else(|| invalid("reactivation initial launch missing"))?;
    let environment = super::terminal_control_history::observed_launch_environment(
        conn,
        config,
        record,
        &allocation,
    )?;
    if observation.effective_environment_digest != environment {
        return Err(invalid(
            "reactivation native environment differs from observed launch",
        ));
    }
    let hash = observation.transfer_ack_digest;
    let published:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='terminal_reacquisition_ack' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live')",params![c.action_id.to_string(),observation.dispatch_id.to_string(),hash.to_string()],|row|row.get(0))?;
    if !published {
        return Err(StoreError::EvidenceUnavailable(
            "native reactivation ACK is not published".into(),
        ));
    }
    let bytes =
        crate::objects::read_verified_evidence_bytes_bounded(conn, config, hash, 2 * 1024 * 1024)?;
    let ack: super::terminal_release_types::TerminalReacquisitionNativeAck =
        serde_json::from_slice(&bytes)?;
    let a = &ack.transfer;
    let h = &a.header;
    if canonical_sha256(&ack)? != hash
        || ack.released_by != source.dispatch_id
        || h.protocol_version != 1
        || h.action_id != c.action_id
        || h.command_id != c.command_id
        || h.payload_hash != record.context_digest
        || h.process_run_uuid != c.process_run_uuid
        || h.authority_generation != c.authority_generation
        || h.daemon_epoch != c.daemon_epoch
        || h.resource_fence != c.next_fence
        || a.dispatch_id != observation.dispatch_id
        || a.initial_context_digest != c.initial_context_digest
        || a.os_identity != c.expected_os_identity
        || a.effective_environment_digest != observation.effective_environment_digest
        || a.durable_log_seq != observation.durable_log_seq
        || a.last_confirmed_input_seq != observation.last_confirmed_input_seq
        || a.unknown_input_seqs != observation.unknown_input_seqs
        || !a.unknown_input_seqs.is_empty()
    {
        return Err(invalid(
            "native reactivation ACK differs from accepted intent and observation",
        ));
    }
    Ok(())
}

pub(super) fn guard_causal_assertions(
    conn: &Connection,
    allocation: &TerminalAllocationRecord,
) -> Result<(), StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 18 {
        return Ok(());
    }
    let c = &allocation.context;
    let unknown:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_release_intents r JOIN broker_dispatches b ON b.action_id=r.action_id JOIN actions a ON a.action_id=r.action_id WHERE r.terminal_id=?1 AND r.authority_generation=?2 AND json_extract(r.context_json,'$.initial_action_id')=?3 AND json_extract(r.context_json,'$.initial_context_digest')=?4 AND json_extract(r.context_json,'$.process_run_uuid')=?5 AND r.state IN ('prepared','retire_resolved','outcome_unknown') AND a.state IN ('dispatching','in_flight','cancel_requested','outcome_unknown'))",params![c.terminal_id.to_string(),c.authority_generation.to_string(),c.action_id.to_string(),allocation.context_digest.to_string(),c.process_run_uuid.to_string()],|row|row.get(0))?;
    if unknown {
        return Err(stale(
            "terminal release requires qualified observation or disposition",
        ));
    }
    Ok(())
}
