// SPDX-License-Identifier: Apache-2.0
//! Versioned model preferences for exact published skill references (REQ-MODEL-002).

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use unleash_domain::{
    ActorId, AuthorityGeneration, IdGeneratorPort, ProjectId, ReceiptId, Sha256Digest,
};
use unleash_protocol::{CommandEnvelope, ReasonCode, canonical_sha256};
use uuid::Uuid;

use crate::{SettingsScopeKind, StoreError, provider_profiles, workflow_run_plan::VersionRef};

const MAX_WIRE_VERSION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Scope {
    Global { owner_id: () },
    Project { owner_id: ProjectId },
    Repository { owner_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SkillRef {
    #[serde(rename = "skill_id")]
    id: String,
    #[serde(rename = "skill_version")]
    version: u64,
    #[serde(rename = "skill_scope")]
    scope: Scope,
}

impl SkillRef {
    fn registry_scope(
        &self,
        project: Option<ProjectId>,
    ) -> Result<(&'static str, String), StoreError> {
        if self.id.len() > 128
            || !self.id.split(['.', '_', '-']).all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
            || !(1..=MAX_WIRE_VERSION).contains(&self.version)
        {
            return Err(conflict("invalid exact skill reference"));
        }
        match self.scope {
            Scope::Global { .. } => Ok(("global", "global".to_owned())),
            Scope::Project { owner_id } if project.is_none_or(|id| id == owner_id) => {
                Ok(("project", owner_id.to_string()))
            }
            _ => Err(conflict("skill is outside the supported settings scope")),
        }
    }

    fn from_role_entry(entry: VersionRef) -> Result<Self, StoreError> {
        let unavailable =
            || conflict("assigned primary skill requires an exact authorized namespace");
        let scope = entry.scope.ok_or_else(unavailable)?;
        let scope = match (scope.kind.as_str(), scope.owner_id.as_deref()) {
            ("global", None) => Scope::Global { owner_id: () },
            ("project", Some(id)) => Scope::Project {
                owner_id: id.parse().map_err(|_| unavailable())?,
            },
            _ => return Err(unavailable()),
        };
        Ok(Self {
            id: entry.id,
            version: u64::from(entry.version),
            scope,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PreferredProfileRef {
    profile_id: Uuid,
    profile_version: u64,
    #[serde(deserialize_with = "deserialize_profile_digest")]
    digest: Sha256Digest,
}

fn deserialize_profile_digest<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Sha256Digest, D::Error> {
    let value = String::deserialize(deserializer)?;
    let hex = value
        .strip_prefix("sha256:")
        .ok_or_else(|| serde::de::Error::custom("expected a prefixed SHA-256 digest"))?;
    Sha256Digest::parse_hex(hex).map_err(serde::de::Error::custom)
}

/// Closed `SetSettings` patch for one versioned model preference.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct ModelSelectionBindingChange(Change);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    Upsert {
        configuration_scope: Scope,
        skill_ref: SkillRef,
        preferred_profile_ref: PreferredProfileRef,
    },
    Remove {
        configuration_scope: Scope,
        skill_ref: SkillRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Active,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct BindingDocument {
    binding_id: Uuid,
    binding_version: u64,
    configuration_scope: Scope,
    skill_ref: SkillRef,
    preferred_profile_ref: Option<PreferredProfileRef>,
    state: State,
    command_receipt_id: ReceiptId,
    principal_id: ActorId,
}

/// Immutable version in the settings history returned by `ReadSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModelSelectionBinding {
    #[serde(flatten)]
    document: BindingDocument,
    digest: Sha256Digest,
}

impl ModelSelectionBinding {
    fn version_ref(&self) -> String {
        format!(
            "model-selection-binding:{}:{}:{}",
            self.document.binding_id, self.document.binding_version, self.digest,
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BindingEdit<'a> {
    pub command: &'a CommandEnvelope,
    pub actor: &'a ActorId,
    pub receipt_id: ReceiptId,
}

fn conflict(message: &str) -> StoreError {
    StoreError::ContractViolation {
        reason: Some(ReasonCode::ModelConfigurationConflict),
        message: message.to_owned(),
    }
}

pub(crate) fn append_tx(
    tx: &Transaction<'_>,
    settings: (SettingsScopeKind, &str),
    edit: BindingEdit<'_>,
    change: &ModelSelectionBindingChange,
    ids: &dyn IdGeneratorPort,
) -> Result<(), StoreError> {
    let (configuration_scope, skill_ref, profile) = match &change.0 {
        Change::Upsert {
            configuration_scope,
            skill_ref,
            preferred_profile_ref,
        } => (configuration_scope, skill_ref, Some(preferred_profile_ref)),
        Change::Remove {
            configuration_scope,
            skill_ref,
        } => (configuration_scope, skill_ref, None),
    };
    let (scope, scope_id) = settings;
    let project = edit.command.project_id;
    match (scope, configuration_scope, project) {
        (SettingsScopeKind::User, Scope::Global { .. }, None)
            if scope_id == "default"
                && edit.command.aggregate_id == edit.command.authority_generation.as_uuid() => {}
        (SettingsScopeKind::Project, Scope::Project { owner_id }, Some(project))
            if *owner_id == project
                && scope_id == project.to_string()
                && edit.command.aggregate_id == project.as_uuid() => {}
        _ => {
            return Err(conflict(
                "model preference is outside the edited settings scope",
            ));
        }
    }
    let (skill_scope, skill_owner) = skill_ref.registry_scope(project)?;

    if let Some(reference) = profile {
        require_published_skill(tx, skill_ref, skill_scope, &skill_owner)?;
        require_profile(tx, edit.command.authority_generation, project, reference)?;
    }
    let history_json: String = tx.query_row(
        "SELECT model_selection_bindings_json FROM settings WHERE scope_kind=?1 AND scope_id=?2",
        params![scope.as_str(), scope_id],
        |row| row.get(0),
    )?;
    let history: Vec<ModelSelectionBinding> = serde_json::from_str(&history_json)?;
    let previous = history.iter().rev().find(|binding| {
        &binding.document.configuration_scope == configuration_scope
            && &binding.document.skill_ref == skill_ref
    });
    if profile.is_none() && !previous.is_some_and(|binding| binding.document.state == State::Active)
    {
        return Err(conflict(
            "no active model preference for this exact skill and scope",
        ));
    }
    let (binding_id, binding_version) = match previous {
        Some(binding) if binding.document.binding_version < MAX_WIRE_VERSION => (
            binding.document.binding_id,
            binding.document.binding_version + 1,
        ),
        Some(_) => return Err(conflict("model preference version limit reached")),
        None => (ids.new_uuid(), 1),
    };
    let document = BindingDocument {
        binding_id,
        binding_version,
        configuration_scope: configuration_scope.clone(),
        skill_ref: skill_ref.clone(),
        preferred_profile_ref: profile.cloned(),
        state: if profile.is_some() {
            State::Active
        } else {
            State::Removed
        },
        command_receipt_id: edit.receipt_id,
        principal_id: edit.actor.clone(),
    };
    let binding = ModelSelectionBinding {
        digest: canonical_sha256(&document)?,
        document,
    };
    tx.execute(
        "UPDATE settings SET model_selection_bindings_json =
            json_insert(model_selection_bindings_json, '$[#]', json(?3))
         WHERE scope_kind=?1 AND scope_id=?2",
        params![scope.as_str(), scope_id, serde_json::to_string(&binding)?],
    )?;
    Ok(())
}

pub(crate) fn resolve_primary_skill_preference(
    conn: &Connection,
    authority: AuthorityGeneration,
    project: ProjectId,
    entry_skill: VersionRef,
) -> Result<Option<ModelSelectionBinding>, StoreError> {
    let skill = SkillRef::from_role_entry(entry_skill)?;
    skill.registry_scope(Some(project))?;
    for config_project in [Some(project), None] {
        let Some(binding) = current_binding(conn, authority, config_project, &skill)? else {
            continue;
        };
        let Some(profile) = binding.document.preferred_profile_ref.as_ref() else {
            continue;
        };
        require_profile(conn, authority, config_project, profile)?;
        return Ok(Some(binding));
    }
    Ok(None)
}

fn current_binding(
    conn: &Connection,
    authority: AuthorityGeneration,
    project: Option<ProjectId>,
    skill: &SkillRef,
) -> Result<Option<ModelSelectionBinding>, StoreError> {
    let (kind, id, aggregate, scope) = match project {
        Some(project) => (
            "project",
            project.to_string(),
            project.as_uuid(),
            Scope::Project { owner_id: project },
        ),
        None => (
            "user",
            "default".to_owned(),
            authority.as_uuid(),
            Scope::Global { owner_id: () },
        ),
    };
    let history: Option<String> = conn.query_row(
        "SELECT model_selection_bindings_json FROM settings WHERE scope_kind=?1 AND scope_id=?2",
        params![kind, id],
        |row| row.get(0),
    ).optional()?;
    let Some(history) = history else {
        return Ok(None);
    };
    let versions: Vec<ModelSelectionBinding> =
        serde_json::from_str(&history).map_err(|_| conflict("invalid model preference history"))?;
    let mut current: Option<ModelSelectionBinding> = None;
    for binding in versions
        .into_iter()
        .filter(|entry| &entry.document.skill_ref == skill)
    {
        let document = &binding.document;
        if document.configuration_scope != scope
            || document.binding_id.is_nil()
            || !(1..=MAX_WIRE_VERSION).contains(&document.binding_version)
            || current.as_ref().is_some_and(|previous| {
                previous.document.binding_id != document.binding_id
                    || previous.document.binding_version >= document.binding_version
            })
        {
            return Err(conflict(
                "conflicting model preference versions in the same scope",
            ));
        }
        current = Some(binding);
    }
    if let Some(binding) = &current {
        require_binding_provenance(conn, authority, aggregate, binding)?;
    }
    Ok(current)
}

fn require_binding_provenance(
    conn: &Connection,
    authority: AuthorityGeneration,
    aggregate: Uuid,
    binding: &ModelSelectionBinding,
) -> Result<(), StoreError> {
    let doc = &binding.document;
    if canonical_sha256(doc)? != binding.digest
        || (doc.state == State::Active) != doc.preferred_profile_ref.is_some()
    {
        return Err(conflict(
            "model preference contents do not match their provenance",
        ));
    }
    let recorded: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts
          WHERE receipt_id=?1 AND actor_id=?2 AND authority_generation=?3 AND aggregate_id=?4
            AND command_type='SetSettings' AND state IN ('accepted','completed')
            AND (response_state='tombstone' OR json_extract(response_json,'$.result_ref')=?5))",
        params![
            doc.command_receipt_id.to_string(),
            doc.principal_id.as_str(),
            authority.to_string(),
            aggregate.to_string(),
            binding.version_ref()
        ],
        |row| row.get(0),
    )?;
    if !recorded {
        return Err(conflict(
            "model preference receipt is unavailable in this scope",
        ));
    }
    Ok(())
}

fn require_published_skill(
    conn: &Connection,
    reference: &SkillRef,
    kind: &str,
    owner: &str,
) -> Result<(), StoreError> {
    let version =
        i64::try_from(reference.version).map_err(|_| conflict("invalid exact skill version"))?;
    let published: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM registry_versions v
         JOIN object_references ref ON ref.owner_kind='registry_version'
           AND ref.owner_id=v.registry_version_id AND ref.source_field='content_hash'
           AND ref.object_hash=v.content_hash AND ref.state='live'
         JOIN object_records o ON o.object_hash=ref.object_hash AND o.state='live'
         WHERE v.entry_kind='skill' AND v.scope_kind=?1 AND v.scope_owner_key=?2
           AND v.entry_id=?3 AND v.version=?4 AND v.publication_state='published')",
        params![kind, owner, reference.id, version],
        |row| row.get(0),
    )?;
    if !published {
        return Err(conflict("exact published skill is unavailable"));
    }
    Ok(())
}

fn require_profile(
    conn: &Connection,
    authority: AuthorityGeneration,
    project: Option<ProjectId>,
    reference: &PreferredProfileRef,
) -> Result<(), StoreError> {
    let unavailable = || conflict("exact configured profile is unavailable in this scope");
    if reference.profile_id.is_nil() || !(1..=MAX_WIRE_VERSION).contains(&reference.profile_version)
    {
        return Err(unavailable());
    }
    let profile =
        provider_profiles::resolve(conn, reference.profile_id, reference.profile_version)?
            .filter(|profile| {
                profile.configuration_digest == reference.digest
                    && profile
                        .configuration
                        .project_id
                        .is_none_or(|id| Some(id) == project)
            })
            .ok_or_else(unavailable)?;
    let retained: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM command_receipts c
         JOIN object_references ref ON ref.owner_kind='provider_profile'
           AND ref.owner_id=?4 AND ref.source_field='configuration'
           AND ref.object_hash=?5 AND ref.state='live'
         JOIN object_records o ON o.object_hash=ref.object_hash AND o.state='live'
         WHERE c.receipt_id=?1 AND c.aggregate_id=?2 AND c.authority_generation=?3
           AND c.actor_id=?6 AND c.command_type='SetProviderProfile'
           AND c.state IN ('accepted','completed'))",
        params![
            profile.receipt_id.to_string(),
            reference.profile_id.to_string(),
            authority.to_string(),
            profile.profile_version_ref,
            reference.digest.as_prefixed_str(),
            profile.actor_id.as_str(),
        ],
        |row| row.get(0),
    )?;
    if !retained {
        return Err(unavailable());
    }
    Ok(())
}

pub(crate) fn result_ref_tx(
    tx: &Transaction<'_>,
    scope: SettingsScopeKind,
    scope_id: &str,
    receipt_id: ReceiptId,
) -> Result<String, StoreError> {
    let result = tx
        .query_row(
            "SELECT json_extract(model_selection_bindings_json, '$[#-1]')
           FROM settings WHERE scope_kind=?1 AND scope_id=?2",
            params![scope.as_str(), scope_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let binding: ModelSelectionBinding = serde_json::from_str(&result.ok_or_else(|| {
        StoreError::InvariantViolation("missing committed settings binding".to_owned())
    })?)?;
    if binding.document.command_receipt_id != receipt_id {
        return Err(StoreError::InvariantViolation(
            "model preference and command receipt diverge".to_owned(),
        ));
    }
    Ok(binding.version_ref())
}
