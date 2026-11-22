//! Versions F012 de configuration, sans réseau, coffre ou qualification implicite.
//! Le service authentifié valide la normalisation et les sémantiques du catalogue.
//! Le Store recoupe les objets publiés, l'autorité F017 et la portée dans le writer.

use crate::commands::{
    AggregateKind, CommandMutation, CommitCommandRequest, DaemonStateRow, EventAppendSpec,
    ensure_scope_not_frozen_tx,
};
use crate::policies_secrets::{SubjectKind, ensure_subject_not_blocked_tx, require_security_admin};
use crate::{SecretReferenceInput, SnapshotProjection, StoreConfig, StoreError, UnleashStore};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unleash_domain::{ActorId, ProjectId, ReceiptId, Revision, Sha256Digest, TimestampMs};
use unleash_protocol::{
    EventCausationKind, ProviderAuthentication, ProviderKind, ProviderPricing, Sensitivity,
    SetProviderProfilePayload, canonical_sha256,
};
use uuid::Uuid;

/// Binding F017 résolu par le service ; aucune valeur de secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSecretBinding {
    /// Métadonnées exactes de la référence opaque.
    pub reference: SecretReferenceInput,
    /// Révision de ces métadonnées, revalidée à l'enregistrement.
    pub revision: Revision,
}

impl std::fmt::Debug for ProviderSecretBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSecretBinding")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// Provenance du catalogue validée par le producteur serveur, jamais une preuve réseau.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCatalogSource {
    /// Version OVPT dont la signature doit être qualifiée par le service.
    OvptRelease,
    /// Import explicite de l'opérateur.
    UserImport,
    /// Déclaration explicite de l'opérateur.
    UserDeclared,
}

impl ProviderCatalogSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::OvptRelease => "ovpt_release",
            Self::UserImport => "user_import",
            Self::UserDeclared => "user_declared",
        }
    }
}

/// Snapshot exact fourni par le resolver serveur ; le Store atteste les octets et l'entrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogBinding {
    /// Identité du catalogue.
    pub catalog_id: String,
    /// Version opaque, sans convention de concaténation pour le profil.
    pub catalog_version: String,
    /// Digest du contenu canonique publié.
    pub catalog_digest: Sha256Digest,
    /// Objet publié contenant le catalogue entier, paliers conservés.
    pub content_object_hash: Sha256Digest,
    /// Pointeur JSON vers l'entrée exacte ; son digest vient du payload fermé.
    pub entry_json_pointer: String,
    /// Origine contrôlée par le service.
    pub source: ProviderCatalogSource,
    /// Date de publication disponible.
    pub published_at_ms: TimestampMs,
    /// Date de validité disponible.
    pub valid_until_ms: TimestampMs,
    /// Date de l'import explicite.
    pub imported_at_ms: TimestampMs,
}

/// Objet complet, canonique et immuable, publié avant `CommitCommand`.
/// Les champs d'autorité sont produits par le service, pas par le payload IPC.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileConfiguration {
    /// Version du document, actuellement 1.
    pub schema_version: u32,
    /// UUID stable égal à l'agrégat de commande.
    pub profile_id: Uuid,
    /// Version attendue + 1, confirmée par le writer.
    pub profile_version: u64,
    /// Portée résolue de la session authentifiée.
    pub project_id: Option<ProjectId>,
    /// Configuration fermée, endpoint normalisé par le service.
    pub payload: SetProviderProfilePayload,
    /// Binding F017 exact, si l'authentification l'exige.
    pub secret_binding: Option<ProviderSecretBinding>,
    /// Provenance de l'entrée catalogue, si ce tarif est choisi.
    pub catalog_binding: Option<ProviderCatalogBinding>,
}

impl std::fmt::Debug for ProviderProfileConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProfileConfiguration")
            .field("profile_id", &self.profile_id)
            .field("profile_version", &self.profile_version)
            .finish_non_exhaustive()
    }
}

/// Préparation interne : objet déjà publié et configuration serveur validée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfilePreparation {
    /// Document exact.
    pub configuration: ProviderProfileConfiguration,
    /// Objet dont les octets canoniques sont vérifiés avant soumission au writer.
    pub content_object_hash: Sha256Digest,
}

/// Une déclaration enregistrée ne fournit aucune capacité ni preuve de placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProfileQualification {
    /// Une observation distincte de cette version est requise.
    Required,
}

/// Version configurée exacte ; les lignes historiques non attestées ne sont pas projetées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredProviderProfile {
    /// Clé technique opaque, indépendante de l'UUID/version.
    pub profile_version_ref: String,
    /// Contenu canonique épinglé.
    pub configuration: ProviderProfileConfiguration,
    /// Empreinte de ce contenu.
    pub configuration_digest: Sha256Digest,
    /// Principal humain réellement résolu en base.
    pub principal_id: ActorId,
    /// Acteur authentifié.
    pub actor_id: ActorId,
    /// Reçu réel de configuration.
    pub receipt_id: ReceiptId,
    /// Aucune qualification inférée de la déclaration.
    pub qualification: ProviderProfileQualification,
}

/// Métadonnées de la dernière version configurée d'un profil.
#[derive(Debug, Serialize)]
struct ProviderProfileListEntry {
    profile_id: Uuid,
    profile_version: u64,
    profile_version_ref: String,
    provider_kind: ProviderKind,
    model: String,
    qualification: ProviderProfileQualification,
}

/// Page de profils dans une seule portée.
#[derive(Debug, Serialize)]
pub struct ProviderProfilePage {
    profiles: Vec<ProviderProfileListEntry>,
    next_after_profile_id: Option<Uuid>,
}

impl UnleashStore {
    /// Dernières versions configurées dans la portée demandée et watermark de la même lecture.
    ///
    /// # Errors
    /// Retourne les erreurs de lecture ou un argument de pagination invalide.
    pub fn list_provider_profiles_snapshot(
        &self,
        project: Option<ProjectId>,
        after_profile_id: Option<Uuid>,
        limit: u32,
    ) -> Result<SnapshotProjection<ProviderProfilePage>, StoreError> {
        if !(1..=100).contains(&limit) || after_profile_id.is_some_and(|id| id.is_nil()) {
            return Err(invalid("pagination des profils invalide"));
        }
        let (scope_kind, scope_owner) = scope(project);
        let after = after_profile_id.map_or_else(String::new, |id| id.to_string());
        self.project_snapshot(|conn| {
            let mut statement = conn.prepare(
                "SELECT profile_id, MAX(profile_version) FROM provider_profiles
                 WHERE scope_kind = ?1 AND scope_owner_key = ?2
                   AND profile_id IS NOT NULL AND profile_version IS NOT NULL AND profile_id > ?3
                 GROUP BY profile_id ORDER BY profile_id LIMIT ?4",
            )?;
            let mut rows = statement.query(params![scope_kind, scope_owner, after, limit + 1])?;
            let mut profiles = Vec::<ProviderProfileListEntry>::new();
            let mut next_after_profile_id = None;
            while let Some(row) = rows.next()? {
                if profiles.len() == limit as usize {
                    next_after_profile_id = profiles.last().map(|entry| entry.profile_id);
                    break;
                }
                let profile_id = row.get::<_, String>(0)?.parse().map_err(|_| {
                    StoreError::InvariantViolation("UUID de profil stocké invalide".into())
                })?;
                let profile_version = u64::try_from(row.get::<_, i64>(1)?).map_err(|_| {
                    StoreError::InvariantViolation("version de profil stockée invalide".into())
                })?;
                let profile = resolve(conn, profile_id, profile_version)?.ok_or_else(|| {
                    StoreError::InvariantViolation(
                        "version de profil absente de la projection".into(),
                    )
                })?;
                let payload = profile.configuration.payload;
                profiles.push(ProviderProfileListEntry {
                    profile_id,
                    profile_version,
                    profile_version_ref: profile.profile_version_ref,
                    provider_kind: payload.provider_kind(),
                    model: payload.model().to_owned(),
                    qualification: profile.qualification,
                });
            }
            Ok(ProviderProfilePage {
                profiles,
                next_after_profile_id,
            })
        })
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("version de profil hors plage SQLite"))
}

fn scope(project: Option<ProjectId>) -> (&'static str, String) {
    project.map_or(("user", "default".to_owned()), |id| {
        ("project", id.to_string())
    })
}

/// Identité authentifiée de l'opération, indépendante d'une préparation recalculée.
pub(crate) fn validate_identity(request: &CommitCommandRequest) -> Result<bool, StoreError> {
    let CommandMutation::SetProviderProfile { caller, .. } = &request.mutation else {
        if request.envelope.command_type == "SetProviderProfile" {
            return Err(invalid("mutation de configuration profil absente"));
        }
        return Ok(false);
    };
    if caller.actor_id != request.actor_id
        || request.target_aggregate != AggregateKind::ProviderProfile
    {
        return Err(invalid("identité de configuration profil incohérente"));
    }
    validate_envelope_identity(&request.envelope, caller)?;
    Ok(true)
}

pub(crate) fn validate_envelope_identity(
    envelope: &unleash_protocol::CommandEnvelope,
    caller: &crate::CallerAuthority,
) -> Result<(), StoreError> {
    require_security_admin(caller, "SetProviderProfile")?;
    if envelope.command_type != "SetProviderProfile"
        || envelope.aggregate_id.is_nil()
        || envelope
            .actor_ref
            .as_ref()
            .is_some_and(|actor| actor != caller.actor_id.as_str())
    {
        return Err(invalid("identité de configuration profil incohérente"));
    }
    Ok(())
}

/// La préparation n'est validée qu'après absence attestée de reçu dans le writer.
pub(crate) fn validate_new_command(request: &CommitCommandRequest) -> Result<(), StoreError> {
    if !validate_identity(request)? {
        return Ok(());
    }
    let CommandMutation::SetProviderProfile { preparation, .. } = &request.mutation else {
        return Ok(());
    };
    let config = &preparation.configuration;
    let wire: SetProviderProfilePayload =
        serde_json::from_value(serde_json::Value::Object(request.envelope.payload.clone()))?;
    if config.schema_version != 1
        || config.profile_id.is_nil()
        || config.profile_id != request.envelope.aggregate_id
        || config.project_id != request.envelope.project_id
        || config.profile_version
            != request
                .envelope
                .expected_revision
                .as_u64()
                .checked_add(1)
                .ok_or_else(|| invalid("version maximale atteinte"))?
        || wire
            .with_normalized_endpoint()
            .map_err(|_| invalid("endpoint de profil invalide"))?
            != config.payload
        || !request.events.is_empty()
        || !request.capacity_reservations.is_empty()
        || !request.budget_reservations.is_empty()
        || request.fence_assertion.is_some()
        || canonical_sha256(config)? != preparation.content_object_hash
    {
        return Err(invalid("enveloppe de configuration profil incohérente"));
    }
    Ok(())
}

/// Vérifie l'objet réel hors transaction writer, sans résolution de secret.
pub(crate) fn verify_published(
    store: &crate::UnleashStore,
    request: &CommitCommandRequest,
) -> Result<(), StoreError> {
    let CommandMutation::SetProviderProfile { preparation, .. } = &request.mutation else {
        return Ok(());
    };
    verify_configuration_objects(
        store,
        &preparation.configuration,
        preparation.content_object_hash,
    )
}

pub(crate) fn verify_configuration_objects(
    store: &crate::UnleashStore,
    configuration: &ProviderProfileConfiguration,
    content_object_hash: Sha256Digest,
) -> Result<(), StoreError> {
    let bytes = store.read_verified_object_bytes(content_object_hash)?;
    if bytes != unleash_protocol::canonicalize_to_vec(configuration)? {
        return Err(invalid("objet de configuration divergent"));
    }
    if let Some(binding) = &configuration.catalog_binding {
        let bytes = store.read_verified_object_bytes(binding.content_object_hash)?;
        let catalog: serde_json::Value = serde_json::from_slice(&bytes)?;
        let ProviderPricing::Catalog {
            catalog_id,
            catalog_version,
            catalog_digest,
            entry_digest,
            ..
        } = configuration.payload.pricing()
        else {
            return Err(invalid("provenance catalogue sans tarif catalogue"));
        };
        let entry = catalog
            .pointer(&binding.entry_json_pointer)
            .ok_or_else(|| invalid("entrée exacte du catalogue absente"))?;
        if binding.entry_json_pointer.is_empty()
            || binding.catalog_id != *catalog_id
            || binding.catalog_version != *catalog_version
            || binding.catalog_digest != *catalog_digest
            || canonical_sha256(&catalog)? != *catalog_digest
            || binding.content_object_hash != *catalog_digest
            || canonical_sha256(entry)? != *entry_digest
        {
            return Err(invalid("catalogue ou entrée exacte divergents"));
        }
    }
    Ok(())
}

pub(crate) fn verify_secret(
    tx: &Transaction<'_>,
    config: &ProviderProfileConfiguration,
) -> Result<(), StoreError> {
    match (config.payload.auth(), &config.secret_binding) {
        (ProviderAuthentication::None, None) => Ok(()),
        (ProviderAuthentication::SecretRef { secret_ref }, Some(binding)) => {
            let r = &binding.reference;
            let (kind, owner) = scope(config.project_id);
            if &r.secret_ref != secret_ref
                || r.namespace != "provider"
                || r.purpose != "api_key"
                || r.secret_kind.trim().is_empty()
                || r.backend_kind.trim().is_empty()
                || r.target_kind != "provider_profile"
                || r.target_id != config.profile_id.to_string()
                || r.scope_kind != kind
                || r.scope_id != owner
            {
                return Err(invalid("binding secret du profil incompatible"));
            }
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM secret_references WHERE secret_ref_id=?1 AND opaque_secret_ref=?2 AND namespace=?3 AND purpose=?4 AND secret_kind=?5 AND target_kind=?6 AND target_id=?7 AND scope_kind=?8 AND scope_id=?9 AND backend_kind=?10 AND revision=?11 AND status='active' AND local_revoked_at_ms IS NULL)", params![r.secret_ref_id,r.secret_ref.as_opaque_str(),r.namespace,r.purpose,r.secret_kind,r.target_kind,r.target_id,r.scope_kind,r.scope_id,r.backend_kind,integer(binding.revision.as_u64())?], |row| row.get(0))?;
            if !valid {
                return Err(invalid("référence secrète absente, modifiée ou révoquée"));
            }
            Ok(())
        }
        _ => Err(invalid("authentification et binding secret divergents")),
    }
}

struct PricingRow {
    version: String,
    until: Option<i64>,
    source: &'static str,
    stale: &'static str,
}

fn pricing_row(config: &ProviderProfileConfiguration) -> Result<PricingRow, StoreError> {
    use unleash_protocol::ProviderPricingStalePolicy;
    let stale = |value: &ProviderPricingStalePolicy| match value {
        ProviderPricingStalePolicy::Block => Ok("block"),
        ProviderPricingStalePolicy::Warn => Ok("warn"),
        _ => Err(invalid("politique tarifaire inconnue")),
    };
    match (config.payload.pricing(), &config.catalog_binding) {
        (ProviderPricing::NoneLocal, None) => Ok(PricingRow {
            version: "none_local".to_owned(),
            until: None,
            source: "user_declared",
            stale: "block",
        }),
        (
            ProviderPricing::UserImport {
                entry,
                stale_policy,
            }
            | ProviderPricing::UserDeclared {
                entry,
                stale_policy,
            },
            None,
        ) => Ok(PricingRow {
            version: canonical_sha256(entry)?.to_string(),
            until: Some(integer(entry.valid_until_ms)?),
            source: if matches!(config.payload.pricing(), ProviderPricing::UserImport { .. }) {
                "user_import"
            } else {
                "user_declared"
            },
            stale: stale(stale_policy)?,
        }),
        (
            ProviderPricing::Catalog {
                catalog_id,
                catalog_version,
                catalog_digest,
                entry_digest,
                stale_policy,
            },
            Some(binding),
        ) if binding.catalog_id == *catalog_id
            && binding.catalog_version == *catalog_version
            && binding.catalog_digest == *catalog_digest =>
        {
            Ok(PricingRow {
                version: format!("{catalog_id}@{catalog_version}#{entry_digest}"),
                until: Some(binding.valid_until_ms.as_i64()),
                source: binding.source.as_str(),
                stale: stale(stale_policy)?,
            })
        }
        _ => Err(invalid("provenance tarifaire absente ou incompatible")),
    }
}

fn pin_object(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    hash: Sha256Digest,
    reference: &str,
    field: &str,
) -> Result<(), StoreError> {
    let live: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM object_records WHERE object_hash=?1 AND state='live')",
        [hash.to_string()],
        |row| row.get(0),
    )?;
    if !live {
        return Err(invalid("objet de configuration non publié"));
    }
    tx.execute("INSERT INTO object_references(reference_id,object_hash,owner_kind,owner_id,source_field,state) VALUES(?1,?2,'provider_profile',?3,?4,'live')", params![config.id_generator.new_uuid().to_string(), hash.to_string(), reference, field])?;
    Ok(())
}

fn bind_catalog(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    binding: &ProviderCatalogBinding,
    reference: &str,
) -> Result<(), StoreError> {
    pin_object(
        tx,
        config,
        binding.content_object_hash,
        reference,
        "catalog",
    )?;
    tx.execute("INSERT INTO model_catalog_snapshots(catalog_id,catalog_version,digest,source_kind,published_at_ms,valid_until_ms,imported_at_ms,content_ref) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(catalog_id,catalog_version) DO NOTHING", params![binding.catalog_id,binding.catalog_version,binding.catalog_digest.to_string(),binding.source.as_str(),binding.published_at_ms.as_i64(),Some(binding.valid_until_ms.as_i64()),binding.imported_at_ms.as_i64(),binding.content_object_hash.to_string()])?;
    let exact: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM model_catalog_snapshots WHERE catalog_id=?1 AND catalog_version=?2 AND digest=?3 AND source_kind=?4 AND published_at_ms IS ?5 AND valid_until_ms IS ?6 AND imported_at_ms=?7 AND content_ref=?8)", params![binding.catalog_id,binding.catalog_version,binding.catalog_digest.to_string(),binding.source.as_str(),binding.published_at_ms.as_i64(),Some(binding.valid_until_ms.as_i64()),binding.imported_at_ms.as_i64(),binding.content_object_hash.to_string()], |row| row.get(0))?;
    if !exact {
        return Err(invalid(
            "version catalogue déjà engagée avec une autre provenance",
        ));
    }
    Ok(())
}

fn validate_new_version(
    tx: &Transaction<'_>,
    doc: &ProviderProfileConfiguration,
    expected: Revision,
) -> Result<(), StoreError> {
    let current: i64 = tx.query_row(
        "SELECT COALESCE(MAX(profile_version),0) FROM provider_profiles WHERE profile_id=?1",
        [doc.profile_id.to_string()],
        |row| row.get(0),
    )?;
    if current != integer(expected.as_u64())? {
        return Err(StoreError::RevisionConflict {
            current_revision: Revision::new(
                u64::try_from(current).map_err(|_| invalid("version corrompue"))?,
            ),
            expected_revision: expected,
            command_id: None,
        });
    }
    if let Some(project) = doc.project_id {
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1 AND state='active')",
            [project.to_string()],
            |row| row.get(0),
        )?;
        if !active {
            return Err(invalid("projet de configuration absent ou inactif"));
        }
    }
    let (kind, owner) = scope(doc.project_id);
    let different: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_profiles WHERE profile_id=?1 AND (scope_kind!=?2 OR scope_owner_key!=?3))", params![doc.profile_id.to_string(),kind,owner], |row| row.get(0))?;
    if different {
        return Err(invalid("la portée du profil est immuable"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct ProfileCommand<'a> {
    pub envelope: &'a unleash_protocol::CommandEnvelope,
    pub actor_id: &'a ActorId,
    pub preparation: &'a ProviderProfilePreparation,
}

/// Engagement atomique de la configuration, références, événement et futur reçu.
pub(crate) fn set_tx(
    tx: &Transaction<'_>,
    config: &StoreConfig,
    daemon: &DaemonStateRow,
    now: TimestampMs,
    request: ProfileCommand<'_>,
    receipt: ReceiptId,
    events: &mut Vec<EventAppendSpec>,
) -> Result<Revision, StoreError> {
    if daemon.mode != unleash_domain::DaemonMode::Ready {
        return Err(StoreError::RecoveryInProgress(
            "configuration profil exige daemon ready".to_owned(),
        ));
    }
    let preparation = request.preparation;
    let doc = &preparation.configuration;
    validate_new_version(tx, doc, request.envelope.expected_revision)?;
    ensure_scope_not_frozen_tx(tx, doc.project_id, None, None)?;
    verify_secret(tx, doc)?;
    let pricing = pricing_row(doc)?;
    let principal: String = tx.query_row("SELECT principal_id FROM principals WHERE actor_id=?1 AND state='active' AND principal_kind='user'", [request.actor_id.as_str()], |row| row.get(0)).optional()?.ok_or_else(|| invalid("principal humain authentifié absent"))?;
    for subject in [principal.as_str(), request.actor_id.as_str()] {
        ensure_subject_not_blocked_tx(tx, SubjectKind::Actor, subject, doc.project_id)?;
    }
    let reference = config.id_generator.new_uuid().to_string();
    pin_object(
        tx,
        config,
        preparation.content_object_hash,
        &reference,
        "configuration",
    )?;
    if let Some(binding) = &doc.catalog_binding {
        bind_catalog(tx, config, binding, &reference)?;
    }
    let (kind, owner) = scope(doc.project_id);
    let digest = canonical_sha256(doc)?;
    let provider = serde_json::to_value(doc.payload.provider_kind())?
        .as_str()
        .ok_or_else(|| invalid("fournisseur inconnu"))?
        .to_owned();
    let locality = serde_json::to_value(doc.payload.locality())?
        .as_str()
        .ok_or_else(|| invalid("localité inconnue"))?
        .to_owned();
    tx.execute("INSERT INTO provider_profiles(profile_version_ref,provider_id,locality,pricing_version,price_valid_until_ms,source_kind,pricing_stale_policy,created_at_ms,profile_id,profile_version,config_object_hash,config_digest,config_json,preparation_digest,scope_kind,scope_owner_key,principal_id,actor_id,receipt_id,catalog_id,catalog_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)", params![reference,provider,locality,pricing.version,pricing.until,pricing.source,pricing.stale,now.as_i64(),doc.profile_id.to_string(),integer(doc.profile_version)?,preparation.content_object_hash.to_string(),digest.to_string(),serde_json::to_string(doc)?,canonical_sha256(preparation)?.to_string(),kind,owner,principal,request.actor_id.as_str(),receipt.to_string(),doc.catalog_binding.as_ref().map(|b| &b.catalog_id),doc.catalog_binding.as_ref().map(|b| &b.catalog_version)])?;
    let version = Revision::new(doc.profile_version);
    events.push(EventAppendSpec { event_type: "ProviderProfileConfigured".to_owned(), aggregate_type: AggregateKind::ProviderProfile, aggregate_id: doc.profile_id, aggregate_revision: version, project_id: doc.project_id, correlation_id: request.envelope.command_id.as_uuid(), causation_kind: EventCausationKind::Command, causation_id: None, payload: json!({"profile_id":doc.profile_id,"profile_version":doc.profile_version,"profile_version_ref":reference,"configuration_digest":digest,"qualification":"required","authority_generation":daemon.authority_generation}).as_object().cloned().ok_or_else(|| invalid("événement profil invalide"))?, sensitivity: Sensitivity::Restricted, outbox_destinations: vec!["clients".to_owned()] });
    Ok(version)
}

struct ProfileRow {
    reference: String,
    json: String,
    digest: String,
    object_hash: String,
    principal: String,
    actor: String,
    receipt: String,
    scope_kind: String,
    scope_owner: String,
    provider: String,
    locality: String,
    pricing_version: String,
    price_until: Option<i64>,
    source: String,
    stale: String,
}

impl ProfileRow {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            reference: row.get(0)?,
            json: row.get(1)?,
            digest: row.get(2)?,
            object_hash: row.get(3)?,
            principal: row.get(4)?,
            actor: row.get(5)?,
            receipt: row.get(6)?,
            scope_kind: row.get(7)?,
            scope_owner: row.get(8)?,
            provider: row.get(9)?,
            locality: row.get(10)?,
            pricing_version: row.get(11)?,
            price_until: row.get(12)?,
            source: row.get(13)?,
            stale: row.get(14)?,
        })
    }
}

/// Lecture exacte sans fallback et sans promotion d'une référence historique.
pub(crate) fn resolve(
    conn: &Connection,
    profile: Uuid,
    version: u64,
) -> Result<Option<ConfiguredProviderProfile>, StoreError> {
    let mut statement = conn.prepare("SELECT profile_version_ref,config_json,config_digest,config_object_hash,principal_id,actor_id,receipt_id,scope_kind,scope_owner_key,provider_id,locality,pricing_version,price_valid_until_ms,source_kind,pricing_stale_policy FROM provider_profiles WHERE profile_id=?1 AND profile_version=?2")?;
    let row = statement
        .query_row(
            params![profile.to_string(), integer(version)?],
            ProfileRow::read,
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let configuration: ProviderProfileConfiguration = serde_json::from_str(&row.json)?;
    let digest: Sha256Digest = row.digest.parse()?;
    let (expected_kind, expected_owner) = scope(configuration.project_id);
    let pricing = pricing_row(&configuration)?;
    if configuration.profile_id != profile
        || configuration.profile_version != version
        || canonical_sha256(&configuration)? != digest
        || row.object_hash != digest.to_string()
        || row.scope_kind != expected_kind
        || row.scope_owner != expected_owner
        || json!(row.provider) != serde_json::to_value(configuration.payload.provider_kind())?
        || json!(row.locality) != serde_json::to_value(configuration.payload.locality())?
        || row.pricing_version != pricing.version
        || row.price_until != pricing.until
        || row.source != pricing.source
        || row.stale != pricing.stale
    {
        return Err(StoreError::InvariantViolation(
            "métadonnées et contenu du profil divergent".to_owned(),
        ));
    }
    Ok(Some(ConfiguredProviderProfile {
        profile_version_ref: row.reference,
        configuration,
        configuration_digest: digest,
        principal_id: ActorId::new(row.principal)?,
        actor_id: ActorId::new(row.actor)?,
        receipt_id: row.receipt.parse()?,
        qualification: ProviderProfileQualification::Required,
    }))
}

/// Recoupe une référence configurée au point d'utilisation, sans promouvoir le legacy.
pub(crate) fn validate_probe_use(
    tx: &Transaction<'_>,
    preparation: &crate::ProviderProbePreparation,
    project: Option<ProjectId>,
) -> Result<(), StoreError> {
    let identity: Option<Option<String>> = tx
        .query_row(
            "SELECT profile_id FROM provider_profiles WHERE profile_version_ref=?1",
            [&preparation.profile_version_ref],
            |row| row.get(0),
        )
        .optional()?;
    // The wire UUID cannot acquire configured provenance by pointing at legacy data.
    // Non-UUID historical fixtures keep their original reference-only interpretation.
    let identity = match (preparation.requested_profile_id.parse::<Uuid>(), identity) {
        (Ok(requested), _) => requested,
        (Err(_), Some(Some(identity))) => identity
            .parse()
            .map_err(|_| invalid("UUID du profil corrompu"))?,
        (Err(_), _) => return Ok(()),
    };
    let record = resolve(tx, identity, preparation.requested_profile_version)?
        .ok_or_else(|| invalid("version exacte du profil absente"))?;
    let doc = &record.configuration;
    let secret_refs = match doc.payload.auth() {
        ProviderAuthentication::None => vec![],
        ProviderAuthentication::SecretRef { secret_ref } => vec![secret_ref.clone()],
        _ => return Err(invalid("authentification inconnue")),
    };
    if record.profile_version_ref != preparation.profile_version_ref
        || doc.profile_id.to_string() != preparation.requested_profile_id
        || doc.project_id != project
        || doc.payload.model() != preparation.resolved_model_id
        || secret_refs != preparation.secret_refs
        || preparation.limits.timeout_ms > doc.payload.limits().generation_timeout_ms
    {
        return Err(invalid("probe divergent de la version configurée exacte"));
    }
    verify_secret(tx, doc)
}
