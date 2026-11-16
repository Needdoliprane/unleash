# OVPT Unleash — instructions Claude Code

Les règles communes à tous les agents sont dans `AGENTS.md`, importé ici :

@AGENTS.md

## Spécifique à Claude Code

- **`/goal <objectif>`** lance l'agent root autonome ([skill](.claude/skills/goal/SKILL.md), [protocole](.claude/dev/goal-protocol.md)). Options : `--no-merge`, `--dry-run`.
- **Sous-agents** : rôles de développement dans `.claude/agents/dev/` (`scout` haiku, `implementer` sonnet, `systems-implementer` opus, `ui-implementer` sonnet, `test-engineer` sonnet, `verifier` sonnet (opus pour le code système), `integrator` sonnet, `doc-keeper` sonnet, `architect` fable) et quatorze experts de revue en lecture seule dans `.claude/agents/` (`expert-01-chief-architect` … `expert-14-product`, sonnet). Le root choisit le modèle à chaque appel selon [le routage](.claude/dev/model-routing.md).
- **Worktrees** : natifs sous `.claude/worktrees/` (`isolation: worktree`, `EnterWorktree`, `claude --worktree`), branchés depuis `HEAD` (`worktree.baseRef = "head"` dans `.claude/settings.json`) pour partir de la branche du goal. Le hook `.claude/hooks/git_guard.py` refuse tout `git worktree add|move` ailleurs autorise `git push origin dev` et demande confirmation pour tout autre push, `reset --hard` ou `clean -f`.
- **Règles chargées automatiquement** : `.claude/rules/rust.md`, `testing.md` et `design-dossier.md` s'activent sur les fichiers concernés ; `worktrees.md` est toujours chargée.
- **Shell** : appliquer les consignes shell globales de l'utilisateur (préfixe RTK). Cargo n'est pas dans le `PATH` de ce préfixe : utiliser la forme composée `rtk proxy /bin/sh -c 'export PATH="$HOME/.cargo/bin:$PATH"; cargo …'`.
- **Réglages personnels** (non versionnés) : `.claude/settings.local.json`, par exemple `{"env": {"CARGO_TARGET_DIR": "/Users/<vous>/.cache/unleash/target"}}` pour builder hors du partage réseau.
