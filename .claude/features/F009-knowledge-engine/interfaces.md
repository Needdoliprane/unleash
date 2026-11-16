# F009 — Interfaces proposées

## Enveloppe commune

Les commandes utilisent l'[enveloppe IPC v2](../../architecture/ipc-and-apis.md#commandenvelope) : protocol_version, request_id, command_id, authority_generation, actor_ref résolu serveur, project_id, aggregate_id, expected_revision, command_type et payload. `command_id` porte l'idempotence ; dans la lignée active, un même identifiant avec un digest différent donne `IdempotencyConflict`. Les exemples ci-dessous sont des fixtures JSON valides et ne décrivent aucune donnée existante. Le schéma discovery V1 est un contrat cible non publié corrigé en R6 puis resserré en R7 (`stream_id` UUID, `namespace_ref` égal à `scope_owner_key`), sans migration V2 ni objets produit supposés.

`DiscoverResources` est une opération explicite du harness. Sa réponse devient une source durable du reçu ; un appel de recherche UI sans intention de création reste possible sans produire artificiellement une décision métier.

```json
{
  "command_type": "DiscoverResources",
  "payload": {
    "intent_id": "intent-email-01",
    "resource_kinds": ["function", "contract", "test"],
    "query": "normaliser une adresse email pour les comptes clients",
    "scope": {"project_id": "project-01", "repo_ids": ["repo-01"], "source_binding_ids": [], "registry_scopes": ["global", "project"]},
    "consumer_context_ref": "consumer-mission-worktree-02",
    "include_unmerged": true,
    "required_freshness": "current_or_explicitly_partial",
    "limit": 20
  }
}
```

La réponse porte `query_id`, `catalog_revision`, `coverage_ref`, `coverage`, `results`, `warnings` et `next_cursor`. `coverage.status` vaut `complete`, `partial`, `stale` ou `unavailable`. Le paramètre `required_freshness` vaut `current` (refus si une source requise est `stale` ou `unknown`) ou `current_or_explicitly_partial` (réponse partielle admise avec limites listées). Un résultat inclut `resource_id`, `kind`, `owner_mission_id` si pertinent, `source_ref: TypedSourceRef`, `consumer_context_ref`, `annotations_observed_at`, méthode/profil/confiance et les états source/code/travail/intégration/fraîcheur/disponibilité. Les annotations suivent [shared-knowledge](../../specs/shared-knowledge.md) ; hors Git, code et intégration valent `not_applicable`. Le serveur filtre les candidats interdits avant calcul du total et ne retourne aucun identifiant de ressource inaccessible. Un projet sans dépôt envoie `repo_ids: []` et les bindings/registres autorisés ; le contexte consommateur désigne ses racines/affectations plutôt qu'un worktree fictif.

`coverage_ref` résout une observation durable : requête, catégories, scope/révision, catalogues et sources consultés même sans résultat, versions/générations/ACL, contexte de profils, curseurs, limites et candidats réellement retournés. Cette observation reste vérifiable si l'index est reconstruit. Un curseur de pagination et une révision de catalogue ne remplacent pas cette provenance.

```json
{
  "command_type": "RecordDiscoveryDecision",
  "payload": {
    "intent_id": "intent-email-01",
    "query_ids": ["query-17"],
    "purpose": "pre_creation",
    "decision": "create",
    "strategy": "copy",
    "candidate_ids": ["symbol-normalize-email-7"],
    "justification": "Le domaine conserve la casse locale et retourne une erreur typée ; le helper trouvé transforme toute la chaîne.",
    "target_resource_id": null,
    "target_version_ref": null,
    "supersedes_receipt_id": null,
    "creation_use_id": null,
    "expected_catalog_revision": 89
  }
}
```

Le reçu obtenu contient `origin`, les `source_refs` dérivées des recherches, `validation_basis`, la couverture et la relation temporelle `before_creation/observed_after_creation`. Ni origine, ni principal, ni cause, ni générations, ACL ou dates de validation ne sont des assertions d'autorité acceptées du client. Le serveur les résout depuis le canal/mandat et les objets durables. Les candidats/cibles proposés doivent appartenir aux recherches et au scope autorisés ; un identifiant lisible n'accorde pas accès.

| `origin.origin_kind` | Champs requis en plus de `principal_id` et de la cause durable | Champs exclus |
|---|---|---|
| `invocation` | `invocation_id`, `attempt_id` ; `run_id` et `step_id` ensemble lorsqu'applicables | `replay_worker_id`, `control_principal_id`, `source_run_id` |
| `replay` | `replay_worker_id`, `run_id`, `step_id` | `invocation_id`, `attempt_id`, `control_principal_id`, `source_run_id` |
| `control_plane` | `control_principal_id` ; `source_run_id` si le mandat dérive d'un run | `invocation_id`, `attempt_id`, `replay_worker_id`, `run_id`, `step_id` |

Pour `control_plane`, le serveur projette le run de l'action ou de sa cause en `source_run_id` et n'inclut pas `step_id` ; c'est la seule différence admise avec les contraintes de `actions`.

Le principal control_plane égale le principal authentifié ou service autorisé résolu ; une référence de cause vaut receipt de commande, action, timer ou événement existant et autorisé. Les appartenances tentative/invocation, worker/step/run et projets suivent [sqlite-design](../../architecture/sqlite-design.md). Une origine ne peut effacer la lignée de mandat ni contourner une politique de modèle. Le schéma vérifie la forme ; ces relations et la dérivation d'autorité sont des contrôles domaine obligatoires.

`AttachDiscoveryReceipt` reçoit `receipt_id`, la référence d'une opération structurée préparée et ses révisions attendues. Le serveur résout l'intention/scope, calcule le digest du payload réel, vérifie ou réserve la cible et enregistre une autorisation durable avant effet. Le demandeur ne choisit pas un digest à la place de l'opération. L'attachement est exclusif à cette opération logique ; un retry utilise son idempotence, une nouvelle création demande un nouveau reçu.

`validation_basis` fixe `intent_revision`, `scope_ref/scope_revision`, `policy_version_ref`, `validator_version`, `event_cursor`, `checked_at`, `max_age_ms` et `valid_until`. La borne positive est choisie par la politique serveur et réduite aux échéances des observations/ACL. Autorisation puis dispatch vérifient les sources/cible/version, révisions, bornes et fences ; une expiration exige une validation mécanique renouvelée si les références sont identiques, sinon nouvelle discovery. Horloge/stream incertains ou historique incomplet ne sont pas assimilés à une absence de changement.

Après effet, `DiscoveryUseObservation` conserve `use_id`, reçu et autorisation, opération/cause, intention/révision, cible/version/digest effectivement créés ou consommés, sources, curseurs et dates avant/après. L'autorisation est déjà durable avant cette observation ; une issue inconnue ne fabrique pas l'observation de succès. `GetDiscoveryReceipt` restitue le reçu immuable et une projection distincte `current_freshness`, `use_state`, invalidations et observations de validation autorisées. Le champ historique `freshness` n'est pas réécrit.

Pour `purpose=completion_revalidation`, `RecordDiscoveryDecision` exige `creation_use_id`, `supersedes_receipt_id` et cible/version livrées ; il conserve `observed_after_creation`. Cette nouvelle recherche est requise après changement extérieur pertinent selon [l'algorithme de clôture](../../specs/shared-knowledge.md). Elle ne remplace pas l'usage préalable. `posthoc_reconciliation` désigne une recherche tardive sans cet usage et ne satisfait jamais normalement l'antériorité. Les [fixtures agente](../../examples/discovery-receipt.json), [humaine sans Git](../../examples/discovery-human.json), [replay](../../examples/discovery-replay.json) et [clôture](../../examples/discovery-completion.json) illustrent ces formes.

## Types de domaine

`SnapshotId`, `RepoId`, `WorktreeId`, `ReceiptId` et `IntentId` sont des identifiants opaques sérialisés en chaînes. `TypedSourceRef` est l'union fermée `git_snapshot/resource_snapshot/file_snapshot/registry_version` du schéma commun. `version_id` résout le snapshot/version exact ; `source_id` porte la lignée de fraîcheur. `Generation` et `CatalogRevision` sont des entiers non négatifs comparés par le serveur dans leur source/stream, jamais entre worktrees sans identité. `ObjectId` porte format et représentation validée ; `ContentDigest` porte algorithme et hexadécimal, dont algorithme/longueur sont vérifiés par le domaine. `ResourceLocation` contient chemin canonique encodé, nom affichable, offsets d'octets et encodage des positions d'origine. Un chemin affichable ne sert jamais de clé d'accès.

`RequestDependency` accepte source mission, cible mission, `wake_condition`, délai métier et receipt_id. La réponse est une arête acceptée ou `dependency_cycle` avec le cycle de missions autorisées à voir par le demandeur. Si les ACL ne permettent pas d'en révéler tous les détails, le système indique le blocage sans divulgation des noms.

## Erreurs

| Code | Sens | Récupération autorisée |
|---|---|---|
| `snapshot_inconsistent` | Mutation détectée pendant capture | Recapture ou usage provisoire explicitement permis |
| `profile_unavailable` | Serveur ou contexte de build inutilisable | Profil dégradé déclaré ; configuration (`SetSettings`) ou installation assistée (`AcquireTerminal` catalogué, [Q-13](../../specs/shared-knowledge.md#couverture-des-langages-v1)) |
| `source_missing` | Source ou version référencée absente, purgée ou non résolue | Réconciliation de la source ou nouvelle sélection ; aucun résultat vide présenté comme complet |
| `index_stale` | Génération d'index en retard sur la source requise | Attente de publication ou recherche partielle déclarée selon `required_freshness` |
| `source_advanced` | Génération changée avant publication | Nouvelle préparation de l'index |
| `receipt_stale` | Préconditions pertinentes changées avant effet ou à la clôture | Nouvelle recherche/décision ; l'usage historique reste conservé |
| `receipt_validation_expired` | Borne écoulée sans préconditions actuelles prouvées | Contrôle mécanique borné et nouvelle observation si références identiques ; sinon nouvelle discovery |
| `discovery_origin_mismatch` | Origine/attribution incompatible avec le canal, les FKs ou le mandat | Refus avant effet, pas d'invocation fictive |
| `discovery_history_unknown` | Chronologie ou réconciliation insuffisante | Réconciliation bornée ; pas de pass automatique |
| `discovery_evidence_missing` | Création identifiable sans trace admissible | Réconciliation ; pas d'antidatage |
| `access_denied` | Source hors autorisation actuelle | Aucun extrait ni détail sensible |
| `budget_exhausted` | Limite atteinte | Réponse partielle et curseur si possible |

Les événements `SourceSnapshotCaptured`, `IndexGenerationPublished`, `SourceFreshnessChanged`, `DiscoveryReceiptRecorded`, `DiscoveryReceiptInvalidated`, `DiscoveryUseAuthorized`, `DiscoveryUseObserved`, `DiscoveryValidationObserved`, `DiscoveryClosureChecked`, `DependencyAdded` et `DependencySatisfied` portent leurs identifiants et séquences. Invalidated désigne l'éligibilité courante, pas la réécriture d'une preuve passée. Les payloads complets de code/documents ne sont pas diffusés sur le bus général.

## Décision de découverte canonique — FINAL-02

La décision reuse|create indique si la ressource logique cible existe déjà. La stratégie indique la manière : pour reuse, use_existing/coordinate/wait/adapt ; pour create, copy/adapt/new. adapt exige candidate_ids ; target_resource_id et target_version_ref sont obligatoires pour reuse et résolvent une identité/version existante autorisée. Pour create/pre_creation ils sont nuls ; l'autorisation réserve l'identité et l'observation relie la version résultante. À la clôture ou en réconciliation tardive, ils désignent la cible effectivement examinée même si la décision historique est create. Une nouvelle identité issue d'adaptation vaut create/adapt ; une nouvelle version de la même ressource peut valoir reuse/adapt. La validation vérifie ces références et ces couples, sans juger l'équivalence métier. Le schéma partagé [discovery-receipt](../../schemas/discovery-receipt.schema.json) fait autorité sur l'enveloppe du reçu.

Les erreurs spécifiques snake_case de ce document sont des reason_code normalisés en UPPER_SNAKE lors de la sérialisation IPC. Le champ code utilise la classe commune : ContractViolation pour DISCOVERY_EVIDENCE_MISSING/RECEIPT_STALE/RECEIPT_VALIDATION_EXPIRED/DISCOVERY_HISTORY_UNKNOWN, ScopeDenied pour ACCESS_DENIED/DISCOVERY_ORIGIN_MISMATCH, ResourceBusy pour SOURCE_ADVANCED, BudgetExceeded pour BUDGET_EXHAUSTED. NotFoundOrHidden porte SOURCE_MISSING ; ResourceBusy porte INDEX_STALE et SNAPSHOT_INCONSISTENT ; CapabilityDenied porte PROFILE_UNAVAILABLE ; ContractViolation porte DEPENDENCY_CYCLE. Aucune raison ne reçoit deux classes, et aucun second format Error n'est créé.
