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

## Acquérir un terminal de projet

Construire l’hôte de processus avec `cargo build --locked -p unleash-process-host`, dans le même répertoire de build que le daemon. Garder le daemon en cours d’exécution et reprendre les identifiants du projet et de sa mission active. Le champ `aggregate_id` du reçu d’affectation donne `MISSION_ID`.

Pour un projet neuf, enregistrer sa définition complète dans `project.json`. Remplacer le chemin de l’exemple par le chemin absolu d’un répertoire existant.

```json
{
  "name": "Découverte",
  "description": "",
  "weight": 1,
  "primary_root_path": "/chemin/vers/projet",
  "sources": [],
  "repositories": [],
  "settings_json": {}
}
```

Lire `result.revision` avec `project show` et utiliser cette valeur dans `PROJECT_REVISION`. La révision remplace la définition complète. Pour modifier un projet existant, reprendre aussi les sources et dépôts à conserver. Chaque dépôt indique explicitement son `default_branch`, y compris si sa valeur est `null`.

```sh
cargo run --locked -p unleash-cli -- --json project show "$PROJECT_ID"
cargo run --locked -p unleash-cli -- --json project revise "$PROJECT_ID" \
  --expected-revision "$PROJECT_REVISION" --file project.json
```

Enregistrer les profils et limites du terminal dans `terminal-runtime.json`.

```json
{
  "schema_version": 1,
  "profiles": [{"profile_id": "local", "version": 1, "variables": {"LANG": "C", "TERM": "xterm"}}],
  "max_hosts": 2,
  "max_processes": 2,
  "max_ptys": 2,
  "lease_ttl_ms": 60000
}
```

`process settings` donne la révision des réglages, initialement zéro. Sur une autorité déjà configurée, reprendre les profils à conserver dans le fichier. Pour l’acquisition, utiliser la révision courante du projet, par exemple `receipt.new_revision` renvoyé par `project revise`.

```sh
cargo run --locked -p unleash-cli -- --json process settings
cargo run --locked -p unleash-cli -- --json process configure \
  --expected-revision "$SETTINGS_REVISION" --file terminal-runtime.json
cargo run --locked -p unleash-cli -- --json process acquire new \
  --project-id "$PROJECT_ID" --scope-revision "$SCOPE_REVISION" --mission-id "$MISSION_ID" \
  --purpose-key demo --environment-profile-id local --program /bin/sleep -- 30
```

Utiliser le `receipt.command_id` et le `receipt.result_ref` renvoyés par l’acquisition pour lire le résultat observé tant que le bail est valide.

```sh
cargo run --locked -p unleash-cli -- --json receipt "$COMMAND_ID" \
  --project-id "$PROJECT_ID" --result-ref "$RESULT_REF"
```

Copier `result.terminal_id` dans `TERMINAL_ID` pour lire la sortie du processus.

```sh
cargo run --locked -p unleash-cli -- --json process log \
  --project-id "$PROJECT_ID" --terminal-id "$TERMINAL_ID" --max-bytes 4096
```

Pour lire la page suivante, passer l’`end_offset` du dernier segment dans `--after-offset`. La réponse distingue les segments durables et volatils. Si la rétention a supprimé l’offset demandé, le daemon renvoie `ResyncRequired` avec la raison `CURSOR_EXPIRED`. Avec `--json`, la sortie d’erreur expose `code`, `reason_code` et `retry_class` pour les erreurs renvoyées par le daemon.

Pour rejouer une révision ou une configuration, conserver le fichier, la révision, le `--command-id` et l’`--authority-generation` d’origine. La saisie interactive reste en développement.

## Envoyer des octets à un terminal

À partir du résultat observé de l’acquisition, utiliser `terminal_id`, `process_run_uuid`, `lease_id`, `fence` et `control_revision`. La séquence de la prochaine entrée est `input_seq + 1`. Le bail doit être encore valide.

```sh
cargo run --locked -p unleash-cli -- --json process input \
  --project-id "$PROJECT_ID" --terminal-id "$TERMINAL_ID" \
  --process-run-uuid "$PROCESS_RUN_UUID" --lease-id "$LEASE_ID" --fence "$FENCE" \
  --control-revision "$CONTROL_REVISION" --input-seq "$NEXT_INPUT_SEQ" \
  --file terminal-input.bin
```

Le fichier contient de 1 à 65 536 octets. Utiliser `--file -` pour une entrée standard redirigée. Le reçu confirme l’acceptation de la demande. Relire le résultat d’acquisition pour vérifier que `input_seq` a avancé.

Pour reprendre la même demande, utiliser les mêmes octets, le même contrôle et la même séquence avec le `--command-id` et l’`--authority-generation` de la demande initiale.

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
