//! Composition of the pure scheduling policy and the transactional admission port.

use std::collections::BTreeMap;
use std::time::Duration;
use unleash_domain::{AdmissionClass, InvocationId, PoolId, ReservationId};
use unleash_scheduler::{
    ActiveExecution, AdmissionSnapshot, CheckpointEligibility, CheckpointRequest,
    ControlReserveMode, Decision, ExecutionUnit, ObservedCheckpoint, ProjectFairness,
    QueuedExecution, SchedulingSettings,
};
use unleash_store::{
    AdmissionDecision, AdmissionOutcome, AdmissionSnapshotToken, AdmissionUnit, AdmissionUrgency,
    BudgetReservationRequest, ExecutionAdmissionSnapshot, StoreError, UnleashStore,
};

/// Trusted scheduling inputs prepared for an admission snapshot.
#[derive(Debug, Default)]
pub struct ExecutionSchedulingInputs {
    /// Plan by unit. An empty plan is explicit and the writer still verifies its adequacy.
    pub budgets: BTreeMap<AdmissionUnit, Vec<BudgetReservationRequest>>,
    /// Observed capability of the exact active reservation, supplied by its executor.
    pub checkpoint_eligibility: BTreeMap<ReservationId, CheckpointEligibility>,
    /// Observed monotonic duration of a pending request in the same authority epoch.
    pub checkpoint_elapsed: BTreeMap<(InvocationId, ReservationId), Duration>,
}

/// Compute policy on the writer's current snapshot and commit one admission if possible.
///
/// The token must match the durable inputs, observations, authority generation and epoch used by
/// the caller. The returned policy retains wait reasons and any proposed checkpoint.
/// The caller requests a checkpoint separately through the execution continuation
/// port. No policy proposal itself frees capacity or runs an executor.
///
/// # Errors
///
/// Returns the writer's error on stale or invalid evidence or when the selected
/// unit has no prepared budget plan.
pub fn schedule_execution(
    store: &UnleashStore,
    pool: PoolId,
    token: AdmissionSnapshotToken,
    inputs: ExecutionSchedulingInputs,
) -> Result<(AdmissionOutcome, Decision), StoreError> {
    schedule_where(store, pool, token, inputs, |_| true)
}

pub(crate) fn schedule_invocation_execution(
    store: &UnleashStore,
    pool: PoolId,
    token: AdmissionSnapshotToken,
    inputs: ExecutionSchedulingInputs,
) -> Result<(AdmissionOutcome, Decision), StoreError> {
    schedule_where(store, pool, token, inputs, |unit| {
        matches!(unit, AdmissionUnit::Invocation(_))
    })
}

fn schedule_where(
    store: &UnleashStore,
    pool: PoolId,
    token: AdmissionSnapshotToken,
    mut inputs: ExecutionSchedulingInputs,
    owns: fn(AdmissionUnit) -> bool,
) -> Result<(AdmissionOutcome, Decision), StoreError> {
    store.commit_execution_admission_with_report(pool, token, move |snapshot| {
        let decision = decide(snapshot, &inputs)?;
        let mutation = match &decision {
            Decision::Idle => AdmissionDecision::Idle,
            Decision::Wait { .. } => AdmissionDecision::Wait,
            Decision::Admit(plan) => {
                let unit = store_unit(plan.unit);
                // Keep global ordering when this executor cannot run the selected unit.
                if !owns(unit) {
                    return Ok((AdmissionDecision::Wait, decision));
                }
                let budgets = inputs.budgets.remove(&unit).ok_or_else(|| {
                    StoreError::InvariantViolation("Plan budgétaire de l’admission absent".into())
                })?;
                AdmissionDecision::Admit {
                    unit,
                    deficit_deltas: plan.deficit_deltas.clone(),
                    consecutive_controls: plan.consecutive_controls_with_work,
                    budgets,
                }
            }
        };
        Ok((mutation, decision))
    })
}

fn decide(
    snapshot: &ExecutionAdmissionSnapshot,
    inputs: &ExecutionSchedulingInputs,
) -> Result<Decision, StoreError> {
    let settings = scheduling_settings(snapshot)?;
    let active = snapshot
        .active
        .iter()
        .map(|active| {
            Ok(ActiveExecution {
                unit: scheduling_unit(active.unit, active.class)?,
                reservation_id: active.reservation_id,
                project_id: active.project_id,
                admitted_at: active.admitted_at,
                checkpoint: inputs
                    .checkpoint_eligibility
                    .get(&active.reservation_id)
                    .copied()
                    .unwrap_or(CheckpointEligibility::Unavailable),
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let eligible = snapshot
        .queued
        .iter()
        .filter_map(|candidate| {
            candidate.eligibility_evidence?;
            let queued_at = candidate.queued_at?;
            Some(
                scheduling_unit(candidate.unit, candidate.class).map(|unit| QueuedExecution {
                    unit,
                    project_id: candidate.project_id,
                    queued_at,
                    urgent: candidate.urgency == AdmissionUrgency::UrgentControl,
                }),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let projects = snapshot
        .fairness
        .iter()
        .map(|(&project, &(weight, deficit))| (project, ProjectFairness { weight, deficit }))
        .collect();
    let checkpoints: Vec<_> = snapshot
        .checkpoints
        .iter()
        .map(|record| ObservedCheckpoint {
            request: CheckpointRequest {
                waiting_control: record.waiting_control,
                target: record.reservation_id,
            },
            elapsed: inputs
                .checkpoint_elapsed
                .get(&(record.waiting_control, record.reservation_id))
                .copied(),
            pending: true,
        })
        .collect();
    unleash_scheduler::decide(&AdmissionSnapshot {
        settings,
        admissions_frozen: snapshot.pool.admissions_frozen,
        active: &active,
        eligible: &eligible,
        projects: &projects,
        consecutive_controls_with_work: snapshot.consecutive_controls,
        checkpoints: &checkpoints,
    })
    .map_err(|error| StoreError::InvariantViolation(error.to_string()))
}

fn scheduling_settings(
    snapshot: &ExecutionAdmissionSnapshot,
) -> Result<SchedulingSettings, StoreError> {
    fn invalid_limit(_: std::num::TryFromIntError) -> StoreError {
        StoreError::InvariantViolation("Limites d’admission hors plage".into())
    }

    let settings = &snapshot.settings;
    Ok(SchedulingSettings {
        effective_limit: u8::try_from(snapshot.pool.effective_limit).map_err(invalid_limit)?,
        control_reserve_mode: match settings.control_reserve_mode {
            unleash_store::ControlReserveMode::Soft => ControlReserveMode::Soft,
            unleash_store::ControlReserveMode::Hard => ControlReserveMode::Hard,
        },
        control_reserved_units: u8::try_from(settings.control_reserved_units)
            .map_err(invalid_limit)?,
        control_burst_limit: u64::try_from(settings.control_burst_limit).map_err(invalid_limit)?,
        checkpoint_escalation_s: u16::try_from(settings.checkpoint_escalation_s)
            .map_err(invalid_limit)?,
    })
}

fn scheduling_unit(
    unit: AdmissionUnit,
    class: AdmissionClass,
) -> Result<ExecutionUnit, StoreError> {
    match (unit, class) {
        (AdmissionUnit::Invocation(id), AdmissionClass::Control) => Ok(ExecutionUnit::Control(id)),
        (AdmissionUnit::Invocation(id), AdmissionClass::Work) => Ok(ExecutionUnit::AgentWork(id)),
        (AdmissionUnit::Replay(id), AdmissionClass::Work) => Ok(ExecutionUnit::ReplayWorker(id)),
        (AdmissionUnit::Probe(id), AdmissionClass::Work) => Ok(ExecutionUnit::ProviderProbe(id)),
        _ => Err(StoreError::InvariantViolation(
            "Classe de contrôle incompatible avec l’unité d’admission".into(),
        )),
    }
}

fn store_unit(unit: ExecutionUnit) -> AdmissionUnit {
    match unit {
        ExecutionUnit::Control(id) | ExecutionUnit::AgentWork(id) => AdmissionUnit::Invocation(id),
        ExecutionUnit::ReplayWorker(id) => AdmissionUnit::Replay(id),
        ExecutionUnit::ProviderProbe(id) => AdmissionUnit::Probe(id),
    }
}
