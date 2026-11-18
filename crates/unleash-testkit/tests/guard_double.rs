//! Tests du double déterministe `GuardPort` et de ses espions d'exécution/secrets (`F017`).
// T-F017-005 / AC-F017-005 / REQ-SEC-005

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
use unleash_action_broker::{
    GUARD_PORT_PROTOCOL_VERSION, GuardMappingProfile, GuardPort, MonotonicClockPort, SecretStore,
    SleepPort,
};
use unleash_domain::SecretRef;
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, ManualClock, ManualClockTimer,
    SecretResolutionSpy,
};

#[test]
fn dropped_manual_timer_cancels_its_wakeup() {
    let clock = ManualClock::new(0);
    let timer = ManualClockTimer::new(clock.clone());
    let future = timer.sleep(Duration::from_secs(2));
    assert_eq!(clock.pending_wakeup_count(), 1);
    drop(future);
    assert_eq!(clock.pending_wakeup_count(), 0);
}

#[tokio::test]
async fn guard_double_and_spies_enforce_zero_dispatch_and_deterministic_timeout() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let mono: &dyn MonotonicClockPort = clock.as_ref();
    assert_eq!(mono.monotonic_now(), Duration::ZERO);

    let timer = ManualClockTimer::new(clock.as_ref().clone());
    let sleep_fut = timer.sleep(Duration::from_millis(2_000));
    let clock_adv = clock.clone();
    let handle = tokio::spawn(async move {
        sleep_fut.await;
    });
    for _ in 0..50 {
        if clock_adv.pending_wakeup_count() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    clock_adv.advance(Duration::from_millis(2_000));
    handle.await.unwrap();

    let double = DeterministicGuardDouble::with_profile(clock, GuardMappingProfile::NoneAccepted);
    assert_eq!(double.protocol_version(), GUARD_PORT_PROTOCOL_VERSION);

    let spy = BrokerExecutionSpy::new();
    spy.assert_zero_dispatches();

    let sec_spy = SecretResolutionSpy::in_memory();
    let sref = SecretRef::new("os-secret:double/test").unwrap();
    sec_spy
        .store(&sref, SecretString::from("canary-double".to_owned()))
        .unwrap();
    sec_spy.assert_zero_secret_resolutions();
}
