# Architecture Decision Records

Les ADR ci-dessous documentent options, risques, coûts de complexité et critères de décision. Les ADR-01 à ADR-19 ont été acceptés le 16 novembre 2026 par l’arbitrage délégué de Louis et consignent leur amendement R7. Le statut confirmé est réservé aux invariants utilisateur. Une décision acceptée se révise par un ADR de supersession. ADR-15 à ADR-17 consignent les clarifications R2–R5 et leurs solutions techniques. ADR-18 et ADR-19 portent les contrats R6. [Révision R7](../../brief/revision-2026-11-16-r7.md)

| ID | Sujet | Statut |
| --- | --- | --- |
| ADR-01 | [SQLite et modèle d’état](ADR-01-sqlite-persistence.md) | Accepté avec amendement (R7) |
| ADR-02 | [Événements et dispatch durable](ADR-02-event-architecture.md) | Accepté avec amendement (R7) |
| ADR-03 | [GUI Tauri 2 + React/TypeScript et parité des interfaces](ADR-03-rust-gui.md) | Accepté avec amendement (R7) ; tranche Q-02 |
| ADR-04 | [Moteur de workflows et frontière de migration](ADR-04-workflow-semantics.md) | Accepté avec amendement (R7) ; tranche Q-03 |
| ADR-05 | [Sessions durables et incarnations éphémères](ADR-05-durable-sessions.md) | Accepté avec amendement (R7) |
| ADR-06 | [Skills et rôles persistants](ADR-06-skill-role-registry.md) | Accepté avec amendement (R7) |
| ADR-07 | [Contrats de sortie et preuves](ADR-07-contract-enforcement.md) | Accepté avec amendement (R7) |
| ADR-08 | [Connaissances partagées et discovery](ADR-08-knowledge-index.md) | Accepté avec amendement (R7) |
| ADR-09 | [Git, worktrees et intégration](ADR-09-git-worktrees.md) | Accepté avec amendement (R7) ; tranche Q-10 |
| ADR-10 | [Autorisation, isolation et AI-Guard](ADR-10-ai-guard-boundary.md) | Accepté avec amendement (R7) ; tranche Q-04, preuve EXT-GUARD restante |
| ADR-11 | [Abstraction des modèles API](ADR-11-model-api-abstraction.md) | Accepté avec amendement (R7) ; tranche Q-07 |
| ADR-12 | [Frontières N+2 / N+1 / N](ADR-12-hierarchy.md) | Accepté avec amendement (R7) |
| ADR-13 | [Concurrence, fairness et budgets](ADR-13-scheduler-budget.md) | Accepté avec amendement (R7) ; tranche Q-15 |
| ADR-14 | [Données locales, open source, plateformes et distribution](ADR-14-local-data-oss.md) | Accepté en R7 pour Q-06, Q-11 et Q-16. Règle de contribution révisée par l'ADR-20 |
| ADR-15 | [Rust pragmatique et parité GUI/TUI](ADR-15-interface-parity-and-pragmatic-rust.md) | Contraintes R2 confirmées ; solutions acceptées avec amendement (R7) |
| ADR-16 | [Automatisations et supervision par blocs](ADR-16-reusable-automation-and-supervision.md) | Besoins R3/R5 confirmés ; mécanismes acceptés avec amendement (R7) |
| ADR-17 | [Fournisseurs de modèles locaux](ADR-17-local-model-providers.md) | Besoin R4 confirmé ; mécanismes acceptés avec amendement (R7) |
| ADR-18 | [Itération bornée sur données et diagnostic bloqué](ADR-18-bounded-data-iteration.md) | Proposé en R6 ; accepté avec amendement (R7) |
| ADR-19 | [Paquet de conception contrôlable par une application](ADR-19-design-package-ingestion.md) | Proposé en R6 ; accepté avec amendement (R7) |
| ADR-20 | [Mention Signed-off-by facultative](ADR-20-contribution-signoff.md) | Accepté le 25 novembre 2026 pour les contributions (Q-06) |

## Procédure d’adoption

Un responsable lit l’ADR avec ses sources et tests, documente les points réfutés ou acceptés, puis enregistre une décision datée avec son autorité. Le statut d’une décision suit `proposed → accepted → superseded` ; l’en-tête d’un ADR accepté porte « Statut : accepté … » et le catalogue la même valeur. Les décisions qui changent un invariant restent un arbitrage du porteur. Les choix ordinaires d’implémentation conformes ne créent pas de gate humain permanent.

Une supersession conserve l’ADR ancien et pointe vers le nouveau. Toutes les questions du [registre](../../brief/open-questions.md) sont tranchées en R7 ; ce qui reste est de la preuve à produire (gates `EXT-*` et `GATE-*` de la [readiness checklist](../../plans/readiness-checklist.md)), dont EXT-GUARD pour AI-Guard, et non une décision en attente. Les versions effectives de runtime sont enregistrées dans les bundles, indépendamment du statut documentaire d’un ADR.

## Décisions R6

- [ADR-18](ADR-18-bounded-data-iteration.md) : itération de données bornée, curseurs et diagnostic sous blocage.
- [ADR-19](ADR-19-design-package-ingestion.md) : paquet documentaire structuré, ingestion et séparation des preuves.

Les corrections R6 de singleton utilisateur, générations de commande, probes, claims, provenance et droits des dérivés précisent les ADR existants ; elles ne sont pas des décisions utilisateur tacitement adoptées et sont acceptées en R7 avec les ADR qu’elles précisent. Le registre détaillé de revue conserve auteur, objection, alternative, correction et recette.
