# Role Registry — capacité de premier ordre V1

## Distinction et schéma

Le registre des rôles est distinct du registre des skills dans le domaine, les identifiants, commandes et écrans. Une implémentation peut partager un stockage de bundles adressés par contenu. Le rôle n'est ni une simple chaîne système ni le profil mutable d'un agent ; il possède une version inspectable et des dépendances exactes.

Une version contient `role_id`, `version`, `name`, `scope`, `level`, `objective`, `responsibilities`, `skills`, `entry_skill`, `model_policy`, `requested_capabilities`, `permission_constraints`, `contracts`, `context_policy` et `scope_policy`. `skills` référence id/version exacts. Les références `skills`, `entry_skill` et `contracts` d'une version de rôle incluent portée et owner de namespace ([schéma](../schemas/role.schema.json)) ; une référence {id, version} non qualifiée n'est admise qu'en draft, avant résolution. `entry_skill` doit être l'un de ces membres et détermine le contrat d'entrée/sortie de la tâche ; les autres compétences ne remplacent pas implicitement cet output. `contracts` est la liste dérivée des contrats inline des skills membres ; une entrée sans skill correspondant ou de version différente est refusée par `ValidateRole`. `model_policy` référence une politique versionnée, les capacités nécessaires et l'autorisation éventuelle de fallback ; l'invocation persiste le fournisseur et modèle réellement résolus. `context_policy` définit sources permises et limite d'entrée ; il n'expose aucun raisonnement privé. Confidentialité et sélection du contexte relèvent de la politique de [model-routing](model-routing.md).

Le [schéma](../schemas/role.schema.json) contrôle la forme. La validation sémantique contrôle l'existence des versions, la compatibilité des contrats et la non-escalade des permissions. Un rôle de niveau N ne reçoit pas la capacité d'organiser des sous-agents uniquement parce que sa prose l'affirme.

La référence `model_policy.policy_ref` désigne l’ID et la version d’une [politique publiée par Settings](../features/F012-model-api-router/interfaces.md#politique-modèle-versionnée-du-rôle) dans le namespace de publication du rôle.

## Portées et résolution

Les portées sont `repository`, `project`, `global`. Pour une recherche non qualifiée dans un repository connu, la priorité de présentation et de résolution est repository > projet > global ([ADR-06](../architecture/adr/ADR-06-skill-role-registry.md), R7). Toutes les variantes autorisées sont visibles, avec indication du masquage éventuel. Une référence pleinement qualifiée inclut portée et owner de namespace ; elle ne retombe pas sur une autre version en cas d'absence.

Un alias tel que « développeur » est résolu en candidat puis confirmé par commande de binding sur version exacte. Deux résultats également prioritaires provoquent une ambiguïté explicite (`AMBIGUOUS_REFERENCE`). Une ACL peut cacher un candidat ; le reçu conserve une limitation de couverture sans divulguer son contenu. La recherche ne peut pas affirmer l'absence d'un rôle auquel l'acteur n'a pas accès.

## Commandes et évènements

`SearchRoles` est une lecture : elle retourne candidats, couverture, fraîcheur et ordre de résolution sans écrire de reçu. Avant création, `DiscoverResources` (resource_kinds `role`) puis `RecordDiscoveryDecision` produisent le `DiscoveryReceipt`. `CreateRoleDraft` exige ce reçu préalable. Si un registre ou une source est indisponible, le reçu porte une couverture partielle explicite (`coverage_ref`, limites) et sa justification ; aucune raison libre ne remplace le reçu, et une recherche tardive relève de `posthoc_reconciliation` avec dérogation distincte. `ValidateRole` produit diagnostics et compatibilité. `PublishRoleVersion` ajoute une version immuable après contrôle d'autorité. `BindRole` relie une version exacte à un slot non engagé, produit `RoleBound` et les `SkillBound` dépendants. `DeprecateRoleVersion` empêche sa sélection par défaut ; `RevokeRoleVersion` bloque les nouveaux bindings et route les mesures sur les usages actifs.

Les mutations utilisent l'enveloppe commune, la révision attendue et l'idempotence. Un changement de responsabilité ou d'outil exige une nouvelle version. `PUBLICATION_CONFLICT`, `MISSING_DEPENDENCY`, `CAPABILITY_MISMATCH`, `PERMISSION_EXPANSION_DENIED`, `BINDING_ALREADY_ENGAGED` et `AMBIGUOUS_REFERENCE` sont des refus distincts. Ce sont les reason_code canoniques des registres ([F007](../features/F007-skill-studio-registry/technical-design.md), ADR-06), portés par une classe commune de l'[enveloppe IPC](../architecture/ipc-and-apis.md).

## Autonomie N+1 et pinning

N+1 peut chercher, composer et publier un rôle dans ses grants, puis l'affecter à des workers de mission. Pour un slot tardif, le harness vérifie que le rôle nouveau satisfait le contrat de sortie attendu et reste sous le plafond des capacités déclaré par le workflow. Cette vérification n'élargit pas les permissions disponibles. Une fois lié, le rôle et ses skills sont figés pour ce slot durant le run, y compris après crash ou nouvelle invocation.

« Satisfait le contrat » suit l'[algorithme fermé V1](../architecture/skills-contract-model.md) : unique entry_skill membre, digests exacts des schémas input/output, règles obligatoires identiques et extensions seulement conjonctives/restrictives allowlistées. Les versions compiler/evaluator/catalogue sont choisies et épinglées avant le contrôle. La tâche utilise l'entrée du rôle effectif ; aucun champ de tâche concurrent ne peut sélectionner un autre primary skill. Une compatibilité par simple ressemblance des descriptions est interdite.

Modifier une version existante serait une substitution de code de conduite invisible ; l'opération est donc interdite. Une demande de mise à jour d'un binding engagé reste candidate pour un run suivant ; aucune migration de binding engagé n'existe en V1 (Q-03, [ADR-04](../architecture/adr/ADR-04-workflow-semantics.md)). L'interface montre clairement « publié », « lié », « actif », « candidat suivant » et « révoqué ».

## Critères vérifiables

Créer des rôles homonymes dans trois portées doit produire une résolution expliquée. Un rôle composé de skills v1 reste v1 après publication des skills v2. Un slot non lié peut recevoir un rôle nouvellement publié compatible ; un slot en cours refuse la même mutation. Trois agents partageant le rôle conservent trois scopes et trois sessions. Un rôle qui requiert un modèle indisponible produit une erreur de capacité, pas un remplacement silencieux. Les exigences atomiques REQ-ROLE figurent dans [F007](../features/F007-skill-studio-registry/requirements.md).
