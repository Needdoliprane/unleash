//! Legacy observation compatibility, using explicitly synthetic SQLite facts.
//! This fixture makes no operating system identity or PTY claim.
use super::*;

#[test]
fn t_f011_005_acquisition_result_does_not_invent_legacy_native_input_state() {
    let fault = Arc::new(FailAt {
        armed: AtomicBool::new(false),
        point: StoreCutpoint::C1BeforeCommandCommit,
    });
    let (store, _config, clock, preparation, _directory) = fixture(fault);
    let request = request(&store, preparation, TERMINAL);
    let actor = request.actor_id.clone();
    let command = request.envelope.command_id;
    store.commit_command(request).unwrap();
    let allocation = read(&store, TERMINAL);
    clock.set_utc_ms(100);
    let dispatch = dispatch(&store, &allocation);
    store
        .resolve_terminal_launch(resolution(&allocation, dispatch))
        .unwrap();
    clock.set_utc_ms(105);
    let (observation, phase) = launch_observation_with_input_state_fixture(
        &store,
        &allocation,
        dispatch,
        101,
        103,
        false,
        None,
    );
    let encoded = serde_json::to_value(&observation).unwrap();
    assert!(!encoded.as_object().unwrap().contains_key("input_state"));
    let decoded: terminal_allocations::TerminalLaunchObservation =
        serde_json::from_value(encoded.clone()).unwrap();
    assert!(
        decoded.input_state.is_none(),
        "absence must never decode as native zero"
    );
    assert_eq!(
        canonical_sha256(&decoded).unwrap(),
        canonical_sha256(&encoded).unwrap(),
        "an absent optional field must preserve the pre-extension canonical digest"
    );
    store
        .commit_terminal_launch_observation(observation, phase)
        .unwrap();
    let ordinary = store.inspect_command_receipt(&actor, command).unwrap();
    let unleash_store::ReceiptInspectionOutcome::Full(inspection) = &ordinary else {
        panic!("retained receipt")
    };
    let reference = inspection.receipt.result_ref.as_ref().unwrap();
    let refused = store
        .inspect_terminal_acquisition_result(&actor, command, reference, PROJECT)
        .unwrap_err();
    assert!(
        matches!(refused, StoreError::EvidenceUnavailable(ref message)
        if message == "native launch input state was not observed")
    );
    assert_eq!(
        store.inspect_command_receipt(&actor, command).unwrap(),
        ordinary
    );
}
