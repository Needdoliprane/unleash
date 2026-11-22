# Travaux en cours et à venir

Vue générée à partir des fiches `goal.md`. Les décisions et preuves sont conservées dans leurs journaux.

[Programme V1](goals/g0000-v1/goal.md) · 1/8 jalons clos

## Travaux en cours

### M1 — En cours

Cœur durable & autorité locale : F001, F002 complet, F017 broker interne, squelette F018

[Fiche](goals/g0002-m1/goal.md) · [Journal](goals/g0002-m1/journal.md)

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| CI M1 | Résultat GitHub Actions constaté pour le SHA publié `8598866` | ROOT. Dernière lecture CI en échec HTTP 401/404. |
| Résidu du checkout t02 | Absence physique et Git vérifiée après résolution du refus SMB | ROOT. Réconcilier le chemin exact, les consommateurs et la copie vérifiée avant retrait. |

### M2 — En cours

Exécutions, router modèles API/locaux & processus : F003, F011, F012

[Fiche](goals/g0003-m2/goal.md) · [Journal](goals/g0003-m2/journal.md)

Au 22 novembre, 13 des 16 lots du plan sont intégrés et acceptés sur `goal/g0003-m2`. Les trois lignes ouvertes sont `t03-terminal`, `t04-probe` et `t05-qualification`. Les raccords de `StartRun`, de `CreateAgent`, des commandes de profils et de la préférence du skill ont progressé sans satisfaire les critères restants de ces lots. Les 808 tests réussis concernent `244e4a4`. Les contrôles ciblés de `f0c0a20` comptent 34 réussites. La dernière publication connue reste M1 sur `origin/dev@8598866`.

| Périmètre | Prochain résultat observable | Dépendance immédiate |
| --- | --- | --- |
| `t05-qualification`, F003 | Par IPC, `StartRun` puis `CreateAgent` et `AssignTask` produisent le bundle et l’observation d’admissibilité de la première tâche. Le démarrage du worker et la reprise doivent aussi être qualifiés pour clore le lot. | ROOT raccorde la politique effective et les sources de contexte autorisées à `AssignTask`. Le test de référence exige la publication par le daemon sur ce parcours. `f0c0a20` a vérifié l’affectation et le rejeu avec un bundle prépublié par sa fixture ainsi que la résolution séparée de la préférence du skill principal. |
| `t03-terminal`, F011 | L’autorité confirme la disposition du contrôle et le sort des ressources prévues après réconciliation. | ROOT vérifie la disposition complète à partir de l’attestation historique reçue dans `64ea50c`. Ce raccord conserve l’action initiale en `OutcomeUnknown` et ses ressources. Les corrections Linux sont intégrées en `244e4a4`. Le scénario CLI/PTY déjà signalé garde son statut non vérifié. |
| `t04-probe`, F012 | Le transport consomme une fois le permis de génération durable, puis le daemon inscrit le résultat ou l’issue inconnue et reprend sans double effet. | ROOT termine l’engagement du lancement Service autorisé et raccorde le permis au transport et au writer d’observation. La route est validée sur une fixture locale. La qualification Q-19 avec les moteurs réels reste ouverte. |
| Composition et sortie M2 | La campagne couvre les raccords intégrés et le rapport regroupe les résultats et limites de F003, F011, F012 et Q-19. | ROOT vérifie la composition, dont les commandes de profils et Settings déjà intégrées, puis la CI au SHA publié. La dernière lecture de la CI M1 a renvoyé HTTP 404. |
| Clôture des ressources héritées | Les quatre entrées Git auxiliaires sont réconciliées, puis les ressources intégrées éligibles sont retirées | ROOT. Une entrée pointe vers un chemin absent. La copie imbriquée contient un fichier modifié et un test non suivi. Vérifier les consommateurs et travaux uniques avant retrait. Trois branches intégrées sans checkout ont été retirées et vérifiées. |

## Travaux à venir

- **M3** — Workflows, contrats, compétences, hiérarchie & automatisations : F005, F008, F006, F007, F004, F019
- **M4** — Travail réel dans les repositories : F010, F009, enrichissement F016
- **M5** — Expérience complète & connecteurs : F013, F014 (10 espaces), F015 (10 espaces), F016, parcours guidé Q-20
- **M6** — Sécurité, isolation `strict` Q-05 et adaptateur AI-Guard F017
- **M7** — Recette A–Z, F018 pannes/restauration, `unleash-update`, packaging macOS/Ubuntu, SBOM, OSS
