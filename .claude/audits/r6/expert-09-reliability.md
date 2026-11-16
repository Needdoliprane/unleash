# R6 — Expert 09 Reliability and Persistence

Audit documentaire indépendant, 16 novembre 2026. Produit non implémenté : aucun crash, restore ou test de concurrence exécuté. Périmètre : F002/F018, SQLite, sessions, ordonnanceur et reprise. Les inconnues externes, les limites matérielles et les risques déjà déclarés ne constituent pas de nouveaux constats. Les constats initiaux sont conservés ; leur correction documentaire figure dans l'addendum.

## R6-E09-01 — BLOCKING_INVARIANT : retry client sans identité de lignée après restauration

**Confirmé.** Le [CommandEnvelope](../../architecture/ipc-and-apis.md#commandenvelope) ne porte ni génération de déduplication ni fenêtre d'émission ; la clé est `(actor_id, command_id)`. Le [schéma SQLite](../../architecture/sqlite-design.md#tables-dautorité-et-commandes) exige pourtant de refuser une ancienne clé dont la déduplication n'est plus garantie. Le [restore](../../architecture/failure-recovery.md#disque-plein-corruption-et-restauration) renouvelle lignée, secrets et flux ; `ResyncRequired` protège le curseur d'événements. Aucun contrat n'impose que la commande conservée par le client garde sa lignée d'origine après reconnexion authentifiée.

**Scénario déduit.** Snapshot à la révision 4 d'un run prêt ; `StartRun(K, expected_revision=4)` est ensuite engagé et produit un effet externe, mais sa réponse est perdue. Après restauration, la révision revient à 4 et le receipt K manque. Le client obtient un canal actuel, recharge son snapshot puis retransmet sa commande en attente. Recherche du receipt et contrôle de révision peuvent tous deux la traiter comme nouvelle. La barrière des anciens hosts ne protège pas cette nouvelle requête légitimement authentifiée. La réconciliation des effets postérieurs reste nécessaire, mais ne définit pas le traitement de cette ancienne commande.

**État requis et preuve.** Refus explicite de la commande ancienne sans nouveau dispatch ; lien vers la récupération et conservation de l'incertitude. L'oracle futur est le compteur du service externe, complété par les receipts et décisions, selon [T-F018-007](../../features/F018-reliability-recovery/tests.md).

**Correction proposée.** Ajouter une génération de déduplication attribuée par le serveur à l'enveloppe et au digest, persistée par le client dès la création de la commande et inchangée lors des retries. Un redémarrage ordinaire la conserve ; un restore l'invalide. Le serveur peut restituer un receipt historique disponible, mais refuse une commande ancienne sans receipt, même avec une connexion actuelle et une révision redevenue égale. Définir aussi une fenêtre vérifiable pour la rétention ; le client ne remplace jamais automatiquement génération ou command_id après ce refus. Toute nouvelle intention exige la disposition de récupération applicable.

**Alternative.** Conserver un registre durable de déduplication hors rollback, avec fusion des receipts/tombstones postérieurs avant activation. Si ce registre manque, bloquer les commandes incertaines. Cette solution ajoute une autorité durable à qualifier.

**Acceptation.** Étendre AC-F002-004/010 et AC-F018-007 : réponse perdue, snapshot antérieur, reconnect/resync, ancien K et ancien payload sur nouvelle connexion ; zéro second effet. Ajouter extinction de la fenêtre après purge, puis un redémarrage ordinaire où le retry doit encore rendre le receipt original. **À trancher :** génération retenue, durée contractuelle et erreur wire.

## R6-E09-02 — HIGH : collecte d'orphelins sans exclusion avec leur rattachement

**Confirmé.** Le [stockage des objets](../../architecture/storage-model.md#écriture-dobjets-volumineux) prépare et synchronise le fichier avant référence SQL. Le collecteur respecte une période de sûreté et les manifestes de sauvegarde. Aucun protocole ne sérialise sa sélection/unlink avec une nouvelle référence au même hash ; les [états d'objet](../../architecture/sqlite-design.md#actions-processus-et-preuves) n'exposent pas de réservation de collecte. [AC-F002-006](../../features/F002-persistence-events/acceptance.md) couvre les coupures de préparation, pas cette concurrence.

**Scénario déduit.** Un ancien orphelin H dépasse la période de sûreté. Le collecteur le sélectionne ; un producteur retrouve les mêmes octets H et engage une nouvelle référence ; le collecteur supprime ensuite le chemin sélectionné. Une vérification immédiatement avant unlink laisse encore un intervalle concurrent. La référence a été publiée après préparation durable, mais son objet disparaît sans purge décidée. Même risque lorsqu'une sauvegarde en préparation doit conserver H avant publication de son manifeste final.

**État requis et preuve.** Toute référence ou sauvegarde déclarée complète conserve ses octets ; sinon échec explicite avant publication. Une attente arbitrairement longue n'établit pas l'absence de producteur.

**Correction proposée.** Ajouter des pins durables pour préparation et sauvegarde, puis une réservation transactionnelle de suppression par hash/génération. La collecte vérifie références et pins sous le writer ; tant que la suppression est engagée, aucun rattachement ne peut réussir. Après unlink, finaliser le tombstone ; après crash, reprendre cette suppression idempotente. Le producteur attend ou prépare de nouveau l'objet après clôture.

**Alternative.** V1 : collecte uniquement en maintenance quiescente, avec producteurs et sauvegardes arrêtés sous exclusion explicite.

**Acceptation.** Étendre T-F002-006/010 : barrières sélection → rattachement → unlink, ordre inverse, snapshot en préparation et crash entre réservation/unlink/finalisation. Vérifier les octets de chaque référence publiée et de chaque backup complet. **À trancher :** pins durables ou maintenance exclusive ; aucune simple période de grâce ne ferme la course.

## Conclusion de revue

Deux corrections de contrat proposées, sans constat d'échec runtime. Leur levée exige enveloppes/transitions et critères alignés ; les preuves d'implémentation restent `not_run`.

## Addendum — corrections documentaires R6

**E09-01/E09-02 corrigés dans le périmètre attribué.** IPC v2 requiert la lignée d'origine, incluse au digest et refusée avant lookup si obsolète ; tombstones conservés et aucune conversion automatique de file cliente. Aucun ABI produit existant supposé. Le writer protège références/pins, deleting précède unlink, tombstone suit synchronisation ; backup sous barrière puis pins. La capsule post-snapshot conserve effets/coûts et blocages avant activation.

**Validation exécutée :** métaschéma, sept cas de forme (un accepté, six rejetés), 19 documents, 46 liens locaux et 20 correspondances REQ/AC/T/EV. Tests produit `not_run`. **Intégration root requise :** exemples/mentions IPC hors périmètre et matrices de traçabilité.
