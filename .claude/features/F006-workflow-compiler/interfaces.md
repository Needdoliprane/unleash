# F006 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
`DraftWorkflowFromIntent` reçoit texte, références de contexte, contraintes et base_version éventuelle. Il retourne draft_id, révision, contenu structuré, hypothèses et questions non résolues. Le contexte est un manifest inspectable, sans raisonnement privé du modèle. Une demande vide ou hors scope est refusée avant appel fournisseur.

### Brouillon manuel : texte et lecture

Le parcours manuel de `CreateWorkflowDraft` exige le projet dans l’enveloppe, l’UUID du brouillon dans `aggregate_id` et `expected_revision = 0`. Son payload est fermé pour la variante texte : `{"source":{"kind":"text","text":"…"}}`. Le texte peut être vide, incomplet, comporter des clés dupliquées ou être sémantiquement invalide. Après décodage de la chaîne JSON en Unicode, le serveur conserve ses octets UTF-8 exacts, sans analyse, normalisation ni appel modèle implicite. Les chaînes qui ne peuvent être encodées en UTF-8 sont refusées. Le maximum est de 1 048 576 octets UTF-8, contrôlé par le daemon après décodage ; `maxLength` du schéma compte les caractères et ne suffit pas à appliquer cette borne. L'échappement JSON maximal de ce texte reste sous la limite de trame de 8 Mio avec l'enveloppe. Un texte excessif est refusé `InvalidArgument` avant création.

Le brouillon est propre au projet et non exécutable. Son texte, son empreinte SHA-256, sa révision et le reçu commun sont engagés selon les garanties IPC et de stockage. La lecture `GetWorkflowDraft` exige le même projet et un payload fermé `{"draft_id":"<uuid>"}`. Pour un brouillon manuel inchangé, elle retourne `project_id`, `draft_id`, `revision`, `text` et `text_hash` au format `sha256:<hex>`, ainsi que le watermark de la transaction de lecture. Le texte et son hash correspondent à la même révision ; le demandeur doit disposer de l’autorité de lecture du projet. Le schéma de réponse de cette tranche est `workflow_text_draft_response`.

Dans un projet que l’acteur peut lire, un brouillon absent de ce projet donne `NotFoundOrHidden`, y compris si son identifiant existe ailleurs. La lecture d’un projet hors de la portée de l’acteur donne `ScopeDenied`.

La seconde variante de `CreateWorkflowDraft` transmet dans `source` la référence exacte d’une version publiée, sous la forme fermée `{kind: workflow_version, id, version, digest}` de `CreateRun`, sans suivre d’alias ni transporter ses octets dans la commande. Le demandeur doit pouvoir lire la source et créer le brouillon dans le projet cible. Les règles d’éligibilité, la filiation précise et le raccord de lecture par blocs pour une source volumineuse restent à préciser ; la borne du texte manuel ne s’applique pas implicitement à cette variante. Le catalogue prévoit `ReadArtifact` pour une référence autorisée et des blocs bornés ; son association au brouillon issu d’une version reste à préciser.

Fixtures de la tranche : [création depuis texte](../../examples/ipc-create-workflow-text-draft.json), [création depuis version](../../examples/ipc-create-workflow-version-draft.json), [lecture](../../examples/ipc-get-workflow-draft-query.json) et [réponse](../../examples/ipc-get-workflow-text-draft-response.json). Les contrôles de schéma ne démontrent pas la conservation réelle des octets, la borne UTF-8 ou l’autorisation ; ces propriétés requièrent des tests produit.

## Rapport de validation

`ValidateWorkflowDraft` exige le projet dans l’enveloppe, un nouvel UUID de rapport dans `aggregate_id` et `expected_revision = 0`. Le [payload fermé](../../schemas/ipc-command.schema.json) porte `draft_id`, `draft_revision` et `catalogue_snapshot_ref`. La révision du brouillon est positive et distincte de celle du rapport. La clé `catalogue_snapshot_ref` est obligatoire. Avec `null`, le daemon fige le catalogue local applicable et autorisé pour le projet avant de compiler. Avec une référence fermée `{schema_version: 1, digest}`, il résout exactement ce catalogue et contrôle son éligibilité dans le projet, sans utiliser un autre catalogue en cas de refus.

Le digest SHA-256 du catalogue identifie la représentation RFC 8785 du manifeste figé par le daemon. Le manifeste lie les profils disponibles et les entrées résolues du [registre](../../architecture/skills-contract-model.md) et du [catalogue d’opérations](../../architecture/automation-execution-profile.md) avec leurs versions, portées et digests. Il épingle les schémas et les évaluateurs requis par le compilateur. Le daemon conserve ce manifeste et les références nécessaires à la relecture du rapport. Le client ne fournit aucun contenu de catalogue. La compilation mécanique est locale et déterministe, avec le même texte, le même catalogue et la même version du compilateur. Elle n’exécute aucune opération du workflow et n’appelle aucun modèle.

Le daemon compile à partir des octets exacts du brouillon. Lors de l’engagement du rapport, le writer vérifie à nouveau le projet, la révision et le digest de cette entrée. Un brouillon modifié avant l’engagement donne `RevisionConflict`. Le rapport engagé est immuable, reste lisible après une correction du brouillon et porte le digest des octets effectivement compilés. Le reçu commun `completed` porte la référence `workflow-validation:<validation_report_id>` pour un rapport `valid` ou `invalid`. Le rejeu exact d’une commande engagée rend le reçu existant sans refixer de catalogue. Une erreur d’autorité, de coordonnées ou de stockage empêche l’engagement et suit l’enveloppe IPC commune.

`GetWorkflowValidation` reçoit `{validation_report_id}` et le projet exact dans l’enveloppe. Le [résultat fermé](../../schemas/ipc-query-response.schema.json) retourne le rapport et le watermark de lecture. Les mêmes règles `NotFoundOrHidden` et `ScopeDenied` que pour le brouillon s’appliquent. Le rapport expose sa version de schéma, son identité, l’identité, la révision et le digest du brouillon, le catalogue exact, la version du compilateur, le profil, le statut, les diagnostics, les bornes et les capacités nécessaires. Un rapport invalide contient au moins un diagnostic d’erreur. Le profil, les bornes ou les capacités peuvent valoir `null` si la passe correspondante ne les établit pas. Un rapport valide établit ces trois éléments, expose le digest des octets normalisés et n’a aucun diagnostic d’erreur.

Les bornes distinguent `activation_upper_bound`, calculée selon le [DSL](../../architecture/workflow-dsl.md), et `max_step_activations`, budget du run. `max_agent_executions` et `max_concurrent_action_executions` sont des sous-plafonds de simultanéité séparés. Le profil v1 a un plafond d’actions de 0. Les capacités nécessaires sont des identifiants de permissions applicatives, présentés dans un ordre stable. Elles ne confèrent aucune autorisation d’exécution.

Les diagnostics sont ordonnés de façon déterministe. Pour respecter la taille maximale de trame IPC, le compilateur borne la collecte et le rendu des diagnostics, conserve au moins une erreur pour un rapport invalide et indique une liste incomplète avec `diagnostics_truncated: true`. Un pointeur JSON vide désigne le document entier.

Un diagnostic de lecture interbranche peut prendre cette forme.

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

Les fixtures de [commande](../../examples/ipc-validate-workflow-draft.json), de [lecture](../../examples/ipc-get-workflow-validation-query.json) et de réponse [valide](../../examples/ipc-get-workflow-validation-valid-response.json) et [invalide](../../examples/ipc-get-workflow-validation-invalid-response.json) couvrent la forme documentée. Le catalogue et le rapport des fixtures sont synthétiques. `NormalizeWorkflowDraft` retourne le diff explicite. `SimulateWorkflow` exige un rapport valide, un jeu d’événements fictifs et une borne d’exploration.

## Publication d’une version

`PublishWorkflowVersion` cible le brouillon par `aggregate_id` et `expected_revision`. Son [payload fermé](../../schemas/ipc-command.schema.json) porte `draft_id` et `draft_revision` pour ce même brouillon, l’UUID `validation_report_id`, `normalized_digest` au format `sha256:<hex>`, l’identifiant logique `workflow_id` et `expected_previous_version`. Ce dernier vaut 0 pour une première publication ou le dernier numéro attribué attendu, même si la version correspondante est dépréciée ou révoquée. La [commande de fixture](../../examples/ipc-publish-workflow-version.json) référence la [définition merge-only](../../examples/merge-only-workflow.json).

Le serveur résout le rapport valide dans le même projet et vérifie sa liaison au brouillon, à sa révision et à ses octets. Le digest de la commande doit être celui du rapport durable. Le writer vérifie que les dépendances épinglées restent éligibles et que les octets normalisés correspondent au digest. Le document normalisé doit porter le même `workflow_id` que la commande et `version = expected_previous_version + 1`. Le serveur refuse une divergence ou un numéro hors de la borne de `CreateRun` opérationnel avant toute publication. Le writer compare le dernier numéro et attribue le suivant dans la même transaction. Il publie exactement les octets validés et leur digest.

La référence de résultat du reçu permet de relire la version et son digest. La plage d’événements du reçu couvre la publication. La nouvelle version est immuable et les runs et bindings déjà engagés conservent leurs références.

Les erreurs de publication sont distinguées des diagnostics de compilation. Un client reçoit `PublicationConflict` s'il publie depuis une base ancienne ; il peut comparer et créer un nouveau draft. Le même command_id avec payload identique retourne le même reçu ; un payload différent produit IdempotencyConflict. Les API sont proposées pour le backend partagé, sans présumer un SDK fournisseur ni un protocole AI-Guard existant.

## V1.1 et procédures

Le rapport de validation indique le profil v1/v1.1/v1.2 établi et le catalogue épinglé. Les diagnostics de définition comprennent `UnsupportedWorkflowProfile`, `ModelForbidden`, `OperationVersionUnavailable`, `RetryClassMismatch` et `InputBindingUnavailable`. Un refus de capacité portant sur l’admission d’une demande d’appel modèle utilise l’erreur IPC `CapabilityDenied/MODEL_CALLS_FORBIDDEN`, comme F019. Les opérations de draft et d’évaluation de F019 réutilisent le même service de compilation.
