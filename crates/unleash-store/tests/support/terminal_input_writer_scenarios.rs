//! T-F011-005 writer only. The observed allocation is the existing SQLite
//! fixture with synthetic OS facts; no native write is performed here.
use super::*;
use unleash_domain::{CommandId, LeaseId};
use unleash_store::terminal_allocations::terminal_input_types::{
    TerminalInputRequest, TerminalInputState,
};

const INPUT: u128 = 0xf011_7100;

fn input_request(
    store: &UnleashStore,
    allocation: &TerminalAllocationRecord,
    n: u128,
) -> CommitCommandRequest {
    let input = TerminalInputRequest {
        terminal_id: allocation.context.terminal_id,
        process_run_uuid: allocation.context.process_run_uuid,
        lease_id: allocation.context.lease_id,
        fence: allocation.context.resource_fence,
        input_seq: 1,
        payload_bytes: b"private-input-sentinel\0\xff".to_vec(),
    };
    let actor = allocation.context.actor_id.clone();
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(n + 1000),
            command_id: CommandId::from_u128(n),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: None,
            project_id: Some(PROJECT),
            aggregate_id: input.terminal_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "SendTerminalInput".into(),
            payload: serde_json::to_value(&input)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        },
        target_aggregate: AggregateKind::TerminalControl,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalInput {
            caller: CallerAuthority {
                actor_id: actor,
                role: CallerRole::HumanOperator,
            },
            request: input,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn control_facts(
    store: &UnleashStore,
    allocation: &TerminalAllocationRecord,
) -> (i64, i64, i64, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT input_seq FROM terminal_sessions WHERE terminal_id=?1",
                    [allocation.context.terminal_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row(
                    "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                    [allocation.context.lease_id.to_string()],
                    |row| row.get(0),
                )?,
                conn.query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| {
                    row.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM actions", [], |row| row.get(0))?,
            ))
        })
        .unwrap()
}

#[test]
fn t_f011_005_input_intent_has_no_bytes_no_effect_and_exact_replay_precedes_expiry() {
    let (store, clock, _fault, allocation, _directory) = observed(false);
    let before = control_facts(&store, &allocation);
    let request = input_request(&store, &allocation, INPUT);
    let receipt = store
        .commit_command(request.clone())
        .unwrap()
        .receipt()
        .clone();
    assert_eq!(receipt.state, ReceiptState::Accepted);
    assert_eq!(receipt.new_revision, Revision::INITIAL);
    let intent = store
        .inspect_terminal_input_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(intent.state, TerminalInputState::Prepared);
    assert_eq!(intent.revision, Revision::new(1));
    assert_eq!(intent.context.receipt_id, receipt.receipt_id);
    assert_eq!(intent.context.input_seq, 1);
    assert_eq!(intent.context.payload_byte_length, 24);
    assert_eq!(
        receipt.result_ref,
        Some(format!("terminal-input:{}", intent.context.action_id))
    );
    let persisted = store.read_snapshot(|conn, _| Ok((
        conn.query_row("SELECT context_json FROM terminal_input_intents WHERE action_id=?1", [intent.context.action_id.to_string()], |row| row.get::<_,String>(0))?,
        conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_type='terminal_input_intent' AND aggregate_id=?1 AND aggregate_revision=1", [intent.context.action_id.to_string()], |row| row.get::<_,i64>(0))?,
    ))).unwrap();
    let encoded: serde_json::Value = serde_json::from_str(&persisted.0).unwrap();
    assert!(encoded.get("payload_bytes").is_none());
    assert!(!persisted.0.contains("private-input-sentinel"));
    assert_eq!(persisted.1, 1);
    assert_eq!(control_facts(&store, &allocation), before);
    clock.set_utc_ms(before.1);
    assert_eq!(
        store.commit_command(request.clone()).unwrap().receipt(),
        &receipt
    );
    assert_eq!(control_facts(&store, &allocation), before);
    let mut changed = request;
    changed
        .envelope
        .payload
        .insert("input_seq".into(), json!(2));
    if let CommandMutation::PrepareTerminalInput { request, .. } = &mut changed.mutation {
        request.input_seq = 2;
    }
    assert!(matches!(
        store.commit_command(changed),
        Err(StoreError::IdempotencyConflict { .. })
    ));
}

#[test]
fn t_f011_005_input_rejects_scope_revision_sequence_fence_and_permission_without_intent() {
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let before = control_facts(&store, &allocation);
    for case in 0..7_u128 {
        let mut request = input_request(&store, &allocation, INPUT + 10 + case);
        match case {
            0 => request.envelope.project_id = Some(ProjectId::from_u128(0xbad)),
            1 => request.envelope.expected_revision = Revision::new(1),
            2 | 3 | 4 => {
                let CommandMutation::PrepareTerminalInput { request: input, .. } =
                    &mut request.mutation
                else {
                    unreachable!()
                };
                match case {
                    2 => input.input_seq = 2,
                    3 => input.fence = Fence::new(input.fence.as_u64() + 1),
                    _ => input.lease_id = LeaseId::from_u128(0xbad),
                }
                request.envelope.payload = serde_json::to_value(input)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone();
            }
            5 => {
                request.actor_id = ActorId::new("uid:fixture-bob").unwrap();
                let CommandMutation::PrepareTerminalInput { caller, .. } = &mut request.mutation
                else {
                    unreachable!()
                };
                caller.actor_id = request.actor_id.clone();
            }
            _ => {
                let CommandMutation::PrepareTerminalInput { caller, .. } = &mut request.mutation
                else {
                    unreachable!()
                };
                caller.role = CallerRole::Agent;
            }
        }
        let actor = request.actor_id.clone();
        let command = request.envelope.command_id;
        assert!(
            store.commit_command(request).is_err(),
            "input authority case {case}"
        );
        assert!(
            store
                .inspect_terminal_input_request(&actor, command)
                .unwrap()
                .value
                .is_none()
        );
        assert!(matches!(
            store.inspect_command_receipt(&actor, command),
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(control_facts(&store, &allocation), before);
    }
}

#[test]
fn t_f011_005_input_preparation_reserves_sequence_until_qualified_outcome() {
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let before = control_facts(&store, &allocation);
    store
        .commit_command(input_request(&store, &allocation, INPUT + 30))
        .unwrap();
    let other = input_request(&store, &allocation, INPUT + 31);
    assert!(matches!(
        store.commit_command(other),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert_eq!(control_facts(&store, &allocation), before);
}
