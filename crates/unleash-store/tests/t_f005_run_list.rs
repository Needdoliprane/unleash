//! T-F005-001 / AC-F005-001 / REQ-WF-001 : pages de runs au port du Store.

#![expect(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    reason = "Fixture Store isolée et assertions du contrat de liste"
)]

use std::collections::HashSet;
use std::path::Path;

use serde_json::json;
use unleash_domain::{AuthorityGeneration, MissionId, ProjectId, Revision, RunId, Sha256Digest};
use unleash_protocol::{
    ErrorCode, ListRunsPayload, ReasonCode, RetryClass, RunCoverageStatus, RunListEntry, RunPage,
    RunPageCoverage, RunPurpose, RunState,
};
use unleash_store::{StoreConfig, UnleashStore, WriteCriticality};
use unleash_testkit::local_test_dir;

const PINNED: &str = "fixture.run-list:pinned";
const PRINCIPAL: &str = "principal.run-list-fixture";
const PROJECT: ProjectId = ProjectId::from_u128(0x5001_0001);
const OTHER_PROJECT: ProjectId = ProjectId::from_u128(0x5001_0002);
const EMPTY_PROJECT: ProjectId = ProjectId::from_u128(0x5001_0003);
const ABSENT_PROJECT: ProjectId = ProjectId::from_u128(0x5001_0004);
const MISSION: MissionId = MissionId::from_u128(0x5001_0011);
const OTHER_MISSION: MissionId = MissionId::from_u128(0x5001_0012);
const FIRST: RunId = RunId::from_u128(0x5001_0021);
const SECOND: RunId = RunId::from_u128(0x5001_0022);
const THIRD: RunId = RunId::from_u128(0x5001_0023);
const OTHER_RUN: RunId = RunId::from_u128(0x5001_0024);

/// Ouvre un Store qui retient trois projets, deux missions et la version publiée de la fixture.
fn seeded(path: &Path) -> UnleashStore {
    let store = UnleashStore::open(StoreConfig::new(path)).unwrap();
    store
        .register_principal(PRINCIPAL, "user", "user:run-list-fixture")
        .unwrap();
    store
        .register_workflow_version(
            PINNED,
            "published",
            Sha256Digest::from_bytes(b"run list definition"),
        )
        .unwrap();
    let projects = [PROJECT, OTHER_PROJECT, EMPTY_PROJECT].map(|project| project.to_string());
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            for project in projects {
                tx.execute(
                    "INSERT INTO projects (project_id,name,created_at_ms)
                     VALUES (?1,'run list fixture',?2)",
                    rusqlite::params![project, now.as_i64()],
                )?;
            }
            for (mission, project) in [(MISSION, PROJECT), (OTHER_MISSION, OTHER_PROJECT)] {
                tx.execute(
                    "INSERT INTO missions (mission_id,project_id,owner_kind,owner_user_id,
                        completion_policy,state,revision)
                     VALUES (?1,?2,'user',?3,'mechanical','active',1)",
                    rusqlite::params![mission.to_string(), project.to_string(), PRINCIPAL],
                )?;
            }
            Ok(())
        })
        .unwrap();
    store
}

/// Retient un run préparé de la version épinglée sur la mission donnée.
fn retain_run(store: &UnleashStore, run: RunId, mission: MissionId, revision: i64) {
    let bundle = Sha256Digest::from_bytes(format!("bundle of run {run}").as_bytes()).to_string();
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
            tx.execute(
                "INSERT INTO workflow_runs (run_id,mission_id,workflow_version_id,selection_reason,
                    effective_bundle_hash,state,revision,started_at_ms)
                 VALUES (?1,?2,?3,'run list fixture',?4,'prepared',?5,?6)",
                rusqlite::params![
                    run.to_string(),
                    mission.to_string(),
                    PINNED,
                    bundle,
                    revision,
                    now.as_i64()
                ],
            )?;
            Ok(())
        })
        .unwrap();
}

/// Construit le payload fermé d'une page.
fn page(page_size: u16, opaque_cursor: Option<&str>) -> ListRunsPayload {
    serde_json::from_value(json!({"page_size": page_size, "opaque_cursor": opaque_cursor})).unwrap()
}

/// Entrée attendue d'un run préparé retenu par la fixture.
fn entry(run: RunId, mission: MissionId, revision: u64) -> RunListEntry {
    RunListEntry {
        run_id: run,
        mission_id: mission,
        workflow_version_id: PINNED.to_owned(),
        state: RunState::Prepared,
        revision: Revision::new(revision),
        purpose: RunPurpose::Operational,
    }
}

fn complete(project: ProjectId, items: Vec<RunListEntry>, next_cursor: Option<String>) -> RunPage {
    RunPage {
        project_id: project,
        items,
        next_cursor,
        coverage: RunPageCoverage {
            status: RunCoverageStatus::Complete,
        },
    }
}

// T-F005-001 / AC-F005-001 / REQ-WF-001
#[test]
fn t_f005_001_pages_follow_ascending_runs_and_keep_the_engaged_version() {
    let dir = local_test_dir("f005-run-list-pages").unwrap();
    let store = seeded(dir.path());
    // Runs retenus dans le désordre : les pages suivent l'identité du run, pas l'ordre d'écriture.
    retain_run(&store, THIRD, MISSION, 3);
    retain_run(&store, OTHER_RUN, OTHER_MISSION, 1);
    retain_run(&store, FIRST, MISSION, 1);
    retain_run(&store, SECOND, MISSION, 2);

    let first = store
        .list_runs_snapshot(&page(2, None), PROJECT, None)
        .unwrap();
    assert_eq!(first.value.project_id, PROJECT);
    assert_eq!(
        first.value.items,
        [entry(FIRST, MISSION, 1), entry(SECOND, MISSION, 2)]
    );
    assert_eq!(first.value.coverage.status, RunCoverageStatus::Complete);
    let cursor = first
        .value
        .next_cursor
        .clone()
        .filter(|cursor| !cursor.is_empty())
        .expect("une autre page suit");

    // Une publication postérieure entre deux pages ne change aucune version engagée.
    store
        .register_workflow_version(
            "fixture.run-list:later",
            "published",
            Sha256Digest::from_bytes(b"later definition"),
        )
        .unwrap();
    let before = store.daemon_state().unwrap().revision;
    let mark = first.cursor.sequence;

    let second = store
        .list_runs_snapshot(&page(2, Some(&cursor)), PROJECT, None)
        .unwrap();
    assert_eq!(
        second.value,
        complete(PROJECT, vec![entry(THIRD, MISSION, 3)], None)
    );
    assert_eq!(second.cursor.stream_id, first.cursor.stream_id);
    assert!(second.cursor.sequence >= mark);

    let other = store
        .list_runs_snapshot(&page(50, None), OTHER_PROJECT, None)
        .unwrap();
    assert_eq!(
        other.value,
        complete(
            OTHER_PROJECT,
            vec![entry(OTHER_RUN, OTHER_MISSION, 1)],
            None
        )
    );

    let empty = store
        .list_runs_snapshot(&page(50, None), EMPTY_PROJECT, None)
        .unwrap();
    assert_eq!(empty.value, complete(EMPTY_PROJECT, Vec::new(), None));
    assert_eq!(empty.cursor.sequence, mark);
    // La lecture n'engage rien : la révision de l'autorité et le filigrane sont inchangés.
    assert_eq!(store.daemon_state().unwrap().revision, before);
    store.shutdown().unwrap();
}

// T-F005-001 / AC-F005-001 / REQ-WF-001
#[test]
fn t_f005_001_refuses_foreign_malformed_and_expired_cursors() {
    let dir = local_test_dir("f005-run-list-cursors").unwrap();
    let store = seeded(dir.path());
    retain_run(&store, FIRST, MISSION, 1);
    retain_run(&store, SECOND, MISSION, 2);
    let cursor = store
        .list_runs_snapshot(&page(1, None), PROJECT, None)
        .unwrap()
        .value
        .next_cursor
        .unwrap();

    // Le curseur d'un projet ne reprend rien dans un autre projet.
    let foreign = store
        .list_runs_snapshot(&page(1, Some(&cursor)), OTHER_PROJECT, None)
        .unwrap_err();
    assert_eq!(foreign.error_code(), ErrorCode::InvalidArgument);

    // Seul le daemon lit son curseur : un jeton de même jeu de caractères est refusé, jamais repris.
    let other_operation = format!("{}02{}", &cursor[..2], &cursor[4..]);
    for token in ["a".repeat(99), "0".repeat(100), other_operation] {
        let refused = store
            .list_runs_snapshot(&page(1, Some(&token)), PROJECT, None)
            .unwrap_err();
        assert_eq!(refused.error_code(), ErrorCode::InvalidArgument);
    }

    // Un curseur ne survit pas à sa lignée : après une restauration, le client repart du début.
    store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, _now| {
            tx.execute(
                "UPDATE daemon_state SET authority_generation=?1 WHERE singleton=1",
                [AuthorityGeneration::from_u128(0x5001_00FF).to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    let expired = store
        .list_runs_snapshot(&page(1, Some(&cursor)), PROJECT, None)
        .unwrap_err();
    assert_eq!(expired.error_code(), ErrorCode::ResyncRequired);
    assert_eq!(expired.reason_code(), Some(ReasonCode::CursorExpired));
    assert_eq!(expired.retry_class(), RetryClass::AfterRefresh);
    // La première page reste lisible sous la lignée servie.
    let fresh = store
        .list_runs_snapshot(&page(1, None), PROJECT, None)
        .unwrap();
    assert_eq!(fresh.value.items, [entry(FIRST, MISSION, 1)]);
    store.shutdown().unwrap();
}

// T-F005-001 / AC-F005-001 / REQ-WF-001
#[test]
fn t_f005_001_denies_scope_before_existence_and_hides_an_absent_project() {
    let dir = local_test_dir("f005-run-list-scope").unwrap();
    let store = seeded(dir.path());
    retain_run(&store, FIRST, MISSION, 1);
    let allowed = HashSet::from([PROJECT]);

    // Le refus de portée ne dépend pas de l'existence du projet demandé.
    for requested in [OTHER_PROJECT, ABSENT_PROJECT] {
        let denied = store
            .list_runs_snapshot(&page(1, None), requested, Some(&allowed))
            .unwrap_err();
        assert_eq!(denied.error_code(), ErrorCode::ScopeDenied);
        assert_eq!(denied.reason_code(), Some(ReasonCode::AccessDenied));
    }
    assert!(
        store
            .list_runs_snapshot(&page(1, None), PROJECT, Some(&allowed))
            .is_ok()
    );

    // Un projet absent de la portée du demandeur est masqué, jamais listé comme vide.
    let granted = HashSet::from([ABSENT_PROJECT]);
    let scoped = store
        .list_runs_snapshot(&page(1, None), ABSENT_PROJECT, Some(&granted))
        .unwrap_err();
    let unrestricted = store
        .list_runs_snapshot(&page(1, None), ABSENT_PROJECT, None)
        .unwrap_err();
    for hidden in [scoped, unrestricted] {
        assert_eq!(hidden.error_code(), ErrorCode::NotFoundOrHidden);
    }
    store.shutdown().unwrap();
}
