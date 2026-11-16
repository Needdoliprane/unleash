# F007 — Design technique proposé

## Registres et stockage

`SkillRegistry` et `RoleRegistry` exposent des interfaces séparées et partagent éventuellement un `BundleStore` adressé par contenu. Les tables canoniques ([ADR-06](../../architecture/adr/ADR-06-skill-role-registry.md), R7 ; [sqlite-design](../../architecture/sqlite-design.md)) sont `registry_entries`, `registry_versions`, `registry_drafts`, `registry_aliases`, `registry_bindings` et `evaluation_reports`. L'unicité d'une entrée porte sur (`entry_kind`, `scope_kind`, `scope_owner_key`, `entry_id`), avec `scope_owner_key` non nul (`global` pour la portée globale) ; aucune clé d'unicité ne contient de NULL. Les reçus et décisions de découverte sont ceux de F009 (`discovery_receipts`, `discovery_uses`), référencés par `provenance.discovery_receipt_id` ; F007 ne crée ni table de reçu ni table de décision de réutilisation. `entry_kind` et namespace empêchent de résoudre un rôle comme un skill. Un bundle publié est immuable ; dépréciation et révocation sont des états de registre séparés.

Une version contient contenu normalisé, digest, auteur, provenance, licence, version de schéma, rapport de validation et versions de dépendances. Les références de rôles pointent des skills exacts. Les ressources du bundle sont locales ou référencées par digest sous ACL ; aucun `$ref` réseau n'est suivi pendant validation. Les alias ne sont résolus que pour une sélection explicite avant engagement.

## Recherche et composition

`RegistryDiscovery` combine filtres exacts, description et éventuelle similarité indexée. Le résultat expose scope, fraîcheur, statut, licence, versions et usage. La priorité repository > projet > global règle une résolution non qualifiée ; elle ne supprime pas les autres candidats autorisés. Le reçu conserve les sources effectivement consultées et leurs limites. La décision de créer avec justification peut être acceptée même si un similaire existe.

Le composeur N+1 choisit objectif et responsabilités, assemble les skills, définit politique modèle et contraintes de contexte. La validation vérifie que sorties/entrées et capacités sont compatibles avec l'usage demandé. Les exigences contradictoires de deux skills produisent un diagnostic, pas un ordre de priorité caché. Le rôle publié reste une proposition d'organisation ; son instance reçoit les permissions effectives de mission.

## Délégation et publication

N+2 peut créer une mission de rédaction de skill pour N. Le livrable est un draft et un rapport, sans pouvoir d'auto-publication implicite. N+2 ou une autre autorité disposant du grant effectue PublishSkillVersion. Le contrôle compare contenu validé, rapport et révision ; toute modification entre validation et publication invalide le rapport. Un auteur ne peut augmenter les permissions de son identité en modifiant `requested_capabilities`.

Les évaluations obligatoires de publication sont définies par politique et profil de contrat : fixtures positives/négatives, résolution des références et absence d'escalade. La politique par défaut d'un nouveau skill est `required_checks` = `schema`, `references`, `permissions`, `fixtures` et `judge_policy` = `none` (ADR-06) ; un juge reste un choix explicite du skill ou du workflow. Les évaluations sémantiques peuvent être humaines ou par modèle ; leur absence ne se cache pas derrière un label de garantie. Un second LLM n'est pas un passage obligatoire. Les rapports affichent méthode, couverture et limites.

## Binding, modèle et révocation

BindRole est atomique : résoudre rôle exact, skills exacts, contrats, outils, schémas et politique modèle, contrôler slot/capacités, écrire manifest puis événements. Un slot tardif encore vierge peut recevoir un rôle créé pendant le run. Le slot lié ne change plus, y compris à nouvelle tentative ou itération. Les prompts envoyés à chaque invocation référencent ces versions, avec la configuration modèle effectivement résolue et les sources de contexte.

Les grants actuels sont recalculés pour chaque effet. Révoquer une permission bloque les nouvelles actions, sans supprimer les octets épinglés. Révoquer une version peut interdire nouveaux bindings et déclencher revue des usages actifs selon motif. Le Studio expose la différence entre contenu ancien encore inspectable et autorisation désormais retirée. Une purge conserve un tombstone avec motif et limites de reconstitution selon politique de rétention.

## Erreurs et observabilité

### Compatibilité vérifiable d'un rôle tardif

Le rôle déclare une `entry_skill` exacte, membre de sa liste ; la tâche ne possède pas de second sélecteur. Le service résout le profil attendu avant évaluation et épingle compiler/evaluator/catalogue dans le manifest. Les digests canoniques des schémas input/output de l'entrée doivent être égaux à ceux du profil. Chaque règle obligatoire doit conserver ID et digest ; des règles nouvelles sont acceptées seulement avec nouveaux IDs, validateurs allowlistés, sévérité obligatoire et agrégation conjonctive. Les capacités effectives sont l'intersection des plafonds et grants, sans changement des règles de permission. Cette procédure refuse schéma logiquement équivalent mais différent plutôt que de supposer un sous-typage général.

Une reprise exige les mêmes versions d'évaluation. Si elles sont indisponibles, la continuation bloque ; une version installée plus récente ne remplace pas silencieusement l'ancienne. Les tests T-F007-016 et T-F007-015 incluent respectivement entrée ambiguë/input/output/règle incompatible et changement de catalogue entre crash et reprise.

Les reason_code canoniques des deux registres (ADR-06) sont `DISCOVERY_REQUIRED`, `AMBIGUOUS_REFERENCE`, `MISSING_DEPENDENCY`, `SCHEMA_LIMIT_EXCEEDED`, `PUBLICATION_CONFLICT`, `PERMISSION_EXPANSION_DENIED`, `INCOMPATIBLE_ROLE`, `CAPABILITY_MISMATCH`, `BINDING_ALREADY_ENGAGED` et `BUNDLE_UNAVAILABLE` ; leur projection wire est enregistrée ci-dessous dans le format commun de l'[enveloppe IPC](../../architecture/ipc-and-apis.md) (nom de domaine, classe/reason_code, retry_class), avec une seule classe par reason_code. `INCOMPATIBLE_ROLE` porte le diagnostic du compilateur (`InputSchemaMismatch`, `OutputSchemaMismatch`, `RequiredRuleChanged` ou `AmbiguousEntrySkill`) ; `CAPABILITY_MISMATCH` reste un refus distinct. Les événements portent référence de version et auteur. Les métriques suivent réutilisation justifiée, refus, versions en usage et latence de recherche, sans transformer la part N+2 en quota. Les logs ne doivent pas exposer le contenu d'un skill privé à un autre scope.

Projection wire des reason_code des registres :

| Nom de domaine | Wire (classe/reason_code) | retry_class | Reprise |
| --- | --- | --- | --- |
| DiscoveryRequired | ContractViolation/DISCOVERY_REQUIRED | never | Nouvelle commande portant le reçu produit par `DiscoverResources` puis `RecordDiscoveryDecision` ; même projection qu'en [F008](../../specs/contract-enforcement.md#erreurs-et-conservation) |
| AmbiguousReference | InvalidArgument/AMBIGUOUS_REFERENCE | never | Référence qualifiée par `id`, `version` et `scope` |
| MissingDependency (ancien MissingSkillVersion) | NotFoundOrHidden/MISSING_DEPENDENCY | after_human_decision | Publier ou rendre visible la version exacte requise, ou référencer une version autorisée ; aucun téléchargement automatique |
| SchemaLimitExceeded | InvalidArgument/SCHEMA_LIMIT_EXCEEDED | never | Réduire l'instance ou la transmettre par référence et digest ; même projection qu'en F008 |
| PublicationConflict (ancien RoleVersionConflict) | RevisionConflict/PUBLICATION_CONFLICT | after_refresh | Enregistrée par l'IPC ; relire la révision puis republier |
| PermissionExpansionDenied | CapabilityDenied/PERMISSION_EXPANSION_DENIED | never | Aucune version numériquement supérieure ne contourne le refus |
| IncompatibleRole | ContractViolation/INCOMPATIBLE_ROLE | never | Diagnostic du compilateur ; choisir un autre rôle ou une version compatible |
| CapabilityMismatch | CapabilityDenied/CAPABILITY_MISMATCH | after_human_decision | Décision explicite sur les grants ou le plafond du slot |
| BindingAlreadyEngaged | ContractViolation/BINDING_ALREADY_ENGAGED | never | Enregistrée par l'IPC ; slot encore vierge ou nouveau run |
| BundleUnavailable | NotFoundOrHidden/BUNDLE_UNAVAILABLE | after_human_decision | La continuation bloque ; aucune version plus récente n'est substituée |

## Attentes de test prépubliées — raccord R6

Les validateurs claim_predicate_matches et test_execution_matches du catalogue v2 utilisent les [contrats F008](../F008-contract-engine/interfaces.md). Chaque règle déclare un expectation_slot littéral ; compiler/registry dérivent les slots requis et vérifient leur type, sans lire un attendu proposé dans la sortie future du worker. L’admission résout chaque slot depuis une version acceptée du plan/contrat/mandat, puis scelle ExpectedExecutionBinding avec action et digest avant dispatch. Une absence, ambiguïté ou tentative de substitution depuis un log/output bloque l’effet concerné.

Les profils/skills v1 existants gardent leur garantie historique d’intégrité des liens. Exiger la vérité observable d’un claim ou la réussite d’une commande déterminée nécessite les validateurs explicites et une attente scellée ; aucun pass sémantique n’est déduit d’une migration de catalogue. Les changements d’attente passent par révision autorisée, jamais par choix a posteriori d’une suite favorable.
