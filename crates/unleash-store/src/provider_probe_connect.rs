//! Préparation F012 d'un contexte de transport, distinct du contexte modèle V009.
//! Aucun état d'action, effet réseau ou réserve n'est créé par la résolution.

use crate::commands::read_daemon_state_conn;
use crate::policies_secrets::read_policy_summary_conn;
use crate::{CallerAuthority, SnapshotProjection, StoreError, UnleashStore, WriteCriticality};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActionId, ActorId, AuthorityGeneration, DaemonEpoch, DataLocality, ProjectId, ProviderProbeId,
    ReceiptId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProviderTestLimits, ProviderTestPhase};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportPhase {
    Connect,
    Inventory,
}

impl TransportPhase {
    pub(crate) const fn wire(self) -> ProviderTestPhase {
        match self {
            Self::Connect => ProviderTestPhase::Connect,
            Self::Inventory => ProviderTestPhase::Inventory,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Inventory => "inventory",
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::Connect => "model.provider.connect",
            Self::Inventory => "model.provider.inventory",
        }
    }

    fn from_wire(phase: ProviderTestPhase) -> Result<Self, StoreError> {
        match phase {
            ProviderTestPhase::Connect => Ok(Self::Connect),
            ProviderTestPhase::Inventory => Ok(Self::Inventory),
            _ => Err(invalid("phase non transport")),
        }
    }
}

/// Contexte immuable de la demande authentifiée ; aucune identité OS ou modèle.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbeConnectContext {
    /// Version du document fermé.
    pub schema_version: u32,
    /// Agrégat stable de `TestProviderProfile`.
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
    /// Inventory v2 only. Omission preserves historical connect JSON and digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_limits: Option<crate::provider_probe_inventory::InventoryTransportLimits>,
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
    pub(crate) wire: unleash_protocol::TestProviderProfilePayload,
    pub(crate) grant: crate::provider_probe_mandates::ResolvedConnectGrant,
    pub(crate) authority_generation: AuthorityGeneration,
    pub(crate) daemon_epoch: DaemonEpoch,
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
    /// Métadonnées d'inspection ; elles ne constituent jamais un nouveau token.
    pub claim: Option<ProviderProbeConnectClaimRecord>,
}

/// Claim durable consultable, sans capacité de démarrer un nouvel effecteur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbeConnectClaimRecord {
    /// Identité créée une seule fois dans le writer.
    pub claim_id: uuid::Uuid,
    /// Révision CAS de la claim.
    pub revision: Revision,
    /// Date du commit de propriété.
    pub claimed_at_ms: TimestampMs,
}

/// Actual effective policy read for the original acknowledged connect owner.
/// These facts never mint another claim or grant an effect on their own.
#[derive(Clone)]
pub struct ProviderConnectPolicySnapshot {
    /// Exact authenticated scope.
    pub project_id: Option<ProjectId>,
    /// Current effective policy revision, verified against the frozen context.
    pub policy_revision: Revision,
    /// Current scope revision, verified against the frozen context.
    pub scope_revision: Revision,
    /// Complete global/project/Guard snapshot identity, including revisions below max.
    pub policy_digest: Sha256Digest,
    /// Current control-plane incarnation, distinct from the resource lease.
    pub control_fence: unleash_domain::LineageFence,
    /// Effective isolation of every applicable policy.
    pub isolation_mode: crate::PolicyIsolationMode,
    /// Real Guard configuration, without loading any referenced credential.
    pub guard_config: Option<crate::GuardConfigRecord>,
}

impl std::fmt::Debug for ProviderConnectPolicySnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConnectPolicySnapshot")
            .field("project_id", &self.project_id)
            .field("policy_revision", &self.policy_revision)
            .finish_non_exhaustive()
    }
}

/// Entrée interne authentifiée ; aucun codec IPC n'accepte cette structure.
#[derive(Clone)]
pub struct ClaimProviderProbeConnectRequest {
    /// Wire original dont le digest est revérifié, jamais normalisé.
    pub envelope: CommandEnvelope,
    /// Autorité résolue par la session serveur.
    pub caller: CallerAuthority,
    /// Reçu du contexte exact préparé.
    pub receipt_id: ReceiptId,
    /// Empreinte CAS lue dans le même contexte.
    pub context_digest: Sha256Digest,
    /// Fence de contrôle, distincte du futur bail d'exécution V008.
    pub control_fence: unleash_domain::LineageFence,
}

impl std::fmt::Debug for ClaimProviderProbeConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimProviderProbeConnectRequest")
            .field("receipt_id", &self.receipt_id)
            .field("probe_id", &self.envelope.aggregate_id)
            .finish_non_exhaustive()
    }
}

/// Token non clonable émis uniquement pour une nouvelle claim acquittée.
#[derive(Debug)]
pub struct ProviderProbeConnectClaim {
    record: ProviderProbeConnectRecord,
    claim: ProviderProbeConnectClaimRecord,
    // Kept from the authenticated call, never reconstructed from a projection.
    request: Box<ClaimProviderProbeConnectRequest>,
    destination: unleash_protocol::provider_profile::ProviderEndpointDestination,
}

/// Private writer capability copied only from the original opaque fresh token.
pub(crate) struct ConnectPhasePermission {
    record: ProviderProbeConnectRecord,
    request: ClaimProviderProbeConnectRequest,
    destination: unleash_protocol::provider_profile::ProviderEndpointDestination,
}

impl ConnectPhasePermission {
    pub(crate) fn destination(
        &self,
    ) -> &unleash_protocol::provider_profile::ProviderEndpointDestination {
        &self.destination
    }
}

pub(crate) fn validate_live_permission(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    permission: &ConnectPhasePermission,
) -> Result<ProviderProbeConnectRecord, StoreError> {
    validate_live_transport_permission(tx, daemon, now, permission, TransportPhase::Connect)
}

pub(crate) fn validate_live_transport_permission(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    permission: &ConnectPhasePermission,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectRecord, StoreError> {
    let record = claim_transport_identity(tx, daemon, &permission.request, phase)?;
    if record != permission.record || record.claim.is_none() {
        return Err(StoreError::StaleFence {
            message: "claim connect différente du propriétaire frais".to_owned(),
        });
    }
    validate_live_transport_context(tx, daemon, now, &permission.request, &record, phase)?;
    Ok(record)
}

pub(crate) enum ConnectPhaseAccess<'a> {
    Live(&'a ConnectPhasePermission),
    Observation(&'a ConnectPhasePermission),
    InventoryLive(&'a crate::provider_probe_inventory::InventoryPhasePermission),
    InventoryObservation(&'a crate::provider_probe_inventory::InventoryPhasePermission),
}

impl ProviderProbeConnectClaim {
    /// Frozen configured destination validated when this original claim was issued.
    #[must_use]
    pub const fn destination(
        &self,
    ) -> &unleash_protocol::provider_profile::ProviderEndpointDestination {
        &self.destination
    }
    pub(crate) fn permission(&self) -> ConnectPhasePermission {
        ConnectPhasePermission {
            record: self.record.clone(),
            request: (*self.request).clone(),
            destination: self.destination.clone(),
        }
    }
    /// Contexte immuable autorisé pour cet unique propriétaire.
    #[must_use]
    pub const fn context(&self) -> &ProviderProbeConnectContext {
        &self.record.context
    }
    /// Empreinte du contexte autorisé.
    #[must_use]
    pub const fn context_digest(&self) -> Sha256Digest {
        self.record.context_digest
    }
    /// Identité durable de cette claim fraîche.
    #[must_use]
    pub const fn claim_id(&self) -> uuid::Uuid {
        self.claim.claim_id
    }
    /// Révision de propriété.
    #[must_use]
    pub const fn claim_revision(&self) -> Revision {
        self.claim.revision
    }
}

/// Validates attribution for a fact after I/O, without granting a new effect.
/// A current policy change cannot rewrite the original native producer's context.
pub(crate) fn fact_context(
    tx: &Connection,
    daemon: &crate::DaemonStateRow,
    permission: &ConnectPhasePermission,
) -> Result<ProviderProbeConnectRecord, StoreError> {
    fact_transport_context(tx, daemon, permission, TransportPhase::Connect)
}

pub(crate) fn fact_transport_context(
    tx: &Connection,
    daemon: &crate::DaemonStateRow,
    permission: &ConnectPhasePermission,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectRecord, StoreError> {
    let original = &permission.record;
    let request = &permission.request;
    crate::provider_profile_validation::validate_envelope_identity(
        &request.envelope,
        &request.caller,
    )?;
    let context = &original.context;
    if request.envelope.authority_generation != daemon.authority_generation
        || context.authority_generation != daemon.authority_generation
    {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: context.authority_generation,
            command_id: Some(request.envelope.command_id),
        });
    }
    if context.daemon_epoch != daemon.daemon_epoch
        || request.control_fence
            != unleash_domain::LineageFence::new(
                daemon.authority_generation,
                daemon.daemon_epoch,
                unleash_domain::Fence::ZERO,
            )
    {
        return Err(StoreError::StaleFence {
            message: format!("fait {} d'une autre incarnation", phase.name()),
        });
    }
    let current = read_transport_context(tx, context.probe_id, phase)?.ok_or_else(hidden)?;
    if &current != original
        || current.claim.is_none()
        || request.receipt_id != context.receipt_id
        || request.context_digest != current.context_digest
        || request.caller.actor_id != context.actor_id
        || unleash_protocol::compute_command_digest_v2(&request.envelope)? != context.command_digest
    {
        return Err(StoreError::StaleFence {
            message: format!("fait {} sans sa claim originale", phase.name()),
        });
    }
    let attributed:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts r JOIN principals p ON p.principal_id=?1 WHERE r.actor_id=?2 AND r.command_id=?3 AND r.receipt_id=?4 AND r.command_type='TestProviderProfile' AND r.aggregate_id=?5 AND r.authority_generation=?6 AND r.command_digest=?7 AND r.state IN ('accepted','completed') AND p.actor_id=r.actor_id)",
        params![context.principal_id.as_str(),context.actor_id.as_str(),request.envelope.command_id.to_string(),context.receipt_id.to_string(),context.probe_id.to_string(),context.authority_generation.to_string(),context.command_digest.to_string()],|row|row.get(0),
    )?;
    if !attributed {
        return Err(hidden());
    }
    Ok(current)
}

/// Seul `FreshClaim` permet au service de commencer l'orchestration broker.
#[derive(Debug)]
pub enum ProviderProbeConnectClaimOutcome {
    /// Nouveau propriétaire connu comme acquitté ; token consommable une fois.
    FreshClaim(ProviderProbeConnectClaim),
    /// Inspection uniquement, notamment après perte d'un accusé précédent.
    AlreadyClaimed(ProviderProbeConnectRecord),
}

pub(crate) struct ConnectCommand<'a> {
    pub envelope: &'a CommandEnvelope,
    pub caller: &'a CallerAuthority,
    pub preparation: &'a ProviderProbeConnectPreparation,
    pub phase: TransportPhase,
}

pub(crate) fn validate_identity(request: &crate::CommitCommandRequest) -> Result<bool, StoreError> {
    let (crate::CommandMutation::PrepareProviderProbeConnect { caller, .. }
    | crate::CommandMutation::PrepareProviderProbeInventory { caller, .. }) = &request.mutation
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
            || request.operation_id.is_some()
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

/// Reuses the exact profile's existing object verifier outside the writer.
/// The common preflight classifies replay before this dynamic access.
pub(crate) fn verify_published(
    store: &UnleashStore,
    request: &crate::CommitCommandRequest,
) -> Result<(), StoreError> {
    let preparation = match &request.mutation {
        crate::CommandMutation::PrepareProviderProbeConnect { preparation, .. } => {
            preparation.as_ref()
        }
        crate::CommandMutation::PrepareProviderProbeInventory { preparation, .. } => {
            &preparation.inner
        }
        _ => return Ok(()),
    };
    crate::provider_profiles::verify_configuration_objects(
        store,
        &preparation.profile.configuration,
        preparation.profile.configuration_digest,
    )
}

/// Dispatch only to a positively validated transport variant. An absent context
/// never falls through as permission for the connect verb. Generation retains
/// its own verifier. The first preparation tranche has no qualified claim yet.
pub(crate) fn validate_intent(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    intent: &crate::BrokerIntent,
) -> Result<bool, StoreError> {
    let connect = intent
        .manifest
        .get("verb")
        .and_then(serde_json::Value::as_str)
        == Some("model.provider.connect");
    let probe: Option<String> = if daemon.schema_version >= 15 {
        tx.query_row(
            "SELECT probe_id FROM provider_probe_transport_contexts WHERE action_id=?1",
            [intent.action_id.to_string()],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    if let Some(probe) = probe {
        let _record = read_context(tx, probe.parse()?)?.ok_or_else(hidden)?;
        if !connect {
            return Err(invalid("verbe divergent du contexte transport"));
        }
        return Err(StoreError::EvidenceUnavailable(
            "claim et exécuteur connect non qualifiés".to_owned(),
        ));
    }
    if connect {
        return Err(hidden());
    }
    Ok(false)
}

fn for_action(
    conn: &Connection,
    action: ActionId,
    phase: TransportPhase,
) -> Result<Option<ProviderProbeConnectRecord>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 15 {
        return Ok(None);
    }
    let probe: Option<String> = conn
        .query_row(
            "SELECT probe_id FROM provider_probe_transport_contexts WHERE action_id=?1",
            [action.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    probe
        .map(|id| read_transport_context(conn, id.parse()?, phase))
        .transpose()
        .map(Option::flatten)
}

/// Runs before the shared phase replay lookup. Inspection cannot mint this capability.
pub(crate) fn guard_phase_access(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    phase: &crate::BrokerPhase,
    access: Option<ConnectPhaseAccess<'_>>,
) -> Result<bool, StoreError> {
    let transport = if matches!(
        &access,
        Some(ConnectPhaseAccess::InventoryLive(_) | ConnectPhaseAccess::InventoryObservation(_))
    ) {
        TransportPhase::Inventory
    } else {
        TransportPhase::Connect
    };
    if is_reserved_inventory_phase(tx, phase)? && transport != TransportPhase::Inventory {
        // Classification only denies access. Neither inspection nor a connect
        // permission can mint the original inventory execution owner.
        return Err(StoreError::EvidenceUnavailable(
            "inventory exige sa claim originale de transport".to_owned(),
        ));
    }
    let record = for_action(tx, phase.action_id(), transport)?;
    let is_connect = matches!(phase, crate::BrokerPhase::Authorizing { intent }
        if intent.manifest.get("verb") == Some(&serde_json::json!(TransportPhase::Connect.verb()))
        || intent.manifest.get("verb") == Some(&serde_json::json!(TransportPhase::Inventory.verb())));
    let Some(record) = record else {
        return if is_connect || access.is_some() {
            Err(hidden())
        } else {
            Ok(false)
        };
    };
    let access = access.ok_or_else(|| {
        StoreError::EvidenceUnavailable(
            "connect exige le token frais original, même pour un rejeu".to_owned(),
        )
    })?;
    let (permission, observation) = match access {
        ConnectPhaseAccess::Live(permission) => (permission, false),
        ConnectPhaseAccess::Observation(permission) => (permission, true),
        ConnectPhaseAccess::InventoryLive(permission) => (permission.inner(), false),
        ConnectPhaseAccess::InventoryObservation(permission) => (permission.inner(), true),
    };
    if observation != matches!(phase, crate::BrokerPhase::Observation { .. }) {
        return Err(StoreError::EvidenceUnavailable(
            "accès connect incompatible avec la phase".to_owned(),
        ));
    }
    let current = if observation {
        fact_transport_context(tx, daemon, permission, transport)?
    } else {
        claim_transport_identity(tx, daemon, &permission.request, transport)?
    };
    if current != record || record != permission.record || record.claim.is_none() {
        return Err(StoreError::StaleFence {
            message: "claim connect différente du propriétaire frais".to_owned(),
        });
    }
    if matches!(phase, crate::BrokerPhase::Observation { .. }) {
        // This permits only reaching the specialized proof verifier after the
        // shared exact replay lookup; it never qualifies a native fact itself.
        return Ok(true);
    }
    let fresh =
        validate_live_transport_context(tx, daemon, now, &permission.request, &record, transport)?;
    if let crate::BrokerPhase::Authorizing { intent } = phase {
        validate_bound_intent(&record.context, &fresh.profile, intent)?;
    }
    if matches!(
        phase,
        crate::BrokerPhase::Authorization { .. } | crate::BrokerPhase::Dispatch { .. }
    ) {
        validate_execution_reservation(tx, daemon, now, &record.context)?;
    }
    Ok(true)
}

fn is_reserved_inventory_phase(
    tx: &Transaction<'_>,
    phase: &crate::BrokerPhase,
) -> Result<bool, StoreError> {
    // Later entries do not carry an intent. Read its immutable verb only to
    // refuse generic access, never to reconstruct a permission from SQLite.
    let verb: Option<String> = if let crate::BrokerPhase::Authorizing { intent } = phase {
        intent
            .manifest
            .get("verb")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    } else {
        tx.query_row(
            "SELECT json_extract(intent_json,'$.manifest.verb') FROM broker_intents WHERE action_id=?1",
            [phase.action_id().to_string()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
    };
    Ok(verb.as_deref() == Some("model.provider.inventory"))
}

fn validate_bound_intent(
    context: &ProviderProbeConnectContext,
    profile: &crate::ConfiguredProviderProfile,
    intent: &crate::BrokerIntent,
) -> Result<(), StoreError> {
    let destination = profile
        .configuration
        .payload
        .endpoint_destination()
        .map_err(|_| invalid("destination connect invalide"))?;
    validate_bound_destination(context, &destination, intent)
}

pub(crate) fn validate_fact_intent(
    permission: &ConnectPhasePermission,
    intent: &crate::BrokerIntent,
) -> Result<(), StoreError> {
    crate::broker_writer::validate_manifest(intent)?;
    validate_bound_destination(&permission.record.context, &permission.destination, intent)
}

fn validate_bound_destination(
    context: &ProviderProbeConnectContext,
    destination: &unleash_protocol::provider_profile::ProviderEndpointDestination,
    intent: &crate::BrokerIntent,
) -> Result<(), StoreError> {
    use unleash_domain::{ActionOrigin, Fence, LineageFence, ReconcileStrategy};
    let phase = TransportPhase::from_wire(context.phase)?;
    if intent.lineage_fence
        != LineageFence::new(
            context.authority_generation,
            context.daemon_epoch,
            Fence::ZERO,
        )
    {
        return Err(StoreError::StaleFence {
            message: "fence de contrôle connect différente du contexte".to_owned(),
        });
    }
    let resources = serde_json::json!([{"kind":"network_endpoint","value":{"host":destination.host(),
        "port":destination.port(),"is_private_or_loopback":destination.is_loopback_literal()}}]);
    if intent.action_id != context.action_id
        || intent.origin
            != (ActionOrigin::ControlPlane {
                control_principal_id: context.principal_id.clone(),
            })
        || intent.principal_id != context.principal_id
        || intent.cause
            != (crate::BrokerCause::CommandReceipt {
                receipt_id: context.receipt_id,
            })
        || intent.project_id != context.project_id
        || intent.payload_digest != context.request_digest
        || intent.scope_revision != context.scope_revision
        || intent.policy_revision != context.policy_revision
        || intent.action_deadline != context.deadline
        || intent.reconcile_strategy != ReconcileStrategy::ObserveThenDecide
        || intent.manifest.get("verb") != Some(&serde_json::json!(phase.verb()))
        || intent.manifest.get("effects_class") != Some(&serde_json::json!("network_egress"))
        || intent.manifest.get("resource_refs") != Some(&resources)
        || intent.manifest.get("secret_refs") != Some(&serde_json::json!([]))
        || intent.manifest.get("provenance")
            != Some(&serde_json::json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "intention connect divergente du contexte authentique",
        ));
    }
    Ok(())
}

fn validate_execution_reservation(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    context: &ProviderProbeConnectContext,
) -> Result<(), StoreError> {
    let valid: bool = tx.query_row(
        "SELECT COUNT(*)=1 FROM provider_probes p JOIN execution_reservation_owners o ON o.provider_probe_id=p.probe_id JOIN capacity_reservations c ON c.reservation_id=o.reservation_id JOIN capacity_pools pool ON pool.pool_id=c.pool_id JOIN resource_leases l ON l.owner_type=c.owner_type AND l.owner_id=c.owner_id WHERE p.probe_id=?1 AND p.state IN ('admitted','running') AND p.fence>0 AND c.state='held' AND c.amount=1 AND c.owner_type='provider_probe' AND c.authority_generation=?2 AND c.daemon_epoch=?3 AND pool.pool_kind='execution' AND pool.scope_key='global' AND l.resource_kind='execution' AND l.resource_key=?4 AND l.authority_generation=?2 AND l.daemon_epoch=?3 AND l.fence=p.fence AND p.authority_generation=?2 AND p.daemon_epoch=?3 AND l.boot_identity=?5 AND l.state='held' AND l.expires_at_ms>?6",
        params![context.probe_id.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,format!("provider_probe:{}",context.probe_id),daemon.boot_identity,now.as_i64()], |row| row.get(0),
    )?;
    if !valid {
        return Err(StoreError::StaleFence {
            message: "réservation V008 ou bail connect non courant".to_owned(),
        });
    }
    if unresolved_connect_occupation(tx)? != 1 {
        return Err(StoreError::CapacityBlocked(
            "connect non identifié : une seule réservation conservatrice".to_owned(),
        ));
    }
    Ok(())
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn hidden() -> StoreError {
    StoreError::NotFound("Mandat d'effet fournisseur introuvable".to_owned())
}

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("borne connect hors plage SQLite"))
}

fn scope_revision(
    tx: &Transaction<'_>,
    project: Option<ProjectId>,
) -> Result<Revision, StoreError> {
    let revision: i64 = if let Some(project) = project {
        tx.query_row(
            "SELECT revision FROM projects WHERE project_id=?1 AND state='active'",
            [project.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(hidden)?
    } else {
        tx.query_row("SELECT COALESCE((SELECT revision FROM settings WHERE scope_kind='user' AND scope_id='default'),0)", [], |row| row.get(0))?
    };
    Ok(Revision::new(u64::try_from(revision).map_err(|_| {
        invalid("révision de scope connect invalide")
    })?))
}

pub(crate) fn policy(
    tx: &Transaction<'_>,
    project: Option<ProjectId>,
) -> Result<(Revision, Sha256Digest), StoreError> {
    let summary = read_policy_summary_conn(tx, project)?;
    let revision = summary
        .global_policy
        .as_ref()
        .map_or(0, |p| p.revision.as_u64())
        .max(
            summary
                .project_policy
                .as_ref()
                .map_or(0, |p| p.revision.as_u64()),
        );
    let digest = unleash_protocol::canonical_sha256(
        &serde_json::json!({"schema_version":1,"global_policy":summary.global_policy,"project_policy":summary.project_policy,"guard_config":summary.guard_config}),
    )?;
    Ok((Revision::new(revision), digest))
}

fn prepare_tx(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
) -> Result<ProviderProbeConnectPreparation, StoreError> {
    prepare_transport_tx(tx, daemon, now, envelope, caller, TransportPhase::Connect)
}

pub(crate) fn prepare_transport_tx(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    envelope: &CommandEnvelope,
    caller: &CallerAuthority,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectPreparation, StoreError> {
    crate::provider_profile_validation::validate_envelope_identity(envelope, caller)?;
    if envelope.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: envelope.authority_generation,
            command_id: Some(envelope.command_id),
        });
    }
    if daemon.mode != unleash_domain::DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "préparation connect exige daemon ready".to_owned(),
        ));
    }
    if envelope.expected_revision != Revision::INITIAL {
        return Err(invalid("connect exige un nouvel agrégat"));
    }
    let wire: unleash_protocol::TestProviderProfilePayload =
        serde_json::from_value(serde_json::Value::Object(envelope.payload.clone()))
            .map_err(|_| invalid("payload connect invalide"))?;
    if wire.phase() != phase.wire() {
        return Err(invalid("phase transport exacte attendue"));
    }
    let profile_id: uuid::Uuid = wire.profile_id().parse().map_err(|_| hidden())?;
    if profile_id.is_nil() || profile_id.to_string() != wire.profile_id() {
        return Err(hidden());
    }
    let profile = crate::provider_profiles::resolve(tx, profile_id, wire.profile_version())?
        .ok_or_else(hidden)?;
    let grant = if phase == TransportPhase::Connect {
        crate::provider_probe_mandates::resolve_connect_grant(
            tx, daemon, now, envelope, caller, &wire, &profile,
        )?
    } else {
        crate::provider_probe_mandates::resolve_transport_grant(
            tx,
            daemon,
            now,
            &crate::provider_probe_mandates::TransportGrantRequest {
                envelope,
                caller,
                wire: &wire,
                profile: &profile,
                phase,
            },
        )?
    };
    if phase == TransportPhase::Inventory {
        crate::provider_probe_inventory::request_endpoint(&profile)?;
        crate::provider_probe_inventory::bounds(&profile, &wire)?;
    }
    let scope_revision = scope_revision(tx, envelope.project_id)?;
    let (policy_revision, policy_digest) = policy(tx, envelope.project_id)?;
    Ok(ProviderProbeConnectPreparation {
        profile,
        wire,
        principal_id: grant.principal_id.clone(),
        grant,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        scope_revision,
        policy_revision,
        policy_digest,
        command_digest: unleash_protocol::compute_command_digest_v2(envelope)?,
        prepared_at_ms: now,
    })
}

fn validate_preparation(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    command: &ConnectCommand<'_>,
) -> Result<TimestampMs, StoreError> {
    let current = prepare_transport_tx(
        tx,
        daemon,
        now,
        command.envelope,
        command.caller,
        command.phase,
    )?;
    let mut original = command.preparation.clone();
    original.prepared_at_ms = now;
    if current != original || command.preparation.prepared_at_ms > now {
        return Err(invalid("préparation connect périmée ou divergente"));
    }
    let timeout = if command.phase == TransportPhase::Inventory {
        crate::provider_probe_inventory::bounds(&current.profile, &current.wire)?.timeout_ms
    } else {
        current.wire.limits().timeout_ms()
    };
    let expiry = command
        .preparation
        .prepared_at_ms
        .as_i64()
        .checked_add(integer(timeout)?)
        .ok_or_else(|| invalid("échéance connect hors plage"))?
        .min(current.grant.expires_at_ms.as_i64());
    if expiry <= now.as_i64() {
        return Err(hidden());
    }
    Ok(TimestampMs::new(expiry))
}

fn transport_digest(
    preparation: &ProviderProbeConnectPreparation,
    phase: TransportPhase,
) -> Result<Sha256Digest, StoreError> {
    if phase == TransportPhase::Inventory {
        return crate::provider_probe_inventory::request_digest(preparation);
    }
    // This is the closed loopback component's request identity, not a claim that
    // a configured name is loopback or that a model runs there. Its executor
    // still rejects every target outside that constraint before any socket.
    unleash_protocol::canonical_sha256(&serde_json::json!({
        "schema":"unleash.provider_connect.v1",
        "endpoint":preparation.profile.configuration.payload.endpoint(),
        "profile_id":preparation.profile.configuration.profile_id,
        "profile_version":preparation.profile.configuration.profile_version,
        "profile_digest":preparation.profile.configuration_digest,
        "destination_scope":"loopback_only", "timeout_ms":preparation.wire.limits().timeout_ms(),
        "http_application_bytes":0,"tls_validation":"offline_system_roots_chain_name","revocation":"unavailable",
    })).map_err(StoreError::from)
}

fn context(
    config: &crate::StoreConfig,
    command: &ConnectCommand<'_>,
    receipt: ReceiptId,
    deadline: TimestampMs,
) -> Result<ProviderProbeConnectContext, StoreError> {
    let preparation = command.preparation;
    let profile = &preparation.profile;
    Ok(ProviderProbeConnectContext {
        schema_version: 1,
        probe_id: ProviderProbeId::from_uuid(command.envelope.aggregate_id),
        receipt_id: receipt,
        actor_id: command.caller.actor_id.clone(),
        principal_id: preparation.principal_id.clone(),
        project_id: command.envelope.project_id,
        authority_generation: preparation.authority_generation,
        daemon_epoch: preparation.daemon_epoch,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        mandate_ref: preparation.grant.mandate_ref,
        mandate_digest: preparation.grant.mandate_digest,
        phase: command.phase.wire(),
        profile_id: profile.configuration.profile_id,
        profile_version: profile.configuration.profile_version,
        profile_version_ref: profile.profile_version_ref.clone(),
        configuration_digest: profile.configuration_digest,
        destination: profile.configuration.payload.endpoint().to_owned(),
        scope_revision: preparation.scope_revision,
        policy_revision: preparation.policy_revision,
        policy_digest: preparation.policy_digest,
        command_digest: preparation.command_digest,
        request_digest: transport_digest(preparation, command.phase)?,
        locality: preparation.wire.locality(),
        capability_set: preparation.wire.capability_set().to_vec(),
        limits: preparation.wire.limits().clone(),
        inventory_limits: if command.phase == TransportPhase::Inventory {
            Some(crate::provider_probe_inventory::bounds(
                &preparation.profile,
                &preparation.wire,
            )?)
        } else {
            None
        },
        deadline,
    })
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    command: &ConnectCommand<'_>,
    receipt: ReceiptId,
    events: &mut Vec<crate::EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let deadline = validate_preparation(tx, daemon, now, command)?;
    let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)", [command.envelope.aggregate_id.to_string()], |row| row.get(0))?;
    if occupied {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: Revision::INITIAL,
            command_id: Some(command.envelope.command_id),
        });
    }
    let context = context(config, command, receipt, deadline)?;
    let digest = unleash_protocol::canonical_sha256(&context)?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,source_run_id,profile_version_ref,context_hash,phase,state,wait_reason,admission_class,authority_generation,daemon_epoch,fence,revision,queued_at_ms) VALUES(?1,?2,?3,NULL,?4,?5,?9,'queued',NULL,'work',?6,?7,0,1,?8)", params![context.probe_id.to_string(),receipt.to_string(),context.principal_id.as_str(),context.profile_version_ref,digest.to_string(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,now.as_i64(),command.phase.name()])?;
    tx.execute("INSERT INTO provider_probe_transport_contexts(probe_id,receipt_id,actor_id,principal_id,project_id,profile_version_ref,authority_generation,daemon_epoch,action_id,mandate_ref,phase,context_digest,context_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?14,?11,?12,?13)", params![context.probe_id.to_string(),receipt.to_string(),context.actor_id.as_str(),context.principal_id.as_str(),context.project_id.map(|id|id.to_string()),context.profile_version_ref,context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.action_id.to_string(),context.mandate_ref.to_string(),digest.to_string(),serde_json::to_string(&context)?,now.as_i64(),command.phase.name()])?;
    events.push(crate::EventAppendSpec {
        event_type:"ProviderProbePrepared".to_owned(), aggregate_type:crate::AggregateKind::ProviderProbe,
        aggregate_id:context.probe_id.as_uuid(), aggregate_revision:Revision::new(1), project_id:context.project_id,
        correlation_id:command.envelope.command_id.as_uuid(), causation_kind:unleash_protocol::EventCausationKind::Command,
        causation_id:None, payload:serde_json::json!({"probe_id":context.probe_id,"action_id":context.action_id,"phase":context.phase,"context_digest":digest,"mandate_ref":context.mandate_ref}).as_object().cloned().ok_or_else(|| invalid("événement transport invalide"))?,
        sensitivity:unleash_protocol::Sensitivity::Restricted, outbox_destinations:vec!["clients".to_owned(),"harness".to_owned()],
    });
    Ok(Revision::new(1))
}

struct ConnectContextRow {
    json: String,
    digest: String,
    claim_id: Option<String>,
    claim_revision: i64,
    claimed_at_ms: Option<i64>,
    phase: String,
}

pub(crate) fn read_context(
    conn: &Connection,
    probe: ProviderProbeId,
) -> Result<Option<ProviderProbeConnectRecord>, StoreError> {
    read_transport_context(conn, probe, TransportPhase::Connect)
}

pub(crate) fn read_transport_context(
    conn: &Connection,
    probe: ProviderProbeId,
    phase: TransportPhase,
) -> Result<Option<ProviderProbeConnectRecord>, StoreError> {
    let row: Option<ConnectContextRow> = conn.query_row("SELECT context_json,context_digest,claim_id,claim_revision,claimed_at_ms,phase FROM provider_probe_transport_contexts WHERE probe_id=?1", [probe.to_string()], |row| Ok(ConnectContextRow { json:row.get(0)?,digest:row.get(1)?,claim_id:row.get(2)?,claim_revision:row.get(3)?,claimed_at_ms:row.get(4)?,phase:row.get(5)? })).optional()?;
    row.map(|row| {
        let context: ProviderProbeConnectContext = serde_json::from_str(&row.json)?;
        let digest: Sha256Digest = row.digest.parse()?;
        if context.probe_id != probe
            || context.schema_version != 1
            || context.phase != phase.wire()
            || row.phase != phase.name()
            || unleash_protocol::canonical_sha256(&context)? != digest
        {
            return Err(invalid("contexte connect durable incohérent"));
        }
        match (phase, &context.inventory_limits) {
            (TransportPhase::Connect, None) => {}
            (TransportPhase::Inventory, Some(limits)) => {
                limits.validate()?;
                if limits.timeout_ms > context.limits.timeout_ms() {
                    return Err(invalid("délai inventory supérieur à sa demande"));
                }
            }
            _ => return Err(invalid("bornes de transport absentes ou d'une autre phase")),
        }
        let claim = match (row.claim_id, row.claimed_at_ms, row.claim_revision) {
            (None, None, 0) => None,
            (Some(id), Some(at), 1) => Some(ProviderProbeConnectClaimRecord {
                claim_id: id
                    .parse()
                    .map_err(|_| invalid("identité de claim incohérente"))?,
                revision: Revision::new(1),
                claimed_at_ms: TimestampMs::new(at),
            }),
            _ => return Err(invalid("état de claim durable incohérent")),
        };
        Ok(ProviderProbeConnectRecord {
            context,
            context_digest: digest,
            claim,
        })
    })
    .transpose()
}

fn read_any_transport_context(
    conn: &Connection,
    probe: ProviderProbeId,
) -> Result<Option<ProviderProbeConnectRecord>, StoreError> {
    // This only classifies an admission candidate's persisted variant. It never
    // reconstructs a caller, an original claim or a broker append capability.
    let phase: Option<String> = conn
        .query_row(
            "SELECT phase FROM provider_probe_transport_contexts WHERE probe_id=?1",
            [probe.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(phase) = phase else {
        return Ok(None);
    };
    let phase = match phase.as_str() {
        "connect" => TransportPhase::Connect,
        "inventory" => TransportPhase::Inventory,
        _ => return Err(invalid("phase transport durable inconnue")),
    };
    read_transport_context(conn, probe, phase)
}

fn current_profile(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    record: &ProviderProbeConnectRecord,
) -> Result<crate::ConfiguredProviderProfile, StoreError> {
    let c = &record.context;
    let phase = TransportPhase::from_wire(c.phase)?;
    if c.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: c.authority_generation,
            command_id: None,
        });
    }
    if c.daemon_epoch != daemon.daemon_epoch || c.deadline <= now || record.claim.is_none() {
        return Err(StoreError::StaleFence {
            message: "claim, incarnation ou échéance connect non courante".to_owned(),
        });
    }
    let profile = crate::provider_profiles::resolve(tx, c.profile_id, c.profile_version)?
        .ok_or_else(hidden)?;
    let grant = if phase == TransportPhase::Connect {
        crate::provider_probe_mandates::revalidate_connect_grant(tx, daemon, now, c, &profile)?
    } else {
        crate::provider_probe_inventory::validate_context_profile(c, &profile)?;
        crate::provider_probe_mandates::revalidate_transport_grant(
            tx, daemon, now, c, &profile, phase,
        )?
    };
    let (revision, digest) = policy(tx, c.project_id)?;
    if profile.profile_version_ref != c.profile_version_ref
        || profile.configuration_digest != c.configuration_digest
        || profile.configuration.payload.endpoint() != c.destination
        || scope_revision(tx, c.project_id)? != c.scope_revision
        || revision != c.policy_revision
        || digest != c.policy_digest
        || grant.principal_id != c.principal_id
        || grant.mandate_ref != c.mandate_ref
        || grant.mandate_digest != c.mandate_digest
        || grant.expires_at_ms < c.deadline
    {
        return Err(StoreError::StaleFence {
            message: "liaison connect modifiée avant admission".to_owned(),
        });
    }
    let valid: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts r JOIN principals p ON p.principal_id=?1 WHERE r.receipt_id=?2 AND r.actor_id=?3 AND p.actor_id=r.actor_id AND p.state='active' AND r.command_type='TestProviderProfile' AND r.aggregate_id=?4 AND r.authority_generation=?5 AND r.command_digest=?6 AND r.state='accepted' AND r.response_state='full') AND EXISTS(SELECT 1 FROM object_records WHERE object_hash=?7 AND state='live') AND NOT EXISTS(SELECT 1 FROM provider_probe_contexts WHERE probe_id=?4)",
        params![c.principal_id.as_str(),c.receipt_id.to_string(),c.actor_id.as_str(),c.probe_id.to_string(),c.authority_generation.to_string(),c.command_digest.to_string(),c.configuration_digest.to_string()], |row| row.get(0),
    )?;
    if !valid {
        return Err(hidden());
    }
    Ok(profile)
}

/// Transport admission uses the existing Probe unit, without a `ModelCall` parent.
pub(crate) fn validate_admission(
    tx: &Transaction<'_>,
    probe: ProviderProbeId,
    now: TimestampMs,
) -> Result<bool, StoreError> {
    let daemon = read_daemon_state_conn(tx)?;
    if daemon.schema_version < 15 {
        return Ok(false);
    }
    let Some(record) = read_any_transport_context(tx, probe)? else {
        return Ok(false);
    };
    let profile = current_profile(tx, &daemon, now, &record)?;
    let c = &record.context;
    let intent: Option<String> = tx.query_row(
        "SELECT b.intent_json FROM provider_probes p JOIN actions a ON a.action_id=?2 JOIN broker_intents b ON b.action_id=a.action_id WHERE p.probe_id=?1 AND p.state='queued' AND p.phase=?9 AND p.admission_class='work' AND p.fence=0 AND p.authority_generation=?3 AND p.daemon_epoch=?4 AND p.context_hash=?5 AND p.command_receipt_id=?6 AND p.principal_id=?7 AND p.profile_version_ref=?8 AND a.state='authorizing' AND a.authority_generation=?3 AND a.daemon_epoch=?4 AND a.fence=0 AND a.origin_kind='control_plane' AND a.control_principal_id=?7",
        params![probe.to_string(),c.action_id.to_string(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,record.context_digest.to_string(),c.receipt_id.to_string(),c.principal_id.as_str(),c.profile_version_ref,TransportPhase::from_wire(c.phase)?.name()], |row| row.get(0),
    ).optional()?;
    let intent: crate::BrokerIntent = serde_json::from_str(&intent.ok_or_else(hidden)?)?;
    validate_bound_intent(c, &profile, &intent)?;
    if unresolved_connect_occupation(tx)? != 0 {
        return Err(StoreError::CapacityBlocked(
            "connect non identifié : réservation held ou reconciling déjà retenue".to_owned(),
        ));
    }
    validate_profile_capacity(tx, &profile)?;
    Ok(true)
}

fn unresolved_connect_occupation(tx: &Transaction<'_>) -> Result<i64, StoreError> {
    // No qualified server identity exists in this first transport. Both passive
    // contexts share one conservative bound, independent of profile/locality,
    // authority generation and epoch. This is not Q-19 server qualification.
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM capacity_reservations r JOIN capacity_pools pool ON pool.pool_id=r.pool_id JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id JOIN provider_probe_transport_contexts c ON c.probe_id=o.provider_probe_id WHERE r.state IN ('held','reconciling') AND r.amount>0 AND r.owner_type='provider_probe' AND c.phase IN ('connect','inventory') AND pool.pool_kind='execution' AND pool.scope_key='global'",
        [], |row| row.get(0),
    )?)
}

fn validate_profile_capacity(
    tx: &Transaction<'_>,
    profile: &crate::ConfiguredProviderProfile,
) -> Result<(), StoreError> {
    // Stable configured profile identity across its immutable versions. This is
    // not a fabricated identity for other profiles pointing at the same server.
    let active: i64 = tx.query_row(
        "SELECT COUNT(*) FROM capacity_reservations r JOIN execution_reservation_owners o ON o.reservation_id=r.reservation_id JOIN provider_probes p ON p.probe_id=o.provider_probe_id JOIN provider_profiles f ON f.profile_version_ref=p.profile_version_ref WHERE r.state IN ('held','reconciling') AND f.profile_id=?1",
        [profile.configuration.profile_id.to_string()], |row| row.get(0),
    )?;
    if active >= integer(profile.configuration.payload.limits().max_inflight)? {
        return Err(StoreError::CapacityBlocked(
            "limite du profil fournisseur atteinte".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_admission_budgets(
    tx: &Transaction<'_>,
    probe: ProviderProbeId,
    budgets: &[crate::BudgetReservationRequest],
) -> Result<bool, StoreError> {
    if read_daemon_state_conn(tx)?.schema_version < 15 {
        return Ok(false);
    }
    let Some(record) = read_any_transport_context(tx, probe)? else {
        return Ok(false);
    };
    if !budgets.is_empty() {
        return Err(invalid(
            "connect ne fabrique aucune réservation de ModelCall",
        ));
    }
    // No model price is invented for TCP/TLS. Existing applicable overspend
    // remains authoritative even when this passive phase adds no monetary charge.
    let overrun: Option<String> = tx.query_row(
        "SELECT a.account_id FROM budget_accounts a JOIN provider_profiles f ON f.profile_version_ref=?1 WHERE a.currency='USD' AND (a.scope_type='global' OR a.scope_type='project' AND a.scope_id=?2 OR a.scope_type='provider' AND a.scope_id=f.provider_id) AND a.spent_micros>a.limit_micros-a.reserved_micros ORDER BY a.account_id LIMIT 1",
        params![record.context.profile_version_ref,record.context.project_id.map(|id|id.to_string())], |row| row.get(0),
    ).optional()?;
    if let Some(account_id) = overrun {
        return Err(StoreError::BudgetExhausted { account_id });
    }
    Ok(true)
}

fn claim_transport_identity(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    request: &ClaimProviderProbeConnectRequest,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectRecord, StoreError> {
    crate::provider_profile_validation::validate_envelope_identity(
        &request.envelope,
        &request.caller,
    )?;
    for authority in [
        request.envelope.authority_generation,
        request.control_fence.authority_generation,
    ] {
        if authority != daemon.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: daemon.authority_generation,
                actual: authority,
                command_id: Some(request.envelope.command_id),
            });
        }
    }
    if request.control_fence.epoch != daemon.daemon_epoch
        || request.control_fence.fence != unleash_domain::Fence::ZERO
    {
        return Err(StoreError::StaleFence {
            message: "fence de contrôle connect périmée ou confondue avec un bail".to_owned(),
        });
    }
    if daemon.mode != unleash_domain::DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "claim connect exige ready".to_owned(),
        ));
    }
    let digest = unleash_protocol::compute_command_digest_v2(&request.envelope)?;
    let receipt = crate::commands::classify_command_replay(
        tx,
        daemon,
        &request.caller.actor_id,
        &request.envelope,
        digest,
    )?
    .ok_or_else(hidden)?;
    let record = read_transport_context(
        tx,
        ProviderProbeId::from_uuid(request.envelope.aggregate_id),
        phase,
    )?
    .ok_or_else(hidden)?;
    let c = &record.context;
    if c.authority_generation != daemon.authority_generation {
        return Err(StoreError::StaleAuthorityGeneration {
            expected: daemon.authority_generation,
            actual: c.authority_generation,
            command_id: Some(request.envelope.command_id),
        });
    }
    if c.daemon_epoch != daemon.daemon_epoch {
        return Err(StoreError::StaleFence {
            message: "contexte connect d'une autre incarnation".to_owned(),
        });
    }
    if c.receipt_id != request.receipt_id
        || c.actor_id != request.caller.actor_id
        || c.project_id != request.envelope.project_id
    {
        return Err(hidden());
    }
    if record.context_digest != request.context_digest {
        return Err(StoreError::StaleFence {
            message: "digest du contexte claim divergent".to_owned(),
        });
    }
    if receipt.receipt_id != c.receipt_id
        || receipt.aggregate_id != c.probe_id.as_uuid()
        || receipt.state != unleash_protocol::ReceiptState::Accepted
        || digest != c.command_digest
    {
        return Err(hidden());
    }
    Ok(record)
}

fn validate_live_transport_context(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    request: &ClaimProviderProbeConnectRequest,
    record: &ProviderProbeConnectRecord,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectPreparation, StoreError> {
    let c = &record.context;
    let fresh = prepare_transport_tx(tx, daemon, now, &request.envelope, &request.caller, phase)?;
    if c.deadline <= now
        || fresh.principal_id != c.principal_id
        || fresh.profile.profile_version_ref != c.profile_version_ref
        || fresh.profile.configuration_digest != c.configuration_digest
        || fresh.profile.configuration.payload.endpoint() != c.destination
        || fresh.scope_revision != c.scope_revision
        || fresh.policy_revision != c.policy_revision
        || fresh.policy_digest != c.policy_digest
        || fresh.grant.mandate_ref != c.mandate_ref
        || fresh.grant.mandate_digest != c.mandate_digest
        || transport_digest(&fresh, phase)? != c.request_digest
    {
        return Err(StoreError::StaleFence {
            message: "préconditions du contexte connect modifiées avant claim".to_owned(),
        });
    }
    Ok(fresh)
}

fn validate_fresh_claim(
    tx: &Transaction<'_>,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    request: &ClaimProviderProbeConnectRequest,
    record: &ProviderProbeConnectRecord,
    phase: TransportPhase,
) -> Result<unleash_protocol::provider_profile::ProviderEndpointDestination, StoreError> {
    let fresh = validate_live_transport_context(tx, daemon, now, request, record, phase)?;
    let c = &record.context;
    let queued: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1 AND state='queued' AND phase=?6 AND revision=1 AND fence=0 AND principal_id=?2 AND authority_generation=?3 AND daemon_epoch=?4) AND NOT EXISTS(SELECT 1 FROM actions WHERE action_id=?5)", params![c.probe_id.to_string(),c.principal_id.as_str(),c.authority_generation.to_string(),integer(c.daemon_epoch.as_u64())?,c.action_id.to_string(),phase.name()], |row|row.get(0))?;
    if !queued {
        return Err(invalid(
            "probe connect non disponible pour sa première claim",
        ));
    }
    fresh
        .profile
        .configuration
        .payload
        .endpoint_destination()
        .map_err(|_| invalid("destination de claim invalide"))
}

fn claim_tx(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    now: TimestampMs,
    request: &ClaimProviderProbeConnectRequest,
) -> Result<ProviderProbeConnectClaimOutcome, StoreError> {
    claim_transport_tx(tx, config, now, request, TransportPhase::Connect)
}

pub(crate) fn claim_transport_tx(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    now: TimestampMs,
    request: &ClaimProviderProbeConnectRequest,
    phase: TransportPhase,
) -> Result<ProviderProbeConnectClaimOutcome, StoreError> {
    let daemon = read_daemon_state_conn(tx)?;
    let record = claim_transport_identity(tx, &daemon, request, phase)?;
    if record.claim.is_some() {
        // This is a readonly inspection. It never returns a new execution token,
        // including if a prior acknowledgement was lost or the mandate expired.
        return Ok(ProviderProbeConnectClaimOutcome::AlreadyClaimed(record));
    }
    let destination = validate_fresh_claim(tx, &daemon, now, request, &record, phase)?;
    let claim_id = config.id_generator.new_uuid();
    if claim_id.is_nil() {
        return Err(invalid("identité de claim nulle"));
    }
    let changed = tx.execute("UPDATE provider_probe_transport_contexts SET claim_state='claimed',claim_revision=1,claim_id=?2,claimed_at_ms=?3 WHERE probe_id=?1 AND receipt_id=?4 AND context_digest=?5 AND claim_state='reserved' AND claim_revision=0", params![record.context.probe_id.to_string(),claim_id.to_string(),now.as_i64(),request.receipt_id.to_string(),request.context_digest.to_string()])?;
    if changed != 1 {
        return Err(StoreError::StaleFence {
            message: "CAS claim connect refusé".to_owned(),
        });
    }
    let record = read_transport_context(tx, record.context.probe_id, phase)?.ok_or_else(hidden)?;
    let claim = record
        .claim
        .clone()
        .ok_or_else(|| invalid("claim connect perdue après CAS"))?;
    config
        .cutpoints
        .hit(crate::StoreCutpoint::C1BeforeCommandCommit)?;
    Ok(ProviderProbeConnectClaimOutcome::FreshClaim(
        ProviderProbeConnectClaim {
            record,
            claim,
            request: Box::new(request.clone()),
            destination,
        },
    ))
}

impl UnleashStore {
    /// Reads the real policy only after revalidating the original fresh owner.
    /// Scope, actor, mandate, full digest and incarnation share this writer snapshot.
    /// Authorization and dispatch still repeat their own checks later.
    /// # Errors
    /// Rejects stale or unqualified ownership and policy; never reconstructs a token.
    pub fn provider_probe_connect_policy_snapshot(
        &self,
        claim: &ProviderProbeConnectClaim,
    ) -> Result<ProviderConnectPolicySnapshot, StoreError> {
        let permission = claim.permission();
        self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            let record = validate_live_permission(tx, &daemon, now, &permission)?;
            let context = &record.context;
            let summary = read_policy_summary_conn(tx, context.project_id)?;
            Ok(ProviderConnectPolicySnapshot {
                project_id: context.project_id,
                policy_revision: context.policy_revision,
                scope_revision: context.scope_revision,
                policy_digest: context.policy_digest,
                control_fence: unleash_domain::LineageFence::new(
                    daemon.authority_generation,
                    daemon.daemon_epoch,
                    unleash_domain::Fence::ZERO,
                ),
                isolation_mode: summary.effective_isolation_mode,
                guard_config: summary.guard_config,
            })
        })
    }
    /// Porte les phases du seul orchestrateur possédant le token frais original.
    /// Le port broker générique reste fermé pour connect, y compris son rejeu.
    /// # Errors
    /// Refuse une permission absente, périmée ou incompatible avec la phase.
    pub fn commit_provider_probe_connect_phase(
        &self,
        claim: &ProviderProbeConnectClaim,
        request: crate::BrokerPhaseRequest,
    ) -> Result<crate::BrokerPhaseCommit, StoreError> {
        let permission = claim.permission();
        let cutpoint = match &request.phase {
            crate::BrokerPhase::Authorizing { .. } => {
                Some(crate::StoreCutpoint::C3AfterBrokerIntentCommit)
            }
            crate::BrokerPhase::Authorization { .. } => {
                Some(crate::StoreCutpoint::C3AfterProviderProbeConnectAuthorizationCommit)
            }
            crate::BrokerPhase::Dispatch { .. } => {
                Some(crate::StoreCutpoint::C4AfterBrokerDispatchCommit)
            }
            crate::BrokerPhase::Observation { .. } => None,
        };
        let config = self.config().clone();
        let committed = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            crate::broker_writer::commit_connect_phase_tx(tx, &config, now, &request, &permission)
        })?;
        if let Some(point) = cutpoint {
            self.config().cutpoints.hit(point)?;
        }
        Ok(committed)
    }

    /// Revendique une seule fois le contexte sous son autorité courante.
    /// # Errors
    /// Refuse identité, reçu, lignée, époque, fence ou contexte incompatibles.
    pub fn claim_provider_probe_connect(
        &self,
        request: ClaimProviderProbeConnectRequest,
    ) -> Result<ProviderProbeConnectClaimOutcome, StoreError> {
        let config = self.config().clone();
        let outcome = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            claim_tx(tx, &config, now, &request)
        })?;
        if matches!(&outcome, ProviderProbeConnectClaimOutcome::FreshClaim(_)) {
            self.config()
                .cutpoints
                .hit(crate::StoreCutpoint::C3AfterProviderProbeConnectClaimCommit)?;
        }
        Ok(outcome)
    }

    /// Résout le mandat authentifié exact sans créer intention, reçu ou réservation.
    /// # Errors
    /// Refuse autorité, scope, version, phase ou mandat incompatibles.
    pub fn prepare_provider_probe_connect(
        &self,
        envelope: &CommandEnvelope,
        caller: &CallerAuthority,
    ) -> Result<ProviderProbeConnectPreparation, StoreError> {
        let envelope = envelope.clone();
        let caller = caller.clone();
        let preparation = self.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            let daemon = read_daemon_state_conn(tx)?;
            prepare_tx(tx, &daemon, now, &envelope, &caller)
        })?;
        crate::provider_profiles::verify_configuration_objects(
            self,
            &preparation.profile.configuration,
            preparation.profile.configuration_digest,
        )?;
        Ok(preparation)
    }

    /// Lit un contexte exact et son watermark, sans lui donner autorité après reprise.
    /// # Errors
    /// Refuse un document ou digest durable incohérent.
    pub fn inspect_provider_probe_connect_snapshot(
        &self,
        probe: ProviderProbeId,
    ) -> Result<SnapshotProjection<Option<ProviderProbeConnectRecord>>, StoreError> {
        self.project_snapshot(|conn| read_context(conn, probe))
    }
}
