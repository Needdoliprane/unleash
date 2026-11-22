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

`AssignTask` suit le [contrat d’affectation](../../architecture/client-operation-catalog.md#affecter-une-tâche). La tâche cible et sa révision se trouvent dans l’enveloppe. Le payload identifie l’agent. L’activation dépend du bundle résolu par le serveur et de l’engagement durable de la file.

Le port Store d’`AssignTask` reçoit l’autorité authentifiée du propriétaire humain et une préparation interne. Le writer compare l’époque et la révision de l’affectation active, la tâche, l’étape, le run et le bundle épinglé. Le reçu, `TaskAssigned`, `InvocationQueued`, les références d’objets et le passage en `queued` sont engagés dans la même transaction. La file expose l’invocation dans la classe `work`. L’admission et la réservation d’un slot relèvent d’une décision ultérieure du scheduler. La route IPC applique le garde de l’administration locale et de la gestion des missions. L’acteur provient de la session authentifiée. Le daemon appelle la résolution du bundle publié puis remet le reçu et les événements par son circuit commun. La résolution des entrées du bundle complet et l’admission effective restent à qualifier.

Pour une mission à propriétaire humain, la préparation interne de `CreateAgent` peut prendre appui sur une tâche activée et prête. Le writer revalide la révision de la mission et de la tâche, le propriétaire, le scope de la tâche et la version publiée du rôle. Il engage l'identité N éligible, la session primaire, l'affectation active (epoch et révision à 1), la référence du rôle et le reçu dans la même transaction. Le résultat référence l'affectation et l'événement `AgentCreated` relie l'agent, la session et l'affectation. Un rejeu identique relit le reçu. La file d'invocations et les réservations de capacité commencent à leurs opérations respectives.

Pour le parcours opérateur, `CreateAgent` reçoit le projet et le nouvel identifiant d’agent dans l’enveloppe à la révision de création zéro. Le payload fermé porte `mission_id`, `mission_revision`, `task_id` et `task_revision`, en utilisant une tâche `ready` du run actif. Le daemon résout la portée et le rôle à partir du nœud activé et du plan immuable du run. Ce parcours exige un rôle retenu de niveau N. Un niveau différent ou absent rend `InvalidArgument` avant la création de l’agent. Le writer revalide ce plan et ses références, le mandat, le rôle et les révisions avant d’engager l’identité, la session et l’affectation. Les versions dépréciées restent utilisables lorsqu’elles sont déjà épinglées par le run et toujours retenues. Le reçu et `GetAgent` permettent de retrouver l’affectation et la session après redémarrage.

Le raccord public couvre les missions à propriétaire humain et la portée du projet à sa révision autorisée. La sélection d’un nouveau modèle et la publication de son bundle d’exécution relèvent de l’affectation et de l’admission. La création par un leader conserve son contrat dédié.

Le harness publie son document résolu via le port interne du Store. Le document canonique, sa ligne `effective_bundles` et la référence de sélection sont engagés ensemble. L’unicité de la sélection est vérifiée pour la lignée, la révision de tâche, l’identité, l’époque et la révision de l’affectation, ainsi que l’identité et la révision de session. Le Store retrouve cette sélection depuis les références durables lors de la mise en file. Une nouvelle révision de session requiert une préparation qui lui est propre. L’ancienne sélection reste accessible dans son contexte historique. Un rejeu du même `AssignTask` rend le reçu avant de relire la sélection courante.


Pour une tâche produite par `StartRun`, l’affectation rattache `NodeActivated` à `RunStarted` dans le même reçu du run. Le flux, la révision, la définition et le plan retenu doivent concorder. Les activations du port interne `ActivateWorkflowTask` conservent leur propre reçu causal.

Le bundle de cette tâche d’entrée porte `requested_model_profile_version_id` et `requested_model_profile_digest`. Le Store vérifie la version configurée par F012 dans le projet ou dans la portée utilisateur de la même autorité. La mise en file retient l’objet de configuration avec le bundle et le contexte. Le choix du profil, la politique effective et les observations d’admissibilité restent du ressort des étapes qui les produisent. La configuration seule laisse la qualification du fournisseur à F012.

Pour produire ce bundle, le harness lit la [politique modèle versionnée](../F012-model-api-router/interfaces.md#politique-modèle-versionnée-du-rôle) du rôle épinglé et les références de politique des sources de contexte. Il vérifie leur portée, leur état et leurs digests avant publication. L’enveloppe `AssignTask` désigne l’agent et la tâche conformément à son contrat fermé.

`GetInvocation` reçoit le payload fermé `{invocation_id, after_attempt_ordinal?, limit?}`. L’ordinal de départ vaut zéro par défaut et la limite vaut 50, avec un maximum de 100. Le projet de l’enveloppe correspond exactement à celui de l’agent persistant. Une invocation globale exige une session autorisée sur les projets sans restriction. La [projection](../../schemas/ipc-query-response.schema.json) expose séparément agent, session engagée, invocation, état, motif d’attente et références de bundle et de contexte. L’ordinal de tentative commence à un. Le résultat présente la page dans cet ordre, avec l’état et les coordonnées d’autorité historiques de chaque tentative. `next_after_attempt_ordinal` reprend le dernier ordinal de la page lorsqu’il reste des tentatives, et vaut `null` à la fin. Chaque page et son watermark proviennent d’une seule transaction lectrice. Les contenus, actions et manifestes d’appel conservent leurs lectures dédiées.

Le registre d’admission a trois propriétaires typés sous le même plafond : invocation agente, replay_worker et provider_probe. Le dernier sert à la configuration initiale sous [F012](../F012-model-api-router/interfaces.md), sans agent factice ni hiérarchie nouvelle. Les phases passives d’inspection ne démarrent aucune génération.
