//! Préparation F012 d'un contexte de transport, distinct du contexte modèle V009.
//! Le writer commun doit encore raccorder la migration et CommitCommand.
//! Aucun état d'action, effet réseau ou réserve n'est créé par la résolution.

use crate::{CallerAuthority, SnapshotProjection, StoreError, UnleashStore};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, ProjectId, ProviderProbeId,
    ReceiptId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProviderTestLimits};

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
