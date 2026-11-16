# R6 — Expert 14 : Technical Product Architect

**16 novembre 2026 — revue finale de couverture et de cohérence documentaire.** Lecture du protocole produit, des invariants, du périmètre, des plans, des contrats d’ingestion/opérations et des treize expertises R6, puis vérification ciblée des corrections. Aucun produit Unleash, importeur applicatif, rendu GUI/TUI ou fournisseur réel exécuté.

## R6-E14-C01 — Périmètre intégral conservé

**Vérifié par contrôle documentaire en lecture seule :** 19 dossiers de features, chacun avec ses sept documents canoniques, soit **133 documents** ; **239 exigences uniques** extraites des sources ; **65 invariants uniques**, tous présents dans la matrice de couverture ; **58 groupes de parité uniques**. Ces comptages établissent la présence et la correspondance des objets, pas leur réalisation. Sources : [périmètre V1](../../brief/product-scope-v1.md), [invariants](../../brief/invariants.md), [couverture](../../testing/invariant-coverage.md), [parité](../../design/gui-tui-parity.md).

| Engagement | Raccord vérifié après corrections |
| --- | --- |
| Rust pragmatique | Cœur métier/système Rust, moteurs réutilisables et présentation HTML/CSS autorisés ; Q-01/Q-17 restent résolues. La frontière client/backend demeure explicite dans la [politique](../../brief/implementation-policy.md). |
| GUI/TUI iso et CLI scriptable | Dix espaces, opérations communes, éditeurs, réglages, erreurs et reprise conservés. Les parcours guidés retrouvent les drafts depuis un autre client et rendent leur application unique ; [catalogue](../../architecture/client-operation-catalog.md), [F004](../../features/F004-hierarchical-orchestration/interfaces.md). |
| Polyvalence avec spécialisation développement | Missions sans dépôt et références documentaires ne retirent ni worktrees, ni intégration, ni connaissance multilangage ; [périmètre généraliste](../../brief/general-purpose-scope.md). |
| Rejeu zéro modèle | `forbidden` couvre les descendants contrôlés, rapports et modèles locaux ; preuve indépendante attendue, aucune réparation cachée ; [F019](../../features/F019-reusable-automations/requirements.md). |
| Supervision par blocs | Profil v1.2, portée de ligne, curseur et `after_batch` complètent le moteur commun. Une ligne inconnue ne rejoint pas artificiellement un checkpoint inatteignable ; [itération](../../architecture/workflow-data-iteration.md). |
| Modèles locaux et départ sans agents forcés | Les trois profils locaux restent à qualifier. Probes, discovery humaine et replay disposent d’origines propres ; le guidage prépare seulement les objets nécessaires ; [scheduling](../../specs/scheduling.md), [guidage](../../design/guided-automation-experience.md). |

**Risque résiduel :** confondre exhaustivité des fiches et qualité du comportement. **Acceptation future :** toutes les features et les scénarios transversaux restent soumis aux preuves de la [readiness checklist](../../plans/readiness-checklist.md), y compris erreurs, reprises et limites de plateforme. Aucun jalon M0–M7 ne réduit la V1.

## R6-E14-C02 — Dossier exploitable pour organiser la réalisation

Le [contrat d’ingestion](../../specs/document-package-ingestion.md) définit l’archive, les formats, l’inventaire exact, les empreintes, les références confinées, les arêtes REQ→AC→T→EV et les statuts. L’application peut consulter les fiches et préparer des lots sous mandat. Importer ne publie aucun skill, n’exécute aucun prompt et n’attribue aucune preuve produit.

**Arbitrage retenu :** le [catalogue structuré](../../catalog/README.md) est une projection contrôlée des sources. `contract_stage=documented` et `payload_schema_ref=null` annoncent correctement la frontière actuelle. Produire les DTO Rust et schémas complets au jalon M0 est un travail de réalisation explicitement attribué ; cela ne masque pas une fonctionnalité supprimée. L’alternative consistant à générer directement des clients runtime depuis ces fiches demanderait des contrats de payload complets et leurs tests, aujourd’hui non livrés.

**Risque résiduel :** un consumer pourrait accorder trop d’autorité à un schéma ou considérer une preuve attendue comme acquise. **Acceptation documentaire :** paquet exact accepté, divergences refusées, aucune résolution réseau ni lecture hors paquet, aucun statut produit `passed`. La validation finale d’E12 et la contrepreuve indépendante d’[E13](expert-13-adversarial.md) doivent porter sur l’export stabilisé ; leurs réserves connues ne sont pas recomptées ici. La compatibilité avec une application réelle restera à vérifier sur son importeur, dont le code n’a pas été fourni.

## R6-E14-C03 — Prêt pour M0 et les travaux indépendants

**Aucun nouveau raccord important retenu dans cette passe.** Les corrections de reprise guidée sont retrouvées dans les contrats et parcours ; les dépendances E2E H/N incluent désormais les preuves de reprise, et F002 possède un scénario discriminant de pression WAL ([intégration](../../testing/integration-tests.md), [tests F002](../../features/F002-persistence-events/tests.md)). La clôture documentaire globale reste celle du contrôle final du paquet.

Le [plan](../../plans/v1-implementation-plan.md) et les [dépendances](../../plans/dependencies.md) permettent de commencer les contrats, expériences M0, fixtures et modules indépendants. Les interfaces communes doivent être stabilisées avant construction parallèle des consommateurs concernés. Les questions déjà attribuées ne constituent pas de nouveaux findings et ne suspendent pas tout le projet.

| Gate réel conservé | Résultat nécessaire avant sa clôture |
| --- | --- |
| GATE-ARCH / Q-03 | Frontière de run adoptée, contrats et DTO communs stabilisés. |
| GATE-UI / Q-02 | Comparatif de prototypes réels, choix motivé, parcours GUI/TUI complets et qualité observée. |
| EXT-GUARD / Q-04 | Protocole et instance AI-Guard réels, obligations imposables, refus/révocation/pannes et audit corrélé. |
| EXT-MODEL et EXT-MODEL-LOCAL | Comptes et tuples serveur/build/modèle réellement éprouvés, coûts, capacités, localité et erreurs. |
| EXT-DOC / Q-08–09 | Parcours de connexion qualifiés par fournisseur, identité stable après réauthentification, droits et révocations. |
| EXT-OS / EXT-DIST | Builds, processus, secrets, packaging, restauration et mises à jour sur la matrice annoncée. |
| EXT-OSS / Q-06 | Licence choisie par OVPT, provenance des dépendances, notices et conditions de publication établies. |

Sources et responsables : [gates](../../plans/readiness-checklist.md), [questions résiduelles](../../brief/open-questions.md). Le navigateur conserve également EXT-BROWSER. Aucun simulateur ne clôt ces qualifications ; aucun arbitrage de licence, de menace ou de cycle n’est attribué tacitement à Louis.

**Conclusion bornée :** base de conception suffisamment raccordée pour engager la réalisation complète, avec remise conditionnée au contrôle final du ZIP. Les garanties de produit restent à démontrer. Ce verdict ne certifie ni un runtime existant, ni une importation applicative déjà réussie, ni la qualité visuelle ou la fiabilité opérationnelle de la future V1.
