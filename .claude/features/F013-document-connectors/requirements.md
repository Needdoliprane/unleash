# F013 — exigences

Les IDs sont stables ; une modification de sens crée une révision documentaire et indique le critère affecté. Statut de toutes les lignes : comportement normatif proposé pour satisfaire les invariants confirmés du brief.

| Exigence | Règle vérifiable | Critère |
| --- | --- | --- |
| REQ-DOC-001 | Fournir les capacités documentaires V1 de Google Drive, OneDrive et Notion. | AC-F013-001 |
| REQ-DOC-002 | Rechercher et parcourir les ressources accessibles avec pagination et couverture visible. | AC-F013-002 |
| REQ-DOC-003 | Épingler une ressource exacte et sa politique de révision. | AC-F013-003 |
| REQ-DOC-004 | Borner l’ingestion à des racines/filtres explicitement affectés. | AC-F013-004 |
| REQ-DOC-005 | Suivre les changements localement sans endpoint public obligatoire. | AC-F013-005 |
| REQ-DOC-006 | Ne pas modifier le contexte d’une invocation déjà engagée. | AC-F013-006 |
| REQ-DOC-007 | Stocker les tokens au coffre et valider le flux desktop. | AC-F013-007 |
| REQ-DOC-008 | Offrir un parcours Notion local sans secret client confidentiel distribué ; un identifiant de client public déclaré non confidentiel par le fournisseur (Q-08) n’est pas un secret. | AC-F013-008 |
| REQ-DOC-009 | Empêcher nouveau contexte depuis une ressource dont accès retiré est observé. | AC-F013-009 |
| REQ-DOC-010 | Signaler contenus non pris en charge et qualité de capture. | AC-F013-010 |
| REQ-DOC-011 | Respecter limites compte/service et Retry-After. | AC-F013-011 |
| REQ-DOC-012 | Étendre par adaptateur qualifié sans confiance automatique annotations. | AC-F013-012 |

Les permissions, versions et résultats incertains sont des données explicites. Aucun critère ne peut être considéré satisfait uniquement parce qu’un modèle l’affirme. Chaque exigence est reliée à son test et à la preuve attendue dans [tests](tests.md).

