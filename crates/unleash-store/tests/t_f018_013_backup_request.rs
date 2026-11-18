//! REQ-REC-001 : réservation causale et claim exclusif avant toute sauvegarde.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures et assertions locales de contrat"
)]
use std::sync::Arc;
use unleash_domain::{ActorId, BackupId, IdGeneratorPort, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, BackupJobState, BackupRequestState, CommandMutation, CommitCommandRequest,
    CommitOutcome, StoreConfig, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, local_test_dir};

fn intent(record: &unleash_store::BackupRequestRecord) -> unleash_store::BrokerIntent {
    let deadline = unleash_domain::TimestampMs::new(record.created_at_ms.as_i64() + 60000);
    let manifest=serde_json::json!({"action_id":record.action_id,"subject":{"kind":"control_plane","principal_id":record.principal_id,"cause_ref":format!("receipt:{}",record.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},"project_id":null,"verb":unleash_store::REQUEST_BACKUP_VERB,"resource_refs":[{"kind":"path","value":record.destination_dir}],"payload_digest":record.payload_digest,"reconcile_strategy":"observe_then_decide","effects_class":"local_write","scope_revision":record.scope_revision,"policy_revision":record.policy_revision,"lineage_fence":record.lineage_fence,"action_deadline":deadline,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}}).as_object().unwrap().clone();
    unleash_store::BrokerIntent {
        action_id: record.action_id,
        origin: unleash_domain::ActionOrigin::ControlPlane {
            control_principal_id: record.principal_id.clone(),
        },
        principal_id: record.principal_id.clone(),
        cause: unleash_store::BrokerCause::CommandReceipt {
            receipt_id: record.receipt_id,
        },
        project_id: None,
        payload_digest: record.payload_digest,
        scope_revision: record.scope_revision,
        policy_revision: record.policy_revision,
        lineage_fence: record.lineage_fence,
        action_deadline: deadline,
        reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
        intent_digest: unleash_protocol::canonical_sha256(&manifest).unwrap(),
        manifest,
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Intention native, rejets de déclassement et cycle jusqu’à observation prématurée refusée"
)]
fn backup_intention_is_not_declassable_and_success_requires_actual_job_completion() {
    let directory = local_test_dir("f018-013-intent").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1303));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    config.clock = Arc::new(unleash_testkit::ManualClock::new(0));
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let record = store
        .claim_backup_request(backup, store.active_authority_generation().unwrap())
        .unwrap();
    let intent = intent(&record);
    let evaluation = unleash_store::BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: None,
    };
    for variant in 0..3 {
        let mut bad = intent.clone();
        match variant {
            0 => {
                bad.manifest
                    .insert("effects_class".to_owned(), serde_json::json!("local_read"));
            }
            1 => {
                bad.manifest
                    .insert("resource_refs".to_owned(), serde_json::json!([]));
            }
            _ => {
                let fake = unleash_domain::ReceiptId::from_uuid(ids.new_uuid());
                bad.cause = unleash_store::BrokerCause::CommandReceipt { receipt_id: fake };
                bad.manifest
                    .get_mut("subject")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(
                        "cause_ref".to_owned(),
                        serde_json::json!(format!("receipt:{fake}")),
                    );
            }
        }
        bad.intent_digest = unleash_protocol::canonical_sha256(&bad.manifest).unwrap();
        assert!(
            store
                .commit_broker_phase(unleash_store::BrokerPhaseRequest {
                    authority_generation: record.authority_generation,
                    expected_revision: Revision::INITIAL,
                    evaluation,
                    phase: unleash_store::BrokerPhase::Authorizing { intent: bad }
                })
                .is_err()
        );
    }
    let phase = |revision, phase| unleash_store::BrokerPhaseRequest {
        authority_generation: record.authority_generation,
        expected_revision: Revision::new(revision),
        evaluation,
        phase,
    };
    store
        .commit_broker_phase(phase(
            0,
            unleash_store::BrokerPhase::Authorizing {
                intent: intent.clone(),
            },
        ))
        .unwrap();
    let decision = unleash_store::BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "decision:backup-fixture".to_owned(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: None,
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: unleash_store::BrokerExecutor::Unleash,
    };
    store
        .commit_broker_phase(phase(
            1,
            unleash_store::BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: unleash_store::BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: unleash_store::BrokerAuthorizationCause::Evaluation,
            },
        ))
        .unwrap();
    let dispatch = unleash_domain::DispatchId::from_uuid(ids.new_uuid());
    store
        .commit_broker_phase(phase(
            2,
            unleash_store::BrokerPhase::Dispatch {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                decision: decision.clone(),
                dispatch_id: dispatch,
                fence: record.lineage_fence,
            },
        ))
        .unwrap();
    let receipt = unleash_store::BrokerActionReceipt {
        action_id: intent.action_id,
        evaluation_id: evaluation.evaluation_id,
        dispatch_id: dispatch,
        decision_ref: decision.decision_ref,
        state: unleash_domain::ActionState::ObservedSuccess,
        output_digest: Some(unleash_domain::Sha256Digest::from_bytes(b"not-a-manifest")),
        enforcement_refs: vec![],
        remote_audit_ref: None,
        remote_audit_unavailable: true,
        observed_at_ms: unleash_domain::TimestampMs::new(0),
    };
    assert!(
        store
            .commit_broker_phase(phase(
                3,
                unleash_store::BrokerPhase::Observation {
                    action_id: intent.action_id,
                    intent_digest: intent.intent_digest,
                    receipt,
                    observation_revision: Revision::new(1),
                    cause: unleash_store::BrokerObservationCause::Execution,
                    reason: None
                }
            ))
            .is_err()
    );
    let snapshot = store.inspect_backup_request_snapshot(backup).unwrap();
    let record = snapshot.value.unwrap();
    assert_eq!(record.state, BackupRequestState::Claimed);
    assert!(record.manifest.is_none());
    assert!(!record.destination_dir.exists());
    assert_eq!(
        store
            .inspect_broker_action(intent.action_id)
            .unwrap()
            .unwrap()
            .state,
        unleash_domain::ActionState::Dispatching
    );
}

fn request(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
    backup: BackupId,
) -> CommitCommandRequest {
    CommitCommandRequest {
        actor_id: ActorId::new("p-sys").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some("p-sys".to_owned()),
            project_id: None,
            aggregate_id: backup.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "RequestBackup".to_owned(),
            payload: serde_json::Map::new(),
        },
        target_aggregate: AggregateKind::BackupJob,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::RequestBackup { backup_id: backup },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

#[test]
fn reservation_replays_same_receipt_and_claims_once_without_filesystem_effect() {
    let directory = local_test_dir("f018-013-reservation").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1301));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    let command = request(&store, &ids, backup);
    let first = store.commit_command(command.clone()).unwrap();
    assert_eq!(first.receipt().state, ReceiptState::Accepted);
    assert_eq!(first.receipt().result_ref, Some(format!("backup:{backup}")));
    assert_eq!(first.receipt().aggregate_id, backup.as_uuid());
    let record = store.inspect_backup_request(backup).unwrap().unwrap();
    assert_eq!(record.backup_id, backup);
    assert_eq!(record.receipt_id, first.receipt().receipt_id);
    assert_eq!(record.state, BackupRequestState::Reserved);
    assert_eq!(record.job_state, Some(BackupJobState::Preparing));
    assert_eq!(
        record.destination_dir,
        config.backups_dir.join(backup.to_string())
    );
    assert!(!record.destination_dir.exists());
    let replay = store.commit_command(command.clone()).unwrap();
    assert!(matches!(replay, CommitOutcome::IdempotentReplay(_)));
    assert_eq!(replay.receipt(), first.receipt());
    let mut another = command.clone();
    another.envelope.command_id = ids.new_command_id();
    assert!(store.commit_command(another).is_err());
    let mut mismatch = command;
    mismatch.mutation = CommandMutation::RequestBackup {
        backup_id: BackupId::from_uuid(ids.new_uuid()),
    };
    assert!(store.commit_command(mismatch).is_err());
    let claimed = store
        .claim_backup_request(backup, record.authority_generation)
        .unwrap();
    assert_eq!(claimed.action_id, record.action_id);
    assert_eq!(claimed.state, BackupRequestState::Claimed);
    assert!(
        store
            .claim_backup_request(backup, record.authority_generation)
            .is_err()
    );
    assert!(!record.destination_dir.exists());
    let counts=store.read_snapshot(|conn,_| {
        let jobs=conn.query_row("SELECT COUNT(*) FROM backup_jobs WHERE backup_id=?1",[backup.to_string()],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
        let actions=conn.query_row("SELECT COUNT(*) FROM actions",[],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
        let events=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='BackupRequested'",[backup.to_string()],|r|r.get::<_,i64>(0)).map_err(StoreError::from_rusqlite)?;
        Ok((jobs,actions,events))
    }).unwrap();
    assert_eq!(counts, (1, 0, 1));
}

#[test]
fn historical_request_never_becomes_claimable_by_relabeling_authority() {
    let directory = local_test_dir("f018-013-historical").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1302));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let source = store.active_authority_generation().unwrap();
    let target = unleash_domain::AuthorityGeneration::from_uuid(ids.new_uuid());
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute(
                "UPDATE daemon_state SET authority_generation=?1 WHERE singleton=1",
                [target.to_string()],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        store.claim_backup_request(backup, source),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert!(matches!(
        store.claim_backup_request(backup, target),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert_eq!(
        store.inspect_backup_request(backup).unwrap().unwrap().state,
        BackupRequestState::Reserved
    );
}
