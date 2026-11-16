# F008 — Moteur de contrats

## Objectif

Cette feature permet au harness de vérifier ce qu'il peut effectivement observer et de distinguer le reste comme jugement. Elle relie les entrées et sorties d'une tâche, ses outils, ses artefacts et ses traces à un contrat versionné. Une affirmation de l'agent ne suffit pas à prouver qu'une commande a été exécutée, qu'un test a réussi ou qu'un merge concerne le bon commit.

Le moteur produit des verdicts traçables par règle, avec pass, fail ou unknown. Le N+1 garde son jugement technique et peut demander correction ou escalade même lorsque les règles formelles passent. Il ne peut pas convertir un refus d'autorisation en succès. Les observations collectées, claims de l'agent et jugements du leader restent des objets distincts. Une contradiction automatiquement établie porte sur un prédicat typé du vocabulaire fermé ; une proposition qualitative reste inconnue pour le moteur.

## Périmètre V1

Le catalogue de validateurs couvre forme JSON, artefacts, digests, scopes, snapshots Git, processus observés, résultats de tests reconnus, liens claims/preuves, intégration et discovery receipts. Les contrats couvrent entrées, préconditions d'action, observations après action et clôture. La recherche préalable porte sur helpers, fonctions, classes, composants, API, tests, contrats, skills, rôles, automatisations et ressources techniques identifiables, selon la [liste canonique F009](../../specs/shared-knowledge.md#discovery--invariant-transversal).

Le catalogue v2, référence V1 depuis R7 ([ADR-07](../../architecture/adr/ADR-07-contract-enforcement.md)), ajoute `claim_predicate_matches` et `test_execution_matches`, sans changer les règles des bindings historiques. Le premier compare `process.exit_code` ou `test.expectation_satisfied` à une projection authentifiée ; le second compare une commande et son résultat à une attente issue du plan/contrat/mandat acceptés, scellée avant effet. Ni la sortie ni les références sélectionnées par le modèle ne choisissent l'attendu. Les [formats et résultats normatifs](../../specs/contract-enforcement.md) définissent correspondances exactes et inconnus, sans juge LLM obligatoire.

Discovery distingue origine agente, replay et contrôle utilisateur, preuve historique de consommation et revalidation finale de la cible livrée. Un reçu tardif ne devient jamais une recherche préalable. Ces contrôles s'appuient sur le contrat [F009](../../specs/shared-knowledge.md).

Le contrat peut exiger que les créations identifiables possèdent un reçu de découverte et une décision reuse/create avec stratégie explicite justifiée. Une nouvelle implémentation reste légitime lorsque la réutilisation n'est pas adaptée. Le moteur ne garantit pas l'absence absolue de doublons et ne confond pas ressource découverte avec ressource disponible dans le worktree courant.

## Limites et sécurité

Un schéma JSON valide ne prouve pas la vérité du contenu. Un exit code nul ne prouve pas que les tests attendus ont été exécutés. Une observation authentique mais relative à un ancien snapshot ne valide pas le code actuel. Les assertions qualitatives restent des jugements attribués ; la publication ne nécessite pas un second juge LLM universel.

Le code des validateurs internes est Rust, allowlisté et revu. Le Markdown d'un skill ne devient jamais code exécutable. Les permissions effectives sont contrôlées à l'action et ne peuvent être élargies par l'auteur du contrat. La frontière avec AI-Guard est fixée par [ADR-10](../../architecture/adr/ADR-10-ai-guard-boundary.md) (Q-04, R7) : GuardPort versionné, fail-closed ; le protocole réel et ses capacités restent à qualifier, sans être présumés.

Références : [contract-enforcement](../../specs/contract-enforcement.md), [modèle](../../architecture/skills-contract-model.md), [tests de contrats](../../testing/contract-tests.md), [audit Expert 8](../../audits/skills-and-contracts.md). Les exigences ci-dessous décrivent les vérifications à construire ; aucun produit implémenté ni garantie d'absence de bugs n'est revendiqué.
