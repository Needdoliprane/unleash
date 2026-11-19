//! Préparation F012 d'un contexte de transport, distinct du contexte modèle V009.
//! Aucun état d'action, effet réseau ou réserve n'est créé par la résolution.

use crate::commands::read_daemon_state_conn;
use crate::policies_secrets::read_policy_summary_conn;
use crate::{CallerAuthority, SnapshotProjection, StoreError, UnleashStore, WriteCriticality};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

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
}

impl ProviderProbeConnectClaim {
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

/// Seul FreshClaim permet au service de commencer l'orchestration broker.
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
    let crate::CommandMutation::PrepareProviderProbeConnect { preparation, .. } = &request.mutation
    else {
        return Ok(());
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

fn policy(
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
    if wire.phase() != ProviderTestPhase::Connect {
        return Err(invalid("phase connect exacte attendue"));
    }
    let profile_id: uuid::Uuid = wire.profile_id().parse().map_err(|_| hidden())?;
    if profile_id.is_nil() || profile_id.to_string() != wire.profile_id() {
        return Err(hidden());
    }
    let profile = crate::provider_profiles::resolve(tx, profile_id, wire.profile_version())?
        .ok_or_else(hidden)?;
    let grant = crate::provider_probe_mandates::resolve_connect_grant(
        tx, daemon, now, envelope, caller, &wire, &profile,
    )?;
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
    let current = prepare_tx(tx, daemon, now, command.envelope, command.caller)?;
    let mut original = command.preparation.clone();
    original.prepared_at_ms = now;
    if current != original || command.preparation.prepared_at_ms > now {
        return Err(invalid("préparation connect périmée ou divergente"));
    }
    let expiry = command
        .preparation
        .prepared_at_ms
        .as_i64()
        .checked_add(integer(current.wire.limits().timeout_ms())?)
        .ok_or_else(|| invalid("échéance connect hors plage"))?
        .min(current.grant.expires_at_ms.as_i64());
    if expiry <= now.as_i64() {
        return Err(hidden());
    }
    Ok(TimestampMs::new(expiry))
}

fn transport_digest(
    preparation: &ProviderProbeConnectPreparation,
) -> Result<Sha256Digest, StoreError> {
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
        phase: ProviderTestPhase::Connect,
        profile_id: profile.configuration.profile_id,
        profile_version: profile.configuration.profile_version,
        profile_version_ref: profile.profile_version_ref.clone(),
        configuration_digest: profile.configuration_digest,
        destination: profile.configuration.payload.endpoint().to_owned(),
        scope_revision: preparation.scope_revision,
        policy_revision: preparation.policy_revision,
        policy_digest: preparation.policy_digest,
        command_digest: preparation.command_digest,
        request_digest: transport_digest(preparation)?,
        locality: preparation.wire.locality(),
        capability_set: preparation.wire.capability_set().to_vec(),
        limits: preparation.wire.limits().clone(),
        deadline,
    })
}

pub(crate) fn commit_tx(
    tx: &Transaction<'_>,
    config: &crate::StoreConfig,
    daemon: &crate::DaemonStateRow,
    now: TimestampMs,
    command: ConnectCommand<'_>,
    receipt: ReceiptId,
    events: &mut Vec<crate::EventAppendSpec>,
) -> Result<Revision, StoreError> {
    let deadline = validate_preparation(tx, daemon, now, &command)?;
    let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)", [command.envelope.aggregate_id.to_string()], |row| row.get(0))?;
    if occupied {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: Revision::INITIAL,
            command_id: Some(command.envelope.command_id),
        });
    }
    let context = context(config, &command, receipt, deadline)?;
    let digest = unleash_protocol::canonical_sha256(&context)?;
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,source_run_id,profile_version_ref,context_hash,phase,state,wait_reason,admission_class,authority_generation,daemon_epoch,fence,revision,queued_at_ms) VALUES(?1,?2,?3,NULL,?4,?5,'connect','queued',NULL,'work',?6,?7,0,1,?8)", params![context.probe_id.to_string(),receipt.to_string(),context.principal_id.as_str(),context.profile_version_ref,digest.to_string(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,now.as_i64()])?;
    tx.execute("INSERT INTO provider_probe_transport_contexts(probe_id,receipt_id,actor_id,principal_id,project_id,profile_version_ref,authority_generation,daemon_epoch,action_id,mandate_ref,phase,context_digest,context_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'connect',?11,?12,?13)", params![context.probe_id.to_string(),receipt.to_string(),context.actor_id.as_str(),context.principal_id.as_str(),context.project_id.map(|id|id.to_string()),context.profile_version_ref,context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.action_id.to_string(),context.mandate_ref.to_string(),digest.to_string(),serde_json::to_string(&context)?,now.as_i64()])?;
    events.push(crate::EventAppendSpec {
        event_type:"ProviderProbePrepared".to_owned(), aggregate_type:crate::AggregateKind::ProviderProbe,
        aggregate_id:context.probe_id.as_uuid(), aggregate_revision:Revision::new(1), project_id:context.project_id,
        correlation_id:command.envelope.command_id.as_uuid(), causation_kind:unleash_protocol::EventCausationKind::Command,
        causation_id:None, payload:serde_json::json!({"probe_id":context.probe_id,"action_id":context.action_id,"phase":"connect","context_digest":digest,"mandate_ref":context.mandate_ref}).as_object().cloned().ok_or_else(|| invalid("événement connect invalide"))?,
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
}

fn read_context(
    conn: &Connection,
    probe: ProviderProbeId,
) -> Result<Option<ProviderProbeConnectRecord>, StoreError> {
    let row: Option<ConnectContextRow> = conn.query_row("SELECT context_json,context_digest,claim_id,claim_revision,claimed_at_ms FROM provider_probe_transport_contexts WHERE probe_id=?1", [probe.to_string()], |row| Ok(ConnectContextRow { json:row.get(0)?,digest:row.get(1)?,claim_id:row.get(2)?,claim_revision:row.get(3)?,claimed_at_ms:row.get(4)? })).optional()?;
    row.map(|row| {
        let context: ProviderProbeConnectContext = serde_json::from_str(&row.json)?;
        let digest: Sha256Digest = row.digest.parse()?;
        if context.probe_id != probe
            || context.schema_version != 1
            || context.phase != ProviderTestPhase::Connect
            || unleash_protocol::canonical_sha256(&context)? != digest
        {
            return Err(invalid("contexte connect durable incohérent"));
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

impl UnleashStore {
    /// Revendique une seule fois le contexte sous son autorité courante.
    /// # Errors
    /// Refuse identité, reçu, lignée, époque, fence ou contexte incompatibles.
    pub fn claim_provider_probe_connect(
        &self,
        _request: ClaimProviderProbeConnectRequest,
    ) -> Result<ProviderProbeConnectClaimOutcome, StoreError> {
        // TDD: no token is minted until the writer/CAS path is qualified.
        Err(StoreError::EvidenceUnavailable(
            "claim connect non raccordée".to_owned(),
        ))
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
