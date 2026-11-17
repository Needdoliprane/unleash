# Qualification M0 — Spike 4 : Contrats observables et connaissance multilangage

- **Jalon / Tâche** : `g0001-m0` / `t08` (branche `task/g0001-m0/t08`)
- **Date d'exécution** : `2026-11-17`
- **Crate du spike** : [`spikes/m0-04-contracts-knowledge/`](../../spikes/m0-04-contracts-knowledge/Cargo.toml) (autonome, exclue du workspace Cargo racine)
- **Exigences éclairées** : `REQ-CONTRACT-004`, `REQ-CONTRACT-005`, `REQ-CONTRACT-006`, `REQ-CONTRACT-007`, `REQ-KNOW-001`, `REQ-KNOW-002`, `REQ-KNOW-003`
- **Statut produit** : Les campagnes d'acceptation `EV-F008-*` et `EV-F009-*` du dossier `.claude/` restent `not_run` ; ce rapport consigne uniquement la qualification expérimentale M0 du spike.

---

## 1. Hypothèses vérifiées

1. **Dégradation multilangage sans vérité inventée (`REQ-KNOW-003`, `REQ-KNOW-001`)** :
   - Lorsqu'un analyseur externe (par exemple `pyright`, `typescript-language-server` ou `rust-analyzer` absent/non qualifié) manque ou échoue à se qualifier, la sonde émet `ProfileAvailability::Unavailable` avec `ReasonCode::ProfileUnavailable` (`profile_unavailable`) et une dégradation lexicale explicite (`RelationConfidence::Lexical`), **sans jamais émettre de relation `RelationConfidence::SemanticResolved` inventée**.
   - Le reçu généré (`DiscoveryReceipt`, `unleash.discovery.v1`) est validé hors ligne contre `.claude/schemas/discovery-receipt.schema.json`.
2. **Capture dirty et détection de mutation concurrente (`REQ-KNOW-002`, `REQ-CONTRACT-005`, `REQ-CONTRACT-006`)** :
   - Une mutation intervenant entre deux passes de capture d'un worktree Git dirty (injectée ici par hook déterministe sans `sleep`) fait diverger le digest canonique RFC 8785 + SHA-256 de l'`OverlayManifest`, classe la capture en `CaptureConsistency::Inconsistent` (`ReasonCode::SnapshotInconsistent`), et force `git_snapshot_matches` ainsi que `test_execution_matches` à retourner `RuleOutcome::Unknown` (et verdict global `VerdictStatus::Unknown` sous `OnUnknownPolicy::Block`), **jamais `Pass`**.
   - Une capture séquentielle dirty sans mutation ni mécanisme OS atomique qualifié demeure `CaptureConsistency::Unverified` et retourne également `RuleOutcome::Unknown` ; seule une capture sur arbre Git committé propre (`CaptureConsistency::GitTreeCoherent`) autorise `Pass`.
   - Un dépôt Git sans commit initial (`unborn`) conserve `base_commit_oid = None` sans fabriquer de faux OID.
3. **Faux rapports de tests et claims mensongères (`REQ-CONTRACT-004`, `REQ-CONTRACT-005`, `REQ-CONTRACT-006`, `REQ-CONTRACT-007`)** :
   - La sortie texte brute de `cargo test` sans JUnit XML est classée `ParseState::Incomplete` (`cases: None`) par `parse_cargo_test_text` et conduit à `RuleOutcome::Unknown` (`VerdictStatus::Unknown`).
   - Un rapport JUnit XML valide et vert mais issu d'un commit/snapshot antérieur produit `RuleOutcome::Fail` avec `ReasonCode::EvidenceSnapshotMismatch`.
   - Une `ContractClaim` affirmant `test.expectation_satisfied = true` (ou `process.exit_code = 0`) tout en omettant l'observation d'échec dans `observation_ids` est confrontée à toutes les observations autoritaires liées à `action_id` dans `SealedActionJournal` et échoue avec `ReasonCode::ClaimContradicted`, **en conservant intactes la claim et l'observation contredisante** dans `ClaimReconciliationRecord`.
   - Tous les DTOs émis (`DiscoveryReceipt`, `ExpectedTestExecution`, `ExpectedExecutionBinding`, `TestExecutionObservation`, `ContractClaim`) sont sérialisés avec `#[serde(deny_unknown_fields)]` et validés hors ligne contre les cinq JSON Schemas 2020-12 de `.claude/schemas/` via `jsonschema` + `referencing::Registry` (`should_validate_formats(true)`).

---

## 2. Environnement et analyseurs observés sur le poste

| Outil / Composant | Observation réelle sur le poste | Statut dans le spike |
|---|---|---|
| `rustc` / `cargo` | `rustc 1.94.1 (e408947bf 2026-03-25)` / `cargo 1.94.1 (29ea6fb6a 2026-03-24)` | Actif |
| `git` | `git version 2.53.0` | Actif sur dépôts temporaires `/tmp` (`unleash_testkit::local_test_dir`) |
| `rust-analyzer` | Proxy rustup présent mais `rust-analyzer --version` retourne `error: Unknown binary 'rust-analyzer' in official toolchain '1.94.1-aarch64-apple-darwin'` | `ProfileAvailability::Unavailable` (`ProfileUnavailable`) |
| `pyright` | Absent (`command not found`) | `ProfileAvailability::Unavailable` (`ProfileUnavailable`) |
| `typescript-language-server` | Absent (`command not found`) | `ProfileAvailability::Unavailable` (`ProfileUnavailable`) |
| `cargo-nextest` | Absent (`command not found`) | Rapports JUnit XML fournis par fixtures déterministes |
| `python3` / `node` | `Python 3.14.6` / `node v25.6.1` | Présents sur le poste (non requis à l'exécution du spike) |

---

## 3. Matrice des cas exécutés (positifs, négatifs, `unknown`)

| Test / Scénario | Cas testé | Résultat attendu & observé | Objets validés par JSON Schema |
|---|---|---|---|
| `scenario_1_multilanguage_patch_profile_unavailable_and_valid_receipt` | Dépôt `/tmp` Rust + Python + TypeScript avec analyseurs absents (`isolated` et `host_toolchain`) | `pyright`, `rust-analyzer`, `typescript-language-server` → `ProfileAvailability::Unavailable` (`ProfileUnavailable`) ; 0 relation `SemanticResolved` ; symboles & relations `Lexical` extraits ; `git_tree_coherent` | `DiscoveryReceipt` positif valide ; rejets négatifs (`coverage_ref` manquant, `target_resource_id` non nul en `create/pre_creation`, `recorded_at = "yesterday"`, `repo_id` sur `file_snapshot`) |
| `scenario_2_dirty_worktree_mutation_during_capture_produces_unknown` | Mutation injectée entre passe 1 et passe 2 d'un worktree dirty | `code_state = Dirty`, `capture_consistency = Inconsistent`, `ReasonCode::SnapshotInconsistent` ; `git_snapshot_matches` → `Unknown` ; `test_execution_matches` → `Unknown` ; verdict global → `Unknown` | `ExpectedTestExecution`, `ExpectedExecutionBinding`, `TestExecutionObservation` |
| `scenario_2_dirty_worktree_mutation_during_capture_produces_unknown` | Capture séquentielle dirty sans mutation vs commit propre | Sans mutation sur dirty → `Unverified` → `Unknown` ; après `git commit` propre → `GitTreeCoherent` → `Pass` | `TypedSourceRef` / `SnapshotRef` |
| `knowledge::tests::unborn_git_repo_yields_none_oid_and_unverified_consistency` | Dépôt Git initialisé sans aucun commit (`unborn`) | `code_state = Unborn`, `base_commit_oid = None`, `base_tree_oid = None`, `capture_consistency = Unverified` | — |
| `scenario_3a_cargo_test_text_without_junit_produces_unknown` | Sortie texte brute `cargo test` (`3 passed; 0 failed`) sans JUnit XML | `parse_cargo_test_text` → `ParseState::Incomplete` (`cases: None`) ; `test_execution_matches` → `Unknown` (`EvidenceMissing`) ; verdict global → `Unknown` | `ExpectedTestExecution`, `ExpectedExecutionBinding`, `TestExecutionObservation` |
| `scenario_3b_stale_junit_xml_produces_snapshot_mismatch` | JUnit XML vert (`2/2 passed`) mais lié à un snapshot antérieur (`git-snap-old-001` ≠ `git-snap-current-002`) ; XML tronqué | Snapshot ancien → `RuleOutcome::Fail` (`ReasonCode::EvidenceSnapshotMismatch`) ; XML tronqué → `ParseState::Incomplete` | `ExpectedExecutionBinding`, `TestExecutionObservation` |
| `scenario_3c_lying_claim_contradicted_preserving_both_objects` | Claim `test.expectation_satisfied = true` avec `observation_ids: []` face à observation échouée (`exit_code = 1`, `failed = 1`) | `claim_predicate_matches` → `Fail` (`ClaimContradicted`) ; `ClaimReconciliationRecord` conserve `claim` ET `obs_3c_fail` ; paraphrase de `statement` sans effet | `ExpectedExecutionBinding`, `TestExecutionObservation`, `ContractClaim` |
| `scenario_3c_lying_claim_contradicted_preserving_both_objects` | Variations F008 : claim honnête `false`, claim qualitatif, mauvais `expectation_ref`, observations concurrentes, `process.exit_code` | Honnête `false` → `claim_predicate_matches` `Pass` mais `test_execution_matches` `Fail` (global `Fail`) ; qualitatif → `Unknown` ; mauvais `expectation_ref` → `EvidenceExecutionMismatch` ; 2 observations incompatibles → `Unknown` (`EvidenceAmbiguous`) | `ContractClaim`, `TestExecutionObservation` |
| `contracts::tests::junit_parser_aggregates_multiple_suites_and_rejects_inconsistent_counts` | JUnit multi-suites et compteurs incohérents (`failed > tests`) | Agrégation exacte multi-suites (`total=5, passed=2, failed=2, skipped=1`) ; compteurs impossibles → `ParseState::Incomplete` | — |
| `contracts::tests::informative_failing_rule_does_not_fail_global_verdict` | Règle obligatoire `Pass` + règle informative `Fail` | `VerdictStatus::Pass` avec les 2 règles conservées visibles | — |
| `validate_dossier_examples_roundtrip_against_schemas` | Aller-retour strict des 8 fixtures officielles `.claude/examples/*.json` | 4 fixtures `discovery-*.json` + `expected-test-execution.json` + `expected-execution-binding.json` (y compris vérification d'égalité du digest RFC 8785) + `test-execution-observation.json` + `contract-claim.json` passent schémas et validateurs | Les 5 JSON Schemas 2020-12 sous URN locaux |

---

## 4. Commandes de vérification et sorties réelles observées

Environnement shell appliqué :

```sh
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/Library/Caches/unleash/target/g0001-m0-t08"
export PNPM_STORE_DIR="$HOME/Library/Caches/unleash/pnpm-store"
```

### Formatage (`cargo fmt`)

```sh
cargo fmt --manifest-path spikes/m0-04-contracts-knowledge/Cargo.toml -- --check
```

Sortie : code de sortie `0` (aucun écart de formatage).

### Analyse statique (`cargo clippy`)

```sh
cargo clippy --manifest-path spikes/m0-04-contracts-knowledge/Cargo.toml --all-targets -- -D warnings
```

Sortie observée :

```text
    Checking spike-m0-04-contracts-knowledge v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t08/spikes/m0-04-contracts-knowledge)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.60s
```

Code de sortie `0` (aucun avertissement).

### Tests (`cargo test`)

```sh
cargo test --manifest-path spikes/m0-04-contracts-knowledge/Cargo.toml
```

Sortie observée :

```text
     Running unittests src/lib.rs (/Users/louis/Library/Caches/unleash/target/g0001-m0-t08/debug/deps/spike_m0_04_contracts_knowledge-c60898d53d41c3e2)

running 3 tests
test contracts::tests::informative_failing_rule_does_not_fail_global_verdict ... ok
test contracts::tests::junit_parser_aggregates_multiple_suites_and_rejects_inconsistent_counts ... ok
test knowledge::tests::unborn_git_repo_yields_none_oid_and_unverified_consistency ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

     Running tests/spike_scenarios.rs (/Users/louis/Library/Caches/unleash/target/g0001-m0-t08/debug/deps/spike_scenarios-7174f078ff439ebc)

running 6 tests
test scenario_3b_stale_junit_xml_produces_snapshot_mismatch ... ok
test scenario_3a_cargo_test_text_without_junit_produces_unknown ... ok
test scenario_3c_lying_claim_contradicted_preserving_both_objects ... ok
test scenario_2_dirty_worktree_mutation_during_capture_produces_unknown ... ok
test validate_dossier_examples_roundtrip_against_schemas ... ok
test scenario_1_multilanguage_patch_profile_unavailable_and_valid_receipt ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.68s
```

### Documentation (`cargo doc`)

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path spikes/m0-04-contracts-knowledge/Cargo.toml --no-deps
```

Code de sortie `0` (aucun avertissement).

---

## 5. Limites connues du spike M0

1. **Absence de session LSP `initialize` et de `tree-sitter` natif** : La détection d'analyseurs vérifie l'existence, l'empreinte SHA-256 et `--version` des binaires, mais n'ouvre pas encore de transport JSON-RPC LSP (`rust-analyzer`, `pyright`, `typescript-language-server`) ni de parseur `tree-sitter`. Le statut `ProfileAvailability::Ready` reste donc non qualifié à ce stade et aucun bord `SemanticResolved` ni `Syntactic` n'est émis.
2. **Parseur JUnit XML minimal et absence de `cargo-nextest` sur le poste** : `parse_junit_xml` extrait les compteurs de `<testsuite>`/`<testsuites>` sur des rapports synthétiques mais n'a pas été confronté à une exécution réelle de `cargo-nextest` (absent du poste) ni à des rapports `go test -json` (`parser.go-test-json`).
3. **Persistance en mémoire et hook synchrone** : `SealedActionJournal` valide les invariants de scellement et de réconciliation en mémoire uniquement (sans persistance SQLite `unleash-store`), et la mutation concurrente du worktree dirty est déclenchée via un hook déterministe entre deux scans plutôt que par un crash/race inter-processus réel.
4. **Périmètre des schémas et validateurs** : Le spike valide les cinq schémas demandés et implémente `git_snapshot_matches`, `test_execution_matches` et `claim_predicate_matches` ; les autres validateurs du catalogue v2 (dont `discovery_receipt_present`, `integration_observed`, `path_within_scope`) relèvent des crates produit ultérieures.
