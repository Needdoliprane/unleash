# Inventaire des dépendances, licences et audit `cargo deny` — Jalon M0 (T12 / `t10`)

**Date :** 2026-11-17  
**Worktree / branche :** `.claude/worktrees/g0001-m0-t10` (`task/g0001-m0/t10`)  
**Références :** `ADR-14` (`Q-06`), `ADR-15`, `deny.toml`, `.claude/dev/m0-kickoff.md` (`T12`)

---

## 1. Environnement d'exécution et commandes reproductibles

| Outil / Plateforme | Version / Valeur observée |
|---|---|
| Système hôte | macOS `27.2.1` (`BuildVersion 26B131`), architecture `arm64` |
| `rustc` / `cargo` | `rustc 1.94.1 (e408947bf 2026-03-25)` / `cargo 1.94.1 (29ea6fb6a 2026-03-24)` |
| `cargo-deny` | `cargo-deny 0.21.0` |
| `node` / `pnpm` | `v25.6.1` / `10.34.9` (`@unleash/gui-renderer`, staging APFS local) |
| Commit de base (`t10`) | `738e910` (complété par `t10` : `unleash tui` dans `unleash-cli` et dev-deps E2E dans `unleash-daemon`) |

Commandes exactes exécutées pour produire et vérifier cet inventaire :

```sh
cargo deny --manifest-path Cargo.toml check
cargo deny --manifest-path Cargo.toml list --layout crate --format json
cargo tree --manifest-path Cargo.toml --workspace --edges normal --format "{p} ({l})"
cargo metadata --manifest-path Cargo.toml --format-version 1
cargo run -p unleash-store --example spike_sqlite
otool -L "$CARGO_TARGET_DIR/debug/unleashd" "$CARGO_TARGET_DIR/debug/unleash" "$CARGO_TARGET_DIR/debug/unleash-gui" "$CARGO_TARGET_DIR/debug/unleash-process-host"
pnpm --dir "$STAGE" licenses list --json
pnpm --dir "$STAGE" licenses list --prod --json
```

---

## 2. Crates du workspace Cargo (`12` crates internes `Apache-2.0`)

Toutes les crates du workspace héritent de `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.94.1"`, `license = "Apache-2.0"`, `publish = false`. Aucune nouvelle dépendance tierce n'a été ajoutée lors de `t10` (`unleash-cli` dépend désormais de la crate interne `unleash-tui`, et `unleash-daemon` ajoute uniquement `unleash-tui`, `unleash-gui`, `ratatui`, `crossterm`, `futures-core` et `tauri` déjà présents dans `[workspace.dependencies]` en `[dev-dependencies]` pour `e2e_m0_surfaces`).

| Crate | Rôle | Dépendances directes (normales) | Dépendances build / dev |
|---|---|---|---|
| `unleash-browser` (`0.1.0`) | OVPT Unleash — Adaptateur BrowserPort WebDriver W3C (fantoccini), épinglage runtime, matrice V1 de locators et proxy d'egress | `bytes`, `fantoccini`, `hex`, `http`, `http-body-util`, `hyper`, `hyper-util`, `serde`, `serde_json`, `sha2`, `thiserror`, `tokio`, `tracing`, `unleash-domain`, `unleash-protocol`, `url`, `uuid` | `tempfile (dev)`, `unleash-testkit (dev)` |
| `unleash-cli` (`0.1.0`) | OVPT Unleash — Interface en ligne de commande stricte passant exclusivement par unleash-client (zéro accès SQLite) | `clap`, `serde_json`, `thiserror`, `tokio`, `unleash-client`, `unleash-domain`, `unleash-platform`, `unleash-protocol`, `unleash-tui`, `uuid` | `unleash-testkit (dev)` |
| `unleash-client` (`0.1.0`) | OVPT Unleash — Client IPC commun v2 (transport Unix 8 MiB, handshake Hello, file de corrélation persistable, SubscribeEvents/ResyncRequired, ClientUploadSegment) | `serde`, `serde_json`, `thiserror`, `tokio`, `unleash-domain`, `unleash-platform`, `unleash-protocol`, `uuid` | `chrono (dev)`, `tempfile (dev)`, `tokio (dev)`, `unleash-testkit (dev)` |
| `unleash-daemon` (`0.1.0`) | OVPT Unleash — Daemon utilisateur unique (autorité locale, socket IPC v2, barrière de démarrage, récupération et cycle de vie M0) | `chrono`, `clap`, `rusqlite`, `serde`, `serde_json`, `thiserror`, `tokio`, `tracing`, `unleash-browser`, `unleash-domain`, `unleash-platform`, `unleash-protocol`, `unleash-store`, `uuid` | `crossterm (dev)`, `futures-core (dev)`, `ratatui (dev)`, `tauri (dev)`, `tempfile (dev)`, `unleash-client (dev)`, `unleash-gui (dev)`, `unleash-testkit (dev)`, `unleash-tui (dev)` |
| `unleash-domain` (`0.1.0`) | OVPT Unleash — Types de domaine, identités, machines à états et transitions pures sans I/O | `hex`, `serde`, `serde_json`, `sha2`, `thiserror`, `uuid` | `proptest (dev)` |
| `unleash-gui` (`0.1.0`) | OVPT Unleash — Hôte desktop Tauri 2 consommant exclusivement unleash-client (zéro SQLite, zéro règle métier) | `pulldown-cmark`, `serde`, `serde_json`, `tauri`, `thiserror`, `tokio`, `tracing`, `ts-rs`, `unleash-client`, `unleash-domain`, `unleash-platform`, `unleash-protocol`, `uuid` | `chrono (dev)`, `tauri (dev)`, `tauri-build (build)`, `tempfile (dev)`, `tokio (dev)`, `unleash-testkit (dev)` |
| `unleash-platform` (`0.1.0`) | OVPT Unleash — Adaptateurs OS (verrou d'autorité UID, rendez-vous socket 0700/0600, peer credentials, identité processus, coffre secrets et chemins ADR-14) | `libc`, `rustix`, `secrecy`, `serde`, `serde_json`, `thiserror`, `unleash-domain`, `zeroize` | `unleash-testkit (dev)` |
| `unleash-process-host` (`0.1.0`) | OVPT Unleash — Hôte propriétaire PTY/service, ancrage non réutilisable, manifeste privé, fencing strict, spool borné et isolation OS stricte (F011, ADR-05, Q-05) | `hex`, `libc`, `portable-pty`, `rustix`, `secrecy`, `serde`, `serde_json`, `sha2`, `thiserror`, `tracing`, `unleash-domain`, `unleash-platform`, `unleash-protocol`, `uuid`, `zeroize` | `proptest (dev)`, `unleash-testkit (dev)` |
| `unleash-protocol` (`0.1.0`) | OVPT Unleash — Protocole wire IPC, DTOs stricts v2, codec de trames et types TypeScript | `chrono`, `hex`, `serde`, `serde_jcs`, `serde_json`, `sha2`, `thiserror`, `ts-rs`, `unleash-domain`, `uuid` | `jsonschema (dev)`, `proptest (dev)`, `referencing (dev)` |
| `unleash-store` (`0.1.0`) | OVPT Unleash — Persistance SQLite WAL qualifiée, transactions atomiques, migrations, outbox/inbox, objets, sauvegarde et restauration | `chrono`, `hex`, `rusqlite`, `serde`, `serde_json`, `sha2`, `thiserror`, `tokio`, `tracing`, `unleash-domain`, `unleash-protocol`, `uuid` | `proptest (dev)`, `unleash-testkit (dev)` |
| `unleash-testkit` (`0.1.0`) | OVPT Unleash — Harnais de test déterministe, horloge/IDs injectables, cutpoints C0-C12, crash harness et garde live | `dotenvy`, `secrecy`, `serde`, `serde_json`, `tempfile`, `thiserror`, `unleash-domain`, `uuid`, `zeroize` | — |
| `unleash-tui` (`0.1.0`) | OVPT Unleash — Interface terminal plein écran Ratatui 0.30 + Crossterm 0.29 à parité fonctionnelle avec la GUI | `crossterm`, `futures-core`, `ratatui`, `ropey`, `serde_json`, `thiserror`, `tokio`, `tracing`, `tui-term`, `unicode-segmentation`, `unicode-width`, `unleash-client`, `unleash-domain`, `unleash-protocol`, `uuid`, `vt100` | `tempfile (dev)`, `tokio (dev)`, `unleash-platform (dev)`, `unleash-testkit (dev)` |

---

## 3. Politique `deny.toml`, avis de sécurité et licences

### 3.1 Résumé `Cargo.lock` et `cargo deny`

- **Total paquets dans `Cargo.lock` :** `627` (`12` crates workspace + `615` crates externes toutes issues exclusivement de `registry+https://github.com/rust-lang/crates.io-index` ; `allow-git = []`).
- **Crates actives évaluées par `cargo deny` (`[graph] all-features = true` sur cible hôte) :** `517` crates externes (`98` crates additionnelles dans `Cargo.lock` correspondent à des cibles Linux/Windows/Android ou features `ratatui` non activées).
- **Statut final `cargo deny check` :** code de sortie **`0`** — `advisories ok, bans ok, licenses ok, sources ok`.
- **Unicité `crossterm` (`ADR-15` règle 4) :** une seule version exacte dans `Cargo.lock` (`crossterm 0.29.0`), avec `ratatui 0.30.2` (feature `crossterm_0_29`).

### 3.2 Dérogations motivées dans `deny.toml` (`[advisories]`)

| Identifiant | Crate verrouillée | Statut / Nature | Chaîne d'inclusion observée | Justification et critère de réexamen |
|---|---|---|---|---|
| `RUSTSEC-2024-0370` | `proc-macro-error 1.0.4` | `unmaintained` (proc-macro de compilation, **aucune CVE connue**) | `glib-macros 0.18.5` / `gtk3-macros 0.18.2` ← `gtk 0.18.2` ← `tao 0.37.2` / `wry 0.57.2` / `tauri 2.12.6` ← `unleash-gui` | Aucune mise à niveau disponible en amont dans la pile GTK3/Tauri 2.12.6 ; exécuté uniquement à la compilation pour cible Linux WebKitGTK, jamais embarqué en runtime. Réévaluation lors de la montée Tauri/GTK. |
| `RUSTSEC-2026-0543` | `chrono 0.4.45` | `unmaintained` (avis informatif sans CVE ni correctif amont) | Directement requis par `ts-rs 12.0.1` (feature `chrono-impl`), `unleash-protocol` et `unleash-store` | Nécessaire pour l'export TypeScript `DateTime<Utc>` (`ts-rs`) et le formatage RFC 3339 strict ; `default-features = false, features = ["serde"]`. Réévaluation prévue lorsque `ts-rs` supportera `jiff` ou `time`. |

### 3.3 Répartition des licences dans `Cargo.lock` et remarques `deny.toml`

Licences autorisées par `deny.toml` (`confidence-threshold = 0.9`) : `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `MIT`, `MIT-0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `0BSD`, `CC0-1.0`, `BSL-1.0`, `Unicode-3.0`, `Unicode-DFS-2016`, `CDLA-Permissive-2.0`, `MPL-2.0`.

- **Avertissements `license-not-encountered` (`3`) :** `CDLA-Permissive-2.0`, `ISC`, `Unicode-DFS-2016` (présentes dans `allow` de `deny.toml` mais non rencontrées dans le graphe actif évalué par `cargo deny`).
- **Crates `MPL-2.0` actives (`8`, admises non modifiées selon `ADR-14`) :** `cssparser 0.37.0`, `cssparser 0.38.0`, `cssparser-macros 0.7.1`, `dtoa-short 0.3.5`, `option-ext 0.2.0`, `selectors 0.38.0`, `selectors 0.41.0` (pile HTML/CSS Tauri) et `webdriver 0.54.0` (tiré par `fantoccini 0.22.2`).
- **Crates `Cargo.lock` hors graphe actif (features/cibles non activées) :** `terminfo 0.9.0` (`WTFPL`), `termina 0.3.3` (`MIT OR MPL-2.0`), `wezterm-bidi 0.2.3` (`MIT AND Unicode-DFS-2016`), `finl_unicode 1.5.1` (`(MIT OR Apache-2.0) AND Unicode-DFS-2016`), `libloading 0.7.4` (`ISC`) proviennent des backends optionnels désactivés de `ratatui` (`termwiz` / `termina`) et ne sont ni compilés ni liés (seul `crossterm_0_29` est activé).
- **Crates multi-licences incluant une branche copyleft optionnelle :** `r-efi 5.3.0` et `6.0.0` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`), consommées sous `MIT OR Apache-2.0`.

| Expression SPDX (`Cargo.lock` / `cargo metadata`) | Nombre de crates externes |
|---|---:|
| `MIT OR Apache-2.0` | `268` |
| `MIT` | `173` |
| `Apache-2.0 OR MIT` | `39` |
| `MIT/Apache-2.0` | `27` |
| `Unicode-3.0` | `22` |
| `Zlib OR Apache-2.0 OR MIT` | `17` |
| `Unlicense OR MIT` | `10` |
| `MPL-2.0` | `8` |
| `Apache-2.0/MIT` | `7` |
| `Apache-2.0` | `5` |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `5` |
| `BSD-3-Clause` | `2` |
| `Apache-2.0 WITH LLVM-exception` | `2` |
| `Zlib` | `2` |
| `MIT OR Zlib OR Apache-2.0` | `2` |
| `BSD-3-Clause OR MIT OR Apache-2.0` | `2` |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `2` |
| `MIT OR Apache-2.0 OR Zlib` | `2` |
| `Apache-2.0 OR BSL-1.0` | `2` |
| `Unlicense/MIT` | `2` |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | `2` |
| `0BSD OR MIT OR Apache-2.0` | `1` |
| `MIT-0` | `1` |
| `BSD-3-Clause AND MIT` | `1` |
| `BSD-3-Clause/MIT` | `1` |
| `Apache-2.0 AND MIT` | `1` |
| `CC0-1.0 OR MIT-0 OR Apache-2.0` | `1` |
| `(MIT OR Apache-2.0) AND Unicode-DFS-2016` | `1` |
| `Apache-2.0 / MIT` | `1` |
| `ISC` | `1` |
| `BSD-2-Clause OR Apache-2.0` | `1` |
| `MIT OR MPL-2.0` | `1` |
| `WTFPL` | `1` |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | `1` |
| `MIT AND Unicode-DFS-2016` | `1` |

### 3.4 Versions multiples détectées (`[bans] multiple-versions = "warn"`, `41` entrées)

Les avertissements `duplicate` sont uniquement informatifs (`warn`, non bloquants) et proviennent des dépendances amont (`tauri 2.12.6`, `fantoccini 0.22.2`, `portable-pty 0.9.0`, `jsonschema 0.61.2`) :

| Crate | Occurrences | Versions dans `Cargo.lock` |
|---|---:|---|
| `base64` | `3` | `0.21.7`, `0.22.1`, `0.23.1` |
| `bit-set` | `2` | `0.11.1`, `0.5.3`, `0.8.0` |
| `bit-vec` | `2` | `0.10.2`, `0.6.3`, `0.8.0` |
| `bitflags` | `2` | `1.3.2`, `2.13.2` |
| `block-buffer` | `2` | `0.10.4`, `0.12.1` |
| `cookie` | `2` | `0.16.2`, `0.18.2` |
| `cpufeatures` | `2` | `0.2.17`, `0.3.1` |
| `crypto-common` | `2` | `0.1.7`, `0.2.2` |
| `cssparser` | `2` | `0.37.0`, `0.38.0` |
| `digest` | `2` | `0.10.7`, `0.11.3` |
| `dom_query` | `2` | `0.28.0`, `0.29.0` |
| `getrandom` | `2` | `0.3.4`, `0.4.3` |
| `hashbrown` | `2` | `0.12.3`, `0.17.1` |
| `heck` | `2` | `0.4.1`, `0.5.0` |
| `html5ever` | `2` | `0.39.0`, `0.40.1` |
| `indexmap` | `2` | `1.9.3`, `2.14.2` |
| `markup5ever` | `2` | `0.39.0`, `0.40.0` |
| `miniz_oxide` | `2` | `0.8.9`, `0.9.1` |
| `phf` | `2` | `0.11.3`, `0.13.1`, `0.14.0` |
| `phf_codegen` | `2` | `0.11.3`, `0.13.1`, `0.14.0` |
| `phf_generator` | `2` | `0.11.3`, `0.13.1`, `0.14.0` |
| `phf_macros` | `2` | `0.11.3`, `0.13.1`, `0.14.0` |
| `phf_shared` | `2` | `0.11.3`, `0.13.1`, `0.14.0` |
| `proc-macro-crate` | `3` | `1.3.1`, `2.0.2`, `3.5.0` |
| `r-efi` | `2` | `5.3.0`, `6.0.0` |
| `selectors` | `2` | `0.38.0`, `0.41.0` |
| `serde_spanned` | `2` | `0.6.9`, `1.1.3` |
| `servo_arc` | `2` | `0.4.3`, `0.5.0` |
| `sha2` | `2` | `0.10.9`, `0.11.1` |
| `string_cache` | `2` | `0.11.0`, `0.9.0` |
| `string_cache_codegen` | `2` | `0.11.2`, `0.6.1` |
| `syn` | `3` | `1.0.109`, `2.0.119`, `3.0.10` |
| `thiserror` | `2` | `1.0.69`, `2.0.21` |
| `thiserror-impl` | `2` | `1.0.69`, `2.0.21` |
| `toml` | `2` | `0.8.2`, `1.2.1+spec-1.1.0` |
| `toml_datetime` | `2` | `0.6.3`, `1.1.3+spec-1.1.0` |
| `toml_edit` | `3` | `0.19.15`, `0.20.2`, `0.25.19+spec-1.1.0` |
| `web_atoms` | `2` | `0.2.6`, `0.3.0` |
| `windows-sys` | `2` | `0.45.0`, `0.61.2` |
| `winnow` | `2` | `0.5.40`, `1.0.5` |
| `winreg` | `2` | `0.10.1`, `0.56.0` |

---

## 4. Dépendances natives et système OS

### 4.1 SQLite embarquée (`unleash-store` / `rusqlite 0.43.0`)

Vérifié directement par exécution de `cargo run -p unleash-store --example spike_sqlite` :
- **Crate :** `rusqlite 0.43.0` (`features = ["bundled", "backup"]`) + `libsqlite3-sys 0.40.0` (compilation C statique via `cc`, zéro dépendance `libsqlite3` système).
- **`sqlite_version()` :** `3.54.1`
- **`sqlite_source_id()` :** `2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76` (concordance exacte `expected_source_id` à l'ouverture).
- **Profil de compilation relevé :** `51` options actives (`COMPILER=clang-21.0.0`, `THREADSAFE=1`, WAL actif sans `SQLITE_OMIT_WAL`, `DEFAULT_FOREIGN_KEYS`, `ENABLE_FTS5`, `ENABLE_API_ARMOR`).

### 4.2 Frameworks et bibliothèques dynamiques macOS observés (`otool -L`)

Extrait exact d'`otool -L` sur les 4 binaires produits dans `$CARGO_TARGET_DIR/debug/` :

```text
/Users/louis/Library/Caches/unleash/target/g0001-m0-t10/debug/unleashd:
	/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 5027.1.8)
	/usr/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.40.3)
/Users/louis/Library/Caches/unleash/target/g0001-m0-t10/debug/unleash:
	/usr/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
	/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 5027.1.8)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.40.3)
/Users/louis/Library/Caches/unleash/target/g0001-m0-t10/debug/unleash-gui:
	/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit (compatibility version 1.0.0, current version 625.2.8)
	/System/Library/Frameworks/ApplicationServices.framework/Versions/A/ApplicationServices (compatibility version 1.0.0, current version 66.0.0)
	/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics (compatibility version 64.0.0, current version 2050.1.6)
	/System/Library/Frameworks/Carbon.framework/Versions/A/Carbon (compatibility version 2.0.0, current version 170.0.0)
	/System/Library/Frameworks/CoreVideo.framework/Versions/A/CoreVideo (compatibility version 1.2.0, current version 776.2.0)
	/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 5027.1.8)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.40.3)
	/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit (compatibility version 45.0.0, current version 2775.20.133)
	/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore (compatibility version 1.2.0, current version 1223.10.17)
	/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation (compatibility version 300.0.0, current version 5027.1.8)
	/usr/lib/libobjc.A.dylib (compatibility version 1.0.0, current version 228.0.0)
	/usr/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
	/System/Library/Frameworks/ColorSync.framework/Versions/A/ColorSync (compatibility version 1.0.0, current version 3929.1.5)
	/System/Library/Frameworks/CoreServices.framework/Versions/A/CoreServices (compatibility version 1.0.0, current version 1226.0.0)
/Users/louis/Library/Caches/unleash/target/g0001-m0-t10/debug/unleash-process-host:
	/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 5027.1.8)
	/usr/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
	/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1359.40.3)
```

### 4.3 `Security.framework` / trousseau OS et plateformes GUI

- **Trousseau OS (`Security.framework` / `Secret Service`) :** en M0, `crates/unleash-platform/src/keyring.rs` définit l'abstraction `KeyringPort` (`KeyringBackendKind::MacosKeychain`, `LinuxSecretService`, `LocalSecureTest`, `Unavailable`) avec `LocalSecureKeyring` (`secrecy 0.10.3` + `zeroize 1.9.1`) et `UnavailableOsKeyring` (fail-closed sans repli en clair). Comme le montre `otool -L` ci-dessus, `Security.framework` n'est pas encore lié dynamiquement dans les binaires M0 et sera branché avec l'identifiant Apple Developer / profil signé au gate `EXT-DIST` (`ADR-14`).
- **GUI macOS :** `unleash-gui` (`tauri 2.12.6`, `wry 0.57.2`, `tao 0.37.2`, `objc2-web-kit 0.3.2`) lie dynamiquement `/System/Library/Frameworks/WebKit.framework` (WKWebView système) ainsi que `AppKit`, `CoreFoundation`, `Foundation`, `CoreGraphics`, `QuartzCore`, `ApplicationServices`, `Carbon`, `CoreVideo`, `ColorSync` et `CoreServices`.
- **GUI Ubuntu 24.04 / 26.04 LTS (`x86_64`) :** WebKitGTK système et GTK3 requis côté build/runtime selon `.github/workflows/ci.yml` (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`, `pkg-config`, `libssl-dev`).

---

## 5. Runtimes externes hors graphe Cargo

| Runtime externe | Rôle prévu | État observé sur ce poste | Détails / Écarts M0 |
|---|---|---|---|
| **Google Chrome for Testing** | Moteur navigateur automatisé (`unleash-browser` / `F019`) | **Présent** (`156.0.8078.4`) | `~/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app` ; SHA-256 binaire vérifié : `c8509aee32abd6977968e9999769980b7a420dea05402d90042d9e5b41db15ef`. |
| **chromedriver** | Serveur W3C WebDriver appairé à `fantoccini 0.22.2` | **Absent** (`not_run` / bloqué `EXT-BROWSER`) | `which chromedriver` introuvable ; absent de `~/Library/Caches/ms-playwright/`. `unleash-browser` utilise `default-features = false` (HTTP loopback sans `cmake`/`aws-lc-sys`). |
| **Ollama** | Backend de modèles locaux (`Q-19` / `ADR-17`) | **Présent** (`0.40.16`, sans poids locaux) | `/Applications/Ollama.app` (`0.40.16`) et `/usr/local/bin/ollama` (`ollama version is 0.40.16`) ; `~/.ollama/models/manifests` vide (aucun modèle téléchargé). |
| **LM Studio (`lms`)** | Backend de modèles locaux (`Q-19`) | **Absent** | `/Applications/LM Studio.app` absent, `which lms` introuvable. |
| **llama.cpp (`llama-server`)** | Backend de modèles locaux (`Q-19`) | **Absent** | `which llama-server` introuvable dans `PATH`. |

---

## 6. Inventaire frontend `apps/gui-renderer` (`pnpm`)

Exécuté sur copie APFS propre (`$HOME/Library/Caches/unleash/gui-stage/g0001-m0-t10`, `pnpm install --frozen-lockfile`, `typecheck`, `lint`, `format:check`, `vitest run` 7/7 passés, `vite build`) :

- **Dépendances directes (`apps/gui-renderer/package.json`) :** `8` dépendances runtime + `16` `devDependencies`.
- **Paquets de production embarqués dans `dist/` (`pnpm licenses list --prod --json`) :** `31` paquets, exclusivement sous licences permissives `MIT` (`22`), `Apache-2.0` (`8`) et `0BSD` (`1`, `tslib@2.9.1`).
- **Total paquets lockfile (`pnpm licenses list --json`, prod + outillage dev/test) :** `400` entrées. Les licences non-MIT/Apache/BSD (`MPL-2.0` sur `axe-core@4.14.1`, `BlueOak-1.0.0` sur `@isaacs/nonrecursive-glob-match@1.1.0` et `minimatch@11.0.9`, `CC-BY-4.0` sur `caniuse-lite@1.0.30001826`, `CC0-1.0` sur `language-subtag-registry@0.3.23`, `Python-2.0` sur `argparse@2.0.1`) appartiennent uniquement aux `devDependencies` (ESLint / Vitest / Vite) et ne sont jamais distribuées dans `dist/`.

### 6.1 Paquets frontend de production (`31` paquets embarqués dans `dist/`)

| Paquet | Version | Licence SPDX | Portée |
|---|---|---|---|
| `@codemirror/language` | `6.13.1` | `MIT` | prod + dev |
| `@codemirror/merge` | `6.12.3` | `MIT` | prod + dev |
| `@codemirror/state` | `6.7.6` | `MIT` | prod + dev |
| `@codemirror/streamparser` | `6.0.0` | `MIT` | prod + dev |
| `@codemirror/view` | `6.43.15` | `MIT` | prod + dev |
| `@internationalized/date` | `3.12.4` | `Apache-2.0` | prod + dev |
| `@internationalized/number` | `3.6.9` | `Apache-2.0` | prod + dev |
| `@internationalized/string` | `3.2.10` | `Apache-2.0` | prod + dev |
| `@lezer/common` | `1.5.5` | `MIT` | prod + dev |
| `@lezer/highlight` | `1.2.5` | `MIT` | prod + dev |
| `@lezer/lr` | `1.4.12` | `MIT` | prod + dev |
| `@marijn/find-cluster-break` | `1.0.4` | `MIT` | prod + dev |
| `@react-types/shared` | `3.37.0` | `Apache-2.0` | prod + dev |
| `@swc/helpers` | `0.5.24` | `Apache-2.0` | prod + dev |
| `@tanstack/react-virtual` | `3.14.26` | `MIT` | prod + dev |
| `@tanstack/virtual-core` | `3.19.11` | `MIT` | prod + dev |
| `@xterm/xterm` | `5.5.0` | `MIT` | prod + dev |
| `aria-hidden` | `1.2.6` | `MIT` | prod + dev |
| `client-only` | `0.0.1` | `MIT` | prod + dev |
| `clsx` | `2.1.1` | `MIT` | prod + dev |
| `crelt` | `1.0.7` | `MIT` | prod + dev |
| `react` | `19.3.1` | `MIT` | prod + dev |
| `react-aria` | `3.53.1` | `Apache-2.0` | prod + dev |
| `react-aria-components` | `1.22.1` | `Apache-2.0` | prod + dev |
| `react-dom` | `19.3.1` | `MIT` | prod + dev |
| `react-stately` | `3.51.0` | `Apache-2.0` | prod + dev |
| `scheduler` | `0.28.0` | `MIT` | prod + dev |
| `style-mod` | `4.1.5` | `MIT` | prod + dev |
| `tslib` | `2.9.1` | `0BSD` | prod + dev |
| `use-sync-external-store` | `1.7.0` | `MIT` | prod + dev |
| `w3c-keyname` | `2.2.8` | `MIT` | prod + dev |

<details>
<summary>Inventaire complet des 400 paquets pnpm (production + devDependencies)</summary>

| Paquet | Version | Licence SPDX | Portée |
|---|---|---|---|
| `@codemirror/language` | `6.13.1` | `MIT` | prod + dev |
| `@codemirror/merge` | `6.12.3` | `MIT` | prod + dev |
| `@codemirror/state` | `6.7.6` | `MIT` | prod + dev |
| `@codemirror/streamparser` | `6.0.0` | `MIT` | prod + dev |
| `@codemirror/view` | `6.43.15` | `MIT` | prod + dev |
| `@internationalized/date` | `3.12.4` | `Apache-2.0` | prod + dev |
| `@internationalized/number` | `3.6.9` | `Apache-2.0` | prod + dev |
| `@internationalized/string` | `3.2.10` | `Apache-2.0` | prod + dev |
| `@lezer/common` | `1.5.5` | `MIT` | prod + dev |
| `@lezer/highlight` | `1.2.5` | `MIT` | prod + dev |
| `@lezer/lr` | `1.4.12` | `MIT` | prod + dev |
| `@marijn/find-cluster-break` | `1.0.4` | `MIT` | prod + dev |
| `@react-types/shared` | `3.37.0` | `Apache-2.0` | prod + dev |
| `@swc/helpers` | `0.5.24` | `Apache-2.0` | prod + dev |
| `@tanstack/react-virtual` | `3.14.26` | `MIT` | prod + dev |
| `@tanstack/virtual-core` | `3.19.11` | `MIT` | prod + dev |
| `@xterm/xterm` | `5.5.0` | `MIT` | prod + dev |
| `aria-hidden` | `1.2.6` | `MIT` | prod + dev |
| `client-only` | `0.0.1` | `MIT` | prod + dev |
| `clsx` | `2.1.1` | `MIT` | prod + dev |
| `crelt` | `1.0.7` | `MIT` | prod + dev |
| `react` | `19.3.1` | `MIT` | prod + dev |
| `react-aria` | `3.53.1` | `Apache-2.0` | prod + dev |
| `react-aria-components` | `1.22.1` | `Apache-2.0` | prod + dev |
| `react-dom` | `19.3.1` | `MIT` | prod + dev |
| `react-stately` | `3.51.0` | `Apache-2.0` | prod + dev |
| `scheduler` | `0.28.0` | `MIT` | prod + dev |
| `style-mod` | `4.1.5` | `MIT` | prod + dev |
| `tslib` | `2.9.1` | `0BSD` | prod + dev |
| `use-sync-external-store` | `1.7.0` | `MIT` | prod + dev |
| `w3c-keyname` | `2.2.8` | `MIT` | prod + dev |
| `@adobe/css-tools` | `4.5.0` | `MIT` | dev |
| `@asamuzakjp/css-color` | `3.2.0` | `MIT` | dev |
| `@babel/code-frame` | `7.29.7` | `MIT` | dev |
| `@babel/compat-data` | `7.29.7` | `MIT` | dev |
| `@babel/core` | `7.29.13` | `MIT` | dev |
| `@babel/generator` | `7.29.14` | `MIT` | dev |
| `@babel/helper-compilation-targets` | `7.29.13` | `MIT` | dev |
| `@babel/helper-globals` | `7.29.7` | `MIT` | dev |
| `@babel/helper-module-imports` | `7.29.7` | `MIT` | dev |
| `@babel/helper-module-transforms` | `7.29.14` | `MIT` | dev |
| `@babel/helper-plugin-utils` | `7.29.7` | `MIT` | dev |
| `@babel/helper-string-parser` | `7.29.14` | `MIT` | dev |
| `@babel/helper-validator-identifier` | `7.29.7` | `MIT` | dev |
| `@babel/helper-validator-option` | `7.29.7` | `MIT` | dev |
| `@babel/helpers` | `7.29.14` | `MIT` | dev |
| `@babel/parser` | `7.29.14` | `MIT` | dev |
| `@babel/plugin-transform-react-jsx-self` | `7.29.7` | `MIT` | dev |
| `@babel/plugin-transform-react-jsx-source` | `7.29.7` | `MIT` | dev |
| `@babel/runtime` | `7.29.14` | `MIT` | dev |
| `@babel/template` | `7.29.7` | `MIT` | dev |
| `@babel/traverse` | `7.29.14` | `MIT` | dev |
| `@babel/types` | `7.29.14` | `MIT` | dev |
| `@csstools/color-helpers` | `5.1.0` | `MIT-0` | dev |
| `@csstools/css-calc` | `2.1.4` | `MIT` | dev |
| `@csstools/css-color-parser` | `3.1.0` | `MIT` | dev |
| `@csstools/css-parser-algorithms` | `3.0.5` | `MIT` | dev |
| `@csstools/css-tokenizer` | `3.0.4` | `MIT` | dev |
| `@esbuild/darwin-arm64` | `0.28.2` | `MIT` | dev |
| `@eslint-community/eslint-utils` | `4.10.1` | `MIT` | dev |
| `@eslint-community/regexpp` | `4.12.3` | `MIT` | dev |
| `@eslint/config-array` | `0.21.2` | `Apache-2.0` | dev |
| `@eslint/config-helpers` | `0.4.2` | `Apache-2.0` | dev |
| `@eslint/core` | `0.17.0` | `Apache-2.0` | dev |
| `@eslint/eslintrc` | `3.3.7` | `MIT` | dev |
| `@eslint/js` | `9.39.5` | `MIT` | dev |
| `@eslint/object-schema` | `2.1.7` | `Apache-2.0` | dev |
| `@eslint/plugin-kit` | `0.4.1` | `Apache-2.0` | dev |
| `@humanfs/core` | `0.19.2` | `Apache-2.0` | dev |
| `@humanfs/node` | `0.16.8` | `Apache-2.0` | dev |
| `@humanfs/types` | `0.15.0` | `Apache-2.0` | dev |
| `@humanwhocodes/module-importer` | `1.0.1` | `Apache-2.0` | dev |
| `@humanwhocodes/retry` | `0.4.3` | `Apache-2.0` | dev |
| `@isaacs/nonrecursive-glob-match` | `1.1.0` | `BlueOak-1.0.0` | dev |
| `@jridgewell/gen-mapping` | `0.3.13` | `MIT` | dev |
| `@jridgewell/remapping` | `2.3.5` | `MIT` | dev |
| `@jridgewell/resolve-uri` | `3.1.3` | `MIT` | dev |
| `@jridgewell/sourcemap-codec` | `1.6.0` | `MIT` | dev |
| `@jridgewell/trace-mapping` | `0.3.31` | `MIT` | dev |
| `@rolldown/pluginutils` | `1.0.0-beta.27` | `MIT` | dev |
| `@rollup/rollup-darwin-arm64` | `4.68.5` | `MIT` | dev |
| `@testing-library/dom` | `10.4.2` | `MIT` | dev |
| `@testing-library/jest-dom` | `6.10.0` | `MIT` | dev |
| `@testing-library/react` | `16.3.3` | `MIT` | dev |
| `@testing-library/user-event` | `14.6.12` | `MIT` | dev |
| `@types/aria-query` | `5.0.4` | `MIT` | dev |
| `@types/babel__core` | `7.20.5` | `MIT` | dev |
| `@types/babel__generator` | `7.27.0` | `MIT` | dev |
| `@types/babel__template` | `7.4.4` | `MIT` | dev |
| `@types/babel__traverse` | `7.28.0` | `MIT` | dev |
| `@types/chai` | `5.2.3` | `MIT` | dev |
| `@types/deep-eql` | `4.0.2` | `MIT` | dev |
| `@types/estree` | `1.0.10` | `MIT` | dev |
| `@types/json-schema` | `7.0.15` | `MIT` | dev |
| `@types/react` | `19.3.0` | `MIT` | dev |
| `@types/react-dom` | `19.3.0` | `MIT` | dev |
| `@typescript-eslint/eslint-plugin` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/parser` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/project-service` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/scope-manager` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/tsconfig-utils` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/type-utils` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/types` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/typescript-estree` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/utils` | `8.73.0` | `MIT` | dev |
| `@typescript-eslint/visitor-keys` | `8.73.0` | `MIT` | dev |
| `@vitejs/plugin-react` | `4.7.0` | `MIT` | dev |
| `@vitest/expect` | `3.2.7` | `MIT` | dev |
| `@vitest/mocker` | `3.2.7` | `MIT` | dev |
| `@vitest/pretty-format` | `3.2.7` | `MIT` | dev |
| `@vitest/runner` | `3.2.7` | `MIT` | dev |
| `@vitest/snapshot` | `3.2.7` | `MIT` | dev |
| `@vitest/spy` | `3.2.7` | `MIT` | dev |
| `@vitest/utils` | `3.2.7` | `MIT` | dev |
| `acorn` | `8.19.0` | `MIT` | dev |
| `acorn-jsx` | `5.3.2` | `MIT` | dev |
| `agent-base` | `7.1.4` | `MIT` | dev |
| `ajv` | `6.15.0` | `MIT` | dev |
| `ansi-regex` | `5.0.1` | `MIT` | dev |
| `ansi-styles` | `4.3.0` | `MIT` | dev |
| `ansi-styles` | `5.2.0` | `MIT` | dev |
| `argparse` | `2.0.1` | `Python-2.0` | dev |
| `aria-query` | `5.3.0` | `Apache-2.0` | dev |
| `aria-query` | `5.3.2` | `Apache-2.0` | dev |
| `array-buffer-byte-length` | `1.0.2` | `MIT` | dev |
| `array-includes` | `3.2.0` | `MIT` | dev |
| `array.prototype.flat` | `1.3.3` | `MIT` | dev |
| `array.prototype.flatmap` | `1.3.3` | `MIT` | dev |
| `arraybuffer.prototype.slice` | `1.0.4` | `MIT` | dev |
| `assertion-error` | `2.0.1` | `MIT` | dev |
| `ast-types-flow` | `0.0.8` | `MIT` | dev |
| `async-function` | `1.0.0` | `MIT` | dev |
| `available-typed-arrays` | `1.0.7` | `MIT` | dev |
| `axe-core` | `4.14.1` | `MPL-2.0` | dev |
| `axobject-query` | `4.1.0` | `Apache-2.0` | dev |
| `balanced-match` | `1.0.2` | `MIT` | dev |
| `balanced-match` | `4.0.4` | `MIT` | dev |
| `baseline-browser-mapping` | `2.11.38` | `Apache-2.0` | dev |
| `brace-expansion` | `1.1.22` | `MIT` | dev |
| `brace-expansion` | `5.0.13` | `MIT` | dev |
| `browserslist` | `4.29.6` | `MIT` | dev |
| `cac` | `6.7.14` | `MIT` | dev |
| `call-bind` | `1.0.9` | `MIT` | dev |
| `call-bind-apply-helpers` | `1.0.2` | `MIT` | dev |
| `call-bound` | `1.0.4` | `MIT` | dev |
| `callsites` | `3.1.0` | `MIT` | dev |
| `caniuse-lite` | `1.0.30001826` | `CC-BY-4.0` | dev |
| `chai` | `5.3.3` | `MIT` | dev |
| `chalk` | `4.1.2` | `MIT` | dev |
| `check-error` | `2.1.3` | `MIT` | dev |
| `color-convert` | `2.0.1` | `MIT` | dev |
| `color-name` | `1.1.4` | `MIT` | dev |
| `concat-map` | `0.0.1` | `MIT` | dev |
| `convert-source-map` | `2.0.0` | `MIT` | dev |
| `cross-spawn` | `7.0.6` | `MIT` | dev |
| `css.escape` | `1.5.1` | `MIT` | dev |
| `cssstyle` | `4.6.0` | `MIT` | dev |
| `csstype` | `3.2.3` | `MIT` | dev |
| `damerau-levenshtein` | `1.0.8` | `BSD-2-Clause` | dev |
| `data-urls` | `5.0.0` | `MIT` | dev |
| `data-view-buffer` | `1.0.2` | `MIT` | dev |
| `data-view-byte-length` | `1.0.2` | `MIT` | dev |
| `data-view-byte-offset` | `1.0.1` | `MIT` | dev |
| `debug` | `4.4.3` | `MIT` | dev |
| `decimal.js` | `10.6.0` | `MIT` | dev |
| `deep-eql` | `5.0.2` | `MIT` | dev |
| `deep-is` | `0.1.4` | `MIT` | dev |
| `define-data-property` | `1.1.4` | `MIT` | dev |
| `define-properties` | `1.2.1` | `MIT` | dev |
| `dequal` | `2.0.3` | `MIT` | dev |
| `dom-accessibility-api` | `0.5.16` | `MIT` | dev |
| `dom-accessibility-api` | `0.6.3` | `MIT` | dev |
| `dunder-proto` | `1.0.1` | `MIT` | dev |
| `electron-to-chromium` | `1.5.487` | `ISC` | dev |
| `emoji-regex` | `9.2.2` | `MIT` | dev |
| `entities` | `6.0.1` | `BSD-2-Clause` | dev |
| `es-abstract` | `1.24.2` | `MIT` | dev |
| `es-abstract-get` | `1.0.0` | `MIT` | dev |
| `es-define-property` | `1.0.1` | `MIT` | dev |
| `es-errors` | `1.3.0` | `MIT` | dev |
| `es-module-lexer` | `1.7.0` | `MIT` | dev |
| `es-object-atoms` | `1.1.2` | `MIT` | dev |
| `es-set-tostringtag` | `2.1.0` | `MIT` | dev |
| `es-shim-unscopables` | `1.1.0` | `MIT` | dev |
| `es-to-primitive` | `1.3.4` | `MIT` | dev |
| `esbuild` | `0.28.2` | `MIT` | dev |
| `escalade` | `3.2.0` | `MIT` | dev |
| `escape-string-regexp` | `4.0.0` | `MIT` | dev |
| `eslint` | `9.39.5` | `MIT` | dev |
| `eslint-plugin-jsx-a11y` | `6.10.2` | `MIT` | dev |
| `eslint-plugin-react-hooks` | `5.2.0` | `MIT` | dev |
| `eslint-scope` | `8.4.0` | `BSD-2-Clause` | dev |
| `eslint-visitor-keys` | `3.4.3` | `Apache-2.0` | dev |
| `eslint-visitor-keys` | `4.2.1` | `Apache-2.0` | dev |
| `eslint-visitor-keys` | `5.0.1` | `Apache-2.0` | dev |
| `espree` | `10.4.0` | `BSD-2-Clause` | dev |
| `esquery` | `1.7.0` | `BSD-3-Clause` | dev |
| `esrecurse` | `4.3.0` | `BSD-2-Clause` | dev |
| `estraverse` | `5.3.0` | `BSD-2-Clause` | dev |
| `estree-walker` | `3.0.3` | `MIT` | dev |
| `esutils` | `2.0.3` | `BSD-2-Clause` | dev |
| `expect-type` | `1.4.0` | `Apache-2.0` | dev |
| `fast-deep-equal` | `3.1.3` | `MIT` | dev |
| `fast-json-stable-stringify` | `2.1.0` | `MIT` | dev |
| `fast-levenshtein` | `2.0.6` | `MIT` | dev |
| `fdir` | `6.5.0` | `MIT` | dev |
| `file-entry-cache` | `8.0.0` | `MIT` | dev |
| `find-up` | `5.0.0` | `MIT` | dev |
| `flat-cache` | `4.0.1` | `MIT` | dev |
| `flatted` | `3.4.4` | `ISC` | dev |
| `for-each` | `0.3.5` | `MIT` | dev |
| `fsevents` | `2.3.3` | `MIT` | dev |
| `function-bind` | `1.1.2` | `MIT` | dev |
| `function.prototype.name` | `1.2.0` | `MIT` | dev |
| `functions-have-names` | `1.2.3` | `MIT` | dev |
| `generator-function` | `2.0.1` | `MIT` | dev |
| `gensync` | `1.0.0-beta.2` | `MIT` | dev |
| `get-intrinsic` | `1.3.0` | `MIT` | dev |
| `get-proto` | `1.0.1` | `MIT` | dev |
| `get-symbol-description` | `1.1.0` | `MIT` | dev |
| `glob-parent` | `6.0.2` | `ISC` | dev |
| `globals` | `14.0.0` | `MIT` | dev |
| `globalthis` | `1.0.4` | `MIT` | dev |
| `gopd` | `1.2.0` | `MIT` | dev |
| `has-bigints` | `1.1.0` | `MIT` | dev |
| `has-flag` | `4.0.0` | `MIT` | dev |
| `has-property-descriptors` | `1.0.2` | `MIT` | dev |
| `has-proto` | `1.2.0` | `MIT` | dev |
| `has-symbols` | `1.1.0` | `MIT` | dev |
| `has-tostringtag` | `1.0.2` | `MIT` | dev |
| `hasown` | `2.0.4` | `MIT` | dev |
| `html-encoding-sniffer` | `4.0.0` | `MIT` | dev |
| `http-proxy-agent` | `7.0.2` | `MIT` | dev |
| `https-proxy-agent` | `7.0.6` | `MIT` | dev |
| `iconv-lite` | `0.6.3` | `MIT` | dev |
| `ignore` | `5.3.2` | `MIT` | dev |
| `ignore` | `7.0.16` | `MIT` | dev |
| `import-fresh` | `3.3.1` | `MIT` | dev |
| `imurmurhash` | `0.1.4` | `MIT` | dev |
| `indent-string` | `4.0.0` | `MIT` | dev |
| `internal-slot` | `1.1.0` | `MIT` | dev |
| `is-array-buffer` | `3.0.5` | `MIT` | dev |
| `is-async-function` | `2.1.1` | `MIT` | dev |
| `is-bigint` | `1.1.0` | `MIT` | dev |
| `is-boolean-object` | `1.2.2` | `MIT` | dev |
| `is-callable` | `1.2.7` | `MIT` | dev |
| `is-data-view` | `1.0.2` | `MIT` | dev |
| `is-date-object` | `1.1.0` | `MIT` | dev |
| `is-document.all` | `1.0.0` | `MIT` | dev |
| `is-extglob` | `2.1.1` | `MIT` | dev |
| `is-finalizationregistry` | `1.1.1` | `MIT` | dev |
| `is-generator-function` | `1.1.2` | `MIT` | dev |
| `is-glob` | `4.0.3` | `MIT` | dev |
| `is-map` | `2.0.3` | `MIT` | dev |
| `is-negative-zero` | `2.0.3` | `MIT` | dev |
| `is-number-object` | `1.1.1` | `MIT` | dev |
| `is-potential-custom-element-name` | `1.0.1` | `MIT` | dev |
| `is-regex` | `1.2.1` | `MIT` | dev |
| `is-set` | `2.0.3` | `MIT` | dev |
| `is-shared-array-buffer` | `1.0.4` | `MIT` | dev |
| `is-string` | `1.1.1` | `MIT` | dev |
| `is-symbol` | `1.1.1` | `MIT` | dev |
| `is-typed-array` | `1.1.15` | `MIT` | dev |
| `is-weakmap` | `2.0.2` | `MIT` | dev |
| `is-weakref` | `1.1.1` | `MIT` | dev |
| `is-weakset` | `2.0.4` | `MIT` | dev |
| `isarray` | `2.0.5` | `MIT` | dev |
| `isexe` | `2.0.0` | `ISC` | dev |
| `js-tokens` | `4.0.0` | `MIT` | dev |
| `js-tokens` | `9.0.1` | `MIT` | dev |
| `js-yaml` | `4.3.2` | `MIT` | dev |
| `jsdom` | `26.1.0` | `MIT` | dev |
| `jsesc` | `3.1.0` | `MIT` | dev |
| `json-buffer` | `3.0.1` | `MIT` | dev |
| `json-schema-traverse` | `0.4.1` | `MIT` | dev |
| `json-stable-stringify-without-jsonify` | `1.0.1` | `MIT` | dev |
| `json5` | `2.2.3` | `MIT` | dev |
| `jsx-ast-utils` | `3.3.5` | `MIT` | dev |
| `keyv` | `4.5.4` | `MIT` | dev |
| `language-subtag-registry` | `0.3.23` | `CC0-1.0` | dev |
| `language-tags` | `1.0.9` | `MIT` | dev |
| `levn` | `0.4.1` | `MIT` | dev |
| `locate-path` | `6.0.0` | `MIT` | dev |
| `lodash.merge` | `4.6.2` | `MIT` | dev |
| `loupe` | `3.2.1` | `MIT` | dev |
| `lru-cache` | `10.4.3` | `ISC` | dev |
| `lru-cache` | `5.1.1` | `ISC` | dev |
| `lz-string` | `1.5.0` | `MIT` | dev |
| `magic-string` | `0.30.21` | `MIT` | dev |
| `math-intrinsics` | `1.1.0` | `MIT` | dev |
| `min-indent` | `1.0.1` | `MIT` | dev |
| `minimatch` | `11.0.9` | `BlueOak-1.0.0` | dev |
| `minimatch` | `3.1.5` | `ISC` | dev |
| `ms` | `2.1.3` | `MIT` | dev |
| `nanoid` | `3.3.20` | `MIT` | dev |
| `natural-compare` | `1.4.0` | `MIT` | dev |
| `node-releases` | `2.0.64` | `MIT` | dev |
| `nwsapi` | `2.2.28` | `MIT` | dev |
| `object-inspect` | `1.13.4` | `MIT` | dev |
| `object-keys` | `1.1.1` | `MIT` | dev |
| `object.assign` | `4.1.7` | `MIT` | dev |
| `object.fromentries` | `2.0.8` | `MIT` | dev |
| `object.values` | `1.2.1` | `MIT` | dev |
| `optionator` | `0.9.4` | `MIT` | dev |
| `own-keys` | `1.0.2` | `MIT` | dev |
| `p-limit` | `3.1.0` | `MIT` | dev |
| `p-locate` | `5.0.0` | `MIT` | dev |
| `parent-module` | `1.0.1` | `MIT` | dev |
| `parse5` | `7.3.0` | `MIT` | dev |
| `path-exists` | `4.0.0` | `MIT` | dev |
| `path-key` | `3.1.1` | `MIT` | dev |
| `pathe` | `2.0.3` | `MIT` | dev |
| `pathval` | `2.0.1` | `MIT` | dev |
| `picocolors` | `1.1.1` | `ISC` | dev |
| `picomatch` | `4.0.7` | `MIT` | dev |
| `possible-typed-array-names` | `1.1.0` | `MIT` | dev |
| `postcss` | `8.5.33` | `MIT` | dev |
| `prelude-ls` | `1.2.1` | `MIT` | dev |
| `prettier` | `3.10.1` | `MIT` | dev |
| `pretty-format` | `27.5.1` | `MIT` | dev |
| `punycode` | `2.3.1` | `MIT` | dev |
| `react-is` | `17.0.2` | `MIT` | dev |
| `react-refresh` | `0.17.0` | `MIT` | dev |
| `redent` | `3.0.0` | `MIT` | dev |
| `reflect.getprototypeof` | `1.0.10` | `MIT` | dev |
| `regexp.prototype.flags` | `1.5.4` | `MIT` | dev |
| `resolve-from` | `4.0.0` | `MIT` | dev |
| `rollup` | `4.68.5` | `MIT` | dev |
| `rrweb-cssom` | `0.8.0` | `MIT` | dev |
| `safe-array-concat` | `1.1.4` | `MIT` | dev |
| `safe-push-apply` | `1.0.0` | `MIT` | dev |
| `safe-regex-test` | `1.1.0` | `MIT` | dev |
| `safer-buffer` | `2.1.2` | `MIT` | dev |
| `saxes` | `6.0.0` | `ISC` | dev |
| `semver` | `6.3.2` | `ISC` | dev |
| `semver` | `7.8.6` | `ISC` | dev |
| `set-function-length` | `1.2.2` | `MIT` | dev |
| `set-function-name` | `2.0.2` | `MIT` | dev |
| `set-proto` | `1.0.0` | `MIT` | dev |
| `shebang-command` | `2.0.0` | `MIT` | dev |
| `shebang-regex` | `3.0.0` | `MIT` | dev |
| `side-channel` | `1.1.1` | `MIT` | dev |
| `side-channel-list` | `1.0.1` | `MIT` | dev |
| `side-channel-map` | `1.0.1` | `MIT` | dev |
| `side-channel-weakmap` | `1.0.2` | `MIT` | dev |
| `siginfo` | `2.0.0` | `ISC` | dev |
| `source-map-js` | `1.2.2` | `BSD-3-Clause` | dev |
| `stackback` | `0.0.2` | `MIT` | dev |
| `std-env` | `3.10.0` | `MIT` | dev |
| `stop-iteration-iterator` | `1.1.0` | `MIT` | dev |
| `string.prototype.includes` | `2.0.1` | `MIT` | dev |
| `string.prototype.trim` | `1.2.11` | `MIT` | dev |
| `string.prototype.trimend` | `1.0.10` | `MIT` | dev |
| `string.prototype.trimstart` | `1.0.8` | `MIT` | dev |
| `strip-indent` | `3.0.0` | `MIT` | dev |
| `strip-json-comments` | `3.1.1` | `MIT` | dev |
| `strip-literal` | `3.1.0` | `MIT` | dev |
| `supports-color` | `7.2.0` | `MIT` | dev |
| `symbol-tree` | `3.2.4` | `MIT` | dev |
| `tinybench` | `2.9.0` | `MIT` | dev |
| `tinyexec` | `0.3.2` | `MIT` | dev |
| `tinyglobby` | `0.2.17` | `MIT` | dev |
| `tinypool` | `1.1.1` | `MIT` | dev |
| `tinyrainbow` | `2.0.0` | `MIT` | dev |
| `tinyspy` | `4.0.6` | `MIT` | dev |
| `tldts` | `6.1.86` | `MIT` | dev |
| `tldts-core` | `6.1.86` | `MIT` | dev |
| `tough-cookie` | `5.1.2` | `BSD-3-Clause` | dev |
| `tr46` | `5.1.1` | `MIT` | dev |
| `ts-api-utils` | `2.5.0` | `MIT` | dev |
| `type-check` | `0.4.0` | `MIT` | dev |
| `typed-array-buffer` | `1.0.3` | `MIT` | dev |
| `typed-array-byte-length` | `1.0.3` | `MIT` | dev |
| `typed-array-byte-offset` | `1.0.5` | `MIT` | dev |
| `typed-array-length` | `1.0.8` | `MIT` | dev |
| `typescript` | `5.9.3` | `Apache-2.0` | dev |
| `typescript-eslint` | `8.73.0` | `MIT` | dev |
| `unbox-primitive` | `1.1.0` | `MIT` | dev |
| `update-browserslist-db` | `1.4.0` | `MIT` | dev |
| `uri-js` | `4.4.1` | `BSD-2-Clause` | dev |
| `vite` | `7.3.8` | `MIT` | dev |
| `vite-node` | `3.2.4` | `MIT` | dev |
| `vitest` | `3.2.7` | `MIT` | dev |
| `w3c-xmlserializer` | `5.0.0` | `MIT` | dev |
| `webidl-conversions` | `7.0.0` | `BSD-2-Clause` | dev |
| `whatwg-encoding` | `3.1.1` | `MIT` | dev |
| `whatwg-mimetype` | `4.0.0` | `MIT` | dev |
| `whatwg-url` | `14.2.0` | `MIT` | dev |
| `which` | `2.0.2` | `ISC` | dev |
| `which-boxed-primitive` | `1.1.1` | `MIT` | dev |
| `which-builtin-type` | `1.2.1` | `MIT` | dev |
| `which-collection` | `1.0.2` | `MIT` | dev |
| `which-typed-array` | `1.1.24` | `MIT` | dev |
| `why-is-node-running` | `2.3.0` | `MIT` | dev |
| `word-wrap` | `1.2.5` | `MIT` | dev |
| `ws` | `8.23.0` | `MIT` | dev |
| `xml-name-validator` | `5.0.0` | `Apache-2.0` | dev |
| `xmlchars` | `2.2.0` | `MIT` | dev |
| `yallist` | `3.1.1` | `ISC` | dev |
| `yocto-queue` | `0.1.0` | `MIT` | dev |

</details>

---

## 7. Inventaire complet des dépendances Rust (`Cargo.lock`, `615` crates externes)

<details>
<summary>Liste exhaustive des 615 crates externes de Cargo.lock (versions exactes, licences SPDX, source crates.io, statut actif cargo-deny)</summary>

| Crate | Version | Licence SPDX (`cargo metadata`) | Actif `cargo deny` | Source |
|---|---|---|---|---|
| `adler2` | `2.0.1` | `0BSD OR MIT OR Apache-2.0` | oui | `crates.io` |
| `ahash` | `0.8.12` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `aho-corasick` | `1.1.5` | `Unlicense OR MIT` | oui | `crates.io` |
| `alloc-no-stdlib` | `3.0.0` | `BSD-3-Clause` | oui | `crates.io` |
| `alloc-stdlib` | `0.3.0` | `BSD-3-Clause` | oui | `crates.io` |
| `allocator-api2` | `0.2.21` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `android_system_properties` | `0.1.6` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anstream` | `1.0.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anstyle` | `1.0.14` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anstyle-parse` | `1.0.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anstyle-query` | `1.1.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anstyle-wincon` | `3.0.11` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `anyhow` | `1.0.104` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `approx` | `0.5.1` | `Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `ar_archive_writer` | `0.5.5` | `Apache-2.0 WITH LLVM-exception` | non (cible/feature inactive) | `crates.io` |
| `arrayvec` | `0.7.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `atk` | `0.18.2` | `MIT` | oui | `crates.io` |
| `atk-sys` | `0.18.2` | `MIT` | oui | `crates.io` |
| `atomic` | `0.6.1` | `Apache-2.0/MIT` | non (cible/feature inactive) | `crates.io` |
| `atomic-waker` | `1.1.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `autocfg` | `1.5.1` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `base64` | `0.21.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `base64` | `0.22.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `base64` | `0.23.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `bit-set` | `0.11.1` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `bit-set` | `0.5.3` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `bit-set` | `0.8.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `bit-vec` | `0.10.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `bit-vec` | `0.6.3` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `bit-vec` | `0.8.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `bitflags` | `1.3.2` | `MIT/Apache-2.0` | oui | `crates.io` |
| `bitflags` | `2.13.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `block-buffer` | `0.10.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `block-buffer` | `0.12.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `block2` | `0.6.2` | `MIT` | oui | `crates.io` |
| `borrow-or-share` | `0.2.4` | `MIT-0` | non (cible/feature inactive) | `crates.io` |
| `borsh` | `1.8.1` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `borsh-derive` | `1.8.1` | `Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `brotli` | `9.1.3` | `BSD-3-Clause AND MIT` | oui | `crates.io` |
| `brotli-decompressor` | `6.3.2` | `BSD-3-Clause/MIT` | oui | `crates.io` |
| `bs58` | `0.5.1` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `bumpalo` | `3.20.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `by_address` | `1.2.1` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `bytecount` | `0.6.9` | `Apache-2.0/MIT` | non (cible/feature inactive) | `crates.io` |
| `bytemuck` | `1.25.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `byteorder` | `1.5.0` | `Unlicense OR MIT` | oui | `crates.io` |
| `bytes` | `1.12.1` | `MIT` | oui | `crates.io` |
| `cairo-rs` | `0.18.5` | `MIT` | oui | `crates.io` |
| `cairo-sys-rs` | `0.18.2` | `MIT` | oui | `crates.io` |
| `camino` | `1.2.6` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cargo-platform` | `0.1.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cargo_metadata` | `0.19.2` | `MIT` | oui | `crates.io` |
| `cargo_toml` | `1.0.1` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `castaway` | `0.2.4` | `MIT` | oui | `crates.io` |
| `cc` | `1.11.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cesu8` | `1.1.0` | `Apache-2.0/MIT` | oui | `crates.io` |
| `cfb` | `0.14.0` | `MIT` | oui | `crates.io` |
| `cfg-expr` | `0.15.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cfg-if` | `1.0.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cfg_aliases` | `0.1.1` | `MIT` | oui | `crates.io` |
| `cfg_aliases` | `0.2.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `chacha20` | `0.10.2` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `chrono` | `0.4.45` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `clap` | `4.6.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `clap_builder` | `4.6.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `clap_derive` | `4.6.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `clap_lex` | `1.1.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `colorchoice` | `1.0.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `combine` | `4.6.9` | `MIT` | oui | `crates.io` |
| `compact_str` | `0.9.1` | `MIT` | oui | `crates.io` |
| `const-oid` | `0.10.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `convert_case` | `0.10.0` | `MIT` | oui | `crates.io` |
| `cookie` | `0.16.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cookie` | `0.18.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `core-foundation` | `0.10.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `core-foundation-sys` | `0.8.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `core-graphics` | `0.25.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `core-graphics-types` | `0.2.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `core_detect` | `1.0.0` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `core_maths` | `0.1.1` | `MIT` | oui | `crates.io` |
| `cpufeatures` | `0.2.17` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `cpufeatures` | `0.3.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `crc32fast` | `1.5.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `critical-section` | `1.2.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `crossbeam-channel` | `0.5.17` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `crossbeam-utils` | `0.8.23` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `crossterm` | `0.29.0` | `MIT` | oui | `crates.io` |
| `crossterm_winapi` | `0.9.1` | `MIT` | oui | `crates.io` |
| `crypto-common` | `0.1.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `crypto-common` | `0.2.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `csscolorparser` | `0.6.2` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `cssparser` | `0.37.0` | `MPL-2.0` | oui | `crates.io` |
| `cssparser` | `0.38.0` | `MPL-2.0` | oui | `crates.io` |
| `cssparser-macros` | `0.7.1` | `MPL-2.0` | oui | `crates.io` |
| `ctor` | `1.0.14` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `darling` | `0.24.1` | `MIT` | oui | `crates.io` |
| `darling_core` | `0.24.1` | `MIT` | oui | `crates.io` |
| `darling_macro` | `0.24.1` | `MIT` | oui | `crates.io` |
| `data-encoding` | `2.11.1` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `dbus` | `0.9.12` | `Apache-2.0/MIT` | oui | `crates.io` |
| `deltae` | `0.3.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `deranged` | `0.5.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `derive_more` | `2.1.1` | `MIT` | oui | `crates.io` |
| `derive_more-impl` | `2.1.1` | `MIT` | oui | `crates.io` |
| `digest` | `0.10.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `digest` | `0.11.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dirs` | `7.0.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dirs-sys` | `0.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dispatch2` | `0.3.1` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `displaydoc` | `0.2.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dlopen2` | `0.8.2` | `MIT` | oui | `crates.io` |
| `dlopen2_derive` | `0.4.3` | `MIT` | oui | `crates.io` |
| `document-features` | `0.2.12` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dom_query` | `0.28.0` | `MIT` | oui | `crates.io` |
| `dom_query` | `0.29.0` | `MIT` | oui | `crates.io` |
| `dotenvy` | `0.15.7` | `MIT` | oui | `crates.io` |
| `downcast-rs` | `1.2.1` | `MIT/Apache-2.0` | oui | `crates.io` |
| `dpi` | `0.1.2` | `Apache-2.0 AND MIT` | oui | `crates.io` |
| `dtoa` | `1.0.11` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `dtoa-short` | `0.3.5` | `MPL-2.0` | oui | `crates.io` |
| `dunce` | `1.0.5` | `CC0-1.0 OR MIT-0 OR Apache-2.0` | oui | `crates.io` |
| `dyn-clone` | `1.0.20` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `either` | `1.19.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `email_address` | `0.2.10` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `embed-resource` | `3.0.12` | `MIT` | oui | `crates.io` |
| `embed_plist` | `1.2.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `equivalent` | `1.0.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `erased-serde` | `0.4.11` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `errno` | `0.3.15` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `euclid` | `0.22.14` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `fallible-iterator` | `0.3.0` | `MIT/Apache-2.0` | oui | `crates.io` |
| `fallible-streaming-iterator` | `0.1.9` | `MIT/Apache-2.0` | oui | `crates.io` |
| `fancy-regex` | `0.11.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `fancy-regex` | `0.20.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `fantoccini` | `0.22.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `fastrand` | `2.5.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `fdeflate` | `0.3.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `field-offset` | `0.3.6` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `filedescriptor` | `0.8.3` | `MIT` | oui | `crates.io` |
| `find-msvc-tools` | `0.1.15` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `finl_unicode` | `1.5.1` | `(MIT OR Apache-2.0) AND Unicode-DFS-2016` | non (cible/feature inactive) | `crates.io` |
| `fixedbitset` | `0.4.2` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `flate2` | `1.1.11` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `fluent-uri` | `0.4.1` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `fnv` | `1.0.7` | `Apache-2.0 / MIT` | oui | `crates.io` |
| `foldhash` | `0.2.0` | `Zlib` | oui | `crates.io` |
| `foreign-types` | `0.5.0` | `MIT/Apache-2.0` | oui | `crates.io` |
| `foreign-types-macros` | `0.2.4` | `MIT/Apache-2.0` | oui | `crates.io` |
| `foreign-types-shared` | `0.3.1` | `MIT/Apache-2.0` | oui | `crates.io` |
| `form_urlencoded` | `1.2.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `fraction` | `0.17.0` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `futures-channel` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-core` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-executor` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-io` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-macro` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-sink` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-task` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `futures-util` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `gdk` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gdk-pixbuf` | `0.18.5` | `MIT` | oui | `crates.io` |
| `gdk-pixbuf-sys` | `0.18.0` | `MIT` | oui | `crates.io` |
| `gdk-sys` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gdkwayland-sys` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gdkx11` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gdkx11-sys` | `0.18.2` | `MIT` | oui | `crates.io` |
| `generic-array` | `0.14.7` | `MIT` | oui | `crates.io` |
| `getrandom` | `0.3.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `getrandom` | `0.4.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `gio` | `0.18.4` | `MIT` | oui | `crates.io` |
| `gio-sys` | `0.18.1` | `MIT` | oui | `crates.io` |
| `glib` | `0.18.5` | `MIT` | oui | `crates.io` |
| `glib-macros` | `0.18.5` | `MIT` | oui | `crates.io` |
| `glib-sys` | `0.18.1` | `MIT` | oui | `crates.io` |
| `glob` | `0.3.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `gobject-sys` | `0.18.0` | `MIT` | oui | `crates.io` |
| `gtk` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gtk-sys` | `0.18.2` | `MIT` | oui | `crates.io` |
| `gtk3-macros` | `0.18.2` | `MIT` | oui | `crates.io` |
| `h2` | `0.4.20` | `MIT` | oui | `crates.io` |
| `hashbrown` | `0.12.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `hashbrown` | `0.17.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `hashlink` | `0.12.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `heck` | `0.4.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `heck` | `0.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `hex` | `0.4.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `html5ever` | `0.39.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `html5ever` | `0.40.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `http` | `1.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `http-body` | `1.1.0` | `MIT` | oui | `crates.io` |
| `http-body-util` | `0.1.5` | `MIT` | oui | `crates.io` |
| `httparse` | `1.10.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `httpdate` | `1.0.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `hybrid-array` | `0.4.15` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `hyper` | `1.12.0` | `MIT` | oui | `crates.io` |
| `hyper-util` | `0.1.21` | `MIT` | oui | `crates.io` |
| `iana-time-zone` | `0.1.65` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `iana-time-zone-haiku` | `0.1.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `ico` | `0.6.0` | `MIT` | oui | `crates.io` |
| `icu_collections` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_locale_core` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_locale_fallback` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_locale_fallback_data` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_normalizer` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_normalizer_data` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_properties` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_properties_data` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_provider` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_segmenter` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `icu_segmenter_data` | `2.4.0` | `Unicode-3.0` | oui | `crates.io` |
| `ident_case` | `1.0.1` | `MIT/Apache-2.0` | oui | `crates.io` |
| `idna` | `1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `idna_adapter` | `1.2.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `indexmap` | `1.9.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `indexmap` | `2.14.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `indoc` | `2.0.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `infer` | `0.22.0` | `MIT` | oui | `crates.io` |
| `instability` | `0.3.14` | `MIT` | oui | `crates.io` |
| `ipnet` | `2.12.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `is_terminal_polyfill` | `1.70.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `itertools` | `0.14.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `itoa` | `1.0.19` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `javascriptcore-rs` | `1.1.2` | `MIT` | oui | `crates.io` |
| `javascriptcore-rs-sys` | `1.1.1` | `MIT` | oui | `crates.io` |
| `jiff` | `0.2.46` | `Unlicense OR MIT` | non (cible/feature inactive) | `crates.io` |
| `jiff-core` | `0.1.8` | `Unlicense OR MIT` | non (cible/feature inactive) | `crates.io` |
| `jiff-static` | `0.2.46` | `Unlicense OR MIT` | non (cible/feature inactive) | `crates.io` |
| `jiff-tzdb` | `0.1.10` | `Unlicense OR MIT` | non (cible/feature inactive) | `crates.io` |
| `jiff-tzdb-platform` | `0.1.3` | `Unlicense OR MIT` | non (cible/feature inactive) | `crates.io` |
| `jni` | `0.21.1` | `MIT/Apache-2.0` | oui | `crates.io` |
| `jni-sys` | `0.3.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `jni-sys` | `0.4.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `jni-sys-macros` | `0.4.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `js-sys` | `0.3.107` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `json-patch` | `4.2.0` | `MIT/Apache-2.0` | oui | `crates.io` |
| `jsonptr` | `0.7.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `jsonschema` | `0.61.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `jsonschema-regex` | `0.61.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `jsonschema-value` | `0.61.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `kasuari` | `0.4.13` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `keyboard-types` | `0.8.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `lab` | `0.11.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `lazy_static` | `1.5.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `libappindicator` | `0.9.0` | `Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `libappindicator-sys` | `0.9.0` | `Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `libc` | `0.2.191` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `libdbus-sys` | `0.2.7` | `Apache-2.0/MIT` | oui | `crates.io` |
| `libloading` | `0.7.4` | `ISC` | non (cible/feature inactive) | `crates.io` |
| `libm` | `0.2.16` | `MIT` | oui | `crates.io` |
| `libredox` | `0.1.25` | `MIT` | oui | `crates.io` |
| `libsqlite3-sys` | `0.40.0` | `MIT` | oui | `crates.io` |
| `line-clipping` | `0.3.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `link-section` | `0.19.4` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `linux-raw-sys` | `0.12.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `litemap` | `0.8.3` | `Unicode-3.0` | oui | `crates.io` |
| `litrs` | `1.0.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `lock_api` | `0.4.14` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `log` | `0.4.35` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `lru` | `0.18.5` | `MIT` | oui | `crates.io` |
| `mac_address` | `1.2.1` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `markup5ever` | `0.39.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `markup5ever` | `0.40.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `memchr` | `2.8.3` | `Unlicense OR MIT` | oui | `crates.io` |
| `memmem` | `0.1.1` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `memoffset` | `0.9.1` | `MIT` | oui | `crates.io` |
| `micromap` | `0.3.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `mime` | `0.3.18` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `minimal-lexical` | `0.2.1` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `miniz_oxide` | `0.8.9` | `MIT OR Zlib OR Apache-2.0` | oui | `crates.io` |
| `miniz_oxide` | `0.9.1` | `MIT OR Zlib OR Apache-2.0` | oui | `crates.io` |
| `mio` | `1.2.5` | `MIT` | oui | `crates.io` |
| `muda` | `0.20.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `ndk` | `0.9.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `ndk-context` | `0.1.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `ndk-sys` | `0.6.0+11769913` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `new_debug_unreachable` | `1.0.6` | `MIT` | oui | `crates.io` |
| `nix` | `0.28.0` | `MIT` | oui | `crates.io` |
| `nix` | `0.29.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `nix` | `0.31.4` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `nom` | `7.1.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `num` | `0.4.3` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-bigint` | `0.4.8` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-cmp` | `0.1.0` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-complex` | `0.4.6` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-conv` | `0.2.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `num-derive` | `0.4.2` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-integer` | `0.1.47` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-iter` | `0.1.46` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-rational` | `0.4.2` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `num-traits` | `0.2.19` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `num_enum` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | oui | `crates.io` |
| `num_enum_derive` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | oui | `crates.io` |
| `num_threads` | `0.1.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `objc2` | `0.6.5` | `MIT` | oui | `crates.io` |
| `objc2-app-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-cloud-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-data` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-foundation` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-graphics` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-image` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-location` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-core-text` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-encode` | `4.1.0` | `MIT` | oui | `crates.io` |
| `objc2-exception-helper` | `0.1.1` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-foundation` | `0.3.2` | `MIT` | oui | `crates.io` |
| `objc2-io-surface` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `objc2-quartz-core` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-ui-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-user-notifications` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `objc2-web-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `object` | `0.39.1` | `Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `once_cell` | `1.21.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `once_cell_polyfill` | `1.70.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `option-ext` | `0.2.0` | `MPL-2.0` | oui | `crates.io` |
| `ordered-float` | `4.6.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `outref` | `0.5.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `palette` | `0.7.7` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `palette_derive` | `0.7.7` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `palette_math` | `0.7.7` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pango` | `0.18.3` | `MIT` | oui | `crates.io` |
| `pango-sys` | `0.18.0` | `MIT` | oui | `crates.io` |
| `parking_lot` | `0.12.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `parking_lot_core` | `0.9.12` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `percent-encoding` | `2.3.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `pest` | `2.9.6` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pest_derive` | `2.9.6` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pest_generator` | `2.9.6` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pest_meta` | `2.9.6` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pest_stack_guard` | `1.0.1` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `phf` | `0.11.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `phf` | `0.13.1` | `MIT` | oui | `crates.io` |
| `phf` | `0.14.0` | `MIT` | oui | `crates.io` |
| `phf_codegen` | `0.11.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `phf_codegen` | `0.13.1` | `MIT` | oui | `crates.io` |
| `phf_codegen` | `0.14.0` | `MIT` | oui | `crates.io` |
| `phf_generator` | `0.11.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `phf_generator` | `0.13.1` | `MIT` | oui | `crates.io` |
| `phf_generator` | `0.14.0` | `MIT` | oui | `crates.io` |
| `phf_macros` | `0.11.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `phf_macros` | `0.13.1` | `MIT` | oui | `crates.io` |
| `phf_macros` | `0.14.0` | `MIT` | oui | `crates.io` |
| `phf_shared` | `0.11.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `phf_shared` | `0.13.1` | `MIT` | oui | `crates.io` |
| `phf_shared` | `0.14.0` | `MIT` | oui | `crates.io` |
| `pin-project-lite` | `0.2.17` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `pkg-config` | `0.3.34` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `plist` | `1.10.4` | `MIT` | oui | `crates.io` |
| `png` | `0.18.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `portable-atomic` | `1.15.0` | `Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `portable-atomic-util` | `0.2.8` | `Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `portable-pty` | `0.9.0` | `MIT` | oui | `crates.io` |
| `potential_utf` | `0.1.6` | `Unicode-3.0` | oui | `crates.io` |
| `powerfmt` | `0.2.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `precomputed-hash` | `0.1.1` | `MIT` | oui | `crates.io` |
| `proc-macro-crate` | `1.3.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proc-macro-crate` | `2.0.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proc-macro-crate` | `3.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proc-macro-error` | `1.0.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proc-macro-error-attr` | `1.0.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proc-macro2` | `1.0.107` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `proptest` | `1.12.0` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `psm` | `0.1.32` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `pulldown-cmark` | `0.13.6` | `MIT` | oui | `crates.io` |
| `quick-error` | `1.2.3` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `quick-xml` | `0.42.0` | `MIT` | oui | `crates.io` |
| `quote` | `1.0.47` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `r-efi` | `5.3.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | oui | `crates.io` |
| `r-efi` | `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | oui | `crates.io` |
| `rand` | `0.10.4` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `rand` | `0.8.8` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `rand_core` | `0.10.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `rand_core` | `0.6.4` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `rand_xorshift` | `0.5.0` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `ratatui` | `0.30.2` | `MIT` | oui | `crates.io` |
| `ratatui-core` | `0.1.2` | `MIT` | oui | `crates.io` |
| `ratatui-crossterm` | `0.1.2` | `MIT` | oui | `crates.io` |
| `ratatui-macros` | `0.7.2` | `MIT` | oui | `crates.io` |
| `ratatui-termina` | `0.1.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `ratatui-termwiz` | `0.1.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `ratatui-widgets` | `0.3.2` | `MIT` | oui | `crates.io` |
| `raw-window-handle` | `0.6.2` | `MIT OR Apache-2.0 OR Zlib` | oui | `crates.io` |
| `redox_syscall` | `0.5.18` | `MIT` | oui | `crates.io` |
| `redox_users` | `0.5.3` | `MIT` | oui | `crates.io` |
| `ref-cast` | `1.0.27` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `ref-cast-impl` | `1.0.27` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `referencing` | `0.61.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `regex` | `1.13.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `regex-automata` | `0.4.18` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `regex-syntax` | `0.8.11` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `reqwest` | `0.13.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `ropey` | `1.6.1` | `MIT` | oui | `crates.io` |
| `rsqlite-vfs` | `0.2.0` | `MIT` | oui | `crates.io` |
| `rusqlite` | `0.43.0` | `MIT` | oui | `crates.io` |
| `rustc-hash` | `2.1.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `rustc_version` | `0.4.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `rustix` | `1.1.6` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `rustversion` | `1.0.23` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `rusty-fork` | `0.3.1` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `ryu` | `1.0.23` | `Apache-2.0 OR BSL-1.0` | oui | `crates.io` |
| `ryu-js` | `0.2.2` | `Apache-2.0 OR BSL-1.0` | oui | `crates.io` |
| `same-file` | `1.0.6` | `Unlicense/MIT` | oui | `crates.io` |
| `schemars` | `0.8.22` | `MIT` | oui | `crates.io` |
| `schemars` | `0.9.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `schemars` | `1.2.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `schemars_derive` | `0.8.22` | `MIT` | oui | `crates.io` |
| `scopeguard` | `1.2.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `secrecy` | `0.10.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `selectors` | `0.38.0` | `MPL-2.0` | oui | `crates.io` |
| `selectors` | `0.41.0` | `MPL-2.0` | oui | `crates.io` |
| `semver` | `1.0.28` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde-untagged` | `0.1.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_core` | `1.0.229` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_derive` | `1.0.229` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_derive_internals` | `0.29.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_jcs` | `0.2.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_json` | `1.0.154` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_repr` | `0.1.21` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_spanned` | `0.6.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_spanned` | `1.1.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_with` | `3.24.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serde_with_macros` | `3.24.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serial2` | `0.2.39` | `BSD-2-Clause OR Apache-2.0` | oui | `crates.io` |
| `serialize-to-javascript` | `0.1.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `serialize-to-javascript-impl` | `0.1.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `servo_arc` | `0.4.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `servo_arc` | `0.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `sha2` | `0.10.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `sha2` | `0.11.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `shared_library` | `0.1.9` | `Apache-2.0/MIT` | oui | `crates.io` |
| `shell-words` | `1.1.1` | `MIT/Apache-2.0` | oui | `crates.io` |
| `shlex` | `2.0.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `signal-hook` | `0.3.18` | `Apache-2.0/MIT` | oui | `crates.io` |
| `signal-hook-mio` | `0.2.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `signal-hook-registry` | `1.4.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `simd-adler32` | `0.3.10` | `MIT` | oui | `crates.io` |
| `siphasher` | `1.0.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `slab` | `0.4.12` | `MIT` | oui | `crates.io` |
| `smallvec` | `1.16.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `socket2` | `0.6.5` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `softbuffer` | `0.4.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `soup3` | `0.5.0` | `MIT` | oui | `crates.io` |
| `soup3-sys` | `0.5.0` | `MIT` | oui | `crates.io` |
| `sqlite-wasm-rs` | `0.6.3` | `MIT` | oui | `crates.io` |
| `stable_deref_trait` | `1.2.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `stacker` | `0.1.25` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `static_assertions` | `1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `static_vcruntime` | `3.0.0` | `MIT OR Apache-2.0 OR Zlib` | oui | `crates.io` |
| `str_indices` | `0.4.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `string_cache` | `0.11.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `string_cache` | `0.9.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `string_cache_codegen` | `0.11.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `string_cache_codegen` | `0.6.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `strsim` | `0.11.1` | `MIT` | oui | `crates.io` |
| `strum` | `0.28.0` | `MIT` | oui | `crates.io` |
| `strum_macros` | `0.28.0` | `MIT` | oui | `crates.io` |
| `swift-rs` | `1.0.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `syn` | `1.0.109` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `syn` | `2.0.119` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `syn` | `3.0.10` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `sync_wrapper` | `1.0.2` | `Apache-2.0` | oui | `crates.io` |
| `synstructure` | `0.14.0` | `MIT` | oui | `crates.io` |
| `system-deps` | `6.2.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `tao` | `0.37.2` | `Apache-2.0` | oui | `crates.io` |
| `tao-macros` | `0.1.4` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `target-lexicon` | `0.12.16` | `Apache-2.0 WITH LLVM-exception` | oui | `crates.io` |
| `tauri` | `2.12.6` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-build` | `2.7.5` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-codegen` | `2.7.5` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-macros` | `2.7.5` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-runtime` | `2.12.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-runtime-wry` | `2.12.4` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-utils` | `2.10.3` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `tauri-winres` | `0.3.7` | `MIT` | oui | `crates.io` |
| `tempfile` | `3.27.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `tendril` | `0.5.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `termcolor` | `1.4.1` | `Unlicense OR MIT` | oui | `crates.io` |
| `termina` | `0.3.3` | `MIT OR MPL-2.0` | non (cible/feature inactive) | `crates.io` |
| `terminfo` | `0.9.0` | `WTFPL` | non (cible/feature inactive) | `crates.io` |
| `termios` | `0.3.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `termwiz` | `0.23.3` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `thiserror` | `1.0.69` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `thiserror` | `2.0.21` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `thiserror-impl` | `1.0.69` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `thiserror-impl` | `2.0.21` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `time` | `0.3.55` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `time-core` | `0.1.9` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `time-macros` | `0.2.32` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `tinystr` | `0.8.5` | `Unicode-3.0` | oui | `crates.io` |
| `tinyvec` | `1.13.3` | `Zlib OR Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `tokio` | `1.54.1` | `MIT` | oui | `crates.io` |
| `tokio-macros` | `2.7.2` | `MIT` | oui | `crates.io` |
| `tokio-util` | `0.7.20` | `MIT` | oui | `crates.io` |
| `toml` | `0.8.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml` | `1.2.1+spec-1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_datetime` | `0.6.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_datetime` | `1.1.3+spec-1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_edit` | `0.19.15` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_edit` | `0.20.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_edit` | `0.25.19+spec-1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_parser` | `1.1.5+spec-1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `toml_writer` | `1.2.0+spec-1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `tower` | `0.5.3` | `MIT` | oui | `crates.io` |
| `tower-http` | `0.6.11` | `MIT` | oui | `crates.io` |
| `tower-layer` | `0.3.3` | `MIT` | oui | `crates.io` |
| `tower-service` | `0.3.3` | `MIT` | oui | `crates.io` |
| `tracing` | `0.1.44` | `MIT` | oui | `crates.io` |
| `tracing-attributes` | `0.1.31` | `MIT` | oui | `crates.io` |
| `tracing-core` | `0.1.36` | `MIT` | oui | `crates.io` |
| `tray-icon` | `0.25.1` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `try-lock` | `0.2.5` | `MIT` | oui | `crates.io` |
| `ts-rs` | `12.0.1` | `MIT` | oui | `crates.io` |
| `ts-rs-macros` | `12.0.1` | `MIT` | oui | `crates.io` |
| `tui-term` | `0.3.4` | `MIT` | oui | `crates.io` |
| `typeid` | `1.0.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `typenum` | `1.20.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `ucd-trie` | `0.1.7` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `unarray` | `0.1.4` | `MIT OR Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `unicase` | `2.10.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `unicode-general-category` | `1.1.0` | `Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `unicode-ident` | `1.0.27` | `(MIT OR Apache-2.0) AND Unicode-3.0` | oui | `crates.io` |
| `unicode-segmentation` | `1.13.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `unicode-truncate` | `2.0.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `unicode-width` | `0.2.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `url` | `2.5.8` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `urlpattern` | `0.6.0` | `MIT` | oui | `crates.io` |
| `utf8_iter` | `1.1.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `utf8parse` | `0.2.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `utf_types` | `1.0.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `uuid` | `1.29.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `uuid-simd` | `0.8.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `vcpkg` | `0.2.15` | `MIT/Apache-2.0` | oui | `crates.io` |
| `version-compare` | `0.2.1` | `MIT` | oui | `crates.io` |
| `version_check` | `0.9.5` | `MIT/Apache-2.0` | oui | `crates.io` |
| `vsimd` | `0.8.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `vswhom` | `0.1.0` | `MIT` | oui | `crates.io` |
| `vswhom-sys` | `0.1.3` | `MIT` | oui | `crates.io` |
| `vt100` | `0.16.2` | `MIT` | oui | `crates.io` |
| `vte` | `0.15.0` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `vtparse` | `0.6.2` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `wait-timeout` | `0.2.1` | `MIT/Apache-2.0` | non (cible/feature inactive) | `crates.io` |
| `walkdir` | `2.5.0` | `Unlicense/MIT` | oui | `crates.io` |
| `want` | `0.3.2` | `MIT` | oui | `crates.io` |
| `wasi` | `0.11.1+wasi-snapshot-preview1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `wasip2` | `1.0.4+wasi-0.2.12` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `wasm-bindgen` | `0.2.130` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `wasm-bindgen-futures` | `0.4.80` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `wasm-bindgen-macro` | `0.2.130` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `wasm-bindgen-macro-support` | `0.2.130` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `wasm-bindgen-shared` | `0.2.130` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `wasm-streams` | `0.5.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `web-sys` | `0.3.107` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `web-time` | `1.1.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `web_atoms` | `0.2.6` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `web_atoms` | `0.3.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `webdriver` | `0.54.0` | `MPL-2.0` | oui | `crates.io` |
| `webkit2gtk` | `2.0.2` | `MIT` | oui | `crates.io` |
| `webkit2gtk-sys` | `2.0.2` | `MIT` | oui | `crates.io` |
| `webview2-com` | `0.39.1` | `MIT` | oui | `crates.io` |
| `webview2-com-macros` | `0.8.1` | `MIT` | oui | `crates.io` |
| `webview2-com-sys` | `0.39.1` | `MIT` | oui | `crates.io` |
| `wezterm-bidi` | `0.2.3` | `MIT AND Unicode-DFS-2016` | non (cible/feature inactive) | `crates.io` |
| `wezterm-blob-leases` | `0.1.1` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `wezterm-color-types` | `0.3.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `wezterm-dynamic` | `0.2.1` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `wezterm-dynamic-derive` | `0.1.1` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `wezterm-input-types` | `0.1.0` | `MIT` | non (cible/feature inactive) | `crates.io` |
| `winapi` | `0.3.9` | `MIT/Apache-2.0` | oui | `crates.io` |
| `winapi-i686-pc-windows-gnu` | `0.4.0` | `MIT/Apache-2.0` | oui | `crates.io` |
| `winapi-util` | `0.1.11` | `Unlicense OR MIT` | oui | `crates.io` |
| `winapi-x86_64-pc-windows-gnu` | `0.4.0` | `MIT/Apache-2.0` | oui | `crates.io` |
| `window-vibrancy` | `0.8.1` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `windows` | `0.62.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-collections` | `0.3.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-core` | `0.62.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-future` | `0.3.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-implement` | `0.60.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-interface` | `0.59.3` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-link` | `0.2.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-numerics` | `0.3.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-result` | `0.4.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-strings` | `0.5.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-sys` | `0.45.0` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-sys` | `0.61.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-targets` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-threading` | `0.2.1` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows-version` | `0.1.7` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_aarch64_gnullvm` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_aarch64_msvc` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_i686_gnu` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_i686_msvc` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_x86_64_gnu` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_x86_64_gnullvm` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `windows_x86_64_msvc` | `0.42.2` | `MIT OR Apache-2.0` | oui | `crates.io` |
| `winnow` | `0.5.40` | `MIT` | oui | `crates.io` |
| `winnow` | `1.0.5` | `MIT` | oui | `crates.io` |
| `winreg` | `0.10.1` | `MIT` | oui | `crates.io` |
| `winreg` | `0.56.0` | `MIT` | oui | `crates.io` |
| `wit-bindgen` | `0.57.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | oui | `crates.io` |
| `writeable` | `0.6.4` | `Unicode-3.0` | oui | `crates.io` |
| `wry` | `0.57.2` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `x11` | `2.21.0` | `MIT` | oui | `crates.io` |
| `x11-dl` | `2.21.0` | `MIT` | oui | `crates.io` |
| `yoke` | `0.8.4` | `Unicode-3.0` | oui | `crates.io` |
| `yoke-derive` | `0.8.4` | `Unicode-3.0` | oui | `crates.io` |
| `zerocopy` | `0.8.63` | `BSD-2-Clause OR Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `zerocopy-derive` | `0.8.63` | `BSD-2-Clause OR Apache-2.0 OR MIT` | non (cible/feature inactive) | `crates.io` |
| `zerofrom` | `0.1.8` | `Unicode-3.0` | oui | `crates.io` |
| `zerofrom-derive` | `0.1.8` | `Unicode-3.0` | oui | `crates.io` |
| `zeroize` | `1.9.1` | `Apache-2.0 OR MIT` | oui | `crates.io` |
| `zerotrie` | `0.2.5` | `Unicode-3.0` | oui | `crates.io` |
| `zerovec` | `0.11.9` | `Unicode-3.0` | oui | `crates.io` |
| `zerovec-derive` | `0.11.7` | `Unicode-3.0` | oui | `crates.io` |
| `zlib-rs` | `0.6.8` | `Zlib` | non (cible/feature inactive) | `crates.io` |
| `zmij` | `1.0.23` | `MIT` | oui | `crates.io` |

</details>

<details>
<summary>Arbre normal du workspace (`cargo tree --workspace --edges normal --format "{p} ({l})"`, 992 lignes)</summary>

```text
unleash-browser v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-browser) (Apache-2.0)
├── bytes v1.12.1 (MIT)
├── fantoccini v0.22.2 (MIT OR Apache-2.0)
│   ├── base64 v0.23.1 (MIT OR Apache-2.0)
│   ├── cookie v0.18.2 (MIT OR Apache-2.0)
│   │   ├── percent-encoding v2.3.2 (MIT OR Apache-2.0)
│   │   └── time v0.3.55 (MIT OR Apache-2.0)
│   │       ├── deranged v0.5.9 (MIT OR Apache-2.0)
│   │       ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │       ├── num-conv v0.2.2 (MIT OR Apache-2.0)
│   │       ├── num_threads v0.1.8 (MIT OR Apache-2.0)
│   │       │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │       ├── powerfmt v0.2.1 (MIT OR Apache-2.0)
│   │       ├── time-core v0.1.9 (MIT OR Apache-2.0)
│   │       └── time-macros v0.2.32 (proc-macro) (MIT OR Apache-2.0)
│   │           ├── num-conv v0.2.2 (MIT OR Apache-2.0)
│   │           └── time-core v0.1.9 (MIT OR Apache-2.0)
│   ├── http v1.5.0 (MIT OR Apache-2.0)
│   │   ├── bytes v1.12.1 (MIT)
│   │   └── itoa v1.0.19 (MIT OR Apache-2.0)
│   ├── http-body-util v0.1.5 (MIT)
│   │   ├── bytes v1.12.1 (MIT)
│   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   ├── http-body v1.1.0 (MIT)
│   │   │   ├── bytes v1.12.1 (MIT)
│   │   │   └── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   └── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   ├── hyper v1.12.0 (MIT)
│   │   ├── atomic-waker v1.1.2 (Apache-2.0 OR MIT)
│   │   ├── bytes v1.12.1 (MIT)
│   │   ├── futures-channel v0.3.34 (MIT OR Apache-2.0)
│   │   │   └── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   ├── h2 v0.4.20 (MIT)
│   │   │   ├── atomic-waker v1.1.2 (Apache-2.0 OR MIT)
│   │   │   ├── bytes v1.12.1 (MIT)
│   │   │   ├── fnv v1.0.7 (Apache-2.0 / MIT)
│   │   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   │   ├── futures-sink v0.3.34 (MIT OR Apache-2.0)
│   │   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   │   ├── indexmap v2.14.2 (Apache-2.0 OR MIT)
│   │   │   │   ├── equivalent v1.0.3 (Apache-2.0 OR MIT)
│   │   │   │   └── hashbrown v0.17.1 (MIT OR Apache-2.0)
│   │   │   │       ├── allocator-api2 v0.2.21 (MIT OR Apache-2.0)
│   │   │   │       ├── equivalent v1.0.3 (Apache-2.0 OR MIT)
│   │   │   │       └── foldhash v0.2.0 (Zlib)
│   │   │   ├── slab v0.4.12 (MIT)
│   │   │   ├── tokio v1.54.1 (MIT)
│   │   │   │   ├── bytes v1.12.1 (MIT)
│   │   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   ├── mio v1.2.5 (MIT)
│   │   │   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   │   └── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   │   ├── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   │   │   ├── signal-hook-registry v1.4.8 (MIT OR Apache-2.0)
│   │   │   │   │   ├── errno v0.3.15 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   ├── socket2 v0.6.5 (MIT OR Apache-2.0)
│   │   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   └── tokio-macros v2.7.2 (proc-macro) (MIT)
│   │   │   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0)
│   │   │   │       │   └── unicode-ident v1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0)
│   │   │   │       ├── quote v1.0.47 (MIT OR Apache-2.0)
│   │   │   │       │   └── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │       └── syn v3.0.10 (MIT OR Apache-2.0)
│   │   │   │           ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │           ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │           └── unicode-ident v1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0)
│   │   │   ├── tokio-util v0.7.20 (MIT)
│   │   │   │   ├── bytes v1.12.1 (MIT)
│   │   │   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   │   │   ├── futures-sink v0.3.34 (MIT OR Apache-2.0)
│   │   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   ├── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   │   │   └── tokio v1.54.1 (MIT) (*)
│   │   │   └── tracing v0.1.44 (MIT)
│   │   │       ├── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   │       ├── tracing-attributes v0.1.31 (proc-macro) (MIT)
│   │   │       │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │       │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │       │   └── syn v2.0.119 (MIT OR Apache-2.0)
│   │   │       │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │       │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │       │       └── unicode-ident v1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0)
│   │   │       └── tracing-core v0.1.36 (MIT)
│   │   │           └── once_cell v1.21.4 (MIT OR Apache-2.0)
│   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   ├── http-body v1.1.0 (MIT) (*)
│   │   ├── httparse v1.10.1 (MIT OR Apache-2.0)
│   │   ├── httpdate v1.0.3 (MIT OR Apache-2.0)
│   │   ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │   ├── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   ├── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   ├── tokio v1.54.1 (MIT) (*)
│   │   └── want v0.3.2 (MIT)
│   │       └── try-lock v0.2.5 (MIT)
│   ├── hyper-util v0.1.21 (MIT)
│   │   ├── bytes v1.12.1 (MIT)
│   │   ├── futures-channel v0.3.34 (MIT OR Apache-2.0) (*)
│   │   ├── futures-util v0.3.34 (MIT OR Apache-2.0)
│   │   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   │   ├── futures-task v0.3.34 (MIT OR Apache-2.0)
│   │   │   └── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   ├── http-body v1.1.0 (MIT) (*)
│   │   ├── httparse v1.10.1 (MIT OR Apache-2.0)
│   │   ├── hyper v1.12.0 (MIT) (*)
│   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   ├── pin-project-lite v0.2.17 (Apache-2.0 OR MIT)
│   │   ├── socket2 v0.6.5 (MIT OR Apache-2.0) (*)
│   │   ├── tokio v1.54.1 (MIT) (*)
│   │   ├── tower-service v0.3.3 (MIT)
│   │   └── tracing v0.1.44 (MIT) (*)
│   ├── mime v0.3.18 (MIT OR Apache-2.0)
│   ├── serde v1.0.229 (MIT OR Apache-2.0)
│   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   └── serde_derive v1.0.229 (proc-macro) (MIT OR Apache-2.0)
│   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0)
│   │   ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │   ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   └── zmij v1.0.23 (MIT)
│   ├── time v0.3.55 (MIT OR Apache-2.0) (*)
│   ├── tokio v1.54.1 (MIT) (*)
│   ├── url v2.5.8 (MIT OR Apache-2.0)
│   │   ├── form_urlencoded v1.2.2 (MIT OR Apache-2.0)
│   │   │   └── percent-encoding v2.3.2 (MIT OR Apache-2.0)
│   │   ├── idna v1.1.0 (MIT OR Apache-2.0)
│   │   │   ├── idna_adapter v1.2.2 (Apache-2.0 OR MIT)
│   │   │   │   ├── icu_normalizer v2.4.0 (Unicode-3.0)
│   │   │   │   │   ├── icu_collections v2.4.0 (Unicode-3.0)
│   │   │   │   │   │   ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0)
│   │   │   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── potential_utf v0.1.6 (Unicode-3.0)
│   │   │   │   │   │   │   ├── writeable v0.6.4 (Unicode-3.0)
│   │   │   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0)
│   │   │   │   │   │   │       ├── yoke v0.8.4 (Unicode-3.0)
│   │   │   │   │   │   │       │   ├── stable_deref_trait v1.2.1 (MIT OR Apache-2.0)
│   │   │   │   │   │   │       │   ├── yoke-derive v0.8.4 (proc-macro) (Unicode-3.0)
│   │   │   │   │   │   │       │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   │   ├── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   │   └── synstructure v0.14.0 (MIT)
│   │   │   │   │   │   │       │   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   │       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │   └── zerofrom v0.1.8 (Unicode-3.0)
│   │   │   │   │   │   │       │       └── zerofrom-derive v0.1.8 (proc-macro) (Unicode-3.0)
│   │   │   │   │   │   │       │           ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │           ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │           ├── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │       │           └── synstructure v0.14.0 (MIT) (*)
│   │   │   │   │   │   │       ├── zerofrom v0.1.8 (Unicode-3.0) (*)
│   │   │   │   │   │   │       └── zerovec-derive v0.11.7 (proc-macro) (Unicode-3.0)
│   │   │   │   │   │   │           ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │           ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │           └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── utf8_iter v1.1.0 (Apache-2.0 OR MIT)
│   │   │   │   │   │   │   └── utf_types v1.0.0 (Apache-2.0 OR MIT)
│   │   │   │   │   │   ├── yoke v0.8.4 (Unicode-3.0) (*)
│   │   │   │   │   │   ├── zerofrom v0.1.8 (Unicode-3.0) (*)
│   │   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   │   │   ├── icu_normalizer_data v2.4.0 (Unicode-3.0)
│   │   │   │   │   ├── icu_provider v2.4.0 (Unicode-3.0)
│   │   │   │   │   │   ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── icu_locale_core v2.4.0 (Unicode-3.0)
│   │   │   │   │   │   │   ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   ├── litemap v0.8.3 (Unicode-3.0)
│   │   │   │   │   │   │   ├── tinystr v0.8.5 (Unicode-3.0)
│   │   │   │   │   │   │   │   ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   │   │   │   │   ├── writeable v0.6.4 (Unicode-3.0)
│   │   │   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   │   │   │   ├── stable_deref_trait v1.2.1 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── writeable v0.6.4 (Unicode-3.0)
│   │   │   │   │   │   ├── yoke v0.8.4 (Unicode-3.0) (*)
│   │   │   │   │   │   ├── zerofrom v0.1.8 (Unicode-3.0) (*)
│   │   │   │   │   │   ├── zerotrie v0.2.5 (Unicode-3.0)
│   │   │   │   │   │   │   ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   ├── yoke v0.8.4 (Unicode-3.0) (*)
│   │   │   │   │   │   │   └── zerofrom v0.1.8 (Unicode-3.0) (*)
│   │   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   │   │   ├── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   │   │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   │   └── icu_properties v2.4.0 (Unicode-3.0)
│   │   │   │       ├── displaydoc v0.2.7 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │       ├── icu_collections v2.4.0 (Unicode-3.0) (*)
│   │   │   │       ├── icu_locale_core v2.4.0 (Unicode-3.0) (*)
│   │   │   │       ├── icu_properties_data v2.4.0 (Unicode-3.0)
│   │   │   │       ├── icu_provider v2.4.0 (Unicode-3.0) (*)
│   │   │   │       ├── zerotrie v0.2.5 (Unicode-3.0) (*)
│   │   │   │       └── zerovec v0.11.9 (Unicode-3.0) (*)
│   │   │   ├── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   │   └── utf8_iter v1.1.0 (Apache-2.0 OR MIT) (*)
│   │   ├── percent-encoding v2.3.2 (MIT OR Apache-2.0)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   └── serde_derive v1.0.229 (proc-macro) (MIT OR Apache-2.0) (*)
│   └── webdriver v0.54.0 (MPL-2.0)
│       ├── base64 v0.22.1 (MIT OR Apache-2.0)
│       ├── bytes v1.12.1 (MIT)
│       ├── cookie v0.16.2 (MIT OR Apache-2.0)
│       │   └── time v0.3.55 (MIT OR Apache-2.0) (*)
│       ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│       ├── icu_segmenter v2.4.0 (Unicode-3.0)
│       │   ├── core_maths v0.1.1 (MIT)
│       │   │   └── libm v0.2.16 (MIT)
│       │   ├── icu_collections v2.4.0 (Unicode-3.0) (*)
│       │   ├── icu_locale_fallback v2.4.0 (Unicode-3.0)
│       │   │   ├── icu_locale_core v2.4.0 (Unicode-3.0) (*)
│       │   │   ├── icu_locale_fallback_data v2.4.0 (Unicode-3.0)
│       │   │   ├── icu_provider v2.4.0 (Unicode-3.0) (*)
│       │   │   ├── potential_utf v0.1.6 (Unicode-3.0) (*)
│       │   │   ├── tinystr v0.8.5 (Unicode-3.0) (*)
│       │   │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│       │   ├── icu_provider v2.4.0 (Unicode-3.0) (*)
│       │   ├── icu_segmenter_data v2.4.0 (Unicode-3.0)
│       │   ├── potential_utf v0.1.6 (Unicode-3.0) (*)
│       │   ├── smallvec v1.16.5 (MIT OR Apache-2.0)
│       │   ├── utf8_iter v1.1.0 (Apache-2.0 OR MIT) (*)
│       │   ├── zerotrie v0.2.5 (Unicode-3.0) (*)
│       │   └── zerovec v0.11.9 (Unicode-3.0) (*)
│       ├── log v0.4.35 (MIT OR Apache-2.0)
│       ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│       ├── serde_derive v1.0.229 (proc-macro) (MIT OR Apache-2.0) (*)
│       ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│       ├── thiserror v1.0.69 (MIT OR Apache-2.0)
│       │   └── thiserror-impl v1.0.69 (proc-macro) (MIT OR Apache-2.0)
│       │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│       │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│       │       └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│       ├── time v0.3.55 (MIT OR Apache-2.0) (*)
│       └── url v2.5.8 (MIT OR Apache-2.0) (*)
├── hex v0.4.3 (MIT OR Apache-2.0)
├── http v1.5.0 (MIT OR Apache-2.0) (*)
├── http-body-util v0.1.5 (MIT) (*)
├── hyper v1.12.0 (MIT) (*)
├── hyper-util v0.1.21 (MIT) (*)
├── serde v1.0.229 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── sha2 v0.10.9 (MIT OR Apache-2.0)
│   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   ├── cpufeatures v0.2.17 (MIT OR Apache-2.0)
│   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   └── digest v0.10.7 (MIT OR Apache-2.0)
│       ├── block-buffer v0.10.4 (MIT OR Apache-2.0)
│       │   └── generic-array v0.14.7 (MIT)
│       │       └── typenum v1.20.1 (MIT OR Apache-2.0)
│       └── crypto-common v0.1.7 (MIT OR Apache-2.0)
│           ├── generic-array v0.14.7 (MIT) (*)
│           └── typenum v1.20.1 (MIT OR Apache-2.0)
├── thiserror v2.0.21 (MIT OR Apache-2.0)
│   └── thiserror-impl v2.0.21 (proc-macro) (MIT OR Apache-2.0)
│       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
├── tokio v1.54.1 (MIT) (*)
├── tracing v0.1.44 (MIT) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0)
│   ├── hex v0.4.3 (MIT OR Apache-2.0)
│   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── sha2 v0.10.9 (MIT OR Apache-2.0) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   └── uuid v1.29.2 (Apache-2.0 OR MIT)
│       └── serde_core v1.0.229 (MIT OR Apache-2.0)
├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0)
│   ├── chrono v0.4.45 (MIT OR Apache-2.0)
│   │   ├── iana-time-zone v0.1.65 (MIT OR Apache-2.0)
│   │   │   └── core-foundation-sys v0.8.7 (MIT OR Apache-2.0)
│   │   ├── num-traits v0.2.19 (MIT OR Apache-2.0)
│   │   └── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── hex v0.4.3 (MIT OR Apache-2.0)
│   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── serde_jcs v0.2.0 (MIT OR Apache-2.0)
│   │   ├── ryu-js v0.2.2 (Apache-2.0 OR BSL-1.0)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   └── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── sha2 v0.10.9 (MIT OR Apache-2.0) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── ts-rs v12.0.1 (MIT)
│   │   ├── chrono v0.4.45 (MIT OR Apache-2.0) (*)
│   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   ├── ts-rs-macros v12.0.1 (proc-macro) (MIT)
│   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   ├── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   └── termcolor v1.4.1 (Unlicense OR MIT)
│   │   └── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   ├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
│   └── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
├── url v2.5.8 (MIT OR Apache-2.0) (*)
└── uuid v1.29.2 (Apache-2.0 OR MIT) (*)

unleash-cli v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-cli) (Apache-2.0)
├── clap v4.6.8 (MIT OR Apache-2.0)
│   ├── clap_builder v4.6.7 (MIT OR Apache-2.0)
│   │   ├── anstream v1.0.0 (MIT OR Apache-2.0)
│   │   │   ├── anstyle v1.0.14 (MIT OR Apache-2.0)
│   │   │   ├── anstyle-parse v1.0.0 (MIT OR Apache-2.0)
│   │   │   │   └── utf8parse v0.2.2 (Apache-2.0 OR MIT)
│   │   │   ├── anstyle-query v1.1.5 (MIT OR Apache-2.0)
│   │   │   ├── colorchoice v1.0.5 (MIT OR Apache-2.0)
│   │   │   ├── is_terminal_polyfill v1.70.2 (MIT OR Apache-2.0)
│   │   │   └── utf8parse v0.2.2 (Apache-2.0 OR MIT)
│   │   ├── anstyle v1.0.14 (MIT OR Apache-2.0)
│   │   ├── clap_lex v1.1.2 (MIT OR Apache-2.0)
│   │   └── strsim v0.11.1 (MIT)
│   └── clap_derive v4.6.8 (proc-macro) (MIT OR Apache-2.0)
│       ├── heck v0.5.0 (MIT OR Apache-2.0)
│       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
├── tokio v1.54.1 (MIT) (*)
├── unleash-client v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-client) (Apache-2.0)
│   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── tokio v1.54.1 (MIT) (*)
│   ├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
│   ├── unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0)
│   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   ├── rustix v1.1.6 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0)
│   │   │   │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   ├── errno v0.3.15 (MIT OR Apache-2.0) (*)
│   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   ├── secrecy v0.10.3 (Apache-2.0 OR MIT)
│   │   │   └── zeroize v1.9.1 (Apache-2.0 OR MIT)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   ├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
│   │   └── zeroize v1.9.1 (Apache-2.0 OR MIT)
│   ├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
│   └── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
├── unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0) (*)
├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
├── unleash-tui v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-tui) (Apache-2.0)
│   ├── crossterm v0.29.0 (MIT)
│   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   ├── derive_more v2.1.1 (MIT)
│   │   │   └── derive_more-impl v2.1.1 (proc-macro) (MIT)
│   │   │       ├── convert_case v0.10.0 (MIT)
│   │   │       │   └── unicode-segmentation v1.13.3 (MIT OR Apache-2.0)
│   │   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │       └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   ├── document-features v0.2.12 (proc-macro) (MIT OR Apache-2.0)
│   │   │   └── litrs v1.0.1 (MIT OR Apache-2.0)
│   │   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   │   ├── mio v1.2.5 (MIT) (*)
│   │   ├── parking_lot v0.12.5 (MIT OR Apache-2.0)
│   │   │   ├── lock_api v0.4.14 (MIT OR Apache-2.0)
│   │   │   │   └── scopeguard v1.2.0 (MIT OR Apache-2.0)
│   │   │   └── parking_lot_core v0.9.12 (MIT OR Apache-2.0)
│   │   │       ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │       ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │       └── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   ├── rustix v1.1.6 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) (*)
│   │   ├── signal-hook v0.3.18 (Apache-2.0/MIT)
│   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   └── signal-hook-registry v1.4.8 (MIT OR Apache-2.0) (*)
│   │   └── signal-hook-mio v0.2.5 (MIT OR Apache-2.0)
│   │       ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │       ├── mio v1.2.5 (MIT) (*)
│   │       └── signal-hook v0.3.18 (Apache-2.0/MIT) (*)
│   ├── futures-core v0.3.34 (MIT OR Apache-2.0)
│   ├── ratatui v0.30.2 (MIT)
│   │   ├── instability v0.3.14 (proc-macro) (MIT)
│   │   │   ├── darling v0.24.1 (MIT)
│   │   │   │   ├── darling_core v0.24.1 (MIT)
│   │   │   │   │   ├── ident_case v1.0.1 (MIT/Apache-2.0)
│   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── strsim v0.11.1 (MIT)
│   │   │   │   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   └── darling_macro v0.24.1 (proc-macro) (MIT)
│   │   │   │       ├── darling_core v0.24.1 (MIT) (*)
│   │   │   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   ├── indoc v2.0.8 (proc-macro) (MIT OR Apache-2.0)
│   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   ├── ratatui-core v0.1.2 (MIT)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   ├── compact_str v0.9.1 (MIT)
│   │   │   │   ├── castaway v0.2.4 (MIT)
│   │   │   │   │   └── rustversion v1.0.23 (proc-macro) (MIT OR Apache-2.0)
│   │   │   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │   │   ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │   │   │   ├── rustversion v1.0.23 (proc-macro) (MIT OR Apache-2.0)
│   │   │   │   ├── ryu v1.0.23 (Apache-2.0 OR BSL-1.0)
│   │   │   │   └── static_assertions v1.1.0 (MIT OR Apache-2.0)
│   │   │   ├── critical-section v1.2.0 (MIT OR Apache-2.0)
│   │   │   ├── hashbrown v0.17.1 (MIT OR Apache-2.0) (*)
│   │   │   ├── itertools v0.14.0 (MIT OR Apache-2.0)
│   │   │   │   └── either v1.19.0 (MIT OR Apache-2.0)
│   │   │   ├── kasuari v0.4.13 (MIT OR Apache-2.0)
│   │   │   │   ├── hashbrown v0.17.1 (MIT OR Apache-2.0) (*)
│   │   │   │   └── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   ├── lru v0.18.5 (MIT)
│   │   │   │   └── hashbrown v0.17.1 (MIT OR Apache-2.0) (*)
│   │   │   ├── strum v0.28.0 (MIT)
│   │   │   │   └── strum_macros v0.28.0 (proc-macro) (MIT)
│   │   │   │       ├── heck v0.5.0 (MIT OR Apache-2.0)
│   │   │   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │       └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   ├── unicode-segmentation v1.13.3 (MIT OR Apache-2.0)
│   │   │   ├── unicode-truncate v2.0.1 (MIT OR Apache-2.0)
│   │   │   │   ├── itertools v0.14.0 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── unicode-segmentation v1.13.3 (MIT OR Apache-2.0)
│   │   │   │   └── unicode-width v0.2.3 (MIT OR Apache-2.0)
│   │   │   └── unicode-width v0.2.3 (MIT OR Apache-2.0)
│   │   ├── ratatui-crossterm v0.1.2 (MIT)
│   │   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │   ├── crossterm v0.29.0 (MIT) (*)
│   │   │   ├── instability v0.3.14 (proc-macro) (MIT) (*)
│   │   │   └── ratatui-core v0.1.2 (MIT) (*)
│   │   ├── ratatui-macros v0.7.2 (MIT)
│   │   │   ├── ratatui-core v0.1.2 (MIT) (*)
│   │   │   └── ratatui-widgets v0.3.2 (MIT)
│   │   │       ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │       ├── hashbrown v0.17.1 (MIT OR Apache-2.0) (*)
│   │   │       ├── indoc v2.0.8 (proc-macro) (MIT OR Apache-2.0)
│   │   │       ├── instability v0.3.14 (proc-macro) (MIT) (*)
│   │   │       ├── itertools v0.14.0 (MIT OR Apache-2.0) (*)
│   │   │       ├── line-clipping v0.3.8 (MIT OR Apache-2.0)
│   │   │       │   └── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │       ├── ratatui-core v0.1.2 (MIT) (*)
│   │   │       ├── strum v0.28.0 (MIT) (*)
│   │   │       ├── time v0.3.55 (MIT OR Apache-2.0) (*)
│   │   │       ├── unicode-segmentation v1.13.3 (MIT OR Apache-2.0)
│   │   │       └── unicode-width v0.2.3 (MIT OR Apache-2.0)
│   │   └── ratatui-widgets v0.3.2 (MIT) (*)
│   ├── ropey v1.6.1 (MIT)
│   │   ├── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   └── str_indices v0.4.4 (MIT OR Apache-2.0)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── tokio v1.54.1 (MIT) (*)
│   ├── tracing v0.1.44 (MIT) (*)
│   ├── tui-term v0.3.4 (MIT)
│   │   ├── ratatui-core v0.1.2 (MIT) (*)
│   │   ├── ratatui-widgets v0.3.2 (MIT) (*)
│   │   └── vt100 v0.16.2 (MIT)
│   │       ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │       ├── unicode-width v0.2.3 (MIT OR Apache-2.0)
│   │       └── vte v0.15.0 (Apache-2.0 OR MIT)
│   │           ├── arrayvec v0.7.8 (MIT OR Apache-2.0)
│   │           └── memchr v2.8.3 (Unlicense OR MIT)
│   ├── unicode-segmentation v1.13.3 (MIT OR Apache-2.0)
│   ├── unicode-width v0.2.3 (MIT OR Apache-2.0)
│   ├── unleash-client v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-client) (Apache-2.0) (*)
│   ├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
│   ├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
│   ├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   └── vt100 v0.16.2 (MIT) (*)
└── uuid v1.29.2 (Apache-2.0 OR MIT) (*)

unleash-client v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-client) (Apache-2.0) (*)

unleash-daemon v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-daemon) (Apache-2.0)
├── chrono v0.4.45 (MIT OR Apache-2.0) (*)
├── clap v4.6.8 (MIT OR Apache-2.0) (*)
├── rusqlite v0.43.0 (MIT)
│   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   ├── fallible-iterator v0.3.0 (MIT/Apache-2.0)
│   ├── fallible-streaming-iterator v0.1.9 (MIT/Apache-2.0)
│   ├── hashlink v0.12.2 (MIT OR Apache-2.0)
│   │   └── hashbrown v0.17.1 (MIT OR Apache-2.0) (*)
│   ├── libsqlite3-sys v0.40.0 (MIT)
│   └── smallvec v1.16.5 (MIT OR Apache-2.0)
├── serde v1.0.229 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
├── tokio v1.54.1 (MIT) (*)
├── tracing v0.1.44 (MIT) (*)
├── unleash-browser v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-browser) (Apache-2.0) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
├── unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0) (*)
├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
├── unleash-store v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-store) (Apache-2.0)
│   ├── chrono v0.4.45 (MIT OR Apache-2.0) (*)
│   ├── hex v0.4.3 (MIT OR Apache-2.0)
│   ├── rusqlite v0.43.0 (MIT) (*)
│   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── sha2 v0.10.9 (MIT OR Apache-2.0) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── tokio v1.54.1 (MIT) (*)
│   ├── tracing v0.1.44 (MIT) (*)
│   ├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
│   ├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
│   └── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
└── uuid v1.29.2 (Apache-2.0 OR MIT) (*)

unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)

unleash-gui v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-gui) (Apache-2.0)
├── pulldown-cmark v0.13.6 (MIT)
│   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   ├── memchr v2.8.3 (Unlicense OR MIT)
│   └── unicase v2.10.0 (MIT OR Apache-2.0)
├── serde v1.0.229 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── tauri v2.12.6 (Apache-2.0 OR MIT)
│   ├── anyhow v1.0.104 (MIT OR Apache-2.0)
│   ├── cookie v0.18.2 (MIT OR Apache-2.0) (*)
│   ├── dirs v7.0.0 (MIT OR Apache-2.0)
│   │   └── dirs-sys v0.5.0 (MIT OR Apache-2.0)
│   │       ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │       └── option-ext v0.2.0 (MPL-2.0)
│   ├── dunce v1.0.5 (CC0-1.0 OR MIT-0 OR Apache-2.0)
│   ├── embed_plist v1.2.2 (MIT OR Apache-2.0)
│   ├── getrandom v0.3.4 (MIT OR Apache-2.0)
│   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   ├── glob v0.3.4 (MIT OR Apache-2.0)
│   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   ├── log v0.4.35 (MIT OR Apache-2.0)
│   ├── mime v0.3.18 (MIT OR Apache-2.0)
│   ├── muda v0.20.0 (Apache-2.0 OR MIT)
│   │   ├── crossbeam-channel v0.5.17 (MIT OR Apache-2.0)
│   │   │   └── crossbeam-utils v0.8.23 (MIT OR Apache-2.0)
│   │   ├── dpi v0.1.2 (Apache-2.0 AND MIT)
│   │   │   └── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   ├── keyboard-types v0.8.3 (MIT OR Apache-2.0)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   └── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   ├── objc2 v0.6.5 (MIT)
│   │   │   ├── objc2-encode v4.1.0 (MIT)
│   │   │   └── objc2-exception-helper v0.1.1 (Zlib OR Apache-2.0 OR MIT)
│   │   ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   ├── block2 v0.6.2 (MIT)
│   │   │   │   └── objc2 v0.6.5 (MIT) (*)
│   │   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   │   ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT)
│   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   │   └── objc2 v0.6.5 (MIT) (*)
│   │   │   ├── objc2-foundation v0.3.2 (MIT)
│   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── block2 v0.6.2 (MIT) (*)
│   │   │   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   │   │   └── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   │   └── objc2-quartz-core v0.3.2 (Zlib OR Apache-2.0 OR MIT)
│   │   │       ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │       ├── objc2 v0.6.5 (MIT) (*)
│   │   │       ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   │       └── objc2-foundation v0.3.2 (MIT) (*)
│   │   ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   ├── objc2-foundation v0.3.2 (MIT) (*)
│   │   ├── once_cell v1.21.4 (MIT OR Apache-2.0)
│   │   ├── png v0.18.1 (MIT OR Apache-2.0)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   ├── crc32fast v1.5.5 (MIT OR Apache-2.0)
│   │   │   │   └── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │   ├── fdeflate v0.3.7 (MIT OR Apache-2.0)
│   │   │   │   └── simd-adler32 v0.3.10 (MIT)
│   │   │   ├── flate2 v1.1.11 (MIT OR Apache-2.0)
│   │   │   │   ├── crc32fast v1.5.5 (MIT OR Apache-2.0) (*)
│   │   │   │   └── miniz_oxide v0.9.1 (MIT OR Zlib OR Apache-2.0)
│   │   │   │       ├── adler2 v2.0.1 (0BSD OR MIT OR Apache-2.0)
│   │   │   │       └── simd-adler32 v0.3.10 (MIT)
│   │   │   └── miniz_oxide v0.8.9 (MIT OR Zlib OR Apache-2.0)
│   │   │       ├── adler2 v2.0.1 (0BSD OR MIT OR Apache-2.0)
│   │   │       └── simd-adler32 v0.3.10 (MIT)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   └── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── objc2 v0.6.5 (MIT) (*)
│   ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   ├── objc2-foundation v0.3.2 (MIT) (*)
│   ├── objc2-web-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT)
│   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   ├── block2 v0.6.2 (MIT) (*)
│   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   └── objc2-foundation v0.3.2 (MIT) (*)
│   ├── percent-encoding v2.3.2 (MIT OR Apache-2.0)
│   ├── plist v1.10.4 (MIT)
│   │   ├── base64 v0.23.1 (MIT OR Apache-2.0)
│   │   ├── indexmap v2.14.2 (Apache-2.0 OR MIT) (*)
│   │   ├── quick-xml v0.42.0 (MIT)
│   │   │   └── memchr v2.8.3 (Unlicense OR MIT)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   └── time v0.3.55 (MIT OR Apache-2.0) (*)
│   ├── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
│   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   ├── serde_repr v0.1.21 (proc-macro) (MIT OR Apache-2.0)
│   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   ├── serialize-to-javascript v0.1.2 (MIT OR Apache-2.0)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   └── serialize-to-javascript-impl v0.1.2 (proc-macro) (MIT OR Apache-2.0)
│   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │       └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   ├── tauri-macros v2.7.5 (proc-macro) (Apache-2.0 OR MIT)
│   │   ├── heck v0.5.0 (MIT OR Apache-2.0)
│   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   ├── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   ├── tauri-codegen v2.7.5 (Apache-2.0 OR MIT)
│   │   │   ├── base64 v0.23.1 (MIT OR Apache-2.0)
│   │   │   ├── brotli v9.1.3 (BSD-3-Clause AND MIT)
│   │   │   │   ├── alloc-no-stdlib v3.0.0 (BSD-3-Clause)
│   │   │   │   ├── alloc-stdlib v0.3.0 (BSD-3-Clause)
│   │   │   │   │   └── alloc-no-stdlib v3.0.0 (BSD-3-Clause)
│   │   │   │   └── brotli-decompressor v6.3.2 (BSD-3-Clause/MIT)
│   │   │   │       ├── alloc-no-stdlib v3.0.0 (BSD-3-Clause)
│   │   │   │       └── alloc-stdlib v0.3.0 (BSD-3-Clause) (*)
│   │   │   ├── ico v0.6.0 (MIT)
│   │   │   │   ├── byteorder v1.5.0 (Unlicense OR MIT)
│   │   │   │   └── png v0.18.1 (MIT OR Apache-2.0) (*)
│   │   │   ├── json-patch v4.2.0 (MIT/Apache-2.0)
│   │   │   │   ├── jsonptr v0.7.1 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde_derive v1.0.229 (proc-macro) (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── serde_json v1.0.154 (MIT OR Apache-2.0)
│   │   │   │   │       ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │   │   │   │       ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   │   │       ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │       └── zmij v1.0.23 (MIT)
│   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   │   └── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   ├── plist v1.10.4 (MIT) (*)
│   │   │   ├── png v0.18.1 (MIT OR Apache-2.0) (*)
│   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   ├── semver v1.0.28 (MIT OR Apache-2.0)
│   │   │   │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   ├── sha2 v0.11.1 (MIT OR Apache-2.0)
│   │   │   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │   │   ├── cpufeatures v0.3.1 (MIT OR Apache-2.0)
│   │   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   └── digest v0.11.3 (MIT OR Apache-2.0)
│   │   │   │       ├── block-buffer v0.12.1 (MIT OR Apache-2.0)
│   │   │   │       │   └── hybrid-array v0.4.15 (MIT OR Apache-2.0)
│   │   │   │       │       └── typenum v1.20.1 (MIT OR Apache-2.0)
│   │   │   │       ├── const-oid v0.10.3 (Apache-2.0 OR MIT)
│   │   │   │       └── crypto-common v0.2.2 (MIT OR Apache-2.0)
│   │   │   │           └── hybrid-array v0.4.15 (MIT OR Apache-2.0) (*)
│   │   │   ├── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   ├── tauri-utils v2.10.3 (Apache-2.0 OR MIT)
│   │   │   │   ├── anyhow v1.0.104 (MIT OR Apache-2.0)
│   │   │   │   ├── brotli v9.1.3 (BSD-3-Clause AND MIT) (*)
│   │   │   │   ├── cargo_metadata v0.19.2 (MIT)
│   │   │   │   │   ├── camino v1.2.6 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   ├── cargo-platform v0.1.9 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── semver v1.0.28 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── ctor v1.0.14 (Apache-2.0 OR MIT)
│   │   │   │   ├── dom_query v0.28.0 (MIT)
│   │   │   │   │   ├── bit-set v0.8.0 (Apache-2.0 OR MIT)
│   │   │   │   │   │   └── bit-vec v0.8.0 (Apache-2.0 OR MIT)
│   │   │   │   │   ├── cssparser v0.37.0 (MPL-2.0)
│   │   │   │   │   │   ├── cssparser-macros v0.7.1 (proc-macro) (MPL-2.0)
│   │   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── dtoa-short v0.3.5 (MPL-2.0)
│   │   │   │   │   │   │   └── dtoa v1.0.11 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── itoa v1.0.19 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── phf v0.13.1 (MIT)
│   │   │   │   │   │   │   ├── phf_macros v0.13.1 (proc-macro) (MIT)
│   │   │   │   │   │   │   │   ├── phf_generator v0.13.1 (MIT)
│   │   │   │   │   │   │   │   │   ├── fastrand v2.5.0 (Apache-2.0 OR MIT)
│   │   │   │   │   │   │   │   │   └── phf_shared v0.13.1 (MIT)
│   │   │   │   │   │   │   │   │       └── siphasher v1.0.4 (MIT OR Apache-2.0)
│   │   │   │   │   │   │   │   ├── phf_shared v0.13.1 (MIT) (*)
│   │   │   │   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   │   └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   └── phf_shared v0.13.1 (MIT) (*)
│   │   │   │   │   │   └── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   │   │   │   ├── foldhash v0.2.0 (Zlib)
│   │   │   │   │   ├── html5ever v0.39.0 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── markup5ever v0.39.0 (MIT OR Apache-2.0)
│   │   │   │   │   │       ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   │   │   │       ├── tendril v0.5.1 (MIT OR Apache-2.0)
│   │   │   │   │   │       │   └── new_debug_unreachable v1.0.6 (MIT)
│   │   │   │   │   │       └── web_atoms v0.2.6 (MIT OR Apache-2.0)
│   │   │   │   │   │           ├── phf v0.13.1 (MIT) (*)
│   │   │   │   │   │           └── string_cache v0.9.0 (MIT OR Apache-2.0)
│   │   │   │   │   │               ├── new_debug_unreachable v1.0.6 (MIT)
│   │   │   │   │   │               ├── parking_lot v0.12.5 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │               ├── phf_shared v0.13.1 (MIT) (*)
│   │   │   │   │   │               └── precomputed-hash v0.1.1 (MIT)
│   │   │   │   │   ├── precomputed-hash v0.1.1 (MIT)
│   │   │   │   │   ├── selectors v0.38.0 (MPL-2.0)
│   │   │   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── cssparser v0.37.0 (MPL-2.0) (*)
│   │   │   │   │   │   ├── derive_more v2.1.1 (MIT)
│   │   │   │   │   │   │   └── derive_more-impl v2.1.1 (proc-macro) (MIT) (*)
│   │   │   │   │   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── new_debug_unreachable v1.0.6 (MIT)
│   │   │   │   │   │   ├── phf v0.13.1 (MIT) (*)
│   │   │   │   │   │   ├── precomputed-hash v0.1.1 (MIT)
│   │   │   │   │   │   ├── rustc-hash v2.1.3 (Apache-2.0 OR MIT)
│   │   │   │   │   │   ├── servo_arc v0.4.3 (MIT OR Apache-2.0)
│   │   │   │   │   │   │   └── stable_deref_trait v1.2.1 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── smallvec v1.16.5 (MIT OR Apache-2.0)
│   │   │   │   │   └── tendril v0.5.1 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── dunce v1.0.5 (CC0-1.0 OR MIT-0 OR Apache-2.0)
│   │   │   │   ├── glob v0.3.4 (MIT OR Apache-2.0)
│   │   │   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── infer v0.22.0 (MIT)
│   │   │   │   │   └── cfb v0.14.0 (MIT)
│   │   │   │   │       ├── fnv v1.0.7 (Apache-2.0 / MIT)
│   │   │   │   │       ├── uuid v1.29.2 (Apache-2.0 OR MIT)
│   │   │   │   │       │   ├── getrandom v0.4.3 (MIT OR Apache-2.0)
│   │   │   │   │       │   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   │   │   │       │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   │       │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │       └── web-time v1.1.0 (MIT OR Apache-2.0)
│   │   │   │   ├── json-patch v4.2.0 (MIT/Apache-2.0) (*)
│   │   │   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   │   ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   │   ├── phf v0.13.1 (MIT) (*)
│   │   │   │   ├── plist v1.10.4 (MIT) (*)
│   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── regex v1.13.1 (MIT OR Apache-2.0)
│   │   │   │   │   ├── aho-corasick v1.1.5 (Unlicense OR MIT)
│   │   │   │   │   │   └── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   │   │   ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   │   │   ├── regex-automata v0.4.18 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── aho-corasick v1.1.5 (Unlicense OR MIT) (*)
│   │   │   │   │   │   ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   │   │   │   └── regex-syntax v0.8.11 (MIT OR Apache-2.0)
│   │   │   │   │   └── regex-syntax v0.8.11 (MIT OR Apache-2.0)
│   │   │   │   ├── schemars v0.8.22 (MIT)
│   │   │   │   │   ├── dyn-clone v1.0.20 (MIT OR Apache-2.0)
│   │   │   │   │   ├── indexmap v1.9.3 (Apache-2.0 OR MIT)
│   │   │   │   │   │   ├── hashbrown v0.12.3 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── schemars_derive v0.8.22 (proc-macro) (MIT)
│   │   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── serde_derive_internals v0.29.1 (MIT OR Apache-2.0)
│   │   │   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   │   └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   └── syn v2.0.119 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   │   │   │   ├── semver v1.0.28 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── serde-untagged v0.1.9 (MIT OR Apache-2.0)
│   │   │   │   │   ├── erased-serde v0.4.11 (MIT OR Apache-2.0)
│   │   │   │   │   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── typeid v1.0.3 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   └── typeid v1.0.3 (MIT OR Apache-2.0)
│   │   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── serde_with v3.24.0 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   └── serde_with_macros v3.24.0 (proc-macro) (MIT OR Apache-2.0)
│   │   │   │   │       ├── darling v0.24.1 (MIT) (*)
│   │   │   │   │       ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │       ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │       └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── swift-rs v1.0.9 (MIT OR Apache-2.0)
│   │   │   │   │   ├── base64 v0.21.7 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── toml v1.2.1+spec-1.1.0 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   ├── serde_spanned v1.1.3 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   ├── toml_datetime v1.1.3+spec-1.1.0 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── serde_core v1.0.229 (MIT OR Apache-2.0)
│   │   │   │   │   ├── toml_parser v1.1.5+spec-1.1.0 (MIT OR Apache-2.0)
│   │   │   │   │   │   └── winnow v1.0.5 (MIT)
│   │   │   │   │   ├── toml_writer v1.2.0+spec-1.1.0 (MIT OR Apache-2.0)
│   │   │   │   │   └── winnow v1.0.5 (MIT)
│   │   │   │   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── urlpattern v0.6.0 (MIT)
│   │   │   │   │   ├── icu_properties v2.4.0 (Unicode-3.0) (*)
│   │   │   │   │   ├── regex v1.13.1 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   │   │   │   └── walkdir v2.5.0 (Unlicense/MIT)
│   │   │   │       └── same-file v1.0.6 (Unlicense/MIT)
│   │   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   ├── time v0.3.55 (MIT OR Apache-2.0)
│   │   │   │   ├── deranged v0.5.9 (MIT OR Apache-2.0)
│   │   │   │   ├── num-conv v0.2.2 (MIT OR Apache-2.0)
│   │   │   │   ├── powerfmt v0.2.1 (MIT OR Apache-2.0)
│   │   │   │   └── time-core v0.1.9 (MIT OR Apache-2.0)
│   │   │   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   │   ├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   │   │   └── walkdir v2.5.0 (Unlicense/MIT) (*)
│   │   └── tauri-utils v2.10.3 (Apache-2.0 OR MIT) (*)
│   ├── tauri-runtime v2.12.3 (Apache-2.0 OR MIT)
│   │   ├── cookie v0.18.2 (MIT OR Apache-2.0) (*)
│   │   ├── dpi v0.1.2 (Apache-2.0 AND MIT) (*)
│   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   ├── objc2-web-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   ├── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
│   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   ├── tauri-utils v2.10.3 (Apache-2.0 OR MIT)
│   │   │   ├── anyhow v1.0.104 (MIT OR Apache-2.0)
│   │   │   ├── brotli v9.1.3 (BSD-3-Clause AND MIT) (*)
│   │   │   ├── ctor v1.0.14 (Apache-2.0 OR MIT)
│   │   │   ├── dunce v1.0.5 (CC0-1.0 OR MIT-0 OR Apache-2.0)
│   │   │   ├── glob v0.3.4 (MIT OR Apache-2.0)
│   │   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   │   ├── infer v0.22.0 (MIT) (*)
│   │   │   ├── json-patch v4.2.0 (MIT/Apache-2.0) (*)
│   │   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   ├── memchr v2.8.3 (Unlicense OR MIT)
│   │   │   ├── phf v0.13.1 (MIT) (*)
│   │   │   ├── plist v1.10.4 (MIT) (*)
│   │   │   ├── regex v1.13.1 (MIT OR Apache-2.0) (*)
│   │   │   ├── semver v1.0.28 (MIT OR Apache-2.0)
│   │   │   ├── serde v1.0.229 (MIT OR Apache-2.0) (*)
│   │   │   ├── serde-untagged v0.1.9 (MIT OR Apache-2.0) (*)
│   │   │   ├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
│   │   │   ├── serde_with v3.24.0 (MIT OR Apache-2.0) (*)
│   │   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   │   ├── toml v1.2.1+spec-1.1.0 (MIT OR Apache-2.0) (*)
│   │   │   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   │   ├── urlpattern v0.6.0 (MIT) (*)
│   │   │   ├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
│   │   │   └── walkdir v2.5.0 (Unlicense/MIT) (*)
│   │   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │   └── url v2.5.8 (MIT OR Apache-2.0) (*)
│   ├── tauri-runtime-wry v2.12.4 (Apache-2.0 OR MIT)
│   │   ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   ├── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
│   │   ├── tao v0.37.2 (Apache-2.0)
│   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   ├── block2 v0.6.2 (MIT) (*)
│   │   │   ├── core-foundation v0.10.1 (MIT OR Apache-2.0)
│   │   │   │   ├── core-foundation-sys v0.8.7 (MIT OR Apache-2.0)
│   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   ├── core-graphics v0.25.0 (MIT OR Apache-2.0)
│   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── core-foundation v0.10.1 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── core-graphics-types v0.2.0 (MIT OR Apache-2.0)
│   │   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   │   │   ├── core-foundation v0.10.1 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   ├── foreign-types v0.5.0 (MIT/Apache-2.0)
│   │   │   │   │   ├── foreign-types-macros v0.2.4 (proc-macro) (MIT/Apache-2.0)
│   │   │   │   │   │   ├── proc-macro2 v1.0.107 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   ├── quote v1.0.47 (MIT OR Apache-2.0) (*)
│   │   │   │   │   │   └── syn v3.0.10 (MIT OR Apache-2.0) (*)
│   │   │   │   │   └── foreign-types-shared v0.3.1 (MIT/Apache-2.0)
│   │   │   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   ├── crossbeam-channel v0.5.17 (MIT OR Apache-2.0) (*)
│   │   │   ├── dispatch2 v0.3.1 (Zlib OR Apache-2.0 OR MIT)
│   │   │   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   │   │   ├── block2 v0.6.2 (MIT) (*)
│   │   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   │   └── objc2 v0.6.5 (MIT) (*)
│   │   │   ├── dpi v0.1.2 (Apache-2.0 AND MIT) (*)
│   │   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   │   ├── log v0.4.35 (MIT OR Apache-2.0)
│   │   │   ├── objc2 v0.6.5 (MIT) (*)
│   │   │   ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │   │   ├── objc2-foundation v0.3.2 (MIT) (*)
│   │   │   ├── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
│   │   │   └── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   ├── tauri-runtime v2.12.3 (Apache-2.0 OR MIT) (*)
│   │   ├── tauri-utils v2.10.3 (Apache-2.0 OR MIT) (*)
│   │   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   │   └── wry v0.57.2 (Apache-2.0 OR MIT)
│   │       ├── block2 v0.6.2 (MIT) (*)
│   │       ├── cookie v0.18.2 (MIT OR Apache-2.0) (*)
│   │       ├── dirs v7.0.0 (MIT OR Apache-2.0) (*)
│   │       ├── dpi v0.1.2 (Apache-2.0 AND MIT) (*)
│   │       ├── http v1.5.0 (MIT OR Apache-2.0) (*)
│   │       ├── objc2 v0.6.5 (MIT) (*)
│   │       ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │       ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │       ├── objc2-foundation v0.3.2 (MIT) (*)
│   │       ├── objc2-web-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│   │       ├── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
│   │       ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   │       └── url v2.5.8 (MIT OR Apache-2.0) (*)
│   ├── tauri-utils v2.10.3 (Apache-2.0 OR MIT) (*)
│   ├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
│   ├── tokio v1.54.1 (MIT) (*)
│   ├── url v2.5.8 (MIT OR Apache-2.0) (*)
│   └── window-vibrancy v0.8.1 (Apache-2.0 OR MIT)
│       ├── objc2 v0.6.5 (MIT) (*)
│       ├── objc2-app-kit v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│       ├── objc2-core-foundation v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│       ├── objc2-foundation v0.3.2 (MIT) (*)
│       ├── objc2-quartz-core v0.3.2 (Zlib OR Apache-2.0 OR MIT) (*)
│       └── raw-window-handle v0.6.2 (MIT OR Apache-2.0 OR Zlib)
├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
├── tokio v1.54.1 (MIT) (*)
├── tracing v0.1.44 (MIT) (*)
├── ts-rs v12.0.1 (MIT) (*)
├── unleash-client v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-client) (Apache-2.0) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
├── unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0) (*)
├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
└── uuid v1.29.2 (Apache-2.0 OR MIT) (*)

unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0) (*)

unleash-process-host v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-process-host) (Apache-2.0)
├── hex v0.4.3 (MIT OR Apache-2.0)
├── libc v0.2.191 (MIT OR Apache-2.0)
├── portable-pty v0.9.0 (MIT)
│   ├── anyhow v1.0.104 (MIT OR Apache-2.0)
│   ├── downcast-rs v1.2.1 (MIT/Apache-2.0)
│   ├── filedescriptor v0.8.3 (MIT)
│   │   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   │   └── thiserror v1.0.69 (MIT OR Apache-2.0) (*)
│   ├── libc v0.2.191 (MIT OR Apache-2.0)
│   ├── log v0.4.35 (MIT OR Apache-2.0)
│   ├── nix v0.28.0 (MIT)
│   │   ├── bitflags v2.13.2 (MIT OR Apache-2.0) (*)
│   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   ├── serial2 v0.2.39 (BSD-2-Clause OR Apache-2.0)
│   │   ├── cfg-if v1.0.5 (MIT OR Apache-2.0)
│   │   └── libc v0.2.191 (MIT OR Apache-2.0)
│   └── shell-words v1.1.1 (MIT/Apache-2.0)
├── rustix v1.1.6 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) (*)
├── secrecy v0.10.3 (Apache-2.0 OR MIT) (*)
├── serde v1.0.229 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── sha2 v0.10.9 (MIT OR Apache-2.0) (*)
├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
├── tracing v0.1.44 (MIT) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
├── unleash-platform v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-platform) (Apache-2.0) (*)
├── unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)
├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
└── zeroize v1.9.1 (Apache-2.0 OR MIT)

unleash-protocol v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-protocol) (Apache-2.0) (*)

unleash-store v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-store) (Apache-2.0) (*)

unleash-testkit v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-testkit) (Apache-2.0)
├── dotenvy v0.15.7 (MIT)
├── secrecy v0.10.3 (Apache-2.0 OR MIT) (*)
├── serde v1.0.229 (MIT OR Apache-2.0) (*)
├── serde_json v1.0.154 (MIT OR Apache-2.0) (*)
├── tempfile v3.27.0 (MIT OR Apache-2.0)
│   ├── fastrand v2.5.0 (Apache-2.0 OR MIT)
│   ├── getrandom v0.4.3 (MIT OR Apache-2.0) (*)
│   ├── once_cell v1.21.4 (MIT OR Apache-2.0)
│   └── rustix v1.1.6 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) (*)
├── thiserror v2.0.21 (MIT OR Apache-2.0) (*)
├── unleash-domain v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-domain) (Apache-2.0) (*)
├── uuid v1.29.2 (Apache-2.0 OR MIT) (*)
└── zeroize v1.9.1 (Apache-2.0 OR MIT)

unleash-tui v0.1.0 (/Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/crates/unleash-tui) (Apache-2.0) (*)
```

</details>

---

## 8. Sortie vérifiée de `cargo deny check`

Exécution exacte : `cargo deny --manifest-path Cargo.toml check` → **exit code `0`** (`5 206` lignes incluant les arbres complets de `highlight = "all"` pour les `41` doublons ; résumé complet des diagnostics et statut final ci-dessous) :

```text
warning[license-not-encountered]: license was not encountered
   ┌─ /Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/deny.toml:32:4
   │
32 │   "CDLA-Permissive-2.0",
   │    ━━━━━━━━━━━━━━━━━━━ unmatched license allowance

warning[license-not-encountered]: license was not encountered
   ┌─ /Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/deny.toml:25:4
   │
25 │   "ISC",
   │    ━━━ unmatched license allowance

warning[license-not-encountered]: license was not encountered
   ┌─ /Volumes/personal_folder/Unleash/.claude/worktrees/g0001-m0-t10/deny.toml:31:4
   │
31 │   "Unicode-DFS-2016",
   │    ━━━━━━━━━━━━━━━━ unmatched license allowance

...
[5186 lignes d'arbres warning[duplicate] pour les 41 entrées listées en §3.4]
...
                  └── (build) unleash-gui v0.1.0 (*)

advisories ok, bans ok, licenses ok, sources ok
```
