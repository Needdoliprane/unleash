//! Tests du décodage JSON strict (rejet impératif des clés dupliquées à tout niveau, données traînantes et `deny_unknown_fields`).
// REQ-CORE-003 / REQ-CORE-004 / REQ-PERSIST-004

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use serde::{Deserialize, Serialize};
use unleash_protocol::{ProtocolError, from_json_str_strict, parse_json_value_strict};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleDto {
    name: String,
    count: u32,
    nested: Option<SampleNested>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleNested {
    flag: bool,
    items: Vec<String>,
}

#[test]
fn test_strict_json_rejects_top_level_duplicate_key() {
    let json_dup = r#"{"name": "test1", "count": 10, "name": "test2"}"#;
    let res = parse_json_value_strict(json_dup);
    match res {
        Err(ProtocolError::DuplicateKey { key }) => {
            assert_eq!(key, "name");
        }
        other => panic!("Attendu ProtocolError::DuplicateKey, obtenu : {other:?}"),
    }

    let dto_res: Result<SampleDto, _> = from_json_str_strict(json_dup);
    assert!(matches!(dto_res, Err(ProtocolError::DuplicateKey { .. })));
}

#[test]
fn test_strict_json_rejects_nested_duplicate_key() {
    let json_nested_dup = r#"{
        "name": "root",
        "count": 5,
        "nested": {
            "flag": true,
            "items": ["a", "b"],
            "flag": false
        }
    }"#;

    let res = parse_json_value_strict(json_nested_dup);
    match res {
        Err(ProtocolError::DuplicateKey { key }) => {
            assert_eq!(key, "flag");
        }
        other => panic!("Attendu ProtocolError::DuplicateKey('flag'), obtenu : {other:?}"),
    }

    let dto_res: Result<SampleDto, _> = from_json_str_strict(json_nested_dup);
    assert!(matches!(dto_res, Err(ProtocolError::DuplicateKey { .. })));
}

#[test]
fn test_strict_json_rejects_duplicate_key_inside_array() {
    let json_array_dup = r#"{
        "name": "root",
        "count": 1,
        "list": [
            {"id": 1, "sub": "ok"},
            {"id": 2, "sub": "first", "sub": "duplicate"}
        ]
    }"#;

    let res = parse_json_value_strict(json_array_dup);
    match res {
        Err(ProtocolError::DuplicateKey { key }) => {
            assert_eq!(key, "sub");
        }
        other => panic!("Attendu ProtocolError::DuplicateKey('sub'), obtenu : {other:?}"),
    }
}

#[test]
fn test_strict_json_accepts_valid_json() {
    let valid = r#"{
        "name": "valid",
        "count": 42,
        "nested": {
            "flag": true,
            "items": ["one", "two"]
        }
    }"#;

    let val = parse_json_value_strict(valid).expect("JSON valide sans doublons");
    assert_eq!(val["name"], "valid");
    assert_eq!(val["count"], 42);

    let dto: SampleDto = from_json_str_strict(valid).expect("Désérialisation valide");
    assert_eq!(dto.name, "valid");
    assert_eq!(dto.count, 42);
    assert_eq!(
        dto.nested,
        Some(SampleNested {
            flag: true,
            items: vec!["one".to_string(), "two".to_string()]
        })
    );
}

#[test]
fn test_deny_unknown_fields_rejects_unexpected_property() {
    let unexpected = r#"{
        "name": "test",
        "count": 10,
        "unknown_extra": "malicious or unexpected"
    }"#;

    let res: Result<SampleDto, _> = from_json_str_strict(unexpected);
    match res {
        Err(ProtocolError::Json(err)) => {
            let msg = err.to_string();
            assert!(
                msg.contains("unknown field"),
                "Le message Serde doit mentionner le champ inconnu : {msg}"
            );
        }
        other => panic!("Attendu ProtocolError::Json avec 'unknown field', obtenu : {other:?}"),
    }
}

#[test]
fn test_strict_json_rejects_trailing_data() {
    for trailing in [
        "{}{}",
        "{} garbage",
        r#"{"name": "ok", "count": 1} extra"#,
        "[1, 2] 3",
    ] {
        let val_res = parse_json_value_strict(trailing);
        assert!(
            matches!(val_res, Err(ProtocolError::Json(_))),
            "Données traînantes '{trailing}' doivent être rejetées par parse_json_value_strict, obtenu : {val_res:?}"
        );

        let dto_res: Result<SampleDto, _> = from_json_str_strict(trailing);
        assert!(
            matches!(dto_res, Err(ProtocolError::Json(_))),
            "Données traînantes '{trailing}' doivent être rejetées par from_json_str_strict, obtenu : {dto_res:?}"
        );
    }

    // Les espaces blancs finaux valides doivent rester acceptés
    let with_ws = "{\"name\": \"ws\", \"count\": 7}  \n\t ";
    let dto: SampleDto = from_json_str_strict(with_ws).expect("Espaces blancs finaux autorisés");
    assert_eq!(dto.name, "ws");
    assert_eq!(dto.count, 7);
}
