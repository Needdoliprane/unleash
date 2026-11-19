//! Unknown input blocks usable causal assertions for the same physical run.
//! These are SQLite-only observations, with no native write or PTY claimed.
use super::*;
use unleash_domain::ActionState;

fn mark_unknown(store: &UnleashStore, record: &TerminalInputRecord, dispatch_id: DispatchId) {
    let first = input_authorizing(record);
    let BrokerPhase::Authorizing { intent } = &first.phase else {
        unreachable!()
    };
    let snapshot = store
        .inspect_broker_action(record.context.action_id)
        .unwrap()
        .unwrap();
    store
        .commit_broker_phase(BrokerPhaseRequest {
            expected_revision: snapshot.revision,
            phase: BrokerPhase::Observation {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                receipt: BrokerActionReceipt {
                    action_id: intent.action_id,
                    evaluation_id: first.evaluation.evaluation_id,
                    dispatch_id,
                    decision_ref: "synthetic-sqlite-input-allow".into(),
                    state: ActionState::OutcomeUnknown,
                    output_digest: None,
                    enforcement_refs: vec![],
                    remote_audit_ref: None,
                    remote_audit_unavailable: false,
                    observed_at_ms: store.config().clock.now_ms(),
                },
                observation_revision: Revision::new(1),
                cause: BrokerObservationCause::Execution,
                reason: Some(unleash_protocol::ReasonCode::TerminalInputUnknown.to_string()),
            },
            ..first.clone()
        })
        .unwrap();
}

fn unknown_input() -> (UnleashStore, TerminalInputRecord, impl std::fmt::Debug) {
    let (store, record, directory) = prepared_input();
    let dispatch = input_dispatch(&store, &record);
    store
        .resolve_terminal_input(resolution(&record, dispatch))
        .unwrap();
    mark_unknown(&store, &record, dispatch);
    let record = store
        .inspect_terminal_input_request(&record.context.actor_id, record.context.command_id)
        .unwrap()
        .value
        .unwrap();
    (store, record, directory)
}

fn initial_query(store: &UnleashStore, record: &TerminalInputRecord) -> (CommandId, String) {
    let allocation = store
        .inspect_terminal_allocation_snapshot(record.context.terminal_id)
        .unwrap()
        .value
        .unwrap();
    let command: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT command_id FROM command_receipts WHERE receipt_id=?1",
                [allocation.context.receipt_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    (
        command.parse().unwrap(),
        format!("terminal-allocation:{}", record.context.terminal_id),
    )
}

fn transfer_without_disposing_input(
    store: &UnleashStore,
    terminal: TerminalId,
) -> (CommandId, String) {
    let allocation = store
        .inspect_terminal_allocation_snapshot(terminal)
        .unwrap()
        .value
        .unwrap();
    // SQLite-only daemon succession. There was no native write in this fixture;
    // an empty native summary does not dispose the exact broker Unknown action.
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute(
                "UPDATE daemon_state SET daemon_epoch=daemon_epoch+1 WHERE singleton=1",
                [],
            )?;
            tx.execute(
                "UPDATE resource_leases SET state='suspect' WHERE state='held'",
                [],
            )?;
            tx.execute(
                "UPDATE capacity_reservations SET state='reconciling' WHERE state='held'",
                [],
            )?;
            tx.execute(
                "UPDATE processes SET state='suspect' WHERE state='live'",
                [],
            )?;
            tx.execute(
                "UPDATE terminal_sessions SET state='suspect' WHERE state='ready'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let request = registered_request(store, &allocation, INPUT + 100);
    let receipt = store
        .commit_command(request.clone())
        .unwrap()
        .receipt()
        .clone();
    let control = store
        .inspect_terminal_control_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    let (dispatch, _) = control_dispatch(store, &control);
    let BrokerPhase::Dispatch { dispatch_id, .. } = &dispatch.phase else {
        unreachable!()
    };
    store
        .resolve_terminal_control(ResolveTerminalControlRequest {
            action_id: control.context.action_id,
            dispatch_id: *dispatch_id,
            authority_generation: control.context.authority_generation,
            expected_revision: control.revision,
            context_digest: control.context_digest,
            caller: CallerAuthority {
                actor_id: request.actor_id,
                role: CallerRole::HumanOperator,
            },
        })
        .unwrap();
    observe_synthetic_transfer(store, &control, *dispatch_id);
    (receipt.command_id, receipt.result_ref.unwrap())
}

fn observe_synthetic_transfer(
    store: &UnleashStore,
    control: &TerminalControlRecord,
    dispatch: DispatchId,
) {
    let c = &control.context;
    let allocation = store
        .inspect_terminal_allocation_snapshot(c.terminal_id)
        .unwrap()
        .value
        .unwrap();
    let observation = terminal_allocations::TerminalControlObservation {
        action_id: c.action_id,
        dispatch_id: dispatch,
        context_digest: control.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        os_identity: c.expected_os_identity.clone(),
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.next_fence,
        transfer_ack_digest: Sha256Digest::from_bytes(b"synthetic SQLite transfer ACK"),
        manifest_digest: Sha256Digest::from_bytes(b"synthetic SQLite manifest"),
        effective_environment_digest: allocation
            .context
            .preparation
            .process_spec
            .environment
            .digest,
        durable_log_seq: 0,
        last_confirmed_input_seq: 0,
        unknown_input_seqs: vec![],
        observed_at_ms: store.config().clock.now_ms(),
    };
    let hash = canonical_sha256(&observation).unwrap();
    let bytes = canonicalize_to_vec(&observation).unwrap();
    let pin = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".into(),
            owner_kind: "terminal_control_observation".into(),
            owner_id: c.action_id.to_string(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&pin, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: pin.pin_id,
            object_hash: hash,
            object_generation: pin.object_generation,
            owner_kind: "terminal_control_observation".into(),
            owner_id: c.action_id.to_string(),
            source_field: dispatch.to_string(),
        })
        .unwrap();
    let first = control_authorizing(control);
    let BrokerPhase::Authorizing { intent } = &first.phase else {
        unreachable!()
    };
    let current = store.inspect_broker_action(c.action_id).unwrap().unwrap();
    store
        .commit_terminal_control_observation(
            observation,
            BrokerPhaseRequest {
                expected_revision: current.revision,
                phase: BrokerPhase::Observation {
                    action_id: c.action_id,
                    intent_digest: intent.intent_digest,
                    receipt: BrokerActionReceipt {
                        action_id: c.action_id,
                        evaluation_id: first.evaluation.evaluation_id,
                        dispatch_id: dispatch,
                        decision_ref: "synthetic-sqlite-control-allow".into(),
                        state: ActionState::ObservedSuccess,
                        output_digest: Some(hash),
                        enforcement_refs: vec![],
                        remote_audit_ref: None,
                        remote_audit_unavailable: false,
                        observed_at_ms: store.config().clock.now_ms(),
                    },
                    observation_revision: Revision::new(1),
                    cause: BrokerObservationCause::Execution,
                    reason: None,
                },
                ..first.clone()
            },
        )
        .unwrap();
}

#[test]
fn t_f011_005_unknown_input_cannot_reissue_usable_acquisition_assertions() {
    let (store, record, _directory) = unknown_input();
    let (command, reference) = initial_query(&store, &record);
    let ordinary = store
        .inspect_command_receipt(&record.context.actor_id, command)
        .unwrap();
    let result = store.inspect_terminal_acquisition_result(
        &record.context.actor_id,
        command,
        &reference,
        record.context.project_id,
    );
    assert!(
        matches!(&result,Err(StoreError::ActionOutcomeUnknown {action_id,..}) if *action_id==record.context.action_id),
        "a dispatched input without a native receipt must not release usable control: {result:?}"
    );
    assert_eq!(
        store
            .inspect_command_receipt(&record.context.actor_id, command)
            .unwrap(),
        ordinary
    );
    let facts: (i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
                    [record.context.terminal_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                    [record.context.lease_id.to_string()],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(
        facts,
        (0, record.context.previous_lease_expires_at.as_i64())
    );
}

#[test]
fn t_f011_005_unknown_broker_phase_projects_unknown_without_releasing_input() {
    let (store, record, _directory) = unknown_input();
    assert_eq!(
        record.broker_action_state,
        Some(ActionState::OutcomeUnknown)
    );
    assert_eq!(record.state, TerminalInputState::OutcomeUnknown);
    assert!(record.dispatch_id.is_some());
    assert!(record.observation_object_hash.is_none());
    let allocation = store
        .inspect_terminal_allocation_snapshot(record.context.terminal_id)
        .unwrap()
        .value
        .unwrap();
    assert!(matches!(
        store.commit_command(input_request(&store, &allocation, INPUT + 90)),
        Err(StoreError::CapacityBlocked(_))
    ));
}

#[test]
fn t_f011_005_unknown_input_does_not_disclose_a_foreign_causal_result() {
    let (store, record, _directory) = unknown_input();
    let (command, reference) = initial_query(&store, &record);
    let other = ActorId::new("uid:fixture-bob").unwrap();
    for (actor, project, reference) in [
        (other, record.context.project_id, reference.clone()),
        (
            record.context.actor_id.clone(),
            ProjectId::from_u128(INPUT + 99),
            reference.clone(),
        ),
        (
            record.context.actor_id.clone(),
            record.context.project_id,
            format!("{reference}.different"),
        ),
    ] {
        assert!(matches!(
            store.inspect_terminal_acquisition_result(&actor, command, &reference, project),
            Err(StoreError::NotFound(_))
        ));
    }
}

#[test]
fn t_f011_005_unknown_input_survives_control_rotation_of_the_same_physical_run() {
    let (store, record, _directory) = unknown_input();
    let (command, reference) = transfer_without_disposing_input(&store, record.context.terminal_id);
    let current = store.daemon_state().unwrap();
    assert!(current.daemon_epoch > record.context.daemon_epoch);
    let ordinary = store
        .inspect_command_receipt(&record.context.actor_id, command)
        .unwrap();
    let result = store.inspect_terminal_acquisition_result(
        &record.context.actor_id,
        command,
        &reference,
        record.context.project_id,
    );
    assert!(
        matches!(&result,Err(StoreError::ActionOutcomeUnknown {action_id,..}) if *action_id==record.context.action_id),
        "rotating control cannot erase the old physical run's unknown input: {result:?}"
    );
    assert_eq!(
        store
            .inspect_command_receipt(&record.context.actor_id, command)
            .unwrap(),
        ordinary
    );
}

#[test]
fn t_f011_005_qualified_transferred_control_admits_one_new_input_permit() {
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let (command, reference) =
        transfer_without_disposing_input(&store, allocation.context.terminal_id);
    let control = store
        .inspect_terminal_control_request(&allocation.context.actor_id, command)
        .unwrap()
        .value
        .unwrap();
    let qualified = store
        .inspect_terminal_acquisition_result(
            &allocation.context.actor_id,
            command,
            &reference,
            PROJECT,
        )
        .unwrap()
        .value;
    assert_eq!(qualified.input_seq, 0);
    assert_eq!(qualified.lease_id, control.context.next_lease_id);
    assert_eq!(qualified.fence, control.context.next_fence);
    let mut request = input_request(&store, &allocation, INPUT + 300);
    request.envelope.expected_revision = qualified.control_revision;
    // Old explicit assertions do not fall back to the newly held controller.
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::StaleFence { .. })
    ));
    let CommandMutation::PrepareTerminalInput {
        request: payload, ..
    } = &mut request.mutation
    else {
        unreachable!()
    };
    payload.lease_id = qualified.lease_id;
    payload.fence = qualified.fence;
    request.envelope.payload = serde_json::to_value(payload)
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    let receipt = store
        .commit_command(request.clone())
        .unwrap()
        .receipt()
        .clone();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let input = store
        .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(input.context.lease_id, qualified.lease_id);
    assert_eq!(input.context.resource_fence, qualified.fence);
    assert_eq!(input.context.daemon_epoch, qualified.daemon_epoch);
    assert_eq!(input.context.control_revision, qualified.control_revision);
    assert_eq!(
        input.context.process_run_uuid,
        allocation.context.process_run_uuid
    );
    let marker = input_dispatch(&store, &input);
    let permit = store
        .resolve_terminal_input(resolution(&input, marker))
        .unwrap();
    assert_eq!(permit.input.state, TerminalInputState::WriteResolved);
    assert_eq!(permit.input.dispatch_id, Some(marker));
    let (sequence, deadline): (i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
                    [qualified.terminal_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                    [qualified.lease_id.to_string()],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(
        (sequence, deadline),
        (0, qualified.lease_expires_at.as_i64())
    );
}
