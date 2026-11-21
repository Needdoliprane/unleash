# Cycle de vie des branches, worktrees et caches de développement

**Statut : règle adoptée en R7.** Résumé applicable : [règle worktrees](../rules/worktrees.md).

## Emplacement unique

Tous les worktrees du dépôt Unleash vivent sous `.claude/worktrees/<nom>/`. Aucun autre emplacement n'est admis, quel que soit l'outil :

| Outil | Comment créer un worktree conforme |
| --- | --- |
| Claude Code | `claude --worktree <nom>`, outil `EnterWorktree`, ou sous-agent `isolation: worktree` : l'emplacement natif est déjà `.claude/worktrees/<nom>` |
| Codex | `git worktree add .claude/worktrees/<nom> -b task/<goal-id>/<task-id> goal/<goal-id>` ; ne pas utiliser un mode worktree de l'application qui crée ailleurs (par exemple sous `~/.codex/worktrees`) |
| Antigravity | même commande `git worktree add` ; ouvrir ensuite le worktree comme dossier de travail de l'agent délégué |

Le hook `.claude/hooks/git_guard.py` refuse dans Claude Code toute commande `git worktree add` ou `git worktree move` dont la destination n'est pas sous `.claude/worktrees/`. Les autres outils appliquent la même règle par leurs instructions.

## Création et intégration

1. ROOT vérifie le registre Git, le volume attribué au projet et les seuils applicables avant toute création. Il réutilise un checkout disponible quand sa base et son périmètre conviennent. Le suivi du goal identifie propriétaire, branche, base, checkout, cache éventuel et condition de clôture.
2. Une branche de tâche part de `goal/<goal-id>`. Un seul agent écrit dans le worktree. Le vérificateur utilise les preuves rattachées à la version exacte ou laisse la validation ouverte.
3. L’intégrateur fusionne dans la branche du goal et consigne le SHA cible et les contrôles applicables. Pour la branche du goal, la cible est `dev` avec publication vérifiée lorsque le mandat la demande.

## Retrait après intégration

Le retrait d’une branche de chantier intégrée et de son worktree est obligatoire dès que les validations applicables sont acquises et les derniers consommateurs libérés. ROOT mène ce retrait à son terme après chaque lot, y compris pour les branches sans worktree. Les branches temporaires et les checkouts hérités suivent les mêmes vérifications.

1. Confirmer l’intégration sur la cible exacte et conserver le SHA, le résultat des tests et les limites dans le journal. Vérifier les commits uniques et les contenus indexés, modifiés, non suivis ou ignorés, notamment les worktrees imbriqués. Réconcilier propriétaires, processus, montages, leases et preuves avant de libérer les ressources.
2. Retirer le worktree exact avec la primitive du dépôt ou avec `git worktree remove <chemin>`. Vérifier l’absence à la fois sur le système de fichiers et dans `git worktree list --porcelain`.
3. Retirer ensuite la branche locale par `git branch -d <branche>`. Vérifier l’absence de la ref. Reporter la clôture au journal et régénérer `.claude/state/current-state.md`.

Un refus de retrait reste une action ouverte avec le chemin ou la ref, le propriétaire, la cause et la condition de reprise. Une entrée `prunable` ne prouve pas l’absence de travail utile. Préserver les éléments concernés jusqu’à réconciliation et utiliser les commandes de retrait ciblées sans forçage ni purge globale.

## Caches

- Réutiliser le cache compatible avec la plateforme, la version d’outil et les entrées de build. Les compilations et tests Rust s’exécutent dans Docker. Garder `CARGO_TARGET_DIR`, SQLite et les temporaires de test dans le conteneur selon les [règles Rust](../rules/rust.md#exécution-locale-dans-docker).
- Garder les caches d’outil propres au projet sous `.claude/cache/<outil>/` ou dans le stockage interne prévu par l’outil. Pour un cache privé, consigner son emplacement, son propriétaire, son motif et son dernier consommateur. Un cache partagé exige un accès compatible avec les écrivains en cours.
- Après le dernier usage, retirer les cibles et caches privés explicitement identifiés et reconstructibles. Pour un conteneur de test, vérifier son retrait et l’état de ses volumes éventuels. Conserver un cache partagé tant qu’un consommateur le requiert.
- Avant nettoyage, vérifier le périmètre physique, le caractère reconstructible et l’absence de consommateur. Mesurer les octets récupérés quand ils sont observables. Reporter au suivi l’état et la condition de reprise pour une ressource inconnue ou encore utilisée.

## Hors périmètre

Cette politique ne décrit pas les worktrees que le produit Unleash crée pour les projets qu'il supervise ; ceux-ci suivent [F010](../features/F010-git-coordination/README.md) et la [coordination Git](../architecture/git-coordination.md).
