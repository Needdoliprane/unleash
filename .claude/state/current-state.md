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

Au 21 novembre, 13 lots sur 16 sont intégrés sur `goal/g0003-m2`. La fusion `244e4a4` ajoute `provider set`, les corrections Linux et la stabilisation des fixtures, avec 808 tests réussis sur l'arbre intégré. La dernière publication connue reste M1 sur `origin/dev@8598866`. Les preuves examinables sur les mêmes entrées servent à la revue des candidats éligibles.

| Périmètre | Prochain résultat observable | Dépendance immédiate |
| --- | --- | --- |
| F003 | `AssignTask` produit une unité durable que le tour déjà intégré admet une seule fois | `e14049a` ferme l'entrée du protocole. `5187df0` matérialise l'étape et la tâche sur une définition épinglée. `879c5bf` engage l'affectation et rend l'invocation visible dans la file du Store avec le bundle exact. `25d7430` publie le manifeste initial en conservant les entrées et la tête de session, vérifié sur onze tests Docker. ROOT poursuit le bundle et la résolution des affectations, puis l’IPC, la composition et l’admission effective. `StartRun` garde son critère propre. |
| F011 | L'autorité confirme la disposition du contrôle et la conservation ou libération des ressources prévues après réconciliation | Corrections Linux de `568c7b2` intégrées en `244e4a4`, avec revue et campagne Docker verte. La disposition complète reste ouverte. Le scénario CLI/PTY signalé reste non vérifié dans son périmètre. |
| F012, profils | Qualifier les remises CLI de liste et de validation sur l'arbre intégré | `provider set` intégré en `244e4a4`. La campagne Docker Linux au parallélisme par défaut donne 808 réussis, 0 échec et 26 ignorés. Le parcours CLI → daemon de configuration et révision passe séparément. |
| F012, génération | L'exécuteur consomme une seule fois le permis durable et inscrit le résultat ou l'issue inconnue | Le propriétaire d'admission reçoit le bail, le corps publié et le placement revalidés. Le writer lie ces faits à l'empreinte de la requête native. La route `/api/chat` de l'origine attestée est qualifiée par la fixture locale, sans envoi. ROOT doit raccorder le permis au transport, traiter l'observation et l'issue inconnue, puis qualifier la reprise et Q-19 sur le moteur réel. |
| Sortie M2 | Une campagne de composition couvre les lots revus, puis un rapport regroupe les résultats et les limites de F003, F011, F012 et Q-19 | L'acceptation et la publication du jalon exigent leurs gates et le contrôle de CI. L'accès à la CI distante a échoué avec HTTP 404. |
| Clôture des ressources héritées | Les quatre entrées Git auxiliaires sont réconciliées, puis les ressources intégrées éligibles sont retirées | ROOT. Une entrée pointe vers un chemin absent. La copie imbriquée contient un fichier modifié et un test non suivi. Vérifier les consommateurs et travaux uniques avant retrait. Trois branches intégrées sans checkout ont été retirées et vérifiées. |

## Travaux à venir

- **M3** — Workflows, contrats, compétences, hiérarchie & automatisations : F005, F008, F006, F007, F004, F019
- **M4** — Travail réel dans les repositories : F010, F009, enrichissement F016
- **M5** — Expérience complète & connecteurs : F013, F014 (10 espaces), F015 (10 espaces), F016, parcours guidé Q-20
- **M6** — Sécurité, isolation `strict` Q-05 et adaptateur AI-Guard F017
- **M7** — Recette A–Z, F018 pannes/restauration, `unleash-update`, packaging macOS/Ubuntu, SBOM, OSS
