//! SQLite/broker fixture only. The executor spy never launches an OS process.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use unleash_action_broker::*;
use unleash_domain::{
    ActionId, ActorId, CommandId, DataLocality, Fence, LineageFence, MissionId, ModelPolicy,
    ProjectId, ReconcileStrategy, RequestId, Revision, TerminalId,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::terminal_allocations::TerminalRuntimeSettings;
use unleash_store::*;
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, SimulatedExecutionFailure, TestIsolationProbe, local_test_dir,
};

struct Fixture {
    store: Option<Arc<UnleashStore>>,
    clock: Arc<ManualClock>,
    record: TerminalAllocationRecord,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(store) = self.store.take() {
            // Store shutdown is synchronous. Keep its last Arc off Tokio even
            // when an assertion unwinds, before removing this owned directory.
            let _ = std::thread::spawn(move || drop(store)).join();
        }
    }
}

fn request(
    store: &UnleashStore,
    actor: &ActorId,
    id: u128,
    target: (AggregateKind, uuid::Uuid, Option<ProjectId>),
    name: &str,
    payload: serde_json::Value,
    mutation: CommandMutation,
) -> CommitCommandRequest {
    let (kind, aggregate, project) = target;
    let serde_json::Value::Object(payload) = payload else {
        panic!("fixture command payload must be an object");
    };
    CommitCommandRequest {
        actor_id: actor.clone(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(id),
            command_id: CommandId::from_u128(id),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: None,
            project_id: project,
            aggregate_id: aggregate,
            expected_revision: Revision::INITIAL,
            command_type: name.into(),
            payload,
        },
        target_aggregate: kind,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

fn configure_runtime(store: &UnleashStore, actor: &ActorId) -> TerminalRuntimeSettings {
    let configured:TerminalRuntimeSettings=serde_json::from_value(serde_json::json!({"schema_version":1,"profiles":[{"profile_id":"journal-terminal","version":1,"variables":{"LANG":"C"}}],"max_hosts":2,"max_processes":2,"max_ptys":2,"lease_ttl_ms":60000})).unwrap();
    let prepared = configured
        .profiles
        .iter()
        .map(|p| {
            store
                .prepare_terminal_environment(p, CommandId::from_u128(1))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let patch = SettingsPatch {
        extra_typed: Some(
            serde_json::json!({"terminal_runtime":configured,"__terminal_prepared":prepared})
                .as_object()
                .unwrap()
                .clone(),
        ),
        ..SettingsPatch::default()
    };
    store.commit_command(request(store,actor,1,(AggregateKind::Settings,store.active_authority_generation().unwrap().as_uuid(),None),"SetSettings",serde_json::json!({"scope_kind":"user","scope_id":"default","patch":{"terminal_runtime":configured}}),CommandMutation::SetSettings {scope_kind:SettingsScopeKind::User,scope_id:"default".into(),caller:CallerAuthority {actor_id:actor.clone(),role:CallerRole::PolicyAdmin},patch})).unwrap();
    configured
}

fn fixture() -> Fixture {
    let directory = local_test_dir("terminal-unknown").unwrap();
    let root = directory.path().canonicalize().unwrap();
    let clock = Arc::new(ManualClock::new(1000));
    let mut cfg = StoreConfig::new(directory.path());
    cfg.clock = clock.clone();
    cfg.id_generator = Arc::new(DeterministicIdGenerator::new(0xf011_aa00));
    let store = Arc::new(UnleashStore::open(cfg).unwrap());
    let actor = ActorId::new("uid:terminal-journal-fixture").unwrap();
    store
        .register_principal("human-terminal-journal", "user", actor.as_str())
        .unwrap();
    store
        .configure_execution_pool(ExecutionPoolParams::default())
        .unwrap();
    let configured = configure_runtime(&store, &actor);
    let project = ProjectId::from_u128(11);
    let mission = MissionId::from_u128(12);
    let root_string = root.to_str().unwrap().to_owned();
    let saved_root = root_string.clone();
    store.execute_in_writer_tx(WriteCriticality::Critical,move|tx,now| {
        tx.execute("INSERT INTO projects(project_id,name,state,revision,created_at_ms,primary_root_path) VALUES(?1,'SQLite terminal fixture','active',2,?2,?3)",rusqlite::params![project.to_string(),now.as_i64(),saved_root])?;
        tx.execute("INSERT INTO missions(mission_id,project_id,owner_kind,owner_user_id,completion_policy,state,revision) VALUES(?1,?2,'user','human-terminal-journal','human','active',1)",rusqlite::params![mission.to_string(),project.to_string()])?;
        Ok(())
    }).unwrap();
    let profile = configured.profiles.first().unwrap();
    let preparation = TerminalAllocationPreparation {
        mission_id: mission,
        mission_revision: Revision::new(1),
        scope: TerminalAllocationScope::ProjectConfiguration {
            project_id: project,
            revision: Revision::new(2),
        },
        purpose_key: "unknown-fixture".into(),
        process_spec: NormalizedTerminalProcessSpec {
            program: "/usr/bin/true".into(),
            arguments: vec![],
            requested_cwd: ".".into(),
            cwd_canonical: root_string,
            environment: TerminalEnvironmentRef {
                profile_id: profile.profile_id.clone(),
                version: profile.version,
                digest: profile.digest().unwrap(),
            },
            rows: 24,
            cols: 80,
            readiness_timeout_ms: 1000,
        },
        auxiliary: configured.pools(),
        lease_ttl_ms: configured.lease_ttl_ms,
    };
    let terminal = TerminalId::from_u128(13);
    store
        .commit_command(request(
            &store,
            &actor,
            2,
            (
                AggregateKind::TerminalAllocation,
                terminal.as_uuid(),
                Some(project),
            ),
            "AcquireTerminal",
            preparation.command_payload(),
            CommandMutation::PrepareTerminalAllocation {
                terminal_id: terminal,
                caller: CallerAuthority {
                    actor_id: actor.clone(),
                    role: CallerRole::HumanOperator,
                },
                preparation,
            },
        ))
        .unwrap();
    let record = store
        .inspect_terminal_allocation_snapshot(terminal)
        .unwrap()
        .value
        .unwrap();
    Fixture {
        store: Some(store),
        clock,
        record,
        _directory: directory,
    }
}

struct RecordingJournal {
    inner: super::SqliteBrokerJournal,
    entries: Mutex<Vec<BrokerJournalEntry>>,
}
impl RecordingJournal {
    fn capture_effect(&self) -> (BrokerJournalEntry, BrokerJournalEntry) {
        let entries = self.entries.lock().unwrap();
        (
            entries
                .iter()
                .find(|e| matches!(e, BrokerJournalEntry::Observation { .. }))
                .unwrap()
                .clone(),
            entries
                .iter()
                .find(|e| matches!(e, BrokerJournalEntry::Dispatch { .. }))
                .unwrap()
                .clone(),
        )
    }
}

impl BrokerJournalPort for RecordingJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        self.entries.lock().unwrap().push(entry.clone());
        self.inner.append(entry)
    }
}

fn fixture_authority(
    c: &TerminalAllocationContext,
    project_id: ProjectId,
    revision: Revision,
) -> EffectiveAuthority {
    compute_effective_authority(
        Some(project_id),
        c.policy_revision,
        revision,
        &[AuthorityLayer {
            layer_name: "terminal SQLite fixture".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::ProcessExec])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from([TERMINAL_ALLOCATION_VERB.into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![
                c.preparation.process_spec.cwd_canonical.clone().into(),
            ]),
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Project,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    )
}

fn fixture_intent(
    record: &TerminalAllocationRecord,
    authority: &EffectiveAuthority,
    fence: LineageFence,
) -> ActionIntent {
    let c = &record.context;
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    ActionIntent {
        action_id: c.action_id,
        subject: ActionSubject::new_control_plane(
            c.principal_id.clone(),
            format!("receipt:{}", c.receipt_id),
        )
        .unwrap(),
        project_id: Some(project_id),
        verb: TERMINAL_ALLOCATION_VERB.into(),
        resource_refs: vec![NormalizedResourceRef::Terminal(c.terminal_id)],
        payload_digest: record.payload_digest(),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::ProcessExec,
        scope_revision: revision,
        policy_revision: c.policy_revision,
        lineage_fence: fence,
        action_deadline: c.lease_expires_at,
        isolation_requirements: IsolationRequirements::from_authority(authority),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    }
}

#[tokio::test]
async fn terminal_unknown_sqlite_preserves_receipt_and_one_dispatch_on_replay() {
    let fixture = tokio::task::spawn_blocking(fixture).await.unwrap();
    let store = fixture.store.as_ref().unwrap().clone();
    let clock = fixture.clock.clone();
    let c = &fixture.record.context;
    let TerminalAllocationScope::ProjectConfiguration {
        project_id,
        revision,
    } = c.preparation.scope;
    let authority = fixture_authority(c, project_id, revision);
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::ZERO);
    let mut inner = super::SqliteBrokerJournal::for_store_test(store.clone());
    inner.terminal_observation = Some(Arc::new(Mutex::new(None)));
    let journal = Arc::new(RecordingJournal {
        inner,
        entries: Mutex::new(vec![]),
    });
    let executor = Arc::new(BrokerExecutionSpy::new());
    executor.set_next_execution_failure(SimulatedExecutionFailure::OutcomeUnknown {
        reason: "EXECUTOR-CANARY-DO-NOT-PERSIST".into(),
    });
    let broker = ActionBroker::new(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        store.config().id_generator.clone(),
        Arc::new(TestIsolationProbe::standard_available()),
        executor.clone(),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(fence),
        journal.clone(),
    );
    broker.bump_scope_revision(Some(project_id), revision);
    let intent = fixture_intent(&fixture.record, &authority, fence);
    let AuthorizationOutcome::Authorized(decision) =
        broker.authorize_action(intent, &authority).await.unwrap()
    else {
        panic!("fixture authorization required");
    };
    let secrets = SecretResolutionSpy::in_memory();
    let outcome = broker
        .execute_authorized(
            c.action_id,
            &decision.decision_ref,
            fence,
            &authority,
            executor.as_ref(),
            &secrets,
        )
        .await;
    assert!(matches!(
        outcome,
        Err(ActionBrokerError::OutcomeUnknown { .. })
    ));
    assert_eq!(executor.dispatch_count(), 1);
    let (observation, dispatch) = journal.capture_effect();
    let BrokerJournalEntry::Observation {
        receipt: result, ..
    } = &observation
    else {
        panic!("captured observation required");
    };
    assert_eq!(result.state, unleash_domain::ActionState::OutcomeUnknown);
    assert_eq!(
        result.dispatch_id,
        executor.dispatches().first().unwrap().dispatch_id
    );
    journal.append(&observation).await.unwrap();
    assert!(journal.append(&dispatch).await.is_err());
    assert!(
        broker
            .execute_authorized(
                c.action_id,
                &decision.decision_ref,
                fence,
                &authority,
                executor.as_ref(),
                &secrets
            )
            .await
            .is_err()
    );
    assert_eq!(executor.dispatch_count(), 1);
    assert_durable_outcome_unknown(store, c.action_id, result).await;
}

async fn assert_durable_outcome_unknown(
    store: Arc<UnleashStore>,
    action: ActionId,
    result: &ActionReceipt,
) {
    let inspected=tokio::task::spawn_blocking(move||store.read_snapshot(|conn,_| {
        let key=action.to_string();
        let state=conn.query_row("SELECT state FROM actions WHERE action_id=?1",[&key],|r|r.get::<_,String>(0))?;
        let dispatches=conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1",[&key],|r|r.get::<_,i64>(0))?;
        let observed=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionObserved'",[&key],|r|r.get::<_,i64>(0))?;
        let markers=conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionDispatchCommitted'",[&key],|r|r.get::<_,i64>(0))?;
        let phase=conn.query_row("SELECT phase_json FROM broker_phase_records WHERE action_id=?1 AND phase_key='observation:1'",[&key],|r|r.get::<_,String>(0))?;
        let processes=conn.query_row("SELECT COUNT(*) FROM processes",[],|r|r.get::<_,i64>(0))?;
        let sessions=conn.query_row("SELECT COUNT(*) FROM terminal_sessions",[],|r|r.get::<_,i64>(0))?;
        Ok((state,dispatches,observed,markers,phase,processes,sessions))
    })).await.unwrap().unwrap();
    assert_eq!(
        (
            &inspected.0,
            inspected.1,
            inspected.2,
            inspected.3,
            inspected.5,
            inspected.6
        ),
        (&"outcome_unknown".to_owned(), 1, 1, 1, 0, 0)
    );
    assert!(!inspected.4.contains("EXECUTOR-CANARY"));
    let stored: serde_json::Value = serde_json::from_str(&inspected.4).unwrap();
    let phase = stored.get("phase").unwrap();
    assert_eq!(
        phase.get("receipt").unwrap(),
        &serde_json::to_value(super::receipt(result)).unwrap()
    );
    assert!(phase.get("reason").is_none_or(serde_json::Value::is_null));
}
