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

#[derive(Default)]
struct SensitiveReferenceExecutor {
    calls: std::sync::atomic::AtomicU64,
}

impl ActionExecutorPort for SensitiveReferenceExecutor {
    fn execute_action<'a>(
        &'a self,
        dispatch_id: DispatchId,
        _intent: &'a ActionIntent,
        _decision: &'a AuthorizationDecision,
        _secrets: &'a [ResolvedSecret],
    ) -> BoxFuture<'a, Result<ActionExecutionResult, ActionExecutionFailure>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            Ok(ActionExecutionResult {
                dispatch_id,
                provider_request_id: Some(JOURNAL_CANARY.into()),
                output_digest: unleash_domain::Sha256Digest::from_bytes(b"local test output"),
                observed_at_ms: TimestampMs::new(1_767_225_600_000),
            })
        })
    }
}

#[tokio::test]
async fn t_f017_003_sensitive_remote_reference_is_omitted_without_changing_observed_success() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let intent = f.intent();
    let id = intent.action_id;
    let decision = f.authorize(intent).await;
    let executor = SensitiveReferenceExecutor::default();
    let receipt = f
        .broker
        .execute_authorized(
            id,
            &decision.decision_ref,
            fence(),
            &f.authority,
            &executor,
            &f.secrets,
        )
        .await
        .unwrap();
    assert_eq!(receipt.state, ActionState::ObservedSuccess);
    assert_eq!(receipt.remote_audit_ref, None);
    assert!(receipt.remote_audit_unavailable);
    assert_eq!(executor.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!format!("{:?}", journal.committed.lock().unwrap()).contains(JOURNAL_CANARY));
}

#[derive(Default)]
struct StrictEvaluationJournal {
    committed: Mutex<Vec<BrokerJournalEntry>>,
    attempts: Mutex<Vec<BrokerJournalEntry>>,
    lose_authorization_ack: std::sync::atomic::AtomicBool,
    hang_authorization_ack: std::sync::atomic::AtomicBool,
    lose_observation_ack: std::sync::atomic::AtomicBool,
    lose_approval_ack: std::sync::atomic::AtomicBool,
    lose_authorizing_ack: std::sync::atomic::AtomicBool,
    hang_authorizing_ack: std::sync::atomic::AtomicBool,
    reject_authorization_before_commit: std::sync::atomic::AtomicBool,
    authorization_gate: tokio::sync::Notify,
}

fn evaluation_of(entry: &BrokerJournalEntry) -> &AuthorizationEvaluation {
    match entry {
        BrokerJournalEntry::Authorizing { evaluation, .. }
        | BrokerJournalEntry::Authorization { evaluation, .. }
        | BrokerJournalEntry::Dispatch { evaluation, .. }
        | BrokerJournalEntry::Observation { evaluation, .. } => evaluation,
    }
}

fn same_journal_slot(a: &BrokerJournalEntry, b: &BrokerJournalEntry) -> bool {
    if evaluation_of(a).evaluation_id != evaluation_of(b).evaluation_id {
        return false;
    }
    match (a, b) {
        (BrokerJournalEntry::Authorizing { .. }, BrokerJournalEntry::Authorizing { .. })
        | (BrokerJournalEntry::Dispatch { .. }, BrokerJournalEntry::Dispatch { .. }) => true,
        (
            BrokerJournalEntry::Authorization { cause: a, .. },
            BrokerJournalEntry::Authorization { cause: b, .. },
        ) => a == b,
        (
            BrokerJournalEntry::Observation {
                observation_revision: a,
                ..
            },
            BrokerJournalEntry::Observation {
                observation_revision: b,
                ..
            },
        ) => a == b,
        _ => false,
    }
}

impl BrokerJournalPort for StrictEvaluationJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        self.attempts.lock().unwrap().push(entry.clone());
        if matches!(
            entry,
            BrokerJournalEntry::Authorization {
                cause: AuthorizationCause::Evaluation,
                ..
            }
        ) && self
            .reject_authorization_before_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Box::pin(async { Err(BrokerJournalError) });
        }
        let mut committed = self.committed.lock().unwrap();
        if let Some(previous) = committed
            .iter()
            .find(|previous| same_journal_slot(previous, entry))
        {
            if previous != entry || matches!(entry, BrokerJournalEntry::Dispatch { .. }) {
                return Box::pin(async { Err(BrokerJournalError) });
            }
        } else {
            committed.push(entry.clone());
        }
        drop(committed);
        let is_result = matches!(
            entry,
            BrokerJournalEntry::Authorization {
                cause: AuthorizationCause::Evaluation,
                ..
            }
        );
        let lost = (is_result
            && self
                .lose_authorization_ack
                .swap(false, std::sync::atomic::Ordering::SeqCst))
            || (matches!(entry, BrokerJournalEntry::Authorizing { .. })
                && self
                    .lose_authorizing_ack
                    .swap(false, std::sync::atomic::Ordering::SeqCst))
            || (matches!(
                entry,
                BrokerJournalEntry::Authorization {
                    cause: AuthorizationCause::HumanApproval { .. },
                    ..
                }
            ) && self
                .lose_approval_ack
                .swap(false, std::sync::atomic::Ordering::SeqCst))
            || (matches!(entry, BrokerJournalEntry::Observation { .. })
                && self
                    .lose_observation_ack
                    .swap(false, std::sync::atomic::Ordering::SeqCst));
        let hang = (is_result
            && self
                .hang_authorization_ack
                .swap(false, std::sync::atomic::Ordering::SeqCst))
            || (matches!(entry, BrokerJournalEntry::Authorizing { .. })
                && self
                    .hang_authorizing_ack
                    .swap(false, std::sync::atomic::Ordering::SeqCst));
        Box::pin(async move {
            if hang {
                self.authorization_gate.notified().await;
            }
            if lost {
                Err(BrokerJournalError)
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn t_f017_008_retry_has_a_new_evaluation_only_after_durable_unavailable_and_backoff() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    guard.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: "offline".into(),
    });
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    let attempts_before = journal.attempts.lock().unwrap().clone();
    let ids_before = f.ids.generated_count();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    assert_eq!(*journal.attempts.lock().unwrap(), attempts_before);
    assert_eq!(f.ids.generated_count(), ids_before);
    f.clock.advance(Duration::from_secs(1));
    let decision = f.authorize(intent).await;
    let committed = journal.committed.lock().unwrap();
    let starts: Vec<_> = committed
        .iter()
        .filter(|entry| matches!(entry, BrokerJournalEntry::Authorizing { .. }))
        .map(evaluation_of)
        .collect();
    assert_eq!(starts.len(), 2);
    assert_ne!(
        starts.first().unwrap().evaluation_id,
        starts.get(1).unwrap().evaluation_id
    );
    assert_eq!(
        starts.get(1).unwrap().previous_evaluation_id,
        Some(starts.first().unwrap().evaluation_id)
    );
    assert_eq!(decision.evaluation_id, starts.get(1).unwrap().evaluation_id);
    assert_eq!(guard.evaluate_call_count(), 2);
    f.spy.assert_zero_dispatches();
}

#[tokio::test]
async fn t_f017_007_authorizing_ack_loss_or_cancel_keeps_the_first_identity() {
    for cancel in [false, true] {
        let journal = Arc::new(StrictEvaluationJournal::default());
        journal
            .lose_authorizing_ack
            .store(!cancel, std::sync::atomic::Ordering::SeqCst);
        journal
            .hang_authorizing_ack
            .store(cancel, std::sync::atomic::Ordering::SeqCst);
        let f = Fixture::with_journal(Some(journal.clone()));
        let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
            f.clock.clone(),
        ));
        f.guard(guard.clone()).await.unwrap();
        let intent = f.intent();
        if cancel {
            let future = f.broker.authorize_action(intent.clone(), &f.authority);
            tokio::pin!(future);
            tokio::select! { _ = &mut future => panic!("expected pending intent commit"), () = async { while journal.attempts.lock().unwrap().is_empty() { tokio::task::yield_now().await; } } => {} }
        } else {
            assert!(
                f.broker
                    .authorize_action(intent.clone(), &f.authority)
                    .await
                    .is_err()
            );
        }
        assert_eq!(guard.evaluate_call_count(), 0);
        let decision = f.authorize(intent).await;
        assert_eq!(guard.evaluate_call_count(), 1);
        let attempts = journal.attempts.lock().unwrap();
        assert_eq!(attempts.first(), attempts.get(1));
        assert_eq!(
            evaluation_of(attempts.first().unwrap()).evaluation_id,
            decision.evaluation_id
        );
    }
}

#[tokio::test]
async fn t_f017_007_result_rejected_before_commit_keeps_the_known_result() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .reject_authorization_before_commit
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    f.authorize(intent).await;
    assert_eq!(guard.evaluate_call_count(), 1);
    let attempts = journal.attempts.lock().unwrap();
    assert_eq!(attempts.get(1), attempts.get(2));
}

#[tokio::test]
async fn t_f017_008_unavailable_ack_loss_must_be_resolved_before_a_new_evaluation() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .lose_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    guard.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: "offline".into(),
    });
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    f.clock.advance(Duration::from_secs(20));
    assert!(matches!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await,
        Err(ActionBrokerError::GuardUnavailable { .. })
    ));
    assert_eq!(guard.evaluate_call_count(), 1);
    let ids_before = f.ids.generated_count();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    assert_eq!(f.ids.generated_count(), ids_before);
    f.clock.advance(Duration::from_secs(1));
    f.authorize(intent).await;
    assert_eq!(guard.evaluate_call_count(), 2);
    let attempts = journal.attempts.lock().unwrap();
    assert_eq!(attempts.get(1), attempts.get(2));
}

#[tokio::test]
async fn t_f017_007_two_retry_callers_share_one_pending_evaluation() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    guard.push_outcome(ScriptedGuardOutcome::Unavailable {
        reason: "offline".into(),
    });
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    f.clock.advance(Duration::from_secs(1));
    journal
        .hang_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    {
        let future = f.broker.authorize_action(intent.clone(), &f.authority);
        tokio::pin!(future);
        tokio::select! { _ = &mut future => panic!("expected pending retry result"), () = async { while guard.evaluate_call_count() < 2 { tokio::task::yield_now().await; } } => {} }
        let generated = f.ids.generated_count();
        assert!(
            f.broker
                .authorize_action(intent.clone(), &f.authority)
                .await
                .is_err()
        );
        assert_eq!(f.ids.generated_count(), generated);
    }
    f.authorize(intent).await;
    assert_eq!(guard.evaluate_call_count(), 2);
    assert_eq!(
        journal
            .committed
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| matches!(entry, BrokerJournalEntry::Authorizing { .. }))
            .count(),
        2
    );
}

#[tokio::test]
async fn t_f017_007_lost_authorization_ack_replays_the_exact_result_without_guard_recontact() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .lose_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    assert!(
        f.broker
            .authorize_action(intent.clone(), &f.authority)
            .await
            .is_err()
    );
    f.clock.advance(Duration::from_secs(10));
    let decision = f.authorize(intent).await;
    assert_eq!(guard.evaluate_call_count(), 1);
    let attempts = journal.attempts.lock().unwrap();
    let results: Vec<_> = attempts
        .iter()
        .filter(|entry| matches!(entry, BrokerJournalEntry::Authorization { .. }))
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results.first().unwrap(), results.get(1).unwrap());
    assert_eq!(
        decision.evaluation_id,
        evaluation_of(results.first().unwrap()).evaluation_id
    );
}

#[tokio::test]
async fn t_f017_007_cached_decisions_keep_the_source_evaluation_and_pending_result() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    f.guard(guard.clone()).await.unwrap();
    let source = f.authorize(f.intent()).await;
    journal
        .lose_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let cached_intent = f.intent();
    assert!(
        f.broker
            .authorize_action(cached_intent.clone(), &f.authority)
            .await
            .is_err()
    );
    let cached = f.authorize(cached_intent).await;
    assert!(cached.from_cache);
    assert_ne!(cached.evaluation_id, source.evaluation_id);
    assert_eq!(cached.source_evaluation_id, Some(source.evaluation_id));
    let next = f.authorize(f.intent()).await;
    assert_eq!(next.source_evaluation_id, Some(source.evaluation_id));
    assert_eq!(guard.evaluate_call_count(), 1);
}

#[tokio::test]
async fn t_f017_007_uncertain_authorization_is_not_published_as_a_cache_source() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .lose_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    f.guard(guard.clone()).await.unwrap();
    assert!(
        f.broker
            .authorize_action(f.intent(), &f.authority)
            .await
            .is_err()
    );
    let other = f.authorize(f.intent()).await;
    assert!(!other.from_cache);
    assert_eq!(guard.evaluate_call_count(), 2);
}

#[tokio::test]
async fn t_f017_007_late_authorization_ack_cannot_resurrect_invalidated_cache() {
    for reconfigure in [true, false] {
        let journal = Arc::new(StrictEvaluationJournal::default());
        let f = Fixture::with_journal(Some(journal.clone()));
        let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
            f.clock.clone(),
        ));
        f.guard(guard.clone()).await.unwrap();
        journal
            .hang_authorization_ack
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let intent = f.intent();
        let future = f.broker.authorize_action(intent, &f.authority);
        tokio::pin!(future);
        tokio::select! { _ = &mut future => panic!("expected pending authorization ack"), () = async { while !journal.committed.lock().unwrap().iter().any(|entry| matches!(entry, BrokerJournalEntry::Authorization { .. })) { tokio::task::yield_now().await; } } => {} }
        let current_guard = if reconfigure {
            let replacement = Arc::new(DeterministicGuardDouble::allowing_unconditional(
                f.clock.clone(),
            ));
            replacement.set_default_outcome(ScriptedGuardOutcome::Deny {
                reason_code: "NEW_POLICY".into(),
                detail: "denied".into(),
            });
            f.guard(replacement.clone()).await.unwrap();
            replacement
        } else {
            let caller = CallerIdentity::admin(ActorId::new("admin").unwrap());
            let subject = TypedSubjectRef::Principal(ActorId::new("test-admin").unwrap());
            f.broker
                .suspend_subject(
                    &caller,
                    SuspendSubjectCommand {
                        subject: subject.clone(),
                        project_id: None,
                        expected_revision: Revision::INITIAL,
                        reason: "review".into(),
                    },
                )
                .await
                .unwrap();
            f.broker
                .resume_subject(
                    &caller,
                    ResumeSubjectCommand {
                        subject,
                        project_id: None,
                        expected_revision: Revision::new(1),
                        reason: "review complete".into(),
                    },
                )
                .await
                .unwrap();
            guard.set_default_outcome(ScriptedGuardOutcome::Deny {
                reason_code: "AFTER_RESUME".into(),
                detail: "denied".into(),
            });
            guard.clone()
        };
        let calls_before = current_guard.evaluate_call_count();
        journal.authorization_gate.notify_one();
        future.await.unwrap();
        assert!(matches!(
            f.broker.authorize_action(f.intent(), &f.authority).await,
            Err(ActionBrokerError::GuardDenied { .. })
        ));
        assert_eq!(current_guard.evaluate_call_count(), calls_before + 1);
        f.spy.assert_zero_dispatches();
        f.secrets.assert_zero_secret_resolutions();
    }
}

#[tokio::test]
async fn t_f017_010_unknown_then_reconciliation_keep_distinct_observation_records() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let intent = f.intent();
    let id = intent.action_id;
    let decision = f.authorize(intent).await;
    f.spy
        .set_next_execution_failure(SimulatedExecutionFailure::OutcomeUnknown {
            reason: "lost receipt".into(),
        });
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
    f.broker
        .reconcile_action(id, ReconcileOutcome::Unknown, None)
        .await
        .unwrap();
    let committed = journal.committed.lock().unwrap();
    let observations: Vec<_> = committed
        .iter()
        .filter(|entry| matches!(entry, BrokerJournalEntry::Observation { .. }))
        .collect();
    assert_eq!(observations.len(), 2);
    assert!(
        matches!(observations.first().unwrap(), BrokerJournalEntry::Observation { cause: ObservationCause::Execution, observation_revision, .. } if *observation_revision == Revision::new(1))
    );
    assert!(
        matches!(observations.get(1).unwrap(), BrokerJournalEntry::Observation { cause: ObservationCause::Reconciliation { evidence_ref: None }, observation_revision, .. } if *observation_revision == Revision::new(2))
    );
    assert_eq!(f.spy.dispatch_count(), 1);
}

#[tokio::test]
async fn t_f017_008_human_resolution_is_distinct_and_its_uncertain_commit_replays_exactly() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    let f = Fixture::with_journal(Some(journal.clone()));
    let profile = GuardMappingProfile::Qualified {
        profile_id: "test.approval.v1".into(),
        version: 1,
        supported_obligations: BTreeSet::new(),
        supports_remote_approval: true,
    };
    let guard = Arc::new(DeterministicGuardDouble::with_profile(
        f.clock.clone(),
        profile.clone(),
    ));
    guard.push_outcome(ScriptedGuardOutcome::RequireApproval {
        approval_scope: unleash_domain::ApprovalScopeId::from_u128(17),
        reason: "test approval".into(),
    });
    let caller = CallerIdentity::admin(ActorId::new("admin").unwrap());
    f.broker
        .configure_guard(
            &caller,
            ConfigureGuardCommand {
                endpoint: Some("unix:///test.guard".into()),
                mapping_profile: Some(profile),
                ..ConfigureGuardCommand::default()
            },
            Some(guard.clone()),
        )
        .await
        .unwrap();
    let intent = f.intent();
    let id = intent.action_id;
    let payload_digest = intent.payload_digest;
    let AuthorizationOutcome::ApprovalRequired {
        evaluation_id,
        approval_scope,
        revision,
        ..
    } = f
        .broker
        .authorize_action(intent, &f.authority)
        .await
        .unwrap()
    else {
        panic!("expected approval")
    };
    // Synthetic receipt provenance qualifies this local contract test only.
    let command = ResolveActionApprovalCommand {
        decision_receipt_id: unleash_domain::ReceiptId::from_u128(18),
        decision_command_digest: unleash_domain::Sha256Digest::from_bytes(
            b"synthetic committed approval command",
        ),
        action_id: id,
        approval_scope,
        payload_digest,
        decision: ApprovalDecision::Approve,
        expected_revision: revision,
        reason: JOURNAL_CANARY.into(),
    };
    journal
        .lose_approval_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        f.broker
            .resolve_action_approval(&caller, command.clone())
            .await
            .is_err()
    );
    let decision = f
        .broker
        .resolve_action_approval(&caller, command)
        .await
        .unwrap();
    assert_eq!(decision.evaluation_id, evaluation_id);
    let attempts = journal.attempts.lock().unwrap();
    let approvals: Vec<_> = attempts
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                BrokerJournalEntry::Authorization {
                    cause: AuthorizationCause::HumanApproval { .. },
                    ..
                }
            )
        })
        .collect();
    assert_eq!(approvals.len(), 2);
    assert_eq!(approvals.first().unwrap(), approvals.get(1).unwrap());
    assert_eq!(guard.evaluate_call_count(), 1);
}

#[tokio::test]
async fn t_f017_007_cancelled_result_commit_retains_the_known_guard_result_and_identity() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .hang_authorization_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal.clone()));
    let guard = Arc::new(DeterministicGuardDouble::allowing_unconditional(
        f.clock.clone(),
    ));
    f.guard(guard.clone()).await.unwrap();
    let intent = f.intent();
    {
        let future = f.broker.authorize_action(intent.clone(), &f.authority);
        tokio::pin!(future);
        tokio::select! {
            _ = &mut future => panic!("expected pending result commit"),
            () = async { while !journal.committed.lock().unwrap().iter().any(|entry| matches!(entry, BrokerJournalEntry::Authorization { .. })) { tokio::task::yield_now().await; } } => {},
        }
    }
    f.clock.advance(Duration::from_secs(10));
    f.authorize(intent).await;
    assert_eq!(guard.evaluate_call_count(), 1);
    let attempts = journal.attempts.lock().unwrap();
    let results: Vec<_> = attempts
        .iter()
        .filter(|entry| matches!(entry, BrokerJournalEntry::Authorization { .. }))
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results.first().unwrap(), results.get(1).unwrap());
}

#[tokio::test]
async fn t_f017_010_lost_observation_ack_replays_original_entry_without_dispatch() {
    let journal = Arc::new(StrictEvaluationJournal::default());
    journal
        .lose_observation_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let f = Fixture::with_journal(Some(journal.clone()));
    let intent = f.intent();
    let id = intent.action_id;
    let decision = f.authorize(intent).await;
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
    f.clock.advance(Duration::from_secs(10));
    let receipt = f
        .broker
        .reconcile_action(id, ReconcileOutcome::ConfirmedSuccess, None)
        .await
        .unwrap();
    assert_eq!(receipt.evaluation_id, decision.evaluation_id);
    assert_eq!(f.spy.dispatch_count(), 1);
    let attempts = journal.attempts.lock().unwrap();
    let observations: Vec<_> = attempts
        .iter()
        .filter(|entry| matches!(entry, BrokerJournalEntry::Observation { .. }))
        .collect();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations.first().unwrap(), observations.get(1).unwrap());
}

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
