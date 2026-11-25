//! Préparation F012 sans agent/session ; aucun réseau ni résolution de secret.
//! Le service authentifié fournit ses références résolues. Le writer les lie au
//! reçu réel et au profil durable, puis Authorizing matérialise les dépendances.
//! La résolution du catalogue et la dérivation de la fixture/politique sont des
//! responsabilités de ce service de confiance, pas des faits établis par un hash
//! publié. Le dispatch reste fermé tant que ces producteurs et le placement ne
//! sont pas qualifiés ; aucune lecture de secret ou de fichier n'a lieu en Tx.

use crate::commands::{
    AggregateKind, CommandMutation, CommitCommandRequest, DaemonStateRow, EventAppendSpec,
    ensure_scope_not_frozen_tx, read_daemon_state_conn,
};
use crate::policies_secrets::{
    SubjectKind, ensure_subject_not_blocked_tx, read_policy_summary_conn, require_security_admin,
};
use crate::{BrokerCause, BrokerIntent, StoreConfig, StoreError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use unleash_domain::{
    ActionId, ActorId, AttemptId, AuthorityGeneration, DaemonEpoch, DaemonMode, DataLocality,
    Fence, InvocationId, LineageFence, ManifestId, MicrosAmount, ModelCallId, ModelPolicy,
    ProjectId, ProviderProbeId, ReceiptId, Revision, RunId, SecretRef, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    EventCausationKind, ProviderDialectKind, ProviderServerManagement, Sensitivity,
    canonical_sha256,
};

/// Verbe du producteur de probe ; aucune autre source ne peut s'y substituer.
pub const PROVIDER_PROBE_VERB: &str = "model.provider_probe";

const WIRE_KEYS: &[&str] = &[
    "profile_id",
    "profile_version",
    "phase",
    "capability_set",
    "limits",
    "locality",
    "mandate_ref",
];
pub(crate) const WAIT_FOR_PLACEMENT: &str = "provider_probe_transport_unqualified";

fn requires_generation_placement(
    schema_version: i64,
    preparation: &ProviderProbePreparation,
) -> bool {
    schema_version >= 15
        && preparation
            .requested_profile_id
            .parse::<uuid::Uuid>()
            .is_ok()
}

/// Bornes réduites par le serveur avant préparation, jamais une autorisation réseau.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeGenerationLimits {
    /// Délai maximal (1..=60 000 ms).
    pub timeout_ms: u64,
    /// Appels maximaux (1..=3).
    pub max_calls: u64,
    /// Sortie cumulée maximale (1..=512 tokens).
    pub max_output_tokens: u64,
    /// Réserve monétaire demandée (au plus 100 000 micro-USD).
    pub max_cost_usd_micros: MicrosAmount,
}

/// Références résolues par le service ; cette structure n'est pas le payload client.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbePreparation {
    /// Identité wire attestée par le resolver authentifié, sans convention de clé.
    pub requested_profile_id: String,
    /// Version wire attestée par ce même resolver.
    pub requested_profile_version: u64,
    /// Clé durable exacte, résolue depuis le profil client par le service.
    pub profile_version_ref: String,
    /// Modèle exact choisi dans ce profil, sans fallback.
    pub resolved_model_id: String,
    /// Capacités demandées ; le snapshot distingue les observations des inconnues.
    pub capability_set: Vec<String>,
    /// Snapshot durable de capacités, sans qualification inventée.
    pub capability_snapshot_hash: Sha256Digest,
    /// Identité de fixture synthétique installée, résolue côté service.
    pub fixture_id: String,
    /// Contenu exact publié de cette fixture.
    pub fixture_digest: Sha256Digest,
    /// Payload effectif déjà publié, exclusivement issu de la fixture.
    pub payload_manifest_hash: Sha256Digest,
    /// Politique modèle effective publiée.
    pub model_policy_hash: Sha256Digest,
    /// Politique de données effective publiée.
    pub data_policy_hash: Sha256Digest,
    /// Politique dérivée de l'origine réelle, pas d'un `origin_kind` client.
    pub model_policy: ModelPolicy,
    /// Localité requise ; une valeur nulle de coût ne prouve pas la localité.
    pub locality: DataLocality,
    /// Mandat autorisé résolu par le service.
    pub mandate_ref: String,
    /// Origine run dérivée par le backend ; reste fermée sans son producteur qualifié.
    pub source_run_id: Option<RunId>,
    /// Références opaques résolues depuis le profil côté service, jamais des secrets.
    pub secret_refs: Vec<SecretRef>,
    /// Bornes effectives au plus égales à celles demandées.
    pub limits: ProbeGenerationLimits,
}

impl fmt::Debug for ProviderProbePreparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderProbePreparation")
            .field("payload_manifest_hash", &self.payload_manifest_hash)
            .field("source_run_id", &self.source_run_id)
            .finish_non_exhaustive()
    }
}

/// Union canonique de propriétaire d'appel modèle, distincte du sujet broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelOrigin {
    /// Invocation réelle et sa tentative.
    Invocation {
        /// Invocation propriétaire.
        invocation_id: InvocationId,
        /// Tentative exacte.
        attempt_id: AttemptId,
        /// Bundle immuable de l'invocation.
        invocation_bundle_digest: Sha256Digest,
    },
    /// Probe authentifié, sans parent agent.
    ProviderProbe {
        /// Probe réel.
        provider_probe_id: ProviderProbeId,
        /// Contexte exact.
        probe_context_digest: Sha256Digest,
    },
}

/// Union canonique de référence de contexte modèle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelContextRef {
    /// Manifeste réel lié à une session d'invocation.
    InvocationContext {
        /// Identité du manifeste.
        context_manifest_id: ManifestId,
    },
    /// Contexte autonome de probe.
    ProviderProbeContext {
        /// Identité du probe.
        provider_probe_id: ProviderProbeId,
        /// Digest du contexte autonome.
        probe_context_digest: Sha256Digest,
    },
}

/// Snapshot immuable des métadonnées présentes dans `provider_profiles`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeProfileSnapshot {
    /// Référence exacte, pas un digest de profil partiel inventé.
    pub profile_version_ref: String,
    /// Fournisseur de ce profil durable.
    pub provider_id: String,
    /// Localité déclarée ; le transport devra qualifier le placement réel.
    pub declared_locality: DataLocality,
    /// Version de tarification figée.
    pub pricing_version: String,
    /// Échéance tarifaire disponible, sans inférence d'une valeur absente.
    pub price_valid_until_ms: Option<TimestampMs>,
}

/// Document complet immuable ; ne comporte aucun agent ni session fictive.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProbeContext {
    /// Version du document.
    pub schema_version: u32,
    /// Identité du probe.
    pub probe_id: ProviderProbeId,
    /// Vrai reçu de préparation.
    pub command_receipt_id: ReceiptId,
    /// Acteur authentifié.
    pub actor_id: ActorId,
    /// Principal résolu en base.
    pub principal_id: ActorId,
    /// Scope projet authentique ou utilisateur global.
    pub project_id: Option<ProjectId>,
    /// Autorité de préparation.
    pub authority_generation: AuthorityGeneration,
    /// Incarnation de préparation.
    pub daemon_epoch: DaemonEpoch,
    /// Identité stable de l'action allouée sans pré-insertion.
    pub action_id: ActionId,
    /// Appel logique stable.
    pub model_call_id: ModelCallId,
    /// Manifest request stable.
    pub manifest_id: ManifestId,
    /// Paramètres résolus et bornés.
    pub preparation: ProviderProbePreparation,
    /// Profil durable exact observé.
    pub profile: ProbeProfileSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ollama_endpoint: Option<OllamaProbeEndpoint>,
    /// Révision du scope.
    pub scope_revision: Revision,
    /// Révision de politique de sécurité.
    pub policy_revision: Revision,
    /// Digest du snapshot de politique de sécurité réellement lu.
    pub security_policy_digest: Sha256Digest,
    /// Deadline calculée une seule fois à la préparation.
    pub deadline: TimestampMs,
    /// Mode imposé par le service.
    pub context_mode: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OllamaProbeEndpoint {
    normalized_url: String,
    host: String,
    port: u16,
}

/// Projection interne, dont le digest ne dépend jamais d'une lecture client.
#[derive(Clone, PartialEq, Eq)]
pub struct PreparedProviderProbe {
    /// Document durable complet.
    pub context: ProviderProbeContext,
    /// Digest RFC 8785 du document.
    pub context_digest: Sha256Digest,
}

impl fmt::Debug for PreparedProviderProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedProviderProbe")
            .field("probe_id", &self.context.probe_id)
            .field("action_id", &self.context.action_id)
            .field("context_digest", &self.context_digest)
            .finish_non_exhaustive()
    }
}

impl PreparedProviderProbe {
    /// Origine modèle exacte, distincte de l'origine `control_plane` du broker.
    #[must_use]
    pub fn model_origin(&self) -> ModelOrigin {
        ModelOrigin::ProviderProbe {
            provider_probe_id: self.context.probe_id,
            probe_context_digest: self.context_digest,
        }
    }
    /// Référence de contexte exacte.
    #[must_use]
    pub fn context_ref(&self) -> ModelContextRef {
        ModelContextRef::ProviderProbeContext {
            provider_probe_id: self.context.probe_id,
            probe_context_digest: self.context_digest,
        }
    }
    /// Document à publier par le protocole objet, hors transaction, avant Authorizing.
    /// # Errors
    /// Retourne une erreur de sérialisation du document fermé.
    pub fn request_manifest(&self) -> Result<Value, StoreError> {
        Ok(
            json!({"schema_version":1,"manifest_id":self.context.manifest_id,"model_call_id":self.context.model_call_id,"action_id":self.context.action_id,"manifest_kind":"request","ordinal":1,"origin_ref":self.model_origin(),"context_ref":self.context_ref(),"provider_profile_version":self.context.profile.profile_version_ref,"provider_id":self.context.profile.provider_id,"resolved_model_id":self.context.preparation.resolved_model_id,"capabilities_hash":self.context.preparation.capability_snapshot_hash,"allowed_model_policy_hash":self.context.preparation.model_policy_hash,"data_policy_hash":self.context.preparation.data_policy_hash,"request_payload_hash":self.context.preparation.payload_manifest_hash,"generation_limits":self.context.preparation.limits,"deadline":self.context.deadline}),
        )
    }
    /// Digest du payload/manifest de transport figé et publié par le service.
    #[must_use]
    pub const fn payload_digest(&self) -> Sha256Digest {
        self.context.preparation.payload_manifest_hash
    }
    /// Ressources exactes du manifeste natif du broker.
    #[must_use]
    pub fn resource_refs(&self) -> Value {
        let mut resources = vec![json!({
            "kind":"model_target","value":{"provider":self.context.profile.provider_id,
                "model_id":self.context.preparation.resolved_model_id,"locality":self.context.profile.declared_locality}
        })];
        if let Some(endpoint) = &self.context.ollama_endpoint {
            resources.push(json!({
                "kind":"network_endpoint","value":{"host":endpoint.host,
                    "port":endpoint.port,"is_private_or_loopback":true}
            }));
        }
        Value::Array(resources)
    }
}

fn invalid(message: &'static str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("entier probe hors plage SQLite"))
}
fn valid_ref(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn validate_command(
    tx: &Transaction<'_>,
    request: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let (probe_id, caller, preparation) = match &request.mutation {
        CommandMutation::PrepareProviderProbe {
            probe_id,
            caller,
            preparation,
        } => (*probe_id, caller, preparation),
        CommandMutation::PrepareAuthenticatedProviderGeneration {
            caller,
            preparation,
        } => (
            ProviderProbeId::from_uuid(request.envelope.aggregate_id),
            caller,
            &preparation.preparation,
        ),
        _ if request.envelope.command_type == "TestProviderProfile" => {
            return Err(invalid("mutation de probe absente"));
        }
        _ => return Ok(()),
    };
    require_security_admin(caller, "TestProviderProfile")?;
    if caller.actor_id != request.actor_id
        || request
            .envelope
            .actor_ref
            .as_ref()
            .is_some_and(|actor| actor != request.actor_id.as_str())
        || request.envelope.command_type != "TestProviderProfile"
        || request.target_aggregate != AggregateKind::ProviderProbe
        || request.envelope.aggregate_id != probe_id.as_uuid()
        || probe_id.as_uuid().is_nil()
        || request.envelope.expected_revision != Revision::INITIAL
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
        || request.fence_assertion.is_some()
    {
        return Err(invalid("enveloppe de préparation probe incohérente"));
    }
    let payload = &request.envelope.payload;

    if payload.len() != WIRE_KEYS.len()
        || payload.keys().any(|key| !WIRE_KEYS.contains(&key.as_str()))
        || payload.get("profile_id") != Some(&json!(preparation.requested_profile_id))
        || payload.get("profile_version") != Some(&json!(preparation.requested_profile_version))
        || payload.get("phase") != Some(&json!("generation_probe"))
        || payload.get("capability_set") != Some(&json!(preparation.capability_set))
        || payload.get("locality") != Some(&json!(preparation.locality))
        || payload.get("mandate_ref") != Some(&json!(preparation.mandate_ref))
    {
        return Err(invalid("payload de génération probe incohérent"));
    }
    let limits = payload
        .get("limits")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("limites probe absentes"))?;
    if limits.len() != 4 {
        return Err(invalid("limites probe doivent être fermées"));
    }
    for (name, value) in [
        ("timeout_ms", preparation.limits.timeout_ms),
        ("max_calls", preparation.limits.max_calls),
        ("max_output_tokens", preparation.limits.max_output_tokens),
        (
            "max_cost_usd_micros",
            preparation.limits.max_cost_usd_micros.as_u64(),
        ),
    ] {
        if limits
            .get(name)
            .and_then(Value::as_u64)
            .is_none_or(|maximum| value > maximum)
        {
            return Err(invalid(
                "les bornes effectives du probe dépassent la demande",
            ));
        }
    }
    let prior:Option<(String,String)>=tx.query_row("SELECT p.probe_id,p.preparation_digest FROM provider_probe_contexts p JOIN command_receipts c ON c.receipt_id=p.receipt_id WHERE c.actor_id=?1 AND c.command_id=?2",params![request.actor_id.as_str(),request.envelope.command_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((prior, digest)) = prior
        && (prior != probe_id.to_string() || digest != canonical_sha256(preparation)?.to_string())
    {
        return Err(StoreError::IdempotencyConflict {
            command_id: request.envelope.command_id,
        });
    }
    Ok(())
}

fn profile(conn: &Connection, reference: &str) -> Result<ProbeProfileSnapshot, StoreError> {
    let (provider,locality,pricing,until):(String,String,String,Option<i64>)=conn.query_row("SELECT provider_id,locality,pricing_version,price_valid_until_ms FROM provider_profiles WHERE profile_version_ref=?1",[reference],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?.ok_or_else(||invalid("profil durable exact absent"))?;
    let declared_locality = match locality.as_str() {
        "local_only" => DataLocality::LocalOnly,
        "approved_remote" => DataLocality::ApprovedRemote,
        _ => return Err(invalid("localité du profil corrompue")),
    };
    Ok(ProbeProfileSnapshot {
        profile_version_ref: reference.to_owned(),
        provider_id: provider,
        declared_locality,
        pricing_version: pricing,
        price_valid_until_ms: until.map(TimestampMs::new),
    })
}

fn ollama_endpoint(
    tx: &Transaction<'_>,
    preparation: &ProviderProbePreparation,
) -> Result<Option<OllamaProbeEndpoint>, StoreError> {
    let identity = preparation
        .requested_profile_id
        .parse()
        .map_err(|_| invalid("identité de profil Ollama invalide"))?;
    let profile =
        crate::provider_profiles::resolve(tx, identity, preparation.requested_profile_version)?
            .ok_or_else(|| invalid("profil Ollama exact absent"))?;
    let payload = &profile.configuration.payload;
    if payload.dialect().kind != ProviderDialectKind::OllamaChat
        || payload.dialect().version != 1
        || payload.server_management() != ProviderServerManagement::UnleashManaged
        || payload.locality() != DataLocality::LocalOnly
        || preparation.locality != DataLocality::LocalOnly
        || !preparation.secret_refs.is_empty()
    {
        return Ok(None);
    }
    let destination = payload
        .endpoint_destination()
        .map_err(|_| invalid("destination Ollama invalide"))?;
    if destination.scheme() != "http" || !destination.is_loopback_literal() {
        return Ok(None);
    }
    Ok(Some(OllamaProbeEndpoint {
        normalized_url: destination.normalized_endpoint().to_owned(),
        host: destination.host().to_owned(),
        port: destination.port(),
    }))
}

fn verify_objects(
    tx: &Transaction<'_>,
    preparation: &ProviderProbePreparation,
) -> Result<(), StoreError> {
    for hash in [
        preparation.fixture_digest,
        preparation.payload_manifest_hash,
        preparation.capability_snapshot_hash,
        preparation.model_policy_hash,
        preparation.data_policy_hash,
    ] {
        let live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
            [hash.to_string()],
            |row| row.get(0),
        )?;
        if !live {
            return Err(invalid("référence de préparation probe non publiée"));
        }
    }
    Ok(())
}

fn validate_preparation_tx(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    preparation: &ProviderProbePreparation,
) -> Result<(), StoreError> {
    if daemon.mode != DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "préparation probe exige daemon ready".to_owned(),
        ));
    }
    if preparation.model_policy != ModelPolicy::AllowedBounded {
        return Err(StoreError::Unauthorized(
            "model_policy forbidden interdit le probe".to_owned(),
        ));
    }
    if let Some(source) = preparation.source_run_id {
        let policy: Option<String> = tx
            .query_row(
                "SELECT model_policy FROM automation_runs WHERE run_id=?1",
                [source.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if policy.as_deref() == Some("forbidden") {
            return Err(StoreError::Unauthorized(
                "le run source interdit tout modèle transitif".to_owned(),
            ));
        }
        return Err(invalid("producteur probe issu d'un run non qualifié"));
    }
    if !valid_ref(&preparation.requested_profile_id)
        || preparation.requested_profile_version == 0
        || !valid_ref(&preparation.profile_version_ref)
        || !valid_ref(&preparation.resolved_model_id)
        || !valid_ref(&preparation.fixture_id)
        || !valid_ref(&preparation.mandate_ref)
        || preparation.capability_set.len() > 64
        || preparation
            .capability_set
            .iter()
            .any(|value| !valid_ref(value))
        || preparation
            .capability_set
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != preparation.capability_set.len()
        || !(1..=60_000).contains(&preparation.limits.timeout_ms)
        || !(1..=3).contains(&preparation.limits.max_calls)
        || !(1..=512).contains(&preparation.limits.max_output_tokens)
        || preparation.limits.max_cost_usd_micros.as_u64() > 100_000
    {
        return Err(invalid("références ou bornes probe invalides"));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Identités et cause explicites de la mutation de préparation atomique"
)]
pub(crate) fn prepare_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    probe: ProviderProbeId,
    receipt: ReceiptId,
    actor: &ActorId,
    project: Option<ProjectId>,
    preparation: &ProviderProbePreparation,
    grant: Option<&crate::provider_probe_mandates::ResolvedConnectGrant>,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    validate_preparation_tx(tx, daemon, preparation)?;
    if daemon.schema_version >= 11 {
        crate::provider_profiles::validate_probe_use(tx, preparation, project)?;
    }
    if requires_generation_placement(daemon.schema_version, preparation) && grant.is_none() {
        return Err(StoreError::EvidenceUnavailable(
            "préparation configurée sans mandat de génération vérifié".into(),
        ));
    }
    ensure_scope_not_frozen_tx(tx, project, None, None)?;
    let prior:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_probes WHERE probe_id=?1) OR EXISTS(SELECT 1 FROM command_receipts WHERE aggregate_id=?1)",[probe.to_string()],|row|row.get(0))?;
    if prior {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(1),
            expected_revision: Revision::INITIAL,
            command_id: None,
        });
    }
    let principal: String = if let Some(grant) = grant {
        grant.principal_id.to_string()
    } else {
        tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind IN ('user','service','system')",[actor.as_str()],|row|row.get(0)).optional()?.ok_or_else(||invalid("principal authentifié absent"))?
    };
    for subject in [&principal, actor.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, project)?;
    }
    let profile = profile(tx, &preparation.profile_version_ref)?;
    if preparation.locality == DataLocality::LocalOnly
        && profile.declared_locality != DataLocality::LocalOnly
    {
        return Err(invalid("profil distant incompatible avec local_only"));
    }
    if profile.declared_locality == DataLocality::LocalOnly
        && preparation.limits.max_cost_usd_micros.as_u64() != 0
    {
        return Err(invalid("coût local qualifié doit être explicite et nul"));
    }
    verify_objects(tx, preparation)?;
    let deadline = now
        .as_i64()
        .checked_add(integer(preparation.limits.timeout_ms)?)
        .ok_or_else(|| invalid("deadline probe hors plage"))?;
    let deadline = grant.map_or(deadline, |grant| deadline.min(grant.expires_at_ms.as_i64()));
    if profile.declared_locality == DataLocality::ApprovedRemote
        && profile
            .price_valid_until_ms
            .is_none_or(|until| until.as_i64() < deadline)
    {
        return Err(invalid(
            "tarification distante non valable sur la fenêtre du probe",
        ));
    }
    let scope: i64 = if let Some(project) = project {
        tx.query_row(
            "SELECT revision FROM projects WHERE project_id=?1 AND state='active'",
            [project.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("projet probe absent ou inactif"))?
    } else {
        tx.query_row("SELECT COALESCE((SELECT revision FROM settings WHERE scope_kind='user' AND scope_id='default'),0)",[],|row|row.get(0))?
    };
    let (policy_revision, security_policy_digest) =
        crate::provider_probe_connect::policy(tx, project)?;
    let ollama_endpoint = if grant.is_some() && profile.provider_id == "ollama" {
        ollama_endpoint(tx, preparation)?
    } else {
        None
    };
    let context = ProviderProbeContext {
        schema_version: if ollama_endpoint.is_some() { 2 } else { 1 },
        probe_id: probe,
        command_receipt_id: receipt,
        actor_id: actor.clone(),
        principal_id: ActorId::new(principal.clone())?,
        project_id: project,
        authority_generation: daemon.authority_generation,
        daemon_epoch: daemon.daemon_epoch,
        action_id: ActionId::from_uuid(config.id_generator.new_uuid()),
        model_call_id: ModelCallId::from_uuid(config.id_generator.new_uuid()),
        manifest_id: ManifestId::from_uuid(config.id_generator.new_uuid()),
        preparation: preparation.clone(),
        profile,
        ollama_endpoint,
        scope_revision: Revision::new(
            u64::try_from(scope).map_err(|_| invalid("scope probe corrompu"))?,
        ),
        policy_revision,
        security_policy_digest,
        deadline: TimestampMs::new(deadline),
        context_mode: "explicit_unleash_history".to_owned(),
    };
    let digest = canonical_sha256(&context)?;
    // CommitCommand creates this receipt later in the SAME transaction.
    tx.pragma_update(None, "defer_foreign_keys", "ON")?;
    tx.execute("INSERT INTO provider_probes(probe_id,command_receipt_id,principal_id,source_run_id,profile_version_ref,context_hash,phase,state,wait_reason,admission_class,authority_generation,daemon_epoch,fence,revision,queued_at_ms) VALUES(?1,?2,?3,NULL,?4,?5,'generation_probe','queued','provider_probe_preparing','work',?6,?7,0,1,?8)",params![probe.to_string(),receipt.to_string(),principal,preparation.profile_version_ref,digest.to_string(),daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,now.as_i64()])?;
    tx.execute("INSERT INTO provider_probe_contexts(probe_id,receipt_id,actor_id,principal_id,project_id,profile_version_ref,authority_generation,daemon_epoch,action_id,model_call_id,manifest_id,preparation_digest,context_digest,context_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![probe.to_string(),receipt.to_string(),actor.as_str(),principal,project.map(|id|id.to_string()),preparation.profile_version_ref,daemon.authority_generation.to_string(),integer(daemon.daemon_epoch.as_u64())?,context.action_id.to_string(),context.model_call_id.to_string(),context.manifest_id.to_string(),canonical_sha256(preparation)?.to_string(),digest.to_string(),serde_json::to_string(&context)?,now.as_i64()])?;
    events.push(EventAppendSpec{event_type:"ProviderProbePrepared".to_owned(),aggregate_type:AggregateKind::ProviderProbe,aggregate_id:probe.as_uuid(),aggregate_revision:Revision::new(1),project_id:project,correlation_id:probe.as_uuid(),causation_kind:EventCausationKind::Command,causation_id:None,payload:json!({"probe_id":probe,"action_id":context.action_id,"model_call_id":context.model_call_id,"context_digest":digest}).as_object().cloned().ok_or_else(||invalid("événement probe invalide"))?,sensitivity:Sensitivity::Restricted,outbox_destinations:vec!["clients".to_owned(),"harness".to_owned()]});
    Ok(Revision::new(1))
}

pub(crate) fn read_context(
    conn: &Connection,
    probe: ProviderProbeId,
) -> Result<Option<PreparedProviderProbe>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 9 {
        return Ok(None);
    }
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT context_json,context_digest FROM provider_probe_contexts WHERE probe_id=?1",
            [probe.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(json, digest)| {
        let context: ProviderProbeContext = serde_json::from_str(&json)?;
        let digest: Sha256Digest = digest.parse()?;
        let expected_version = if context.ollama_endpoint.is_some() {
            2
        } else {
            1
        };
        if context.probe_id != probe
            || context.schema_version != expected_version
            || canonical_sha256(&context)? != digest
        {
            return Err(invalid("contexte probe durable divergent"));
        }
        Ok(PreparedProviderProbe {
            context,
            context_digest: digest,
        })
    })
    .transpose()
}

fn for_action(
    conn: &Connection,
    action: ActionId,
) -> Result<Option<PreparedProviderProbe>, StoreError> {
    if read_daemon_state_conn(conn)?.schema_version < 9 {
        return Ok(None);
    }
    let probe: Option<String> = conn
        .query_row(
            "SELECT probe_id FROM provider_probe_contexts WHERE action_id=?1",
            [action.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    probe
        .map(|probe| read_context(conn, probe.parse()?))
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn validate_incarnation(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    action: ActionId,
) -> Result<(), StoreError> {
    if let Some(record) = for_action(tx, action)? {
        if record.context.authority_generation != daemon.authority_generation {
            return Err(StoreError::StaleAuthorityGeneration {
                expected: daemon.authority_generation,
                actual: record.context.authority_generation,
                command_id: None,
            });
        }
        if record.context.daemon_epoch != daemon.daemon_epoch {
            return Err(StoreError::StaleFence {
                message: "ancienne incarnation du probe préparé".to_owned(),
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_intent(
    tx: &Transaction<'_>,
    daemon: &DaemonStateRow,
    intent: &BrokerIntent,
) -> Result<(), StoreError> {
    let record = for_action(tx, intent.action_id)?;
    let receipt_type: Option<String> =
        if let BrokerCause::CommandReceipt { receipt_id } = intent.cause {
            tx.query_row(
                "SELECT command_type FROM command_receipts WHERE receipt_id=?1",
                [receipt_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
        } else {
            None
        };
    if record.is_none()
        && receipt_type.as_deref() != Some("TestProviderProfile")
        && intent.manifest.get("verb") != Some(&json!(PROVIDER_PROBE_VERB))
    {
        return Ok(());
    }
    let record = record.ok_or_else(|| invalid("préparation de probe absente"))?;
    validate_incarnation(tx, daemon, intent.action_id)?;
    let context = &record.context;
    if context.principal_id != intent.principal_id
        || intent.cause
            != (BrokerCause::CommandReceipt {
                receipt_id: context.command_receipt_id,
            })
        || context.project_id != intent.project_id
        || intent.scope_revision != context.scope_revision
        || intent.policy_revision != context.policy_revision
        || intent.action_deadline != context.deadline
        || intent.lineage_fence
            != LineageFence::new(
                context.authority_generation,
                context.daemon_epoch,
                Fence::new(0),
            )
        || record.payload_digest() != intent.payload_digest
        || intent.manifest.get("verb") != Some(&json!(PROVIDER_PROBE_VERB))
        || intent.manifest.get("effects_class") != Some(&json!("model_dispatch"))
        || intent.manifest.get("resource_refs") != Some(&record.resource_refs())
        || intent.manifest.get("secret_refs") != Some(&json!(context.preparation.secret_refs))
        || intent.manifest.get("provenance") != Some(&json!({"origin":"engine_internal"}))
    {
        return Err(invalid(
            "intention probe divergente de sa préparation authentique",
        ));
    }
    verify_frozen_inputs(tx, &record)?;
    Ok(())
}

pub(crate) fn verify_frozen_inputs(
    tx: &Transaction<'_>,
    record: &PreparedProviderProbe,
) -> Result<(), StoreError> {
    let context = &record.context;
    ensure_scope_not_frozen_tx(tx, context.project_id, None, None)?;
    let schema_version = read_daemon_state_conn(tx)?.schema_version;
    if schema_version >= 11 {
        crate::provider_profiles::validate_probe_use(tx, &context.preparation, context.project_id)?;
    }
    if let Some(expected) = &context.ollama_endpoint
        && ollama_endpoint(tx, &context.preparation)?.as_ref() != Some(expected)
    {
        return Err(invalid("destination Ollama différente du profil figé"));
    }
    for subject in [context.principal_id.as_str(), context.actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, context.project_id)
            .map_err(|error| match error {
                StoreError::CapabilityDenied { reason, .. } => StoreError::CapabilityDenied {
                    reason,
                    message: "sujet du probe suspendu ou révoqué".to_owned(),
                },
                other => other,
            })?;
    }
    if profile(tx, &context.profile.profile_version_ref)? != context.profile {
        return Err(invalid("profil du probe changé depuis préparation"));
    }
    let summary = read_policy_summary_conn(tx, context.project_id)?;
    if crate::policies_secrets::engaged_policy_digest(&summary)? != context.security_policy_digest
    {
        return Err(invalid("politique du probe changée depuis préparation"));
    }
    verify_objects(tx, &context.preparation)?;
    let receipt:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM command_receipts c JOIN principals p ON p.principal_id=?1 WHERE c.receipt_id=?2 AND c.actor_id=?3 AND p.actor_id=c.actor_id AND p.state='active' AND c.command_type='TestProviderProfile' AND c.aggregate_id=?4 AND c.authority_generation=?5 AND c.state='accepted' AND c.response_state='full')",params![context.principal_id.as_str(),context.command_receipt_id.to_string(),context.actor_id.as_str(),context.probe_id.to_string(),context.authority_generation.to_string()],|row|row.get(0))?;
    if !receipt {
        return Err(invalid("reçu ou propriétaire du probe non attribuable"));
    }
    let manifest_hash = canonical_sha256(&record.request_manifest()?)?;
    let available: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
        [manifest_hash.to_string()],
        |row| row.get(0),
    )?;
    if !available {
        return Err(invalid("manifest request probe non publié"));
    }
    if requires_generation_placement(schema_version, &context.preparation) {
        let owned: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM object_references WHERE object_hash=?1
             AND owner_kind='provider_probe' AND owner_id=?2
             AND source_field='request_manifest' AND state='live')",
            params![manifest_hash.to_string(), context.probe_id.to_string()],
            |row| row.get(0),
        )?;
        if !owned {
            return Err(StoreError::EvidenceUnavailable(
                "manifest request sans référence du probe exact".into(),
            ));
        }
    }
    let scope: i64 = if let Some(project) = context.project_id {
        tx.query_row(
            "SELECT revision FROM projects WHERE project_id=?1 AND state='active'",
            [project.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("projet probe absent ou inactif"))?
    } else {
        tx.query_row("SELECT COALESCE((SELECT revision FROM settings WHERE scope_kind='user' AND scope_id='default'),0)",[],|row|row.get(0))?
    };
    if scope != integer(context.scope_revision.as_u64())? {
        return Err(invalid("scope du probe changé depuis préparation"));
    }
    Ok(())
}

pub(crate) fn materialize(
    tx: &Transaction<'_>,
    intent: &BrokerIntent,
    trigger_event: &str,
    now: TimestampMs,
) -> Result<(), StoreError> {
    let Some(record) = for_action(tx, intent.action_id)? else {
        return Ok(());
    };
    let context = &record.context;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM model_calls WHERE model_call_id=?1)",
        [context.model_call_id.to_string()],
        |row| row.get(0),
    )?;
    if exists {
        return validate_admission(tx, context.probe_id, now, None);
    }
    tx.execute("INSERT INTO model_calls(model_call_id,action_id,owner_kind,provider_probe_id,trigger_event_id,state,revision) VALUES(?1,?2,'provider_probe',?3,?4,'prepared',1)",params![context.model_call_id.to_string(),context.action_id.to_string(),context.probe_id.to_string(),trigger_event])?;
    tx.execute("INSERT INTO model_call_manifests(manifest_id,model_call_id,manifest_kind,ordinal,context_manifest_id,context_kind,provider_probe_context_id,provider_id,resolved_model_id,capabilities_hash,allowed_model_policy_hash,request_payload_hash,content_object_hash,created_at_ms) VALUES(?1,?2,'request',1,NULL,'provider_probe_context',?3,?4,?5,?6,?7,?8,?9,?10)",params![context.manifest_id.to_string(),context.model_call_id.to_string(),context.probe_id.to_string(),context.profile.provider_id,context.preparation.resolved_model_id,context.preparation.capability_snapshot_hash.to_string(),context.preparation.model_policy_hash.to_string(),context.preparation.payload_manifest_hash.to_string(),canonical_sha256(&record.request_manifest()?)?.to_string(),now.as_i64()])?;
    let pending_placement = requires_generation_placement(
        read_daemon_state_conn(tx)?.schema_version,
        &context.preparation,
    )
    .then_some(WAIT_FOR_PLACEMENT);
    let changed=tx.execute("UPDATE provider_probes SET wait_reason=?4,revision=revision+1 WHERE probe_id=?1 AND state='queued' AND wait_reason='provider_probe_preparing' AND authority_generation=?2 AND daemon_epoch=?3 AND fence=0",params![context.probe_id.to_string(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,pending_placement])?;
    if changed != 1 {
        return Err(invalid("état du probe ne permet pas sa matérialisation"));
    }
    Ok(())
}

pub(crate) fn validate_admission(
    tx: &Transaction<'_>,
    probe: ProviderProbeId,
    now: TimestampMs,
    eligibility: Option<uuid::Uuid>,
) -> Result<(), StoreError> {
    let Some(record) = read_context(tx, probe)? else {
        if read_daemon_state_conn(tx)?.schema_version >= 9 {
            return Err(invalid("contexte probe V009 absent avant admission"));
        }
        return Ok(());
    };
    let context = &record.context;
    let daemon = read_daemon_state_conn(tx)?;
    validate_incarnation(tx, &daemon, context.action_id)?;
    if context.deadline.as_i64() <= now.as_i64() {
        return Err(invalid("deadline probe expirée avant admission"));
    }
    verify_frozen_inputs(tx, &record)?;
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM model_calls c JOIN actions a ON a.action_id=c.action_id JOIN model_call_manifests m ON m.model_call_id=c.model_call_id WHERE c.model_call_id=?1 AND c.action_id=?2 AND c.owner_kind='provider_probe' AND c.provider_probe_id=?3 AND a.origin_kind='control_plane' AND a.control_principal_id=?4 AND a.authority_generation=?5 AND a.daemon_epoch=?6 AND a.state='authorizing' AND m.manifest_id=?7 AND m.context_kind='provider_probe_context' AND m.provider_probe_context_id=?3 AND m.context_manifest_id IS NULL)",params![context.model_call_id.to_string(),context.action_id.to_string(),probe.to_string(),context.principal_id.as_str(),context.authority_generation.to_string(),integer(context.daemon_epoch.as_u64())?,context.manifest_id.to_string()],|row|row.get(0))?;
    if !valid {
        return Err(invalid(
            "relation action/model_call/probe absente avant admission",
        ));
    }
    if requires_generation_placement(daemon.schema_version, &context.preparation) {
        crate::provider_probe_generation::placement::validate_admission(
            tx,
            &daemon,
            now,
            &record,
            eligibility,
        )?;
    }
    Ok(())
}

pub(crate) fn reject_unqualified_dispatch(
    tx: &Transaction<'_>,
    action: ActionId,
) -> Result<(), StoreError> {
    if for_action(tx, action)?.is_some() {
        return Err(invalid(
            "dispatch probe fermé : producteurs placement, SecretStore et transport non qualifiés",
        ));
    }
    Ok(())
}

/// Validate a proposal against the frozen product budget without reserving anything.
pub(crate) fn validate_admission_budgets(
    tx: &Transaction<'_>,
    probe: ProviderProbeId,
    budgets: &[crate::BudgetReservationRequest],
) -> Result<(), StoreError> {
    let Some(record) = read_context(tx, probe)? else {
        return Ok(());
    };
    let context = &record.context;
    if context.profile.declared_locality == DataLocality::LocalOnly && budgets.is_empty() {
        // A local profile needs no invented monetary price/reservation, but a
        // zero new charge still observes overruns of every applicable account.
        let overrun:Option<String>=tx.query_row("SELECT account_id FROM budget_accounts WHERE currency='USD' AND (scope_type='global' OR scope_type='project' AND scope_id=?1 OR scope_type='provider' AND scope_id=?2) AND spent_micros>limit_micros-reserved_micros ORDER BY account_id LIMIT 1",params![context.project_id.map(|id|id.to_string()),context.profile.provider_id],|row|row.get(0)).optional()?;
        if let Some(account_id) = overrun {
            return Err(StoreError::BudgetExhausted { account_id });
        }
        return Ok(());
    }
    let [budget] = budgets else {
        return Err(invalid("budget du probe absent ou multiple"));
    };
    if budget.action_id != context.action_id
        || budget.model_id != context.preparation.resolved_model_id
        || budget.pricing_version != context.profile.pricing_version
        || budget.pricing_status != crate::PricingCheckStatus::Valid
        || budget.max_request_hash != record.payload_digest()
        || budget.charges.is_empty()
        || budget
            .charges
            .iter()
            .any(|charge| charge.max_micros != context.preparation.limits.max_cost_usd_micros)
    {
        return Err(invalid("budget du probe divergent de sa préparation"));
    }
    let mut statement=tx.prepare("SELECT account_id,scope_type FROM budget_accounts WHERE currency='USD' AND (scope_type='global' OR scope_type='project' AND scope_id=?1 OR scope_type='provider' AND scope_id=?2)")?;
    let rows = statement
        .query_map(
            params![
                context.project_id.map(|id| id.to_string()),
                context.profile.provider_id
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    if !rows.iter().any(|(_, scope)| scope == "global")
        || (context.project_id.is_some() && !rows.iter().any(|(_, scope)| scope == "project"))
    {
        return Err(invalid("comptes obligatoires du probe non configurés"));
    }
    let applicable = rows
        .into_iter()
        .map(|(id, _)| id)
        .collect::<std::collections::BTreeSet<_>>();
    let supplied = budget
        .charges
        .iter()
        .map(|charge| charge.account_id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    if supplied.len() != budget.charges.len() || applicable != supplied {
        return Err(invalid(
            "ensemble des comptes probe incomplet, dupliqué ou hors portée",
        ));
    }
    Ok(())
}
