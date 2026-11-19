//! Native state attestation is required before a new input intention. These
//! local `SQLite` fixtures contain synthetic OS facts and perform no PTY effect.
use super::*;

fn without_native_input_state() -> (UnleashStore, TerminalAllocationRecord, impl std::fmt::Debug) {
    let (store, _config, clock, preparation, directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, INPUT + 200))
        .unwrap();
    let allocation = read(&store, INPUT + 200);
    clock.set_utc_ms(100);
    let marker = dispatch(&store, &allocation);
    store
        .resolve_terminal_launch(super::super::super::resolution(&allocation, marker))
        .unwrap();
    clock.set_utc_ms(105);
    let (observation, phase) = launch_observation_with_input_state_fixture(
        &store,
        &allocation,
        marker,
        101,
        103,
        false,
        None,
    );
    store
        .commit_terminal_launch_observation(observation, phase)
        .unwrap();
    let allocation = read(&store, INPUT + 200);
    (Arc::try_unwrap(store).unwrap(), allocation, directory)
}

fn assert_no_admission(store: &UnleashStore, allocation: &TerminalAllocationRecord, n: u128) {
    let before = control_facts(store, allocation);
    let request = input_request(store, allocation, n);
    let actor = request.actor_id.clone();
    let command = request.envelope.command_id;
    let result = store.commit_command(request);
    assert!(
        matches!(&result, Err(StoreError::EvidenceUnavailable(_))),
        "native state cannot be inferred from the SQL zero or an unqualified artifact: {result:?}"
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
    assert_eq!(control_facts(store, allocation), before);
}

#[test]
fn t_f011_005_missing_native_input_state_cannot_prepare_seq_one() {
    let (store, allocation, _directory) = without_native_input_state();
    let command: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT command_id FROM command_receipts WHERE receipt_id=?1",
                [allocation.context.receipt_id.to_string()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let command = command.parse().unwrap();
    let reference = format!("terminal-allocation:{}", allocation.context.terminal_id);
    assert!(matches!(
        store.inspect_terminal_acquisition_result(
            &allocation.context.actor_id,
            command,
            &reference,
            PROJECT
        ),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_no_admission(&store, &allocation, INPUT + 201);
}

#[cfg(unix)]
#[test]
fn t_f011_005_symlink_native_evidence_cannot_prepare_input() {
    let (store, _clock, _fault, allocation, _directory) = observed(false);
    let dispatch = allocation.dispatch_id.unwrap();
    let relative:String=store.read_snapshot(|conn,_|Ok(conn.query_row(
        "SELECT o.relative_path FROM object_records o JOIN object_references r ON r.object_hash=o.object_hash WHERE r.owner_kind='terminal_launch_observation' AND r.owner_id=?1 AND r.source_field=?2 AND r.state='live'",
        rusqlite::params![allocation.context.terminal_id.to_string(),dispatch.to_string()],|row|row.get(0))?)).unwrap();
    let artifact = store.config().artifacts_dir.join(relative);
    let retained = artifact.with_file_name("synthetic-native-proof-original.json");
    std::fs::rename(&artifact, &retained).unwrap();
    std::os::unix::fs::symlink(&retained, &artifact).unwrap();
    // Same canonical bytes and SQL hash. The file type itself is inadmissible.
    assert_no_admission(&store, &allocation, INPUT + 202);
}
