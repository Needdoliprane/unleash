//! REQ-SEC-005 / REQ-MODEL-013: effective durable snapshot initialization.
//! Pure broker fixture only: no SQLite, network, vault or provider is exercised.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use unleash_action_broker::{
    ActionBroker, ActionBrokerError, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer,
    AuthorizationOutcome, BoxFuture, BrokerConfig, BrokerJournalEntry, BrokerJournalError,
    BrokerJournalPort, EffectsClass, GuardMode, InitialEffectivePolicy, IsolationMode,
    IsolationRequirements, LocalAuthorityFallbackPolicy, NormalizedResourceRef, ScopeKind,
    StrictNoneAcceptedObligationChecker, compute_effective_authority,
};
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence, LineageFence,
    ModelPolicy, ProjectId, ReconcileStrategy, Revision, Sha256Digest, TimestampMs,
};
use unleash_testkit::{
    DeterministicIdGenerator, ManualClock, ManualClockTimer, TestIsolationProbe,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
#[derive(Default)]
struct Journal(Mutex<Vec<&'static str>>);
impl BrokerJournalPort for Journal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            self.0
                .lock()
                .map_err(|_| BrokerJournalError)?
                .push(match entry {
                    BrokerJournalEntry::Authorizing { .. } => "authorizing",
                    BrokerJournalEntry::Authorization { .. } => "authorization",
                    BrokerJournalEntry::Dispatch { .. } => "dispatch",
                    BrokerJournalEntry::Observation { .. } => "observation",
                });
            Ok(())
        })
    }
}

fn fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(0xf017_1200),
        DaemonEpoch::INITIAL,
        Fence::ZERO,
    )
}
fn snapshot() -> InitialEffectivePolicy {
    InitialEffectivePolicy {
        project_id: None,
        policy_revision: Revision::new(7),
        scope_revision: Revision::new(2),
        policy_digest: Sha256Digest::from_bytes(b"synthetic complete durable policy fixture"),
        isolation_mode: IsolationMode::Standard,
        guard_mode: GuardMode::BrokerOnly,
        lineage_fence: fence(),
    }
}
fn construct(
    snapshot: InitialEffectivePolicy,
    journal: Arc<Journal>,
) -> Result<ActionBroker, ActionBrokerError> {
    let clock = Arc::new(ManualClock::new(10_000));
    ActionBroker::new_with_policy_snapshot(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        Arc::new(DeterministicIdGenerator::new(1200)),
        Arc::new(TestIsolationProbe::standard_available()),
        Arc::new(StrictNoneAcceptedObligationChecker),
        Arc::new(LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(fence()),
        journal,
        snapshot,
    )
}
fn authority(snapshot: &InitialEffectivePolicy) -> unleash_action_broker::EffectiveAuthority {
    compute_effective_authority(
        snapshot.project_id,
        snapshot.policy_revision,
        snapshot.scope_revision,
        &[AuthorityLayer {
            layer_name: "synthetic_effective_policy_fixture".into(),
            allowed_effects: Some(BTreeSet::from([EffectsClass::NetworkEgress])),
            denied_effects: BTreeSet::new(),
            allowed_verbs: Some(BTreeSet::from(["model.provider.connect".into()])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: Some(vec![]),
            allowed_network_hosts: Some(BTreeSet::from(["127.0.0.1".into()])),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: if snapshot.project_id.is_some() {
                ScopeKind::Project
            } else {
                ScopeKind::Global
            },
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    )
}
fn intent(snapshot: &InitialEffectivePolicy) -> Result<ActionIntent, ActionBrokerError> {
    Ok(ActionIntent {
        action_id: ActionId::from_u128(0xf017_1201),
        subject: ActionSubject::new_control_plane(
            ActorId::new("synthetic-policy-owner").map_err(|_| {
                ActionBrokerError::InvalidArgument {
                    reason: "fixture actor".into(),
                }
            })?,
            "receipt:00000000-0000-0000-0000-000000000012".into(),
        )?,
        project_id: snapshot.project_id,
        verb: "model.provider.connect".into(),
        resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
            host: "127.0.0.1".into(),
            port: Some(1234),
            is_private_or_loopback: true,
        }],
        payload_digest: Sha256Digest::from_bytes(b"synthetic connect payload"),
        reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
        effects_class: EffectsClass::NetworkEgress,
        scope_revision: snapshot.scope_revision,
        policy_revision: snapshot.policy_revision,
        lineage_fence: snapshot.lineage_fence,
        action_deadline: TimestampMs::new(20_000),
        isolation_requirements: IsolationRequirements::standard_unconfined(),
        secret_refs: vec![],
        provenance: ActionProvenance::EngineInternal,
    })
}

#[tokio::test]
async fn t_f017_012_initial_policy_snapshot_preserves_revision_without_admin_mutation() -> TestResult
{
    let snapshot = snapshot();
    let journal = Arc::new(Journal::default());
    let broker = construct(snapshot.clone(), journal.clone())?;
    let summary = broker.read_policy_summary(None);
    assert_eq!(summary.policy_revision, snapshot.policy_revision);
    assert_eq!(summary.guard_mode, GuardMode::BrokerOnly);
    assert!(
        broker.audit_events().is_empty(),
        "loading is not a PolicyUpdated mutation"
    );
    assert!(
        EffectsClass::NetworkEgress.requires_guard_evaluation(),
        "network effects require Evaluate when Guard is configured"
    );
    assert!(
        matches!(
            broker
                .authorize_action(intent(&snapshot)?, &authority(&snapshot))
                .await?,
            AuthorizationOutcome::Authorized(_)
        ),
        "BrokerOnly is explicit and still checks local authority"
    );
    assert_eq!(
        *journal.0.lock().map_err(|_| "fixture journal poisoned")?,
        vec!["authorizing", "authorization"]
    );
    Ok(())
}

#[test]
fn t_f017_012_initial_policy_snapshot_rejects_wrong_fence_and_guard_without_port() {
    let mut wrong = snapshot();
    wrong.lineage_fence.epoch = DaemonEpoch::new(2);
    assert!(matches!(
        construct(wrong, Arc::new(Journal::default())),
        Err(ActionBrokerError::StaleFence { .. })
    ));
    for guard_mode in [
        GuardMode::GuardRequired,
        GuardMode::GuardOptional {
            fallback_policy_ref: "synthetic.fallback.v1".into(),
        },
    ] {
        let mut guarded = snapshot();
        guarded.guard_mode = guard_mode;
        assert!(matches!(
            construct(guarded, Arc::new(Journal::default())),
            Err(ActionBrokerError::GuardUnavailable { .. })
        ));
    }
}

#[tokio::test]
async fn t_f017_012_initial_policy_snapshot_rejects_other_scope_and_weaker_isolation() -> TestResult
{
    let mut scoped = snapshot();
    scoped.project_id = Some(ProjectId::from_u128(0xf017_1202));
    let journal = Arc::new(Journal::default());
    let broker = construct(scoped.clone(), journal.clone())?;
    let mut foreign = intent(&scoped)?;
    foreign.project_id = None;
    assert!(
        broker
            .authorize_action(foreign, &authority(&scoped))
            .await
            .is_err()
    );
    assert!(
        journal
            .0
            .lock()
            .map_err(|_| "fixture journal poisoned")?
            .is_empty(),
        "scope must be rejected before intent publication"
    );
    let mut strict = snapshot();
    strict.isolation_mode = IsolationMode::Strict;
    let broker = construct(strict.clone(), Arc::new(Journal::default()))?;
    assert!(
        broker
            .authorize_action(intent(&strict)?, &authority(&strict))
            .await
            .is_err(),
        "a strict snapshot cannot be weakened by a standard declaration"
    );
    Ok(())
}
