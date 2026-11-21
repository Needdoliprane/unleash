//! Sources publiées communes aux bancs Store et IPC du run.

#[expect(
    dead_code,
    reason = "La fixture de workflow est partagée par plusieurs suites"
)]
#[path = "workflow_task.rs"]
mod workflow_task;

use serde_json::{Value, json};
use unleash_domain::{ActorId, Revision, RunId, Sha256Digest};
use unleash_protocol::{CommandEnvelope, canonicalize_to_vec};
use unleash_store::{CallerAuthority, CallerRole, StoreConfig, UnleashStore, WriteCriticality};

pub(crate) use workflow_task::Fixture;

pub(crate) const RUN: RunId = RunId::from_u128(0x5901);
pub(crate) const PUBLISHED: &str = "fixture.prepared-workflow:v1";

pub(crate) fn prepare(
    config: StoreConfig,
    actor: ActorId,
) -> (Fixture, CommandEnvelope, CallerAuthority) {
    let store = UnleashStore::open(config).unwrap();
    let principal = actor.to_string();
    let fixture = Fixture::on_store(store, &principal, actor.clone(), true);
    let mut command = fixture.envelope(21, RUN.as_uuid(), Revision::INITIAL, "CreateRun");
    let project = command.project_id.unwrap();
    let mut workflow: Value = serde_json::from_str(include_str!(
        "../../../../.claude/examples/merge-only-workflow.json"
    ))
    .unwrap();
    let task = workflow.pointer("/body/steps/0").unwrap().clone();
    let workflow_fields = workflow.as_object_mut().unwrap();
    workflow_fields.insert("bindings".to_owned(), json!({
        "developer": {"mode":"fixed","role":{"id":"role.fixture","version":1}},
        "future": {"mode":"late","level":"N","capability_ceiling":[],"contract_profile":{"id":"contract.fixture","version":1}}
    }));
    workflow_fields.insert("body".to_owned(), task);
    let definition = canonicalize_to_vec(&workflow).unwrap();
    let definition_hash = Sha256Digest::from_bytes(&definition);
    fixture.retain_object(&definition, "workflow_version", PUBLISHED, "content_hash");
    fixture
        .store
        .register_workflow_version(PUBLISHED, "published", definition_hash)
        .unwrap();

    let scope_document = json!({
        "schema_version": 1,
        "mission_id": fixture.mission,
        "mission_revision": 1,
        "scopes": {"implementation": {"scope_id":"fixture.restricted", "scope_revision":7}},
    });
    let scope_bytes = canonicalize_to_vec(&scope_document).unwrap();
    let scope_hash = Sha256Digest::from_bytes(&scope_bytes);
    fixture.retain_object(
        &scope_bytes,
        "mission",
        &fixture.mission.to_string(),
        "scope_manifest:1",
    );

    let role_scope = json!({"kind": "project", "owner_id": project});
    let skill_ref = json!({"id": "skill.fixture", "version": 1, "scope": role_scope});
    let contract_ref = json!({"id": "contract.fixture", "version": 1, "scope": role_scope});
    publish_fixture_version(
        &fixture,
        &principal,
        "role",
        "role.fixture",
        "role-v1",
        &json!({
            "schema_version": "unleash.role.v1", "role_id": "role.fixture", "version": 1,
            "scope": role_scope, "entry_skill": skill_ref, "skills": [skill_ref], "contracts": [contract_ref],
        }),
    );
    publish_fixture_version(
        &fixture,
        &principal,
        "skill",
        "skill.fixture",
        "skill-v1",
        &json!({
            "schema_version": "unleash.skill.v1", "skill_id": "skill.fixture", "version": 1,
            "scope": role_scope, "contract_profile": contract_ref,
        }),
    );
    publish_fixture_version(
        &fixture,
        &principal,
        "contract",
        "contract.fixture",
        "contract-v1",
        &json!({
            "schema_version": "unleash.contract-profile.v1", "profile_id": "contract.fixture",
            "version": 1, "scope": role_scope,
        }),
    );
    command.payload = serde_json::from_value(json!({
        "mission_id": fixture.mission,
        "mission_revision": 1,
        "workflow_version_id": PUBLISHED,
        "scope_manifest_hash": scope_hash,
        "predecessor_run_id": null,
        "selection_reason": "selected exact fixture version",
    }))
    .unwrap();
    let caller = CallerAuthority {
        actor_id: actor,
        role: CallerRole::HumanOperator,
    };
    (fixture, command, caller)
}

fn publish_fixture_version(
    fixture: &Fixture,
    principal: &str,
    kind: &str,
    entry: &str,
    version: &str,
    document: &Value,
) {
    let bytes = canonicalize_to_vec(document).unwrap();
    let digest = Sha256Digest::from_bytes(&bytes);
    fixture.retain_object(&bytes, "registry_version", version, "content_hash");
    let lineage = fixture.store.active_authority_generation().unwrap();
    let project = fixture
        .envelope(22, RUN.as_uuid(), Revision::INITIAL, "CreateRun")
        .project_id
        .unwrap();
    let (kind, entry, version) = (kind.to_owned(), entry.to_owned(), version.to_owned());
    let principal = principal.to_owned();
    fixture.store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, time| {
        let discovery = format!("{version}-discovery");
        let evaluation = format!("{version}-evaluation");
        tx.execute(
            "INSERT INTO discovery_receipts
              (discovery_receipt_id,authority_generation,decision,content_hash,created_at_ms)
             VALUES (?1,?2,'reuse',?3,?4)",
            rusqlite::params![discovery,lineage.to_string(),digest.to_string(),time.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO evaluation_reports
              (evaluation_report_id,subject_kind,subject_id,required_checks,judge_policy,result,
               content_hash,created_at_ms)
             VALUES (?1,?2,?3,'[]','fixture','passed',?4,?5)",
            rusqlite::params![evaluation,kind,version,digest.to_string(),time.as_i64()],
        )?;
        tx.execute(
            "INSERT INTO registry_entries (entry_kind,scope_kind,scope_owner_key,entry_id,state,revision)
             VALUES (?1,'project',?2,?3,'active',1)",
            rusqlite::params![kind,project.to_string(),entry],
        )?;
        tx.execute(
            "INSERT INTO registry_versions
              (registry_version_id,entry_kind,scope_kind,scope_owner_key,entry_id,version,
               schema_version,content_hash,publication_state,author_principal_id,
               discovery_receipt_id,evaluation_report_id,published_at_ms)
             VALUES (?1,?2,'project',?3,?4,1,1,?5,'published',?6,?7,?8,?9)",
            rusqlite::params![version,kind,project.to_string(),entry,digest.to_string(),principal,
                              discovery,evaluation,time.as_i64()],
        )?;
        Ok(())
    }).unwrap();
}
