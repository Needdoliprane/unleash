# F006 — Exigences atomiques

| Exigence | Règle testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-COMP-001 | Transformer NL en draft avec hypothèses, questions ouvertes et références au contexte réellement fourni. | AC-F006-001 | T-F006-001 | EV-F006-001 |
| REQ-COMP-002 | Refuser champs inconnus, doublons de clés, tailles/profondeurs excessives et schémas hors profil. | AC-F006-002 | T-F006-002 | EV-F006-002 |
| REQ-COMP-003 | Valider unicité/références des nœuds, bindings et dépendances exactes ou slots tardifs déclarés. | AC-F006-003 | T-F006-003 | EV-F006-003 |
| REQ-COMP-004 | Vérifier types et domination des lectures des guards, portées parallèles et résultats de boucles. | AC-F006-004 | T-F006-004 | EV-F006-004 |
| REQ-COMP-005 | Calculer une borne d'activations sans overflow et imposer limites/budgets explicites. | AC-F006-005 | T-F006-005 | EV-F006-005 |
| REQ-COMP-006 | Refuser eval, scripts libres, références réseau et capacité hors grants/plafonds. | AC-F006-006 | T-F006-006 | EV-F006-006 |
| REQ-COMP-007 | Normaliser de façon déterministe avec diff, digest et version de compilateur, sans ajout d'étape métier cachée. | AC-F006-007 | T-F006-007 | EV-F006-007 |
| REQ-COMP-008 | Fournir simulation bornée marquée comme telle, avec branches couvertes et non explorées. | AC-F006-008 | T-F006-008 | EV-F006-008 |
| REQ-COMP-009 | Publier une version immuable par opération révisionnée/idempotente sans altérer un run courant. | AC-F006-009 | T-F006-009 | EV-F006-009 |
| REQ-COMP-010 | Produire diagnostics stables localisés et permettre correction sans perte du draft ou de l'intention. | AC-F006-010 | T-F006-010 | EV-F006-010 |

Les règles sont appliquées aux profils `unleash.workflow.v1`, `unleash.workflow.v1.1` et `unleash.workflow.v1.2` explicitement versionnés, pas à tout langage de workflow imaginable. Un changement de profil demande une version et des tests de compatibilité. Une sortie structurée du modèle est une entrée non fiable pour le compilateur. Un refus doit citer le champ et le mécanisme concernés ; il ne doit pas inventer une contrainte métier non demandée.

Une intention Dev → Merge est valide si sa définition et ses permissions le sont, sans obligation de tests/audit. Un besoin qualitatif non automatiquement vérifiable est déclaré comme jugement. La publication ne constitue ni autorisation d'une action future ni choix implicite de version pour les runs déjà créés.
