# Spike 5 — Adaptateur `fantoccini`, W3C `WebDriver`, proxy d'egress et écarts locaux (`M0`)

**Date : 17 novembre 2026 — Statut :**
- **Contrat Rust `BrowserPort` / `FantocciniBrowserAdapter` sur serveur `WebDriver` W3C synthétique : vérifié (`12` unitaires + `6` intégration passés, zéro réseau externe).**
- **Qualification navigateur réel (`EXT-BROWSER`) : `blocked` / `not_run` sur ce poste (aucun `chromedriver` appairé présent).**

Références : `.claude/architecture/browser-execution.md`, `.claude/specs/browser-automation.md`, `.claude/features/F019-reusable-automations/`, `crates/unleash-browser/`.

---

## 1. Inventaire réel du poste hôte (constats factuels en lecture seule)

L'inventaire passif a été exécuté sur le poste sans lancer de navigateur ni déclencher aucun téléchargement :

| Composant | État observé sur disque | Détails vérifiés |
|---|---|---|
| Système hôte | macOS `27.2` (`BuildVersion 26B123`), `arm64` | `Apple M4 Max`, `137 438 953 472` octets RAM (128 GiB) |
| Cache `ms-playwright/chromium-1248` | **Présent** : `Google Chrome for Testing.app` | Version `156.0.8078.4` ; `SHA-256` binaire : `c8509aee32abd6977968e9999769980b7a420dea05402d90042d9e5b41db15ef` |
| Autres builds `ms-playwright` | **Présents** | `chromium-1223` (`148.0.7778.96`, `SHA-256` `2447c7bdea8e8ad38b52594f1ba53e18821c05445e8a07ad1cf3b0f23931d89d`), `chromium-1234` (`151.0.7922.34`, `SHA-256` `a596b1cfc6353e987fcec8d71a23a28cd6a9e7a6b4e20b908e4c4fcffe51158e`), `chromium_headless_shell-{1223,1234,1248}`, `ffmpeg-{1011,1013}`, `firefox-1538`, `webkit-2336` |
| `chromedriver` | **Absent** | Absent de `~/Library/Caches/ms-playwright/` et introuvable dans `PATH` (`which chromedriver`) |
| `geckodriver` | **Absent** | Absent de `~/Library/Caches/ms-playwright/` et introuvable dans `PATH` |
| `/Applications/` | Navigateurs utilisateurs présents | `Google Chrome.app` (`157.0.8103.79`), `Firefox.app` (`155.0.1`), `Safari.app` (lien symbolique vers `/Applications` Cryptex système) |
| Outillage build natif | `cmake`, `ninja`, `nasm` **absents** | Constate une contrainte directe sur les features Cargo TLS natives |

**Écart bloquant pour `EXT-BROWSER` :** `Google Chrome for Testing.app` (`156.0.8078.4`) est présent dans `~/Library/Caches/ms-playwright/chromium-1248`, mais **aucun `chromedriver` appairé n'est présent sur le poste** (et la règle produit interdit tout téléchargement implicite à l'exécution). La fonction `inventory_local_runtimes` reflète fidèlement `paired_chrome_driver_ready = false` et l'ouverture réelle d'un contexte Chrome reste bloquée par `BrowserError::RuntimeMismatch` tant que le couple binaire appairé (`Google Chrome for Testing` + `chromedriver` du même build et de `SHA-256` consignés) n'a pas été préparé explicitement hors exécution.

---

## 2. Décisions de conception vérifiées dans `crates/unleash-browser`

### 2.1 Chaîne `fantoccini 0.22.2` et dépendances (`cargo deny check`)

- **`default-features = false` obligatoire sur ce poste** : activer `rustls-tls` ou `native-tls` sur `fantoccini` tire `aws-lc-sys` ou `openssl`, qui échouent ou imposent `cmake` (absent sur la machine). Avec `default-features = false` et `hyper_util::client::legacy::connect::HttpConnector::new()`, `fantoccini 0.22.2` dialogue avec le endpoint `WebDriver` en HTTP loopback pur sans aucun téléchargement ni certificat TLS fictif.
- **`cargo deny check`** : passe entièrement (`advisories ok, bans ok, licenses ok, sources ok`). Des avertissements non bloquants `duplicate` subsistent sur 5 entrées (`base64` `0.22.1`/`0.23.1`, `cookie` `0.16.2`/`0.18.2`, `syn` `2.0.119`/`3.0.10`, `thiserror` `1.0.69`/`2.0.21` et `thiserror-impl` `1.0.69`/`2.0.21`), ainsi que 8 avertissements `license-not-encountered` pour des licences autorisées dans `deny.toml` non rencontrées dans l'arbre courant (`0BSD`, `BSD-2-Clause`, `BSD-3-Clause`, `CC0-1.0`, `CDLA-Permissive-2.0`, `ISC`, `MIT-0`, `Unicode-DFS-2016`).
- **`#![forbid(unsafe_code)]` et `#![warn(missing_docs)]`** : respectés sur l'ensemble du crate.

### 2.2 Lacunes W3C de `fantoccini` et extensions ajoutées

Lors de l'implémentation de `FantocciniBrowserAdapter` contre un serveur `WebDriver` W3C fidèle (`crates/unleash-browser/src/webdriver_ext.rs` et `tests/support/mod.rs`), cinq points concrets ont été relevés :

1. **Endpoints W3C 2 absents de `fantoccini`** : `fantoccini 0.22.2` n'expose pas de méthodes natives pour `GET /session/{id}/element/{eid}/computedrole` ni `computedlabel`. L'adaptateur fournit `GetComputedRole`, `GetComputedLabel` et `WebDriverStatusCommand` via `WebDriverCompatibleCommand` + `Client::issue_cmd`, indispensables pour qualifier `LocatorStrategy::RoleName` et `LocatorStrategy::Label` sans recourir à un script arbitraire.
2. **Séquence de navigation `Client::goto`** : `fantoccini::Client::goto` interroge d'abord `GET /session/{id}/url` (`current_url()`) avant d'émettre `POST /session/{id}/url`. Une session `WebDriver` W3C fraîchement créée doit donc initialiser `current_url = "about:blank"` (comportement implémenté dans notre serveur fixture).
3. **Inspection passive avant lancement** : `inspect_runtime` et `verify_pinned_runtime` contrôlent version, type fichier régulier et empreinte `SHA-256` en flux sans lancer aucun processus ni appeler `chromedriver`. En mode `ExecutionMode::Mediated`, `BrowserEngineKind::SafariUnsupported` est explicitement rejeté par `BrowserError::CapabilityMissing` (`ErrorCode::CapabilityDenied` / `ReasonCode::CapabilityUnavailable`).
4. **Matrice V1 fermée et blocage d'ambiguïté** : `TargetSpec::try_from_raw` (`crates/unleash-browser/src/locator.rs`) refuse `XPath`, coordonnées écran, tout script fourni par un modèle (`ScriptOrigin::ModelSupplied`) via `UnsupportedLocatorOrScript`, ainsi qu'un `frame_scope` vide (`InvalidConfiguration`). `resolve_target` applique un comptage strict sur la collection retournée par `find_all` (`0` → `TargetMissing`, `> 1` → `AmbiguousTarget` avant tout clic, sans jamais sélectionner `.first()`) puis contrôle `ExpectedTargetIdentity` (`TargetDrift`).
5. **Rejet explicite de `TargetSpec::frame_scope`** : `resolve_target`, `evaluate_assertions` et `resolve_unique_for_assertion` appellent `ensure_frame_scope_supported`, rejetant explicitement toute portée `frame_scope: Some(...)` avec `BrowserError::CapabilityMissing` (`details_ref = "target.frame_scope"`, `ErrorCode::CapabilityDenied` / `ReasonCode::CapabilityUnavailable`) au lieu de l'ignorer silencieusement ; les changements explicites de frame passent par `BrowserActionKind::SwitchFrame`.

### 2.3 Proxy d'egress HTTP local (`EgressProxy`)

- Démarre sur `127.0.0.1:0`, génère les drapeaux Chromium d'accompagnement (`--proxy-server=…`, `--proxy-bypass-list=<-loopback>`, `--disable-quic`, `--webrtc-ip-handling-policy=disable_non_proxied_udp`, `--force-webrtc-ip-handling-policy`) et journalise chaque requête/blocage via `EgressAuditEvent`.
- Toutes les tâches de connexion et de tunnel `CONNECT` sont possédées par un `JoinSet` borné (`MAX_CONCURRENT_PROXY_TASKS = 128`, acceptation bornée à `64` connexions, file `mpsc` de tunnels de capacité `64`, signal d'annulation `tokio::sync::watch`, délais `10 s` connexion/upgrade et `300 s` I/O), sans aucune tâche `tokio::spawn` détachée ; `EgressProxy::shutdown` révoque le mandat, émet l'annulation et attend la fermeture bornée du `JoinSet`.
- En HTTP clair (`browser-execution.md` §5.3), `EgressProxy` vérifie l'origine normalisée avant acheminement et intercepte toute réponse `3xx` redirigeant vers une origine absente de `allowed_origins` (ou vers un `model_canary_origins` sous `ModelPolicy::Forbidden`) en retournant `403 Forbidden`.
- **Limite connue sur `CONNECT` HTTPS** : pour les tunnels `CONNECT`, `EgressProxy` filtre l'autorité cible normalisée (`https://host:port`) sans déchiffrement TLS MITM ; les en-têtes/redirections chiffrés à l'intérieur d'un tunnel `CONNECT` ne sont pas lisibles par le proxy HTTP local sans casser TLS. Le blocage des origines tierces et canaris au niveau de la résolution `CONNECT` reste actif, mais le confinement réseau OS complet demeure requis au niveau `Strict`.

### 2.4 Limites du profil `project_script` et distinction d'isolation (`policy.rs`)

- **Exclusion structurelle en garantie sans modèle** : un runner tiers (`project_script`, ex. Playwright Test TypeScript) pouvant exécuter du code arbitraire et contacter le réseau si non confiné ne peut pas bénéficier d'une attestation `model_policy = forbidden` sans preuve de confinement OS.
- `check_execution_isolation` (et `FantocciniBrowserAdapter::open_context`) impose :
  - `ExecutionMode::Mediated` + `IsolationLevel::Standard` + `ModelPolicy::Forbidden` : **admis** (canal fermé sous contrôle du harness, sans modèle, avec `EgressProxy`).
  - `ExecutionMode::Mediated` + `IsolationLevel::Strict` + `ConfinementProof::Unconfined` : **refusé** (`BrowserError::IsolationUnavailable` → `AUTOMATION_PROFILE_UNAVAILABLE`).
  - `ExecutionMode::ProjectScript` + `ModelPolicy::Forbidden` + `ConfinementProof::Unconfined` : **refusé** (`BrowserError::IsolationUnavailable`). La garantie sans modèle en `project_script` exige un `ConfinementProof::VerifiedOsSandbox` fourni ultérieurement par `unleash-platform` / `unleash-process-host` (macOS Seatbelt / Linux bubblewrap+seccomp).

---

## 3. Résultats des tests sur serveur `WebDriver` fixture + site marchand synthétique

Commande vérifiée : `cargo test --workspace --all-features` (`94 passed; 0 failed; 0 ignored` sur le workspace après toutes éditions) :

- **Unitaires `unleash-browser` (`12 passed; 0 failed`)** :
  - Épinglage binaire/SHA-256, refus Safari en `mediated`, inventaire local détectant l'absence de `chromedriver` appairé (`local_inventory_reports_missing_paired_chromedriver`).
  - Rejet `XPath`, coordonnées, `execute_script` modèle ; contrôle cardinalité/identité.
  - Normalisation d'origine `EgressProxy` (rejet schémas non HTTP/HTTPS et `userinfo`), blocage canari sous `Forbidden`, et exécution/fermeture propre d'un tunnel `CONNECT` autorisé (`proxy_connect_tunnel_runs_and_shuts_down_cleanly`).
  - Distinction `mediated` / `project_script` + isolation `Standard` vs `Strict`.
  - Correspondance exacte des codes filaires F019 (`BrowserError` → `ErrorCode` / `ReasonCode` / `RetryClass`, y compris `OutcomeUnknown` → `ErrorCode::OutcomeUnknown` / `ReasonCode::UnresolvedEffect` / `RetryClass::AfterReconciliation`).
- **Intégration `tests/browser_integration.rs` (`6 passed; 0 failed`, zéro réseau externe)** :
  1. `t_f019_006_zero_model_calls_under_forbidden_with_canary_and_isolation_checks` (`REQ-AUTO-006`) : contrôle positif du serveur canari (`HitCounterServer`), blocage d'un `ActionIntent` à `requires_model_call = true` et d'un `ModelSuppliedExecuteScript`, parcours mécanique nominal complet par `Label` (`Fill`) + re-observation + `TestId` (`Click`) + `evaluate_assertions` retournant `all_passed == true` (toutes règles à `AssertionVerdict::Pass`, `ensure_passed(true).is_ok()`, `mutation_count == 1`), preuve `ZeroModelProofReport` à `0` appel modèle (`model_denied_attempt_count == 2`, `canary.hits() == 0`), plus rejet d'un `project_script` non confiné (`IsolationUnavailable`).
  2. `t_f019_009_cardinality_ambiguous_missing_drift_and_reordered_identity` (`REQ-AUTO-009`) : blocage avant clic sur DOM ambigu (`AmbiguousTarget` avec `candidate_count = 2`, 0 mutation sur le marchand), cible absente (`TargetMissing`), dérive SKU (`TargetDrift`), et réussite uniquement lorsque l'élément réordonné conserve l'identité métier et la cardinalité `1`.
  3. `t_f019_010_valid_click_wrong_business_outcome_blocks_without_retry` (`REQ-AUTO-010`) : exécute les 3 sous-scénarios (`MerchantMode::WrongQuantity`, `MerchantMode::WrongPrice`, `MerchantMode::WrongCurrency`) avec réinitialisation du marchand entre chaque cas ; chaque clic WebDriver est techniquement acquitté (`dispatch_confirmed == true`, `mutation_count == 1`) face à quantité `99`, prix unitaire `999` ou devise `"USD"`, puis `evaluate_assertions` retourne `all_passed == false`, **`AssertionVerdict::Fail`** sur la règle correspondante (`apple_qty_2`, `price_cap`, `currency_eur`) et `BrowserError::BusinessPostconditionFailed` via `report.ensure_passed(true)` sans aucun retry automatique (`mutation_count` reste à `1`).
  4. `t_f019_011_cut_after_dispatch_returns_outcome_unknown_without_retry` (`REQ-AUTO-011`) : enregistrement préalable **`EffectState::None`** par `InMemoryEffectJournal::record_pre_dispatch` avant envoi `WebDriver`, coupure simulée du serveur WebDriver juste après exécution de la mutation marchand (`mutation_count == 1`) basculant l'état vers **`EffectState::OutcomeUnknown`** → retour `BrowserError::OutcomeUnknown` (`ErrorCode::OutcomeUnknown` / `ReasonCode::UnresolvedEffect` / `RetryClass::AfterReconciliation`), deuxième soumission immédiate bloquée avant tout second dispatch (`mutation_count` reste à `1`), checkpoint invalidant les handles éphémères, puis réconciliation explicite par `reconcile_outcome`.
  5. `t_egress_proxy_blocks_unauthorized_origins_redirects_and_model_canary` : blocage direct d'une origine hors mandat, blocage d'une redirection `302` du marchand vers une origine tierce (`0` hit), blocage d'une requête directe vers le canari modèle sous `Forbidden` (`0` hit), blocage d'un `CONNECT` hors mandat (`403`) et blocage après `proxy.revoke()`.
  6. `t_runtime_pinning_safari_xpath_coordinates_and_model_script_rejections` : rejet `RuntimeMismatch` sans binaire réel ou empreinte divergente, rejet `CapabilityMissing` sur `SafariUnsupported`, rejet `UnsupportedLocatorOrScript` sur XPath/coordonnées/`execute_script` modèle via `TargetSpec::try_from_raw`, rejet `InvalidConfiguration` sur `frame_scope` vide, rejet explicite `BrowserError::CapabilityMissing` (`details_ref = "target.frame_scope"`, `ErrorCode::CapabilityDenied` / `ReasonCode::CapabilityUnavailable`) dans `resolve_target` et `evaluate_assertions` pour `frame_scope: Some(...)`, et inventaire passif `inventory_local_runtimes` (`paired_chrome_driver_ready = false`).

---

## 4. Prochaines étapes hors périmètre M0 (vers `EXT-BROWSER` en M2/M4)

1. Provisionner dans un répertoire runtime approuvé un couple `Google Chrome for Testing` + `chromedriver` **du même build** avec version et empreintes `SHA-256` fixées (notamment `chromedriver` appairé à `156.0.8078.4` sur macOS `arm64`, et couple Linux `x86_64` pour `H2`).
2. Implémenter côté `unleash-process-host` / `unleash-platform` la génération du `ConfinementProof::VerifiedOsSandbox` (Seatbelt / bubblewrap+seccomp) et le lancement supervisé du processus `chromedriver` en passif jusqu'à l'ouverture de contexte sous mandat.
3. Rejouer `T-F019-006`, `T-F019-009`, `T-F019-010`, `T-F019-011` contre un véritable navigateur Chrome for Testing headless + site marchand synthétique pour produire une preuve réelle `EXT-BROWSER`.
