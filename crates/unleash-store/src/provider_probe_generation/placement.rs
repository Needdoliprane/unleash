//! Trusted observer ingress and the writer's admission check for generation probes.

use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{ActionId, DataLocality, ProviderProbeId, Sha256Digest, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, ProviderServerManagement, ProviderTestLimits, ProviderTestPhase,
    TestProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
};
use uuid::Uuid;

use crate::commands::{DaemonStateRow, read_daemon_state_conn};
use crate::objects::{OwnedReferenceAdoption, PublishedObjectReference, tx_adopt_owned_reference};
use crate::{
    AdmissionEligibilityObservation, AdmissionUnit, AdmissionUrgency, CallerAuthority,
    ConfiguredProviderProfile, PreparedProviderProbe, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};

use super::authorizing::{read_bound_record, verify_live_generation};
use super::unavailable;

const OBSERVER: &str = "system:provider-generation-placement";
const OWNER: &str = "provider_generation_placement_preparation";
const FIELD: &str = "placement";
const PREFIX: &str = "provider-generation-placement:v1:";

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
    observed_at_ms: TimestampMs,
    valid_until_ms: TimestampMs,
}

fn verify_observation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    record: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
) -> Result<(), StoreError> {
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
           AND c.state='prepared' AND a.state='authorizing' AND a.fence=0
           AND e.event_type='ActionAuthorizationRequested'
           AND m.manifest_id=?4 AND m.content_object_hash=?5",
            params![
                context.model_call_id.to_string(),
                context.action_id.to_string(),
                context.probe_id.to_string(),
                context.manifest_id.to_string(),
                fact.request_manifest_hash.to_string()
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
    Ok(())
}

fn record_placement(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
    candidate: &PublishedObjectReference,
) -> Result<Uuid, StoreError> {
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
        let changed = tx.execute(
            "UPDATE provider_probes SET wait_reason=NULL,revision=revision+1
             WHERE probe_id=?1 AND state='queued' AND wait_reason=?2 AND fence=0",
            params![probe, crate::provider_probes::WAIT_FOR_PLACEMENT],
        )?;
        if changed != 1 {
            return Err(unavailable("le probe n'attend plus son placement"));
        }
    }
    observe_eligibility(tx, config, now, profile, server, fact, hash)?;
    Ok(evidence)
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
        && verified_server_key(
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

fn verified_server_key(
    tx: &Transaction<'_>,
    action: ActionId,
    probe: ProviderProbeId,
    context_digest: Sha256Digest,
    eligibility: Uuid,
    now: TimestampMs,
) -> Result<Option<Sha256Digest>, StoreError> {
    let prefix = format!("{PREFIX}{context_digest}/");
    let observed: Option<(String, i64)> = tx.query_row(
        "SELECT e.snapshot_ref,a.valid_until_ms FROM evidence e
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
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let Some((binding, valid_until)) = observed else {
        return Ok(None);
    };
    binding
        .strip_prefix(&prefix)
        .and_then(|rest| rest.split_once('/'))
        .filter(|(_, expiry)| expiry.parse::<i64>() == Ok(valid_until))
        .map(|(key, _)| key.parse().map_err(StoreError::from))
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
    let key = verified_server_key(
        tx,
        c.action_id,
        c.probe_id,
        record.context_digest,
        eligibility.ok_or_else(unavailable)?,
        now,
    )?
    .ok_or_else(unavailable)?;
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
    validate_capacity(tx, key, &profile)
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

impl UnleashStore {
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
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let hash = candidate.object_hash();
        let pin = self.acquire_read_pin(
            hash,
            "provider_generation_placement_reader",
            &hash.to_string(),
        )?;
        let config = self.config().clone();
        let bytes = self.read_snapshot(move |conn, _| {
            crate::objects::read_verified_evidence_bytes_bounded(conn, &config, hash, 16_384)
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
                &candidate,
            )
        })
    }
}
