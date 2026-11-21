# F005 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
Toutes les mutations utilisent l'enveloppe commune avec `command_id`, `expected_revision` et acteur résolu serveur. Le payload de création de run est illustratif :

```json
{
  "mission_id": "mission-feature-12",
  "source": {"kind": "workflow_version", "id": "workflow.feature", "version": 1, "digest": "sha256:<hex>"},
  "purpose": "operational",
  "predecessor_run_id": null,
  "selection_reason": "Version publiée sélectionnée par le plan de mission",
  "input_ref": "artifact-input-12",
  "scope_ref": "scope-feature-12",
  "budget_ref": "budget-mission-12"
}
```

Pour `kind=automation_version`, `source` porte la référence exacte `id`/`version`/`digest` de la version d'automatisation, `data_revision_ref` et `automation_mode` ; paramètres et limites restent dans le payload ([IPC](../../architecture/ipc-and-apis.md)).

`CreateRun` accepte source.kind workflow_version ou automation_version, avec référence exacte et données lorsque applicables, puis retourne run, digest de définition, révision initiale, bindings résolus et slots non liés. Un `predecessor_run_id` désigne un successeur unique (Q-03) : CreateRun est refusé `ResourceBusy/PREDECESSOR_NOT_TERMINAL` (retry_class=after_refresh) tant que le prédécesseur n'est pas terminal, et `InvalidArgument/SUCCESSOR_EXISTS` si ce prédécesseur a déjà un successeur direct. `StartRun` vérifie préconditions et réserve les premiers budgets. Pour un Run `automation_version`, `StartRun` porte le digest de l'`EffectPreview` lu par `GetRun` sur le Run `prepared`, ou le `preview_digest` de `GetSetupPreview` pour une préparation guidée ; un changement de version, de données, de mandat ou de fait référencé invalide ce digest et `StartRun` est refusé `ContractViolation/STALE_EVIDENCE` ([aperçu](../../specs/reusable-automations.md#aperçu-autorisation-et-effets)). `PauseRun`, `ResumeRun` et `CancelRun` produisent un reçu avec état, intention et actions encore en réconciliation. `BindRole` exige slot vierge, référence exacte, discovery receipt et justification ; le retour inclut digests et événements RoleBound/SkillBound.

Le port interne de préparation d’un run opérationnel utilise une version de workflow publiée et un manifeste de scopes retenu par la mission à sa révision exacte. Le document de scopes V1 porte `schema_version`, `mission_id`, `mission_revision` et `scopes`. Chaque clé de `scopes` donne `scope_id` et `scope_revision`. La référence d’objet de la mission porte le champ `scope_manifest:<mission_revision>`. Le writer revalide cette révision et la référence avant le commit.

Le plan effectif V1 du run reprend la version et le digest de définition, l’identité et la révision de la mission, le digest du document de scopes, les scopes résolus et les slots. Un slot fixe retient la version et le digest du rôle, de son skill d’entrée, des skills et des profils de contrat déclarés. Le résolveur couvre les entrées publiées de portée globale ou projet. Un slot tardif porte `state=unbound`, son niveau, son plafond et sa référence de profil. Le run `prepared`, le plan et les références des sources, `RunCreated` et le reçu s’engagent ensemble. L’activation d’un run préparé par ce port vérifie le manifeste exact, le binding du nœud, son scope et la version du contrat. L’interpréteur de progression, le raccord IPC et le traitement de `BindRole` restent des étapes distinctes.

`SubmitObservation` est réservé aux adaptateurs authentifiés. Il reçoit action, activation, principal d’exécution agent ou replay_worker, invocation optionnelle selon ce principal, epoch, type d'observation, références de contenu et état de résultat. Un worker ne peut se déclarer adaptateur Git pour fabriquer une observation de merge. `GetRun` retourne définition effective, état, activations, compteurs, bindings et blocages. `SubscribeEvents` avec filtre run doit reprendre depuis stream_id et after_sequence durables sans imposer une resynchronisation par conversation.

Les événements comprennent RunCreated, RunStarted, NodeActivated, EffectRequested, ObservationRecorded, ContractEvaluated, BranchStopRequested, JoinClosed, RunBlocked et RunTerminated. La version d'un payload est portée par son `event_type` : un changement incompatible crée un nouveau type d'événement ([event-model](../../architecture/event-model.md#enveloppe-dévénement)) ; `schema_version` versionne l'enveloppe. Le reducer déduplique les événements entrants ; des noms identiques ne suffisent pas, une identité stable est requise.

Une erreur contient code, run, activation éventuelle, révision courante et action recommandée. `UnresolvedEffect` retourne les références nécessaires à la réconciliation, sans autoriser la répétition. `BindingAlreadyEngaged` retourne la version effective et l'identifiant d'engagement. Les interfaces sont internes proposées ; aucune de ces méthodes n'est annoncée comme API existante d'un fournisseur.

### Persistance d'une activation de tâche

`ActivateWorkflowTask` est un port interne du Store. Le reducer
fournit les coordonnées de l'activation logique, le contrat et le
scope résolus ainsi que les digests de la définition et du plan
effectif. L'enveloppe cible la révision exacte du run. La disponibilité
de l'étape, le binding et le budget relèvent de la décision du reducer.

Le Store vérifie que le run actif référence les mêmes digests et que
la définition est retenue par la version épinglée. Il vérifie les octets
et le nœud `task` hors transaction, avec sa borne de tentatives. Le
writer revalide la lignée, l'époque, le run, la portée et la référence
active. Il engage dans la même transaction l'étape et la tâche
initiales `ready`, la révision suivante du run, le reçu et
`NodeActivated`. La tâche commence en révision 1.

Pour ce nœud, l'événement appartient à l'agrégat du run. Il porte
`run_id`, `workflow_version_id`, `definition_hash`,
`definition_node_id`, `step_id`, `task_id`, `iteration_vector`,
`step_attempt` et `state`. Le vecteur vient de l'itération
persistante ou vaut `[]` hors boucle. Le reçu référence `task:<task_id>`.
Le rejeu exact relit le reçu. Une autre commande sur la même activation
logique ne duplique pas la tâche. Affectation et admission suivent leurs
propres transitions.

### Snapshot de qualification

Une évaluation F019 utilise le Run commun de source workflow_version avec purpose=evaluation, candidat/digest et cible autorisée. Le snapshot immuable est marqué qualification_only dans les métadonnées de version ; il n’est pas une version publiée sélectionnable en exécution opérationnelle. Le broker vérifie cette visibilité et l’autorité de qualification avant CreateRun/StartRun. Les colonnes purpose/candidate_ref du Run et visibility du snapshot relient les preuves sans modifier les octets exécutés. Le contrat détaillé est dans [F019](../F019-reusable-automations/technical-design.md).
