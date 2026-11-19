//! Historical V012 evidence boundaries in `SQLite`; no native ACK is fabricated.
use super::*;
use unleash_domain::{CommandId, ReconcileOutcome};
use unleash_protocol::ReasonCode;

fn unknown_control_after_epoch_change() -> (
    UnleashStore,
    TerminalControlRecord,
    CommitCommandRequest,
    impl std::fmt::Debug,
) {
    let (store, clock, _fault, control, original, directory) = prepared();
    clock.set_utc_ms(110);
    let (dispatch, unknown) = control_dispatch(&store, &control);
    let BrokerPhase::Dispatch { dispatch_id, .. } = dispatch.phase else {
        unreachable!()
    };
    store
        .resolve_terminal_control(ResolveTerminalControlRequest {
            action_id: control.context.action_id,
            dispatch_id,
            authority_generation: control.context.authority_generation,
            expected_revision: control.revision,
            context_digest: control.context_digest,
            caller: CallerAuthority {
                actor_id: control.context.actor_id.clone(),
                role: CallerRole::HumanOperator,
            },
        })
        .unwrap();
    store.commit_broker_phase(unknown).unwrap();
    // Model a daemon-only epoch change, retaining the physical boot. This is
    // a writer fixture, not a claim that a native rotation or restart occurred.
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute(
                "UPDATE daemon_state SET daemon_epoch=daemon_epoch+1 WHERE singleton=1",
                [],
            )?;
            tx.execute(
                "UPDATE resource_leases SET state='suspect' WHERE state IN ('held','reconciling')",
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
    (store, control, original, directory)
}

fn publish_unrelated_object(store: &UnleashStore) -> Sha256Digest {
    let bytes = b"opaque stored bytes are not a qualified native transfer ACK";
    let hash = Sha256Digest::from_bytes(bytes);
    let pin = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/octet-stream".into(),
            owner_kind: "fixture".into(),
            owner_id: "unrelated-object".into(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&pin, bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: pin.pin_id,
            object_hash: hash,
            object_generation: pin.object_generation,
            owner_kind: "fixture".into(),
            owner_id: "unrelated-object".into(),
            source_field: "opaque_bytes".into(),
        })
        .unwrap();
    hash
}

fn control_history_rows(store: &UnleashStore) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    store
        .read_snapshot(|conn, _| {
            [
                "actions",
                "action_dispatches",
                "terminal_control_intents",
                "terminal_sessions",
                "resource_leases",
                "resource_fences",
                "capacity_reservations",
                "broker_phase_records",
                "command_receipts",
                "events",
                "outbox",
            ]
            .into_iter()
            .map(|table| {
                let mut query = conn.prepare(&format!("SELECT * FROM {table} ORDER BY 1"))?;
                let columns = query.column_count();
                Ok(query
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get(index))
                            .collect::<Result<Vec<_>, _>>()
                    })?
                    .collect::<Result<Vec<_>, _>>()?)
            })
            .collect()
        })
        .unwrap()
}

fn generic_success(
    store: &UnleashStore,
    control: &TerminalControlRecord,
    evidence: Sha256Digest,
) -> CommitCommandRequest {
    let action = store
        .inspect_broker_action(control.context.action_id)
        .unwrap()
        .unwrap();
    let daemon = store.daemon_state().unwrap();
    let actor = control.context.actor_id.clone();
    let payload = json!({
        "action_id": action.action_id,
        "outcome": "confirmed_success",
        "evidence_digest": evidence,
        "reason": "inspect historical transfer evidence"
    });
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(CONTROL + 502),
            command_id: CommandId::from_u128(CONTROL + 501),
            authority_generation: daemon.authority_generation,
            actor_ref: None,
            project_id: Some(PROJECT),
            aggregate_id: action.action_id.as_uuid(),
            expected_revision: action.revision,
            command_type: "ReconcileAction".into(),
            payload: payload.as_object().unwrap().clone(),
        },
        target_aggregate: AggregateKind::Action,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::ReconcileAction {
            action_id: action.action_id,
            caller: CallerAuthority {
                actor_id: actor,
                role: CallerRole::HumanOperator,
            },
            outcome: ReconcileOutcome::ConfirmedSuccess,
            evidence_digest: Some(evidence),
            receipt_hash: None,
            reason: "inspect historical transfer evidence".into(),
            // The request is made by the current reconciler. An old source
            // epoch must not make this oracle pass at the stale-fence guard.
            expected_source_epoch: Some(daemon.daemon_epoch),
            expected_source_fence: Some(action.fence.fence),
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

#[test]
fn t_f011_003_historical_transfer_cannot_be_promoted_by_generic_success_digest() {
    let (store, control, original, _directory) = unknown_control_after_epoch_change();
    let evidence = publish_unrelated_object(&store);
    let before = store
        .inspect_broker_action(control.context.action_id)
        .unwrap()
        .unwrap();
    assert_eq!(before.state, ActionState::OutcomeUnknown);
    assert!(before.dispatch_id.is_some());
    assert!(store.daemon_state().unwrap().daemon_epoch > before.fence.epoch);
    assert_eq!(
        state(&store, &control),
        (0, control.context.previous_lease_id.to_string(), 2, 2, 3)
    );
    store
        .read_snapshot(|conn, _| {
            let held: i64 = conn.query_row(
                "SELECT COUNT(*) FROM resource_leases WHERE state='held'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(held, 0, "neither old nor planned lease is exercisable");
            Ok(())
        })
        .unwrap();
    let rows = control_history_rows(&store);
    let original_receipt = store
        .inspect_command_receipt(&original.actor_id, original.envelope.command_id)
        .unwrap();

    let result = store.commit_command(generic_success(&store, &control, evidence));
    let after = store
        .inspect_broker_action(control.context.action_id)
        .unwrap()
        .unwrap();
    assert!(
        matches!(&result, Err(error) if error.reason_code() == Some(ReasonCode::EvidenceUnavailable)),
        "unqualified object must be refused, not historical-success authority: result={result:?}, action_state={:?}",
        after.state
    );
    assert_eq!(after.state, ActionState::OutcomeUnknown);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.dispatch_id, before.dispatch_id);
    assert_eq!(control_history_rows(&store), rows);
    assert_eq!(
        store
            .inspect_command_receipt(&original.actor_id, original.envelope.command_id)
            .unwrap(),
        original_receipt
    );
    assert!(!store.config().data_root.join("process-hosts").exists());
}
