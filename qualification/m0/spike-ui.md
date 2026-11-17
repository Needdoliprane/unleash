# Qualification M0 — Spike 2 UI (`ADR-03`, `ADR-15`, `AC-F014-011`)

Date d'exécution : 2026-11-17
Branche / worktree : `task/g0001-m0/t07` (`.claude/worktrees/g0001-m0-t07`)
Exigences ciblées : `REQ-UI-001..014`, `REQ-CLI-001..016`, `AC-F014-010`, `AC-F014-011`, `AC-F014-013`, `AC-F014-014`, `T-F015-005..015`

## 1. Verdict M0

**Aucun défaut éliminatoire au sens d'`ADR-03` n'a été observé sur le spike UI M0.**

La tranche verticale complète :
- hôte desktop Tauri 2 (`crates/unleash-gui`) connecté exclusivement via `unleash-client` à un daemon socket Unix privé, avec génération `ts-rs` des DTOs de façade dans `apps/gui-renderer/src/generated-gui/` et flux continu `SubscribeEvents` ;
- renderer React 19 + TypeScript strict (`apps/gui-renderer`) consommant exclusivement les types `ts-rs` générés dans `apps/gui-renderer/src/generated/` (intact) et `apps/gui-renderer/src/generated-gui/` (aucune duplication manuelle de DTO) ;
- TUI Ratatui + Crossterm (`crates/unleash-tui`) à parité d'architecture (`10` espaces, palette clavier, éditeur multiligne Unicode, terminal PTY, virtualisation/pagination, inspecteur accessible en `80×24`, attente passive à la fermeture du flux backend)

tient la charge `AC-F014-011` (`1 000` agents logiques, `100 000` événements paginés par lots de `200`, diff de `10 000` lignes) avec des latences `p95 < 100 ms` sur l'ensemble des interactions mesurées côté GUI et TUI, neutralisation effective des vecteurs HTML/Markdown/SVG/URL/ANSI/OSC et survie du daemon après fermeture de l'hôte UI.

## 2. Environnement et versions vérifiées

- Système : macOS (`darwin` / `arm64`, dépôt sur partage SMB, `CARGO_TARGET_DIR`, `PNPM_STORE_DIR` et staging frontend sur disque APFS local)
- Toolchain Rust : `rustc 1.94.1`, `cargo 1.94.1`
- Toolchain Node / pnpm : `node v25.6.1`, `pnpm 10.34.9`, `vite/7.3.8`
- Crates Rust clés (`Cargo.lock`) :
  - GUI : `tauri 2.12.6`, `tauri-build 2.7.5`, `pulldown-cmark 0.13.6`, `ts-rs 12.0.1`
  - TUI : `ratatui 0.30.2`, `crossterm 0.29.0`, `ropey 1.6.1`, `unicode-segmentation 1.13.3`, `unicode-width 0.2.3`, `vt100 0.16.2`, `tui-term 0.3.4`, `futures-core 0.3.34`
- Paquets frontend (`apps/gui-renderer/pnpm-lock.yaml`) :
  - `react 19.3.1`, `react-dom 19.3.1`, `typescript 5.9.3`, `vite 7.3.8`, `vitest 3.2.7`
  - `@tanstack/react-virtual 3.14.26`, `@codemirror/view 6.43.15`, `@codemirror/state 6.7.6`, `@codemirror/merge 6.12.3`, `@xterm/xterm 5.5.0`, `react-aria-components 1.22.1`
  - `eslint 9.39.5`, `typescript-eslint 8.73.0`, `eslint-plugin-jsx-a11y 6.10.2`, `eslint-plugin-react-hooks 5.2.0`, `prettier 3.10.1`

### Arbitrage technique TUI appliqué selon `ADR-15` (règles 4 et 5)

- `tui-textarea 0.7.0` dépend de `ratatui 0.29` et `crossterm 0.28`, ce qui introduit deux versions majeures de `crossterm` dans l'arbre de dépendances en violation de la règle 4 d'`ADR-15`.
- Conformément à la règle 5 d'`ADR-15`, l'éditeur multiligne a été implémenté en module interne dédié (`unleash_tui::editor::TuiEditor`) sur `ropey 1.6.1` + `unicode-segmentation 1.13.3` + `unicode-width 0.2.3`, et l'intégration PTY sur `vt100 0.16.2` + `tui-term 0.3.4` alignée sur une unique version `crossterm 0.29.0` et `ratatui 0.30.2`.

## 3. Mesures sur la charge `AC-F014-011` (`1 000` agents / `100 000` événements / diff `10 000` lignes)

### 3.1 Renderer GUI (`apps/gui-renderer`, staging APFS propre + Vitest `jsdom`)

Pipeline exécuté sur copie APFS propre (`install --frozen-lockfile`, zéro `node_modules/` dans le worktree SMB) :

- `pnpm --dir "$STAGE" exec vite --version` : `vite/7.3.8 darwin-arm64 node-v25.6.1` (exit `0`).
- `pnpm --dir "$STAGE" run typecheck` (`tsc --noEmit` avec `strict: true`, `noUncheckedIndexedAccess: true`, `exactOptionalPropertyTypes: true`) : exit `0`.
- `pnpm --dir "$STAGE" run lint` (`eslint . --max-warnings 0`, incluant blocage `dangerouslySetInnerHTML`, `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `@typescript-eslint/no-explicit-any` et règles `jsx-a11y`) : exit `0` (`0` erreur, `0` avertissement).
- `pnpm --dir "$STAGE" run format:check` (`prettier --check`) : exit `0`.
- `pnpm --dir "$STAGE" test` (`vitest run`) : `4` fichiers, `7` tests passés (`0` échec, durée `1.27s`).
  - Mesures relevées dans `src/__tests__/load.test.tsx` (`[GUI-LOAD]`, `20` échantillons d'interaction par métrique avec sélecteurs DOM résolus avant chronométrage et transitions réelles de pages `0 -> 99 800 -> 99 600`, seuil d'assertion strict `< 100 ms`) :
    - `initialRenderMs = 68.63 ms`
    - `agentFilter` : `p50 = 3.70 ms`, `p95 = 5.94 ms` (`< 100 ms`)
    - `timelineFilter` : `p50 = 2.27 ms`, `p95 = 4.43 ms` (`< 100 ms`)
    - `pageJump` : `p50 = 2.06 ms`, `p95 = 3.02 ms` (`< 100 ms`)
    - `domAgentRows = 17` nœuds `role="treeitem"` montés dans le DOM sur `1 000` agents logiques (virtualisation `@tanstack/react-virtual`)
    - Projection paginée vérifiée : `200` éléments par page sur `100 000` événements, diff `10 000` lignes original/modifié sans matérialisation complète côté DOM.
- `pnpm --dir "$STAGE" run build` (`vite build`) : exit `0`, construit en `1.10s` (`1 377` modules transformés : `dist/index.html` `0.40 kB`, `dist/assets/index-CwYZx_DI.css` `6.40 kB`, `dist/assets/index-D3Gyivo-.js` `873.63 kB`), copié dans `apps/gui-renderer/dist/` pour l'embarquement `frontendDist` de `unleash-gui`.

### 3.2 TUI Ratatui (`crates/unleash-tui`, `TestBackend`)

Commande : `cargo test --workspace --all-features -- --nocapture` (`5` tests unitaires + `6` tests d'intégration `unleash-tui` passés, `0` échec).

Mesures relevées dans `t_f015_015_ac_f014_011_load_benchmark_1000_agents_100k_events_10k_diff_p95` (`120` cycles clavier/scroll/filtre/pagination en `120×40` et `60` cycles en `80×24` incluant basculement `i` de l'inspecteur) :

| Profil terminal | Initialisation fixture | Latence p50 | Latence p95 | Max observé | Seuil assertion |
|---|---:|---:|---:|---:|---:|
| `120 × 40` | `1.88 ms` | `2.25 ms` | `3.02 ms` | `6.13 ms` | `p95 <= 100 ms` |
| `80 × 24` | `1.88 ms` | `0.61 ms` | `0.93 ms` | `1.51 ms` | `p95 <= 100 ms` |

- L'éditeur interne `TuiEditor` (`editor_supports_unicode_cjk_undo_redo_and_search`) valide les saisies UTF-8 accentuées et CJK par grappèmes, annulation/rétablissement et recherche.
- En profil compact `80×24`, `draw_compact_body` affiche `draw_inspector` lorsque `state.inspector_open` est vrai (couvert par test sur `FocusPane::DiffViewer` dans `t_f015_006_009_013_keyboard_10_spaces_palette_resize_and_accessible_modes`).
- `t_f015_015_no_busy_redraw_loop_when_backend_channel_closes` prouve l'absence de boucle de redraw à 100 % CPU lorsque `run_backend_io_worker` ferme `upd_tx` (`total_draws <= 4`, sans `sleep`).

## 4. Sécurité, isolation, accessibilité et cycle de vie

### 4.1 Hôte Tauri 2 (`crates/unleash-gui`)

`5` tests d'intégration passés en `0.31s` (`tests/gui_security_and_lifecycle.rs`, sans ouverture de fenêtre interactive).

- **CSP & capabilities (`tauri.conf.json` / `capabilities/main.json`)** :
  - CSP configurée : `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; object-src 'none'`.
  - Fenêtre unique autorisée : `"windows": ["main"]`, permissions restreintes à `"core:default"` et `"default"`, aucun plugin `shell`, `fs` ou `http` déclaré.
  - `t_f014_014_tauri_capabilities_csp_markdown_ast_and_actor_spoofing` confirme le rejet de la création d'une seconde fenêtre non autorisée, le rejet de toute commande contenant `actor_ref` ou dépassant `MAX_GUI_PAYLOAD_BYTES` (`1 MiB`).
- **Catalogue fermé d'opérations** :
  - `t_gui_registry_matches_200_exposed_catalog_operations` prouve la bijection exacte avec les `200` opérations exposées du catalogue `unleash_protocol::OperationName` et l'exclusion explicite de `Hello`, `RecordClaim`, `RecordObservation` et `RecordJudgment`.
- **Bindings `ts-rs` et flux continu `SubscribeEvents`** :
  - `t_gui_typescript_bindings_in_sync_with_repository` garantit la synchronisation exacte des `9` DTOs de façade générés par `ts-rs` dans `apps/gui-renderer/src/generated-gui/` (`crates/unleash-gui/src/ts_export.rs`) tout en préservant `apps/gui-renderer/src/generated/` intact (`26` fichiers d'`unleash-protocol`).
  - `t_gui_subscribe_events_streams_continuously` valide la persistance de la tâche continue `EventSynchronizer` sur `Channel<GuiStreamNotification>` (snapshot initial + lots continus suivants + annulation propre sur `Drop`), exposée côté frontend via `subscribeEvents` réordonnant les trames par `index` dans `src/ipc/tauriBridge.ts` (`contract.test.ts`).
- **Parseur Markdown -> AST borné côté Rust (`crates/unleash-gui/src/markdown.rs`)** :
  - `pulldown-cmark 0.13.6` avec plafonds `MAX_MARKDOWN_INPUT_BYTES = 262 144` (`256 KiB`), `MAX_AST_NODES = 4 096`, `MAX_AST_DEPTH = 16`.
  - Tout HTML brut, SVG embarqué, attribut `onload`/`onerror`, et schéma URL autre que `http://`/`https://` (`javascript:`, `data:`, `file:`, `vbscript:`) est bloqué côté Rust avant envoi au renderer React (`blocked_raw_html_count`, `blocked_url_count`).
- **Survie du daemon et identité noyau (`t_f014_013_daemon_survives_gui_close_and_actor_ref_is_kernel_derived`)** :
  - Appel `op_get_overview`, rejet explicite d'un `actor_ref` forgé (`deny_unknown_fields` au sommet de `GuiCommandInput` et `InvalidArgument` dans `payload`) sur la commande mutante enregistrée `op_pause_mission` sans transmission au daemon, puis soumission nominale `op_pause_mission` via `unleash-client` -> socket Unix privé ; fermeture et destruction (`drop`) de la fenêtre `main` et de l'hôte sans émission de `CancelMission`/`StopProcess`/`StopDaemon`, suivies d'une reconnexion CLI retrouvant le `CommandReceipt` par `command_id`.

### 4.2 Renderer React (`src/__tests__/security.test.tsx`, `accessibility.test.tsx`, `contract.test.ts`)

- **Inertie HTML/Markdown/SVG/URL** : aucun nœud `<script>`, `svg[onload]`, `img[onerror]`, ni `<a href="javascript:...">` ou `<a href="data:...">` matérialisé dans le DOM ; rendu exclusivement par éléments React échappés depuis l'AST.
- **Canari secret** : absence vérifiée dans le DOM, les attributs, `localStorage` et `sessionStorage`.
- **xterm.js borné (`TerminalPanel.tsx`)** : `disableStdin: true`, `scrollback: 2000`, `allowProposedApi: false`, `linkHandler.activate` neutralisé (OSC 8 désactivé), aucun addon presse-papiers OSC 52 activé, envoi de commandes typé exclusivement via `SendTerminalInput`.
- **Accessibilité, thèmes & IME (`accessibility.test.tsx`)** : bascule thème clair/sombre (`data-theme="light"` / `"dark"`), navigation dans les `10` onglets ARIA, navigation arbre au clavier (`ArrowDown`), composition IME japonaise (`compositionstart` -> `change` sans filtrage prématuré -> `compositionend`).
- **Pont Tauri (`contract.test.ts`)** : exclusion des `4` opérations internes (`Hello`, `RecordClaim`, `RecordObservation`, `RecordJudgment`) et réordonnancement des trames `Channel` sur `subscribeEvents`.

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
| `cargo test --workspace --all-features` (borne Python `300s`) | Exit `0` — `143` tests passés (`0` échec, `0` ignoré) |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Exit `0` (`9` crates documentées, `0` avertissement) |
| `cargo deny check` | Exit `0` (`advisories ok, bans ok, licenses ok, sources ok`) |
| `git diff --exit-code apps/gui-renderer/src/generated` | Exit `0` (`26` bindings `ts-rs` d'`unleash-protocol` strictement intacts) |
| `uv run --with jsonschema python3 tools/design-package/validate.py .` | Exit `0` (`== 0 errors, 0 warnings`) |
| Frontend APFS (`install --frozen-lockfile`, `vite --version`, `typecheck`, `lint`, `format:check`, `test`, `build`) | Exit `0` (`4` fichiers / `7` tests passés, `p95 < 100 ms`, zéro `node_modules/` dans le worktree) |

Dérogations motivées consignées sous `[advisories].ignore` dans `deny.toml` :
1. `RUSTSEC-2024-0370` : `proc-macro-error 1.0.4` (`unmaintained`, proc-macro de compilation sans CVE) tiré transitivement par `glib-macros 0.18.5` / `gtk3-macros 0.18.2` <- `gtk 0.18.2` <- `tao`/`wry`/`tauri 2.12.6` sans mise à niveau disponible amont.
2. `RUSTSEC-2026-0543` : `chrono 0.4.45` (avis informatif `unmaintained` sans CVE ni correctif amont) requis par `ts-rs 12.0.1` (`chrono-impl`) et le formatage RFC 3339 d'`unleash-protocol`/`unleash-store` ; réévaluation prévue lors du support `jiff`/`time` par `ts-rs`.

## 6. Limites connues du spike M0 (hors périmètre headless)

Les mesures et preuves ci-dessus ont été collectées en environnement headless (`Vitest` + `jsdom`, `tauri::test::MockRuntime`, `ratatui::backend::TestBackend`) sans modification des autres tâches parallèles (`unleash-daemon`, `unleash-cli`, `unleash-process-host`). Ne sont donc pas encore testés en M0 :
1. Rendu réel sous WebView OS (`Wry` / WebKit macOS / WebKitGTK Linux) et profilage GPU/WebDriver natif.
2. Tests manuels ou assistés avec lecteurs d'écran réels (`VoiceOver` / `Orca`) et saisie IME système native sur WebView/TTY réel.
3. Rattachement CLI `unleash tui` et wiring complet des `200` flux vers `unleash-daemon` réel (planifiés hors `t07` lors de l'intégration M0+).
