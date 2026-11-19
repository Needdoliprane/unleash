//! T-F012-006/009/014/018 : reçu, contexte, origine et admission réels dans `SQLite`.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures synthétiques, sans réseau ni secret, et assertions de contrat"
)]

use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{
    AccountId, ActionOrigin, ActorId, AuthorityGeneration, DataLocality, Fence, MicrosAmount,
    ModelPolicy, ProviderProbeId, RequestId, Revision,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonical_sha256};
use unleash_store::{
    AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit,
    AdmissionUrgency, AggregateKind, BrokerPhase, BudgetAccountCharge, BudgetReservationRequest,
    BudgetScopeType, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CutpointObserver, ExecutionPoolParams, NoopCutpointObserver, PricingCheckStatus,
    ProbeGenerationLimits, ProviderProbePreparation, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};
use uuid::Uuid;

#[expect(
    dead_code,
    reason = "Shared helper also supplies the admission suite fixture"
)]
#[path = "common/provider_probe.rs"]
mod probe_bench;
use probe_bench::{phase, publish};

const PROBE: ProviderProbeId = ProviderProbeId::from_u128(1001);
const POOL: unleash_domain::PoolId = unleash_domain::PoolId::from_u128(1);

fn fixture(
    cutpoints: Arc<dyn CutpointObserver>,
) -> (
    UnleashStore,
    StoreConfig,
    ProviderProbePreparation,
    impl std::fmt::Debug,
) {
    let directory = local_test_dir("f012-probe-preparation").unwrap();
    let mut config = StoreConfig::new(directory.path());
    config.clock = Arc::new(ManualClock::new(0));
    config.id_generator = Arc::new(DeterministicIdGenerator::new(0x0F01_2014));
    config.cutpoints = cutpoints;
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal("probe-principal", "user", "user:probe")
        .unwrap();
    store
        .register_principal("other-principal", "user", "user:other")
        .unwrap();
    store
        .register_principal("probe-observer", "system", "system:probe-observer")
        .unwrap();
    store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,price_valid_until_ms,source_kind,created_at_ms) VALUES('fixture-profile@1','fixture-provider','approved_remote','fixture-price@1',120000,'user_declared',0)",[])?;Ok(())}).unwrap();
    let fixture = publish(
        &store,
        &json!({"schema_version":1,"fixture_id":"installed-probe-fixture@1","origin":"installed_synthetic_fixture","input":"Return probe-ok.","source_lineage":[]}),
    );
    let payload = publish(
        &store,
        &json!({"schema":"synthetic_probe_transport.v1","profile_version_ref":"fixture-profile@1","model":"fixture-model","input":"Return probe-ok.","max_output_tokens":128}),
    );
    let capabilities = publish(
        &store,
        &json!({"schema_version":1,"declared":["text"],"tested":[],"unknown":["text"],"provenance":"synthetic_sqlite_bench"}),
    );
    let policy = publish(
        &store,
        &json!({"schema_version":1,"model_policy":"allowed_bounded","source":"independent_configuration_command"}),
    );
    let data_policy = publish(
        &store,
        &json!({"schema_version":1,"data_locality":"approved_remote","content":"installed_synthetic_fixture_only"}),
    );
    let preparation = ProviderProbePreparation {
        requested_profile_id: "wire-profile".to_owned(),
        requested_profile_version: 1,
        profile_version_ref: "fixture-profile@1".to_owned(),
        resolved_model_id: "fixture-model".to_owned(),
        capability_set: vec!["text".to_owned()],
        capability_snapshot_hash: capabilities,
        fixture_id: "installed-probe-fixture@1".to_owned(),
        fixture_digest: fixture,
        payload_manifest_hash: payload,
        model_policy_hash: policy,
        data_policy_hash: data_policy,
        model_policy: ModelPolicy::AllowedBounded,
        locality: DataLocality::ApprovedRemote,
        mandate_ref: "operator-probe-fixture".to_owned(),
        source_run_id: None,
        secret_refs: vec![],
        limits: ProbeGenerationLimits {
            timeout_ms: 60_000,
            max_calls: 1,
            max_output_tokens: 128,
            max_cost_usd_micros: MicrosAmount::new(100_000),
        },
    };
    (store, config, preparation, directory)
}

fn request(
    store: &UnleashStore,
    preparation: ProviderProbePreparation,
    number: u128,
) -> CommitCommandRequest {
    let actor = ActorId::new("user:probe").unwrap();
    CommitCommandRequest{actor_id:actor.clone(),envelope:CommandEnvelope{protocol_version:ProtocolVersion,request_id:RequestId::from_u128(number),command_id:unleash_domain::CommandId::from_u128(number),authority_generation:store.active_authority_generation().unwrap(),actor_ref:Some(actor.to_string()),project_id:None,aggregate_id:PROBE.as_uuid(),expected_revision:Revision::INITIAL,command_type:"TestProviderProfile".to_owned(),payload:json!({"profile_id":"wire-profile","profile_version":1,"phase":"generation_probe","capability_set":preparation.capability_set,"limits":preparation.limits,"locality":preparation.locality,"mandate_ref":preparation.mandate_ref}).as_object().unwrap().clone()},target_aggregate:AggregateKind::ProviderProbe,operation_id:None,receipt_state:ReceiptState::Completed,result_ref:None,retention_days_override:None,fence_assertion:None,mutation:CommandMutation::PrepareProviderProbe{probe_id:PROBE,caller:CallerAuthority{actor_id:actor,role:CallerRole::HumanOperator},preparation},events:vec![],capacity_reservations:vec![],budget_reservations:vec![]}
}

fn counts(store: &UnleashStore) -> (i64, i64, i64, i64, i64, i64) {
    store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM provider_probes", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM actions", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM model_calls", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM model_call_manifests", [], |r| {
                    r.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM capacity_reservations", [], |r| {
                    r.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM budget_reservations", [], |r| r.get(0))?,
            ))
        })
        .unwrap()
}

#[test]
fn t_f012_009_authenticated_receipt_context_and_authorizing_replay_are_unique() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let command = request(&store, preparation.clone(), 11);
    let first = store.commit_command(command.clone()).unwrap();
    assert_eq!(first.receipt().state, ReceiptState::Accepted);
    assert_eq!(
        first.receipt().result_ref,
        Some(format!("provider-probe:{PROBE}"))
    );
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    assert_eq!(
        record.context.command_receipt_id,
        first.receipt().receipt_id
    );
    assert_eq!(record.context.actor_id.as_str(), "user:probe");
    assert_eq!(record.context.principal_id.as_str(), "probe-principal");
    assert_eq!(
        record.context.profile.profile_version_ref,
        "fixture-profile@1"
    );
    assert_eq!(counts(&store), (1, 0, 0, 0, 0, 0));
    assert!(
        store
            .execution_admission_snapshot(POOL)
            .unwrap()
            .queued
            .is_empty()
    );
    let replay = store.commit_command(command.clone()).unwrap();
    assert_eq!(replay.receipt(), first.receipt());
    assert_eq!(
        store.inspect_provider_probe(PROBE).unwrap().unwrap(),
        record
    );
    let mut divergent = command;
    if let CommandMutation::PrepareProviderProbe { preparation, .. } = &mut divergent.mutation {
        preparation.fixture_id = "another-installed-fixture".to_owned();
    }
    assert!(matches!(
        store.commit_command(divergent),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert!(
        store
            .commit_command(request(&store, preparation, 12))
            .is_err()
    );
    // The full request manifest must be published before Authorizing, not invented.
    let intent = phase(&record);
    assert!(store.commit_broker_phase(intent.clone()).is_err());
    assert_eq!(counts(&store), (1, 0, 0, 0, 0, 0));
    let manifest_hash = publish(&store, &record.request_manifest().unwrap());
    assert_eq!(
        manifest_hash,
        canonical_sha256(&record.request_manifest().unwrap()).unwrap()
    );
    let committed = store.commit_broker_phase(intent.clone()).unwrap();
    let replay = store.commit_broker_phase(intent).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let (agents, sessions, contexts): (i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM context_manifests", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((agents, sessions, contexts), (0, 0, 0));
    let document = serde_json::to_value(&record.context).unwrap();
    assert!(document.get("agent_id").is_none());
    assert!(document.get("session_id").is_none());
    assert_eq!(
        record.model_origin(),
        unleash_store::ModelOrigin::ProviderProbe {
            provider_probe_id: PROBE,
            probe_context_digest: record.context_digest
        }
    );
}

struct LoseIntentAck(AtomicBool);
impl CutpointObserver for LoseIntentAck {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C3AfterBrokerIntentCommit && self.0.swap(false, Ordering::AcqRel)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C3".to_owned(),
                message: "probe durable intent acknowledgement lost".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f012_009_lost_authorizing_ack_replays_the_same_relation_but_not_after_restart() {
    let failure = Arc::new(LoseIntentAck(AtomicBool::new(false)));
    let (store, _config, preparation, _directory) = fixture(failure.clone());
    store
        .commit_command(request(&store, preparation, 71))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    let intent = phase(&record);
    failure.0.store(true, Ordering::Release);
    assert!(matches!(
        store.commit_broker_phase(intent.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let replay = store.commit_broker_phase(intent.clone()).unwrap();
    assert!(replay.replayed);
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let store = store.restart_same_lineage().unwrap();
    assert!(matches!(
        store.commit_broker_phase(intent),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
}

#[test]
fn t_f012_018_restored_preparation_is_history_without_new_authority() {
    let (store, config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, 111))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    let portable = config.data_root.join("portable-probe-fixture");
    store
        .prepare_backup(unleash_store::PrepareBackupRequest {
            destination_dir: portable.clone(),
            retention_tier: unleash_store::BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();
    let target = local_test_dir("f012-probe-restored").unwrap();
    let target_config = StoreConfig::new(target.path());
    UnleashStore::restore_backup(
        unleash_store::RestoreBackupRequest {
            backup_dir: portable,
            target_data_root: target.path().to_owned(),
            actor_id: "user:probe".to_owned(),
            command_id: unleash_domain::CommandId::from_u128(112),
            command_digest: unleash_domain::Sha256Digest::from_bytes(
                b"synthetic restore of prepared probe",
            ),
            source_authority_generation: record.context.authority_generation,
            source_db_unreadable: false,
            source_db_path: Some(config.db_path),
            source_artifacts_dir: Some(config.artifacts_dir),
            neutralizer: Arc::new(unleash_store::EmptyExecutorNeutralizer),
        },
        target_config.clone(),
    )
    .unwrap();
    let restored = UnleashStore::open(target_config).unwrap();
    assert_ne!(
        restored.active_authority_generation().unwrap(),
        record.context.authority_generation
    );
    assert_eq!(
        restored.inspect_provider_probe(PROBE).unwrap().unwrap(),
        record
    );
    let mut intent = phase(&record);
    intent.authority_generation = restored.active_authority_generation().unwrap();
    assert!(matches!(
        restored.commit_broker_phase(intent),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    assert_eq!(counts(&restored), (1, 0, 0, 0, 0, 0));
}

#[test]
fn t_f012_014_preparation_and_authorization_do_not_open_transport_dispatch() {
    use unleash_store::{
        BrokerAuthorizationCause, BrokerAuthorizationDecision, BrokerAuthorizationOutcome,
        BrokerExecutor,
    };
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, 121))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    let start = phase(&record);
    store.commit_broker_phase(start.clone()).unwrap();
    let BrokerPhase::Authorizing { intent } = &start.phase else {
        panic!("fixture intent");
    };
    let decision = BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: start.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "synthetic-allow-probe".to_owned(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(intent.action_deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let mut authorization = start.clone();
    authorization.expected_revision = Revision::new(1);
    authorization.phase = BrokerPhase::Authorization {
        action_id: intent.action_id,
        intent_digest: intent.intent_digest,
        outcome: BrokerAuthorizationOutcome::Authorized {
            decision: decision.clone(),
        },
        cause: BrokerAuthorizationCause::Evaluation,
    };
    let committed = store.commit_broker_phase(authorization).unwrap();
    let mut dispatch = start.clone();
    dispatch.expected_revision = committed.revision;
    dispatch.phase = BrokerPhase::Dispatch {
        action_id: intent.action_id,
        intent_digest: intent.intent_digest,
        decision,
        dispatch_id: unleash_domain::DispatchId::from_u128(12100),
        fence: intent.lineage_fence,
    };
    let result = store.commit_broker_phase(dispatch);
    assert!(
        matches!(result,Err(StoreError::InvalidArgument{message,..}) if message.contains("dispatch probe fermé"))
    );
    let dispatches: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row("SELECT COUNT(*) FROM action_dispatches", [], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(dispatches, 0);
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
}

#[test]
fn t_f012_006_probe_budget_and_capacity_exist_only_after_the_v008_writer_port() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, 21))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    let before = store.execution_admission_snapshot(POOL).unwrap();
    assert!(
        store
            .record_admission_eligibility(AdmissionEligibilityObservation {
                evidence_id: Uuid::from_u128(99),
                observer_principal_id: "probe-observer".to_owned(),
                object_hash: record.context.preparation.capability_snapshot_hash,
                unit: AdmissionUnit::Probe(PROBE),
                unit_revision: Revision::new(1),
                source_revision: before.source_revision,
                authority_generation: before.authority_generation,
                daemon_epoch: before.daemon_epoch,
                valid_until: record.context.deadline,
                urgency: AdmissionUrgency::Ordinary
            })
            .is_err()
    );
    publish(&store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let account = AccountId::from_u128(4001);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1_000_000),
        )
        .unwrap();
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let candidate = snapshot.queued.first().unwrap();
    assert_eq!(candidate.unit, AdmissionUnit::Probe(PROBE));
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: Uuid::from_u128(100),
            observer_principal_id: "probe-observer".to_owned(),
            object_hash: record.context.preparation.capability_snapshot_hash,
            unit: candidate.unit,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: record.context.deadline,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    let ready = store.execution_admission_snapshot(POOL).unwrap();
    let action = record.context.action_id;
    let digest = record.payload_digest();
    assert!(
        store
            .commit_execution_admission(POOL, ready.token, move |_| Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(PROBE),
                deficit_deltas: BTreeMap::from([(None, 0)]),
                consecutive_controls: 0,
                budgets: vec![]
            }))
            .is_err(),
        "a remote probe must reserve its frozen budget before admission"
    );
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let outcome = store
        .commit_execution_admission(POOL, ready.token, move |_| {
            Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(PROBE),
                deficit_deltas: BTreeMap::from([(None, 0)]),
                consecutive_controls: 0,
                budgets: vec![BudgetReservationRequest {
                    action_id: action,
                    model_id: "fixture-model".to_owned(),
                    pricing_version: "fixture-price@1".to_owned(),
                    pricing_status: PricingCheckStatus::Valid,
                    max_request_hash: digest,
                    charges: vec![BudgetAccountCharge {
                        account_id: account,
                        max_micros: MicrosAmount::new(100_000),
                    }],
                }],
            })
        })
        .unwrap();
    let AdmissionOutcome::Admitted(admitted) = outcome else {
        panic!("probe must be admitted");
    };
    assert_eq!(admitted.unit, AdmissionUnit::Probe(PROBE));
    assert!(admitted.attempt_id.is_none());
    assert_eq!(admitted.lease.fence, Fence::new(1));
    assert_eq!(counts(&store), (1, 1, 1, 1, 1, 1));
    assert_eq!(
        store.budget_account(account).unwrap().reserved_micros,
        MicrosAmount::new(100_000)
    );
}

#[test]
fn t_f012_009_changed_profile_cannot_become_a_new_admission() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, 81))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{tx.execute("UPDATE provider_profiles SET pricing_version='changed-price@2' WHERE profile_version_ref='fixture-profile@1'",[])?;Ok(())}).unwrap();
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let candidate = snapshot.queued.first().unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: Uuid::from_u128(8100),
            observer_principal_id: "probe-observer".to_owned(),
            object_hash: record.context.preparation.capability_snapshot_hash,
            unit: candidate.unit,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: record.context.deadline,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    let ready = store.execution_admission_snapshot(POOL).unwrap();
    let result = store.commit_execution_admission(POOL, ready.token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit: AdmissionUnit::Probe(PROBE),
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![],
        })
    });
    assert!(
        matches!(result,Err(StoreError::InvalidArgument{message,..}) if message.contains("profil du probe changé")),
        "frozen profile must be revalidated inside the admission writer"
    );
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
}

fn observe_probe(
    store: &UnleashStore,
    record: &unleash_store::PreparedProviderProbe,
) -> unleash_store::AdmissionSnapshotToken {
    observe_probe_with_id(store, record, 9000)
}

fn observe_probe_with_id(
    store: &UnleashStore,
    record: &unleash_store::PreparedProviderProbe,
    evidence: u128,
) -> unleash_store::AdmissionSnapshotToken {
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let candidate = snapshot
        .queued
        .iter()
        .find(|row| row.unit == AdmissionUnit::Probe(PROBE))
        .unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: Uuid::from_u128(evidence),
            observer_principal_id: "probe-observer".to_owned(),
            object_hash: record.context.preparation.capability_snapshot_hash,
            unit: candidate.unit,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: record.context.deadline,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    store.execution_admission_snapshot(POOL).unwrap().token
}

fn frozen_budget(
    record: &unleash_store::PreparedProviderProbe,
    accounts: Vec<AccountId>,
) -> BudgetReservationRequest {
    BudgetReservationRequest {
        action_id: record.context.action_id,
        model_id: record.context.preparation.resolved_model_id.clone(),
        pricing_version: record.context.profile.pricing_version.clone(),
        pricing_status: PricingCheckStatus::Valid,
        max_request_hash: record.payload_digest(),
        charges: accounts
            .into_iter()
            .map(|account_id| BudgetAccountCharge {
                account_id,
                max_micros: record.context.preparation.limits.max_cost_usd_micros,
            })
            .collect(),
    }
}

#[test]
fn t_f012_006_omitted_exhausted_project_account_never_bypasses_the_budget() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let project = unleash_domain::ProjectId::from_u128(91);
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{tx.execute("INSERT INTO projects(project_id,name,state,revision,created_at_ms) VALUES(?1,'probe-project','active',0,?2)",rusqlite::params![project.to_string(),now.as_i64()])?;Ok(())}).unwrap();
    let mut command = request(&store, preparation, 91);
    command.envelope.project_id = Some(project);
    store.commit_command(command).unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
    let global = AccountId::from_u128(9100);
    let project_account = AccountId::from_u128(9101);
    store
        .upsert_budget_account(
            global,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1_000_000),
        )
        .unwrap();
    store
        .upsert_budget_account(
            project_account,
            BudgetScopeType::Project,
            &project.to_string(),
            "USD",
            MicrosAmount::ZERO,
        )
        .unwrap();
    let token = observe_probe(&store, &record);
    let budget = frozen_budget(&record, vec![global]);
    assert!(
        store
            .commit_execution_admission(POOL, token, move |_| Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(PROBE),
                deficit_deltas: BTreeMap::from([(Some(project), 0)]),
                consecutive_controls: 0,
                budgets: vec![budget]
            }))
            .is_err(),
        "all applicable accounts must be reserved, including an exhausted project"
    );
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    let budget = frozen_budget(&record, vec![global, project_account]);
    assert!(
        store
            .commit_execution_admission(POOL, token, move |_| Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(PROBE),
                deficit_deltas: BTreeMap::from([(Some(project), 0)]),
                consecutive_controls: 0,
                budgets: vec![budget]
            }))
            .is_err()
    );
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    assert_eq!(
        store.budget_account(global).unwrap().reserved_micros,
        MicrosAmount::ZERO
    );
}

#[test]
fn t_f012_018_suspended_principal_is_rechecked_before_admission() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    store
        .commit_command(request(&store, preparation, 101))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,now|{tx.execute("INSERT INTO subject_suspensions(suspension_id,subject_kind,subject_id,scope_kind,scope_id,status,reason,acted_by_actor,revision,created_at_ms,updated_at_ms) VALUES('probe-suspension','actor','probe-principal','global','default','suspended','synthetic fixture','user:probe',1,?1,?1)",[now.as_i64()])?;Ok(())}).unwrap();
    let account = AccountId::from_u128(10100);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1_000_000),
        )
        .unwrap();
    let token = observe_probe(&store, &record);
    let budget = frozen_budget(&record, vec![account]);
    let result = store.commit_execution_admission(POOL, token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit: AdmissionUnit::Probe(PROBE),
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![budget],
        })
    });
    assert!(
        matches!(result, Err(StoreError::CapabilityDenied { .. })),
        "principal suspension is distinct from actor suspension"
    );
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
}

#[test]
fn t_f012_006_local_unpriced_probe_cannot_skip_an_existing_budget_overrun() {
    let (store, _config, mut preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    preparation.locality = DataLocality::LocalOnly;
    preparation.limits.max_cost_usd_micros = MicrosAmount::ZERO;
    store.execute_in_writer_tx(WriteCriticality::Critical,|tx,_|{tx.execute("UPDATE provider_profiles SET locality='local_only',pricing_version='none_local',price_valid_until_ms=NULL WHERE profile_version_ref='fixture-profile@1'",[])?;Ok(())}).unwrap();
    store
        .commit_command(request(&store, preparation, 131))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    assert_eq!(record.context.profile.price_valid_until_ms, None);
    publish(&store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
    let account = AccountId::from_u128(13100);
    store
        .upsert_budget_account(
            account,
            BudgetScopeType::Global,
            "global",
            "USD",
            MicrosAmount::new(1),
        )
        .unwrap();
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            tx.execute(
                "UPDATE budget_accounts SET spent_micros=2 WHERE account_id=?1",
                [account.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    let token = observe_probe(&store, &record);
    let result = store.commit_execution_admission(POOL, token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit: AdmissionUnit::Probe(PROBE),
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![],
        })
    });
    assert!(matches!(result, Err(StoreError::BudgetExhausted { .. })));
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
    // At the limit, zero-price local work can advance without inventing a price
    // or a monetary reservation. Its time/token bounds remain frozen.
    store
        .execute_in_writer_tx(WriteCriticality::Critical, move |tx, _| {
            tx.execute(
                "UPDATE budget_accounts SET spent_micros=1 WHERE account_id=?1",
                [account.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    let token = observe_probe_with_id(&store, &record, 13101);
    assert!(matches!(
        store
            .commit_execution_admission(POOL, token, move |_| Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(PROBE),
                deficit_deltas: BTreeMap::from([(None, 0)]),
                consecutive_controls: 0,
                budgets: vec![]
            }))
            .unwrap(),
        AdmissionOutcome::Admitted(_)
    ));
    assert_eq!(counts(&store), (1, 1, 1, 1, 1, 0));
}

#[test]
fn t_f012_018_wrong_authority_owner_and_forbidden_source_never_materialize() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let mut forbidden = preparation.clone();
    forbidden.model_policy = ModelPolicy::Forbidden;
    assert!(
        store
            .commit_command(request(&store, forbidden, 31))
            .is_err()
    );
    assert_eq!(counts(&store), (0, 0, 0, 0, 0, 0));
    let mut agent = request(&store, preparation.clone(), 32);
    if let CommandMutation::PrepareProviderProbe { caller, .. } = &mut agent.mutation {
        caller.role = CallerRole::Agent;
    }
    assert!(store.commit_command(agent).is_err());
    store
        .commit_command(request(&store, preparation, 33))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    let mut wrong = phase(&record);
    if let BrokerPhase::Authorizing { intent } = &mut wrong.phase {
        intent.principal_id = ActorId::new("other-principal").unwrap();
        intent.origin = ActionOrigin::ControlPlane {
            control_principal_id: intent.principal_id.clone(),
        };
        intent
            .manifest
            .get_mut("subject")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("principal_id".to_owned(), json!(intent.principal_id));
        intent.intent_digest = canonical_sha256(&intent.manifest).unwrap();
    }
    assert!(store.commit_broker_phase(wrong).is_err());
    let mut stale = phase(&record);
    stale.authority_generation = AuthorityGeneration::from_u128(777);
    assert!(matches!(
        store.commit_broker_phase(stale),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let store = store.restart_same_lineage().unwrap();
    assert!(matches!(
        store.commit_broker_phase(phase(&record)),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(counts(&store), (1, 0, 0, 0, 0, 0));
}

struct FailBeforeCommit(AtomicBool);
impl CutpointObserver for FailBeforeCommit {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeCommandCommit && self.0.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C1".to_owned(),
                message: "probe rollback fixture".to_owned(),
            });
        }
        Ok(())
    }
}

#[test]
fn t_f012_009_authorizing_rollback_has_no_action_call_manifest_or_admission_remainder() {
    let failure = Arc::new(FailBeforeCommit(AtomicBool::new(false)));
    let (store, _config, preparation, _directory) = fixture(failure.clone());
    store
        .commit_command(request(&store, preparation, 41))
        .unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    publish(&store, &record.request_manifest().unwrap());
    failure.0.store(true, Ordering::Release);
    assert!(matches!(
        store.commit_broker_phase(phase(&record)),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&store), (1, 0, 0, 0, 0, 0));
    assert!(
        store
            .execution_admission_snapshot(POOL)
            .unwrap()
            .queued
            .is_empty()
    );
    store.commit_broker_phase(phase(&record)).unwrap();
    assert_eq!(counts(&store), (1, 1, 1, 1, 0, 0));
}

#[test]
fn t_f012_009_wire_profile_and_resolved_preparation_must_agree() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let mut wrong = request(&store, preparation.clone(), 51);
    wrong
        .envelope
        .payload
        .insert("profile_id".to_owned(), json!("another-profile"));
    assert!(
        store.commit_command(wrong).is_err(),
        "wire pair diverging from trusted resolution must be refused"
    );
    assert_eq!(counts(&store), (0, 0, 0, 0, 0, 0));
    let mut wrong = request(&store, preparation.clone(), 52);
    wrong
        .envelope
        .payload
        .insert("profile_version".to_owned(), json!(2));
    assert!(store.commit_command(wrong).is_err());
    assert_eq!(counts(&store), (0, 0, 0, 0, 0, 0));
    let correct = request(&store, preparation, 53);
    store.commit_command(correct).unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    assert_eq!(
        record.context.preparation.requested_profile_id,
        "wire-profile"
    );
    assert_eq!(record.context.preparation.requested_profile_version, 1);
}

#[test]
fn t_f012_009_legacy_probe_keeps_its_state_but_cannot_acquire_new_capacity() {
    let (store, _config, preparation, _directory) = fixture(Arc::new(NoopCutpointObserver));
    let receipt = store
        .commit_command(request(&store, preparation, 61))
        .unwrap()
        .receipt()
        .receipt_id;
    let daemon = store.daemon_state().unwrap();
    let old = ProviderProbeId::from_u128(888);
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,profile_version_ref,context_hash,phase,state,admission_class,authority_generation,daemon_epoch,fence,revision,queued_at_ms) VALUES(?1,?2,'probe-principal','fixture-profile@1','legacy-unqualified','generation_probe','queued','work',?3,?4,0,0,?5)",rusqlite::params![old.to_string(),receipt.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;Ok(())
    }).unwrap();
    let snapshot = store.execution_admission_snapshot(POOL).unwrap();
    let record = store.inspect_provider_probe(PROBE).unwrap().unwrap();
    store
        .record_admission_eligibility(AdmissionEligibilityObservation {
            evidence_id: Uuid::from_u128(888),
            observer_principal_id: "probe-observer".to_owned(),
            object_hash: record.context.preparation.capability_snapshot_hash,
            unit: AdmissionUnit::Probe(old),
            unit_revision: Revision::INITIAL,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: record.context.deadline,
            urgency: AdmissionUrgency::Ordinary,
        })
        .unwrap();
    let ready = store.execution_admission_snapshot(POOL).unwrap();
    assert!(
        store
            .commit_execution_admission(POOL, ready.token, move |_| Ok(AdmissionDecision::Admit {
                unit: AdmissionUnit::Probe(old),
                deficit_deltas: BTreeMap::from([(None, 0)]),
                consecutive_controls: 0,
                budgets: vec![]
            }))
            .is_err(),
        "legacy probe has no authenticated V009 context/action/manifest"
    );
    let (state, capacity): (String, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT state FROM provider_probes WHERE probe_id=?1",
                    [old.to_string()],
                    |r| r.get(0),
                )?,
                conn.query_row("SELECT COUNT(*) FROM capacity_reservations", [], |r| {
                    r.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(capacity, 0);
}
