# F006 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
`DraftWorkflowFromIntent` reçoit texte, références de contexte, contraintes et base_version éventuelle. Il retourne draft_id, révision, contenu structuré, hypothèses et questions non résolues. Le contexte est un manifest inspectable, sans raisonnement privé du modèle. Une demande vide ou hors scope est refusée avant appel fournisseur.

`ValidateWorkflowDraft` prend draft_id, expected_revision et catalogue_snapshot_ref. Le rapport contient compiler_version, schema_version, diagnostics, bornes calculées, besoins de permission et normalized_digest éventuel. `NormalizeWorkflowDraft` retourne le diff explicite. `SimulateWorkflow` exige un rapport valide, un jeu d'événements fictifs et une borne d'exploration.

Exemple de diagnostic interne :

```json
{
  "code": "CrossBranchRead",
  "severity": "error",
  "pointer": "/body/branches/1/body/until/path",
  "message": "Le guard lit le résultat d'une autre branche avant jointure",
  "related_node_ids": ["branch-a", "branch-b"],
  "source_span_ref": "intent-span-4",
  "suggestion": "Déplacer la décision après le bloc parallel ou utiliser une entrée immuable"
}
```

`PublishWorkflowVersion` utilise l'enveloppe commune et contient draft_id, draft_revision, validation_report_id, normalized_digest, workflow_id et expected_previous_version. Le reçu retourne version, digest et événement. Le service vérifie que le rapport correspond toujours aux mêmes octets et aux dépendances applicables. Il ne démarre pas de run et ne remplace pas les bindings courants.

Les erreurs de publication sont distinguées des diagnostics de compilation. Un client reçoit `PublicationConflict` s'il publie depuis une base ancienne ; il peut comparer et créer un nouveau draft. Le même command_id avec payload identique retourne le même reçu ; un payload différent produit IdempotencyConflict. Les API sont proposées pour le backend partagé, sans présumer un SDK fournisseur ni un protocole AI-Guard existant.

## V1.1 et procédures

Le rapport de validation indique explicitement le profil v1/v1.1/v1.2 et le catalogue d’opérations utilisé. Les diagnostics supplémentaires comprennent UnsupportedWorkflowProfile, ModelForbidden (wire `CapabilityDenied/MODEL_CALLS_FORBIDDEN`, même raison que F019), OperationVersionUnavailable, RetryClassMismatch et InputBindingUnavailable, dans l’enveloppe Error commune. F019 présente ses opérations produit de draft/évaluation, puis réutilise ce service de compilation ; aucune API modèle n’est appelée pour valider mécaniquement une définition.
