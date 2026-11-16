# Interfaces F002

Les opérations ci-dessous sont des contrats internes Unleash proposés. Elles n'inventent pas des méthodes de rusqlite ; les API bibliothèques réellement consultées sont recensées dans [systems-sources.md](../../research/systems-sources.md).

| Opération de port | Entrée | Sortie et erreurs |
|---|---|---|
| `CommitCommand` | Acteur résolu, authority_generation d'origine, command_id, digest incluant lignée, expected_revision et décision | Contrôle de lignée avant lookup ; receipt stable ou StaleFence/STALE_AUTHORITY_GENERATION, ContractViolation/IDEMPOTENCY_WINDOW_EXPIRED, RevisionConflict/IdempotencyConflict/StorageBusy. |
| `ReadSnapshot` | Projection, filtres autorisés | Données et cursor lus au même snapshot. |
| `ReadEvents` | stream_id, after_sequence, limite | Événements ordonnés ou ResyncRequired. |
| `ApplyInboxEvent` | consumer_id, event_id, décision déterministe | Déjà traité ou mutation+inbox atomiques. |
| `ReserveResources` | Invocation/action, comptes et bornes | Réservations atomiques ou CapacityBlocked/BudgetBlocked. |
| `AcquireLease` | Ressource, propriétaire, expected_revision | lease_id/epoch/fence ou ResourceBusy. |
| `PrepareObject` | Hash/taille, propriétaire et mandat d'origine | Incarnation/pin durable ou ResourceBusy/OBJECT_DELETING ; aucune référence métier avant octets durables. |
| `PublishObjectReference` | Pin, génération, propriétaire et référence métier | Référence + registre + libération pin atomiques, ou refus de génération/état. |
| `PrepareObjectDeletion` | Hash/génération, motif et décision de purge si nécessaire | Intention durable + deleting si aucune référence/pin/barrière ne protège ; aucun unlink dans cette transaction. |
| `FinalizeObjectDeletion` | Intention enregistrée, observation du chemin exact et synchronisation qualifiée | Tombstone/completed atomiques, ou deleting/quarantined avec diagnostic. |
| `PrepareBackup` | Destination autorisée et politique d'objets/effets | backup_id, barrière durable avant copie DB puis pins exacts avant copie objets ; complete seulement après vérification et synchronisation du manifeste. |

Ces ports restent internes au writer. Ils ne créent pas des verbes wire concurrents : RequestBackup/PurgeHistory/RestoreBackup utilisent le catalogue commun. Les pins de lecture/préparation/sauvegarde et leur réconciliation suivent [storage-model.md](../../architecture/storage-model.md). La commande publique unleash.ipc.v2 conserve authority_generation dans sa file durable ; le transport ne la remplit pas à nouveau lors d'un retry et refuse les enveloppes v1 avec ProtocolIncompatible. Les erreurs de lignée et de fenêtre ont retry_class=never. Ce nouveau format de conception ne suppose aucune ABI produit existante ; les événements/receipts historiques restent dans leur version d'origine.

Exemple d'événement wire proposé conforme à [event.schema.json](../../schemas/event.schema.json). Les colonnes physiques `aggregate_type`, `event_ordinal` et la causalité normalisée restent internes au stockage ; elles ne sont pas ajoutées au wire. Le payload_hash doit être recalculé selon la canonicalisation retenue à chaque changement du payload.

```json
{
  "event_id": "00000000-0000-4000-8000-000000000017",
  "stream_id": "00000000-0000-4000-8000-000000000006",
  "sequence": 204,
  "aggregate_id": "00000000-0000-4000-8000-000000000020",
  "aggregate_revision": 3,
  "event_type": "InvocationAdmitted",
  "schema_version": 1,
  "occurred_at": "2026-11-16T12:00:00Z",
  "recorded_at": "2026-11-16T12:00:00Z",
  "project_id": "00000000-0000-4000-8000-000000000003",
  "actor_ref": "system:scheduler",
  "correlation_id": "00000000-0000-4000-8000-000000000004",
  "causation": {
    "kind": "command",
    "id": "00000000-0000-4000-8000-000000000007"
  },
  "command_id": "00000000-0000-4000-8000-000000000002",
  "payload_hash": "sha256:396829e2a6811b428d99e673f23edf4797fd06a2b78736afd2b92ba42fc77cc7",
  "payload": {
    "attempt_id": "00000000-0000-4000-8000-000000000021",
    "authority_generation": "00000000-0000-4000-8000-000000000018",
    "daemon_epoch": 12,
    "slot_reservation_id": "00000000-0000-4000-8000-000000000022"
  },
  "sensitivity": "project"
}
```

Les UUID sont des fixtures valides. Pour une cause command, causation.id désigne receipt_id global, ici distinct du command_id informatif ; il se résout vers la paire acteur authentifié/command_id. Le payload métier InvocationAdmitted illustre attempt_id, authority_generation, daemon_epoch et réservation ; son hash est calculé sur le JSON canonique à clés triées de cet exemple sans nombres flottants. Le contrat complet du payload reste à valider séparément de l'envelope. Les payloads sont versionnés, bornés et validés ; les secrets sont des références. Les erreurs wire reprennent `code`, `reason_code`, `message`, `retry_class`, `details_ref`, `current_revision` éventuelle et `command_id` du [contrat commun](../../architecture/ipc-and-apis.md). Les mutations ne sont accessibles aux autres crates que par le port du writer : aucun worker ni client UI n'ouvre une connexion d'écriture parallèle.
