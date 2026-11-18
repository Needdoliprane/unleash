//! Broker boundary regressions: T-F017-003/005/007/008/010; local doubles only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use secrecy::SecretString;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use unleash_action_broker::*;
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, DispatchId, Fence, IdGeneratorPort,
    LineageFence, ModelPolicy, ReconcileOutcome, ReconcileStrategy, Revision, TimestampMs,
};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicGuardDouble, DeterministicIdGenerator, ManualClock,
    ManualClockTimer, ScriptedGuardOutcome, SecretResolutionSpy, SimulatedExecutionFailure,
    TestIsolationProbe,
};

struct Fixture {
    broker: ActionBroker,
    authority: EffectiveAuthority,
    clock: Arc<ManualClock>,
    ids: Arc<DeterministicIdGenerator>,
    spy: Arc<BrokerExecutionSpy>,
    secrets: SecretResolutionSpy<unleash_testkit::InMemoryTestSecretStore>,
}

const JOURNAL_CANARY: &str = "test-sensitive-guard-canary";
const ESCAPED_CANARY: &str = "guard-\"quoted\\backslash\nsecret";

fn fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(101),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

impl Fixture {
    fn new() -> Self {
        Self::with_journal(None)
    }

    fn with_journal(journal: Option<Arc<dyn BrokerJournalPort>>) -> Self {
        let clock = Arc::new(ManualClock::new(1_767_225_600_000));
        let ids = Arc::new(DeterministicIdGenerator::new(101));
        let spy = Arc::new(BrokerExecutionSpy::new());
        let authority = compute_effective_authority(
            None,
            Revision::INITIAL,
            Revision::INITIAL,
            &[AuthorityLayer {
                layer_name: "test-mandate".into(),
                allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
                denied_effects: BTreeSet::new(),
                allowed_verbs: Some(BTreeSet::from(["http.post".into()])),
                denied_verbs: BTreeSet::new(),
                allowed_roots: None,
                allowed_network_hosts: Some(BTreeSet::from(["localhost".into()])),
                allowed_secret_refs: Some(BTreeSet::from(["os-secret:test/journal".into()])),
                model_policy: ModelPolicy::Forbidden,
                data_locality: DataLocality::LocalOnly,
                isolation_mode: IsolationMode::Standard,
                require_action_isolation: false,
                max_scope_kind: ScopeKind::Global,
                allow_delegation: false,
                allow_cross_repository: false,
            }],
        );
        let mut config = BrokerConfig::for_test(fence());
        config.canaries_to_redact = vec![
            SecretString::from(JOURNAL_CANARY.to_owned()),
            SecretString::from(ESCAPED_CANARY.to_owned()),
        ];
        let mut broker = ActionBroker::new_for_test_memory(
            clock.clone(),
            clock.clone(),
            Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
            ids.clone(),
            Arc::new(TestIsolationProbe::standard_available()),
            spy.clone(),
            Arc::new(LocalAuthorityFallbackPolicy::default()),
            config.clone(),
        );
        if let Some(journal) = journal {
            broker = ActionBroker::new(
                clock.clone(),
                clock.clone(),
                Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
                ids.clone(),
                Arc::new(TestIsolationProbe::standard_available()),
                spy.clone(),
                Arc::new(LocalAuthorityFallbackPolicy::default()),
                config,
                journal,
            );
        }
        let secrets = SecretResolutionSpy::in_memory();
        secrets
            .store(
                &SecretRef::new("os-secret:test/journal").unwrap(),
                SecretString::from(JOURNAL_CANARY.to_owned()),
            )
            .unwrap();
        Self {
            broker,
            authority,
            clock,
            ids,
            spy,
            secrets,
        }
    }

    fn intent(&self) -> ActionIntent {
        ActionIntent {
            action_id: self.ids.new_action_id(),
            subject: ActionSubject::new_control_plane(
                ActorId::new("test-admin").unwrap(),
                "cmd:test".into(),
            )
            .unwrap(),
            project_id: None,
            verb: "http.post".into(),
            resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
                host: "localhost".into(),
                port: Some(8000),
                is_private_or_loopback: true,
            }],
            payload_digest: ActionIntent::compute_payload_digest(
                &serde_json::json!({"body":"test"}),
            )
            .unwrap(),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::NetworkEgress,
            scope_revision: Revision::INITIAL,
            policy_revision: Revision::INITIAL,
            lineage_fence: fence(),
            action_deadline: TimestampMs::new(self.clock.utc_ms() + 60_000),
            isolation_requirements: IsolationRequirements::from_authority(&self.authority),
            secret_refs: vec![SecretRef::new("os-secret:test/journal").unwrap()],
            provenance: ActionProvenance::EngineInternal,
        }
    }

    async fn authorize(&self, intent: ActionIntent) -> AuthorizationDecision {
        let AuthorizationOutcome::Authorized(decision) = self
            .broker
            .authorize_action(intent, &self.authority)
            .await
            .unwrap()
        else {
            panic!("expected authorization")
        };
        decision
    }

    async fn guard(
        &self,
        double: Arc<DeterministicGuardDouble>,
    ) -> Result<GuardHealthSnapshot, ActionBrokerError> {
        self.broker
            .configure_guard(
                &CallerIdentity::admin(ActorId::new("admin").unwrap()),
                ConfigureGuardCommand {
                    endpoint: Some("unix:///guard-test.sock".into()),
                    ..ConfigureGuardCommand::default()
                },
                Some(double),
            )
            .await
    }
}

#[derive(Default)]
struct JournalDouble {
    fail_on: Option<&'static str>,
    entries: Mutex<Vec<&'static str>>,
    surfaces: Mutex<Vec<String>>,
    hang_on: Mutex<Option<&'static str>>,
}

#[derive(Default)]
struct LostDispatchAcknowledgement {
    marker: Mutex<Option<DispatchId>>,
    attempts: Mutex<usize>,
}

impl BrokerJournalPort for LostDispatchAcknowledgement {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        if let BrokerJournalEntry::Dispatch { dispatch_id, .. } = entry {
            let mut marker = self.marker.lock().unwrap();
            if let Some(existing) = *marker {
                assert_eq!(existing, *dispatch_id);
            } else {
                *marker = Some(*dispatch_id);
            }
            *self.attempts.lock().unwrap() += 1;
            // First commit lost its acknowledgement; an exact duplicate is also refused.
            return Box::pin(async { Err(BrokerJournalError) });
        }
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn t_f017_010_preexisting_durable_dispatch_never_grants_a_new_launch() {
    let journal = Arc::new(LostDispatchAcknowledgement::default());
    for _ in 0..2 {
        // Same deterministic identity stream represents rehydrating the same command after restart.
        let f = Fixture::with_journal(Some(journal.clone()));
        let intent = f.intent();
        let id = intent.action_id;
        let dec = f.authorize(intent).await;
        assert!(matches!(
            f.broker
                .execute_authorized(
                    id,
                    &dec.decision_ref,
                    fence(),
                    &f.authority,
                    f.spy.as_ref(),
                    &f.secrets
                )
                .await,
            Err(ActionBrokerError::OutcomeUnknown { .. })
        ));
        f.spy.assert_zero_dispatches();
        f.secrets.assert_zero_secret_resolutions();
        assert!(
            f.broker
                .execute_authorized(
                    id,
                    &dec.decision_ref,
                    fence(),
                    &f.authority,
                    f.spy.as_ref(),
                    &f.secrets
                )
                .await
                .is_err()
        );
    }
    assert!(journal.marker.lock().unwrap().is_some());
    assert_eq!(*journal.attempts.lock().unwrap(), 2);
}

impl BrokerJournalPort for JournalDouble {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        let name = match entry {
            BrokerJournalEntry::Authorizing { .. } => "authorizing",
            BrokerJournalEntry::Authorization { .. } => "authorization",
            BrokerJournalEntry::Dispatch { .. } => "dispatch",
            BrokerJournalEntry::Observation { .. } => "observation",
        };
        self.entries.lock().unwrap().push(name);
        self.surfaces.lock().unwrap().push(format!("{entry:?}"));
        let hang = *self.hang_on.lock().unwrap() == Some(name);
        Box::pin(async move {
            if hang {
                std::future::pending::<()>().await;
            }
            if self.fail_on == Some(name) {
                Err(BrokerJournalError)
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn t_f017_003_guard_transport_errors_are_redacted_before_journal_and_audit() {
    let journal = Arc::new(JournalDouble::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    double.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: JOURNAL_CANARY.into(),
    });
    f.guard(double).await.unwrap();
    let error = f
        .broker
        .authorize_action(f.intent(), &f.authority)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(JOURNAL_CANARY));
    assert!(!format!("{:?}", f.broker.audit_events()).contains(JOURNAL_CANARY));
    assert!(
        !journal
            .surfaces
            .lock()
            .unwrap()
            .join("")
            .contains(JOURNAL_CANARY)
    );
}

#[tokio::test]
async fn t_f017_003_json_escaping_does_not_hide_sensitive_guard_metadata() {
    let f = Fixture::new();
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    double.push_outcome(ScriptedGuardOutcome::Deny {
        reason_code: ESCAPED_CANARY.into(),
        detail: "denied".into(),
    });
    f.guard(double).await.unwrap();
    let error = f
        .broker
        .authorize_action(f.intent(), &f.authority)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(ESCAPED_CANARY));
    f.spy.assert_zero_dispatches();
    f.secrets.assert_zero_secret_resolutions();
}

#[tokio::test]
async fn t_f017_007_cancelled_authorization_releases_its_in_memory_reservation() {
    for phase in ["authorizing", "authorization"] {
        let journal = Arc::new(JournalDouble {
            hang_on: Mutex::new(Some(phase)),
            ..JournalDouble::default()
        });
        let f = Fixture::with_journal(Some(journal.clone()));
        let intent = f.intent();
        {
            let future = f.broker.authorize_action(intent.clone(), &f.authority);
            tokio::pin!(future);
            tokio::select! {
                _ = &mut future => panic!("journal should be pending"),
                () = async { while !journal.entries.lock().unwrap().contains(&phase) { tokio::task::yield_now().await; } } => {},
            }
            assert!(
                !f.broker
                    .audit_events()
                    .iter()
                    .any(|event| matches!(event, BrokerAuditEvent::ActionAuthorized { .. }))
            );
        }
        *journal.hang_on.lock().unwrap() = None;
        f.clock.advance(Duration::from_secs(1));
        f.authorize(intent).await;
        f.spy.assert_zero_dispatches();
    }
}

#[tokio::test]
async fn t_f017_010_cancelled_observation_stays_reconcilable_without_new_dispatch() {
    let journal = Arc::new(JournalDouble {
        hang_on: Mutex::new(Some("observation")),
        ..JournalDouble::default()
    });
    let f = Fixture::with_journal(Some(journal.clone()));
    let intent = f.intent();
    let id = intent.action_id;
    let dec = f.authorize(intent).await;
    {
        let future = f.broker.execute_authorized(
            id,
            &dec.decision_ref,
            fence(),
            &f.authority,
            f.spy.as_ref(),
            &f.secrets,
        );
        tokio::pin!(future);
        tokio::select! {
            _ = &mut future => panic!("journal should be pending"),
            () = async { while !journal.entries.lock().unwrap().contains(&"observation") { tokio::task::yield_now().await; } } => {},
        }
    }
    *journal.hang_on.lock().unwrap() = None;
    let receipt = f
        .broker
        .reconcile_action(
            id,
            ReconcileOutcome::ConfirmedSuccess,
            Some("local-proof".into()),
        )
        .await
        .unwrap();
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    assert_eq!(f.spy.dispatch_count(), 1);
}

#[tokio::test]
async fn t_f017_002_journal_commits_all_four_phases_in_order() {
    let journal = Arc::new(JournalDouble::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let intent = f.intent();
    let id = intent.action_id;
    let decision = f.authorize(intent).await;
    f.broker
        .execute_authorized(
            id,
            &decision.decision_ref,
            fence(),
            &f.authority,
            f.spy.as_ref(),
            &f.secrets,
        )
        .await
        .unwrap();
    assert_eq!(
        *journal.entries.lock().unwrap(),
        vec!["authorizing", "authorization", "dispatch", "observation"]
    );
}

#[tokio::test]
async fn t_f017_002_journal_failures_before_dispatch_prevent_every_effect() {
    for phase in ["authorizing", "authorization", "dispatch"] {
        let journal = Arc::new(JournalDouble {
            fail_on: Some(phase),
            ..JournalDouble::default()
        });
        let f = Fixture::with_journal(Some(journal));
        let intent = f.intent();
        let id = intent.action_id;
        let result = f.broker.authorize_action(intent, &f.authority).await;
        if phase == "dispatch" {
            let AuthorizationOutcome::Authorized(dec) = result.unwrap() else {
                panic!("expected allow")
            };
            assert!(
                f.broker
                    .execute_authorized(
                        id,
                        &dec.decision_ref,
                        fence(),
                        &f.authority,
                        f.spy.as_ref(),
                        &f.secrets
                    )
                    .await
                    .is_err()
            );
        } else {
            assert!(result.is_err());
        }
        f.spy.assert_zero_dispatches();
        f.secrets.assert_zero_secret_resolutions();
    }
}

#[tokio::test]
async fn t_f017_010_observation_commit_failure_stays_unknown_and_never_redispatches() {
    let f = Fixture::with_journal(Some(Arc::new(JournalDouble {
        fail_on: Some("observation"),
        ..JournalDouble::default()
    })));
    let intent = f.intent();
    let id = intent.action_id;
    let dec = f.authorize(intent).await;
    assert!(matches!(
        f.broker
            .execute_authorized(
                id,
                &dec.decision_ref,
                fence(),
                &f.authority,
                f.spy.as_ref(),
                &f.secrets
            )
            .await,
        Err(ActionBrokerError::OutcomeUnknown { .. })
    ));
    assert_eq!(f.broker.action_state(id), Some(ActionState::OutcomeUnknown));
    assert!(
        f.broker
            .execute_authorized(
                id,
                &dec.decision_ref,
                fence(),
                &f.authority,
                f.spy.as_ref(),
                &f.secrets
            )
            .await
            .is_err()
    );
    assert_eq!(f.spy.dispatch_count(), 1);
}

#[tokio::test]
async fn t_f017_008_same_action_retries_only_after_backoff() {
    let f = Fixture::new();
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    double.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: "offline".into(),
    });
    f.guard(double.clone()).await.unwrap();
    let intent = f.intent();
    let id = intent.action_id;
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    assert_eq!(f.broker.action_state(id), Some(ActionState::Authorizing));
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    assert_eq!(double.evaluate_call_count(), 1);
    f.clock.advance(Duration::from_secs(1));
    f.authorize(intent).await;
    assert_eq!(double.evaluate_call_count(), 2);
}

#[tokio::test]
async fn t_f017_008_negotiation_cannot_promote_the_none_accepted_profile() {
    let f = Fixture::new();
    let profile = GuardMappingProfile::Qualified {
        profile_id: "unrequested-test-profile.v1".into(),
        version: 1,
        supported_obligations: BTreeSet::new(),
        supports_remote_approval: true,
    };
    let double = Arc::new(DeterministicGuardDouble::with_profile(
        f.clock.clone(),
        profile,
    ));
    assert!(f.guard(double).await.is_err());
    assert_eq!(
        f.broker.read_guard_health().await.unwrap().mode,
        GuardMode::BrokerOnly
    );
}

#[test]
fn t_f017_005_resource_references_round_trip_without_panicking() {
    let resource = NormalizedResourceRef::Path("/safe/path".into());
    let wire = serde_json::to_value(&resource).unwrap();
    assert_eq!(
        serde_json::from_value::<NormalizedResourceRef>(wire).unwrap(),
        resource
    );
}

#[test]
fn t_f017_008_deserialization_cannot_create_a_replay_with_a_fake_agent() {
    let f = Fixture::new();
    let mut wire = serde_json::to_value(f.intent().subject).unwrap();
    wire.as_object_mut()
        .unwrap()
        .insert("kind".into(), serde_json::json!("replay"));
    wire.as_object_mut().unwrap().insert(
        "agent_id".into(),
        serde_json::json!("00000000-0000-0000-0000-000000000001"),
    );
    assert!(serde_json::from_value::<ActionSubject>(wire).is_err());
}

#[tokio::test]
async fn t_f017_008_guard_reconfiguration_invalidates_existing_local_decisions() {
    let f = Fixture::new();
    let intent = f.intent();
    let id = intent.action_id;
    let decision = f.authorize(intent).await;
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    double.set_default_outcome(ScriptedGuardOutcome::Deny {
        reason_code: "DENIED".into(),
        detail: "new policy".into(),
    });
    f.guard(double).await.unwrap();
    assert!(
        f.broker
            .execute_authorized(
                id,
                &decision.decision_ref,
                fence(),
                &f.authority,
                f.spy.as_ref(),
                &f.secrets
            )
            .await
            .is_err()
    );
    f.spy.assert_zero_dispatches();
    f.secrets.assert_zero_secret_resolutions();
}

#[tokio::test]
async fn t_f017_007_cache_is_bound_to_the_complete_authenticated_subject() {
    let f = Fixture::new();
    let double = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    double.set_default_outcome(ScriptedGuardOutcome::Allow {
        expiry_offset_ms: Some(60_000),
        constraints: vec![],
    });
    f.guard(double.clone()).await.unwrap();
    f.authorize(f.intent()).await;
    let mut changed = f.intent();
    changed.subject = ActionSubject::new_control_plane(
        ActorId::new("test-admin").unwrap(),
        "different-mandate".into(),
    )
    .unwrap();
    f.authorize(changed).await;
    assert_eq!(double.evaluate_call_count(), 2);
}

#[tokio::test]
async fn t_f017_005_action_cannot_remove_authority_isolation_requirements() {
    let f = Fixture::new();
    let intent = f.intent();
    let mut authority = f.authority.clone();
    authority.require_action_isolation = true;
    assert!(f.broker.authorize_action(intent, &authority).await.is_err());
    f.spy.assert_zero_dispatches();
}

#[tokio::test]
async fn t_f017_005_broker_policy_cannot_be_downgraded_by_supplied_authority() {
    let f = Fixture::new();
    f.broker
        .set_policy(
            &CallerIdentity::admin(ActorId::new("admin").unwrap()),
            SetPolicyCommand {
                project_id: None,
                expected_revision: Revision::INITIAL,
                isolation_mode: IsolationMode::Strict,
                reason: "strict".into(),
            },
        )
        .unwrap();
    let mut authority = f.authority.clone();
    authority.policy_revision = Revision::INITIAL.next();
    let mut intent = f.intent();
    intent.policy_revision = authority.policy_revision;
    assert!(f.broker.authorize_action(intent, &authority).await.is_err());
}

#[test]
fn t_f017_005_host_binary_presence_does_not_prove_enforced_isolation() {
    assert!(
        verify_isolation_requirements(
            &IsolationRequirements::strict_default(),
            &HostIsolationProbe
        )
        .is_err()
    );
}

#[tokio::test]
async fn t_f017_010_executor_observation_uses_the_committed_dispatch_identity() {
    let f = Fixture::new();
    let intent = f.intent();
    let id = intent.action_id;
    let dec = f.authorize(intent).await;
    let receipt = f
        .broker
        .execute_authorized(
            id,
            &dec.decision_ref,
            fence(),
            &f.authority,
            f.spy.as_ref(),
            &f.secrets,
        )
        .await
        .unwrap();
    let committed = f
        .broker
        .audit_events()
        .into_iter()
        .find_map(|e| match e {
            BrokerAuditEvent::ActionDispatchCommitted { dispatch_id, .. } => Some(dispatch_id),
            _ => None,
        })
        .unwrap();
    assert_eq!(receipt.dispatch_id, committed);
    assert_eq!(f.spy.dispatches().first().unwrap().dispatch_id, committed);
}

#[tokio::test]
async fn t_f017_010_foreign_dispatch_receipt_does_not_resolve_unknown_action() {
    let f = Fixture::new();
    let intent = f.intent();
    let id = intent.action_id;
    let dec = f.authorize(intent).await;
    f.spy
        .set_next_execution_failure(SimulatedExecutionFailure::OutcomeUnknown {
            reason: "lost observation".into(),
        });
    assert!(
        f.broker
            .execute_authorized(
                id,
                &dec.decision_ref,
                fence(),
                &f.authority,
                f.spy.as_ref(),
                &f.secrets
            )
            .await
            .is_err()
    );
    assert!(
        f.broker
            .apply_external_receipt(
                id,
                DispatchId::from_u128(999),
                fence(),
                ReconcileOutcome::ConfirmedSuccess,
                Some("foreign".into())
            )
            .await
            .is_err()
    );
    assert_eq!(f.broker.action_state(id), Some(ActionState::OutcomeUnknown));
}
