//! Writer-only oracle. Eligibility is attributed to an explicitly synthetic
//! system observer, not to a client declaration or a native transport result.

use super::super::TestResult;
use super::{
    PRINCIPAL, caller, claim_request, command, configure, counts, envelope, fixture, inventory,
    issue, prepare,
};
use serde_json::json;
use std::collections::BTreeMap;
use unleash_domain::{
    ActionOrigin, AuthorizationEvaluationId, Fence, LineageFence, PoolId, ReconcileStrategy,
    Revision, Sha256Digest,
};
use unleash_protocol::{ProviderTestPhase, canonical_sha256, canonicalize_to_vec};
use unleash_store::{
    AdmissionDecision, AdmissionEligibilityObservation, AdmissionOutcome, AdmissionUnit,
    AdmissionUrgency, AggregateKind, BrokerCause, BrokerEvaluation, BrokerIntent, BrokerPhase,
    BrokerPhaseRequest, CommandMutation, ExecutionPoolSettings, PrepareObjectRequest,
    ProviderProbeConnectContext, ProviderProbeInventoryClaim, ProviderProbeInventoryClaimOutcome,
    PublishObjectReferenceRequest, SettingsPatch, SettingsScopeKind, StoreError, UnleashStore,
};

const OBSERVER: &str = "fixture:inventory-admission-observer";

#[path = "provider_probe_inventory_adoption.rs"]
mod adoption;

fn initialize_pool(store: &UnleashStore) -> TestResult<PoolId> {
    let execution_pool = ExecutionPoolSettings::default();
    store.commit_command(command(
        envelope(store,0xf012_2100,store.active_authority_generation()?.as_uuid(),"SetSettings",
            json!({"scope_kind":"user","scope_id":"default","patch":{"execution_pool":execution_pool}}))?,
        AggregateKind::Settings,
        CommandMutation::SetSettings {
            scope_kind:SettingsScopeKind::User,scope_id:"default".to_owned(),caller:caller()?,
            patch:SettingsPatch {
                execution_pool:Some(execution_pool),retention_policy:None,confirm_retention_shortening:false,
                analyzer_paths:None,extra_typed:Some(serde_json::Map::new()),
            },
        },
    )?)?;
    Ok(store.global_execution_pool_id()?)
}

fn intent(claim: &ProviderProbeInventoryClaim) -> TestResult<BrokerPhaseRequest> {
    let c = claim.context();
    let destination = claim.destination();
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::ZERO);
    let manifest = json!({
        "action_id":c.action_id,"subject":{"kind":"control_plane","principal_id":c.principal_id,
            "cause_ref":format!("receipt:{}",c.receipt_id),"invocation_id":null,"attempt_id":null,
            "replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
        "project_id":c.project_id,"verb":"model.provider.inventory",
        "resource_refs":[{"kind":"network_endpoint","value":{"host":destination.host(),"port":destination.port(),
            "is_private_or_loopback":destination.is_loopback_literal()}}],
        "payload_digest":c.request_digest,"reconcile_strategy":"observe_then_decide","effects_class":"network_egress",
        "scope_revision":c.scope_revision,"policy_revision":c.policy_revision,"lineage_fence":fence,"action_deadline":c.deadline,
        "isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},
        "secret_refs":[],"provenance":{"origin":"engine_internal"}
    }).as_object().cloned().ok_or("intent manifest must be an object")?;
    Ok(BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_u128(0xf012_2102),
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
                project_id: c.project_id,
                payload_digest: c.request_digest,
                scope_revision: c.scope_revision,
                policy_revision: c.policy_revision,
                lineage_fence: fence,
                action_deadline: c.deadline,
                reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
                intent_digest: canonical_sha256(&manifest)?,
                manifest,
            },
        },
    })
}

fn publish_eligibility(
    store: &UnleashStore,
    claim: &ProviderProbeInventoryClaim,
    pool: PoolId,
) -> TestResult {
    let c = claim.context();
    assert_eq!(
        Sha256Digest::from_bytes(&store.read_verified_object_bytes(c.configuration_digest)?),
        c.configuration_digest,
    );
    let document = json!({
        "schema_version":1,"provenance":"synthetic_sqlite_admission_observer","phase":"inventory",
        "probe_id":c.probe_id,"action_id":c.action_id,"receipt_id":c.receipt_id,
        "context_digest":claim.context_digest(),"claim_id":claim.claim_id(),
        "configuration_object_hash":c.configuration_digest,"policy_digest":c.policy_digest,
        "mandate_digest":c.mandate_digest,"transport_constraint":"loopback_only",
    });
    let bytes = canonicalize_to_vec(&document)?;
    let object_hash = Sha256Digest::from_bytes(&bytes);
    let owner = store.config().id_generator.new_uuid().to_string();
    let object = store.prepare_object(PrepareObjectRequest {
        object_hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".to_owned(),
        owner_kind: "inventory_admission_fixture".to_owned(),
        owner_id: owner.clone(),
    })?;
    store.write_prepared_object_bytes(&object, &bytes)?;
    store.publish_owned_object_reference(PublishObjectReferenceRequest {
        pin_id: object.pin_id,
        object_hash,
        object_generation: object.object_generation,
        owner_kind: "inventory_admission_fixture".to_owned(),
        owner_id: owner,
        source_field: "eligibility".to_owned(),
    })?;
    let snapshot = store.execution_admission_snapshot(pool)?;
    let candidate = snapshot
        .queued
        .iter()
        .find(|candidate| candidate.unit == AdmissionUnit::Probe(c.probe_id))
        .ok_or("real queued inventory missing from V008 snapshot")?;
    store.record_admission_eligibility(AdmissionEligibilityObservation {
        evidence_id: store.config().id_generator.new_uuid(),
        observer_principal_id: OBSERVER.to_owned(),
        object_hash,
        unit: candidate.unit,
        unit_revision: candidate.revision,
        source_revision: snapshot.source_revision,
        authority_generation: snapshot.authority_generation,
        daemon_epoch: snapshot.daemon_epoch,
        valid_until: c.deadline,
        urgency: AdmissionUrgency::Ordinary,
    })?;
    Ok(())
}

fn admit(
    store: &UnleashStore,
    context: &ProviderProbeConnectContext,
    pool: PoolId,
) -> Result<AdmissionOutcome, StoreError> {
    let snapshot = store.execution_admission_snapshot(pool)?;
    let unit = AdmissionUnit::Probe(context.probe_id);
    // Proposal only: the actual writer/CAS retains the complete queue and pool.
    store.commit_execution_admission(pool, snapshot.token, move |_| {
        Ok(AdmissionDecision::Admit {
            unit,
            deficit_deltas: BTreeMap::from([(None, 0)]),
            consecutive_controls: 0,
            budgets: vec![],
        })
    })
}

fn assert_no_model_or_dispatch(store: &UnleashStore) -> TestResult {
    let baseline = counts(store)?;
    assert_eq!(
        baseline.get(4..6),
        Some([0, 0].as_slice()),
        "passive inventory has no model parent"
    );
    assert_eq!(
        baseline.get(7),
        Some(&0),
        "no invented monetary reservation"
    );
    assert_eq!(
        store.read_snapshot(|db, _| db
            .query_row("SELECT COUNT(*) FROM action_dispatches", [], |row| row
                .get::<_, i64>(0))
            .map_err(StoreError::from))?,
        0
    );
    Ok(())
}

#[test]
fn inventory_admission_requires_original_claim_and_the_real_v008_writer() -> TestResult {
    let fixture = fixture()?;
    // Real SetSettings initializes the actual global pool and scope revision.
    let pool = initialize_pool(&fixture.store)?;
    fixture
        .store
        .register_principal(OBSERVER, "system", OBSERVER)?;
    configure(&fixture.store)?;
    let mandate = issue(&fixture.store, ProviderTestPhase::Inventory)?;
    let wire = inventory(&fixture.store, 0xf012_2101, mandate)?;
    fixture
        .store
        .commit_command(prepare(&fixture.store, &wire)?)?;
    let request = claim_request(&fixture.store, wire)?;
    let ProviderProbeInventoryClaimOutcome::FreshClaim(claim) =
        fixture.store.claim_provider_probe_inventory(request)?
    else {
        return Err("real original fresh inventory claim required".into());
    };
    assert_eq!(claim.context().principal_id.as_str(), PRINCIPAL);
    assert_no_model_or_dispatch(&fixture.store)?;
    let authorizing = intent(&claim)?;
    let before = counts(&fixture.store)?;
    assert_eq!(
        before.get(3),
        Some(&0),
        "a fresh claim does not preinsert its action"
    );
    assert_eq!(
        before.get(6),
        Some(&0),
        "a fresh claim does not fabricate capacity"
    );
    assert!(matches!(
        fixture.store.commit_broker_phase(authorizing.clone()),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    assert_eq!(counts(&fixture.store)?, before);
    fixture
        .store
        .commit_provider_probe_inventory_phase(&claim, authorizing.clone())?;
    assert_eq!(
        counts(&fixture.store)?.get(3),
        Some(&1),
        "Authorizing alone owns the one real action"
    );
    assert_eq!(
        counts(&fixture.store)?.get(6),
        Some(&0),
        "Authorizing cannot itself reserve a slot"
    );
    assert!(matches!(
        fixture.store.commit_broker_phase(authorizing),
        Err(StoreError::EvidenceUnavailable(_))
    ));
    publish_eligibility(&fixture.store, &claim, pool)?;
    assert!(matches!(
        admit(&fixture.store, claim.context(), pool)?,
        AdmissionOutcome::Admitted(_)
    ));
    assert_eq!(counts(&fixture.store)?.get(6), Some(&1));
    assert_no_model_or_dispatch(&fixture.store)
}
