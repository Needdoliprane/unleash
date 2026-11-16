# Politique des worktrees de développement

**Statut : règle adoptée en R7.** Résumé applicable : [règle worktrees](../rules/worktrees.md).

## Emplacement unique

Tous les worktrees du dépôt Unleash vivent sous `.claude/worktrees/<nom>/`. Aucun autre emplacement n'est admis, quel que soit l'outil :

| Outil | Comment créer un worktree conforme |
| --- | --- |
| Claude Code | `claude --worktree <nom>`, outil `EnterWorktree`, ou sous-agent `isolation: worktree` : l'emplacement natif est déjà `.claude/worktrees/<nom>` |
| Codex | `git worktree add .claude/worktrees/<nom> -b task/<goal-id>/<task-id> goal/<goal-id>` ; ne pas utiliser un mode worktree de l'application qui crée ailleurs (par exemple sous `~/.codex/worktrees`) |
| Antigravity | même commande `git worktree add` ; ouvrir ensuite le worktree comme dossier de travail de l'agent délégué |

Le hook `.claude/hooks/git_guard.py` refuse dans Claude Code toute commande `git worktree add` ou `git worktree move` dont la destination n'est pas sous `.claude/worktrees/`. Les autres outils appliquent la même règle par leurs instructions.

## Cycle de vie

1. **Création** depuis la branche du goal `goal/<goal-id>` (jamais depuis un worktree de tâche non intégré).
2. **Travail** : un seul agent écrivain par worktree, dans le périmètre de fichiers de sa tâche. Commits locaux fréquents, messages au format Conventional Commits (`feat(store): …`, `fix(protocol): …`, `test(harness): …`, `docs(claude): …`).
3. **Vérification** par le `verifier`, qui peut lire et exécuter les tests dans le worktree sans y écrire.
4. **Intégration** : `git merge --no-ff` dans `goal/<goal-id>` par l'`integrator`.
5. **Nettoyage** : `git worktree remove .claude/worktrees/<nom>`, `git branch -d task/<goal-id>/<task-id>`, puis `git worktree prune` en fin de goal. Un worktree abandonné est consigné dans le journal avant suppression.

## Contraintes de ce poste

Le dépôt est sur un partage SMB. Les worktrees y fonctionnent, mais :

- le répertoire `target/` doit être sur disque local (`CARGO_TARGET_DIR`, voir `AGENTS.md`) ; chaque worktree peut partager ce répertoire en ajoutant un suffixe par worktree si des builds parallèles se gênent (`$CARGO_TARGET_DIR/<nom>`) ;
- les bases SQLite de test sont créées dans un répertoire temporaire local, jamais dans le worktree ;
- en cas de verrou Git persistant (`index.lock`) après un crash, vérifier qu'aucun processus Git n'est actif avant de le supprimer, et le consigner.

## Hors périmètre

Cette politique ne décrit pas les worktrees que le produit Unleash crée pour les projets qu'il supervise ; ceux-ci suivent [F010](../features/F010-git-coordination/README.md) et la [coordination Git](../architecture/git-coordination.md).
