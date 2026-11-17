//! Test vérifiant que les types TypeScript dans `apps/gui-renderer/src/generated/`
//! sont strictement synchronisés et octet pour octet identiques à la génération Rust (`ts-rs`).

#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation
)]

use std::fs;
use std::path::Path;
use unleash_protocol::{generate_all_ts_files, write_ts_bindings};

#[test]
fn test_typescript_bindings_in_sync_with_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generated_dir = root.join("apps/gui-renderer/src/generated");

    // Génération dans un répertoire temporaire sous TMPDIR
    let temp_path = std::env::temp_dir().join(format!("unleash-ts-test-{}", std::process::id()));
    write_ts_bindings(&temp_path).expect("Écriture des bindings TS dans le dossier temporaire");

    // Si demandé explicitement ou si le dossier cible n'existe pas encore dans apps/,
    // on synchronise apps/gui-renderer/src/generated
    if std::env::var("UPDATE_TS_BINDINGS").is_ok() || !generated_dir.exists() {
        fs::create_dir_all(&generated_dir).expect("Création de apps/gui-renderer/src/generated");
        write_ts_bindings(&generated_dir).expect("Écriture des bindings TS dans apps/gui-renderer");
    }

    let expected_files = generate_all_ts_files();

    // Garde-fou : s'assurer que serde-compat et l'override de ProtocolVersion restent actifs
    assert!(
        expected_files["ProtocolVersion.ts"].contains("\"unleash.ipc.v2\""),
        "ProtocolVersion.ts doit exposer le littéral \"unleash.ipc.v2\""
    );
    assert!(
        expected_files["ReasonCode.ts"].contains("\"PUBLICATION_CONFLICT\""),
        "ReasonCode.ts doit respecter #[serde(rename = \"UPPER_SNAKE\")]"
    );
    assert!(
        expected_files["SubscribeEventsFrame.ts"].contains("\"kind\": \"events\""),
        "SubscribeEventsFrame.ts doit respecter #[serde(tag = \"kind\", rename_all = \"snake_case\")]"
    );

    // Vérification de chaque fichier généré
    for (filename, expected_content) in &expected_files {
        let repo_file = generated_dir.join(filename);
        assert!(
            repo_file.exists(),
            "Le fichier TypeScript commité {} est manquant dans {}",
            filename,
            generated_dir.display()
        );

        let repo_content = fs::read_to_string(&repo_file)
            .unwrap_or_else(|e| panic!("Impossible de lire {} : {e}", repo_file.display()));

        assert_eq!(
            repo_content, *expected_content,
            "Le fichier TypeScript commité {} diverge de la génération Rust ts-rs !",
            filename
        );

        let temp_content = fs::read_to_string(temp_path.join(filename))
            .expect("Lecture du fichier généré dans TMPDIR");

        assert_eq!(
            repo_content, temp_content,
            "Le fichier généré sous TMPDIR diffère octet par octet de {}",
            filename
        );
    }

    // Vérifier qu'aucun fichier orphelin n'existe dans generated_dir
    let repo_entries = fs::read_dir(&generated_dir).expect("Lecture du dossier apps/.../generated");
    for entry in repo_entries {
        let entry = entry.expect("Entrée de répertoire valide");
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            expected_files.contains_key(&name),
            "Fichier inattendu trouvé dans apps/gui-renderer/src/generated/ : {name}"
        );
    }

    let _ = fs::remove_dir_all(&temp_path);
}
