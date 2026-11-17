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

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use unleash_protocol::{generate_all_ts_files, write_ts_bindings};

/// Supprime commentaires (`//`, `/* ... */`) et littéraux chaînes (`"..."`, `'...'`)
/// tout en préservant des séparateurs d'espaces pour extraire les identifiants TS réels.
fn strip_ts_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(' ');
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
        } else if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else {
            let ch = src[i..].chars().next().expect("Index UTF-8 valide");
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

/// Extrait les jetons d'identifiants `[A-Za-z_][A-Za-z0-9_]*` d'un source déjà nettoyé.
fn extract_ts_identifiers(cleaned: &str) -> BTreeSet<String> {
    let mut idents = BTreeSet::new();
    let mut current = String::new();

    for ch in cleaned.chars() {
        if current.is_empty() {
            if ch.is_ascii_alphabetic() || ch == '_' {
                current.push(ch);
            }
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            idents.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        idents.insert(current);
    }

    idents
}

#[test]
fn test_typescript_bindings_in_sync_with_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generated_dir = root.join("apps/gui-renderer/src/generated");

    // Génération dans un répertoire temporaire sous TMPDIR
    let temp_path = std::env::temp_dir().join(format!("unleash-ts-test-{}", std::process::id()));
    write_ts_bindings(&temp_path).expect("Écriture des bindings TS dans le dossier temporaire");

    // Régénérer uniquement si UPDATE_TS_BINDINGS est explicitement positionné (non vide et != "0")
    let update_requested =
        std::env::var_os("UPDATE_TS_BINDINGS").is_some_and(|v| !v.is_empty() && v != "0");
    if update_requested {
        fs::create_dir_all(&generated_dir).expect("Création de apps/gui-renderer/src/generated");
        write_ts_bindings(&generated_dir).expect("Écriture des bindings TS dans apps/gui-renderer");
    }

    assert!(
        generated_dir.is_dir(),
        "Le dossier {} est absent ; exécuter avec UPDATE_TS_BINDINGS=1 pour le régénérer",
        generated_dir.display()
    );

    let expected_files = generate_all_ts_files();

    // Garde-fous : s'assurer que serde-compat, les types JS sûrs et le catalogue fermé restent conformes
    assert!(
        expected_files["ProtocolVersion.ts"].contains("\"unleash.ipc.v2\""),
        "ProtocolVersion.ts doit exposer le littéral \"unleash.ipc.v2\""
    );
    assert!(
        expected_files["ReasonCode.ts"].contains("\"PUBLICATION_CONFLICT\""),
        "ReasonCode.ts doit respecter #[serde(rename = \"UPPER_SNAKE\")]"
    );
    assert!(
        !expected_files["ReasonCode.ts"].contains("\"STREAM_RESYNC_REQUIRED\""),
        "ReasonCode.ts ne doit pas contenir STREAM_RESYNC_REQUIRED"
    );
    for required_reason in [
        "\"UNAUTHORIZED_ACTION\"",
        "\"ACCESS_DENIED\"",
        "\"PORT_IN_USE\"",
        "\"GUARD_DENIED\"",
        "\"AMBIGUOUS_REFERENCE\"",
    ] {
        assert!(
            expected_files["ReasonCode.ts"].contains(required_reason),
            "ReasonCode.ts doit contenir {required_reason}"
        );
    }
    assert!(
        expected_files["HelloResponse.ts"].contains("idempotency_retention_seconds: number,"),
        "HelloResponse.ts doit typer idempotency_retention_seconds en number"
    );
    assert!(
        !expected_files["HelloResponse.ts"].contains("bigint"),
        "HelloResponse.ts ne doit pas exposer bigint"
    );
    assert!(
        expected_files["ClientUploadSegment.ts"].contains("byte_offset: number,"),
        "ClientUploadSegment.ts doit typer byte_offset en number"
    );
    assert!(
        !expected_files["ClientUploadSegment.ts"].contains("bigint"),
        "ClientUploadSegment.ts ne doit pas exposer bigint"
    );
    assert!(
        expected_files["IpcError.ts"].contains("current_revision?: number,"),
        "IpcError.ts doit typer current_revision?: number sans | null"
    );
    assert!(
        !expected_files["IpcError.ts"].contains("current_revision?: number | null"),
        "IpcError.ts ne doit pas autoriser null sur current_revision"
    );
    assert!(
        expected_files["SubscribeEventsFrame.ts"].contains("\"kind\": \"events\""),
        "SubscribeEventsFrame.ts doit respecter #[serde(tag = \"kind\", rename_all = \"snake_case\")]"
    );
    assert!(
        expected_files["QueryResponse.ts"].contains("result: unknown,"),
        "QueryResponse.ts doit typer result en 'unknown' (pas 'any')"
    );

    let exported_types: BTreeSet<&str> = expected_files
        .keys()
        .filter(|name| name.as_str() != "index.ts")
        .map(|name| name.trim_end_matches(".ts"))
        .collect();

    for (filename, content) in &expected_files {
        if filename == "index.ts" {
            continue;
        }
        let type_name = filename.trim_end_matches(".ts");
        let decl_start = content.find("\nexport ").map_or_else(
            || panic!("Déclaration export manquante dans {filename}"),
            |idx| idx + 1,
        );
        let header = &content[..decl_start];
        let decl_body = &content[decl_start..];

        let mut imported = BTreeSet::new();
        for line in header.lines() {
            if line.starts_with("import ") {
                let rest = line.strip_prefix("import type { ").unwrap_or_else(|| {
                    panic!("Import inattendu (non 'import type') dans {filename}: {line}")
                });
                let (dep, path_part) = rest
                    .split_once(" } from \"./")
                    .unwrap_or_else(|| panic!("Format d'import invalide dans {filename}: {line}"));
                let path_dep = path_part.strip_suffix("\";").unwrap_or_else(|| {
                    panic!("Fin de ligne d'import invalide dans {filename}: {line}")
                });
                assert_eq!(
                    dep, path_dep,
                    "Cible d'import incohérente dans {filename}: {line}"
                );
                assert!(
                    exported_types.contains(dep),
                    "Import d'un type inconnu '{dep}' dans {filename}"
                );
                assert_ne!(dep, type_name, "Auto-import interdit dans {filename}");
                imported.insert(dep.to_owned());
            }
        }

        let cleaned = strip_ts_comments_and_strings(decl_body);
        let tokens = extract_ts_identifiers(&cleaned);
        assert!(
            !tokens.contains("any"),
            "Aucun type 'any' autorisé dans {filename}"
        );
        for token in &tokens {
            if exported_types.contains(token.as_str()) && token != type_name {
                assert!(
                    imported.contains(token.as_str()),
                    "Dans {filename}, le type '{token}' est référencé sans `import type {{ {token} }} from \"./{token}\";`"
                );
            }
        }
        for dep in &imported {
            assert!(
                tokens.contains(dep.as_str()),
                "Import inutilisé '{dep}' dans {filename}"
            );
        }
    }

    let expected_imports: &[(&str, &[&str])] = &[
        ("CommandEnvelope.ts", &["ProtocolVersion"]),
        ("CommandReceipt.ts", &["EventCursor", "ReceiptState"]),
        (
            "CommandReceiptInspection.ts",
            &["CommandReceipt", "LineageStatus"],
        ),
        ("EventCausation.ts", &["EventCausationKind"]),
        ("EventEnvelope.ts", &["EventCausation", "Sensitivity"]),
        ("HelloRequest.ts", &["ClientKind", "ProtocolVersion"]),
        ("HelloResponse.ts", &["ProtocolVersion"]),
        ("IpcError.ts", &["ErrorCode", "ReasonCode", "RetryClass"]),
        ("QueryEnvelope.ts", &["ProtocolVersion"]),
        ("QueryResponse.ts", &["EventCursor"]),
        ("SubscribeEventsFrame.ts", &["EventCursor", "EventEnvelope"]),
    ];
    for (file, deps) in expected_imports {
        for dep in *deps {
            let expected_line = format!("import type {{ {dep} }} from \"./{dep}\";");
            assert!(
                expected_files[*file].contains(&expected_line),
                "{file} doit contenir `{expected_line}`"
            );
        }
    }

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
