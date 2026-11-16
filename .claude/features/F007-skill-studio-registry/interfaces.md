# F007 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
Les opérations sont distinctes : `SearchSkills`, `CreateSkillDraft`, `ValidateSkill`, `PublishSkillVersion`, `SearchRoles`, `CreateRoleDraft`, `ValidateRole`, `PublishRoleVersion`, `BindRole`, `DeprecateSkillVersion`, `DeprecateRoleVersion`, `RevokeSkillVersion` et `RevokeRoleVersion`. Les mutations utilisent l'enveloppe commune, révision attendue et command_id idempotent. L'acteur réel vient du transport.

`SearchSkills` et `SearchRoles` sont des lectures : elles retournent `candidates`, `coverage`, `freshness` et `resolution_order` ; elles n'appellent pas `DiscoverResources` et n'écrivent ni requête durable ni reçu. Le `query_id` repris par `RecordDiscoveryDecision` est retourné uniquement par la commande `DiscoverResources` (resource_kinds `skill` ou `role`). Avant création, `DiscoverResources` puis `RecordDiscoveryDecision` produisent le `DiscoveryReceipt` ([F009](../F009-knowledge-engine/interfaces.md)). Un candidat possède identité pleinement qualifiée, version, digest, scope, permissions de lecture et raisons de classement. Une source inaccessible n'est pas présentée comme vide ; la couverture indique la limitation sans exposer la ressource cachée.

Exemple de payload de binding tardif :

```json
{
  "run_id": "run-feature-12",
  "slot_id": "implementation",
  "role_ref": {
    "id": "role.repository.developer",
    "version": 3,
    "scope": {"kind": "repository", "owner_id": "repo-ovpt-fixture"}
  },
  "discovery_receipt_id": "discovery-roles-12",
  "justification": "Version adaptée au format de sortie requis par la mission",
  "evidence_bundle_id": "evidence-role-fit-12"
}
```

Le serveur lit la décision dans le reçu référencé et vérifie que son `target_resource_id` (reuse) ou l'identité réservée (create) correspond à l'identité qualifiée de `role_ref` (`id` et `scope` {`kind`, `owner_id`}, soit le tuple `entry_kind=role`, `scope_kind`, `scope_owner_key`, `entry_id` d'[ADR-06](../../architecture/adr/ADR-06-skill-role-registry.md)), et que la version liée `role_ref.version` est celle que relie la `DiscoveryUseObservation` de cet usage. Un rôle homonyme d'une autre portée ne satisfait pas ce contrôle. Aucun champ de décision n'est réenregistré par BindRole ; `justification` porte seulement l'adéquation du rôle au slot.

Le retour expose binding_id, digest de rôle, digests des skills, contrat effectif, new_revision et event_ids. Le service refuse si le slot est déjà engagé ou si la nouvelle version élargit son plafond. L'appel ne change pas le workflow. Le nom du modèle effectif est enregistré lors de l'invocation, après résolution de la politique versionnée et négociation de capacités.

`GetVersionDiff` compare toutes les composantes du bundle, pas seulement le Markdown. `ListVersionUsages` liste runs, slots et invocations autorisés à l'acteur. `GetEvaluationReport` distingue validation structurelle, fixtures, mesure métier et jugement. `ExportBundle` respecte ACL, licence et redaction ; les secrets ne sont jamais embarqués dans le bundle.

Le protocole de publication retourne le digest et un rapport attaché. Les codes d'erreur sont structurés avec règle, champ, dépendance et remédiation possible. `MISSING_DEPENDENCY` ne déclenche aucun téléchargement automatique. `PERMISSION_EXPANSION_DENIED` ne peut être contourné par la création d'une version numériquement supérieure. La liste canonique des reason_code est dans le [design technique](technical-design.md). Les schemas du dossier sont des contrats internes proposés, pas un format de fournisseur déjà pris en charge.

Les sauvegardes de drafts et évaluations exposent SaveSkillDraft, SaveRoleDraft et RequestSkillEvaluation selon le [catalogue client commun R2](../../architecture/client-operation-catalog.md). Elles sont accessibles depuis le Studio GUI et TUI ; aucune opération locale de l’éditeur ne publie implicitement une version.

ExportBundle est une commande de préparation d’artefact, avec reçu commun et contrôle d’accès ; ReadArtifact lit le résultat autorisé. Son coût de préparation ne transforme pas une lecture de catalogue en effet implicite. Le [catalogue commun](../../architecture/client-operation-catalog.md) fixe ce classement.
