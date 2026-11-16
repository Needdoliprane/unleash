# F016 — Exigences

Les exigences définissent des preuves attendues. Aucun test de cette feature n'est déclaré exécuté dans ce dossier de conception.

| ID | Obligation testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-OBS-001 | Toute invocation et action doit conserver les identités et versions nécessaires à sa reconstruction après disparition du worker. | AC-F016-001 | T-F016-001 | EV-F016-001 |
| REQ-OBS-002 | Le modèle de données et les vues doivent distinguer observations, claims et judgments, avec liens de preuve et contradictions visibles. | AC-F016-002 | T-F016-002 | EV-F016-002 |
| REQ-OBS-003 | Une preuve de validation doit citer snapshot, contexte de build, commande, issue et complétude ; son réemploi hors périmètre doit être explicite. | AC-F016-003 | T-F016-003 | EV-F016-003 |
| REQ-OBS-004 | Les créations identifiées doivent être rapprochées de reçus discovery incluant résultats, décision, justification, antériorité et fraîcheur vérifiables à la clôture applicable. | AC-F016-004 | T-F016-004 | EV-F016-004 |
| REQ-OBS-005 | Les transitions métier et leurs événements/outbox doivent être écrits atomiquement dans la base autoritaire et dédupliqués à la redélivrance. | AC-F016-005 | T-F016-005 | EV-F016-005 |
| REQ-OBS-006 | Le manifeste d'invocation doit rendre inspectable le contexte réellement envoyé, ses versions et compactions, sans inventer ni exposer de chain-of-thought privé. | AC-F016-006 | T-F016-006 | EV-F016-006 |
| REQ-OBS-007 | Les métriques doivent distinguer activité réelle, attente, budgets auxiliaires, usage rapporté, coûts estimés et valeurs inconnues, avec labels bornés. | AC-F016-007 | T-F016-007 | EV-F016-007 |
| REQ-OBS-008 | Logs, preuves, rapports et contextes doivent appliquer minimisation, masquage et ACL avant restitution ou export, avec limites de filtrage déclarées. | AC-F016-008 | T-F016-008 | EV-F016-008 |
| REQ-OBS-009 | Chaque rapport doit fixer sa séquence de référence, versions et politique de vue, et exposer done effectif, livrables, vérifications omises et incertitudes. | AC-F016-009 | T-F016-009 | EV-F016-009 |
| REQ-OBS-010 | Tous les clients doivent reprendre le même flux par curseur, dédupliquer les événements et obtenir une resynchronisation explicite après perte de fenêtre. | AC-F016-010 | T-F016-010 | EV-F016-010 |
| REQ-OBS-011 | Une panne de journal ou un effet sans résultat doit produire blocage des nouveaux effets non journalisables ou `outcome_unknown`, sans succès ni échec certain inventé. | AC-F016-011 | T-F016-011 | EV-F016-011 |
| REQ-OBS-012 | Exports et rétention doivent être versionnés, autorisés et inspectables ; une suppression doit retirer les octets visés et marquer les preuves indisponibles sans réécrire l'histoire. | AC-F016-012 | T-F016-012 | EV-F016-012 |

## Bornes des garanties

REQ-OBS-005 porte sur une transition dans une base locale, pas sur une transaction répartie avec Git ou une API. REQ-OBS-008 n'affirme pas que la détection automatique élimine tous les secrets ; les champs structurés connus sont masqués à la source et les flux libres restent soumis à des limites explicites.

REQ-OBS-009 permet un résumé optionnel rédigé par un modèle, mais sa présence n'est pas nécessaire à la validité du rapport. Les sources et statuts déterministes doivent rester consultables. Une observation « commande terminée avec code 0 » n'est pas, à elle seule, un jugement « mission conforme ».
