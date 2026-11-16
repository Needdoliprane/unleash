# F004 — Dépendances et frontières d'équipe

La feature dépend du noyau transactionnel pour révisions, idempotence, journal et outbox. Le scheduler est l'autorité du nombre d'invocations actives et de l'équité ; F004 lui transmet niveau, priorité, budget et dépendances sans gérer sa propre file cachée. Le service de sessions conserve l'identité et le contexte entre workers. Le broker contrôle toute action à effet de bord.

F005 fournit runs et activations. F007 fournit recherche/publication de rôles et skills ; les commandes de N+1 ne court-circuitent pas ces validations. F008 fournit verdicts et evidence bundles. L'intégration Git fournit des observations sur conflits et merges ; le knowledge fournit discovery receipts et provenance, y compris branches non mergées. GUI, CLI et TUI consomment les mêmes commandes et événements.

Les équipes peuvent travailler indépendamment en convenant d'abord des identifiants d'agrégat, de l'enveloppe de commande, des événements de mailbox et du format du manifest d'invocation. Les doubles de tests représentent scheduler, broker et fournisseur par événements ; ils ne simulent pas une garantie d'exécution externe. Une équipe ne doit pas attendre un modèle réel pour vérifier unicité d'owner ou atomicité d'un message.

L'autorité de permission est hors de la hiérarchie conversationnelle. Le contrat externe AI-Guard demeure inconnu ; F004 peut demander une décision au broker local proposé sans inventer un endpoint AI-Guard. Le système doit signaler un service requis indisponible et ne pas poursuivre une action interdite. La documentation `.claude` est un dossier de conception, pas l'emplacement runtime des sessions ou registres.
