//! Tests d'intégrité et de bijection exacte de `ReasonCode` avec tous les registres du dossier `.claude/`.
// REQ-CORE-005 / REQ-CORE-009

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
use std::path::{Path, PathBuf};
use unleash_protocol::{ErrorCode, IpcError, ProtocolError, ReasonCode, from_json_str_strict};

const ALL_ERROR_CODES: [ErrorCode; 17] = [
    ErrorCode::InvalidArgument,
    ErrorCode::AuthenticationFailed,
    ErrorCode::CapabilityDenied,
    ErrorCode::ScopeDenied,
    ErrorCode::RevisionConflict,
    ErrorCode::IdempotencyConflict,
    ErrorCode::NotFoundOrHidden,
    ErrorCode::ResourceBusy,
    ErrorCode::BudgetExceeded,
    ErrorCode::RateLimited,
    ErrorCode::ProtocolIncompatible,
    ErrorCode::StaleFence,
    ErrorCode::IsolationUnavailable,
    ErrorCode::ContractViolation,
    ErrorCode::OutcomeUnknown,
    ErrorCode::InternalFailure,
    ErrorCode::ResyncRequired,
];

fn collect_markdown_files(claude_dir: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("Impossible de lire le dossier {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("Entrée de répertoire valide");
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|s| s.to_str());
            if dir_name == Some(".git") {
                continue;
            }
            if dir == claude_dir && matches!(dir_name, Some("state" | "worktrees")) {
                continue;
            }
            collect_markdown_files(claude_dir, &path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn extract_class_reason_pairs(text: &str, pairs: &mut BTreeMap<String, BTreeSet<String>>) {
    for err_code in ALL_ERROR_CODES {
        let class_name = err_code.as_str();
        let needle = format!("{class_name}/");
        let mut search_start = 0;

        while let Some(rel_idx) = text[search_start..].find(&needle) {
            let match_start = search_start + rel_idx;
            let prev_ok = match_start == 0 || !is_ident_byte(text.as_bytes()[match_start - 1]);
            let after_slash = match_start + needle.len();

            if prev_ok && after_slash < text.len() {
                let tail = &text.as_bytes()[after_slash..];
                let mut len = 0;
                while len < tail.len()
                    && (tail[len].is_ascii_uppercase()
                        || tail[len].is_ascii_digit()
                        || tail[len] == b'_')
                {
                    len += 1;
                }
                let next_ok = len >= 2
                    && tail[0].is_ascii_uppercase()
                    && (len == tail.len() || !is_ident_byte(tail[len]));

                if next_ok {
                    let reason = &text[after_slash..after_slash + len];
                    pairs
                        .entry(reason.to_owned())
                        .or_default()
                        .insert(class_name.to_owned());
                }
            }

            search_start = after_slash;
        }
    }
}

fn is_valid_upper_snake(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_uppercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
}

#[test]
fn test_reason_codes_bijection_with_dossier_registries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let claude_dir = root.join(".claude");
    let mut md_files = Vec::new();
    collect_markdown_files(&claude_dir, &claude_dir, &mut md_files);
    md_files.sort();

    let excluded_state = claude_dir.join("state");
    let excluded_worktrees = claude_dir.join("worktrees");
    for path in &md_files {
        assert!(
            !path.starts_with(&excluded_state),
            "Le chemin {} ne doit pas provenir de .claude/state/",
            path.display()
        );
        assert!(
            !path.starts_with(&excluded_worktrees),
            "Le chemin {} ne doit pas provenir de .claude/worktrees/",
            path.display()
        );
    }

    let mut expected_pairs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in &md_files {
        let content = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("Impossible de lire {}: {e}", path.display()));
        extract_class_reason_pairs(&content, &mut expected_pairs);
    }

    // 147 paires Class/REASON_CODE dans .claude/ : les 146 de la ligne M2, plus
    // ScopeDenied/ACCESS_DENIED que le contrat F005 écrit sous cette forme depuis M3.
    assert_eq!(
        expected_pairs.len(),
        147,
        "Attendu exactement 147 paires Class/REASON_CODE distinctes dans .claude/"
    );
    assert!(
        expected_pairs
            .get("ACCESS_DENIED")
            .is_some_and(|classes| classes.contains("ScopeDenied")),
        "ScopeDenied/ACCESS_DENIED doit figurer dans le dossier sous la forme Class/REASON_CODE"
    );

    // Compléter avec les raisons enregistrées en prose dans F009/interfaces.md:98
    let f009_path = claude_dir.join("features/F009-knowledge-engine/interfaces.md");
    let f009_text = fs::read_to_string(&f009_path).expect("Lecture F009/interfaces.md");
    let f009_prose = [
        ("RECEIPT_STALE", "ContractViolation"),
        ("RECEIPT_VALIDATION_EXPIRED", "ContractViolation"),
        ("DISCOVERY_HISTORY_UNKNOWN", "ContractViolation"),
        ("ACCESS_DENIED", "ScopeDenied"),
        ("DISCOVERY_ORIGIN_MISMATCH", "ScopeDenied"),
        ("SOURCE_ADVANCED", "ResourceBusy"),
        ("INDEX_STALE", "ResourceBusy"),
        ("SNAPSHOT_INCONSISTENT", "ResourceBusy"),
        ("PROFILE_UNAVAILABLE", "CapabilityDenied"),
        ("DEPENDENCY_CYCLE", "ContractViolation"),
    ];
    for (reason, class_name) in f009_prose {
        assert!(
            f009_text.contains(reason),
            "La raison F009 {reason} doit figurer dans F009/interfaces.md"
        );
        expected_pairs
            .entry(reason.to_owned())
            .or_default()
            .insert(class_name.to_owned());
    }

    // Total attendu : 147 paires, plus les 9 raisons F009 restées en prose (ACCESS_DENIED est
    // déjà comptée) : 156 raisons canoniques, sans aucun conflit de classe
    assert_eq!(expected_pairs.len(), 156);
    for (reason, classes) in &expected_pairs {
        assert_eq!(
            classes.len(),
            1,
            "Conflit de classe pour {reason} dans le dossier : {classes:?}"
        );
    }

    // Vérifier la bijection exacte 1-à-1 avec ReasonCode::all_reasons()
    let all_rust = ReasonCode::all_reasons();
    assert_eq!(
        all_rust.len(),
        156,
        "ReasonCode::all_reasons() doit exposer exactement 156 variantes"
    );

    let schema_path = claude_dir.join("schemas/ipc-error.schema.json");
    let schema_val: Value = serde_json::from_str(
        &fs::read_to_string(&schema_path).expect("Lecture ipc-error.schema.json"),
    )
    .expect("JSON schéma valide");
    let error_validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema_val)
        .expect("Validateur ipc-error.schema.json");

    let error_class_upper: BTreeSet<String> = ALL_ERROR_CODES
        .iter()
        .map(|c| {
            let mut out = String::new();
            for (idx, ch) in c.as_str().chars().enumerate() {
                if idx > 0 && ch.is_uppercase() {
                    out.push('_');
                }
                out.push(ch.to_ascii_uppercase());
            }
            out
        })
        .collect();

    let mut rust_set = BTreeSet::new();
    for reason in all_rust {
        let code_str = reason.as_str();
        assert!(
            rust_set.insert(code_str.to_owned()),
            "Doublon détecté dans ReasonCode::all_reasons() : {code_str}"
        );
        assert!(
            is_valid_upper_snake(code_str),
            "ReasonCode '{code_str}' ne respecte pas ^[A-Z][A-Z0-9_]+$"
        );
        assert!(
            !error_class_upper.contains(code_str)
                && ALL_ERROR_CODES.iter().all(|c| c.as_str() != code_str),
            "ReasonCode '{code_str}' ne doit jamais reprendre le nom d'une classe commune"
        );

        let expected_classes = expected_pairs
            .get(code_str)
            .unwrap_or_else(|| panic!("ReasonCode Rust '{code_str}' non enregistré dans .claude/"));
        let expected_class = expected_classes.iter().next().unwrap();
        assert_eq!(
            reason.error_code().as_str(),
            expected_class,
            "Classe erronée pour ReasonCode '{code_str}'"
        );

        // Roundtrips FromStr / TryFrom / JSON
        let parsed: ReasonCode = code_str.parse().expect("FromStr doit réussir");
        assert_eq!(parsed, *reason);
        let tried = ReasonCode::try_from(code_str).expect("TryFrom doit réussir");
        assert_eq!(tried, *reason);
        let serialized = serde_json::to_string(reason).expect("Sérialisation ReasonCode");
        assert_eq!(serialized, format!("\"{code_str}\""));
        let deserialized: ReasonCode =
            from_json_str_strict(&serialized).expect("Désérialisation stricte ReasonCode");
        assert_eq!(deserialized, *reason);

        // Validation de IpcError::from_reason contre ipc-error.schema.json
        let ipc_err = IpcError::from_reason(*reason, "Erreur contractuelle de test");
        let err_val = serde_json::to_value(&ipc_err).expect("Sérialisation IpcError");
        assert!(
            error_validator.is_valid(&err_val),
            "IpcError::from_reason({code_str}) doit valider ipc-error.schema.json"
        );
    }

    for expected_code in expected_pairs.keys() {
        assert!(
            rust_set.contains(expected_code),
            "Le reason_code '{expected_code}' enregistré dans .claude/ est absent de ReasonCode"
        );
    }

    // Rejet explicite de STREAM_RESYNC_REQUIRED (non enregistré dans .claude/)
    assert!("STREAM_RESYNC_REQUIRED".parse::<ReasonCode>().is_err());
    assert!(matches!(
        from_json_str_strict::<ReasonCode>("\"STREAM_RESYNC_REQUIRED\""),
        Err(ProtocolError::Json(_))
    ));
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_collect_markdown_files_excludes_state_worktrees_and_git() {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let unique_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Horloge système postérieure à UNIX_EPOCH")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unleash-test-reason-codes-{}-{unique_nanos}-{seq}",
        std::process::id()
    ));
    let _guard = TempDirGuard(root.clone());

    let brief_dir = root.join("brief");
    let state_dir = root.join("state");
    let worktrees_dir = root.join("worktrees/g0001-m0-t02b");
    let root_git_dir = root.join(".git");
    let nested_git_dir = root.join("features/.git");

    fs::create_dir_all(&brief_dir).expect("Création dossier brief");
    fs::create_dir_all(&state_dir).expect("Création dossier state");
    fs::create_dir_all(&worktrees_dir).expect("Création dossier worktrees");
    fs::create_dir_all(&root_git_dir).expect("Création dossier .git racine");
    fs::create_dir_all(&nested_git_dir).expect("Création dossier .git imbriqué");

    let expected_md = brief_dir.join("doc.md");
    fs::write(&expected_md, "# Conception\n").expect("Écriture brief/doc.md");
    fs::write(state_dir.join("journal.md"), "ScopeDenied/ACCESS_DENIED\n")
        .expect("Écriture state/journal.md");
    fs::write(worktrees_dir.join("notes.md"), "Worktree note\n")
        .expect("Écriture worktrees/notes.md");
    fs::write(root_git_dir.join("ignored.md"), "Git root note\n")
        .expect("Écriture .git/ignored.md");
    fs::write(nested_git_dir.join("ignored.md"), "Git nested note\n")
        .expect("Écriture features/.git/ignored.md");

    let mut collected = Vec::new();
    collect_markdown_files(&root, &root, &mut collected);
    collected.sort();

    assert_eq!(collected, vec![expected_md]);
}
