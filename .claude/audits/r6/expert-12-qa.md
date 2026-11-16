# R6 — Expert 12 : QA et vérification

**16 novembre 2026 — revue documentaire indépendante.** Trois écarts retenus. Aucun résultat produit n’est revendiqué ; les statuts `not_run` sont correctement annoncés. Le catalogue et le contrat d’import en cours de consolidation ne sont pas jugés absents.

## Mesures réellement observées

Lecture du [protocole QA](../../agents/qa-expert.md), de la [stratégie](../../testing/test-strategy.md), des matrices, campagnes et gates. Exécution des deux helpers par `runpy`, avec `Path.write_text` neutralisé : aucun document modifié.

- Contrôle documentaire : 19 features, 133 documents canoniques, 239 REQ, 65 invariants, 12 schémas, 16 fixtures ; `issues=[]`.
- Campagne R5 : 19 cas, quatre positifs et quinze négatifs, zéro divergence, quatre vérifications de références.
- Cinq mutations en mémoire de la matrice ont ensuite été éprouvées, indépendamment du produit.

Empreinte SHA-256 du helper documentaire examiné : `03731a9b2add8c616f777cddb0c491b8bd0718b9f26ed0beee239def75138df8`. Les helpers restent des instruments de revue hors dossier livré.

## R6-E12-01 — Élevé : la vérification des chaînes accepte des faux positifs

**Constat.** [DOC-03](../../plans/readiness-checklist.md) et le [rapport](../../testing/document-validation.md) réclament une couverture REQ → AC → T → EV. Le helper vérifie seulement que chaque identifiant de la [matrice](../../testing/traceability.json) apparaît quelque part dans son fichier cible ; il ne vérifie ni exhaustivité inverse, ni unicité, ni correspondance des arêtes.

**Reproduction observée.** Substituer `Path.read_text` uniquement pour cette matrice, neutraliser les écritures, puis exécuter `_work/validate_documents.py` avec chacune de ces modifications :

| Mutation isolée | Résultat observé |
| --- | --- |
| Retirer la dernière REQ | 238 REQ, `issues=[]` |
| Remplacer la dernière REQ par la première | 239 REQ, `issues=[]` |
| Permuter AC/T/EV entre les deux premières REQ | 239 REQ, `issues=[]` |
| Mettre le premier statut à `passed` sans preuve | `issues=[]` |
| Remplacer le premier test par `T-F001-999` | Anomalie affichée, mais retour normal du script |

**Correction / alternative.** Comparer les ensembles et les arêtes exactes extraits des sources, contrôler unicité et statuts, puis rendre un code non nul en présence d’anomalies. Une génération depuis une source canonique reste possible, avec comparaison indépendante des fichiers livrés. Un hash garantit les octets ; il ne remplace pas ces contrôles sémantiques.

**AC.** Les cinq mutations sont rejetées avec diagnostic ciblé ; les 239 chaînes originales passent. Un statut exécuté réclame un paquet de preuve applicable. La clôture documentaire ne dépend pas d’une inspection humaine de stdout.

## R6-E12-02 — Moyen : les dépendances de preuve H/N omettent leur reprise

**Constat.** La [recette](../../testing/acceptance-scenarios.md) rattache H à REQ-REC-001/004/005 et REQ-AGENT-006 ; N à REQ-REC-002/003/004/007 et REQ-PERSIST-001. La [matrice d’intégration](../../testing/integration-tests.md) cite pour H uniquement T-F016-001/011 et, pour N, F010/F009/F016. Elle omet les sous-preuves [F018](../../features/F018-reliability-recovery/tests.md) exigées par ces critères. Les descriptions narratives sont plus complètes que leurs dépendances déclarées.

**Contre-exemple documentaire.** Affecter `passed` aux seuls tests listés dans les lignes H/N et laisser F018 en `not_run` : ces listes semblent complètes alors que fence, inconnues et restauration ne sont pas qualifiés. Ce contre-exemple est une analyse du graphe, pas une exécution produit.

**Correction / alternative.** Déclarer les dépendances obligatoires, notamment H → F018-001/004/005 et F003-006 ; N → F018-002/003/004/007 et F002-001. Conserver les autres preuves complémentaires. Sinon, interdire explicitement toute clôture E2E par agrégation de ces listes partielles.

**AC.** Une sous-preuve obligatoire absente, périmée ou bloquée interdit la clôture correspondante ; un résultat E2E propre conserve également sa commande, son environnement et son oracle indépendant.

## R6-E12-03 — Moyen : la pression WAL manque d’un scénario discriminant

**Constat.** [R-18](../../plans/risk-register.md) et la [conception F002](../../features/F002-persistence-events/technical-design.md) prescrivent readers bornés, checkpoint, file writer bornée et export ne retenant pas indéfiniment une lecture. Les [tests F002](../../features/F002-persistence-events/tests.md) couvrent versions, contention et erreurs, sans scénario explicite combinant lecteur retenu, écritures et pression WAL.

**Scénario proposé, non exécuté.** Retenir une lecture par barrière, lancer écritures et export jusqu’au budget de fixture, puis interrompre/reprendre. Observer simultanément durée de lecture, octets WAL, profondeur de file, admissions, receipts et reprise ; un simple test FULL ne démontre pas la prévention attendue.

**Correction / alternative.** Ajouter ce sous-cas à F002 avec budgets fixés avant essai, politique d’expiration/backpressure et oracles externes. L’alternative est un mode dégradé explicite, borné et testé.

**AC.** Aucune croissance non bornée, aucun faux receipt ni effacement WAL ; pression visible, commandes admises expliquées et reprise cohérente après libération du lecteur.

## Points conservés

Les [pannes](../../testing/crash-recovery-tests.md) séparent processus/machine et comptent les effets indépendamment. La [charge mixte](../../testing/concurrency-tests.md) vérifie les préfixes et le plafond partagé 5/50. La recette garde modèles locaux réels, absence de modèle en replay, multilangage et parité V1 distincts des contrôles documentaires.

## Addendum — corrections appliquées

E12-01 : extraction indépendante des sources, ensembles/identifiants/arêtes/textes/projection Markdown contrôlés, statuts `not_run` obligatoires, retour 1 sur anomalie. Le générateur R6 refuse les sources incohérentes avant écriture et ne réordonne plus les documents. La convention AC/T/EV à suffixe commun est explicite.

Format temporel documentaire : calendrier réel, `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, secondes 00–59, fraction de 1–9 chiffres ; offsets et secondes intercalaires hors profil. Registry fermé, sans résolution réseau/fichier.

Mesures en mémoire : 239 chaînes, 22 schémas, 27 fixtures ; vingt mutations rejetées, deux refus du générateur sans écriture, deux dates positives/cinq négatives. Zéro appel réseau. Le contrôle global conserve alors un lien de consolidation pendant vers expert-14-product.md ; génération finale à rejouer.

E12-02 : dépendances H/N obligatoires et preuve E2E propre exigées. E12-03 : sous-campagne WAL/lecteur/export ajoutée avec quotas, barrières, mesures indépendantes et reprise. Ces corrections documentaires ferment les écarts de conception ; essais produit toujours `not_run`.
