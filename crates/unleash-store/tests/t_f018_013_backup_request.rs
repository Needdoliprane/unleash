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
    let other_backup = BackupId::from_uuid(ids.new_uuid());
    store
        .register_principal("p-other", "system", "p-other")
        .unwrap();
    let mut other = request(&store, &ids, other_backup);
    other.actor_id = ActorId::new("p-other").unwrap();
    other.envelope.actor_ref = Some("p-other".to_owned());
    let other_receipt = store.commit_command(other).unwrap().receipt().receipt_id;
    for variant in 0..4 {
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
                let fake = if variant == 3 {
                    other_receipt
                } else {
                    unleash_domain::ReceiptId::from_uuid(ids.new_uuid())
                };
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

fn dispatch(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
    record: &unleash_store::BackupRequestRecord,
) -> (
    unleash_store::BrokerIntent,
    unleash_store::BrokerEvaluation,
    unleash_store::BrokerAuthorizationDecision,
    unleash_domain::DispatchId,
) {
    let intent = intent(record);
    let evaluation = unleash_store::BrokerEvaluation {
        evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
        previous_evaluation_id: None,
    };
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
        decision_ref: "decision:backup-execution".to_owned(),
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
    (intent, evaluation, decision, dispatch)
}

#[test]
fn reserved_execution_requires_marker_runs_once_and_observes_exact_completed_manifest() {
    let directory = local_test_dir("f018-013-execute").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1304));
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
    let missing = unleash_store::ExecuteReservedBackupRequest {
        backup_id: backup,
        action_id: record.action_id,
        dispatch_id: unleash_domain::DispatchId::from_uuid(ids.new_uuid()),
        authority_generation: record.authority_generation,
    };
    assert!(store.execute_reserved_backup(missing).is_err());
    assert!(!record.destination_dir.exists());
    let (intent, evaluation, decision, dispatch) = dispatch(&store, &ids, &record);
    let execute = unleash_store::ExecuteReservedBackupRequest {
        dispatch_id: dispatch,
        ..missing
    };
    let completed = store.execute_reserved_backup(execute).unwrap();
    assert_eq!(completed.manifest.backup_id, backup);
    assert_eq!(completed.manifest.state, BackupJobState::Complete);
    assert_eq!(
        completed.output_digest,
        unleash_protocol::canonical_sha256(&serde_json::to_value(&completed.manifest).unwrap())
            .unwrap()
    );
    assert!(record.destination_dir.join("db.sqlite").exists());
    assert!(store.execute_reserved_backup(execute).is_err());
    let snapshot = store
        .inspect_backup_request_snapshot(backup)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(snapshot.state, BackupRequestState::Completed);
    assert_eq!(snapshot.job_state, Some(BackupJobState::Complete));
    assert_eq!(snapshot.manifest, Some(completed.manifest));
    assert_eq!(snapshot.manifest_digest, Some(completed.output_digest));
    assert!(!snapshot.barrier_pin_held);
    assert_eq!(snapshot.held_object_pins, 0);
    let receipt = unleash_store::BrokerActionReceipt {
        action_id: record.action_id,
        evaluation_id: evaluation.evaluation_id,
        dispatch_id: dispatch,
        decision_ref: decision.decision_ref,
        state: unleash_domain::ActionState::ObservedSuccess,
        output_digest: Some(completed.output_digest),
        enforcement_refs: vec![],
        remote_audit_ref: None,
        remote_audit_unavailable: true,
        observed_at_ms: unleash_domain::TimestampMs::new(0),
    };
    let observation = unleash_store::BrokerPhaseRequest {
        authority_generation: record.authority_generation,
        expected_revision: Revision::new(3),
        evaluation,
        phase: unleash_store::BrokerPhase::Observation {
            action_id: record.action_id,
            intent_digest: intent.intent_digest,
            receipt: receipt.clone(),
            observation_revision: Revision::new(1),
            cause: unleash_store::BrokerObservationCause::Execution,
            reason: None,
        },
    };
    let mut wrong = observation.clone();
    if let unleash_store::BrokerPhase::Observation { receipt, .. } = &mut wrong.phase {
        receipt.output_digest = Some(unleash_domain::Sha256Digest::from_bytes(b"wrong-manifest"));
    }
    assert!(store.commit_broker_phase(wrong).is_err());
    store.commit_broker_phase(observation).unwrap();
    assert_eq!(
        store
            .inspect_broker_action(record.action_id)
            .unwrap()
            .unwrap()
            .state,
        unleash_domain::ActionState::ObservedSuccess
    );
}

fn execution_fixture(
    cutpoints: Arc<dyn unleash_store::CutpointObserver>,
) -> (
    Arc<UnleashStore>,
    Arc<DeterministicIdGenerator>,
    StoreConfig,
    unleash_store::BackupRequestRecord,
    unleash_store::ExecuteReservedBackupRequest,
    impl std::fmt::Debug,
) {
    let directory = local_test_dir("f018-013-boundary").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1305));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    config.clock = Arc::new(unleash_testkit::ManualClock::new(0));
    config.cutpoints = cutpoints;
    let store = Arc::new(UnleashStore::open(config.clone()).unwrap());
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let record = store
        .claim_backup_request(backup, store.active_authority_generation().unwrap())
        .unwrap();
    let (_, _, _, dispatch) = dispatch(&store, &ids, &record);
    let execute = unleash_store::ExecuteReservedBackupRequest {
        backup_id: backup,
        action_id: record.action_id,
        dispatch_id: dispatch,
        authority_generation: record.authority_generation,
    };
    (store, ids, config, record, execute, directory)
}

struct FailAt(unleash_store::StoreCutpoint);
impl unleash_store::CutpointObserver for FailAt {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), StoreError> {
        if point == self.0 {
            Err(StoreError::CutpointInjected {
                cutpoint: format!("{point:?}"),
                message: "backup fixture".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[test]
fn backup_c11_boundaries_preserve_truth_and_never_reexecute() {
    for point in [
        unleash_store::StoreCutpoint::C11aAfterBackupBarrier,
        unleash_store::StoreCutpoint::C11eAfterCompleteManifestBeforeJobCommit,
        unleash_store::StoreCutpoint::C11fAfterBackupJobCommit,
    ] {
        let (store, _ids, config, record, execute, _directory) =
            execution_fixture(Arc::new(FailAt(point)));
        assert!(store.execute_reserved_backup(execute).is_err());
        let snapshot = store
            .inspect_backup_request_snapshot(record.backup_id)
            .unwrap()
            .value
            .unwrap();
        let complete = point == unleash_store::StoreCutpoint::C11fAfterBackupJobCommit;
        assert_eq!(
            snapshot.state,
            if complete {
                BackupRequestState::Completed
            } else {
                BackupRequestState::Executing
            }
        );
        assert_eq!(snapshot.manifest.is_some(), complete);
        assert_eq!(snapshot.manifest_digest.is_some(), complete);
        assert_eq!(
            snapshot.action_state,
            Some(unleash_domain::ActionState::Dispatching)
        );
        if point == unleash_store::StoreCutpoint::C11aAfterBackupBarrier {
            assert!(snapshot.barrier_pin_held);
            assert!(!record.destination_dir.exists());
        } else {
            let physical: unleash_store::BackupManifest = serde_json::from_slice(
                &std::fs::read(record.destination_dir.join("manifest.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(physical.state, BackupJobState::Complete);
            // M1 : validation portable des octets distincte de l'acceptation SQL du job géré.
            let validated =
                UnleashStore::validate_backup(&record.destination_dir, &config).unwrap();
            assert_eq!(validated.manifest.state, BackupJobState::Complete);
        }
        assert!(store.execute_reserved_backup(execute).is_err());
        let store = Arc::try_unwrap(store)
            .unwrap()
            .restart_same_lineage()
            .unwrap();
        store.reconcile_recovered_actions_after_restart().unwrap();
        assert_eq!(
            store
                .inspect_backup_request(record.backup_id)
                .unwrap()
                .unwrap()
                .action_state,
            Some(unleash_domain::ActionState::OutcomeUnknown)
        );
        assert_eq!(
            store
                .inspect_broker_action(record.action_id)
                .unwrap()
                .unwrap()
                .state,
            unleash_domain::ActionState::OutcomeUnknown
        );
        assert!(store.execute_reserved_backup(execute).is_err());
    }
}

#[cfg(unix)]
#[test]
fn managed_backup_root_symlink_is_rejected_without_following_it() {
    let (store, _ids, config, record, execute, _directory) =
        execution_fixture(Arc::new(unleash_store::NoopCutpointObserver));
    let external = config
        .backups_dir
        .parent()
        .unwrap()
        .join("external-fixture");
    std::fs::create_dir(&external).unwrap();
    std::os::unix::fs::symlink(&external, &config.backups_dir).unwrap();
    assert!(store.execute_reserved_backup(execute).is_err());
    assert_eq!(std::fs::read_dir(&external).unwrap().count(), 0);
    let snapshot = store
        .inspect_backup_request(record.backup_id)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.state, BackupRequestState::Executing);
    assert!(snapshot.barrier_pin_held);
    assert!(snapshot.manifest.is_none());
}

struct RootRace(std::sync::Barrier);
impl unleash_store::CutpointObserver for RootRace {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), StoreError> {
        if point == unleash_store::StoreCutpoint::C11gBeforeManagedBackupRootCreate {
            self.0.wait();
        }
        Ok(())
    }
}

#[test]
fn concurrent_backups_share_root_but_keep_exclusive_distinct_leaves() {
    let (store, ids, _config, first, execute, _directory) =
        execution_fixture(Arc::new(RootRace(std::sync::Barrier::new(2))));
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let second = store
        .claim_backup_request(backup, store.active_authority_generation().unwrap())
        .unwrap();
    let (_, _, _, dispatch) = dispatch(&store, &ids, &second);
    let second_execute = unleash_store::ExecuteReservedBackupRequest {
        backup_id: backup,
        action_id: second.action_id,
        dispatch_id: dispatch,
        authority_generation: second.authority_generation,
    };
    let first_store = store.clone();
    let first_worker = std::thread::spawn(move || first_store.execute_reserved_backup(execute));
    let second_store = store.clone();
    let second_worker =
        std::thread::spawn(move || second_store.execute_reserved_backup(second_execute));
    let first_result = first_worker.join().unwrap();
    let second_result = second_worker.join().unwrap();
    assert!(first_result.is_ok(), "{first_result:?}");
    assert!(second_result.is_ok(), "{second_result:?}");
    assert_ne!(first.destination_dir, second.destination_dir);
    assert!(first.destination_dir.join("db.sqlite").exists());
    assert!(second.destination_dir.join("db.sqlite").exists());
}

struct PausedAtBarrier {
    entered: std::sync::mpsc::SyncSender<()>,
    release: Arc<std::sync::Barrier>,
}
impl unleash_store::CutpointObserver for PausedAtBarrier {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), StoreError> {
        if point == unleash_store::StoreCutpoint::C11aAfterBackupBarrier {
            self.entered.send(()).unwrap();
            self.release.wait();
            return Err(StoreError::CutpointInjected {
                cutpoint: "C11a".to_owned(),
                message: "paused worker fixture".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn abandon_cannot_release_pins_of_an_active_reserved_executor() {
    let (entered, notification) = std::sync::mpsc::sync_channel(1);
    let release = Arc::new(std::sync::Barrier::new(2));
    let (store, _ids, _config, record, execute, _directory) =
        execution_fixture(Arc::new(PausedAtBarrier {
            entered,
            release: release.clone(),
        }));
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || worker_store.execute_reserved_backup(execute));
    notification
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let abandoned = store.abandon_backup_job(record.backup_id);
    let held = store
        .inspect_backup_request(record.backup_id)
        .unwrap()
        .unwrap()
        .barrier_pin_held;
    release.wait();
    assert!(worker.join().unwrap().is_err());
    assert!(abandoned.is_err());
    assert!(held);
}

#[test]
fn pre_dispatch_profile_block_is_durable_typed_and_never_reclassifies_a_dispatch() {
    let directory = local_test_dir("f018-013-block").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1306));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Normal,|tx,_| {
        tx.execute("INSERT INTO guard_configs (config_id,guard_mode,endpoint,transport,updated_by_actor,updated_at_ms) VALUES ('default','guard_required','https://guard.invalid','https_tls','p-sys',0)",[]).map_err(StoreError::from_rusqlite)?;
        Ok(())
    }).unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    let accepted = store
        .commit_command(request(&store, &ids, backup))
        .unwrap()
        .receipt()
        .clone();
    let record = store
        .claim_backup_request(backup, store.active_authority_generation().unwrap())
        .unwrap();
    assert!(
        store
            .block_claimed_backup(
                backup,
                record.authority_generation,
                unleash_store::BackupBlockReason::IsolationProfileUnavailable
            )
            .is_err()
    );
    let blocked = store
        .block_claimed_backup(
            backup,
            record.authority_generation,
            unleash_store::BackupBlockReason::GuardProfileUnavailable,
        )
        .unwrap();
    assert_eq!(blocked.state, BackupRequestState::Blocked);
    assert_eq!(
        blocked.block_reason,
        Some(unleash_store::BackupBlockReason::GuardProfileUnavailable)
    );
    assert_eq!(blocked.job_state, Some(BackupJobState::Aborted));
    assert!(!record.destination_dir.exists());
    let replay = store
        .block_claimed_backup(
            backup,
            record.authority_generation,
            unleash_store::BackupBlockReason::GuardProfileUnavailable,
        )
        .unwrap();
    assert_eq!(replay.last_event_cursor, blocked.last_event_cursor);
    let (events,receipt_link): (i64,String)=store.read_snapshot(|conn,_| {
        let events=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='BackupRequestBlocked'",[backup.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        let link=conn.query_row("SELECT causation_receipt_id FROM events WHERE aggregate_id=?1 AND event_type='BackupRequestBlocked'",[backup.to_string()],|r|r.get(0)).map_err(StoreError::from_rusqlite)?;
        Ok((events,link))
    }).unwrap();
    assert_eq!(events, 1);
    assert_eq!(receipt_link, accepted.receipt_id.to_string());
    let (dispatched, _ids, _cfg, record, _execute, _directory) =
        execution_fixture(Arc::new(unleash_store::NoopCutpointObserver));
    dispatched.execute_in_writer_tx(WriteCriticality::Normal,|tx,_| {
        tx.execute("INSERT INTO guard_configs (config_id,guard_mode,endpoint,transport,updated_by_actor,updated_at_ms) VALUES ('default','guard_required','https://guard.invalid','https_tls','p-sys',0)",[]).map_err(StoreError::from_rusqlite)?;
        Ok(())
    }).unwrap();
    assert!(
        dispatched
            .block_claimed_backup(
                record.backup_id,
                record.authority_generation,
                unleash_store::BackupBlockReason::GuardProfileUnavailable
            )
            .is_err()
    );
    assert_eq!(
        dispatched
            .inspect_backup_request(record.backup_id)
            .unwrap()
            .unwrap()
            .state,
        BackupRequestState::Claimed
    );
    assert_eq!(
        dispatched
            .inspect_broker_action(record.action_id)
            .unwrap()
            .unwrap()
            .state,
        unleash_domain::ActionState::Dispatching
    );
}

#[test]
fn restart_claims_only_an_unclaimed_request_in_the_current_epoch() {
    let directory = local_test_dir("f018-013-restart").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1307));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let original = store.inspect_backup_request(backup).unwrap().unwrap();
    let store = store.restart_same_lineage().unwrap();
    let claimed = store
        .claim_backup_request(backup, store.active_authority_generation().unwrap())
        .unwrap();
    assert_eq!(claimed.action_id, original.action_id);
    assert_ne!(claimed.lineage_fence.epoch, original.lineage_fence.epoch);
    assert_eq!(
        claimed.lineage_fence.epoch,
        store.daemon_state().unwrap().daemon_epoch
    );
    let store = store.restart_same_lineage().unwrap();
    assert!(
        store
            .claim_backup_request(backup, store.active_authority_generation().unwrap())
            .is_err()
    );
    assert_eq!(
        store
            .inspect_backup_request(backup)
            .unwrap()
            .unwrap()
            .lineage_fence,
        claimed.lineage_fence
    );
    assert!(!claimed.destination_dir.exists());
}

#[test]
fn real_restore_preserves_reserved_request_as_historical_and_never_reactivates_it() {
    let source = local_test_dir("f018-013-restore-source").unwrap();
    let target = local_test_dir("f018-013-restore-target").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1308));
    let mut config = StoreConfig::new(source.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal("p-sys", "system", "p-sys")
        .unwrap();
    let backup = BackupId::from_uuid(ids.new_uuid());
    store.commit_command(request(&store, &ids, backup)).unwrap();
    let record = store.inspect_backup_request(backup).unwrap().unwrap();
    let portable = source.path().join("portable-fixture");
    store
        .prepare_backup(unleash_store::PrepareBackupRequest {
            destination_dir: portable.clone(),
            retention_tier: unleash_store::BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let restore = unleash_store::RestoreBackupRequest {
        backup_dir: portable,
        target_data_root: target.path().to_owned(),
        command_id: ids.new_command_id(),
        actor_id: "p-sys".to_owned(),
        command_digest: unleash_domain::Sha256Digest::from_bytes(
            b"restore reserved request fixture",
        ),
        source_authority_generation: record.authority_generation,
        source_db_unreadable: false,
        source_db_path: Some(config.db_path),
        source_artifacts_dir: Some(config.artifacts_dir),
        neutralizer: Arc::new(unleash_store::EmptyExecutorNeutralizer),
    };
    UnleashStore::restore_backup(restore, StoreConfig::new(target.path())).unwrap();
    let restored = UnleashStore::open(StoreConfig::new(target.path())).unwrap();
    let current = restored.active_authority_generation().unwrap();
    assert_ne!(current, record.authority_generation);
    let historical = restored.inspect_backup_request(backup).unwrap().unwrap();
    assert_eq!(historical.action_id, record.action_id);
    assert_eq!(historical.authority_generation, record.authority_generation);
    assert_eq!(historical.state, BackupRequestState::Reserved);
    assert!(matches!(
        restored.claim_backup_request(backup, current),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert!(
        restored
            .inspect_broker_action(record.action_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn strict_profile_and_durable_denial_can_block_only_before_dispatch() {
    for strict in [false, true] {
        let directory = local_test_dir("f018-013-closed-reasons").unwrap();
        let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1309));
        let mut config = StoreConfig::new(directory.path());
        config.id_generator = ids.clone();
        config.clock = Arc::new(unleash_testkit::ManualClock::new(0));
        let store = UnleashStore::open(config).unwrap();
        store
            .register_principal("p-sys", "system", "p-sys")
            .unwrap();
        let backup = BackupId::from_uuid(ids.new_uuid());
        if strict {
            let mut policy = request(&store, &ids, backup);
            policy.target_aggregate = AggregateKind::SecurityPolicy;
            policy.envelope.aggregate_id = ids.new_uuid();
            policy.envelope.command_type = "SetPolicy".to_owned();
            policy.mutation = CommandMutation::SetPolicy {
                scope_kind: unleash_store::PolicyScopeKind::Global,
                scope_id: "global".to_owned(),
                isolation_mode: unleash_store::PolicyIsolationMode::Strict,
                policy_version: "strict-fixture".to_owned(),
                reason: "fixture".to_owned(),
                caller: unleash_store::CallerAuthority {
                    actor_id: ActorId::new("p-sys").unwrap(),
                    role: unleash_store::CallerRole::HumanOperator,
                },
            };
            store.commit_command(policy).unwrap();
        }
        store.commit_command(request(&store, &ids, backup)).unwrap();
        let record = store
            .claim_backup_request(backup, store.active_authority_generation().unwrap())
            .unwrap();
        if !strict {
            let intent = intent(&record);
            let evaluation = unleash_store::BrokerEvaluation {
                evaluation_id: unleash_domain::AuthorizationEvaluationId::from_uuid(ids.new_uuid()),
                previous_evaluation_id: None,
            };
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
            store
                .commit_broker_phase(phase(
                    1,
                    unleash_store::BrokerPhase::Authorization {
                        action_id: record.action_id,
                        intent_digest: intent.intent_digest,
                        outcome: unleash_store::BrokerAuthorizationOutcome::Failed {
                            failure: unleash_store::BrokerAuthorizationFailure {
                                code: unleash_protocol::ErrorCode::InvalidArgument,
                                reason_code: None,
                                retry_class: unleash_protocol::RetryClass::Never,
                                details_ref: None,
                                retry_not_before_ms: None,
                            },
                        },
                        cause: unleash_store::BrokerAuthorizationCause::Evaluation,
                    },
                ))
                .unwrap();
        }
        let reason = if strict {
            unleash_store::BackupBlockReason::IsolationProfileUnavailable
        } else {
            unleash_store::BackupBlockReason::AuthorizationDenied
        };
        let record = store
            .block_claimed_backup(backup, record.authority_generation, reason)
            .unwrap();
        assert_eq!(record.block_reason, Some(reason));
        assert_eq!(record.state, BackupRequestState::Blocked);
        assert!(record.dispatch_id.is_none());
        assert!(!record.destination_dir.exists());
    }
}
