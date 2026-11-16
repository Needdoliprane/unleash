# ADR-01 — SQLite et modèle d’état

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : profil de connexion, bornes du writer, sauvegarde et sérialisation des erreurs de stockage fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Un produit local à cinquante invocations au maximum a besoin d’engagements atomiques et de reprise ; cette concurrence ne justifie pas cinquante writers.

Exigences : REQ-PERSIST, REQ-REC. Risques/hypothèses/questions : R-01 R-18 ; H-01 H-02.

## Options et compromis

Option recommandée : **SQLite état+journal+outbox avec writer unique**.

Alternatives considérées : Event sourcing intégral ; PostgreSQL local ; fichiers seuls.

SQLite simplifie installation et transaction ; event sourcing augmente le coût de migrations/replay ; serveur DB ajoute exploitation ; fichiers seuls compliquent cohérence multi-objets.

## Décision

WAL sur stockage local, synchronous=FULL pour engagements reconnus, transactions courtes, objets préparés avant refs, backup cohérent. Qualifier version/source SQLite et correctifs, jamais présumer SQLite système.

## Amendement R7

1. Une seule base SQLite d’autorité par identité utilisateur OS (H-01), sur stockage local dans la racine de données. État, journal, outbox/inbox, receipts et réservations sont écrits dans la même transaction. Ni event sourcing intégral, ni serveur de base de données.
2. `rusqlite` avec les features `bundled` et `backup`, version figée par Cargo.lock. SQLite est embarqué et inclut le correctif WAL-reset ; aucune SQLite système en build de release. Au démarrage, `sqlite_source_id()` doit égaler la valeur compilée, sinon refus `InternalFailure/STORAGE_BUILD_UNQUALIFIED`. Une montée de version SQLite est un changement isolé qui rejoue T-F002-002.
3. Writer unique sur thread dédié : `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`, `trusted_schema=OFF`, `busy_timeout=250`, `wal_autocheckpoint=0` ; sur macOS `fullfsync=ON`, checkpoints compris. Lecteurs sur connexions séparées `query_only=ON`, transaction de lecture de 5 s au plus.
4. File du writer bornée à 8 demandes, au-delà `ResourceBusy/BACKPRESSURE`. Checkpoint `PASSIVE` dès 16 MiB de WAL ou après 30 s d’inactivité ; à 32 MiB, refus des écritures non critiques jusqu’au checkpoint ; `TRUNCATE` en maintenance.
5. Sauvegarde par l’API Online Backup en une passe `step(-1)` depuis une connexion de lecture dédiée, puis `integrity_check` et `foreign_key_check` sur la copie avant de la déclarer complète.
6. Fenêtre de réponse complète d’un receipt : 7 jours par défaut, réglable de 1 à 90 jours, fixée au commit.
7. Sérialisation wire : StorageBusy → `ResourceBusy/STORAGE_BUSY` ; StorageFull → `InternalFailure/STORAGE_FULL` ; StorageUnavailable → `InternalFailure/STORAGE_UNAVAILABLE` ; StorageCorrupt → `InternalFailure/STORAGE_CORRUPT` ; InvariantViolation → `InternalFailure/INVARIANT_VIOLATION`.

Si le p95 de commit dépasse le budget F014, le repli décidé regroupe plusieurs commandes dans une transaction avec un SAVEPOINT par commande, sans toucher `synchronous` ni `fullfsync`. Ces valeurs restent à mesurer (T-F002-002, T-F002-010). Détail : [sqlite-design](../sqlite-design.md).

## Conséquences, complexité et impact V1

Concurrence d’écriture sérialisée et nécessité de gérer checkpoint/backpressure ; pas atomicité des effets externes. Coût modéré, justifié dès F002.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

F002/F018 : crash cutpoints, charge mixte, backup/restore et liste corrigée du WAL.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../sqlite-design.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

