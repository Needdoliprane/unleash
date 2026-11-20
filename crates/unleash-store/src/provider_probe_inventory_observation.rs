//! Attribution of the two native inventory objects to one durable dispatch.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use unleash_domain::{
    ActionId, ActionState, AuthorityGeneration, DispatchId, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    canonical_sha256, canonicalize_to_vec,
    provider_inventory::{MAX_INVENTORY_FACT_BYTES, MAX_INVENTORY_LIST_BYTES, OllamaInventory},
};

use crate::objects::{OwnedReferenceAdoption, PublishedObjectReference};
use crate::provider_probe_connect::{
    ProviderProbeConnectRecord, TransportPhase, fact_transport_context,
};
use crate::provider_probe_connect_observation::{
    CompletionEvidence, TransportCompletion, execution_receipt, validate_continuation,
    validate_native_dispatch, validate_observed_peer, validate_success_timing,
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

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone)]
pub struct AdoptedProviderInventoryObservation {
    evidence_id: uuid::Uuid,
    document: NativeFact,
    continuation: ExecutionContinuation,
    fact: PublishedObjectReference,
    list: PublishedObjectReference,
}

impl std::fmt::Debug for AdoptedProviderInventoryObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdoptedProviderInventoryObservation")
            .field("evidence_id", &self.evidence_id)
            .finish_non_exhaustive()
    }
}

/// A completion from the original owner of the inventory dispatch.
#[derive(Debug, Clone)]
pub struct CommitProviderInventoryObservation {
    /// Exact execution admission retained by the caller.
    pub continuation: ExecutionContinuation,
    /// The adopted pair required for verified success.
    pub fact: Option<AdoptedProviderInventoryObservation>,
    /// Canonical phase recording the broker receipt.
    pub request: crate::BrokerPhaseRequest,
}

pub(crate) struct InventoryObservationPermission {
    original: InventoryPhasePermission,
    continuation: ExecutionContinuation,
    fact: Option<AdoptedProviderInventoryObservation>,
}

impl InventoryObservationPermission {
    pub(crate) const fn permission(&self) -> &InventoryPhasePermission {
        &self.original
    }
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
        self.fact.object_hash()
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
    action_id: ActionId,
    fact_hash: Sha256Digest,
    list_hash: Sha256Digest,
) -> Result<NativeFact, StoreError> {
    read_native_pair(store, action_id, fact_hash, list_hash, |_, fact, _| {
        Ok(fact)
    })
}

fn read_native_pair<T>(
    store: &UnleashStore,
    action_id: ActionId,
    fact_hash: Sha256Digest,
    list_hash: Sha256Digest,
    project: impl FnOnce(&Connection, NativeFact, OllamaInventory) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
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
        project(
            conn,
            fact,
            canonical_document::<OllamaInventory>(&list_bytes)?,
        )
    });
    let released_fact = store.release_pin(&fact_pin);
    let released_list = store.release_pin(&list_pin);
    let fact = result?;
    released_fact?;
    released_list?;
    Ok(fact)
}

#[derive(PartialEq, Eq)]
pub(crate) struct InventoryProjection {
    probe: unleash_domain::ProviderProbeId,
    action: ActionId,
    dispatch: DispatchId,
    request_digest: Sha256Digest,
    evidence_id: uuid::Uuid,
    fact: Sha256Digest,
    list: Sha256Digest,
    observed_at: TimestampMs,
    profile_ref: String,
}

pub(crate) fn read_projection(
    conn: &Connection,
    profile_ref: &str,
) -> Result<Option<InventoryProjection>, StoreError> {
    let latest: Option<(String, String)> = conn
        .query_row(
            "SELECT p.probe_id,p.state FROM provider_probes p
         JOIN command_receipts c ON c.receipt_id=p.command_receipt_id
         WHERE p.phase='inventory' AND p.profile_version_ref=?1
         ORDER BY c.last_event_seq DESC,p.probe_id DESC LIMIT 1",
            [profile_ref],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((probe, state)) = latest else {
        return Ok(None);
    };
    let probe = probe.parse()?;
    if state != "ended"
        || crate::provider_probe_connect_disposition::has_closed_refusal(conn, probe)?
    {
        return Ok(None);
    }
    let mut statement = conn.prepare(
        "SELECT c.action_id,d.dispatch_id,json_extract(c.context_json,'$.request_digest'),
                f.evidence_id,f.object_hash,l.object_hash,f.observed_at_ms
         FROM provider_probe_transport_contexts c
         JOIN actions a ON a.action_id=c.action_id AND a.state='observed_success'
         JOIN action_dispatches d ON d.action_id=a.action_id AND d.transport_state='observed_success'
         JOIN broker_phase_records b ON b.action_id=a.action_id AND b.phase_key='observation:1'
         JOIN events e ON e.event_id=b.event_id AND e.event_type='ActionObserved'
             AND e.aggregate_id=a.action_id AND json_extract(e.payload_json,'$.dispatch_id')=d.dispatch_id
             AND json_extract(e.payload_json,'$.state')='observed_success'
         JOIN evidence f ON f.action_id=a.action_id AND f.kind='observation'
             AND f.producer_id=?2 AND f.schema_version=1 AND f.availability='available'
             AND f.snapshot_ref LIKE 'provider-inventory-fact:v1:%'
             AND f.object_hash=json_extract(e.payload_json,'$.output_digest')
         JOIN evidence z ON z.action_id=a.action_id AND z.kind='observation'
             AND z.producer_id=?2 AND z.schema_version=1 AND z.availability='available'
             AND z.snapshot_ref LIKE 'provider-inventory-disposition:v1:%:'||e.event_id
             AND z.object_hash=f.object_hash
         JOIN object_references r ON r.owner_kind='provider_probe' AND r.owner_id=c.probe_id
             AND r.source_field='inventory_observation' AND r.state='live' AND r.object_hash=f.object_hash
         JOIN object_references l ON l.owner_kind='provider_probe' AND l.owner_id=c.probe_id
             AND l.source_field='inventory_models' AND l.state='live'
         WHERE c.probe_id=?1 AND c.phase='inventory' AND c.profile_version_ref=?3",
    )?;
    let mut rows = statement.query(params![
        probe.to_string(),
        INVENTORY_OBSERVER_PRINCIPAL,
        profile_ref
    ])?;
    let row = rows
        .next()?
        .ok_or_else(|| unavailable("preuve du dernier inventaire indisponible"))?;
    let string = |index| -> Result<String, StoreError> { Ok(row.get(index)?) };
    let projection = InventoryProjection {
        probe,
        action: string(0)?.parse()?,
        dispatch: string(1)?.parse()?,
        request_digest: string(2)?.parse()?,
        evidence_id: string(3)?
            .parse()
            .map_err(|_| unavailable("identité de preuve inventory illisible"))?,
        fact: string(4)?.parse()?,
        list: string(5)?.parse()?,
        observed_at: TimestampMs::new(row.get(6)?),
        profile_ref: profile_ref.to_owned(),
    };
    if rows.next()?.is_some() {
        return Err(unavailable("preuve du dernier inventaire ambiguë"));
    }
    Ok(Some(projection))
}

pub(crate) fn finish_projection(
    store: &UnleashStore,
    proof: &InventoryProjection,
) -> Result<crate::provider_profile_validation::ProviderValidationSummary, StoreError> {
    use crate::provider_profile_validation::{
        LocalValidationStatus, ProviderInventoryFactRef, ProviderInventoryView,
        ProviderValidationSummary,
    };
    read_native_pair(
        store,
        proof.action,
        proof.fact,
        proof.list,
        |conn, fact, list| {
            if read_projection(conn, &proof.profile_ref)?.as_ref() != Some(proof)
                || fact.schema_version != 1
                || fact.action_id != proof.action
                || fact.dispatch_id != proof.dispatch
                || fact.request_digest != proof.request_digest
                || fact.observed_at_ms != proof.observed_at
                || fact.status_code != 200
                || fact.revocation_checked
                || !fact.peer.ip().is_loopback()
            {
                return Err(unavailable("preuve d'inventaire projetée divergente"));
            }
            Ok(ProviderValidationSummary {
                phase: unleash_protocol::ProviderTestPhase::Inventory,
                probe_id: proof.probe,
                result_ref: format!("provider-probe:{}", proof.probe),
                observed_at_ms: proof.observed_at,
                status: LocalValidationStatus::InventoryObserved,
                inventory: Some(ProviderInventoryView {
                    fact_ref: ProviderInventoryFactRef {
                        evidence_id: proof.evidence_id,
                        object_hash: proof.fact,
                    },
                    inventory_digest: proof.list,
                    models: list.models,
                }),
            })
        },
    )
    .map_err(|error| match error {
        StoreError::NotFound(_) => unavailable("objet du dernier inventaire indisponible"),
        other => other,
    })
}

fn fact_snapshot_ref(
    record: &ProviderProbeConnectRecord,
    continuation: &ExecutionContinuation,
    content_hash: Sha256Digest,
    list_digest: Sha256Digest,
) -> Result<String, StoreError> {
    let context = &record.context;
    let claim = record
        .claim
        .as_ref()
        .ok_or_else(|| unavailable("claim inventory absente"))?;
    let binding = canonical_sha256(&(
        context.probe_id,
        context.receipt_id,
        record.context_digest,
        claim.claim_id,
        claim.revision,
        continuation,
        content_hash,
        list_digest,
    ))?;
    Ok(format!("{FACT_PREFIX}{binding}"))
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
    let content_hash = input.fact.object_hash();
    let binding = fact_snapshot_ref(
        &record,
        &input.continuation,
        content_hash,
        fact.inventory_digest,
    )?;
    let evidence_id = adoption_evidence(tx, config, fact, content_hash, &binding)?;
    let owner_id = context.probe_id.to_string();
    let adopt_reference = |candidate: &PublishedObjectReference, producer_field, source_field| {
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
        )
    };
    Ok(AdoptedProviderInventoryObservation {
        evidence_id,
        document: *fact,
        continuation: input.continuation.clone(),
        fact: adopt_reference(&input.fact, "fact", "inventory_observation")?,
        list: adopt_reference(&input.list, "list", "inventory_models")?,
    })
}

fn require_adopted_pair(
    tx: &Transaction<'_>,
    authority: AuthorityGeneration,
    record: &ProviderProbeConnectRecord,
    fact: &AdoptedProviderInventoryObservation,
) -> Result<CompletionEvidence, StoreError> {
    crate::objects::tx_require_owned_reference(tx, &fact.fact, authority)?;
    crate::objects::tx_require_owned_reference(tx, &fact.list, authority)?;
    let content_hash = fact.content_hash();
    let snapshot_ref = fact_snapshot_ref(
        record,
        &fact.continuation,
        content_hash,
        fact.list.object_hash(),
    )?;
    let exact: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM evidence WHERE evidence_id=?1 AND action_id=?2
              AND kind='observation' AND snapshot_ref=?3 AND object_hash=?4 AND producer_id=?5
              AND observed_at_ms=?6 AND schema_version=1 AND availability='available')",
        params![
            fact.evidence_id.to_string(),
            record.context.action_id.to_string(),
            snapshot_ref,
            content_hash.to_string(),
            INVENTORY_OBSERVER_PRINCIPAL,
            fact.document.observed_at_ms.as_i64(),
        ],
        |row| row.get(0),
    )?;
    if !exact {
        return Err(unavailable("preuve d'inventaire absente ou remplacée"));
    }
    Ok(CompletionEvidence {
        id: fact.evidence_id,
        binding: snapshot_ref,
        hash: content_hash,
    })
}

pub(crate) fn validate_completion(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    request: &crate::BrokerPhaseRequest,
    permission: &InventoryObservationPermission,
) -> Result<TransportCompletion, StoreError> {
    let original = permission.original.inner();
    let record = fact_transport_context(tx, daemon, original, TransportPhase::Inventory)?;
    let receipt = execution_receipt(request, &record)?;
    let continuation = &permission.continuation;
    let fact = match receipt.state {
        ActionState::ObservedSuccess => {
            let fact = permission
                .fact
                .as_ref()
                .ok_or_else(|| unavailable("succès sans paire inventory adoptée"))?;
            if &fact.continuation != continuation
                || fact.document.action_id != receipt.action_id
                || fact.document.dispatch_id != receipt.dispatch_id
                || fact.document.request_digest != record.context.request_digest
                || receipt.output_digest != Some(fact.content_hash())
            {
                return Err(unavailable("preuve native d'un autre inventaire"));
            }
            validate_continuation(tx, daemon, now, &record, continuation, true)?;
            validate_native_dispatch(tx, &record, receipt.dispatch_id, original)?;
            validate_success_timing(
                tx,
                config,
                now,
                &record,
                receipt,
                fact.document.observed_at_ms,
            )?;
            Some(require_adopted_pair(
                tx,
                daemon.authority_generation,
                &record,
                fact,
            )?)
        }
        ActionState::OutcomeUnknown
            if receipt.output_digest.is_none() && permission.fact.is_none() =>
        {
            validate_continuation(tx, daemon, now, &record, continuation, false)?;
            None
        }
        _ => return Err(unavailable("résultat inventory sans disposition qualifiée")),
    };
    TransportCompletion::new(
        TransportPhase::Inventory,
        record.context,
        continuation,
        fact,
        request,
        receipt.observed_at_ms,
    )
}

impl UnleashStore {
    /// Commits the verified inventory result and its execution disposition.
    ///
    /// # Errors
    ///
    /// Refuses an incompatible observation or execution authority.
    pub fn commit_provider_probe_inventory_observation(
        &self,
        claim: &ProviderProbeInventoryClaim,
        input: CommitProviderInventoryObservation,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        if !matches!(&input.request.phase, crate::BrokerPhase::Observation { .. }) {
            return Err(unavailable("phase d'observation inventory requise"));
        }
        let original = claim.permission();
        let replay = || {
            self.lookup_transport_observation(
                original.inner(),
                TransportPhase::Inventory,
                &input.request,
            )
        };
        if let Some(committed) = replay()? {
            return Ok(committed);
        }
        if let Some(fact) = &input.fact {
            let checked = read_native_fact(
                self,
                claim.context().action_id,
                fact.content_hash(),
                fact.list.object_hash(),
            )
            .and_then(|document| {
                if document != fact.document {
                    return Err(unavailable("preuve native d'inventaire divergente"));
                }
                Ok(())
            });
            if let Err(error) = checked {
                return replay()?.ok_or(error);
            }
        }
        let permission = InventoryObservationPermission {
            original,
            continuation: input.continuation,
            fact: input.fact,
        };
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::broker_writer::commit_transport_observation_tx(
                tx,
                &config,
                now,
                &input.request,
                crate::broker_writer::TransportObservation::Inventory(&permission),
            )
        })
    }

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
        let fact = read_native_fact(
            self,
            claim.context().action_id,
            adoption.fact.object_hash(),
            adoption.list.object_hash(),
        )?;
        let permission = claim.permission();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            adopt_tx(tx, &config, now, &permission, &adoption, &fact)
        })
    }
}
