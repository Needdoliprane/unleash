//! Tests d'intégration du déterminisme : horloge manuelle, générateur d'IDs, cutpoints et répertoires temporaires.
// T-F001-008 / REQ-CORE-008
// T-F002-001 / REQ-PERSIST-001

use std::collections::HashSet;
use std::error::Error;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use unleash_domain::{AttemptId, ClockPort, IdGeneratorPort, TimestampMs};
use unleash_testkit::{
    Cutpoint, CutpointAction, CutpointController, CutpointError, DeterministicIdGenerator,
    MACOS_SUN_LEN, MAX_UNIX_SOCKET_PATH_BYTES, ManualClock, TempDirError, local_test_dir,
    socket_path_in,
};

const ENV_SEED_PANIC_CHILD: &str = "UNLEASH_TEST_SEED_PANIC_CHILD";
const TEST_SEED_VALUE: u64 = 0x1234_5678_9ABC_DEF0;

#[test]
fn test_manual_clock_forward_backward_and_thread_wakeups_without_sleep()
-> Result<(), Box<dyn Error>> {
    let clock = ManualClock::new(1_700_000_000_000);
    let port: Arc<dyn ClockPort> = Arc::new(clock.clone());
    assert_eq!(port.now_ms(), TimestampMs::new(1_700_000_000_000));
    assert_eq!(clock.monotonic_now(), Duration::ZERO);

    let wakeup_mono = clock.register_monotonic_wakeup(Duration::from_secs(10));
    let wakeup_utc = clock.register_utc_wakeup(TimestampMs::new(1_700_000_005_000));
    assert!(!wakeup_mono.is_triggered());
    assert!(!wakeup_utc.is_triggered());

    clock.advance_ms(5_000);
    assert_eq!(port.now_ms(), TimestampMs::new(1_700_000_005_000));
    assert_eq!(clock.monotonic_ms(), 5_000);
    assert!(wakeup_utc.is_triggered());
    assert!(!wakeup_mono.is_triggered());

    // Recul de l'horloge murale : le temps monotone ne doit JAMAIS reculer.
    clock.rewind_utc_ms(120_000);
    assert_eq!(port.now_ms(), TimestampMs::new(1_699_999_885_000));
    assert_eq!(clock.monotonic_ms(), 5_000);

    // Synchronisation inter-threads par Condvar sans aucun thread::sleep.
    let worker_clock = clock.clone();
    let handle = std::thread::spawn(move || {
        worker_clock.wait_for_monotonic_advance(Duration::from_secs(5), Duration::from_secs(5))
    });

    assert!(clock.wait_for_waiters(1, Duration::from_secs(5)));
    clock.advance(Duration::from_secs(5));
    assert!(wakeup_mono.wait_bounded(Duration::from_secs(5)));
    assert_eq!(clock.monotonic_ms(), 10_000);

    let woke = handle.join().map_err(|_| "thread ManualClock a paniqué")?;
    assert!(woke);
    Ok(())
}

#[test]
fn test_deterministic_id_generator_reproducibility_and_uniqueness() {
    let seed = 0xCAFE_BABE_1234_5678;
    let g1 = DeterministicIdGenerator::new(seed);
    let g2 = DeterministicIdGenerator::new(seed);

    assert_eq!(g1.seed(), seed);
    assert_eq!(g1.new_command_id(), g2.new_command_id());
    assert_eq!(g1.new_run_id(), g2.new_run_id());
    assert_eq!(g1.new_step_id(), g2.new_step_id());
    assert_eq!(g1.new_task_id(), g2.new_task_id());
    assert_eq!(g1.new_agent_id(), g2.new_agent_id());
    assert_eq!(g1.new_session_id(), g2.new_session_id());
    assert_eq!(g1.new_invocation_id(), g2.new_invocation_id());
    assert_eq!(g1.new_action_id(), g2.new_action_id());
    assert_eq!(g1.next_id::<AttemptId>(), g2.next_id::<AttemptId>());
    assert_eq!(g1.generated_count(), 9);

    g1.reset();
    let first_after_reset = g1.new_command_id();
    assert_eq!(
        first_after_reset,
        DeterministicIdGenerator::new(seed).new_command_id()
    );

    let g_unique = DeterministicIdGenerator::new(seed);
    let mut set = HashSet::with_capacity(10_000);
    for _ in 0..10_000 {
        assert!(set.insert(g_unique.new_uuid()));
    }
    assert_eq!(set.len(), 10_000);

    let g_other = DeterministicIdGenerator::new(seed + 1);
    assert_ne!(
        g_other.new_uuid(),
        DeterministicIdGenerator::new(seed).new_uuid()
    );
}

#[test]
fn seed_panic_child_entrypoint() {
    if std::env::var(ENV_SEED_PANIC_CHILD).as_deref() != Ok("1") {
        return;
    }
    let id_gen = DeterministicIdGenerator::new(TEST_SEED_VALUE);
    let _id = id_gen.new_run_id();
    let _explicit_guard = id_gen.failure_guard();
    assert_eq!(
        1, 2,
        "échec volontaire pour vérifier l'impression de graine"
    );
}

#[test]
fn test_deterministic_id_generator_prints_seed_on_failure() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let output = Command::new(exe)
        .args(["seed_panic_child_entrypoint", "--exact", "--nocapture"])
        .env(ENV_SEED_PANIC_CHILD, "1")
        .output()?;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let expected_hex = format!("0x{TEST_SEED_VALUE:016x}");
    let expected_dec = TEST_SEED_VALUE.to_string();
    assert!(
        stderr.contains(&expected_hex) && stderr.contains(&expected_dec),
        "stderr attendu avec graine {expected_hex}/{expected_dec}, obtenu: {stderr}"
    );
    Ok(())
}

#[test]
fn test_cutpoint_controller_barriers_errors_and_all_variants() -> Result<(), Box<dyn Error>> {
    assert_eq!(Cutpoint::ALL.len(), 21);
    let expected_names = [
        "C0", "C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8", "C9", "C10", "C10a", "C10b", "C10c",
        "C10d", "C11", "C11a", "C11b", "C11c", "C11d", "C12",
    ];
    for (cp, name) in Cutpoint::ALL.iter().zip(expected_names.iter()) {
        assert_eq!(cp.as_str(), *name);
        assert_eq!(name.parse::<Cutpoint>()?, *cp);
    }

    let ctrl = CutpointController::new();
    ctrl.set_action(Cutpoint::C2, CutpointAction::Continue);
    ctrl.hit(Cutpoint::C2)?;

    ctrl.set_action_on_hit(Cutpoint::C7, 2, CutpointAction::Fail("outbox_down"))?;
    ctrl.hit(Cutpoint::C7)?;
    assert_eq!(
        ctrl.hit(Cutpoint::C7),
        Err(CutpointError::InjectedFailure {
            cutpoint: Cutpoint::C7,
            message: "outbox_down",
        })
    );

    // Barrière Condvar/Mutex sans aucun thread::sleep
    let barrier = ctrl.arm_barrier(Cutpoint::C10b, Duration::from_secs(5));
    let worker_ctrl = ctrl.clone();
    let handle = std::thread::spawn(move || worker_ctrl.hit(Cutpoint::C10b));

    ctrl.wait_for_hit(Cutpoint::C10b, 1, Duration::from_secs(5))?;
    barrier.wait_arrived(1, Duration::from_secs(5))?;
    barrier.release();

    let worker_res = handle
        .join()
        .map_err(|_| "thread CutpointBarrier a paniqué")?;
    assert_eq!(worker_res, Ok(()));
    assert!(barrier.is_released());

    // Expiration bornée d'une barrière non libérée
    let short_barrier = ctrl.arm_barrier(Cutpoint::C11a, Duration::from_millis(20));
    let timeout_res = ctrl.hit(Cutpoint::C11a);
    assert!(matches!(
        timeout_res,
        Err(CutpointError::BarrierTimeout {
            cutpoint: Cutpoint::C11a,
            ..
        })
    ));
    assert!(!short_barrier.is_released());

    // Annulation explicite d'une barrière
    let aborted_barrier = ctrl.arm_barrier(Cutpoint::C11c, Duration::from_secs(5));
    aborted_barrier.abort();
    assert_eq!(
        ctrl.hit(Cutpoint::C11c),
        Err(CutpointError::BarrierAborted {
            cutpoint: Cutpoint::C11c,
        })
    );
    Ok(())
}

#[test]
fn test_local_test_dir_and_unix_socket_sun_len() -> Result<(), Box<dyn Error>> {
    let dir = local_test_dir("unleash_really_long_prefix_that_must_be_bounded_safely")?;
    let path = dir.path();
    assert!(path.starts_with("/tmp"));

    let canonical = std::fs::canonicalize(path)?;
    assert!(
        canonical.as_os_str().len() < MACOS_SUN_LEN,
        "chemin canonique trop long: {}",
        canonical.display()
    );

    let sock_path = socket_path_in(path, "daemon.sock")?;
    assert!(sock_path.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_BYTES);

    #[cfg(unix)]
    {
        let _listener = std::os::unix::net::UnixListener::bind(&sock_path)?;
        assert!(sock_path.exists());
    }

    assert!(matches!(
        socket_path_in(path, "a/b.sock"),
        Err(TempDirError::InvalidSocketName { .. })
    ));
    assert!(matches!(
        socket_path_in(path, &format!("{}.sock", "x".repeat(100))),
        Err(TempDirError::PathTooLong { .. })
    ));
    Ok(())
}
