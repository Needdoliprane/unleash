# Kit du rédacteur — banc Docker Unleash

Ce kit s'adresse à un sous-agent qui écrit du code Rust ou renderer dans un worktree attribué.
Il complète le mandat : résultat attendu, périmètre, preuve, remise.

## Règles permanentes

- Toute commande shell porte le préfixe imposé par les consignes shell globales de l'utilisateur,
  quand elles existent (voir `AGENTS.md`, « Environnement de ce poste »).
- Aucune compilation, aucun test, aucun formatage Rust ni `node`/`pnpm` sur l'hôte : tout passe par le banc Docker.
- Le worktree attribué est le seul emplacement d'écriture du dépôt. Un seul écrivain par worktree.
- Ne jamais toucher au checkout canonique (hors lecture), aux autres conteneurs, aux autres worktrees,
  aux branches de jalon, à `main`, à `dev`, ni au dépôt distant.
- Ne jamais lire ou afficher `.env`. Aucun accès réseau : un rédacteur ne construit pas l'image.
  Aucun `git push`, `git reset --hard`, `git clean`, `git stash`.
- Ne jamais modifier un script de banc pendant que des bancs l'exécutent ou démarrent, et ne pas lancer
  de préchauffage : il n'y en a qu'un à la fois par machine.
- TDD : un test du dossier (`T-…`) à la fois, au point d'entrée nommé par le dossier ; observer le rouge, implémenter,
  observer le vert. Un test affaibli, ignoré ou supprimé pour passer est un échec de la tâche.
- Rust : `.claude/rules/rust.md` ; tests : `.claude/rules/testing.md`. Pas d'`unwrap`/`expect`/`panic` hors tests.

## Banc

- Conteneur attribué : donné par le mandat (`UNLEASH_BENCH`). Réseau aucun, CPU et mémoire bornés.
- Scripts : `tools/bench/` du worktree attribué. Usage complet, variables et codes de sortie : [README.md](README.md).
- Le banc lit les sources **depuis Git** : committer d'abord dans le worktree (un commit de travail suffit),
  puis transférer la révision.

```sh
B=tools/bench/bench.sh                 # depuis la racine du worktree
E=<dossier de preuves attribué>
C=<conteneur attribué>
L=<label court, par exemple g0005-list-runs>

# 1. Transfert initial de la révision (une fois)
sh "$B" source "$C" "$L" <sha>

# 2. Révision suivante (remplace l'arbre, garde node_modules et dist)
sh "$B" resync "$C" "$L" <sha>

# 3. Après chaque transfert : le rendu du renderer, que le crate de la GUI exige
sh "$B" renderer "$C" "$L" "$E/renderer-install.log" 1800 install --frozen-lockfile --offline --store-dir /opt/pnpm-store
sh "$B" renderer "$C" "$L" "$E/renderer-build.log" 1800 run build

# 4. Un seul contrôle à la fois dans le conteneur ; lire le journal entier ensuite
UNLEASH_BENCH_SOURCE_LABEL="$L" sh "$B" run "$C" "$L-fmt" "$E/fmt.log" 900 cargo +1.94.1 fmt --all -- --check
UNLEASH_BENCH_SOURCE_LABEL="$L" sh "$B" run "$C" "$L-clippy" "$E/clippy.log" 5400 cargo +1.94.1 clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
UNLEASH_BENCH_SOURCE_LABEL="$L" sh "$B" run "$C" "$L-doc" "$E/doc.log" 3600 env RUSTDOCFLAGS=-Dwarnings cargo +1.94.1 doc --workspace --no-deps --locked --offline
UNLEASH_BENCH_SOURCE_LABEL="$L" sh "$B" run "$C" "$L-test" "$E/test.log" 3600 cargo +1.94.1 test --locked --offline -p <crate> --test <fichier-de-test> <filtre> -- --exact

# 5. Avant les scénarios terminal réels : le binaire de l'hôte des processus
UNLEASH_BENCH_SOURCE_LABEL="$L" sh "$B" run "$C" "$L-process-host" "$E/process-host.log" 5400 cargo +1.94.1 build --workspace --all-features --locked --offline --bin unleash-process-host
```

- `RUSTDOCFLAGS=-Dwarnings` s'écrit sans espace : cette forme passe à travers le préfixe de commande imposé.
- Environnement fixé dans le banc : `CARGO_TARGET_DIR=/cargo-target` (volume du conteneur), `RUSTFLAGS=-D warnings`
  (comme la CI), `CARGO_HOME` avec registre en lecture seule. `--locked --offline` obligatoire.
- Le volume de compilation peut être partagé avec d'autres bancs. Chaque contrôle y prend un verrou global et
  marque l'arbre compilé : « Attente du verrou des bancs » ou « Blocking waiting for file lock on
  build directory » signifie qu'un autre banc travaille : attendre, ne pas relancer.
- Le disque de la machine virtuelle Docker est lent. Les délais des appels sont larges ; laisser faire.
- Le banc refuse un contrôle si le conteneur n'est pas en service, si son réseau n'est pas `none` ou s'il est
  occupé : c'est voulu. Attendre, ne pas forcer.
- Un code de sortie est `0` (succès), `1` (échec de contrôle : fmt, clippy, test) ou `101` (erreur cargo).
  `2` signale un usage incorrect, `3` un banc refusé ou une « issue inconnue » : délai dépassé, marqueur absent.
  Une issue inconnue n'est jamais un résultat : réconcilier les effets du banc avant de relancer.
- Chaque contrôle laisse un journal dans le dossier de preuves ; le rendre avec son code de sortie.
- Pour mettre en forme : lancer `cargo +1.94.1 fmt --all` dans le banc (`run`), puis recopier chaque fichier modifié
  du banc vers le worktree : `sh "$B" shell "$C" "cat /work/src-$L/<chemin>" > <chemin-du-worktree>"`.
  Vérifier ensuite `git diff --stat` dans le worktree : seuls les fichiers attendus doivent changer.
- Renderer : `pnpm --dir apps/gui-renderer run typecheck|lint|test` dans le banc, après l'étape 3.

## Remise

Rendre : branche et SHA, fichiers modifiés, chaque commande avec son code de sortie et le chemin du journal,
résultat par critère (satisfait, non satisfait, non vérifié), écarts et risques. « Terminé » sans sortie réelle
ne vaut pas remise.
