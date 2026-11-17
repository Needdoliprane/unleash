# Qualification M0 — Spike 2 UI (`ADR-03`, `ADR-15`, `AC-F014-011`)

Date d'exécution : 2026-11-17
Branche / worktree : `task/g0001-m0/t07` (`.claude/worktrees/g0001-m0-t07`)
Exigences ciblées : `REQ-UI-001..014`, `REQ-CLI-001..016`, `AC-F014-010`, `AC-F014-011`, `AC-F014-013`, `AC-F014-014`, `T-F015-005..015`

## 1. Verdict M0

**Aucun défaut éliminatoire au sens d'`ADR-03` n'a été observé sur le spike UI M0.**

La tranche verticale complète :
- hôte desktop Tauri 2 (`crates/unleash-gui`) connecté exclusivement via `unleash-client` à un daemon socket Unix privé ;
- renderer React 19 + TypeScript strict (`apps/gui-renderer`) consommant exclusivement les types `ts-rs` générés dans `apps/gui-renderer/src/generated/` ;
- TUI Ratatui + Crossterm (`crates/unleash-tui`) à parité d'architecture (`10` espaces, palette clavier, éditeur multiligne Unicode, terminal PTY, virtualisation/pagination)

tient la charge `AC-F014-011` (`1 000` agents logiques, `100 000` événements paginés par lots de `200`, diff de `10 000` lignes) sous les budgets fixés dans les bancs de test headless, avec neutralisation effective des vecteurs HTML/Markdown/SVG/URL/ANSI/OSC et survie du daemon après fermeture de l'hôte UI.

## 2. Environnement et versions vérifiées

- Système : macOS (`darwin` / `arm64`, dépôt sur partage SMB, `CARGO_TARGET_DIR` et `PNPM_STORE_DIR` sur disque APFS local)
- Toolchain Rust : `rustc 1.94.1 (e408947bf 2026-03-25)`, `cargo 1.94.1 (29ea6fb6a 2026-03-24)`
- Toolchain Node / pnpm : `node v25.6.1`, `pnpm 10.34.9`
- Crates Rust clés (`Cargo.lock`) :
  - GUI : `tauri 2.12.6`, `tauri-build 2.7.5`, `pulldown-cmark 0.13.6`
  - TUI : `ratatui 0.30.2`, `crossterm 0.29.0`, `ropey 1.6.1`, `unicode-segmentation 1.13.3`, `unicode-width 0.2.3`, `vt100 0.16.2`, `tui-term 0.3.4`
- Paquets frontend (`apps/gui-renderer/pnpm-lock.yaml`) :
  - `react 19.3.1`, `react-dom 19.3.1`, `typescript 5.9.3`, `vite 7.3.8`, `vitest 3.2.7`
  - `@tanstack/react-virtual 3.14.26`, `@codemirror/view 6.43.15`, `@codemirror/state 6.7.6`, `@codemirror/merge 6.12.3`, `@xterm/xterm 5.5.0`, `react-aria-components 1.22.1`
  - `eslint 9.39.5`, `typescript-eslint 8.73.0`, `eslint-plugin-jsx-a11y 6.10.2`, `eslint-plugin-react-hooks 5.2.0`, `prettier 3.10.1`

### Arbitrage technique TUI appliqué selon `ADR-15` (règles 4 et 5)

- `tui-textarea 0.7.0` dépend de `ratatui 0.29` et `crossterm 0.28`, ce qui introduit deux versions majeures de `crossterm` dans l'arbre de dépendances en violation de la règle 4 d'`ADR-15`.
- Conformément à la règle 5 d'`ADR-15`, l'éditeur multiligne a été implémenté en module interne dédié (`unleash_tui::editor::TuiEditor`) sur `ropey 1.6.1` + `unicode-segmentation 1.13.3` + `unicode-width 0.2.3`, et l'intégration PTY sur `vt100 0.16.2` + `tui-term 0.3.4` alignée sur une unique version `crossterm 0.29.0` et `ratatui 0.30.2`.

## 3. Mesures sur la charge `AC-F014-011` (`1 000` agents / `100 000` événements / diff `10 000` lignes)

### 3.1 Renderer GUI (`apps/gui-renderer`, Vitest + `jsdom`)

Commande exécutée sur la copie APFS synchronisée à l'identique (`diff -rq -x node_modules -x .vite -x coverage` = code `0`) :

- `pnpm ... run typecheck` (`tsc --noEmit` avec `strict: true`, `noUncheckedIndexedAccess: true`, `exactOptionalPropertyTypes: true`) : `0` erreur.
- `pnpm ... run lint` (`eslint . --max-warnings 0`, incluant blocage `dangerouslySetInnerHTML`, `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `@typescript-eslint/no-explicit-any` et règles `jsx-a11y`) : `0` erreur, `0` avertissement.
- `pnpm ... run format:check` (`prettier --check`) : conforme.
- `pnpm ... test` (`vitest run`) : `4` fichiers, `6` tests passés (`0` échec, durée `1.50s`).
  - Mesures relevées dans `src/__tests__/load.test.tsx` (`[GUI-LOAD]`) :
    - `initialRenderMs = 68.41 ms`
    - `agentFilterMs = 5.27 ms` (budget test `< 150 ms`)
    - `timelineFilterMs = 4.24 ms` (budget test `< 150 ms`)
    - `pageJumpMs = 122.34 ms` (saut vers la fin à l'index `100 000`, budget test `< 150 ms`)
    - `domAgentRows = 17` nœuds `role="treeitem"` montés dans le DOM sur `1 000` agents logiques (virtualisation `@tanstack/react-virtual`)
    - Projection paginée vérifiée : `200` éléments par page sur `100 000` événements, diff `10 000` lignes original/modifié sans matérialisation complète côté DOM.
- `pnpm ... run build` (`vite build`) : construit en `1.18s` (`dist/index.html` `0.40 kB`, `dist/assets/index-CwYZx_DI.css` `6.40 kB`, `dist/assets/index-wz5Mt9xR.js` `872.85 kB`), copié dans `apps/gui-renderer/dist` pour l'embarquement `frontendDist` de `unleash-gui`.

### 3.2 TUI Ratatui (`crates/unleash-tui`, `TestBackend`)

Commande : `cargo test -p unleash-tui -- --nocapture` (`5` tests unitaires + `5` tests d'intégration passés, `0` échec).

Mesures relevées dans `t_f015_015_ac_f014_011_load_benchmark_1000_agents_100k_events_10k_diff_p95` (`120` cycles clavier/scroll/filtre/pagination en `120×40` et `60` cycles en `80×24`) :

| Profil terminal | Initialisation fixture | Latence p50 | Latence p95 | Max observé | Seuil assertion |
|---|---:|---:|---:|---:|---:|
| `120 × 40` | `2.82 ms` | `2.26 ms` | `2.81 ms` | `5.12 ms` | `p95 <= 100 ms` |
| `80 × 24` | `2.82 ms` | `0.81 ms` | `0.94 ms` | `1.66 ms` | `p95 <= 100 ms` |

L'éditeur interne `TuiEditor` (`editor_supports_unicode_cjk_undo_redo_and_search`) valide les saisies UTF-8 accentuées et CJK par grappèmes, annulation/rétablissement et recherche.

## 4. Sécurité, isolation, accessibilité et cycle de vie

### 4.1 Hôte Tauri 2 (`crates/unleash-gui`)

Commande : `cargo test -p unleash-gui -- --nocapture` (`3` tests d'intégration passés en `0.04s`, sans ouverture de fenêtre interactive).

- **CSP & capabilities (`tauri.conf.json` / `capabilities/main.json`)** :
  - CSP configurée : `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; object-src 'none'`.
  - Fenêtre unique autorisée : `"windows": ["main"]`, permissions restreintes à `"core:default"` et `"default"`, aucun plugin `shell`, `fs` ou `http` déclaré.
  - `t_f014_014_tauri_capabilities_csp_markdown_ast_and_actor_spoofing` confirme le rejet de la création d'une seconde fenêtre non autorisée, le rejet de toute commande contenant `actor_ref` ou dépassant `MAX_GUI_PAYLOAD_BYTES` (`1 MiB`).
- **Catalogue fermé d'opérations** :
  - `t_gui_registry_matches_200_exposed_catalog_operations` prouve la bijection exacte avec les `200` opérations exposées du catalogue `unleash_protocol::OperationName` et l'exclusion explicite de `Hello`, `RecordClaim`, `RecordObservation` et `RecordJudgment`.
- **Parseur Markdown -> AST borné côté Rust (`crates/unleash-gui/src/markdown.rs`)** :
  - `pulldown-cmark 0.13.6` avec plafonds `MAX_MARKDOWN_INPUT_BYTES = 262 144` (`256 KiB`), `MAX_AST_NODES = 4 096`, `MAX_AST_DEPTH = 16`.
  - Tout HTML brut, SVG embarqué, attribut `onload`/`onerror`, et schéma URL autre que `http://`/`https://` (`javascript:`, `data:`, `file:`, `vbscript:`) est bloqué côté Rust avant envoi au renderer React (`blocked_raw_html_count`, `blocked_url_count`).
- **Survie du daemon et identité noyau (`t_f014_013_daemon_survives_gui_close_and_actor_ref_is_kernel_derived`)** :
  - Appel `op_get_overview` et commande `op_pause_mission` acheminés de Tauri (`MockRuntime`) -> `unleash-client` -> socket Unix privé ; fermeture et destruction (`drop`) de la fenêtre `main` et de l'hôte sans émission de `CancelMission`/`StopProcess`/`StopDaemon`, suivies d'une reconnexion CLI retrouvant le `CommandReceipt` par `command_id`.

### 4.2 Renderer React (`src/__tests__/security.test.tsx`, `accessibility.test.tsx`)

- **Inertie HTML/Markdown/SVG/URL** : aucun nœud `<script>`, `svg[onload]`, `img[onerror]`, ni `<a href="javascript:...">` ou `<a href="data:...">` matérialisé dans le DOM ; rendu exclusivement par éléments React échappés depuis l'AST.
- **Canari secret** : absence vérifiée dans le DOM, les attributs, `localStorage` et `sessionStorage`.
- **xterm.js borné (`TerminalPanel.tsx`)** : `disableStdin: true`, `scrollback: 2000`, `allowProposedApi: false`, `linkHandler.activate` neutralisé (OSC 8 désactivé), aucun addon presse-papiers OSC 52 activé, envoi de commandes typé exclusivement via `SendTerminalInput`.
- **Accessibilité, thèmes & IME (`accessibility.test.tsx`)** : bascule thème clair/sombre (`data-theme="light"` / `"dark"`), navigation dans les `10` onglets ARIA, navigation arbre au clavier (`ArrowDown`), composition IME japonaise (`compositionstart` -> `change` sans filtrage prématuré -> `compositionend`).

### 4.3 TUI (`crates/unleash-tui`)

- **Neutralisation ANSI/OSC (`t_f015_007_neutralizes_hostile_ansi_osc_and_isolates_pty_keys`)** : suppression des séquences `CSI`, `OSC 8` et `OSC 52` dans les notifications et libellés (`\x1b` et `\x07` absents du tampon final) et isolation clavier lorsque le focus est attaché au PTY (`'q'` transmis au PTY sans quitter la TUI, détachement par `Ctrl+]`).
- **Restauration terminal RAII & hook de panique (`t_f015_014_panic_hook_triggers_idempotent_restore`, `non_tty_guard_refuses_raw_mode_cleanly`)** : refus propre du raw mode sur flux non-TTY et restauration idempotente vérifiée sous `catch_unwind`.
- **Modes d'accessibilité** : support vérifié de `NO_COLOR`, `ascii_only` (`TERM=dumb`) et `linear_reader_mode` (`t_f015_006_009_013_keyboard_10_spaces_palette_resize_and_accessible_modes`), plus connexion client IPC indépendante (`t_f015_005_tui_client_connects_and_quits_without_stopping_daemon`).

## 5. Gates du dépôt et état `cargo deny`

Toutes les vérifications suivantes ont été exécutées après la dernière édition de code Rust et TypeScript :

| Vérification | Résultat observé |
|---|---|
| `cargo fmt --all -- --check` | Exit `0` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Exit `0` |
| `cargo test --workspace --all-features` (borne Python `240s`) | Exit `0` — tous les tests unitaires et d'intégration du workspace passent |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit `0` (`9` crates documentés, `0` avertissement) |
| `git diff --exit-code apps/gui-renderer/src/generated` | Exit `0` (bindings `ts-rs` strictement intacts) |
| `python3 tools/design-package/validate.py .` | `== 0 errors, 1 warnings` (jsonschema Python absent du poste) |
| `cargo deny check bans licenses sources` | Exit `0` (`bans ok, licenses ok, sources ok`) |
| `cargo deny check advisories` | Exit `1` — **unique alerte** : `RUSTSEC-2024-0370` (`proc-macro-error 1.0.4` unmaintained) via `glib-macros` / `gtk` -> `wry` / `tauri 2.12.6` (`Solution: No safe upgrade is available!`) |

Conformément au cadrage M0, `deny.toml` n'a pas été modifié dans la tâche `t07` : `RUSTSEC-2024-0370` (dépendance proc-macro de compilation amont sans correctif disponible dans l'écosystème `gtk`/`tauri 2`) est consigné ici pour décision explicite en `t12` (`ignore` documenté dans `deny.toml` ou alternative retenue).

## 6. Limites connues du spike M0 (hors périmètre headless)

Les mesures et preuves ci-dessus ont été collectées en environnement headless (`Vitest` + `jsdom`, `tauri::test::MockRuntime`, `ratatui::backend::TestBackend`) sans modification des autres tâches parallèles (`unleash-daemon`, `unleash-cli`, `unleash-process-host`). Ne sont donc pas encore testés en M0 :
1. Rendu réel sous WebView OS (`Wry` / WebKit macOS / WebKitGTK Linux) et profilage GPU/WebDriver natif.
2. Tests manuels ou assistés avec lecteurs d'écran réels (`VoiceOver` / `Orca`) et saisie IME système native sur WebView/TTY réel.
3. Rattachement CLI `unleash tui` et wiring complet des `200` flux vers `unleash-daemon` réel (planifiés hors `t07` lors de l'intégration M0+).
