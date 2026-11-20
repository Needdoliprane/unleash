//! Lost Release ACK on real instrumented host/daemon subprocesses and owned PTY.
//! Restart/replay proves quarantine only; no historical activation is inferred.
use super::*;
use std::os::unix::process::ExitStatusExt;

#[path = "terminal_release_history_native_scenarios.rs"]
mod history_native_scenarios;

#[derive(Debug, PartialEq, Eq)]
struct ReleaseSnapshot {
    action_id: String,
    dispatch_id: String,
    context_digest: String,
    evaluation_id: String,
    request_hash: String,
    dispatch_phase_digest: String,
    intent: String,
    action: String,
    observations: i64,
    success_phases: i64,
    success_events: i64,
    dispatches: i64,
    proof_references: i64,
    proof: Option<String>,
    refusal: Option<String>,
    disposition: Option<String>,
}

fn release_state(fixture: &LiveFixture, receipt: &CommandReceipt) -> ReleaseSnapshot {
    let conn = rusqlite::Connection::open_with_flags(
        fixture.data.path().join("unleash-authority.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    conn.query_row("SELECT r.action_id,r.dispatch_id,r.context_digest,b.evaluation_id,d.request_hash,phase.phase_digest,r.state,a.state,
      (SELECT COUNT(*) FROM broker_phase_records p WHERE p.action_id=r.action_id AND p.phase_key LIKE 'observation:%'),
      (SELECT COUNT(*) FROM broker_phase_records p WHERE p.action_id=r.action_id AND p.phase_key LIKE 'observation:%' AND json_extract(p.phase_json,'$.phase.receipt.state')='observed_success'),
      (SELECT COUNT(*) FROM events e WHERE e.aggregate_id=r.action_id AND e.event_type='ActionObserved' AND (json_extract(e.payload_json,'$.state')='observed_success' OR json_extract(e.payload_json,'$.terminal_release') IS NOT NULL)),
      (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=r.action_id),
      (SELECT COUNT(*) FROM object_references f WHERE f.owner_kind='terminal_release_observation' AND f.owner_id=r.action_id),
      r.observation_object_hash,r.refusal_event_id,r.disposition_event_id
      FROM terminal_release_intents r JOIN actions a ON a.action_id=r.action_id
      JOIN broker_dispatches b ON b.action_id=r.action_id AND b.dispatch_id=r.dispatch_id
      JOIN action_dispatches d ON d.action_id=r.action_id AND d.dispatch_id=r.dispatch_id
      JOIN broker_phase_records phase ON phase.action_id=r.action_id AND phase.evaluation_id=b.evaluation_id AND phase.phase_key='dispatch'
      WHERE r.receipt_id=?1",
      [receipt.receipt_id.to_string()],|row|Ok(ReleaseSnapshot {action_id:row.get(0)?,dispatch_id:row.get(1)?,context_digest:row.get(2)?,evaluation_id:row.get(3)?,request_hash:row.get(4)?,dispatch_phase_digest:row.get(5)?,intent:row.get(6)?,action:row.get(7)?,observations:row.get(8)?,success_phases:row.get(9)?,success_events:row.get(10)?,dispatches:row.get(11)?,proof_references:row.get(12)?,proof:row.get(13)?,refusal:row.get(14)?,disposition:row.get(15)?})).unwrap()
}

fn release_count(paths: &Paths) -> usize {
    match std::fs::read_to_string(paths.release_counter()) {
        Ok(text) => text.parse().unwrap(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("owned native retirement counter unavailable: {error}"),
    }
}

fn host_identity(paths: &Paths) -> unleash_platform::OsProcessIdentity {
    serde_json::from_slice(&std::fs::read(paths.host_identity()).unwrap()).unwrap()
}

fn same_live_host(
    fixture: &LiveFixture,
    paths: &Paths,
    host: &unleash_platform::OsProcessIdentity,
    native: &unleash_process_host::HostManifest,
) {
    assert_eq!(host_identity(paths), *host, "no replacement host startup");
    assert_eq!(
        unleash_platform::verify_process_identity(host).unwrap(),
        unleash_platform::ProcessVerification::Match
    );
    same_child(fixture, native);
    let current = fixture.host_manifest();
    assert_eq!(current.terminal_id, native.terminal_id);
    assert_eq!(
        (current.daemon_epoch, current.highest_fence),
        (native.daemon_epoch, native.highest_fence),
        "quarantine/replay must not transfer native control"
    );
}

fn retained_resources(before: &ControlFacts, after: &ControlFacts) {
    assert_eq!(
        (
            &after.lease,
            after.expiry,
            after.fence,
            after.revision,
            after.processes,
            after.active_holds,
            after.leases
        ),
        (
            &before.lease,
            before.expiry,
            before.fence,
            before.revision,
            before.processes,
            before.active_holds,
            before.leases
        )
    );
    assert_eq!(after.dispatches, before.dispatches + 1);
    assert!(
        matches!(after.lease_state.as_str(), "reconciling" | "suspect"),
        "no release or held controller may be inferred from a lost ACK: {after:?}"
    );
}

fn quarantined(before: &ReleaseSnapshot, now: &ReleaseSnapshot) {
    assert_eq!(
        (
            &now.action_id,
            &now.dispatch_id,
            &now.context_digest,
            &now.evaluation_id,
            &now.request_hash,
            &now.dispatch_phase_digest
        ),
        (
            &before.action_id,
            &before.dispatch_id,
            &before.context_digest,
            &before.evaluation_id,
            &before.request_hash,
            &before.dispatch_phase_digest
        )
    );
    assert_eq!(now.action, "outcome_unknown");
    assert!(matches!(
        now.intent.as_str(),
        "retire_resolved" | "outcome_unknown"
    ));
    assert_eq!(
        (
            now.dispatches,
            now.success_phases,
            now.success_events,
            now.proof_references
        ),
        (1, 0, 0, 0)
    );
    assert!(now.proof.is_none() && now.refusal.is_none() && now.disposition.is_none());
}

async fn wait_for_release_eof(paths: &Paths) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match std::fs::read(paths.release_eof()) {
            Ok(bytes) => {
                assert_eq!(bytes, RELEASE_EOF);
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("owned retirement EOF fact unavailable: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "the native callback must observe controller EOF, not just a delay"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn native_log(
    client: &mut HostClient,
    native: &unleash_process_host::HostManifest,
    secret: &HostControlSecret,
    after: u64,
) -> unleash_process_host::LogReadResult {
    let header = client.make_header(
        CommandId::from_u128(0xf011_8173),
        native.action_id,
        native.command_digest,
    );
    match client
        .call(&HostRequest::ReadLog {
            header,
            secret: secret.expose_secret().into(),
            after_offset: after,
            max_bytes: 65536,
            strict_cursor: true,
        })
        .unwrap()
    {
        HostResponse::LogChunk { result } => result,
        other => panic!("owned native log probe refused: {other:?}"),
    }
}

async fn fresh_pty_output(native: &unleash_process_host::HostManifest, after: u64) -> u64 {
    // Fixture-only read at the ORIGINAL native coordinates. No handshake,
    // rotation, input or current daemon capability is acquired by this probe.
    // ReadLog can sync its spool; it is not the quiescence proof (drain is).
    let secret = unleash_process_host::load_control_secret(
        &native
            .socket_path
            .parent()
            .unwrap()
            .join(unleash_process_host::CONTROL_SECRET_FILENAME),
    )
    .unwrap();
    let mut client = HostClient::connect(
        &native.socket_path,
        native.process_run_uuid,
        native.authority_generation,
        native.daemon_epoch,
        native.highest_fence,
    )
    .unwrap();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let result = native_log(&mut client, native, &secret, after);
        let bytes: Vec<u8> = result
            .segments
            .iter()
            .flat_map(|segment| segment.bytes.iter().copied())
            .collect();
        if result.volatile_head_offset > after
            && bytes
                .windows(b"live-terminal-output".len())
                .any(|window| window == b"live-terminal-output")
        {
            return result.volatile_head_offset;
        }
        assert!(
            Instant::now() < deadline,
            "the same live child must still produce actual PTY output after the boundary"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn native_retirement(fixture: &LiveFixture) -> unleash_process_host::ControlReleaseAck {
    let manifest = fixture.host_manifest();
    match manifest.control_state {
        Some(unleash_process_host::HostControlState::Released { ack }) => *ack,
        other => panic!("native retirement not committed before retained ACK: {other:?}"),
    }
}

#[derive(Clone, Copy)]
struct ReplayBoundary<'a> {
    acquired: &'a TerminalAcquisitionResult,
    request: &'a CommandEnvelope,
    receipt: &'a CommandReceipt,
    before: &'a ControlFacts,
    pending: &'a ReleaseSnapshot,
    native: &'a unleash_process_host::HostManifest,
    host: &'a unleash_platform::OsProcessIdentity,
    native_ack: &'a unleash_process_host::ControlReleaseAck,
    handshakes: usize,
    retirements: usize,
    output: u64,
}

async fn replay_then_drain(harness: &mut Harness, boundary: ReplayBoundary<'_>) {
    let ReplayBoundary {
        acquired,
        request,
        receipt,
        before,
        pending,
        native,
        host,
        native_ack,
        handshakes,
        retirements,
        output,
    } = boundary;
    let fixture = &mut harness.fixture;
    fixture.start_successor().await;
    assert_eq!(
        fixture.client.hello().authority_generation,
        native.authority_generation
    );
    assert!(fixture.client.hello().daemon_epoch > native.daemon_epoch);
    quarantined(pending, &release_state(fixture, receipt));
    retained_resources(before, &facts(fixture, acquired));
    same_live_host(fixture, &harness.paths, host, native);
    let output = fresh_pty_output(native, output).await;
    assert_eq!(native_retirement(fixture), *native_ack);
    assert_eq!(
        fixture
            .client
            .get_command_receipt(RequestId::from_u128(0xf011_8171), request.command_id)
            .await
            .unwrap()
            .receipt,
        *receipt
    );
    assert_eq!(
        fixture.client.send_command(request).await.unwrap(),
        *receipt
    );
    refuse_new_control(fixture, acquired).await;
    quarantined(pending, &release_state(fixture, receipt));
    retained_resources(before, &facts(fixture, acquired));
    assert_eq!(
        (count(&harness.paths), release_count(&harness.paths)),
        (handshakes, retirements)
    );
    let _ = fresh_pty_output(native, output).await;
    // Wait for a real completed/reaped successor drain, then sample again.
    // No sleep by itself can exclude a late re-emitted retirement.
    drain(fixture).await;
    quarantined(pending, &release_state(fixture, receipt));
    retained_resources(before, &facts(fixture, acquired));
    assert_eq!(
        (count(&harness.paths), release_count(&harness.paths)),
        (handshakes, retirements)
    );
    same_live_host(fixture, &harness.paths, host, native);
    assert_eq!(native_retirement(fixture), *native_ack);
}

async fn refuse_new_control(fixture: &mut LiveFixture, acquired: &TerminalAcquisitionResult) {
    let input = command(
        fixture.authority,
        0xf011_8172,
        fixture.project,
        acquired.terminal_id.as_uuid(),
        acquired.control_revision.as_u64(),
        "SendTerminalInput",
        json!({"terminal_id":acquired.terminal_id,"process_run_uuid":acquired.process_run_uuid,
            "lease_id":acquired.lease_id,"fence":acquired.fence,"input_seq":acquired.input_seq+1,
            "payload_bytes":b"apply-effect\n".to_vec()}),
    );
    let acquisition = registered(
        fixture,
        &serde_json::to_value(acquired).unwrap(),
        0xf011_8174,
    );
    for request in [input, acquisition] {
        assert!(matches!(fixture.client.send_command(&request).await,
            Err(unleash_client::ClientError::Remote(error)) if error.code==ErrorCode::ResourceBusy));
        no_receipt(fixture, &request);
    }
}

#[tokio::test]
async fn t_f011_010_release_c4_lost_ack_quarantines_without_second_retirement() {
    let mut harness = harness(false).await;
    let fixture = &mut harness.fixture;
    let acquired = acquire(fixture).await;
    let native = fixture.host_manifest();
    let host = host_identity(&harness.paths);
    let mut old = OldNativeControl::authenticate(fixture);
    let handshakes = count(&harness.paths);
    let baseline = release_count(&harness.paths);
    assert_eq!(baseline, 0);
    let before = facts(fixture, &acquired);
    let output = fresh_pty_output(&native, 0).await;
    unleash_platform::write_private_file_atomic(&harness.paths.release_arm(), b"armed").unwrap();
    let request = release_request(fixture, &acquired, 0xf011_8170);
    let accepted = fixture.client.send_command(&request).await.unwrap();
    assert_eq!(accepted.state, ReceiptState::Accepted);
    let retained_ack = receive(harness.listener.as_ref().unwrap(), RELEASE).await;
    assert_eq!(release_count(&harness.paths), baseline + 1);
    let pending = release_state(fixture, &accepted);
    assert_eq!(
        (pending.intent.as_str(), pending.action.as_str()),
        ("retire_resolved", "dispatching")
    );
    assert_eq!(
        (
            pending.dispatches,
            pending.observations,
            pending.success_phases,
            pending.success_events,
            pending.proof_references
        ),
        (1, 0, 0, 0, 0)
    );
    assert!(pending.proof.is_none() && pending.refusal.is_none() && pending.disposition.is_none());
    let ack = native_retirement(fixture);
    assert_eq!(ack.key.header.action_id.to_string(), pending.action_id);
    assert_eq!(ack.key.dispatch_id.to_string(), pending.dispatch_id);
    assert_eq!(
        ack.key.header.payload_hash.to_string(),
        pending.context_digest
    );
    assert_eq!(
        (ack.key.lease_id, ack.key.control_revision),
        (acquired.lease_id, acquired.control_revision)
    );
    old.assert_revoked();
    retained_resources(&before, &facts(fixture, &acquired));
    same_live_host(fixture, &harness.paths, &host, &native);
    let output = fresh_pty_output(&native, output).await;
    assert!(
        !harness.paths.release_done().try_exists().unwrap(),
        "the real ACK barrier must still be waiting before SIGKILL"
    );
    assert!(!harness.paths.release_eof().try_exists().unwrap());
    assert_eq!(count(&harness.paths), handshakes);
    fixture.daemon.kill_for_restart();
    assert_eq!(
        fixture.daemon.child.try_wait().unwrap().unwrap().signal(),
        Some(9)
    );
    retained_resources(&before, &facts(fixture, &acquired));
    same_live_host(fixture, &harness.paths, &host, &native);
    std::fs::remove_file(harness.paths.release_arm()).unwrap();
    drop(retained_ack);
    wait_for_release_eof(&harness.paths).await;
    assert_eq!(
        native_retirement(fixture),
        ack,
        "the persisted native ACK remains exact despite its lost response"
    );
    replay_then_drain(
        &mut harness,
        ReplayBoundary {
            acquired: &acquired,
            request: &request,
            receipt: &accepted,
            before: &before,
            pending: &pending,
            native: &native,
            host: &host,
            native_ack: &ack,
            handshakes,
            retirements: baseline + 1,
            output,
        },
    )
    .await;
}
