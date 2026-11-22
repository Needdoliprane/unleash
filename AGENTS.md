# OVPT Unleash — consignes des agents de développement

Ce fichier fait foi pour tous les agents (Codex, Claude Code via `CLAUDE.md`, Antigravity avec `GEMINI.md`). Il est court : les détails sont dans les documents cités, à lire avant d'agir.

## 1. Le projet et son état

OVPT Unleash est un orchestrateur local d'agents et de procédures réutilisables : daemon Rust par utilisateur, SQLite, sessions durables, workers éphémères, workflows bornés, skills et contrats versionnés, Git et worktrees, GUI et TUI à parité, automatisation navigateur, modèles cloud et locaux, AI-Guard. Produit open source sous licence Apache-2.0, visé comme V1 distribuable.

La **révision R7** de `.claude/` fixe les contrats. Dans le dépôt de réalisation, le socle M0 est intégré à `dev` et décrit dans `qualification/m0/exit.md`. Le programme `.claude/state/goals/g0000-v1/goal.md` indique le jalon actif et ses preuves. Les tests non exécutés restent `not_run`.

## 2. Ordre de lecture

1. Ce fichier, puis [`.claude/brief/revision-2026-11-16-r7.md`](.claude/brief/revision-2026-11-16-r7.md) (décisions en vigueur).
2. [`.claude/brief/invariants.md`](.claude/brief/invariants.md) (65 invariants confirmés) et [`.claude/README.md`](.claude/README.md) (autorités documentaires).
3. [`.claude/dev/goal-protocol.md`](.claude/dev/goal-protocol.md), [`.claude/dev/m0-kickoff.md`](.claude/dev/m0-kickoff.md), [`.claude/dev/definition-of-done.md`](.claude/dev/definition-of-done.md).
4. Pour une tâche : la feature concernée dans `.claude/features/Fnnn-*/` (requirements, technical-design, interfaces, dependencies, acceptance, tests), puis les contrats canoniques qu'elle cite.

## 3. Autorités

En cas de divergence : invariants confirmés > ADR acceptés et contrats canoniques > features > résumés > audits (historiques). Contrats canoniques : `session-model` (Agent/Invocation/Attempt), `workflow-dsl` + `state-machines` (exécution), `ipc-and-apis` + `schemas/` (wire), `sqlite-design` (stockage), `shared-knowledge` + schéma discovery (provenance), `skills-contract-model` (bindings), `workflow-data-iteration` (profil v1.2), `model-routing` (historique et données dérivées). Si le code révèle une erreur de contrat : corriger le dossier dans le même goal ([règles du dossier](.claude/rules/design-dossier.md)), jamais contourner en silence. Un invariant `INV-…` ne change que sur décision de Louis.

## 4. Invariants à ne jamais casser (résumé)

- Cœur applicatif et système en **Rust** ; bibliothèques existantes et HTML/CSS autorisés ; aucune règle métier, transition ou autorisation dans un client (GUI, TUI, CLI).
- **Un daemon, une autorité, un ledger par utilisateur OS** ; GUI, TUI et CLI sont des clients reconnectables des mêmes opérations.
- **Admission globale 5 par défaut, 50 au maximum**, partagée par invocations d'agents, workers de replay et probes ; jamais de sixième place cachée.
- **Hiérarchie N+2 / N+1 / N** ; le harness exécute seul les transitions ; une définition engagée ne change pas en cours de run (frontière de migration = le Run).
- **Effets externes** : intention durable, autorisation, dispatch, observation, issue inconnue, réconciliation ; une issue inconnue n'est jamais un succès ni un retry automatique.
- **Contrats fermés et versionnés** ; claims, observations et jugements séparés ; une preuve attendue n'est jamais déclarée acquise sans exécution réelle.
- **Replay sans modèle** et **supervision par blocs** distincts ; `model_policy=forbidden` interdit tout appel modèle transitif ; `local_only` interdit tout repli cloud.
- **Parité fonctionnelle GUI/TUI complète** (58 groupes `PAR-…`) ; GUI et TUI soignées et accessibles.
- **Secrets** jamais dans les logs, le contexte d'un modèle, les arguments de commande ou le dépôt.
- `.claude/` est la référence documentaire : le produit ne le lit jamais à l'exécution et n'en stocke rien dedans.

## 5. Décisions techniques arrêtées

Rust 1.94.1 (`rust-toolchain.toml`), édition 2024, MSRV = toolchain ; Tokio ; `rusqlite` avec SQLite embarquée, writer unique sur thread dédié ; `serde` avec `deny_unknown_fields`, digests RFC 8785 + SHA-256 ; `thiserror` dans les bibliothèques ; `tracing` ; `clap` 4 ; TUI Ratatui + Crossterm ; GUI Tauri 2 avec renderer React + TypeScript strict (types générés par ts-rs) ; clients modèles écrits par Unleash (`reqwest` + `rustls`) ; navigateur `fantoccini` sur Chrome for Testing ; crates `unleash-*` du [découpage canonique](.claude/architecture/rust-workspace.md), dont `unleash-client` pour l'accès au daemon. Détails et raisons : ADR de `.claude/architecture/adr/`.

## 6. Disposition du dépôt

```text
crates/unleash-*/        crates du produit (workspace Cargo racine, créé en M0)
apps/gui-renderer/       renderer React/TypeScript (hors workspace Cargo)
spikes/                  expériences M0, exclues du workspace
qualification/           rapports M0 (m0/) et preuves réelles (evidence/EV-….json)
tools/design-package/    sync.py, regen.py, validate.py (outillage du dossier)
.claude/                 dossier de conception + outillage des agents ; worktrees sous .claude/worktrees/
.agents/ .codex/         skill et règles pour Codex/Antigravity ; agents Codex
```

## 7. Comment travailler

- **Goal autonome** : un objectif donné par `/goal …` (Claude Code, Antigravity), `$unleash-goal …` (Codex) ou un message commençant par `GOAL:` est conduit selon le [protocole de goal](.claude/dev/goal-protocol.md) : cadrage, plan, délégation, vérification indépendante, intégration, journal dans `.claude/state/goals/<goal-id>/`.
- **Choix du modèle** : l'agent root choisit pour chaque délégation le niveau `frontier`, `strong`, `standard` ou `fast` selon [le routage](.claude/dev/model-routing.md) et le consigne dans le journal.
- **Worktrees** : uniquement sous `.claude/worktrees/<goal-id>-<task-id>`, nulle part ailleurs ([règle](.claude/rules/worktrees.md)). Un worktree = une tâche = un seul agent écrivain.
- **Suivi** — après chaque transition, mettre à jour les fiches et régénérer `.claude/state/current-state.md` selon le [protocole de goal](.claude/dev/goal-protocol.md#suivi-des-travaux-journal-et-reprise). Retirer branches intégrées, worktrees et caches arrivés en fin d’usage selon leur [cycle de vie](.claude/dev/worktrees.md).
- **Retex** — à la suite d’une correction de Louis, d’une preuve invalidée ou d’un blocage qui revient, appliquer le [skill `retex`](.claude/skills/retex/SKILL.md) avant de reprendre l’action concernée.
- **Tests d'abord** pour chaque exigence `REQ-…` : écrire le test `T-…` du dossier, le voir échouer, implémenter ([règles de test](.claude/rules/testing.md)).
- **Règles de code** : [Rust](.claude/rules/rust.md), [renderer GUI](.claude/rules/gui-renderer.md). Commits au format Conventional Commits citant les `REQ-…`.
- **Interfaces**. Lire et appliquer le [design system OVPT d’Unleash](.claude/design/visual-design-system.md) avant tout changement GUI ou TUI. Inventorier le rendu et les composants existants, puis vérifier l’accessibilité et les dimensions applicables.
- **Git** : `main` = releases (jamais modifiée ni poussée sans demande de l'utilisateur) ; `dev` = intégration, poussée sur `origin` (GitHub) après chaque goal intégré et vert, jamais en force ; `goal/…` et `task/…` restent locales. Vérifier la CI de `dev` après chaque push.
- **Tests payants** : uniquement des tests live marqués `#[ignore]`, lancés avec `UNLEASH_LIVE_TESTS=1`, sur les seuls modèles `claude-haiku-5-5` et `gpt-6-luna`, dans le plafond `UNLEASH_LIVE_TEST_BUDGET_USD` ; clés `UNLEASH_DEV_*` lues dans `.env` par le testkit (`dotenvy`), jamais affichées ni committées ; jamais en CI ; aucun test avec un modèle intelligent (sol, astra, fable, opus, sonnet).

## 8. Commandes

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check                          # dépendances (installer : cargo install --locked cargo-deny)
python3 tools/design-package/validate.py .   # dossier : doit finir sur « 0 errors »
python3 tools/design-package/sync.py .       # après modification d'exigences, questions, ADR, risques
python3 tools/design-package/regen.py .      # manifest et CHECKSUMS.sha256, toujours en dernier avant validate
```

Ne jamais annoncer un passage sans avoir lu la sortie réelle de la commande.

## 9. Interdits sans demande explicite de l'utilisateur

push sur `main`, push forcé, tag, publication ou release ; appel payant à un modèle hors `claude-haiku-5-5`/`gpt-6-luna`, OAuth réel, achat, envoi de message ; affichage, copie ou commit d'un secret (`.env`) ; secrets autres que les clés de dev de `.env` ; modification d'un invariant ; réécriture d'historique (`reset --hard` sur `main`, `push --force`) ; installation système (`sudo`, gestionnaires de paquets globaux) ; édition manuelle du manifest ou des checksums ; écriture d'état runtime du produit dans `.claude/`.

## 10. Environnement de ce poste

- **Rust local** exclusivement dans Docker selon les [règles Rust](.claude/rules/rust.md#exécution-locale-dans-docker). Les contrôles natifs macOS passent par un runner dédié.
- Le dépôt est sur un **partage réseau SMB**. Conserver `git config core.fileMode false`.
- Respecter les consignes shell globales de l'utilisateur lorsqu'elles existent (préfixe de commande imposé).
