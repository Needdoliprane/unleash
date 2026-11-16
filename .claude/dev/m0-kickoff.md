# Démarrage de la réalisation — jalon M0

**Statut : feuille de route adoptée en R7.** M0 réduit les inconnues qui changent les fondations ([plan V1](../plans/v1-implementation-plan.md), [chemin critique](../plans/critical-path.md)). Chaque tâche ci-dessous est un candidat de goal ; un goal peut en regrouper plusieurs. Les critères de terminé s'ajoutent à la [définition de terminé](definition-of-done.md).

## Arborescence du dépôt

```text
AGENTS.md CLAUDE.md GEMINI.md        consignes des agents (AGENTS.md fait foi)
Cargo.toml Cargo.lock                workspace virtuel unique (créé en T1)
rust-toolchain.toml rustfmt.toml clippy.toml deny.toml
crates/unleash-*/                    crates du produit (découpage canonique de rust-workspace.md)
apps/gui-renderer/                   renderer React/TypeScript de la GUI (ADR-03), hors workspace Cargo
spikes/m0-0N-*/                      expériences M0, chacune avec son Cargo.toml, exclues du workspace
qualification/m0/                    rapports d'expériences, inventaire des dépendances, sortie M0
qualification/evidence/              preuves réelles EV-… (jamais dans .claude/)
tools/design-package/                validation et régénération du dossier
.claude/                             dossier de conception et outillage des agents (worktrees sous .claude/worktrees/)
```

## Modèle de `Cargo.toml` racine

Configuration vérifiée avec la toolchain 1.94.1 (lints acceptés, `unwrap` refusé hors tests, autorisé dans les tests via `clippy.toml`). Les tests live payants utilisent `dotenvy` pour lire `.env` (voir [règles de test](../rules/testing.md)). Ajouter les membres au fur et à mesure : un workspace dont le motif `crates/*` ne correspond à aucune crate ne se charge pas.

```toml
[workspace]
resolver = "3"
members = ["crates/unleash-domain"]
exclude = ["spikes"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.94.1"
license = "Apache-2.0"
publish = false
repository = "https://github.com/Needdoliprane/unleash"

[workspace.lints.rust]
unsafe_code = "deny"  # forbid interdirait la dérogation des adaptateurs OS
missing_docs = "warn"
unreachable_pub = "warn"
unused_must_use = "deny"
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
must_use_candidate = "allow"
unwrap_used = "deny"
expect_used = "warn"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
print_stdout = "warn"
print_stderr = "warn"
indexing_slicing = "warn"

[profile.release]
lto = "thin"
codegen-units = 1
```

Chaque crate : `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `license.workspace = true`, `publish.workspace = true` et `[lints] workspace = true`. Toutes les crates hors adaptateurs placent `#![forbid(unsafe_code)]` à leur racine ; seules les crates d'adaptateurs OS recensées (`unleash-platform`, `unleash-process-host`) lèvent `unsafe_code` localement avec `#[allow(unsafe_code)]` ciblé et un commentaire `// SAFETY:` justifié (conformément à [rust-workspace.md](../architecture/rust-workspace.md), évitant l'erreur `rustc E0453`), selon les [règles Rust](../rules/rust.md).

## Tâches M0, dans l'ordre

| # | Tâche | À lire | Terminé quand |
| --- | --- | --- | --- |
| T1 | Gouvernance du dépôt : `git init -b main`, `git config core.fileMode false` (partage SMB), vérifier `.gitignore` et `.gitattributes`, commit de base | `AGENTS.md`, [worktrees](worktrees.md) | `git status` propre ; `python3 tools/design-package/validate.py .` à 0 erreur sur le commit de base |
| T2 | Workspace : `Cargo.toml` ci-dessus, crate `unleash-domain` minimale | [rust-workspace](../architecture/rust-workspace.md), [règles Rust](../rules/rust.md) | fmt, clippy `-D warnings`, test et doc verts ; `Cargo.lock` versionné |
| T3 | `unleash-domain` : identifiants newtypes (Command, Request, Receipt, Agent, Invocation, Attempt, Run, Action, Session…), générations, époques et fences, états des machines à états et transitions pures | [session-model](../architecture/session-model.md), [state-machines](../architecture/state-machines.md) | tests table-driven reproduisant les tables de transitions, propriétés sur les transitions interdites |
| T4 | `unleash-protocol` : DTO versionnés (CommandEnvelope v2, receipt, Error et classes, Event, handshake), `deny_unknown_fields`, digests RFC 8785 + SHA-256 | [IPC](../architecture/ipc-and-apis.md), [schémas](../schemas/README.md), F001/F002 `interfaces.md` | aller-retour et validation JSON Schema réussis sur `.claude/examples/ipc-command.json` et les blocs wire des features ; cas de rejet testés |
| T5 | `unleash-testkit` : horloge et identifiants injectés, points d'injection de pannes nommés, harness de crash par processus enfant | [crash-recovery-tests](../testing/crash-recovery-tests.md) | un test tue un enfant à un point d'injection et lit un compteur d'effet indépendant ; aucun `sleep` de synchronisation |
| T6 | Expérience SQLite dans `unleash-store` : migration initiale, writer unique sur thread dédié et file bornée, état + événement + outbox dans la même transaction, crash injecté entre commits | [sqlite-design](../architecture/sqlite-design.md), [ADR-01](../architecture/adr/ADR-01-sqlite-persistence.md), F002 | tests T-F002 visés au vert sur SQLite locale temporaire ; `sqlite_source_id()` consigné dans `qualification/m0/` |
| T7 | `unleash-platform`, `unleash-client`, `unleash-daemon`, `unleash-cli` minimaux : verrou d'autorité par UID, socket protégé, handshake, `GetDaemonStatus` | [runtime-lifecycle](../architecture/runtime-lifecycle.md), F001 | T-F001 visés au vert sur macOS ; second démarrage refusé `AuthorityAlreadyActive` |
| T8 | Expérience runtime : worker et process-host séparés, kill du daemon, du host et de l'enfant à des points identifiés | [process-supervision](../architecture/process-supervision.md), [ADR-05](../architecture/adr/ADR-05-durable-sessions.md) | rapport `qualification/m0/spike-runtime.md` : critères annoncés avant mesure, matrice point de coupure × composant × état observé |
| T9 | Tranche verticale GUI/TUI : écran mission (arbre de 1 000 agents logiques, 100 000 événements paginés/virtualisés, diff de 10 000 lignes, timeline virtualisée et inspecteur, charge d'AC-F014-011) en Tauri 2 + React/TypeScript et en TUI Ratatui/Crossterm, sur un serveur de fixtures du protocole | [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md), [parité](../design/gui-tui-parity.md) | rapport `qualification/m0/spike-ui.md` : clavier, IME, lecteur d'écran, CPU, restauration du terminal ; défaut éliminatoire signalé au rôle `architect` |
| T10 | Expérience contrats et connaissance : patch multilangage, worktree sale modifié pendant analyse, faux rapport de tests | [contract-enforcement](../specs/contract-enforcement.md), [shared-knowledge](../specs/shared-knowledge.md) | les trois cas produisent `unknown`/`insufficient` attendus avec observations conformes aux schémas |
| T11 | Expérience navigateur : `fantoccini` sur Chrome for Testing et chromedriver épinglés, fixture locale ; cible ambiguë, clic valide au résultat métier faux, effet inconnu, arrêt et reprise | [browser-execution](../architecture/browser-execution.md), F019 | rapport `qualification/m0/spike-browser.md` avec versions exactes, zéro appel modèle mesuré, limites du profil `project_script` |
| T12 | Inventaire des dépendances : `cargo tree`, `cargo deny check`, dépendances natives et runtimes externes, licences | [ADR-14](../architecture/adr/ADR-14-local-data-oss.md), `deny.toml` | `qualification/m0/dependencies.md` généré par commande reproductible ; `cargo deny check` vert |
| T13 | Sortie M0 : bilan critère par critère, écarts de contrat corrigés dans `.claude/`, demandes externes lancées (AI-Guard, OAuth, Apple Developer) | [readiness checklist](../plans/readiness-checklist.md) | `qualification/m0/exit.md` relie chaque critère à sa preuve ; validateur du dossier à 0 erreur |

T3 à T5 précèdent tout développement parallèle des autres crates : ce sont les contrats partagés. T6 à T11 peuvent ensuite avancer en parallèle dans des worktrees distincts.
