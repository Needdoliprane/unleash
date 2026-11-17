//! Tests des trois scénarios du spike M0 n°4 (contrats observables et connaissance multilangage).
// REQ-CONTRACT-004 / REQ-CONTRACT-005 / REQ-CONTRACT-006 / REQ-CONTRACT-007
// REQ-KNOW-001 / REQ-KNOW-002 / REQ-KNOW-003

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use spike_m0_04_contracts_knowledge::{
    ActivationRef, AnalyzerProbeConfig, AuthorityKind, AuthorityRef, CaptureConsistency,
    ClaimProposition, CodeState, CommandSpec, ContractClaim, DiscoveryReceipt, ExactRef,
    ExecutionResult, ExpectedExecutionBinding, ExpectedTestExecution, HexSha256, LanguageProfile,
    ObservationRecord, OnUnknownPolicy, ParseState, ProcessState, ProfileAvailability,
    RelationConfidence, ResultPolicy, RuleOutcome, SchemaKind, SchemaRegistry, SealBindingRequest,
    SealedActionJournal, SnapshotKind, SnapshotRef, SubjectRef, TestCasesSummary,
    TestExecutionObservation, ToolOrParserRef, VerdictStatus, capture_git_snapshot_with_hook,
    discover_multilanguage_repo, evaluate_contract_verdict, parse_cargo_test_text, parse_junit_xml,
    probe_analyzer, validate_claim_predicate_matches, validate_git_snapshot_matches,
    validate_test_execution_matches,
};
use unleash_domain::{IdGeneratorPort, StreamId};
use unleash_protocol::ReasonCode;
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

fn git(repo_dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .expect("Exécution de git");
    assert!(
        out.status.success(),
        "git {:?} a échoué : {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_temp_git_repo(prefix: &str) -> tempfile::TempDir {
    let dir = local_test_dir(prefix).expect("Répertoire temporaire sous /tmp");
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Unleash Spike Test"]);
    git(dir.path(), &["config", "user.email", "spike@unleash.local"]);
    dir
}

fn sample_command_spec() -> CommandSpec {
    CommandSpec {
        tool_ref: ToolOrParserRef {
            id: String::from("tool.process.run"),
            version: 1,
        },
        argv_sha256: HexSha256::of_bytes(b"cargo nextest run --profile junit"),
        cwd_resource_id: String::from("resource-repo-root-01"),
        environment_profile_ref: ExactRef {
            id: String::from("env-profile-v1"),
            version: 1,
            sha256: HexSha256::of_bytes(b"env-profile-v1"),
        },
        environment_sha256: HexSha256::of_bytes(b"LANG=C.UTF-8"),
        suite_ref: ExactRef {
            id: String::from("suite-workspace-v1"),
            version: 1,
            sha256: HexSha256::of_bytes(b"suite-workspace-v1"),
        },
        parser_ref: ToolOrParserRef {
            id: String::from("parser.junit-xml"),
            version: 1,
        },
    }
}

fn build_subject(snapshot_ref: SnapshotRef) -> SubjectRef {
    SubjectRef {
        subject_id: String::from("task-spike-m0-04"),
        subject_revision: 1,
        scope_id: String::from("scope-spike-repo"),
        scope_revision: 1,
        snapshot_ref,
    }
}

#[test]
// REQ-KNOW-001 / REQ-KNOW-003 : Patch multilangage avec analyseur absent -> profile_unavailable
fn scenario_1_multilanguage_patch_profile_unavailable_and_valid_receipt() {
    let registry = SchemaRegistry::load_default().expect("Chargement des schémas locaux");
    let repo = init_temp_git_repo("m0-s1-multi");
    let clock = ManualClock::new(1_763_301_600_000); // 2025-11-16T14:00:00Z
    let ids = DeterministicIdGenerator::new(0x51_0001);

    fs::create_dir_all(repo.path().join("src")).unwrap();
    fs::create_dir_all(repo.path().join("app")).unwrap();
    fs::create_dir_all(repo.path().join("web")).unwrap();

    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn normalize_email_rs(raw: &str) -> String {\n    raw.trim().to_lowercase()\n}\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("app/email.py"),
        "def normalize_email_py(raw: str) -> str:\n    return raw.strip().lower()\n\ndef verify_sender():\n    return normalize_email_py('a@b.c')\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("web/email.ts"),
        "export function normalize_email_ts(raw: string): string {\n  return raw.trim().toLowerCase();\n}\n",
    )
    .unwrap();

    git(repo.path(), &["add", "."]);
    git(
        repo.path(),
        &["commit", "-m", "chore: initial multilang fixture"],
    );

    // Probe isolé : aucun analyseur externe disponible -> profile_unavailable explicite
    let isolated_probe = AnalyzerProbeConfig::isolated();
    let out = discover_multilanguage_repo(
        repo.path(),
        "repo-multilang-01",
        Some("worktree-main"),
        "normalize_email",
        &isolated_probe,
        &clock,
        &ids,
    )
    .expect("Découverte multilangage réussie en mode dégradé");

    assert_eq!(out.snapshot.code_state, CodeState::Committed);
    assert_eq!(
        out.snapshot.capture_consistency,
        CaptureConsistency::GitTreeCoherent
    );

    // Pyright absent -> Unavailable + ReasonCode::ProfileUnavailable
    let py_qual = out
        .qualifications
        .get(&LanguageProfile::Python)
        .expect("Qualification Python présente");
    assert_eq!(py_qual.availability, ProfileAvailability::Unavailable);
    assert_eq!(py_qual.reason_code, Some(ReasonCode::ProfileUnavailable));

    let rs_qual = out
        .qualifications
        .get(&LanguageProfile::Rust)
        .expect("Qualification Rust présente");
    assert_eq!(rs_qual.availability, ProfileAvailability::Unavailable);
    assert_eq!(rs_qual.reason_code, Some(ReasonCode::ProfileUnavailable));

    let ts_qual = out
        .qualifications
        .get(&LanguageProfile::TypeScript)
        .expect("Qualification TypeScript présente");
    assert_eq!(ts_qual.availability, ProfileAvailability::Unavailable);
    assert_eq!(ts_qual.reason_code, Some(ReasonCode::ProfileUnavailable));

    // Aucune relation semantic_resolved ne doit jamais être inventée sans LSP qualifié
    assert!(
        !out.relations
            .iter()
            .any(|rel| rel.confidence == RelationConfidence::SemanticResolved),
        "Aucune relation semantic_resolved ne peut être émise lorsque l'analyseur est absent/non qualifié"
    );
    assert!(
        out.relations
            .iter()
            .any(|rel| rel.confidence == RelationConfidence::Lexical
                && rel.to == "symbol:python:normalize_email_py"),
        "La dégradation lexicale doit rester explicite et fonctionnelle"
    );

    // Tous les symboles des 3 langages sont découverts lexicalement
    assert!(
        out.symbols
            .iter()
            .any(|s| s.candidate_id == "symbol:rust:normalize_email_rs")
    );
    assert!(
        out.symbols
            .iter()
            .any(|s| s.candidate_id == "symbol:python:normalize_email_py")
    );
    assert!(
        out.symbols
            .iter()
            .any(|s| s.candidate_id == "symbol:typescript:normalize_email_ts")
    );

    // Validation stricte du DiscoveryReceipt produit contre discovery-receipt.schema.json
    let receipt_val = registry
        .validate_serializable(SchemaKind::DiscoveryReceipt, &out.receipt)
        .expect("DiscoveryReceipt doit valider discovery-receipt.schema.json");
    assert_eq!(
        receipt_val["source_refs"][0]["capture_consistency"],
        "git_tree_coherent"
    );

    // Vérification sur l'environnement hôte réel (sans jamais dépendre de présence d'outil)
    let host_probe = AnalyzerProbeConfig::host_toolchain();
    let host_py = probe_analyzer(LanguageProfile::Python, &host_probe).unwrap();
    assert_ne!(
        host_py.availability,
        ProfileAvailability::Ready,
        "Pyright ne peut être Ready sans handshake LSP + fixture"
    );
    let host_rs = probe_analyzer(LanguageProfile::Rust, &host_probe).unwrap();
    assert_ne!(
        host_rs.availability,
        ProfileAvailability::Ready,
        "rust-analyzer détecté ou absent ne doit pas se déclarer Ready sans qualification LSP"
    );

    // Mutations négatives du DiscoveryReceipt
    let mut bad_missing_coverage = receipt_val.clone();
    bad_missing_coverage
        .as_object_mut()
        .unwrap()
        .remove("coverage_ref");
    assert!(
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &bad_missing_coverage)
            .is_err(),
        "Suppression de coverage_ref doit être rejetée par le schéma"
    );

    let mut bad_non_null_target = receipt_val.clone();
    bad_non_null_target["target_resource_id"] = json!("res-invented-01");
    assert!(
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &bad_non_null_target)
            .is_err(),
        "create + pre_creation avec target_resource_id non nul doit être rejeté"
    );

    let mut bad_date = receipt_val;
    bad_date["recorded_at"] = json!("yesterday");
    assert!(
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &bad_date)
            .is_err(),
        "Une date-time non RFC3339 doit être rejetée avec should_validate_formats(true)"
    );

    // Ajouter un champ Git (repo_id) sur une variante file_snapshot doit être rejeté (unevaluatedProperties: false)
    let mut bad_file_with_git = json!({
        "schema_version": "unleash.discovery.v1",
        "receipt_id": "rec-neg-file-git",
        "intent_id": "intent-neg-01",
        "origin": {
            "origin_kind": "control_plane",
            "principal_id": "user-01",
            "control_principal_id": "user-01",
            "cause": { "kind": "command_receipt", "id": "cmd-01" }
        },
        "query_ids": ["q-01"],
        "catalog_revision": 1,
        "purpose": "pre_creation",
        "decision": "create",
        "strategy": "new",
        "candidate_ids": [],
        "target_resource_id": null,
        "target_version_ref": null,
        "justification": "Test négatif mélange champs Git et file_snapshot",
        "source_refs": [{
            "source_kind": "file_snapshot",
            "source_id": "src-files",
            "version_id": "v1",
            "content_digest": {
                "algorithm": "sha256",
                "hex": "1111111111111111111111111111111111111111111111111111111111111111"
            },
            "generation": 1,
            "access_policy_ref": "policy-01",
            "acl_revision": 1,
            "observed_at": "2026-11-16T14:00:00Z",
            "capture_consistency": "unverified",
            "source_binding_id": "bind-01",
            "root_ref": "root-01",
            "repo_id": "forbidden-git-field"
        }],
        "freshness": "valid",
        "temporal_relation": "before_creation",
        "validation_basis": {
            "intent_revision": 1,
            "scope_ref": "scope-01",
            "scope_revision": 1,
            "policy_version_ref": "pol-v1",
            "validator_version": "val-v1",
            "event_cursor": {
                "stream_id": "00000000-0000-4000-8000-000000000201",
                "sequence": 1
            },
            "checked_at": "2026-11-16T14:00:00Z",
            "valid_until": "2026-11-16T14:05:00Z",
            "max_age_ms": 300_000
        },
        "recorded_at": "2026-11-16T14:00:01Z",
        "supersedes_receipt_id": null,
        "creation_use_id": null,
        "coverage_ref": "cov-01"
    });
    assert!(
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &bad_file_with_git)
            .is_err(),
        "Des champs Git sur file_snapshot doivent être rejetés"
    );
    bad_file_with_git["source_refs"][0]
        .as_object_mut()
        .unwrap()
        .remove("repo_id");
    assert!(
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &bad_file_with_git)
            .is_ok()
    );
}

#[test]
// REQ-KNOW-002 / REQ-CONTRACT-005 / REQ-CONTRACT-006 : Worktree dirty muté pendant l'analyse -> inconsistent -> unknown
fn scenario_2_dirty_worktree_mutation_during_capture_produces_unknown() {
    let registry = SchemaRegistry::load_default().expect("Chargement des schémas locaux");
    let repo = init_temp_git_repo("m0-s2-dirty");
    let clock = ManualClock::new(1_763_301_600_000);
    let ids = DeterministicIdGenerator::new(0x52_0002);

    fs::create_dir_all(repo.path().join("src")).unwrap();
    fs::write(repo.path().join("src/lib.rs"), "pub fn v() -> i32 { 1 }\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "init"]);

    // Rendre le worktree dirty (fichier suivi modifié + fichier non suivi ajouté)
    fs::write(repo.path().join("src/lib.rs"), "pub fn v() -> i32 { 2 }\n").unwrap();
    fs::write(
        repo.path().join("src/extra.py"),
        "def helper():\n    return 2\n",
    )
    .unwrap();

    // 1. Capture avec mutation déterministe injectée entre passe 1 et passe 2 (sans sleep)
    let inconsistent_snap = capture_git_snapshot_with_hook(
        repo.path(),
        "repo-dirty-01",
        Some("worktree-dirty-01"),
        "source-dirty-01",
        10,
        &clock,
        |dir| {
            fs::write(dir.join("src/lib.rs"), "pub fn v() -> i32 { 3 }\n")?;
            Ok(())
        },
    )
    .expect("Capture avec hook");

    assert_eq!(inconsistent_snap.code_state, CodeState::Dirty);
    assert_eq!(
        inconsistent_snap.capture_consistency,
        CaptureConsistency::Inconsistent
    );
    assert_eq!(
        inconsistent_snap.diagnostic_reason,
        Some(ReasonCode::SnapshotInconsistent)
    );
    assert_ne!(
        inconsistent_snap.manifest_sha256,
        inconsistent_snap.verification_sha256
    );
    assert_eq!(
        inconsistent_snap.snapshot_ref.kind,
        SnapshotKind::SourceSnapshot
    );

    let subject = build_subject(inconsistent_snap.snapshot_ref.clone());
    let expected = ExpectedTestExecution {
        schema_version: String::from("unleash.expected-test-execution.v1"),
        expectation_id: String::from("exp-dirty-01"),
        version: 1,
        authority_ref: AuthorityRef {
            kind: AuthorityKind::Contract,
            id: String::from("contract-dirty-01"),
            version: 1,
            sha256: HexSha256::of_bytes(b"contract-dirty-01"),
        },
        subject: subject.clone(),
        command: sample_command_spec(),
        result_policy: ResultPolicy {
            expected_exit_code: 0,
            min_cases: 1,
            require_all_passed: true,
        },
    };
    registry
        .validate_serializable(SchemaKind::ExpectedTestExecution, &expected)
        .expect("ExpectedTestExecution valide le schéma");

    let mut journal = SealedActionJournal::new();
    let stream_id = StreamId::from_uuid(ids.new_uuid());
    let action_id = String::from("action-dirty-01");
    let binding = journal
        .seal_binding(SealBindingRequest {
            binding_id: String::from("binding-dirty-01"),
            contract_ref: ExactRef {
                id: String::from("contract-dirty-01"),
                version: 1,
                sha256: HexSha256::of_bytes(b"contract-dirty-01"),
            },
            rule_ids: vec![
                String::from("snapshot-coherent"),
                String::from("tests-required"),
            ],
            expectation_slot: String::from("workspace-tests"),
            expectation: expected.clone(),
            activation_ref: ActivationRef {
                id: String::from("activation-dirty-01"),
                revision: 1,
            },
            action_id: action_id.clone(),
            stream_id,
        })
        .expect("Scellement du binding");
    registry
        .validate_serializable(SchemaKind::ExpectedExecutionBinding, &binding)
        .expect("ExpectedExecutionBinding valide le schéma");

    // Même si une observation prétend 1/1 tests verts, la capture incohérente interdit `Pass`
    let obs = TestExecutionObservation {
        schema_version: String::from("unleash.test-execution-observation.v1"),
        observation_id: String::from("obs-dirty-01"),
        producer_ref: ToolOrParserRef {
            id: String::from("adapter.nextest"),
            version: 1,
        },
        action_id: action_id.clone(),
        execution_binding_id: binding.binding_id.clone(),
        expectation_ref: expected.exact_ref().unwrap(),
        subject: subject.clone(),
        observed_command: sample_command_spec(),
        result: ExecutionResult {
            process_state: ProcessState::Exited,
            exit_code: Some(0),
            parse_state: ParseState::Complete,
            cases: Some(TestCasesSummary {
                total: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
            }),
        },
    };
    registry
        .validate_serializable(SchemaKind::TestExecutionObservation, &obs)
        .expect("TestExecutionObservation valide le schéma");
    journal.record_observation(obs).unwrap();

    let v_snap = validate_git_snapshot_matches(
        "snapshot-coherent",
        true,
        &subject,
        &inconsistent_snap.typed_source_ref,
    );
    assert_eq!(v_snap.outcome, RuleOutcome::Unknown);
    assert_eq!(v_snap.reason, Some(ReasonCode::SnapshotInconsistent));

    let v_exec = validate_test_execution_matches(
        "tests-required",
        true,
        &expected,
        &binding,
        journal.observations_for_action(&action_id),
        inconsistent_snap.capture_consistency,
    )
    .unwrap();
    assert_eq!(v_exec.outcome, RuleOutcome::Unknown);
    assert_eq!(v_exec.reason, Some(ReasonCode::SnapshotInconsistent));

    let verdict = evaluate_contract_verdict(vec![
        (v_snap, OnUnknownPolicy::Block),
        (v_exec, OnUnknownPolicy::Block),
    ]);
    assert_eq!(verdict.status, VerdictStatus::Unknown);

    // 2. Capture dirty séquentielle sans mutation -> reste Unverified (jamais atomic_fs_coherent) -> Unknown
    let unverified_snap = capture_git_snapshot_with_hook(
        repo.path(),
        "repo-dirty-01",
        Some("worktree-dirty-01"),
        "source-dirty-01",
        11,
        &clock,
        |_| Ok(()),
    )
    .expect("Capture séquentielle dirty sans mutation");
    assert_eq!(unverified_snap.code_state, CodeState::Dirty);
    assert_eq!(
        unverified_snap.capture_consistency,
        CaptureConsistency::Unverified
    );
    let v_unverified = validate_git_snapshot_matches(
        "snapshot-coherent",
        true,
        &build_subject(unverified_snap.snapshot_ref),
        &unverified_snap.typed_source_ref,
    );
    assert_eq!(v_unverified.outcome, RuleOutcome::Unknown);

    // 3. Voie forte : commit propre -> GitTreeCoherent -> Pass
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "commit clean state"]);
    let clean_snap = capture_git_snapshot_with_hook(
        repo.path(),
        "repo-dirty-01",
        Some("worktree-dirty-01"),
        "source-dirty-01",
        12,
        &clock,
        |_| Ok(()),
    )
    .expect("Capture tree propre");
    assert_eq!(clean_snap.code_state, CodeState::Committed);
    assert_eq!(
        clean_snap.capture_consistency,
        CaptureConsistency::GitTreeCoherent
    );
    let v_clean = validate_git_snapshot_matches(
        "snapshot-coherent",
        true,
        &build_subject(clean_snap.snapshot_ref),
        &clean_snap.typed_source_ref,
    );
    assert_eq!(v_clean.outcome, RuleOutcome::Pass);
}

struct Scenario3Fixture {
    registry: SchemaRegistry,
    stream_id: StreamId,
    current_subject: SubjectRef,
    old_subject: SubjectRef,
    expected: ExpectedTestExecution,
    expected_ref: ExactRef,
    contract_ref: ExactRef,
}

fn scenario_3_fixture() -> Scenario3Fixture {
    let registry = SchemaRegistry::load_default().expect("Chargement des schémas locaux");
    let ids = DeterministicIdGenerator::new(0x53_0003);
    let stream_id = StreamId::from_uuid(ids.new_uuid());

    let current_subject = build_subject(SnapshotRef {
        kind: SnapshotKind::GitTree,
        id: String::from("git-snap-current-002"),
        sha256: HexSha256::of_bytes(b"manifest-commit-002"),
    });
    let old_subject = build_subject(SnapshotRef {
        kind: SnapshotKind::GitTree,
        id: String::from("git-snap-old-001"),
        sha256: HexSha256::of_bytes(b"manifest-commit-001"),
    });

    let expected = ExpectedTestExecution {
        schema_version: String::from("unleash.expected-test-execution.v1"),
        expectation_id: String::from("exp-suite-01"),
        version: 1,
        authority_ref: AuthorityRef {
            kind: AuthorityKind::ExecutionPlan,
            id: String::from("plan-m0-04"),
            version: 1,
            sha256: HexSha256::of_bytes(b"plan-m0-04"),
        },
        subject: current_subject.clone(),
        command: sample_command_spec(),
        result_policy: ResultPolicy {
            expected_exit_code: 0,
            min_cases: 1,
            require_all_passed: true,
        },
    };
    registry
        .validate_serializable(SchemaKind::ExpectedTestExecution, &expected)
        .unwrap();
    let expected_ref = expected.exact_ref().unwrap();
    let contract_ref = ExactRef {
        id: String::from("contract.repository.verify-tests"),
        version: 1,
        sha256: HexSha256::of_bytes(b"contract.repository.verify-tests"),
    };

    Scenario3Fixture {
        registry,
        stream_id,
        current_subject,
        old_subject,
        expected,
        expected_ref,
        contract_ref,
    }
}

#[test]
// REQ-CONTRACT-005 / REQ-CONTRACT-006 : Sortie texte brute `cargo test` sans JUnit -> parsing incomplete -> verdict unknown
fn scenario_3a_cargo_test_text_without_junit_produces_unknown() {
    let fix = scenario_3_fixture();
    let mut journal = SealedActionJournal::new();

    let cargo_text = "\
running 3 tests
test alpha ... ok
test beta ... ok
test gamma ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";
    let parsed_cargo = parse_cargo_test_text(cargo_text);
    assert_eq!(parsed_cargo.parse_state, ParseState::Incomplete);
    assert_eq!(parsed_cargo.cases, None);

    let mut cmd_cargo_text = sample_command_spec();
    cmd_cargo_text.parser_ref = parsed_cargo.parser_ref;
    let mut expected_cargo = fix.expected;
    expected_cargo.command = cmd_cargo_text.clone();
    fix.registry
        .validate_serializable(SchemaKind::ExpectedTestExecution, &expected_cargo)
        .unwrap();

    let binding = journal
        .seal_binding(SealBindingRequest {
            binding_id: String::from("binding-3a"),
            contract_ref: fix.contract_ref,
            rule_ids: vec![String::from("tests-required")],
            expectation_slot: String::from("workspace-tests"),
            expectation: expected_cargo.clone(),
            activation_ref: ActivationRef {
                id: String::from("act-3a"),
                revision: 1,
            },
            action_id: String::from("action-3a"),
            stream_id: fix.stream_id,
        })
        .unwrap();
    fix.registry
        .validate_serializable(SchemaKind::ExpectedExecutionBinding, &binding)
        .unwrap();

    let obs = TestExecutionObservation {
        schema_version: String::from("unleash.test-execution-observation.v1"),
        observation_id: String::from("obs-3a"),
        producer_ref: ToolOrParserRef {
            id: String::from("adapter.process"),
            version: 1,
        },
        action_id: String::from("action-3a"),
        execution_binding_id: binding.binding_id.clone(),
        expectation_ref: expected_cargo.exact_ref().unwrap(),
        subject: fix.current_subject,
        observed_command: cmd_cargo_text,
        result: ExecutionResult {
            process_state: ProcessState::Exited,
            exit_code: Some(0),
            parse_state: parsed_cargo.parse_state,
            cases: parsed_cargo.cases,
        },
    };
    fix.registry
        .validate_serializable(SchemaKind::TestExecutionObservation, &obs)
        .unwrap();
    journal.record_observation(obs).unwrap();

    let verdict = validate_test_execution_matches(
        "tests-required",
        true,
        &expected_cargo,
        &binding,
        journal.observations_for_action("action-3a"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(verdict.outcome, RuleOutcome::Unknown);
    assert_eq!(verdict.reason, Some(ReasonCode::EvidenceMissing));
    let global = evaluate_contract_verdict(vec![(verdict, OnUnknownPolicy::Block)]);
    assert_eq!(global.status, VerdictStatus::Unknown);
}

#[test]
// REQ-CONTRACT-005 : JUnit XML valide mais issu d'un commit/snapshot antérieur -> EvidenceSnapshotMismatch
fn scenario_3b_stale_junit_xml_produces_snapshot_mismatch() {
    let fix = scenario_3_fixture();
    let stale_junit_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest" tests="2" failures="0" errors="0" skipped="0">
  <testsuite name="unit" tests="2" failures="0" errors="0" skipped="0">
    <testcase name="test_one" />
    <testcase name="test_two" />
  </testsuite>
</testsuites>"#;
    let parsed_stale = parse_junit_xml(stale_junit_xml);
    assert_eq!(parsed_stale.parse_state, ParseState::Complete);
    assert_eq!(
        parsed_stale.cases,
        Some(TestCasesSummary {
            total: 2,
            passed: 2,
            failed: 0,
            skipped: 0,
        })
    );

    let mut journal = SealedActionJournal::new();
    let binding = journal
        .seal_binding(SealBindingRequest {
            binding_id: String::from("binding-3b"),
            contract_ref: fix.contract_ref,
            rule_ids: vec![String::from("tests-required")],
            expectation_slot: String::from("workspace-tests"),
            expectation: fix.expected.clone(),
            activation_ref: ActivationRef {
                id: String::from("act-3b"),
                revision: 1,
            },
            action_id: String::from("action-3b"),
            stream_id: fix.stream_id,
        })
        .unwrap();
    fix.registry
        .validate_serializable(SchemaKind::ExpectedExecutionBinding, &binding)
        .unwrap();

    let obs = TestExecutionObservation {
        schema_version: String::from("unleash.test-execution-observation.v1"),
        observation_id: String::from("obs-3b-stale"),
        producer_ref: ToolOrParserRef {
            id: String::from("adapter.nextest"),
            version: 1,
        },
        action_id: String::from("action-3b"),
        execution_binding_id: binding.binding_id.clone(),
        expectation_ref: fix.expected_ref,
        subject: fix.old_subject,
        observed_command: sample_command_spec(),
        result: ExecutionResult {
            process_state: ProcessState::Exited,
            exit_code: Some(0),
            parse_state: parsed_stale.parse_state,
            cases: parsed_stale.cases,
        },
    };
    fix.registry
        .validate_serializable(SchemaKind::TestExecutionObservation, &obs)
        .unwrap();
    journal.record_observation(obs).unwrap();

    let verdict = validate_test_execution_matches(
        "tests-required",
        true,
        &fix.expected,
        &binding,
        journal.observations_for_action("action-3b"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(verdict.outcome, RuleOutcome::Fail);
    assert_eq!(verdict.reason, Some(ReasonCode::EvidenceSnapshotMismatch));

    let truncated_xml = "<testsuite name=\"unit\" tests=\"2\" failures=\"0\">";
    assert_eq!(
        parse_junit_xml(truncated_xml).parse_state,
        ParseState::Incomplete
    );
}

#[test]
// REQ-CONTRACT-004 / REQ-CONTRACT-007 : Claim mensongère (omettant l'observation d'échec) -> ClaimContradicted + conservation
fn scenario_3c_lying_claim_contradicted_preserving_both_objects() {
    let fix = scenario_3_fixture();
    let registry = fix.registry;
    let stream_id = fix.stream_id;
    let current_subject = fix.current_subject;
    let expected = fix.expected;
    let expected_ref = fix.expected_ref;
    let contract_ref = fix.contract_ref;

    let failing_junit_xml = r#"<testsuite name="unit" tests="2" failures="1" errors="0" skipped="0">
  <testcase name="test_ok" />
  <testcase name="test_ko"><failure message="boom"/></testcase>
</testsuite>"#;
    let parsed_fail = parse_junit_xml(failing_junit_xml);
    assert_eq!(parsed_fail.parse_state, ParseState::Complete);
    assert_eq!(
        parsed_fail.cases,
        Some(TestCasesSummary {
            total: 2,
            passed: 1,
            failed: 1,
            skipped: 0,
        })
    );

    let mut journal_3c = SealedActionJournal::new();
    let binding_3c = journal_3c
        .seal_binding(SealBindingRequest {
            binding_id: String::from("binding-3c"),
            contract_ref,
            rule_ids: vec![String::from("tests-required"), String::from("claim-truth")],
            expectation_slot: String::from("workspace-tests"),
            expectation: expected.clone(),
            activation_ref: ActivationRef {
                id: String::from("act-3c"),
                revision: 1,
            },
            action_id: String::from("action-3c"),
            stream_id,
        })
        .unwrap();
    registry
        .validate_serializable(SchemaKind::ExpectedExecutionBinding, &binding_3c)
        .unwrap();

    let obs_3c_fail = TestExecutionObservation {
        schema_version: String::from("unleash.test-execution-observation.v1"),
        observation_id: String::from("obs-3c-fail"),
        producer_ref: ToolOrParserRef {
            id: String::from("adapter.nextest"),
            version: 1,
        },
        action_id: String::from("action-3c"),
        execution_binding_id: binding_3c.binding_id.clone(),
        expectation_ref: expected_ref.clone(),
        subject: current_subject.clone(),
        observed_command: sample_command_spec(),
        result: ExecutionResult {
            process_state: ProcessState::Exited,
            exit_code: Some(1),
            parse_state: parsed_fail.parse_state,
            cases: parsed_fail.cases,
        },
    };
    registry
        .validate_serializable(SchemaKind::TestExecutionObservation, &obs_3c_fail)
        .unwrap();
    journal_3c.record_observation(obs_3c_fail.clone()).unwrap();

    // Le modèle tente d'omettre `obs-3c-fail` dans `observation_ids` et affirme true
    let lying_claim = ContractClaim {
        schema_version: String::from("unleash.contract-claim.v1"),
        claim_id: String::from("claim-lying-3c"),
        author_ref: String::from("principal-agent-01"),
        subject: current_subject.clone(),
        statement: String::from("Tous les tests ont réussi sans erreur."),
        proposition: ClaimProposition::ObservableTestExpectationSatisfied {
            expectation_ref: expected_ref.clone(),
            execution_binding_id: binding_3c.binding_id.clone(),
            asserted_value: true,
        },
        observation_ids: vec![],
    };
    registry
        .validate_serializable(SchemaKind::ContractClaim, &lying_claim)
        .unwrap();

    let rec = validate_claim_predicate_matches(
        "claim-truth",
        true,
        &lying_claim,
        &expected,
        &binding_3c,
        journal_3c.observations_for_action("action-3c"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec.verdict.outcome, RuleOutcome::Fail);
    assert_eq!(rec.verdict.reason, Some(ReasonCode::ClaimContradicted));
    // Conservation obligatoire des deux objets : claim mensonger + observation échouée
    assert_eq!(rec.claim, lying_claim);
    assert_eq!(
        rec.linked_observations,
        vec![ObservationRecord::projected(obs_3c_fail.clone())]
    );

    // Modifier uniquement `statement` ne change aucun verdict
    let mut paraphrased_claim = lying_claim.clone();
    paraphrased_claim.statement = String::from("Je suis convaincu que la CI passe.");
    let rec_para = validate_claim_predicate_matches(
        "claim-truth",
        true,
        &paraphrased_claim,
        &expected,
        &binding_3c,
        journal_3c.observations_for_action("action-3c"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_para.verdict.outcome, RuleOutcome::Fail);
    assert_eq!(rec_para.verdict.reason, Some(ReasonCode::ClaimContradicted));

    // Un claim honnête (`asserted_value = false`) satisfait `claim_predicate_matches`,
    // mais ne satisfait pas `test_execution_matches` obligatoire -> verdict global Fail
    let honest_claim = ContractClaim {
        schema_version: String::from("unleash.contract-claim.v1"),
        claim_id: String::from("claim-honest-3c"),
        author_ref: String::from("principal-agent-01"),
        subject: current_subject.clone(),
        statement: String::from("Un test échoue dans la suite."),
        proposition: ClaimProposition::ObservableTestExpectationSatisfied {
            expectation_ref: expected_ref.clone(),
            execution_binding_id: binding_3c.binding_id.clone(),
            asserted_value: false,
        },
        observation_ids: vec![String::from("obs-3c-fail")],
    };
    registry
        .validate_serializable(SchemaKind::ContractClaim, &honest_claim)
        .unwrap();

    let rec_honest = validate_claim_predicate_matches(
        "claim-truth",
        true,
        &honest_claim,
        &expected,
        &binding_3c,
        journal_3c.observations_for_action("action-3c"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_honest.verdict.outcome, RuleOutcome::Pass);

    let exec_fail = validate_test_execution_matches(
        "tests-required",
        true,
        &expected,
        &binding_3c,
        journal_3c.observations_for_action("action-3c"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(exec_fail.outcome, RuleOutcome::Fail);

    let global_honest_on_failing_suite = evaluate_contract_verdict(vec![
        (rec_honest.verdict, OnUnknownPolicy::Block),
        (exec_fail, OnUnknownPolicy::Block),
    ]);
    assert_eq!(global_honest_on_failing_suite.status, VerdictStatus::Fail);

    // Un claim qualitatif vaut toujours Unknown sans jamais être promu en Pass
    let qual_claim = ContractClaim {
        schema_version: String::from("unleash.contract-claim.v1"),
        claim_id: String::from("claim-qual-3c"),
        author_ref: String::from("principal-agent-01"),
        subject: current_subject,
        statement: String::from("Le code semble propre et idiomatique."),
        proposition: ClaimProposition::Qualitative,
        observation_ids: vec![],
    };
    registry
        .validate_serializable(SchemaKind::ContractClaim, &qual_claim)
        .unwrap();
    let rec_qual = validate_claim_predicate_matches(
        "claim-truth",
        true,
        &qual_claim,
        &expected,
        &binding_3c,
        journal_3c.observations_for_action("action-3c"),
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_qual.verdict.outcome, RuleOutcome::Unknown);

    // Collecte incomplète d'une commande : aucune projection V1 émise -> Unknown
    journal_3c
        .record_incomplete_collection("action-3c", "binding-3c", "argv_sha256")
        .unwrap();
    // Ajouter une seconde observation aurait été détecté comme ambiguïté s'il y avait 2 projetées,
    // ici la collecte incomplète reste visible et n'ajoute pas de projection V1.
    assert_eq!(journal_3c.observations_for_action("action-3c").len(), 2);

    // Prédicat fermé `process.exit_code` v1 : contredit vs corroboré
    let exit_claim_false = ContractClaim {
        schema_version: String::from("unleash.contract-claim.v1"),
        claim_id: String::from("claim-exit-lying"),
        author_ref: String::from("principal-agent-01"),
        subject: obs_3c_fail.subject.clone(),
        statement: String::from("Le processus s'est terminé avec code 0."),
        proposition: ClaimProposition::ObservableProcessExitCode {
            expectation_ref: expected_ref,
            execution_binding_id: binding_3c.binding_id.clone(),
            asserted_value: 0,
        },
        observation_ids: vec![],
    };
    registry
        .validate_serializable(SchemaKind::ContractClaim, &exit_claim_false)
        .unwrap();
    let rec_exit = validate_claim_predicate_matches(
        "claim-exit",
        true,
        &exit_claim_false,
        &expected,
        &binding_3c,
        &[ObservationRecord::projected(obs_3c_fail.clone())],
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_exit.verdict.outcome, RuleOutcome::Fail);
    assert_eq!(rec_exit.verdict.reason, Some(ReasonCode::ClaimContradicted));

    // Claim avec mauvaise référence d'attendu -> EvidenceExecutionMismatch (pas ClaimContradicted)
    let mut wrong_ref_claim = exit_claim_false.clone();
    wrong_ref_claim.proposition = ClaimProposition::ObservableProcessExitCode {
        expectation_ref: ExactRef {
            id: String::from("other-expectation"),
            version: 1,
            sha256: HexSha256::of_bytes(b"other-expectation"),
        },
        execution_binding_id: binding_3c.binding_id.clone(),
        asserted_value: 0,
    };
    let rec_mismatch = validate_claim_predicate_matches(
        "claim-exit",
        true,
        &wrong_ref_claim,
        &expected,
        &binding_3c,
        &[ObservationRecord::projected(obs_3c_fail.clone())],
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_mismatch.verdict.outcome, RuleOutcome::Fail);
    assert_eq!(
        rec_mismatch.verdict.reason,
        Some(ReasonCode::EvidenceExecutionMismatch)
    );

    // Deux observations incompatibles pour la même action -> Unknown / EvidenceAmbiguous
    let mut obs_3c_alt = obs_3c_fail.clone();
    obs_3c_alt.observation_id = String::from("obs-3c-alt");
    obs_3c_alt.result.exit_code = Some(0);
    let rec_ambiguous = validate_claim_predicate_matches(
        "claim-exit",
        true,
        &exit_claim_false,
        &expected,
        &binding_3c,
        &[
            ObservationRecord::projected(obs_3c_fail),
            ObservationRecord::projected(obs_3c_alt),
        ],
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_ambiguous.verdict.outcome, RuleOutcome::Unknown);
    assert_eq!(
        rec_ambiguous.verdict.reason,
        Some(ReasonCode::EvidenceAmbiguous)
    );
}

#[test]
// Vérifie l'aller-retour strict des fixtures officielles de `.claude/examples/` contre nos DTOs et schémas
fn validate_dossier_examples_roundtrip_against_schemas() {
    let registry = SchemaRegistry::load_default().expect("Chargement des schémas locaux");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let examples_dir = root.join(".claude/examples");

    for file in [
        "discovery-receipt.json",
        "discovery-human.json",
        "discovery-replay.json",
        "discovery-completion.json",
    ] {
        let raw = fs::read_to_string(examples_dir.join(file)).unwrap();
        let val: Value = serde_json::from_str(&raw).unwrap();
        registry
            .validate_value(SchemaKind::DiscoveryReceipt, &val)
            .unwrap_or_else(|e| panic!("Fixture {file} invalide : {e}"));
        let parsed: DiscoveryReceipt = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("Désérialisation {file} échouée : {e}"));
        registry
            .validate_serializable(SchemaKind::DiscoveryReceipt, &parsed)
            .unwrap_or_else(|e| panic!("Round-trip {file} invalide : {e}"));
    }

    let exp_raw = fs::read_to_string(examples_dir.join("expected-test-execution.json")).unwrap();
    let exp: ExpectedTestExecution = serde_json::from_str(&exp_raw).unwrap();
    registry
        .validate_serializable(SchemaKind::ExpectedTestExecution, &exp)
        .unwrap();

    let bind_raw =
        fs::read_to_string(examples_dir.join("expected-execution-binding.json")).unwrap();
    let bind: ExpectedExecutionBinding = serde_json::from_str(&bind_raw).unwrap();
    registry
        .validate_serializable(SchemaKind::ExpectedExecutionBinding, &bind)
        .unwrap();
    assert_eq!(
        exp.exact_ref().unwrap(),
        bind.expectation_ref,
        "La fixture officielle expected-execution-binding.json doit correspondre au digest JCS d'expected-test-execution.json"
    );

    let obs_raw = fs::read_to_string(examples_dir.join("test-execution-observation.json")).unwrap();
    let obs: TestExecutionObservation = serde_json::from_str(&obs_raw).unwrap();
    registry
        .validate_serializable(SchemaKind::TestExecutionObservation, &obs)
        .unwrap();

    let claim_raw = fs::read_to_string(examples_dir.join("contract-claim.json")).unwrap();
    let claim: ContractClaim = serde_json::from_str(&claim_raw).unwrap();
    registry
        .validate_serializable(SchemaKind::ContractClaim, &claim)
        .unwrap();

    // Validation croisée des fixtures nominales par les validateurs Rust
    let obs_records = vec![ObservationRecord::projected(obs)];
    let v_exec = validate_test_execution_matches(
        "tests-required",
        true,
        &exp,
        &bind,
        &obs_records,
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(v_exec.outcome, RuleOutcome::Pass);

    let rec_claim = validate_claim_predicate_matches(
        "claim-truth",
        true,
        &claim,
        &exp,
        &bind,
        &obs_records,
        CaptureConsistency::GitTreeCoherent,
    )
    .unwrap();
    assert_eq!(rec_claim.verdict.outcome, RuleOutcome::Pass);
}
