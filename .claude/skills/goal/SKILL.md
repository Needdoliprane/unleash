---
name: goal
description: Conduit en autonomie un objectif de développement d'Unleash (planification, délégation à des sous-agents avec le modèle adapté, worktrees sous .claude/worktrees, vérification indépendante, intégration, rapport). À utiliser quand l'utilisateur tape /goal suivi d'un objectif.
argument-hint: "<objectif> [--no-merge] [--dry-run]"
disable-model-invocation: true
---

# /goal — agent root autonome

Objectif reçu : $ARGUMENTS

Tu es l'**agent root** de ce goal. Applique intégralement le protocole canonique [`.claude/dev/goal-protocol.md`](../../dev/goal-protocol.md), avec [le routage des modèles](../../dev/model-routing.md), [la définition de terminé](../../dev/definition-of-done.md) et [la politique des worktrees](../../dev/worktrees.md). Lis-les maintenant si ce n'est pas déjà fait dans cette session.

## Mécanique Claude Code

- **Options** : `--no-merge` laisse la branche `goal/<goal-id>` sans fusion dans `dev` ; `--dry-run` s'arrête après le cadrage et le plan écrits dans `.claude/state/goals/<goal-id>/goal.md`.
- **Délégation** : utilise l'outil Agent avec les rôles de `.claude/agents/dev/` (`scout`, `implementer`, `systems-implementer`, `ui-implementer`, `test-engineer`, `verifier`, `integrator`, `doc-keeper`, `architect`) ou les experts de `.claude/agents/`. Fixe le paramètre `model` (`fable`, `opus`, `sonnet`, `haiku`) selon le niveau choisi quand il diffère du défaut du rôle, et consigne le choix dans le journal.
- **Isolation** : les rôles écrivains de code ont `isolation: worktree` ; Claude Code crée leur worktree sous `.claude/worktrees/`. Pour un worktree créé à la main, utilise `git worktree add .claude/worktrees/<goal-id>-<task-id> -b task/<goal-id>/<task-id> goal/<goal-id>`. Aucun autre emplacement : le hook `git_guard.py` le refuse.
- **Parallélisme** : lance en un seul message les délégations indépendantes (au plus 4 écrivains). Ne relance pas toi-même un travail délégué ; attends son compte rendu.
- **Suivi** : tiens à jour `goal.md` et ajoute chaque événement à `journal.md` dans `.claude/state/goals/<goal-id>/`. Si le contexte devient long, le journal suffit pour reprendre.
- **Shell** : respecte les consignes shell de l'utilisateur (préfixe RTK) ; Cargo est dans `$HOME/.cargo/bin`, à ajouter au `PATH` dans la forme composée `/bin/sh -c '…'`.

## Autonomie et arrêt

Travaille sans demander de confirmation à chaque étape. Ne t'arrête que sur les conditions d'arrêt du protocole : succès observé, action hors mandat (push hors `dev`, modèle payant hors liste, secret non fourni, changement d'invariant), gate externe bloquante sans autre travail possible, ou budget atteint. Termine par un rapport court : résultat, preuves (commandes et sorties), branches, ce qui reste et le prochain goal recommandé.
