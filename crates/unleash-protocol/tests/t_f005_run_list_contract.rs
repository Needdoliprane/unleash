//! T-F005-001 / AC-F005-001 / REQ-WF-001 : forme wire de `ListRuns`.
//!
//! Le payload fermé, la page de runs et les fixtures du dossier sont confrontés aux schémas
//! `ipc-query.schema.json` et `ipc-query-response.schema.json`.

#![expect(
    clippy::unwrap_used,
    reason = "Fixtures du dossier et assertions du contrat wire"
)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use unleash_domain::{
    MissionId, ProjectId, RequestId, Revision, RunId, RunState, Sequence, StreamId,
};
use unleash_protocol::{
    EventCursor, ListRunsPayload, QueryEnvelope, QueryResponse, RunCoverageStatus, RunListEntry,
    RunPage, RunPageCoverage, RunPurpose, from_json_str_strict,
};

const LIST_RUNS_PROJECT: &str = "00000000-0000-4000-8000-000000000010";
const QUERY_FIXTURE: &str = ".claude/examples/ipc-list-runs-query.json";
const RESPONSE_FIXTURE: &str = ".claude/examples/ipc-list-runs-response.json";

fn dossier(relative: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(relative)).unwrap()
}

fn schema(relative: &str) -> Value {
    serde_json::from_str(&dossier(relative)).unwrap()
}

/// Validateur du schéma de requête, qui ne référence que lui-même.
fn query_validator() -> jsonschema::Validator {
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema(".claude/schemas/ipc-query.schema.json"))
        .unwrap()
}

/// Validateur d'une définition de la réponse, les deux schémas étant enregistrés sous leur `$id`.
fn definition_validator(name: &str) -> jsonschema::Validator {
    let query = schema(".claude/schemas/ipc-query.schema.json");
    let response = schema(".claude/schemas/ipc-query-response.schema.json");
    let query_id = query
        .get("$id")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let response_id = response
        .get("$id")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    let registry = referencing::Registry::new()
        .add(query_id, query)
        .unwrap()
        .add(response_id.as_str(), response)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .with_registry(&registry)
        .build(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": format!("{response_id}#/$defs/{name}"),
        }))
        .unwrap()
}

/// Enveloppe `ListRuns` du projet et du payload donnés.
fn list_runs(project: Value, payload: Value) -> Value {
    json!({
        "protocol_version": "unleash.ipc.v2",
        "request_id": "00000000-0000-4000-8000-000000000001",
        "query_type": "ListRuns",
        "project_id": project,
        "payload": payload,
    })
}

/// Remplace la valeur désignée par un pointeur JSON du document.
fn replace(document: &mut Value, pointer: &str, value: Value) {
    *document.pointer_mut(pointer).unwrap() = value;
}

/// Page de runs de l'exemple de réponse du dossier.
fn example_page() -> RunPage {
    RunPage {
        project_id: ProjectId::from_u128(0x10),
        items: vec![RunListEntry {
            run_id: RunId::from_u128(0x21),
            mission_id: MissionId::from_u128(0x31),
            workflow_version_id: "demo.reconcile.csv:1".to_owned(),
            state: RunState::Prepared,
            revision: Revision::new(1),
            purpose: RunPurpose::Operational,
        }],
        next_cursor: Some("fixture-next-page".to_owned()),
        coverage: RunPageCoverage {
            status: RunCoverageStatus::Complete,
        },
    }
}

// T-F005-001 / AC-F005-001 / REQ-WF-001 : payload fermé et borné.
#[test]
fn t_f005_001_list_runs_payload_is_closed_and_bounded() {
    let query: QueryEnvelope = from_json_str_strict(&dossier(QUERY_FIXTURE)).unwrap();
    let decoded: ListRunsPayload = serde_json::from_value(Value::Object(query.payload.clone())).unwrap();
    assert_eq!((decoded.page_size(), decoded.opaque_cursor()), (1, None));
    let wire = query_validator();
    assert!(wire.is_valid(&serde_json::to_value(&query).unwrap()));

    for (payload, page_size, cursor) in [
        (json!({}), 50, None),
        (json!({"opaque_cursor": null}), 50, None),
        (json!({"page_size": 100}), 100, None),
        (
            json!({"page_size": 1, "opaque_cursor": "fixture-next-page"}),
            1,
            Some("fixture-next-page")
        ),
        (json!({"page_size": 1, "opaque_cursor": "AZaz09-_"}), 1, Some("AZaz09-_")),
    ] {
        let decoded: ListRunsPayload = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!((decoded.page_size(), decoded.opaque_cursor()), (page_size, cursor));
        assert!(wire.is_valid(&list_runs(json!(LIST_RUNS_PROJECT), payload)));
        let encoded = serde_json::to_value(&decoded).unwrap();
        let again: ListRunsPayload = serde_json::from_value(encoded).unwrap();
        assert_eq!(again, decoded);
    }

    for (payload, refusal) in [
        (json!({"page_size": 0}), "from 1 to 100"),
        (json!({"page_size": 101}), "from 1 to 100"),
        (json!({"opaque_cursor": ""}), "dashes or underscores"),
        (json!({"opaque_cursor": "a.b"}), "dashes or underscores"),
        (json!({"opaque_cursor": "été"}), "dashes or underscores"),
        (json!({"opaque_cursor": "a".repeat(513)}), "dashes or underscores"),
    ] {
        let refused = serde_json::from_value::<ListRunsPayload>(payload.clone()).unwrap_err();
        assert!(refused.to_string().contains(refusal), "{refused}");
        assert!(!wire.is_valid(&list_runs(json!(LIST_RUNS_PROJECT), payload)));
    }
    for payload in [
        json!({"page_size": null}),
        json!({"page_size": -1}),
        json!({"page_size": "1"}),
        json!({"opaque_cursor": 7}),
        json!({"unknown": 1}),
    ] {
        assert!(serde_json::from_value::<ListRunsPayload>(payload.clone()).is_err());
        assert!(!wire.is_valid(&list_runs(json!(LIST_RUNS_PROJECT), payload)));
    }
    // Le projet de la liste est obligatoire et non nul.
    for project in [Value::Null, json!("00000000-0000-0000-0000-000000000000")] {
        assert!(!wire.is_valid(&list_runs(project, json!({"page_size": 1}))));
    }
}

// T-F005-001 / AC-F005-001 / REQ-WF-001 : la page garde la version engagée de chaque run.
#[test]
fn t_f005_001_list_runs_page_matches_the_wire_contract() {
    let fixture: Value = serde_json::from_str(&dossier(RESPONSE_FIXTURE)).unwrap();
    let validator = definition_validator("workflow_run_list_response");
    assert!(validator.is_valid(&fixture));
    let response: QueryResponse = from_json_str_strict(&dossier(RESPONSE_FIXTURE)).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), fixture);

    let page = example_page();
    assert_eq!(serde_json::to_value(&page).unwrap(), *fixture.get("result").unwrap());
    let built = serde_json::to_value(QueryResponse {
        request_id: RequestId::from_u128(1),
        watermark: Some(EventCursor::new(StreamId::from_u128(6), Sequence::new(42))),
        result: serde_json::to_value(&page).unwrap(),
    })
    .unwrap();
    assert_eq!(built, fixture);

    // Le filigrane de la lecture fait partie de la réponse.
    let mut unmarked = built.clone();
    unmarked.as_object_mut().unwrap().remove("watermark");
    assert!(!validator.is_valid(&unmarked));

    // Une page vide termine la liste ; un curseur garde la forme fermée du contrat.
    let mut trailing = built.clone();
    replace(&mut trailing, "/result/items", json!([]));
    assert!(!validator.is_valid(&trailing));
    for cursor in [json!("a.b"), json!(""), json!("a".repeat(513))] {
        let mut deviant = built.clone();
        replace(&mut deviant, "/result/next_cursor", cursor);
        assert!(!validator.is_valid(&deviant));
    }
    // Les états, la finalité, la couverture et les champs sont fermés, pour le schéma et le DTO.
    for (pointer, value) in [
        ("/result/coverage/status", json!("partial")),
        ("/result/items/0/state", json!("unknown")),
        ("/result/items/0/purpose", json!("test")),
        ("/result/items/0/revision", json!(-1)),
    ] {
        let mut deviant = built.clone();
        replace(&mut deviant, pointer, value);
        assert!(!validator.is_valid(&deviant), "champ accepté : {pointer}");
        let page = deviant.get("result").unwrap().clone();
        assert!(
            serde_json::from_value::<RunPage>(page).is_err(),
            "DTO accepté : {pointer}"
        );
    }
    let mut counted = built.clone();
    counted
        .get_mut("result")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("total".to_owned(), json!(1));
    assert!(!validator.is_valid(&counted));
    assert!(serde_json::from_value::<RunPage>(counted.get("result").unwrap().clone()).is_err());

    // La fin de liste est une page vide sans curseur, et elle fait l'aller-retour.
    let ended = RunPage {
        items: Vec::new(),
        next_cursor: None,
        ..example_page()
    };
    let mut expected = built.clone();
    replace(&mut expected, "/result/items", json!([]));
    replace(&mut expected, "/result/next_cursor", Value::Null);
    let ended = serde_json::to_value(QueryResponse {
        request_id: RequestId::from_u128(1),
        watermark: Some(EventCursor::new(StreamId::from_u128(6), Sequence::new(42))),
        result: serde_json::to_value(&ended).unwrap(),
    })
    .unwrap();
    assert_eq!(ended, expected);
    assert!(validator.is_valid(&ended));
}
