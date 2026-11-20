//! Exercises `SQLite` attribution using synthetic native bytes, without a socket.

use super::*;
use unleash_domain::DispatchId;
use unleash_store::objects::PublishedObjectReference;
use unleash_store::{
    AdoptProviderInventoryObservation, BrokerAuthorizationCause, BrokerAuthorizationDecision,
    BrokerAuthorizationOutcome, BrokerExecutor, ExecutionContinuation,
    INVENTORY_OBSERVER_PRINCIPAL,
};

const PRODUCER: &str = "provider_probe_inventory_observation_preparation";

#[path = "provider_probe_inventory_disposition.rs"]
mod disposition;

struct Dispatched {
    dispatch_id: DispatchId,
    continuation: ExecutionContinuation,
    evaluation: BrokerEvaluation,
    intent_digest: Sha256Digest,
    decision_ref: String,
}

fn prepare_claim(store: &UnleashStore) -> TestResult<ProviderProbeInventoryClaim> {
    initialize_pool(store)?;
    store.register_principal(OBSERVER, "system", OBSERVER)?;
    store.register_principal(
        INVENTORY_OBSERVER_PRINCIPAL,
        "system",
        INVENTORY_OBSERVER_PRINCIPAL,
    )?;
    configure(store)?;
    let wire = inventory(
        store,
        0xf012_2201,
        issue(store, ProviderTestPhase::Inventory)?,
    )?;
    store.commit_command(prepare(store, &wire)?)?;
    let ProviderProbeInventoryClaimOutcome::FreshClaim(claim) =
        store.claim_provider_probe_inventory(claim_request(store, wire)?)?
    else {
        return Err("real original inventory claim required".into());
    };
    Ok(claim)
}

fn prepare_candidate(
    store: &UnleashStore,
    source_field: &str,
    bytes: &[u8],
) -> TestResult<PublishedObjectReference> {
    let object_hash = Sha256Digest::from_bytes(bytes);
    let owner_id = store.config().id_generator.new_uuid().to_string();
    let object = store.prepare_object(PrepareObjectRequest {
        object_hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
        owner_kind: PRODUCER.into(),
        owner_id: owner_id.clone(),
    })?;
    store.write_prepared_object_bytes(&object, bytes)?;
    Ok(
        store.publish_owned_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash,
            object_generation: object.object_generation,
            owner_kind: PRODUCER.into(),
            owner_id,
            source_field: source_field.into(),
        })?,
    )
}

fn dispatch(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    pool: PoolId,
) -> TestResult<Dispatched> {
    let authorizing = intent(claim)?;
    store.commit_provider_probe_inventory_phase(claim, authorizing.clone())?;
    publish_eligibility(store, claim, pool)?;
    let AdmissionOutcome::Admitted(commit) = admit(store, claim.context(), pool)? else {
        return Err("actual inventory admission required".into());
    };
    let BrokerPhase::Authorizing { intent } = authorizing.phase else {
        return Err("fixture expected Authorizing".into());
    };
    let phase_request = |revision, phase| BrokerPhaseRequest {
        authority_generation: authorizing.authority_generation,
        expected_revision: Revision::new(revision),
        evaluation: authorizing.evaluation,
        phase,
    };
    let decision = BrokerAuthorizationDecision {
        action_id: intent.action_id,
        evaluation_id: authorizing.evaluation.evaluation_id,
        source_evaluation_id: None,
        decision_ref: format!("synthetic-sqlite-decision:{}", intent.action_id),
        policy_revision: intent.policy_revision,
        scope_revision: intent.scope_revision,
        payload_digest: intent.payload_digest,
        expiry: Some(intent.action_deadline),
        applied_obligations: vec![],
        fallback_policy_ref: None,
        from_cache: false,
        executor: BrokerExecutor::Unleash,
    };
    store.commit_provider_probe_inventory_phase(
        claim,
        phase_request(
            1,
            BrokerPhase::Authorization {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                outcome: BrokerAuthorizationOutcome::Authorized {
                    decision: decision.clone(),
                },
                cause: BrokerAuthorizationCause::Evaluation,
            },
        ),
    )?;
    let dispatch_id = DispatchId::from_uuid(store.config().id_generator.new_uuid());
    let decision_ref = decision.decision_ref.clone();
    store.commit_provider_probe_inventory_phase(
        claim,
        phase_request(
            2,
            BrokerPhase::Dispatch {
                action_id: intent.action_id,
                intent_digest: intent.intent_digest,
                decision,
                dispatch_id,
                fence: intent.lineage_fence,
            },
        ),
    )?;
    Ok(Dispatched {
        dispatch_id,
        evaluation: authorizing.evaluation,
        intent_digest: intent.intent_digest,
        decision_ref,
        continuation: ExecutionContinuation {
            reservation_id: commit.reservation_id,
            unit: commit.unit,
            unit_revision: commit.revision,
            lease_id: commit.lease.lease_id,
            authority_generation: commit.lease.authority_generation,
            daemon_epoch: commit.lease.daemon_epoch,
            fence: commit.lease.fence,
            boot_identity: commit.lease.boot_identity,
        },
    })
}

fn native_document(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    dispatch_id: DispatchId,
    list: &[u8],
) -> serde_json::Value {
    let list_digest = Sha256Digest::from_bytes(list);
    json!({
        "schema_version":1,"action_id":claim.context().action_id,"dispatch_id":dispatch_id,
        "request_digest":claim.context().request_digest,"peer":"127.0.0.1:11434",
        "certificate_validation":"not_tls","revocation_checked":false,"status_code":200,
        "response_digest":list_digest,"response_body_bytes":list.len(),
        "inventory_digest":list_digest,"observed_at_ms":store.config().clock.now_ms(),
    })
}

fn durable_state(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
) -> TestResult<(i64, i64, i64, String)> {
    let context = claim.context();
    Ok(store.read_snapshot(|db, _| {
        Ok(db.query_row(
            "SELECT
                (SELECT COUNT(*) FROM object_references WHERE owner_kind='provider_probe'
                    AND owner_id=?1 AND source_field IN ('inventory_observation','inventory_models')),
                (SELECT COUNT(*) FROM evidence WHERE snapshot_ref LIKE 'provider-inventory-fact:v1:%'),
                (SELECT COUNT(*) FROM capacity_reservations WHERE state='held'),
                (SELECT state FROM actions WHERE action_id=?2)",
            [context.probe_id.to_string(), context.action_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    })?)
}

#[test]
fn t_f012_inventory_adopts_one_exact_fact_list_pair_without_disposing_dispatch() -> TestResult {
    let fixture = fixture()?;
    let store = &fixture.store;
    let claim = prepare_claim(store)?;
    let execution = dispatch(store, &claim, store.global_execution_pool_id()?)?;
    let list = br#"{"models":[]}"#;
    let document = native_document(store, &claim, execution.dispatch_id, list);
    let fact = canonicalize_to_vec(&document)?;
    let input = AdoptProviderInventoryObservation {
        continuation: execution.continuation,
        fact: prepare_candidate(store, "fact", &fact)?,
        list: prepare_candidate(store, "list", list)?,
    };
    let before = (0, 0, 1, "dispatching".into());
    assert_eq!(durable_state(store, &claim)?, before);
    let different_list = AdoptProviderInventoryObservation {
        list: prepare_candidate(store, "list", br#"{"models":[] }"#)?,
        ..input.clone()
    };
    assert!(matches!(
        store.adopt_provider_probe_inventory_observation(&claim, different_list),
        Err(StoreError::EvidenceUnavailable(message)) if message == "liste inventory divergente du fait natif"
    ));
    let mut wrong_dispatch = document.clone();
    *wrong_dispatch
        .get_mut("dispatch_id")
        .ok_or("missing fixture dispatch")? = json!(DispatchId::from_uuid(
        store.config().id_generator.new_uuid()
    ));
    let wrong_dispatch = AdoptProviderInventoryObservation {
        fact: prepare_candidate(store, "fact", &canonicalize_to_vec(&wrong_dispatch)?)?,
        ..input.clone()
    };
    assert!(matches!(
        store.adopt_provider_probe_inventory_observation(&claim, wrong_dispatch),
        Err(StoreError::EvidenceUnavailable(message)) if message == "fait natif sans dispatch ou temps admissible"
    ));
    let wrong_field = AdoptProviderInventoryObservation {
        list: prepare_candidate(store, "other", list)?,
        ..input.clone()
    };
    assert!(matches!(
        store.adopt_provider_probe_inventory_observation(&claim, wrong_field),
        Err(StoreError::EvidenceUnavailable(message)) if message == "candidat d'adoption incompatible"
    ));
    assert_eq!(
        durable_state(store, &claim)?,
        before,
        "no partial evidence or ownership"
    );
    let adopted = store.adopt_provider_probe_inventory_observation(&claim, input.clone())?;
    assert_eq!(adopted.content_hash(), Sha256Digest::from_bytes(&fact));
    let after = (2, 1, 1, "dispatching".into());
    assert_eq!(durable_state(store, &claim)?, after);
    let mut different_fact = document;
    *different_fact
        .get_mut("response_digest")
        .ok_or("missing fixture response digest")? =
        json!(Sha256Digest::from_bytes(b"another native body"));
    let different_fact = AdoptProviderInventoryObservation {
        fact: prepare_candidate(store, "fact", &canonicalize_to_vec(&different_fact)?)?,
        ..input.clone()
    };
    assert!(matches!(
        store.adopt_provider_probe_inventory_observation(&claim, different_fact),
        Err(StoreError::EvidenceUnavailable(message)) if message == "fait inventory déjà attribué différemment ou purgé"
    ));
    let replay = store.adopt_provider_probe_inventory_observation(&claim, input)?;
    assert_eq!(adopted.evidence_id(), replay.evidence_id());
    assert_eq!(durable_state(store, &claim)?, after);
    assert_eq!(counts(store)?.get(4..6), Some([0, 0].as_slice()));
    Ok(())
}
