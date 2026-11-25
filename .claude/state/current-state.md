# Travaux en cours et à venir

Vue générée à partir des fiches `goal.md`. Les décisions et preuves sont conservées dans leurs journaux.

[Programme V1](goals/g0000-v1/goal.md) · 1/8 jalons clos

## Travaux en cours

### M1 — En cours

Cœur durable & autorité locale : F001, F002 complet, F017 broker interne, squelette F018

[Fiche](goals/g0002-m1/goal.md) · [Journal](goals/g0002-m1/journal.md)

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| CI M1 | Identifiant d’exécution et résultat des jobs GitHub Actions pour `b7c73bb`. | ROOT. Le workflow corrigé passe `actionlint` et a été publié sur `dev`. Le SHA distant est confirmé par Git. Le 24 novembre, le seul compte `gh` configuré renvoie HTTP 401 sur la lecture du compte et HTTP 404 sur Actions. ROOT attend l’accès en lecture ou le résultat primaire avec ID d’exécution et conclusions des jobs. |

### M2 — En cours

Exécutions, router modèles API/locaux & processus : F003, F011, F012

[Fiche](goals/g0003-m2/goal.md) · [Journal](goals/g0003-m2/journal.md)

13 des 16 lots sont intégrés sur `goal/g0003-m2@a4b5ed0`. Le dernier workspace Rust complet connu donne 808 réussites sur `244e4a4`. Le journal et l’état du 23 novembre portent les preuves par lot.

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| F003 · Affectation | La politique publiée et le contexte produisent un bundle, une admission et un worker observables sur le socket. | ROOT. Les candidats `9f3dac2` et `1c32455` sont prêts pour leurs premiers essais ciblés. Commencer par la publication `9f3dac2` dans le banc Docker rétabli. |
| F011 · Terminal | La disposition autorisée du contrôle et le devenir de ses ressources sont observés sur le socket. | ROOT. `64ea50c` atteste l’effet historique. Le contrat `b6b9d91` passe le validateur documentaire. Le test SQLite `4c9e464` est relu et son export est vérifié. Le premier essai Docker précède le writer, la migration et la qualification Acquire. |
| F012 · Génération | Un permis durable produit un seul appel de fixture. Le résultat ou l’issue inconnue et le compte d’appels survivent au redémarrage. | ROOT. Les preuves historiques portent sur la préparation ou les ports Store. Le nominal `b103442` reste `not_run`. Il attend un runner Darwin dédié avec le process-host de la même composition. |
| Q-19 | Les trois moteurs et les deux modèles produisent chacun les preuves des tuples de la matrice. | ROOT. Le manifeste Ollama A a été retrouvé. La sonde de reprise `f905e524` relue exige le digest chargé après chaque embedding. Essai des templates sur runner macOS dédié `not_run`. LM Studio `smoke`, `nomic`, `text` et usage dans le daemon restent ouverts. Le lancement macOS de llama.cpp attend son autorisation normale. |
| Composition M2 | Une commande puis une écriture réussissent dans le banc isolé. F003 puis F005 reprennent sur le même cache. | ROOT. Le premier passage `run-m2-m3` précède la suite `run-independent` F011/F006/F010. Les archives et les contrôles statiques sont acquis. Le 24 novembre à 11:28 UTC, Desktop affiche encore `stopping` pour la même session et le ping du moteur expire après cinq secondes. Hector coordonne le rétablissement. Les prévols Lima et Virtualization n’ont pas atteint le démarrage d’un moteur indépendant. |
| Ressources héritées | Réconcilier les branches historiques divergentes avec les candidats qualifiés. | ROOT. Les deux worktrees Antigravity et leurs refs puis quatre dossiers SMB désenregistrés ont été retirés. Seul le checkout canonique demeure. Les refs divergentes conservent leurs commits. |

### M3 — En cours

Workflows, contrats, compétences, hiérarchie & automatisations : F005, F008, F006, F007, F004, F019

[Fiche](goals/g0004-m3/goal.md) · [Journal](goals/g0004-m3/journal.md)

`goal/g0004-m3@966fa7d` intègre les tranches Workflows acceptées sur fixture et le contrat de brouillon et de validation F006. Le journal détaille les contrôles de chaque tranche.

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| F005 · Découverte par IPC | Paginer un projet, ouvrir un run et vérifier sa version engagée, sa portée et la fin de liste. | ROOT. `f5c87b9` place les deux fichiers de test inchangés de `f952523` sur M3. Exécuter le premier test ciblé dans Docker, puis raccorder les projections Store et daemon. |
| F005 · Workflows | Depuis Tauri et TUI, préparer, retrouver et démarrer le même run sur le daemon. | ROOT. La composition renderer `453339a` réunit création et liste. Le test `face239` vérifie la fraîcheur après confirmation. Exécuter ce premier essai dans Docker avant correction. |
| F006 · Publication | Créer et relire le brouillon, valider sa révision, puis publier une version immuable et constater que le run déjà engagé garde sa version. | ROOT. Contrat de brouillon et de validation intégré en `966fa7d`, avec dossier à zéro erreur et zéro avertissement. Les deux fichiers de test Python sont conservés sur `bdaff49`. Les deux tests socket sont réunis dans l’export `120f95a`. Premiers essais Rust et Python `not_run`. Le compilateur et le writer suivent les premiers résultats Docker. |
| F006 · Brouillon dérivé | Copier une version exacte puis relire la filiation et les octets de chaque révision autorisée, y compris au-delà de 1 Mio. | ROOT. Contrat et rattachement à PAR-WORKFLOWS-02 dans `5a883a8`, dossier à zéro erreur et zéro avertissement. Le test ciblé du catalogue Rust précède l’intégration. Le raccord et les essais sur le socket gardent leur propre validation. |
| F005/F008 · Moteur et contrats | Rejouer sans Git, agent ni modèle une procédure publiée, avec terminaison et preuve durable. | ROOT. Les contrats F005 et F008 précèdent la composition F006/F007/F004/F019. Le premier essai moteur suit le banc Rust. |

### M4 — En cours

Travail réel dans les repositories : F010, F009, enrichissement F016

[Fiche](goals/g0005-m4/goal.md) · [Journal](goals/g0005-m4/journal.md)

`goal/g0005-m4@37d4431` intègre les contrats `ListWorktrees` et `GetDiscoveryReceipt` sur la base documentaire M3. Le validateur de cette composition rend zéro erreur et zéro avertissement.

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| F010 · Inventaire | Sur le socket, lire le worktree dirty d’un dépôt rattaché et vérifier les octets du fichier et de l’index, HEAD et la ref. | ROOT. Le test `791d78a` dans l’export `120f95a` reste `not_run`. Exécuter son filtre dans Docker après le prévol et la première séquence F003/F005. Raccorder ensuite l’inventaire et la pagination. |
| F010 · Adoption | Une cible checkout rend `blocked_target_checkout` et une ref qui avance empêche une adoption obsolète. | ROOT. L’essai d’inventaire précède la qualification Q-10, T-F010-001/006, et l’intégration conditionnelle. |
| F009 · Producteur de discovery | Sur le socket, attester la visite du registre projet vide, engager une décision humaine et relire la page et le reçu après redémarrage. | ROOT. Contrat et premier test figés en `5a19f51`, 0 erreur et 0 avertissement documentaires. Exécuter le filtre F009 dans Docker sur cette composition. Le routage et l’écriture durable suivent le premier rouge observé. |
| F009 · Relecture de discovery | Sur le socket, relire le même reçu après invalidation et parcourir les observations autorisées. | ROOT. Contrat intégré en `37d4431`. Raccorder la projection et vérifier ACL, rétention et expiration du curseur dans Docker. |
| F009/F016 · Reçu volumineux | Assembler le reçu au-delà d’une trame et vérifier taille et digest sur le socket. Refuser le bloc suivant après révocation. | ROOT. Le contrat composé F006/F009 est relu sur `d7cd82d`. Exécuter le contrôle du catalogue Rust et la suite Python de contrat dans Docker avant intégration. Le raccord de `RecordDiscoveryDecision` et de `ReadArtifact` et l’essai socket restent à qualifier. |
| F009/F016 | Un run exploite le contexte du dépôt lié et montre les preuves de son parcours de travail. | ROOT. Raccorder les interfaces de F009 et F016 à l’inventaire du dépôt et au moteur M3. Exécuter les scénarios M4 sur les versions composées. |

## Travaux à venir

- **M5** — Expérience complète & connecteurs : F013, F014 (10 espaces), F015 (10 espaces), F016, parcours guidé Q-20
- **M6** — Sécurité, isolation `strict` Q-05 et adaptateur AI-Guard F017
- **M7** — Recette A–Z, F018 pannes/restauration, `unleash-update`, packaging macOS/Ubuntu, SBOM, OSS
