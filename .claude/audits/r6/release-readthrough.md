# R6 — Contrelecture de remise

**16 novembre 2026 — sous-agent distinct, lecture seule, puis relecture de la correction.** Cette passe complète les quatorze expertises ; elle ne remplace pas leur examen de domaine. Mandat limité : raccords restant dans les contrats prioritaires et absence de confusion entre documentation, importeur applicatif non fourni et garanties du produit.

## R6-REL-01 — Ordre de traitement d’une commande

**Constat initial.** Les étapes 1–2 de [workflow-engine](../../specs/workflow-engine.md) prescrivaient une recherche de reçu avant tout contrôle d’authority_generation et mélangeaient expected_revision avec le traitement d’un doublon. Cela contredisait [IPC v2](../../architecture/ipc-and-apis.md) et la [reprise](../../architecture/failure-recovery.md). Une équipe pouvait conserver l’ancien ordre et retourner un reçu importé d’une autre lignée après restauration.

**Correction intégrée.** Authentification, décodage/schéma et digest ; contrôle de la lignée avant receipt/tombstone ; traitement de la clé dans la même lignée ; révision et autorité pour une commande nouvelle ; recontrôle transactionnel. Un doublon identique ne vérifie pas sa révision attendue contre l’état courant. Une ancienne lignée ne devient pas une nouvelle intention par réémission automatique.

**Relecture indépendante.** Le même relecteur a relu les étapes 1–3 modifiées et leur contrat canonique. Correction confirmée dans les documents ; aucun test produit exécuté. La recette reste rattachée aux tests T-F018-002/007 et T-F002-004 ; les preuves sont attendues.

## Résultat borné de la passe

Aucun autre raccord important retenu dans le périmètre inspecté : découverte/disponibilité/fraîcheur, curseur d’itération et checkpoint inaccessible sous inconnue, prédicats de preuve, restauration, ingestion et statuts. Les 19 features, la parité, les modèles locaux et le replay sans agent sont conservés. Les quantités et projections encore en régénération ont été exclues de cette relecture ; elles relèvent du [contrôle final](../../testing/document-validation.md).

Ce résultat ne certifie ni absence d’erreur dans tous les textes, ni compatibilité avec un importeur absent, ni réalisation ou qualification du produit. Il ferme un raccord identifié avec une correction observable et une relecture attribuée.
