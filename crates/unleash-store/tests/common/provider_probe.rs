//! Internal synthetic `SQLite` bench; no network and no claimed runtime qualification.
#![expect(
    clippy::unwrap_used,
    reason = "Synthetic closed fixture and contract assertions"
)]
use serde_json::{Value, json};
use unleash_domain::{
    ActionOrigin, ActorId, AuthorizationEvaluationId, DataLocality, Fence, LineageFence,
    MicrosAmount, ModelPolicy, ProjectId, ProviderProbeId, RequestId, Revision, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ReceiptState, canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, BrokerCause, BrokerEvaluation, BrokerIntent, BrokerPhase, BrokerPhaseRequest,
    CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest, PROVIDER_PROBE_VERB,
    PrepareObjectRequest, PreparedProviderProbe, ProbeGenerationLimits, ProviderProbePreparation,
    PublishObjectReferenceRequest, UnleashStore,
};

pub(super) fn publish(store: &UnleashStore, document: &Value) -> Sha256Digest {
    let bytes = canonicalize_to_vec(document).unwrap();
    let hash = Sha256Digest::from_bytes(&bytes);
    let handle = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            owner_kind: "probe_fixture".to_owned(),
            owner_id: "installed-probe-fixture@1".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&handle, &bytes).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: handle.pin_id,
            object_hash: hash,
            object_generation: handle.object_generation,
            owner_kind: "probe_fixture".to_owned(),
            owner_id: "installed-probe-fixture@1".to_owned(),
            source_field: "content".to_owned(),
        })
        .unwrap();
    hash
}

pub(super) fn phase(record: &PreparedProviderProbe) -> BrokerPhaseRequest {
    let context = &record.context;
    let fence = LineageFence::new(
        context.authority_generation,
        context.daemon_epoch,
        Fence::new(0),
    );
    let manifest=json!({"action_id":context.action_id,"subject":{"kind":"control_plane","principal_id":context.principal_id,"cause_ref":format!("receipt:{}",context.command_receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},"project_id":context.project_id,"verb":PROVIDER_PROBE_VERB,"resource_refs":record.resource_refs(),"payload_digest":record.payload_digest(),"reconcile_strategy":"observe_then_decide","effects_class":"model_dispatch","scope_revision":context.scope_revision,"policy_revision":context.policy_revision,"lineage_fence":fence,"action_deadline":context.deadline,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":context.preparation.secret_refs,"provenance":{"origin":"engine_internal"}}).as_object().unwrap().clone();
    let intent = BrokerIntent {
        action_id: context.action_id,
        origin: ActionOrigin::ControlPlane {
            control_principal_id: context.principal_id.clone(),
        },
        principal_id: context.principal_id.clone(),
        cause: BrokerCause::CommandReceipt {
            receipt_id: context.command_receipt_id,
        },
        project_id: context.project_id,
        payload_digest: record.payload_digest(),
        scope_revision: context.scope_revision,
        policy_revision: context.policy_revision,
        lineage_fence: fence,
        action_deadline: context.deadline,
        reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
        intent_digest: canonical_sha256(&manifest).unwrap(),
        manifest,
    };
    BrokerPhaseRequest {
        authority_generation: context.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_uuid(context.action_id.as_uuid()),
            previous_evaluation_id: None,
        },
        phase: BrokerPhase::Authorizing { intent },
    }
}

/// Build through real receipt/preparation/Authorizing APIs for admission invariants.
pub(super) fn seed_admission_probe(
    store: &UnleashStore,
    id: ProviderProbeId,
    project: ProjectId,
    hash: Sha256Digest,
) {
    let actor = ActorId::new("test:admission-bench").unwrap();
    let preparation = ProviderProbePreparation {
        requested_profile_id: "bench-profile".to_owned(),
        requested_profile_version: 1,
        profile_version_ref: "profile".to_owned(),
        resolved_model_id: "fixture".to_owned(),
        capability_set: vec!["text".to_owned()],
        capability_snapshot_hash: hash,
        fixture_id: "synthetic-admission-bench@1".to_owned(),
        fixture_digest: hash,
        payload_manifest_hash: hash,
        model_policy_hash: hash,
        data_policy_hash: hash,
        model_policy: ModelPolicy::AllowedBounded,
        locality: DataLocality::LocalOnly,
        mandate_ref: "synthetic-admission-bench".to_owned(),
        source_run_id: None,
        secret_refs: vec![],
        limits: ProbeGenerationLimits {
            timeout_ms: 60_000,
            max_calls: 1,
            max_output_tokens: 1,
            max_cost_usd_micros: MicrosAmount::ZERO,
        },
    };
    let envelope=CommandEnvelope{protocol_version:ProtocolVersion,request_id:RequestId::from_uuid(id.as_uuid()),command_id:unleash_domain::CommandId::from_uuid(id.as_uuid()),authority_generation:store.active_authority_generation().unwrap(),actor_ref:Some(actor.to_string()),project_id:Some(project),aggregate_id:id.as_uuid(),expected_revision:Revision::INITIAL,command_type:"TestProviderProfile".to_owned(),payload:json!({"profile_id":preparation.requested_profile_id,"profile_version":preparation.requested_profile_version,"phase":"generation_probe","capability_set":preparation.capability_set,"limits":preparation.limits,"locality":preparation.locality,"mandate_ref":preparation.mandate_ref}).as_object().unwrap().clone()};
    store
        .commit_command(CommitCommandRequest {
            actor_id: actor.clone(),
            envelope,
            target_aggregate: AggregateKind::ProviderProbe,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::PrepareProviderProbe {
                probe_id: id,
                caller: CallerAuthority {
                    actor_id: actor,
                    role: CallerRole::PolicyAdmin,
                },
                preparation,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    let record = store.inspect_provider_probe(id).unwrap().unwrap();
    publish(store, &record.request_manifest().unwrap());
    store.commit_broker_phase(phase(&record)).unwrap();
}
