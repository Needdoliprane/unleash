# OVPT Unleash — Dossier d'ingénierie V1

**Audit contradictoire, architecture retenue et spécification complète — révision consolidée R7.** Référence de rédaction : 16 novembre 2026. Tous les arbitrages sont rendus par délégation de Louis (21 questions résolues, 19 ADR acceptés) ; la réalisation est ouverte dès le jalon M0. Produit OVPT destiné à l'open source. Cœur applicatif et système Rust, bibliothèques existantes et présentation HTML/CSS autorisées ; GUI et TUI à parité fonctionnelle complète. Les projets supervisés peuvent être multilangages.

Ce répertoire est le livrable documentaire. Il n'est ni le stockage runtime d'Unleash, ni une dépendance à Claude Code, ni une implémentation déjà construite. Il héberge aussi l'outillage optionnel des agents de développement (`dev/`, `rules/`, `agents/dev/`, `skills/goal`, `hooks/git_guard.py`) ; le code du produit vit à la racine du dépôt. Toutes les fonctionnalités V1 sont conservées ; les jalons décrivent leur ordre de réalisation.

## Verdict critique

Un noyau local Rust, SQLite, des sessions durables et des workers éphémères constituent une base défendable. La difficulté principale réside dans les frontières entre décisions non déterministes, actions externes et preuves. Le dossier refuse notamment de déduire une isolation OS d'un worktree, une équivalence sémantique d'une recherche similaire, une reprise sûre d'un PID et une garantie qualité d'une sortie JSON valide.

Quatre décisions acceptées structurent la V1 : snapshots/provenances explicites pour la connaissance multi-worktrees ; invocations bornées avec attentes durables libérant les slots ; journal d'actions avec réconciliation des résultats inconnus ; skills associés à des contrats fermés et versionnés. Le détail et les alternatives sont dans [le verdict consolidé](audits/consolidated-verdict.md).

## Parcours de lecture

1. [Vision](brief/vision.md), [périmètre V1](brief/product-scope-v1.md), [invariants](brief/invariants.md) et [clarifications](brief/clarifications.md).
2. [Révision R7](brief/revision-2026-11-16-r7.md) (arbitrages et audit : 16 revues par domaine, 238 constats, 218 confirmés et corrigés, 20 réfutés), puis, en historique, les [audits R6 des quatorze expertises](audits/r6/README.md), le [verdict consolidé](audits/consolidated-verdict.md), les [25 questions contradictoires](audits/adversarial-findings.md) et la [contrelecture de remise](audits/r6/release-readthrough.md).
3. [Architecture globale](architecture/system-overview.md), [interfaces communes](architecture/ipc-and-apis.md), [machines à états](architecture/state-machines.md), [sessions](architecture/session-model.md), [ADR](architecture/adr/README.md).
4. [Exigences produit](specs/product-requirements.md), dossiers des 19 features et [schémas](schemas/README.md).
5. [Design system OVPT appliqué à Unleash](design/visual-design-system.md), [annexe historique OVPT/AIFM](design/ovpt-aifm-historical-guide.md), [écrans](design/gui-screen-specifications.md), [parité GUI/TUI](design/gui-tui-parity.md) et [politique d’implémentation](brief/implementation-policy.md).
6. [Traçabilité](testing/traceability.md), [couverture des invariants](testing/invariant-coverage.md), [scénarios](testing/acceptance-scenarios.md), [contrôles documentaires](testing/document-validation.md).
7. [Plan V1](plans/v1-implementation-plan.md), [chemin critique](plans/critical-path.md) et [registre des questions résolues](brief/open-questions.md).
8. Outillage des agents : [protocole de goal](dev/goal-protocol.md), [démarrage M0](dev/m0-kickoff.md), [définition de terminé](dev/definition-of-done.md), [routage des modèles](dev/model-routing.md), [worktrees](dev/worktrees.md) et [règles](rules/design-dossier.md).

L'[INDEX](INDEX.md) référence tous les fichiers.

## Statuts et autorité documentaire

« Confirmé » désigne une demande explicite de Louis. « Accepté » désigne une décision d'architecture arrêtée par l'autorité déléguée de Louis : elle fait autorité pour l'implémentation, reste révisable par un ADR de supersession et ne devient jamais un invariant confirmé. « À challenger » identifie un risque ou une alternative sérieuse. « À confirmer » identifie une capacité externe, un compte ou des fichiers non vérifiés. Les ADR n'ont pas modifié les invariants.

En cas de divergence, corriger la documentation avant implémentation : les invariants et décisions explicitement adoptées priment ; les schémas et documents canoniques définissent leurs interfaces ; les features précisent comportements, erreurs et preuves. Les audits conservent l'histoire d'une objection et son statut de résolution, sans devenir des variantes concurrentes d'une interface.

Autorités canoniques : session-model pour Agent/Invocation/Attempt ; workflow-dsl et state-machines pour exécution ; ipc-and-apis et schemas pour le wire ; sqlite-design pour stockage ; shared-knowledge et discovery schema pour provenance/découverte ; skills-contract-model pour bindings/profils. Les résumés ne remplacent pas ces contrats.

## Comment commencer le développement

Lire d'abord `AGENTS.md` à la racine, puis lancer `/goal` (Claude Code ou Antigravity) ou `$unleash-goal` (Codex) ; ou, sans agent, lire le mandat et les exigences de la feature attribuée, puis ses dépendances. Stabiliser les types partagés et les points de panne au jalon M0. Écrire des tests Rust correspondant aux critères avant d'annoncer leur passage. Toute nouvelle action précise intention durable, autorisation, dispatch, observation, issue inconnue et reprise.

Les [protocoles d'experts](agents/README.md) indiquent qui mobiliser selon changement. Ils servent à la revue de conception et de code ; ils ne remplacent pas le modèle N+2/N+1/N propre au produit.

## Ce qui est vérifié et ce qui reste à qualifier

Les sources officielles et les artefacts documentaires ont été contrôlés selon leurs rapports. Restent des preuves à produire, et non des décisions : EXT-GUARD, EXT-MODEL (avec des modèles intelligents, sur décision de Louis), EXT-MODEL-LOCAL, EXT-BROWSER, EXT-DOC, EXT-OS, EXT-OSS et EXT-DIST. Aucun binaire Unleash, test de crash réel, benchmark, accès API facturé ou intégration AI-Guard existante n'a été exécuté dans cette mission. Les EV des features sont des preuves **attendues**, à produire pendant le développement.

GUI et TUI doivent être visuellement soignées et couvrir les mêmes fonctions, éditeurs et réglages compris. Choix retenus : GUI Tauri 2 avec renderer React/TypeScript strict ([ADR-03](architecture/adr/ADR-03-rust-gui.md)), TUI Ratatui avec Crossterm ; leur ergonomie se mesure sur les écrans et parcours complets dès M0. La réutilisation de bibliothèques et HTML/CSS est autorisée par Louis. Frontière de migration des workflows (le Run), AI-Guard (ActionBroker et GuardPort), licence Apache-2.0 et distribution sont décidés ; leurs preuves restent attribuées aux gates.

La [révision R2](brief/revision-2026-11-16-r2.md) consigne les corrections du langage et de la parité ; les IDs historiques restent stables. La [revue ciblée](audits/frontend-terminal-revision-review.md) contrôle leur propagation sans prétendre rejouer les quatorze audits initiaux.

## Réutilisation par Unleash

Le futur produit pourra ingérer les exigences, matrices et features de ce dossier, créer ses missions et collecter les preuves. Il ne pourra pas convertir automatiquement une EV attendue en preuve acquise. L'auto-développement du produit devra respecter les mêmes limites de contexte, de permission, de causalité et de réconciliation que les projets qu'il supervise.

## Extension polyvalente, modèles locaux et démarrage guidé

La spécialisation développement est conservée. F019 ajoute les procédures réutilisables et l’automatisation navigateur, utiles aussi pour données, documents et usages quotidiens. Deux modes d’exécution sont explicites : replay sans modèle, ou supervision LLM de blocs et résultats. Les apprentissages/réparations produisent des versions distinctes ; une nouvelle donnée ne force pas une nouvelle génération de code.

LM Studio, Ollama et llama.cpp complètent les fournisseurs API distants. Le parcours guidé permet de commencer sans agent configuré ni connaissance des workflows : objectif ou template, sources, paramètres, modèle si nécessaire, périmètre autorisé, puis exécution et résultats. Les mêmes fonctions existent en TUI et en mode expert.

Lire [la révision R3–R5](brief/revision-2026-11-16-r3-r5.md), [le périmètre généraliste](brief/general-purpose-scope.md), [la procédure et les modes](specs/reusable-automations.md), [l’exécution navigateur](architecture/browser-execution.md), [les modèles locaux](specs/local-models.md) et [le parcours guidé](design/guided-automation-experience.md). Les exemples ne prétendent pas être des automatisations Carrefour ou une qualification comptable déjà exécutées.

La [revue indépendante R3–R5](audits/generalist-local-consistency-review.md) conserve quatre raccords corrigés puis relus. Le [rapport de validation](testing/document-validation.md) donne l’inventaire exact, les résultats des schémas et la comparaison de l’archive, sans assimiler ces contrôles à des tests du produit.

## Révision R7 : arbitrages, audit et réalisation ouverte

La [révision R7](brief/revision-2026-11-16-r7.md) rend tous les arbitrages, corrige 218 constats confirmés sur 238 issus de seize revues par domaine contre-vérifiées (20 réfutés), écarte Q-21 de la V1 et ouvre la réalisation. Le paquet comprend 389 fichiers inventoriés par le manifest, plus le manifest lui-même (sont exclus `state/`, `worktrees/` et `settings.local.json`), 19 features de 7 documents, 239 exigences, 65 invariants, 204 opérations documentées, 58 groupes de parité, 22 schémas JSON et 3 666 liens locaux contrôlés. `CHECKSUMS.sha256` couvre `.claude/`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `LIRE-MOI.md`, `SYNTHESE.md`, `COMMENCER-ICI.html`, `.agents/` et `.codex/`, mais ni le code produit ni `.env`. L'outillage `tools/design-package/` (`sync.py`, `index.py`, `regen.py`, `validate.py`) régénère et contrôle le paquet. Worktrees uniquement sous `.claude/worktrees/` ; Git : `main` pour les releases, `dev` poussée sur GitHub ; tests payants de développement limités à `claude-haiku-5-5` et `gpt-6-luna`. Tous les tests du produit restent `not_run`.

## Révision R6 : revue spécialisée et paquet consommable (historique)

La [révision R6](brief/revision-2026-11-16-r6.md) corrige les raccords relevés par des agents spécialisés réellement délégués ; les [rapports attribués](audits/r6/README.md) conservent chaque objection et sa résolution. Le [catalogue structuré](catalog/README.md) rend exigences, statuts, features, opérations et parité contrôlables par une application. Le [contrat d’ingestion](specs/document-package-ingestion.md) distingue planification documentaire et exécution du produit ; aucun autostart ni promotion d’EV attendue en preuve acquise.

Les autorités de contrat comprennent désormais workflow-data-iteration pour le profil v1.2, state-machines pour les diagnostics sous blocage, model-routing pour historique/données dérivées, et IPC v2 pour la génération de commande. Les rapports historiques R2–R5 ne remplacent pas ces contrats consolidés.
