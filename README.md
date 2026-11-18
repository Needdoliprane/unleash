# OVPT Unleash

Unleash est un orchestrateur local d’agents et de procédures. Un daemon Rust conserve l’état dans SQLite et expose une interface IPC commune à la ligne de commande, à l’interface terminal et à l’interface graphique.

Le dépôt contient un aperçu de développement. Il permet de démarrer le daemon, de créer un projet, d’affecter une mission et de relire l’état et les reçus. Le [plan V1](.claude/plans/v1-implementation-plan.md) décrit les jalons d’exécution des agents, d’automatisation et de distribution.

## Essayer le daemon et la CLI

Avec la toolchain définie dans [rust-toolchain.toml](rust-toolchain.toml), lancer le daemon depuis le dépôt.

```sh
cargo run --locked -p unleash-daemon -- --foreground --ready-json
```

Ouvrir un second terminal dans le même dépôt après le message `ready`.

```sh
cargo run --locked -p unleash-cli -- --json status
cargo run --locked -p unleash-cli -- --json project create --name Découverte
```

Copier le champ `aggregate_id` du reçu de création dans `PROJECT_ID`.

```sh
PROJECT_ID='identifiant-du-projet'
cargo run --locked -p unleash-cli -- --json mission assign --project-id "$PROJECT_ID"
cargo run --locked -p unleash-cli -- --json snapshot --project-id "$PROJECT_ID"
cargo run --locked -p unleash-cli -- tui --linear
```

La touche `q` ferme l’interface terminal. Pour arrêter le daemon, utiliser `cargo run --locked -p unleash-cli -- daemon stop`.

## Sauvegarder les données locales

Avec le daemon en cours d’exécution, demander une sauvegarde manuelle.

```sh
cargo run --locked -p unleash-cli -- --json backup request
```

Le résultat contient le reçu d’acceptation et la lignée d’autorité. Copier `receipt.aggregate_id` dans `BACKUP_ID` pour consulter l’avancement.

```sh
BACKUP_ID='identifiant-de-la-sauvegarde'
cargo run --locked -p unleash-cli -- --json backup show "$BACKUP_ID"
```

L’état `completed` donne accès au manifeste de la sauvegarde créée sous `backups/<BACKUP_ID>` dans le répertoire de données du daemon. Pour rejouer une demande, utiliser `backup request` avec les valeurs d’origine pour `--backup-id`, `--command-id` et `--authority-generation`.

## Vérifier et contribuer

Le [rapport M0](qualification/m0/exit.md) indique les composants testés, les environnements et les limites de l’aperçu. Les commandes de vérification du workspace figurent dans le [workflow CI](.github/workflows/ci.yml). Voir les [règles de contribution](CONTRIBUTING.md) et le [canal de signalement sécurité](SECURITY.md).

Le dépôt est sous [licence Apache-2.0](LICENSE). Les mentions de licence sont dans [NOTICE](NOTICE).
