# Tests F002

**Tous les tests sont prévus, non exécutés sur un produit Unleash.** Ils doivent utiliser une vraie SQLite qualifiée et des fixtures Rust ; un mock seul ne vérifie ni WAL ni contraintes.

| Mapping | Injection et oracle |
|---|---|
| T-F002-001 → AC-F002-001 → EV-F002-001 | Arrêts C0/C1/C2 ; requêtes de contrôle sur chaque table et compteur zéro du broker avant commit. |
| T-F002-002 → AC-F002-002 → EV-F002-002 | Matrice de builds approuvés/refusés, version/source_id/compile_options et configuration effective ; lecture retenue, pression WAL/file et export sous quota selon campagne ci-dessous ; mesures indépendantes des octets, transactions, admissions et receipts, puis reprise. |
| T-F002-003 → AC-F002-003 → EV-F002-003 | Duplicate et reorder d'outbox, crash avant/après inbox ; compteur de mutation, reçu causal et dead-letter inspectable. |
| T-F002-004 → AC-F002-004 → EV-F002-004 | Commande v2 avec réponse perdue puis restart de même lignée ; v1 refusée sans ajout automatique de lignée, champs réordonnés/digest changé, omission/invalidité authority_generation ; purge de réponse et retry sur tombstone ; restore puis reconnexion avec ancienne commande/révision redevenue égale ; identité canonique, refus avant lookup et zéro seconde mutation/effet. |
| T-F002-005 → AC-F002-005 → EV-F002-005 | Interrompre chaque migration et ouvrir avec binaire ancien ; version/checksum et refus sans écriture. |
| T-F002-006 → AC-F002-006 → EV-F002-006 | Coupures pin/objet/rename/ref et C10a–d ; GC sélectionne H, producteur rattache H, GC reprend, puis ordre inverse ; crashes autour de deleting/unlink/synchronisation/tombstone ; pins suspects et recréation du hash dans un nouveau chemin ; aucun octet manquant derrière une référence publiée. |
| T-F002-007 → AC-F002-007 → EV-F002-007 | Injecteur I/O ou filesystem de test pour FULL/IOERR, copies corrompues isolées ; erreurs et invariants du receipt. |
| T-F002-008 → AC-F002-008 → EV-F002-008 | Barrières de réservations concurrentes et settle unknown ; sommes par compte, absence d'allocation partielle. |
| T-F002-009 → AC-F002-009 → EV-F002-009 | Acquisition/révocation/réattribution puis résultat tardif ; fences monotones et état inchangé. |
| T-F002-010 → AC-F002-010 → EV-F002-010 | C11a–d pendant backup concurrent à GC/publication ; puis backup40, host100, ancien token/commande et effets terminés depuis le snapshot ; neutralisation/capsule avant nouvelle lignée, aucun hot adopt ni retry ancien ; delta perdu gèle ses scopes ou tous les effets ; coûts non remis à zéro, objets manquants refusés. |

Chaque paquet EV inclut schéma, profil natif, seed/cutpoint, séquence de commandes, événements, résultats SQL et hashes des fixtures. Les données sensibles sont synthétiques. Une panne processus et une panne machine forment deux variantes séparées ; le test ne doit pas appeler l'une « power loss » si seul le daemon a reçu SIGKILL.

Les tests de propriétés génèrent transitions et retries pour vérifier absence de receipt sans état, de budget négatif et de fence décroissant. Leurs contre-exemples sont conservés comme régressions. Les tests de restauration vérifient aussi les artefacts et l'état de réconciliation, pas seulement `integrity_check`.

Les variantes R6 restent **not_run**. L'oracle de rétention inspecte une même clé avant/après purge puis après redémarrage ; diminuer la configuration de durée ne réduit pas une fenêtre déjà promise. L'oracle de restauration conserve un compteur d'effets externe et une file client durable inchangée à travers handshake/resync. L'oracle objets lit/recalcule les octets de chaque référence live et de chaque backup complete, et injecte un ancien collecteur ciblant l'ancienne incarnation après recréation du hash. Une simple vérification du texte des statuts ne prouve pas ces invariants.

Les variantes R7 restent **not_run** et ne créent aucun identifiant :

- T-F002-002 : mesurer p50/p95 du commit avec `fullfsync=ON` sur APFS et fsync standard sur ext4 sous la même charge ; un `sqlite_source_id()` différent de la valeur compilée refuse le démarrage (`STORAGE_BUILD_UNQUALIFIED`).
- T-F002-003 : dead_letter après 8 échecs non transitoires, réarmement unique après changement de `consumer_version`, ligne inbox conservée moins de 7 jours après acked, reprise d'un curseur à 6 j 23 h et ResyncRequired au-delà de 7 jours ou après restore ([ADR-02](../../architecture/adr/ADR-02-event-architecture.md)).
- T-F002-004 : même clé et même payload avec une autre expected_revision ou un autre aggregate_id donnent IdempotencyConflict, jamais l'ancien reçu.
- T-F002-008 : réadmission d'une même invocation après waiting_durable ; nouvelle réservation, ligne released précédente conservée, aucune violation d'unicité.
- T-F002-010 : sauvegarde `step(-1)` sous écriture concurrente, `integrity_check` et `foreign_key_check` de la copie avant complete, sauvegarde obligatoire avant migration, rotation 7+4 (Q-12) ; purge d'un événement référencé en tombstone, sans suppression physique de la ligne.

## R6 — Pression WAL, lecteur retenu et quota d'export

Cette sous-campagne complète T-F002-002 et T-F002-007, REQ-PERSIST-002/007 et le risque R-18. **Plan produit `not_run` ; aucune mesure de performance acquise.** Utiliser le writer et les lectures réels sur une base SQLite locale qualifiée. Un lecteur de fixture expose une barrière après ouverture de sa transaction ; le producteur de commandes et le collecteur de mesures sont indépendants du compteur d'interface.

Le manifeste fixe avant l'essai OS/filesystem, build natif, taille de page, paramètres de checkpoint, corpus/seed, débit offert, durée, taille maximale d'une transaction et budgets. Profil de qualification, dont les cinq premières valeurs sont aussi les valeurs par défaut décidées par [ADR-01](../../architecture/adr/ADR-01-sqlite-persistence.md) (R7) : file writer de huit demandes au plus, lecture de cinq secondes au plus, seuil WAL de pression à 16 MiB et arrêt des nouvelles écritures concernées à 32 MiB, transaction bornée à 1 MiB de croissance WAL, export limité à 8 MiB avec un fragment de 256 KiB au plus. Le corpus contient plus de 32 MiB d'écritures et un export supérieur à 8 MiB. Le quota d'export reste un paramètre de fixture. Aucune de ces valeurs n'est une capacité annoncée sur toute machine ; toute variante conserve ses limites chiffrées avant exécution.

1. Établir le WAL initial et un état connu, retenir le lecteur, puis soumettre des commandes assez rapidement pour rencontrer la pression et la file pleine. Mesurer après chaque admission/commit la profondeur de file, les octets WAL, le plus vieux lecteur et le reçu observé côté client. Au seuil d'arrêt, aucune nouvelle écriture concernée ne part ; la transaction déjà admise peut seulement consommer sa borne publiée. L'oracle est `WAL ≤ 32 MiB + 1 MiB`, hors taille initiale consignée ; pas une promesse de suppression immédiate des octets déjà alloués.
2. Expirer le lecteur par l'horloge contrôlée, puis vérifier la fin réelle de sa transaction et le progrès d'un checkpoint. Refaire avec neutralisation bloquée : backpressure/refus restent bornés, aucun effacement manuel du WAL et aucune confirmation inventée. Le test distingue ce cas de l'injection FULL/IOERR.
3. Exécuter l'export trop grand en même temps. Vérifier compteur d'octets de sortie, mémoire/file bornées et quota effectif : au plus 8 MiB plus le fragment déjà admis. Dépassement ou annulation donne un résultat incomplet/erreur explicite et libère la lecture ; aucun artefact n'est publié comme export complet. Les octets temporaires et leur nettoyage sont inspectés.
4. Interrompre avant puis après un commit sous pression, redémarrer et libérer les barrières. Comparer commandes/receipts captés par le client, état SQL, événements et compteur d'effets externe. Une réponse perdue est retrouvée par sa même clé ; chaque commande reconnue est expliquée, aucune mutation dupliquée, aucun journal/WAL supprimé pour masquer le défaut.

EV-F002-002/007 conserve séries temporelles et maxima, ordres des barrières, paramètres exacts, journaux du client et de l'oracle, taille et état des exports, état SQL avant/après, résultat des checkpoints et limites d'environnement. Un test passant seulement après baisse du débit ou hausse du quota conserve l'échec initial et la nouvelle configuration comme essai distinct.
