# Banc Docker de qualification

Le dépôt est sur un partage SMB et sa toolchain Rust est épinglée ; Tauri 2 exige GTK 3 et WebKitGTK. Le banc exécute les contrôles Rust et renderer du dépôt dans Docker, sur une révision Git, dans un conteneur sans réseau, avec un journal par contrôle. Il met en œuvre la règle [« Exécution locale dans Docker »](../../.claude/rules/rust.md#exécution-locale-dans-docker).

## Fichiers

| Fichier | Rôle |
| --- | --- |
| `Dockerfile` | Image : `rust:1.94.1-bookworm`, composants `rustfmt`, `clippy` et `rust-src`, dépendances système de la CI, Node 22, pnpm et son magasin préchargé pour les verrous du renderer donnés à `build.sh`. |
| `build.sh` | Assemble le contexte sous `.claude/cache/bench/image`, à partir de Git, puis construit l’image. |
| `bench.sh` | Crée et sonde un conteneur, transfère une révision Git, lance un contrôle borné avec son journal. |
| `gates.sh` | Enchaîne les contrôles de la [CI](../../.github/workflows/ci.yml) sur une source déjà transférée. |
| `WRITER-KIT.md` | Règles et commandes d’un agent qui écrit dans un worktree. |

Les scripts se lancent par `sh tools/bench/<script>` : le dépôt ne pose pas de bit exécutable (`core.fileMode false` sur SMB).

## Prérequis

- Docker Desktop lancé. Les scripts utilisent son contexte `desktop-linux` et son socket `$HOME/.docker/run/docker.sock`.
- `git`, `curl` et `timeout` de GNU coreutils, que macOS ne fournit pas.
- Un registre Cargo sur l’hôte, `$HOME/.cargo/registry`, qui contient déjà l’index et les crates de `Cargo.lock`. Le banc le monte en lecture seule et ne le remplit jamais.
- L’image, construite par `build.sh`.

## Usage

Depuis la racine du dépôt ou du worktree.

```sh
# 1. Image : seule étape qui accède au réseau. Les verrous du renderer sont lus dans Git.
UNLEASH_BENCH_LOCK_REVISIONS="<révision-1> <révision-2>" sh tools/bench/build.sh [étiquette]

# 2. Conteneur, puis prévol. Le volume de compilation est facultativement partagé.
export UNLEASH_BENCH_IMAGE=unleash-bench:<étiquette>
export UNLEASH_BENCH_TARGET_VOLUME=unleash-bench-<volume-partagé>   # optionnel
sh tools/bench/bench.sh create <conteneur> --cpus 1 --memory 4g
sh tools/bench/bench.sh preflight <conteneur>

# 3. Révision commitée (une fois), puis révision suivante
sh tools/bench/bench.sh source <conteneur> <label> <sha>
sh tools/bench/bench.sh resync <conteneur> <label> <sha>

# 4. Après chaque transfert : le rendu du renderer, que le crate de la GUI exige
sh tools/bench/bench.sh renderer <conteneur> <label> <journal>.log 1800 install --frozen-lockfile --offline --store-dir /opt/pnpm-store
sh tools/bench/bench.sh renderer <conteneur> <label> <journal>.log 1800 run build

# 5. Un contrôle à la fois ; le journal reçoit toute la sortie
sh tools/bench/bench.sh cargo <conteneur> <label> <journal>.log 900 fmt --all -- --check
sh tools/bench/bench.sh run <conteneur> <label> <journal>.log 3600 env RUSTDOCFLAGS=-Dwarnings cargo +1.94.1 doc --workspace --no-deps --locked --offline

# 6. Les contrôles d’un domaine : renderer, rust ou all
UNLEASH_BENCH_LOGS=<dossier-des-journaux> sh tools/bench/gates.sh <conteneur> <label> renderer

# 7. Fin d’usage, sans force : arrêter et retirer le conteneur, puis son volume cargo-home.
#    Le volume de compilation se retire seulement s’il n’est pas partagé.
docker stop <conteneur>
docker rm <conteneur>
docker volume rm unleash-bench-<conteneur>-cargo-home
```

Le nom complet des volumes s’obtient par `docker volume ls --filter name=<conteneur>`.

| Sous-commande de `bench.sh` | Effet |
| --- | --- |
| `preflight <conteneur>` | Vérifie l’API, l’état `running` et le réseau `none`, l’inactivité, les versions, une écriture dans `/work`, `/tmp`, `/cargo-target` et `/cargo-home`, le lien du registre, la présence de GTK 3 et de WebKitGTK. |
| `create <conteneur> [--cpus N] [--memory T]` | Conteneur détaché, sans réseau, avec init Docker ; 4 CPU et 20g par défaut, 4096 processus au plus ; registre de l’hôte en lecture seule, volume `unleash-bench-<conteneur>-cargo-home`, volume de compilation sur `/cargo-target`. |
| `source <conteneur> <label> <révision>` | Extrait les chemins suivis de la révision dans `/work/src-<label>`, par `git archive` puis `tar -m` ; refuse une source déjà présente. |
| `resync <conteneur> <label> <révision>` | Remplace l’arbre d’une source existante et garde `node_modules` et `dist`. |
| `run <conteneur> <label> <journal> <délai-s> <commande…>` | Lance une commande invitée dans `/work/src-<label>`, bornée par le délai. |
| `cargo <conteneur> <label> <journal> <délai-s> <arguments…>` | `run` pour `cargo +1.94.1 <arguments…>`. |
| `renderer <conteneur> <label> <journal> <délai-s> <arguments…>` | `run` pour `pnpm <arguments…>`, dans `apps/gui-renderer`. |
| `shell <conteneur> <commande…>` | Commande libre dans `/work`, bornée à 900 s : inspecter, recopier un fichier. |

Le banc lit une révision **commitée** : committer dans le worktree (un commit de travail suffit) avant de transférer. Les sources transférées sont `Cargo.toml`, `Cargo.lock`, les fichiers de configuration Rust, `crates`, `apps` et `.claude` hors `state`. Ni `node_modules`, ni `dist`, ni état, ni preuve.

Après chaque `source` ou `resync`, installer puis construire le renderer de l’étiquette : le crate de la GUI exige `apps/gui-renderer/dist`, sans quoi Cargo rend `101` (« frontendDist … doesn't exist »). Après un `resync`, l’installation est immédiate puisque `node_modules` est conservé.

Avant une campagne de tests qui touche aux scénarios terminal réels, construire le binaire de l’hôte des processus :

```sh
sh tools/bench/bench.sh run <conteneur> <label> <journal>.log 5400 cargo +1.94.1 build --workspace --all-features --locked --offline --bin unleash-process-host
```

## Variables

| Variable | Effet | Valeur par défaut |
| --- | --- | --- |
| `UNLEASH_BENCH_IMAGE` | Image construite par `build.sh` et lancée par `create`. | `unleash-bench:1.94.1-bookworm-20261124` |
| `UNLEASH_BENCH_LABEL` | Valeur de l’étiquette Docker `io.unleash.bench` de l’image et du conteneur. | `tools/bench` |
| `UNLEASH_BENCH_LOCK_REVISIONS` | Révisions Git, une ou deux, des verrous du renderer que `build.sh` sème dans le magasin pnpm. | `HEAD` |
| `UNLEASH_BENCH_TARGET_VOLUME` | Volume de compilation monté sur `/cargo-target`. Un même volume sert plusieurs bancs d’une même ligne : les dépendances déjà construites servent à tous, et Cargo attend son tour. | `unleash-bench-<conteneur>-target` |
| `UNLEASH_BENCH_LOCK_DIR` | Dossier du [verrou global des bancs](#verrou-global-des-bancs). | `.claude/cache/bench` du checkout racine |
| `UNLEASH_BENCH_SOURCE_LABEL` | Source de `run`, `cargo` et `renderer` quand l’étape porte un autre nom que la source. | le nom de l’étape |
| `UNLEASH_BENCH_LOGS` | Dossier des journaux de `gates.sh`, un fichier `<label>-<contrôle>.log` par contrôle. | `.claude/cache/bench/logs` du dépôt |
| `UNLEASH_BENCH_SHELL_LIMIT` | Délai de `shell`. | `900s` |
| `UNLEASH_BENCH_BUILD_LIMIT` | Délai de `build.sh`. | `3600s` |
| `UNLEASH_BENCH_IN_MEMORY` | Chemin du script déjà lu en mémoire par sa garde. Ne pas le fixer. | aucun |

L’étiquette par défaut figure dans `build.sh` et dans `bench.sh`. Toute modification du `Dockerfile`, des verrous du renderer ou de la toolchain appelle une nouvelle étiquette : une étiquette identifie une recette, et la reconstruire effacerait la preuve de ce qu’elle contenait.

## Règles d’emploi

- Aucune commande `cargo`, `rustc`, `rustfmt`, `rustdoc`, `node` ni `pnpm` ne s’exécute sur l’hôte : tout passe par le banc.
- Un seul contrôle à la fois par conteneur, et un seul à la fois sur la machine pour les bancs qui partagent un volume de compilation (voir [verrou global des bancs](#verrou-global-des-bancs)). Un contrôle, un journal, un délai. Lire le journal entier avant de conclure.
- Un seul préchauffage à la fois par machine : transfert de sources, `pnpm install` et compilation écrivent beaucoup, et le disque de la machine virtuelle Docker est lent, souvent sur un partage réseau. Les autres bancs continuent de servir des contrôles courts.
- Ne jamais modifier un script de banc pendant que des bancs l’exécutent ou démarrent (voir [lecture des scripts en mémoire](#lecture-des-scripts-en-mémoire)).
- Cargo s’exécute avec `--locked --offline`. `RUSTFLAGS=-D warnings` et `CI=1` sont fixés comme dans la CI ; `CARGO_TARGET_DIR=/cargo-target` et `CARGO_HOME=/cargo-home` vivent dans les volumes du conteneur, jamais sur le partage SMB.
- Le banc refuse un conteneur arrêté, au réseau inattendu ou occupé, et signale une « issue inconnue » quand le processus invité n’a pas fini normalement. Attendre, réconcilier les effets déjà engagés, ne pas forcer.
- Docker est partagé avec d’autres piles. Ne toucher qu’aux conteneurs et volumes créés pour son propre usage, et les retirer sans force en fin d’usage. Un volume de compilation partagé ne se retire pas avec le banc qui l’a monté.
- Avant une longue série, exécuter chaque chemin du banc sur un cas réel et lire le résultat : `sh -n` et ShellCheck ne qualifient pas un script. Voir [RETEX-007](../../.claude/dev/retours-experience.md#retex-007--qualifier-un-script-de-banc-par-sa-première-exécution-réelle).

## Lecture des scripts en mémoire

Le shell lit un script commande par commande, depuis son fichier. Un script modifié en place pendant qu’un banc l’exécute peut donc rendre une erreur de syntaxe, ou exécuter un contenu altéré. Chaque script de `tools/bench/` se relit une fois en mémoire à son lancement, puis exécute cette copie : une modification ne touche que les invocations futures, y compris celles qu’un script en lance à un autre (`gates.sh` vers `bench.sh`).

**Non couvert** : une réécriture du fichier qui chevauche l’instant du lancement. Le script lu alors peut être vide ou tronqué, et le shell rend `0` sans rien exécuter. La garde ne protège que les instances déjà lancées ; elle ne dispense pas d’attendre la fin des bancs pour modifier un script.

## Verrou global des bancs

Les bancs qui partagent un volume de compilation y déversent des binaires à chemin unique (`target/debug/unleashd`, `unleash-process-host`) : deux bancs aux arbres différents pouvaient se les écraser entre la compilation et l’exécution. Chaque contrôle de `run`, `cargo` et `renderer` prend donc un verrou global, du début de la compilation à la fin de l’exécution.

Le verrou est un dossier `hold`, créé par `mkdir` sous `$UNLEASH_BENCH_LOCK_DIR`, qui contient l’identifiant de son propriétaire. Un contrôle qui attend affiche « Attente du verrou des bancs ». Il reprend le verrou d’un propriétaire disparu et abandonne au bout de quatre heures (code `3`). Le verrou est libéré à la fin du contrôle, y compris sur interruption.

Par défaut, le dossier du verrou est `.claude/cache/bench` du checkout racine du dépôt : il est commun à tous ses worktrees, donc à tous les bancs qui en sont issus. Deux clônes du dépôt ont deux verrous distincts ; leur donner le même `UNLEASH_BENCH_LOCK_DIR`.

## Codes de sortie de `bench.sh`

| Code | Sens |
| --- | --- |
| `0` | Le contrôle a réussi. |
| `1` | Le contrôle a échoué (`fmt --check`, Clippy, un test, un lint). `docker` rend aussi `1` pour un conteneur absent : lire le message. |
| `101` | Erreur cargo. |
| `2` | Usage. |
| `3` | Banc refusé : API muette, conteneur arrêté ou réseau inattendu, banc occupé, source déjà présente ou absente, verrou des bancs occupé depuis plus de quatre heures. Issue inconnue : délai dépassé, marqueur absent, code invité autre que `0`, `1` et `101`. |
| `124` ou `137` | Un appel Docker au banc a dépassé son délai. Le script s’arrête sans message et ses effets sont à réconcilier. |
| autre | Une commande hôte a échoué, par exemple `128` pour une révision inconnue. |

Un contrôle terminé se termine par le marqueur `__UNLEASH_BENCH_TERMINAL__ <étape> <code>` dans son journal, écrit seulement quand le processus invité s’est arrêté lui-même avec un code connu. Un échec de contrôle (`1`, `101`) est un résultat ; une issue inconnue n’en est pas un.

Après une interruption, vérifier l’inactivité du banc (`docker top <conteneur> -eo pid,comm`), puis l’état de `/work/src-<label>` : un répertoire vide laissé par un transfert interrompu se retire par `shell <conteneur> "rmdir /work/src-<label>"`, un arbre complet se remplace par `resync`.

`gates.sh` sort à `0` si tous les contrôles demandés sont verts, à `1` sinon : contrôle en échec ou non exécuté. Les contrôles renderer qui suivent une installation en échec ne sont pas exécutés, car un `resync` conserve `node_modules` et un test vert aurait pu tourner sous l’ancienne version. Le résultat de chaque contrôle est dans le résumé final et dans les journaux.

## Ce que prouve le banc

Sur la révision citée par le transfert (`source` imprime le commit), dans un conteneur Linux sans réseau, sur la plateforme de l’hôte Docker (arm64 sur Apple Silicon), une commande terminée par son marqueur s’est exécutée avec le code annoncé. Le journal conserve toute sa sortie. `gates.sh` lance les mêmes commandes que la CI, aux options près qu’impose l’absence de réseau (`--locked --offline`, magasin pnpm préchargé).

## Ce que le banc ne prouve pas

- Les validations propres à Darwin : elles exigent le [runner macOS dédié](../../.claude/rules/rust.md#exécution-locale-dans-docker).
- Le passage de la CI distante, qui reste à observer sur `ubuntu-latest` et `macos-latest`. Le banc ne remplace ni ne devance ce résultat.
- `cargo deny check`, que `gates.sh` ne lance pas.
- L’absence de dérive ts-rs : le banc extrait une archive sans répertoire `.git` et ne peut pas reproduire le `git diff --exit-code` de la CI.
- Les tests live, qui exigent un réseau que le conteneur n’a pas.

## Limites

- Qualifié sur Docker Desktop pour Mac seulement (conteneur Linux arm64 sur Apple Silicon). Autre moteur, autre contexte ou autre socket : adapter `bench.sh`, `build.sh` et refaire la qualification.
- Le disque de la machine virtuelle est lent : les appels Docker du banc sont bornés entre 90 s et 30 min, et un contrôle dispose de son délai plus 600 s. Une expiration reste une issue inconnue, à réconcilier avant de reprendre.
- `--cpus` est un quota, pas un jeu de processeurs : `nproc` voit ceux de la machine virtuelle. Un banc à un processeur n’a pas la capacité d’un poste ; un test sensible au temps peut y expirer.
- Un contrôle dont la commande sort avec un autre code que `0`, `1` et `101` est rapporté comme issue inconnue (`3`), pas comme un échec de contrôle : c’est le cas de `tsc` sur une erreur de type (code `2`). Le journal dit lequel.
- Un volume de compilation ne se copie pas par liens physiques : les volumes de Docker Desktop sont sur des périphériques distincts (« Invalid cross-device link »). On le partage, ou on recompile.
- `create` lance l’image sans vérifier qu’elle existe : une étiquette absente provoque une tentative de tirage, donc un accès réseau.
- Le registre Cargo de l’hôte n’est pas vérifié contre `Cargo.lock` : une crate absente se traduit par une erreur cargo hors ligne.
- La version de la toolchain figure dans `rust-toolchain.toml`, dans le `Dockerfile` et dans la variable `toolchain` de `bench.sh` et de `gates.sh`. Une montée de version les change ensemble et reconstruit l’image.
