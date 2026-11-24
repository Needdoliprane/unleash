//! Present a staged probe to the scheduler only inside its owner's admission transaction.

use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, Transaction, params};
use unleash_domain::{Sha256Digest, TimestampMs};
use unleash_protocol::{CommandEnvelope, canonicalize_to_vec};
use uuid::Uuid;

use super::{
    AdmittedOllamaChatInputs, FIELD, MAX_PLACEMENT_BYTES, OBSERVER, Observation, PREFIX,
    admitted_chat_inputs, clear_placement_wait, observe_eligibility, read_bound_record,
    read_daemon_state_conn, read_verified_evidence_bytes_bounded, unavailable,
    verify_live_generation, verify_observation,
};
use crate::{
    AdmissionCommit, AdmissionDecision, AdmissionOutcome, AdmissionUnit, CallerAuthority,
    ExecutionAdmissionSnapshot, PreparedProviderProbe, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};

/// The exact held admission and the frozen inputs ready for the separate transport authorization.
#[must_use = "the generation executor owns the reserved continuation"]
pub struct ManagedGenerationAdmission {
    /// Committed lease, reservation and observation cursor.
    pub commit: Box<AdmissionCommit>,
    /// Revalidated original payload bytes and managed placement.
    pub inputs: AdmittedOllamaChatInputs,
}

fn read_staged_fact(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    probe: &PreparedProviderProbe,
    evidence: Uuid,
) -> Result<(Observation, Sha256Digest), StoreError> {
    let hash: String = tx
        .query_row(
            "SELECT e.object_hash FROM evidence e
             JOIN principals s ON s.principal_id=e.producer_id
             JOIN provider_probes p ON p.probe_id=?1
             JOIN object_references r ON r.owner_kind='provider_probe' AND r.owner_id=p.probe_id
              AND r.object_hash=e.object_hash AND r.source_field=?2 AND r.state='live'
             WHERE e.evidence_id=?3 AND e.action_id=?4 AND e.producer_id=?5
               AND e.snapshot_ref GLOB (?6 || '*') AND e.kind='observation'
               AND e.schema_version=1 AND e.availability='available'
               AND s.principal_kind='system' AND s.actor_id=s.principal_id AND s.state='active'
               AND p.state='queued' AND p.wait_reason=?7 AND p.fence=0",
            params![
                probe.context.probe_id.to_string(),
                FIELD,
                evidence.to_string(),
                probe.context.action_id.to_string(),
                OBSERVER,
                format!("{PREFIX}{}/", probe.context_digest),
                crate::provider_probes::WAIT_FOR_PLACEMENT,
            ],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| unavailable("placement géré en attente indisponible"))?;
    let hash = hash.parse()?;
    let bytes = read_verified_evidence_bytes_bounded(tx, config, hash, MAX_PLACEMENT_BYTES)?;
    let fact: Observation = serde_json::from_slice(&bytes)?;
    if fact.managed_launch.is_none() || canonicalize_to_vec(&fact)? != bytes {
        return Err(unavailable("preuve de placement géré divergente"));
    }
    Ok((fact, hash))
}

fn handoff<F, R>(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    now: TimestampMs,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
    evidence: Uuid,
    decide: F,
) -> Result<(Option<ManagedGenerationAdmission>, R), StoreError>
where
    F: FnOnce(
        &ExecutionAdmissionSnapshot,
        AdmissionUnit,
    ) -> Result<(AdmissionDecision, R), StoreError>,
{
    if config.clock_uncertain.load(Ordering::Acquire) {
        return Err(unavailable("horloge du placement incertaine"));
    }
    let daemon = read_daemon_state_conn(tx)?;
    let record = read_bound_record(tx, &daemon, command, caller)?;
    let resolved = verify_live_generation(tx, &daemon, now, command, caller, &record)?;
    let (fact, hash) = read_staged_fact(tx, config, &record, evidence)?;
    verify_observation(tx, &daemon, now, &record, &resolved.profile, &fact)?;
    crate::provider_probes::verify_frozen_inputs(tx, &record)?;
    let pool = crate::budgets::tx_global_execution_pool_id(tx)?
        .ok_or_else(|| unavailable("pool exécution absent avant admission génération"))?;

    // An unowned or waiting candidate never becomes visible to another executor.
    tx.execute_batch("SAVEPOINT managed_generation_admission")?;
    if clear_placement_wait(tx, fact.probe_id)? != 1 {
        return Err(unavailable("le probe n'attend plus son placement"));
    }
    observe_eligibility(
        tx,
        config,
        now,
        &resolved.profile,
        fact.server.key()?,
        &fact,
        hash,
    )?;
    let snapshot = crate::admission::read_snapshot(tx, pool, now)?;
    let unit = AdmissionUnit::Probe(fact.probe_id);
    let (outcome, report) = crate::admission::commit(
        tx,
        config,
        now,
        pool,
        snapshot.token,
        |current| {
            let (decision, report) = decide(current, unit)?;
            if matches!(&decision, AdmissionDecision::Admit { unit: selected, .. } if *selected != unit)
            {
                return Err(unavailable("sélection étrangère au relais du probe"));
            }
            Ok((decision, report))
        },
    )?;
    let admitted = match outcome {
        AdmissionOutcome::Unchanged => {
            tx.execute_batch("ROLLBACK TO managed_generation_admission")?;
            None
        }
        AdmissionOutcome::Admitted(commit) => {
            let inputs = admitted_chat_inputs(tx, config, config.clock.now_ms(), &commit, true)?;
            Some(ManagedGenerationAdmission { commit, inputs })
        }
    };
    tx.execute_batch("RELEASE managed_generation_admission")?;
    Ok((admitted, report))
}

impl UnleashStore {
    /// Atomically present and admit a staged managed probe for its waiting executor.
    ///
    /// The decision callback runs under the writer and must stay pure. An unchanged decision
    /// leaves the original proof staged. The admitted result belongs to the executor that asked.
    ///
    /// # Errors
    /// Refuses divergent or expired evidence, a second claim and admission of any other unit.
    pub fn admit_staged_managed_generation<F, R>(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
        evidence: Uuid,
        decide: F,
    ) -> Result<(Option<ManagedGenerationAdmission>, R), StoreError>
    where
        F: FnOnce(
                &ExecutionAdmissionSnapshot,
                AdmissionUnit,
            ) -> Result<(AdmissionDecision, R), StoreError>
            + Send
            + 'static,
        R: Send + 'static,
    {
        crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
        let command = command.clone();
        let caller = caller.clone();
        let config = self.config().clone();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            handoff(tx, &config, now, &command, &caller, evidence, decide)
        })
    }
}
