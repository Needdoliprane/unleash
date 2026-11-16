# Règle Git : worktrees uniquement dans `.claude/worktrees/`

Tout worktree Git du dépôt Unleash est créé sous `.claude/worktrees/<nom>/`, et **nulle part ailleurs** : ni dans un répertoire frère, ni dans `/tmp`, ni dans `~/.codex/worktrees`, ni dans un répertoire choisi par l'outil. C'est l'emplacement natif de Claude Code (`claude --worktree`, outil `EnterWorktree`, sous-agents `isolation: worktree`) ; les autres outils créent leurs worktrees à la main au même endroit.

- Nom : `<goal-id>-<task-id>` en minuscules ASCII (ex. `g0003-t02`). Branche : `task/<goal-id>/<task-id>`, ou `worktree-<nom>` quand l'outil la crée lui-même.
- Création manuelle : `git worktree add .claude/worktrees/<nom> -b task/<goal-id>/<task-id> <branche-de-base>`.
- Un worktree appartient à une seule tâche et à un seul agent écrivain. Aucun autre agent n'y écrit.
- Fin de tâche : après intégration, `git worktree remove .claude/worktrees/<nom>` puis suppression de la branche fusionnée. `git worktree prune` en fin de goal.
- `.claude/worktrees/` est ignoré par Git et exclu du manifest du dossier. Il ne contient jamais de documentation de référence.
- Un hook Claude Code (`.claude/hooks/git_guard.py`) refuse `git worktree add|move` vers un autre emplacement ; il autorise `git push origin dev` et demande confirmation pour tout autre push.

Le détail du cycle de vie est dans le [protocole de goal](../dev/goal-protocol.md) et la [politique des worktrees](../dev/worktrees.md). Cette règle concerne le développement d'Unleash ; la gestion des worktrees des projets supervisés par le produit reste définie par [F010](../features/F010-git-coordination/README.md).
