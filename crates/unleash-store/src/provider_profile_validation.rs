//! Validation locale F012 : aucun appel fournisseur ni qualification implicite.
//! Les octets sont vérifiés hors du writer ; celui-ci revalide la causalité,
//! l'autorité et les liaisons avant l'engagement indivisible du résultat.

use crate::commands::{
    AggregateKind, CommandMutation, CommitCommandRequest, DaemonStateRow, EventAppendSpec,
    ensure_scope_not_frozen_tx,
};
use crate::policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_security_admin};
use crate::{CallerAuthority, ConfiguredProviderProfile, StoreConfig, StoreError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unleash_domain::{
    ActorId, AuthorityGeneration, DaemonEpoch, DaemonMode, ProjectId, ProviderProbeId, ReceiptId,
    Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProviderTestPhase, ReceiptState, Sensitivity,
    TestProviderProfilePayload, canonical_sha256, canonicalize_to_vec,
};
use uuid::Uuid;

/// Observation des seuls contrôles locaux d'une version configurée exacte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalProviderValidation {
    /// Version fermée du document.
    pub schema_version: u32,
    /// Identité de validation égale à l'agrégat de commande.
    pub probe_id: ProviderProbeId,
    /// Profil configuré, sans résolution latest.
    pub profile_id: Uuid,
    /// Version exacte.
    pub profile_version: u64,
    /// Référence durable opaque.
    pub profile_version_ref: String,
    /// Empreinte exacte des octets de configuration vérifiés.
    pub configuration_digest: Sha256Digest,
    /// Reçu causal de la configuration, distinct du reçu de cette validation.
    pub configuration_receipt_id: ReceiptId,
    /// Auteur historique de la version configurée.
    pub configuration_actor_id: ActorId,
    /// Principal historique du configurateur.
    pub configuration_principal_id: ActorId,
    /// Auteur courant authentifié.
    pub actor_id: ActorId,
    /// Principal courant résolu par le Store.
    pub principal_id: ActorId,
    /// Portée authentifiée exacte.
    pub project_id: Option<ProjectId>,
    /// Lignée de cette observation locale.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation de cette observation locale.
    pub daemon_epoch: DaemonEpoch,
    /// Phase effectivement exécutée.
    pub phase: ProviderTestPhase,
    /// Statut limité aux contrôles locaux.
    pub status: LocalValidationStatus,
    /// Instant de vérification des octets, hors transaction.
    pub observed_at_ms: TimestampMs,
    /// Phases qui ne sont pas exécutées par ce résultat.
    pub untested_phases: [ProviderTestPhase; 3],
}

/// Statut local, distinct de la qualification du fournisseur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalValidationStatus {
    /// Les contrôles locaux ont réellement réussi.
    Validated,
    /// The bounded native TCP/TLS connect completed; no model capability is qualified.
    Connected,
    /// The complete native inventory was retained and the probe disposed.
    InventoryObserved,
}

/// Objet local publié avant la mutation, sans reçu préalloué.
#[derive(Debug, Clone)]
pub struct ProviderValidationPreparation {
    /// Faits observés et identité serveur.
    pub observation: LocalProviderValidation,
    /// Objet publié contenant exactement ce document canonique.
    pub content_object_hash: Sha256Digest,
}

/// Résumé public d'une observation locale conservée.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderValidationSummary {
    /// Phase réellement exécutée.
    pub phase: ProviderTestPhase,
    /// Identité du résultat.
    pub probe_id: ProviderProbeId,
    /// Référence canonique vers ce résultat.
    pub result_ref: String,
    /// Date réellement observée.
    pub observed_at_ms: TimestampMs,
    /// Résultat limité à cette phase.
    pub status: LocalValidationStatus,
    /// Exact retained inventory, present only for an observed inventory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<ProviderInventoryView>,
}

/// Read-only projection of the verified native pair.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInventoryView {
    /// Writer identity and digest of the retained fact.
    pub fact_ref: ProviderInventoryFactRef,
    /// Digest of the canonical list bound by the fact.
    pub inventory_digest: Sha256Digest,
    /// Complete bounded native declarations.
    pub models: Vec<unleash_protocol::provider_inventory::OllamaListedModel>,
}

/// Closed reference to the observed inventory fact.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInventoryFactRef {
    /// Evidence row identity.
    pub evidence_id: Uuid,
    /// Verified canonical object digest.
    pub object_hash: Sha256Digest,
}

/// Projection autorisable d'une version exacte ; aucun détail privé de liaison.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderProfileView {
    /// UUID canonique du profil.
    pub profile_id: Uuid,
    /// Version exacte demandée.
    pub profile_version: u64,
    /// Référence durable exacte.
    pub profile_version_ref: String,
    /// Empreinte de configuration immuable.
    pub configuration_digest: Sha256Digest,
    /// Reçu authentique de configuration.
    pub configuration_receipt_id: ReceiptId,
    /// La validation locale ne qualifie pas un fournisseur.
    pub qualification: crate::ProviderProfileQualification,
    /// Dernière observation conservée pour chaque phase qualifiée.
    pub observations: Vec<ProviderValidationSummary>,
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

pub(crate) fn validate_envelope_identity(
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<(), StoreError> {
    require_security_admin(caller, "TestProviderProfile")?;
    if envelope.command_type != "TestProviderProfile"
        || envelope.aggregate_id.is_nil()
        || envelope
            .actor_ref
            .as_ref()
            .is_some_and(|actor| actor != caller.actor_id.as_str())
    {
        return Err(invalid("identité de validation locale incohérente"));
    }
    Ok(())
}

pub(crate) fn validate_identity(request: &CommitCommandRequest) -> Result<bool, StoreError> {
    let CommandMutation::ValidateProviderProfile { caller, .. } = &request.mutation else {
        return Ok(false);
    };
    validate_envelope_identity(&request.envelope, caller)?;
    if request.actor_id != caller.actor_id
        || request.target_aggregate != AggregateKind::ProviderProbe
    {
        return Err(invalid("autorité de validation locale incohérente"));
    }
    Ok(true)
}

fn payload(
    envelope: &CommandEnvelope,
) -> Result<(TestProviderProfilePayload, Uuid, ReceiptId), StoreError> {
    let wire: TestProviderProfilePayload =
        serde_json::from_value(Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("payload de validation locale invalide"))?;
    let profile: Uuid = wire
        .profile_id()
        .parse()
        .map_err(|_| invalid("UUID de profil invalide"))?;
    let receipt: ReceiptId = wire
        .mandate_ref()
        .parse()
        .map_err(|_| invalid("reçu causal de configuration invalide"))?;
    if wire.phase() != ProviderTestPhase::ValidateConfig
        || profile.is_nil()
        || profile.to_string() != wire.profile_id()
        || receipt.to_string() != wire.mandate_ref()
    {
        return Err(invalid(
            "phase locale ou limites de validation incompatibles",
        ));
    }
    Ok((wire, profile, receipt))
}

fn source_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<(ConfiguredProviderProfile, ActorId), StoreError> {
    validate_envelope_identity(envelope, caller)?;
    if envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "validation locale exige daemon ready".to_owned(),
        ));
    }
    let (wire, profile_id, receipt) = payload(envelope)?;
    let profile = crate::provider_profiles::resolve(tx, profile_id, wire.profile_version())?
        .ok_or_else(|| StoreError::NotFound("version de profil indisponible".to_owned()))?;
    if profile.configuration.project_id != envelope.project_id || profile.receipt_id != receipt {
        return Err(StoreError::NotFound(
            "source de configuration hors portée ou divergente".to_owned(),
        ));
    }
    let source: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts WHERE receipt_id=?1 AND command_type='SetProviderProfile' AND aggregate_id=?2 AND actor_id=?3 AND authority_generation=?4 AND state IN ('accepted','completed'))", params![receipt.to_string(),profile_id.to_string(),profile.actor_id.as_str(),daemon.authority_generation.to_string()], |row| row.get(0))?;
    if !source {
        return Err(StoreError::NotFound(
            "reçu causal de configuration non courant".to_owned(),
        ));
    }
    if let Some(project) = envelope.project_id {
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1 AND state='active')",
            [project.to_string()],
            |row| row.get(0),
        )?;
        if !active {
            return Err(StoreError::NotFound("projet indisponible".to_owned()));
        }
    }
    ensure_scope_not_frozen_tx(tx, envelope.project_id, None, None)?;
    let principal: String = tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind IN ('user','service','system')", [caller.actor_id.as_str()], |row| row.get(0)).optional()?.ok_or_else(|| invalid("principal authentifié courant absent"))?;
    for subject in [principal.as_str(), caller.actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, envelope.project_id)?;
    }
    crate::provider_profiles::verify_secret(tx, &profile.configuration)?;
    Ok((profile, ActorId::new(principal)?))
}

pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<(LocalProviderValidation, ConfiguredProviderProfile), StoreError> {
    let (profile, principal) = source_tx(tx, daemon, envelope, caller)?;
    let observation = LocalProviderValidation {
        schema_version: 1,
        probe_id: ProviderProbeId::from_uuid(envelope.aggregate_id),
        profile_id: profile.configuration.profile_id,
        profile_version: profile.configuration.profile_version,
        profile_version_ref: profile.profile_version_ref.clone(),
        configuration_digest: profile.configuration_digest,
        configuration_receipt_id: profile.receipt_id,
        configuration_actor_id: profile.actor_id.clone(),
        configuration_principal_id: profile.principal_id.clone(),
        actor_id: caller.actor_id.clone(),
        principal_id: principal,
        project_id: envelope.project_id,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        phase: ProviderTestPhase::ValidateConfig,
        status: LocalValidationStatus::Validated,
        observed_at_ms: now,
        untested_phases: [
            ProviderTestPhase::Connect,
            ProviderTestPhase::Inventory,
            ProviderTestPhase::GenerationProbe,
        ],
    };
    Ok((observation, profile))
}

pub(crate) fn validate_new_command(request: &CommitCommandRequest) -> Result<(), StoreError> {
    let CommandMutation::ValidateProviderProfile { preparation, .. } = &request.mutation else {
        return Ok(());
    };
    validate_identity(request)?;
    let (wire, profile, source) = payload(&request.envelope)?;
    let observation = &preparation.observation;
    if request.envelope.expected_revision != Revision::INITIAL
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
        || request.fence_assertion.is_some()
        || request.receipt_state != ReceiptState::Completed
        || request.result_ref != Some(format!("provider-probe:{}", observation.probe_id))
        || observation.probe_id.as_uuid() != request.envelope.aggregate_id
        || observation.profile_id != profile
        || observation.profile_version != wire.profile_version()
        || observation.configuration_receipt_id != source
        || canonical_sha256(observation)? != preparation.content_object_hash
    {
        return Err(invalid("préparation de validation locale incohérente"));
    }
    Ok(())
}

pub(crate) fn verify_published(
    store: &crate::UnleashStore,
    request: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let CommandMutation::ValidateProviderProfile { preparation, .. } = &request.mutation else {
        return Ok(());
    };
    validate_new_command(request)?;
    let bytes = store.read_verified_object_bytes(preparation.content_object_hash)?;
    if bytes != canonicalize_to_vec(&preparation.observation)? {
        return Err(invalid("objet de validation locale divergent"));
    }
    let source = store
        .resolve_provider_profile_snapshot(
            preparation.observation.profile_id,
            preparation.observation.profile_version,
        )?
        .value
        .ok_or_else(|| StoreError::NotFound("configuration indisponible".to_owned()))?;
    verify_configuration_bytes(store, &source)
}

pub(crate) fn verify_configuration_bytes(
    store: &crate::UnleashStore,
    source: &ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    crate::provider_profiles::verify_configuration_objects(
        store,
        &source.configuration,
        source.configuration_digest,
    )
}

/// Entrées empruntées de la seule mutation locale.
#[derive(Clone, Copy)]
pub(crate) struct ValidationCommand<'a> {
    pub envelope: &'a CommandEnvelope,
    pub caller: &'a CallerAuthority,
    pub preparation: &'a ProviderValidationPreparation,
}

fn validate_observation_time(
    tx: &Transaction<'_>,
    observation: &LocalProviderValidation,
    now: TimestampMs,
) -> Result<(), StoreError> {
    let earliest: i64 = tx.query_row(
        "SELECT MAX(d.started_at_ms,c.committed_at_ms) FROM daemon_state d JOIN command_receipts c ON c.receipt_id=?1 WHERE d.singleton=1",
        [observation.configuration_receipt_id.to_string()],
        |row| row.get(0),
    )?;
    if observation.observed_at_ms.as_i64() < earliest
        || observation.observed_at_ms.as_i64() > now.as_i64()
    {
        return Err(invalid("date de validation locale non attribuable"));
    }
    Ok(())
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    command: ValidationCommand<'_>,
    receipt: ReceiptId,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let (expected, _) = prepare_tx(
        tx,
        daemon,
        command.preparation.observation.observed_at_ms,
        command.envelope,
        command.caller,
    )?;
    let observation = &command.preparation.observation;
    if &expected != observation {
        return Err(invalid("faits de validation locale périmés ou divergents"));
    }
    validate_observation_time(tx, observation, now)?;
    let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)", [observation.probe_id.to_string()], |row| row.get(0))?;
    if present {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: Revision::INITIAL,
            command_id: Some(command.envelope.command_id),
        });
    }
    let hash = command.preparation.content_object_hash;
    let live: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
        [hash.to_string()],
        |row| row.get(0),
    )?;
    if !live {
        return Err(invalid("observation locale non publiée"));
    }
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,source_run_id,profile_version_ref,context_hash,phase,state,wait_reason,admission_class,authority_generation,daemon_epoch,fence,revision) VALUES(?1,?2,?3,NULL,?4,?5,'validate_config','ended',NULL,NULL,?6,?7,0,1)", params![observation.probe_id.to_string(),receipt.to_string(),observation.principal_id.as_str(),observation.profile_version_ref,hash.to_string(),daemon.authority_generation.to_string(),i64::try_from(daemon.daemon_epoch.as_u64()).map_err(|_| invalid("époque hors plage"))?])?;
    tx.execute("INSERT INTO object_references(reference_id,object_hash,owner_kind,owner_id,source_field,state) VALUES(?1,?2,'provider_probe',?3,'validate_config','live')", params![config.id_generator.new_uuid().to_string(),hash.to_string(),observation.probe_id.to_string()])?;
    events.push(EventAppendSpec {
        event_type: "ProviderProfileValidated".to_owned(),
        aggregate_type: AggregateKind::ProviderProbe,
        aggregate_id: observation.probe_id.as_uuid(),
        aggregate_revision: Revision::new(1),
        project_id: observation.project_id,
        correlation_id: command.envelope.command_id.as_uuid(),
        causation_kind: EventCausationKind::Command,
        causation_id: None,
        payload: serde_json::to_value(observation)?
            .as_object()
            .cloned()
            .ok_or_else(|| invalid("observation locale invalide"))?,
        sensitivity: Sensitivity::Restricted,
        outbox_destinations: vec!["clients".to_owned()],
    });
    Ok(Revision::new(1))
}

pub(crate) struct ProfileProjection {
    profile: ConfiguredProviderProfile,
    latest: Option<LatestValidation>,
    connect: Option<crate::provider_probe_connect_observation::ConnectProjection>,
    inventory: Option<crate::provider_probe_inventory_observation::InventoryProjection>,
}

struct LatestValidation {
    probe_id: String,
    object_hash: String,
    principal_id: String,
    actor_id: String,
    authority_generation: String,
    daemon_epoch: i64,
    committed_at_ms: i64,
    command_type: String,
}

pub(crate) fn read_projection(
    conn: &Connection,
    profile_id: Uuid,
    version: u64,
    project: Option<ProjectId>,
) -> Result<Option<ProfileProjection>, StoreError> {
    let Some(profile) = crate::provider_profiles::resolve(conn, profile_id, version)? else {
        return Ok(None);
    };
    if profile.configuration.project_id != project {
        return Ok(None);
    }
    // Le payload de notification est purgeable ; le dernier résultat durable
    // reste choisi même si ses octets ont depuis disparu. Aucun fallback ancien.
    let latest = conn.query_row(
        "SELECT p.probe_id,p.context_hash,p.principal_id,c.actor_id,p.authority_generation,p.daemon_epoch,c.committed_at_ms,c.command_type FROM provider_probes p JOIN command_receipts c ON c.receipt_id=p.command_receipt_id WHERE p.profile_version_ref=?1 AND p.phase='validate_config' AND p.state='ended' ORDER BY c.last_event_seq DESC,p.probe_id DESC LIMIT 1",
        [&profile.profile_version_ref],
        |row| Ok(LatestValidation {
            probe_id: row.get(0)?, object_hash: row.get(1)?, principal_id: row.get(2)?,
            actor_id: row.get(3)?, authority_generation: row.get(4)?, daemon_epoch: row.get(5)?,
            committed_at_ms: row.get(6)?, command_type: row.get(7)?,
        }),
    ).optional()?;
    let (connect, inventory) =
        if crate::commands::read_daemon_state_conn(conn)?.schema_version >= 15 {
            (
                crate::provider_probe_connect_observation::read_projection(
                    conn,
                    &profile.profile_version_ref,
                )?,
                crate::provider_probe_inventory_observation::read_projection(
                    conn,
                    &profile.profile_version_ref,
                )?,
            )
        } else {
            (None, None)
        };
    Ok(Some(ProfileProjection {
        profile,
        latest,
        connect,
        inventory,
    }))
}

fn validate_projected_observation(
    observed: &LocalProviderValidation,
    latest: &LatestValidation,
    profile: &ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    if observed.schema_version != 1
        || observed.probe_id.to_string() != latest.probe_id
        || observed.principal_id.as_str() != latest.principal_id
        || observed.actor_id.as_str() != latest.actor_id
        || observed.authority_generation.to_string() != latest.authority_generation
        || i64::try_from(observed.daemon_epoch.as_u64()).ok() != Some(latest.daemon_epoch)
        || observed.observed_at_ms.as_i64() > latest.committed_at_ms
        || latest.command_type != "TestProviderProfile"
        || observed.profile_id != profile.configuration.profile_id
        || observed.profile_version != profile.configuration.profile_version
        || observed.configuration_digest != profile.configuration_digest
        || observed.configuration_receipt_id != profile.receipt_id
        || observed.configuration_actor_id != profile.actor_id
        || observed.configuration_principal_id != profile.principal_id
        || observed.profile_version_ref != profile.profile_version_ref
        || observed.project_id != profile.configuration.project_id
        || observed.phase != ProviderTestPhase::ValidateConfig
        || observed.status != LocalValidationStatus::Validated
        || observed.untested_phases
            != [
                ProviderTestPhase::Connect,
                ProviderTestPhase::Inventory,
                ProviderTestPhase::GenerationProbe,
            ]
    {
        return Err(StoreError::EvidenceUnavailable(
            "faits du résultat local incohérents".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn finish_view(
    store: &crate::UnleashStore,
    projection: ProfileProjection,
) -> Result<ProviderProfileView, StoreError> {
    let ProfileProjection {
        profile,
        latest,
        connect,
        inventory,
    } = projection;
    let mut observations = Vec::new();
    if let Some(latest) = latest {
        let observed = read_observation(store, &latest)?;
        validate_projected_observation(&observed, &latest, &profile)?;
        observations.push(ProviderValidationSummary {
            phase: observed.phase,
            probe_id: observed.probe_id,
            result_ref: format!("provider-probe:{}", observed.probe_id),
            observed_at_ms: observed.observed_at_ms,
            status: observed.status,
            inventory: None,
        });
    }
    if let Some(connect) = connect {
        observations
            .push(crate::provider_probe_connect_observation::finish_projection(store, &connect)?);
    }
    if let Some(inventory) = inventory {
        observations.push(
            crate::provider_probe_inventory_observation::finish_projection(store, &inventory)?,
        );
    }
    Ok(ProviderProfileView {
        profile_id: profile.configuration.profile_id,
        profile_version: profile.configuration.profile_version,
        profile_version_ref: profile.profile_version_ref,
        configuration_digest: profile.configuration_digest,
        configuration_receipt_id: profile.receipt_id,
        qualification: profile.qualification,
        observations,
    })
}

fn read_observation(
    store: &crate::UnleashStore,
    latest: &LatestValidation,
) -> Result<LocalProviderValidation, StoreError> {
    let unavailable = || StoreError::EvidenceUnavailable("résultat local indisponible".to_owned());
    let hash: Sha256Digest = latest.object_hash.parse().map_err(|_| unavailable())?;
    let pin = store
        .acquire_read_pin(hash, "provider_probe_inspection", &latest.probe_id)
        .map_err(|_| unavailable())?;
    let result = (|| {
        let bytes = store
            .read_verified_object_bytes(hash)
            .map_err(|_| unavailable())?;
        let observed: LocalProviderValidation =
            serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if canonical_sha256(&observed)? != hash {
            return Err(unavailable());
        }
        Ok(observed)
    })();
    let released = store.release_pin(&pin);
    let observed = result?;
    released?;
    Ok(observed)
}
