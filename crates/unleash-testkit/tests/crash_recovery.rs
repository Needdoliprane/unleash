//! Tests d'intégration du harnais de crash par processus enfant et de l'oracle externe.
// T-F002-001 / REQ-PERSIST-001
// T-F001-008 / REQ-CORE-008

use std::error::Error;
use std::path::Path;

use unleash_testkit::{
    ChildCrashContext, ChildCrashHarness, CrashHarnessError, CrashKillMode, Cutpoint,
    CutpointController, ExternalEffectOracle, NewExternalEffect, TempDirError, local_test_dir,
};

const CHILD_SCENARIO_ENV: &str = "UNLEASH_TEST_CRASH_SCENARIO";

#[test]
fn child_worker_entrypoint() -> Result<(), Box<dyn Error>> {
    let Some(ctx) = ChildCrashContext::from_env()? else {
        return Ok(());
    };

    let controller = CutpointController::new();
    let _bridge = ctx.arm_controller(&controller)?;

    let effect_key =
        std::env::var(CHILD_SCENARIO_ENV).unwrap_or_else(|_| String::from("action-default"));

    // Phase avant dispatch (C0 -> C1 -> C3) : aucun effet externe ne doit exister.
    controller.hit(Cutpoint::C0)?;
    controller.hit(Cutpoint::C1)?;
    controller.hit(Cutpoint::C3)?;

    // Point C4 : marqueur durable engagé, juste avant l'I/O externe.
    controller.hit(Cutpoint::C4)?;

    // Exécution de l'effet externe dans l'oracle indépendant, receipt perdu au retour (C5).
    ctx.oracle().record_effect_detailed(NewExternalEffect {
        effect_key,
        kind: String::from("external_mutation"),
        timestamp_ms: 1_767_225_600_123,
        action_id: Some(String::from("act-crash-test")),
        receipt_lost: true,
    })?;

    // Point C5 : effet externe accompli, crash avant persistance du receipt local.
    controller.hit(Cutpoint::C5)?;
    controller.hit(Cutpoint::C6)?;

    Ok(())
}

#[test]
fn test_child_crash_c4_abort_process_has_zero_external_effects() -> Result<(), Box<dyn Error>> {
    let (_oracle_dir_guard, oracle) = ExternalEffectOracle::create_in_temp()?;
    let mut spec = ChildCrashHarness::for_current_exe_test(
        "child_worker_entrypoint",
        Cutpoint::C4,
        CrashKillMode::AbortProcess,
        oracle.dir().to_path_buf(),
    )?;
    spec.env
        .push((String::from(CHILD_SCENARIO_ENV), String::from("c4-abort")));

    let outcome = ChildCrashHarness::run_until_crash(&spec)?;
    assert_eq!(outcome.target_cutpoint, Cutpoint::C4);
    assert_eq!(outcome.kill_mode, CrashKillMode::AbortProcess);
    if cfg!(unix) {
        assert!(
            outcome.is_sigabrt(),
            "Signal SIGABRT attendu, observé: {:?}",
            outcome.signal
        );
    }
    assert_eq!(outcome.oracle_count()?, 0);
    assert_eq!(oracle.count_for_key("c4-abort")?, 0);
    Ok(())
}

#[test]
fn test_child_crash_c4_sigkill_has_zero_external_effects_without_sleep()
-> Result<(), Box<dyn Error>> {
    let (_oracle_dir_guard, oracle) = ExternalEffectOracle::create_in_temp()?;
    let mut spec = ChildCrashHarness::for_current_exe_test(
        "child_worker_entrypoint",
        Cutpoint::C4,
        CrashKillMode::Sigkill,
        oracle.dir().to_path_buf(),
    )?;
    spec.env
        .push((String::from(CHILD_SCENARIO_ENV), String::from("c4-sigkill")));

    let outcome = ChildCrashHarness::run_until_crash(&spec)?;
    assert_eq!(outcome.target_cutpoint, Cutpoint::C4);
    assert_eq!(outcome.kill_mode, CrashKillMode::Sigkill);
    if cfg!(unix) {
        assert!(
            outcome.is_sigkill(),
            "Signal SIGKILL attendu, observé: {:?}",
            outcome.signal
        );
    }
    assert_eq!(outcome.oracle_count()?, 0);
    assert_eq!(oracle.count_for_key("c4-sigkill")?, 0);
    Ok(())
}

#[test]
fn test_child_crash_c5_sigkill_records_single_external_effect_without_sleep()
-> Result<(), Box<dyn Error>> {
    let (_oracle_dir_guard, oracle) = ExternalEffectOracle::create_in_temp()?;
    let mut spec = ChildCrashHarness::for_current_exe_test(
        "child_worker_entrypoint",
        Cutpoint::C5,
        CrashKillMode::Sigkill,
        oracle.dir().to_path_buf(),
    )?;
    spec.env
        .push((String::from(CHILD_SCENARIO_ENV), String::from("c5-sigkill")));

    let outcome = ChildCrashHarness::run_until_crash(&spec)?;
    assert_eq!(outcome.target_cutpoint, Cutpoint::C5);
    assert_eq!(outcome.kill_mode, CrashKillMode::Sigkill);
    if cfg!(unix) {
        assert!(
            outcome.is_sigkill(),
            "Signal SIGKILL attendu, observé: {:?}",
            outcome.signal
        );
    }

    assert_eq!(outcome.oracle_count()?, 1);
    assert_eq!(oracle.count_for_key("c5-sigkill")?, 1);
    let records = outcome.oracle()?.records()?;
    assert_eq!(records.len(), 1);
    let rec = records.first().ok_or("enregistrement oracle absent")?;
    assert_eq!(rec.sequence, 1);
    assert_eq!(rec.effect_key, "c5-sigkill");
    assert_eq!(rec.action_id.as_deref(), Some("act-crash-test"));
    assert!(rec.receipt_lost);
    Ok(())
}

#[test]
fn test_child_crash_c5_abort_process_records_single_external_effect() -> Result<(), Box<dyn Error>>
{
    let (_oracle_dir_guard, oracle) = ExternalEffectOracle::create_in_temp()?;
    let mut spec = ChildCrashHarness::for_current_exe_test(
        "child_worker_entrypoint",
        Cutpoint::C5,
        CrashKillMode::AbortProcess,
        oracle.dir().to_path_buf(),
    )?;
    spec.env
        .push((String::from(CHILD_SCENARIO_ENV), String::from("c5-abort")));

    let outcome = ChildCrashHarness::run_until_crash(&spec)?;
    if cfg!(unix) {
        assert!(outcome.is_sigabrt());
    }
    assert_eq!(outcome.oracle_count()?, 1);
    assert_eq!(oracle.count_for_key("c5-abort")?, 1);
    Ok(())
}

#[test]
fn test_oracle_lives_in_local_tmp_and_rejects_claude_dir() -> Result<(), Box<dyn Error>> {
    let dir = local_test_dir("oracle-int")?;
    assert!(dir.path().starts_with("/tmp"));
    let oracle = ExternalEffectOracle::open(dir.path())?;
    oracle.record_effect("k1", "op", 10)?;
    oracle.record_effect("k1", "op", 20)?;
    oracle.record_effect("k2", "op", 30)?;

    let reopened = ExternalEffectOracle::open(dir.path())?;
    assert_eq!(reopened.count()?, 3);
    assert_eq!(reopened.count_for_key("k1")?, 2);
    assert_eq!(reopened.count_for_key("k2")?, 1);

    let err = ExternalEffectOracle::open(Path::new("/tmp/.claude/forbidden"));
    assert!(matches!(
        err,
        Err(CrashHarnessError::TempDir(
            TempDirError::ForbiddenDocumentationPath { .. }
        ))
    ));
    Ok(())
}
