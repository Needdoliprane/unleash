# M1 — Cœur durable et autorité locale

Les sources de production de `c03273c` sont acceptées sur `goal/g0002-m1`. Le commit `2357de8` précise la version de la dépendance de test CLI du daemon. Le contenu résolu de `Cargo.lock` est identique.

## Parcours validés

- Daemon local unique, socket Unix authentifié, preuve administrative éphémère, refus par portée et reprise des reçus dans leur lignée.
- Sauvegarde manuelle par les vrais binaires CLI et daemon. Le rejeu rend le même reçu, le manifeste consulté correspond au fichier et le journal contient un seul dispatch. Les coupures C2, C4 et C11 vérifient l’acceptation durable, les résultats inconnus et le maintien du verrou pendant le drainage.
- Restauration SQLite sur une fixture isolée avec capsule, reçu historique et refus de l’ancienne enveloppe. La qualification des exécuteurs sur la matrice OS reste prévue en M7.
- Diagnostic CLI/IPC quand le contrôle SQLite en lecture seule constate une corruption. Les essais vérifient la conservation des octets DB/WAL et le maintien du verrou d’autorité. Le rapport indique une couverture d’inventaire inconnue. Le handshake diagnostique laisse les identifiants durables indisponibles. Les tests de démarrage couvrent le nettoyage du socket et de la preuve après une erreur et la conservation des entrées étrangères.
- Lecture interne d’un manifeste historique supérieur à 2 Mio exercée par une vraie restauration. Le rapport de diagnostic conserve sa borne d’IPC.

## Vérification locale

macOS 27.2.1 arm64, Rust 1.94.1, SQLite embarquée 3.54.1. Bases, processus et sockets de test isolés sur le disque local.

| Contrôle exécuté | Résultat |
| --- | --- |
| `cargo fmt --all -- --check` | Vert |
| `cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings` | Vert |
| `cargo build --locked --offline -p unleash-cli -p unleash-daemon` | Vert |
| `cargo test --locked --offline --workspace --all-features --no-fail-fast` | 369 passés, 0 échec, 0 ignoré, 114 suites |
| `RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --workspace --no-deps` | Vert |
| `cargo deny check` | Vert après version `0.1.0`. 44 avertissements sur doublons et licences non rencontrées. |
| `uv run --offline --with jsonschema python tools/design-package/validate.py .` | 0 erreurs, 0 avertissements |

Les deux trajets binaires sur `fe47bc7` sont réutilisés sur le merge après comparaison des sources de production et du verrou Cargo. Le merge ajoute uniquement une correction de synchronisation dans les tests process-host par rapport à ces sources. Les reçus sont conservés dans [qualification/evidence](../evidence) et les commandes détaillées dans le [journal du jalon](../../.claude/state/goals/g0002-m1/journal.md).

## Revues et limites

- L’intégrateur a rejoué les gates et les scénarios SQLite, IPC et processus sur le candidat assemblé. ROOT a contrôlé la filiation du code testé et les reçus.
- La contre-revue de `fe47bc7` confirme les deux corrections après création d’artefact. Si la première lecture d’identité du socket échoue, le démarrage est refusé et l’entrée incertaine est conservée pour réconciliation.
- La lecture du diagnostic confirme le contrôle pair/credential, l’inspection réservée à l’administration globale, le refus des mutations et les vérifications des manifestes privés. AI-Guard réel et isolation forte appartiennent à M6.
- Le diagnostic qualifié est la voie CLI/IPC. Les parcours GUI, TUI et `--attach` en mode diagnostic restent à qualifier. Les reçus dont la provenance projet ne peut pas être attestée restent protégés dans les accès scoped.
