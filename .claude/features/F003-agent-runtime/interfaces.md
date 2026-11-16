# Interfaces F003

Contrats Unleash proposés, transmis dans l'envelope F001. Les IDs pédagogiques ci-dessous illustrent les relations ; l'encodage de production est opaque et validé.

```json
{
  "invocation_id": "invocation-demo",
  "agent_id": "agent-n-demo",
  "session_id": "session-demo",
  "parent_invocation_id": "invocation-leader-demo",
  "trigger_event_id": "event-assignment-demo",
  "mission_id": "mission-demo",
  "run_id": "run-demo",
  "step_id": "step-demo",
  "task_id": "task-demo",
  "effective_bundle": {
    "role_version_id": "role-developer-v3",
    "skill_version_ids": ["skill-analysis-v2"],
    "contract_version_ids": ["contract-task-v1"],
    "workflow_version_id": "workflow-v1",
    "requested_model_profile_version_id": "model-profile-v4",
    "allowed_model_policy_ref": "model-policy-v4",
    "scope_id": "scope-demo",
    "scope_revision": 4,
    "document_pin_ids": ["pin-demo-v7"],
    "initial_context_manifest_id": "manifest-demo"
  },
  "state": "queued"
}
```

Le bundle complet inclut hashes des contenus, politique de capacités/providers/modèles autorisés, policy revision et budget maximal. Le router résout un ID fournisseur officiel à chaque appel puis persiste un ModelCallManifest distinct : model_call_id, action_id, origin_ref de variante invocation, parentmanifest/trigger_event, provider/modèle/capacités effectifs, context_ref et request_hash. Usage et résultat sont des receipts/manifests append-only liés. Un fallback autorisé conserve bundle_hash et crée un nouvel appel ; hors politique figée, il crée une nouvelle invocation liée.

| Commande | Effet attendu |
|---|---|
| `CreateInvocation` | Matérialiser intention et bundle depuis versions autorisées ; aucune exécution immédiate garantie. |
| `RequestCheckpoint` | Demande coopérative, reason et deadline ; une seule demande par contrôle en attente, étendue au candidat suivant après `checkpoint_escalation_s` ; aucun slot rendu avant événement de checkpoint. |
| `ResumeInvocation` | Réserver et créer une nouvelle tentative pour une invocation `waiting_durable`, ou `reconciling` dont l'ancien exécuteur est neutralisé, après préconditions ; jamais depuis `completed`, `failed` ou `cancelled`. |
| `CancelInvocation` | Révoquer continuation et classifier effets engagés. |
| `InspectInvocation` | Bundle, contextes, tentatives, actions, décisions, preuves et wait_reason. |
| `SetConcurrencyLimit` | Vérifier 1..50 et la cohérence de la paire limite/réserve, sinon InvalidArgument/CONTROL_RESERVE_INCOHERENT sans ajustement silencieux ; exposer requested_limit pending, effective_limit actuelle et admissions_frozen si baisse sous occupation (condition dérivée `draining_to_limit`, sans lien avec le mode daemon `draining`) ; activer atomiquement après drainage. |

Événements : `InvocationQueued`, `InvocationAdmitted`, `InvocationStarted`, `InvocationCheckpointRequested`, `InvocationCheckpointed`, `InvocationReconciling`, `InvocationCompleted`, `InvocationFailed`, `InvocationCancelled`, `InvocationRecoveryBlocked`. Les erreurs internes CapacityBlocked, BudgetBlocked, ProviderRateLimited, ContextIncomplete et ActionOutcomeUnknown sont mappées aux classes communes ResourceBusy, BudgetExceeded, RateLimited, ContractViolation et OutcomeUnknown avec reason_code ; StaleFence/RevisionConflict sont déjà communes. Le wire utilise code/reason_code/message/retry_class/details_ref/current_revision (clé absente si non applicable)/command_id du [contrat central](../../architecture/ipc-and-apis.md). Le résultat ne contient aucune chaîne de pensée privée ; il contient les raisons opérationnelles et preuves accessibles.

## Frontière interne / client R6

CreateInvocation, RequestCheckpoint, ResumeInvocation, CancelInvocation, InspectInvocation et SetConcurrencyLimit ci-dessus nomment les ports internes du runtime, non des verbes publics wire. Le client utilise AssignTask/StartRun, RequestInterruption, ResumeRecoveredInvocation selon contexte, GetInvocation et SetSettings ; le [catalogue public](../../architecture/client-operation-catalog.md) est l’autorité des noms exposés. Aucun client ou modèle n’obtient l’accès au scheduler interne par un nom JSON.

Le registre d’admission a trois propriétaires typés sous le même plafond : invocation agente, replay_worker et provider_probe. Le dernier sert à la configuration initiale sous [F012](../F012-model-api-router/interfaces.md), sans agent factice ni hiérarchie nouvelle. Les phases passives d’inspection ne démarrent aucune génération.
