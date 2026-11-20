//! The real journal, capacity and lease are observed around the writer transaction.

use super::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use unleash_domain::ActionState;
use unleash_store::{
    AdoptedProviderInventoryObservation, BrokerActionReceipt, BrokerObservationCause,
    CommitProviderInventoryObservation,
};

fn pair(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    execution: &Dispatched,
) -> TestResult<AdoptedProviderInventoryObservation> {
    let list = br#"{"models":[]}"#;
    let fact = canonicalize_to_vec(&native_document(store, claim, execution.dispatch_id, list))?;
    Ok(store.adopt_provider_probe_inventory_observation(
        claim,
        AdoptProviderInventoryObservation {
            continuation: execution.continuation.clone(),
            fact: prepare_candidate(store, "fact", &fact)?,
            list: prepare_candidate(store, "list", list)?,
        },
    )?)
}

fn observation(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    execution: &Dispatched,
    state: ActionState,
    fact: Option<AdoptedProviderInventoryObservation>,
) -> CommitProviderInventoryObservation {
    let context = claim.context();
    let output_digest = fact
        .as_ref()
        .map(AdoptedProviderInventoryObservation::content_hash);
    CommitProviderInventoryObservation {
        continuation: execution.continuation.clone(),
        fact,
        request: BrokerPhaseRequest {
            authority_generation: context.authority_generation,
            expected_revision: Revision::new(3),
            evaluation: execution.evaluation,
            phase: BrokerPhase::Observation {
                action_id: context.action_id,
                intent_digest: execution.intent_digest,
                receipt: BrokerActionReceipt {
                    action_id: context.action_id,
                    evaluation_id: execution.evaluation.evaluation_id,
                    dispatch_id: execution.dispatch_id,
                    decision_ref: execution.decision_ref.clone(),
                    state,
                    output_digest,
                    enforcement_refs: vec![],
                    remote_audit_ref: None,
                    remote_audit_unavailable: true,
                    observed_at_ms: store.config().clock.now_ms(),
                },
                observation_revision: Revision::new(1),
                cause: BrokerObservationCause::Execution,
                reason: None,
            },
        },
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LedgerState {
    probe_state: String,
    lease_state: String,
    reservation_state: String,
    event_count: i64,
    outbox_count: i64,
    disposition_count: i64,
}

fn state(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    execution: &Dispatched,
) -> TestResult<LedgerState> {
    let context = claim.context();
    Ok(store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT
                (SELECT state FROM provider_probes WHERE probe_id=?1),
                (SELECT state FROM resource_leases WHERE lease_id=?2),
                (SELECT state FROM capacity_reservations WHERE reservation_id=?3),
                (SELECT COUNT(*) FROM broker_phase_records b JOIN events e ON e.event_id=b.event_id
                     WHERE b.action_id=?4 AND e.event_type='ActionObserved'),
                (SELECT COUNT(*) FROM outbox o JOIN events e USING(event_id)
                     WHERE e.aggregate_id=?4 AND e.event_type='ActionObserved'),
                (SELECT COUNT(*) FROM evidence WHERE action_id=?4
                     AND snapshot_ref LIKE 'provider-inventory-disposition:v1:%')",
            [
                context.probe_id.to_string(),
                execution.continuation.lease_id.to_string(),
                execution.continuation.reservation_id.to_string(),
                context.action_id.to_string(),
            ],
            |row| {
                Ok(LedgerState {
                    probe_state: row.get(0)?,
                    lease_state: row.get(1)?,
                    reservation_state: row.get(2)?,
                    event_count: row.get(3)?,
                    outbox_count: row.get(4)?,
                    disposition_count: row.get(5)?,
                })
            },
        )?)
    })?)
}

fn list_object_path(store: &UnleashStore) -> TestResult<PathBuf> {
    let hash = Sha256Digest::from_bytes(br#"{"models":[]}"#);
    let relative: String = store.read_snapshot(|conn, _| {
        Ok(conn.query_row(
            "SELECT relative_path FROM object_records WHERE object_hash=?1",
            [hash.to_string()],
            |row| row.get(0),
        )?)
    })?;
    Ok(store.config().artifacts_dir.join(relative))
}

#[test]
fn t_f012_inventory_commits_result_and_release_in_one_transaction() -> TestResult {
    let fixture = fixture()?;
    let store = &fixture.store;
    let claim = prepare_claim(store)?;
    let execution = dispatch(store, &claim, store.global_execution_pool_id()?)?;
    let fact = pair(store, &claim, &execution)?;
    let input = observation(
        store,
        &claim,
        &execution,
        ActionState::ObservedSuccess,
        Some(fact),
    );
    let before = state(store, &claim, &execution)?;
    assert_eq!(before.lease_state, "held");
    assert_eq!(before.reservation_state, "held");
    assert_eq!(
        (
            before.event_count,
            before.outbox_count,
            before.disposition_count
        ),
        (0, 0, 0)
    );
    assert!(matches!(
        store.commit_broker_phase(input.request.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let mut no_fact = input.clone();
    no_fact.fact = None;
    assert!(matches!(
        store.commit_provider_probe_inventory_observation(&claim, no_fact),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let mut wrong_continuation = input.clone();
    wrong_continuation.continuation.fence = Fence::ZERO;
    assert!(matches!(
        store.commit_provider_probe_inventory_observation(&claim, wrong_continuation),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    let list_path = list_object_path(store)?;
    let hidden_path = list_path.with_extension("missing");
    std::fs::rename(&list_path, &hidden_path)?;
    assert!(matches!(
        store.commit_provider_probe_inventory_observation(&claim, input.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(state(store, &claim, &execution)?, before);
    std::fs::rename(&hidden_path, &list_path)?;
    fixture.failure.0.store(true, Ordering::SeqCst);
    match store.commit_provider_probe_inventory_observation(&claim, input.clone()) {
        Err(StoreError::CutpointInjected { .. }) => {}
        Err(error) => return Err(error.into()),
        Ok(_) => return Err("C1 must roll back the entire disposition".into()),
    }
    assert_eq!(state(store, &claim, &execution)?, before);
    assert_eq!(
        durable_state(store, &claim)?,
        (2, 1, 1, "dispatching".into())
    );

    let committed = store.commit_provider_probe_inventory_observation(&claim, input.clone())?;
    assert_eq!(
        state(store, &claim, &execution)?,
        LedgerState {
            probe_state: "ended".into(),
            lease_state: "released".into(),
            reservation_state: "released".into(),
            event_count: 1,
            outbox_count: 1,
            disposition_count: 1,
        }
    );
    assert_eq!(
        durable_state(store, &claim)?,
        (2, 1, 0, "observed_success".into())
    );
    // The fixture removes only its own bytes after durable completion.
    std::fs::remove_file(list_path)?;
    let replay = store.commit_provider_probe_inventory_observation(&claim, input.clone())?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    assert!(matches!(
        store.commit_broker_phase(input.request),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    Ok(())
}

#[test]
fn t_f012_inventory_unknown_keeps_the_exact_lease_and_blocks_late_success() -> TestResult {
    let fixture = fixture()?;
    let store = &fixture.store;
    let claim = prepare_claim(store)?;
    let execution = dispatch(store, &claim, store.global_execution_pool_id()?)?;
    let late_fact = pair(store, &claim, &execution)?;
    let unknown = observation(store, &claim, &execution, ActionState::OutcomeUnknown, None);
    let committed = store.commit_provider_probe_inventory_observation(&claim, unknown.clone())?;
    let expected = LedgerState {
        probe_state: "reconciling".into(),
        lease_state: "held".into(),
        reservation_state: "held".into(),
        event_count: 1,
        outbox_count: 1,
        disposition_count: 0,
    };
    assert_eq!(state(store, &claim, &execution)?, expected);
    let replay = store.commit_provider_probe_inventory_observation(&claim, unknown)?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, committed.event_cursor);
    let late = observation(
        store,
        &claim,
        &execution,
        ActionState::ObservedSuccess,
        Some(late_fact),
    );
    assert!(
        store
            .commit_provider_probe_inventory_observation(&claim, late)
            .is_err()
    );
    assert_eq!(state(store, &claim, &execution)?, expected);
    assert_eq!(
        durable_state(store, &claim)?,
        (2, 1, 1, "outcome_unknown".into())
    );
    Ok(())
}
