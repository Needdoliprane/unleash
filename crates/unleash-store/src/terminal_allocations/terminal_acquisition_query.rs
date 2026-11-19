//! Causal inspection of an observed acquisition. This module never contacts a
//! host, renews a lease, dispatches an action or infers a native input receipt.

use std::sync::atomic::Ordering;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, ActorId, AuthorityGeneration, CommandId, DaemonEpoch, DaemonMode,
    DispatchId, Fence, LeaseId, ProcessId, ProcessRunUuid, ProjectId, ReceiptId, Revision,
    Sequence, Sha256Digest, TerminalId, TimestampMs,
};
use unleash_protocol::{EventCursor, LineageStatus, ReceiptState, canonical_sha256};

use crate::broker_phases::{BrokerEvaluation, BrokerPhase};
use crate::commands::{read_command_receipt_inspection, read_daemon_state_conn};
use crate::terminal_allocations::{
    self, TerminalAllocationRecord, TerminalAllocationScope, TerminalControlRecord,
    TerminalControlState, TerminalIdentityBackend, TerminalLaunchObservation,
    TerminalProcessIdentity,
};
use crate::{ReceiptInspectionOutcome, SnapshotProjection, StoreConfig, StoreError, UnleashStore};

const MAX_EXACT_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

/// Facts captured from the native host, before publishing a launch observation.
/// Absence in an older proof means unknown; it never means sequence zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalObservedInputState {
    /// Last input with an actual native receipt, rather than the next free slot.
    pub last_confirmed_input_seq: u64,
    /// Native dispatched writes with no receipt. Such a state grants no input capability.
    pub unknown_input_seqs: Vec<u64>,
}

/// States for which the exact observed owner can currently use control assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalAcquisitionState {
    /// Observed terminal available for input.
    Ready,
    /// Observed terminal currently busy.
    Busy,
    /// Live terminal with no attached view; its explicit controller is unchanged.
    Detached,
}

/// Closed, opt-in result of one exact acquisition receipt. Host authentication
/// secrets, private paths and tokens are deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAcquisitionResult {
    /// Closed response format.
    pub schema_version: u32,
    /// Original command receipt, unchanged by this query.
    pub receipt_id: ReceiptId,
    /// Original authenticated command.
    pub command_id: CommandId,
    /// Original allocation/control reference, never an alias supplied by another receipt.
    pub result_ref: String,
    /// Current authority lineage.
    pub authority_generation: AuthorityGeneration,
    /// Current, observed controller incarnation.
    pub daemon_epoch: DaemonEpoch,
    /// Logical terminal.
    pub terminal_id: TerminalId,
    /// Logical process.
    pub process_id: ProcessId,
    /// Exact physical process generation.
    pub process_run_uuid: ProcessRunUuid,
    /// Whether this receipt transferred an already registered terminal.
    pub reused: bool,
    /// Current qualified session state.
    pub state: TerminalAcquisitionState,
    /// Exact currently held lease; never a planned successor lease.
    pub lease_id: LeaseId,
    /// Current monotone resource fence.
    pub fence: Fence,
    /// Current deadline, read without renewal.
    pub lease_expires_at: TimestampMs,
    /// Effective control revision after the observed activation.
    pub control_revision: Revision,
    /// Last input confirmed by a durable native receipt, not the next expected sequence.
    pub input_seq: u64,
    /// Exact event of the qualified acquisition observation.
    pub cursor: EventCursor,
}

impl UnleashStore {
    /// Inspect a result only for its authenticated owner and exact live acquisition.
    /// This is separate from ordinary receipt inspection, whose shape is unchanged.
    ///
    /// # Errors
    /// Hides foreign receipts/references, refuses stale control assertions and
    /// unavailable native proof. A refusal never causes host I/O or lease renewal.
    pub fn inspect_terminal_acquisition_result(
        &self,
        actor: &ActorId,
        command: CommandId,
        reference: &str,
        project: ProjectId,
    ) -> Result<SnapshotProjection<TerminalAcquisitionResult>, StoreError> {
        self.read_snapshot(|conn, _| {
            let value = read_result(conn, self.config(), actor, command, reference, project)?;
            let high: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM events WHERE stream_id=?1",
                [value.cursor.stream_id.to_string()],
                |row| row.get(0),
            )?;
            let cursor = EventCursor::new(value.cursor.stream_id, Sequence::new(unsigned(high)?));
            if cursor.sequence < value.cursor.sequence {
                return Err(unavailable(
                    "acquisition observation is outside the read watermark",
                ));
            }
            // Artifact verification may outlive the initial lease check. Sample
            // UTC again only after all snapshot/proof reads, before releasing
            // assertions; this never renews the lease or changes its state.
            let now = self.config().clock.now_ms();
            if self.config().clock_uncertain.load(Ordering::Acquire) {
                return Err(stale(
                    "terminal clock became uncertain during result inspection",
                ));
            }
            if now >= value.lease_expires_at {
                return Err(stale(
                    "terminal acquisition lease expired during result inspection",
                ));
            }
            Ok(SnapshotProjection { value, cursor })
        })
    }
}

pub(super) fn read_result(
    conn: &Connection,
    config: &StoreConfig,
    actor: &ActorId,
    command: CommandId,
    reference: &str,
    project: ProjectId,
) -> Result<TerminalAcquisitionResult, StoreError> {
    let receipt = owned_receipt(conn, actor, command, reference)?;
    let daemon = read_daemon_state_conn(conn)?;
    if daemon.mode != DaemonMode::Ready || config.clock_uncertain.load(Ordering::Acquire) {
        return Err(stale("terminal control cannot be asserted by this daemon"));
    }
    let binding = binding(conn, reference, receipt, actor, command, project)?;
    let base = &binding.allocation.context;
    let metadata = terminal_allocations::read_terminal_metadata(
        conn,
        actor,
        Some(project),
        None::<&std::collections::HashSet<ProjectId>>,
    )?;
    if !metadata
        .items
        .iter()
        .any(|item| item.allows_control_result(base.terminal_id, base.process_run_uuid))
    {
        return Err(stale("terminal control has no qualified live registration"));
    }
    let session = live_session(conn, &binding, &daemon, config.clock.now_ms())?;
    if !(1..=MAX_EXACT_WIRE_INTEGER).contains(&binding.fence.as_u64())
        || session.input_seq > MAX_EXACT_WIRE_INTEGER
        || session.revision.as_u64() > MAX_EXACT_WIRE_INTEGER
        || daemon.daemon_epoch.as_u64() > MAX_EXACT_WIRE_INTEGER
    {
        return Err(stale(
            "terminal assertions exceed the exact wire integer domain",
        ));
    }
    // Authenticate the exact result and live controller first. A dispatched
    // input belongs to the physical run, including after control rotates;
    // consulting this projection cannot dispose or retry that uncertain write.
    super::terminal_input_writer::guard_causal_assertions(conn, &binding.allocation)?;
    let (mut proof, cursor) = observation(conn, config, &binding, &daemon)?;
    if !proof.unknown_input_seqs.is_empty() {
        return Err(stale("terminal native input state requires reconciliation"));
    }
    if proof.last_confirmed_input_seq < session.input_seq {
        proof = confirmed_input_state(conn, config, &binding, &daemon, session.input_seq)?;
    }
    if proof.last_confirmed_input_seq != session.input_seq {
        return Err(stale("terminal native input state requires reconciliation"));
    }
    Ok(TerminalAcquisitionResult {
        schema_version: 1,
        receipt_id: receipt,
        command_id: command,
        result_ref: reference.to_owned(),
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        terminal_id: base.terminal_id,
        process_id: base.process_id,
        process_run_uuid: base.process_run_uuid,
        reused: binding.control.is_some(),
        state: session.state,
        lease_id: binding.lease,
        fence: binding.fence,
        lease_expires_at: session.expires,
        control_revision: session.revision,
        input_seq: proof.last_confirmed_input_seq,
        cursor,
    })
}

fn owned_receipt(
    conn: &Connection,
    actor: &ActorId,
    command: CommandId,
    reference: &str,
) -> Result<ReceiptId, StoreError> {
    let ReceiptInspectionOutcome::Full(inspection) =
        read_command_receipt_inspection(conn, actor, command)?
    else {
        return Err(hidden());
    };
    if inspection.lineage_status != LineageStatus::Current
        || inspection.receipt.state != ReceiptState::Accepted
        || inspection.receipt.command_id != command
        || inspection.receipt.result_ref.as_deref() != Some(reference)
    {
        return Err(hidden());
    }
    Ok(inspection.receipt.receipt_id)
}

struct AcquisitionBinding {
    allocation: TerminalAllocationRecord,
    control: Option<TerminalControlRecord>,
    action: ActionId,
    dispatch: DispatchId,
    digest: Sha256Digest,
    lease: LeaseId,
    fence: Fence,
    revision: Revision,
    epoch: DaemonEpoch,
}

fn binding(
    conn: &Connection,
    reference: &str,
    receipt: ReceiptId,
    actor: &ActorId,
    command: CommandId,
    project: ProjectId,
) -> Result<AcquisitionBinding, StoreError> {
    let binding = if let Some(id) = reference.strip_prefix("terminal-allocation:") {
        let terminal: TerminalId = id.parse().map_err(|_| hidden())?;
        if reference != format!("terminal-allocation:{terminal}") {
            return Err(hidden());
        }
        let allocation = terminal_allocations::read_conn(conn, terminal)?.ok_or_else(hidden)?;
        let context = &allocation.context;
        AcquisitionBinding {
            action: context.action_id,
            dispatch: allocation
                .dispatch_id
                .ok_or_else(|| stale("acquisition is unobserved"))?,
            digest: allocation.context_digest,
            lease: context.lease_id,
            fence: context.resource_fence,
            revision: Revision::INITIAL,
            epoch: context.daemon_epoch,
            allocation,
            control: None,
        }
    } else if let Some(id) = reference.strip_prefix("terminal-control:") {
        let action: ActionId = id.parse().map_err(|_| hidden())?;
        if reference != format!("terminal-control:{action}") {
            return Err(hidden());
        }
        control_binding(conn, action, receipt, actor, command, project)?
    } else {
        return Err(hidden());
    };
    let base = &binding.allocation.context;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = base.preparation.scope;
    if base.actor_id != *actor || project_id != project {
        return Err(hidden());
    }
    let owned: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND actor_id=?2
         AND command_id=?3 AND aggregate_id=?4 AND command_type='AcquireTerminal'
         AND state='accepted' AND response_state='full')",
        params![
            receipt.to_string(),
            actor.as_str(),
            command.to_string(),
            base.terminal_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if !owned || (binding.control.is_none() && base.receipt_id != receipt) {
        return Err(hidden());
    }
    Ok(binding)
}

fn control_binding(
    conn: &Connection,
    action: ActionId,
    receipt: ReceiptId,
    actor: &ActorId,
    command: CommandId,
    project: ProjectId,
) -> Result<AcquisitionBinding, StoreError> {
    let control = terminal_allocations::control_read_conn(conn, action)?.ok_or_else(hidden)?;
    let context = &control.context;
    if context.receipt_id != receipt
        || context.command_id != command
        || context.actor_id != *actor
        || context.project_id != project
    {
        return Err(hidden());
    }
    if control.state != TerminalControlState::Observed
        || control.broker_action_state != Some(ActionState::ObservedSuccess)
        || control.observation_object_hash.is_none()
    {
        return Err(stale("terminal transfer has no observed activation"));
    }
    let allocation =
        terminal_allocations::read_conn(conn, context.terminal_id)?.ok_or_else(hidden)?;
    if context.initial_action_id != allocation.context.action_id
        || context.initial_context_digest != allocation.context_digest
        || context.process_id != allocation.context.process_id
        || context.process_run_uuid != allocation.context.process_run_uuid
        || context.host_id != allocation.context.host_id
        || context.principal_id != allocation.context.principal_id
        || context.authority_generation != allocation.context.authority_generation
        || context.daemon_boot_identity != allocation.context.daemon_boot_identity
    {
        return Err(stale("terminal transfer differs from the observed launch"));
    }
    let revision = context
        .previous_control_revision
        .as_u64()
        .checked_add(1)
        .ok_or_else(|| stale("terminal control revision is exhausted"))?;
    Ok(AcquisitionBinding {
        allocation,
        action,
        dispatch: control
            .dispatch_id
            .ok_or_else(|| stale("terminal transfer has no dispatch"))?,
        digest: control.context_digest,
        lease: context.next_lease_id,
        fence: context.next_fence,
        epoch: context.daemon_epoch,
        revision: Revision::new(revision),
        control: Some(control),
    })
}

struct LiveSession {
    input_seq: u64,
    expires: TimestampMs,
    revision: Revision,
    state: TerminalAcquisitionState,
}

fn live_session(
    conn: &Connection,
    binding: &AcquisitionBinding,
    daemon: &crate::commands::DaemonStateRow,
    now: TimestampMs,
) -> Result<LiveSession, StoreError> {
    let base = &binding.allocation.context;
    if base.authority_generation != daemon.authority_generation
        || base.daemon_boot_identity != daemon.boot_identity
        || binding.epoch != daemon.daemon_epoch
    {
        return Err(stale("acquisition belongs to an older controller"));
    }
    let row: Option<(i64, i64, i64, String)> = conn.query_row(
        "SELECT s.input_seq,l.expires_at_ms,s.control_revision,s.state FROM terminal_sessions s
         JOIN processes p ON p.process_id=s.process_id
         JOIN resource_leases l ON l.lease_id=s.lease_id
         JOIN resource_leases initial ON initial.lease_id=?9
         JOIN resource_fences f ON f.authority_generation=l.authority_generation
           AND f.resource_kind=l.resource_kind AND f.resource_key=l.resource_key
         WHERE s.terminal_id=?1 AND s.process_id=?2 AND p.process_run_uuid=?3
           AND s.lease_id=?4 AND l.fence=?5 AND s.control_revision=?6
           AND l.daemon_epoch=?7 AND l.authority_generation=?8 AND l.state='held'
           AND l.boot_identity=?10 AND l.resource_kind='terminal' AND l.resource_key=initial.resource_key
           AND l.owner_type='principal' AND l.owner_id=?11 AND f.last_fence=l.fence",
        params![base.terminal_id.to_string(),base.process_id.to_string(),base.process_run_uuid.to_string(),
            binding.lease.to_string(),integer(binding.fence.as_u64())?,integer(binding.revision.as_u64())?,
            integer(daemon.daemon_epoch.as_u64())?,daemon.authority_generation.to_string(),
            base.lease_id.to_string(),daemon.boot_identity,base.principal_id.as_str()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    let (input, expires, revision, state) =
        row.ok_or_else(|| stale("acquisition is no longer the live lease"))?;
    if now.as_i64() >= expires {
        return Err(stale("terminal acquisition lease has expired"));
    }
    let state = match state.as_str() {
        "ready" => TerminalAcquisitionState::Ready,
        "busy" => TerminalAcquisitionState::Busy,
        "detached" => TerminalAcquisitionState::Detached,
        _ => return Err(stale("terminal control is not ready")),
    };
    Ok(LiveSession {
        input_seq: unsigned(input)?,
        expires: TimestampMs::new(expires),
        revision: Revision::new(unsigned(revision)?),
        state,
    })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedPhase {
    schema_version: u32,
    evaluation: BrokerEvaluation,
    phase: BrokerPhase,
}

fn observation(
    conn: &Connection,
    config: &StoreConfig,
    binding: &AcquisitionBinding,
    daemon: &crate::commands::DaemonStateRow,
) -> Result<(TerminalObservedInputState, EventCursor), StoreError> {
    let (receipt, cursor) = sealed_observation(
        conn,
        daemon,
        binding.action,
        binding.dispatch,
        binding.digest,
        &binding.allocation.context.principal_id,
    )?;
    let hash = receipt
        .output_digest
        .ok_or_else(|| unavailable("acquisition has no observation digest"))?;
    let input = native_proof(conn, config, binding, hash)?;
    Ok((input, cursor))
}

fn sealed_observation(
    conn: &Connection,
    daemon: &crate::commands::DaemonStateRow,
    expected_action: ActionId,
    expected_dispatch: DispatchId,
    expected_payload: Sha256Digest,
    principal: &ActorId,
) -> Result<(crate::BrokerActionReceipt, EventCursor), StoreError> {
    let row: Option<(String, String, String, i64)> = conn
        .query_row(
            "SELECT phase.phase_json,phase.phase_digest,event.stream_id,event.sequence
         FROM broker_phase_records phase JOIN events event ON event.event_id=phase.event_id
         JOIN broker_dispatches dispatch ON dispatch.action_id=phase.action_id
         JOIN actions action ON action.action_id=phase.action_id
         WHERE phase.action_id=?1 AND phase.phase_key LIKE 'observation:%'
           AND dispatch.dispatch_id=?2 AND action.state='observed_success'
           AND action.payload_hash=?3 AND action.control_principal_id=?4
           AND dispatch.authority_generation=?5 AND dispatch.daemon_epoch=?6
         ORDER BY phase.action_revision DESC LIMIT 1",
            params![
                expected_action.to_string(),
                expected_dispatch.to_string(),
                expected_payload.to_string(),
                principal.as_str(),
                daemon.authority_generation.to_string(),
                integer(daemon.daemon_epoch.as_u64())?
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (encoded, digest, stream, sequence) =
        row.ok_or_else(|| unavailable("acquisition observation is absent"))?;
    let phase: PersistedPhase = serde_json::from_str(&encoded)?;
    if phase.schema_version != crate::broker_phases::BROKER_PHASE_SCHEMA_VERSION
        || canonical_sha256(&phase)? != digest.parse::<Sha256Digest>()?
        || stream != daemon.stream_id.to_string()
    {
        return Err(unavailable("acquisition observation integrity differs"));
    }
    let BrokerPhase::Observation {
        action_id, receipt, ..
    } = phase.phase
    else {
        return Err(unavailable("acquisition has no observation receipt"));
    };
    if action_id != expected_action
        || receipt.action_id != expected_action
        || receipt.dispatch_id != expected_dispatch
        || receipt.state != ActionState::ObservedSuccess
        || receipt.evaluation_id != phase.evaluation.evaluation_id
    {
        return Err(unavailable("acquisition observation receipt differs"));
    }
    Ok((
        receipt,
        EventCursor::new(daemon.stream_id, Sequence::new(unsigned(sequence)?)),
    ))
}

fn confirmed_input_state(
    conn: &Connection,
    config: &StoreConfig,
    binding: &AcquisitionBinding,
    daemon: &crate::commands::DaemonStateRow,
    sequence: u64,
) -> Result<TerminalObservedInputState, StoreError> {
    let base = &binding.allocation.context;
    if daemon.schema_version < 14 {
        return Err(unavailable("confirmed native input receipt is absent"));
    }
    let action:Option<String>=conn.query_row(
        "SELECT action_id FROM terminal_input_intents WHERE terminal_id=?1 AND authority_generation=?2 AND state='observed' AND input_seq=?3 AND lease_id=?4 AND resource_fence=?5 AND daemon_epoch=?6 AND json_extract(context_json,'$.process_run_uuid')=?7 AND json_extract(context_json,'$.initial_context_digest')=?8",
        params![base.terminal_id.to_string(),daemon.authority_generation.to_string(),integer(sequence)?,binding.lease.to_string(),integer(binding.fence.as_u64())?,integer(binding.epoch.as_u64())?,base.process_run_uuid.to_string(),binding.allocation.context_digest.to_string()], |row|row.get(0),
    ).optional()?;
    let action = action.ok_or_else(|| unavailable("confirmed native input receipt is absent"))?;
    let input = super::terminal_input_writer::read_input_conn(conn, action.parse()?)?
        .ok_or_else(|| unavailable("confirmed native input intention disappeared"))?;
    let c = &input.context;
    let TerminalAllocationScope::ProjectConfiguration { project_id, .. } = base.preparation.scope;
    if input.state != super::terminal_input_types::TerminalInputState::Observed
        || c.actor_id != base.actor_id
        || c.principal_id != base.principal_id
        || c.initial_action_id != base.action_id
        || c.process_id != base.process_id
        || c.host_id != base.host_id
        || c.control_revision != binding.revision
        || c.project_id != project_id
        || input.broker_action_state != Some(ActionState::ObservedSuccess)
        || input.broker_dispatch_id != input.dispatch_id
    {
        return Err(unavailable(
            "confirmed native input is outside the current physical controller",
        ));
    }
    let dispatch = input
        .dispatch_id
        .ok_or_else(|| unavailable("confirmed native input dispatch missing"))?;
    let (receipt, _) = sealed_observation(
        conn,
        daemon,
        c.action_id,
        dispatch,
        input.context_digest,
        &c.principal_id,
    )?;
    let hash = receipt
        .output_digest
        .ok_or_else(|| unavailable("confirmed native input proof missing"))?;
    if input.observation_object_hash != Some(hash) {
        return Err(unavailable("confirmed native input proof hash differs"));
    }
    let native = super::terminal_input_observation::read_published_native_observation(
        conn,
        config,
        c.action_id,
        dispatch,
        hash,
    )?;
    super::terminal_input_observation::validate_native_observation_context(&input, &native)?;
    let started: i64 = conn.query_row(
        "SELECT started_at_ms FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2",
        params![c.action_id.to_string(), dispatch.to_string()],
        |row| row.get(0),
    )?;
    if native.observed_at_ms.as_i64() < started
        || native.observed_at_ms > receipt.observed_at_ms
        || native.native_receipt.input_seq != sequence
    {
        return Err(unavailable(
            "confirmed native input observation time or sequence differs",
        ));
    }
    Ok(TerminalObservedInputState {
        last_confirmed_input_seq: native.native_receipt.input_seq,
        unknown_input_seqs: vec![],
    })
}

fn native_proof(
    conn: &Connection,
    config: &StoreConfig,
    binding: &AcquisitionBinding,
    hash: Sha256Digest,
) -> Result<TerminalObservedInputState, StoreError> {
    let base = &binding.allocation.context;
    let (owner_kind, owner_id) = if binding.control.is_some() {
        ("terminal_control_observation", binding.action.to_string())
    } else {
        ("terminal_launch_observation", base.terminal_id.to_string())
    };
    let published: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_references WHERE owner_kind=?1 AND owner_id=?2
         AND source_field=?3 AND object_hash=?4 AND state='live')",
        params![
            owner_kind,
            owner_id,
            binding.dispatch.to_string(),
            hash.to_string()
        ],
        |row| row.get(0),
    )?;
    if !published {
        return Err(unavailable("acquisition observation is not published"));
    }
    // The same reader can be used for new input admission under the writer.
    // Bound physical I/O and retain anchored no-follow/owner checks.
    let bytes =
        crate::objects::read_verified_evidence_bytes_bounded(conn, config, hash, 2 * 1024 * 1024)?;
    if let Some(control) = &binding.control {
        control_input_state(conn, binding, control, hash, &bytes)
    } else {
        launch_input_state(conn, binding, hash, &bytes)
    }
}

fn control_input_state(
    conn: &Connection,
    binding: &AcquisitionBinding,
    control: &TerminalControlRecord,
    hash: Sha256Digest,
    bytes: &[u8],
) -> Result<TerminalObservedInputState, StoreError> {
    let base = &binding.allocation.context;
    let proof: terminal_allocations::TerminalControlObservation = serde_json::from_slice(bytes)?;
    // The phase seals the native effective environment observation. The public
    // profile object's digest has a different domain (the host adds TMPDIR),
    // and cannot serve as an expected native digest in this passive read.
    if control.observation_object_hash != Some(hash)
        || canonical_sha256(&proof)? != hash
        || proof.action_id != binding.action
        || proof.dispatch_id != binding.dispatch
        || proof.context_digest != binding.digest
        || proof.terminal_id != base.terminal_id
        || proof.host_id != base.host_id
        || proof.process_run_uuid != base.process_run_uuid
        || proof.resource_fence != binding.fence
        || proof.daemon_epoch != binding.epoch
        || proof.os_identity != control.context.expected_os_identity
    {
        return Err(unavailable(
            "native transfer proof differs from acquisition",
        ));
    }
    require_observed_identity(conn, binding, &proof.os_identity)?;
    Ok(TerminalObservedInputState {
        last_confirmed_input_seq: proof.last_confirmed_input_seq,
        unknown_input_seqs: proof.unknown_input_seqs,
    })
}

fn launch_input_state(
    conn: &Connection,
    binding: &AcquisitionBinding,
    hash: Sha256Digest,
    bytes: &[u8],
) -> Result<TerminalObservedInputState, StoreError> {
    let base = &binding.allocation.context;
    let proof: TerminalLaunchObservation = serde_json::from_slice(bytes)?;
    if canonical_sha256(&proof)? != hash
        || proof.action_id != binding.action
        || proof.dispatch_id != binding.dispatch
        || proof.context_digest != binding.digest
        || proof.terminal_id != base.terminal_id
        || proof.host_id != base.host_id
        || proof.process_run_uuid != base.process_run_uuid
        || proof.exited
    {
        return Err(unavailable("native launch proof differs from acquisition"));
    }
    let backend = match proof.identity_backend.as_str() {
        "macos_proc_pidinfo" => TerminalIdentityBackend::MacosProcPidinfo,
        "linux_procfs" => TerminalIdentityBackend::LinuxProcfs,
        _ => return Err(unavailable("native launch identity backend is unqualified")),
    };
    require_observed_identity(
        conn,
        binding,
        &TerminalProcessIdentity {
            pid: proof.pid,
            pgid: proof.pgid,
            owner_uid: proof.owner_uid,
            boot_identity: proof.boot_identity,
            start_identity: proof.start_identity,
            backend,
        },
    )?;
    proof
        .input_state
        .ok_or_else(|| unavailable("native launch input state was not observed"))
}

fn require_observed_identity(
    conn: &Connection,
    binding: &AcquisitionBinding,
    identity: &TerminalProcessIdentity,
) -> Result<(), StoreError> {
    let backend = match identity.backend {
        TerminalIdentityBackend::MacosProcPidinfo => "macos_proc_pidinfo",
        TerminalIdentityBackend::LinuxProcfs => "linux_procfs",
    };
    let base = &binding.allocation.context;
    let exact: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM processes WHERE process_id=?1 AND action_id=?2
         AND process_run_uuid=?3 AND host_id=?4 AND authority_generation=?5
         AND pid=?6 AND pgid=?7 AND owner_uid=?8 AND boot_identity=?9
         AND start_identity=?10 AND identity_backend=?11 AND state='live')",
        params![
            base.process_id.to_string(),
            base.action_id.to_string(),
            base.process_run_uuid.to_string(),
            base.host_id.to_string(),
            base.authority_generation.to_string(),
            identity.pid,
            identity.pgid,
            identity.owner_uid,
            identity.boot_identity,
            identity.start_identity,
            backend
        ],
        |row| row.get(0),
    )?;
    if !exact {
        return Err(stale("terminal identity differs from its observed proof"));
    }
    Ok(())
}

fn hidden() -> StoreError {
    StoreError::NotFound("qualified terminal acquisition".into())
}
fn stale(message: &'static str) -> StoreError {
    StoreError::StaleFence {
        message: message.into(),
    }
}
fn unavailable(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.into())
}
fn unsigned(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| unavailable("invalid terminal acquisition counter"))
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| unavailable("terminal acquisition counter is out of range"))
}
