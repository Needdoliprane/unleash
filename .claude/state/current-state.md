# Travaux en cours et à venir

Vue générée à partir des fiches `goal.md`. Les décisions et preuves sont conservées dans leurs journaux.

[Programme V1](goals/g0000-v1/goal.md) · 1/8 jalons clos

## Travaux en cours

### M1 — En cours

Cœur durable & autorité locale : F001, F002 complet, F017 broker interne, squelette F018

[Fiche](goals/g0002-m1/goal.md) · [Journal](goals/g0002-m1/journal.md)

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| CI M1 | Identifiant d’exécution et résultat des jobs GitHub Actions pour le SHA publié sur `dev`. | ROOT. Le workflow corrigé passe `actionlint` et est publié sur `dev` (`b7c73bb`, confirmé par Git). Le 25 novembre, l’accès GitHub est rétabli (`gh` connecté, dépôt `Needdoliprane/unleash`) : le dépôt compte zéro exécution d’Actions, le push de `b7c73bb` n’a jamais déclenché de workflow ; Actions est activé depuis. La preuve viendra du prochain push sur `dev` (après intégration de M2 et gates vertes) : relever l’identifiant d’exécution et les conclusions des jobs `ubuntu-latest` et `macos-latest` pour le SHA publié. |

### M2 — En cours

Exécutions, router modèles API/locaux & processus : F003, F011, F012

[Fiche](goals/g0003-m2/goal.md) · [Journal](goals/g0003-m2/journal.md)

13 des 16 lots sont intégrés sur `goal/g0003-m2@a4b5ed0`. Le travail des trois lots ouverts est réuni sans conflit dans le candidat `task/g0003-m2/integrate-m2@acc8b10` (lot noyau `2b9ad12`, lot F012 `38370df`, montée de vitest à 4.1.12 `c49f1a4`). Le 24 novembre à 23:04 UTC, dans le banc Docker Linux : renderer, mise en forme, Clippy du workspace et rustdoc passent ; la campagne de tests compte 881 réussites, 7 échecs et 29 ignorés. Le candidat est figé, non intégré. Trois revues indépendantes l'ont examiné : « sous conditions » pour les exigences comme pour les standards, la relecture des exigences relevant qu'aucune des huit exigences citées n'est pleinement réalisée et que la génération via un service local managé s'arrête au permis de dispatch. [Journal du programme](../g0000-v1/journal.md)

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| Schémas du protocole | Les deux tests de validation des schémas passent sur le candidat corrigé. | Fait : correctif `cbf99d4` sur `task/g0003-m2/fix-m2-gates`, vert dans le banc (`evidence/g0003-m2/fix-m2-gates-20261124/`). |
| F003 · Affectation | Une politique de modèle de rôle se publie par `SetSettings` et se relit par `ReadSettings` (REQ-MODEL-002). | ROOT. Rédacteur en cours sur `task/g0003-m2/settings-model-policy` (banc `unleash-bench-d-20261125`). |
| F012 · Modèle interdit | `StartRun` d'un run `model_policy=forbidden` est refusé avant tout effet par `CapabilityDenied` et `ModelCallsForbidden` (REQ-MODEL-018). | ROOT. Rédacteur en cours sur `task/g0003-m2/forbidden-run` (banc `unleash-bench-c-20261125`). |
| F012 · Génération managée | Le parcours de génération via un service fournisseur managé va du placement à l'observation du résultat, sur le socket (REQ-MODEL-015, REQ-MODEL-013). | ROOT. Relecture « exigences » : parcours arrêté au permis de dispatch, aucun appelant de production. Deux rédacteurs en cours : « génération managée de bout en bout » (`task/g0003-m2/f012-managed-generation`) et « transport Ollama : délais, refus, métadonnées, jeton » (`task/g0003-m2/f012-transport`). |
| F012 · Inventaire | Les trois tests `t_f012_013_inventory_*` passent dans une campagne complète. | ROOT. En échec dans la campagne du 24 novembre (délai de reçu de 30 s), verts en moins de 1,3 s isolément ou rejoués dans l'ordre de la campagne. Cause probable : attentes d'E/S du disque de la VM Docker, situé sur un partage réseau ; `StorageBusy` observé sur d'autres tests du même passage. À confirmer par une campagne complète sur disque au calme. |
| F011 · Terminal | `e2e_m2_terminal_recovery` exécuté en entier (50 tests) et le changement non commité du lot noyau validé. | ROOT. Défaut `ESRCH` corrigé (`c49896a`). Le prérequis d'hôte natif du lot noyau (`d4a8cac`) ne casse aucun scénario : les 12 `t_f011_010` et l'acquisition nominale passent. Deux scénarios C5 et deux tests de l'hôte échouent par attente sous charge ; relecture isolée en cours. |
| REQ-SEC-007 | Une politique standard validée ne refuse plus la sauvegarde ni les opérations de terminal. | Sauvegarde : corrigée et vérifiée (`task/g0003-m2/secfix@78c33be`). Terminaux (acquisition, saisie) : rédacteur en cours sur la même branche. |
| Secret de contrôle et aléa de l'hôte | Les secrets de contrôle ne dérivent plus du générateur d'identifiants ; l'aléa rend une erreur au lieu d'un repli déterministe (REQ-TERM-003). | Corrigés et vérifiés : secrets de terminal (`038bb0f`), aléa (`bff1eb7`), secret du service fournisseur (`task/g0003-m2/secfix-review@4d89356`). Revue indépendante : « sous conditions » ; reste à faire après la tranche terminal : échec d'aléa classé en refus net avant toute écriture durable. |
| Dépendances | `cargo deny check` rend le code 0. | ROOT. Première exécution le 25 novembre : `advisories` en échec (`url` 2.5.8 ; `cookie` 0.16.2 et 0.18.2). Rédacteur en cours sur `task/g0003-m2/dependency-advisories`. |
| Répertoire de rendez-vous | Sous Linux, un seul candidat `/run/user/<uid>/unleash`, refus explicite avant toute création si la racine runtime manque (REQ-CORE-001). | ROOT. Verdict de l'avocat du diable rendu ; rédacteur en cours sur `task/g0003-m2/rendezvous-linux`. Part macOS différée à une qualification native. |
| Q-19 | Les trois moteurs et les deux modèles produisent chacun les preuves des tuples de la matrice. | ROOT. Inchangé : manifeste Ollama A retrouvé, sonde de reprise `f905e524` relue. Les essais macOS et le lancement llama.cpp attendent leur autorisation normale. |
| Composition M2 | Gates complètes vertes sur le candidat composé et revu, puis fusion `--no-ff` dans `goal/g0003-m2` et cascade vers M3 et M4. | ROOT. Après la remise des rédacteurs ci-dessus. Conditions de la relecture npm : cinq contrôles renderer rejoués sur chaque pointe cascadée, avec `RUN v4.1.12` et 6 fichiers / 37 cas. La CI distante attend toujours un accès GitHub en lecture (`gh` : 401 et 404). |
| Ressources héritées | Réconcilier les 16 branches de tâche héritées avec le travail intégré, puis les fermer. | ROOT. Aucune n'est reprise par équivalent d'après `git cherry` face à `goal/g0005-m4` (24 novembre) ; analyse en cours, fermeture une par une, après vérification du travail. |

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
