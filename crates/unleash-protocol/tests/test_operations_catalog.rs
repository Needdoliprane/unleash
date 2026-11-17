//! Test d'intégration vérifiant la bijection exacte 1-à-1 entre `OperationName`
//! et les 204 opérations du catalogue de conception (`operations.json` et `operation-name.schema.json`).

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use unleash_protocol::{OperationKind, OperationName};

#[test]
fn test_operations_bijection_with_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog_path = root.join(".claude/catalog/operations.json");
    let schema_path = root.join(".claude/schemas/operation-name.schema.json");

    let catalog_content = fs::read_to_string(&catalog_path)
        .unwrap_or_else(|e| panic!("Impossible de lire {}: {e}", catalog_path.display()));
    let schema_content = fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("Impossible de lire {}: {e}", schema_path.display()));

    let catalog_json: Value =
        serde_json::from_str(&catalog_content).expect("JSON invalide pour operations.json");
    let schema_json: Value = serde_json::from_str(&schema_content)
        .expect("JSON invalide pour operation-name.schema.json");

    let ops_array = catalog_json["operations"]
        .as_array()
        .expect("operations.json doit contenir une liste 'operations'");
    assert_eq!(
        ops_array.len(),
        204,
        "Le catalogue doit contenir exactement 204 opérations"
    );

    let schema_enum = schema_json["enum"]
        .as_array()
        .expect("operation-name.schema.json doit contenir un tableau 'enum'");
    assert_eq!(
        schema_enum.len(),
        204,
        "Le schéma d'énumération doit contenir exactement 204 opérations"
    );

    let mut schema_names = BTreeSet::new();
    for v in schema_enum {
        let name = v
            .as_str()
            .expect("Nom d'opération non-chaîne dans le schéma");
        schema_names.insert(name.to_owned());
    }

    let mut catalog_ops = BTreeMap::new();
    for op in ops_array {
        let name = op["name"]
            .as_str()
            .expect("Champ 'name' manquant")
            .to_owned();
        let kind_str = op["kind"].as_str().expect("Champ 'kind' manquant");
        let parity_groups: Vec<String> = op["parity_group_ids"]
            .as_array()
            .expect("Champ 'parity_group_ids' manquant")
            .iter()
            .map(|pg| pg.as_str().expect("Group ID non-chaîne").to_owned())
            .collect();
        catalog_ops.insert(name, (kind_str.to_owned(), parity_groups));
    }

    // 1. Bijection operations.json <-> operation-name.schema.json
    assert_eq!(catalog_ops.len(), 204);
    assert_eq!(schema_names.len(), 204);
    for name in catalog_ops.keys() {
        assert!(
            schema_names.contains(name),
            "L'opération '{name}' de operations.json est absente de operation-name.schema.json"
        );
    }

    // 2. Vérification contre l'enum Rust `OperationName`
    let all_rust_ops = OperationName::all_operations();
    assert_eq!(
        all_rust_ops.len(),
        204,
        "L'enum Rust doit contenir exactement 204 variantes"
    );

    let mut rust_names = BTreeSet::new();
    for op in all_rust_ops {
        let name = op.as_str();
        rust_names.insert(name.to_owned());

        let (expected_kind, expected_pgs) = catalog_ops
            .get(name)
            .unwrap_or_else(|| panic!("L'opération Rust '{name}' n'est pas dans operations.json"));

        let rust_kind = match op.kind() {
            OperationKind::Command => "command",
            OperationKind::Query => "query",
            OperationKind::Stream => "stream",
            OperationKind::Handshake => "handshake",
            other => panic!("OperationKind non reconnu : {other:?}"),
        };
        assert_eq!(
            rust_kind, expected_kind,
            "Kind mismatch pour l'opération '{name}' : attendu {expected_kind}, obtenu {rust_kind}"
        );

        let rust_pgs = op.parity_group_ids();
        let expected_pgs_str: Vec<&str> = expected_pgs.iter().map(String::as_str).collect();
        assert_eq!(
            rust_pgs,
            expected_pgs_str.as_slice(),
            "Parity group IDs mismatch pour l'opération '{name}'"
        );

        // Test d'exposition GUI
        let expected_gui_hidden = matches!(
            name,
            "Hello" | "RecordClaim" | "RecordObservation" | "RecordJudgment"
        );
        assert_eq!(
            op.is_exposed_to_gui(),
            !expected_gui_hidden,
            "Exposition GUI incorrecte pour '{name}'"
        );

        // Test de sérialisation / désérialisation
        let parsed: OperationName = name.parse().expect("FromStr doit réussir");
        assert_eq!(&parsed, op);
        let serialized = serde_json::to_string(op).expect("Sérialisation JSON doit réussir");
        assert_eq!(serialized, format!("\"{name}\""));
        let deserialized: OperationName =
            serde_json::from_str(&serialized).expect("Désérialisation JSON doit réussir");
        assert_eq!(&deserialized, op);
    }

    assert_eq!(rust_names.len(), 204);
}
