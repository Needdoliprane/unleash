# OVPT Unleash — Synthèse de conception R7

**Révision R7, 16 novembre 2026.** Ce dossier décrit l’architecture retenue et la réalisation de toute la V1 d’OVPT Unleash. Il fournit des spécifications, des contrats, des audits, des critères d’acceptation, un plan de construction et l’outillage des agents qui la réalisent. Tous les arbitrages sont rendus par délégation de Louis : les 21 questions sont résolues et les 19 ADR acceptés. La réalisation est ouverte dès le jalon M0. Les tests du futur produit restent `not_run`. Le [rapport documentaire](.claude/testing/document-validation.md) précise les contrôles effectivement exécutés sur les fichiers ; il ne qualifie pas un runtime déjà développé.

## 1. Le résultat et ses limites

La conception conserve un cœur local Rust, un daemon utilisateur, SQLite, des sessions durables et des workers éphémères. Le harness applique les états, les autorisations, les contrats, les budgets et les transitions. Les modèles prennent les décisions qui demandent du jugement. Ce partage évite de confier au prompt la mécanique de reprise ou le suivi de dépendances.

La V1 reste complète : 19 features, chacune accompagnée de sept documents canoniques ; 239 exigences atomiques ; 65 invariants suivis ; 19 ADR acceptés. Les interfaces partagent le backend et 58 groupes de parité GUI/TUI. Ces nombres mesurent la structure du dossier. La réussite du produit dépendra des preuves demandées dans les tests et de la qualification des dépendances, non d’arbitrages en attente. [Architecture globale](.claude/architecture/system-overview.md), [invariants](.claude/brief/invariants.md), [traçabilité](.claude/testing/traceability.md).

## 2. Une contradiction réellement organisée

### Audit R7

L’audit R7 comprend seize revues par domaine, chacune contre-vérifiée. Il a produit 238 constats : 218 confirmés et corrigés dans les documents canoniques, 20 réfutés et laissés en l’état. Les corrections restent des corrections de conception ; les tests du produit sont `not_run`. La [révision R7](.claude/brief/revision-2026-11-16-r7.md) en résume les familles.

### Expertises R6 (historique)

En R6, quatorze instances d’agents spécialisés ont reçu des mandats distincts correspondant aux expertises du brief : architecture générale, systèmes Rust, workflows, multi-agents, API et contexte, Git, connaissance, skills et contrats, persistance, sécurité, UX, QA, contradiction indépendante et architecture produit. Il s’agit d’agents IA, sans prétention de diversité de modèles ou d’équivalence à quatorze professionnels humains.

Cette passe R6 a produit trente constats corrigés en conception. Les rapports conservent les objections initiales, les alternatives, les corrections et les limites de vérification. Une contrelecture de remise a repéré un raccord supplémentaire : un résumé du moteur cherchait un reçu de commande avant de vérifier sa lignée de restauration. Le passage a été corrigé, puis relu. Les [rapports R6](.claude/audits/r6/README.md) et la [contrelecture](.claude/audits/r6/release-readthrough.md) rendent ces conclusions attribuables.

Le verdict R6 était favorable à l’engagement de la réalisation, avec les gates explicites du dossier ; R7 l’a ouverte. Il ne signifie ni absence de défaut futur, ni certification de sécurité, ni réussite d’une intégration réelle. Les réserves restent dans le [verdict produit](.claude/audits/r6/expert-14-product.md).

## 3. Les quatre frontières les plus importantes

### Connaissances entre worktrees

Une fonction découverte ailleurs n’est pas nécessairement disponible dans l’environnement de l’agent. Les références distinguent code intégré, commit de feature, état non commité, travail en cours, suppression et représentation périmée. La provenance inclut le repository, le worktree et la version observée lorsque ces objets existent.

La recherche avant création devient une opération observable, que le contrat peut exiger. Sa preuve historique reste distincte de sa fraîcheur courante : créer précisément le composant autorisé ne doit pas effacer la recherche qui l’a précédé. Une modification concurrente pertinente impose une nouvelle vérification. L’index fournit de l’information pour réutiliser ou coordonner ; il ne garantit pas l’absence de doublons sémantiques. [Connaissance partagée](.claude/specs/shared-knowledge.md).

### Invocations et autonomie des managers

L’appel à une API, l’invocation d’un agent logique, l’étape et le run ont des frontières propres. Le N+2 organise ; le N+1 décide de la conduite de sa mission ; le harness exécute les transitions autorisées. Les attentes peuvent devenir durables et libérer la capacité après les conditions de neutralisation prévues.

Les décisions de clôture possèdent des commandes typées. Un compteur persistant borne les chaînes de décisions sans progrès et ne se réinitialise pas à chaque réveil. Une nouvelle version de workflow ne remplace pas celle du run engagé : la frontière de migration est le Run (Q-03 résolue), et une nouvelle version n’est adoptée que par un nouveau Run à référence exacte, sans migration en place en V1. [Hiérarchie](.claude/specs/agent-hierarchy.md), [sessions](.claude/architecture/session-model.md).

### Reprise après crash

Le PID indique un processus observé ; il ne suffit pas à garantir que le prochain signal vise la même incarnation. De même, un worker mort ne prouve pas qu’une commande externe a échoué. Le journal d’actions distingue effets confirmés, absents et inconnus. L’inconnue impose une réconciliation avant répétition.

La restauration introduit une nouvelle lignée d’autorité. Les anciennes commandes restent liées à leur lignée et sont refusées avant la recherche d’un reçu idempotent. Les effets et coûts postérieurs à la sauvegarde doivent être pris en compte ; une couverture inconnue crée une barrière visible. Les plans de test coupent l’exécution aux endroits où résultat externe et persistance peuvent diverger. [Reprise](.claude/architecture/failure-recovery.md), [IPC v2](.claude/architecture/ipc-and-apis.md).

### Compilation des skills en contrats

Un contrat valide une forme, des faits observables et des relations explicitement prévues. Il ne transforme pas un texte libre en garantie universelle de qualité. Le vocabulaire des prédicats est fermé et versionné ; les déclarations du modèle restent séparées des observations et du jugement du leader.

Pour un test attendu, l’attente et son association à une exécution sont scellées avant l’effet. Commande, contexte, suite, parser et snapshot doivent correspondre. Sélectionner après coup un log favorable ne satisfait pas ce contrat. La qualité sémantique reste une responsabilité des agents et de leur configuration, sans second juge LLM obligatoire. [Contrats](.claude/specs/contract-enforcement.md).

## 4. Développement et usages quotidiens partagent le moteur

La spécialisation développement est préservée : branches, worktrees, agents spécialisés, tests, audits choisis et intégration. La contrainte Rust concerne le cœur d’Unleash ; les projets supervisés peuvent être multilangages. Les bibliothèques et moteurs existants sont réutilisables, et HTML/CSS est permis pour la présentation. La GUI est un shell Tauri 2 avec renderer React/TypeScript strict, la TUI utilise Ratatui/Crossterm ([ADR-03](.claude/architecture/adr/ADR-03-rust-gui.md)) ; l’ergonomie se mesure sur les parcours réels dès M0. Les tests payants de développement sont limités à `claude-haiku-5-5` et `gpt-6-luna`. [Politique d’implémentation](.claude/brief/implementation-policy.md).

F019 rend la même mécanique utilisable pour des procédures documentaires, des données et le navigateur. Une phase de conception ou de réparation peut solliciter des modèles. La procédure publiée se rejoue ensuite avec une nouvelle version de données, sans modèle dans le mode interdit. Le mode supervisé appelle un modèle à des points publiés pour interpréter observations, assertions et logs ; un avis favorable ne supprime pas un effet inconnu.

Le profil de workflow v1.2 formalise les lignes, les lots, le curseur et les checkpoints. Une ligne bloquée avant la fin du lot ne peut pas attendre une supervision située après un checkpoint qu’elle empêche d’atteindre. La route de diagnostic doit être explicitement admissible. Cette conception sert aussi bien les tests E2E que des tâches personnelles, sans prétendre qu’un script Carrefour a été construit ou exécuté. [Automatisations](.claude/specs/reusable-automations.md), [itération](.claude/architecture/workflow-data-iteration.md).

Le guidage part du résultat souhaité ou d’une procédure proposée, sans exiger d’agent préconfiguré ni de dépôt. Il montre les données, les effets, les autorisations et les limites. Les profils LM Studio, Ollama et llama.cpp complètent les API distantes ; leur placement et leurs capacités restent à qualifier. Une adresse localhost ne constitue pas une preuve d’inférence locale. La parité inclut les réglages, les éditeurs, les erreurs et la reprise. [Guidage](.claude/design/guided-automation-experience.md), [modèles locaux](.claude/specs/local-models.md).

## 5. Ce que l’application peut exploiter

Le répertoire `.claude/` est documentaire. Il ne constitue ni le stockage runtime, ni une dépendance à Claude Code. Le [catalogue](.claude/catalog/README.md) expose les exigences, features, invariants, décisions, risques et questions. Son registre comporte 204 opérations documentées, dont 8 déjà citées par les contrats et ajoutées en R7, et leur correspondance avec la parité GUI/TUI.

L’import doit vérifier l’inventaire, les empreintes, les formats, les identifiants et chaque relation exacte REQ → architecture → feature → acceptation → test → preuve attendue. Une exigence omise, une chaîne permutée ou un statut de réussite sans preuve doivent provoquer un diagnostic. Les schémas ne déclenchent aucun téléchargement ou exécution implicite. [Contrat d’ingestion](.claude/specs/document-package-ingestion.md).

Le catalogue porte `contract_stage=documented` et, lorsque nécessaire, `payload_schema_ref=null`. Les contrats métier sont reliés, mais les DTO Rust et payloads runtime exhaustifs restent à produire en M0. L’importeur d’une application réelle n’a pas été fourni : sa compatibilité devra être testée. Le profil d’ingestion s’applique à l’export du paquet de conception, non au dépôt de réalisation qui l’héberge. Importer permet la consultation et la préparation de missions sous mandat ; cela ne publie aucun skill et ne transforme aucune EV attendue en résultat acquis.

## 6. L’ordre de réalisation

Les huit jalons organisent toute la V1 ; ils ne constituent pas des versions réduites du périmètre. M0 est ouvert : le code vit à la racine du même dépôt (`crates/`, `apps/gui-renderer/`, `spikes/`, `qualification/`). La reprise et la sécurité commencent dès le socle ; les interfaces peuvent avancer avec des fixtures du protocole partagé. Le [plan complet](.claude/plans/v1-implementation-plan.md) donne dépendances, critères et expériences.

| Jalon | Résultat visé |
| --- | --- |
| M0 | Types et contrats communs, DTO, expériences discriminantes SQLite, GUI/TUI, processus, connaissance, contrats et navigateur. |
| M1 | Daemon utilisateur, autorité locale, IPC, persistance, journal, outbox et premières preuves de reprise. |
| M2 | Exécutions, admission 5/50, sessions, fournisseurs cloud/locaux, budgets, terminaux et processus. |
| M3 | Workflows, compilation, contrats, skills/rôles, équipes et automatisations sans modèle ou supervisées. |
| M4 | Git, worktrees, intégration et connaissances reconstruites après modifications et incidents. |
| M5 | Connecteurs, GUI/TUI complètes, CLI, guidage et contexte documentaire inspectable. |
| M6 | Qualification de l’adaptateur AI-Guard réel et de la frontière d’exécution retenue (ActionBroker interne, GuardPort versionné). |
| M7 | Recette transversale, charge, pannes, restauration, plateformes et distribution. |

Aucune durée calendaire n’est inventée sans équipe, budget et résultats M0. Une feature se prend avec ses sept documents, ses dépendances, son contrat canonique et ses preuves attendues. Les modifications d’interface doivent être propagées aux consommateurs et aux tests dans le même travail coordonné.

## 7. Décisions rendues et preuves restantes

Les arbitrages sont rendus ; ils font autorité pour l’implémentation et restent révisables par un ADR de supersession. Principaux choix : GUI Tauri 2 avec React/TypeScript (Q-02) ; frontière de migration = le Run (Q-03) ; ActionBroker unique et GuardPort fail-closed quand AI-Guard est configuré (Q-04) ; modes de menace `standard` et `strict` (Q-05) ; licence Apache-2.0, noms OVPT et Unleash non concédés (Q-06) ; clés API fournies par l’utilisateur (Q-07) ; OAuth public avec PKCE pour Drive et OneDrive, jeton utilisateur en lecture seule pour Notion (Q-08, Q-09) ; matrice macOS 15, 26 et 27 sur Apple Silicon et Ubuntu 24.04 et 26.04 x86_64 (Q-11) ; navigateur WebDriver en Rust sur Chrome for Testing (Q-18) ; six templates intégrés du parcours guidé (Q-20). Le [registre des questions](.claude/brief/open-questions.md) donne la liste complète, avec les 21 questions résolues.

Q-21, le déclenchement calendaire autonome de nouveaux runs, est écartée de la V1. Restent dans la V1 la relance explicite d’une procédure publiée avec de nouvelles données, les timers internes des workflows et la CLI non interactive qu’un ordonnanceur externe choisi par l’utilisateur peut appeler.

Ce qui reste est de la preuve à produire, non une décision : EXT-GUARD, EXT-MODEL (avec des modèles intelligents, sur décision de Louis), EXT-MODEL-LOCAL, EXT-BROWSER, EXT-DOC, EXT-OS, EXT-OSS et EXT-DIST. La [readiness checklist](.claude/plans/readiness-checklist.md) conserve leurs conditions de clôture.

## 8. Réalisation par des agents

L’outillage optionnel de développement est dans le dépôt : `AGENTS.md`, `CLAUDE.md` et `GEMINI.md` à la racine ; `.claude/dev/` (protocole de goal, routage des modèles, définition de terminé, worktrees, démarrage M0) ; `.claude/rules/` ; neuf rôles dans `.claude/agents/dev/` ; en-têtes de sous-agents pour les quatorze experts ; skill `/goal` ; hook `git_guard.py` ; `.agents/` et `.codex/`. Les worktrees vivent uniquement sous `.claude/worktrees/`. Git : `main` pour les releases, `dev` poussée sur GitHub. Le produit ne dépend pas de ces outils.

Pour commencer, lire [AGENTS.md](AGENTS.md), puis lancer `/goal` (Claude Code ou Antigravity) ou `$unleash-goal` (Codex). Pour vérifier une réalisation, partir de la chaîne d’exigence et produire l’observation attendue. Les contrôles documentaires ne remplacent aucune preuve opérationnelle.
