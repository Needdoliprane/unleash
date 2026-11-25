//! T-F006-010 / AC-F006-010 / REQ-COMP-010 : brouillon manuel au port du Store.

#![expect(
    clippy::unwrap_used,
    reason = "Fixture Store isolée et assertions du contrat de brouillon"
)]

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value, json};
use unleash_domain::{ActorId, CommandId, DraftId, ProjectId, RequestId, Revision, Sha256Digest};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, ProtocolVersion, ReceiptState, ReasonCode, WorkflowDraftProjection,
};
use unleash_store::{
    AggregateKind, BackupRetentionTier, CommandMutation, CommitCommandRequest, CommitOutcome,
    PrepareBackupRequest, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::local_test_dir;

/// Texte retenu tel quel : fins de ligne CRLF, caractère non ASCII et clé dupliquée.
const TEXT: &str = "{\r\n  \"nom\": \"été\",\r\n  \"nom\": \"deux\"\r\n}\r\n";
/// Empreinte indépendante des octets UTF-8 de `TEXT`.
const TEXT_HASH: &str = "sha256:14d14d88e93a53254026a0c0adfaf3db12ef7cb681b1b6c8711b4316755b21e6";
const OTHER_TEXT: &str = "{\"nom\": \"autre\"}";
const SECOND_TEXT: &str = "{\"nom\": \"second\"}";
const OWNER: &str = "uid:draft-fixture";
const PROJECT: ProjectId = ProjectId::from_u128(0x6610);
const OTHER_PROJECT: ProjectId = ProjectId::from_u128(0x6611);
const ABSENT_PROJECT: ProjectId = ProjectId::from_u128(0x6612);
const DRAFT: DraftId = DraftId::from_u128(0x6641);
const OTHER_DRAFT: DraftId = DraftId::from_u128(0x6642);

fn commit_envelope(
    store: &UnleashStore,
    command: u128,
    verb: &str,
    project: ProjectId,
    aggregate: uuid::Uuid,
    expected: Revision,
    payload: Map<String, Value>,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(command),
        command_id: CommandId::from_u128(command),
        authority_generation: store.active_authority_generation().unwrap(),
        actor_ref: None,
        project_id: Some(project),
        aggregate_id: aggregate,
        expected_revision: expected,
        command_type: verb.to_owned(),
        payload,
    }
}

fn create_project(store: &UnleashStore, project: ProjectId, command: u128) {
    let outcome = store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new(OWNER).unwrap(),
            envelope: commit_envelope(
                store,
                command,
                "CreateProject",
                project,
                project.as_uuid(),
                Revision::INITIAL,
                Map::new(),
            ),
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id: project,
                name: "draft fixture".to_owned(),
                weight: 1,
            },
            events: Vec::new(),
            capacity_reservations: Vec::new(),
            budget_reservations: Vec::new(),
        })
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
}

/// Soumet la création d'un brouillon de texte ; le texte voyage dans l'enveloppe, comme sur le
/// fil, donc il entre dans le digest de la commande.
fn retain_draft(
    store: &UnleashStore,
    draft: DraftId,
    project: ProjectId,
    expected: Revision,
    command: u128,
    text: &str,
) -> Result<CommitOutcome, StoreError> {
    let aggregate = draft.as_uuid();
    let payload: Map<String, Value> = serde_json::from_value(json!({
        "source": {"kind": "text", "text": text}
    }))
    .unwrap();
    store.commit_command(CommitCommandRequest {
        actor_id: ActorId::new(OWNER).unwrap(),
        envelope: commit_envelope(
            store,
            command,
            "CreateWorkflowDraft",
            project,
            aggregate,
            expected,
            payload,
        ),
        target_aggregate: AggregateKind::WorkflowDraft,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::CreateWorkflowDraft {
            draft_id: draft,
            project_id: project,
            text_utf8: text.as_bytes().to_vec(),
        },
        events: Vec::new(),
        capacity_reservations: Vec::new(),
        budget_reservations: Vec::new(),
    })
}

/// Ouvre un Store qui détient deux projets.
fn seeded(path: &Path) -> UnleashStore {
    let store = UnleashStore::open(StoreConfig::new(path)).unwrap();
    create_project(&store, PROJECT, 0x6620);
    create_project(&store, OTHER_PROJECT, 0x6621);
    store
}

fn retained() -> WorkflowDraftProjection {
    WorkflowDraftProjection {
        project_id: PROJECT,
        draft_id: DRAFT,
        revision: Revision::new(1),
        text: TEXT.to_owned(),
        text_hash: Sha256Digest::from_bytes(TEXT.as_bytes()),
    }
}

// T-F006-010 / AC-F006-010 / REQ-COMP-010
#[test]
fn t_f006_010_draft_keeps_its_exact_bytes_across_read_replay_and_restart() {
    let dir = local_test_dir("f006-draft-bytes").unwrap();
    let store = seeded(dir.path());
    let outcome = retain_draft(&store, DRAFT, PROJECT, Revision::INITIAL, 0x6622, TEXT).unwrap();
    let receipt = outcome.receipt().clone();
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    assert_eq!(receipt.state, ReceiptState::Completed);
    assert_eq!(receipt.new_revision, Revision::new(1));
    assert_eq!(receipt.aggregate_id, DRAFT.as_uuid());

    let read = store.get_workflow_draft_snapshot(DRAFT, PROJECT, None).unwrap();
    assert_eq!(read.value, retained());
    assert_eq!(read.value.text.as_bytes(), TEXT.as_bytes());
    assert_eq!(read.value.text_hash.to_string(), TEXT_HASH);
    assert!(read.cursor.sequence >= receipt.event_cursor.sequence);

    // Le rejeu exact rend le reçu engagé, sans seconde création.
    let replay = retain_draft(&store, DRAFT, PROJECT, Revision::INITIAL, 0x6622, TEXT).unwrap();
    assert_eq!(replay, CommitOutcome::IdempotentReplay(receipt));

    // La même clé avec une autre intention est un conflit : le texte engagé ne change pas.
    let conflict = retain_draft(&store, DRAFT, PROJECT, Revision::INITIAL, 0x6622, OTHER_TEXT);
    assert_eq!(conflict.unwrap_err().error_code(), ErrorCode::IdempotencyConflict);
    assert_eq!(store.get_workflow_draft_snapshot(DRAFT, PROJECT, None).unwrap().value, retained());

    // Un redémarrage dans la même lignée relit les mêmes octets et la même empreinte.
    let store = store.restart_same_lineage().unwrap();
    assert_eq!(store.get_workflow_draft_snapshot(DRAFT, PROJECT, None).unwrap().value, retained());
    store.shutdown().unwrap();
}

// T-F006-010 / AC-F006-010 / REQ-COMP-010
#[test]
fn t_f006_010_draft_is_hidden_outside_its_project_and_refuses_bad_creations() {
    let dir = local_test_dir("f006-draft-scope").unwrap();
    let store = seeded(dir.path());
    retain_draft(&store, DRAFT, PROJECT, Revision::INITIAL, 0x6630, TEXT).unwrap();

    // L'identité d'un brouillon n'ouvre aucun autre projet, même autorisé.
    let other = store
        .get_workflow_draft_snapshot(DRAFT, OTHER_PROJECT, None)
        .unwrap_err();
    assert_eq!(other.error_code(), ErrorCode::NotFoundOrHidden);
    let allowed = HashSet::from([OTHER_PROJECT]);
    let denied = store
        .get_workflow_draft_snapshot(DRAFT, PROJECT, Some(&allowed))
        .unwrap_err();
    assert_eq!(denied.error_code(), ErrorCode::ScopeDenied);
    assert_eq!(denied.reason_code(), Some(ReasonCode::AccessDenied));

    // Une création annonce la révision zéro ; une identité retenue ne se crée pas deux fois.
    let stale = retain_draft(&store, DRAFT, PROJECT, Revision::new(1), 0x6631, TEXT);
    assert_eq!(stale.unwrap_err().error_code(), ErrorCode::RevisionConflict);
    let twice = retain_draft(&store, DRAFT, PROJECT, Revision::INITIAL, 0x6632, SECOND_TEXT);
    assert_eq!(twice.unwrap_err().error_code(), ErrorCode::RevisionConflict);

    // Un projet absent ne retient rien.
    let orphan = retain_draft(&store, OTHER_DRAFT, ABSENT_PROJECT, Revision::INITIAL, 0x6633, TEXT);
    assert_eq!(orphan.unwrap_err().error_code(), ErrorCode::NotFoundOrHidden);

    // Rien des tentatives refusées n'est lisible ; le brouillon engagé est inchangé.
    let absent = store
        .get_workflow_draft_snapshot(OTHER_DRAFT, ABSENT_PROJECT, None)
        .unwrap_err();
    assert_eq!(absent.error_code(), ErrorCode::NotFoundOrHidden);
    assert_eq!(store.get_workflow_draft_snapshot(DRAFT, PROJECT, None).unwrap().value, retained());
    store.shutdown().unwrap();
}

// T-F006-010 / AC-F006-010 / REQ-COMP-010 : la montée de version sert les brouillons d'une base existante.
#[test]
fn t_f006_010_migration_v021_to_v022_keeps_the_base_and_serves_the_drafts() {
    let dir = local_test_dir("f006-draft-v021").unwrap();
    let mut previous = StoreConfig::new(dir.path());
    previous.max_supported_schema_version = 21;
    let store = UnleashStore::open(previous).unwrap();
    create_project(&store, PROJECT, 0x6640);
    let project_before = store.get_project_snapshot(PROJECT).unwrap().value;
    let backup = store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: dir.path().join("v021-backup"),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();
    store.shutdown().unwrap();

    let mut next = StoreConfig::new(dir.path());
    next.max_supported_schema_version = 22;
    assert!(matches!(
        UnleashStore::open(next.clone()),
        Err(StoreError::MigrationBackupRequired)
    ));
    let upgraded = UnleashStore::open_with_migration_backup(next, Some(backup.backup_id));
    let upgraded = upgraded.unwrap();
    assert_eq!(upgraded.daemon_state().unwrap().schema_version, 22);
    assert_eq!(upgraded.get_project_snapshot(PROJECT).unwrap().value, project_before);
    // La table est servie : un brouillon absent est masqué, ce n'est pas une panne SQL.
    let absent = upgraded.get_workflow_draft_snapshot(DRAFT, PROJECT, None);
    assert_eq!(absent.unwrap_err().error_code(), ErrorCode::NotFoundOrHidden);
    retain_draft(&upgraded, DRAFT, PROJECT, Revision::INITIAL, 0x6641, TEXT).unwrap();
    assert_eq!(
        upgraded.get_workflow_draft_snapshot(DRAFT, PROJECT, None).unwrap().value,
        retained()
    );
    upgraded.shutdown().unwrap();
}
