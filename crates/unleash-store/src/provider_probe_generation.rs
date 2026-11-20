//! Authenticated preparation of a bounded generation probe and its immutable inputs.
//! The existing dispatch gate stays closed until the native producers are qualified.

mod authorizing;

use crate::commands::{DaemonStateRow, EventAppendSpec, read_daemon_state_conn};
use crate::objects::{OwnedReferenceAdoption, PublishedObjectReference, tx_adopt_owned_reference};
use crate::provider_probe_mandates::{ResolvedConnectGrant, resolve_generation_grant};
use crate::{
    AggregateKind, CallerAuthority, CommandMutation, CommitCommandRequest, CommitOutcome,
    ConfiguredProviderProfile, PrepareObjectRequest, ProbeGenerationLimits,
    ProviderProbePreparation, PublishObjectReferenceRequest, StoreConfig, StoreError, UnleashStore,
    WriteCriticality,
};
use rusqlite::Transaction;
use serde_json::{Value, json};
use unleash_domain::{
    DataLocality, ModelPolicy, ProviderProbeId, ReceiptId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, ProviderAuthentication, ProviderTestPhase, ReceiptState,
    TestProviderProfilePayload, canonical_sha256, canonicalize_to_vec, compute_command_digest_v2,
};

const PREPARATION_OWNER: &str = "provider_generation_preparation";
const FIXTURE_ID: &str = "f012.synthetic_text.v1";

/// Immutable inputs retained by the authenticated preparation path.
#[derive(Debug, Clone)]
pub struct AuthenticatedGenerationPreparation {
    pub(crate) preparation: ProviderProbePreparation,
    command_digest: Sha256Digest,
    profile_digest: Sha256Digest,
    grant_digest: Sha256Digest,
    policy_digest: Sha256Digest,
    candidates: Vec<ArtifactCandidate>,
}

#[derive(Debug, Clone)]
struct ArtifactCandidate {
    field: &'static str,
    hash: Sha256Digest,
    reference: PublishedObjectReference,
}

struct Resolution {
    wire: TestProviderProfilePayload,
    profile: ConfiguredProviderProfile,
    grant: ResolvedConnectGrant,
    policy_digest: Sha256Digest,
}

fn unavailable(message: &'static str) -> StoreError {
    StoreError::EvidenceUnavailable(message.to_owned())
}

fn hidden() -> StoreError {
    StoreError::NotFound("Mandat d'effet fournisseur introuvable".into())
}

fn resolve_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<Resolution, StoreError> {
    crate::provider_profile_validation::validate_envelope_identity(command, caller)?;
    if command.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: command.authority_generation,
            command_id: Some(command.command_id),
        });
    }
    if daemon.mode != unleash_domain::DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "préparation génération exige daemon ready".into(),
        ));
    }
    let wire: TestProviderProfilePayload =
        serde_json::from_value(Value::Object(command.payload.clone())).map_err(|_| hidden())?;
    if wire.phase() != ProviderTestPhase::GenerationProbe {
        return Err(hidden());
    }
    let profile_id = wire.profile_id().parse().map_err(|_| hidden())?;
    let profile = crate::provider_profiles::resolve(tx, profile_id, wire.profile_version())?
        .ok_or_else(hidden)?;
    let grant = resolve_generation_grant(
        tx,
        daemon,
        now,
        &caller.actor_id,
        command.project_id,
        &wire,
        &profile,
    )?;
    if wire.capability_set().iter().any(|cap| cap != "text") {
        return Err(unavailable(
            "aucune fixture de génération pour les capacités demandées",
        ));
    }
    if profile.configuration.payload.locality() == DataLocality::LocalOnly
        && wire.limits().max_cost_usd_micros().as_u64() != 0
    {
        return Err(hidden());
    }
    let policy_digest = crate::provider_probe_connect::policy(tx, command.project_id)?.1;
    Ok(Resolution {
        wire,
        profile,
        grant,
        policy_digest,
    })
}

fn publish(
    store: &UnleashStore,
    owner_id: &str,
    field: &'static str,
    document: &Value,
) -> Result<ArtifactCandidate, StoreError> {
    let bytes = canonicalize_to_vec(document)?;
    let hash = Sha256Digest::from_bytes(&bytes);
    let handle = store.prepare_object(PrepareObjectRequest {
        object_hash: hash,
        byte_length: u64::try_from(bytes.len())
            .map_err(|_| unavailable("objet génération trop grand"))?,
        media_type: "application/json".into(),
        owner_kind: PREPARATION_OWNER.into(),
        owner_id: owner_id.into(),
    })?;
    let published = store
        .write_prepared_object_bytes(&handle, &bytes)
        .and_then(|()| {
            store.publish_owned_object_reference(PublishObjectReferenceRequest {
                pin_id: handle.pin_id.clone(),
                object_hash: hash,
                object_generation: handle.object_generation,
                owner_kind: PREPARATION_OWNER.into(),
                owner_id: owner_id.into(),
                source_field: field.into(),
            })
        });
    match published {
        Ok(reference) => Ok(ArtifactCandidate {
            field,
            hash,
            reference,
        }),
        Err(error) => {
            store.release_pin(&handle.pin_id)?;
            Err(error)
        }
    }
}

fn retire_candidates(store: &UnleashStore, candidates: &[ArtifactCandidate]) {
    for candidate in candidates {
        if let Err(error) = store.retire_owned_object_reference(
            candidate.reference.clone(),
            "generation_preparation_finished",
        ) {
            tracing::warn!(code=%error.error_code(), "Retrait d'un candidat de génération non confirmé");
        }
    }
}

fn limits(resolved: &Resolution) -> ProbeGenerationLimits {
    let wire = resolved.wire.limits();
    let profile = resolved.profile.configuration.payload.limits();
    ProbeGenerationLimits {
        timeout_ms: wire.timeout_ms().min(profile.generation_timeout_ms),
        max_calls: wire.max_calls(),
        max_output_tokens: wire.max_output_tokens(),
        max_cost_usd_micros: wire.max_cost_usd_micros(),
    }
}

fn build_preparation(
    store: &UnleashStore,
    command: &CommandEnvelope,
    resolved: Resolution,
    candidates: &mut Vec<ArtifactCandidate>,
) -> Result<AuthenticatedGenerationPreparation, StoreError> {
    let owner = store.config().id_generator.new_uuid().to_string();
    let profile = &resolved.profile.configuration.payload;
    let effective = limits(&resolved);
    let fixture = json!({
        "schema_version":1,"fixture_id":FIXTURE_ID,
        "messages":[{"role":"user","content":"Reply with the word OK."}],
    });
    let fixture_digest = canonical_sha256(&fixture)?;
    let docs = [
        ("fixture", fixture),
        (
            "request_payload",
            json!({
                "schema_version":1,"fixture_digest":fixture_digest,"model":profile.model(),
                "profile_digest":resolved.profile.configuration_digest,"limits":effective,
            }),
        ),
        (
            "capabilities",
            json!({
                "schema_version":1,"requested":resolved.wire.capability_set(),
                "declared":profile.declared_capabilities(),"observed":[],
                "unknown":resolved.wire.capability_set(),
            }),
        ),
        (
            "model_policy",
            json!({
                "schema_version":1,"origin":"independent_control_plane",
                "policy":ModelPolicy::AllowedBounded,"security_policy_digest":resolved.policy_digest,
                "mandate_digest":resolved.grant.mandate_digest,
            }),
        ),
        (
            "data_policy",
            json!({
                "schema_version":1,"locality_required":resolved.wire.locality(),
                "observed_placement":null,"source":"synthetic_fixture",
            }),
        ),
    ];
    for (field, doc) in docs {
        candidates.push(publish(store, &owner, field, &doc)?);
    }
    let hash = |field| {
        candidates
            .iter()
            .find(|candidate| candidate.field == field)
            .map(|candidate| candidate.hash)
            .ok_or_else(|| unavailable("pièce de génération absente"))
    };
    let secret_refs = match profile.auth() {
        ProviderAuthentication::None => vec![],
        ProviderAuthentication::SecretRef { secret_ref } => vec![secret_ref.clone()],
        _ => return Err(unavailable("authentification du profil non reconnue")),
    };
    Ok(AuthenticatedGenerationPreparation {
        preparation: ProviderProbePreparation {
            requested_profile_id: resolved.wire.profile_id().into(),
            requested_profile_version: resolved.wire.profile_version(),
            profile_version_ref: resolved.profile.profile_version_ref,
            resolved_model_id: profile.model().into(),
            capability_set: resolved.wire.capability_set().to_vec(),
            capability_snapshot_hash: hash("capabilities")?,
            fixture_id: FIXTURE_ID.into(),
            fixture_digest: hash("fixture")?,
            payload_manifest_hash: hash("request_payload")?,
            model_policy_hash: hash("model_policy")?,
            data_policy_hash: hash("data_policy")?,
            model_policy: ModelPolicy::AllowedBounded,
            locality: resolved.wire.locality(),
            mandate_ref: resolved.grant.mandate_ref.to_string(),
            source_run_id: None,
            secret_refs,
            limits: effective,
        },
        command_digest: compute_command_digest_v2(command)?,
        profile_digest: resolved.profile.configuration_digest,
        grant_digest: resolved.grant.mandate_digest,
        policy_digest: resolved.policy_digest,
        candidates: candidates.clone(),
    })
}

impl UnleashStore {
    /// Prepares and commits the exact authorized context with its synthetic inputs.
    ///
    /// # Errors
    ///
    /// Rejects a mismatched, expired or revoked mandate or unavailable evidence.
    pub fn commit_provider_generation_preparation(
        &self,
        command: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<CommitOutcome, StoreError> {
        if let Some(receipt) = self.lookup_provider_validation_command(command, caller)? {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        let envelope = command.clone();
        let authority = caller.clone();
        let resolved = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            resolve_tx(tx, &daemon, now, &envelope, &authority)
        })?;
        crate::provider_profiles::verify_configuration_objects(
            self,
            &resolved.profile.configuration,
            resolved.profile.configuration_digest,
        )?;
        let mut candidates = Vec::new();
        let outcome =
            build_preparation(self, command, resolved, &mut candidates).and_then(|preparation| {
                self.commit_command(CommitCommandRequest {
                    actor_id: caller.actor_id.clone(),
                    envelope: command.clone(),
                    target_aggregate: AggregateKind::ProviderProbe,
                    operation_id: None,
                    receipt_state: ReceiptState::Accepted,
                    result_ref: Some(format!("provider-probe:{}", command.aggregate_id)),
                    retention_days_override: None,
                    fence_assertion: None,
                    mutation: CommandMutation::PrepareAuthenticatedProviderGeneration {
                        caller: caller.clone(),
                        preparation: Box::new(preparation),
                    },
                    events: vec![],
                    capacity_reservations: vec![],
                    budget_reservations: vec![],
                })
            });
        retire_candidates(self, &candidates);
        if outcome.is_err()
            && let Some(receipt) = self.lookup_provider_validation_command(command, caller)?
        {
            return Ok(CommitOutcome::IdempotentReplay(receipt));
        }
        outcome
    }
}

pub(crate) struct GenerationCommit<'a> {
    pub(crate) command: &'a CommandEnvelope,
    pub(crate) caller: &'a CallerAuthority,
    pub(crate) receipt: ReceiptId,
    pub(crate) preparation: &'a AuthenticatedGenerationPreparation,
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    input: &GenerationCommit<'_>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let resolved = resolve_tx(tx, daemon, now, input.command, input.caller)?;
    let prepared = input.preparation;
    if compute_command_digest_v2(input.command)? != prepared.command_digest
        || resolved.profile.configuration_digest != prepared.profile_digest
        || resolved.grant.mandate_digest != prepared.grant_digest
        || resolved.policy_digest != prepared.policy_digest
        || limits(&resolved) != prepared.preparation.limits
    {
        return Err(unavailable(
            "contexte authentifié de génération modifié avant commit",
        ));
    }
    let probe = ProviderProbeId::from_uuid(input.command.aggregate_id);
    let revision = crate::provider_probes::prepare_tx(
        tx,
        config,
        daemon,
        now,
        probe,
        input.receipt,
        &input.caller.actor_id,
        input.command.project_id,
        &prepared.preparation,
        Some(&resolved.grant),
        events,
    )?;
    let owner_id = probe.to_string();
    for candidate in &prepared.candidates {
        adopt_candidate(tx, config, daemon, &owner_id, candidate)?;
    }
    Ok(revision)
}

fn adopt_candidate(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    owner_id: &str,
    candidate: &ArtifactCandidate,
) -> Result<(), StoreError> {
    tx_adopt_owned_reference(
        tx,
        config.id_generator.as_ref(),
        daemon.authority_generation,
        &OwnedReferenceAdoption {
            candidate: &candidate.reference,
            object_hash: candidate.hash,
            producer_kind: PREPARATION_OWNER,
            producer_field: candidate.field,
            owner_kind: "provider_probe",
            owner_id,
            source_field: candidate.field,
        },
    )?;
    Ok(())
}
