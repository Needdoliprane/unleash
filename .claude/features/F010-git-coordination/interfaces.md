# F010 — Interfaces proposées

Les payloads suivants sont imbriqués dans la CommandEnvelope v2 commune de l'[IPC](../../architecture/ipc-and-apis.md), `authority_generation` comprise. Le serveur résout l'acteur depuis la session locale et vérifie ses capacités ; le payload ne permet pas d'emprunter l'identité d'un intégrateur. La clé d'idempotence est (acteur authentifié, command_id), couplée au digest canonique défini par l'IPC v2. Un nouvel essai après déplacement de cible utilise un nouveau command_id avec lien vers l'essai précédent.

## Inventaire des worktrees : `ListWorktrees`

`ListWorktrees` est une lecture `QueryEnvelope` v2. `project_id` est requis ; le payload fermé contient `repo_id` non vide, `page_size` entier de 1 à 100 (50 par défaut) et `opaque_cursor` facultatif ou `null`. Un curseur présent comporte de 1 à 512 caractères ASCII alphanumériques, tirets ou underscores ; tout curseur mal formé ou lié à un autre projet, dépôt ou opération retourne `InvalidArgument`. Le serveur authentifie le pair et réapplique la capacité de lecture et la portée projet avant chaque page ; un projet hors mandat retourne `ScopeDenied/ACCESS_DENIED`. Il résout `repo_id` dans les repositories du projet enregistrés par `ReviseProject` ; aucun chemin ni identité de dépôt fourni par le client ne remplace cette association. Un dépôt absent ou masqué ne révèle aucune information hors portée. Un projet sans dépôt reste valide.

`QueryResponse.result` est un objet fermé contenant `project_id`, `repo_id`, `object_format` (`sha1` ou `sha256`), `git_profile_digest`, `observed_at`, `items`, `coverage` dont `status` vaut `complete` ou `incomplete`, et `next_cursor` (opaque ou `null`). `project_id` et `repo_id` reflètent les références exactes demandées et autorisées ; le daemon vérifie cette concordance. `observed_at` est une date RFC 3339 et chaque `head_oid` non nul utilise le `object_format` observé. Les items sont triés par les octets UTF-8 de `path` en ordre lexicographique croissant, sans normalisation ; ce tri stable est conservé sur toutes les pages. La dernière page porte `next_cursor=null`. Les pages relisent la même collecte bornée ; le curseur est lié à cette collecte, au projet, au dépôt, aux filtres et à l’autorité. Un curseur expiré ou une collecte devenue indisponible retourne `ResyncRequired/CURSOR_EXPIRED`, `retry_class=after_refresh` : recommencer la lecture, sans poursuivre sur une autre collecte. Un curseur de portée ou de filtre différent est refusé ; il ne donne accès à aucune donnée hors portée. La couverture décrit la collecte entière, pas seulement la page. Une observation trop volumineuse ou illisible est explicitement `incomplete` si sa restitution bornée est possible, sinon une `Error` commune signale l’échec, par exemple `ResourceBusy/LOCAL_RESOURCE_EXHAUSTED`, `retry_class=after_human_decision`, pour une limite locale atteinte ; aucune liste tronquée n’est dite complète. Une collecte incomplète ne permet jamais de conclure que la cible est libre.

Chaque élément de `items` est fermé et contient `path`, `head_oid` (`GitObjectId` ou `null` pour une branche unborn), `branch_ref` (nom complet de ref ou `null` si HEAD détaché), `dirty` (`clean`, `dirty` ou `unknown`), `operations` (ensemble des opérations Git observées : `merge`, `rebase`, `bisect`, `cherry_pick`), `operations_complete`, `operation_branches`, `operation_branches_complete` et `ownership` (`managed`, `external` ou `unknown`). Chaque entrée de `operation_branches` associe `operation` (`rebase` ou `bisect`), `branch_ref` (nom complet) et `provenance` (`rebase_metadata` ou `bisect_metadata`) aux métadonnées Git effectivement observées. La branche réservée par un rebase ou un bisect reste exposée avec sa provenance même si `branch_ref` est `null` parce que HEAD est détaché. Si une association est illisible ou incertaine, elle n’est pas inventée et `operation_branches_complete=false` ; des opérations illisibles donnent `operations_complete=false`. Dans ces cas la couverture est `incomplete` et la cible n’est jamais supposée libre. Un item de worktree observé ne peut avoir simultanément `head_oid=null` et `branch_ref=null` : une branche unborn conserve sa ref et un HEAD détaché conserve son OID. Si un worktree illisible ne peut être représenté, il est omis et la couverture globale est `incomplete`, jamais une preuve de cible libre. Un dépôt bare sans checkout ne crée aucun item de HEAD détaché : si son inspection est complète, `items` est vide et `next_cursor=null`. Les worktrees réellement liés restent observables. `path` est une localisation observée, non une identité durable ni une preuve de propriété. `unknown` ne devient jamais `clean` ou absence certaine.

La réponse commune conserve son `request_id` et son watermark SQLite lorsqu’il est applicable. La collecte est une suite d’observations Git datées, pas un snapshot Git atomique ; le watermark ne rend pas ces observations atomiques. Même après lecture de toutes les pages d’une collecte complète, un changement extérieur peut la périmer et impose une réobservation avant effet.

Le daemon utilise l’inspecteur Git et le broker, avec version et capacités sondées, profil Git effectif épinglé, replacement refs désactivées, arguments structurés et sorties machine à séparateurs NUL. Il lit les worktrees, leurs branches, leur état dirty et leurs opérations en cours, sans exécuter de hooks, filtres ou helpers non autorisés. Le profil désactive les écritures facultatives de Git (`GIT_OPTIONAL_LOCKS=0`) et ne permet ni accès réseau ni exécution de configuration externe non autorisée. La lecture ne crée ni allocation, branche, ref, commit ou candidat et ne modifie ni configuration, index ou fichiers du dépôt. Un chemin impossible à représenter, une observation illisible ou incomplète est signalé explicitement, sans fabriquer une disponibilité.

L’inventaire expose les faits observés ; `branch_ref` égal à la branche par défaut enregistrée ne suffit pas à choisir la cible d’un workflow. Lorsque la cible effective du workflow est connue, l’adoption expose `integration_readiness=blocked_target_checkout` si cette cible est checkout ou réservée par un rebase ou un bisect ; cette disponibilité d’adoption est distincte de l’état d’un essai d’intégration. Une collecte ou association incomplète laisse la disponibilité inconnue, jamais libre. Seul le traitement d’un essai existant peut le placer en `blocked_target_checkout` selon les préconditions de `PrepareIntegration` et de publication ; `ListWorktrees` ne crée ni ne fait évoluer cet essai. Les remèdes et la cible d’un Run déjà engagé restent ceux de Q-10 et Q-03.

Pour la première tranche de T-F010-001, un vrai dépôt temporaire local est rattaché au projet par `ReviseProject`, avec une branche cible connue checkout et un fichier dirty. Depuis l’IPC, l’inventaire doit retrouver ce worktree et sa branche, signaler `dirty`, puis conserver octet pour octet la ref, l’index et les fichiers. Le test mesure dans Docker la sortie réelle, le profil/version Git, les erreurs et les états avant/après ; une lecture macOS exige son runner dédié. Les cas ultérieurs vérifient séparément rebase et bisect avec HEAD détaché, branche et provenance restituées, métadonnées illisibles et opérations inconnues sans cible déclarée libre, puis pagination sur une même collecte, changement de portée, expiration et dépassement de borne. Cette première tranche ne qualifie pas à elle seule ces cas, les chemins atypiques, la concurrence externe ou la publication. Son premier résultat reste `not_run` jusqu’à exécution réelle.

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
