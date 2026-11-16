# F004 — Exigences atomiques

Les exigences ci-dessous définissent le comportement proposé V1. Chaque preuve EV doit être produite par une future implémentation et rester liée au snapshot testé. Les numéros ne sont pas des métriques d'avancement.

| Exigence | Règle testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-ORCH-001 | Représenter exactement trois niveaux conceptuels ; chaque mission a un owner unique et aucun cycle organisationnel. | AC-F004-001 | T-F004-001 | EV-F004-001 |
| REQ-ORCH-002 | Autoriser N+2 à créer N directement seulement avec mission, objectif borné, owner, rôle et scope explicites. | AC-F004-002 | T-F004-002 | EV-F004-002 |
| REQ-ORCH-003 | Appliquer les décisions leader uniquement via commande authentifiée, révision attendue et evidence bundle ; rejeter les conflits sans mutation partielle. | AC-F004-003 | T-F004-003 | EV-F004-003 |
| REQ-ORCH-004 | Séparer identités dormantes et invocations actives ; respecter 5 par défaut, configurable ≤50, managers compris. | AC-F004-004 | T-F004-004 | EV-F004-004 |
| REQ-ORCH-005 | Persister atomiquement message humain N+1, événement et visibilité mailbox N+2 sous ACL. | AC-F004-005 | T-F004-005 | EV-F004-005 |
| REQ-ORCH-006 | Déclencher les leaders selon politique d'événements, avec budget par invocation et borne des décisions sans progression. | AC-F004-006 | T-F004-006 | EV-F004-006 |
| REQ-ORCH-007 | Réaffecter owner ou scope avec nouvel epoch ; refuser les effets d'un worker sous ancien epoch. | AC-F004-007 | T-F004-007 | EV-F004-007 |
| REQ-ORCH-008 | Distinguer task, step, run, mission et goal ; clôturer chacun selon ses critères et preuves propres. | AC-F004-008 | T-F004-008 | EV-F004-008 |
| REQ-ORCH-009 | Permettre N+1 de rechercher/composer rôles et skills et de justifier la décision reuse/create et sa stratégie explicite dans ses grants. | AC-F004-009 | T-F004-009 | EV-F004-009 |
| REQ-ORCH-010 | Router les conflits d'intégration vers une mission spécialisée traçable, sans déléguer silencieusement l'arbitrage métier au worker. | AC-F004-010 | T-F004-010 | EV-F004-010 |

Les bornes d'invocation incluent nombre d'appels, durée, tokens et commandes. La borne sans progression doit être configurable avec valeur par défaut documentée ; l'acceptation vérifie son respect, pas une valeur arbitraire imposée ici. Les transitions ne peuvent pas être autorisées par le seul texte d'une conversation. Les claims, observations et jugements restent distincts.

Un événement visible n'implique pas un réveil LLM. Un nouveau skill peut servir un slot tardif déclaré non engagé, avec binding exact et validation ; il ne remplace pas le skill d'une invocation en cours. Le critère de done utilisateur peut rester merge seulement. Les contrôles d'authentification, d'intégrité et de permissions sont indépendants de ce choix.

## Compléments R3–R5

| Exigence | Règle vérifiable | Critère |
| --- | --- | --- |
| REQ-ORCH-011 | Permettre une mission généraliste sans repository ni agent actif, avec OwnerRef user ou agent et clôture mechanical/human/owner_agent explicite ; forbidden interdit tout réveil modèle. | AC-F004-011 |
| REQ-ORCH-012 | Préparer idempotemment les rôles/agents nécessaires à un parcours guidé et borner la supervision de blocs aux points publiés ; aucune décision LLM ne remplace une transition du harness. | AC-F004-012 |
