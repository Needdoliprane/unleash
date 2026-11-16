# F007 — Plan de tests

| Test | AC | Procédure | Preuve attendue |
|---|---|---|---|
| T-F007-001 | AC-F007-001 | Similaires, index partiel, create justifié. | EV-F007-001 : discovery receipt et décision. |
| T-F007-002 | AC-F007-002 | Draft NL puis éditions/révisions. | EV-F007-002 : filiation et diagnostics. |
| T-F007-003 | AC-F007-003 | Modifier une composante à la fois. | EV-F007-003 : digests et refus mutation publiée. |
| T-F007-004 | AC-F007-004 | Délégation rédacteur et tentative d'auto-grant. | EV-F007-004 : auteurs, publication autorisée et refus. |
| T-F007-005 | AC-F007-005 | Publication sans fournisseur juge branché. | EV-F007-005 : rapport déterministe et zéro appel juge. |
| T-F007-006 | AC-F007-006 | Diff prompt/outils/contrat/ACL. | EV-F007-006 : diff complet et usages actifs. |
| T-F007-007 | AC-F007-007 | Course publication/binding/activation. | EV-F007-007 : manifest append-only et binding figé. |
| T-F007-008 | AC-F007-008 | Import malveillant, path escape, script. | EV-F007-008 : aucun effet, diagnostics et provenance. |
| T-F007-009 | AC-F007-009 | Révocation entre deux actions. | EV-F007-009 : refus broker et version intacte. |
| T-F007-010 | AC-F007-010 | Deux clients, perte connexion, même command_id. | EV-F007-010 : état commun et idempotence. |
| T-F007-011 | AC-F007-011 | Collisions de noms entre namespaces. | EV-F007-011 : résolutions exactes. |
| T-F007-012 | AC-F007-012 | Versions du rôle sur chaque champ. | EV-F007-012 : schémas et filiation de dépendances. |
| T-F007-013 | AC-F007-013 | Trois scopes, ACL partielle, ambiguïté locale. | EV-F007-013 : priorité visible et refus explicite. |
| T-F007-014 | AC-F007-014 | Composition N+1 dans/hors grants. | EV-F007-014 : discovery, succès et refus. |
| T-F007-015 | AC-F007-015 | Skill v2 et evaluator/catalogue v2 installés pendant rôle v1, crash puis reprise ; retirer ancien evaluator. | EV-F007-015 : bindings et versions préépinglées conservés, blocage si version indisponible. |
| T-F007-016 | AC-F007-016 | Deux skills sans entrée unique, entrée non membre, input/output différents, règle retirée, niveau/modèle/permission incompatibles. | EV-F007-016 : diagnostics distincts et aucun binding partiel. |
| T-F007-017 | AC-F007-017 | Trois instances et accès croisés. | EV-F007-017 : séparation et refus de scope. |
| T-F007-018 | AC-F007-018 | Purge autorisée avec usages historiques. | EV-F007-018 : tombstones et inspection honnête. |

Cette campagne inclut concurrence, panne et sécurité. Les fixtures de registres doivent être versionnées avec leurs attentes ; un index est supprimé puis reconstruit pour vérifier qu'il n'est pas source unique de vérité. Les tests sur capacités fournisseur utilisent des réponses connues et distinctes des modèles réellement disponibles. Aucun de ces tests n'est annoncé comme réalisé sur Unleash ; les schémas et exemples joints font l'objet d'un contrôle documentaire séparé.
