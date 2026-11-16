# Campagne de pannes et restauration

**Spécification future, non exécutée.** La campagne relie T-F002-001/004/005/006/007/010, T-F003-001/006, T-F011-002/003/005/010 et T-F018-001 à 010. Les outils d'injection et fixtures internes seront Rust ; ils travaillent dans des racines synthétiques et des VM dédiées lorsque nécessaire.

## Oracle indépendant

Un service de fixture externe au daemon conserve son propre compteur d'effets et journal. Il peut appliquer un effet puis perdre son receipt, répondre après timeout, refuser la réconciliation ou renvoyer un identifiant idempotent. Après reprise, vérifier à la fois DB/journal Unleash et service externe. L'absence d'un événement « second effet » dans Unleash ne prouve pas que l'effet n'a pas été doublé.

Pour chaque commande reconnue, rechercher receipt ou tombstone, lignée d'origine, état et causalité. Pour chaque action, vérifier sa classification : non dispatchée (`not_dispatched`), en cours (`in_flight`), succès confirmé (`confirmed_success`), échec sans effet confirmé (`confirmed_failure_no_effect`), conflit (`conflict`) ou inconnue (`unknown`), et l'état d'action correspondant selon la [machine d'état d'action](../architecture/failure-recovery.md#machine-détat-daction). Pour chaque ressource, vérifier propriétaire/génération/fence et réserve. Pour chaque référence d'objet live et backup complete, lire les octets attendus et recalculer leur hash ; un statut SQL seul ne prouve pas leur présence. Les succès inconnus ne deviennent ni échec sans effet ni retry automatique.

## Matrice de cutpoints

| Coupure | Injection | Résultat attendu |
|---|---|---|
| C0 | Avant transaction | Aucun effet et aucune commande reconnue. |
| C1 | Pendant écriture avant commit | Tout l'ensemble transactionnel absent ou restauré atomiquement. |
| C2 | Après commit avant réponse | Même lignée : receipt conservé ou refus du tombstone ; autre lignée : refus avant lookup ; pas de seconde décision. |
| C3 | Intention durable, avant marqueur durable de dispatch | Exécution possible après revalidation si absence de départ certaine. |
| C4 | Après marqueur dispatch et avant/pendant I/O | Unknown si le départ/effet ne peut pas être exclu. |
| C5 | Effet externe accompli, receipt perdu | Aucun redispatch aveugle ; réconciliation par preuve ou décision. |
| C6 | Receipt observé, transaction locale interrompue | Appliquer observation dédupliquée, jamais refaire l'effet. |
| C7 | Observation commit, notification absente | Outbox/inbox rattrapent et les vues convergent. |
| C8 | Checkpoint/libération slot en cours | Réserve et checkpoint atomiques ; continuation locale neutralisée avant libération. |
| C9 | Réaffectation de lease puis résultat ancien | Ancien fence refusé, historique diagnostic conservé. |
| C10 | Objet durable avant référence, collecte concurrente | Pin protecteur ; référence et réservation deleting exclusives sous writer, pas de référence publiée vers un chemin supprimable. |
| C11 | Snapshot/migration incomplet | Barrière/pins conservés jusqu'à reprise/abandon ; pas de publication d'un backup incomplet ni ouverture incohérente. |
| C12 | Reboot/perte de machine | Boot différent, récupération durable et inconnues distantes conservées. |

Le marqueur durable de dispatch précède toute I/O susceptible d'effet. Si l'exécuteur ne peut pas établir qu'il n'a rien envoyé, il classe C4, même si le crash était proche de C3. Cette règle est indispensable à une distinction exploitable entre « préparé » et « peut avoir agi ».

### Variantes C10 et C11 — collecte et sauvegarde

| Variante | Interruption ou course déterministe | Oracle requis |
|---|---|---|
| C10a | Pin de préparation committé, octets ou référence non publiés | Pin held/reconciling empêche la collecte jusqu'à neutralisation et reprise/abandon ; aucune libération par âge seul. |
| C10b | GC sélectionne H, puis publication de H avant réservation deleting ; ordre inverse en variante | La publication sous pin gagne et la collecte est refusée, ou deleting gagne et tout rattachement/pin échoue OBJECT_DELETING ; aucun succès des deux. |
| C10c | Intention deleting/prepared committée, avant/après unlink | Recovery retrouve chemin/génération/motif, conserve exclusion et reprend uniquement cette suppression ; I/O error garde deleting, divergence conduit au diagnostic. |
| C10d | Synchronisation effectuée, tombstone non committé ; ancienne opération rejouée après recréation de H | Finalisation idempotente ; nouvelle incarnation utilise un autre chemin et ne subit aucun unlink tardif. |
| C11a | Backup_barrier durable, suppression antérieure encore en cours | Attendre sa clôture avant copie DB ; aucune nouvelle suppression engagée ; crash conserve la barrière. |
| C11b | Snapshot DB terminé, pins exacts non encore acquis | Barrière protège les objets jusqu'à lecture du graphe du snapshot et acquisition atomique des pins ; aucun manifeste final. |
| C11c | Pins acquis puis copie objets interrompue | GC concurrent ne retire aucun objet nécessaire ; reprise ou abandon réconcilié conserve/libère les pins explicitement. |
| C11d | Octets/manifeste synchronisés, état complete non committé | Vérifier destination puis publier complete sans nouvelle copie d'effet ; complete implique tous les hashes/couvertures vérifiables. |

Inclure une purge autorisée alors qu'un producteur ou un backup possède un pin : elle doit attendre sa clôture ou obtenir un abandon réconcilié, sans retirer le pin par simple mise à jour. Injecter un second crash pendant chacun des chemins de récupération. Les fixtures testent aussi fichier absent, hash divergent et génération de chemin inattendue ; aucun unlink opportuniste d'un chemin étranger n'est permis.

## Classes de panne séparées

Crash worker : conserver session/bundle/décisions, invalider tentative et rapprocher actions. Crash daemon : conserver hosts potentiellement vivants, revalider canaux et reservations avant admission. Crash host : maître PTY potentiellement perdu, enfant éventuellement encore vivant, entrée ambiguë non répétée. Reboot : anciens PID non contrôlables, données RAM perdues et effet fournisseur encore possible.

Une séquence SIGKILL simule la panne du processus ciblé, pas une coupure de courant. La campagne machine emploie une VM ou un injecteur filesystem dont le modèle de cache/flush est déclaré. Les essais sur APFS et filesystem Linux indiquent leurs paramètres ; une simulation ne certifie pas tous les disques physiques. Les hypothèses de durabilité sont celles qualifiées pour le build natif et sa configuration.

## Pannes de stockage et de ressources

Injecter SQLITE_BUSY, file writer pleine, espace libre sous 1 GiB, FULL, IOERR et corruption sur copies isolées ; contrôler les erreurs et leur sérialisation wire ([SQLite](../architecture/sqlite-design.md#algorithme-transactionnel-et-gestion-derreurs)), le mode de disponibilité et l'absence de fausse confirmation. Le journal/WAL n'est pas effacé automatiquement. Les logs de host saturés créent un gap déclaré ou l'arrêt prévu, jamais une croissance infinie ou une prétendue preuve complète.

Introduire un PID réutilisé, start_identity manquante, port repris par un programme inconnu et descendant détaché. Aucun signal ne doit atteindre un témoin qui ne correspond pas à la génération autorisée. Introduire horloge reculée, longue veille et changement boot ; vérifier qu'une deadline murale ne libère pas une propriété à elle seule.

Conserver coûts inconnus et ressources suspectes. Après neutralisation locale, le slot peut être rendu suivant son contrat alors que la réserve financière demeure ; ce cas doit être explicitement vérifié. Un AccountingMismatch bloque seulement les pools concernés, sans réécrire silencieusement le ledger pour obtenir zéro.

## Sauvegarde et restauration

Produire un backup pendant activité sous barrière globale puis pins exacts avant copie des objets, en concurrence avec collecte et publication ; accomplir ensuite un effet externe. Restaurer dans une nouvelle racine avec objet manquant, hash divergent ou schéma incompatible comme variantes. Un backup valide doit passer contrôles structurels, foreign keys, références et hashes ; la couverture Git/non committé, effets/receipts/tombstones et budgets doit correspondre au manifeste. Les fonctions [SQLite Backup API](https://sqlite.org/backup.html), en une passe `sqlite3_backup_step(-1)` sur connexion de lecture dédiée suivie d'`integrity_check` et de `foreign_key_check` de la copie, fondent le snapshot DB mais ne couvrent pas seules ces objets externes.

La restauration gèle admissions/effets et exige inventaire complet et neutralisation vérifiée des exécuteurs de la lignée courante avant d'activer une autre authority_generation, de nouveaux secrets et stream_id. Une capsule durable du delta post-snapshot est copiée/vérifiée/épinglée puis importée avec ses blocages avant activation. Elle inclut actions déjà terminales, receipts, preuves et coûts ; aucun hot adopt d'ancien host ni dispatch d'une action prepared du snapshot sans réconciliation du delta. Le test hostile utilise backup epoch40/fence7, host100/fence20 et ancien token : aucun +1 naïf, ordre entre UUID ou acceptation ancienne. Un host introuvable/inventaire local incomplet impose RestorationBlocked ; boot différent exclut seulement les anciens processus locaux. Perdre le delta externe gèle ses scopes, globalement si l'étendue est inconnue, jusqu'à décision explicite ; les coûts ne sont pas remis à zéro. Le test interrompt recovery avant/après import de capsule puis redémarre : observations répétables peuvent se répéter, effets ambigus non rejoués. Variantes R7 : restauration bloquée avant capsule, persistée dans `restore_requests` sans ligne `restore_imports` ; événement pending dans l'outbox du snapshot dont le consommateur a créé une action après le snapshot, avec scope gelé et aucune nouvelle action pour le même effet après bascule.

### Commandes persistées et rollback

T-F002-004/010 et T-F018-007 partagent les variantes suivantes, toutes **not_run** :

- Snapshot d'un Run prêt à révision 4 ; StartRun(K) engagé ensuite, effet externe observé, réponse perdue. Restore à révision 4 puis nouveau handshake/resync authentifié. Le client transmet son enveloppe persistée avec ancienne authority_generation : STALE_AUTHORITY_GENERATION avant lookup, zéro second dispatch malgré absence du receipt dans le snapshot. Répéter avec receipt historique présent.
- Remplacer le restore par un restart ordinaire : même lignée et même digest rendent le receipt original. Changer uniquement request_id ne change pas le digest ; changer lignée/type/payload/révision change le digest. Une commande v1 reçoit ProtocolIncompatible, sans upgrade automatique d'enveloppe ou de file. Omettre ou mal former authority_generation dans v2 échoue la validation, sans remplissage implicite par le handshake. Inspecter les receipts/événements historiques dans leur version d'origine sans les redispatcher.
- Après la fenêtre annoncée, purger uniquement la réponse full : le tombstone garde clé, lignée, digest et causalité. Retry identique donne IDEMPOTENCY_WINDOW_EXPIRED ; digest divergent donne IdempotencyConflict ; aucune seconde mutation. Diminuer la rétention ou avancer/reculer une horloge incertaine ne supprime pas prématurément la garantie engagée.
- Inspecter la file client après refus : génération et command_id d'origine inchangés, aucune copie automatiquement réémise. Une nouvelle intention exige disposition de récupération et nouveau command_id causalement lié. Perdre également la réponse RestoreBackup puis retrouver restore_id par InspectRecovery ; aucune seconde restauration automatique. Variante : RestoreBackup accepté en `read_only_recovery` après StorageCorrupt, base source non inscriptible ; receipt, restore_id et RestorationBlocked éventuel sont retrouvés dans le manifeste privé de maintenance par retry identique et InspectRecovery avant bascule, puis dans `restore_imports` après bascule.

La validation JSON Schema peut vérifier la présence/forme de la lignée ; elle ne peut prouver sa comparaison à l'autorité active, la conservation des files clientes, le contenu d'un tombstone ou l'absence réelle de second effet.

## Acceptation et paquet EV

Le paquet conserve commit produit/fixture, OS/kernel/filesystem, toolchain, SQLite source_id/options, paramètres de flush, cutpoint/seed, historique de commandes, DB/artefacts synthétiques, journal externe, rapport de reprise et résultat attendu/observé. Le rapport sépare confirmé, inconnu et bloqué. Les délais de reprise sont mesurés par classe ; une action sans mécanisme de réconciliation peut légitimement rester inconnue avec disposition guidée.

La campagne est acceptée lorsque les invariants transactionnels tiennent, toutes les commandes reconnues sont expliquées, les anciennes capacités sont refusées, aucun effet ambigu n'est automatiquement répété et les trois clients convergent. Une suite partiellement exécutée conserve ses cases not_run/blocked. L'ambition de fiabilité ne devient pas une promesse d'absence de bugs.
