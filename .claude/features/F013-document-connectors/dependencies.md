# F013 — dépendances et intégration

| Dépendance | Contrat consommé |
| --- | --- |
| F002 | État comptes, curseurs, affectations et événements |
| F009 | Index dérivé et discovery sous ACL |
| F003 | Contextes épinglés par invocation |
| F012 | Destination autorisée et budgets contexte |
| F017 | OAuth, secrets, réseaux et politiques |
| F014 | Sélection et prévisualisation GUI |

Une dépendance de contrat peut être développée avec un double, mais le jalon d’intégration exige l’implémentation réelle. Les doubles n’accèdent ni au réseau de production ni à des secrets. Les équipes figent un schéma commun avant de travailler en parallèle ; une modification incompatible impose nouvelle version et test interfeatures.

Les clients consomment cette feature par le backend. Les sorties sont des résultats, événements et références de preuve ; elles ne modifient pas directement les projections d’une autre feature. La transaction de domaine engage les décisions avant dispatch d’I/O.

