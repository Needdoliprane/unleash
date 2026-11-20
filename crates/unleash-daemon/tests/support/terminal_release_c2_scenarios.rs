//! Recovery of an accepted release whose executor never started.
use super::*;

// Short read-only snapshots leave no transaction open across daemon restarts.
fn c2_release_snapshot(data: &std::path::Path, command: CommandId) -> Value {
    let connection = rusqlite::Connection::open_with_flags(
        data.join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let text: String = connection.query_row(
        "SELECT json_object('action_id',r.action_id,'context',json(r.context_json),
          'digest',r.context_digest,'authority',r.authority_generation,'epoch',r.daemon_epoch,
          'state',r.state,'revision',r.revision,'dispatch',r.dispatch_id,
          'proof',r.observation_object_hash,'refusal',r.refusal_event_id,
          'disposition',r.disposition_event_id,
          'actions',(SELECT COUNT(*) FROM actions WHERE action_id=r.action_id),
          'phases',(SELECT COUNT(*) FROM broker_phase_records WHERE action_id=r.action_id),
          'dispatches',(SELECT COUNT(*) FROM action_dispatches WHERE action_id=r.action_id),
          'broker_dispatches',(SELECT COUNT(*) FROM broker_dispatches WHERE action_id=r.action_id),
          'proof_refs',(SELECT COUNT(*) FROM object_references WHERE owner_kind='terminal_release_observation' AND owner_id=r.action_id),
          'refusal_events',(SELECT COUNT(*) FROM events WHERE event_type='TerminalControlDispatchRefused' AND causation_receipt_id=r.receipt_id AND aggregate_id=r.action_id),
          'success_events',(SELECT COUNT(*) FROM events WHERE event_type='ActionObserved' AND aggregate_id=r.action_id AND json_extract(payload_json,'$.state')='observed_success'))
         FROM terminal_release_intents r WHERE r.command_id=?1",
        [command.to_string()], |r| r.get(0),
    ).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn c2_reservation_identity(data: &std::path::Path) -> String {
    let connection = rusqlite::Connection::open_with_flags(
        data.join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT json_group_array(json_object('id',reservation_id,'pool',pool_id,
          'owner_type',owner_type,'owner_id',owner_id,'amount',amount))
         FROM (SELECT * FROM capacity_reservations ORDER BY reservation_id)",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn assert_c2_unstarted(pending: &Value) {
    assert_eq!(pending.get("state"), Some(&json!("prepared")));
    assert_eq!(pending.get("revision"), Some(&json!(1)));
    for field in ["dispatch", "proof", "refusal", "disposition"] {
        assert!(
            pending.get(field).is_some_and(Value::is_null),
            "unexpected C2 marker: {field}"
        );
    }
    for field in [
        "actions",
        "phases",
        "dispatches",
        "broker_dispatches",
        "proof_refs",
        "refusal_events",
        "success_events",
    ] {
        assert_eq!(
            pending.get(field),
            Some(&json!(0)),
            "C2 has not started: {field}"
        );
    }
}

fn assert_c2_refusal(pending: &Value, refused: &Value, event: &EventEnvelope) {
    assert_eq!(
        event.payload.get("reason"),
        Some(&json!("daemon_superseded"))
    );
    assert_eq!(refused.get("state"), Some(&json!("not_dispatched")));
    assert_eq!(refused.get("revision"), Some(&json!(2)));
    assert_eq!(refused.get("refusal"), Some(&json!(event.event_id)));
    assert_eq!(refused.get("refusal_events"), Some(&json!(1)));
    for field in ["context", "digest", "authority", "epoch", "action_id"] {
        assert_eq!(
            refused.get(field),
            pending.get(field),
            "historical coordinate retained: {field}"
        );
    }
    for field in ["dispatch", "proof", "disposition"] {
        assert!(refused.get(field).is_some_and(Value::is_null));
    }
    for field in [
        "actions",
        "phases",
        "dispatches",
        "broker_dispatches",
        "proof_refs",
        "success_events",
    ] {
        assert_eq!(
            refused.get(field),
            Some(&json!(0)),
            "no synthetic broker/native work: {field}"
        );
    }
}

fn assert_c2_kept_resources(before: &ControlFacts, after: &ControlFacts) {
    assert_eq!(
        (
            &after.lease,
            after.expiry,
            after.fence,
            after.revision,
            after.processes,
            after.active_holds,
            after.leases,
            after.dispatches
        ),
        (
            &before.lease,
            before.expiry,
            before.fence,
            before.revision,
            before.processes,
            before.active_holds,
            before.leases,
            before.dispatches
        )
    );
    assert_eq!(
        after.lease_state, "suspect",
        "restart quarantines the original controller, never releases or reactivates it"
    );
}

fn assert_c2_late_dispatch_refused(data: &std::path::Path, pending: &Value) {
    let mut connection = rusqlite::Connection::open(data.join("unleash-authority.db")).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let tx = connection.transaction().unwrap();
    let late = tx.execute(
        "INSERT INTO action_dispatches(dispatch_id,action_id,ordinal,adapter_id,
          request_hash,started_at_ms,transport_state)
         VALUES(?1,?2,1,'owned-late-release',?3,1,'dispatched')",
        rusqlite::params![
            Uuid::from_u128(0xf011_81a2).to_string(),
            pending.get("action_id").and_then(Value::as_str).unwrap(),
            pending.get("digest").and_then(Value::as_str).unwrap()
        ],
    );
    let error = late.expect_err("the old release must never gain a late dispatch");
    assert!(
        error
            .to_string()
            .contains("terminal release refusal forbids a dispatch"),
        "a generic FK failure is not the expected no-dispatch proof: {error}"
    );
    tx.rollback().unwrap();
}

async fn assert_passive_replay(
    fixture: &mut LiveFixture,
    request: &CommandEnvelope,
    receipt: &CommandReceipt,
    refused: &Value,
    attempts: usize,
) {
    for _ in 0..attempts {
        assert_eq!(
            fixture.client.send_command(request).await.unwrap(),
            *receipt
        );
        assert_eq!(
            c2_release_snapshot(fixture.data.path(), request.command_id),
            *refused
        );
    }
}

fn assert_native_control_unchanged(
    fixture: &LiveFixture,
    native: &unleash_process_host::HostManifest,
    previous_secret: &HostControlSecret,
) {
    let current = fixture.host_manifest();
    assert_eq!(current.control_state, native.control_state);
    assert_eq!(current.daemon_epoch, native.daemon_epoch);
    assert_eq!(current.highest_fence, native.highest_fence);
    let secret = unleash_process_host::load_control_secret(
        &native
            .socket_path
            .parent()
            .unwrap()
            .join(unleash_process_host::CONTROL_SECRET_FILENAME),
    )
    .unwrap();
    assert!(
        secret.expose_secret() == previous_secret.expose_secret(),
        "C2 disposition must not rotate a native capability"
    );
}

#[tokio::test]
async fn t_f011_010_release_c2_disposes_unstarted_intent_once_without_native_retirement() {
    let arm_root = local_test_dir("release-receipt-c2").unwrap();
    let armed = arm_root.path().join("armed");
    let mut fixture = renewal_fixture::start_with_receipt_cutpoint(&armed).await;
    fixture.provision(true).await;
    let acquired = acquire(&mut fixture).await;
    let native = fixture.host_manifest();
    assert!(
        matches!(
            native.control_state,
            Some(unleash_process_host::HostControlState::Active { .. })
        ),
        "the packaged sibling must expose the current release protocol"
    );
    let before = facts(&fixture, &acquired);
    let initial_authority = fixture.authority;
    let initial_epoch = fixture.client.hello().daemon_epoch;
    let old = OldNativeControl::authenticate(&fixture);
    let orphan = release_request(&fixture, &acquired, 0xf011_81a0);

    let reservations_before = c2_reservation_identity(fixture.data.path());
    unleash_platform::write_private_file_atomic(&armed, b"armed").unwrap();
    assert!(
        fixture.client.send_command(&orphan).await.is_err(),
        "C2 loses the response only after committing the command"
    );
    // C2 returns an injected error after commit; handle_command_frame must
    // therefore never call start_release. The fixture retains its arm file.
    std::fs::remove_file(&armed).unwrap();
    let committed = fixture
        .client
        .get_command_receipt(RequestId::from_u128(0xf011_81a1), orphan.command_id)
        .await
        .expect("exact receipt survives lost IPC acknowledgement")
        .receipt;
    assert_eq!(committed.state, ReceiptState::Accepted);
    let pending = c2_release_snapshot(fixture.data.path(), orphan.command_id);
    assert_eq!(pending.get("authority"), Some(&json!(initial_authority)));
    assert_eq!(pending.get("epoch"), Some(&json!(initial_epoch)));
    assert_c2_unstarted(&pending);
    assert_eq!(facts(&fixture, &acquired), before);
    same_child(&fixture, &native);
    assert_eq!(fixture.host_manifest().control_state, native.control_state);

    fixture.restart_daemon_only().await;
    assert_eq!(fixture.authority, initial_authority);
    assert!(fixture.client.hello().daemon_epoch > initial_epoch);
    let event = terminal_event(&fixture, &committed, |event| {
        event.event_type == "TerminalControlDispatchRefused"
            && event.payload.get("receipt_id") == Some(&json!(committed.receipt_id))
            && event.payload.get("operation") == Some(&json!("release"))
    })
    .await;
    let refused = c2_release_snapshot(fixture.data.path(), orphan.command_id);
    assert_c2_refusal(&pending, &refused, &event);
    let after = facts(&fixture, &acquired);
    assert_c2_kept_resources(&before, &after);
    assert_eq!(
        c2_reservation_identity(fixture.data.path()),
        reservations_before
    );
    same_child(&fixture, &native);
    assert_native_control_unchanged(&fixture, &native, &old.secret);

    assert_passive_replay(&mut fixture, &orphan, &committed, &refused, 2).await;
    fixture.restart_daemon_only().await;
    assert_passive_replay(&mut fixture, &orphan, &committed, &refused, 1).await;
    assert_eq!(facts(&fixture, &acquired), after);
    assert_eq!(
        c2_reservation_identity(fixture.data.path()),
        reservations_before
    );
    same_child(&fixture, &native);
    assert_eq!(fixture.host_manifest().control_state, native.control_state);

    // Only an explicit acquisition may install a successor after the refusal.
    let reacquire = registered(
        &fixture,
        &serde_json::to_value(&acquired).unwrap(),
        0xf011_81a3,
    );
    let receipt = fixture.client.send_command(&reacquire).await.unwrap();
    let successor = observed_acquisition(&mut fixture, &receipt).await;
    assert_eq!(successor.process_run_uuid, acquired.process_run_uuid);
    assert_ne!(successor.lease_id, acquired.lease_id);
    assert!(successor.fence > acquired.fence);

    // Last check is a rollback-only native DB constraint probe, AFTER stopping
    // the daemon writer. It proves the durable no-dispatch marker survives and
    // blocks the exact old action, not a fresh action or current-epoch substitute.
    drain(&mut fixture).await;
    assert_eq!(
        c2_release_snapshot(fixture.data.path(), orphan.command_id),
        refused
    );
    same_child(&fixture, &native);
    assert_c2_late_dispatch_refused(fixture.data.path(), &pending);
    assert_eq!(
        c2_release_snapshot(fixture.data.path(), orphan.command_id),
        refused
    );
}
