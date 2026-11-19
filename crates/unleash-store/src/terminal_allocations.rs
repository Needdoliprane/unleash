//! Préparation transactionnelle F011 ; aucune identité OS ni opération de host.
//! Le resolver serveur qualifie chemins et environnement. Les fixtures de tests
//! n'attestent ni ce resolver ni un rattachement réel de PTY.

pub mod terminal_acquisition_query;
pub mod terminal_control_history;
mod terminal_input_execution;
mod terminal_input_observation;
pub mod terminal_input_types;
pub(crate) mod terminal_input_writer;
pub(crate) mod terminal_renewal;
mod terminal_renewal_observation;
mod terminal_renewal_recovery;
pub mod terminal_renewal_types;

use crate::{CallerAuthority, FenceAssertion, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, DaemonEpoch, DispatchId, Fence, HostId,
    LeaseId, MissionId, PoolId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, ReservationId,
    Revision, Sha256Digest, TerminalId, TimestampMs,
};

/// Verbe fermé du producteur d'allocation humaine.
pub const TERMINAL_ALLOCATION_VERB: &str = "terminal.acquire";

/// Identity of the explicitly configured user/default host pool; not a grant.
pub const TERMINAL_HOST_POOL_ID: PoolId = PoolId::from_u128(0xf011_1001);
/// Identity of the explicitly configured user/default child pool; not a grant.
pub const TERMINAL_CHILD_POOL_ID: PoolId = PoolId::from_u128(0xf011_1002);
/// Identity of the explicitly configured user/default PTY pool; not a grant.
pub const TERMINAL_PTY_POOL_ID: PoolId = PoolId::from_u128(0xf011_1003);

/// Separate authority transfer; it can never authorize another physical launch.
pub const TERMINAL_CONTROL_VERB: &str = "terminal.acquire_registered";

/// Initial supported control request; shared/observer leases are not inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlMode {
    /// One current controller under a monotone resource fence.
    Exclusive,
}

/// Closed wire payload for an already registered terminal. It contains no OS
/// identity, environment, launch command or caller-supplied authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredTerminalControl {
    /// Registered logical terminal, also the command envelope aggregate.
    pub terminal_id: TerminalId,
    /// Physical execution identity already observed by the initial host.
    pub process_run_uuid: ProcessRunUuid,
    /// Exact current project scope revision.
    pub scope_revision: Revision,
    /// Requested control mode; no implicit lease upgrade exists.
    pub control_mode: TerminalControlMode,
}

/// Qualified OS identity backend already recorded by the initial observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalIdentityBackend {
    /// macOS process information tied to boot/start identity.
    MacosProcPidinfo,
    /// Linux procfs identity tied to boot/start identity.
    LinuxProcfs,
}

/// Store-owned facts read from the existing process record. The public control
/// request cannot supply any of them, and the live host must confirm them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalProcessIdentity {
    /// Previously observed PID, never an adoption request.
    pub pid: u32,
    /// Previously observed process group.
    pub pgid: u32,
    /// Previously observed OS owner.
    pub owner_uid: u32,
    /// Previously observed boot identity.
    pub boot_identity: String,
    /// Previously observed process start identity.
    pub start_identity: String,
    /// Qualified source of those facts.
    pub backend: TerminalIdentityBackend,
}

/// Immutable transfer intent constructed by the writer from authenticated
/// identity and existing OS observations, before the broker is contacted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalControlContext {
    /// Closed context format.
    pub schema_version: u32,
    /// Exact accepted wire command; reused after an uncertain reply.
    pub command_id: unleash_domain::CommandId,
    /// Durable cause of this transfer action.
    pub receipt_id: ReceiptId,
    /// New control action; distinct from the original process launch action.
    pub action_id: ActionId,
    /// Registered resource.
    pub terminal_id: TerminalId,
    /// Original launch action; never redispatched by this intent.
    pub initial_action_id: ActionId,
    /// Unchanged V010 context acknowledged by the physical host.
    pub initial_context_digest: Sha256Digest,
    /// Existing host, never a newly selected host.
    pub host_id: HostId,
    /// Existing logical process.
    pub process_id: ProcessId,
    /// Existing physical execution.
    pub process_run_uuid: ProcessRunUuid,
    /// Actor resolved from the authenticated connection.
    pub actor_id: ActorId,
    /// Existing human principal resolved by the writer.
    pub principal_id: ActorId,
    /// Current authority lineage, identical to the launch lineage.
    pub authority_generation: AuthorityGeneration,
    /// Current daemon incarnation requesting control.
    pub daemon_epoch: DaemonEpoch,
    /// Current OS boot; another boot does not preserve a PTY.
    pub daemon_boot_identity: String,
    /// Existing project scope.
    pub project_id: ProjectId,
    /// Pinned project scope revision.
    pub scope_revision: Revision,
    /// Existing mission whose owner must still match.
    pub mission_id: MissionId,
    /// Pinned mission revision.
    pub mission_revision: Revision,
    /// Current effective policy revision.
    pub policy_revision: Revision,
    /// Exact security policy snapshot.
    pub security_policy_digest: Sha256Digest,
    /// Exact configured runtime settings read for this transfer.
    pub runtime_settings_digest: Sha256Digest,
    /// Session's former lease, retained historically.
    pub previous_lease_id: LeaseId,
    /// Former controller incarnation.
    pub previous_epoch: DaemonEpoch,
    /// Former resource fence, never reset.
    pub previous_fence: Fence,
    /// Planned successor lease; it is not held before proof activation.
    pub next_lease_id: LeaseId,
    /// Planned higher resource fence.
    pub next_fence: Fence,
    /// Current control CAS observed before this command was accepted.
    pub previous_control_revision: Revision,
    /// Stable deadline derived from configured TTL once, never refreshed by replay.
    pub expires_at_ms: TimestampMs,
    /// Actual identity previously recorded for this exact child, not a requested PID.
    pub expected_os_identity: TerminalProcessIdentity,
}

/// State of a transfer intent. Pending/unknown states never grant a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalControlState {
    /// Accepted intent, before a transfer effect permit.
    Prepared,
    /// Unique dispatch and transfer permit consumed before host contact.
    HandshakeResolved,
    /// Real host acknowledgement and activation committed atomically.
    Observed,
    /// Host effect/commit may have happened; inspect, never automatically repeat.
    OutcomeUnknown,
    /// Explicit qualified disposition retained historically.
    Disposed,
}
impl TerminalControlState {
    /// Closed `SQLite` spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::HandshakeResolved => "handshake_resolved",
            Self::Observed => "observed",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Disposed => "disposed",
        }
    }
}

/// Durable internal transfer snapshot, separate from immutable launch context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalControlRecord {
    /// Exact accepted transfer.
    pub context: TerminalControlContext,
    /// Canonical hash of that context, also the control action payload digest.
    pub context_digest: Sha256Digest,
    /// Lifecycle of this transfer, never a process launch state.
    pub state: TerminalControlState,
    /// Intent CAS, independent of session control revision.
    pub revision: Revision,
    /// Original unique control dispatch if engaged.
    pub dispatch_id: Option<DispatchId>,
    /// Published real host proof, only once verified and committed.
    pub observation_object_hash: Option<Sha256Digest>,
    /// Broker state from the same snapshot; a prepared intent may already have an unknown dispatch.
    pub broker_action_state: Option<ActionState>,
    /// Original broker marker, distinct from the consumed-effect permit above.
    pub broker_dispatch_id: Option<DispatchId>,
}

pub(crate) fn control_read_conn(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalControlRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 12 {
        return Ok(None);
    }
    let row:Option<(String,String,String,i64,Option<String>,Option<String>)>=conn.query_row("SELECT context_json,context_digest,state,revision,dispatch_id,observation_object_hash FROM terminal_control_intents WHERE action_id=?1",[action.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
    row.map(
        |(encoded, digest, state, revision, dispatch, observation)| {
            let context: TerminalControlContext = serde_json::from_str(&encoded)?;
            let context_digest: Sha256Digest = digest.parse()?;
            if context.action_id != action || canonical_sha256(&context)? != context_digest {
                return Err(invalid("terminal control context digest differs"));
            }
            let state = match state.as_str() {
                "prepared" => TerminalControlState::Prepared,
                "handshake_resolved" => TerminalControlState::HandshakeResolved,
                "observed" => TerminalControlState::Observed,
                "outcome_unknown" => TerminalControlState::OutcomeUnknown,
                "disposed" => TerminalControlState::Disposed,
                _ => return Err(invalid("terminal control state invalid")),
            };
            let broker = crate::broker_writer::inspect_action_conn(conn, action)?;
            Ok(TerminalControlRecord {
                context,
                context_digest,
                state,
                revision: Revision::new(
                    u64::try_from(revision)
                        .map_err(|_| invalid("terminal control revision invalid"))?,
                ),
                dispatch_id: dispatch.map(|id| id.parse()).transpose()?,
                observation_object_hash: observation.map(|hash| hash.parse()).transpose()?,
                broker_action_state: broker.as_ref().map(|snapshot| snapshot.state),
                broker_dispatch_id: broker.and_then(|snapshot| snapshot.dispatch_id),
            })
        },
    )
    .transpose()
}

impl crate::UnleashStore {
    /// Inspect the exact transfer accepted for this actor and command.
    /// # Errors
    /// Refuses malformed durable data; missing/foreign requests remain absent.
    pub fn inspect_terminal_control_request(
        &self,
        actor: &ActorId,
        command: unleash_domain::CommandId,
    ) -> Result<crate::SnapshotProjection<Option<TerminalControlRecord>>, StoreError> {
        self.read_snapshot(|conn,_| {
            let daemon=read_daemon_state_conn(conn)?;
            let high:i64=conn.query_row("SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",[daemon.stream_id.to_string()],|row|row.get(0))?;
            let value=if daemon.schema_version<12 {None}else {
                let action:Option<String>=conn.query_row("SELECT action_id FROM terminal_control_intents WHERE actor_id=?1 AND command_id=?2 AND authority_generation=?3",params![actor.as_str(),command.to_string(),daemon.authority_generation.to_string()],|row|row.get(0)).optional()?;
                action.map(|id|control_read_conn(conn,id.parse()?)).transpose()?.flatten()
            };
            Ok(crate::SnapshotProjection {value,cursor:unleash_protocol::EventCursor::new(daemon.stream_id,unleash_domain::Sequence::new(u64::try_from(high).map_err(|_|invalid("terminal watermark invalid"))?))})
        })
    }

    /// Consume one exact durable control dispatch before any host contact.
    /// A returned error, including a lost commit ACK, never grants a retry permit.
    /// # Errors
    /// Refuses another incarnation, stale context, an already consumed marker or expired authority.
    pub fn resolve_terminal_control(
        &self,
        request: ResolveTerminalControlRequest,
    ) -> Result<TerminalControlRecord, StoreError> {
        let config = self.config().clone();
        let result = self.execute_in_writer_tx(crate::WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            require_security_admin(&request.caller, "AcquireTerminal")?;
            let record = control_read_conn(tx, request.action_id)?
                .ok_or_else(|| invalid("terminal control intention missing"))?;
            let c = &record.context;
            if request.caller.actor_id != c.actor_id
                || request.context_digest != record.context_digest
                || request.authority_generation != c.authority_generation
                || request.expected_revision != record.revision
                || record.state != TerminalControlState::Prepared
                || record.dispatch_id.is_some()
            {
                return Err(invalid("terminal control permit consumed or request differs"));
            }
            if config.clock_uncertain.load(std::sync::atomic::Ordering::Acquire) {
                return Err(invalid("terminal control clock uncertain"));
            }
            let allocation = validate_control_live(tx, &daemon, now, &record)?;
            crate::broker_writer::validate_execution_dispatch_tx(
                tx, &daemon, now, c.action_id, request.dispatch_id,
            )?;
            let resource_key = key(&allocation.context.preparation)?;
            let fence = integer(c.next_fence.as_u64())?;
            if tx.execute("UPDATE resource_fences SET last_fence=?1 WHERE authority_generation=?2 AND resource_kind='terminal' AND resource_key=?3 AND last_fence=?4",params![fence,c.authority_generation.to_string(),resource_key,fence-1])? != 1 {
                return Err(stale("terminal planned fence is no longer available"));
            }
            tx.execute("UPDATE resource_leases SET state='revoked' WHERE lease_id=?1 AND state IN ('held','suspect','reconciling')",[c.previous_lease_id.to_string()])?;
            let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM resource_leases WHERE authority_generation=?1 AND resource_kind='terminal' AND resource_key=?2 AND state IN ('held','suspect','reconciling'))",params![c.authority_generation.to_string(),resource_key],|row|row.get(0))?;
            if active { return Err(invalid("terminal has another active controller")); }
            let runtime = control_runtime(tx, now, &allocation)?;
            tx.execute("INSERT INTO resource_leases(lease_id,authority_generation,resource_kind,resource_key,owner_type,owner_id,daemon_epoch,fence,boot_identity,ttl_ms,last_heartbeat_seq,expires_at_ms,state) VALUES(?1,?2,'terminal',?3,'principal',?4,?5,?6,?7,?8,0,?9,'reconciling')",params![c.next_lease_id.to_string(),c.authority_generation.to_string(),resource_key,c.principal_id.as_str(),integer(c.daemon_epoch.as_u64())?,fence,c.daemon_boot_identity,runtime.lease_ttl_ms,c.expires_at_ms.as_i64()])?;
            if tx.execute("UPDATE terminal_control_intents SET state='handshake_resolved',revision=revision+1,dispatch_id=?2 WHERE action_id=?1 AND state='prepared' AND revision=?3 AND dispatch_id IS NULL",params![c.action_id.to_string(),request.dispatch_id.to_string(),integer(record.revision.as_u64())?])? != 1 {
                return Err(invalid("terminal control resolution CAS lost"));
            }
            config.cutpoints.hit(crate::StoreCutpoint::C1BeforeCommandCommit)?;
            control_read_conn(tx,c.action_id)?.ok_or_else(||invalid("terminal control disappeared"))
        })?;
        self.config()
            .cutpoints
            .hit(crate::StoreCutpoint::C2AfterCommandCommit)?;
        Ok(result)
    }
}

/// Internal one-use request produced only after the broker's dispatch ACK.
#[derive(Debug, Clone)]
pub struct ResolveTerminalControlRequest {
    /// Exact control action, never the initial launch action.
    pub action_id: ActionId,
    /// Unique durable marker for this action.
    pub dispatch_id: DispatchId,
    /// Current lineage.
    pub authority_generation: AuthorityGeneration,
    /// Exact pending-intention CAS.
    pub expected_revision: Revision,
    /// Accepted immutable context.
    pub context_digest: Sha256Digest,
    /// Authenticated internal caller.
    pub caller: CallerAuthority,
}

/// Non-secret, correlated facts from the already running host after transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalControlObservation {
    /// Exact transfer action.
    pub action_id: ActionId,
    /// Unique durable transfer marker.
    pub dispatch_id: DispatchId,
    /// Immutable accepted transfer.
    pub context_digest: Sha256Digest,
    /// Existing terminal, never a fresh PTY.
    pub terminal_id: TerminalId,
    /// Existing host.
    pub host_id: HostId,
    /// Existing physical execution.
    pub process_run_uuid: ProcessRunUuid,
    /// Same verified OS identity as the original launch.
    pub os_identity: TerminalProcessIdentity,
    /// Exact acknowledged new daemon incarnation.
    pub daemon_epoch: DaemonEpoch,
    /// Exact acknowledged higher resource fence.
    pub resource_fence: Fence,
    /// Hash of the actual non-secret correlated ACK.
    pub transfer_ack_digest: Sha256Digest,
    /// Hash of the actual protected manifest read after the ACK.
    pub manifest_digest: Sha256Digest,
    /// Host-attested effective environment, unchanged by transfer.
    pub effective_environment_digest: Sha256Digest,
    /// Durable output offset at the observation.
    pub durable_log_seq: u64,
    /// Largest confirmed native input sequence.
    pub last_confirmed_input_seq: u64,
    /// Ambiguous native writes retained across transfer; never replayed.
    pub unknown_input_seqs: Vec<u64>,
    /// Capture time before proof publication and the broker receipt.
    pub observed_at_ms: TimestampMs,
}

impl crate::UnleashStore {
    /// Activate the new controller only with a published host proof and its exact broker observation.
    /// # Errors
    /// Refuses mismatched identities, stale incarnation/CAS, absent proof, or noncausal timestamps.
    pub fn commit_terminal_control_observation(
        &self,
        observation: TerminalControlObservation,
        phase: crate::BrokerPhaseRequest,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(crate::WriteCriticality::Normal,move|tx,now| {
            let d=read_daemon_state_conn(tx)?;
            let record=control_read_conn(tx,observation.action_id)?.ok_or_else(||invalid("terminal control observation has no intention"))?;
            let c=&record.context;
            let digest=canonical_sha256(&observation)?;
            let crate::BrokerPhase::Observation {action_id,receipt,..}=&phase.phase else {return Err(invalid("terminal control requires observation phase"));};
            if *action_id!=c.action_id || receipt.dispatch_id!=observation.dispatch_id
                || record.dispatch_id!=Some(observation.dispatch_id)
                || receipt.state!=ActionState::ObservedSuccess || receipt.output_digest!=Some(digest)
                || observation.context_digest!=record.context_digest
                || observation.terminal_id!=c.terminal_id || observation.host_id!=c.host_id
                || observation.process_run_uuid!=c.process_run_uuid
                || observation.os_identity!=c.expected_os_identity
                || observation.daemon_epoch!=c.daemon_epoch || observation.resource_fence!=c.next_fence
                || observation.unknown_input_seqs.len()>65536
                || observation.unknown_input_seqs.contains(&0)
            {return Err(invalid("terminal control observation differs from accepted transfer"));}
            if record.state==TerminalControlState::Observed {
                if record.observation_object_hash!=Some(digest) {return Err(invalid("terminal control proof replay differs"));}
                return crate::broker_writer::commit_phase_tx(tx,&config,now,&phase);
            }
            if !matches!(record.state,TerminalControlState::HandshakeResolved|TerminalControlState::OutcomeUnknown) {
                return Err(invalid("terminal control has no consumed effect permit"));
            }
            let allocation=validate_control_live(tx,&d,now,&record)?;
            let next=control_lease(tx,c.next_lease_id)?;
            if next.resource_kind!="terminal" || next.resource_key!=key(&allocation.context.preparation)? {
                return Err(invalid("terminal successor lease resource differs"));
            }
            crate::broker_writer::validate_execution_dispatch_tx(tx,&d,now,c.action_id,observation.dispatch_id)?;
            let started:i64=tx.query_row("SELECT started_at_ms FROM action_dispatches WHERE dispatch_id=?1 AND action_id=?2",params![observation.dispatch_id.to_string(),c.action_id.to_string()],|row|row.get(0))?;
            if observation.observed_at_ms.as_i64()<started || observation.observed_at_ms>receipt.observed_at_ms {
                return Err(invalid("terminal control capture timestamp is outside dispatch and receipt"));
            }
            let evidence:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='terminal_control_observation' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live')",params![c.action_id.to_string(),observation.dispatch_id.to_string(),digest.to_string()],|row|row.get(0))?;
            if !evidence {return Err(invalid("terminal control observation object is not published"));}
            if tx.execute("UPDATE resource_leases SET state='held' WHERE lease_id=?1 AND state='reconciling' AND authority_generation=?2 AND daemon_epoch=?3 AND fence=?4 AND boot_identity=?5 AND owner_type='principal' AND owner_id=?6 AND expires_at_ms=?7",params![c.next_lease_id.to_string(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(c.next_fence.as_u64())?,c.daemon_boot_identity,c.principal_id.as_str(),c.expires_at_ms.as_i64()])?!=1 {return Err(stale("terminal successor lease differs"));}
            if tx.execute("UPDATE terminal_sessions SET lease_id=?2,control_revision=control_revision+1,durable_log_seq=MAX(durable_log_seq,?3),state='ready' WHERE terminal_id=?1 AND process_id=?4 AND lease_id=?5 AND control_revision=?6",params![c.terminal_id.to_string(),c.next_lease_id.to_string(),integer(observation.durable_log_seq)?,c.process_id.to_string(),c.previous_lease_id.to_string(),integer(c.previous_control_revision.as_u64())?])?!=1 {return Err(invalid("terminal activation CAS lost"));}
            tx.execute("UPDATE processes SET state='live',last_observed_at_ms=?2 WHERE process_id=?1 AND process_run_uuid=?3",params![c.process_id.to_string(),observation.observed_at_ms.as_i64(),c.process_run_uuid.to_string()])?;
            for id in [allocation.context.host_reservation_id,allocation.context.child_reservation_id,allocation.context.pty_reservation_id] {
                if tx.execute("UPDATE capacity_reservations SET state='held',daemon_epoch=?2 WHERE reservation_id=?1 AND state IN ('held','reconciling') AND authority_generation=?3",params![id.to_string(),integer(c.daemon_epoch.as_u64())?,c.authority_generation.to_string()])?!=1 {return Err(invalid("terminal retained hold disappeared"));}
            }
            if tx.execute("UPDATE terminal_control_intents SET state='observed',revision=revision+1,observation_object_hash=?2 WHERE action_id=?1 AND state IN ('handshake_resolved','outcome_unknown') AND revision=?3 AND dispatch_id=?4",params![c.action_id.to_string(),digest.to_string(),integer(record.revision.as_u64())?,observation.dispatch_id.to_string()])?!=1 {return Err(invalid("terminal observation CAS lost"));}
            terminal_renewal_recovery::dispose_linked_transfer_tx(tx,&record,&observation)?;
            let proof=TerminalObservationProof {phase_digest:canonical_sha256(&phase.phase)?};
            crate::broker_writer::commit_terminal_observation_tx(tx,&config,now,&phase,&proof)
        })?;
        if !committed.replayed {
            self.config()
                .cutpoints
                .hit(crate::StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(committed)
    }
}

fn validate_control_live(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    record: &TerminalControlRecord,
) -> Result<TerminalAllocationRecord, StoreError> {
    let c = &record.context;
    validate_incarnation(tx, d, c.action_id)?;
    if d.mode != unleash_domain::DaemonMode::Ready
        || c.expires_at_ms <= now
        || principal(tx, &c.actor_id)? != c.principal_id
    {
        return Err(invalid("terminal control authority expired or unavailable"));
    }
    let allocation =
        read_conn(tx, c.terminal_id)?.ok_or_else(|| invalid("terminal launch missing"))?;
    if allocation.context_digest != c.initial_context_digest
        || allocation.context.action_id != c.initial_action_id
        || allocation.context.host_id != c.host_id
        || allocation.context.process_id != c.process_id
        || allocation.context.process_run_uuid != c.process_run_uuid
        || allocation.context.authority_generation != c.authority_generation
        || allocation.context.actor_id != c.actor_id
        || allocation.context.principal_id != c.principal_id
        || control_identity(tx, d, &allocation)? != c.expected_os_identity
    {
        return Err(invalid("terminal control initial registration differs"));
    }
    let runtime = control_runtime(tx, now, &allocation)?;
    if canonical_sha256(&runtime)? != c.runtime_settings_digest
        || policy(tx, c.project_id)? != (c.policy_revision, c.security_policy_digest)
    {
        return Err(invalid("terminal control settings or policy changed"));
    }
    let mut scoped = allocation.context.preparation.clone();
    scoped.scope = TerminalAllocationScope::ProjectConfiguration {
        project_id: c.project_id,
        revision: c.scope_revision,
    };
    scoped.mission_revision = c.mission_revision;
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, &c.actor_id, &c.principal_id)?;
    control_holds(tx, d, &allocation)?;
    let session:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions WHERE terminal_id=?1 AND process_id=?2 AND lease_id=?3 AND control_revision=?4)",params![c.terminal_id.to_string(),c.process_id.to_string(),c.previous_lease_id.to_string(),integer(c.previous_control_revision.as_u64())?],|row|row.get(0))?;
    let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2 AND command_id=?3 AND aggregate_id=?4 AND command_type='AcquireTerminal' AND authority_generation=?5 AND state='accepted' AND response_state='full')",params![c.receipt_id.to_string(),c.actor_id.as_str(),c.command_id.to_string(),c.terminal_id.to_string(),c.authority_generation.to_string()],|row|row.get(0))?;
    let previous = control_lease(tx, c.previous_lease_id)?;
    if !session
        || !receipt
        || previous.authority_generation != c.authority_generation
        || previous.boot_identity != c.daemon_boot_identity
        || previous.resource_kind != "terminal"
        || previous.resource_key != key(&allocation.context.preparation)?
        || previous.owner_type != "principal"
        || previous.owner_id != c.principal_id.as_str()
        || previous.daemon_epoch != c.previous_epoch
        || previous.fence != c.previous_fence
    {
        return Err(invalid(
            "terminal control receipt, session or former lease differs",
        ));
    }
    Ok(allocation)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlLease {
    lease_id: LeaseId,
    authority_generation: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    fence: Fence,
    resource_kind: String,
    resource_key: String,
    owner_type: String,
    owner_id: String,
    boot_identity: String,
    state: String,
}
fn control_lease(conn: &Connection, id: LeaseId) -> Result<ControlLease, StoreError> {
    let encoded:String=conn.query_row("SELECT json_object('lease_id',lease_id,'authority_generation',authority_generation,'daemon_epoch',daemon_epoch,'fence',fence,'resource_kind',resource_kind,'resource_key',resource_key,'owner_type',owner_type,'owner_id',owner_id,'boot_identity',boot_identity,'state',state) FROM resource_leases WHERE lease_id=?1",[id.to_string()],|row|row.get(0))?;
    Ok(serde_json::from_str(&encoded)?)
}
fn control_identity(
    conn: &Connection,
    d: &DaemonStateRow,
    allocation: &TerminalAllocationRecord,
) -> Result<TerminalProcessIdentity, StoreError> {
    let c = &allocation.context;
    if allocation.state != TerminalAllocationState::LaunchResolved
        || allocation.action_state != Some(ActionState::ObservedSuccess)
    {
        return Err(invalid("terminal has no confirmed registered launch"));
    }
    let encoded:Option<String>=conn.query_row("SELECT json_object('pid',p.pid,'pgid',p.pgid,'owner_uid',p.owner_uid,'boot_identity',p.boot_identity,'start_identity',p.start_identity,'backend',p.identity_backend) FROM terminal_sessions s JOIN processes p ON p.process_id=s.process_id WHERE s.terminal_id=?1 AND p.process_id=?2 AND p.action_id=?3 AND p.process_run_uuid=?4 AND p.host_id=?5 AND p.authority_generation=?6 AND p.cwd_ref=?7 AND s.mission_id=?8 AND s.purpose_key=?9 AND s.owner_agent_id IS NULL AND p.state IN ('live','suspect') AND s.state IN ('ready','busy','detached','suspect')",params![c.terminal_id.to_string(),c.process_id.to_string(),c.action_id.to_string(),c.process_run_uuid.to_string(),c.host_id.to_string(),c.authority_generation.to_string(),c.preparation.process_spec.cwd_canonical,c.preparation.mission_id.to_string(),c.preparation.purpose_key],|row|row.get(0)).optional()?;
    let identity: TerminalProcessIdentity = serde_json::from_str(
        &encoded.ok_or_else(|| invalid("terminal process registration is not controllable"))?,
    )?;
    if identity.pid == 0
        || identity.pgid == 0
        || identity.owner_uid != d.owner_uid
        || identity.boot_identity != d.boot_identity
        || identity.start_identity.trim().is_empty()
    {
        return Err(invalid(
            "terminal registered identity is from another owner or boot",
        ));
    }
    Ok(identity)
}
fn control_holds(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    allocation: &TerminalAllocationRecord,
) -> Result<(), StoreError> {
    let c = &allocation.context;
    validate_pools(tx, &c.preparation.auxiliary)?;
    for ((pool, kind), id) in pool_bindings(&c.preparation.auxiliary).into_iter().zip([
        c.host_reservation_id,
        c.child_reservation_id,
        c.pty_reservation_id,
    ]) {
        let (owner_type, owner) = if kind == "pty" {
            ("terminal", c.terminal_id.to_string())
        } else {
            ("process", c.process_id.to_string())
        };
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_reservations WHERE reservation_id=?1 AND pool_id=?2 AND owner_type=?3 AND owner_id=?4 AND amount=1 AND admission_class IS NULL AND state IN ('held','reconciling') AND authority_generation=?5 AND daemon_epoch BETWEEN ?6 AND ?7)",params![id.to_string(),pool.to_string(),owner_type,owner,d.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,integer(d.daemon_epoch.as_u64())?],|row|row.get(0))?;
        if !valid {
            return Err(invalid(
                "terminal transfer must retain its original capacity holds",
            ));
        }
    }
    Ok(())
}
fn control_runtime(
    tx: &Transaction<'_>,
    now: TimestampMs,
    allocation: &TerminalAllocationRecord,
) -> Result<TerminalRuntimeSettings, StoreError> {
    let settings = crate::projects_settings::read_settings_conn(
        tx,
        now,
        crate::SettingsScopeKind::User,
        "default",
    )?;
    let raw = settings
        .extra_typed
        .get("terminal_runtime")
        .ok_or_else(|| invalid("terminal runtime is not configured"))?;
    let runtime: TerminalRuntimeSettings = serde_json::from_value(raw.clone())?;
    runtime.validate()?;
    if runtime.pools() != allocation.context.preparation.auxiliary {
        return Err(invalid("terminal transfer pool identities changed"));
    }
    Ok(runtime)
}

struct ValidatedControlTarget {
    runtime: TerminalRuntimeSettings,
    mission_revision: Revision,
    identity: TerminalProcessIdentity,
    previous: ControlLease,
    next_fence: Fence,
}

fn validate_control_target(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    request: &RegisteredTerminalControl,
    allocation: &TerminalAllocationRecord,
    lease_id: &str,
) -> Result<ValidatedControlTarget, StoreError> {
    let original = &allocation.context;
    let principal = &original.principal_id;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
        original.preparation.scope;
    let runtime = control_runtime(tx, now, allocation)?;
    let mission_revision:i64=tx.query_row("SELECT revision FROM missions WHERE mission_id=?1 AND project_id=?2 AND owner_kind='user' AND owner_user_id=?3 AND state IN ('ready','active')",params![original.preparation.mission_id.to_string(),project_id.to_string(),principal.as_str()],|row|row.get(0))?;
    let mut scoped = original.preparation.clone();
    scoped.scope = TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision: request.scope_revision,
    };
    scoped.mission_revision = Revision::new(
        u64::try_from(mission_revision)
            .map_err(|_| invalid("terminal mission revision invalid"))?,
    );
    scoped.lease_ttl_ms = runtime.lease_ttl_ms;
    validate_preparation(tx, now, &scoped, &original.actor_id, principal)?;
    control_holds(tx, d, allocation)?;
    let identity = control_identity(tx, d, allocation)?;
    let previous = control_lease(tx, lease_id.parse()?)?;
    if previous.daemon_epoch == d.daemon_epoch {
        return Err(stale(
            "registered terminal transfer requires a successor daemon epoch",
        ));
    }
    let resource_key = key(&original.preparation)?;
    if previous.authority_generation != d.authority_generation
        || previous.boot_identity != d.boot_identity
        || previous.resource_kind != "terminal"
        || previous.resource_key != resource_key
        || previous.owner_type != "principal"
        || previous.owner_id != principal.as_str()
        || previous.daemon_epoch > d.daemon_epoch
        || !["held", "suspect", "reconciling", "released", "revoked"]
            .contains(&previous.state.as_str())
    {
        return Err(invalid("terminal previous lease is not attributable"));
    }
    let active:Option<String>=tx.query_row("SELECT lease_id FROM resource_leases WHERE authority_generation=?1 AND resource_kind='terminal' AND resource_key=?2 AND state IN ('held','suspect','reconciling')",params![d.authority_generation.to_string(),resource_key],|row|row.get(0)).optional()?;
    if active.is_some_and(|id| id != previous.lease_id.to_string()) {
        return Err(StoreError::CapacityBlocked(
            "terminal lease belongs to another controller".into(),
        ));
    }
    let last:i64=tx.query_row("SELECT last_fence FROM resource_fences WHERE authority_generation=?1 AND resource_kind='terminal' AND resource_key=?2",params![d.authority_generation.to_string(),resource_key],|row|row.get(0))?;
    let next_fence = Fence::new(
        u64::try_from(
            last.checked_add(1)
                .ok_or_else(|| invalid("terminal fence exhausted"))?,
        )
        .map_err(|_| invalid("terminal fence invalid"))?,
    );
    if next_fence <= previous.fence {
        return Err(stale("terminal resource fence is inconsistent"));
    }
    Ok(ValidatedControlTarget {
        runtime,
        mission_revision: scoped.mission_revision,
        identity,
        previous,
        next_fence,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Authenticated command identities and one transfer transaction"
)]
pub(crate) fn prepare_control_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    request: &RegisteredTerminalControl,
    receipt: ReceiptId,
    command: unleash_domain::CommandId,
    actor: &ActorId,
    caller: &CallerAuthority,
    project: Option<ProjectId>,
    expected: Revision,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if d.schema_version < 12
        || d.mode != unleash_domain::DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "terminal control requires ready V012 authority and a reliable clock",
        ));
    }
    let allocation = read_conn(tx, request.terminal_id)?
        .ok_or_else(|| invalid("registered terminal allocation missing"))?;
    let original = &allocation.context;
    let principal = principal(tx, actor)?;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } =
        original.preparation.scope;
    if original.authority_generation != d.authority_generation
        || original.daemon_boot_identity != d.boot_identity
        || actor != &original.actor_id
        || principal != original.principal_id
        || project != Some(project_id)
        || request.process_run_uuid != original.process_run_uuid
    {
        return Err(invalid("terminal control lineage, actor or scope differs"));
    }
    let (lease_id, current): (String, i64) = tx.query_row(
        "SELECT lease_id,control_revision FROM terminal_sessions WHERE terminal_id=?1",
        [request.terminal_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let revision = Revision::new(
        u64::try_from(current).map_err(|_| invalid("terminal control revision invalid"))?,
    );
    if expected != revision {
        return Err(StoreError::RevisionConflict {
            current_revision: revision,
            expected_revision: expected,
            command_id: Some(command),
        });
    }
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_control_intents WHERE terminal_id=?1 AND state IN ('prepared','handshake_resolved','outcome_unknown'))",[request.terminal_id.to_string()],|row|row.get(0))?;
    if pending {
        return Err(StoreError::CapacityBlocked(
            "terminal control transfer requires observation or disposition".into(),
        ));
    }
    let ValidatedControlTarget {
        runtime,
        mission_revision,
        identity,
        previous,
        next_fence,
    } = validate_control_target(tx, d, now, request, &allocation, &lease_id)?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalControlContext {
        schema_version: 1,
        command_id: command,
        receipt_id: receipt,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        terminal_id: request.terminal_id,
        initial_action_id: original.action_id,
        initial_context_digest: allocation.context_digest,
        host_id: original.host_id,
        process_id: original.process_id,
        process_run_uuid: original.process_run_uuid,
        actor_id: actor.clone(),
        principal_id: principal,
        authority_generation: d.authority_generation,
        daemon_epoch: d.daemon_epoch,
        daemon_boot_identity: d.boot_identity.clone(),
        project_id,
        scope_revision: request.scope_revision,
        mission_id: original.preparation.mission_id,
        mission_revision,
        policy_revision,
        security_policy_digest,
        runtime_settings_digest: canonical_sha256(&runtime)?,
        previous_lease_id: previous.lease_id,
        previous_epoch: previous.daemon_epoch,
        previous_fence: previous.fence,
        next_lease_id: LeaseId::from_uuid(config.id_generator.new_uuid()),
        next_fence,
        previous_control_revision: revision,
        expires_at_ms: TimestampMs::new(
            now.as_i64()
                .checked_add(runtime.lease_ttl_ms)
                .ok_or_else(|| invalid("terminal control deadline overflow"))?,
        ),
        expected_os_identity: identity,
    };
    let digest = canonical_sha256(&context)?;
    terminal_renewal_recovery::link_transfer_tx(tx, d, now, &context, caller, events)?;
    tx.execute("INSERT INTO terminal_control_intents(action_id,terminal_id,receipt_id,command_id,actor_id,principal_id,authority_generation,daemon_epoch,context_digest,context_json,previous_lease_id,next_lease_id,next_fence,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'prepared',1)",params![context.action_id.to_string(),context.terminal_id.to_string(),receipt.to_string(),command.to_string(),actor.as_str(),context.principal_id.as_str(),d.authority_generation.to_string(),integer(d.daemon_epoch.as_u64())?,digest.to_string(),serde_json::to_string(&context)?,previous.lease_id.to_string(),context.next_lease_id.to_string(),integer(next_fence.as_u64())?])?;
    events.push(EventAppendSpec {event_type:"TerminalControlPrepared".into(),aggregate_type:AggregateKind::TerminalControlIntent,aggregate_id:context.action_id.as_uuid(),aggregate_revision:Revision::new(1),project_id:Some(project_id),correlation_id:request.terminal_id.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,payload:json!({"terminal_id":request.terminal_id,"process_run_uuid":request.process_run_uuid,"control_revision":revision,"state":"preparing","reused":true}).as_object().cloned().ok_or_else(||invalid("terminal control event invalid"))?,sensitivity:Sensitivity::Project,outbox_destinations:vec!["clients".into(),"harness".into()]});
    Ok(revision)
}

/// Non-secret environment explicitly configured by an authenticated administrator.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEnvironmentProfile {
    /// Stable identifier, unique within the active configuration.
    pub profile_id: String,
    /// Immutable version of this profile.
    pub version: u64,
    /// Explicit, closed locale/terminal values; never an inherited environment.
    pub variables: std::collections::BTreeMap<String, String>,
}
impl std::fmt::Debug for TerminalEnvironmentProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalEnvironmentProfile")
            .field("profile_id", &self.profile_id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl TerminalEnvironmentProfile {
    /// Digest of the complete explicit profile, including its values.
    /// # Errors
    /// Returns a canonical serialization error.
    pub fn digest(&self) -> Result<Sha256Digest, StoreError> {
        Ok(canonical_sha256(self)?)
    }
}

/// Version-one global terminal settings, persisted by the existing `SetSettings` CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalRuntimeSettings {
    /// Closed format version.
    pub schema_version: u32,
    /// Explicit non-secret profiles; no implicit default profile exists.
    pub profiles: Vec<TerminalEnvironmentProfile>,
    /// Auxiliary host limit, distinct from execution admission.
    pub max_hosts: u32,
    /// Auxiliary child limit.
    pub max_processes: u32,
    /// Auxiliary PTY limit.
    pub max_ptys: u32,
    /// Explicit lease TTL in milliseconds.
    pub lease_ttl_ms: i64,
}
impl TerminalRuntimeSettings {
    /// Validate the closed initial profile namespace and finite non-secret values.
    /// # Errors
    /// Refuses missing/duplicate profiles, secrets, unknown fields or unsafe bounds.
    pub fn validate(&self) -> Result<(), StoreError> {
        let mut ids = std::collections::BTreeSet::new();
        if self.schema_version != 1
            || !(1..=16).contains(&self.profiles.len())
            || [self.max_hosts, self.max_processes, self.max_ptys]
                .iter()
                .any(|n| !(1..=50).contains(n))
            || !(1000..=300_000).contains(&self.lease_ttl_ms)
        {
            return Err(invalid("invalid terminal runtime settings"));
        }
        for p in &self.profiles {
            if p.profile_id.is_empty()
                || p.profile_id.len() > 64
                || !p.profile_id.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b"._-".contains(&b))
                })
                || !ids.insert(&p.profile_id)
                || !(1..=9_007_199_254_740_991).contains(&p.version)
                || p.variables.is_empty()
                || p.variables.len() > 4
            {
                return Err(invalid("invalid terminal environment profile"));
            }
            for (key, value) in &p.variables {
                let allowed = match key.as_str() {
                    "LANG" | "LC_ALL" => {
                        ["C", "C.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"].contains(&value.as_str())
                    }
                    "TERM" => {
                        ["xterm", "xterm-256color", "vt100", "dumb"].contains(&value.as_str())
                    }
                    "TZ" => value == "UTC",
                    _ => false,
                };
                if !allowed {
                    return Err(invalid(
                        "terminal profile contains an unsupported environment value",
                    ));
                }
            }
        }
        Ok(())
    }
    /// Stable identities of the three configured global auxiliary pools.
    #[must_use]
    pub const fn pools(&self) -> TerminalAuxiliaryPools {
        TerminalAuxiliaryPools {
            host: TERMINAL_HOST_POOL_ID,
            child: TERMINAL_CHILD_POOL_ID,
            pty: TERMINAL_PTY_POOL_ID,
        }
    }
}

/// Private transport between the authenticated producer and the configuration writer.
/// It is never accepted from the public IPC payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTerminalEnvironment {
    profile_id: String,
    version: u64,
    digest: Sha256Digest,
    pin_id: String,
    object_generation: i64,
}

pub(crate) fn configure_runtime_tx(
    tx: &Transaction<'_>,
    now: TimestampMs,
    next: &TerminalRuntimeSettings,
    previous: Option<&Value>,
    prepared: &[PreparedTerminalEnvironment],
    command_id: unleash_domain::CommandId,
    ids: &dyn unleash_domain::IdGeneratorPort,
) -> Result<(), StoreError> {
    next.validate()?;
    if let Some(old) = previous {
        let old: TerminalRuntimeSettings = serde_json::from_value(old.clone())?;
        for p in &next.profiles {
            if let Some(prior) = old.profiles.iter().find(|v| v.profile_id == p.profile_id)
                && (p.version < prior.version || (p.version == prior.version && p != prior))
            {
                return Err(invalid("terminal profile version is immutable"));
            }
        }
    }
    if prepared.len() != next.profiles.len() {
        return Err(invalid(
            "terminal environments were not prepared by the service",
        ));
    }
    for p in &next.profiles {
        let digest = p.digest()?.to_string();
        let highest:i64=tx.query_row("SELECT COALESCE(MAX(CAST(source_field AS INTEGER)),0) FROM object_references WHERE owner_kind='terminal_environment' AND owner_id=?1",[&p.profile_id],|r|r.get(0))?;
        if integer(p.version)? < highest {
            return Err(invalid("terminal profile version cannot decrease"));
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_references WHERE owner_kind='terminal_environment' AND owner_id=?1 AND source_field=?2 AND object_hash!=?3)",params![p.profile_id,p.version.to_string(),digest],|r|r.get(0))?;
        if conflict {
            return Err(invalid("terminal environment version is conflicting"));
        }
        let pin = prepared
            .iter()
            .find(|v| {
                v.profile_id == p.profile_id
                    && v.version == p.version
                    && v.digest.to_string() == digest
            })
            .ok_or_else(|| invalid("terminal prepared bytes differ from configuration"))?;
        let owner:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_pins WHERE pin_id=?1 AND object_hash=?2 AND owner_kind='terminal_configuration_candidate' AND owner_id=?3 AND state='held')",params![pin.pin_id.as_str(),digest,command_id.to_string()],|r|r.get(0))?;
        if !owner {
            return Err(invalid(
                "terminal prepared pin is not owned by this command",
            ));
        }
        crate::objects::tx_publish_object_reference(
            tx,
            ids,
            now,
            &crate::PublishObjectReferenceRequest {
                pin_id: pin.pin_id.clone(),
                object_hash: pin.digest,
                object_generation: pin.object_generation,
                owner_kind: "terminal_environment".into(),
                owner_id: p.profile_id.clone(),
                source_field: p.version.to_string(),
            },
        )?;
    }
    for ((id, kind), limit) in pool_bindings(&next.pools()).into_iter().zip([
        next.max_hosts,
        next.max_processes,
        next.max_ptys,
    ]) {
        let occupation:i64=tx.query_row("SELECT COALESCE(SUM(amount),0) FROM capacity_reservations WHERE pool_id=?1 AND state IN ('held','reconciling')",[id.to_string()],|r|r.get(0))?;
        if occupation > i64::from(limit) {
            return Err(invalid("terminal limit is below held capacity"));
        }
        tx.execute("INSERT INTO capacity_pools(pool_id,pool_kind,scope_key,unit,requested_limit,effective_limit,admissions_frozen,hard_limit,revision) VALUES(?1,?2,'global','slot',?3,?3,0,50,1) ON CONFLICT(pool_id) DO UPDATE SET requested_limit=excluded.requested_limit,effective_limit=excluded.effective_limit,revision=capacity_pools.revision+1",params![id.to_string(),kind,limit])?;
    }
    Ok(())
}

impl crate::UnleashStore {
    /// Read and validate the actual user/default terminal configuration.
    /// # Errors
    /// Missing or invalid configuration is refused, never filled with defaults.
    pub fn terminal_runtime_settings(&self) -> Result<TerminalRuntimeSettings, StoreError> {
        let settings = self.read_settings(crate::SettingsScopeKind::User, "default")?;
        let raw = settings
            .extra_typed
            .get("terminal_runtime")
            .ok_or_else(|| invalid("terminal runtime is not configured"))?;
        let cfg: TerminalRuntimeSettings = serde_json::from_value(raw.clone())?;
        cfg.validate()?;
        Ok(cfg)
    }
    /// Prepare explicit profile bytes without reserving the immutable version.
    /// # Errors
    /// Refuses invalid bytes. Publication belongs exclusively to the accepted `SetSettings` transaction.
    pub fn prepare_terminal_environment(
        &self,
        p: &TerminalEnvironmentProfile,
        command: unleash_domain::CommandId,
    ) -> Result<PreparedTerminalEnvironment, StoreError> {
        let bytes = unleash_protocol::canonicalize_to_vec(p)?;
        let hash = p.digest()?;
        let handle = self.prepare_object(crate::PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".into(),
            owner_kind: "terminal_configuration_candidate".into(),
            owner_id: command.to_string(),
        })?;
        self.write_prepared_object_bytes(&handle, &bytes)?;
        let record = self.object_record(hash)?;
        crate::objects::verify_object_file_before_publish(
            self.config(),
            &record.relative_path,
            hash,
        )?;
        Ok(PreparedTerminalEnvironment {
            profile_id: p.profile_id.clone(),
            version: p.version,
            digest: hash,
            pin_id: handle.pin_id,
            object_generation: handle.object_generation,
        })
    }
}

/// Source de portée authentifiée du sous-chemin humain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalAllocationScope {
    /// Configuration du projet de la mission, relue dans le writer.
    ProjectConfiguration {
        /// Projet durable.
        project_id: ProjectId,
        /// Révision exacte.
        revision: Revision,
    },
}

/// Référence d'environnement qualifiée par le resolver serveur ; aucun secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEnvironmentRef {
    /// Identité du profil effectif.
    pub profile_id: String,
    /// Version exacte du profil.
    pub version: u64,
    /// Objet publié représentant ce profil, sans valeur secrète.
    pub digest: Sha256Digest,
}

/// Spécification normalisée de ce premier sous-chemin, sans worktree implicite.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTerminalProcessSpec {
    /// Programme absolu résolu ; aucun shell implicite.
    pub program: String,
    /// Arguments exacts.
    pub arguments: Vec<String>,
    /// Chemin relatif demandé dans la racine de projet.
    pub requested_cwd: String,
    /// Chemin canonique qualifié par le resolver.
    pub cwd_canonical: String,
    /// Profil d'environnement exact, jamais un environnement vide déduit.
    pub environment: TerminalEnvironmentRef,
    /// Lignes PTY.
    pub rows: u16,
    /// Colonnes PTY.
    pub cols: u16,
    /// Readiness `process_started`, délai borné par le serveur.
    pub readiness_timeout_ms: u64,
}

/// Pools auxiliaires préalablement configurés ; aucun défaut chiffré implicite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAuxiliaryPools {
    /// Pool `process_host` global, unité `slot`.
    pub host: PoolId,
    /// Pool `process` global, unité `slot`.
    pub child: PoolId,
    /// Pool `pty` global, unité `slot`.
    pub pty: PoolId,
}

/// Paramètres résolus par un service authentifié, distincts du payload client.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAllocationPreparation {
    /// Mission réelle.
    pub mission_id: MissionId,
    /// Révision de mission à relire.
    pub mission_revision: Revision,
    /// Portée configurée réelle.
    pub scope: TerminalAllocationScope,
    /// Propriété exclusive dans la mission.
    pub purpose_key: String,
    /// Commande et environnement qualifiés par le resolver.
    pub process_spec: NormalizedTerminalProcessSpec,
    /// Réserves auxiliaires séparées du pool d'exécution.
    pub auxiliary: TerminalAuxiliaryPools,
    /// TTL effectif fourni par la configuration serveur.
    pub lease_ttl_ms: i64,
}

impl std::fmt::Debug for TerminalAllocationPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalAllocationPreparation")
            .field("mission_id", &self.mission_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}
impl TerminalAllocationPreparation {
    /// Payload demandé correspondant à cette résolution (premier sous-chemin).
    #[must_use]
    pub fn command_payload(&self) -> Value {
        let TerminalAllocationScope::ProjectConfiguration {
            project_id,
            revision,
        } = self.scope;
        json!({"mission_id":self.mission_id,"invocation_id":null,"purpose_key":self.purpose_key,"reuse_policy":"reuse_compatible","scope_id":project_id.to_string(),"scope_revision":revision,"worktree_id":null,"cwd":self.process_spec.requested_cwd,"program":self.process_spec.program,"arguments":self.process_spec.arguments,"environment_profile_id":self.process_spec.environment.profile_id,"pty":{"rows":self.process_spec.rows,"cols":self.process_spec.cols},"readiness":{"kind":"process_started","timeout_ms":self.process_spec.readiness_timeout_ms}})
    }
}

/// Identités et autorité figées ; les IDs OS restent absents.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAllocationContext {
    /// Version du document.
    pub schema_version: u32,
    /// Terminal logique stable.
    pub terminal_id: TerminalId,
    /// Processus logique stable.
    pub process_id: ProcessId,
    /// Génération physique planifiée, sans preuve de spawn.
    pub process_run_uuid: ProcessRunUuid,
    /// Host planifié, sans preuve de présence.
    pub host_id: HostId,
    /// Action allouée, créée seulement à Authorizing.
    pub action_id: ActionId,
    /// Reçu causal authentique.
    pub receipt_id: ReceiptId,
    /// Acteur authentifié.
    pub actor_id: ActorId,
    /// Principal résolu depuis cet acteur.
    pub principal_id: ActorId,
    /// Lignée de préparation.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation de préparation.
    pub daemon_epoch: DaemonEpoch,
    /// Boot du daemon, jamais une identité de processus observée.
    pub daemon_boot_identity: String,
    /// Bail de ressource distinct du fence broker.
    pub lease_id: LeaseId,
    /// Fence de ressource.
    pub resource_fence: Fence,
    /// Expiration initiale du bail.
    pub lease_expires_at: TimestampMs,
    /// Réservation host.
    pub host_reservation_id: ReservationId,
    /// Réservation enfant.
    pub child_reservation_id: ReservationId,
    /// Réservation PTY.
    pub pty_reservation_id: ReservationId,
    /// Résolution immuable.
    pub preparation: TerminalAllocationPreparation,
    /// Révision de politique de sécurité exacte.
    pub policy_revision: Revision,
    /// Snapshot de sécurité effectivement lu.
    pub security_policy_digest: Sha256Digest,
}

/// État de la préparation, distinct de l'état OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalAllocationState {
    /// IDs et ressources réservés ; aucun lancement.
    Allocated,
    /// Marqueur consommé une fois ; inspection seulement en cas d'ACK perdu.
    LaunchResolved,
    /// Retrait sous observation qualifiée par un futur port interne.
    Retired,
}

/// Lecture interne et autorité figée.
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalAllocationRecord {
    /// Document exact.
    pub context: TerminalAllocationContext,
    /// Digest canonique également utilisé comme payload broker.
    pub context_digest: Sha256Digest,
    /// État de préparation.
    pub state: TerminalAllocationState,
    /// Révision CAS.
    pub revision: Revision,
    /// Marqueur consommé éventuel.
    pub dispatch_id: Option<DispatchId>,
    /// État broker observé dans le même snapshot.
    pub action_state: Option<ActionState>,
    /// Observation liée au retrait, conservée historiquement.
    pub retirement_evidence_id: Option<String>,
}
impl std::fmt::Debug for TerminalAllocationRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalAllocationRecord")
            .field("terminal_id", &self.context.terminal_id)
            .field("context_digest", &self.context_digest)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl TerminalAllocationRecord {
    /// Empreinte exacte autorisée.
    #[must_use]
    pub const fn payload_digest(&self) -> Sha256Digest {
        self.context_digest
    }
    /// Référence native du terminal logique.
    #[must_use]
    pub fn resource_refs(&self) -> Value {
        json!([{"kind":"terminal","value":self.context.terminal_id}])
    }
}

/// Résolution interne avant effet ; le client ne produit pas cette autorité.
#[derive(Debug, Clone)]
pub struct ResolveTerminalLaunchRequest {
    /// Terminal exact.
    pub terminal_id: TerminalId,
    /// Génération exacte.
    pub process_run_uuid: ProcessRunUuid,
    /// Action exacte.
    pub action_id: ActionId,
    /// Dispatch exact.
    pub dispatch_id: DispatchId,
    /// Autorité active, vérifiée avant lecture.
    pub authority_generation: AuthorityGeneration,
    /// Révision CAS.
    pub expected_revision: Revision,
    /// Document exact.
    pub context_digest: Sha256Digest,
    /// Payload exact.
    pub payload_digest: Sha256Digest,
    /// Autorité authentifiée du service.
    pub caller: CallerAuthority,
    /// Bail exact porté par la préparation.
    pub lease: FenceAssertion,
}

/// Résolution unique remise au daemon ; aucune opération OS dans le Store.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedTerminalLaunch {
    /// Préparation exacte consommée.
    pub allocation: TerminalAllocationRecord,
}
impl std::fmt::Debug for ResolvedTerminalLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTerminalLaunch")
            .field("allocation", &self.allocation)
            .finish()
    }
}

use crate::commands::{ensure_scope_not_frozen_tx, read_daemon_state_conn};
use crate::leases::{LeaseLineageCtx, tx_acquire_lease, tx_validate_lease_fence};
use crate::policies_secrets::{
    SubjectKind, ensure_subject_not_blocked_tx, read_policy_summary_conn, require_security_admin,
};
use crate::{
    AggregateKind, BrokerCause, BrokerIntent, CapacityReservationRequest, CommandMutation,
    CommitCommandRequest, DaemonStateRow, EventAppendSpec, StoreConfig,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use unleash_protocol::{EventCausationKind, Sensitivity, canonical_sha256};
fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
fn stale(message: &'static str) -> StoreError {
    StoreError::StaleFence {
        message: message.to_owned(),
    }
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("terminal integer out of range"))
}
fn key(prep: &TerminalAllocationPreparation) -> Result<String, StoreError> {
    Ok(format!(
        "terminal-purpose:{}",
        canonical_sha256(&json!({"mission_id":prep.mission_id,"purpose_key":prep.purpose_key}))?
    ))
}
fn valid_ref(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

pub(crate) fn validate_command(
    tx: &Transaction<'_>,
    req: &CommitCommandRequest,
) -> Result<(), StoreError> {
    if let CommandMutation::PrepareTerminalInput { caller, request } = &req.mutation {
        return terminal_input_writer::validate_command(req, caller, request);
    }
    if req.envelope.command_type == "SendTerminalInput" {
        return Err(invalid("terminal input mutation missing"));
    }
    if let CommandMutation::PrepareTerminalControl { caller, request } = &req.mutation {
        if req.target_aggregate != AggregateKind::TerminalControl
            || req.envelope.command_type != "AcquireTerminal"
            || req.envelope.aggregate_id != request.terminal_id.as_uuid()
            || req.envelope.project_id.is_none()
            || caller.actor_id != req.actor_id
            || !matches!(
                caller.role,
                crate::CallerRole::PolicyAdmin | crate::CallerRole::HumanOperator
            )
            || req.operation_id.is_some()
            || req.result_ref.is_some()
            || !req.events.is_empty()
            || !req.capacity_reservations.is_empty()
            || !req.budget_reservations.is_empty()
            || serde_json::to_value(request)?.as_object() != Some(&req.envelope.payload)
        {
            return Err(invalid(
                "registered terminal command identity or closed payload invalid",
            ));
        }
        return Ok(());
    }
    let CommandMutation::PrepareTerminalAllocation {
        terminal_id,
        caller,
        preparation,
    } = &req.mutation
    else {
        if req.envelope.command_type == "AcquireTerminal" {
            return Err(invalid("terminal allocation mutation missing"));
        }
        return Ok(());
    };
    require_security_admin(caller, "AcquireTerminal")?;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = preparation.scope;
    if caller.actor_id != req.actor_id
        || req
            .envelope
            .actor_ref
            .as_ref()
            .is_some_and(|a| a != req.actor_id.as_str())
        || req.envelope.command_type != "AcquireTerminal"
        || req.target_aggregate != AggregateKind::TerminalAllocation
        || req.envelope.aggregate_id != terminal_id.as_uuid()
        || terminal_id.as_uuid().is_nil()
        || req.envelope.expected_revision != Revision::INITIAL
        || req.envelope.project_id != Some(project_id)
        || !req.events.is_empty()
        || !req.capacity_reservations.is_empty()
        || !req.budget_reservations.is_empty()
        || req.fence_assertion.is_some()
        || serde_json::to_value(&req.envelope.payload)? != preparation.command_payload()
    {
        return Err(invalid(
            "terminal allocation envelope differs from resolved request",
        ));
    }
    if read_daemon_state_conn(tx)?.schema_version < 10 {
        return Err(invalid("terminal allocation requires V010"));
    }
    let digest = canonical_sha256(preparation)?.to_string();
    let prior:Option<(String,String)>=tx.query_row("SELECT t.terminal_id,t.preparation_digest FROM terminal_allocations t JOIN command_receipts c ON c.receipt_id=t.receipt_id WHERE c.actor_id=?1 AND c.command_id=?2",params![req.actor_id.as_str(),req.envelope.command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((id, old)) = prior
        && (id != terminal_id.to_string() || old != digest)
    {
        return Err(StoreError::IdempotencyConflict {
            command_id: req.envelope.command_id,
        });
    }
    Ok(())
}
fn policy(
    tx: &Transaction<'_>,
    project: ProjectId,
) -> Result<(Revision, Sha256Digest), StoreError> {
    let s = read_policy_summary_conn(tx, Some(project))?;
    let rev = s
        .global_policy
        .as_ref()
        .map_or(Revision::INITIAL, |p| p.revision)
        .max(
            s.project_policy
                .as_ref()
                .map_or(Revision::INITIAL, |p| p.revision),
        );
    Ok((
        rev,
        canonical_sha256(
            &json!({"schema_version":1,"global_policy":s.global_policy,"project_policy":s.project_policy,"guard_config":s.guard_config}),
        )?,
    ))
}
fn validate_preparation(
    tx: &Transaction<'_>,
    now: TimestampMs,
    prep: &TerminalAllocationPreparation,
    actor: &ActorId,
    principal: &ActorId,
) -> Result<(), StoreError> {
    use std::path::{Component, Path};
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = prep.scope;
    let (root,current,owner,mission_revision,state):(Option<String>,i64,String,i64,String)=tx.query_row("SELECT p.primary_root_path,p.revision,m.owner_user_id,m.revision,m.state FROM missions m JOIN projects p ON p.project_id=m.project_id WHERE m.mission_id=?1 AND m.project_id=?2 AND p.state='active' AND m.owner_kind='user'",params![prep.mission_id.to_string(),project_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or_else(||invalid("terminal mission/project scope absent"))?;
    if current != integer(revision.as_u64())?
        || mission_revision != integer(prep.mission_revision.as_u64())?
        || owner != principal.as_str()
        || !["ready", "active"].contains(&state.as_str())
    {
        return Err(invalid(
            "terminal mission, owner or project revision changed",
        ));
    }
    ensure_scope_not_frozen_tx(tx, Some(project_id), None, None)?;
    for id in [principal.as_str(), actor.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, id, Some(project_id)).map_err(
            |e| match e {
                StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
                    reason,
                    message: "terminal subject suspended or revoked".to_owned(),
                },
                other => other,
            },
        )?;
    }
    let s = &prep.process_spec;
    // Every new preparation/launch re-reads configured authority under the
    // writer transaction. Historical lookup, disposition and exact replay do
    // not call this guard and never manufacture a new environment grant.
    let settings = crate::projects_settings::read_settings_conn(
        tx,
        now,
        crate::SettingsScopeKind::User,
        "default",
    )?;
    let raw = settings
        .extra_typed
        .get("terminal_runtime")
        .ok_or_else(|| invalid("terminal runtime is not configured"))?;
    let runtime: TerminalRuntimeSettings = serde_json::from_value(raw.clone())?;
    runtime.validate()?;
    let profile = runtime
        .profiles
        .iter()
        .find(|p| p.profile_id == s.environment.profile_id && p.version == s.environment.version)
        .ok_or_else(|| invalid("terminal configured profile changed"))?;
    if profile.digest()? != s.environment.digest
        || runtime.pools() != prep.auxiliary
        || runtime.lease_ttl_ms != prep.lease_ttl_ms
    {
        return Err(invalid(
            "terminal configured scope, profile or pools changed",
        ));
    }
    let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_references WHERE owner_kind='terminal_environment' AND owner_id=?1 AND source_field=?2 AND object_hash=?3 AND state='live')",params![profile.profile_id,profile.version.to_string(),s.environment.digest.to_string()],|r|r.get(0))?;
    if !bound {
        return Err(invalid(
            "terminal configured profile has no durable reference",
        ));
    }
    let root = root.ok_or_else(|| invalid("terminal primary project root absent"))?;
    let relative = Path::new(&s.requested_cwd);
    if !Path::new(&root).is_absolute()
        || !Path::new(&s.program).is_absolute()
        || !Path::new(&s.cwd_canonical).is_absolute()
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || Path::new(&root).join(relative) != Path::new(&s.cwd_canonical)
        || [s.program.as_str(), s.cwd_canonical.as_str()]
            .iter()
            .any(|s| s.contains('\0'))
        || s.arguments.iter().any(|a| a.contains('\0'))
        || !(1..=500).contains(&s.rows)
        || !(1..=1000).contains(&s.cols)
        || s.readiness_timeout_ms == 0
        || i64::try_from(s.readiness_timeout_ms).is_err()
        || prep.lease_ttl_ms <= 0
        || i64::try_from(s.readiness_timeout_ms).is_ok_and(|ms| ms > prep.lease_ttl_ms)
        || !valid_ref(&prep.purpose_key)
        || !valid_ref(&s.environment.profile_id)
        || s.environment.version == 0
    {
        return Err(invalid("terminal normalized process specification invalid"));
    }
    let published: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
        [s.environment.digest.to_string()],
        |r| r.get(0),
    )?;
    if !published {
        return Err(invalid(
            "resolved terminal environment object is not published",
        ));
    }
    Ok(())
}
fn principal(tx: &Connection, actor: &ActorId) -> Result<ActorId, StoreError> {
    let id:Option<String>=tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND principal_kind='user' AND state='active'",[actor.as_str()],|r|r.get(0)).optional()?;
    Ok(ActorId::new(id.ok_or_else(|| {
        invalid("active human terminal principal absent")
    })?)?)
}
fn pool_bindings(p: &TerminalAuxiliaryPools) -> [(PoolId, &'static str); 3] {
    [
        (p.host, "process_host"),
        (p.child, "process"),
        (p.pty, "pty"),
    ]
}
fn validate_pools(tx: &Transaction<'_>, p: &TerminalAuxiliaryPools) -> Result<(), StoreError> {
    if p.host == p.child || p.host == p.pty || p.child == p.pty {
        return Err(invalid("terminal auxiliary roles must use distinct pools"));
    }
    for (id, kind) in pool_bindings(p) {
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_pools WHERE pool_id=?1 AND pool_kind=?2 AND scope_key='global' AND unit='slot' AND admissions_frozen=0 AND effective_limit<=hard_limit AND (SELECT COALESCE(SUM(r.amount),0) FROM capacity_reservations r WHERE r.pool_id=capacity_pools.pool_id AND r.state IN ('held','reconciling'))<=effective_limit)",params![id.to_string(),kind],|r|r.get(0))?;
        if !valid {
            return Err(invalid(
                "terminal auxiliary pool not configured with the expected role",
            ));
        }
    }
    Ok(())
}

fn available_property_key(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    terminal: TerminalId,
    prep: &TerminalAllocationPreparation,
) -> Result<String, StoreError> {
    let taken:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_allocations WHERE terminal_id=?1 OR (mission_id=?2 AND purpose_key=?3 AND state IN ('allocated','launch_resolved'))) OR EXISTS(SELECT 1 FROM terminal_sessions s WHERE (s.terminal_id=?1 OR (s.mission_id=?2 AND s.purpose_key=?3)) AND NOT EXISTS(SELECT 1 FROM terminal_allocations old JOIN processes p ON p.process_id=s.process_id JOIN actions a ON a.action_id=old.action_id WHERE old.terminal_id=s.terminal_id AND old.state='retired' AND p.process_id=old.process_id AND p.process_run_uuid=old.process_run_uuid AND p.action_id=old.action_id AND p.authority_generation=a.authority_generation AND p.authority_generation=json_extract(old.context_json,'$.authority_generation') AND a.daemon_epoch=json_extract(old.context_json,'$.daemon_epoch') AND a.fence=0 AND a.origin_kind='control_plane' AND a.control_principal_id=old.principal_id AND s.mission_id=old.mission_id AND s.purpose_key=old.purpose_key AND s.lease_id=old.lease_id AND s.owner_agent_id IS NULL AND s.state='exited' AND p.state='exited')) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)",params![terminal.to_string(),prep.mission_id.to_string(),prep.purpose_key],|r|r.get(0))?;
    if taken {
        return Err(StoreError::CapacityBlocked(
            "terminal property already reserved; reconciliation or registered acquisition required"
                .to_owned(),
        ));
    }
    let resource_key = key(prep)?;
    let last:Option<i64>=tx.query_row("SELECT last_fence FROM resource_fences WHERE authority_generation=?1 AND resource_kind='terminal' AND resource_key=?2",params![daemon.authority_generation.to_string(),resource_key],|r|r.get(0)).optional()?;
    if last == Some(i64::MAX) {
        return Err(stale("terminal resource fence exhausted"));
    }
    Ok(resource_key)
}
fn ensure_no_registered_resource(
    tx: &Transaction<'_>,
    c: &TerminalAllocationContext,
) -> Result<(), StoreError> {
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_sessions s WHERE (s.terminal_id=?1 OR (s.mission_id=?2 AND s.purpose_key=?3)) AND NOT EXISTS(SELECT 1 FROM terminal_allocations old JOIN processes p ON p.process_id=s.process_id JOIN actions a ON a.action_id=old.action_id WHERE old.terminal_id=s.terminal_id AND old.state='retired' AND p.process_id=old.process_id AND p.process_run_uuid=old.process_run_uuid AND p.action_id=old.action_id AND p.authority_generation=a.authority_generation AND p.authority_generation=json_extract(old.context_json,'$.authority_generation') AND a.daemon_epoch=json_extract(old.context_json,'$.daemon_epoch') AND a.fence=0 AND a.origin_kind='control_plane' AND a.control_principal_id=old.principal_id AND s.mission_id=old.mission_id AND s.purpose_key=old.purpose_key AND s.lease_id=old.lease_id AND s.owner_agent_id IS NULL AND s.state='exited' AND p.state='exited')) OR EXISTS(SELECT 1 FROM processes WHERE process_id=?4 OR process_run_uuid=?5)",params![c.terminal_id.to_string(),c.preparation.mission_id.to_string(),c.preparation.purpose_key,c.process_id.to_string(),c.process_run_uuid.to_string()],|r|r.get(0))?;
    if exists {
        return Err(invalid(
            "registered terminal/process requires reconciliation before launch",
        ));
    }
    Ok(())
}
fn reserve_terminal_capacity_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    terminal: TerminalId,
    process_id: ProcessId,
    pools: &TerminalAuxiliaryPools,
) -> Result<[ReservationId; 3], StoreError> {
    let ids = [
        ReservationId::from_uuid(config.id_generator.new_uuid()),
        ReservationId::from_uuid(config.id_generator.new_uuid()),
        ReservationId::from_uuid(config.id_generator.new_uuid()),
    ];
    for ((pool, kind), id) in pool_bindings(pools).into_iter().zip(ids) {
        let (owner_type, owner_id) = if kind == "pty" {
            ("terminal", terminal.to_string())
        } else {
            ("process", process_id.to_string())
        };
        crate::budgets::tx_reserve_capacity(
            tx,
            daemon.authority_generation,
            daemon.daemon_epoch,
            now,
            &CapacityReservationRequest {
                reservation_id: id,
                pool_id: pool,
                execution_owner: None,
                owner_type: owner_type.to_owned(),
                owner_id,
                admission_class: None,
                amount: 1,
            },
        )?;
    }
    Ok(ids)
}
#[expect(
    clippy::too_many_arguments,
    reason = "Explicit identities and authenticated scope of one atomic preparation"
)]
pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    terminal: TerminalId,
    receipt: ReceiptId,
    actor: &ActorId,
    project: Option<ProjectId>,
    prep: &TerminalAllocationPreparation,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != unleash_domain::DaemonMode::Ready
        || config
            .clock_uncertain
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid(
            "terminal preparation requires ready daemon and certain clock",
        ));
    }
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = prep.scope;
    if project != Some(project_id) {
        return Err(invalid("terminal project differs from mission scope"));
    }
    let principal = principal(tx, actor)?;
    validate_preparation(tx, now, prep, actor, &principal)?;
    validate_pools(tx, &prep.auxiliary)?;
    let resource_key = available_property_key(tx, daemon, terminal, prep)?;
    now.as_i64()
        .checked_add(prep.lease_ttl_ms)
        .ok_or_else(|| invalid("terminal lease deadline overflow"))?;
    let action_id = ActionId::from_uuid(config.id_generator.new_uuid());
    let process_id = ProcessId::from_uuid(config.id_generator.new_uuid());
    let process_run_uuid = ProcessRunUuid::from_uuid(config.id_generator.new_uuid());
    let host_id = HostId::from_uuid(config.id_generator.new_uuid());
    let lease_id = LeaseId::from_uuid(config.id_generator.new_uuid());
    let lease = tx_acquire_lease(
        tx,
        &LeaseLineageCtx {
            authority: daemon.authority_generation,
            daemon_epoch: daemon.daemon_epoch,
            boot_identity: &daemon.boot_identity,
            now_ms: now,
        },
        lease_id,
        &crate::AcquireLeaseRequest {
            resource_kind: "terminal".to_owned(),
            resource_key,
            owner_type: "principal".to_owned(),
            owner_id: principal.to_string(),
            ttl_ms: prep.lease_ttl_ms,
        },
    )?;
    let [
        host_reservation_id,
        child_reservation_id,
        pty_reservation_id,
    ] = reserve_terminal_capacity_tx(
        tx,
        config,
        daemon,
        now,
        terminal,
        process_id,
        &prep.auxiliary,
    )?;
    let (policy_revision, security_policy_digest) = policy(tx, project_id)?;
    let context = TerminalAllocationContext {
        schema_version: 1,
        terminal_id: terminal,
        process_id,
        process_run_uuid,
        host_id,
        action_id,
        receipt_id: receipt,
        actor_id: actor.clone(),
        principal_id: principal,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        daemon_boot_identity: daemon.boot_identity.clone(),
        lease_id,
        resource_fence: lease.fence,
        lease_expires_at: lease.expires_at_ms,
        host_reservation_id,
        child_reservation_id,
        pty_reservation_id,
        preparation: prep.clone(),
        policy_revision,
        security_policy_digest,
    };
    let digest = canonical_sha256(&context)?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO terminal_allocations(terminal_id,receipt_id,action_id,process_id,process_run_uuid,principal_id,mission_id,project_id,purpose_key,lease_id,preparation_digest,context_digest,context_json,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'allocated',1)",params![terminal.to_string(),receipt.to_string(),action_id.to_string(),process_id.to_string(),process_run_uuid.to_string(),context.principal_id.as_str(),prep.mission_id.to_string(),project_id.to_string(),prep.purpose_key,lease_id.to_string(),canonical_sha256(prep)?.to_string(),digest.to_string(),serde_json::to_string(&context)?])?;
    events.push(EventAppendSpec{event_type:"TerminalAcquired".to_owned(),aggregate_type:AggregateKind::TerminalAllocation,aggregate_id:terminal.as_uuid(),aggregate_revision:Revision::new(1),project_id:Some(project_id),correlation_id:terminal.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,payload:json!({"terminal_id":terminal,"process_id":process_id,"process_run_uuid":process_run_uuid,"action_id":action_id,"lease_id":lease_id,"resource_fence":context.resource_fence,"context_digest":digest,"state":"preparing","reused":false,"discovery":{"candidates":[],"decision":"new","reason":"no_registered_resource"}}).as_object().cloned().ok_or_else(||invalid("invalid terminal event"))?,sensitivity:Sensitivity::Project,outbox_destinations:vec!["clients".to_owned(),"harness".to_owned()]});
    Ok(Revision::new(1))
}

struct AllocationSqlRow {
    json: String,
    digest: String,
    state: String,
    revision: i64,
    dispatch: Option<String>,
    action_state: Option<String>,
    retirement_evidence_id: Option<String>,
}

pub(crate) fn read_conn(
    conn: &Connection,
    terminal: TerminalId,
) -> Result<Option<TerminalAllocationRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 10 {
        return Ok(None);
    }
    let row:Option<AllocationSqlRow>=conn.query_row("SELECT t.context_json,t.context_digest,t.state,t.revision,t.dispatch_id,a.state,t.retirement_evidence_id FROM terminal_allocations t LEFT JOIN actions a ON a.action_id=t.action_id WHERE t.terminal_id=?1",[terminal.to_string()],|r|Ok(AllocationSqlRow{json:r.get(0)?,digest:r.get(1)?,state:r.get(2)?,revision:r.get(3)?,dispatch:r.get(4)?,action_state:r.get(5)?,retirement_evidence_id:r.get(6)?})).optional()?;
    row.map(|row| {
        let context: TerminalAllocationContext = serde_json::from_str(&row.json)?;
        let context_digest: Sha256Digest = row.digest.parse()?;
        if context.schema_version != 1
            || context.terminal_id != terminal
            || canonical_sha256(&context)? != context_digest
        {
            return Err(invalid("terminal context integrity mismatch"));
        }
        Ok(TerminalAllocationRecord {
            retirement_evidence_id: row.retirement_evidence_id,
            context,
            context_digest,
            state: match row.state.as_str() {
                "allocated" => TerminalAllocationState::Allocated,
                "launch_resolved" => TerminalAllocationState::LaunchResolved,
                "retired" => TerminalAllocationState::Retired,
                _ => return Err(invalid("terminal allocation state invalid")),
            },
            revision: Revision::new(
                u64::try_from(row.revision).map_err(|_| invalid("terminal revision invalid"))?,
            ),
            dispatch_id: row.dispatch.map(|s| s.parse()).transpose()?,
            action_state: row
                .action_state
                .as_deref()
                .map(crate::recovery_ops::parse_action_state)
                .transpose()?,
        })
    })
    .transpose()
}
fn for_action(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<TerminalAllocationRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 10 {
        return Ok(None);
    }
    let terminal: Option<String> = conn
        .query_row(
            "SELECT terminal_id FROM terminal_allocations WHERE action_id=?1",
            [action.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    terminal
        .map(|s| read_conn(conn, s.parse()?))
        .transpose()
        .map(Option::flatten)
}
pub(crate) fn validate_incarnation(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    action: ActionId,
) -> Result<(), StoreError> {
    terminal_input_writer::validate_incarnation(tx, d, action)?;
    if let Some(record) = control_read_conn(tx, action)? {
        let c = &record.context;
        if c.authority_generation != d.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: d.authority_generation,
                actual: c.authority_generation,
                command_id: None,
            });
        }
        if c.daemon_epoch != d.daemon_epoch || c.daemon_boot_identity != d.boot_identity {
            return Err(stale("terminal control belongs to an old incarnation"));
        }
    }
    if let Some(record) = for_action(tx, action)? {
        let c = &record.context;
        if c.authority_generation != d.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: d.authority_generation,
                actual: c.authority_generation,
                command_id: None,
            });
        }
        if c.daemon_epoch != d.daemon_epoch || c.daemon_boot_identity != d.boot_identity {
            return Err(stale("terminal preparation belongs to an old incarnation"));
        }
    }
    Ok(())
}
fn validate_lease(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalAllocationContext,
) -> Result<(), StoreError> {
    let row:Option<(String,String,String,String,String,i64,i64,String,i64)>=tx.query_row("SELECT authority_generation,resource_kind,resource_key,owner_type,owner_id,daemon_epoch,fence,boot_identity,expires_at_ms FROM resource_leases WHERE lease_id=?1 AND state='held'",[c.lease_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let Some((auth, kind, resource, owner_type, owner, epoch, fence, boot, expiry)) = row else {
        return Err(stale("terminal resource lease is not held"));
    };
    if auth != d.authority_generation.to_string()
        || kind != "terminal"
        || resource != key(&c.preparation)?
        || owner_type != "principal"
        || owner != c.principal_id.as_str()
        || epoch != integer(d.daemon_epoch.as_u64())?
        || fence != integer(c.resource_fence.as_u64())?
        || boot != d.boot_identity
        || expiry <= now.as_i64()
        || c.lease_expires_at.as_i64() <= now.as_i64()
    {
        return Err(stale(
            "terminal resource lease target, incarnation, fence or TTL differs",
        ));
    }
    tx_validate_lease_fence(
        tx,
        d.authority_generation,
        d.daemon_epoch,
        &FenceAssertion {
            lease_id: c.lease_id,
            authority_generation: d.authority_generation,
            daemon_epoch: d.daemon_epoch,
            fence: c.resource_fence,
            owner_id: c.principal_id.to_string(),
        },
    )
}
fn validate_holds(tx: &Transaction<'_>, c: &TerminalAllocationContext) -> Result<(), StoreError> {
    validate_pools(tx, &c.preparation.auxiliary)?;
    for ((pool, kind), id) in pool_bindings(&c.preparation.auxiliary).into_iter().zip([
        c.host_reservation_id,
        c.child_reservation_id,
        c.pty_reservation_id,
    ]) {
        let (owner_type, owner) = if kind == "pty" {
            ("terminal", c.terminal_id.to_string())
        } else {
            ("process", c.process_id.to_string())
        };
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_reservations WHERE reservation_id=?1 AND pool_id=?2 AND owner_type=?3 AND owner_id=?4 AND amount=1 AND admission_class IS NULL AND state='held' AND authority_generation=?5 AND daemon_epoch=?6)",params![id.to_string(),pool.to_string(),owner_type,owner,c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?],|r|r.get(0))?;
        if !valid {
            return Err(invalid("terminal auxiliary hold differs from preparation"));
        }
    }
    Ok(())
}
fn validate_live(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    r: &TerminalAllocationRecord,
) -> Result<(), StoreError> {
    let c = &r.context;
    validate_incarnation(tx, d, c.action_id)?;
    if r.state == TerminalAllocationState::Retired {
        return Err(invalid(
            "terminal allocation has a qualified disposition; old generation cannot relaunch",
        ));
    }
    ensure_no_registered_resource(tx, c)?;
    if d.mode != unleash_domain::DaemonMode::Ready {
        return Err(invalid("terminal launch requires ready daemon"));
    }
    if principal(tx, &c.actor_id)? != c.principal_id {
        return Err(invalid("terminal principal changed"));
    }
    validate_preparation(tx, now, &c.preparation, &c.actor_id, &c.principal_id)?;
    validate_lease(tx, d, now, c)?;
    validate_holds(tx, c)?;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = c.preparation.scope;
    if policy(tx, project_id)? != (c.policy_revision, c.security_policy_digest) {
        return Err(invalid("terminal security policy changed"));
    }
    let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2 AND aggregate_id=?3 AND command_type='AcquireTerminal' AND authority_generation=?4 AND state='accepted' AND response_state='full')",params![c.receipt_id.to_string(),c.actor_id.as_str(),c.terminal_id.to_string(),d.authority_generation.to_string()],|r|r.get(0))?;
    if !receipt {
        return Err(invalid(
            "terminal receipt is not attributable to the preparation",
        ));
    }
    Ok(())
}
pub(crate) fn validate_intent(
    tx: &Transaction<'_>,
    d: &DaemonStateRow,
    now: TimestampMs,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    if terminal_renewal::validate_intent(tx, d, now, intent)? {
        return Ok(());
    }
    terminal_input_writer::validate_intent(tx, d, now, intent)?;
    if let Some(record) = control_read_conn(tx, intent.action_id)? {
        let c = &record.context;
        validate_control_live(tx, d, now, &record)?;
        if !matches!(
            record.state,
            TerminalControlState::Prepared | TerminalControlState::HandshakeResolved
        ) || intent.principal_id != c.principal_id
            || intent.cause
                != (BrokerCause::CommandReceipt {
                    receipt_id: c.receipt_id,
                })
            || intent.project_id != Some(c.project_id)
            || intent.scope_revision != c.scope_revision
            || intent.policy_revision != c.policy_revision
            || intent.payload_digest != record.context_digest
            || intent.action_deadline != c.expires_at_ms
            || intent.manifest.get("verb") != Some(&json!(TERMINAL_CONTROL_VERB))
            || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
            || intent.manifest.get("resource_refs")
                != Some(&json!([{"kind":"terminal","value":c.terminal_id}]))
            || intent.manifest.get("secret_refs") != Some(&json!([]))
            || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
        {
            return Err(invalid(
                "terminal control intent differs from accepted request",
            ));
        }
        return Ok(());
    }
    let record = for_action(tx, intent.action_id)?;
    let receipt_type: Option<String> =
        if let BrokerCause::CommandReceipt { receipt_id } = intent.cause {
            tx.query_row(
                "SELECT command_type FROM command_receipts WHERE receipt_id=?1",
                [receipt_id.to_string()],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
    if record.is_none()
        && receipt_type.as_deref() != Some("AcquireTerminal")
        && intent.manifest.get("verb") != Some(&json!(TERMINAL_ALLOCATION_VERB))
        && intent.manifest.get("verb") != Some(&json!(TERMINAL_CONTROL_VERB))
    {
        return Ok(());
    }
    let r = record.ok_or_else(|| invalid("terminal allocation preparation missing"))?;
    let c = &r.context;
    validate_live(tx, d, now, &r)?;
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    if intent.principal_id != c.principal_id
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: c.receipt_id,
            })
        || intent.project_id != Some(project_id)
        || intent.scope_revision != revision
        || intent.policy_revision != c.policy_revision
        || intent.payload_digest != r.payload_digest()
        || intent.action_deadline != c.lease_expires_at
        || intent.manifest.get("verb") != Some(&json!(TERMINAL_ALLOCATION_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("process_exec"))
        || intent.manifest.get("resource_refs") != Some(&r.resource_refs())
        || intent.manifest.get("secret_refs") != Some(&json!([]))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "terminal intent differs from authenticated allocation",
        ));
    }
    Ok(())
}
pub(crate) fn guard_observation(
    tx: &Transaction<'_>,
    phase: &crate::BrokerPhase,
    proof: Option<&TerminalObservationProof>,
) -> Result<(), StoreError> {
    terminal_renewal_observation::guard_observation(tx, phase, proof)?;
    terminal_input_writer::guard_observation(tx, phase, proof)?;
    if let crate::BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase
        && let Some(record) = control_read_conn(tx, *action_id)?
    {
        if receipt.state == ActionState::OutcomeUnknown {
            if record.state == TerminalControlState::HandshakeResolved {
                if record.dispatch_id != Some(receipt.dispatch_id) {
                    return Err(invalid("unknown terminal control dispatch differs"));
                }
                tx.execute("UPDATE terminal_control_intents SET state='outcome_unknown',revision=revision+1 WHERE action_id=?1 AND state='handshake_resolved' AND dispatch_id=?2",params![action_id.to_string(),receipt.dispatch_id.to_string()])?;
            }
        } else if !proof
            .is_some_and(|p| canonical_sha256(phase).is_ok_and(|digest| p.phase_digest == digest))
        {
            return Err(invalid(
                "terminal control observation producer not qualified",
            ));
        }
    }
    if let crate::BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase
        && for_action(tx, *action_id)?.is_some()
        && receipt.state != ActionState::OutcomeUnknown
        && !proof
            .is_some_and(|p| canonical_sha256(phase).is_ok_and(|digest| p.phase_digest == digest))
    {
        return Err(invalid("terminal OS observation producer not qualified"));
    }
    Ok(())
}

/// Exact non-secret facts captured from the live, authenticated host by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalLaunchObservation {
    /// Planned terminal identity.
    pub terminal_id: TerminalId,
    /// Exact action.
    pub action_id: ActionId,
    /// Unique durable dispatch.
    pub dispatch_id: DispatchId,
    /// Immutable preparation.
    pub context_digest: Sha256Digest,
    /// Planned host acknowledged by the real host.
    pub host_id: HostId,
    /// Physical process identity, never an adopted PID.
    pub process_run_uuid: ProcessRunUuid,
    /// Actual OS PID.
    pub pid: u32,
    /// Actual OS PGID.
    pub pgid: u32,
    /// Actual owner UID.
    pub owner_uid: u32,
    /// Actual boot identity.
    pub boot_identity: String,
    /// Actual start identity.
    pub start_identity: String,
    /// Qualified identity backend.
    pub identity_backend: String,
    /// Digest of the protected host manifest actually inspected.
    pub manifest_digest: Sha256Digest,
    /// Effective environment attested by the host.
    pub effective_environment_digest: Sha256Digest,
    /// Last durable output watermark.
    pub durable_log_seq: u64,
    /// Native facts captured with this observation. Older objects omit the field
    /// and preserve their original digest; absence is never inferred as zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_state: Option<terminal_acquisition_query::TerminalObservedInputState>,
    /// Time the authenticated host facts were captured, before proof publication.
    pub observed_at_ms: TimestampMs,
    /// Exit code, only when observed.
    pub exit_code: Option<i32>,
    /// Exit signal, only when observed.
    pub exit_signal: Option<i32>,
    /// Whether completion has been observed by the owning host.
    pub exited: bool,
}

pub(crate) struct TerminalObservationProof {
    phase_digest: Sha256Digest,
}

impl crate::UnleashStore {
    /// Commit actual process/PTY facts and the exact broker observation atomically.
    /// # Errors
    /// Refuses unbound observations, another incarnation, missing published evidence or replay divergence.
    pub fn commit_terminal_launch_observation(
        &self,
        observation: TerminalLaunchObservation,
        phase: crate::BrokerPhaseRequest,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        let cfg = self.config().clone();
        let committed=self.execute_in_writer_tx(crate::WriteCriticality::Normal,move|tx,now| {
            let d=read_daemon_state_conn(tx)?;
            let r=read_conn(tx,observation.terminal_id)?.ok_or_else(||invalid("terminal observation has no allocation"))?;
            let c=&r.context;
            let digest=canonical_sha256(&observation)?;
            if observation.input_state.as_ref().is_some_and(|state|
                state.last_confirmed_input_seq != 0 || !state.unknown_input_seqs.is_empty()) {
                return Err(invalid("new terminal input state differs from its native initial observation"));
            }
            let crate::BrokerPhase::Observation {action_id,receipt,..}=&phase.phase else {return Err(invalid("terminal observation requires observation phase"));};
            if *action_id!=c.action_id || observation.action_id!=c.action_id || observation.host_id!=c.host_id
                || observation.process_run_uuid!=c.process_run_uuid || observation.context_digest!=r.context_digest
                || r.dispatch_id!=Some(observation.dispatch_id) || receipt.dispatch_id!=observation.dispatch_id
                || receipt.output_digest!=Some(digest) || receipt.state!=ActionState::ObservedSuccess
                || observation.owner_uid!=d.owner_uid
                || observation.pid==0 || observation.pgid==0 || observation.boot_identity!=d.boot_identity
                || observation.start_identity.trim().is_empty() || !["macos_proc_pidinfo","linux_procfs"].contains(&observation.identity_backend.as_str())
                || (!observation.exited && (observation.exit_code.is_some() || observation.exit_signal.is_some()))
                || (observation.exit_code.is_some() && observation.exit_signal.is_some()) {
                return Err(invalid("terminal observation differs from its actual allocation"));
            }
            let existing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM processes WHERE process_id=?1)",[c.process_id.to_string()],|row|row.get(0))?;
            if existing {return crate::broker_writer::commit_phase_tx(tx,&cfg,now,&phase);}
            validate_incarnation(tx,&d,c.action_id)?;
            validate_lease(tx,&d,now,c)?;
            validate_holds(tx,c)?;
            crate::broker_writer::validate_execution_dispatch_tx(tx,&d,now,c.action_id,observation.dispatch_id)?;
            let started_at_ms:i64=tx.query_row("SELECT started_at_ms FROM action_dispatches WHERE dispatch_id=?1 AND action_id=?2",params![observation.dispatch_id.to_string(),c.action_id.to_string()],|row|row.get(0))?;
            // Facts are captured before publication/fsync; the broker receipt is
            // created only after the executor returns. Preserve both times.
            // The generic broker writer also checks receipt <= writer_now.
            if observation.observed_at_ms.as_i64()<started_at_ms || observation.observed_at_ms>receipt.observed_at_ms {
                return Err(invalid("terminal capture timestamp is outside dispatch and receipt"));
            }
            let evidence:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM object_references f JOIN object_records o ON o.object_hash=f.object_hash WHERE f.owner_kind='terminal_launch_observation' AND f.owner_id=?1 AND f.source_field=?2 AND f.object_hash=?3 AND f.state='live' AND o.state='live')",params![c.terminal_id.to_string(),observation.dispatch_id.to_string(),digest.to_string()],|row|row.get(0))?;
            if !evidence {return Err(invalid("actual terminal observation object is not published"));}
            let process_state=if observation.exited {"exited"}else{"live"};
            let terminal_state=if observation.exited {"exited"}else{"ready"};
            tx.execute("INSERT INTO processes(process_id,action_id,process_run_uuid,authority_generation,host_id,pid,pgid,boot_identity,start_identity,identity_backend,owner_uid,cwd_ref,worktree_id,state,last_observed_at_ms,exit_code,exit_signal) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,?13,?14,?15,?16)",params![c.process_id.to_string(),c.action_id.to_string(),c.process_run_uuid.to_string(),c.authority_generation.to_string(),c.host_id.to_string(),observation.pid,observation.pgid,observation.boot_identity,observation.start_identity,observation.identity_backend,observation.owner_uid,c.preparation.process_spec.cwd_canonical,process_state,observation.observed_at_ms.as_i64(),observation.exit_code,observation.exit_signal])?;
            tx.execute("INSERT INTO terminal_sessions(terminal_id,process_id,owner_agent_id,mission_id,purpose_key,rows,cols,lease_id,input_seq,durable_log_seq,state) VALUES(?1,?2,NULL,?3,?4,?5,?6,?7,0,?8,?9)",params![c.terminal_id.to_string(),c.process_id.to_string(),c.preparation.mission_id.to_string(),c.preparation.purpose_key,c.preparation.process_spec.rows,c.preparation.process_spec.cols,c.lease_id.to_string(),integer(observation.durable_log_seq)?,terminal_state])?;
            let proof=TerminalObservationProof {phase_digest:canonical_sha256(&phase.phase)?};
            crate::broker_writer::commit_terminal_observation_tx(tx,&cfg,now,&phase,&proof)
        })?;
        if !committed.replayed {
            self.config()
                .cutpoints
                .hit(crate::StoreCutpoint::C2AfterCommandCommit)?;
        }
        Ok(committed)
    }
}
pub(crate) fn resolve_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    request: &ResolveTerminalLaunchRequest,
) -> Result<ResolvedTerminalLaunch, StoreError> {
    let d = read_daemon_state_conn(tx)?;
    if request.authority_generation != d.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: d.authority_generation,
            actual: request.authority_generation,
            command_id: None,
        });
    }
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(invalid("terminal lease clock uncertain"));
    }
    require_security_admin(&request.caller, "AcquireTerminal")?;
    let record = read_conn(tx, request.terminal_id)?
        .ok_or_else(|| invalid("terminal preparation absent"))?;
    let c = &record.context;
    if record.state != TerminalAllocationState::Allocated || record.dispatch_id.is_some() {
        return Err(invalid(
            "terminal launch already resolved; inspection and reconciliation required",
        ));
    }
    if record.revision != request.expected_revision {
        return Err(StoreError::RevisionConflict {
            current_revision: record.revision,
            expected_revision: request.expected_revision,
            command_id: None,
        });
    }
    if request.caller.actor_id != c.actor_id
        || request.action_id != c.action_id
        || request.process_run_uuid != c.process_run_uuid
        || request.context_digest != record.context_digest
        || request.payload_digest != record.payload_digest()
    {
        return Err(invalid("terminal resolution differs from preparation"));
    }
    validate_live(tx, &d, now, &record)?;
    if request.lease.lease_id != c.lease_id
        || request.lease.owner_id != c.principal_id.as_str()
        || request.lease.authority_generation != c.authority_generation
        || request.lease.daemon_epoch != c.daemon_epoch
        || request.lease.fence != c.resource_fence
    {
        return Err(stale("terminal resolution resource assertion differs"));
    }
    crate::broker_writer::validate_execution_dispatch_tx(
        tx,
        &d,
        now,
        c.action_id,
        request.dispatch_id,
    )?;
    let changed=tx.execute("UPDATE terminal_allocations SET state='launch_resolved',revision=revision+1,dispatch_id=?2,resolved_at_ms=?3 WHERE terminal_id=?1 AND state='allocated' AND revision=?4 AND dispatch_id IS NULL",params![c.terminal_id.to_string(),request.dispatch_id.to_string(),now.as_i64(),integer(request.expected_revision.as_u64())?])?;
    if changed != 1 {
        return Err(invalid("terminal allocation resolution race"));
    }
    append_resolution_event(tx, config, &d, now, c, request.dispatch_id)?;
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeCommandCommit)?;
    Ok(ResolvedTerminalLaunch {
        allocation: read_conn(tx, c.terminal_id)?
            .ok_or_else(|| invalid("terminal preparation disappeared"))?,
    })
}
fn append_resolution_event(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    d: &DaemonStateRow,
    now: TimestampMs,
    c: &TerminalAllocationContext,
    dispatch: DispatchId,
) -> Result<(), StoreError> {
    let event = config.id_generator.new_uuid().to_string();
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = c.preparation.scope;
    let payload = json!({"terminal_id":c.terminal_id,"process_id":c.process_id,"process_run_uuid":c.process_run_uuid,"action_id":c.action_id,"dispatch_id":dispatch,"lease_id":c.lease_id,"resource_fence":c.resource_fence,"state":"preparing","os_observation":null});
    tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'terminal_allocation',?3,2,0,'ProcessLaunchPrepared',1,?4,?5,?5,?6,?6,?7,'full',?8,'restricted','command',?9,?9,?3)",params![event,d.stream_id.to_string(),c.terminal_id.to_string(),project_id.to_string(),c.actor_id.as_str(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.to_string(),c.receipt_id.to_string()])?;
    tx.execute("INSERT INTO outbox(event_id,destination,state,delivery_attempt,next_attempt_at_ms,authority_generation,consumer_version,delivery_key,rearm_count) VALUES(?1,'clients','pending',0,?2,?3,?4,?5,0)",params![event,now.as_i64(),d.authority_generation.to_string(),config.consumer_version,format!("{event}:clients")])?;
    Ok(())
}

/// Bounded `SQLite` projection for the daemon snapshot, without host contact or lease renewal.
mod metadata_projection {
    use crate::StoreError;
    use serde::Serialize;
    use std::collections::HashSet;
    use unleash_domain::{ActorId, ProcessRunUuid, ProjectId, Revision, TerminalId};

    const PAGE_LIMIT: usize = 256;

    /// Bounded terminal-control metadata in the caller snapshot, without capabilities.
    #[derive(Serialize)]
    pub struct TerminalMetadataPage {
        /// Actor/project-filtered generations eligible for inspection.
        pub items: Vec<TerminalMetadata>,
        /// More eligible generations existed beyond the bound.
        pub truncated: bool,
    }

    /// One registered generation; no usable lease ID, capability or fence is exposed.
    #[derive(Serialize)]
    pub struct TerminalMetadata {
        terminal_id: TerminalId,
        process_run_uuid: ProcessRunUuid,
        project_id: ProjectId,
        control_revision: Revision,
        terminal_state: String,
        process_state: String,
        lease_state: String,
        control_state: Option<String>,
    }

    impl TerminalMetadata {
        pub(crate) fn allows_control_result(
            &self,
            terminal: TerminalId,
            run: ProcessRunUuid,
        ) -> bool {
            self.terminal_id == terminal
                && self.process_run_uuid == run
                && self.process_state == "live"
                && ["ready", "busy", "detached"].contains(&self.terminal_state.as_str())
                && self.lease_state == "held"
                && self.control_state.is_none()
        }
    }

    /// Project persisted control facts within the caller existing read transaction.
    /// # Errors
    /// Rejects malformed identities, context digests, watermarks or pending dispatch bindings.
    pub fn read_terminal_metadata<S: std::hash::BuildHasher>(
        conn: &rusqlite::Connection,
        actor: &ActorId,
        project_filter: Option<ProjectId>,
        allowed_projects: Option<&HashSet<ProjectId, S>>,
    ) -> Result<TerminalMetadataPage, StoreError> {
        if super::read_daemon_state_conn(conn)?.schema_version < 12 {
            return Ok(TerminalMetadataPage {
                items: vec![],
                truncated: false,
            });
        }
        let allowed = allowed_projects
            .map(|projects| {
                serde_json::to_string(&projects.iter().map(ToString::to_string).collect::<Vec<_>>())
            })
            .transpose()?;
        let project = project_filter.map(|project| project.to_string());
        // Filtering precedes LIMIT so neither rows nor the truncation bit disclose
        // another actor/project. All values are bound parameters, never SQL text.
        let mut statement=conn.prepare("SELECT s.terminal_id,p.process_run_uuid,a.project_id,s.control_revision,s.state,p.state,l.state,control.action_id
        FROM terminal_sessions s
        JOIN processes p ON p.process_id=s.process_id
        JOIN terminal_allocations a ON a.terminal_id=s.terminal_id AND a.process_id=p.process_id AND a.process_run_uuid=p.process_run_uuid AND a.action_id=p.action_id
        JOIN actions launch ON launch.action_id=a.action_id AND launch.origin_kind='control_plane' AND launch.control_principal_id=a.principal_id AND launch.action_type='terminal.acquire' AND launch.state='observed_success' AND launch.authority_generation=json_extract(a.context_json,'$.authority_generation') AND launch.payload_hash=a.context_digest
        JOIN principals owner ON owner.principal_id=a.principal_id
        JOIN resource_leases l ON l.lease_id=s.lease_id
        JOIN resource_leases initial_lease ON initial_lease.lease_id=a.lease_id AND initial_lease.resource_kind='terminal'
        JOIN daemon_state d ON d.singleton=1
        LEFT JOIN terminal_control_intents control ON control.terminal_id=s.terminal_id AND control.previous_lease_id=s.lease_id AND control.authority_generation=d.authority_generation AND control.actor_id=owner.actor_id AND control.principal_id=a.principal_id AND json_extract(control.context_json,'$.initial_action_id')=a.action_id AND json_extract(control.context_json,'$.initial_context_digest')=a.context_digest AND json_extract(control.context_json,'$.previous_control_revision')=s.control_revision AND control.state IN ('prepared','handshake_resolved','outcome_unknown')
        LEFT JOIN broker_dispatches transfer ON transfer.action_id=control.action_id AND (control.dispatch_id IS NULL OR transfer.dispatch_id=control.dispatch_id) AND transfer.authority_generation=control.authority_generation AND transfer.daemon_epoch=control.daemon_epoch
        LEFT JOIN resource_leases pending_lease ON pending_lease.lease_id=control.next_lease_id AND pending_lease.authority_generation=d.authority_generation AND pending_lease.boot_identity=d.boot_identity AND pending_lease.resource_kind='terminal' AND pending_lease.resource_key=initial_lease.resource_key AND pending_lease.owner_type='principal' AND pending_lease.owner_id=a.principal_id AND pending_lease.daemon_epoch=control.daemon_epoch AND pending_lease.fence=control.next_fence AND pending_lease.state IN ('reconciling','suspect')
        JOIN capacity_reservations cap ON cap.reservation_id=json_extract(a.context_json,'$.pty_reservation_id') AND cap.owner_type='terminal' AND cap.owner_id=s.terminal_id AND cap.amount=1
        JOIN capacity_pools pool ON pool.pool_id=cap.pool_id AND pool.pool_id=?4 AND pool.pool_kind='pty' AND pool.scope_key='global' AND pool.hard_limit<=50
        WHERE p.authority_generation=d.authority_generation AND p.boot_identity=d.boot_identity AND p.owner_uid=d.owner_uid
        AND p.pid>0 AND p.pgid>0 AND length(trim(p.start_identity))>0 AND p.identity_backend IN ('macos_proc_pidinfo','linux_procfs')
        AND l.authority_generation=d.authority_generation AND l.boot_identity=d.boot_identity AND l.resource_kind='terminal' AND l.resource_key=initial_lease.resource_key
        AND l.owner_type='principal' AND l.owner_id=a.principal_id AND (l.state IN ('held','suspect','reconciling') OR (l.state='revoked' AND control.state IN ('handshake_resolved','outcome_unknown') AND transfer.dispatch_id IS NOT NULL AND pending_lease.lease_id IS NOT NULL))
        AND p.state IN ('live','suspect') AND s.state IN ('ready','busy','detached','suspect')
        AND cap.authority_generation=d.authority_generation AND cap.state IN ('held','reconciling')
        AND owner.actor_id=?1 AND owner.principal_kind='user' AND owner.state='active' AND json_extract(a.context_json,'$.actor_id')=?1
        AND (?2 IS NULL OR a.project_id=?2) AND (?3 IS NULL OR a.project_id IN (SELECT value FROM json_each(?3)))
        ORDER BY s.terminal_id LIMIT 257")?;
        let rows = statement.query_map(
            rusqlite::params![
                actor.as_str(),
                project,
                allowed,
                super::TERMINAL_PTY_POOL_ID.to_string()
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )?;
        let mut items = Vec::new();
        for row in rows {
            let (
                terminal,
                run,
                project,
                revision,
                terminal_state,
                process_state,
                lease_state,
                control_action,
            ) = row?;
            let invalid = || {
                StoreError::InvariantViolation(
                    "invalid terminal metadata identity or revision".into(),
                )
            };
            items.push(TerminalMetadata {
                terminal_id: terminal.parse().map_err(|_| invalid())?,
                process_run_uuid: run.parse().map_err(|_| invalid())?,
                project_id: project.parse().map_err(|_| invalid())?,
                control_revision: Revision::new(u64::try_from(revision).map_err(|_| invalid())?),
                terminal_state,
                process_state,
                lease_state,
                control_state: pending_state(conn, control_action)?,
            });
        }
        let truncated = items.len() > PAGE_LIMIT;
        items.truncate(PAGE_LIMIT);
        Ok(TerminalMetadataPage { items, truncated })
    }

    fn pending_state(
        conn: &rusqlite::Connection,
        action: Option<String>,
    ) -> Result<Option<String>, StoreError> {
        let Some(action) = action else {
            return Ok(None);
        };
        let record = super::control_read_conn(conn, action.parse()?)?.ok_or_else(|| {
            StoreError::InvariantViolation("terminal control projection has no intention".into())
        })?;
        let Some(dispatch) = record.broker_dispatch_id else {
            if record.state != super::TerminalControlState::Prepared || record.dispatch_id.is_some()
            {
                return Err(StoreError::InvariantViolation(
                    "terminal control projection has no broker dispatch".into(),
                ));
            }
            return Ok(Some("prepared".into()));
        };
        let c = &record.context;
        let attributable:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM action_dispatches marker JOIN broker_dispatches binding ON binding.dispatch_id=marker.dispatch_id JOIN actions action ON action.action_id=marker.action_id WHERE marker.dispatch_id=?1 AND marker.action_id=?2 AND binding.action_id=?2 AND binding.authority_generation=?3 AND binding.daemon_epoch=?4 AND action.action_type='terminal.acquire_registered' AND action.payload_hash=?5 AND action.control_principal_id=?6 AND (SELECT COUNT(*) FROM action_dispatches WHERE action_id=?2)=1)",rusqlite::params![dispatch.to_string(),c.action_id.to_string(),c.authority_generation.to_string(),i64::try_from(c.daemon_epoch.as_u64()).map_err(|_|StoreError::InvariantViolation("terminal control epoch invalid".into()))?,record.context_digest.to_string(),c.principal_id.as_str()],|row|row.get(0))?;
        if !attributable || record.dispatch_id.is_some_and(|id| id != dispatch) {
            return Err(StoreError::InvariantViolation(
                "terminal control broker marker differs".into(),
            ));
        }
        if record.broker_action_state == Some(unleash_domain::ActionState::OutcomeUnknown) {
            return Ok(Some("outcome_unknown".into()));
        }
        let state = match record.state {
            super::TerminalControlState::Prepared => "dispatch_committed",
            super::TerminalControlState::HandshakeResolved => "handshake_resolved",
            super::TerminalControlState::OutcomeUnknown => {
                return Err(StoreError::InvariantViolation(
                    "terminal unknown differs from broker state".into(),
                ));
            }
            _ => {
                return Err(StoreError::InvariantViolation(
                    "terminal pending projection state invalid".into(),
                ));
            }
        };
        Ok(Some(state.into()))
    }
}

pub use metadata_projection::{TerminalMetadata, TerminalMetadataPage, read_terminal_metadata};
