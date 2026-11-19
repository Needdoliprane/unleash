//! A qualified first native receipt must permit reading the actual confirmed
//! sequence and preparing the next input. SQLite fixture, no PTY write claimed.
use super::*;

#[test]
fn t_f011_005_known_native_input_is_readable_and_admits_sequence_two() {
    let (store, clock, _fault, first, _directory) = executing_input();
    clock.set_utc_ms(120);
    let (native, phase) = native_observation(&store, &first, 110, 115, true);
    store
        .commit_terminal_input_observation(native, phase)
        .unwrap();
    let allocation = store
        .inspect_terminal_allocation_snapshot(first.context.terminal_id)
        .unwrap()
        .value
        .unwrap();
    let source: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT command_id FROM command_receipts WHERE receipt_id=?1",
                [allocation.context.receipt_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let source = source.parse().unwrap();
    let reference = format!("terminal-allocation:{}", first.context.terminal_id);
    let confirmed = store
        .inspect_terminal_acquisition_result(
            &first.context.actor_id,
            source,
            &reference,
            first.context.project_id,
        )
        .unwrap()
        .value;
    assert_eq!(confirmed.input_seq, 1);
    assert_eq!(confirmed.process_run_uuid, first.context.process_run_uuid);
    assert_eq!(confirmed.lease_id, first.context.lease_id);
    assert_eq!(confirmed.fence, first.context.resource_fence);
    assert_eq!(
        confirmed.lease_expires_at,
        first.context.proposed_lease_expires_at
    );
    let mut request = input_request(&store, &allocation, INPUT + 500);
    request.envelope.expected_revision = confirmed.control_revision;
    let CommandMutation::PrepareTerminalInput {
        request: payload, ..
    } = &mut request.mutation
    else {
        unreachable!()
    };
    payload.input_seq = 2;
    payload.lease_id = confirmed.lease_id;
    payload.fence = confirmed.fence;
    request.envelope.payload = serde_json::to_value(payload)
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    store.commit_command(request.clone()).unwrap();
    let second = store
        .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_ne!(second.context.action_id, first.context.action_id);
    assert_eq!(second.context.input_seq, 2);
    assert_eq!(
        second.context.process_run_uuid,
        first.context.process_run_uuid
    );
    assert_eq!(second.context.lease_id, confirmed.lease_id);
    assert_eq!(second.context.resource_fence, confirmed.fence);
    let marker = input_dispatch(&store, &second);
    assert_ne!(Some(marker), first.dispatch_id);
    let permit = store
        .resolve_terminal_input(resolution(&second, marker))
        .unwrap();
    assert_eq!(permit.input.state, TerminalInputState::WriteResolved);
    assert_eq!(permit.input.context.input_seq, 2);
    let facts = input_facts(&store, &first);
    assert_eq!((facts.0, facts.1), (1, confirmed.lease_expires_at.as_i64()));
    assert!(
        matches!(store.inspect_terminal_acquisition_result(&first.context.actor_id,source,&reference,first.context.project_id),Err(StoreError::ActionOutcomeUnknown {action_id,..}) if action_id==second.context.action_id)
    );
}
