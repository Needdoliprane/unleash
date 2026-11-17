# Journal g0001-m0

- 2026-11-16T23:50:00Z | cadrage | Ouverture du sous-goal M0 (`g0001-m0`) rattaché au programme `g0000-v1` ; Vague 1 sérielle (`t01` → `t02` → `t03`) puis Vague 2 (`t04`..`t09`) et Vague 3 (`t10`..`t11`).
- 2026-11-16T23:55:20Z | délégation | t01 → systems-implementer, niveau strong (`pro`), worktree `.claude/worktrees/g0001-m0-t01` (branche `task/g0001-m0/t01`) : workspace Cargo + `unleash-domain`.
- 2026-11-17T00:13:35Z | compte rendu | t01 commit `6a6aa00` ; 17 tests (`11 state_transitions + 6 proptests`) + fmt/clippy/doc/cargo-deny verts.
- 2026-11-17T00:13:40Z | vérification | t01 → verifier indépendant, niveau strong (`pro`), worktree `.claude/worktrees/g0001-m0-t01`.
- 2026-11-17T00:20:20Z | vérification | t01 verifier pro : refusé (`ids.rs:28-30,205-207` `Uuid::new_v4()` interdit dans `unleash-domain` ; `crates/unleash-domain/Cargo.toml:21` dépendance `chrono` inutilisée tirant horloge OS). Renvoyé à `systems-implementer`.
- 2026-11-17T00:23:00Z | compte rendu | t01 correctif commit `d54d335` ; `new_v4()` et `chrono` supprimés, `cargo tree -p unleash-domain` propre, 17 tests + gates verts. Nouvelle passe `verifier`.
- 2026-11-17T00:25:00Z | vérification | t01 verifier pro : accepté (`d54d335`, 17/17 tests, fmt/clippy/doc/deny verts, `cargo tree -p unleash-domain` sans `getrandom`/`chrono`).
- 2026-11-17T00:27:15Z | intégration | t01 fusionnée dans `goal/g0001-m0` (`cf8443f`), worktree `g0001-m0-t01` supprimé, gates vertes.
- 2026-11-17T00:27:20Z | délégation | t02 → systems-implementer, niveau strong (`pro`), worktree `.claude/worktrees/g0001-m0-t02` (branche `task/g0001-m0/t02`) : `unleash-protocol`, JCS RFC 8785, 204 ops, `ts-rs`, 4 schémas JSON M0.
