//! Oracles partiels F011 dans `SQLite` ; aucun host ni processus n'est lancé.
#![expect(
    clippy::unwrap_used,
    reason = "Fixtures synthétiques locales et assertions de contrat"
)]
use serde_json::json;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};
use unleash_domain::{
    ActionOrigin, ActorId, AuthorityGeneration, AuthorizationEvaluationId, DispatchId, Fence,
    LineageFence, MissionId, PoolId, ProcessRunUuid, ProjectId, RequestId, Revision, Sha256Digest,
    TerminalId,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ReceiptState, canonical_sha256, canonicalize_to_vec,
};
use unleash_store::*;
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

const PROJECT: ProjectId = ProjectId::from_u128(11);
const MISSION: MissionId = MissionId::from_u128(12);
fn fixture(
    observer: Arc<dyn CutpointObserver>,
) -> (
    Arc<UnleashStore>,
    StoreConfig,
    Arc<ManualClock>,
    TerminalAllocationPreparation,
    impl std::fmt::Debug,
) {
    fixture_at_schema(observer, CURRENT_SCHEMA_VERSION)
}
fn fixture_at_schema(
    observer: Arc<dyn CutpointObserver>,
    schema: i64,
) -> (
    Arc<UnleashStore>,
    StoreConfig,
    Arc<ManualClock>,
    TerminalAllocationPreparation,
    impl std::fmt::Debug,
) {
    let dir = local_test_dir("f011-store-synthetic").unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let clock = Arc::new(ManualClock::new(0));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.max_supported_schema_version = schema;
    cfg.clock = clock.clone();
    cfg.id_generator = Arc::new(DeterministicIdGenerator::new(0xF011_030C));
    cfg.cutpoints = observer;
    let store = Arc::new(UnleashStore::open(cfg.clone()).unwrap());
    store
        .register_principal("human-alice", "user", "uid:fixture-alice")
        .unwrap();
    store
        .register_principal("human-bob", "user", "uid:fixture-bob")
        .unwrap();
    store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap();
    let root_string = root.canonicalize().unwrap().to_str().unwrap().to_owned();
    let saved = root_string.clone();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
  tx.execute("INSERT INTO projects(project_id,name,state,revision,created_at_ms,primary_root_path) VALUES(?1,'synthetic terminal fixture','active',2,?2,?3)",rusqlite::params![PROJECT.to_string(),now.as_i64(),saved])?;
  tx.execute("INSERT INTO missions(mission_id,project_id,owner_kind,owner_user_id,completion_policy,state,revision) VALUES(?1,?2,'user','human-alice','human','active',1)",rusqlite::params![MISSION.to_string(),PROJECT.to_string()])?;
  Ok(())
 }).unwrap();
    let runtime = configure_fixture_runtime(&store);
    let profile = runtime.profiles.first().unwrap();
    let prep = TerminalAllocationPreparation {
        mission_id: MISSION,
        mission_revision: Revision::new(1),
        scope: TerminalAllocationScope::ProjectConfiguration {
            project_id: PROJECT,
            revision: Revision::new(2),
        },
        purpose_key: "fixture-terminal".to_owned(),
        process_spec: NormalizedTerminalProcessSpec {
            program: "/usr/bin/true".to_owned(),
            arguments: vec!["literal; not a shell".to_owned()],
            requested_cwd: ".".to_owned(),
            cwd_canonical: root_string,
            environment: TerminalEnvironmentRef {
                profile_id: profile.profile_id.clone(),
                version: profile.version,
                digest: profile.digest().unwrap(),
            },
            rows: 24,
            cols: 80,
            readiness_timeout_ms: 10_000,
        },
        auxiliary: runtime.pools(),
        lease_ttl_ms: runtime.lease_ttl_ms,
    };
    (store, cfg, clock, prep, dir)
}

fn configure_fixture_runtime(
    store: &UnleashStore,
) -> terminal_allocations::TerminalRuntimeSettings {
    let runtime:terminal_allocations::TerminalRuntimeSettings=serde_json::from_value(json!({"schema_version":1,"profiles":[{"profile_id":"store-terminal-fixture","version":1,"variables":{"LANG":"C"}}],"max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000})).unwrap();
    let command = unleash_domain::CommandId::from_u128(0xf011_fe01);
    let prepared = runtime
        .profiles
        .iter()
        .map(|p| store.prepare_terminal_environment(p, command).unwrap())
        .collect::<Vec<_>>();
    let patch = SettingsPatch {
        extra_typed: Some(
            json!({"terminal_runtime":runtime,"__terminal_prepared":prepared})
                .as_object()
                .unwrap()
                .clone(),
        ),
        ..SettingsPatch::default()
    };
    let actor = ActorId::new("uid:fixture-alice").unwrap();
    let authority = store.active_authority_generation().unwrap();
    store.commit_command(CommitCommandRequest {actor_id:actor.clone(),envelope:CommandEnvelope {protocol_version:ProtocolVersion,request_id:RequestId::from_u128(0xf011_fe02),command_id:command,authority_generation:authority,actor_ref:None,project_id:None,aggregate_id:authority.as_uuid(),expected_revision:Revision::INITIAL,command_type:"SetSettings".into(),payload:json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":runtime}}).as_object().unwrap().clone()},target_aggregate:AggregateKind::Settings,operation_id:None,receipt_state:ReceiptState::Accepted,result_ref:None,retention_days_override:None,fence_assertion:None,mutation:CommandMutation::SetSettings {scope_kind:SettingsScopeKind::User,scope_id:"default".into(),caller:CallerAuthority {actor_id:actor,role:CallerRole::PolicyAdmin},patch},events:vec![],capacity_reservations:vec![],budget_reservations:vec![]}).unwrap();
    runtime
}
fn request(
    store: &UnleashStore,
    prep: TerminalAllocationPreparation,
    n: u128,
) -> CommitCommandRequest {
    let actor = ActorId::new("uid:fixture-alice").unwrap();
    let terminal = TerminalId::from_u128(n);
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(n + 2000),
            command_id: unleash_domain::CommandId::from_u128(n + 1000),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some(actor.to_string()),
            project_id: Some(PROJECT),
            aggregate_id: terminal.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "AcquireTerminal".to_owned(),
            payload: prep.command_payload().as_object().unwrap().clone(),
        },
        target_aggregate: AggregateKind::TerminalAllocation,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PrepareTerminalAllocation {
            terminal_id: terminal,
            caller: CallerAuthority {
                actor_id: actor,
                role: CallerRole::HumanOperator,
            },
            preparation: prep,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}
fn read(store: &UnleashStore, id: u128) -> TerminalAllocationRecord {
    store
        .inspect_terminal_allocation_snapshot(TerminalId::from_u128(id))
        .unwrap()
        .value
        .unwrap()
}
fn counts(store: &UnleashStore) -> (i64, i64, i64, i64, i64, i64, i64) {
    store
        .read_snapshot(|c, _| {
            Ok((
                c.query_row("SELECT COUNT(*) FROM terminal_allocations", [], |r| {
                    r.get(0)
                })?,
                c.query_row("SELECT COUNT(*) FROM actions", [], |r| r.get(0))?,
                c.query_row("SELECT COUNT(*) FROM processes", [], |r| r.get(0))?,
                c.query_row("SELECT COUNT(*) FROM terminal_sessions", [], |r| r.get(0))?,
                c.query_row("SELECT COUNT(*) FROM resource_leases", [], |r| r.get(0))?,
                c.query_row("SELECT COUNT(*) FROM capacity_reservations", [], |r| {
                    r.get(0)
                })?,
                // Configuration is an accepted fixture prerequisite. Count
                // only terminal command receipts for these allocation oracles.
                c.query_row(
                    "SELECT COUNT(*) FROM command_receipts WHERE command_type='AcquireTerminal'",
                    [],
                    |r| r.get(0),
                )?,
            ))
        })
        .unwrap()
}
fn authorizing(record: &TerminalAllocationRecord) -> BrokerPhaseRequest {
    let c = &record.context;
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let manifest=json!({"action_id":c.action_id,"subject":{"kind":"control_plane","principal_id":c.principal_id,"cause_ref":format!("receipt:{}",c.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},"project_id":project_id,"verb":TERMINAL_ALLOCATION_VERB,"resource_refs":record.resource_refs(),"payload_digest":record.payload_digest(),"reconcile_strategy":"observe_then_decide","effects_class":"process_exec","scope_revision":revision,"policy_revision":c.policy_revision,"lineage_fence":fence,"action_deadline":c.lease_expires_at,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}}).as_object().unwrap().clone();
    BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_uuid(c.action_id.as_uuid()),
            previous_evaluation_id: None,
        },
        phase: BrokerPhase::Authorizing {
            intent: BrokerIntent {
                action_id: c.action_id,
                origin: ActionOrigin::ControlPlane {
                    control_principal_id: c.principal_id.clone(),
                },
                principal_id: c.principal_id.clone(),
                cause: BrokerCause::CommandReceipt {
                    receipt_id: c.receipt_id,
                },
                project_id: Some(project_id),
                payload_digest: record.payload_digest(),
                scope_revision: revision,
                policy_revision: c.policy_revision,
                lineage_fence: fence,
                action_deadline: c.lease_expires_at,
                reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
                intent_digest: canonical_sha256(&manifest).unwrap(),
                manifest,
            },
        },
    }
}
fn dispatch(store: &UnleashStore, record: &TerminalAllocationRecord) -> DispatchId {
    let first = authorizing(record);
    let committed = store.commit_broker_phase(first.clone()).unwrap();
    let BrokerPhase::Authorizing { intent } = first.phase.clone() else {
        unreachable!()
    };
    let decision = BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: first.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: "synthetic-sqlite-allow".to_owned(),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(intent.action_deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    let auth = store
        .commit_broker_phase(BrokerPhaseRequest {
            expected_revision: committed.revision,
            phase: BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
            ..first.clone()
        })
        .unwrap();
    let id = DispatchId::from_uuid(record.context.process_id.as_uuid());
    store
        .commit_broker_phase(BrokerPhaseRequest {
            expected_revision: auth.revision,
            phase: BrokerPhase::Dispatch {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                decision,
                dispatch_id: id,
                fence: intent.lineage_fence,
            },
            ..first
        })
        .unwrap();
    id
}
fn resolution(
    record: &TerminalAllocationRecord,
    dispatch_id: DispatchId,
) -> ResolveTerminalLaunchRequest {
    let c = &record.context;
    ResolveTerminalLaunchRequest {
        terminal_id: c.terminal_id,
        process_run_uuid: c.process_run_uuid,
        action_id: c.action_id,
        dispatch_id,
        authority_generation: c.authority_generation,
        expected_revision: record.revision,
        context_digest: record.context_digest,
        payload_digest: record.payload_digest(),
        caller: CallerAuthority {
            actor_id: c.actor_id.clone(),
            role: CallerRole::HumanOperator,
        },
        lease: FenceAssertion {
            lease_id: c.lease_id,
            authority_generation: c.authority_generation,
            daemon_epoch: c.daemon_epoch,
            fence: c.resource_fence,
            owner_id: c.principal_id.to_string(),
        },
    }
}
#[test]
fn t_f011_001_prepare_receipt_replay_and_exclusive_human_owner() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    let req = request(&s, p.clone(), 101);
    let receipt = s.commit_command(req.clone()).unwrap();
    assert_eq!(receipt.receipt().state, ReceiptState::Accepted);
    let record = read(&s, 101);
    assert_eq!(record.context.receipt_id, receipt.receipt().receipt_id);
    assert_eq!(record.context.actor_id.as_str(), "uid:fixture-alice");
    assert_eq!(record.context.principal_id.as_str(), "human-alice");
    assert_eq!(record.context.preparation, p);
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
    assert_eq!(
        s.execution_pool_state(PoolId::from_u128(1))
            .unwrap()
            .current_occupation,
        0
    );
    assert_eq!(
        s.commit_command(req.clone()).unwrap().receipt(),
        receipt.receipt()
    );
    assert_eq!(read(&s, 101), record);
    let mut divergent = req;
    if let CommandMutation::PrepareTerminalAllocation { preparation, .. } = &mut divergent.mutation
    {
        preparation.process_spec.environment.version = 2;
    }
    assert!(matches!(
        s.commit_command(divergent),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert!(s.commit_command(request(&s, p.clone(), 102)).is_err());
    let mut bob = request(&s, p, 103);
    let actor = ActorId::new("uid:fixture-bob").unwrap();
    bob.actor_id = actor.clone();
    bob.envelope.actor_ref = Some(actor.to_string());
    if let CommandMutation::PrepareTerminalAllocation { caller, .. } = &mut bob.mutation {
        caller.actor_id = actor;
    }
    assert!(s.commit_command(bob).is_err());
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
}

#[test]
fn t_f011_001_missing_runtime_configuration_cannot_prepare_new_terminal() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    // Models historical storage without the new settings, retaining published
    // objects. A published object alone must never become an environment grant.
    s.execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
        tx.execute(
            "DELETE FROM settings WHERE scope_kind='user' AND scope_id='default'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let before = counts(&s);
    let result = s.commit_command(request(&s, p, 1601));
    assert!(
        matches!(&result,Err(StoreError::InvalidArgument {message,..}) if message=="terminal runtime is not configured"),
        "unexpected result: {result:?}"
    );
    assert_eq!(
        counts(&s),
        before,
        "no receipt, intention, lease, hold, process or PTY without configuration"
    );
}

#[test]
fn t_f011_003_missing_runtime_preserves_known_receipt_and_phase_but_never_grants_launch() {
    for committed_dispatch in [false, true] {
        let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
        let req = request(&s, p, 1701);
        let receipt = s.commit_command(req.clone()).unwrap();
        let record = read(&s, 1701);
        let marker = committed_dispatch.then(|| dispatch(&s, &record));
        s.execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
            tx.execute(
                "DELETE FROM settings WHERE scope_kind='user' AND scope_id='default'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let before = counts(&s);
        assert_eq!(
            s.commit_command(req).unwrap().receipt(),
            receipt.receipt(),
            "historical wire replay does not revalidate new settings"
        );
        if let Some(marker) = marker {
            assert!(
                s.commit_broker_phase(authorizing(&record))
                    .unwrap()
                    .replayed,
                "exact acquired phase remains historical"
            );
            let refused = s.resolve_terminal_launch(resolution(&record, marker));
            assert!(
                matches!(refused,Err(StoreError::InvalidArgument {message,..}) if message=="terminal runtime is not configured")
            );
            assert_eq!(
                read(&s, 1701).state,
                TerminalAllocationState::Allocated,
                "no launch permit consumed"
            );
        } else {
            let refused = s.commit_broker_phase(authorizing(&record));
            assert!(
                matches!(refused,Err(StoreError::InvalidArgument {message,..}) if message=="terminal runtime is not configured")
            );
        }
        assert_eq!(counts(&s), before);
        assert_eq!((counts(&s).2, counts(&s).3), (0, 0));
    }
}
struct FailAt {
    armed: AtomicBool,
    point: StoreCutpoint,
}
impl CutpointObserver for FailAt {
    fn hit(&self, p: StoreCutpoint) -> Result<(), StoreError> {
        if p == self.point && self.armed.swap(false, Ordering::AcqRel) {
            return Err(StoreError::CutpointInjected {
                cutpoint: format!("{p:?}"),
                message: "synthetic terminal allocation cut".to_owned(),
            });
        }
        Ok(())
    }
}
#[test]
fn t_f011_001_c1_preparation_rollback_has_no_receipt_lease_or_auxiliary_remainder() {
    let f = Arc::new(FailAt {
        armed: AtomicBool::new(false),
        point: StoreCutpoint::C1BeforeCommandCommit,
    });
    let (s, _cfg, _clock, p, _dir) = fixture(f.clone());
    let req = request(&s, p, 201);
    f.armed.store(true, Ordering::Release);
    assert!(matches!(
        s.commit_command(req.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&s), (0, 0, 0, 0, 0, 0, 0));
    let fences: i64 = s
        .read_snapshot(|c, _| {
            Ok(c.query_row("SELECT COUNT(*) FROM resource_fences", [], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(fences, 0);
    s.commit_command(req).unwrap();
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
}
#[test]
fn t_f011_001_two_attempts_same_purpose_have_one_durable_owner() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    let barrier = Arc::new(Barrier::new(2));
    let mut jobs = vec![];
    for n in [301, 302] {
        let s = s.clone();
        let gate = barrier.clone();
        let request = request(&s, p.clone(), n);
        jobs.push(std::thread::spawn(move || {
            gate.wait();
            s.commit_command(request)
        }));
    }
    let results = jobs
        .into_iter()
        .map(|j| j.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
}
#[test]
fn t_f011_004_auxiliary_saturation_is_separate_from_execution_and_atomic() {
    let (s, _cfg, _clock, mut p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p.clone(), 401)).unwrap();
    p.purpose_key = "second".to_owned();
    s.commit_command(request(&s, p.clone(), 402)).unwrap();
    p.purpose_key = "third".to_owned();
    assert!(matches!(
        s.commit_command(request(&s, p, 403)),
        Err(StoreError::CapacityBlocked(_))
    ));
    assert_eq!(counts(&s), (2, 0, 0, 0, 2, 6, 2));
    assert_eq!(
        s.execution_pool_state(PoolId::from_u128(1))
            .unwrap()
            .current_occupation,
        0
    );
}
#[test]
fn t_f011_005_exact_dispatch_and_resource_fence_resolve_only_once_without_os_rows() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 501)).unwrap();
    let record = read(&s, 501);
    assert!(
        s.resolve_terminal_launch(resolution(&record, DispatchId::from_u128(1)))
            .is_err()
    );
    let id = dispatch(&s, &record);
    assert_eq!(
        s.inspect_broker_action(record.context.action_id)
            .unwrap()
            .unwrap()
            .fence
            .fence,
        Fence::new(0)
    );
    assert_eq!(record.context.resource_fence, Fence::new(1));
    let good = resolution(&record, id);
    let mut wrong = good.clone();
    wrong.lease.fence = Fence::new(2);
    assert!(matches!(
        s.resolve_terminal_launch(wrong),
        Err(StoreError::StaleFence { .. })
    ));
    let mut wrong = good.clone();
    wrong.process_run_uuid = ProcessRunUuid::from_u128(999);
    assert!(s.resolve_terminal_launch(wrong).is_err());
    let mut wrong = good.clone();
    wrong.context_digest = Sha256Digest::from_bytes(b"not context");
    assert!(s.resolve_terminal_launch(wrong).is_err());
    let mut wrong = good.clone();
    wrong.dispatch_id = DispatchId::from_u128(999);
    assert!(s.resolve_terminal_launch(wrong).is_err());
    let resolved = s.resolve_terminal_launch(good.clone()).unwrap();
    assert_eq!(
        resolved.allocation.state,
        TerminalAllocationState::LaunchResolved
    );
    assert_eq!(resolved.allocation.dispatch_id, Some(id));
    assert!(s.resolve_terminal_launch(good).is_err());
    assert_eq!(counts(&s), (1, 1, 0, 0, 1, 3, 1));
}
#[test]
fn t_f011_003_expired_resource_context_cannot_resolve_or_release_its_holds() {
    let (s, _cfg, clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 601)).unwrap();
    let record = read(&s, 601);
    let id = dispatch(&s, &record);
    clock.advance_ms(60_000);
    assert!(matches!(
        s.resolve_terminal_launch(resolution(&record, id)),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(read(&s, 601).state, TerminalAllocationState::Allocated);
    assert_eq!(counts(&s), (1, 1, 0, 0, 1, 3, 1));
}
#[test]
fn t_f011_003_changed_project_or_mission_is_not_a_current_preparation() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 701)).unwrap();
    let record = read(&s, 701);
    s.execute_in_writer_tx(WriteCriticality::Critical, |tx, _| {
        tx.execute(
            "UPDATE projects SET revision=revision+1 WHERE project_id=?1",
            [PROJECT.to_string()],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(s.commit_broker_phase(authorizing(&record)).is_err());
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
}
#[test]
fn t_f011_005_resolution_cutpoints_preserve_rollback_and_never_reissue_a_lost_ack() {
    for point in [
        StoreCutpoint::C1BeforeCommandCommit,
        StoreCutpoint::C2AfterCommandCommit,
    ] {
        let f = Arc::new(FailAt {
            armed: AtomicBool::new(false),
            point,
        });
        let (s, _cfg, _clock, p, _dir) = fixture(f.clone());
        s.commit_command(request(&s, p, 801)).unwrap();
        let record = read(&s, 801);
        let id = dispatch(&s, &record);
        let req = resolution(&record, id);
        f.armed.store(true, Ordering::Release);
        assert!(matches!(
            s.resolve_terminal_launch(req.clone()),
            Err(StoreError::CutpointInjected { .. })
        ));
        if point == StoreCutpoint::C1BeforeCommandCommit {
            assert_eq!(read(&s, 801).state, TerminalAllocationState::Allocated);
            s.resolve_terminal_launch(req).unwrap();
        } else {
            assert_eq!(read(&s, 801).state, TerminalAllocationState::LaunchResolved);
            assert!(s.resolve_terminal_launch(req).is_err());
        }
        assert_eq!(counts(&s), (1, 1, 0, 0, 1, 3, 1));
    }
}
#[test]
fn t_f011_003_restart_and_other_authority_never_reactivate_preparation() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 901)).unwrap();
    let record = read(&s, 901);
    let mut stale = authorizing(&record);
    stale.authority_generation = AuthorityGeneration::from_u128(999);
    assert!(matches!(
        s.commit_broker_phase(stale),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let owned = Arc::try_unwrap(s).unwrap();
    let s = owned.restart_same_lineage().unwrap();
    assert!(matches!(
        s.commit_broker_phase(authorizing(&record)),
        Err(StoreError::StaleFence { .. })
    ));
    assert_eq!(counts(&s), (1, 0, 0, 0, 1, 3, 1));
}

fn legacy_rows(store: &UnleashStore) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    store
        .read_snapshot(|conn, _| {
            ["processes", "terminal_sessions"]
                .into_iter()
                .map(|table| {
                    let mut query = conn.prepare(&format!("SELECT * FROM {table} ORDER BY 1"))?;
                    let n = query.column_count();
                    Ok(query
                        .query_map([], |r| {
                            (0..n).map(|i| r.get(i)).collect::<Result<Vec<_>, _>>()
                        })?
                        .collect::<Result<Vec<_>, _>>()?)
                })
                .collect()
        })
        .unwrap()
}

fn seed_historical_terminal(store: &UnleashStore, hash: Sha256Digest) -> TerminalId {
    let lease = store
        .acquire_lease(AcquireLeaseRequest {
            resource_kind: "terminal".to_owned(),
            resource_key: "synthetic-legacy-resource".to_owned(),
            owner_type: "principal".to_owned(),
            owner_id: "human-alice".to_owned(),
            ttl_ms: 60_000,
        })
        .unwrap();
    let d = store.daemon_state().unwrap();
    let old_terminal = TerminalId::from_u128(10001);
    // Stored historical fixture only; these rows do not attest an OS process.
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO actions(action_id,origin_kind,control_principal_id,action_type,payload_hash,policy_revision,authority_generation,daemon_epoch,fence,state,reconcile_strategy,revision,created_at_ms) VALUES('legacy-terminal-action','control_plane','human-alice','process_exec',?1,0,?2,?3,0,'outcome_unknown','observe_then_decide',1,?4)",rusqlite::params![hash.to_string(),d.authority_generation.to_string(),i64::try_from(d.daemon_epoch.as_u64()).unwrap(),now.as_i64()])?;
        tx.execute("INSERT INTO processes(process_id,action_id,process_run_uuid,authority_generation,identity_backend,owner_uid,cwd_ref,state,last_observed_at_ms,exit_code) VALUES('legacy-process','legacy-terminal-action','legacy-run',?1,'macos_proc_pidinfo',1,'synthetic historical cwd','exited',0,0)",[d.authority_generation.to_string()])?;
        tx.execute("INSERT INTO terminal_sessions(terminal_id,process_id,owner_agent_id,mission_id,purpose_key,rows,cols,lease_id,input_seq,durable_log_seq,state) VALUES(?1,'legacy-process',NULL,?2,'legacy-purpose',24,80,?3,0,0,'exited')",rusqlite::params![old_terminal.to_string(),MISSION.to_string(),lease.lease_id.to_string()])?;Ok(())
    }).unwrap();
    old_terminal
}

#[test]
fn t_f011_001_v010_is_additive_and_interrupted_migration_keeps_v009_history() {
    let (s, cfg, _clock, p, _dir) = fixture_at_schema(Arc::new(NoopCutpointObserver), 9);
    let old_terminal = seed_historical_terminal(&s, p.process_spec.environment.digest);
    let before = legacy_rows(&s);
    let backup = s
        .prepare_backup(PrepareBackupRequest {
            destination_dir: cfg.data_root.join("migration-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    Arc::try_unwrap(s).unwrap().shutdown().unwrap();
    let mut next = cfg.clone();
    next.max_supported_schema_version = 10;
    next.cutpoints = Arc::new(FailAt {
        armed: AtomicBool::new(true),
        point: StoreCutpoint::C11MigrationInProgress,
    });
    assert!(matches!(
        UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let old = UnleashStore::open(cfg.clone()).unwrap();
    assert_eq!(old.daemon_state().unwrap().schema_version, 9);
    assert_eq!(legacy_rows(&old), before);
    let tables: i64 = old
        .read_snapshot(|c, _| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='terminal_allocations'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(tables, 0);
    old.shutdown().unwrap();
    let mut next = cfg;
    next.max_supported_schema_version = 10;
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 10);
    assert_eq!(legacy_rows(&upgraded), before);
    assert!(
        upgraded
            .inspect_terminal_allocation_snapshot(old_terminal)
            .unwrap()
            .value
            .is_none()
    );
    let foreign_keys: i64 = upgraded
        .read_snapshot(|c, _| {
            Ok(
                c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(foreign_keys, 0);
}

#[test]
fn t_f011_003_v012_rolls_back_then_preserves_v011_terminal_history() {
    let (store, old_config, _clock, preparation, _dir) =
        fixture_at_schema(Arc::new(NoopCutpointObserver), 11);
    let terminal = seed_historical_terminal(&store, preparation.process_spec.environment.digest);
    store
        .commit_command(request(&store, preparation, 1201))
        .unwrap();
    let allocation_before = read(&store, 1201);
    let rows_before = legacy_rows(&store);
    let checksums_through_v011 = |store: &UnleashStore| {
        store
            .read_snapshot(|conn, _| {
                let mut query = conn.prepare(
                    "SELECT version, checksum_sha256 FROM schema_migrations
                     WHERE version <= 11 ORDER BY version",
                )?;
                Ok(query
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap()
    };
    let before_checksums = checksums_through_v011(&store);
    assert_eq!(before_checksums.len(), 11);
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: old_config.data_root.join("before-v012"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    Arc::try_unwrap(store).unwrap().shutdown().unwrap();

    let mut next_config = old_config.clone();
    next_config.max_supported_schema_version = 12;
    let mut interrupted = next_config.clone();
    interrupted.cutpoints = Arc::new(FailAt {
        armed: AtomicBool::new(true),
        point: StoreCutpoint::C11MigrationInProgress,
    });
    assert!(matches!(
        UnleashStore::open_with_migration_backup(interrupted, Some(backup.backup_id)),
        Err(StoreError::CutpointInjected { .. })
    ));
    let rolled_back = UnleashStore::open(old_config).unwrap();
    assert_eq!(rolled_back.daemon_state().unwrap().schema_version, 11);
    assert_eq!(checksums_through_v011(&rolled_back), before_checksums);
    assert_eq!(legacy_rows(&rolled_back), rows_before);
    assert_eq!(read(&rolled_back, 1201), allocation_before);
    rolled_back
        .read_snapshot(|conn, _| {
            assert!(
                !conn
                    .prepare("SELECT 1 FROM sqlite_master WHERE name='terminal_control_intents'")?
                    .exists([])?
            );
            assert!(
                !conn
                    .prepare("SELECT 1 FROM schema_migrations WHERE version=12")?
                    .exists([])?
            );
            Ok(())
        })
        .unwrap();
    rolled_back.shutdown().unwrap();

    let upgraded =
        UnleashStore::open_with_migration_backup(next_config, Some(backup.backup_id)).unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 12);
    assert_eq!(checksums_through_v011(&upgraded), before_checksums);
    assert_eq!(read(&upgraded, 1201), allocation_before);
    let mut expected_rows = rows_before;
    expected_rows
        .get_mut(1)
        .unwrap()
        .first_mut()
        .unwrap()
        .push(rusqlite::types::Value::Integer(0));
    assert_eq!(legacy_rows(&upgraded), expected_rows);
    upgraded
        .read_snapshot(|conn, _| {
            let revision: i64 = conn.query_row(
                "SELECT control_revision FROM terminal_sessions WHERE terminal_id=?1",
                [terminal.to_string()],
                |row| row.get(0),
            )?;
            assert_eq!(revision, 0);
            assert!(
                !conn
                    .prepare("SELECT 1 FROM terminal_control_intents")?
                    .exists([])?
            );
            assert!(!conn.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_f011_003_real_restore_keeps_preparation_historical_without_reactivating_it() {
    let (s, cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 1101)).unwrap();
    let record = read(&s, 1101);
    let portable = cfg.data_root.join("portable-terminal");
    s.prepare_backup(PrepareBackupRequest {
        destination_dir: portable.clone(),
        retention_tier: BackupRetentionTier::Manual,
        coverage: "db_and_objects".to_owned(),
    })
    .unwrap();
    Arc::try_unwrap(s).unwrap().shutdown().unwrap();
    let target = local_test_dir("f011-terminal-restore").unwrap();
    let target_cfg = StoreConfig::new(target.path());
    UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir: portable,
            target_data_root: target.path().to_owned(),
            actor_id: "uid:fixture-alice".to_owned(),
            command_id: unleash_domain::CommandId::from_u128(1102),
            command_digest: Sha256Digest::from_bytes(
                b"synthetic restoration of terminal preparation",
            ),
            source_authority_generation: record.context.authority_generation,
            source_db_unreadable: false,
            source_db_path: Some(cfg.db_path),
            source_artifacts_dir: Some(cfg.artifacts_dir),
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        target_cfg.clone(),
    )
    .unwrap();
    let restored = UnleashStore::open(target_cfg).unwrap();
    let current = restored.active_authority_generation().unwrap();
    assert_ne!(current, record.context.authority_generation);
    assert_eq!(read(&restored, 1101), record);
    let mut intent = authorizing(&record);
    intent.authority_generation = current;
    assert!(matches!(
        restored.commit_broker_phase(intent),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let (allocations, actions, processes, terminals, leases, capacity, _) = counts(&restored);
    assert_eq!(
        (allocations, actions, processes, terminals, leases, capacity),
        (1, 0, 0, 0, 1, 3)
    );
    let acquired: i64 = restored
        .read_snapshot(|c, _| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM command_receipts WHERE command_type='AcquireTerminal'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(acquired, 1);
}

#[test]
fn t_f011_001_registered_unknown_resource_between_dispatch_and_resolution_blocks_launch() {
    let (s, _cfg, _clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p, 1201)).unwrap();
    let record = read(&s, 1201);
    let marker = dispatch(&s, &record);
    let c = record.context.clone();
    // Negative injection of a legacy registration with incomplete identity.
    // The backend label is historical fixture data, never an OS observation.
    s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO processes(process_id,action_id,process_run_uuid,authority_generation,identity_backend,owner_uid,cwd_ref,state,last_observed_at_ms) VALUES('foreign-unknown-process',?1,'foreign-unknown-generation',?2,'macos_proc_pidinfo',1,'synthetic legacy cwd','unknown',?3)",rusqlite::params![c.action_id.to_string(),c.authority_generation.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO terminal_sessions(terminal_id,process_id,owner_agent_id,mission_id,purpose_key,rows,cols,lease_id,input_seq,durable_log_seq,state) VALUES('foreign-unknown-terminal','foreign-unknown-process',NULL,?1,?2,24,80,?3,0,0,'suspect')",rusqlite::params![c.preparation.mission_id.to_string(),c.preparation.purpose_key,c.lease_id.to_string()])?;Ok(())
    }).unwrap();
    assert!(
        s.resolve_terminal_launch(resolution(&record, marker))
            .is_err(),
        "a registered resource is not disproved by a separate preparation row"
    );
    assert_eq!(read(&s, 1201).state, TerminalAllocationState::Allocated);
}

#[test]
fn t_f011_001_future_disposition_requires_bound_observation_and_frees_only_active_property() {
    let (s, _cfg, clock, p, _dir) = fixture(Arc::new(NoopCutpointObserver));
    s.commit_command(request(&s, p.clone(), 1301)).unwrap();
    let old = read(&s, 1301);
    s.commit_broker_phase(authorizing(&old)).unwrap();
    s.register_principal(
        "synthetic-disposition-observer",
        "system",
        "system:synthetic-disposition-observer",
    )
    .unwrap();
    let digest = old.context_digest;
    let hash = old.context.preparation.process_spec.environment.digest;
    let action = old.context.action_id;
    s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        for(id,kind,binding)in[("early","observation",format!("terminal-disposition:v1:{digest}")),("claim","claim",format!("terminal-disposition:v1:{digest}")),("other","observation","terminal-disposition:v1:other-context".to_owned())]{tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,?2,?3,?4,?5,'synthetic-disposition-observer',?6,1,'available')",rusqlite::params![id,kind,action.to_string(),binding,hash.to_string(),now.as_i64()])?;}Ok(())
    }).unwrap();
    clock.advance_ms(1);
    let marker = dispatch(&s, &old);
    // A pre-dispatch observation must not retire a later marker.
    for evidence in ["early", "claim", "other"] {
        assert!(s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{tx.execute("UPDATE terminal_allocations SET state='retired',revision=revision+1,retirement_evidence_id=?2,retired_at_ms=?3 WHERE terminal_id=?1",rusqlite::params![TerminalId::from_u128(1301).to_string(),evidence,now.as_i64()])?;Ok(())}).is_err());
    }
    s.resolve_terminal_launch(resolution(&old, marker)).unwrap();
    // Old evidence cannot be retroactively attached after the marker.
    assert!(s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,_|{tx.execute("INSERT INTO terminal_disposition_observations(evidence_id,terminal_id,context_digest,observation_object_hash,dispatch_id) VALUES('early',?1,?2,?3,?4)",rusqlite::params![TerminalId::from_u128(1301).to_string(),digest.to_string(),hash.to_string(),marker.to_string()])?;Ok(())}).is_err());
    assert!(s.commit_command(request(&s, p.clone(), 1302)).is_err());
    let proof_hash = publish_disposition_fixture(&s, &old, marker, clock.utc_ms());
    // A fresh evidence ID cannot wrap an observation dated before its marker.
    let terminal = old.context.terminal_id;
    assert!(s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,_|{
      tx.execute("INSERT INTO terminal_disposition_observations(evidence_id,terminal_id,context_digest,observation_object_hash,dispatch_id) VALUES('repacked',?1,?2,?3,?4)",rusqlite::params![terminal.to_string(),digest.to_string(),proof_hash.to_string(),marker.to_string()])?;
      tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES('repacked','observation',?1,?2,?3,'synthetic-disposition-observer',0,1,'available')",rusqlite::params![action.to_string(),format!("terminal-disposition:v1:{digest}"),proof_hash.to_string()])?;Ok(())
    }).is_err());
    let context = old.context.clone();
    // Models ONLY the future verified producer's atomic disposition transaction.
    // Metadata precedes its NEW evidence after the marker; no real OS proof here.
    s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO terminal_disposition_observations(evidence_id,terminal_id,context_digest,observation_object_hash,dispatch_id) VALUES('bound',?1,?2,?3,?4)",rusqlite::params![context.terminal_id.to_string(),digest.to_string(),proof_hash.to_string(),marker.to_string()])?;
        tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES('bound','observation',?1,?2,?3,'synthetic-disposition-observer',?4,1,'available')",rusqlite::params![context.action_id.to_string(),format!("terminal-disposition:v1:{digest}"),proof_hash.to_string(),now.as_i64()])?;
        tx.execute("UPDATE terminal_allocations SET state='retired',revision=revision+1,retirement_evidence_id='bound',retired_at_ms=?2 WHERE terminal_id=?1",rusqlite::params![context.terminal_id.to_string(),now.as_i64()])?;
        tx.execute("UPDATE resource_leases SET state='released' WHERE lease_id=?1",[context.lease_id.to_string()])?;
        for id in[context.host_reservation_id,context.child_reservation_id,context.pty_reservation_id]{tx.execute("UPDATE capacity_reservations SET state='released',released_at_ms=?2 WHERE reservation_id=?1",rusqlite::params![id.to_string(),now.as_i64()])?;}Ok(())
    }).unwrap();
    s.commit_command(request(&s, p, 1303)).unwrap();
    let next = read(&s, 1303);
    assert_eq!(next.context.resource_fence, Fence::new(2));
    assert_ne!(next.context.process_run_uuid, old.context.process_run_uuid);
    assert!(s.resolve_terminal_launch(resolution(&old, marker)).is_err());
    // A later unknown registration reusing the old terminal ID is not covered
    // by retirement of another process/run. No neutralization is fabricated.
    let c = old.context.clone();
    s.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now|{
        tx.execute("INSERT INTO processes(process_id,action_id,process_run_uuid,authority_generation,identity_backend,owner_uid,cwd_ref,state,last_observed_at_ms) VALUES('rebound-unknown-process',?1,'rebound-unknown-run',?2,'macos_proc_pidinfo',1,'synthetic legacy cwd','unknown',?3)",rusqlite::params![c.action_id.to_string(),c.authority_generation.to_string(),now.as_i64()])?;
        tx.execute("INSERT INTO terminal_sessions(terminal_id,process_id,owner_agent_id,mission_id,purpose_key,rows,cols,lease_id,input_seq,durable_log_seq,state) VALUES(?1,'rebound-unknown-process',NULL,?2,?3,24,80,?4,0,0,'suspect')",rusqlite::params![c.terminal_id.to_string(),c.preparation.mission_id.to_string(),c.preparation.purpose_key,c.lease_id.to_string()])?;Ok(())
    }).unwrap();
    assert!(s.commit_broker_phase(authorizing(&next)).is_err());
}

// Writer fixtures only: synthetic OS facts exercise atomic correlation, not OS qualification.
fn launch_observation_fixture(
    s: &UnleashStore,
    r: &TerminalAllocationRecord,
    dispatch_id: DispatchId,
    captured: i64,
    received: i64,
) -> (
    terminal_allocations::TerminalLaunchObservation,
    BrokerPhaseRequest,
) {
    launch_observation_state_fixture(s, r, dispatch_id, captured, received, true)
}

fn launch_observation_state_fixture(
    s: &UnleashStore,
    r: &TerminalAllocationRecord,
    dispatch_id: DispatchId,
    captured: i64,
    received: i64,
    exited: bool,
) -> (
    terminal_allocations::TerminalLaunchObservation,
    BrokerPhaseRequest,
) {
    launch_observation_with_input_state_fixture(
        s,
        r,
        dispatch_id,
        captured,
        received,
        exited,
        Some(
            terminal_allocations::terminal_acquisition_query::TerminalObservedInputState {
                last_confirmed_input_seq: 0,
                unknown_input_seqs: vec![],
            },
        ),
    )
}

fn launch_observation_with_input_state_fixture(
    s: &UnleashStore,
    r: &TerminalAllocationRecord,
    dispatch_id: DispatchId,
    captured: i64,
    received: i64,
    exited: bool,
    input_state: Option<
        terminal_allocations::terminal_acquisition_query::TerminalObservedInputState,
    >,
) -> (
    terminal_allocations::TerminalLaunchObservation,
    BrokerPhaseRequest,
) {
    let c = &r.context;
    let d = s.daemon_state().unwrap();
    let observation = terminal_allocations::TerminalLaunchObservation {
        terminal_id: c.terminal_id,
        action_id: c.action_id,
        dispatch_id,
        context_digest: r.context_digest,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        pid: 123,
        pgid: 123,
        owner_uid: d.owner_uid,
        boot_identity: d.boot_identity,
        start_identity: "synthetic-sqlite-start".into(),
        identity_backend: "macos_proc_pidinfo".into(),
        manifest_digest: Sha256Digest::from_bytes(b"synthetic manifest"),
        effective_environment_digest: c.preparation.process_spec.environment.digest,
        durable_log_seq: 0,
        input_state,
        observed_at_ms: unleash_domain::TimestampMs::new(captured),
        exit_code: exited.then_some(0),
        exit_signal: None,
        exited,
    };
    let hash = canonical_sha256(&observation).unwrap();
    let bytes = canonicalize_to_vec(&observation).unwrap();
    let pin = s
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".into(),
            owner_kind: "terminal_launch_observation".into(),
            owner_id: c.terminal_id.to_string(),
        })
        .unwrap();
    s.write_prepared_object_bytes(&pin, &bytes).unwrap();
    s.publish_object_reference(PublishObjectReferenceRequest {
        pin_id: pin.pin_id,
        object_hash: hash,
        object_generation: pin.object_generation,
        owner_kind: "terminal_launch_observation".into(),
        owner_id: c.terminal_id.to_string(),
        source_field: dispatch_id.to_string(),
    })
    .unwrap();
    let original = authorizing(r);
    let intent_digest = match original.phase {
        BrokerPhase::Authorizing { intent } => intent.intent_digest,
        _ => unreachable!(),
    };
    let snapshot = s.inspect_broker_action(c.action_id).unwrap().unwrap();
    let phase = BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: snapshot.revision,
        evaluation: original.evaluation,
        phase: BrokerPhase::Observation {
            action_id: c.action_id,
            intent_digest,
            receipt: BrokerActionReceipt {
                action_id: c.action_id,
                evaluation_id: original.evaluation.evaluation_id,
                dispatch_id,
                decision_ref: "synthetic-sqlite-allow".into(),
                state: unleash_domain::ActionState::ObservedSuccess,
                output_digest: Some(hash),
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: false,
                observed_at_ms: unleash_domain::TimestampMs::new(received),
            },
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
        },
    };
    (observation, phase)
}

#[path = "support/terminal_control_scenarios.rs"]
mod control_scenarios;

#[test]
fn t_f011_003_capture_precedes_receipt_and_observation_commit_replays_without_new_rows() {
    for point in [
        StoreCutpoint::C1BeforeCommandCommit,
        StoreCutpoint::C2AfterCommandCommit,
    ] {
        let fault = Arc::new(FailAt {
            armed: AtomicBool::new(false),
            point,
        });
        let (s, _cfg, clock, prep, _dir) = fixture(fault.clone());
        s.commit_command(request(&s, prep, 1401)).unwrap();
        let before = read(&s, 1401);
        clock.set_utc_ms(100);
        let id = dispatch(&s, &before);
        s.resolve_terminal_launch(resolution(&before, id)).unwrap();
        let record = read(&s, 1401);
        clock.set_utc_ms(105);
        let (observation, phase) = launch_observation_fixture(&s, &record, id, 101, 103);
        fault.armed.store(true, Ordering::Release);
        let cut_result = s.commit_terminal_launch_observation(observation.clone(), phase.clone());
        assert!(
            matches!(&cut_result, Err(StoreError::CutpointInjected { .. })),
            "point={point:?}, result={cut_result:?}"
        );
        let committed_before_retry = point == StoreCutpoint::C2AfterCommandCommit;
        assert_eq!(
            (counts(&s).2, counts(&s).3),
            if committed_before_retry {
                (1, 1)
            } else {
                (0, 0)
            }
        );
        let committed = s
            .commit_terminal_launch_observation(observation.clone(), phase.clone())
            .unwrap();
        assert_eq!(committed.replayed, committed_before_retry);
        let replay = s
            .commit_terminal_launch_observation(observation, phase.clone())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.event_cursor, committed.event_cursor);
        assert_eq!((counts(&s).2, counts(&s).3), (1, 1));
        let times=s.read_snapshot(|conn,_| {
            Ok(conn.query_row("SELECT p.last_observed_at_ms,d.ended_at_ms FROM processes p JOIN action_dispatches d ON d.action_id=p.action_id WHERE p.process_id=?1 AND d.dispatch_id=?2",rusqlite::params![record.context.process_id.to_string(),id.to_string()],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?)))?)
        }).unwrap();
        assert_eq!(
            times,
            (101, 103),
            "capture and later broker receipt are distinct durable facts"
        );
        let mut forged = phase;
        if let BrokerPhase::Observation { receipt, .. } = &mut forged.phase {
            receipt.enforcement_refs.push("unproven-control".into());
        }
        assert!(s.commit_broker_phase(forged).is_err());
        assert!(
            s.resolve_terminal_launch(resolution(&before, id)).is_err(),
            "observation replay never reissues launch permit"
        );
    }
}

#[test]
fn t_f011_003_capture_before_dispatch_or_after_receipt_cannot_publish_process_rows() {
    for captured in [99, 104] {
        let (s, _cfg, clock, prep, _dir) = fixture(Arc::new(NoopCutpointObserver));
        s.commit_command(request(&s, prep, 1501)).unwrap();
        let before = read(&s, 1501);
        clock.set_utc_ms(100);
        let id = dispatch(&s, &before);
        s.resolve_terminal_launch(resolution(&before, id)).unwrap();
        let record = read(&s, 1501);
        clock.set_utc_ms(105);
        let (observation, phase) = launch_observation_fixture(&s, &record, id, captured, 103);
        let prior = s
            .inspect_broker_action(record.context.action_id)
            .unwrap()
            .unwrap();
        let refused = s.commit_terminal_launch_observation(observation, phase);
        assert!(
            matches!(refused,Err(StoreError::InvalidArgument {message,..}) if message=="terminal capture timestamp is outside dispatch and receipt")
        );
        assert_eq!((counts(&s).2, counts(&s).3), (0, 0));
        assert_eq!(
            s.inspect_broker_action(record.context.action_id)
                .unwrap()
                .unwrap(),
            prior,
            "failed evidence cannot append a phase or advance action"
        );
        let (phases,events)=s.read_snapshot(|conn,_| {
            let action=record.context.action_id.to_string();
            Ok((conn.query_row("SELECT COUNT(*) FROM broker_phase_records WHERE action_id=?1 AND phase_key LIKE 'observation:%'",[&action],|row|row.get::<_,i64>(0))?,conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionObserved'",[&action],|row|row.get::<_,i64>(0))?))
        }).unwrap();
        assert_eq!((phases, events), (0, 0));
    }
}

fn publish_disposition_fixture(
    store: &UnleashStore,
    record: &TerminalAllocationRecord,
    marker: DispatchId,
    observed_at: i64,
) -> Sha256Digest {
    let c = &record.context;
    let bytes=canonicalize_to_vec(&json!({"schema_version":1,"fixture":"sqlite_contract_only_not_os_observation","terminal_id":c.terminal_id,"process_run_uuid":c.process_run_uuid,"action_id":c.action_id,"dispatch_id":marker,"authority_generation":c.authority_generation,"daemon_epoch":c.daemon_epoch,"resource_fence":c.resource_fence,"context_digest":record.context_digest,"observed_at_ms":observed_at,"observation_kind":"generation_neutralized"})).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "fixture".to_owned(),
            owner_id: "f011-synthetic-disposition".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "fixture".to_owned(),
            owner_id: "f011-synthetic-disposition".to_owned(),
            source_field: "typed_observation".to_owned(),
        })
        .unwrap();
    hash
}
