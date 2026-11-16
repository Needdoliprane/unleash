# F008 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
`ValidateContractDefinition` vérifie le contrat et retourne diagnostics avant publication, dont la liste dérivée des `expectation_slot` requis. `EvaluateContract` prend contract_ref exact, phase, subject_ref, input/output refs, evidence_bundle_id et expected_subject_revision. Le serveur résout les `ExpectedExecutionBinding` à cette révision depuis son manifest, ainsi que la projection `context` des règles ([projection d'évaluation](../../specs/contract-enforcement.md)) ; un payload client ne remplace ni ce contexte ni le slot par un pointeur dans la sortie. `AuthorizeAction` reçoit action typée, ressources, acteur serveur, scope et epoch ; il retourne grant effectif limité ou refus. Les opérations d'évaluation ne permettent pas au client de fabriquer une observation authentifiée.

`RecordClaim`, `RecordObservation` et `RecordJudgment` sont des commandes du canal worker/harness authentifié (CommandEnvelope, capability de tentative, permissions séparées), inscrites au [registre des opérations](../../architecture/client-operation-catalog.md) et à sa [projection](../../catalog/operations.json) hors de tout groupe PAR ; GUI, TUI et CLI ne les appellent pas. Leur émetteur et son autorité sont résolus par ce canal : tentative du worker pour un claim, adaptateur authentifié pour une observation, décideur autorisé pour un jugement. `ValidateContractDefinition`, `EvaluateContract`, `AuthorizeAction` ([F017](../F017-ai-guard-integration/interfaces.md)), `ExplainVerdict` et `ListEvidence` nomment des ports internes du daemon et ne sont pas des `command_type` ; les clients GUI, CLI et TUI lisent verdicts et preuves par `ReadEvidence` et `GetReportSnapshot` du [catalogue commun](../../architecture/client-operation-catalog.md).

Exemple de règle proposée :

```json
{
  "id": "require-discovery",
  "validator": "discovery_receipt_present",
  "phase": "completion",
  "severity": "required",
  "on_unknown": "block",
  "parameters": {
    "resource_kinds": ["function", "test", "skill", "role"],
    "require_prior_event": true,
    "require_decision": true
  }
}
```

`RecordClaim` reçoit sujet `SubjectRef`, texte explicatif (`statement`), proposition et `observation_ids` candidats selon [contract-claim.v1](../../schemas/contract-claim.schema.json) ; `claim_id` et `author_ref` sont attribués par le serveur depuis le canal authentifié, pas acceptés comme identité déclarée. Les prédicats V1 sont `process.exit_code` et `test.expectation_satisfied`, version 1, ou `kind=qualitative` sans vérité calculable. Un nom/version inconnu ne déclenche aucun appel modèle ni exécution libre. `RecordObservation` est réservé aux adaptateurs dont l'identité et la provenance sont vérifiées ; les tests utilisent la [projection V1](../../schemas/test-execution-observation.schema.json). `RecordJudgment` reçoit décideur, décision, révision et bundle examiné. Les trois réponses ont des identifiants de types différents ; une chaîne d'ID ne peut être substituée sans validation.

`ExpectedTestExecution` suit [son schéma V1](../../schemas/expected-test-execution.schema.json). Sa référence id/version/digest et le binding à une action sont créés côté harness avant effet depuis une version autorisée de plan/contrat/mandat. Une proposition modèle n'a aucune autorité avant cette admission. Le slot est une dépendance obligatoire du contexte de tâche ; une absence bloque l'admission, sans fabriquer d'observation. Les [règles normatives](../../specs/contract-enforcement.md) fixent commandes, sujets/snapshots, sélection de tentative, résultats et erreurs.

Exemple de règle du catalogue v2, indépendante du contrôle de liens historique :

```json
{
  "id": "claim-truth",
  "validator": "claim_predicate_matches",
  "phase": "completion",
  "severity": "required",
  "on_unknown": "block",
  "parameters": {
    "claim_pointer": "/output/claim_id",
    "expectation_slot": "workspace-tests",
    "predicate_ref": {"id": "test.expectation_satisfied", "version": 1}
  }
}
```

Les reçus discovery et `DiscoveryUseObservation` suivent [F009/interfaces](../F009-knowledge-engine/interfaces.md). F008 charge origin/source_refs/validation_basis et l'usage historique vérifié ; il n'infère pas une invocation à partir d'une action manuelle et ne transforme pas une revalidation de clôture en trace préalable.

Un `ContractVerdict` retourne verdict global, résultats par règle, versions de validateurs, artefacts consultés, lacunes, contradictions et `evaluated_subject_revision`. Les résultats distinguent intégrité des liens, correspondance d'exécution et `claim_truth=pass/fail/unknown` ; un pass de liens ne devient pas un pass de vérité. Le payload d'un échec indique si nouvelle preuve, correction de sortie ou autorisation est nécessaire. Un verdict périmé ne peut pas être réutilisé sur une autre révision ; une simple copie de texte ne porte aucune autorité.

`ExplainVerdict` fournit une explication dérivée des règles et références autorisées, sans demander de raisonnement privé. `ListEvidence` filtre selon ACL et rétention. Les erreurs doivent conserver leur code stable dans GUI, CLI et TUI. L'enveloppe commune protège les mutations par révision et idempotence. La frontière AI-Guard relève d'[ADR-10](../../architecture/adr/ADR-10-ai-guard-boundary.md) (Q-04, R7) : `AuthorizeAction` passe par l'ActionBroker, qui consulte le GuardPort versionné quand AI-Guard est configuré ; le mapping réel reste à qualifier.
