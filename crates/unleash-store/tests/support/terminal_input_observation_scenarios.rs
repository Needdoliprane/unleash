//! Native receipt projection only: the response values are synthetic `SQLite`
//! fixture data. This file never claims an actual host write or PTY effect.
use super::*;

#[path = "terminal_input_followup_scenarios.rs"]
mod followup_scenarios;
use unleash_store::terminal_allocations::terminal_input_types::{
    TerminalInputObservation, TerminalInputRenewal, TerminalNativeInputReceipt,
};

fn executing_input() -> (
    UnleashStore,
    Arc<ManualClock>,
    Arc<FailAt>,
    TerminalInputRecord,
    impl std::fmt::Debug,
) {
    let (store, clock, fault, allocation, directory) = observed(false);
    let request = input_request(&store, &allocation, INPUT + 400);
    store.commit_command(request.clone()).unwrap();
    let input = store
        .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    let marker = input_dispatch(&store, &input);
    let input = store
        .resolve_terminal_input(resolution(&input, marker))
        .unwrap()
        .input;
    (store, clock, fault, input, directory)
}

fn native_observation(
    store: &UnleashStore,
    input: &TerminalInputRecord,
    captured: i64,
    received: i64,
    publish: bool,
) -> (TerminalInputObservation, BrokerPhaseRequest) {
    let c = &input.context;
    let marker = input.dispatch_id.unwrap();
    let native = TerminalNativeInputReceipt {
        input_seq: c.input_seq,
        command_id: c.command_id,
        action_id: c.action_id,
        receipt_id: unleash_domain::ReceiptId::from_u128(INPUT + 401),
        payload_hash: c.payload_digest,
    };
    let native_response_digest =
        canonical_sha256(&json!({"synthetic_native_response":native,"deduplicated":false}))
            .unwrap();
    let observation = TerminalInputObservation {
        action_id: c.action_id,
        dispatch_id: marker,
        context_digest: input.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.resource_fence,
        native_receipt: native,
        native_response_digest,
        observed_at_ms: TimestampMs::new(captured),
    };
    let hash = canonical_sha256(&observation).unwrap();
    if publish {
        publish_native_observation(store, &observation);
    }
    let first = input_authorizing(input);
    let BrokerPhase::Authorizing { intent } = &first.phase else {
        unreachable!()
    };
    let current = store.inspect_broker_action(c.action_id).unwrap().unwrap();
    let phase = BrokerPhaseRequest {
        expected_revision: current.revision,
        phase: BrokerPhase::Observation {
            action_id: c.action_id,
            intent_digest: intent.intent_digest,
            receipt: BrokerActionReceipt {
                action_id: c.action_id,
                evaluation_id: first.evaluation.evaluation_id,
                dispatch_id: marker,
                decision_ref: "synthetic-sqlite-input-allow".into(),
                state: unleash_domain::ActionState::ObservedSuccess,
                output_digest: Some(hash),
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: false,
                observed_at_ms: TimestampMs::new(received),
            },
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
        },
        ..first.clone()
    };
    (observation, phase)
}

fn publish_native_observation(store: &UnleashStore, observation: &TerminalInputObservation) {
    let hash = canonical_sha256(observation).unwrap();
    let bytes = canonicalize_to_vec(observation).unwrap();
    let pin = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".into(),
            owner_kind: "terminal_input_observation".into(),
            owner_id: observation.action_id.to_string(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&pin, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: pin.pin_id,
            object_hash: hash,
            object_generation: pin.object_generation,
            owner_kind: "terminal_input_observation".into(),
            owner_id: observation.action_id.to_string(),
            source_field: observation.dispatch_id.to_string(),
        })
        .unwrap();
}

fn input_facts(store: &UnleashStore, input: &TerminalInputRecord) -> (i64, i64, Option<i64>) {
    store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
                    [input.context.terminal_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                    [input.context.lease_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT ended_at_ms FROM action_dispatches WHERE dispatch_id=?1",
                    [input.dispatch_id.unwrap().to_string()],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap()
}

#[test]
fn t_f011_005_known_native_receipt_projects_seq_and_renews_once() {
    let (store, clock, _fault, input, _directory) = executing_input();
    clock.set_utc_ms(120);
    let (observation, phase) = native_observation(&store, &input, 110, 115, true);
    let committed = store
        .commit_terminal_input_observation(observation.clone(), phase.clone())
        .unwrap();
    assert!(!committed.replayed);
    let after = store
        .inspect_terminal_input_request(&input.context.actor_id, input.context.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(after.state, TerminalInputState::Observed);
    assert_eq!(after.renewal, Some(TerminalInputRenewal::Renewed));
    assert_eq!(
        after.renewed_lease_expires_at,
        Some(input.context.proposed_lease_expires_at)
    );
    assert_eq!(
        input_facts(&store, &input),
        (
            1,
            input.context.proposed_lease_expires_at.as_i64(),
            Some(115)
        )
    );
    clock.set_utc_ms(input.context.proposed_lease_expires_at.as_i64() + 1);
    let replay = store
        .commit_terminal_input_observation(observation, phase)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(
        input_facts(&store, &input),
        (
            1,
            input.context.proposed_lease_expires_at.as_i64(),
            Some(115)
        )
    );
}

#[test]
fn t_f011_005_late_known_native_receipt_retains_effect_without_reactivating_expired_lease() {
    let (store, clock, _fault, input, _directory) = executing_input();
    let expired = input.context.previous_lease_expires_at.as_i64();
    clock.set_utc_ms(expired + 2);
    let (observation, phase) = native_observation(&store, &input, expired - 1, expired + 1, true);
    store
        .commit_terminal_input_observation(observation.clone(), phase.clone())
        .unwrap();
    let after = store
        .inspect_terminal_input_request(&input.context.actor_id, input.context.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(after.state, TerminalInputState::Observed);
    assert_eq!(after.renewal, Some(TerminalInputRenewal::LeaseExpired));
    assert_eq!(after.renewed_lease_expires_at, None);
    assert_eq!(
        after.observation_object_hash,
        Some(canonical_sha256(&observation).unwrap())
    );
    assert_eq!(input_facts(&store, &input), (1, expired, Some(expired + 1)));
    assert!(
        store
            .commit_terminal_input_observation(observation, phase)
            .unwrap()
            .replayed
    );
    assert_eq!(input_facts(&store, &input), (1, expired, Some(expired + 1)));
}

#[test]
fn t_f011_005_native_receipt_needs_exact_published_proof_and_causal_time() {
    for case in 0..5 {
        let (store, clock, _fault, input, _directory) = executing_input();
        clock.set_utc_ms(120);
        let before = input_facts(&store, &input);
        let (mut observation, mut phase) = native_observation(&store, &input, 110, 115, false);
        let BrokerPhase::Observation { receipt, .. } = &mut phase.phase else {
            unreachable!()
        };
        match case {
            0 => {}
            1 => observation.native_receipt.command_id = CommandId::from_u128(INPUT + 499),
            2 => {
                observation.native_receipt.payload_hash =
                    Sha256Digest::from_bytes(b"different raw bytes");
            }
            3 => observation.observed_at_ms = TimestampMs::new(104),
            _ => observation.observed_at_ms = TimestampMs::new(116),
        }
        if case != 0 {
            receipt.output_digest = Some(canonical_sha256(&observation).unwrap());
            publish_native_observation(&store, &observation);
        }
        let result = store.commit_terminal_input_observation(observation, phase);
        if case >= 3 {
            assert!(
                matches!(&result,Err(StoreError::InvalidArgument {message,..}) if message=="terminal input capture timestamp is outside dispatch and receipt"),
                "temporal guard {case}: {result:?}"
            );
        } else {
            assert!(result.is_err(), "native proof guard {case}");
        }
        assert_eq!(input_facts(&store, &input), before);
        let after = store
            .inspect_terminal_input_request(&input.context.actor_id, input.context.command_id)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(after.state, TerminalInputState::WriteResolved);
    }
}
