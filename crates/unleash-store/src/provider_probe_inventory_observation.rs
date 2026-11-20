//! Attribution of the two native inventory objects to one durable dispatch.

use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use unleash_domain::{ActionId, DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::{
    canonical_sha256, canonicalize_to_vec,
    provider_inventory::{MAX_INVENTORY_FACT_BYTES, MAX_INVENTORY_LIST_BYTES, OllamaInventory},
};

use crate::objects::{OwnedReferenceAdoption, PublishedObjectReference};
use crate::provider_probe_connect::{TransportPhase, fact_transport_context};
use crate::provider_probe_connect_observation::{
    validate_continuation, validate_native_dispatch, validate_observed_peer,
};
use crate::provider_probe_inventory::InventoryPhasePermission;
use crate::{
    ConnectCertificateValidation, ExecutionContinuation, ProviderProbeInventoryClaim, StoreConfig,
    StoreError, UnleashStore, WriteCriticality,
};

/// Internal principal for the closed native inventory producer.
pub const INVENTORY_OBSERVER_PRINCIPAL: &str = "system:provider-probe-inventory";
/// Owner kind for the daemon's two published inventory candidates.
pub const INVENTORY_PRODUCER_KIND: &str = "provider_probe_inventory_observation_preparation";
const FACT_PREFIX: &str = "provider-inventory-fact:v1:";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeFact {
    schema_version: u32,
    action_id: ActionId,
    dispatch_id: DispatchId,
    request_digest: Sha256Digest,
    peer: std::net::SocketAddr,
    certificate_validation: ConnectCertificateValidation,
    revocation_checked: bool,
    status_code: u16,
    response_digest: Sha256Digest,
    response_body_bytes: u64,
    inventory_digest: Sha256Digest,
    observed_at_ms: TimestampMs,
}

/// Native candidates offered by the holder of the original admission.
#[derive(Debug, Clone)]
pub struct AdoptProviderInventoryObservation {
    /// Exact admission returned by the execution writer.
    pub continuation: ExecutionContinuation,
    /// Published candidate containing the native fact.
    pub fact: PublishedObjectReference,
    /// Published candidate containing the canonical, complete list.
    pub list: PublishedObjectReference,
}

/// The writer's immutable attribution of both objects to the native execution.
#[derive(Debug, Clone)]
pub struct AdoptedProviderInventoryObservation {
    evidence_id: uuid::Uuid,
    content_hash: Sha256Digest,
}

impl AdoptedProviderInventoryObservation {
    /// Durable evidence identity for the native fact.
    #[must_use]
    pub const fn evidence_id(&self) -> uuid::Uuid {
        self.evidence_id
    }

    /// Canonical digest of the native fact, which binds the list digest.
    #[must_use]
    pub const fn content_hash(&self) -> Sha256Digest {
        self.content_hash
    }
}

fn unavailable(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.to_owned())
}

fn canonical_document<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, StoreError> {
    let document = serde_json::from_slice(bytes)
        .map_err(|_| unavailable("document inventory natif invalide"))?;
    let encoded = canonicalize_to_vec(&document)
        .map_err(|_| unavailable("document inventory non canonique"))?;
    if encoded != bytes {
        return Err(unavailable("document inventory non canonique"));
    }
    Ok(document)
}

fn read_native_fact(
    store: &UnleashStore,
    input: &AdoptProviderInventoryObservation,
    action_id: ActionId,
) -> Result<NativeFact, StoreError> {
    let fact_hash = input.fact.object_hash();
    let list_hash = input.list.object_hash();
    let owner = action_id.to_string();
    let fact_pin = store.acquire_read_pin(fact_hash, "provider_inventory_fact_reader", &owner)?;
    let list_pin = match store.acquire_read_pin(list_hash, "provider_inventory_list_reader", &owner)
    {
        Ok(pin) => pin,
        Err(error) => {
            store.release_pin(&fact_pin)?;
            return Err(error);
        }
    };
    let config = store.config().clone();
    let result = store.read_snapshot(move |conn, _| {
        let fact_bytes = crate::objects::read_verified_evidence_bytes_bounded(
            conn,
            &config,
            fact_hash,
            MAX_INVENTORY_FACT_BYTES,
        )?;
        let fact: NativeFact = canonical_document(&fact_bytes)?;
        if fact.inventory_digest != list_hash {
            return Err(unavailable("liste inventory divergente du fait natif"));
        }
        let list_bytes = crate::objects::read_verified_evidence_bytes_bounded(
            conn,
            &config,
            list_hash,
            MAX_INVENTORY_LIST_BYTES,
        )?;
        canonical_document::<OllamaInventory>(&list_bytes)?;
        Ok(fact)
    });
    let released_fact = store.release_pin(&fact_pin);
    let released_list = store.release_pin(&list_pin);
    let fact = result?;
    released_fact?;
    released_list?;
    Ok(fact)
}

fn adoption_evidence(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    fact: &NativeFact,
    content_hash: Sha256Digest,
    snapshot_ref: &str,
) -> Result<uuid::Uuid, StoreError> {
    let action = fact.action_id.to_string();
    let hash = content_hash.to_string();
    let observed_at = fact.observed_at_ms.as_i64();
    let mut statement = tx.prepare(
        "SELECT evidence_id, snapshot_ref=?2 AND object_hash=?3 AND producer_id=?4
             AND observed_at_ms=?5 AND schema_version=1 AND availability='available'
         FROM evidence WHERE action_id=?1 AND kind='observation' AND snapshot_ref LIKE ?6",
    )?;
    let mut rows = statement.query(params![
        action,
        snapshot_ref,
        hash,
        INVENTORY_OBSERVER_PRINCIPAL,
        observed_at,
        format!("{FACT_PREFIX}%"),
    ])?;
    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let exact: Option<bool> = row.get(1)?;
        if exact != Some(true) || rows.next()?.is_some() {
            return Err(unavailable(
                "fait inventory déjà attribué différemment ou purgé",
            ));
        }
        return id
            .parse()
            .map_err(|_| unavailable("preuve inventory illisible"));
    }
    let id = config.id_generator.new_uuid();
    if id.is_nil() {
        return Err(unavailable("identité de preuve inventory nulle"));
    }
    tx.execute(
        "INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,
                              observed_at_ms,schema_version,availability)
         VALUES(?1,'observation',?2,?3,?4,?5,?6,1,'available')",
        params![
            id.to_string(),
            action,
            snapshot_ref,
            hash,
            INVENTORY_OBSERVER_PRINCIPAL,
            observed_at
        ],
    )?;
    Ok(id)
}

fn adopt_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    permission: &InventoryPhasePermission,
    input: &AdoptProviderInventoryObservation,
    fact: &NativeFact,
) -> Result<AdoptedProviderInventoryObservation, StoreError> {
    let original = permission.inner();
    let daemon = crate::commands::read_daemon_state_conn(tx)?;
    let record = fact_transport_context(tx, &daemon, original, TransportPhase::Inventory)?;
    let context = &record.context;
    let limits = context
        .inventory_limits
        .as_ref()
        .ok_or_else(|| unavailable("bornes inventory absentes"))?;
    limits.validate()?;
    validate_observed_peer(original, &record, fact.peer, fact.certificate_validation)?;
    if fact.schema_version != 1
        || fact.action_id != context.action_id
        || fact.request_digest != context.request_digest
        || fact.revocation_checked
        || fact.status_code != 200
        || !(1..=limits.max_response_bytes).contains(&fact.response_body_bytes)
    {
        return Err(unavailable("fait natif divergent du contexte inventory"));
    }
    // Attribution remains possible with an uncertain clock. Disposition is separate.
    validate_continuation(tx, &daemon, now, &record, &input.continuation, false)?;
    validate_native_dispatch(tx, &record, fact.dispatch_id, original)?;
    let registered: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM principals WHERE principal_id=?1 AND actor_id=?1
             AND principal_kind='system' AND state='active')",
        [INVENTORY_OBSERVER_PRINCIPAL],
        |row| row.get(0),
    )?;
    if !registered {
        return Err(unavailable("observateur natif inventory non qualifié"));
    }
    let claim = record
        .claim
        .as_ref()
        .ok_or_else(|| unavailable("claim inventory absente"))?;
    let content_hash = input.fact.object_hash();
    let binding = canonical_sha256(&(
        context.probe_id,
        context.receipt_id,
        record.context_digest,
        claim.claim_id,
        claim.revision,
        &input.continuation,
        content_hash,
        fact.inventory_digest,
    ))?;
    let evidence_id = adoption_evidence(
        tx,
        config,
        fact,
        content_hash,
        &format!("{FACT_PREFIX}{binding}"),
    )?;
    let owner_id = context.probe_id.to_string();
    for (candidate, producer_field, source_field) in [
        (&input.fact, "fact", "inventory_observation"),
        (&input.list, "list", "inventory_models"),
    ] {
        crate::objects::tx_adopt_owned_reference(
            tx,
            config.id_generator.as_ref(),
            daemon.authority_generation,
            &OwnedReferenceAdoption {
                candidate,
                object_hash: candidate.object_hash(),
                producer_kind: INVENTORY_PRODUCER_KIND,
                producer_field,
                owner_kind: "provider_probe",
                owner_id: &owner_id,
                source_field,
            },
        )?;
    }
    Ok(AdoptedProviderInventoryObservation {
        evidence_id,
        content_hash,
    })
}

impl UnleashStore {
    /// Binds both retained objects to the original inventory dispatch.
    ///
    /// # Errors
    ///
    /// Returns an error if either object or the owning execution cannot be verified.
    pub fn adopt_provider_probe_inventory_observation(
        &self,
        claim: &ProviderProbeInventoryClaim,
        adoption: AdoptProviderInventoryObservation,
    ) -> Result<AdoptedProviderInventoryObservation, StoreError> {
        let fact = read_native_fact(self, &adoption, claim.context().action_id)?;
        let permission = claim.permission();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            adopt_tx(tx, &config, now, &permission, &adoption, &fact)
        })
    }
}
