# F017 — dépendances

| Feature / service | Interaction |
| --- | --- |
| F002 | Intentions, décisions, reçus, audit et révocations atomiques |
| F003/F007 | Identités, mandats, rôles et skills |
| F010/F011/F012/F013 | Tous points d’effet et transports sensibles |
| F014/F015/F016 | Administration, incidents et preuves |
| AI-Guard externe | Code/contrat réel et environnement de qualification pour recette complète |

Les fixtures communes doivent être verrouillées avec le contrat. Les stubs permettent un développement parallèle, mais une dépendance à un système réel exige sa recette. Chaque équipe signale les changements de schéma aux consommateurs et publie une migration de version lorsqu’ils sont incompatibles.

Les dépendances du plan n’imposent pas de finir toutes les interfaces avant un premier flux utile. Elles imposent en revanche de ne pas prétendre livrer une V1 complète tant qu’un écran, un contrôle, un connecteur ou une reprise requis manque. Voir le [chemin critique](../../plans/critical-path.md).

