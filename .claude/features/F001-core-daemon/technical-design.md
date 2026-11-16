# Conception technique F001

## Composition Rust

`unleash-daemon` initialise les adaptateurs OS, le writer, le harness, le broker et le scheduler puis expose `unleash-protocol`. Le domaine décide à partir de données, sans appeler le réseau ni lire l'heure directement. Les identités et horloges sont injectées. Le détail des crates est dans [rust-workspace.md](../../architecture/rust-workspace.md), qui fait autorité pour leurs noms ; ce sont des crates internes, pas des API de bibliothèques existantes. Le binaire du daemon est `unleashd` ; CLI, TUI et GUI l'atteignent par `unleash-client`.

Le writer long vivant est un thread propriétaire. Les I/O réseau sont asynchrones ; les traitements bloquants ont une capacité séparée et un arrêt défini. Aucun calcul volumineux ou accès fournisseur ne s'exécute pendant la transaction. Le daemon n'attend pas un jugement LLM pour authentifier un client, imposer le plafond d'admission ou classer une corruption.

## Démarrage déterministe

Valider le répertoire de rendez-vous canonique (propriétaire = UID effectif, mode 0700, absence de lien symbolique), acquérir le verrou de rendez-vous canonique de l’utilisateur OS, indépendant de data_root/profil et des surcharges client (dont `XDG_RUNTIME_DIR`), puis valider racines de données et permissions et vérifier la base et le binaire natif (`sqlite_source_id()` égal à la valeur compilée, sinon InternalFailure/STORAGE_BUILD_UNQUALIFIED). Une autre racine ne crée pas une deuxième autorité ; changement offline après neutralisation des anciens exécuteurs, selon runtime-lifecycle. Une instance déjà active est reconnue par handshake ; un PID dans un fichier n'est pas suffisant. Le nouvel epoch et `DaemonRecoveryStarted` sont committés avant reprise. Le socket peut servir `GetDaemonStatus` pendant cette phase mais aucune invocation nouvelle n'est admise.

Les modes persistés sont exactement starting, recovering, ready, degraded, read_only_recovery, draining, stopped. `already_running`, `AuthorityAlreadyActive` et `failed_start` sont des issues du démarrage ; le résultat d'arrêt complete/partial/unknown n'est pas un mode ; `read_only_recovery` porte l'exclusion de maintenance du restore offline. Les transitions et conditions se trouvent dans [runtime-lifecycle.md](../../architecture/runtime-lifecycle.md). `DaemonReady` est publié seulement après classification des réservations et ressources. Une mission ambiguë reste bloquée même si le daemon peut servir d'autres projets indépendants.

## Chemin de commande

Authentifier le pair, décoder longueur et JSON sous limites et valider le schéma, négocier la version, résoudre `actor_ref`, calculer le digest canonique incluant authority_generation, puis soumettre au writer. La transaction vérifie d'abord la lignée active (sinon StaleFence/STALE_AUTHORITY_GENERATION, avant tout lookup), recherche ensuite le reçu ou tombstone par acteur+command_id, puis seulement vérifie autorité et expected_revision, réduit et écrit état/events/outbox/réservations/receipt et commit, conformément à [ipc-and-apis.md](../../architecture/ipc-and-apis.md). Les contrôles sensibles sont revalidés au dispatch. L'accusé est retourné ensuite. Une perte de réponse se résout par la même identité de commande ; elle ne déclenche pas une seconde mutation.

Les abonnements ne bloquent jamais le writer sur un client lent. Une file d'envoi bornée coupe le flux avec un curseur de reprise ; les événements durables restent en base selon rétention. Snapshot et watermark sont lus dans la même transaction courte. Une restauration renouvelle stream_id afin qu'un vieux curseur ne masque pas un retour à un snapshot antérieur.

## Cycle utilisateur et erreurs

Fermer la GUI détruit seulement sa connexion. L'arrêt explicite déclenche `DaemonStopRequested`, refuse de nouvelles admissions, demande checkpoints, traite les hosts selon leur contrat et expose l'état final. Une deadline d'arrêt produit un résultat partiel si un effet reste engagé. Le daemon ne transforme pas un `OutcomeUnknown` en échec sans effet.

`ProtocolIncompatible`, `Unauthorized`, `RevisionConflict`, `IdempotencyConflict`, `Backpressure`, `StorageUnavailable`, `RecoveryInProgress`, `DaemonAlreadyRunning` (même racine, quand le handshake de rattachement n'est pas demandé) et `AuthorityAlreadyActive` (autre racine) sont des erreurs internes distinctes ; sur le wire, elles suivent la correspondance classe commune/reason_code d'[interfaces](interfaces.md) (Backpressure → ResourceBusy/BACKPRESSURE). Les détails diagnostics ne révèlent pas de secrets. Un échec de journalisation interdit l'accusé durable ; les erreurs répétées peuvent faire passer en mode lecture/récupération. Le service utilisateur doit être testé sur macOS et Linux, notamment les politiques de restart et le sort des hosts, sans extrapoler à partir d'un test foreground.

## Service et mise à jour

Le service utilisateur suit [runtime-lifecycle.md](../../architecture/runtime-lifecycle.md) : agent `SMAppService` du bundle ou LaunchAgent `gui/<uid>` de l'archive sous macOS, unité `systemd --user` sous Linux, sans root. Les mises à jour signées sont appliquées hors protocole daemon par l'updater de l'application ou du paquet CLI, sur commande de l'utilisateur (Q-16, [ADR-14](../../architecture/adr/ADR-14-local-data-oss.md)) ; F001 n'expose aucune opération de mise à jour. Le daemon n'en observe qu'une sauvegarde et un arrêt demandés par `RequestBackup` et `StopDaemon`, puis un démarrage dont la migration éventuelle est la phase exclusive de `recovering`.
