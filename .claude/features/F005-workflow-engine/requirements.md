# F005 — Exigences atomiques

| Exigence | Règle testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-WF-001 | Épingler définition et digest du workflow pour toute la durée du run ; ne jamais suivre un alias à l'appel LLM. | AC-F005-001 | T-F005-001 | EV-F005-001 |
| REQ-WF-002 | Réduire état/commande/événement sans I/O externe ; mêmes entrées ordonnées donnent mêmes états et intentions. | AC-F005-002 | T-F005-002 | EV-F005-002 |
| REQ-WF-003 | Appliquer exactement les sémantiques structurées et guards vrai/faux/inconnu du DSL. | AC-F005-003 | T-F005-003 | EV-F005-003 |
| REQ-WF-004 | Faire respecter bornes de boucle, tentatives, activations, temps, coût et appels de manière cumulative. | AC-F005-004 | T-F005-004 | EV-F005-004 |
| REQ-WF-005 | Implémenter all/any_success/quorum avec arrêt et réconciliation des branches restantes avant clôture. | AC-F005-005 | T-F005-005 | EV-F005-005 |
| REQ-WF-006 | Interdire retry aveugle d'un effet inconnu ; conserver son identité et imposer réconciliation. | AC-F005-006 | T-F005-006 | EV-F005-006 |
| REQ-WF-007 | Terminer selon done publié sans imposer tests/audits, tout en gardant contrôles d'autorisation. | AC-F005-007 | T-F005-007 | EV-F005-007 |
| REQ-WF-008 | Distinguer appel, invocation, étape, run et cycle (= run, Q-03) ; adoption workflow nouvelle seulement sur nouveau run éligible. | AC-F005-008 | T-F005-008 | EV-F005-008 |
| REQ-WF-009 | Reprendre depuis état/journal durables sans réémettre automatiquement les effets déjà observés. | AC-F005-009 | T-F005-009 | EV-F005-009 |
| REQ-WF-010 | Permettre binding tardif explicite sur slot déclaré vierge et refuser toute substitution de binding engagé. | AC-F005-010 | T-F005-010 | EV-F005-010 |

Une définition valide ne garantit pas un résultat métier. La valeur `unknown` n'est jamais convertie en pass pour faciliter la progression. Les budgets de mission couvrent la succession de runs ; les retries et changements de worker ne remettent pas les compteurs à zéro. Les contrôles de sécurité s'appliquent même si aucune étape d'audit n'est définie.

Pour REQ-WF-008, la frontière terminale est la décision Q-03 ; aucune migration en place. Cette décision ne diminue pas REQ-WF-001 : aucune solution acceptable ne substitue une définition engagée au prochain appel LLM sans frontière définie et enregistrée. Les tests doivent afficher la règle d'adoption qu'ils vérifient.
