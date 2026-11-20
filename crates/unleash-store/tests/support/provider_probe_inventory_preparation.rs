//! Real configuration/mandate receipts; no action or model parent is fabricated.

use super::{TestResult, fixture};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use unleash_domain::{
    ActorId, AuthorityGeneration, CommandId, DaemonEpoch, DataLocality, Fence, LineageFence,
    MicrosAmount, ProviderProbeId, ReceiptId, RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, ProtocolVersion, ProviderTestLimits, ProviderTestPhase, ReceiptState,
    canonical_sha256, canonicalize_to_vec,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, ClaimProviderProbeConnectRequest,
    ClaimProviderProbeInventoryRequest, CommandMutation, CommitCommandRequest, PolicyIsolationMode,
    PolicyScopeKind, PrepareObjectRequest, ProviderProbeInventoryClaimOutcome,
    ProviderProbeMandateChange, ProviderProbeMandateIssue, ProviderProfileConfiguration,
    ProviderProfilePreparation, PublishObjectReferenceRequest, StoreError, UnleashStore,
};
use uuid::Uuid;

const PROFILE: Uuid = Uuid::from_u128(0xf012_2001);
const PRINCIPAL: &str = "inventory-principal";

fn caller() -> TestResult<CallerAuthority> {
    Ok(CallerAuthority {
        actor_id: ActorId::new("user:inventory-store")?,
        role: CallerRole::HumanOperator,
    })
}

fn envelope(
    store: &UnleashStore,
    id: u128,
    aggregate: Uuid,
    kind: &str,
    payload: Value,
) -> TestResult<CommandEnvelope> {
    let Value::Object(payload) = payload else {
        return Err("fixture wire must be an object".into());
    };
    Ok(CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(id),
        command_id: CommandId::from_u128(id),
        authority_generation: store.active_authority_generation()?,
        actor_ref: Some(caller()?.actor_id.to_string()),
        project_id: None,
        aggregate_id: aggregate,
        expected_revision: Revision::INITIAL,
        command_type: kind.to_owned(),
        payload,
    })
}

fn command(
    envelope: CommandEnvelope,
    target_aggregate: AggregateKind,
    mutation: CommandMutation,
) -> TestResult<CommitCommandRequest> {
    Ok(CommitCommandRequest {
        actor_id: caller()?.actor_id,
        envelope,
        target_aggregate,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

fn configure(store: &UnleashStore) -> TestResult {
    let configuration = ProviderProfileConfiguration {
        schema_version: 1,
        profile_id: PROFILE,
        profile_version: 1,
        project_id: None,
        payload: serde_json::from_value(json!({
            "provider_kind":"ollama", "dialect":{"kind":"ollama_chat","version":1},
            "endpoint":"http://127.0.0.1:11434/", "model":"synthetic-inventory-store",
            "locality":"local_only", "server_management":"user_managed", "auth":{"kind":"none"},
            "limits":{"max_inflight":1,"connect_timeout_ms":250,"queue_timeout_ms":1000,
                "load_timeout_ms":180000,"first_token_timeout_ms":60000,"generation_timeout_ms":60000},
            "declared_capabilities":["text"], "pricing":{"kind":"none_local"}
        }))?,
        secret_binding: None,
        catalog_binding: None,
    };
    let bytes = canonicalize_to_vec(&configuration)?;
    let hash = Sha256Digest::from_bytes(&bytes);
    let prepared = store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".to_owned(),
        owner_kind: "inventory_store_fixture".to_owned(),
        owner_id: PROFILE.to_string(),
    })?;
    store.write_prepared_object_bytes(&prepared, &bytes)?;
    store.publish_owned_object_reference(PublishObjectReferenceRequest {
        pin_id: prepared.pin_id,
        object_hash: hash,
        object_generation: prepared.object_generation,
        owner_kind: "inventory_store_fixture".to_owned(),
        owner_id: PROFILE.to_string(),
        source_field: "configuration".to_owned(),
    })?;
    store.commit_command(command(
        envelope(
            store,
            0xf012_2001,
            PROFILE,
            "SetProviderProfile",
            serde_json::to_value(&configuration.payload)?,
        )?,
        AggregateKind::ProviderProfile,
        CommandMutation::SetProviderProfile {
            caller: caller()?,
            preparation: Box::new(ProviderProfilePreparation {
                configuration,
                content_object_hash: hash,
            }),
        },
    )?)?;
    Ok(())
}

fn issue(store: &UnleashStore, phase: ProviderTestPhase) -> TestResult<ReceiptId> {
    let change = ProviderProbeMandateChange::Issue(ProviderProbeMandateIssue {
        principal_id: ActorId::new(PRINCIPAL)?,
        profile_id: PROFILE,
        profile_version: 1,
        phase,
        locality: DataLocality::LocalOnly,
        capability_set: vec![],
        limits: ProviderTestLimits::new(2000, 0, 0, MicrosAmount::ZERO),
        expires_at_ms: TimestampMs::new(61000),
    });
    let wire = envelope(
        store,
        0xf012_2002,
        Uuid::from_u128(0xf012_2002),
        "SetPolicy",
        json!({
            "scope_kind":"global","scope_id":"global","isolation_mode":"standard",
            "policy_version":"inventory-fixture-v1","reason":"synthetic passive inventory permission",
            "provider_probe_mandate":change
        }),
    )?;
    Ok(store
        .commit_command(command(
            wire,
            AggregateKind::SecurityPolicy,
            CommandMutation::SetPolicyWithProviderProbeMandate {
                scope_kind: PolicyScopeKind::Global,
                scope_id: "global".to_owned(),
                isolation_mode: PolicyIsolationMode::Standard,
                policy_version: "inventory-fixture-v1".to_owned(),
                reason: "synthetic passive inventory permission".to_owned(),
                caller: caller()?,
                mandate: change,
            },
        )?)?
        .receipt()
        .receipt_id)
}

fn inventory(store: &UnleashStore, id: u128, mandate: ReceiptId) -> TestResult<CommandEnvelope> {
    envelope(
        store,
        id,
        Uuid::from_u128(id),
        "TestProviderProfile",
        json!({
            "profile_id":PROFILE,"profile_version":1,"phase":"inventory","capability_set":[],
            "limits":{"timeout_ms":1000,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0},
            "locality":"local_only","mandate_ref":mandate
        }),
    )
}

fn prepare(store: &UnleashStore, wire: &CommandEnvelope) -> TestResult<CommitCommandRequest> {
    let preparation = store.prepare_provider_probe_inventory(wire, &caller()?)?;
    command(
        wire.clone(),
        AggregateKind::ProviderProbe,
        CommandMutation::PrepareProviderProbeInventory {
            caller: caller()?,
            preparation: Box::new(preparation),
        },
    )
}

fn counts(store: &UnleashStore) -> TestResult<Vec<i64>> {
    Ok(store.read_snapshot(|db, _| {
        [
            "command_receipts",
            "provider_probes",
            "provider_probe_transport_contexts",
            "actions",
            "model_calls",
            "model_call_manifests",
            "capacity_reservations",
            "budget_reservations",
            "events",
            "outbox",
        ]
        .into_iter()
        .map(|table| {
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .map_err(StoreError::from)
        })
        .collect()
    })?)
}

fn claim_request(
    store: &UnleashStore,
    wire: CommandEnvelope,
) -> TestResult<ClaimProviderProbeInventoryRequest> {
    let record = store
        .inspect_provider_probe_inventory_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))?
        .value
        .ok_or("missing inventory context")?;
    Ok(ClaimProviderProbeInventoryRequest {
        envelope: wire,
        caller: caller()?,
        receipt_id: record.context().receipt_id,
        context_digest: record.context_digest(),
        control_fence: LineageFence::new(
            record.context().authority_generation,
            record.context().daemon_epoch,
            Fence::ZERO,
        ),
    })
}

#[test]
fn inventory_preparation_pins_two_bounds_and_replays_its_receipt() -> TestResult {
    let fixture = fixture()?;
    configure(&fixture.store)?;
    let mandate = issue(&fixture.store, ProviderTestPhase::Inventory)?;
    let wire = inventory(&fixture.store, 0xf012_2010, mandate)?;
    let before = counts(&fixture.store)?;
    let request = prepare(&fixture.store, &wire)?;
    assert_eq!(
        counts(&fixture.store)?,
        before,
        "resolution alone is read-only"
    );
    let committed = fixture.store.commit_command(request.clone())?;
    let record = fixture
        .store
        .inspect_provider_probe_inventory_snapshot(ProviderProbeId::from_uuid(wire.aggregate_id))?
        .value
        .ok_or("missing inventory context")?;
    let context = record.context();
    assert_eq!(context.receipt_id, committed.receipt().receipt_id);
    assert_eq!(context.phase, ProviderTestPhase::Inventory);
    assert_eq!(context.actor_id, caller()?.actor_id);
    assert_eq!(context.principal_id.as_str(), PRINCIPAL);
    assert_eq!(context.mandate_ref, mandate);
    let limits = context
        .inventory_limits
        .as_ref()
        .ok_or("inventory bounds absent")?;
    assert_eq!(
        (
            limits.connect_timeout_ms,
            limits.timeout_ms,
            limits.max_response_bytes
        ),
        (250, 1000, 1048576)
    );
    assert_eq!(context.deadline, TimestampMs::new(2000));
    assert_eq!(
        context.request_digest,
        canonical_sha256(&json!({
            "schema":"unleash.ollama_inventory.v2","method":"GET","endpoint":"http://127.0.0.1:11434/api/tags",
            "profile_id":PROFILE,"profile_version":1,"profile_digest":context.configuration_digest,
            "connect_timeout_ms":250,"timeout_ms":1000,"max_response_bytes":1048576,
            "request_body_bytes":0,"redirects":false,"retries":0
        }))?
    );
    let after = counts(&fixture.store)?;
    assert_eq!(
        &after[3..8],
        &[0, 0, 0, 0, 0],
        "no action, model or reservation before typed Authorizing"
    );
    assert_eq!(
        fixture.store.commit_command(request.clone())?.receipt(),
        committed.receipt()
    );
    assert_eq!(counts(&fixture.store)?, after);
    let mut divergent = request;
    divergent.envelope.payload.insert(
        "limits".to_owned(),
        json!({"timeout_ms":999,"max_calls":0,"max_output_tokens":0,"max_cost_usd_micros":0}),
    );
    assert!(matches!(
        fixture.store.commit_command(divergent),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    assert!(
        fixture
            .store
            .inspect_provider_probe_connect_snapshot(context.probe_id)
            .is_err()
    );
    assert_eq!(counts(&fixture.store)?, after);
    Ok(())
}

#[test]
fn inventory_claim_requires_its_original_actor_receipt_phase_and_fence() -> TestResult {
    let fixture = fixture()?;
    fixture
        .store
        .register_principal("inventory-other", "user", "user:inventory-other")?;
    configure(&fixture.store)?;
    let mandate = issue(&fixture.store, ProviderTestPhase::Inventory)?;
    let wire = inventory(&fixture.store, 0xf012_2020, mandate)?;
    fixture
        .store
        .commit_command(prepare(&fixture.store, &wire)?)?;
    let request = claim_request(&fixture.store, wire)?;
    let before = counts(&fixture.store)?;
    let mut other = request.clone();
    other.caller.actor_id = ActorId::new("user:inventory-other")?;
    other.envelope.actor_ref = Some(other.caller.actor_id.to_string());
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(other),
        Err(StoreError::NotFound(_))
    ));
    let mut wrong_receipt = request.clone();
    wrong_receipt.receipt_id = mandate;
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(wrong_receipt),
        Err(StoreError::NotFound(_))
    ));
    let mut wrong_digest = request.clone();
    wrong_digest.context_digest = Sha256Digest::from_bytes(b"foreign inventory context");
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(wrong_digest),
        Err(StoreError::StaleFence { .. })
    ));
    let mut wrong_epoch = request.clone();
    wrong_epoch.control_fence.epoch =
        DaemonEpoch::new(wrong_epoch.control_fence.epoch.as_u64() + 1);
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(wrong_epoch),
        Err(StoreError::StaleFence { .. })
    ));
    let mut wrong_fence = request.clone();
    wrong_fence.control_fence.fence = Fence::new(1);
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(wrong_fence),
        Err(StoreError::StaleFence { .. })
    ));
    let mut wrong_lineage = request.clone();
    wrong_lineage.envelope.authority_generation =
        AuthorityGeneration::from_uuid(Uuid::from_u128(0xf012_dead));
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(wrong_lineage),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
    let connect_request = ClaimProviderProbeConnectRequest {
        envelope: request.envelope.clone(),
        caller: request.caller.clone(),
        receipt_id: request.receipt_id,
        context_digest: request.context_digest,
        control_fence: request.control_fence,
    };
    assert!(
        fixture
            .store
            .claim_provider_probe_connect(connect_request)
            .is_err()
    );
    assert_eq!(counts(&fixture.store)?, before);
    let ProviderProbeInventoryClaimOutcome::FreshClaim(first) = fixture
        .store
        .claim_provider_probe_inventory(request.clone())?
    else {
        return Err("first acknowledged inventory claim must own execution".into());
    };
    let ProviderProbeInventoryClaimOutcome::AlreadyClaimed(second) =
        fixture.store.claim_provider_probe_inventory(request)?
    else {
        return Err("inventory replay must remain inspection".into());
    };
    assert_eq!(
        second.claim().ok_or("claim missing")?.claim_id,
        first.claim_id()
    );
    assert_eq!(second.context_digest(), first.context_digest());
    assert_eq!(counts(&fixture.store)?, before);
    Ok(())
}

#[test]
fn inventory_prepare_and_claim_roll_back_without_any_partial_owner() -> TestResult {
    let fixture = fixture()?;
    configure(&fixture.store)?;
    let mandate = issue(&fixture.store, ProviderTestPhase::Inventory)?;
    let wire = inventory(&fixture.store, 0xf012_2030, mandate)?;
    let request = prepare(&fixture.store, &wire)?;
    let before = counts(&fixture.store)?;
    fixture.failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture.store.commit_command(request.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    assert_eq!(counts(&fixture.store)?, before);
    assert!(
        fixture
            .store
            .inspect_provider_probe_inventory_snapshot(ProviderProbeId::from_uuid(
                wire.aggregate_id
            ))?
            .value
            .is_none()
    );
    fixture.store.commit_command(request)?;
    let claim = claim_request(&fixture.store, wire)?;
    let prepared = counts(&fixture.store)?;
    fixture.failure.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(claim.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    let record = fixture
        .store
        .inspect_provider_probe_inventory_snapshot(ProviderProbeId::from_uuid(
            claim.envelope.aggregate_id,
        ))?
        .value
        .ok_or("context lost after rollback")?;
    assert!(
        record.claim().is_none(),
        "rollback must retain an unclaimed row"
    );
    assert_eq!(counts(&fixture.store)?, prepared);
    assert!(matches!(
        fixture.store.claim_provider_probe_inventory(claim)?,
        ProviderProbeInventoryClaimOutcome::FreshClaim(_)
    ));
    Ok(())
}

#[test]
fn inventory_preparation_refuses_other_phase_version_and_declared_actor() -> TestResult {
    let fixture = fixture()?;
    configure(&fixture.store)?;
    let mandate = issue(&fixture.store, ProviderTestPhase::Connect)?;
    let wire = inventory(&fixture.store, 0xf012_2040, mandate)?;
    let before = counts(&fixture.store)?;
    assert!(matches!(
        fixture
            .store
            .prepare_provider_probe_inventory(&wire, &caller()?),
        Err(StoreError::NotFound(_))
    ));
    let mut wrong_version = wire.clone();
    wrong_version
        .payload
        .insert("profile_version".to_owned(), json!(2));
    assert!(matches!(
        fixture
            .store
            .prepare_provider_probe_inventory(&wrong_version, &caller()?),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(counts(&fixture.store)?, before);

    let valid = super::fixture()?;
    configure(&valid.store)?;
    let mandate = issue(&valid.store, ProviderTestPhase::Inventory)?;
    let wire = inventory(&valid.store, 0xf012_2041, mandate)?;
    let request = prepare(&valid.store, &wire)?;
    let before = counts(&valid.store)?;
    let mut wrong_request_actor = request.clone();
    wrong_request_actor.actor_id = ActorId::new("user:inventory-other")?;
    assert!(matches!(
        valid.store.commit_command(wrong_request_actor),
        Err(StoreError::InvalidArgument { .. })
    ));
    let mut wrong_wire_actor = request;
    wrong_wire_actor.envelope.actor_ref = Some("user:inventory-other".to_owned());
    assert!(matches!(
        valid.store.commit_command(wrong_wire_actor),
        Err(StoreError::InvalidArgument { .. })
    ));
    assert_eq!(counts(&valid.store)?, before);
    Ok(())
}
