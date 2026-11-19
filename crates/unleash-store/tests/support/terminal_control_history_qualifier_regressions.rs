//! Additional Store regressions, isolated from the original adoption oracles.
//! Native observations remain explicit synthetic fixtures, with distinct
//! configured-profile and observed-effective environment digests.
use super::*;

fn publish_launch_proof(
    store: &UnleashStore,
    allocation: &TerminalAllocationRecord,
    marker: DispatchId,
    environment: Sha256Digest,
) {
    let c = &allocation.context;
    let d = store.daemon_state().unwrap();
    let observation = terminal_allocations::TerminalLaunchObservation {
        terminal_id: c.terminal_id,
        action_id: c.action_id,
        dispatch_id: marker,
        context_digest: allocation.context_digest,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        pid: 123,
        pgid: 123,
        owner_uid: d.owner_uid,
        boot_identity: d.boot_identity,
        start_identity: "synthetic-sqlite-start".into(),
        identity_backend: "macos_proc_pidinfo".into(),
        manifest_digest: canonical_sha256(
            &json!({"fixture":"launch manifest","environment":environment}),
        )
        .unwrap(),
        effective_environment_digest: environment,
        durable_log_seq: 0,
        input_state: Some(
            terminal_allocations::terminal_acquisition_query::TerminalObservedInputState {
                last_confirmed_input_seq: 0,
                unknown_input_seqs: vec![],
            },
        ),
        observed_at_ms: TimestampMs::new(101),
        exit_code: None,
        exit_signal: None,
        exited: false,
    };
    let bytes = canonicalize_to_vec(&observation).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let pin = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "terminal_launch_observation".into(),
            owner_id: c.terminal_id.to_string(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&pin, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: pin.pin_id,
            object_hash: hash,
            object_generation: pin.object_generation,
            owner_kind: "terminal_launch_observation".into(),
            owner_id: c.terminal_id.to_string(),
            source_field: marker.to_string(),
        })
        .unwrap();
    let first = authorizing(allocation);
    let BrokerPhase::Authorizing { intent } = first.phase else {
        unreachable!()
    };
    let current = store.inspect_broker_action(c.action_id).unwrap().unwrap();
    let phase = BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: current.revision,
        evaluation: first.evaluation,
        phase: BrokerPhase::Observation {
            action_id: c.action_id,
            intent_digest: intent.intent_digest,
            receipt: BrokerActionReceipt {
                action_id: c.action_id,
                evaluation_id: first.evaluation.evaluation_id,
                dispatch_id: marker,
                decision_ref: "synthetic-sqlite-allow".into(),
                state: ActionState::ObservedSuccess,
                output_digest: Some(hash),
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: false,
                observed_at_ms: TimestampMs::new(103),
            },
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
        },
    };
    store
        .commit_terminal_launch_observation(observation, phase)
        .unwrap();
}

fn advance_fixture_epoch(store: &UnleashStore) {
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
}

fn fixture_with_observed_environment() -> HistoryFixture<impl std::fmt::Debug> {
    let fault = Arc::new(FailAt {
        armed: AtomicBool::new(false),
        point: StoreCutpoint::C1BeforeCommandCommit,
    });
    let (store, _config, clock, preparation, directory) = fixture(fault.clone());
    store
        .commit_command(request(&store, preparation, TERMINAL))
        .unwrap();
    let allocation = read(&store, TERMINAL);
    let environment = std::collections::BTreeMap::from([
        ("LANG", "C".to_owned()),
        (
            "TMPDIR",
            store
                .config()
                .data_root
                .join("synthetic-child-tmp")
                .to_str()
                .unwrap()
                .to_owned(),
        ),
    ]);
    let observed_environment =
        canonical_sha256(&json!({"version":1,"environment":environment})).unwrap();
    assert_ne!(
        observed_environment,
        allocation
            .context
            .preparation
            .process_spec
            .environment
            .digest
    );
    clock.set_utc_ms(100);
    let marker = dispatch(&store, &allocation);
    store
        .resolve_terminal_launch(resolution(&allocation, marker))
        .unwrap();
    clock.set_utc_ms(105);
    publish_launch_proof(&store, &allocation, marker, observed_environment);
    let store = Arc::try_unwrap(store).unwrap();
    advance_fixture_epoch(&store);
    let original = registered_request(&store, &allocation, CONTROL);
    store.commit_command(original.clone()).unwrap();
    let control = store
        .inspect_terminal_control_request(&original.actor_id, original.envelope.command_id)
        .unwrap()
        .value
        .unwrap();
    clock.set_utc_ms(110);
    let (marker, unknown) = control_dispatch(&store, &control);
    let BrokerPhase::Dispatch { dispatch_id, .. } = marker.phase else {
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
    advance_fixture_epoch(&store);
    clock.set_utc_ms(130);
    let mut proof = complete_fixture_proof(&store, &control, dispatch_id);
    proof.ack.effective_environment_digest = observed_environment;
    proof.inspection.effective_environment_digest = observed_environment;
    proof.inspection.transfer_ack_digest = canonical_sha256(&proof.ack).unwrap();
    HistoryFixture {
        store,
        clock,
        fault,
        control,
        original,
        proof,
        _directory: directory,
    }
}

#[test]
fn t_f011_003_historical_environment_comes_from_qualified_launch_observation() {
    let fixture = fixture_with_observed_environment();
    let rows = immutable_control_rows(&fixture.store);
    let request = history_request(&fixture, &fixture.proof, true);
    let result = fixture.store.seal_terminal_control_history(request);
    assert!(
        result.is_ok(),
        "qualified observed environment differs from the profile: {result:?}"
    );
    let result = result.unwrap();
    assert!(!result.replayed);
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_event_binding(&fixture, &result.fact);
}

#[test]
fn t_f011_003_identical_historical_bytes_release_the_exact_new_pin() {
    let fixture = history_fixture();
    let first = fixture
        .store
        .seal_terminal_control_history(history_request(&fixture, &fixture.proof, true))
        .unwrap();
    let request = history_request(&fixture, &fixture.proof, false);
    let pin = request.prepared_proof.pin_id.clone();
    let artifacts = artifact_rows(&fixture.store);
    let replay = fixture
        .store
        .seal_terminal_control_history(request)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.fact, first.fact);
    fixture
        .store
        .read_snapshot(|conn, _| {
            let state: String = conn.query_row(
                "SELECT state FROM object_pins WHERE pin_id=?1",
                [pin],
                |row| row.get(0),
            )?;
            assert_eq!(
                state, "released",
                "a new exact replay handle must not leak a preparation pin"
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}

#[test]
fn t_f011_003_forged_historical_replay_pin_is_refused_without_publication() {
    let fixture = history_fixture();
    let mut request = history_request(&fixture, &fixture.proof, true);
    fixture
        .store
        .seal_terminal_control_history(request.clone())
        .unwrap();
    request.prepared_proof.pin_id = "not-a-produced-pin".into();
    let artifacts = artifact_rows(&fixture.store);
    let result = fixture.store.seal_terminal_control_history(request);
    assert!(
        matches!(result, Err(StoreError::EvidenceUnavailable(_))),
        "{result:?}"
    );
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}

#[test]
fn t_f011_003_historical_read_rechecks_reassigned_actor_principal() {
    let fixture = history_fixture();
    let fact = fixture
        .store
        .seal_terminal_control_history(history_request(&fixture, &fixture.proof, true))
        .unwrap()
        .fact;
    assert!(
        fixture
            .store
            .inspect_terminal_control_history(
                &fixture.control.context.actor_id,
                fact.action_id,
                fact.dispatch_id
            )
            .unwrap()
            .value
            .is_some()
    );
    fixture.store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{
        tx.execute("UPDATE principals SET actor_id='uid:retired-fixture-alice' WHERE principal_id='human-alice'",[])?;
        tx.execute("UPDATE principals SET actor_id='uid:fixture-alice' WHERE principal_id='human-bob'",[])?;
        Ok(())
    }).unwrap();
    let artifacts = artifact_rows(&fixture.store);
    let result = fixture.store.inspect_terminal_control_history(
        &fixture.control.context.actor_id,
        fact.action_id,
        fact.dispatch_id,
    );
    assert!(matches!(result, Err(StoreError::NotFound(_))), "{result:?}");
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}
