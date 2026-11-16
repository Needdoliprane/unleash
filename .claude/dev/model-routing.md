# Routage des modèles pour les agents de développement

**Statut : politique adoptée en R7, observée le 16 novembre 2026.** L'agent root choisit le modèle de chaque délégation selon la difficulté et le risque de la tâche, pas selon l'habitude. Les noms de modèles évoluent : vérifier la liste disponible (`/model` dans chaque outil) et mettre ce tableau à jour plutôt que d'improviser.

Cette politique concerne les agents qui développent Unleash. Le routage des modèles **du produit** est défini par [model-routing](../specs/model-routing.md) et F012.

## Niveaux

| Niveau | Quand l'utiliser | Exemples de tâches |
| --- | --- | --- |
| `frontier` | Raisonnement le plus difficile, décision coûteuse à défaire, après deux échecs au niveau `strong` | Arbitrer une contradiction de contrat, amender un ADR, plan d'un goal transverse, diagnostic d'un bug de concurrence ou de reprise résistant, revue finale d'une gate de jalon |
| `strong` | Code où une erreur est grave ou subtile ; vérification indépendante | Store SQLite, journal/outbox, restauration, IPC v2 et lignées, scheduler 5/50, process-host, action broker, AI-Guard, sécurité, vérification `verifier`, revues des 14 experts |
| `standard` | Implémentation courante bien spécifiée | DTO et adaptateurs, CLI, écrans GUI/TUI, connecteurs, tests de contrat, intégration, propagation documentaire |
| `fast` | Travail mécanique ou de lecture, peu de jugement | Cartographie, inventaires, recherche, renommages, formatage, régénération via scripts, résumé de logs |

Règles : commencer au niveau le plus bas qui tient la qualité — la plupart des tâches relèvent de `standard` (Sonnet) ou de `fast` (Haiku) ; `strong` (Opus) est réservé aux cas listés, `frontier` aux décisions coûteuses à défaire ; escalader d'un niveau après deux échecs de vérification ; ne jamais utiliser `fast` pour une décision de contrat, de sécurité ou de reprise ; le `verifier` est au niveau de l'implémenteur qu'il contrôle (Sonnet par défaut, Opus pour le code système et les gates de jalon) ; les quatorze experts de revue tournent en Sonnet, en Opus pour une gate de jalon.

## Correspondance par outil

| Niveau | Claude Code (`model` d'agent ou d'appel) | Codex (`model` dans `.codex/agents/*.toml` ou à la délégation) | Antigravity (sélecteur de modèle) |
| --- | --- | --- | --- |
| `frontier` | `fable` (Claude Fable 5.2, ou 5.1 selon la version de Claude Code) | `gpt-6-astra` si le compte y a accès, sinon `gpt-6.1-sol` à l'effort le plus élevé accepté | Modèle de pointe disponible (classe Pro ou Claude Fable/Opus) |
| `strong` | `opus` (Claude Opus 5.5), effort `high` | `gpt-6.1-sol`, effort `high` | Modèle Pro / Claude Opus |
| `standard` | `sonnet` (Claude Sonnet 5.5), effort `medium`–`high` | `gpt-6.1-sol`, effort `medium` | Modèle Pro standard / Claude Sonnet |
| `fast` | `haiku` (Claude Haiku 5.5), effort `low` | `gpt-6-luna`, effort `low` | Modèle Flash / rapide |

Repères de coût Claude (API, par million de jetons entrée/sortie, observés le 2026-11-16) : Fable 5.2 8 $/40 $ (5.1 : 10 $/50 $), Opus 5.5 4 $/20 $, Sonnet 5.5 2 $/10 $, Haiku 5.5 0,10 $/0,50 $. Le niveau `frontier` coûte plusieurs fois `strong` : le réserver aux cas listés.

## Mécanique

- **Claude Code** : chaque rôle de `.claude/agents/dev/` porte un `model` par défaut ; le root peut le remplacer à l'appel (paramètre `model` de l'outil Agent : `fable`, `opus`, `sonnet`, `haiku`). La variable `CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1` désactive ce choix : ne pas la définir dans ce dépôt.
- **Codex** : chaque agent de `.codex/agents/` fixe `model` et `model_reasoning_effort` ; le root peut nommer l'agent ou demander un autre modèle à la délégation. Les sous-agents héritent du sandbox et de la politique d'approbation de la session principale.
- **Antigravity** : choisir le modèle de chaque agent dans le gestionnaire d'agents selon ce tableau ; en mode `/goal`, l'agent principal applique le protocole et consigne le niveau choisi dans le journal.

Chaque délégation consignée dans le journal du goal indique rôle, niveau et modèle effectivement utilisés.
