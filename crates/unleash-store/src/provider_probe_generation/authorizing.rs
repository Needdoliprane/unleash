//! Adopt the exact request manifest with the first durable broker phase.

use rusqlite::Transaction;
use unleash_domain::{ProviderProbeId, Revision, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, ReceiptState, canonical_sha256, compute_command_digest_v2,
};

use crate::commands::{DaemonStateRow, read_daemon_state_conn};
use crate::{
    BrokerPhase, BrokerPhaseCommit, BrokerPhaseRequest, CallerAuthority, PolicyIsolationMode,
    PreparedProviderProbe, StoreCutpoint, StoreError, UnleashStore, WriteCriticality,
};

use super::{adopt_candidate, hidden, limits, publish, resolve_tx, retire_candidates, unavailable};

pub(super) fn read_bound_record(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<PreparedProviderProbe, StoreError> {
    let receipt = crate::commands::classify_command_replay(
        tx,
        daemon,
        &caller.actor_id,
        command,
        compute_command_digest_v2(command)?,
    )?
    .filter(|receipt| receipt.state == ReceiptState::Accepted)
    .ok_or_else(hidden)?;
    let probe = ProviderProbeId::from_uuid(command.aggregate_id);
    let record = crate::provider_probes::read_context(tx, probe)?.ok_or_else(hidden)?;
    let context = &record.context;
    if context.command_receipt_id != receipt.receipt_id
        || context.actor_id != caller.actor_id
        || context.project_id != command.project_id
        || context.authority_generation != daemon.authority_generation
        || context.daemon_epoch != daemon.daemon_epoch
    {
        return Err(hidden());
    }
    Ok(record)
}

pub(super) fn verify_live_generation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
    record: &PreparedProviderProbe,
) -> Result<super::Resolution, StoreError> {
    let context = &record.context;
    let resolved = resolve_tx(tx, daemon, now, command, caller)?;
    if context.principal_id != resolved.grant.principal_id
        || context.preparation.mandate_ref != resolved.grant.mandate_ref.to_string()
        || context.preparation.profile_version_ref != resolved.profile.profile_version_ref
        || context.preparation.limits != limits(&resolved)
        || context.security_policy_digest != resolved.policy_digest
        || context.deadline > resolved.grant.expires_at_ms
        || context.deadline <= now
    {
        return Err(unavailable("préparation de génération modifiée ou expirée"));
    }
    Ok(resolved)
}

fn read_pending(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Option<PreparedProviderProbe>, StoreError> {
    let record = read_bound_record(tx, daemon, command, caller)?;
    if crate::broker_writer::inspect_action_conn(tx, record.context.action_id)?.is_some() {
        return Ok(None);
    }
    verify_live_generation(tx, daemon, now, command, caller, &record)?;
    Ok(Some(record))
}

impl UnleashStore {
    /// Retains the exact request manifest and writes the first broker phase.
    ///
    /// A repeat of the original receipt may recover an unstarted preparation.
    /// Once Authorizing is durable it cannot start another evaluation.
    ///
    /// # Errors
    ///
    /// Refuses missing, stale or divergent authenticated preparation and evidence.
    pub fn commit_provider_generation_authorizing(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        build_phase: impl FnOnce(
            &PreparedProviderProbe,
            PolicyIsolationMode,
        ) -> Result<BrokerPhaseRequest, StoreError>,
    ) -> Result<Option<BrokerPhaseCommit>, StoreError> {
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let wire = command.clone();
        let authority = caller.clone();
        let pending = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            read_pending(tx, &daemon, now, &wire, &authority)?
                .map(|record| {
                    let policy =
                        crate::policies_secrets::read_policy_summary_conn(tx, wire.project_id)?;
                    Ok((record, policy.effective_isolation_mode))
                })
                .transpose()
        })?;
        let Some((record, isolation)) = pending else {
            return Ok(None);
        };
        let request = build_phase(&record, isolation)?;
        let BrokerPhase::Authorizing { intent } = &request.phase else {
            return Err(unavailable("première phase de génération invalide"));
        };
        if intent.action_id != record.context.action_id
            || request.expected_revision != Revision::INITIAL
            || request.evaluation.previous_evaluation_id.is_some()
            || intent
                .manifest
                .get("isolation_requirements")
                .and_then(|requirements| requirements.get("mode"))
                .and_then(serde_json::Value::as_str)
                != Some(isolation.as_str())
        {
            return Err(unavailable(
                "identité de la première phase de génération divergente",
            ));
        }
        let preparation = &record.context.preparation;
        for hash in [
            preparation.fixture_digest,
            preparation.payload_manifest_hash,
            preparation.capability_snapshot_hash,
            preparation.model_policy_hash,
            preparation.data_policy_hash,
        ] {
            self.read_verified_object_bytes(hash)?;
        }
        let manifest = record.request_manifest()?;
        let candidate = publish(
            self,
            &self.config().id_generator.new_uuid().to_string(),
            "request_manifest",
            &manifest,
        )?;
        let wire = command.clone();
        let authority = caller.clone();
        let config = self.config().clone();
        let verified = candidate.clone();
        let result = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            let Some(current) = read_pending(tx, &daemon, now, &wire, &authority)? else {
                return Ok(None);
            };
            if current != record || canonical_sha256(&current.request_manifest()?)? != verified.hash
            {
                return Err(unavailable(
                    "manifest de génération changé avant Authorizing",
                ));
            }
            adopt_candidate(
                tx,
                &config,
                &daemon,
                &current.context.probe_id.to_string(),
                &verified,
            )?;
            crate::broker_writer::commit_phase_tx(tx, &config, now, &request).map(Some)
        });
        retire_candidates(self, &[candidate]);
        let committed = result?;
        if committed.is_some() {
            self.config()
                .cutpoints
                .hit(StoreCutpoint::C3AfterBrokerIntentCommit)?;
        }
        Ok(committed)
    }
}
