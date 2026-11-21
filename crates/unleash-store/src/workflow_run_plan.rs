//! Resolve a published workflow and its exact role, skill, contract and scope inputs.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use unleash_domain::{MissionId, ProjectId, Revision, Sha256Digest};
use unleash_protocol::from_json_str_strict;

use crate::{
    StoreConfig, StoreError, objects::read_verified_object_bytes_conn,
    workflow_activation::push_workflow_children,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionScope {
    scope_id: String,
    scope_revision: Revision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MissionScopes {
    schema_version: u32,
    mission_id: MissionId,
    mission_revision: Revision,
    scopes: BTreeMap<String, ExecutionScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VersionRef {
    id: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<RegistryScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegistryScope {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_id: Option<String>,
}

impl RegistryScope {
    fn owner(&self) -> Option<&str> {
        self.owner_id
            .as_deref()
            .or_else(|| (self.kind == "global").then_some("global"))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
enum WorkflowLevel {
    N,
    #[serde(rename = "N+1")]
    NPlusOne,
    #[serde(rename = "N+2")]
    NPlusTwo,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Slot {
    Fixed {
        role: VersionRef,
    },
    Late {
        level: WorkflowLevel,
        capability_ceiling: Vec<String>,
        contract_profile: VersionRef,
    },
}

#[derive(Deserialize)]
struct Definition {
    schema_version: String,
    bindings: BTreeMap<String, Slot>,
    body: Value,
}

#[derive(Deserialize)]
struct Role {
    schema_version: String,
    #[serde(rename = "role_id")]
    id: String,
    version: u32,
    scope: RegistryScope,
    entry_skill: VersionRef,
    skills: Vec<VersionRef>,
    contracts: Vec<VersionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegistryVersion {
    #[serde(rename = "registry_version_id")]
    id: String,
    content_hash: Sha256Digest,
    scope: RegistryScope,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum RunBinding {
    Bound {
        role: RegistryVersion,
        entry_skill: RegistryVersion,
        skills: Vec<RegistryVersion>,
        contracts: Vec<RegistryVersion>,
    },
    Unbound {
        level: WorkflowLevel,
        capability_ceiling: Vec<String>,
        contract_profile: VersionRef,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunPlan {
    schema_version: u32,
    pub workflow_version_id: String,
    pub definition_hash: Sha256Digest,
    pub mission_id: MissionId,
    pub mission_revision: Revision,
    pub scope_manifest_hash: Sha256Digest,
    scopes: BTreeMap<String, ExecutionScope>,
    bindings: BTreeMap<String, RunBinding>,
}

pub(crate) struct PlanSource<'a> {
    pub project_id: ProjectId,
    pub mission_id: MissionId,
    pub mission_revision: Revision,
    pub workflow_version_id: &'a str,
    pub scope_manifest_hash: Sha256Digest,
}

pub(crate) fn resolve(
    conn: &Connection,
    config: &StoreConfig,
    source: &PlanSource<'_>,
) -> Result<RunPlan, StoreError> {
    let definition_hash = workflow_hash(conn, source.workflow_version_id)?;
    let scope_id = source.mission_id.to_string();
    require_reference(
        conn,
        "mission",
        &scope_id,
        &scope_field(source.mission_revision),
        source.scope_manifest_hash,
    )?;
    let scopes: MissionScopes = read_document(conn, config, source.scope_manifest_hash)?;
    if scopes.schema_version != 1
        || scopes.mission_id != source.mission_id
        || scopes.mission_revision != source.mission_revision
        || scopes
            .scopes
            .iter()
            .any(|(key, scope)| key.trim().is_empty() || scope.scope_id.trim().is_empty())
    {
        return Err(invalid(
            "mission scope manifest does not match the exact mandate",
        ));
    }
    let workflow: Definition = read_document(conn, config, definition_hash)?;
    if !matches!(
        workflow.schema_version.as_str(),
        "unleash.workflow.v1" | "unleash.workflow.v1.1" | "unleash.workflow.v1.2"
    ) {
        return Err(invalid("unsupported published workflow definition"));
    }
    verify_task_coordinates(&workflow, &scopes)?;
    let mut bindings = BTreeMap::new();
    for (id, slot) in workflow.bindings {
        let binding = match slot {
            Slot::Fixed { role } => resolve_role(conn, config, source.project_id, &role)?,
            Slot::Late {
                level,
                capability_ceiling,
                contract_profile,
            } => RunBinding::Unbound {
                level,
                capability_ceiling,
                contract_profile,
            },
        };
        bindings.insert(id, binding);
    }
    Ok(RunPlan {
        schema_version: 1,
        workflow_version_id: source.workflow_version_id.to_owned(),
        definition_hash,
        mission_id: source.mission_id,
        mission_revision: source.mission_revision,
        scope_manifest_hash: source.scope_manifest_hash,
        scopes: scopes.scopes,
        bindings,
    })
}

fn verify_task_coordinates(
    workflow: &Definition,
    scopes: &MissionScopes,
) -> Result<(), StoreError> {
    let mut pending = vec![&workflow.body];
    while let Some(node) = pending.pop() {
        if node.get("kind").and_then(Value::as_str) == Some("task") {
            let bound = node
                .get("binding")
                .and_then(Value::as_str)
                .is_some_and(|slot| workflow.bindings.contains_key(slot));
            let has_scope = node
                .get("scope_key")
                .and_then(Value::as_str)
                .is_some_and(|key| scopes.scopes.contains_key(key));
            if !bound || !has_scope {
                return Err(invalid(
                    "a workflow task references an undeclared binding or scope",
                ));
            }
        }
        push_workflow_children(&mut pending, node);
    }
    Ok(())
}

fn resolve_role(
    conn: &Connection,
    config: &StoreConfig,
    project: ProjectId,
    requested: &VersionRef,
) -> Result<RunBinding, StoreError> {
    let role = registry_version(conn, project, "role", requested)?;
    let document: Role = read_document(conn, config, role.content_hash)?;
    if document.schema_version != "unleash.role.v1"
        || document.id != requested.id
        || document.version != requested.version
        || document.scope.kind != role.scope.kind
        || document.scope.owner() != role.scope.owner()
        || document.contracts.is_empty()
        || !document.skills.contains(&document.entry_skill)
    {
        return Err(invalid(
            "published role contents do not match its registry coordinates",
        ));
    }
    let skills = resolve_versions(conn, project, "skill", &document.skills)?;
    let entry_skill = registry_version(conn, project, "skill", &document.entry_skill)?;
    let contracts = resolve_versions(conn, project, "contract", &document.contracts)?;
    for version in skills.iter().chain(&contracts) {
        read_verified_object_bytes_conn(conn, config, version.content_hash)?;
    }
    Ok(RunBinding::Bound {
        role,
        entry_skill,
        skills,
        contracts,
    })
}

fn resolve_versions(
    conn: &Connection,
    project: ProjectId,
    kind: &str,
    references: &[VersionRef],
) -> Result<Vec<RegistryVersion>, StoreError> {
    references
        .iter()
        .map(|reference| registry_version(conn, project, kind, reference))
        .collect()
}

fn registry_version(
    conn: &Connection,
    project: ProjectId,
    kind: &str,
    reference: &VersionRef,
) -> Result<RegistryVersion, StoreError> {
    let requested_kind = reference.scope.as_ref().map(|scope| scope.kind.as_str());
    let requested_owner = reference.scope.as_ref().and_then(RegistryScope::owner);
    if reference.id.trim().is_empty()
        || reference.version == 0
        || (requested_kind.is_some() && requested_owner.is_none())
    {
        return Err(invalid("invalid registry version reference"));
    }
    let mut stmt = conn.prepare(
        "SELECT v.registry_version_id,v.content_hash,v.scope_kind,v.scope_owner_key
           FROM registry_versions v
           JOIN object_records o ON o.object_hash=v.content_hash AND o.state='live'
           JOIN object_references rr ON rr.object_hash=o.object_hash
             AND rr.owner_kind='registry_version' AND rr.owner_id=v.registry_version_id
             AND rr.source_field='content_hash' AND rr.state='live'
          WHERE v.entry_kind=?1 AND v.entry_id=?2 AND v.version=?3
            AND v.publication_state='published'
            AND ((v.scope_kind='global' AND v.scope_owner_key='global')
                OR (v.scope_kind='project' AND v.scope_owner_key=?4))
            AND (?5 IS NULL OR (v.scope_kind=?5 AND v.scope_owner_key=?6))
          LIMIT 2",
    )?;
    let rows = stmt
        .query_map(
            params![
                kind,
                reference.id,
                reference.version,
                project.to_string(),
                requested_kind,
                requested_owner
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let [(id, hash, scope_kind, scope_owner)] = rows.as_slice() else {
        return Err(StoreError::EvidenceUnavailable(format!(
            "no unique published {kind} version for {}",
            reference.id
        )));
    };
    Ok(RegistryVersion {
        id: id.clone(),
        content_hash: parse_hash(hash)?,
        scope: RegistryScope {
            kind: scope_kind.clone(),
            owner_id: (scope_kind != "global").then(|| scope_owner.clone()),
        },
    })
}

fn workflow_hash(conn: &Connection, version: &str) -> Result<Sha256Digest, StoreError> {
    let hash: String = conn.query_row(
        "SELECT content_hash FROM workflow_versions WHERE workflow_version_id=?1 AND visibility='published'",
        [version], |row| row.get(0),
    ).optional()?.ok_or_else(|| StoreError::EvidenceUnavailable("published workflow version unavailable".to_owned()))?;
    let hash = parse_hash(&hash)?;
    require_reference(conn, "workflow_version", version, "content_hash", hash)?;
    Ok(hash)
}

fn read_document<T: DeserializeOwned>(
    conn: &Connection,
    config: &StoreConfig,
    hash: Sha256Digest,
) -> Result<T, StoreError> {
    let bytes = read_verified_object_bytes_conn(conn, config, hash)?;
    let json =
        std::str::from_utf8(&bytes).map_err(|_| invalid("published document must be UTF-8"))?;
    Ok(from_json_str_strict(json)?)
}

pub(crate) fn require_run_manifest(
    conn: &Connection,
    run: &str,
    hash: Sha256Digest,
) -> Result<(), StoreError> {
    require_reference(conn, "workflow_run", run, "effective_bundle_hash", hash)
}

pub(crate) fn verify_activation(
    conn: &Connection,
    config: &StoreConfig,
    mission: &str,
    version: &str,
    activation: &crate::WorkflowTaskActivationPlan,
    task: &Value,
) -> Result<(), StoreError> {
    let plan: RunPlan = read_document(conn, config, activation.effective_bundle_hash)?;
    if plan.schema_version != 1
        || plan.mission_id.to_string() != mission
        || plan.workflow_version_id != version
        || plan.definition_hash != activation.definition_hash
    {
        return Err(StoreError::EvidenceUnavailable(
            "run manifest coordinates do not match".to_owned(),
        ));
    }
    let scope = task
        .get("scope_key")
        .and_then(Value::as_str)
        .and_then(|key| plan.scopes.get(key));
    let binding = task
        .get("binding")
        .and_then(Value::as_str)
        .and_then(|slot| plan.bindings.get(slot));
    if !scope.is_some_and(|scope| {
        scope.scope_id == activation.scope_id && scope.scope_revision == activation.scope_revision
    }) {
        return Err(invalid(
            "task activation does not match the scope pinned to its node",
        ));
    }
    let Some(RunBinding::Bound { contracts, .. }) = binding else {
        return Err(invalid("the workflow task slot is not bound"));
    };
    if !contracts
        .iter()
        .any(|version| version.id == activation.contract_version_id)
    {
        return Err(invalid(
            "task activation does not match a contract pinned to its slot",
        ));
    }
    Ok(())
}

fn parse_hash(value: &str) -> Result<Sha256Digest, StoreError> {
    value
        .parse()
        .map_err(|_| StoreError::InvariantViolation("invalid published content hash".to_owned()))
}

pub(crate) fn scope_field(revision: Revision) -> String {
    format!("scope_manifest:{}", revision.as_u64())
}

fn require_reference(
    conn: &Connection,
    kind: &str,
    owner: &str,
    field: &str,
    hash: Sha256Digest,
) -> Result<(), StoreError> {
    let exact: bool = conn.query_row(
        "SELECT COUNT(*)=1 AND MAX(r.object_hash)=?4
         FROM object_references r
         JOIN object_records o ON o.object_hash=r.object_hash AND o.state='live'
         WHERE r.owner_kind=?1 AND r.owner_id=?2 AND r.source_field=?3 AND r.state='live'",
        params![kind, owner, field, hash.to_string()],
        |row| row.get(0),
    )?;
    if exact {
        Ok(())
    } else {
        Err(StoreError::EvidenceUnavailable(format!(
            "missing or ambiguous {kind} publication"
        )))
    }
}

impl RunPlan {
    pub(crate) fn registry_versions(&self) -> BTreeMap<&str, Sha256Digest> {
        let mut versions = BTreeMap::new();
        for binding in self.bindings.values() {
            if let RunBinding::Bound {
                role,
                entry_skill,
                skills,
                contracts,
            } = binding
            {
                for version in [role, entry_skill]
                    .into_iter()
                    .chain(skills)
                    .chain(contracts)
                {
                    versions.insert(version.id.as_str(), version.content_hash);
                }
            }
        }
        versions
    }

    pub(crate) fn revalidate(&self, conn: &Connection) -> Result<(), StoreError> {
        if workflow_hash(conn, &self.workflow_version_id)? != self.definition_hash {
            return Err(StoreError::EvidenceUnavailable(
                "workflow definition changed during preparation".to_owned(),
            ));
        }
        require_reference(
            conn,
            "mission",
            &self.mission_id.to_string(),
            &scope_field(self.mission_revision),
            self.scope_manifest_hash,
        )?;
        for (version, hash) in self.registry_versions() {
            let published: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM registry_versions
                 WHERE registry_version_id=?1 AND content_hash=?2 AND publication_state='published')",
                params![version, hash.to_string()], |row| row.get(0),
            )?;
            if !published {
                return Err(StoreError::EvidenceUnavailable(
                    "selected registry version changed during preparation".to_owned(),
                ));
            }
            require_reference(conn, "registry_version", version, "content_hash", hash)?;
        }
        Ok(())
    }
}

pub(crate) fn invalid(message: &str) -> StoreError {
    StoreError::InvalidArgument {
        reason: None,
        message: message.to_owned(),
    }
}
