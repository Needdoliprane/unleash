//! Closed projection of broker phases to the durable Store writer.
//!
//! This adapter does not start an executor. Human decisions and observations
//! requiring evidence remain closed until their verifiers exist.

use crate::ipc::{DaemonRuntimeState, publish_committed_events_ordered};
use std::sync::{Arc, Weak};
use unleash_action_broker::{
    ActionBrokerError, ActionIntent, ActionProvenance, AuthorizationCause, AuthorizationDecision,
    AuthorizationOutcome, BoxFuture, BrokerJournalEntry, BrokerJournalError, BrokerJournalPort,
    ExecutorKind, ObservationCause, SubjectKind,
};
use unleash_domain::{ActionOrigin, ReceiptId, Revision};
use unleash_protocol::canonical_sha256;
use unleash_store::{
    BrokerActionReceipt, BrokerAuthorizationCause, BrokerAuthorizationDecision,
    BrokerAuthorizationFailure, BrokerAuthorizationOutcome, BrokerCause, BrokerEvaluation,
    BrokerExecutor, BrokerIntent, BrokerObservationCause, BrokerPhase, BrokerPhaseRequest,
    UnleashStore,
};

/// Broker journal backed by a durable Store port.
pub(crate) struct SqliteBrokerJournal {
    store: Arc<UnleashStore>,
    runtime: Option<Weak<DaemonRuntimeState>>,
}

impl SqliteBrokerJournal {
    /// Connect the journal to its Store port. This does not activate effects.
    #[must_use]
    pub(crate) fn new(runtime: &Arc<DaemonRuntimeState>) -> Self {
        Self {
            store: Arc::clone(&runtime.store),
            runtime: Some(Arc::downgrade(runtime)),
        }
    }

    #[cfg(test)]
    fn for_store_test(store: Arc<UnleashStore>) -> Self {
        Self {
            store,
            runtime: None,
        }
    }
}

fn intention(intent: &ActionIntent) -> Result<BrokerIntent, BrokerJournalError> {
    // Invocation and replay parents have no qualified producers yet.
    if intent.subject.kind() != SubjectKind::ControlPlane
        || !matches!(intent.provenance, ActionProvenance::EngineInternal)
        || !intent.secret_refs.is_empty()
    {
        return Err(BrokerJournalError);
    }
    let principal_id = intent.subject.principal_id().clone();
    let cause_ref = intent.subject.cause_ref();
    let cause = if let Some(id) = cause_ref.strip_prefix("receipt:") {
        let receipt_id: ReceiptId = id.parse().map_err(|_| BrokerJournalError)?;
        if format!("receipt:{receipt_id}") != cause_ref {
            return Err(BrokerJournalError);
        }
        BrokerCause::CommandReceipt { receipt_id }
    } else if let Some(id) = cause_ref.strip_prefix("event:") {
        let event_id = uuid::Uuid::parse_str(id).map_err(|_| BrokerJournalError)?;
        if format!("event:{event_id}") != cause_ref {
            return Err(BrokerJournalError);
        }
        BrokerCause::Event { event_id }
    } else {
        return Err(BrokerJournalError);
    };
    let value = serde_json::to_value(intent).map_err(|_| BrokerJournalError)?;
    let intent_digest = canonical_sha256(&value).map_err(|_| BrokerJournalError)?;
    let manifest = value.as_object().cloned().ok_or(BrokerJournalError)?;
    Ok(BrokerIntent {
        action_id: intent.action_id,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: principal_id.clone(),
        },
        principal_id,
        cause,
        project_id: intent.project_id,
        payload_digest: intent.payload_digest,
        scope_revision: intent.scope_revision,
        policy_revision: intent.policy_revision,
        lineage_fence: intent.lineage_fence,
        action_deadline: intent.action_deadline,
        reconcile_strategy: intent.reconcile_strategy,
        manifest,
        intent_digest,
    })
}

#[cfg(test)]
#[path = "broker_journal_sqlite_tests.rs"]
mod sqlite_tests;

fn decision(d: &AuthorizationDecision) -> Result<BrokerAuthorizationDecision, BrokerJournalError> {
    Ok(BrokerAuthorizationDecision {
        action_id: d.action_id,
        evaluation_id: d.evaluation_id,
        source_evaluation_id: d.source_evaluation_id,
        decision_ref: d.decision_ref.as_str().to_owned(),
        policy_revision: d.policy_revision,
        scope_revision: d.scope_revision,
        payload_digest: d.payload_digest,
        expiry: d.expiry,
        applied_obligations: d
            .applied_obligations
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(|_| BrokerJournalError)?,
        fallback_policy_ref: d.fallback_policy_ref.clone(),
        from_cache: d.from_cache,
        executor: match d.executor {
            ExecutorKind::Unleash => BrokerExecutor::Unleash,
            ExecutorKind::Guard => BrokerExecutor::Guard,
        },
    })
}

fn receipt(value: &unleash_action_broker::ActionReceipt) -> BrokerActionReceipt {
    BrokerActionReceipt {
        action_id: value.action_id,
        evaluation_id: value.evaluation_id,
        dispatch_id: value.dispatch_id,
        decision_ref: value.decision_ref.as_str().to_owned(),
        state: value.state,
        output_digest: value.output_digest,
        enforcement_refs: value.enforcement_refs.clone(),
        remote_audit_ref: value.remote_audit_ref.clone(),
        remote_audit_unavailable: value.remote_audit_unavailable,
        observed_at_ms: value.observed_at_ms,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Projection exhaustive et contrôles de corrélation des quatre phases"
)]
fn project(
    entry: &BrokerJournalEntry,
) -> Result<(BrokerEvaluation, BrokerIntent, BrokerPhase), BrokerJournalError> {
    let (evaluation, source) = match entry {
        BrokerJournalEntry::Authorizing { evaluation, intent }
        | BrokerJournalEntry::Authorization {
            evaluation, intent, ..
        }
        | BrokerJournalEntry::Dispatch {
            evaluation, intent, ..
        }
        | BrokerJournalEntry::Observation {
            evaluation, intent, ..
        } => (evaluation, intent),
    };
    let intent = intention(source)?;
    let phase = match entry {
        BrokerJournalEntry::Authorizing { .. } => BrokerPhase::Authorizing {
            intent: intent.clone(),
        },
        BrokerJournalEntry::Authorization {
            cause,
            outcome,
            retry_not_before_ms,
            ..
        } => {
            if !matches!(cause, AuthorizationCause::Evaluation) {
                return Err(BrokerJournalError);
            }
            if retry_not_before_ms.is_some()
                != matches!(outcome, Err(ActionBrokerError::GuardUnavailable { .. }))
            {
                return Err(BrokerJournalError);
            }
            let outcome = match outcome {
                Ok(AuthorizationOutcome::Authorized(d)) => {
                    if d.action_id != intent.action_id
                        || d.evaluation_id != evaluation.evaluation_id
                    {
                        return Err(BrokerJournalError);
                    }
                    BrokerAuthorizationOutcome::Authorized {
                        decision: decision(d)?,
                    }
                }
                Ok(AuthorizationOutcome::ApprovalRequired {
                    evaluation_id,
                    action_id,
                    approval_scope,
                    decision_ref,
                    revision,
                }) => {
                    if *action_id != intent.action_id || *evaluation_id != evaluation.evaluation_id
                    {
                        return Err(BrokerJournalError);
                    }
                    BrokerAuthorizationOutcome::ApprovalRequired {
                        action_id: *action_id,
                        approval_scope: *approval_scope,
                        decision_ref: decision_ref.as_str().to_owned(),
                        revision: *revision,
                    }
                }
                Err(error) => BrokerAuthorizationOutcome::Failed {
                    failure: BrokerAuthorizationFailure {
                        code: error.error_code(),
                        reason_code: error.reason_code(),
                        retry_class: error.retry_class(),
                        details_ref: None,
                        retry_not_before_ms: *retry_not_before_ms,
                    },
                },
            };
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome,
                cause: BrokerAuthorizationCause::Evaluation,
            }
        }
        BrokerJournalEntry::Dispatch {
            decision: d,
            dispatch_id,
            fence,
            ..
        } => {
            if d.action_id != intent.action_id
                || d.evaluation_id != evaluation.evaluation_id
                || *fence != intent.lineage_fence
            {
                return Err(BrokerJournalError);
            }
            BrokerPhase::Dispatch {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                decision: decision(d)?,
                dispatch_id: *dispatch_id,
                fence: *fence,
            }
        }
        BrokerJournalEntry::Observation {
            receipt,
            observation_revision,
            cause,
            reason,
            ..
        } => {
            if !matches!(cause, ObservationCause::Execution)
                || reason.is_some()
                || receipt.action_id != intent.action_id
                || receipt.evaluation_id != evaluation.evaluation_id
            {
                return Err(BrokerJournalError);
            }
            BrokerPhase::Observation {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                receipt: self::receipt(receipt),
                observation_revision: *observation_revision,
                cause: BrokerObservationCause::Execution,
                reason: None,
            }
        }
    };
    Ok((
        BrokerEvaluation {
            evaluation_id: evaluation.evaluation_id,
            previous_evaluation_id: evaluation.previous_evaluation_id,
        },
        intent,
        phase,
    ))
}

impl BrokerJournalPort for SqliteBrokerJournal {
    fn append<'a>(
        &'a self,
        entry: &'a BrokerJournalEntry,
    ) -> BoxFuture<'a, Result<(), BrokerJournalError>> {
        Box::pin(async move {
            let (evaluation, intent, phase) = project(entry)?;
            let store = Arc::clone(&self.store);
            let runtime = self.runtime.clone();
            let activity = match &runtime {
                Some(runtime) => Some(Arc::new(
                    runtime
                        .upgrade()
                        .ok_or(BrokerJournalError)?
                        .broker_activity
                        .admit()
                        .ok_or(BrokerJournalError)?,
                )),
                None => None,
            };
            // L'opération détachée poursuit aussi la publication si son appelant
            // est annulé après mise en file du writer.
            tokio::spawn(async move {
                let worker_activity = activity.clone();
                let activity_guard = activity;
                let action_id = intent.action_id;
                let recovery_store = Arc::clone(&store);
                let result = tokio::task::spawn_blocking(move || {
                    let _worker_activity = worker_activity;
                    let dispatch = matches!(phase, BrokerPhase::Dispatch { .. });
                    let expected_revision = store
                        .inspect_broker_action(intent.action_id)
                        .map_err(|_| BrokerJournalError)?
                        .map_or(Revision::INITIAL, |snapshot| snapshot.revision);
                    let request = BrokerPhaseRequest {
                        authority_generation: intent.lineage_fence.authority_generation,
                        expected_revision,
                        evaluation,
                        phase,
                    };
                    let commit = store
                        .commit_broker_phase(request)
                        .map_err(|_| BrokerJournalError)?;
                    Ok::<_, BrokerJournalError>((commit, dispatch, intent.action_id))
                })
                .await;
                let Ok(Ok((commit, dispatch, action_id))) = result else {
                    // Une erreur peut suivre un commit. La lecture ne confirme
                    // pas la phase demandée : elle permet seulement de publier
                    // un curseur effectivement engagé, puis l'erreur demeure.
                    if let Some(runtime) = runtime.and_then(|runtime| runtime.upgrade()) {
                        let recovery_activity = activity_guard.clone();
                        let snapshot = tokio::task::spawn_blocking(move || {
                            let _recovery_activity = recovery_activity;
                            recovery_store.inspect_broker_action(action_id)
                        })
                        .await;
                        if let Ok(Ok(Some(snapshot))) = snapshot
                            && snapshot.last_event_cursor.stream_id == runtime.stream_id
                        {
                            publish_committed_events_ordered(
                                &runtime,
                                snapshot.last_event_cursor.stream_id,
                                snapshot.last_event_cursor.sequence.as_u64(),
                                1,
                                &[],
                            )
                            .await;
                        }
                    }
                    return Err(BrokerJournalError);
                };
                if commit.action_id != action_id {
                    return Err(BrokerJournalError);
                }
                if let Some(runtime) = runtime {
                    let Some(runtime) = runtime.upgrade() else {
                        return Err(BrokerJournalError);
                    };
                    if commit.event_cursor.stream_id != runtime.stream_id {
                        return Err(BrokerJournalError);
                    }
                    publish_committed_events_ordered(
                        &runtime,
                        commit.event_cursor.stream_id,
                        commit.event_cursor.sequence.as_u64(),
                        1,
                        &[],
                    )
                    .await;
                }
                if dispatch && commit.replayed {
                    return Err(BrokerJournalError);
                }
                Ok(())
            })
            .await
            .map_err(|_| BrokerJournalError)?
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use unleash_action_broker::{
        ActionProvenance, ActionSubject, AuthorizationEvaluation, EffectsClass, ExternalSourceKind,
        GuardDecisionRef, IsolationRequirements, TrustLevel,
    };
    use unleash_domain::{
        ActionId, ActorId, ApprovalScopeId, AuthorityGeneration, AuthorizationEvaluationId,
        DaemonEpoch, Fence, LineageFence, ReconcileStrategy, SecretRef, TimestampMs,
    };

    pub(super) fn fixture() -> (AuthorizationEvaluation, ActionIntent) {
        let principal = ActorId::new("p-sys").unwrap();
        let receipt = ReceiptId::from_u128(10);
        let intent = ActionIntent {
            action_id: ActionId::from_u128(11),
            subject: ActionSubject::new_control_plane(principal, format!("receipt:{receipt}"))
                .unwrap(),
            project_id: None,
            verb: "http.post".into(),
            resource_refs: Vec::new(),
            payload_digest: canonical_sha256(&serde_json::json!({"key":"value"})).unwrap(),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::NetworkEgress,
            scope_revision: Revision::new(1),
            policy_revision: Revision::INITIAL,
            lineage_fence: LineageFence::new(
                AuthorityGeneration::from_uuid(uuid::Uuid::from_u128(12)),
                DaemonEpoch::INITIAL,
                Fence::ZERO,
            ),
            action_deadline: TimestampMs::new(42_000),
            isolation_requirements: IsolationRequirements::standard_unconfined(),
            secret_refs: Vec::new(),
            provenance: ActionProvenance::EngineInternal,
        };
        (
            AuthorizationEvaluation {
                evaluation_id: AuthorizationEvaluationId::from_u128(13),
                previous_evaluation_id: None,
            },
            intent,
        )
    }

    #[test]
    fn exact_manifest_and_cause_are_projected_without_inventing_authority() {
        let (evaluation, intent) = fixture();
        let (_, stored, phase) = project(&BrokerJournalEntry::Authorizing {
            evaluation,
            intent: intent.clone(),
        })
        .unwrap();
        assert_eq!(
            stored.manifest,
            serde_json::to_value(&intent)
                .unwrap()
                .as_object()
                .unwrap()
                .clone()
        );
        assert_eq!(stored.intent_digest, canonical_sha256(&intent).unwrap());
        assert_ne!(stored.intent_digest, stored.payload_digest);
        assert!(
            matches!(stored.cause, BrokerCause::CommandReceipt { receipt_id } if receipt_id == ReceiptId::from_u128(10))
        );
        assert!(matches!(phase, BrokerPhase::Authorizing { .. }));

        let (_, mut bad) = fixture();
        bad.subject =
            ActionSubject::new_control_plane(ActorId::new("p-sys").unwrap(), "receipt:bad".into())
                .unwrap();
        assert!(intention(&bad).is_err());

        let (_, mut external) = fixture();
        external.provenance = ActionProvenance::ExternalSource {
            source_kind: ExternalSourceKind::ModelProposal,
            source_label: "untrusted".into(),
            trust_level: TrustLevel::UntrustedExternal,
        };
        assert!(intention(&external).is_err());
        let (_, mut secret) = fixture();
        secret
            .secret_refs
            .push(SecretRef::new("unattested-reference").unwrap());
        assert!(intention(&secret).is_err());
    }

    #[test]
    fn failure_projection_omits_untrusted_text_and_unqualified_retry() {
        let (evaluation, intent) = fixture();
        let entry = BrokerJournalEntry::Authorization {
            evaluation: evaluation.clone(),
            cause: AuthorizationCause::Evaluation,
            intent: intent.clone(),
            retry_not_before_ms: None,
            outcome: Err(ActionBrokerError::PolicyDenied {
                reason: "SECRET-CANARY".into(),
                limiting_authorities: vec!["SECRET-CANARY".into()],
            }),
        };
        let (_, _, phase) = project(&entry).unwrap();
        let json = serde_json::to_string(&phase).unwrap();
        assert!(!json.contains("SECRET-CANARY"));
        assert!(matches!(
            phase,
            BrokerPhase::Authorization {
                outcome: BrokerAuthorizationOutcome::Failed { .. },
                ..
            }
        ));
        let unavailable = BrokerJournalEntry::Authorization {
            evaluation: evaluation.clone(),
            cause: AuthorizationCause::Evaluation,
            intent: intent.clone(),
            retry_not_before_ms: None,
            outcome: Err(ActionBrokerError::GuardUnavailable {
                reason: "SECRET-CANARY".into(),
            }),
        };
        assert!(project(&unavailable).is_err());
        let deadline = TimestampMs::new(12345);
        let with_backoff = BrokerJournalEntry::Authorization {
            evaluation,
            cause: AuthorizationCause::Evaluation,
            intent,
            retry_not_before_ms: Some(deadline),
            outcome: Err(ActionBrokerError::GuardUnavailable {
                reason: "SECRET-CANARY".into(),
            }),
        };
        let (_, _, phase) = project(&with_backoff).unwrap();
        assert!(matches!(phase, BrokerPhase::Authorization {
            outcome: BrokerAuthorizationOutcome::Failed { failure }, ..
        } if failure.retry_not_before_ms == Some(deadline)));
    }

    #[test]
    fn approval_cannot_replace_a_mismatched_evaluation() {
        let (evaluation, intent) = fixture();
        let entry = BrokerJournalEntry::Authorization {
            evaluation,
            cause: AuthorizationCause::Evaluation,
            retry_not_before_ms: None,
            outcome: Ok(AuthorizationOutcome::ApprovalRequired {
                evaluation_id: AuthorizationEvaluationId::from_u128(99),
                action_id: intent.action_id,
                approval_scope: ApprovalScopeId::from_u128(100),
                decision_ref: GuardDecisionRef::new("local-decision").unwrap(),
                revision: Revision::new(1),
            }),
            intent,
        };
        assert!(project(&entry).is_err());
    }
}
