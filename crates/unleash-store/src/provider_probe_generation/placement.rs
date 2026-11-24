//! Trusted observer ingress and the writer's admission check for generation probes.

use std::{collections::BTreeMap, sync::atomic::Ordering};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActionState, AttemptId, DataLocality, Fence, LeaseId, ProviderProbeId, ReservationId,
    Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, ProviderDialectKind, ProviderServerManagement, ProviderTestLimits,
    ProviderTestPhase, TestProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
    from_json_str_strict,
    provider_probe::{OllamaChatPlacement, ollama_chat_body},
};
use uuid::Uuid;

use crate::commands::{DaemonStateRow, read_daemon_state_conn};
use crate::objects::{
    OwnedReferenceAdoption, PublishedObjectReference, read_verified_evidence_bytes_bounded,
    tx_adopt_owned_reference,
};
use crate::{
    AdmissionCommit, AdmissionEligibilityObservation, AdmissionUnit, AdmissionUrgency,
    CallerAuthority, ConfiguredProviderProfile, PreparedProviderProbe, StoreConfig, StoreError,
    UnleashStore, WriteCriticality,
};

mod admission_handoff;
mod managed;
pub use admission_handoff::ManagedGenerationAdmission;
pub use managed::ManagedProviderListener;

use super::authorizing::{read_bound_record, verify_live_generation};
use super::{FIXTURE_PROMPT, unavailable};

/// Internal principal for managed provider placement observations.
pub const PLACEMENT_OBSERVER_PRINCIPAL: &str = "system:provider-generation-placement";
use PLACEMENT_OBSERVER_PRINCIPAL as OBSERVER;
const OWNER: &str = "provider_generation_placement_preparation";
const FIELD: &str = "placement";
const PREFIX: &str = "provider-generation-placement:v1:";
const MAX_PLACEMENT_BYTES: usize = 16_384;
const MAX_CHAT_BODY_BYTES: usize = 65_536;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlacementMethod {
    ManagedProcessNoEgress,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerInstance {
    boot_identity: String,
    process_id: u32,
    process_start_identity: String,
    build_digest: Sha256Digest,
    confinement_digest: Sha256Digest,
}

impl ServerInstance {
    fn key(&self) -> Result<Sha256Digest, StoreError> {
        // Build, model, address and profile aliases do not split one process's capacity.
        Ok(canonical_sha256(&(
            &self.boot_identity,
            self.process_id,
            &self.process_start_identity,
        ))?)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    schema_version: u32,
    method: PlacementMethod,
    probe_id: ProviderProbeId,
    action_id: ActionId,
    context_digest: Sha256Digest,
    request_manifest_hash: Sha256Digest,
    profile_configuration_digest: Sha256Digest,
    model_id: String,
    destination: String,
    server: ServerInstance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    managed_launch: Option<managed::LaunchBinding>,
    observed_at_ms: TimestampMs,
    valid_until_ms: TimestampMs,
}

/// Corps canonique et placement observé pour une admission Ollama précise.
pub struct AdmittedOllamaChatInputs {
    /// Contexte de préparation durable dont le corps est issu.
    pub probe: PreparedProviderProbe,
    /// Octets canoniques du corps Ollama Chat v1.
    pub body: Vec<u8>,
    /// Octets canoniques du fait de placement retenu lors de l'admission.
    pub placement: Vec<u8>,
    /// Coordonnées et preuves sélectionnées qui bornent la préparation du transport.
    pub transport_placement: OllamaChatPlacement,
    /// Native binding for a placement backed by a managed launch observation.
    pub managed_transport: Option<super::dispatch::ManagedOllamaTransport>,
}

impl std::fmt::Debug for AdmittedOllamaChatInputs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmittedOllamaChatInputs")
            .field("probe", &self.probe)
            .finish_non_exhaustive()
    }
}

fn verify_observation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
) -> Result<(), StoreError> {
    verify_observation_at_state(
        tx,
        daemon,
        now,
        record,
        profile,
        fact,
        ActionState::Authorizing,
    )
    .map(|_| ())
}

fn verify_observation_at_state(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
    state: ActionState,
) -> Result<Option<crate::AttestedProviderServiceLaunch>, StoreError> {
    let context = &record.context;
    let payload = &profile.configuration.payload;
    let destination = payload
        .endpoint_destination()
        .map_err(|_| unavailable("destination de génération invalide"))?;
    let started: Option<i64> = tx
        .query_row(
            "SELECT e.occurred_at_ms FROM model_calls c
         JOIN actions a USING(action_id) JOIN events e ON e.event_id=c.trigger_event_id
         JOIN broker_phase_records b ON b.action_id=a.action_id AND b.event_id=e.event_id
         JOIN model_call_manifests m ON m.model_call_id=c.model_call_id
         WHERE c.model_call_id=?1 AND c.action_id=?2 AND c.provider_probe_id=?3
           AND c.state='prepared' AND a.state=?6 AND a.fence=0
           AND e.event_type='ActionAuthorizationRequested'
           AND m.manifest_id=?4 AND m.content_object_hash=?5",
            params![
                context.model_call_id.to_string(),
                context.action_id.to_string(),
                context.probe_id.to_string(),
                context.manifest_id.to_string(),
                fact.request_manifest_hash.to_string(),
                state.to_string(),
            ],
            |row| row.get(0),
        )
        .optional()?;
    if started.is_none_or(|at| at > fact.observed_at_ms.as_i64())
        || fact.schema_version != 1
        || fact.probe_id != context.probe_id
        || fact.action_id != context.action_id
        || fact.context_digest != record.context_digest
        || fact.request_manifest_hash != canonical_sha256(&record.request_manifest()?)?
        || fact.profile_configuration_digest != profile.configuration_digest
        || fact.model_id != context.preparation.resolved_model_id
        || payload.server_management() != ProviderServerManagement::UnleashManaged
        || payload.locality() != DataLocality::LocalOnly
        || context.preparation.locality != DataLocality::LocalOnly
        || fact.destination != destination.normalized_endpoint()
        || !destination.is_loopback_literal()
        || fact.server.boot_identity != daemon.boot_identity
        || fact.server.process_id == 0
        || fact.server.process_start_identity.is_empty()
        || fact.server.process_start_identity.len() > 256
        || fact
            .server
            .process_start_identity
            .chars()
            .any(char::is_control)
        || fact.observed_at_ms > now
        || fact.valid_until_ms <= now
        || fact.valid_until_ms > context.deadline
    {
        return Err(unavailable(
            "placement natif de génération divergent ou expiré",
        ));
    }
    fact.managed_launch
        .as_ref()
        .map(|binding| managed::verify_launch_binding(tx, record, profile, fact, binding))
        .transpose()
}

#[derive(Clone, Copy)]
enum PlacementDisposition {
    Staged,
    Schedulable,
}

#[derive(Clone, Copy)]
struct PlacementAdoption<'a> {
    candidate: &'a PublishedObjectReference,
    disposition: PlacementDisposition,
}

fn record_placement(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
    adoption: PlacementAdoption<'_>,
) -> Result<Uuid, StoreError> {
    let PlacementAdoption {
        candidate,
        disposition,
    } = adoption;
    let action = fact.action_id.to_string();
    let probe = fact.probe_id.to_string();
    let hash = candidate.object_hash();
    let server = fact.server.key()?;
    let binding = format!(
        "{PREFIX}{}/{server}/{}",
        fact.context_digest,
        fact.valid_until_ms.as_i64()
    );
    let known: Option<(String, String, String)> = tx
        .query_row(
            "SELECT evidence_id,snapshot_ref,object_hash FROM evidence
         WHERE action_id=?1 AND producer_id=?2 AND snapshot_ref GLOB (?3 || '*')",
            params![action, OBSERVER, PREFIX],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let fresh = known.is_none();
    let evidence = match known {
        Some((id, prior_binding, prior_hash))
            if prior_binding == binding && prior_hash == hash.to_string() =>
        {
            id.parse()
                .map_err(|_| unavailable("identité du placement illisible"))?
        }
        Some(_) => return Err(unavailable("placement de génération déjà attribué")),
        None => config.id_generator.new_uuid(),
    };
    let producer: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM principals WHERE principal_id=?1 AND actor_id=?1
         AND principal_kind='system' AND state='active')",
        [OBSERVER],
        |row| row.get(0),
    )?;
    if !producer {
        return Err(unavailable("observateur du placement non qualifié"));
    }
    tx_adopt_owned_reference(
        tx,
        config.id_generator.as_ref(),
        daemon.authority_generation,
        &OwnedReferenceAdoption {
            candidate,
            object_hash: hash,
            producer_kind: OWNER,
            producer_field: FIELD,
            owner_kind: "provider_probe",
            owner_id: &probe,
            source_field: FIELD,
        },
    )?;
    if fresh {
        let waiting: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1
             AND state='queued' AND wait_reason=?2 AND fence=0)",
            params![probe, crate::provider_probes::WAIT_FOR_PLACEMENT],
            |row| row.get(0),
        )?;
        if !waiting {
            return Err(unavailable("le probe n'attend plus son placement"));
        }
        tx.execute(
            "INSERT INTO evidence(evidence_id,kind,action_id,snapshot_ref,object_hash,producer_id,
             observed_at_ms,schema_version,availability)
             VALUES(?1,'observation',?2,?3,?4,?5,?6,1,'available')",
            params![
                evidence.to_string(),
                action,
                binding,
                hash.to_string(),
                OBSERVER,
                fact.observed_at_ms.as_i64()
            ],
        )?;
    }
    if matches!(disposition, PlacementDisposition::Schedulable) {
        clear_placement_wait(tx, fact.probe_id)?;
        observe_eligibility(tx, config, now, profile, server, fact, hash)?;
    }
    Ok(evidence)
}

fn clear_placement_wait(tx: &Transaction<'_>, probe: ProviderProbeId) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE provider_probes SET wait_reason=NULL,revision=revision+1
         WHERE probe_id=?1 AND state='queued' AND wait_reason=?2 AND fence=0",
        params![
            probe.to_string(),
            crate::provider_probes::WAIT_FOR_PLACEMENT
        ],
    )?)
}

fn observe_eligibility(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    profile: &ConfiguredProviderProfile,
    server: Sha256Digest,
    fact: &Observation,
    hash: Sha256Digest,
) -> Result<(), StoreError> {
    let pool = crate::budgets::tx_global_execution_pool_id(tx)?
        .ok_or_else(|| unavailable("pool exécution absent avant admission génération"))?;
    let snapshot = crate::admission::read_snapshot(tx, pool, now)?;
    let unit = AdmissionUnit::Probe(fact.probe_id);
    let Some(candidate) = snapshot.queued.iter().find(|row| row.unit == unit) else {
        return Ok(()); // The exact placement may be replayed after admission.
    };
    let available = validate_capacity(tx, server, profile)
        .and_then(|()| crate::provider_probes::validate_admission_budgets(tx, fact.probe_id, &[]));
    match available {
        Ok(()) => {}
        Err(StoreError::CapacityBlocked(_) | StoreError::BudgetExhausted { .. }) => return Ok(()),
        Err(error) => return Err(error),
    }
    if let Some(id) = candidate.eligibility_evidence
        && verified_placement(
            tx,
            fact.action_id,
            fact.probe_id,
            fact.context_digest,
            id,
            now,
        )?
        .is_some()
    {
        return Ok(());
    }
    crate::admission::record_eligibility(
        tx,
        now,
        &AdmissionEligibilityObservation {
            evidence_id: config.id_generator.new_uuid(),
            observer_principal_id: OBSERVER.to_owned(),
            object_hash: hash,
            unit,
            unit_revision: candidate.revision,
            source_revision: snapshot.source_revision,
            authority_generation: snapshot.authority_generation,
            daemon_epoch: snapshot.daemon_epoch,
            valid_until: fact.valid_until_ms,
            urgency: AdmissionUrgency::Ordinary,
        },
    )
}

struct VerifiedPlacement {
    server_key: Sha256Digest,
    object_hash: Sha256Digest,
    expires_at_ms: i64,
}

fn verified_placement(
    tx: &Transaction<'_>,
    action: ActionId,
    probe: ProviderProbeId,
    context_digest: Sha256Digest,
    eligibility: Uuid,
    now: TimestampMs,
) -> Result<Option<VerifiedPlacement>, StoreError> {
    let prefix = format!("{PREFIX}{context_digest}/");
    let observed: Option<(String, i64, String)> = tx.query_row(
        "SELECT e.snapshot_ref,a.valid_until_ms,e.object_hash FROM evidence e
         JOIN principals p ON p.principal_id=e.producer_id
         JOIN object_records o ON o.object_hash=e.object_hash
         JOIN evidence q ON q.object_hash=e.object_hash AND q.producer_id=e.producer_id
         JOIN admission_eligibility_observations a ON a.evidence_id=q.evidence_id
         WHERE e.action_id=?1 AND e.producer_id=?2 AND e.kind='observation'
           AND e.schema_version=1 AND e.availability='available' AND o.state='live'
           AND p.principal_kind='system' AND p.actor_id=p.principal_id AND p.state='active'
           AND q.evidence_id=?3 AND q.availability='available' AND a.valid_until_ms>?4
           AND e.snapshot_ref GLOB (?5 || '*')
           AND EXISTS(SELECT 1 FROM object_references r WHERE r.object_hash=e.object_hash
               AND r.owner_kind='provider_probe' AND r.owner_id=?6 AND r.source_field=?7 AND r.state='live')",
        params![action.to_string(), OBSERVER, eligibility.to_string(), now.as_i64(),
            prefix, probe.to_string(), FIELD],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    let Some((binding, valid_until, object_hash)) = observed else {
        return Ok(None);
    };
    binding
        .strip_prefix(&prefix)
        .and_then(|rest| rest.split_once('/'))
        .filter(|(_, expiry)| expiry.parse::<i64>() == Ok(valid_until))
        .map(|(key, _)| {
            Ok(VerifiedPlacement {
                server_key: key.parse()?,
                object_hash: object_hash.parse()?,
                expires_at_ms: valid_until,
            })
        })
        .transpose()
}

pub(crate) fn validate_admission(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &PreparedProviderProbe,
    eligibility: Option<Uuid>,
) -> Result<(), StoreError> {
    let unavailable = || unavailable("placement natif du probe génération indisponible");
    let c = &record.context;
    let placement = verified_placement(
        tx,
        c.action_id,
        c.probe_id,
        record.context_digest,
        eligibility.ok_or_else(unavailable)?,
        now,
    )?
    .ok_or_else(unavailable)?;
    let profile = verify_live_grant(tx, daemon, now, record)?;
    validate_capacity(tx, placement.server_key, &profile)
}

fn verify_live_grant(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &PreparedProviderProbe,
) -> Result<ConfiguredProviderProfile, StoreError> {
    let unavailable = || unavailable("mandat actif du probe génération indisponible");
    let c = &record.context;
    let prep = &c.preparation;
    let profile = crate::provider_profiles::resolve(
        tx,
        prep.requested_profile_id
            .parse()
            .map_err(|_| unavailable())?,
        prep.requested_profile_version,
    )?
    .ok_or_else(unavailable)?;
    let limits = &prep.limits;
    let wire = TestProviderProfilePayload::try_new(
        prep.requested_profile_id.clone(),
        prep.requested_profile_version,
        ProviderTestPhase::GenerationProbe,
        prep.capability_set.clone(),
        ProviderTestLimits::new(
            limits.timeout_ms,
            limits.max_calls,
            limits.max_output_tokens,
            limits.max_cost_usd_micros,
        ),
        prep.locality,
        prep.mandate_ref.clone(),
    )
    .map_err(|_| unavailable())?;
    let grant = crate::provider_probe_mandates::resolve_generation_grant(
        tx,
        daemon,
        now,
        &c.actor_id,
        c.project_id,
        &wire,
        &profile,
    )?;
    if grant.principal_id != c.principal_id || grant.expires_at_ms < c.deadline {
        return Err(unavailable());
    }
    Ok(profile)
}

fn validate_capacity(
    tx: &Transaction<'_>,
    server: Sha256Digest,
    profile: &ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    if occupied_slots(tx, Some(server))? != 0 {
        return Err(StoreError::CapacityBlocked(
            "serveur du probe déjà occupé ou non identifié".to_owned(),
        ));
    }
    crate::provider_probe_connect::validate_profile_capacity(tx, profile)
}

pub(crate) fn occupied_slots(
    tx: &Transaction<'_>,
    server: Option<Sha256Digest>,
) -> Result<i64, StoreError> {
    // A passive or unattributable reservation may occupy any server. A resolved
    // reservation's identity remains relevant until release, even after proof expiry.
    let same_server = server.map(|key| format!("{PREFIX}*/{key}/*"));
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM capacity_reservations r JOIN capacity_pools pool ON pool.pool_id=r.pool_id
         JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id
         WHERE r.state IN ('held','reconciling') AND r.amount>0 AND r.owner_type='provider_probe'
           AND pool.pool_kind='execution' AND pool.scope_key='global'
           AND (EXISTS(SELECT 1 FROM provider_probe_transport_contexts c
                      WHERE c.probe_id=o.provider_probe_id AND c.phase IN ('connect','inventory'))
                OR EXISTS(SELECT 1 FROM provider_probe_contexts c
                          JOIN provider_profiles f ON f.profile_version_ref=c.profile_version_ref
                          WHERE c.probe_id=o.provider_probe_id AND f.profile_id IS NOT NULL
                            AND (?1 IS NULL OR NOT EXISTS(
                                SELECT 1 FROM evidence e WHERE e.action_id=c.action_id
                                  AND e.producer_id=?2 AND e.snapshot_ref GLOB (?3 || '*')
                                  AND e.snapshot_ref NOT GLOB ?1))))",
        params![same_server, OBSERVER, PREFIX], |row| row.get(0),
    )?)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionEvent {
    reservation_id: ReservationId,
    attempt_id: Option<AttemptId>,
    lease_id: LeaseId,
    fence: Fence,
    eligibility_evidence_id: Uuid,
}

fn selected_eligibility(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    probe: ProviderProbeId,
    admission: &AdmissionCommit,
) -> Result<(Uuid, Sha256Digest), StoreError> {
    let unavailable = || unavailable("admission native Ollama divergente ou périmée");
    let lease = &admission.lease;
    let owner_id = probe.to_string();
    if admission.event_cursor.stream_id != daemon.stream_id
        || admission.attempt_id.is_some()
        || lease.boot_identity != daemon.boot_identity
        || lease.authority_generation != daemon.authority_generation
        || lease.daemon_epoch != daemon.daemon_epoch
        || lease.fence == Fence::ZERO
        || lease.resource_kind != "execution"
        || lease.resource_key != format!("provider_probe:{owner_id}")
        || lease.owner_type != "provider_probe"
        || lease.owner_id != owner_id
        || lease.state != crate::LeaseState::Held
    {
        return Err(unavailable());
    }
    let integer = |value| i64::try_from(value).map_err(|_| unavailable());
    let persisted: Option<(String, String)> = tx
        .query_row(
            "SELECT e.payload_json,e.payload_hash FROM events e
         JOIN provider_probes p ON p.probe_id=e.aggregate_id AND p.revision=e.aggregate_revision
         JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id
         JOIN capacity_reservations r ON r.reservation_id=o.reservation_id
         JOIN capacity_pools pool ON pool.pool_id=r.pool_id
         JOIN resource_leases l ON l.owner_type='provider_probe' AND l.owner_id=p.probe_id
         JOIN resource_fences f ON f.authority_generation=l.authority_generation
              AND f.resource_kind=l.resource_kind AND f.resource_key=l.resource_key
         WHERE e.stream_id=:stream AND e.sequence=:sequence
           AND e.aggregate_type='provider_probe' AND e.event_type='ProviderProbeAdmitted'
           AND e.actor_id='system:scheduler' AND e.payload_state='full'
           AND p.probe_id=:probe AND p.phase='generation_probe' AND p.state='admitted'
           AND p.revision=:revision AND p.fence=:fence
           AND p.authority_generation=:authority AND p.daemon_epoch=:epoch
           AND r.reservation_id=:reservation AND r.state='held' AND r.amount=1
           AND r.owner_type='provider_probe' AND r.owner_id=p.probe_id
           AND r.authority_generation=:authority AND r.daemon_epoch=:epoch
           AND pool.pool_kind='execution' AND pool.scope_key='global'
           AND l.lease_id=:lease AND l.resource_kind='execution'
           AND l.resource_key='provider_probe:'||p.probe_id AND l.state='held'
           AND l.authority_generation=:authority AND l.daemon_epoch=:epoch
           AND l.fence=:fence AND f.last_fence=l.fence AND l.boot_identity=:boot
           AND l.expires_at_ms=:expires AND l.expires_at_ms>:now",
            rusqlite::named_params! {
                ":stream": daemon.stream_id.to_string(),
                ":sequence": integer(admission.event_cursor.sequence.as_u64())?,
                ":probe": owner_id,
                ":revision": integer(admission.revision.as_u64())?,
                ":fence": integer(lease.fence.as_u64())?,
                ":authority": daemon.authority_generation.to_string(),
                ":epoch": integer(daemon.daemon_epoch.as_u64())?,
                ":reservation": admission.reservation_id.to_string(),
                ":lease": lease.lease_id.to_string(),
                ":boot": daemon.boot_identity,
                ":expires": lease.expires_at_ms.as_i64(),
                ":now": now.as_i64(),
            },
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (json, hash) = persisted.ok_or_else(unavailable)?;
    let event: AdmissionEvent = from_json_str_strict(&json).map_err(|_| unavailable())?;
    let event_digest = canonical_sha256(&event)?;
    if event.reservation_id != admission.reservation_id
        || event.lease_id != lease.lease_id
        || event.fence != lease.fence
        || event.attempt_id.is_some()
        || event_digest.to_string() != hash
    {
        return Err(unavailable());
    }
    Ok((event.eligibility_evidence_id, event_digest))
}

fn admitted_chat_inputs(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    admission: &AdmissionCommit,
    require_launch: bool,
) -> Result<AdmittedOllamaChatInputs, StoreError> {
    admitted_chat_inputs_at_state(
        tx,
        config,
        now,
        admission,
        require_launch,
        ActionState::Authorizing,
    )
}

pub(super) fn admitted_chat_inputs_at_state(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    admission: &AdmissionCommit,
    require_launch: bool,
    state: ActionState,
) -> Result<AdmittedOllamaChatInputs, StoreError> {
    let unavailable = || unavailable("préparation native Ollama indisponible");
    let AdmissionUnit::Probe(probe_id) = admission.unit else {
        return Err(unavailable());
    };
    let daemon = read_daemon_state_conn(tx)?;
    if config.clock_uncertain.load(Ordering::Acquire)
        || daemon.mode != unleash_domain::DaemonMode::Ready
    {
        return Err(unavailable());
    }
    let (eligible, admission_digest) = selected_eligibility(tx, &daemon, now, probe_id, admission)?;
    let probe = crate::provider_probes::read_context(tx, probe_id)?.ok_or_else(unavailable)?;
    let context = &probe.context;
    if context.authority_generation != daemon.authority_generation
        || context.daemon_epoch != daemon.daemon_epoch
        || now >= context.deadline
        || !context.preparation.secret_refs.is_empty()
    {
        return Err(unavailable());
    }
    crate::provider_probes::verify_frozen_inputs(tx, &probe)?;
    let profile = verify_live_grant(tx, &daemon, now, &probe)?;
    let dialect = profile.configuration.payload.dialect();
    if dialect.kind != ProviderDialectKind::OllamaChat || dialect.version != 1 {
        return Err(unavailable());
    }
    let proof = verified_placement(
        tx,
        context.action_id,
        probe_id,
        probe.context_digest,
        eligible,
        now,
    )?
    .ok_or_else(unavailable)?;
    let placement =
        read_verified_evidence_bytes_bounded(tx, config, proof.object_hash, MAX_PLACEMENT_BYTES)?;
    let fact: Observation = serde_json::from_slice(&placement)?;
    if fact.server.key()? != proof.server_key
        || fact.valid_until_ms.as_i64() != proof.expires_at_ms
        || canonicalize_to_vec(&fact)? != placement
        || (require_launch && fact.managed_launch.is_none())
    {
        return Err(unavailable());
    }
    let managed_launch =
        verify_observation_at_state(tx, &daemon, now, &probe, &profile, &fact, state)?;
    let body_hash = probe.payload_digest();
    let owns_body: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_references WHERE object_hash=?1 AND
           owner_kind='provider_probe' AND owner_id=?2 AND source_field='request_payload' AND state='live')",
        params![body_hash.to_string(), probe_id.to_string()],
        |row| row.get(0),
    )?;
    if !owns_body {
        return Err(unavailable());
    }
    let body = read_verified_evidence_bytes_bounded(tx, config, body_hash, MAX_CHAT_BODY_BYTES)?;
    let expected = ollama_chat_body(
        &context.preparation.resolved_model_id,
        FIXTURE_PROMPT,
        context.preparation.limits.max_output_tokens,
        &BTreeMap::new(),
    );
    if canonicalize_to_vec(&expected)? != body {
        return Err(unavailable());
    }
    let valid_until_ms = context
        .deadline
        .min(fact.valid_until_ms)
        .min(admission.lease.expires_at_ms);
    let transport_placement = OllamaChatPlacement {
        endpoint: fact.destination,
        boot_identity: fact.server.boot_identity,
        process_id: fact.server.process_id,
        process_start_identity: fact.server.process_start_identity,
        evidence_digest: proof.object_hash,
        admission_digest,
        valid_until_ms,
    };
    let (transport_placement, managed_transport) = if let Some(launch) = &managed_launch {
        let (placement, transport) =
            super::dispatch::derive_transport(&probe, &profile, transport_placement, launch)?;
        (placement, Some(transport))
    } else {
        (transport_placement, None)
    };
    Ok(AdmittedOllamaChatInputs {
        probe,
        body,
        placement,
        transport_placement,
        managed_transport,
    })
}

impl UnleashStore {
    /// Relit les artefacts de l'admission Ollama dans un même snapshot du writer.
    ///
    /// Le consommateur doit obtenir sa propre permission de dispatch.
    ///
    /// # Errors
    /// Refuse les admissions ou les preuves indisponibles.
    pub fn read_admitted_ollama_chat(
        &self,
        admission: &AdmissionCommit,
    ) -> Result<AdmittedOllamaChatInputs, StoreError> {
        let admission = admission.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            admitted_chat_inputs(tx, &config, now, &admission, false)
        })
    }

    /// Read generation inputs with a durable launch binding for the managed Ollama runtime.
    ///
    /// # Errors
    /// Refuses an older placement without an authenticated launch or a stale admission.
    pub fn read_admitted_managed_ollama_chat(
        &self,
        admission: &AdmissionCommit,
    ) -> Result<AdmittedOllamaChatInputs, StoreError> {
        let admission = admission.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            admitted_chat_inputs(tx, &config, now, &admission, true)
        })
    }

    /// Adopts a bounded placement observation from the trusted in-process observer.
    ///
    /// The observer owns the candidate until adoption. Registration and proof
    /// acquisition belong to the native producer, never the client protocol.
    ///
    /// # Errors
    /// Refuses stale, mismatched or unverified placement observations.
    pub fn adopt_provider_generation_placement(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        candidate: PublishedObjectReference,
    ) -> Result<Uuid, StoreError> {
        self.record_provider_generation_placement(
            command,
            caller,
            candidate,
            PlacementDisposition::Schedulable,
        )
    }

    fn record_provider_generation_placement(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        candidate: PublishedObjectReference,
        disposition: PlacementDisposition,
    ) -> Result<Uuid, StoreError> {
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let hash = candidate.object_hash();
        let pin = self.acquire_read_pin(
            hash,
            "provider_generation_placement_reader",
            &hash.to_string(),
        )?;
        let config = self.config().clone();
        let bytes = self.read_snapshot(move |conn, _| {
            read_verified_evidence_bytes_bounded(conn, &config, hash, MAX_PLACEMENT_BYTES)
        });
        let released = self.release_pin(&pin);
        let bytes = bytes?;
        released?;
        let fact: Observation = serde_json::from_slice(&bytes)?;
        if canonicalize_to_vec(&fact)? != bytes {
            return Err(unavailable("objet de placement non canonique"));
        }
        let command = command.clone();
        let caller = caller.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            if config.clock_uncertain.load(Ordering::Acquire) {
                return Err(unavailable("horloge du placement incertaine"));
            }
            let daemon = read_daemon_state_conn(tx)?;
            let record = read_bound_record(tx, &daemon, &command, &caller)?;
            let resolved = verify_live_generation(tx, &daemon, now, &command, &caller, &record)?;
            crate::provider_probes::verify_frozen_inputs(tx, &record)?;
            verify_observation(tx, &daemon, now, &record, &resolved.profile, &fact)?;
            record_placement(
                tx,
                &config,
                &daemon,
                now,
                &resolved.profile,
                &fact,
                PlacementAdoption {
                    candidate: &candidate,
                    disposition,
                },
            )
        })
    }
}
