# Interfaces F001

## Envelope proposé

Le flux Unix transporte une longueur uint32 big-endian puis un JSON UTF-8, plafond proposé 8 MiB. Le contrat wire est [ipc-and-apis.md](../../architecture/ipc-and-apis.md) et [ipc-command.schema.json](../../schemas/ipc-command.schema.json). Cet exemple utilise des UUID de fixture valides ; il est un DTO Unleash proposé, non un appel à une bibliothèque existante.

```json
{
  "protocol_version": "unleash.ipc.v2",
  "request_id": "00000000-0000-4000-8000-000000000001",
  "command_id": "00000000-0000-4000-8000-000000000002",
  "authority_generation": "00000000-0000-4000-8000-000000000100",
  "actor_ref": "client:local-user",
  "project_id": "00000000-0000-4000-8000-000000000003",
  "aggregate_id": "00000000-0000-4000-8000-000000000004",
  "expected_revision": 8,
  "command_type": "PauseMission",
  "payload": {
    "reason": "inspection utilisateur"
  }
}
```

`actor_ref` est confirmé ou remplacé par le serveur à partir de la connexion et de sa capacité ; il ne fait pas autorité fourni seul. `command_id` reste stable lors du retry, contrairement à `request_id`. L'accusé ci-dessous expose l'intention acceptée durablement ; seul l'état completed certifierait la fin de l'opération suivant son contrat.

```json
{
  "receipt_id": "00000000-0000-4000-8000-000000000007",
  "command_id": "00000000-0000-4000-8000-000000000002",
  "accepted_at": "2026-11-16T12:00:00Z",
  "aggregate_id": "00000000-0000-4000-8000-000000000004",
  "new_revision": 9,
  "operation_id": "00000000-0000-4000-8000-000000000005",
  "state": "accepted",
  "event_cursor": {"stream_id": "00000000-0000-4000-8000-000000000006", "sequence": 125}
}
```

## Opérations

`Hello`, handshake du [transport](../../architecture/ipc-and-apis.md#transport), négocie protocole et capacités ; `GetDaemonStatus` retourne modes/version/epoch ; `GetStateSnapshot` retourne projection d’état et watermark ; `SubscribeEvents` reprend un flux ; `GetCommandReceipt` résout un timeout dans la lignée active et signale explicitement un reçu d'une ancienne lignée ; `GetHealth` expose aussi les entrées `dead_letter` (consumer_id, event_id, erreur, tentatives, consumer_version, [ADR-02](../../architecture/adr/ADR-02-event-architecture.md)) ainsi que l'inventaire des runtimes navigateur préparés (version, SHA-256 de Chrome for Testing et de chromedriver, plateforme, état), en lecture seule, sans téléchargement ni probe implicite ([exécution navigateur](../../architecture/browser-execution.md#3-contenu-de-la-version-et-du-manifest-de-run)) ; `PauseMission`, `ResumeMission`, `CancelMission`, `StopDaemon` sont des mutations versionnées. Les commandes métier appartiennent aux mêmes services quel que soit le client. receipt_id est un UUID global attribué serveur et restitué identiquement au retry ; la déduplication reste indexée par acteur authentifié/command_id.

La réponse de `GetCommandReceipt` (propriétaire F002, forme canonique dans [ipc-and-apis](../../architecture/ipc-and-apis.md#receipt-et-erreurs)) est `{receipt, authority_generation, lineage_status, provenance_ref}`, avec `lineage_status` `current` ou `historical`. Un reçu `historical` ne vaut jamais réponse d'acceptation, résolution de timeout dans la lignée active ni autorisation de retry. `StopDaemon` vise l'agrégat daemon : `aggregate_id` est l'authority_generation active, UUID stable dans la lignée, et `expected_revision` est `daemon_state.revision` ([IPC](../../architecture/ipc-and-apis.md#commandenvelope)).

L'erreur wire possède `code`, `reason_code` (clé toujours présente, null si aucune raison), `message`, `retry_class`, `details_ref` (null si aucun détail), `current_revision` (clé absente si non applicable) et `command_id` (null si inconnu), suivant le contrat central. Le `code` prend l'une des dix-sept classes communes, dont `ResyncRequired`. `RevisionConflict` inclut current_revision ; `ResyncRequired` indique de refaire un snapshot ; `IdempotencyConflict` ne permet pas de remplacer le payload historique. Un receipt de stop n'est pas un certificat d'arrêt final : les événements `DaemonStopRequested` et `DaemonStopped` complètent la réponse ; `DaemonStopped` porte `stop_outcome` (`complete`, `partial` ou `unknown`) et la liste des services conservés ou inconnus ([runtime-lifecycle](../../architecture/runtime-lifecycle.md)). Ce résultat d'arrêt n'est pas un mode du daemon.

Les noms techniques de panne employés dans les autres fichiers sont des erreurs internes ; leur sérialisation utilise une classe commune et un reason_code, pas un second format. Backpressure n'est pas une classe wire. Correspondance des erreurs F001 et stockage ([ADR-01](../../architecture/adr/ADR-01-sqlite-persistence.md), [SQLite](../../architecture/sqlite-design.md)) : Backpressure → ResourceBusy/BACKPRESSURE ; StorageBusy → ResourceBusy/STORAGE_BUSY ; StorageLowSpace → ResourceBusy/STORAGE_LOW_SPACE (after_human_decision) ; StorageFull → InternalFailure/STORAGE_FULL (after_human_decision) ; StorageUnavailable → InternalFailure/STORAGE_UNAVAILABLE ; StorageCorrupt → InternalFailure/STORAGE_CORRUPT (never) ; InvariantViolation → InternalFailure/INVARIANT_VIOLATION (never) ; build SQLite non qualifié → InternalFailure/STORAGE_BUILD_UNQUALIFIED ; RecoveryInProgress → ResourceBusy/RECOVERY_IN_PROGRESS (after_backoff) ; AuthorityAlreadyActive → ResourceBusy/AUTHORITY_ALREADY_ACTIVE ; DaemonAlreadyRunning → ResourceBusy/DAEMON_ALREADY_RUNNING lorsqu'elle est exposée à un client. Unauthorized se sérialise, conformément à [security-boundaries](../../architecture/security-boundaries.md), en AuthenticationFailed (pair ou credential invalide), CapabilityDenied/OPERATION_NOT_AUTHORIZED (capacité absente ou expirée) ou ScopeDenied (projet ou ressource hors scope). Les erreurs de reprise F018 (ActionOutcomeUnknown, IdentityUnverified, RestorationBlocked, RestoreIncomplete, AccountingMismatch, EvidenceUnavailable) sont enregistrées, avec leur retry_class, par la projection wire de [F018](../F018-reliability-recovery/interfaces.md), dans ce même format ; elles ne sont pas recopiées ici. Le détail sensible est une référence protégée, jamais un objet libre exposé sans contrôle.

Les logs et événements excluent tokens, clés et texte sensible inutile. Les permissions de socket et credentials sont décrits dans [le lifecycle](../../architecture/runtime-lifecycle.md). La compatibilité protocole doit refuser explicitement les champs/types incompatibles avant toute mutation, tout en permettant l'évolution additive négociée.

L’enveloppe mutante v2 conserve authority_generation de sa création durable dans toute retransmission. Une reconnexion ne réécrit jamais cette valeur ; après restauration, une ancienne commande est refusée avant idempotence. L’UI rend le conflit de lignée visible et demande une réconciliation distincte, pas un changement automatique de command_id. Le reçu courant conserve son schéma distinct sans champ protocol_version supplémentaire.
