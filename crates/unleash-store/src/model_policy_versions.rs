// SPDX-License-Identifier: Apache-2.0
//! Versioned model authority published through Settings (REQ-MODEL-002).

use std::{collections::BTreeMap, fmt};

use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{ActorId, ProjectId, ReceiptId, Sha256Digest};
use unleash_protocol::{CommandEnvelope, ReasonCode, canonical_sha256};

use crate::SettingsScopeKind;
use crate::StoreError;
use crate::model_selection_bindings::{PreferredProfileRef, require_profile};
use crate::policies_secrets::{CallerAuthority, require_human_decider};

/// Plus grand entier qui garde sa valeur exacte entre clients JSON (borne du contrat IPC).
const MAX_WIRE_VERSION: u64 = 9_007_199_254_740_991;
/// Durée maximale d'une version, en secondes (trente jours).
const MAX_DURATION_SECONDS: u64 = 2_592_000;
/// Limite d'octets UTF-8 d'une référence de capacité sur le fil.
const MAX_CAPABILITY_BYTES: usize = 256;

/// Identité stable d'une politique, publiée dans le namespace de son scope.
///
/// La forme est celle, fermée, du contrat IPC : segments `a-z0-9` reliés par `.`, `_` ou `-`,
/// dans la limite de 128 octets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelPolicyId(String);

impl ModelPolicyId {
    /// Construit une identité validée.
    ///
    /// # Errors
    /// Retourne [`StoreError::InvalidArgument`] si la forme n'est pas celle du contrat IPC.
    pub fn new(value: impl Into<String>) -> Result<Self, StoreError> {
        let value = value.into();
        if value.len() > 128
            || !value.split(['.', '_', '-']).all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
        {
            return Err(invalid(
                "model policy identity is outside the closed contract form",
            ));
        }
        Ok(Self(value))
    }

    /// Identité sous forme textuelle.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelPolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ModelPolicyId {
    type Error = StoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ModelPolicyId> for String {
    fn from(value: ModelPolicyId) -> Self {
        value.0
    }
}

/// Plafonds d'une version publiée ; chacun s'intersecte avec les autres limites effectives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPolicyLimits {
    /// Nombre maximal d'appels de modèle ; zéro n'en autorise aucun.
    pub max_llm_calls: u64,
    /// Tokens de sortie cumulés autorisés.
    pub max_output_tokens: u64,
    /// Coût maximal, en millionièmes de dollar.
    pub max_cost_microusd: u64,
    /// Durée maximale, en secondes, de 1 à 2 592 000.
    pub max_duration_seconds: u64,
}

impl ModelPolicyLimits {
    /// Refuse un plafond hors des bornes du contrat IPC.
    fn validate(&self) -> Result<(), StoreError> {
        if self.max_llm_calls > MAX_WIRE_VERSION
            || self.max_output_tokens > MAX_WIRE_VERSION
            || self.max_cost_microusd > MAX_WIRE_VERSION
            || self.max_duration_seconds == 0
            || self.max_duration_seconds > MAX_DURATION_SECONDS
        {
            return Err(invalid(
                "model policy limits are outside the IPC contract bounds",
            ));
        }
        Ok(())
    }
}

/// Patch `SetSettings` fermé pour une politique de modèle versionnée.
///
/// Le numéro de version et l'auteur ne viennent jamais de l'appelant : la publication refuse une
/// version fournie et la révocation refuse tout paramètre de publication.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct ModelPolicyVersionChange(Change);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    /// Publie une nouvelle version dans le scope édité.
    Publish {
        policy_id: ModelPolicyId,
        allowed_profiles: Vec<PreferredProfileRef>,
        allowed_capabilities: Vec<String>,
        fallback_profiles: Vec<PreferredProfileRef>,
        limits: ModelPolicyLimits,
    },
    /// Révoque une version publiée exacte de ce scope.
    Revoke {
        policy_id: ModelPolicyId,
        version: u64,
    },
}

/// Fait distinct de révocation : il cible une version exacte sans modifier le document publié.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPolicyRevocation {
    /// Auteur de la révocation, dérivé du transport.
    principal_id: ActorId,
    /// Reçu de la commande de révocation.
    command_receipt_id: ReceiptId,
}

/// Version immuable de l'historique des réglages, rendue par `ReadSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPolicyVersion {
    policy_id: ModelPolicyId,
    version: u64,
    digest: Sha256Digest,
    policy_version_ref: String,
    allowed_profiles: Vec<PreferredProfileRef>,
    allowed_capabilities: Vec<String>,
    fallback_profiles: Vec<PreferredProfileRef>,
    limits: ModelPolicyLimits,
    /// Auteur de la publication ; une révocation ne le remplace pas.
    principal_id: ActorId,
    /// Reçu de la publication ; une révocation ne le remplace pas.
    command_receipt_id: ReceiptId,
    /// Révocation de cette version exacte, ou `None` tant qu'elle est active.
    revocation: Option<ModelPolicyRevocation>,
}

/// Document publié couvert par le digest ; ni la référence ni la révocation n'en font partie.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    policy_id: ModelPolicyId,
    version: u64,
    allowed_profiles: Vec<PreferredProfileRef>,
    allowed_capabilities: Vec<String>,
    fallback_profiles: Vec<PreferredProfileRef>,
    limits: ModelPolicyLimits,
    principal_id: ActorId,
    command_receipt_id: ReceiptId,
}

impl PolicyDocument {
    /// Digest canonique RFC 8785 du document publié.
    fn digest(&self) -> Result<Sha256Digest, StoreError> {
        Ok(canonical_sha256(self)?)
    }
}

/// Fait de l'historique `settings.model_policy_history_json` d'un scope.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Fact {
    /// Une version publiée : document et digest restent stables quand une révocation survient.
    Published {
        document: PolicyDocument,
        digest: Sha256Digest,
    },
    /// Une révocation, fait distinct ciblant une version exacte.
    Revoked {
        policy_id: ModelPolicyId,
        version: u64,
        revocation: ModelPolicyRevocation,
    },
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}

fn conflict(message: &str) -> StoreError {
    StoreError::ContractViolation {
        reason: Some(ReasonCode::ModelConfigurationConflict),
        message: message.to_owned(),
    }
}

fn version_ref(policy_id: &ModelPolicyId, version: u64, digest: &Sha256Digest) -> String {
    format!("model-policy:{policy_id}:{version}:{digest}")
}

/// Projette l'historique d'un scope en versions publiées, dans l'ordre des écritures.
fn project(history: &[Fact]) -> Result<Vec<ModelPolicyVersion>, StoreError> {
    let mut revocations: BTreeMap<(ModelPolicyId, u64), ModelPolicyRevocation> = BTreeMap::new();
    for fact in history {
        if let Fact::Revoked {
            policy_id,
            version,
            revocation,
        } = fact
            && revocations
                .insert((policy_id.clone(), *version), revocation.clone())
                .is_some()
        {
            return Err(conflict(
                "a model policy version carries more than one revocation",
            ));
        }
    }
    let mut versions: Vec<ModelPolicyVersion> = Vec::new();
    for fact in history {
        let Fact::Published { document, digest } = fact else {
            continue;
        };
        let identity = (document.policy_id.clone(), document.version);
        let duplicated = versions.iter().any(|published| {
            published.policy_id == document.policy_id && published.version == document.version
        });
        if &document.digest()? != digest || duplicated {
            return Err(conflict(
                "conflicting model policy versions in the same scope",
            ));
        }
        let revocation = revocations.remove(&identity);
        versions.push(ModelPolicyVersion {
            policy_version_ref: version_ref(&document.policy_id, document.version, digest),
            policy_id: document.policy_id.clone(),
            version: document.version,
            digest: *digest,
            allowed_profiles: document.allowed_profiles.clone(),
            allowed_capabilities: document.allowed_capabilities.clone(),
            fallback_profiles: document.fallback_profiles.clone(),
            limits: document.limits,
            principal_id: document.principal_id.clone(),
            command_receipt_id: document.command_receipt_id,
            revocation,
        });
    }
    if !revocations.is_empty() {
        return Err(conflict(
            "a model policy revocation has no published version",
        ));
    }
    Ok(versions)
}

/// Lit et projette l'historique brut `settings.model_policy_history_json` d'un scope.
pub(crate) fn parse(history_json: &str) -> Result<Vec<ModelPolicyVersion>, StoreError> {
    project(&serde_json::from_str::<Vec<Fact>>(history_json)?)
}

/// Fait de publication ou de révocation, engagé sous l'autorité authentifiée du transport.
#[derive(Clone, Copy)]
pub(crate) struct PolicyEdit<'a> {
    /// Enveloppe de la commande `SetSettings` ; son acteur et sa lignée font foi.
    pub command: &'a CommandEnvelope,
    /// Autorité authentifiée ; elle fournit le principal, jamais le payload.
    pub caller: &'a CallerAuthority,
    /// Reçu engagé dans la même transaction que le fait.
    pub receipt_id: ReceiptId,
}

/// Ajoute le fait à l'historique du scope, dans la transaction qui engage la révision des
/// réglages et le reçu (REQ-MODEL-002, historique V021 du contrat `SQLite`).
pub(crate) fn append_tx(
    tx: &Transaction<'_>,
    settings: (SettingsScopeKind, &str),
    edit: PolicyEdit<'_>,
    change: &ModelPolicyVersionChange,
) -> Result<(), StoreError> {
    // Une politique de modèle est une autorité : seul un décideur humain la publie ou la révoque,
    // jamais un agent ni un modèle (F012/interfaces, « autorité humaine sur le scope »).
    require_human_decider(edit.caller, "publish or revoke a model policy")?;
    let project = edited_scope(settings, edit.command)?;
    let history = read_history(tx, settings)?;
    let fact = match &change.0 {
        Change::Publish {
            policy_id,
            allowed_profiles,
            allowed_capabilities,
            fallback_profiles,
            limits,
        } => {
            validate_publish(
                allowed_profiles,
                allowed_capabilities,
                fallback_profiles,
                limits,
            )?;
            // L'éditeur n'autorise que des profils exacts de sa portée : la configuration n'élargit
            // jamais les droits (AC-F012-002).
            for reference in allowed_profiles.iter().chain(fallback_profiles) {
                require_profile(tx, edit.command.authority_generation, project, reference)?;
            }
            let document = PolicyDocument {
                policy_id: policy_id.clone(),
                version: next_version(&history, policy_id)?,
                allowed_profiles: allowed_profiles.clone(),
                allowed_capabilities: allowed_capabilities.clone(),
                fallback_profiles: fallback_profiles.clone(),
                limits: *limits,
                principal_id: edit.caller.actor_id.clone(),
                command_receipt_id: edit.receipt_id,
            };
            Fact::Published {
                digest: document.digest()?,
                document,
            }
        }
        Change::Revoke { policy_id, version } => {
            if !(1..=MAX_WIRE_VERSION).contains(version) {
                return Err(invalid(
                    "model policy version is outside the IPC contract bounds",
                ));
            }
            if !is_published(&history, policy_id, *version) {
                return Err(conflict(
                    "no published model policy version for this exact identity",
                ));
            }
            if is_revoked(&history, policy_id, *version) {
                return Err(conflict("model policy version is already revoked"));
            }
            Fact::Revoked {
                policy_id: policy_id.clone(),
                version: *version,
                revocation: ModelPolicyRevocation {
                    principal_id: edit.caller.actor_id.clone(),
                    command_receipt_id: edit.receipt_id,
                },
            }
        }
    };
    append_fact(tx, settings, &fact)
}

/// Portée de l'enveloppe, qui doit être celle du patch : un réglage utilisateur vise la lignée
/// active, un réglage projet vise le projet exact de la commande.
fn edited_scope(
    (scope, scope_id): (SettingsScopeKind, &str),
    envelope: &CommandEnvelope,
) -> Result<Option<ProjectId>, StoreError> {
    match (scope, envelope.project_id) {
        (SettingsScopeKind::User, None)
            if scope_id == "default"
                && envelope.aggregate_id == envelope.authority_generation.as_uuid() =>
        {
            Ok(None)
        }
        (SettingsScopeKind::Project, Some(project))
            if scope_id == project.to_string() && envelope.aggregate_id == project.as_uuid() =>
        {
            Ok(Some(project))
        }
        _ => Err(conflict(
            "model policy is outside the edited settings scope",
        )),
    }
}

/// Forme fermée du contrat IPC que la désérialisation seule ne couvre pas.
fn validate_publish(
    allowed_profiles: &[PreferredProfileRef],
    allowed_capabilities: &[String],
    fallback_profiles: &[PreferredProfileRef],
    limits: &ModelPolicyLimits,
) -> Result<(), StoreError> {
    limits.validate()?;
    if !is_unique(allowed_profiles)
        || !is_unique(fallback_profiles)
        || !is_unique(allowed_capabilities)
    {
        return Err(invalid("model policy lists carry a duplicate entry"));
    }
    if allowed_capabilities
        .iter()
        .any(|capability| !is_capability(capability))
    {
        return Err(invalid(
            "model policy capability is outside the closed contract form",
        ));
    }
    // Un profil est une version positive exacte, jamais « latest » (contrat IPC).
    if allowed_profiles
        .iter()
        .chain(fallback_profiles)
        .any(|reference| !reference.is_well_formed())
    {
        return Err(invalid(
            "model policy profile reference is outside the closed contract form",
        ));
    }
    // Chaque repli doit figurer parmi les profils autorisés (F012/interfaces).
    if fallback_profiles
        .iter()
        .any(|fallback| !allowed_profiles.contains(fallback))
    {
        return Err(conflict(
            "a model policy fallback is not an allowed profile",
        ));
    }
    Ok(())
}

fn is_unique<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(index, value)| !values.iter().take(index).any(|other| other == value))
}

/// Forme `probe_reference` du contrat IPC : au moins un caractère non blanc, aucun caractère de
/// contrôle, 256 octets UTF-8 au plus.
fn is_capability(capability: &str) -> bool {
    capability.len() <= MAX_CAPABILITY_BYTES
        && !capability.trim().is_empty()
        && !capability.chars().any(char::is_control)
}

fn is_published(history: &[Fact], policy_id: &ModelPolicyId, version: u64) -> bool {
    history.iter().any(|fact| {
        matches!(
            fact,
            Fact::Published { document, .. }
                if &document.policy_id == policy_id && document.version == version
        )
    })
}

fn is_revoked(history: &[Fact], policy_id: &ModelPolicyId, version: u64) -> bool {
    history.iter().any(|fact| {
        matches!(
            fact,
            Fact::Revoked {
                policy_id: revoked,
                version: revoked_version,
                ..
            } if revoked == policy_id && *revoked_version == version
        )
    })
}

fn read_history(
    tx: &Transaction<'_>,
    settings: (SettingsScopeKind, &str),
) -> Result<Vec<Fact>, StoreError> {
    let (scope, scope_id) = settings;
    let history: String = tx.query_row(
        "SELECT model_policy_history_json FROM settings WHERE scope_kind=?1 AND scope_id=?2",
        params![scope.as_str(), scope_id],
        |row| row.get(0),
    )?;
    Ok(serde_json::from_str(&history)?)
}

fn next_version(history: &[Fact], policy_id: &ModelPolicyId) -> Result<u64, StoreError> {
    let latest = history
        .iter()
        .filter_map(|fact| match fact {
            Fact::Published { document, .. } if &document.policy_id == policy_id => {
                Some(document.version)
            }
            _ => None,
        })
        .max();
    match latest {
        None => Ok(1),
        Some(version) if version < MAX_WIRE_VERSION => Ok(version + 1),
        Some(_) => Err(conflict("model policy version limit reached")),
    }
}

fn append_fact(
    tx: &Transaction<'_>,
    settings: (SettingsScopeKind, &str),
    fact: &Fact,
) -> Result<(), StoreError> {
    let (scope, scope_id) = settings;
    tx.execute(
        "UPDATE settings SET model_policy_history_json =
            json_insert(model_policy_history_json, '$[#]', json(?3))
         WHERE scope_kind=?1 AND scope_id=?2",
        params![scope.as_str(), scope_id, serde_json::to_string(fact)?],
    )?;
    Ok(())
}

/// Référence de résultat du reçu : la version publiée, ou le fait distinct de révocation.
pub(crate) fn result_ref_tx(
    tx: &Transaction<'_>,
    settings: (SettingsScopeKind, &str),
    receipt_id: ReceiptId,
) -> Result<String, StoreError> {
    let history = read_history(tx, settings)?;
    let diverged = || {
        StoreError::InvariantViolation("model policy fact and command receipt diverge".to_owned())
    };
    match history.last() {
        Some(Fact::Published { document, digest }) if document.command_receipt_id == receipt_id => {
            Ok(version_ref(&document.policy_id, document.version, digest))
        }
        Some(Fact::Revoked {
            policy_id,
            version,
            revocation,
        }) if revocation.command_receipt_id == receipt_id => {
            // Le dossier ne fixe pas la référence du reçu de révocation : elle désigne le fait
            // distinct par un suffixe, sans se confondre avec celle de la publication.
            let digest = published_digest(&history, policy_id, *version).ok_or_else(diverged)?;
            Ok(format!(
                "{}:revoked",
                version_ref(policy_id, *version, digest)
            ))
        }
        _ => Err(diverged()),
    }
}

fn published_digest<'a>(
    history: &'a [Fact],
    policy_id: &ModelPolicyId,
    version: u64,
) -> Option<&'a Sha256Digest> {
    history.iter().find_map(|fact| match fact {
        Fact::Published { document, digest }
            if &document.policy_id == policy_id && document.version == version =>
        {
            Some(digest)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn patch(command: &Value) -> &Value {
        command
            .pointer("/payload/patch/model_policy_version")
            .unwrap()
    }

    fn refused(value: Value) -> bool {
        serde_json::from_value::<ModelPolicyVersionChange>(value).is_err()
    }

    // T-F012-002 — Les DTO fermés ont exactement la forme des fixtures du dossier et refusent ce
    // que le contrat IPC interdit (REQ-MODEL-002).
    #[test]
    fn closed_dtos_follow_the_dossier_fixtures_and_refuse_forbidden_shapes() {
        let command: Value = serde_json::from_str(include_str!(
            "../../../.claude/examples/ipc-set-model-policy-version.json"
        ))
        .unwrap();
        let publish = patch(&command).clone();
        // Le patch n'est qu'une entrée : la fixture se décode dans sa variante, valeurs exactes.
        let ModelPolicyVersionChange(Change::Publish {
            policy_id,
            allowed_profiles,
            allowed_capabilities,
            fallback_profiles,
            limits,
        }) = serde_json::from_value(publish.clone()).unwrap()
        else {
            panic!("la fixture de publication doit se décoder comme une publication");
        };
        assert_eq!(policy_id.as_str(), "policy.fixture");
        assert_eq!(allowed_profiles.len(), 1);
        assert!(allowed_profiles.first().unwrap().is_well_formed());
        assert_eq!(allowed_capabilities, ["text", "streaming"]);
        assert!(fallback_profiles.is_empty());
        assert_eq!(
            limits,
            ModelPolicyLimits {
                max_llm_calls: 1,
                max_output_tokens: 128,
                max_cost_microusd: 10000,
                max_duration_seconds: 60,
            }
        );

        let mut revoke = publish.clone();
        let fields = revoke.as_object_mut().unwrap();
        fields.insert("operation".to_owned(), json!("revoke"));
        fields.insert("version".to_owned(), json!(1));
        for field in [
            "allowed_profiles",
            "allowed_capabilities",
            "fallback_profiles",
            "limits",
        ] {
            fields.remove(field);
        }
        let ModelPolicyVersionChange(Change::Revoke { policy_id, version }) =
            serde_json::from_value(revoke.clone()).unwrap()
        else {
            panic!("une révocation sans paramètre de publication doit se décoder");
        };
        assert_eq!(policy_id.as_str(), "policy.fixture");
        assert_eq!(version, 1);

        let mut refused_shapes = Vec::new();
        for (field, value) in [
            ("version", json!(1)),
            ("principal_id", json!("caller")),
            ("state", json!("active")),
        ] {
            let mut forbidden = publish.clone();
            forbidden
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), value);
            refused_shapes.push(forbidden);
        }
        let mut missing = publish.clone();
        missing.as_object_mut().unwrap().remove("limits");
        refused_shapes.push(missing);
        let mut negative = publish.clone();
        negative
            .pointer_mut("/limits/max_cost_microusd")
            .unwrap()
            .clone_from(&json!(-1));
        refused_shapes.push(negative);
        for (pointer, value) in [
            ("/allowed_profiles/0/profile_version", json!(0)),
            (
                "/allowed_profiles/0/digest",
                json!("sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            ),
        ] {
            let mut malformed = publish.clone();
            malformed.pointer_mut(pointer).unwrap().clone_from(&value);
            refused_shapes.push(malformed);
        }
        let oversized = "a".repeat(129);
        for identity in ["", "Policy", "policy..fixture", oversized.as_str()] {
            let mut malformed = publish.clone();
            malformed
                .as_object_mut()
                .unwrap()
                .insert("policy_id".to_owned(), json!(identity));
            refused_shapes.push(malformed);
        }
        let mut unnumbered = revoke.clone();
        unnumbered.as_object_mut().unwrap().remove("version");
        refused_shapes.push(unnumbered);
        let mut with_limits = revoke.clone();
        with_limits
            .as_object_mut()
            .unwrap()
            .insert("limits".to_owned(), json!({"max_llm_calls": 1}));
        refused_shapes.push(with_limits);
        for forbidden in refused_shapes {
            assert!(refused(forbidden));
        }

        let response: Value = serde_json::from_str(include_str!(
            "../../../.claude/examples/ipc-read-settings-model-policies-response.json"
        ))
        .unwrap();
        let history = response.pointer("/result/model_policy_versions").unwrap();
        let decoded: Vec<ModelPolicyVersion> = serde_json::from_value(history.clone()).unwrap();
        assert_eq!(&serde_json::to_value(&decoded).unwrap(), history);
        assert!(decoded.first().unwrap().revocation.is_some());
        assert!(decoded.get(1).unwrap().revocation.is_none());
    }
}
