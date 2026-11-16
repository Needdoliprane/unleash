# F010 — Interfaces proposées

Les payloads suivants sont imbriqués dans la CommandEnvelope v2 commune de l'[IPC](../../architecture/ipc-and-apis.md), `authority_generation` comprise. Le serveur résout l'acteur depuis la session locale et vérifie ses capacités ; le payload ne permet pas d'emprunter l'identité d'un intégrateur. La clé d'idempotence est (acteur authentifié, command_id), couplée au digest canonique défini par l'IPC v2. Un nouvel essai après déplacement de cible utilise un nouveau command_id avec lien vers l'essai précédent.

## Préparer

```json
{
  "command_type": "PrepareIntegration",
  "payload": {
    "repo_id": "repo-01",
    "mission_id": "mission-accounts",
    "source_oid": {"format": "sha1", "hex": "1111111111111111111111111111111111111111"},
    "target_ref": "refs/heads/integration",
    "expected_target_oid": {"format": "sha1", "hex": "2222222222222222222222222222222222222222"},
    "strategy": "merge_commit_or_ff",
    "workflow_run_id": "run-01",
    "policy_version": "git-policy-3",
    "required_validation_ids": []
  }
}
```

Ces OID sont des fixtures illustratives, pas des objets réels. Le serveur vérifie existence et type avant d'agir. Une liste vide de validations signifie que ce contrat n'en demande pas ; elle ne peut pas contredire un workflow qui en exige. La réponse `IntegrationCandidate` comporte candidate_id, oldOID, newOID, sourceOID, stratégie, worktree, état de préparation, preuves et éventuelles conditions bloquantes.

`strategy` n'est pas un choix de l'appelant : il doit égaler la stratégie effective résolue par le harness (entrée `strategy` de la tâche `integrate` du workflow épinglé, admise seulement si l'`input_schema` du contrat d'intégrateur la type ; sinon politique Git du repository désignée par `policy_version` ; sinon `merge_commit_or_ff`, selon la [spécification](../../specs/git-integration.md)), sinon `policy_refused`. Les fixtures feature et merge-only ne portent pas `inputs.strategy` : l'`input_schema` de l'intégrateur de référence est fermé, donc la politique Git du repository, sinon le défaut, s'applique. Le `target_ref` textuel est résolu par le serveur en `TargetRef` à la préparation.

## Publier et réconcilier

```json
{
  "command_type": "PublishIntegration",
  "payload": {
    "candidate_id": "candidate-7",
    "expected_candidate_revision": 4,
    "target_ref": "refs/heads/integration",
    "lease_id": "lease-target-9",
    "lease_epoch": 12,
    "scope_id": "scope-integration-1"
  }
}
```

Le serveur relit les OID depuis le candidat immuable ; le client ne peut pas substituer un nouvel OID à un candidat validé. Il retourne un reçu d'admission distinct du résultat final. `lease_id` et `lease_epoch` sont facultatifs. Ils ne sont présents que lorsqu'une tentative intégratrice détient déjà la lease de cible et la renouvelle. Pour une publication demandée par GUI/CLI/TUI, le handler de PublishIntegration acquiert la lease courte côté broker (REQ-GIT-005) ; il restitue lease_id et epoch dans le reçu et dans GetIntegration. Un epoch périmé donne StaleFence/STALE_EPOCH.

`GetIntegration` expose intention, dispatch, résultat observé, provenance, état de réconciliation et lien vers la mission. `ReconcileAction` (F018) observe un essai ambigu ; il n'est pas un alias de `PublishIntegration`.

## Allocations et types

Intentions internes, non wire : `AllocateWorktree` reçoit repo_id, mission_id, base_oid, branche demandée et scope. La localisation finale est choisie par le service selon configuration runtime (défaut `<données Unleash>/worktrees/<repo_id>/<allocation_id>`, hors dépôt utilisateur et hors `.claude/`). `RequestMutationOwnership` reçoit worktree_id, invocation_id et epoch attendu. `ApplyWorkspacePatch` reçoit chemins canoniques, digests avant modification, patch ou octets de remplacement et receipts discovery nécessaires. Les patches sans précondition sont refusés pour les opérations contrôlées qui risquent un écrasement.

`RetireWorktree` est la commande wire de retrait : worktree_id, révision attendue, motif et politique de rétention. Elle exige l'absence de propriétaire vivant et de toute protection Q-12 ; le délai de 7 jours de Q-12 ne vaut que pour le balayage automatique. Elle refuse `dirty_worktree` (travail non commité, candidat de conflit ou commit non atteignable depuis une ref publiée) ou `active_process`, déverrouille le worktree seulement au retrait autorisé et ne force jamais la suppression d'un worktree humain inconnu.

`GitObjectId` est un couple format/hex validé ; `TargetRef`, forme résolue par le serveur à la préparation à partir du `target_ref` textuel du payload, contient nom complet normalisé, kind=direct obligatoire, namespace autorisé, précondition attendue (OID exact ou `ref_absent`) et git_profile_digest ; une symref est refusée ; `LeaseEpoch` est monotone par ressource ; `GitActionState` est l'enum métier fermée `reconciliation_status` de la [coordination Git](../../architecture/git-coordination.md) (`effect_observed`, `desired_state_present`, `candidate_included`, `target_diverged`, `observation_unavailable`, `outcome_unknown`), indépendante d'un simple code de sortie. `GitObservation` conserve timestamp, repo, commande structurée expurgée, identité de processus, exit/signal/inconnu, old/new OID observés et références de sorties.

La création initiale d'une cible utilise `expected_target_state` valant `ref_absent` au lieu de `expected_target_oid`; les deux champs sont exclusifs et validés. Une source unborn sans commit retourne `source_not_committed` pour une préparation d'intégration, tout en restant admissible à l'enregistrement et au développement. La représentation du zéro Git éventuel est une responsabilité de l'adaptateur, jamais une constante de longueur SHA-1 dans le domaine.

## Erreurs et événements

Les noms snake_case de ce document sont des raisons de domaine ; le wire les sérialise en Error commun (classe, reason_code UPPER_SNAKE, retry_class) de l'[IPC](../../architecture/ipc-and-apis.md). F010 enregistre la projection suivante.

| Nom de domaine | Wire (classe/reason_code) | retry_class | Sens | Reprise |
|---|---|---|---|---|
| `target_checked_out` | ResourceBusy/TARGET_CHECKED_OUT | after_human_decision | La cible est checkout dans un worktree, y compris en rebase ou bisect | Branche cible gérée non checkout pour les Runs créés ensuite, ou libération du checkout par l'utilisateur, puis réobservation (Q-10) ; un Run engagé garde sa cible (Q-03) : seule la libération le débloque, sinon un Run successeur (`predecessor_run_id`) créé après son état terminal adopte la nouvelle cible ; conserver le candidat |
| `target_moved` | RevisionConflict/TARGET_MOVED | after_refresh | Valeur actuelle différente de oldOID | Nouvelle préparation sur base observée |
| `StaleEpoch` (remplace `lease_stale`) | StaleFence/STALE_EPOCH | after_reconciliation | Epoch périmé ou propriétaire révoqué | Réacquisition après réconciliation |
| `source_missing` | NotFoundOrHidden/SOURCE_MISSING | after_human_decision | Objet/ref source absent | Restaurer une source autorisée ou revoir la mission |
| `source_changed` | RevisionConflict/SOURCE_CHANGED | after_refresh | Source déplacée depuis sa résolution | Nouvelle préparation sur OID observés |
| `source_not_committed` | InvalidArgument/SOURCE_NOT_COMMITTED | after_human_decision | Source unborn ou sans commit candidat | Committer puis préparer |
| `target_already_exists` | RevisionConflict/TARGET_ALREADY_EXISTS | after_refresh | Création `ref_absent` sur une ref apparue | Nouvelle préparation sur l'état observé |
| `merge_conflict` | ContractViolation/MERGE_CONFLICT | after_human_decision | Préparation bloquée textuellement | Mandat d'intégration spécialisé |
| `validation_missing` | ContractViolation/VALIDATION_MISSING | after_human_decision | Preuve demandée absente ou ancien candidat | Exécuter exactement la validation requise |
| `policy_refused` | CapabilityDenied/POLICY_REFUSED | after_human_decision | Politique Git ou stratégie non conforme | Corriger workflow ou politique par nouvelle version |
| `path_occupied` | ResourceBusy/PATH_OCCUPIED | after_human_decision | Chemin d'allocation occupé | Libérer le chemin ou changer la racine |
| `branch_in_use` | ResourceBusy/BRANCH_IN_USE | after_human_decision | Branche déjà checkout ou allouée | Choisir une autre branche ou réconcilier |
| `dirty_worktree` | ResourceBusy/DIRTY_WORKTREE | after_human_decision | Retrait mettrait en danger du travail | Conserver et décider explicitement |
| `active_process` | ResourceBusy/ACTIVE_PROCESS | after_human_decision | Un processus utilise le worktree | Arrêter ou détacher selon F011, puis retirer |
| `symbolic_target_unsupported` | CapabilityDenied/SYMBOLIC_TARGET_UNSUPPORTED | never | Cible ou ref de conservation symbolique | Choisir une ref directe |
| `git_version_unsupported` | CapabilityDenied/GIT_VERSION_UNSUPPORTED | after_human_decision | Git système inférieur à 2.39 (ADR-09) | Installer un Git qualifié puis réinspecter |
| `outcome_unknown` | OutcomeUnknown/GIT_OUTCOME_UNKNOWN | after_reconciliation | Effet possible non attribuable avec certitude | Observer et décider ; aucun retry automatique |

ResourceBusy et ScopeDenied restent des classes communes sans nom de domaine propre.

`WorktreeAllocated`, `MutationOwnershipChanged`, `IntegrationPrepared`, `IntegrationLeaseGranted`, `IntegrationConflicted`, `IntegrationPublishIntended`, `IntegrationEffectObserved`, `GitActionReconciled`, `RepositoryChanged` et `WorktreeRetired` sont les événements proposés. Chaque événement comporte seq, schema_version, command_id et source de causalité ; les consommateurs dédupliquent.
