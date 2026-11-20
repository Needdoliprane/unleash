//! Native connection facts and the separate, verified execution disposition.
//! Neither a published object nor a broker return alone authorizes release.

use crate::objects::{OwnedReferenceAdoption, PublishedObjectReference};
use crate::provider_probe_connect::{
    ConnectPhasePermission, ProviderProbeConnectClaim, ProviderProbeConnectRecord, fact_context,
};
use crate::{ExecutionContinuation, StoreError, UnleashStore, WriteCriticality};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, ActorId, DispatchId, LineageFence, Sha256Digest, TimestampMs,
};
use unleash_protocol::canonical_sha256;

/// The actual internal native producer, never selected by an IPC payload.
pub const CONNECT_OBSERVER_PRINCIPAL: &str = "system:provider-probe-connect";
const PRODUCER_KIND: &str = "provider_probe_connect_observation_preparation";
const FACT_FIELD: &str = "connect_observation";
const MAX_NATIVE_BYTES: usize = 16_384;

/// Certificate checks actually reported by the closed native transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectCertificateValidation {
    /// The endpoint uses plain TCP.
    NotTls,
    /// The transport verified chain and name using locally loaded system roots.
    OfflineSystemRoots,
}

/// Closed native document. Only the trusted transport calls its publication port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConnectObservation {
    /// Document version.
    pub schema_version: u32,
    /// Actual action.
    pub action_id: ActionId,
    /// Actual durable dispatch.
    pub dispatch_id: DispatchId,
    /// Frozen native request identity, not a provider receipt hash.
    pub request_digest: Sha256Digest,
    /// Pinned socket address actually connected.
    pub peer: std::net::SocketAddr,
    /// Actual certificate validation mode.
    pub certificate_validation: ConnectCertificateValidation,
    /// This transport performs no online revocation lookup.
    pub revocation_checked: bool,
    /// This connect operation sends no application HTTP bytes.
    pub http_application_bytes_sent: u64,
    /// Native observation time after the socket is closed.
    pub observed_at_ms: TimestampMs,
}

/// Internal producer input, with its original admission and exact candidate handle.
#[derive(Debug, Clone)]
pub struct AdoptProviderConnectObservation {
    /// Real admission result retained by the orchestrator.
    pub continuation: ExecutionContinuation,
    /// Closed document from the native transport.
    pub document: ProviderConnectObservation,
    /// Canonical digest of that exact document.
    pub content_hash: Sha256Digest,
    /// This producer's opaque, published candidate reference.
    pub candidate: PublishedObjectReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FactBinding {
    schema_version: u32,
    probe_id: unleash_domain::ProviderProbeId,
    receipt_id: unleash_domain::ReceiptId,
    actor_id: ActorId,
    principal_id: ActorId,
    context_digest: Sha256Digest,
    claim_id: uuid::Uuid,
    claim_revision: unleash_domain::Revision,
    control_fence: LineageFence,
    continuation: ExecutionContinuation,
    document: ProviderConnectObservation,
    content_hash: Sha256Digest,
}

/// Adopted native fact. It does not attest completion or release on its own.
/// No constructor or deserializer can turn client data into this capability.
#[derive(Debug, Clone)]
pub struct AdoptedProviderConnectObservation {
    binding: FactBinding,
    evidence_id: uuid::Uuid,
    reference: PublishedObjectReference,
}

/// Exact execution observation, separate from adoption of the native fact.
#[derive(Debug, Clone)]
pub struct CommitProviderConnectObservation {
    /// The real admission retained by the original orchestrator.
    pub continuation: ExecutionContinuation,
    /// Required for success; an unknown outcome never releases capacity.
    pub fact: Option<AdoptedProviderConnectObservation>,
    /// Canonical broker entry projected only after its executor returned.
    pub request: crate::BrokerPhaseRequest,
}

pub(crate) struct ConnectObservationPermission {
    original: ConnectPhasePermission,
    continuation: ExecutionContinuation,
    fact: Option<AdoptedProviderConnectObservation>,
}

impl ConnectObservationPermission {
    pub(crate) fn permission(&self) -> &ConnectPhasePermission {
        &self.original
    }
}

pub(crate) struct ConnectCompletion {
    record: ProviderProbeConnectRecord,
    continuation: ExecutionContinuation,
    fact: Option<AdoptedProviderConnectObservation>,
    phase_digest: Sha256Digest,
    observed_at: TimestampMs,
}

impl AdoptedProviderConnectObservation {
    /// Actual native document retained after adoption.
    #[must_use]
    pub const fn document(&self) -> &ProviderConnectObservation {
        &self.binding.document
    }
    /// Digest of the retained document.
    #[must_use]
    pub const fn content_hash(&self) -> Sha256Digest {
        self.binding.content_hash
    }
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.to_owned())
}

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("borne native hors plage SQLite"))
}

fn binding(
    record: &ProviderProbeConnectRecord,
    input: &AdoptProviderConnectObservation,
) -> Result<FactBinding, StoreError> {
    let context = &record.context;
    let claim = record
        .claim
        .as_ref()
        .ok_or_else(|| invalid("fait sans claim"))?;
    Ok(FactBinding {
        schema_version: 1,
        probe_id: context.probe_id,
        receipt_id: context.receipt_id,
        actor_id: context.actor_id.clone(),
        principal_id: context.principal_id.clone(),
        context_digest: record.context_digest,
        claim_id: claim.claim_id,
        claim_revision: claim.revision,
        control_fence: LineageFence::new(
            context.authority_generation,
            context.daemon_epoch,
            unleash_domain::Fence::ZERO,
        ),
        continuation: input.continuation.clone(),
        document: input.document.clone(),
        content_hash: input.content_hash,
    })
}

fn snapshot_ref(binding: &FactBinding) -> Result<String, StoreError> {
    Ok(format!(
        "provider-connect-fact:v1:{}",
        canonical_sha256(binding)?
    ))
}

fn validate_document(
    permission: &ConnectPhasePermission,
    record: &ProviderProbeConnectRecord,
    input: &AdoptProviderConnectObservation,
) -> Result<(), StoreError> {
    let context = &record.context;
    let document = &input.document;
    let destination = permission.destination();
    let address_matches = destination.host() == "localhost"
        || destination.host().parse::<std::net::IpAddr>().ok() == Some(document.peer.ip());
    let certificate_matches = match destination.scheme() {
        "http" => document.certificate_validation == ConnectCertificateValidation::NotTls,
        "https" => {
            document.certificate_validation == ConnectCertificateValidation::OfflineSystemRoots
        }
        _ => false,
    };
    if document.schema_version != 1
        || document.action_id != context.action_id
        || document.request_digest != context.request_digest
        || !document.peer.ip().is_loopback()
        || !address_matches
        || document.peer.port() != destination.port()
        || !certificate_matches
        || destination.normalized_endpoint() != context.destination
        || document.revocation_checked
        || document.http_application_bytes_sent != 0
        || canonical_sha256(document)? != input.content_hash
    {
        return Err(invalid("fait natif divergent du contexte"));
    }
    Ok(())
}

pub(crate) fn validate_continuation(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    record: &ProviderProbeConnectRecord,
    continuation: &ExecutionContinuation,
    require_live_ttl: bool,
) -> Result<(), StoreError> {
    let c = &record.context;
    if continuation.unit != crate::AdmissionUnit::Probe(c.probe_id)
        || continuation.authority_generation != daemon.authority_generation
        || continuation.daemon_epoch != daemon.daemon_epoch
        || continuation.fence.as_u64() == 0
        || continuation.boot_identity != daemon.boot_identity
    {
        return Err(invalid("continuation native d'une autre incarnation"));
    }
    let valid:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM capacity_reservations r JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id JOIN capacity_pools pool ON pool.pool_id=r.pool_id JOIN provider_probes p ON p.probe_id=o.provider_probe_id JOIN resource_leases l ON l.lease_id=?2 WHERE r.reservation_id=?1 AND o.provider_probe_id=?3 AND r.owner_type='provider_probe' AND r.owner_id=p.probe_id AND r.amount=1 AND r.state IN ('held','reconciling') AND pool.pool_kind='execution' AND pool.scope_key='global' AND r.authority_generation=?4 AND r.daemon_epoch=?5 AND l.resource_kind='execution' AND l.resource_key='provider_probe:'||p.probe_id AND l.owner_type='provider_probe' AND l.owner_id=p.probe_id AND l.authority_generation=?4 AND l.daemon_epoch=?5 AND l.fence=?6 AND l.boot_identity=?7 AND l.state='held' AND (?8=0 OR l.expires_at_ms>?9) AND p.fence=?6 AND p.authority_generation=?4 AND p.daemon_epoch=?5 AND p.revision=?10 AND p.state IN ('admitted','running'))",
        params![continuation.reservation_id.to_string(),continuation.lease_id.to_string(),c.probe_id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,integer(continuation.fence.as_u64())?,daemon.boot_identity,require_live_ttl,now.as_i64(),integer(continuation.unit_revision.as_u64())?],|row|row.get(0),
    )?;
    if !valid {
        return Err(invalid("réservation ou bail natif non exact"));
    }
    Ok(())
}

fn validate_native_dispatch(
    tx: &Transaction<'_>,
    record: &ProviderProbeConnectRecord,
    document: &ProviderConnectObservation,
    permission: &ConnectPhasePermission,
) -> Result<(), StoreError> {
    let c = &record.context;
    let valid:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM action_dispatches d JOIN broker_dispatches b ON b.dispatch_id=d.dispatch_id JOIN actions a ON a.action_id=d.action_id JOIN broker_intents i ON i.action_id=a.action_id WHERE d.dispatch_id=?1 AND d.action_id=?2 AND a.state IN ('dispatching','in_flight') AND a.authority_generation=?3 AND a.daemon_epoch=?4 AND a.fence=0 AND b.authority_generation=?3 AND b.daemon_epoch=?4 AND b.fence=0 AND json_extract(i.intent_json,'$.payload_digest')=?5)",
        params![document.dispatch_id.to_string(),c.action_id.to_string(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,c.request_digest.to_string()],|row|row.get(0),
    )?;
    if !valid {
        return Err(invalid("fait natif sans dispatch ou temps admissible"));
    }
    let encoded: String = tx.query_row(
        "SELECT intent_json FROM broker_intents WHERE action_id=?1",
        [c.action_id.to_string()],
        |row| row.get(0),
    )?;
    let intent: crate::BrokerIntent = serde_json::from_str(&encoded)?;
    crate::provider_probe_connect::validate_fact_intent(permission, &intent)?;
    crate::broker_writer::validate_recorded_dispatch_binding_tx(tx, &intent, document.dispatch_id)?;
    Ok(())
}

fn verify_document_bytes(
    store: &UnleashStore,
    document: &ProviderConnectObservation,
    hash: Sha256Digest,
) -> Result<(), StoreError> {
    let pin = store.acquire_read_pin(
        hash,
        "provider_connect_fact_reader",
        &document.action_id.to_string(),
    )?;
    let config = store.config().clone();
    let result = store.read_snapshot(move |conn, _| {
        crate::objects::read_verified_evidence_bytes_bounded(conn, &config, hash, MAX_NATIVE_BYTES)
    });
    let released = store.release_pin(&pin);
    let bytes = result?;
    released?;
    let actual: ProviderConnectObservation = serde_json::from_slice(&bytes)?;
    if &actual != document || canonical_sha256(&actual)? != hash {
        return Err(invalid("octets natifs indisponibles ou divergents"));
    }
    Ok(())
}

struct ExistingFactEvidence {
    id: String,
    binding: String,
    hash: String,
    producer: String,
    observed: i64,
    availability: String,
    version: i64,
}

fn evidence_identity(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    record: &ProviderProbeConnectRecord,
    input: &AdoptProviderConnectObservation,
    reference: &str,
) -> Result<(uuid::Uuid, bool), StoreError> {
    let mut statement=tx.prepare("SELECT evidence_id,snapshot_ref,object_hash,producer_id,observed_at_ms,availability,schema_version FROM evidence WHERE action_id=?1 AND kind='observation' AND snapshot_ref LIKE 'provider-connect-fact:v1:%'")?;
    let rows = statement
        .query_map([record.context.action_id.to_string()], |row| {
            Ok(ExistingFactEvidence {
                id: row.get(0)?,
                binding: row.get(1)?,
                hash: row.get(2)?,
                producer: row.get(3)?,
                observed: row.get(4)?,
                availability: row.get(5)?,
                version: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    match rows.as_slice() {
        [] => {
            let id = config.id_generator.new_uuid();
            if id.is_nil() {
                return Err(invalid("identité de preuve native nulle"));
            }
            Ok((id, true))
        }
        [row]
            if row.binding == reference
                && row.hash == input.content_hash.to_string()
                && row.producer == CONNECT_OBSERVER_PRINCIPAL
                && row.observed == input.document.observed_at_ms.as_i64()
                && row.availability == "available"
                && row.version == 1 =>
        {
            Ok((
                row.id
                    .parse()
                    .map_err(|_| invalid("preuve native illisible"))?,
                false,
            ))
        }
        _ => Err(invalid("fait natif déjà attribué différemment ou purgé")),
    }
}

fn adopt_tx(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    now: TimestampMs,
    permission: &ConnectPhasePermission,
    input: &AdoptProviderConnectObservation,
) -> Result<AdoptedProviderConnectObservation, StoreError> {
    let daemon = crate::commands::read_daemon_state_conn(tx)?;
    let record = fact_context(tx, &daemon, permission)?;
    validate_document(permission, &record, input)?;
    // Attribution is retained even when UTC is uncertain. Completion separately
    // requires an admissible chronology and a live lease before any release.
    validate_continuation(tx, &daemon, now, &record, &input.continuation, false)?;
    validate_native_dispatch(tx, &record, &input.document, permission)?;
    let producer:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM principals WHERE principal_id=?1 AND actor_id=?1 AND principal_kind='system' AND state='active')",[CONNECT_OBSERVER_PRINCIPAL],|row|row.get(0))?;
    if !producer {
        return Err(invalid("observateur natif non qualifié"));
    }
    let binding = binding(&record, input)?;
    let reference = snapshot_ref(&binding)?;
    let (evidence_id, insert) = evidence_identity(tx, config, &record, input, &reference)?;
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeProviderProbeConnectFactAdoption)?;
    let permanent = crate::objects::tx_adopt_owned_reference(
        tx,
        config.id_generator.as_ref(),
        daemon.authority_generation,
        &OwnedReferenceAdoption {
            candidate: &input.candidate,
            object_hash: input.content_hash,
            producer_kind: PRODUCER_KIND,
            producer_field: "observation",
            owner_kind: "provider_probe",
            owner_id: &record.context.probe_id.to_string(),
            source_field: FACT_FIELD,
        },
    )?;
    if insert {
        tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,?4,?5,?6,1,'available')",params![evidence_id.to_string(),record.context.action_id.to_string(),reference,input.content_hash.to_string(),CONNECT_OBSERVER_PRINCIPAL,input.document.observed_at_ms.as_i64()])?;
    }
    Ok(AdoptedProviderConnectObservation {
        binding,
        evidence_id,
        reference: permanent,
    })
}

impl UnleashStore {
    /// Revalidate an admission retained by the original fresh owner.
    /// This validates a supplied continuation; it never remints ownership from a read.
    /// # Errors
    /// Rejects stale context, policy, incarnation or execution lease.
    pub fn validate_provider_probe_connect_continuation(
        &self,
        claim: &ProviderProbeConnectClaim,
        continuation: ExecutionContinuation,
    ) -> Result<(), StoreError> {
        let permission = claim.permission();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = crate::commands::read_daemon_state_conn(tx)?;
            let record = crate::provider_probe_connect::validate_live_permission(
                tx,
                &daemon,
                now,
                &permission,
            )?;
            validate_continuation(tx, &daemon, now, &record, &continuation, true)
        })
    }
    /// Adopt a native closed-socket observation without completing its execution.
    /// # Errors
    /// Rejects missing original ownership, stale fences, divergent objects or dispatch.
    pub fn adopt_provider_probe_connect_observation(
        &self,
        claim: &ProviderProbeConnectClaim,
        input: AdoptProviderConnectObservation,
    ) -> Result<AdoptedProviderConnectObservation, StoreError> {
        verify_document_bytes(self, &input.document, input.content_hash)?;
        let permission = claim.permission();
        let config = self.config().clone();
        let adopted = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            adopt_tx(tx, &config, now, &permission, &input)
        })?;
        self.config()
            .cutpoints
            .hit(crate::StoreCutpoint::C2AfterProviderProbeConnectFactAdoption)?;
        Ok(adopted)
    }

    /// Commit a verified completion and its exact execution disposition atomically.
    /// # Errors
    /// Success requires the adopted native fact, exact phase and live continuation.
    /// Unknown keeps the reservation and cannot be promoted by late fact delivery.
    pub fn commit_provider_probe_connect_observation(
        &self,
        claim: &ProviderProbeConnectClaim,
        input: CommitProviderConnectObservation,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        if !matches!(&input.request.phase, crate::BrokerPhase::Observation { .. }) {
            return Err(invalid("phase d'observation connect requise"));
        }
        let original = claim.permission();
        if let Some(replay) = self.lookup_connect_observation(&original, &input.request)? {
            return Ok(replay);
        }
        if let Some(fact) = &input.fact {
            if let Err(error) =
                verify_document_bytes(self, &fact.binding.document, fact.binding.content_hash)
            {
                if let Some(replay) = self.lookup_connect_observation(&original, &input.request)? {
                    return Ok(replay);
                }
                return Err(error);
            }
        }
        let permission = ConnectObservationPermission {
            original,
            continuation: input.continuation,
            fact: input.fact,
        };
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::broker_writer::commit_connect_observation_tx(
                tx,
                &config,
                now,
                &input.request,
                &permission,
            )
        })
    }

    fn lookup_connect_observation(
        &self,
        permission: &ConnectPhasePermission,
        request: &crate::BrokerPhaseRequest,
    ) -> Result<Option<crate::BrokerPhaseCommit>, StoreError> {
        self.read_snapshot(|conn, _| {
            let daemon = crate::commands::read_daemon_state_conn(conn)?;
            if request.authority_generation != daemon.authority_generation {
                return Err(StoreError::StaleAuthorityGeneration {
                    expected: daemon.authority_generation,
                    actual: request.authority_generation,
                    command_id: None,
                });
            }
            let record = fact_context(conn, &daemon, permission)?;
            if request.phase.action_id() != record.context.action_id {
                return Err(invalid("rejeu d'observation d'une autre action"));
            }
            crate::broker_writer::lookup_phase_record(conn, &daemon, request)
        })
    }
}

pub(crate) fn validate_completion(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    request: &crate::BrokerPhaseRequest,
    permission: &ConnectObservationPermission,
) -> Result<ConnectCompletion, StoreError> {
    let record = fact_context(tx, daemon, &permission.original)?;
    let crate::BrokerPhase::Observation {
        receipt,
        cause,
        reason,
        ..
    } = &request.phase
    else {
        return Err(invalid("observation d'exécution connect absente"));
    };
    if receipt.action_id != record.context.action_id
        || reason.is_some()
        || *cause != crate::BrokerObservationCause::Execution
        || receipt.remote_audit_ref.is_some()
        || !receipt.enforcement_refs.is_empty()
    {
        return Err(invalid(
            "phase native d'une autre cause ou d'un autre exécuteur",
        ));
    }
    let fact = match receipt.state {
        ActionState::ObservedSuccess => {
            let fact = permission
                .fact
                .as_ref()
                .ok_or_else(|| invalid("succès sans preuve native adoptée"))?;
            if fact.binding.continuation != permission.continuation {
                return Err(invalid("preuve native d'une autre admission"));
            }
            validate_continuation(tx, daemon, now, &record, &permission.continuation, true)?;
            validate_native_dispatch(tx, &record, &fact.binding.document, &permission.original)?;
            validate_success_fact(tx, config, daemon, now, &record, fact, receipt)?;
            Some(fact.clone())
        }
        ActionState::OutcomeUnknown if receipt.output_digest.is_none() => {
            validate_continuation(tx, daemon, now, &record, &permission.continuation, false)?;
            None
        }
        _ => return Err(invalid("résultat natif sans disposition qualifiée")),
    };
    let exact = serde_json::json!({"schema_version":crate::broker_phases::BROKER_PHASE_SCHEMA_VERSION,"evaluation":request.evaluation,"phase":request.phase});
    Ok(ConnectCompletion {
        record,
        continuation: permission.continuation.clone(),
        fact,
        phase_digest: canonical_sha256(&exact)?,
        observed_at: receipt.observed_at_ms,
    })
}

fn validate_success_fact(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    record: &ProviderProbeConnectRecord,
    fact: &AdoptedProviderConnectObservation,
    receipt: &crate::BrokerActionReceipt,
) -> Result<(), StoreError> {
    let binding = &fact.binding;
    let started: i64 = tx.query_row(
        "SELECT started_at_ms FROM action_dispatches WHERE action_id=?1 AND dispatch_id=?2",
        params![
            receipt.action_id.to_string(),
            receipt.dispatch_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if config
        .clock_uncertain
        .load(std::sync::atomic::Ordering::Acquire)
        || binding.context_digest != record.context_digest
        || binding.probe_id != record.context.probe_id
        || binding.document.action_id != receipt.action_id
        || binding.document.dispatch_id != receipt.dispatch_id
        || receipt.output_digest != Some(binding.content_hash)
        || binding.document.observed_at_ms.as_i64() < started
        || receipt.observed_at_ms < binding.document.observed_at_ms
        || receipt.observed_at_ms > now
        || receipt.observed_at_ms > record.context.deadline
    {
        return Err(invalid(
            "fin native non bornée ou preuve d'un autre faisceau",
        ));
    }
    crate::objects::tx_require_owned_reference(tx, &fact.reference, daemon.authority_generation)?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM evidence WHERE evidence_id=?1 AND action_id=?2 AND kind='observation' AND snapshot_ref=?3 AND object_hash=?4 AND producer_id=?5 AND observed_at_ms=?6 AND schema_version=1 AND availability='available')",params![fact.evidence_id.to_string(),receipt.action_id.to_string(),snapshot_ref(binding)?,binding.content_hash.to_string(),CONNECT_OBSERVER_PRINCIPAL,binding.document.observed_at_ms.as_i64()],|row|row.get(0))?;
    if !exact {
        return Err(invalid("preuve native absente ou remplacée"));
    }
    Ok(())
}

pub(crate) fn finish_completion(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    now: TimestampMs,
    completion: &ConnectCompletion,
    event_id: &str,
) -> Result<(), StoreError> {
    let context = &completion.record.context;
    let continuation = &completion.continuation;
    if let Some(fact) = &completion.fact {
        let disposition = canonical_sha256(&serde_json::json!({
            "schema":"unleash.provider_connect_disposition.v1","native_evidence_id":fact.evidence_id,
            "native_binding":snapshot_ref(&fact.binding)?,"phase_digest":completion.phase_digest,
            "action_observed_event_id":event_id,"continuation":continuation,
        }))?;
        tx.execute("INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,observed_at_ms,schema_version,availability) VALUES(?1,'observation',?2,?3,?4,?5,?6,1,'available')",params![config.id_generator.new_uuid().to_string(),context.action_id.to_string(),format!("provider-connect-disposition:v1:{disposition}:{event_id}"),fact.binding.content_hash.to_string(),CONNECT_OBSERVER_PRINCIPAL,completion.observed_at.as_i64()])?;
        transition_probe(tx, context, continuation, "ended")?;
        crate::leases::tx_release_lease(tx, continuation.lease_id)?;
        crate::budgets::tx_release_capacity_reservation_verified(
            tx,
            now,
            continuation.reservation_id,
        )?;
    } else {
        // An uncertain outcome is recorded but never treated as physical stop.
        transition_probe(tx, context, continuation, "reconciling")?;
    }
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeProviderProbeConnectObservationCommit)?;
    Ok(())
}

pub(crate) fn transition_probe(
    tx: &Transaction<'_>,
    context: &crate::ProviderProbeConnectContext,
    continuation: &ExecutionContinuation,
    state: &str,
) -> Result<(), StoreError> {
    let changed=tx.execute("UPDATE provider_probes SET state=?2,wait_reason=NULL,revision=revision+1 WHERE probe_id=?1 AND revision=?3 AND authority_generation=?4 AND daemon_epoch=?5 AND fence=?6 AND state IN ('admitted','running')",params![context.probe_id.to_string(),state,integer(continuation.unit_revision.as_u64())?,context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,integer(continuation.fence.as_u64())?])?;
    if changed != 1 {
        return Err(invalid("unité native modifiée avant disposition"));
    }
    Ok(())
}

pub(crate) struct ConnectProjection {
    probe: unleash_domain::ProviderProbeId,
    action: ActionId,
    dispatch: DispatchId,
    digest: Sha256Digest,
    request_digest: Sha256Digest,
    observed_at: TimestampMs,
}

pub(crate) fn read_projection(
    conn: &rusqlite::Connection,
    profile_ref: &str,
) -> Result<Option<ConnectProjection>, StoreError> {
    use rusqlite::OptionalExtension;
    let latest:Option<(String,String)>=conn.query_row(
        "SELECT p.probe_id,p.state FROM provider_probes p JOIN command_receipts c ON c.receipt_id=p.command_receipt_id WHERE p.phase='connect' AND p.profile_version_ref=?1 ORDER BY c.last_event_seq DESC,p.probe_id DESC LIMIT 1",
        [profile_ref],|r|Ok((r.get(0)?,r.get(1)?)),
    ).optional()?;
    let Some((probe, state)) = latest else {
        return Ok(None);
    };
    // An unknown/newer result is never replaced by an older success.
    if state != "ended" {
        return Ok(None);
    }
    if crate::provider_probe_connect_disposition::has_closed_refusal(conn, probe.parse()?)? {
        return Ok(None);
    }
    let proof=conn.query_row(
        "SELECT c.action_id,d.dispatch_id,e.object_hash,json_extract(c.context_json,'$.request_digest'),e.observed_at_ms FROM provider_probe_transport_contexts c JOIN actions a ON a.action_id=c.action_id AND a.state='observed_success' JOIN action_dispatches d ON d.action_id=a.action_id JOIN broker_phase_records b ON b.action_id=a.action_id AND b.phase_key='observation:1' JOIN events v ON v.event_id=b.event_id AND v.event_type='ActionObserved' JOIN evidence e ON e.action_id=a.action_id AND e.snapshot_ref LIKE 'provider-connect-fact:v1:%' AND e.availability='available' AND e.producer_id=?2 JOIN evidence disposition ON disposition.action_id=a.action_id AND disposition.snapshot_ref LIKE 'provider-connect-disposition:v1:%:'||v.event_id AND disposition.object_hash=e.object_hash AND disposition.availability='available' JOIN object_references r ON r.owner_kind='provider_probe' AND r.owner_id=c.probe_id AND r.source_field='connect_observation' AND r.object_hash=e.object_hash AND r.state='live' WHERE c.probe_id=?1 AND json_extract(v.payload_json,'$.output_digest')=e.object_hash",
        params![probe,CONNECT_OBSERVER_PRINCIPAL],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?)),
    ).optional()?.ok_or_else(||invalid("preuve du dernier connect indisponible"))?;
    Ok(Some(ConnectProjection {
        probe: probe.parse()?,
        action: proof.0.parse()?,
        dispatch: proof.1.parse()?,
        digest: proof.2.parse()?,
        request_digest: proof.3.parse()?,
        observed_at: TimestampMs::new(proof.4),
    }))
}

pub(crate) fn finish_projection(
    store: &UnleashStore,
    proof: &ConnectProjection,
) -> Result<crate::provider_profile_validation::ProviderValidationSummary, StoreError> {
    let pin = store.acquire_read_pin(
        proof.digest,
        "provider_connect_inspection",
        &proof.probe.to_string(),
    )?;
    let config = store.config().clone();
    let digest = proof.digest;
    let result = store.read_snapshot(move |conn, _| {
        crate::objects::read_verified_evidence_bytes_bounded(
            conn,
            &config,
            digest,
            MAX_NATIVE_BYTES,
        )
    });
    let released = store.release_pin(&pin);
    let bytes = result?;
    released?;
    let document: ProviderConnectObservation = serde_json::from_slice(&bytes)?;
    if canonical_sha256(&document)? != digest
        || document.action_id != proof.action
        || document.dispatch_id != proof.dispatch
        || document.request_digest != proof.request_digest
        || document.observed_at_ms != proof.observed_at
        || document.http_application_bytes_sent != 0
        || document.revocation_checked
        || !document.peer.ip().is_loopback()
    {
        return Err(invalid("preuve de connect projetée divergente"));
    }
    Ok(
        crate::provider_profile_validation::ProviderValidationSummary {
            phase: unleash_protocol::ProviderTestPhase::Connect,
            probe_id: proof.probe,
            result_ref: format!("provider-probe:{}", proof.probe),
            observed_at_ms: proof.observed_at,
            status: crate::provider_profile_validation::LocalValidationStatus::Connected,
        },
    )
}
