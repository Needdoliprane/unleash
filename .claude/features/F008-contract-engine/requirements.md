# F008 — Exigences atomiques

| Exigence | Règle testable | AC | Test | EV |
|---|---|---|---|---|
| REQ-CONTRACT-001 | Valider entrées/sorties par schéma local borné sans exécuter prose ni références réseau. | AC-F008-001 | T-F008-001 | EV-F008-001 |
| REQ-CONTRACT-002 | Exécuter uniquement validateurs allowlistés et paramétrés ; épingler compiler/evaluator/catalogue avant évaluation et conserver ces versions à la reprise. | AC-F008-002 | T-F008-002 | EV-F008-002 |
| REQ-CONTRACT-003 | Calculer autorisation effective par intersection de grants/scopes/rôle/skill/politique à chaque action. | AC-F008-003 | T-F008-003 | EV-F008-003 |
| REQ-CONTRACT-004 | Représenter observations, claims typés/qualitatifs et jugements séparément ; résoudre auteur/producteur, sujet/version et liens sans autorité issue du JSON. | AC-F008-004 | T-F008-004 | EV-F008-004 |
| REQ-CONTRACT-005 | Comparer une preuve de commande exigée à ExpectedTestExecution et son binding autoritaires, scellés avant effet : action/origine, sujet/snapshot, outil, argv, cwd, environnement, suite, parseur ; refuser substitution depuis la sortie. | AC-F008-005 | T-F008-005 | EV-F008-005 |
| REQ-CONTRACT-006 | Produire pass/fail/unknown par règle ; une règle obligatoire inconnue ne vaut jamais pass. | AC-F008-006 | T-F008-006 | EV-F008-006 |
| REQ-CONTRACT-007 | Comparer uniquement les prédicats fermés/versionnés des claims aux projections authentifiées ; exposer contradictions, conserver qualitatif/nonvérifiable en unknown et exiger une règle de vérité prépubliée quand cette vérité conditionne l'acceptation. | AC-F008-007 | T-F008-007 | EV-F008-007 |
| REQ-CONTRACT-008 | Vérifier discovery préalable, origin/source_refs/validation_basis et DiscoveryUseObservation pour l'opération/cible ; réconcilier la cible livrée sans antidater une revalidation finale. | AC-F008-008 | T-F008-008 | EV-F008-008 |
| REQ-CONTRACT-009 | Évaluer publication et conformité sans second juge LLM obligatoire ; distinguer qualité sémantique et contrôles formels. | AC-F008-009 | T-F008-009 | EV-F008-009 |
| REQ-CONTRACT-010 | Conserver verdict, versions et preuves sous ACL/rétention ; purge ou révocation rend les limites explicites. | AC-F008-010 | T-F008-010 | EV-F008-010 |

Les règles obligatoires sont fixées par le contrat publié et la politique de sécurité. Le leader peut proposer une autre version pour un futur engagement, mais ne peut désactiver un contrôle courant par prose. Une sortie conforme avec une affirmation fausse doit rester distinguable d'une observation collectée par le harness.

Les contrôles de découverte vérifient ordre, scope, catégorie et références ; ils n'évaluent pas automatiquement l'équivalence sémantique entre deux helpers. Un reçu à couverture partielle indique cette limite. Les tests de cette feature portent sur garanties bornées et refus corrects, pas sur la promesse que les logiciels réalisés ne contiendront aucun défaut.

Le catalogue v2, référence V1 depuis R7, et les formats V1 de claim, attendu, binding et projection suivent [contract-enforcement](../../specs/contract-enforcement.md). Les anciennes formes restent lisibles ; les deux nouvelles obligations ont des règles/digests distincts. Aucune preuve produit n'est acquise par la validation documentaire de leurs schémas : statut des campagnes d'acceptation `not_run`.
