//! Same-daemon renewal intentions. Existing transfer guards stay unchanged.
use super::terminal_renewal_types::{
    TERMINAL_RENEWAL_VERB, TerminalRenewalContext, TerminalRenewalRecord, TerminalRenewalState,
};
use super::{
    RegisteredTerminalControl, TerminalAllocationRecord, TerminalAllocationScope, control_holds,
    control_identity, control_lease, control_runtime, integer, invalid, key, policy, principal,
    read_conn, stale, validate_incarnation, validate_preparation,
};
use crate::commands::read_daemon_state_conn;
use crate::{
    AggregateKind, BrokerCause, BrokerIntent, DaemonStateRow, EventAppendSpec, StoreConfig,
    StoreError, UnleashStore,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use unleash_domain::{
    ActionId, ActorId, CommandId, DaemonMode, ProjectId, ReceiptId, Revision, Sha256Digest,
    TimestampMs,
};
use unleash_protocol::{EventCausationKind, Sensitivity, canonical_sha256};

pub(crate) struct RegisteredCommand<'a> {
    pub request: &'a RegisteredTerminalControl,
    pub receipt: ReceiptId,
    pub command: CommandId,
    pub actor: &'a ActorId,
    pub caller: &'a crate::CallerAuthority,
    pub project: Option<ProjectId>,
    pub expected: Revision,
}

/// Classify only inside the accepted writer transaction, after exact replay.
pub(crate) fn prepare_registered_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    command: &RegisteredCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    super::terminal_release::guard_pending(tx, d, command.request.terminal_id)?;
    let same_incarnation: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id WHERE s.terminal_id=?1 AND l.daemon_epoch=?2)",
        params![command.request.terminal_id.to_string(),integer(d.daemon_epoch.as_u64())?],|row|row.get(0),
    )?;
    let released = if d.schema_version >= 18 {
        tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id WHERE s.terminal_id=?1 AND l.state='released')",[command.request.terminal_id.to_string()],|row|row.get::<_,bool>(0))?
    } else {
        false
    };
    if d.schema_version >= 16 && same_incarnation && !released {
        guard_pending(tx, d, command.request.terminal_id)?;
        prepare_tx(tx, config, d, now, command, events)
    } else {
        super::prepare_control_tx(
            tx,
            config,
            d,
            now,
            command.request,
            command.receipt,
            command.command,
            command.actor,
            command.caller,
            command.project,
            command.expected,
            events,
        )
    }
}

pub(super) fn guard_pending(
    conn: &Connection,
    d: &DaemonStateRow,
    terminal: unleash_domain::TerminalId,
) -> Result<(), StoreError> {
    if d.schema_version >= 16 && conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE terminal_id=?1 AND state IN ('prepared','handshake_resolved','outcome_unknown'))",
        [terminal.to_string()],|row|row.get::<_,bool>(0),
    )? {
        return Err(StoreError::CapacityBlocked("terminal renewal requires observation or disposition".into()));
    }
    Ok(())
}

pub(super) fn guard_causal_assertions(
    conn: &Connection,
    allocation: &TerminalAllocationRecord,
) -> Result<(), StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 16 {
        return Ok(());
    }
    let c = &allocation.context;
    let unknown:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM terminal_renewal_intents r JOIN broker_dispatches b ON b.action_id=r.action_id JOIN actions a ON a.action_id=r.action_id WHERE r.terminal_id=?1 AND r.authority_generation=?2 AND json_extract(r.context_json,'$.initial_action_id')=?3 AND json_extract(r.context_json,'$.initial_context_digest')=?4 AND json_extract(r.context_json,'$.process_run_uuid')=?5 AND r.state IN ('prepared','handshake_resolved','outcome_unknown') AND a.state IN ('dispatching','in_flight','cancel_requested','outcome_unknown'))",
        params![c.terminal_id.to_string(),c.authority_generation.to_string(),c.action_id.to_string(),allocation.context_digest.to_string(),c.process_run_uuid.to_string()],|row|row.get(0))?;
    if unknown {
        return Err(stale(
            "terminal renewal requires observation or qualified transfer disposition",
        ));
    }
    Ok(())
}

fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    command: &RegisteredCommand<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if d.mode != DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(stale(
            "terminal renewal requires ready authority and a reliable clock",
        ));
    }
    let allocation = read_conn(tx, command.request.terminal_id)?
        .ok_or_else(|| invalid("registered terminal missing"))?;
    let c = &allocation.context;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = c.preparation.scope;
    if command.actor != &c.actor_id
        || principal(tx, command.actor)? != c.principal_id
        || command.project != Some(project_id)
        || command.request.process_run_uuid != c.process_run_uuid
        || c.authority_generation != d.authority_generation
        || c.daemon_boot_identity != d.boot_identity
    {
        return Err(invalid("terminal renewal actor, lineage or scope differs"));
    }
    let (lease,revision,expires):(String,i64,i64)=tx.query_row(
        "SELECT s.lease_id,s.control_revision,l.expires_at_ms FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id WHERE terminal_id=?1",
        [c.terminal_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    let revision =
        Revision::new(u64::try_from(revision).map_err(|_| invalid("terminal revision invalid"))?);
    if revision != command.expected {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: command.expected,
            command_id: Some(command.command),
        });
    }
    super::terminal_input_writer::guard_causal_assertions(tx, &allocation)?;
    let runtime = control_runtime(tx, now, &allocation)?;
    let mission:i64=tx.query_row("SELECT revision FROM missions WHERE mission_id=?1 AND project_id=?2 AND owner_kind='user' AND owner_user_id=?3 AND state IN ('ready','active')",
        params![c.preparation.mission_id.to_string(),project_id.to_string(),c.principal_id.as_str()],|row|row.get(0))?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalRenewalContext {
        schema_version: 1,
        command_id: command.command,
        receipt_id: command.receipt,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        terminal_id: c.terminal_id,
        initial_action_id: c.action_id,
        initial_context_digest: allocation.context_digest,
        host_id: c.host_id,
        process_id: c.process_id,
        process_run_uuid: c.process_run_uuid,
        actor_id: c.actor_id.clone(),
        principal_id: c.principal_id.clone(),
        authority_generation: d.authority_generation,
        daemon_epoch: d.daemon_epoch,
        daemon_boot_identity: d.boot_identity.clone(),
        project_id,
        scope_revision: command.request.scope_revision,
        mission_id: c.preparation.mission_id,
        mission_revision: Revision::new(
            u64::try_from(mission).map_err(|_| invalid("mission revision invalid"))?,
        ),
        policy_revision,
        security_policy_digest,
        runtime_settings_digest: canonical_sha256(&runtime)?,
        lease_id: lease.parse()?,
        resource_fence: control_lease(tx, lease.parse()?)?.fence,
        control_revision: revision,
        previous_expires_at_ms: TimestampMs::new(expires),
        proposed_expires_at_ms: TimestampMs::new(
            now.as_i64()
                .checked_add(runtime.lease_ttl_ms)
                .ok_or_else(|| invalid("renewal deadline overflow"))?,
        ),
        expected_os_identity: control_identity(tx, d, &allocation)?,
    };
    validate_controller(tx, d, now, &context)?;
    let digest = canonical_sha256(&context)?;
    tx.execute("INSERT INTO terminal_renewal_intents(action_id,terminal_id,receipt_id,command_id,actor_id,principal_id,authority_generation,daemon_epoch,lease_id,resource_fence,control_revision,previous_expires_at_ms,proposed_expires_at_ms,context_digest,context_json,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'prepared',1)",
        params![context.action_id.to_string(),context.terminal_id.to_string(),context.receipt_id.to_string(),context.command_id.to_string(),context.actor_id.as_str(),context.principal_id.as_str(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.lease_id.to_string(),integer(context.resource_fence.as_u64())?,integer(context.control_revision.as_u64())?,expires,context.proposed_expires_at_ms.as_i64(),digest.to_string(),serde_json::to_string(&context)?])?;
    events.push(EventAppendSpec {event_type:"TerminalRenewalPrepared".into(),aggregate_type:AggregateKind::TerminalControlIntent,
        aggregate_id:context.action_id.as_uuid(),aggregate_revision:Revision::new(1),project_id:Some(project_id),
        correlation_id:context.terminal_id.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,
        payload:json!({"terminal_id":context.terminal_id,"process_run_uuid":context.process_run_uuid,"control_revision":revision,"state":"preparing","reused":true}).as_object().cloned().ok_or_else(||invalid("renewal event invalid"))?,
        sensitivity:Sensitivity::Project,outbox_destinations:vec!["clients".into(),"harness".into()]});
    Ok(revision)
}

pub(super) fn validate_controller(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalRenewalContext,
) -> Result<TerminalAllocationRecord, StoreError> {
    if d.mode != DaemonMode::Ready
        || d.authority_generation != c.authority_generation
        || d.daemon_epoch != c.daemon_epoch
        || d.boot_identity != c.daemon_boot_identity
        || now >= c.previous_expires_at_ms
        || now >= c.proposed_expires_at_ms
        || principal(tx, &c.actor_id)? != c.principal_id
    {
        return Err(stale("terminal renewal authority or lease expired"));
    }
    let allocation =
        read_conn(tx, c.terminal_id)?.ok_or_else(|| invalid("terminal initial launch missing"))?;
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
        return Err(invalid("terminal renewal registration differs"));
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    if canonical_sha256(&runtime)? != c.runtime_settings_digest
        || policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest)
    {
        return Err(invalid("terminal renewal policy or runtime changed"));
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
        || lease.state != "held"
    {
        return Err(stale("terminal renewal is not the held controller"));
    }
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id JOIN resource_fences f ON f.authority_generation=l.authority_generation AND f.resource_kind=l.resource_kind AND f.resource_key=l.resource_key WHERE s.terminal_id=?1 AND s.process_id=?2 AND s.lease_id=?3 AND s.control_revision=?4 AND s.state IN ('ready','busy','detached') AND l.expires_at_ms=?5 AND f.last_fence=?6)",
        params![c.terminal_id.to_string(),c.process_id.to_string(),c.lease_id.to_string(),integer(c.control_revision.as_u64())?,c.previous_expires_at_ms.as_i64(),integer(c.resource_fence.as_u64())?],|row|row.get(0))?;
    if !exact {
        return Err(stale("terminal renewal controller CAS differs"));
    }
    Ok(allocation)
}

pub(super) fn validate_live(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    record: &TerminalRenewalRecord,
) -> Result<(), StoreError> {
    let c = &record.context;
    validate_incarnation(tx, d, c.action_id)?;
    validate_controller(tx, d, now, c)?;
    let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND command_id=?2 AND actor_id=?3 AND aggregate_id=?4 AND authority_generation=?5 AND command_type='AcquireTerminal' AND state='accepted' AND response_state='full')",
        params![c.receipt_id.to_string(),c.command_id.to_string(),c.actor_id.as_str(),c.terminal_id.to_string(),c.authority_generation.to_string()],|row|row.get(0))?;
    if !owned {
        return Err(invalid("terminal renewal receipt differs"));
    }
    Ok(())
}

pub(crate) fn read_renewal_conn(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalRenewalRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 16 {
        return Ok(None);
    }
    let row:Option<(String,String,String,i64,Option<String>,Option<String>)>=conn.query_row("SELECT context_json,context_digest,state,revision,dispatch_id,observation_object_hash FROM terminal_renewal_intents WHERE action_id=?1",[action.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
    row.map(
        |(encoded, digest, state, revision, dispatch, observation)| {
            let context: TerminalRenewalContext = serde_json::from_str(&encoded)?;
            let context_digest: Sha256Digest = digest.parse()?;
            if context.action_id != action || canonical_sha256(&context)? != context_digest {
                return Err(invalid("terminal renewal context differs"));
            }
            let state = match state.as_str() {
                "prepared" => TerminalRenewalState::Prepared,
                "handshake_resolved" => TerminalRenewalState::HandshakeResolved,
                "observed" => TerminalRenewalState::Observed,
                "outcome_unknown" => TerminalRenewalState::OutcomeUnknown,
                "disposed" => TerminalRenewalState::Disposed,
                _ => return Err(invalid("terminal renewal state invalid")),
            };
            let broker = crate::broker_writer::inspect_action_conn(conn, action)?;
            Ok(TerminalRenewalRecord {
                context,
                context_digest,
                state,
                revision: Revision::new(
                    u64::try_from(revision).map_err(|_| invalid("renewal revision invalid"))?,
                ),
                dispatch_id: dispatch.map(|id| id.parse()).transpose()?,
                observation_object_hash: observation.map(|id| id.parse()).transpose()?,
                broker_action_state: broker.as_ref().map(|row| row.state),
                broker_dispatch_id: broker.and_then(|row| row.dispatch_id),
            })
        },
    )
    .transpose()
}

impl UnleashStore {
    /// Resolve one accepted acquisition kind atomically, without contacting a host.
    /// # Errors
    /// Refuses ambiguous durable records or integrity failures; foreign commands remain absent.
    pub fn inspect_registered_terminal_request(
        &self,
        actor: &ActorId,
        command: CommandId,
    ) -> Result<
        crate::SnapshotProjection<Option<super::terminal_renewal_types::RegisteredTerminalIntent>>,
        StoreError,
    > {
        use super::terminal_renewal_types::RegisteredTerminalIntent;
        self.read_snapshot(|conn,_| {
            let d=read_daemon_state_conn(conn)?;
            let lookup=|table:&str| -> Result<Option<String>,StoreError> {
                // Names come solely from this closed implementation, never a request.
                Ok(conn.query_row(&format!("SELECT action_id FROM {table} WHERE actor_id=?1 AND command_id=?2 AND authority_generation=?3"),
                    params![actor.as_str(),command.to_string(),d.authority_generation.to_string()],|row|row.get(0)).optional()?)
            };
            let transfer=if d.schema_version>=12 {lookup("terminal_control_intents")?} else {None};
            let renewal=if d.schema_version>=16 {lookup("terminal_renewal_intents")?} else {None};
            let value=match (transfer,renewal) {
                (Some(_),Some(_))=>return Err(invalid("registered acquisition identity is ambiguous")),
                (Some(id),None)=>super::control_read_conn(conn,id.parse()?)?.map(RegisteredTerminalIntent::Transfer),
                (None,Some(id))=>read_renewal_conn(conn,id.parse()?)?.map(RegisteredTerminalIntent::Renewal),
                (None,None)=>None,
            };
            let high:i64=conn.query_row("SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",[d.stream_id.to_string()],|row|row.get(0))?;
            Ok(crate::SnapshotProjection {value,cursor:unleash_protocol::EventCursor::new(d.stream_id,
                unleash_domain::Sequence::new(u64::try_from(high).map_err(|_|invalid("terminal renewal watermark invalid"))?))})
        })
    }

    /// Inspect the exact authenticated renewal, never dispatching it.
    /// # Errors
    /// Returns storage/integrity errors; foreign or absent commands are hidden.
    pub fn inspect_terminal_renewal_request(
        &self,
        actor: &ActorId,
        command: CommandId,
    ) -> Result<crate::SnapshotProjection<Option<TerminalRenewalRecord>>, StoreError> {
        self.read_snapshot(|conn,_| {
            let d=read_daemon_state_conn(conn)?;
            let value=if d.schema_version<16 {None} else {
                let action:Option<String>=conn.query_row("SELECT action_id FROM terminal_renewal_intents WHERE actor_id=?1 AND command_id=?2 AND authority_generation=?3",
                    params![actor.as_str(),command.to_string(),d.authority_generation.to_string()],|row|row.get(0)).optional()?;
                action.map(|id|read_renewal_conn(conn,id.parse()?)).transpose()?.flatten()
            };
            let high:i64=conn.query_row("SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",[d.stream_id.to_string()],|row|row.get(0))?;
            Ok(crate::SnapshotProjection {value,cursor:unleash_protocol::EventCursor::new(d.stream_id,
                unleash_domain::Sequence::new(u64::try_from(high).map_err(|_|invalid("terminal renewal watermark invalid"))?))})
        })
    }
}

pub(super) fn validate_intent(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
) -> Result<bool, StoreError> {
    let Some(record) = read_renewal_conn(tx, intent.action_id)? else {
        return Ok(false);
    };
    let c = &record.context;
    validate_live(tx, d, now, &record)?;
    if !matches!(
        record.state,
        TerminalRenewalState::Prepared | TerminalRenewalState::HandshakeResolved
    ) || intent.principal_id != c.principal_id
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.project_id != Some(c.project_id)
        || intent.scope_revision != c.scope_revision
        || intent.policy_revision != c.policy_revision
        || intent.payload_digest != record.context_digest
        || intent.action_deadline != c.previous_expires_at_ms
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_RENEWAL_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs")
            != Some(&json!([{"kind":"terminal","value":c.terminal_id}]))
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "terminal renewal intent differs from accepted command",
        ));
    }
    Ok(true)
}
