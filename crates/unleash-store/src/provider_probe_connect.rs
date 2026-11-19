//! Préparation F012 d'un contexte de transport, distinct du contexte modèle V009.
//! Le writer commun doit encore raccorder la migration et CommitCommand.
//! Aucun état d'action, effet réseau ou réserve n'est créé par la résolution.

use crate::{CallerAuthority, SnapshotProjection, StoreError, UnleashStore};

/// Additive transport context; deliberately separate from V009 model contexts.
pub(crate) const SCHEMA_SQL: &str = r"
CREATE TABLE provider_probe_transport_contexts (
 probe_id TEXT PRIMARY KEY REFERENCES provider_probes(probe_id),
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 project_id TEXT REFERENCES projects(project_id),
 profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
 authority_generation TEXT NOT NULL,
 daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
 action_id TEXT NOT NULL UNIQUE,
 mandate_ref TEXT NOT NULL REFERENCES provider_probe_mandates(mandate_ref),
 phase TEXT NOT NULL CHECK(phase IN ('connect','inventory')),
 context_digest TEXT NOT NULL UNIQUE,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 created_at_ms INTEGER NOT NULL,
 claim_state TEXT NOT NULL DEFAULT 'reserved' CHECK(claim_state IN ('reserved','claimed')),
 claim_revision INTEGER NOT NULL DEFAULT 0 CHECK(claim_revision>=0),
 claim_id TEXT UNIQUE,
 claimed_at_ms INTEGER,
 CHECK((claim_state='reserved' AND claim_revision=0 AND claim_id IS NULL AND claimed_at_ms IS NULL)
  OR (claim_state='claimed' AND claim_revision=1 AND claim_id IS NOT NULL AND claimed_at_ms IS NOT NULL)),
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
  AND json_extract(context_json,'$.probe_id')=probe_id
  AND json_extract(context_json,'$.receipt_id')=receipt_id
  AND json_extract(context_json,'$.actor_id')=actor_id
  AND json_extract(context_json,'$.principal_id')=principal_id
  AND json_extract(context_json,'$.project_id') IS project_id
  AND json_extract(context_json,'$.profile_version_ref')=profile_version_ref
  AND json_extract(context_json,'$.authority_generation')=authority_generation
  AND json_extract(context_json,'$.daemon_epoch')=daemon_epoch
  AND json_extract(context_json,'$.action_id')=action_id
  AND json_extract(context_json,'$.phase')=phase
  AND json_extract(context_json,'$.mandate_ref')=mandate_ref,0))
) STRICT;
CREATE TRIGGER provider_probe_transport_exclusive BEFORE INSERT ON provider_probe_transport_contexts
WHEN EXISTS(SELECT 1 FROM provider_probe_contexts WHERE probe_id=NEW.probe_id)
 OR NOT EXISTS(SELECT 1 FROM provider_probes p WHERE p.probe_id=NEW.probe_id
  AND p.command_receipt_id=NEW.receipt_id AND p.principal_id=NEW.principal_id
  AND p.profile_version_ref=NEW.profile_version_ref AND p.phase=NEW.phase
  AND p.authority_generation=NEW.authority_generation AND p.daemon_epoch=NEW.daemon_epoch)
BEGIN SELECT RAISE(ABORT,'transport probe context owner mismatch'); END;
CREATE TRIGGER provider_probe_generation_exclusive BEFORE INSERT ON provider_probe_contexts
WHEN EXISTS(SELECT 1 FROM provider_probe_transport_contexts WHERE probe_id=NEW.probe_id)
BEGIN SELECT RAISE(ABORT,'generation and transport contexts are exclusive'); END;
CREATE TRIGGER provider_probe_transport_immutable BEFORE UPDATE ON provider_probe_transport_contexts
WHEN OLD.probe_id IS NOT NEW.probe_id OR OLD.receipt_id IS NOT NEW.receipt_id
 OR OLD.actor_id IS NOT NEW.actor_id OR OLD.principal_id IS NOT NEW.principal_id
 OR OLD.project_id IS NOT NEW.project_id OR OLD.profile_version_ref IS NOT NEW.profile_version_ref
 OR OLD.authority_generation IS NOT NEW.authority_generation OR OLD.daemon_epoch IS NOT NEW.daemon_epoch
 OR OLD.action_id IS NOT NEW.action_id OR OLD.mandate_ref IS NOT NEW.mandate_ref
 OR OLD.phase IS NOT NEW.phase OR OLD.context_digest IS NOT NEW.context_digest
 OR OLD.context_json IS NOT NEW.context_json OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR (OLD.claim_state='claimed' AND (OLD.claim_state IS NOT NEW.claim_state
  OR OLD.claim_revision IS NOT NEW.claim_revision OR OLD.claim_id IS NOT NEW.claim_id
  OR OLD.claimed_at_ms IS NOT NEW.claimed_at_ms))
BEGIN SELECT RAISE(ABORT,'transport probe immutable identity and claim'); END;
CREATE TRIGGER provider_probe_transport_durable BEFORE DELETE ON provider_probe_transport_contexts
BEGIN SELECT RAISE(ABORT,'transport probe context durable'); END;
CREATE TRIGGER admission_source_probe_transport_insert AFTER INSERT ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_transport_update AFTER UPDATE ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_transport_delete AFTER DELETE ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
";
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, ProjectId, ProviderProbeId,
    ReceiptId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProviderTestLimits, ProviderTestPhase};

/// Contexte immuable de la demande authentifiée ; aucune identité OS ou modèle.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbeConnectContext {
    /// Version du document fermé.
    pub schema_version: u32,
    /// Agrégat stable de TestProviderProfile.
    pub probe_id: ProviderProbeId,
    /// Nouveau reçu de cette préparation, distinct du mandat et de la configuration.
    pub receipt_id: ReceiptId,
    /// Acteur de la session authentifiée.
    pub actor_id: ActorId,
    /// Principal courant attribué à cet acteur.
    pub principal_id: ActorId,
    /// Projet réel de la version configurée, ou portée globale.
    pub project_id: Option<ProjectId>,
    /// Lignée de préparation.
    pub authority_generation: AuthorityGeneration,
    /// Époque de préparation ; une reprise ne réhydrate pas une permission d'effet.
    pub daemon_epoch: DaemonEpoch,
    /// Action stable réservée, sans préinsérer actions.
    pub action_id: ActionId,
    /// Reçu d'émission de la permission de réseau.
    pub mandate_ref: ReceiptId,
    /// Empreinte exacte de cette émission.
    pub mandate_digest: Sha256Digest,
    /// Phase exacte du transport ; aucune génération implicite.
    pub phase: ProviderTestPhase,
    /// Identité du profil demandé.
    pub profile_id: uuid::Uuid,
    /// Version exacte, sans fallback latest.
    pub profile_version: u64,
    /// Clé historique opaque résolue en base.
    pub profile_version_ref: String,
    /// Empreinte des octets de configuration.
    pub configuration_digest: Sha256Digest,
    /// Destination dérivée de cette configuration et du mandat.
    pub destination: String,
    /// Révision du scope broker canonique, distincte de la version du profil.
    pub scope_revision: Revision,
    /// Révision de politique réellement applicable.
    pub policy_revision: Revision,
    /// Snapshot de politique comprenant les permissions courantes.
    pub policy_digest: Sha256Digest,
    /// Empreinte du wire original, sans normalisation de l'enveloppe.
    pub command_digest: Sha256Digest,
    /// Empreinte de la requête de transport fermée, sans reçu fournisseur présumé.
    pub request_digest: Sha256Digest,
    /// Localité permise, sans affirmation de placement de modèle.
    pub locality: DataLocality,
    /// Capacités demandées, sans résultat de qualification.
    pub capability_set: Vec<String>,
    /// Bornes demandées et vérifiées.
    pub limits: ProviderTestLimits,
    /// Échéance absolue réduite par le mandat et le profil.
    pub deadline: TimestampMs,
}

impl std::fmt::Debug for ProviderProbeConnectContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProbeConnectContext")
            .field("probe_id", &self.probe_id)
            .field("action_id", &self.action_id)
            .finish_non_exhaustive()
    }
}

/// Résolution serveur préalable ; le commit relit toutes ses entrées dans le writer.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderProbeConnectPreparation {
    pub(crate) profile: crate::ConfiguredProviderProfile,
    pub(crate) principal_id: ActorId,
    pub(crate) scope_revision: Revision,
    pub(crate) policy_revision: Revision,
    pub(crate) policy_digest: Sha256Digest,
    pub(crate) command_digest: Sha256Digest,
    pub(crate) prepared_at_ms: TimestampMs,
}

impl std::fmt::Debug for ProviderProbeConnectPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProbeConnectPreparation")
            .field("profile_version_ref", &self.profile.profile_version_ref)
            .finish_non_exhaustive()
    }
}

/// Projection historique ; une lecture ne donne pas le droit de relancer l'effet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbeConnectRecord {
    /// Contexte exact, avec ses identités de préparation.
    pub context: ProviderProbeConnectContext,
    /// Empreinte canonique du contexte immuable.
    pub context_digest: Sha256Digest,
}

pub(crate) struct ConnectCommand<'a> {
    pub envelope: &'a CommandEnvelope,
    pub caller: &'a CallerAuthority,
    pub preparation: &'a ProviderProbeConnectPreparation,
}

pub(crate) fn validate_identity(request: &crate::CommitCommandRequest) -> Result<bool, StoreError> {
    let crate::CommandMutation::PrepareProviderProbeConnect { caller, .. } = &request.mutation
    else {
        return Ok(false);
    };
    crate::provider_profile_validation::validate_envelope_identity(&request.envelope, caller)?;
    if request.actor_id != caller.actor_id
        || request.target_aggregate != crate::AggregateKind::ProviderProbe
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "identité connect incohérente".to_owned(),
        });
    }
    Ok(true)
}

pub(crate) fn validate_new_command(
    request: &crate::CommitCommandRequest,
) -> Result<(), StoreError> {
    if validate_identity(request)?
        && (request.envelope.expected_revision != Revision::INITIAL
            || !request.events.is_empty()
            || !request.capacity_reservations.is_empty()
            || !request.budget_reservations.is_empty()
            || request.fence_assertion.is_some())
    {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "préparation connect sans effet ni réserve attendue".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn commit_tx(
    _tx: &rusqlite::Transaction<'_>,
    _config: &crate::StoreConfig,
    _daemon: &crate::DaemonStateRow,
    _now: TimestampMs,
    _command: ConnectCommand<'_>,
    _receipt: ReceiptId,
    _events: &mut Vec<crate::EventAppendSpec>,
) -> Result<Revision, StoreError> {
    Err(pending())
}

fn pending() -> StoreError {
    StoreError::EvidenceUnavailable("préparation connect non raccordée".to_owned())
}

impl UnleashStore {
    /// Résout le mandat authentifié exact sans créer intention, reçu ou réservation.
    /// # Errors
    /// Refuse autorité, scope, version, phase ou mandat incompatibles.
    pub fn prepare_provider_probe_connect(
        &self,
        _envelope: &CommandEnvelope,
        _caller: &CallerAuthority,
    ) -> Result<ProviderProbeConnectPreparation, StoreError> {
        // TDD : le premier oracle comportemental doit constater ce refus fermé.
        Err(pending())
    }

    /// Lit un contexte exact et son watermark, sans lui donner autorité après reprise.
    /// # Errors
    /// Refuse un document ou digest durable incohérent.
    pub fn inspect_provider_probe_connect_snapshot(
        &self,
        _probe: ProviderProbeId,
    ) -> Result<SnapshotProjection<Option<ProviderProbeConnectRecord>>, StoreError> {
        Err(pending())
    }
}
