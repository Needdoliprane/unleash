//! Bind a current local listener observation to the recorded service launch.

use std::{net::SocketAddr, sync::atomic::Ordering};

use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};
use unleash_domain::{DispatchId, Sha256Digest, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, ProviderDialectKind, ProviderServerManagement, canonical_sha256,
};
use uuid::Uuid;

use super::{
    FIELD, OBSERVER, OWNER, Observation, PREFIX, PlacementDisposition, PlacementMethod,
    ServerInstance, read_bound_record, read_daemon_state_conn, unavailable, verify_live_generation,
};
use crate::{
    AttestedProviderServiceLaunch, CallerAuthority, ConfiguredProviderProfile,
    PreparedProviderProbe, ProviderServiceProcessIdentity, StoreError, UnleashStore,
    WriteCriticality,
    provider_probe_generation::{publish_candidate, retire_candidates},
    provider_service_launch::attested::read_attested_launch,
};

/// Fresh, in-process measurement of the listener of a daemon-owned provider.
#[derive(Clone)]
pub struct ManagedProviderListener {
    /// Exact launch dispatch matched by the controller.
    pub dispatch_id: DispatchId,
    /// Child identity authenticated by the host and observed at the local listener.
    pub child: ProviderServiceProcessIdentity,
    /// Numeric loopback socket reported by the OS.
    pub endpoint: SocketAddr,
    /// Time of the native observation.
    pub observed_at: TimestampMs,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LaunchBinding {
    dispatch_id: DispatchId,
    attestation_digest: Sha256Digest,
}

fn verify_profile_binding(
    launch: &AttestedProviderServiceLaunch,
    probe: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    let context = &launch.context;
    let dialect = profile.configuration.payload.dialect();
    if dialect.kind != ProviderDialectKind::OllamaChat
        || dialect.version != 1
        || context.profile_version_ref != profile.profile_version_ref
        || context.configuration_digest != profile.configuration_digest
        || context.project_id != probe.context.project_id
        || context.scope_revision != probe.context.scope_revision
        || context.policy_digest != probe.context.security_policy_digest
    {
        return Err(unavailable(
            "managed launch does not match the generation profile",
        ));
    }
    Ok(())
}

pub(super) fn verify_launch_binding(
    tx: &Transaction<'_>,
    probe: &PreparedProviderProbe,
    profile: &ConfiguredProviderProfile,
    fact: &Observation,
    binding: &LaunchBinding,
) -> Result<AttestedProviderServiceLaunch, StoreError> {
    let launch = read_attested_launch(tx, binding.dispatch_id)?;
    verify_profile_binding(&launch, probe, profile)?;
    let expected = &launch.attestation.child;
    if launch.attestation_digest != binding.attestation_digest
        || launch.recorded_at > fact.observed_at_ms
        || launch.context.endpoint != fact.destination
        || launch.context.plan.executable_digest != fact.server.build_digest
        || launch.attestation.confinement_digest != fact.server.confinement_digest
        || expected.pid != fact.server.process_id
        || expected.start_identity != fact.server.process_start_identity
        || expected.boot_identity != fact.server.boot_identity
    {
        return Err(unavailable(
            "placement diverges from the attested service launch",
        ));
    }
    Ok(launch)
}

fn listener_matches(
    conn: &Connection,
    profile: &ConfiguredProviderProfile,
    probe: &PreparedProviderProbe,
    observed: &ManagedProviderListener,
) -> Result<AttestedProviderServiceLaunch, StoreError> {
    let launch = read_attested_launch(conn, observed.dispatch_id)?;
    verify_profile_binding(&launch, probe, profile)?;
    let expected = profile
        .configuration
        .payload
        .endpoint_destination()
        .map_err(|_| unavailable("managed endpoint unavailable"))?;
    let ip: std::net::IpAddr = expected
        .host()
        .parse()
        .map_err(|_| unavailable("managed endpoint must be a numeric address"))?;
    if !ip.is_loopback()
        || SocketAddr::new(ip, expected.port()) != observed.endpoint
        || expected.normalized_endpoint() != launch.context.endpoint
        || observed.child != launch.attestation.child
        || observed.observed_at < launch.recorded_at
    {
        return Err(unavailable("listener does not match the attested launch"));
    }
    Ok(launch)
}

impl UnleashStore {
    /// Read the exact authorized probe still waiting for an owned Ollama listener.
    ///
    /// # Errors
    /// Refuses commands with a divergent receipt, policy or authority.
    pub fn read_waiting_managed_generation(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<Option<PreparedProviderProbe>, StoreError> {
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let command = command.clone();
        let caller = caller.clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            let record = read_bound_record(tx, &daemon, &command, &caller)?;
            let waiting: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM provider_probes p
                 JOIN provider_probe_contexts c ON c.probe_id=p.probe_id
                 JOIN actions a ON a.action_id=c.action_id
                 WHERE p.probe_id=?1 AND p.state='queued' AND p.wait_reason=?2 AND p.fence=0
                   AND a.state='authorizing' AND a.fence=0
                   AND NOT EXISTS(SELECT 1 FROM evidence e WHERE e.action_id=a.action_id
                       AND e.producer_id=?3 AND e.snapshot_ref GLOB (?4 || '*')))",
                rusqlite::params![
                    record.context.probe_id.to_string(),
                    crate::provider_probes::WAIT_FOR_PLACEMENT,
                    OBSERVER,
                    PREFIX,
                ],
                |row| row.get(0),
            )?;
            if !waiting {
                return Ok(None);
            }
            let resolved = verify_live_generation(tx, &daemon, now, &command, &caller, &record)?;
            let profile = &resolved.profile.configuration.payload;
            let dialect = profile.dialect();
            Ok(
                (profile.server_management() == ProviderServerManagement::UnleashManaged
                    && dialect.kind == ProviderDialectKind::OllamaChat
                    && dialect.version == 1)
                    .then_some(record),
            )
        })
    }

    /// Build and adopt a placement using the launch recorded for the measured listener.
    ///
    /// # Errors
    /// Refuses mismatched, expired or unavailable launch, probe and listener evidence.
    pub fn adopt_managed_provider_generation_placement(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        listener: ManagedProviderListener,
    ) -> Result<Uuid, StoreError> {
        self.capture_managed_listener(command, caller, listener, PlacementDisposition::Schedulable)
    }

    /// Retain the exact placement until a generation executor can take responsibility for it.
    ///
    /// # Errors
    /// Refuses mismatched, expired or unavailable launch, probe and listener evidence.
    pub fn stage_managed_provider_generation_placement(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        listener: ManagedProviderListener,
    ) -> Result<Uuid, StoreError> {
        self.capture_managed_listener(command, caller, listener, PlacementDisposition::Staged)
    }

    fn capture_managed_listener(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        listener: ManagedProviderListener,
        disposition: PlacementDisposition,
    ) -> Result<Uuid, StoreError> {
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let request = command.clone();
        let actor = caller.clone();
        let config = self.config().clone();
        let fact = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            if config.clock_uncertain.load(Ordering::Acquire) || listener.observed_at > now {
                return Err(unavailable("managed placement clock unavailable"));
            }
            let daemon = read_daemon_state_conn(tx)?;
            let record = read_bound_record(tx, &daemon, &request, &actor)?;
            let resolved = verify_live_generation(tx, &daemon, now, &request, &actor, &record)?;
            let launch = listener_matches(tx, &resolved.profile, &record, &listener)?;
            let context = &record.context;
            let freshness = i64::try_from(
                resolved
                    .profile
                    .configuration
                    .payload
                    .limits()
                    .connect_timeout_ms,
            )
            .map_err(|_| unavailable("managed placement timing invalid"))?;
            let valid_until_ms = context.deadline.min(TimestampMs::new(
                listener.observed_at.as_i64().saturating_add(freshness),
            ));
            if valid_until_ms <= now {
                return Err(unavailable("managed placement expired"));
            }
            Ok(Observation {
                schema_version: 1,
                method: PlacementMethod::ManagedProcessNoEgress,
                probe_id: context.probe_id,
                action_id: context.action_id,
                context_digest: record.context_digest,
                request_manifest_hash: canonical_sha256(&record.request_manifest()?)?,
                profile_configuration_digest: resolved.profile.configuration_digest,
                model_id: context.preparation.resolved_model_id.clone(),
                destination: launch.context.endpoint,
                server: ServerInstance {
                    boot_identity: listener.child.boot_identity,
                    process_id: listener.child.pid,
                    process_start_identity: listener.child.start_identity,
                    build_digest: launch.context.plan.executable_digest,
                    confinement_digest: launch.attestation.confinement_digest,
                },
                managed_launch: Some(LaunchBinding {
                    dispatch_id: listener.dispatch_id,
                    attestation_digest: launch.attestation_digest,
                }),
                observed_at_ms: listener.observed_at,
                valid_until_ms,
            })
        })?;
        let owner = self.config().id_generator.new_uuid().to_string();
        let candidate =
            publish_candidate(self, OWNER, &owner, FIELD, &serde_json::to_value(fact)?)?;
        let result = self.record_provider_generation_placement(
            command,
            caller,
            candidate.reference.clone(),
            disposition,
        );
        retire_candidates(self, &[candidate]);
        result
    }
}
