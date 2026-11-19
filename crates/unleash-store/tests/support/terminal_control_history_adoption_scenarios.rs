//! Store contract fixtures only: the ACK shape and identities below are
//! synthetic. The real host's four ACK-inspection tests remain separate;
//! these tests do not claim an OS rotation or a daemon reconciliation.
#[path = "terminal_control_history_qualifier_regressions.rs"]
mod qualifier_regressions;

use super::*;
use unleash_domain::HostId;
use unleash_store::terminal_allocations::terminal_control_history::{
    HistoricalTransferAck, HistoricalTransferHeader, SealTerminalControlHistoryRequest,
    TERMINAL_CONTROL_HISTORY_EVENT, TerminalControlHistoryCommit, TerminalControlHistoryFact,
    TerminalControlHistoryProof,
};

struct HistoryFixture<Directory> {
    store: UnleashStore,
    clock: Arc<ManualClock>,
    fault: Arc<FailAt>,
    control: TerminalControlRecord,
    original: CommitCommandRequest,
    proof: TerminalControlHistoryProof,
    _directory: Directory,
}

fn history_fixture() -> HistoryFixture<impl std::fmt::Debug> {
    let (store, clock, fault, control, original, directory) = prepared();
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
    store
        .execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute(
                "UPDATE daemon_state SET daemon_epoch=daemon_epoch+1 WHERE singleton=1",
                [],
            )?;
            tx.execute(
                "UPDATE resource_leases SET state='suspect' WHERE state='reconciling'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    clock.set_utc_ms(130);
    let proof = complete_fixture_proof(&store, &control, dispatch_id);
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

fn complete_fixture_proof(
    store: &UnleashStore,
    control: &TerminalControlRecord,
    dispatch_id: DispatchId,
) -> TerminalControlHistoryProof {
    let c = &control.context;
    let broker = control_authorizing(control);
    let BrokerPhase::Authorizing { intent } = broker.phase else {
        unreachable!()
    };
    let allocation = read(store, TERMINAL);
    let environment = allocation
        .context
        .preparation
        .process_spec
        .environment
        .digest;
    let ack = HistoricalTransferAck {
        header: HistoricalTransferHeader {
            protocol_version: 1,
            process_run_uuid: c.process_run_uuid,
            authority_generation: c.authority_generation,
            daemon_epoch: c.daemon_epoch,
            resource_fence: c.next_fence,
            command_id: c.command_id,
            action_id: c.action_id,
            payload_hash: control.context_digest,
        },
        dispatch_id,
        initial_context_digest: c.initial_context_digest,
        effective_environment_digest: environment,
        os_identity: c.expected_os_identity.clone(),
        durable_log_seq: 0,
        last_confirmed_input_seq: 0,
        unknown_input_seqs: vec![],
    };
    // This hash is computed from the complete fixture ACK, not from a label
    // pretending to be an acknowledgement or a hash of a secret capability.
    let ack_digest = canonical_sha256(&ack).unwrap();
    let fixture_manifest = json!({
        "fixture": "sqlite_only_not_a_native_manifest",
        "host_id": c.host_id,
        "process_run_uuid": c.process_run_uuid,
        "daemon_epoch": c.daemon_epoch,
        "highest_fence": c.next_fence
    });
    TerminalControlHistoryProof {
        schema_version: 1,
        source_receipt_id: c.receipt_id,
        source_evaluation_id: broker.evaluation.evaluation_id,
        source_intent_digest: intent.intent_digest,
        inspection: terminal_allocations::TerminalControlObservation {
            action_id: c.action_id,
            dispatch_id,
            context_digest: control.context_digest,
            terminal_id: c.terminal_id,
            host_id: c.host_id,
            process_run_uuid: c.process_run_uuid,
            os_identity: c.expected_os_identity.clone(),
            daemon_epoch: c.daemon_epoch,
            resource_fence: c.next_fence,
            transfer_ack_digest: ack_digest,
            manifest_digest: canonical_sha256(&fixture_manifest).unwrap(),
            effective_environment_digest: environment,
            durable_log_seq: ack.durable_log_seq,
            last_confirmed_input_seq: ack.last_confirmed_input_seq,
            unknown_input_seqs: ack.unknown_input_seqs.clone(),
            observed_at_ms: TimestampMs::new(120),
        },
        ack,
    }
}

fn history_request(
    fixture: &HistoryFixture<impl std::fmt::Debug>,
    proof: &TerminalControlHistoryProof,
    write_bytes: bool,
) -> SealTerminalControlHistoryRequest {
    let bytes = canonicalize_to_vec(proof).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let pin = fixture
        .store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "action".into(),
            owner_id: fixture.control.context.action_id.to_string(),
        })
        .unwrap();
    if write_bytes {
        fixture
            .store
            .write_prepared_object_bytes(&pin, &bytes)
            .unwrap();
    }
    SealTerminalControlHistoryRequest {
        caller: CallerAuthority {
            actor_id: fixture.control.context.actor_id.clone(),
            role: CallerRole::HumanOperator,
        },
        source_action_id: fixture.control.context.action_id,
        dispatch_id: proof.ack.dispatch_id,
        context_digest: fixture.control.context_digest,
        prepared_proof: pin,
    }
}

fn immutable_control_rows(store: &UnleashStore) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    store
        .read_snapshot(|conn, _| {
            [
                "actions",
                "action_dispatches",
                "broker_phase_records",
                "broker_intents",
                "broker_evaluations",
                "broker_dispatches",
                "terminal_control_intents",
                "processes",
                "terminal_sessions",
                "resource_leases",
                "resource_fences",
                "capacity_reservations",
                "command_receipts",
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

fn history_counts(fixture: &HistoryFixture<impl std::fmt::Debug>) -> (i64, i64, i64) {
    fixture.store.read_snapshot(|conn,_| {
        let action=fixture.control.context.action_id.to_string();
        let event_count=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_type='action' AND aggregate_id=?1 AND event_type=?2",rusqlite::params![action,TERMINAL_CONTROL_HISTORY_EVENT],|row|row.get(0))?;
        let references=conn.query_row("SELECT COUNT(*) FROM object_references WHERE owner_kind='action' AND owner_id=?1 AND source_field=?2 AND state='live'",rusqlite::params![action,format!("terminal_control_history:{}",fixture.proof.ack.dispatch_id)],|row|row.get(0))?;
        let outbox=conn.query_row("SELECT COUNT(*) FROM outbox o JOIN events e ON e.event_id=o.event_id WHERE e.aggregate_type='action' AND e.aggregate_id=?1 AND e.event_type=?2",rusqlite::params![action,TERMINAL_CONTROL_HISTORY_EVENT],|row|row.get(0))?;
        Ok((event_count,references,outbox))
    }).unwrap()
}

fn artifact_rows(store: &UnleashStore) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    store
        .read_snapshot(|conn, _| {
            ["object_references", "events", "outbox"]
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

fn assert_event_binding(
    fixture: &HistoryFixture<impl std::fmt::Debug>,
    fact: &TerminalControlHistoryFact,
) {
    fixture.store.read_snapshot(|conn,_|{
        let event:(String,String,String,String,i64)=conn.query_row("SELECT stream_id,aggregate_type,aggregate_id,event_type,schema_version FROM events WHERE sequence=?1",[i64::try_from(fact.event_cursor.sequence.as_u64()).unwrap()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
        assert_eq!(event,(fact.event_cursor.stream_id.to_string(),"action".into(),fixture.control.context.action_id.to_string(),TERMINAL_CONTROL_HISTORY_EVENT.into(),1));
        Ok(())
    }).unwrap();
}

fn assert_first_fact(
    fixture: &HistoryFixture<impl std::fmt::Debug>,
    request: &SealTerminalControlHistoryRequest,
    first: &TerminalControlHistoryCommit,
) {
    let c = &fixture.control.context;
    assert!(!first.replayed);
    assert_eq!(first.fact.action_id, c.action_id);
    assert_eq!(first.fact.dispatch_id, fixture.proof.ack.dispatch_id);
    assert_eq!(first.fact.source_receipt_id, c.receipt_id);
    assert_eq!(
        first.fact.source_evaluation_id,
        fixture.proof.source_evaluation_id
    );
    assert_eq!(
        first.fact.source_intent_digest,
        fixture.proof.source_intent_digest
    );
    assert_eq!(first.fact.context_digest, fixture.control.context_digest);
    assert_eq!(
        first.fact.native_ack_digest,
        canonical_sha256(&fixture.proof.ack).unwrap()
    );
    assert_eq!(
        first.fact.observation_object_hash,
        request.prepared_proof.object_hash
    );
    assert_eq!(first.fact.rotation_epoch, c.daemon_epoch);
    assert_eq!(first.fact.rotation_fence, c.next_fence);
    assert_eq!(
        first.fact.inspected_by_epoch,
        fixture.store.daemon_state().unwrap().daemon_epoch
    );
    assert_eq!(first.fact.inspected_by, c.actor_id);
    assert_eq!(first.fact.observed_at_ms, TimestampMs::new(120));
}

fn assert_readback_and_replay(
    fixture: &HistoryFixture<impl std::fmt::Debug>,
    request: &SealTerminalControlHistoryRequest,
    first: &TerminalControlHistoryCommit,
) {
    let c = &fixture.control.context;
    let readback = fixture
        .store
        .inspect_terminal_control_history(&c.actor_id, c.action_id, fixture.proof.ack.dispatch_id)
        .unwrap();
    assert_eq!(readback.value, Some(first.fact.clone()));
    assert_eq!(readback.cursor.stream_id, first.fact.event_cursor.stream_id);
    assert!(readback.cursor.sequence >= first.fact.event_cursor.sequence);
    let replay = fixture
        .store
        .seal_terminal_control_history(request.clone())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.fact, first.fact);
    fixture.clock.set_utc_ms(150);
    let mut later = fixture.proof.clone();
    later.inspection.observed_at_ms = TimestampMs::new(140);
    let later = fixture
        .store
        .seal_terminal_control_history(history_request(fixture, &later, true))
        .unwrap();
    assert!(later.replayed);
    assert_eq!(
        later.fact, first.fact,
        "inspection time is not native fact identity"
    );
}

#[test]
fn t_f011_003_exact_historical_ack_is_readable_once_without_current_control() {
    let fixture = history_fixture();
    let c = &fixture.control.context;
    let rows = immutable_control_rows(&fixture.store);
    let before_receipt = fixture
        .store
        .inspect_command_receipt(
            &fixture.original.actor_id,
            fixture.original.envelope.command_id,
        )
        .unwrap();
    assert_eq!(history_counts(&fixture), (0, 0, 0));
    assert!(fixture.store.daemon_state().unwrap().daemon_epoch > c.daemon_epoch);
    let request = history_request(&fixture, &fixture.proof, true);
    let first = fixture.store.seal_terminal_control_history(request.clone());
    if let Err(error) = &first {
        assert_eq!(
            error.reason_code(),
            Some(unleash_protocol::ReasonCode::EvidenceUnavailable),
            "complete fixture must reach the closed historical-evidence seam"
        );
    }
    assert!(
        first.is_ok(),
        "complete correlated historical ACK must be adopted as facts, without current control: {first:?}"
    );
    let first = first.unwrap();
    assert_first_fact(&fixture, &request, &first);
    assert_event_binding(&fixture, &first.fact);
    let counts = history_counts(&fixture);
    let sealed_artifacts = artifact_rows(&fixture.store);
    assert_eq!((counts.0, counts.1), (1, 1));
    assert!(counts.2 > 0, "fact and notification seal together");
    assert_readback_and_replay(&fixture, &request, &first);
    assert_eq!(history_counts(&fixture), counts);
    assert_eq!(artifact_rows(&fixture.store), sealed_artifacts);
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_eq!(
        fixture
            .store
            .inspect_command_receipt(
                &fixture.original.actor_id,
                fixture.original.envelope.command_id
            )
            .unwrap(),
        before_receipt
    );
    assert_eq!(
        fixture
            .store
            .inspect_broker_action(c.action_id)
            .unwrap()
            .unwrap()
            .state,
        ActionState::OutcomeUnknown
    );
    assert_eq!(
        state(&fixture.store, &fixture.control),
        (0, c.previous_lease_id.to_string(), 2, 2, 3)
    );
    assert!(
        !fixture
            .store
            .config()
            .data_root
            .join("process-hosts")
            .exists()
    );
}

#[test]
fn t_f011_003_historical_missing_bytes_or_foreign_caller_cannot_publish_facts() {
    let fixture = history_fixture();
    let rows = immutable_control_rows(&fixture.store);
    let artifacts = artifact_rows(&fixture.store);
    let mut absent = fixture.proof.clone();
    absent.inspection.observed_at_ms = TimestampMs::new(121);
    let missing = history_request(&fixture, &absent, false);
    assert!(matches!(
        fixture.store.seal_terminal_control_history(missing),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let mut foreign = history_request(&fixture, &fixture.proof, true);
    foreign.caller.actor_id = ActorId::new("uid:fixture-bob").unwrap();
    assert!(matches!(
        fixture.store.seal_terminal_control_history(foreign),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(history_counts(&fixture), (0, 0, 0));
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}

#[test]
fn t_f011_003_historical_source_and_native_bindings_must_all_match() {
    let fixture = history_fixture();
    let rows = immutable_control_rows(&fixture.store);
    let artifacts = artifact_rows(&fixture.store);
    for field in [
        "receipt",
        "evaluation",
        "intent",
        "command",
        "action",
        "dispatch",
        "payload",
        "initial_context",
        "lineage",
        "epoch",
        "fence",
        "run",
        "host",
        "boot",
        "start",
        "environment",
        "ack_digest",
    ] {
        let mut proof = fixture.proof.clone();
        let different_hash = Sha256Digest::from_bytes(b"different historical binding");
        match field {
            "receipt" => {
                proof.source_receipt_id = unleash_domain::ReceiptId::from_u128(CONTROL + 701)
            }
            "evaluation" => {
                proof.source_evaluation_id = AuthorizationEvaluationId::from_u128(CONTROL + 702)
            }
            "intent" => proof.source_intent_digest = different_hash,
            "command" => {
                proof.ack.header.command_id = unleash_domain::CommandId::from_u128(CONTROL + 703)
            }
            "action" => {
                proof.ack.header.action_id = unleash_domain::ActionId::from_u128(CONTROL + 704)
            }
            "dispatch" => proof.ack.dispatch_id = DispatchId::from_u128(CONTROL + 705),
            "payload" => proof.ack.header.payload_hash = different_hash,
            "initial_context" => proof.ack.initial_context_digest = different_hash,
            "lineage" => {
                proof.ack.header.authority_generation =
                    AuthorityGeneration::from_u128(CONTROL + 706)
            }
            "epoch" => proof.ack.header.daemon_epoch = unleash_domain::DaemonEpoch::new(1),
            "fence" => proof.ack.header.resource_fence = Fence::new(1),
            "run" => proof.ack.header.process_run_uuid = ProcessRunUuid::from_u128(CONTROL + 707),
            "host" => proof.inspection.host_id = HostId::from_u128(CONTROL + 708),
            "boot" => proof.ack.os_identity.boot_identity = "foreign boot".into(),
            "start" => proof.ack.os_identity.start_identity = "foreign start".into(),
            "environment" => proof.ack.effective_environment_digest = different_hash,
            "ack_digest" => proof.inspection.transfer_ack_digest = different_hash,
            _ => unreachable!(),
        }
        if field != "ack_digest" {
            proof.inspection.transfer_ack_digest = canonical_sha256(&proof.ack).unwrap();
        }
        let mut request = history_request(&fixture, &proof, true);
        request.dispatch_id = fixture.proof.ack.dispatch_id;
        let refused = fixture.store.seal_terminal_control_history(request);
        let error = refused.unwrap_err();
        let expected = match field {
            "lineage" | "epoch" | "fence" => unleash_protocol::ErrorCode::StaleFence,
            "host" | "run" | "boot" | "start" | "environment" => {
                unleash_protocol::ErrorCode::CapabilityDenied
            }
            _ => unleash_protocol::ErrorCode::InvalidArgument,
        };
        assert_eq!(error.error_code(), expected, "foreign {field}: {error:?}");
        if expected == unleash_protocol::ErrorCode::CapabilityDenied {
            assert_eq!(
                error.reason_code(),
                Some(unleash_protocol::ReasonCode::IdentityUnverified),
                "{field}"
            );
        }
        assert_eq!(history_counts(&fixture), (0, 0, 0), "{field}");
        assert_eq!(immutable_control_rows(&fixture.store), rows, "{field}");
        assert_eq!(artifact_rows(&fixture.store), artifacts, "{field}");
    }
}

#[test]
fn t_f011_003_historical_reference_and_event_commit_together_or_not_at_all() {
    let fixture = history_fixture();
    let request = history_request(&fixture, &fixture.proof, true);
    let rows = immutable_control_rows(&fixture.store);
    let artifacts = artifact_rows(&fixture.store);
    fixture.fault.armed.store(true, Ordering::Release);
    let refused = fixture.store.seal_terminal_control_history(request.clone());
    assert!(
        matches!(refused, Err(StoreError::CutpointInjected { .. })),
        "{refused:?}"
    );
    assert_eq!(history_counts(&fixture), (0, 0, 0));
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_eq!(artifact_rows(&fixture.store), artifacts);
    let committed = fixture
        .store
        .seal_terminal_control_history(request)
        .unwrap();
    assert!(!committed.replayed);
    let counts = history_counts(&fixture);
    assert_eq!((counts.0, counts.1), (1, 1));
    assert!(counts.2 > 0);
    assert_eq!(immutable_control_rows(&fixture.store), rows);
}

#[test]
fn t_f011_003_historical_fact_is_acl_bound_and_cannot_be_recreated_after_tombstone() {
    let fixture = history_fixture();
    let request = history_request(&fixture, &fixture.proof, true);
    let adopted = fixture
        .store
        .seal_terminal_control_history(request.clone())
        .unwrap();
    assert_event_binding(&fixture, &adopted.fact);
    let bob = ActorId::new("uid:fixture-bob").unwrap();
    assert!(matches!(
        fixture.store.inspect_terminal_control_history(
            &bob,
            fixture.control.context.action_id,
            fixture.proof.ack.dispatch_id
        ),
        Err(StoreError::NotFound(_))
    ));
    let deliveries: Vec<(String, String)> = fixture
        .store
        .read_snapshot(|conn, _| {
            let mut query = conn.prepare(
                "SELECT o.event_id,o.destination FROM outbox o JOIN events e ON e.event_id=o.event_id WHERE e.sequence=?1",
            )?;
            Ok(query
                .query_map(
                    [i64::try_from(adopted.fact.event_cursor.sequence.as_u64()).unwrap()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap();
    for (event, destination) in deliveries {
        fixture
            .store
            .ack_outbox_delivery(event.parse().unwrap(), &destination)
            .unwrap();
    }
    fixture.clock.set_utc_ms(40_000_000_000);
    fixture
        .store
        .purge_history(TimestampMs::new(39_000_000_000))
        .unwrap();
    fixture
        .store
        .read_snapshot(|conn, _| {
            let state: String = conn.query_row(
                "SELECT payload_state FROM events WHERE sequence=?1",
                [i64::try_from(adopted.fact.event_cursor.sequence.as_u64()).unwrap()],
                |row| row.get(0),
            )?;
            assert_eq!(state, "tombstone");
            Ok(())
        })
        .unwrap();
    let rows = immutable_control_rows(&fixture.store);
    let counts = history_counts(&fixture);
    let artifacts = artifact_rows(&fixture.store);
    let read = fixture.store.inspect_terminal_control_history(
        &fixture.control.context.actor_id,
        fixture.control.context.action_id,
        fixture.proof.ack.dispatch_id,
    );
    assert!(
        matches!(read, Err(StoreError::EvidenceUnavailable(_))),
        "{read:?}"
    );
    assert!(matches!(
        fixture.store.seal_terminal_control_history(request),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(history_counts(&fixture), counts);
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}

#[test]
fn t_f011_003_missing_historical_reference_never_revives_from_replay() {
    let fixture = history_fixture();
    let request = history_request(&fixture, &fixture.proof, true);
    fixture
        .store
        .seal_terminal_control_history(request.clone())
        .unwrap();
    assert_eq!(
        fixture
            .store
            .tombstone_object_references(request.prepared_proof.object_hash, "fixture retention")
            .unwrap(),
        1
    );
    let rows = immutable_control_rows(&fixture.store);
    let counts = history_counts(&fixture);
    let artifacts = artifact_rows(&fixture.store);
    let read = fixture.store.inspect_terminal_control_history(
        &fixture.control.context.actor_id,
        fixture.control.context.action_id,
        fixture.proof.ack.dispatch_id,
    );
    assert!(
        matches!(read, Err(StoreError::EvidenceUnavailable(_))),
        "{read:?}"
    );
    assert!(matches!(
        fixture.store.seal_terminal_control_history(request),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(history_counts(&fixture), counts);
    assert_eq!(immutable_control_rows(&fixture.store), rows);
    assert_eq!(artifact_rows(&fixture.store), artifacts);
}
