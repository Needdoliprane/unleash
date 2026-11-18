//! REQ-SEC-002 / REQ-PERSIST-001 : le journal `SQLite` précède tout effet et interdit un second dispatch.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]
use super::SqliteBrokerJournal;
use rusqlite::OptionalExtension;
use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionExecutionFailure, ActionExecutionResult,
    ActionExecutorPort, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationDecision, AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry,
    BrokerJournalError, BrokerJournalPort, EffectsClass, IsolationMode, IsolationRequirements,
    LocalAuthorityFallbackPolicy, NormalizedResourceRef, ResolvedSecret, ScopeKind,
    compute_effective_authority,
};
use unleash_domain::{
    ActorId, DataLocality, DispatchId, Fence, IdGeneratorPort, LineageFence, ModelPolicy,
    ReconcileStrategy, RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    SettingsPatch, SettingsScopeKind, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe, local_test_dir,
};

#[derive(Default)]
struct PauseAfterIntent {
    armed: AtomicBool,
    fail_after_commit: AtomicBool,
    entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    condvar: std::sync::Condvar,
}

impl unleash_store::CutpointObserver for PauseAfterIntent {
    fn hit(&self, point: unleash_store::StoreCutpoint) -> Result<(), StoreError> {
        if point == unleash_store::StoreCutpoint::C3AfterBrokerIntentCommit
            && self.fail_after_commit.swap(false, Ordering::SeqCst)
        {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C3".into(),
                message: "test".into(),
            });
        }
        if point == unleash_store::StoreCutpoint::C3AfterBrokerIntentCommit
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.notify_one();
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.condvar.wait(released).unwrap();
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn daemon_publishes_the_committed_broker_cursor_to_live_ipc() {
    use crate::{DaemonConfig, DaemonStartOutcome, UnleashDaemon};
    use std::os::unix::fs::PermissionsExt;
    use unleash_client::{ClientConfig, UnleashClient};
    use unleash_domain::{CommandId, Sequence};
    use unleash_protocol::{ClientKind, SubscribeEventsFrame, SubscribeEventsRequest};

    let rdv = local_test_dir("m1-broker-events-rdv").unwrap();
    std::fs::set_permissions(rdv.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = local_test_dir("m1-broker-events-data").unwrap();
    let pause = Arc::new(PauseAfterIntent::default());
    let cfg = DaemonConfig {
        data_root: Some(data.path().to_path_buf()),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        store_overrides: crate::StoreOverrides {
            cutpoints: Some(pause.clone()),
            ..Default::default()
        },
        ..DaemonConfig::default()
    };
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(cfg).await.unwrap() else {
        panic!("daemon attendu");
    };
    let runtime = handle.test_runtime_state();
    let store = Arc::clone(&runtime.store);
    let generation = runtime.authority_generation;
    let cause = tokio::task::spawn_blocking(move || {
        store.register_principal("p-sys", "system", "p-sys").unwrap();
        let actor = ActorId::new("p-sys").unwrap();
        let payload = serde_json::json!({"scope_kind":"user", "scope_id":"default", "patch":{"confirm_retention_shortening":false}});
        store.commit_command(CommitCommandRequest {
            actor_id: actor.clone(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion, request_id: RequestId::from_u128(71),
                command_id: CommandId::from_u128(72), authority_generation: generation,
                actor_ref: None, project_id: None, aggregate_id: generation.as_uuid(),
                expected_revision: Revision::INITIAL, command_type: "SetSettings".into(),
                payload: payload.as_object().unwrap().clone(),
            },
            target_aggregate: AggregateKind::Settings, operation_id: None, receipt_state: ReceiptState::Accepted,
            result_ref: None, retention_days_override: None, fence_assertion: None,
            mutation: CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::User, scope_id: "default".into(),
                caller: CallerAuthority { actor_id: actor, role: CallerRole::PolicyAdmin },
                patch: SettingsPatch::default(),
            },
            events: Vec::new(), capacity_reservations: Vec::new(), budget_reservations: Vec::new(),
        }).unwrap().receipt().clone()
    }).await.unwrap();
    let client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "broker-events-test",
    ))
    .await
    .unwrap();
    let mut stream = client
        .subscribe_events(&SubscribeEventsRequest {
            stream_id: cause.event_cursor.stream_id,
            after_sequence: Sequence::new(cause.event_cursor.sequence.as_u64() - 1),
            project_id: None,
            aggregate_id: None,
            batch_limit: 16,
        })
        .await
        .unwrap();
    let initial = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(initial, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e| e.sequence == cause.event_cursor.sequence))
    );
    let (evaluation, mut intent) = super::tests::fixture();
    intent.subject = ActionSubject::new_control_plane(
        ActorId::new("p-sys").unwrap(),
        format!("receipt:{}", cause.receipt_id),
    )
    .unwrap();
    intent.lineage_fence = LineageFence::new(generation, runtime.daemon_epoch, Fence::ZERO);
    intent.action_deadline = TimestampMs::new(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
            + 60_000,
    );
    let action_id = intent.action_id;
    let journal = SqliteBrokerJournal::new(Arc::clone(&runtime));
    journal
        .append(&BrokerJournalEntry::Authorizing { evaluation, intent })
        .await
        .unwrap();
    let expected_sequence = {
        let store = Arc::clone(&runtime.store);
        tokio::task::spawn_blocking(move || store.read_snapshot(|conn, _| {
            conn.query_row("SELECT sequence FROM events WHERE aggregate_id=?1 AND event_type='ActionAuthorizationRequested'",
                [action_id.to_string()], |r| r.get::<_, i64>(0))
                .map_err(StoreError::from_rusqlite)
        })).await.unwrap().unwrap()
    };
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(frame, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e|
        e.event_type == "ActionAuthorizationRequested" && e.sequence.as_u64() == u64::try_from(expected_sequence).unwrap()))
    );
    // Annulation après commit C3 et avant accusé : le worker détaché doit
    // encore publier le curseur réellement engagé à l'abonné actif.
    let (mut evaluation, mut cancelled_intent) = super::tests::fixture();
    evaluation.evaluation_id = unleash_domain::AuthorizationEvaluationId::from_u128(501);
    cancelled_intent.action_id = unleash_domain::ActionId::from_u128(500);
    cancelled_intent.subject = ActionSubject::new_control_plane(
        ActorId::new("p-sys").unwrap(),
        format!("receipt:{}", cause.receipt_id),
    )
    .unwrap();
    cancelled_intent.lineage_fence =
        LineageFence::new(generation, runtime.daemon_epoch, Fence::ZERO);
    cancelled_intent.action_deadline = TimestampMs::new(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
            + 60_000,
    );
    let cancelled_id = cancelled_intent.action_id;
    pause.armed.store(true, Ordering::SeqCst);
    let journal = Arc::new(journal);
    let publisher = Arc::clone(&journal);
    let caller = tokio::spawn(async move {
        publisher
            .append(&BrokerJournalEntry::Authorizing {
                evaluation,
                intent: cancelled_intent,
            })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), pause.entered.notified())
        .await
        .unwrap();
    caller.abort();
    *pause.released.lock().unwrap() = true;
    pause.condvar.notify_one();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let cancelled_sequence = {
        let store = Arc::clone(&runtime.store);
        tokio::task::spawn_blocking(move || store.read_snapshot(|conn, _| {
            conn.query_row("SELECT sequence FROM events WHERE aggregate_id=?1 AND event_type='ActionAuthorizationRequested'",
                [cancelled_id.to_string()], |r| r.get::<_, i64>(0))
                .map_err(StoreError::from_rusqlite)
        })).await.unwrap().unwrap()
    };
    assert!(
        matches!(frame, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e|
        e.event_type == "ActionAuthorizationRequested" && e.sequence.as_u64() == u64::try_from(cancelled_sequence).unwrap()))
    );
    // C3 renvoie une erreur APRÈS commit : aucune confirmation de succès,
    // mais l'abonné doit recevoir le curseur relu dans la base.
    let (mut evaluation, mut uncertain) = super::tests::fixture();
    evaluation.evaluation_id = unleash_domain::AuthorizationEvaluationId::from_u128(601);
    uncertain.action_id = unleash_domain::ActionId::from_u128(600);
    uncertain.subject = ActionSubject::new_control_plane(
        ActorId::new("p-sys").unwrap(),
        format!("receipt:{}", cause.receipt_id),
    )
    .unwrap();
    uncertain.lineage_fence = LineageFence::new(generation, runtime.daemon_epoch, Fence::ZERO);
    uncertain.action_deadline = TimestampMs::new(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
            + 60_000,
    );
    let uncertain_id = uncertain.action_id;
    pause.fail_after_commit.store(true, Ordering::SeqCst);
    assert!(
        journal
            .append(&BrokerJournalEntry::Authorizing {
                evaluation,
                intent: uncertain
            })
            .await
            .is_err()
    );
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let uncertain_sequence = {
        let store = Arc::clone(&runtime.store);
        tokio::task::spawn_blocking(move || store.read_snapshot(|conn, _| {
            conn.query_row("SELECT sequence FROM events WHERE aggregate_id=?1 AND event_type='ActionAuthorizationRequested'",
                [uncertain_id.to_string()], |r| r.get::<_, i64>(0)).map_err(StoreError::from_rusqlite)
        })).await.unwrap().unwrap()
    };
    assert!(
        matches!(frame, SubscribeEventsFrame::Events { events, .. } if events.iter().any(|e|
        e.event_type == "ActionAuthorizationRequested" && e.sequence.as_u64() == u64::try_from(uncertain_sequence).unwrap()))
    );
    drop(journal);
    drop(stream);
    drop(client);
    drop(runtime);
    handle.shutdown_graceful("test").await.unwrap();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Loss {
    None,
    BeforeDispatch,
    AfterDispatch,
    AfterObservation,
}
struct FaultJournal {
    inner: Arc<SqliteBrokerJournal>,
    loss: Loss,
    fired: AtomicBool,
    dispatch: Mutex<Option<BrokerJournalEntry>>,
}
impl BrokerJournalPort for FaultJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            if matches!(entry, BrokerJournalEntry::Dispatch { .. }) {
                *self.dispatch.lock().unwrap() = Some(entry.clone());
                if self.loss == Loss::BeforeDispatch && !self.fired.swap(true, Ordering::SeqCst) {
                    return Err(BrokerJournalError);
                }
            }
            self.inner.append(entry).await?;
            if ((self.loss == Loss::AfterDispatch
                && matches!(entry, BrokerJournalEntry::Dispatch { .. }))
                || (self.loss == Loss::AfterObservation
                    && matches!(entry, BrokerJournalEntry::Observation { .. })))
                && !self.fired.swap(true, Ordering::SeqCst)
            {
                return Err(BrokerJournalError);
            }
            Ok(())
        })
    }
}
struct LedgerCheckingExecutor {
    store: Arc<UnleashStore>,
    effects: Arc<AtomicUsize>,
}
impl ActionExecutorPort for LedgerCheckingExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        intent: &'a ActionIntent,
        _decision: &'a AuthorizationDecision,
        _secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        Box::pin(async move {
            let store = Arc::clone(&self.store);
            let action_id = intent.action_id;
            let (state, dispatch_count, events) = tokio::task::spawn_blocking(move || store.read_snapshot(|conn, _| {
                let state: String = conn.query_row("SELECT state FROM actions WHERE action_id=?1", [action_id.to_string()], |r| r.get(0)).map_err(StoreError::from_rusqlite)?;
                let count: i64 = conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2", rusqlite::params![action_id.to_string(), dispatch_id.to_string()], |r| r.get(0)).map_err(StoreError::from_rusqlite)?;
                let mut stmt = conn.prepare("SELECT event_type FROM events WHERE aggregate_id=?1 ORDER BY sequence").map_err(StoreError::from_rusqlite)?;
                let rows = stmt.query_map([action_id.to_string()], |r| r.get::<_, String>(0)).map_err(StoreError::from_rusqlite)?;
                let mut events = Vec::new(); for row in rows { events.push(row.map_err(StoreError::from_rusqlite)?); }
                Ok((state,count,events))
            })).await.unwrap().unwrap();
            assert_eq!(state, "dispatching");
            assert_eq!(dispatch_count, 1);
            let required = [
                "ActionAuthorizationRequested",
                "ActionAuthorized",
                "ActionDispatchCommitted",
            ];
            let mut reached = 0;
            for event in &events {
                if required
                    .get(reached)
                    .is_some_and(|name| *name == event.as_str())
                {
                    reached += 1;
                }
            }
            assert_eq!(reached, required.len());
            self.effects.fetch_add(1, Ordering::SeqCst);
            Ok(ActionExecutionResult {
                dispatch_id,
                provider_request_id: None,
                output_digest: Sha256Digest::from_bytes(b"benign-local-result"),
                observed_at_ms: TimestampMs::new(1_767_225_601_000),
            })
        })
    }
}

#[tokio::test]
async fn sqlite_journal_commits_before_effect_and_restart_never_duplicates_dispatch() {
    for loss in [
        Loss::None,
        Loss::BeforeDispatch,
        Loss::AfterDispatch,
        Loss::AfterObservation,
    ] {
        run_case(loss).await;
    }
}

async fn run_case(loss: Loss) {
    let dir = local_test_dir("m1-broker-ledger").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xfeed));
    let mut config = StoreConfig::new(dir.path());
    config.clock = clock.clone();
    config.id_generator = ids.clone();
    let open = config.clone();
    let store = Arc::new(
        tokio::task::spawn_blocking(move || {
            let store = UnleashStore::open(open).unwrap();
            store
                .register_principal("p-sys", "system", "p-sys")
                .unwrap();
            store
                .register_principal("p-other", "system", "p-other")
                .unwrap();
            store
        })
        .await
        .unwrap(),
    );
    let row = {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || store.daemon_state())
            .await
            .unwrap()
            .unwrap()
    };
    let fence = LineageFence::new(row.authority_generation, row.daemon_epoch, Fence::new(0));
    // Une vraie commande de maintenance engagée fournit la cause control_plane durable.
    let cause_receipt = {
        let store = Arc::clone(&store);
        let ids = Arc::clone(&ids);
        let authority_generation = row.authority_generation;
        tokio::task::spawn_blocking(move || {
            let actor = ActorId::new("p-sys").unwrap();
            let payload = serde_json::json!({"scope_kind":"user", "scope_id":"default", "patch":{"confirm_retention_shortening":false}});
            let request = CommitCommandRequest {
                actor_id: actor.clone(),
                envelope: CommandEnvelope { protocol_version: ProtocolVersion, request_id: RequestId::from_uuid(ids.new_uuid()), command_id: ids.new_command_id(), authority_generation, actor_ref: None, project_id: None, aggregate_id: authority_generation.as_uuid(), expected_revision: Revision::INITIAL, command_type: "SetSettings".into(), payload: payload.as_object().unwrap().clone() },
                target_aggregate: AggregateKind::Settings, operation_id: None, receipt_state: ReceiptState::Accepted,
                result_ref: None, retention_days_override: None, fence_assertion: None,
                mutation: CommandMutation::SetSettings { scope_kind: SettingsScopeKind::User, scope_id: "default".into(), caller: CallerAuthority { actor_id: actor, role: CallerRole::PolicyAdmin }, patch: SettingsPatch::default() },
                events: Vec::new(), capacity_reservations: Vec::new(), budget_reservations: Vec::new(),
            };
            store.commit_command(request).unwrap().receipt().receipt_id
        }).await.unwrap()
    };
    let authority = compute_effective_authority(
        None,
        Revision::INITIAL,
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: "test-mandate".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from(["http.post".into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: None,
            allowed_network_hosts: Some(BTreeSet::from(["localhost".into()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Global,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );
    let journal = Arc::new(SqliteBrokerJournal::for_store_test(Arc::clone(&store)));
    let fault = Arc::new(FaultJournal {
        inner: Arc::clone(&journal),
        loss,
        fired: AtomicBool::new(false),
        dispatch: Mutex::new(None),
    });
    let obligation = Arc::new(BrokerExecutionSpy::new());
    let broker = ActionBroker::new(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        Arc::new(TestIsolationProbe::standard_available()),
        obligation,
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(fence),
        fault.clone(),
    );
    let intent = ActionIntent {
        action_id: ids.new_action_id(),
        subject: ActionSubject::new_control_plane(
            ActorId::new("p-sys").unwrap(),
            format!("receipt:{cause_receipt}"),
        )
        .unwrap(),
        project_id: None,
        verb: "http.post".into(),
        resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
            host: "localhost".into(),
            port: Some(8000),
            is_private_or_loopback: true,
        }],
        payload_digest: ActionIntent::compute_payload_digest(&serde_json::json!({"body":"test"}))
            .unwrap(),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::NetworkEgress,
        scope_revision: Revision::new(1),
        policy_revision: Revision::INITIAL,
        lineage_fence: fence,
        action_deadline: TimestampMs::new(clock.utc_ms() + 60_000),
        isolation_requirements: IsolationRequirements::from_authority(&authority),
        secret_refs: Vec::new(),
        provenance: ActionProvenance::EngineInternal,
    };
    let id = intent.action_id;
    let mut restart_intent = intent.clone();
    let mut no_cause = intent.clone();
    no_cause.action_id = ids.new_action_id();
    let rejected_action = no_cause.action_id;
    no_cause.subject = ActionSubject::new_control_plane(
        ActorId::new("p-sys").unwrap(),
        format!("receipt:{}", uuid::Uuid::from_u128(0xbad)),
    )
    .unwrap();
    assert!(broker.authorize_action(no_cause, &authority).await.is_err());
    let count = {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || {
            store.read_snapshot(|conn, _| {
                conn.query_row(
                    "SELECT COUNT(*) FROM actions WHERE action_id=?1",
                    [rejected_action.to_string()],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(StoreError::from_rusqlite)
            })
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(count, 0);
    // Un reçu réel ne peut pas devenir une cause pour un autre principal.
    let mut wrong_actor = intent.clone();
    wrong_actor.action_id = ids.new_action_id();
    let wrong_actor_action = wrong_actor.action_id;
    wrong_actor.subject = ActionSubject::new_control_plane(
        ActorId::new("p-other").unwrap(),
        format!("receipt:{cause_receipt}"),
    )
    .unwrap();
    assert!(
        broker
            .authorize_action(wrong_actor, &authority)
            .await
            .is_err()
    );
    let count = {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || {
            store.read_snapshot(|conn, _| {
                conn.query_row(
                    "SELECT COUNT(*) FROM actions WHERE action_id=?1",
                    [wrong_actor_action.to_string()],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(StoreError::from_rusqlite)
            })
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(count, 0);
    let AuthorizationOutcome::Authorized(decision) =
        broker.authorize_action(intent, &authority).await.unwrap()
    else {
        panic!("autorisation attendue")
    };
    let effects = Arc::new(AtomicUsize::new(0));
    let executor = LedgerCheckingExecutor {
        store: Arc::clone(&store),
        effects: Arc::clone(&effects),
    };
    let secrets = SecretResolutionSpy::in_memory();
    let outcome = broker
        .execute_authorized(
            id,
            &decision.decision_ref,
            fence,
            &authority,
            &executor,
            &secrets,
        )
        .await;
    if loss == Loss::None {
        assert!(outcome.is_ok());
    } else {
        assert!(matches!(
            outcome,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
    }
    let expected_effects = usize::from(matches!(loss, Loss::None | Loss::AfterObservation));
    assert_eq!(effects.load(Ordering::SeqCst), expected_effects);
    let dispatch = fault.dispatch.lock().unwrap().clone().unwrap();
    let BrokerJournalEntry::Dispatch { dispatch_id, .. } = &dispatch else {
        panic!("dispatch attendu")
    };
    let (before_restart, observed_events, dispatches, dispatch_events, observed_payload) = {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || {
            store.read_snapshot(|conn, _| {
                let state = conn.query_row(
                    "SELECT state FROM actions WHERE action_id=?1",
                    [id.to_string()],
                    |r| r.get::<_, String>(0),
                )
                .map_err(StoreError::from_rusqlite)?;
                let observed: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionObserved'", [id.to_string()], |r| r.get(0)).map_err(StoreError::from_rusqlite)?;
                let dispatches: i64 = conn.query_row("SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1", [id.to_string()], |r| r.get(0)).map_err(StoreError::from_rusqlite)?;
                let dispatch_events: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionDispatchCommitted'", [id.to_string()], |r| r.get(0)).map_err(StoreError::from_rusqlite)?;
                let payload: Option<String> = conn.query_row("SELECT payload_json FROM events WHERE aggregate_id=?1 AND event_type='ActionObserved'", [id.to_string()], |r| r.get(0)).optional().map_err(StoreError::from_rusqlite)?;
                Ok((state, observed, dispatches, dispatch_events, payload))
            })
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(usize::try_from(observed_events).unwrap(), expected_effects);
    assert_eq!(dispatches, i64::from(loss != Loss::BeforeDispatch));
    assert_eq!(dispatch_events, dispatches);
    if expected_effects == 1 {
        let payload: serde_json::Value =
            serde_json::from_str(observed_payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["action_id"], id.to_string());
        assert_eq!(payload["evaluation_id"], decision.evaluation_id.to_string());
        assert_eq!(payload["dispatch_id"], dispatch_id.to_string());
        assert_eq!(payload["observation_revision"], 1);
        assert_eq!(
            payload["output_digest"],
            Sha256Digest::from_bytes(b"benign-local-result").to_string()
        );
    }
    match loss {
        Loss::None | Loss::AfterObservation => assert_eq!(before_restart, "observed_success"),
        Loss::AfterDispatch => assert_eq!(before_restart, "dispatching"),
        Loss::BeforeDispatch => assert_eq!(before_restart, "authorized"),
    }
    if loss != Loss::BeforeDispatch {
        // Même dans la même époque, un marqueur déjà engagé ne peut accorder un nouveau lancement.
        assert!(journal.append(&dispatch).await.is_err());
    }
    drop(broker);
    drop(fault);
    drop(journal);
    drop(executor);
    drop(secrets);
    let reopened = Arc::new(
        tokio::task::spawn_blocking(move || {
            let owned = Arc::try_unwrap(store).unwrap();
            let reopened = owned.restart_same_lineage().unwrap();
            reopened
                .reconcile_recovered_actions_after_restart()
                .unwrap();
            reopened
        })
        .await
        .unwrap(),
    );
    let reopened_row = {
        let reopened = Arc::clone(&reopened);
        tokio::task::spawn_blocking(move || reopened.daemon_state())
            .await
            .unwrap()
            .unwrap()
    };
    assert_eq!(reopened_row.authority_generation, row.authority_generation);
    assert_eq!(
        reopened_row.daemon_epoch.as_u64(),
        row.daemon_epoch.as_u64() + 1
    );
    let after_restart = {
        let reopened = Arc::clone(&reopened);
        tokio::task::spawn_blocking(move || {
            reopened.read_snapshot(|conn, _| {
                conn.query_row(
                    "SELECT state FROM actions WHERE action_id=?1",
                    [id.to_string()],
                    |r| r.get::<_, String>(0),
                )
                .map_err(StoreError::from_rusqlite)
            })
        })
        .await
        .unwrap()
        .unwrap()
    };
    if loss == Loss::AfterDispatch {
        assert_eq!(after_restart, "outcome_unknown");
    }
    if matches!(loss, Loss::None | Loss::AfterObservation) {
        assert_eq!(after_restart, "observed_success");
    }
    let journal2 = Arc::new(SqliteBrokerJournal::for_store_test(Arc::clone(&reopened)));
    if loss != Loss::BeforeDispatch {
        assert!(journal2.append(&dispatch).await.is_err());
    }
    let restart_fence = LineageFence::new(
        reopened_row.authority_generation,
        reopened_row.daemon_epoch,
        Fence::new(0),
    );
    restart_intent.lineage_fence = restart_fence;
    let fresh_broker = ActionBroker::new(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        Arc::new(TestIsolationProbe::standard_available()),
        Arc::new(BrokerExecutionSpy::new()),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(restart_fence),
        journal2.clone(),
    );
    assert!(
        fresh_broker
            .authorize_action(restart_intent, &authority)
            .await
            .is_err()
    );
    // Le fence de la nouvelle époque modifie le manifeste. Ce refus ne prouve
    // donc pas à lui seul la réhydratation d'une évaluation identique ; on
    // vérifie aussi qu'aucune nouvelle intention n'a été engagée.
    let after_attempt = {
        let reopened = Arc::clone(&reopened);
        tokio::task::spawn_blocking(move || {
            reopened.read_snapshot(|conn, _| {
                let state: String = conn.query_row(
                    "SELECT state FROM actions WHERE action_id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                ).map_err(StoreError::from_rusqlite)?;
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE aggregate_id=?1 AND event_type='ActionAuthorizationRequested'",
                    [id.to_string()],
                    |r| r.get(0),
                ).map_err(StoreError::from_rusqlite)?;
                Ok((state, count))
            })
        }).await.unwrap().unwrap()
    };
    assert_eq!(after_attempt, (after_restart, 1));
    assert_eq!(effects.load(Ordering::SeqCst), expected_effects);
    drop(fresh_broker);
    drop(journal2);
    tokio::task::spawn_blocking(move || drop(reopened))
        .await
        .unwrap();
}
