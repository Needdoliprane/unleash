# Périmètre fonctionnel complet de la V1

La V1 inclut les dix-huit features initiales et F019 ajoutée par les clarifications d’automatisation, soit dix-neuf features du [plan](../plans/v1-implementation-plan.md). Les jalons sont des étapes de construction, pas des versions commerciales amputées.

| Capacité V1 | Feature responsable | Résultat attendu |
| --- | --- | --- |
| Daemon local, clients déconnectables, service utilisateur | F001 | Une fermeture de fenêtre ne clôt pas un goal |
| SQLite, états, journal, diffusion durable | F002 | Mutation et événement liés par une transaction |
| Agents logiques, invocations, workers, contexte | F003 | Mémoire persistante distincte du processus |
| N+2 / N+1 / N, staffing et goals | F004 | Décisions autonomes dans des mandats explicites |
| Machine de workflows, conditions utilisateur | F005 | Procédure versionnée exécutée sans pilote LLM continu |
| Création en langage naturel et compilation | F006 | Draft normalisé, diagnostics et définition exécutable |
| Skills, rôles persistants, studio et évaluations | F007 | Deux registres conceptuels, versions et provenance |
| Contrats mécaniques et recherche préalable | F008 | Déclarations rapprochées de preuves observables |
| Code, travaux et preuves partagés multilangages | F009 | Découverte de composants intégrés ou non, fraîcheur visible |
| Branches, worktrees, intégration et conflits | F010 | Publication contrôlée, provenance des commits |
| Terminaux, processus, ports et logs | F011 | Découverte et supervision des processus existants |
| Modèles API distants et locaux, capacités, routage et budgets | F012 | OpenAI/Anthropic, LM Studio/Ollama/llama.cpp et profils compatibles qualifiés ; aucun abonnement CLI |
| Google Drive, OneDrive, Notion, extension MCP | F013 | Explorer, épingler, délimiter, surveiller les ressources choisies |
| GUI de supervision, hôte Rust et présentation qualifiée | F014 | Dix espaces fonctionnels, design soigné et contexte réellement envoyé |
| CLI scriptable et TUI Rust plein écran | F015 | Parité fonctionnelle complète avec la GUI, dont éditeurs, connecteurs et réglages ; navigation terminal aboutie |
| Rapports, activité et causalité | F016 | Du goal aux preuves et décisions |
| Sécurité minimale et adaptateur AI-Guard | F017 | Contrôle local, points d’intégration versionnés, qualification réelle requise |
| Reprise worker/daemon/machine, maintenance | F018 | Réconciliation sans répéter aveuglément les effets externes |
| Automatisations paramétrées et navigateur | F019 | Apprentissage, publication, replay sans modèle, supervision par blocs et réparation versionnée, avec parcours guidé GUI/TUI |

## Frontières qui ne retirent pas de fonctionnalités

Les profondeurs d’analyse varient selon le langage et l’analyseur disponible ; la recherche, les snapshots et le suivi des travaux restent offerts pour tous les projets cibles. Une référence syntaxique incertaine ne devient pas une dépendance compilée certaine.

La prévisualisation documentaire dépend des formats accessibles. Les contenus non pris en charge doivent être signalés, pas rendus sous forme de faux extrait. Une sélection peut référencer un document que le fournisseur ne permet plus de récupérer ; cela bloque son utilisation nouvelle, sans falsifier l’historique de transmission.

AI-Guard reste dans la V1. La frontière côté Unleash est décidée (Q-04, [ADR-10](../architecture/adr/ADR-10-ai-guard-boundary.md)) et ses doubles de test peuvent avancer. La recette d’une intégration réelle nécessite ses interfaces et un environnement qualifié : une simulation ne clôt pas F017.

La liberté du workflow inclut Dev → Merge et une terminaison « merge réussi ». Des critères supplémentaires de qualité ne sont pas injectés par le harness. Les contrôles d’autorisation, d’état, de portée et de cohérence des preuves restent indépendants.

## Restrictions techniques retenues, à ne pas confondre avec des invariants

La matrice qualifiée V1 est fixée par Q-11 ([ADR-14](../architecture/adr/ADR-14-local-data-oss.md)) : macOS 15, 26 et 27 sur Apple Silicon, Ubuntu 24.04 et 26.04 LTS x86_64 ; Windows, Mac Intel et autres distributions ne sont pas promis. Aucun besoin multi-hôte, cloud obligatoire ou Windows n’a été exprimé. Les interfaces et le stockage n’empêchent pas une extension, mais la V1 ne promet pas ces environnements sans demande.

Les connecteurs documentaires sont destinés à la lecture et à l’affectation du contexte. Une publication documentaire explicitement demandée dans un workflow utilise un outil d’écriture déclaré et autorisé, ou un artefact/local Git. Aucune autorisation générale d’écrire dans tous les services n’est déduite du seul branchement d’un connecteur.

La distribution open source, la provenance des dépendances et les règles de contribution font partie de la préparation de sortie ; la distribution signée et ses mises à jour sont décidées par Q-16 (ADR-14). La licence Apache-2.0 est décidée (Q-06) ; aucun framework ni aucune dépendance ne peut imposer une licence hors de l’allow-list (`deny.toml`) sans dérogation écrite.

## Polyvalence sans retrait du développement

Les missions de développement conservent intégralement branches, worktrees, analyses, tests, intégration, connaissances et hiérarchie. Une mission de rapprochement CSV, de documents ou de navigateur peut ne posséder aucun repository. Les sources, procédures et preuves prennent alors des références de données ou ressources, sans faux commit Git.

Le pilote navigateur de référence du profil `mediated` réutilise Chrome for Testing par WebDriver W3C piloté en Rust (Q-18, [exécution navigateur](../architecture/browser-execution.md)) ; un script de projet peut réutiliser Playwright ; MCP est une possibilité de transport/outillage, pas l’exigence d’appeler un LLM. Le contrôle d’un bureau entier reste un point d’extension de drivers à qualifier, sans prétendre couvrir toutes les applications OS. Le navigateur est le cas concret V1 demandé. Les exemples de courses/comptabilité ne promettent pas un connecteur marchand fonctionnel ni un logiciel de conformité comptable universel.

Le parcours guidé crée les objets nécessaires à partir d’un objectif ou d’une procédure recommandée. Les workflows par défaut sont des templates modifiables, sans test, audit ou juge ajouté à tous les workflows personnalisés. Localité des modèles, mode sans modèle ou supervisé et autorisations sont lisibles avant exécution.
