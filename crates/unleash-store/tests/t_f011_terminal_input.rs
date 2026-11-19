//! Writer boundary tests. The initial routing oracle has no terminal, PID or
//! permission fixture: an unsupported input mutation must never create a receipt.

#![expect(
    clippy::unwrap_used,
    reason = "Assertions on an exclusively owned local Store"
)]

use serde_json::Map;
use unleash_domain::{ActorId, CommandId, RequestId};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;

#[test]
fn t_f011_005_input_name_cannot_commit_an_unrelated_noop_mutation() {
    let directory = local_test_dir("f011-input-writer-routing").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let daemon = store.daemon_state().unwrap();
    let actor = ActorId::new("uid:input-routing-fixture").unwrap();
    let command_id = CommandId::from_u128(0xf011_7001);
    let request = CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf011_7002),
            command_id,
            authority_generation: daemon.authority_generation,
            actor_ref: None,
            project_id: None,
            aggregate_id: daemon.authority_generation.as_uuid(),
            expected_revision: daemon.revision,
            command_type: "SendTerminalInput".into(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Daemon,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    };
    let outcome = store.commit_command(request);
    assert!(
        matches!(outcome, Err(StoreError::InvalidArgument {ref message,..})
        if message=="terminal input mutation missing"),
        "the input command name cannot attest an effect or intent through a different mutation: {outcome:?}"
    );
    assert_eq!(store.daemon_state().unwrap().revision, daemon.revision);
    assert!(matches!(
        store.inspect_command_receipt(&actor, command_id),
        Err(StoreError::NotFound(_))
    ));
}
