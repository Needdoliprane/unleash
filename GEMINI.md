# OVPT Unleash — compléments Antigravity

Antigravity charge aussi `AGENTS.md`, qui contient toutes les règles communes ; ce fichier n'ajoute que ce qui est propre à Antigravity. Si votre version ne charge pas `AGENTS.md`, lisez-le entièrement avant toute action.

- **Goal autonome** : la commande native `/goal <objectif>` ou la skill `/unleash-goal <objectif>` (`.agents/skills/unleash-goal/`) appliquent le [protocole de goal](.claude/dev/goal-protocol.md) : cadrage, plan, délégation, vérification indépendante, intégration, journal sous `.claude/state/goals/`.
- **Modèles** : choisir le modèle de chaque agent du gestionnaire d'agents selon les niveaux `frontier` / `strong` / `standard` / `fast` du [routage](.claude/dev/model-routing.md), et consigner le choix dans le journal du goal.
- **Worktrees** : uniquement sous `.claude/worktrees/<goal-id>-<task-id>`, créés avec `git worktree add` ; ouvrir le worktree comme espace de travail de l'agent délégué. Aucun autre emplacement.
- **Règles** : `.agents/rules/rust.md` renvoie aux règles canoniques de `.claude/rules/` ; les appliquer avant d'écrire du Rust ou de modifier le dossier `.claude/`.
- **Navigateur** : le sous-agent navigateur d'Antigravity ne sert qu'à inspecter l'interface en développement ; il ne remplace pas les tests de F019 ni la qualification d'un moteur navigateur du produit.
