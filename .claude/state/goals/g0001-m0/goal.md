# g0001-m0 — Jalon M0 : Fondations, Contrats partagés & 5 Spikes de qualification

- Statut : in_progress
- Demandé : 2026-11-17T00:32:48+01:00 par programme `g0000-v1` ; outil : Antigravity
- Branche : goal/g0001-m0 ; base : dev@HEAD
- Options : merge=oui, dry-run=non

## Objectif reformulé
Livrer l'intégralité du jalon M0 : workspace Cargo racine, contrats partagés (`unleash-domain`, `unleash-protocol` avec JCS RFC 8785, DTO IPC v2, 204 `OperationName`, `ts-rs` et 4 schémas JSON M0, `unleash-testkit` avec cutpoints C0–C12 et `ChildCrashHarness` + garde live Haiku/Luna), et exécuter les 5 expériences de qualification M0 (SQLite `unleash-store` T-F002-001..010, OS/daemon/client/CLI `unleash-platform`/`unleash-client`/`unleash-daemon`/`unleash-cli` T-F001-001..010, runtime `unleash-process-host`, tranche verticale UI Tauri 2 + React/TS + Ratatui sur charge AC-F014-011, spike contrats/connaissance, adaptateur `unleash-browser`), inventaire `cargo deny check`, preuves `EV-*`, revues d'experts et rapport `qualification/m0/exit.md`.

## Rattachement
Jalon : M0 (T2–T13 de `m0-kickoff.md`) — Features : F001, F002, F008/F009/F011/F014/F015/F019 (tranches M0) — Exigences : REQ-CORE-001..010, REQ-PERSIST-001..010 — Gates : GATE-ARCH, GATE-RUST, GATE-UI, GATE-AUTH, GATE-AUTOMATION

## Critères de terminé
- [ ] `cargo fmt --all -- --check` → 0 diff
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` → 0 erreur/avertissement
- [ ] `cargo test --workspace --all-features` et `cargo test --manifest-path spikes/m0-04-contracts-knowledge/Cargo.toml` → 100 % passed
- [ ] `cargo doc --workspace --no-deps` (avec `RUSTDOCFLAGS="-D warnings"`) → 0 avertissement
- [ ] `cargo deny check` → passed (`qualification/m0/dependencies.md` généré)
- [ ] `apps/gui-renderer/` (`typecheck`, `lint`, `test`, `build`) + contrôle non-dérive `ts-rs` → passed
- [ ] `~/.local/bin/uv run --with jsonschema python3 tools/design-package/validate.py .` → `0 errors, 0 warnings`
- [ ] 5 rapports de spikes (`qualification/m0/spike-*.md`), fiches `local-models-matrix-prep.md` & `guided-templates-prep.md`, preuves `qualification/evidence/EV-*.json` et `qualification/m0/exit.md` produits, relus par `qa-expert` et `rust-systems-expert`
- [ ] Fusion `--no-ff` dans `dev`, zéro worktree orphelin, `git push origin dev` réussi

## Hors périmètre
- Implémentation complète des jalons M1–M7 ; secrets/comptes externes `EXT-*` (testés contre doubles fidèles sans masquer la limite).

## Plan
| Tâche | Objectif | Écriture exclusive | Dépend de | Rôle | Niveau / modèle | Vérification | Statut |
| --- | --- | --- | --- | --- | --- | --- | --- |
| t01 | Workspace Cargo + `unleash-domain` (IDs, fences, ports, machines à états, proptest) | `Cargo.toml`, `Cargo.lock`, `crates/unleash-domain/**` | — | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | done (`d54d335`, fusion `cf8443f`) |
| t02 | `unleash-protocol` (JCS, IPC v2, 204 ops, ts-rs, 4 schémas M0) | `Cargo.*`, `crates/unleash-protocol/**`, `apps/gui-renderer/src/generated/**`, `.claude/schemas/*` | t01 | systems-implementer | strong / pro | fmt/clippy/test/doc + validate.py | done (`3db5584`, fusion `3c0130a`) |
| t02b | Exclure `.claude/state/` et `.claude/worktrees/` du scan `test_reason_codes.rs` | `crates/unleash-protocol/tests/test_reason_codes.rs` | t02 | implementer | standard / flash (verifier pro) | fmt/clippy/test/doc | in_progress |
| t02c | Déroger l'avis informatif `RUSTSEC-2026-0543` (`chrono 0.4.45`) dans `deny.toml` | `deny.toml` | t02b | implementer | standard / flash (verifier pro) | cargo deny check | in_progress |
| t03 | `unleash-testkit` (horloge/IDs, cutpoints C0–C12, crash harness, LiveTestGuard) | `Cargo.*`, `crates/unleash-testkit/**` | t02 | systems-implementer | strong / pro | fmt/clippy/test/doc | done (`5bf4606`, fusion `0916a4d`) |
| t04 | `unleash-store` + spike SQLite (`T-F002-001..010`, `spike-sqlite.md`) | `crates/unleash-store/**`, `qualification/m0/spike-sqlite.md` | t03 | systems-implementer | strong / pro | fmt/clippy/test/doc | done (`9acd641`, fusion `06db777`) |
| t05a | `unleash-platform` + `unleash-client` (verrou UID, peercred, IPC v2, resync) | `Cargo.*`, `crates/unleash-{platform,client}/**` | t03 | systems-implementer | strong / pro | fmt/clippy/test/doc | done (`85ca324`, fusion `8219688`) |
| t05b | `unleash-daemon` (`unleashd`) + `unleash-cli` (`unleash`) (`T-F001-001..010` + E2E binaire M0) | `Cargo.*`, `deny.toml`, `crates/unleash-{daemon,cli}/**` | t04, t05a | systems-implementer | strong / pro | fmt/clippy/test/doc + E2E | in_progress |
| t06 | `unleash-process-host` + spike runtime (`spike-runtime.md`) | `Cargo.*`, `crates/unleash-process-host/**`, `qualification/m0/spike-runtime.md` | t05a | systems-implementer | strong / pro | fmt/clippy/test/doc | in_progress |
| t07 | Tranche verticale UI (`unleash-tui`, `unleash-gui`, `apps/gui-renderer/`, `spike-ui.md`) | `Cargo.*`, `deny.toml`, `crates/unleash-{tui,gui}/**`, `apps/gui-renderer/**`, `qualification/m0/spike-ui.md` | t05a | ui-implementer | strong / pro (verifier pro) | cargo + pnpm typecheck/lint/test/build | in_progress |
| t08 | Spike contrats & connaissance (`spikes/m0-04-contracts-knowledge`, `spike-contracts-knowledge.md`) | `spikes/m0-04-contracts-knowledge/**`, `qualification/m0/spike-contracts-knowledge.md` | t03 | implementer | standard / flash | cargo test --manifest-path | done (`42702ee`, fusion `07e696b`) |
| t09 | `unleash-browser` + spike navigateur + fiches Q-19/Q-20 | `crates/unleash-browser/**`, `qualification/m0/{spike-browser,local-models-matrix-prep,guided-templates-prep}.md` | t03 | systems-implementer | strong / pro | fmt/clippy/test/doc | done (`cd32188`, fusion `a445cbc`) |
| t10 | Inventaire dépendances (`dependencies.md`, `cargo deny check`), `unleash tui` + E2E transversal M0 | `Cargo.*`, `deny.toml`, `.github/workflows/ci.yml`, `crates/unleash-cli/**`, `crates/unleash-daemon/tests/**`, `qualification/m0/dependencies.md` | t04..t09 | implementer | standard / flash (verifier pro) | cargo deny + tree + E2E M0 | pending |
| t11 | Preuves `EV-*`, `exit.md`, revues `qa-expert` & `rust-systems-expert`, fusion `dev` & push | `qualification/**`, `.claude/state/goals/**` | t10 | root + verifier + experts | strong / pro | Toutes gates sur `dev` + `validate.py` | pending |

## Résultat
En attente d'exécution.
