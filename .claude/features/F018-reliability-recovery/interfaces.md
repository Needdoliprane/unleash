# Interfaces F018

Les commandes utilisent l'envelope F001 et les permissions du broker. Les DTO sont des propositions internes.

| Opération | Précondition | Résultat |
|---|---|---|
| `InspectRecovery` | Lecture autorisée | Rapport, items classifiés, progression, blocages, preuves. |
| `ReconcileAction` | Action non terminale/unknown et stratégie disponible | Observation attribuée ou unknown conservé. |
| `ResolveActionOutcome` | Décideur habilité, expected_revision, preuves ou disposition explicite | Décision enregistrée ; aucune preuve inventée. |
| `ResumeRecoveredInvocation` | Invocation `waiting_durable` ou `reconciling`, ancien exécuteur neutralisé, contexte et budgets valides ; une invocation `completed`, `failed` ou `cancelled` n'est jamais reprise, sa suite est une nouvelle invocation liée | Nouvelle tentative admise sous le même invocation_id ou raison de blocage. |
| `RequestBackup` manuel | Administration globale authentifiée. Enveloppe v2 de révision 0, `aggregate_id` égal au BackupId UUID non nul, payload `{}`. Le daemon choisit `backups/<BackupId>` sous sa racine de données. | Le reçu `accepted` référence `backup:<BackupId>`. La réservation, son identifiant d’action et son événement sont engagés ensemble. Le writer vérifie la cause, la lignée et le dispatch avant de créer la barrière de copie. Le rejeu du même identifiant de commande dans sa lignée restitue le reçu. |
| `GetBackupManifest` | Administration globale authentifiée, payload fermé `{"backup_id":"<UUID>"}`. | Snapshot avec watermark, `backup_id`, `authority_generation`, `lineage_status` (`current` ou `historical`), `state`, `action_state` et `blocked_reason` éventuels. `manifest` et `manifest_digest` proviennent de la complétion SQLite. La projection distingue un job complet et une observation de dispatch encore inconnue. Les blocages pré-dispatch indiquent `guard_profile_unavailable`, `isolation_profile_unavailable` ou `authorization_denied`. |
| `ValidateBackup` | Chemin/manifest autorisés | Couverture des objets/effets/receipts/budgets, état complete, hashes, compatibilité et erreurs ; un job interrompu n'est pas un backup validé. |
| `RestoreBackup` | CommandEnvelope avec lignée d'origine, daemon `ready`, `degraded` ou `read_only_recovery` ; l'acceptation fait entrer ou maintient le daemon en `read_only_recovery`, mode de la procédure offline exclusive ; inventaire local complet et neutralisation, destination distincte, capsule du delta et de sa couverture | restore_id durable dans `restore_requests` de la lignée source, ou dans le manifeste privé de maintenance hors base si la base source est illisible ou non inscriptible ; nouvelle lignée/secrets/stream après import capsule/blocages, ou RestorationBlocked sans activation, persisté sans lignée cible ni capsule ; couverture externe partielle garde ses scopes gelés. |

Sur la connexion diagnostique ouverte après un préflight SQLite corrompu, les deux lectures renvoient un watermark nul. `GetDaemonStatus` expose le mode `read_only_recovery` et `storage_corrupt: true`. `InspectRecovery` exige l’administration globale et renvoie `{report, inventory_coverage: "unknown"}`. Le rapport utilise le manifeste privé vérifié et garde les identités actives indisponibles. Les listes vides dans ce rapport sont lues avec la couverture inconnue de l’inventaire.

```json
{
  "recovery_id": "recovery-demo",
  "authority_generation": "00000000-0000-4000-8000-000000000018",
  "daemon_epoch": 18,
  "state": "degraded",
  "items": [{
    "action_id": "action-demo",
    "classification": "unknown",
    "reason_code": "EFFECT_MAY_HAVE_PRECEDED_RECEIPT",
    "evidence_ids": [],
    "slot_disposition": "released_after_local_fencing",
    "cost_reservation_state": "unknown",
    "allowed_operations": ["ReconcileAction", "ResolveActionOutcome"]
  }]
}
```

Une disposition porte `decision_kind`, décideur résolu, justification, policy_revision, références de preuves et impact sur dépendances/budget. `accept_risk_retry` autorise une suite explicitement distincte ; il ne modifie pas l'historique en succès confirmé. Une absence d'autorisation retourne `CapabilityDenied/OPERATION_NOT_AUTHORIZED` (erreur interne Unauthorized) et ne débloque pas l'action. La classification d'un item suit le vocabulaire `not_dispatched`, `in_flight`, `confirmed_success`, `confirmed_failure_no_effect`, `conflict` ou `unknown` de la [machine d'état d'action](../../architecture/failure-recovery.md#machine-détat-daction) ; ce n'est pas un nom d'état d'action.

Toutes les mutations unleash.ipc.v2 portent l'authority_generation épinglée lors de leur création durable ; une enveloppe v1 est refusée avec ProtocolIncompatible, sans upgrade automatique de la file cliente. L'ancienne lignée reçoit StaleFence/STALE_AUTHORITY_GENERATION avant lookup du receipt, indépendamment du handshake, de ResyncRequired et de la révision restaurée. Le client garde sa commande en attente comme refusée, sans en changer automatiquement la lignée ou l'ID. Une inspection du receipt historique ou du restore_id reste une lecture autorisée, selon sa version/provenance d'origine. RestoreBackup lui-même produit d'abord un receipt et une ligne `restore_requests` (restore_id) sous la lignée source ; RestorationBlocked y est persisté sans lignée cible ni capsule, et aucune ligne `restore_imports` n'existe alors ([SQLite](../../architecture/sqlite-design.md#actions-processus-et-preuves)). Si la base source est illisible ou non inscriptible (cas principal d'un restore accepté en `read_only_recovery` après StorageCorrupt), receipt, restore_id et RestorationBlocked sont écrits dans le manifeste privé de maintenance hors base ([failure-recovery](../../architecture/failure-recovery.md#disque-plein-corruption-et-restauration)) ; la recherche idempotente et InspectRecovery le lisent jusqu'à la bascule, puis la capsule l'importe dans `restore_imports` de la base restaurée. La capsule préserve ce lien, puis InspectRecovery permet de retrouver son issue après bascule, sans resoumettre une nouvelle restauration.

Le rapport de récupération expose capsule_ref, coverage=complete/partial/unknown, scopes gelés, provenances d'actions absentes du snapshot, dispositions de coûts et pins/suppressions/backups en reprise. Une étendue d'effets inconnue impose un gel global des effets ; elle n'est pas réduite à une liste vide. Les erreurs de lignée et d'expiration d'un tombstone ont retry_class=never. Les clients peuvent présenter les opérations de résolution autorisées, sans recréer l'intention d'origine par défaut.

Événements : `DaemonRecoveryStarted`, `RecoveryItemClassified`, `ActionOutcomeUnknown`, `ActionReconciled`, `RecoveryDecisionRecorded`, `RecoveryCompleted`, `RecoveryBlocked`, `BackupValidated`, `RestoreCompleted`. ActionReconciled et RecoveryDecisionRecorded (agrégat action) sont écrits dans la même transaction que EffectOutcomeResolved sur chaque Run bloqué par cette action ([state-machines](../../architecture/state-machines.md)) ; ActionOutcomeUnknown s'accompagne de EffectUnknown sur le Run concerné.

Projection wire enregistrée par F018, au format commun de [F001](../F001-core-daemon/interfaces.md) et d'[ipc-and-apis](../../architecture/ipc-and-apis.md) (nom de domaine, classe/reason_code, retry_class) :

| Nom de domaine | Wire (classe/reason_code) | retry_class |
|---|---|---|
| RecoveryInProgress (correspondance de F001) | ResourceBusy/RECOVERY_IN_PROGRESS | after_backoff |
| ActionOutcomeUnknown | OutcomeUnknown/UNRESOLVED_EFFECT (raison transverse UnresolvedEffect d'ipc-and-apis) | after_reconciliation |
| IdentityUnverified | CapabilityDenied/IDENTITY_UNVERIFIED | after_reconciliation |
| RestorationBlocked | ResourceBusy/RESTORATION_BLOCKED | after_human_decision |
| RestoreIncomplete | ContractViolation/RESTORE_INCOMPLETE | after_human_decision |
| AccountingMismatch | ResourceBusy/ACCOUNTING_MISMATCH | after_human_decision |
| EvidenceUnavailable | ContractViolation/EVIDENCE_UNAVAILABLE | never |
| StorageCorrupt (enregistrée par F001) | InternalFailure/STORAGE_CORRUPT | never |

La cause détaillée d'un effet inconnu (par exemple EFFECT_MAY_HAVE_PRECEDED_RECEIPT) reste le reason_code de l'item de reprise, jamais un second reason_code de la classe OutcomeUnknown. Les champs sont code/reason_code/message/retry_class/details_ref/current_revision éventuelle/command_id. `RecoveryCompleted` signifie plan de reprise exécuté et blocages exposés, pas tous les effets externes conclus ; son payload contient unresolved_count et ready_scopes. Aucun UUID de lignée n'est comparé par ordre et aucun vieux host ne reçoit une capacité de la nouvelle lignée.
