---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
  - "rustfmt.toml"
  - "clippy.toml"
  - "deny.toml"
---

# Règles de code Rust d'Unleash

Ces règles s'appliquent à tout code Rust du dépôt. Elles traduisent la [politique d'implémentation](../brief/implementation-policy.md), le [découpage du workspace](../architecture/rust-workspace.md) et les [invariants](../brief/invariants.md) en règles de code. En cas de conflit, les invariants et les contrats canoniques priment ; corriger alors la règle, ne pas contourner le contrat. Cible : une V1 distribuable et maintenable, pas un prototype.

## Toolchain, workspace et dépendances

- Toolchain figée par `rust-toolchain.toml` ; édition 2024 ; MSRV = version figée, déclarée dans `[workspace.package] rust-version`. Une montée de toolchain est un changement isolé, avec CI verte.
- Un seul workspace Cargo à la racine, un seul `Cargo.lock` versionné. Les crates vivent dans `crates/<nom>` et portent le préfixe `unleash-` du [découpage canonique](../architecture/rust-workspace.md). Une crate supplémentaire exige une responsabilité exclusive écrite dans sa doc de crate.
- Versions, métadonnées, profils et lints sont hérités du workspace (`version.workspace = true`, `[lints] workspace = true`). Les dépendances communes sont déclarées dans `[workspace.dependencies]`.
- Toute nouvelle dépendance est justifiée dans le commit : besoin, alternative interne, licence, maintenance et taille. `cargo deny check` doit passer : licences compatibles Apache-2.0, aucune vulnérabilité connue non dérogée, sources crates.io uniquement sauf dérogation écrite.
- Features Cargo additives ; aucune feature ne change une sémantique métier ou de sécurité. Pas de `default-features` qui active un effet réseau ou un accès disque implicite.

## Architecture et frontières

- `unleash-domain` ne fait aucune I/O : pas de Tokio, de SQLite, de HTTP, de système de fichiers, d'horloge murale ni d'aléa implicite. Les horloges, l'aléa et les identifiants arrivent par des ports (traits) injectés.
- Les états, transitions, autorisations et budgets vivent dans le backend Rust (domain/harness/scheduler). GUI, TUI et CLI appellent les opérations du protocole partagé ; elles n'implémentent aucune règle métier et n'accèdent jamais à SQLite.
- Un seul writer SQLite par autorité, sur un thread dédié derrière une file bornée (`ResourceBusy/BACKPRESSURE` en surcharge). L'état et l'événement qui le décrit sont écrits dans la même transaction. Aucune I/O réseau, aucun calcul lourd et aucun outil projet dans une transaction.
- Les DTO du protocole (`unleash-protocol`) sont versionnés, `#[serde(deny_unknown_fields)]`, testés par aller-retour contre les fixtures de `.claude/examples/` et les schémas de `.claude/schemas/`. Un changement incompatible incrémente la version du protocole et met à jour schéma, fixtures et consommateurs dans le même changement.
- Le domaine ne dépend jamais de l'adaptateur qui l'exécute. Les dépendances entre crates suivent le graphe canonique ; une dépendance inverse est un défaut de conception, pas un raccourci.

## Types et API

- Identifiants en newtypes (`struct RunId(Uuid)`), jamais de `String`/`u64` nus entre crates. États en `enum` exhaustifs ; pas de booléens multiples qui encodent un état.
- Rendre les états invalides non représentables : constructeurs validants, champs privés, types distincts pour « proposé », « observé », « vérifié ».
- `#[non_exhaustive]` sur les enums et structs publics susceptibles d'évoluer dans `unleash-protocol` ; `#[must_use]` sur les résultats et gardes qui ne doivent pas être ignorés.
- Suivre les [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) : nommage, conversions (`From`/`TryFrom`), `Debug` sur tous les types publics, `Display` pour les erreurs.
- Temps : UTC en stockage et sur le fil ; durées mesurées avec une horloge monotone ; jamais de logique de reprise fondée sur l'horloge murale seule.

## Erreurs et panics

- Bibliothèques : erreurs typées avec `thiserror`, une enum d'erreurs par frontière, variantes alignées sur le catalogue d'erreurs IPC. `anyhow` est réservé aux binaires, outils et tests.
- Interdits hors tests : `unwrap()`, `expect()` sans invariant documenté, `panic!`, `todo!`, `unimplemented!`, `dbg!`, indexation non vérifiée sur des données externes. Un `expect("...")` justifié énonce l'invariant qui le rend impossible.
- Une issue inconnue reste `OutcomeUnknown` : ne jamais la convertir en succès, en échec ou en retry automatique. Un effet externe sans preuve est réconcilié avant toute répétition.
- Pas de `std::process::exit` dans une bibliothèque. Les binaires terminent proprement : flush, fermeture du journal, libération des verrous.

## Async, concurrence et processus

- Tokio pour l'I/O asynchrone. Aucun appel bloquant dans une tâche async (`spawn_blocking` ou thread dédié). Aucun verrou synchrone tenu à travers un `.await`.
- Canaux et files bornés. Chaque attente externe (réseau, processus, verrou, IPC) a un délai et un chemin d'annulation. Les tâches lancées sont rattachées à un propriétaire (`JoinSet`, `CancellationToken`) ; pas de tâche orpheline.
- Documenter la sûreté à l'annulation de chaque future publique qui fait un effet. Un effet externe passe par l'action broker : intention durable, autorisation, dispatch, observation, issue inconnue, réconciliation.
- Processus : programme et arguments séparés (`Command::new(prog).args(..)`), jamais de shell construit par interpolation. Le shell interactif est une action explicite et observable. L'identité d'un processus ne se déduit jamais du PID seul.

## `unsafe`, FFI et sécurité

- `unsafe_code = "deny"` au niveau du workspace (`forbid` empêcherait toute dérogation locale). Seules les crates d'adaptateurs OS recensées (`unleash-platform`, `unleash-process-host`) peuvent le lever localement par `#![allow(unsafe_code)]`, avec un commentaire `// SAFETY:` par bloc, des tests OS et une revue par l'expert sécurité.
- Secrets : type dédié (`secrecy`, `zeroize`), jamais dans les logs, les erreurs, les arguments de commande, le contexte d'un modèle ou les fixtures. Les secrets fournisseur restent dans le daemon ou le trousseau OS.
- Toute entrée externe (fichier, réseau, sortie de modèle, DOM, document) est non fiable : validation stricte, limites de taille, aucune exécution implicite.

## Observabilité

- `tracing` pour tous les logs structurés, avec spans corrélés aux identifiants Run/Invocation/Action. Pas de `println!`/`eprintln!` dans les bibliothèques. Rédaction des champs sensibles avant émission.
- Les métriques et événements suivent le [modèle d'événements](../architecture/event-model.md) ; un log n'est jamais une preuve de contrat.

## Tests

Voir aussi [les règles de test](testing.md).

- Toute logique a des tests unitaires dans le module (`#[cfg(test)]`) et des tests d'intégration par crate (`tests/`). Les tests des contrats utilisent les fixtures du dossier et les cas négatifs des features.
- Propriétés (`proptest`) pour les machines à états, reducers, parseurs et sérialisation ; injection de pannes déterministe (`unleash-testkit`) pour la reprise.
- Un test ne doit pas vérifier un mock qui recopie l'implémentation. Les tests sont déterministes : pas de sommeil arbitraire, horloge et aléa injectés, ports réseau éphémères, répertoires temporaires isolés.
- SQLite de test sur disque local temporaire (jamais sur un partage réseau) ; aucun effet réel par défaut (pas d'appel modèle facturé, d'OAuth réel, d'achat, de push Git).

## Style et documentation

- `cargo fmt` (configuration du dépôt) et `cargo clippy --workspace --all-targets --all-features -- -D warnings` sans dérogation non justifiée. Un `#[allow(...)]` est local, porte une raison (`#[expect(lint, reason = "...")]`) et ne masque pas une règle de ce fichier.
- Doc de crate (`//!`) qui énonce responsabilité exclusive et dépendances admises ; doc (`///`) de chaque élément public avec erreurs et sûreté d'annulation. `cargo doc --no-deps` sans avertissement.
- Fonctions courtes, modules par responsabilité, pas de code mort ni de code commenté. Les commentaires expliquent le pourquoi et citent l'ID de l'exigence (`REQ-…`, `INV-…`) quand le code réalise un contrat.

## Exécution locale dans Docker

Exécuter les compilations, tests, commandes `cargo`, `rustc`, `rustfmt`, Clippy, rustdoc et binaires Rust de vérification exclusivement dans Docker sur ce poste. Cette règle s’applique aux agents et sous-agents. Utiliser la version de `rust-toolchain.toml` dans l’image et un `CARGO_TARGET_DIR` interne au conteneur.

Le dépôt est sur SMB. Placer aussi les bases SQLite, sockets, PTY et dépôts Git de test dans l’espace temporaire du conteneur. Pour les validations propres à Darwin, utiliser un runner macOS dédié. Si l’environnement nécessaire est indisponible, consigner le contrôle comme non exécuté et poursuivre les contrôles indépendants.

## Commandes de vérification obligatoires

Avant de déclarer une tâche Rust terminée, exécuter et lire la sortie de :

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check   # quand des dépendances changent
```

Une commande qui échoue bloque la clôture. Rapporter la sortie réelle ; ne jamais annoncer un passage non observé.
