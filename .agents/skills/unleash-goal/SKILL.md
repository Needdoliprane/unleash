---
name: unleash-goal
description: Conduit en autonomie un objectif de développement du dépôt Unleash selon le protocole .claude/dev/goal-protocol.md (planification, délégation à des sous-agents avec le modèle adapté, worktrees uniquement sous .claude/worktrees, vérification indépendante, intégration, rapport). À utiliser quand l'utilisateur donne un goal ($unleash-goal, /unleash-goal, /goal ou un message commençant par GOAL:).
---

# Goal autonome Unleash (Codex, Antigravity)

Tu es l'**agent root** du goal décrit par l'utilisateur. Applique intégralement [le protocole de goal](../../../.claude/dev/goal-protocol.md), avec [le routage des modèles](../../../.claude/dev/model-routing.md), [la définition de terminé](../../../.claude/dev/definition-of-done.md) et [la politique des worktrees](../../../.claude/dev/worktrees.md). Lis-les avant de commencer.

## Codex

- Délègue aux agents du projet définis dans `.codex/agents/` (`scout`, `implementer`, `systems_implementer`, `ui_implementer`, `test_engineer`, `verifier`, `integrator`, `doc_keeper`, `architect`) en les nommant dans ta demande ; chacun porte son modèle et son effort. Demande un autre modèle à la délégation quand le niveau choisi diffère.
- Les sous-agents héritent du sandbox et des approbations de ta session : lance Codex avec un sandbox d'écriture sur le dépôt.
- Worktrees : crée-les toi-même avant délégation, `git worktree add .claude/worktrees/<goal-id>-<task-id> -b task/<goal-id>/<task-id> goal/<goal-id>`, et donne ce chemin comme répertoire de travail du sous-agent. N'utilise aucun mode worktree qui crée ailleurs.

## Antigravity

- Le mode `/goal` natif et cette skill appliquent le même protocole. Choisis le modèle de chaque agent dans le gestionnaire d'agents selon le niveau du routage, et consigne-le dans le journal.
- Worktrees : même commande `git worktree add` sous `.claude/worktrees/` ; ouvre le worktree comme espace de travail de l'agent délégué.

## Dans tous les cas

Tiens `.claude/state/goals/<goal-id>/goal.md` et `journal.md` à jour. Travaille sans confirmation à chaque étape ; arrête-toi seulement sur les conditions d'arrêt du protocole. Push autorisé seulement sur `dev` (`git push origin dev`, jamais forcé) ; jamais sur `main`. Tests payants uniquement avec `claude-haiku-5-5` ou `gpt-6-luna` et les clés de `.env`, jamais affichées. Pas de changement d'invariant sans demande explicite de l'utilisateur.
