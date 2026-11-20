//! Read-only assertions over the real writer's rows and native object bytes.
//! Inventory list projection/status are deliberately not invented by this bank.

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::{Component, Path};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use unleash_client::{EventStreamReader, UnleashClient};
use unleash_domain::{RequestId, Sha256Digest};
use unleash_model_router::inventory_transport::OllamaInventory;
use unleash_protocol::{
    CommandEnvelope, CommandReceipt, ProtocolVersion, QueryEnvelope, QueryResponse,
};
use uuid::Uuid;

use super::fixture::{Capture, INVENTORY_BODY, TestResult, open_db, require};

pub(super) struct NetworkExpectation {
    pub(super) request: CommandEnvelope,
    pub(super) configured: CommandReceipt,
    pub(super) mandate: CommandReceipt,
    pub(super) profile: Uuid,
}

pub(super) struct NetworkWitness {
    action_id: String,
    dispatch_id: String,
    request_digest: String,
    reservation_id: String,
    lease_id: String,
}

const NETWORK_SQL: &str = r"
SELECT a.action_id,d.dispatch_id,json_extract(i.intent_json,'$.payload_digest'),
 r.reservation_id,l.lease_id
FROM provider_probe_transport_contexts context
JOIN provider_probes p ON p.probe_id=context.probe_id
JOIN command_receipts c ON c.receipt_id=context.receipt_id AND p.command_receipt_id=c.receipt_id
JOIN provider_profiles f ON f.profile_version_ref=context.profile_version_ref
JOIN provider_probe_mandates m ON m.mandate_ref=context.mandate_ref
JOIN principals principal ON principal.principal_id=context.principal_id
JOIN daemon_state daemon ON daemon.singleton=1
JOIN actions a ON a.action_id=context.action_id
JOIN broker_intents i ON i.action_id=a.action_id
JOIN broker_phase_records requested_phase ON requested_phase.action_id=a.action_id AND requested_phase.phase_key='authorizing'
JOIN events requested ON requested.event_id=requested_phase.event_id AND requested.event_type='ActionAuthorizationRequested'
JOIN broker_phase_records authorized_phase ON authorized_phase.action_id=a.action_id AND authorized_phase.phase_key='authorization'
JOIN events authorized ON authorized.event_id=authorized_phase.event_id AND authorized.event_type='ActionAuthorized'
JOIN action_dispatches d ON d.action_id=a.action_id
JOIN broker_phase_records dispatch_phase ON dispatch_phase.action_id=a.action_id AND dispatch_phase.phase_key='dispatch'
JOIN events dispatched ON dispatched.event_id=dispatch_phase.event_id AND dispatched.event_type='ActionDispatchCommitted'
JOIN execution_reservation_owners owner ON owner.provider_probe_id=p.probe_id
JOIN capacity_reservations r ON r.reservation_id=owner.reservation_id
JOIN capacity_pools pool ON pool.pool_id=r.pool_id
JOIN resource_leases l ON l.resource_key='provider_probe:'||p.probe_id AND l.resource_kind='execution'
WHERE p.probe_id=?1 AND p.phase='inventory' AND p.state IN ('admitted','running')
 AND context.phase='inventory' AND context.context_digest=p.context_hash
 AND context.claim_state='claimed' AND context.claim_revision=1 AND context.claim_id IS NOT NULL
 AND context.claimed_at_ms<=requested.recorded_at_ms
 AND context.authority_generation=daemon.authority_generation AND context.daemon_epoch=daemon.daemon_epoch
 AND context.authority_generation=p.authority_generation AND context.daemon_epoch=p.daemon_epoch
 AND context.project_id IS NULL AND a.project_id IS NULL
 AND c.command_type='TestProviderProfile' AND c.aggregate_id=p.probe_id AND c.command_digest=?5
 AND c.actor_id=context.actor_id AND c.authority_generation=context.authority_generation
 AND f.profile_id=?2 AND f.profile_version=1 AND f.receipt_id=?3
 AND f.profile_version_ref=p.profile_version_ref AND f.principal_id=principal.principal_id
 AND f.config_digest=m.configuration_digest
 AND m.mandate_ref=?4 AND m.phase='inventory' AND m.profile_id=f.profile_id AND m.profile_version=1
 AND m.profile_version_ref=f.profile_version_ref AND m.principal_id=principal.principal_id
 AND m.scope_kind='global' AND m.scope_id='global' AND m.authority_generation=daemon.authority_generation
 AND m.revocation_receipt_id IS NULL AND m.expires_at_ms>?6
 AND m.mandate_digest=json_extract(context.context_json,'$.mandate_digest')
 AND f.config_digest=json_extract(context.context_json,'$.configuration_digest')
 AND principal.actor_id=context.actor_id AND principal.state='active' AND principal.principal_kind='user'
 AND a.state='dispatching' AND a.origin_kind='control_plane' AND a.control_principal_id=principal.principal_id
 AND a.action_type='model.provider.inventory' AND a.fence=0
 AND a.authority_generation=context.authority_generation AND a.daemon_epoch=context.daemon_epoch
 AND json_extract(i.intent_json,'$.cause.kind')='command_receipt'
 AND json_extract(i.intent_json,'$.cause.receipt_id')=c.receipt_id
 AND json_extract(i.intent_json,'$.manifest.effects_class')='network_egress'
 AND json_array_length(json_extract(i.intent_json,'$.manifest.secret_refs'))=0
 AND json_extract(i.intent_json,'$.payload_digest')=json_extract(context.context_json,'$.request_digest')
 AND r.owner_type='provider_probe' AND r.owner_id=p.probe_id AND r.admission_class='work'
 AND r.amount=1 AND r.state='held' AND r.authority_generation=p.authority_generation AND r.daemon_epoch=p.daemon_epoch
 AND pool.pool_kind='execution' AND pool.scope_key='global' AND pool.unit='slot'
 AND l.owner_type='provider_probe' AND l.owner_id=p.probe_id AND l.state='held'
 AND l.authority_generation=p.authority_generation AND l.daemon_epoch=p.daemon_epoch
 AND l.fence=p.fence AND p.fence>0 AND l.expires_at_ms>?6 AND l.boot_identity=daemon.boot_identity
 AND json_extract(dispatched.payload_json,'$.dispatch_id')=d.dispatch_id
 AND requested.sequence<authorized.sequence AND authorized.sequence<dispatched.sequence
 AND EXISTS(SELECT 1 FROM events admitted WHERE admitted.aggregate_id=p.probe_id
  AND admitted.event_type='ProviderProbeAdmitted' AND admitted.stream_id=dispatched.stream_id
  AND requested.sequence<admitted.sequence AND admitted.sequence<authorized.sequence
  AND json_extract(admitted.payload_json,'$.reservation_id')=r.reservation_id
  AND json_extract(admitted.payload_json,'$.lease_id')=l.lease_id
  AND json_extract(admitted.payload_json,'$.fence')=p.fence)
";

pub(super) fn validate_network_query(db: &Connection) -> TestResult {
    db.prepare(NETWORK_SQL)?;
    Ok(())
}

pub(super) fn assert_at_network(
    root: &Path,
    expected: &NetworkExpectation,
) -> TestResult<NetworkWitness> {
    let db = open_db(root)?;
    let now = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let digest = unleash_protocol::compute_command_digest_v2(&expected.request)?;
    let witnesses = db
        .prepare(NETWORK_SQL)?
        .query_map(
            params![
                expected.request.aggregate_id.to_string(),
                expected.profile.to_string(),
                expected.configured.receipt_id.to_string(),
                expected.mandate.receipt_id.to_string(),
                digest.to_string(),
                now
            ],
            |row| {
                Ok(NetworkWitness {
                    action_id: row.get(0)?,
                    dispatch_id: row.get(1)?,
                    request_digest: row.get(2)?,
                    reservation_id: row.get(3)?,
                    lease_id: row.get(4)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    require(
        witnesses.len() == 1,
        "network requires exactly one authentic durable dispatch, reservation and live lease",
    )?;
    assert_no_model_or_secret_rows(&db)?;
    witnesses
        .into_iter()
        .next()
        .ok_or_else(|| "missing network witness".into())
}

pub(super) fn assert_no_model_or_secret_rows(db: &Connection) -> TestResult {
    for table in [
        "agents",
        "invocations",
        "model_calls",
        "model_call_manifests",
        "secret_references",
        "budget_reservations",
    ] {
        let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
        require(
            count == 0,
            "passive inventory must not create model, agent, secret or monetary reservations",
        )?;
    }
    Ok(())
}

pub(super) async fn wait_for_inventory_view(
    client: &mut UnleashClient,
    profile: Uuid,
    probe: Uuid,
    receipt: &CommandReceipt,
    events: &mut EventStreamReader,
) -> TestResult<QueryResponse> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = client
                .query(&QueryEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(10),
                    query_type: "GetProviderProfile".to_owned(),
                    project_id: None,
                    payload: serde_json::Map::from_iter([
                        ("profile_id".to_owned(), json!(profile)),
                        ("profile_version".to_owned(), json!(1)),
                    ]),
                })
                .await?;
            require(
                view.result.get("profile_id") == Some(&json!(profile))
                    && view.result.get("profile_version") == Some(&json!(1)),
                "inventory query must resolve the exact configured version",
            )?;
            let observations = view
                .result
                .get("observations")
                .and_then(Value::as_array)
                .ok_or("missing observation summaries")?;
            if let Some(entry) = observations
                .iter()
                .find(|entry| entry.get("phase") == Some(&json!("inventory")))
            {
                require(
                    observations.len() == 1,
                    "only the explicitly requested inventory phase may be observed",
                )?;
                require(
                    entry.get("probe_id") == Some(&json!(probe))
                        && entry.get("result_ref") == Some(&json!(receipt.result_ref)),
                    "inventory summary must link the actual probe and receipt result",
                )?;
                return Ok(view);
            }
            require(
                events.next_frame().await?.is_some(),
                "event stream closed before durable inventory observation",
            )?;
        }
    })
    .await?
}

pub(super) struct CommittedProof {
    pub(super) stream_id: String,
    pub(super) event_sequence: u64,
    action_id: String,
    dispatch_id: String,
    request_digest: String,
    output_digest: String,
    relative_path: String,
    started_at_ms: i64,
    observed_at_ms: i64,
    committed_at_ms: i64,
}

pub(super) fn assert_committed_inventory(
    root: &Path,
    request: &CommandEnvelope,
    receipt: &CommandReceipt,
    capture: &Capture,
) -> TestResult<CommittedProof> {
    let db = open_db(root)?;
    let proof = db.query_row(r"
SELECT e.stream_id,e.sequence,a.action_id,d.dispatch_id,json_extract(i.intent_json,'$.payload_digest'),
 json_extract(e.payload_json,'$.output_digest'),object.relative_path,d.started_at_ms,d.ended_at_ms,e.recorded_at_ms
FROM provider_probes p
JOIN provider_probe_transport_contexts context ON context.probe_id=p.probe_id
JOIN actions a ON a.action_id=context.action_id
JOIN broker_intents i ON i.action_id=a.action_id
JOIN action_dispatches d ON d.action_id=a.action_id
JOIN broker_phase_records b ON b.action_id=a.action_id AND b.phase_key='observation:1'
JOIN events e ON e.event_id=b.event_id AND e.event_type='ActionObserved'
JOIN object_records object ON object.object_hash=json_extract(e.payload_json,'$.output_digest') AND object.state='live'
JOIN execution_reservation_owners owner ON owner.provider_probe_id=p.probe_id
JOIN capacity_reservations r ON r.reservation_id=owner.reservation_id
JOIN resource_leases l ON l.lease_id=?3
WHERE p.probe_id=?1 AND p.command_receipt_id=?2 AND p.phase='inventory' AND p.state='ended'
 AND context.phase='inventory' AND a.state='observed_success'
 AND r.reservation_id=?4 AND r.state='released' AND r.owner_type='provider_probe' AND r.owner_id=p.probe_id
 AND l.state='released' AND l.owner_type='provider_probe' AND l.owner_id=p.probe_id AND l.fence=p.fence
 AND json_extract(i.intent_json,'$.cause.receipt_id')=p.command_receipt_id
 AND json_extract(e.payload_json,'$.dispatch_id')=d.dispatch_id
 AND EXISTS(SELECT 1 FROM evidence fact WHERE fact.action_id=a.action_id AND fact.kind='observation' AND fact.availability='available' AND fact.object_hash=object.object_hash)
 AND EXISTS(SELECT 1 FROM object_references ref WHERE ref.owner_kind='provider_probe' AND ref.owner_id=p.probe_id AND ref.object_hash=object.object_hash AND ref.state='live')
", params![request.aggregate_id.to_string(),receipt.receipt_id.to_string(),capture.witness.lease_id,capture.witness.reservation_id], |row| Ok(CommittedProof { stream_id:row.get(0)?,event_sequence:row.get(1)?,action_id:row.get(2)?,dispatch_id:row.get(3)?,request_digest:row.get(4)?,output_digest:row.get(5)?,relative_path:row.get(6)?,started_at_ms:row.get(7)?,observed_at_ms:row.get(8)?,committed_at_ms:row.get(9)? }))?;
    require(
        proof.action_id == capture.witness.action_id
            && proof.dispatch_id == capture.witness.dispatch_id
            && proof.request_digest == capture.witness.request_digest,
        "inventory completion must use the same native dispatch witnessed before HTTP",
    )?;
    require(
        proof.started_at_ms <= proof.observed_at_ms
            && proof.observed_at_ms <= proof.committed_at_ms,
        "inventory timestamps must preserve dispatch/observation/commit order",
    )?;
    assert_native_document(root, &proof)?;
    let counts = db.query_row("SELECT (SELECT COUNT(*) FROM action_dispatches WHERE action_id=?1),(SELECT COUNT(*) FROM broker_phase_records WHERE action_id=?1 AND phase_key='observation:1')", [&proof.action_id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?)))?;
    require(
        counts == (1, 1),
        "inventory requires exactly one dispatch and one causal final observation",
    )?;
    assert_no_model_or_secret_rows(&db)?;
    Ok(proof)
}

fn assert_native_document(root: &Path, proof: &CommittedProof) -> TestResult {
    let relative = Path::new(&proof.relative_path);
    require(
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "native inventory object path must stay within artifacts",
    )?;
    let bytes = std::fs::read(root.join("artifacts").join(relative))?;
    let document: Value = serde_json::from_slice(&bytes)?;
    require(
        unleash_protocol::canonicalize_to_vec(&document)? == bytes,
        "native inventory document must be canonical",
    )?;
    require(
        unleash_protocol::canonical_sha256(&document)?.to_string() == proof.output_digest,
        "ActionObserved must bind the actual durable native object",
    )?;
    require(
        document.get("action_id") == Some(&json!(proof.action_id))
            && document.get("dispatch_id") == Some(&json!(proof.dispatch_id))
            && document.get("request_digest") == Some(&json!(proof.request_digest)),
        "native inventory fact must correlate the actual request and dispatch",
    )?;
    require(
        document.get("response_digest") == Some(&json!(Sha256Digest::from_bytes(INVENTORY_BODY))),
        "native inventory fact must bind the exact response bytes",
    )?;
    let expected: OllamaInventory = serde_json::from_slice(INVENTORY_BODY)?;
    require(
        document.get("inventory_digest")
            == Some(&json!(unleash_protocol::canonical_sha256(&expected)?)),
        "native inventory fact must bind the typed list, with missing metadata preserved",
    )?;
    require(
        document.get("response_body_bytes") == Some(&json!(INVENTORY_BODY.len())),
        "native inventory byte count must be observed",
    )?;
    let observed = document
        .get("observed_at_ms")
        .and_then(Value::as_i64)
        .ok_or("native observation time missing")?;
    require(
        proof.started_at_ms <= observed && observed <= proof.observed_at_ms,
        "native observation must occur between dispatch and broker receipt",
    )?;
    // Public list/status projection is intentionally left to the contract
    // arbitration: the existing summary has neither field. No new format is
    // introduced here; the keys above belong to the delivered native transport.
    Ok(())
}
