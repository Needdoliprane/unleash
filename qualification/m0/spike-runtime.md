# Spike 3 Runtime — Qualification `unleash-process-host` et supervision de processus (M0)

Date : 2026-11-17
Jalon : M0 (`g0001-m0`, tâche `t06`)
Périmètre : `crates/unleash-process-host` (`REQ-TERM-002..008`, `REQ-TERM-010`, `F011`, `ADR-05`, `Q-05`, `T-F011-002..010`, `T-SEC-03/04`)
Statut des tests produit dans le dossier de conception : inchangé (`not_run` hors présent rapport M0)

---

## 1. Critères de qualification annoncés avant mesure

Avant implémentation de `crates/unleash-process-host`, le spike runtime M0 devait trancher expérimentalement cinq exigences d'architecture sur l'OS cible :

1. **Survie du `process-host` au `SIGKILL` du daemon et fencing de lignée (`REQ-TERM-003`)** : un `unleash-process-host` lancé par un processus daemon en session détachée (`setsid()`) doit survivre à la mort brutale (`SIGKILL`) du daemon, conserver le PTY actif et le spool durable sur disque, accepter le rattachement d'un nouveau `daemon_epoch` strictement croissant dans la même `authority_generation` avec rotation de secret de contrôle, et rejeter immédiatement les anciens canaux, anciens secrets, anciennes époques/fences ainsi que toute autre `authority_generation`.
2. **Ancrage non réutilisable et sérialisation récolte vs signal (`REQ-TERM-002`, `REQ-TERM-006`)** : aucun signal (`SIGTERM`/`SIGKILL` sur PID ou `-PGID`) ne doit être émis sur la seule foi d'un PID observé. L'ancrage doit exiger un enfant direct non récolté et une disposition `SIGCHLD` qualifiée (non `SIG_IGN`, sans `SA_NOCLDWAIT`), sérialiser toute lecture/signal avec la récolte sous un unique verrou, et exposer honnêtement les descendants détachés (`setsid`) lors d'un `Stop`.
3. **Ambiguïté d'entrée PTY sans répétition automatique (`REQ-TERM-005`)** : l'écriture au maître PTY est encadrée par deux transitions durables (`WriteDispatched` avant `write_all`, `ReceiptConfirmed` après). Un `SIGKILL` du `process-host` au point de coupure `C5` (après écriture effective au PTY et avant persistance du reçu) doit classer l'entrée en `OutcomeUnknown` lors de la réconciliation et interdire toute répétition automatique.
4. **Spool borné, séparation volatile/durable, redaction et zéro modèle (`REQ-TERM-004`, `REQ-TERM-008`)** : les flux PTY/service sont drainés en continu (y compris sans client connecté), rédigés contre les secrets canaris (mono-ligne et multi-lignes, même fragmentés entre lectures ou normalisés par `ONLCR`), bornés en segments `0600` avec empreinte SHA-256 et création explicite de `LogGap` en cas de saturation. Le `process-host` rejette toute requête d'invocation modèle (`ModelCallsForbidden`).
5. **Mode `strict` Q-05 sans repli silencieux (`T-SEC-03`, `T-SEC-04`)** : sur macOS, génération et application d'un profil Seatbelt `/usr/bin/sandbox-exec` bloquant la lecture de `data_root`, `socket_dir`, des trousseaux et des emplacements de credentials déclarés, bloquant l'écriture hors des racines assignées, limitant la sortie réseau au proxy loopback autorisé, et refusant explicitement par `IsolationUnavailable` (`IsolationBackendMissing` / `IsolationObligationUnenforceable`) sans jamais rétrograder silencieusement vers `standard`.

Toute synchronisation de test par `sleep` était interdite : les scénarios d'intégration utilisent exclusivement des tubes, sockets Unix de synchronisation, FIFOs (`mkfifo`) et marqueurs ligne `stdout`.

---

## 2. Environnement, dépendances et observations système XNU/macOS

### 2.1 Plateforme et outillage mesurés

| Élément | Valeur observée |
| --- | --- |
| Système hôte | macOS `27.2`, Build `26B123` (Apple Silicon) |
| Chaîne Rust | `rustc 1.94.1 (e408947bf 2026-03-25)`, `cargo 1.94.1 (29ea6fb6a 2026-03-24)` |
| Audit dépendances | `cargo-deny 0.21.0` (`advisories ok, bans ok, licenses ok, sources ok`) |
| Backend PTY | `portable-pty = "0.9.0"` (POSIX `openpty` + spawn enfant) |
| Primitives système | `rustix = "1.1.6"` + `libc` (limités aux modules ciblés `anchor`, `isolation`, `manifest`, `main` avec `#[allow(unsafe_code)]` justifiés) |
| Backend sandbox macOS | `/usr/bin/sandbox-exec` présent (`-rwxr-xr-x 1 root wheel 135136`) |
| Stockage de travail | Dépôt sur partage réseau SMB ; répertoires runtime des tests placés sur disque local sous `/tmp` via `unleash_testkit::local_test_dir` |

### 2.2 Constats empiriques critiques sur noyau XNU / PTY

Cinq comportements réels du noyau macOS et de `portable-pty` ont été mis en évidence et intégrés au design de `unleash-process-host` :

1. **Symlink `/tmp` → `/private/tmp` dans Seatbelt (`sandbox-exec`)** : les règles SBPL `(subpath ...)` évaluent les chemins canoniques du noyau. Autoriser `/tmp/...` sans résolution échoue à l'ouverture en écriture. `render_macos_seatbelt_profile` canonicalise les chemins résolubles et émet les deux formes lorsqu'elles diffèrent, tout en bloquant explicitement toute racine d'écriture englobant `data_root` ou `socket_dir`.
2. **Fenêtre `exit1()` / zombie `SZOMB` sous `proc_pidinfo` vs `waitid(WNOWAIT)`** : sur macOS, `proc_pidinfo(PROC_PIDTBSDINFO)` retourne `0` (`ESRCH`) dès que le processus enfant entre dans `exit1()` ou devient un zombie non récolté (`SZOMB`), alors même qu'il est encore notre enfant direct non récolté. Inversement, `waitid(P_PID, pid, &siginfo, WEXITED | WNOHANG | WNOWAIT)` retourne `0` tant que l'enfant direct n'a pas été récolté (sans le récolter), puis `-1` (`ECHILD`) après `waitpid`. `OwnedChildAnchor` combine donc la vérification d'identité sur processus vivant et `is_unreaped_direct_child()` via `waitid(WNOWAIT)` sous le même `Mutex<AnchorInner>`.
3. **`kill(-pgid, sig)` sur macOS lorsqu'il ne reste qu'un leader zombie dans le PGID** : lorsque le leader du groupe est déjà zombie non récolté et qu'aucun autre processus vivant n'appartient au groupe, XNU renvoie `EPERM` (`errno = 1`) sur `kill(-pgid, SIGTERM/SIGKILL)`. En revanche, si au moins un petit-enfant vivant reste dans le même PGID (même après sortie du leader), `kill(-pgid, SIGTERM)` retourne `0` et atteint ce petit-enfant. `anchor.rs` tolère `EPERM` uniquement après constat que le groupe ne contient plus de membre actif, sans jamais masquer la signalisation d'un descendant vivant.
4. **Normalisation `\n` → `\r\n` par la discipline de ligne PTY (`OPOST | ONLCR`)** : le maître PTY reçoit `\r\n` lorsque l'enfant écrit `\n` sur sa sortie esclave. Un secret canari multi-lignes contenant `\n` apparaîtrait donc avec `\r\n` dans le flux capté, y compris coupé en frontière de lecture. `SecretRedactor::new` enregistre automatiquement la variante `\r\n` de chaque canari contenant `\n` et applique un tampon de retenue minimal égal au plus long préfixe partiel possible sans retarder les octets ordinaires.
5. **Héritage d'environnement par défaut de `portable-pty::CommandBuilder::new`** : contrairement à `std::process::Command` après `env_clear()`, `portable-pty` pré-remplit `CommandBuilder::new` avec `std::env::vars_os()`. `unleash-process-host` force systématiquement `cmd.env_clear()` avant d'appliquer la liste blanche nettoyée `build_clean_child_env` (excluant clés d'API, secrets et préfixes `UNLEASH_*`), puis ferme le descripteur esclave PTY côté parent immédiatement après `spawn_command`.

---

## 3. Matrice de qualification : point de coupure × composant × état observé

Toutes les lignes ci-dessous proviennent des exécutions réelles multi-processus (`cargo test -p unleash-process-host`) et des tests unitaires déterministes exécutés sur cette machine :

| Scénario / Point de coupure | Composant touché | État et comportement observés | Vérification |
| --- | --- | --- | --- |
| **`SIGKILL` daemon pendant PTY actif** | `daemon` | Terminé par `SIGKILL` (`status.signal() == Some(SIGKILL)`), ancien socket client fermé. | `t_f011_003` |
| **`SIGKILL` daemon pendant PTY actif** | `unleash-process-host` | Processus `setsid()` reste vivant (même PID), secret de contrôle absent de `argv` (`ps -o args=`), drainage PTY maintenu pendant l'absence du daemon. | `t_f011_003` |
| **`SIGKILL` daemon pendant PTY actif** | `enfant` (PTY worker) | Continue l'exécution sans coupure et produit `PTY_POST_KILL_DATA`. | `t_f011_003` |
| **Reprise même lignée (`gen=A`, `epoch=2 > 1`) + rotation secret** | `daemon` ↔ `process-host` | `HandshakeAck { secret_rotated: true }` ; canal connecté pré-rotation et ancien `secret_1` immédiatement rejetés (`AuthenticationFailed`) ; `SendInput` (`InputReceipt`), `Resize` (`ResizeAck`), `Stop` (`StopObserved`) et `ReadLog` restituent l'intégralité des logs pré/post-crash avec `LogSpool::verify_durable_segments` valide. | `t_f011_003` |
| **Reprise autre `authority_generation` (`gen=B`, même `epoch=9999`) ou `epoch` inférieur (`1 < 2`)** | `daemon` ↔ `process-host` | Refus strict `StaleFence` (`ReasonCode::StaleAuthorityGeneration` ou `StaleEpoch`, `RetryClass::Never`). | `t_f011_003` |
| **Point `C3` / `C4` (avant ou juste après `WriteDispatched`, avant écriture PTY)** | `pty_input` / `enfant` | En absence d'écriture PTY confirmée : en `C3` l'entrée n'est pas engagée ; en `C4` un marqueur `WriteDispatched` sans écriture réelle bascule à la réconciliation en `OutcomeUnknown` sans répétition silencieuse. | Unitaires `pty_input` |
| **Point `C5` (`SIGKILL` `process-host` après `write_all` PTY effectif + commit oracle, avant `ReceiptConfirmed`)** | `unleash-process-host` | Bloqué sur barrière `UNLEASH_CRASH_REACHED:C5` puis tué par `SIGKILL` avant écriture du reçu durable. | `t_f011_005` |
| **Point `C5` (`SIGKILL` `process-host`)** | `enfant` + `ExternalEffectOracle` | Effet exécuté exactement 1 fois (`oracle.count() == 1`). | `t_f011_005` |
| **Point `C5` → Réconciliation & tentative de replay `input_seq=1`** | Réconciliateur hors ligne + `PtyInputTracker` | `reconcile_after_lost_host` classe `unknown_input_seqs = [1]` et refuse tout signal (`IDENTITY_UNVERIFIED`) ; relancer `dispatch_input` avec `input_seq=1` échoue en `TerminalInputOutcomeUnknown` avec 0 octet écrit et oracle toujours à `1`. | `t_f011_005` |
| **`SIGKILL` `process-host` hors `C5` sur ressource PTY** | `unleash-process-host` / `enfant` | Segments durables `spool/seg-NNNNNN.log` conservés avec SHA-256 ; la fermeture du maître PTY provoque `Exited` (SIGHUP/EOF) ou `Running` si SIGHUP ignoré ; signal sur simple PID refusé (`IDENTITY_UNVERIFIED`). | `t_f011_006` |
| **`SIGKILL` `process-host` hors `C5` sur ressource `Service`** | `unleash-process-host` / `enfant` | L'enfant survit (`HostLaunchState::Running`) ; réconciliation refuse le contrôle par signal (`IDENTITY_UNVERIFIED`). | `t_f011_006` |
| **Redémarrage machine simulé (`boot_identity` modifiée)** | Réconciliateur hors ligne | `reconcile_after_lost_host` clôt le run en `HostLaunchState::Lost` sans émettre de signal. | `t_f011_006` |
| **`StopProcess` sur arbre leader + petit-enfant même PGID + descendant `setsid()`** | `OwnedChildAnchor` / `enfant` arbre | Leader et petit-enfant même PGID terminés (`direct_exit: Some(...)`, `group_signaled: true`) ; descendant `setsid()` signalé explicitement dans `detached_or_unknown_descendants` avec `confirmed_group_clear: false`. | `t_f011_006` |
| **Saturation spool + canaris + requête modèle autonome** | `LogSpool` / `SecretRedactor` / `ProcessHostServer` | Rotation disque bornée + `LogGap` explicite ; canaris mono et multi-lignes absents des segments et du manifeste (`redaction_count >= 2`) ; `ModelCallAttempt` rejeté (`CapabilityDenied` / `ModelCallsForbidden`). | `t_f011_008`, `t_f011_004` |
| **Mode `strict` Q-05 sous macOS Seatbelt (`/usr/bin/sandbox-exec`)** | `enfant` sandboxé | Écriture dans `work_dir` autorisée ; écriture hors racine refusée (`must_not_exist`) ; lecture `data_root`, `socket_dir`, credentials, Keychains refusée ; TCP permis uniquement vers `egress_proxy_addr` ; aucune variable secrète héritée ; manifeste mis à jour avec `macos-seatbelt` et digest du profil. | `t_sec_03` |
| **Backend absent ou obligation inapplicable en mode `strict`** | `ProcessHostServer::start` | Refus avant spawn (`IsolationBackendMissing` / `IsolationObligationUnenforceable` → `ErrorCode::IsolationUnavailable`), marqueur enfant absent, zéro repli vers `standard`. | `t_sec_04` |
| **Signal sans ancrage, post-récolte, réutilisation PID/PGID ou `SIGCHLD` invalide** | `OwnedChildAnchor` | Refus `IdentityUnverified` avant émission ; processus témoin innocent maintenu vivant ; refus si `SIGCHLD` vaut `SIG_IGN` ou `SA_NOCLDWAIT`. | `t_f011_002` (unit.) |

---

## 4. Résultats des suites de tests

Toutes les commandes ci-dessous ont été exécutées sur la révision finale du worktree `g0001-m0-t06` (avec `CARGO_TARGET_DIR="$HOME/Library/Caches/unleash/target/g0001-m0-t06"`) :

| Vérification | Commande | Résultat observé |
| --- | --- | --- |
| Formatage | `cargo fmt --all -- --check` | **Passé (`exit 0`)** — aucune divergence |
| Clippy strict | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **Passé (`exit 0`)** — zéro warning/erreur (`1.34s`) |
| Tests workspace (borne 240s) | `cargo test --workspace --all-features` | **Passé (`exit 0`)** — **151 passés, 0 échoué, 0 ignoré** (`unleash-process-host` : 15 unitaires `0.15s` + 9 intégration `0.62s` = 24 passés ; `unleash-browser` 18 ; `unleash-client` 9 ; `unleash-domain` 17 ; `unleash-platform` 6 ; `unleash-protocol` 30 ; `unleash-store` 18 ; `unleash-testkit` 29 ; doc-tests : 0) |
| Documentation sans warning | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | **Passé (`exit 0`)** — zéro warning |
| Politique dépendances | `cargo deny check` | **Passé (`exit 0`)** — `advisories ok, bans ok, licenses ok, sources ok` |

Détail des tests de `crates/unleash-process-host` :
- **Tests unitaires (`src/lib.rs`) — 15 tests** : `anchor::tests::{t_f011_002_refuses_unanchored_signal_and_pid_reuse_keeping_witness_alive, t_f011_002_refuses_when_sigchld_unqualified}`, `error::tests::error_ipc_projections_match_canonical_catalog`, `fencing::tests::{rejects_foreign_authority_generation_even_with_higher_epoch, rejects_lower_epoch_or_lower_fence_in_same_lineage}`, `isolation::tests::{linux_bwrap_plan_includes_unshare_net_clearenv_and_binds, strict_refuses_when_backend_missing_never_falls_back_to_standard, strict_rejects_overlapping_write_root_over_data_root}`, `manifest::tests::{control_secret_redacted_in_debug_and_display_and_ct_eq, manifest_and_secret_are_stored_0600_and_never_contain_raw_secret}`, `protocol::tests::host_request_debug_never_leaks_secrets`, `pty_input::tests::{confirmed_input_is_deduplicated_without_rewriting, t_f011_005_crash_after_pty_write_before_receipt_becomes_outcome_unknown_without_retry}`, `spool::tests::{redactor_masks_split_chunk_and_multiline_canaries_without_delaying_ordinary_text, spool_separates_volatile_from_durable_and_records_explicit_gap_on_quota}`.
- **Tests d'intégration runtime (`tests/process_host_runtime.rs`) — 5 scénarios réels + 4 entrypoints de fixtures** : `t_f011_003_sigkill_daemon_during_active_pty_survives_and_reattaches_same_lineage_only`, `t_f011_005_sigkill_host_after_pty_write_before_receipt_outcome_unknown_zero_repeat`, `t_f011_006_t_f011_010_sigkill_host_preserves_logs_and_stop_reports_detached_descendant`, `t_f011_008_t_f011_004_spool_redaction_gap_and_zero_model`, `t_sec_macos_seatbelt_strict_mode_and_isolation_unavailable_negative`.

---

## 5. Architecture retenue et limites constatées

### 5.1 Décisions d'architecture validées

- **Modèle 1-host / 1-ressource en session détachée (`setsid`)** : le binaire `unleash-process-host` ne reçoit qu'un chemin `--bootstrap <path>` en ligne de commande ; secret de contrôle (`0600`, `secrecy::SecretString` + `zeroize`), manifeste privé durable (`host.manifest.json`, `0600`) et canaris restent sur disque local privé, jamais en `argv`.
- **Fencing à deux axes via `unleash_domain::LineageFence`** : l'identité de lignée (`authority_generation`) impose une égalité exacte sans comparaison d'UUID ; dans la même lignée, `(daemon_epoch, resource_fence)` progresse de manière monotone. Toute rotation de secret révoque immédiatement les connexions antérieures.
- **Sérialisation stricte ancrage / signal / récolte** : `OwnedChildAnchor` vérifie à la fois la qualification `SIGCHLD`, la non-récolte locale, `boot_identity` + `start_identity` (et `waitid(WNOWAIT)` lors de la fenêtre `exit1()`/`SZOMB` macOS ou `pidfd_open`/`pidfd_send_signal` sous Linux) sous le même mutex que `waitpid`.
- **Spool par segments immuables + journal d'entrées 2 phases** : séparation explicite entre segments durables `sync_all()` avec SHA-256 et queue volatile ; redaction consciente de `\r\n` PTY ; aucune répétition automatique après `C5`.
- **Fail-closed sur l'isolation `strict`** : sonde d'exécution réelle (`/usr/bin/sandbox-exec -f <profil> -- /usr/bin/true` sur macOS, plan `bwrap --unshare-all --clearenv` + seccomp sous Linux) avant lancement de l'enfant ; tout défaut d'outillage ou de politique retourne `IsolationUnavailable`.

### 5.2 Limites et écarts constatés

1. **Runtime Linux non exécuté sur cet hôte** : les chemins Linux (`pidfd_open`/`pidfd_send_signal`, `build_linux_bwrap_plan`, `prepare_linux_bwrap_launch` avec `bwrap` + `seccomp`) sont implémentés et vérifiés côté construction de plan (`linux_bwrap_plan_includes_unshare_net_clearenv_and_binds`), mais l'exécution réelle sur un noyau Linux avec `bubblewrap` et filtres `seccomp` reste à valider en environnement Linux cible avant V1.
2. **Enfants détachés par `setsid()` côté macOS** : conformément au modèle POSIX/XNU, un descendant qui crée sa propre session (`setsid`) quitte le PGID du leader et ne peut pas être garanti terminé par un signal sur `-pgid`. Le `process-host` ne fabrique pas une confirmation fausse : il expose ces PID dans `detached_or_unknown_descendants` et marque `confirmed_group_clear = false` pour signalement au superviseur/opérateur.
3. **Note de portabilité sur macOS Seatbelt (`/usr/bin/sandbox-exec`)** : Apple déprécie formellement `sandbox-exec` dans les notes publiques bien qu'il reste pleinement fonctionnel et imposé par le noyau sur macOS 27.2 (`Build 26B123`). Le test `T-SEC-03` doit être maintenu en exécution continue à chaque mise à jour majeure de macOS.
