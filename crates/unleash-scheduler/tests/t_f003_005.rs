//! T-F003-005 / REQ-AGENT-005 — politique sur snapshots et échéances contrôlées.

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use unleash_domain::{
    InvocationId, ProjectId, ProviderProbeId, ReplayWorkerId, ReservationId, TimestampMs,
};
use unleash_scheduler::CheckpointEligibility::{Cooperative, SafeBoundary, Unavailable};
use unleash_scheduler::{
    ActiveExecution, AdmissionPlan, AdmissionSnapshot, CapacityWait, CheckpointEligibility,
    CheckpointRequest, ControlReserveMode, Decision, ExecutionUnit, ObservedCheckpoint,
    ProjectFairness, QueuedExecution, SchedulingError, SchedulingSettings, decide,
};

type TestResult = Result<(), Box<dyn Error>>;
type Projects = BTreeMap<Option<ProjectId>, ProjectFairness>;

const PROJECT_A: Option<ProjectId> = Some(ProjectId::from_u128(1));
const PROJECT_B: Option<ProjectId> = Some(ProjectId::from_u128(2));
const PROJECT_C: Option<ProjectId> = Some(ProjectId::from_u128(3));

fn work(id: u128) -> ExecutionUnit {
    ExecutionUnit::AgentWork(InvocationId::from_u128(id))
}

fn control(id: u128) -> ExecutionUnit {
    ExecutionUnit::Control(InvocationId::from_u128(id))
}

fn replay(id: u128) -> ExecutionUnit {
    ExecutionUnit::ReplayWorker(ReplayWorkerId::from_u128(id))
}

fn probe(id: u128) -> ExecutionUnit {
    ExecutionUnit::ProviderProbe(ProviderProbeId::from_u128(id))
}

fn held(
    id: u128,
    unit: ExecutionUnit,
    project_id: Option<ProjectId>,
    admitted_at: i64,
    checkpoint: CheckpointEligibility,
) -> ActiveExecution {
    ActiveExecution {
        reservation_id: ReservationId::from_u128(id),
        unit,
        project_id,
        admitted_at: TimestampMs::new(admitted_at),
        checkpoint,
    }
}

fn queued(
    unit: ExecutionUnit,
    project_id: Option<ProjectId>,
    queued_at: i64,
    urgent: bool,
) -> QueuedExecution {
    QueuedExecution {
        unit,
        project_id,
        queued_at: TimestampMs::new(queued_at),
        urgent,
    }
}

fn mixed_work() -> [ActiveExecution; 5] {
    [
        held(1, work(11), PROJECT_A, 100, Cooperative),
        held(2, work(12), PROJECT_A, 120, Unavailable),
        held(3, work(13), PROJECT_B, 130, SafeBoundary),
        held(4, replay(14), PROJECT_B, 140, SafeBoundary),
        held(5, probe(15), None, 150, Unavailable),
    ]
}

fn snapshot<'a>(
    active: &'a [ActiveExecution],
    eligible: &'a [QueuedExecution],
    projects: &'a Projects,
) -> AdmissionSnapshot<'a> {
    AdmissionSnapshot {
        settings: SchedulingSettings::default(),
        admissions_frozen: false,
        active,
        eligible,
        projects,
        consecutive_controls_with_work: 0,
        checkpoints: &[],
    }
}

fn admission(snapshot: &AdmissionSnapshot<'_>) -> Result<AdmissionPlan, Box<dyn Error>> {
    match decide(snapshot)? {
        Decision::Admit(plan) => Ok(plan),
        other => Err(format!("admission attendue, reçu {other:?}").into()),
    }
}

fn request(control_id: u128, target_id: u128) -> CheckpointRequest {
    CheckpointRequest {
        waiting_control: InvocationId::from_u128(control_id),
        target: ReservationId::from_u128(target_id),
    }
}

fn observed(request: CheckpointRequest, secs: Option<u64>) -> ObservedCheckpoint {
    ObservedCheckpoint {
        request,
        elapsed: secs.map(Duration::from_secs),
        pending: true,
    }
}

fn capacity_wait(control_id: u128, target_id: Option<u128>) -> Decision {
    Decision::Wait {
        unit: control(control_id),
        reason: CapacityWait::GlobalCapacity,
        checkpoint: target_id.map(|id| request(control_id, id)),
    }
}

#[test]
fn t_f003_005_soft_waits_for_observed_release_and_escalates_at_deadline() -> TestResult {
    let active = mixed_work();
    let projects = Projects::new();
    let probe = [queued(probe(15), None, 0, false)];
    assert_eq!(decide(&snapshot(&active, &[], &projects))?, Decision::Idle);
    assert_eq!(
        admission(&snapshot(&active[..4], &probe, &projects))?.unit,
        probe[0].unit
    );

    let queue = [
        queued(replay(16), PROJECT_A, 0, false),
        queued(control(91), PROJECT_C, 160, true),
    ];
    let mut full = snapshot(&active, &queue, &projects);
    assert_eq!(decide(&full)?, capacity_wait(91, Some(4)));

    let first = request(91, 4);
    for elapsed in [Some(0), Some(59), None] {
        let recorded = [observed(first, elapsed)];
        assert_eq!(
            decide(&AdmissionSnapshot {
                checkpoints: &recorded,
                ..full
            })?,
            capacity_wait(91, None)
        );
    }
    let first_expired = [observed(first, Some(60))];
    full.checkpoints = &first_expired;
    assert_eq!(decide(&full)?, capacity_wait(91, Some(3)));
    let both = [first_expired[0], observed(request(91, 3), Some(0))];
    full.checkpoints = &both;
    assert_eq!(decide(&full)?, capacity_wait(91, None));

    let released = [active[0], active[1], active[2], active[4]];
    let after_release = AdmissionSnapshot {
        active: &released,
        ..full
    };
    let plan = admission(&after_release)?;
    assert_eq!(plan.unit, control(91));
    assert_eq!(released.len() + 1, 5);
    assert_eq!(plan.consecutive_controls_with_work, 1);

    let surplus = [
        active[0],
        active[1],
        active[4],
        held(6, control(91), PROJECT_C, 220, Unavailable),
    ];
    let after_surplus_release = AdmissionSnapshot {
        active: &surplus,
        eligible: &queue[..1],
        consecutive_controls_with_work: plan.consecutive_controls_with_work,
        ..full
    };
    assert_eq!(admission(&after_surplus_release)?.unit, replay(16));
    Ok(())
}

#[test]
fn t_f003_005_checkpoint_order_resolves_each_tie_independently() -> TestResult {
    let mut active = [
        held(1, work(11), PROJECT_A, 1000, SafeBoundary),
        held(2, work(12), PROJECT_A, 500, Unavailable),
        held(3, probe(13), PROJECT_A, 550, Unavailable),
        held(4, work(14), PROJECT_B, 800, Cooperative),
        held(5, replay(15), PROJECT_B, 700, Cooperative),
    ];
    let projects = Projects::from([(
        PROJECT_A,
        ProjectFairness {
            weight: 10,
            deficit: 0,
        },
    )]);
    let queue = [queued(control(91), PROJECT_C, 1010, true)];
    assert_eq!(
        decide(&snapshot(&active, &queue, &projects))?,
        capacity_wait(91, Some(1))
    );

    active[3].checkpoint = SafeBoundary;
    active[4].checkpoint = SafeBoundary;
    // Le projet 2 dépasse davantage sa part bien qu'il occupe moins d'unités.
    assert_eq!(
        decide(&snapshot(&active, &queue, &projects))?,
        capacity_wait(91, Some(4))
    );

    active[4].admitted_at = TimestampMs::new(900);
    assert_eq!(
        decide(&snapshot(&active, &queue, &projects))?,
        capacity_wait(91, Some(5))
    );

    active[3].admitted_at = active[4].admitted_at;
    for _ in 0..2 {
        assert_eq!(
            decide(&snapshot(&active, &queue, &projects))?,
            capacity_wait(91, Some(4))
        );
        active.reverse();
    }
    Ok(())
}

#[test]
fn t_f003_005_checkpoint_skips_controls_unavailable_work_and_pending_targets() -> TestResult {
    let mut active = mixed_work();
    active[0].unit = control(90);
    active[0].checkpoint = SafeBoundary;
    active[2].checkpoint = Unavailable;
    active[4].checkpoint = Cooperative;
    let projects = Projects::new();
    let queue = [queued(control(91), PROJECT_C, 1010, true)];
    let other_control = [observed(request(92, 4), Some(0))];
    let full = AdmissionSnapshot {
        checkpoints: &other_control,
        ..snapshot(&active, &queue, &projects)
    };
    assert_eq!(decide(&full)?, capacity_wait(91, Some(5)));

    let mut blocked = active;
    blocked[4].checkpoint = Unavailable;
    assert_eq!(
        decide(&AdmissionSnapshot {
            active: &blocked,
            ..full
        })?,
        capacity_wait(91, None)
    );

    let ordinary_control = [queued(control(91), PROJECT_C, 1010, false)];
    assert_eq!(
        decide(&AdmissionSnapshot {
            eligible: &ordinary_control,
            ..full
        })?,
        capacity_wait(91, None)
    );
    Ok(())
}

#[test]
fn t_f003_005_hard_reserve_survives_work_pressure_and_burst_limit() -> TestResult {
    let mut active = mixed_work();
    let projects = Projects::new();
    let only_work = [queued(probe(15), None, 0, false)];
    let settings = SchedulingSettings {
        control_reserve_mode: ControlReserveMode::Hard,
        ..SchedulingSettings::default()
    };
    let reserve = AdmissionSnapshot {
        settings,
        consecutive_controls_with_work: 2,
        ..snapshot(&active[..4], &only_work, &projects)
    };
    assert_eq!(
        decide(&reserve)?,
        Decision::Wait {
            unit: only_work[0].unit,
            reason: CapacityWait::HardControlReserve,
            checkpoint: None,
        }
    );
    let queue = [only_work[0], queued(control(91), PROJECT_C, 100, true)];
    let plan = admission(&AdmissionSnapshot {
        eligible: &queue,
        ..reserve
    })?;
    assert_eq!(plan.unit, control(91));
    assert_eq!(plan.consecutive_controls_with_work, 3);

    active[4] = held(6, control(91), PROJECT_C, 160, Unavailable);
    let next_queue = [only_work[0], queued(control(92), PROJECT_C, 170, true)];
    let after_work_release = AdmissionSnapshot {
        settings,
        consecutive_controls_with_work: plan.consecutive_controls_with_work,
        ..snapshot(&active[1..], &next_queue, &projects)
    };
    assert_eq!(admission(&after_work_release)?.unit, only_work[0].unit);
    Ok(())
}

#[test]
fn t_f003_005_weighted_deficit_shares_admissions_without_charging_blocked_projects() -> TestResult {
    let mut projects = Projects::from([
        (
            PROJECT_A,
            ProjectFairness {
                weight: 1,
                deficit: 0,
            },
        ),
        (
            PROJECT_B,
            ProjectFairness {
                weight: 3,
                deficit: 0,
            },
        ),
        (
            PROJECT_C,
            ProjectFairness {
                weight: 10,
                deficit: 50,
            },
        ),
    ]);
    let blocked = projects.get(&PROJECT_C).copied();
    let mut light = 0;
    let mut heavy = 0;
    for turn in 0..24 {
        let queue = [
            queued(replay(100 + turn), PROJECT_B, 20, false),
            queued(work(200 + turn), PROJECT_A, 10, false),
        ];
        let snapshot = snapshot(&[], &queue, &projects);
        let plan = admission(&snapshot)?;
        assert_eq!(plan, admission(&snapshot)?);
        if plan.unit == queue[0].unit {
            heavy += 1;
        } else {
            assert_eq!(plan.unit, queue[1].unit);
            light += 1;
        }
        for (id, delta) in plan.deficit_deltas {
            projects.entry(id).or_default().deficit += delta;
        }
        if turn % 4 == 3 {
            assert_eq!(heavy, 3 * light);
        }
    }
    assert_eq!((light, heavy), (6, 18));
    assert_eq!(projects.get(&PROJECT_C).copied(), blocked);
    Ok(())
}

#[test]
fn t_f003_005_equal_credit_ages_work_and_then_breaks_ties_by_id() -> TestResult {
    let projects = Projects::new();
    let queue = [
        queued(work(10), PROJECT_A, 20, false),
        queued(work(30), PROJECT_B, 10, false),
        queued(work(20), PROJECT_B, 10, false),
    ];
    assert_eq!(admission(&snapshot(&[], &queue, &projects))?.unit, work(20));
    Ok(())
}

#[test]
fn t_f003_005_two_committed_controls_yield_to_oldest_eligible_work_at_c_one() -> TestResult {
    let projects = Projects::from([(
        PROJECT_B,
        ProjectFairness {
            weight: 9,
            deficit: 30,
        },
    )]);
    let queue = [
        queued(control(91), PROJECT_C, 10, true),
        queued(control(92), PROJECT_C, 20, true),
        queued(control(93), PROJECT_C, 30, true),
        queued(replay(41), PROJECT_B, 5, false),
        queued(probe(42), None, 0, false),
    ];
    let mut snapshot = snapshot(&[], &queue, &projects);
    snapshot.settings.effective_limit = 1;

    for (eligible, candidate, streak) in
        [(&queue[..], control(91), 1), (&queue[1..], control(92), 2)]
    {
        snapshot.eligible = eligible;
        let plan = admission(&snapshot)?;
        assert_eq!(plan.unit, candidate);
        assert_eq!(plan.consecutive_controls_with_work, streak);
        snapshot.consecutive_controls_with_work = plan.consecutive_controls_with_work;
    }
    snapshot.eligible = &queue[2..];
    let plan = admission(&snapshot)?;
    assert_eq!(plan.unit, probe(42));
    assert_eq!(plan.consecutive_controls_with_work, 0);

    snapshot.eligible = &queue[2..3];
    let plan = admission(&snapshot)?;
    assert_eq!(plan.unit, control(93));
    assert_eq!(plan.consecutive_controls_with_work, 0);
    Ok(())
}

#[test]
fn t_f003_005_invalid_pool_and_admission_freeze_fail_closed() -> TestResult {
    let active = mixed_work();
    let projects = Projects::new();
    let queue = [queued(control(91), PROJECT_C, 160, true)];
    let frozen = AdmissionSnapshot {
        admissions_frozen: true,
        ..snapshot(&[], &queue, &projects)
    };
    for active in [&[][..], &active[..]] {
        assert_eq!(
            decide(&AdmissionSnapshot { active, ..frozen })?,
            Decision::Wait {
                unit: control(91),
                reason: CapacityWait::AdmissionsFrozen,
                checkpoint: None,
            }
        );
    }

    let mut invalid = snapshot(&[], &queue, &projects);
    invalid.settings.effective_limit = 51;
    assert_eq!(
        decide(&invalid),
        Err(SchedulingError::ControlReserveIncoherent)
    );
    invalid.settings.effective_limit = 1;
    invalid.settings.control_reserve_mode = ControlReserveMode::Hard;
    assert_eq!(
        decide(&invalid),
        Err(SchedulingError::ControlReserveIncoherent)
    );
    invalid = snapshot(&active, &queue, &projects);
    invalid.settings.effective_limit = 4;
    assert_eq!(
        decide(&invalid),
        Err(SchedulingError::OverCapacity {
            occupied: 5,
            limit: 4
        })
    );

    let invalid_weight = Projects::from([(
        PROJECT_A,
        ProjectFairness {
            weight: 11,
            deficit: 0,
        },
    )]);
    assert_eq!(
        decide(&snapshot(&[], &queue, &invalid_weight)),
        Err(SchedulingError::InvalidProjectWeight {
            project_id: PROJECT_A,
            weight: 11
        })
    );
    Ok(())
}
