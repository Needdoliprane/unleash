# Moteur de contrats et preuves

## Périmètre et principe

Le moteur Rust applique un catalogue de validations typées aux entrées, actions, sorties et preuves. Il ne démontre pas la correction universelle d'un programme. Il sépare conformité observable et jugement technique. Les formats et rôles sont détaillés dans [skills-contract-model.md](../architecture/skills-contract-model.md) ; les tests transversaux dans [contract-tests.md](../testing/contract-tests.md).

Un `ContractVersion` contient identité/version, schémas locaux d'entrée/sortie, règles et politique d'inconnu. Chaque règle référence un validateur allowlisté, des paramètres typés, sa phase (`input`, `pre_action`, `post_action`, `completion`) et un niveau obligatoire ou informatif. Les règles obligatoires ne sont pas désactivables par une phrase de l'agent. Une politique métier peut choisir ses règles ; elle ne désactive pas les contrôles d'autorisation du broker.

## Phases d'application

À l'entrée, le moteur valide schéma, scope et références. Avant l'action, il calcule les permissions effectives, contrôle resource identity et epoch puis réserve les budgets. Après l'action, il enregistre l'observation et vérifie sa structure sans la transformer en succès métier. À la clôture, il rassemble les règles applicables, relie claims et observations, puis produit un `ContractVerdict`.

Ce verdict contient `pass`, `fail` ou `unknown`, résultats par règle, evidence bundle, versions des validateurs et objets contrôlés. Le verdict global vaut `fail` dès qu'une règle obligatoire échoue, sinon `unknown` si une règle obligatoire est inconnue, sinon `pass` ; les règles informatives restent visibles sans influencer le verdict ([ADR-07](../architecture/adr/ADR-07-contract-enforcement.md), R7). Une règle obligatoire `fail` empêche l'acceptation. Une règle obligatoire `unknown` applique son `on_unknown` publié : `block` met le run en `blocked` (`NeedEvidence`), `fail` fait échouer l'étape ; elle ne devient jamais vraie par défaut. Le Studio et le compilateur proposent `block` par défaut pour une règle obligatoire ; `fail` est un choix explicite. Un jugement N+1 peut constater que le livrable n'est pas adéquat malgré un verdict formel favorable ; il ne peut pas faire disparaître un refus de sécurité.

Les versions de compilateur, évaluateur et catalogue sont sélectionnées et épinglées dans le bundle/manifest avant toute évaluation, pour bindings fixes comme tardifs. Le rapport référence ces versions déjà fixées. Une reprise ne prend pas une version plus récente par défaut ; si l'ancienne n'est plus disponible, la continuation bloque. La compatibilité d'un slot tardif suit l'[algorithme exact V1](../architecture/skills-contract-model.md), sans inclusion générale de schémas ni interprétation de prose comme preuve.

## Catalogue V1 et limites

| Validateur | Entrées minimales | Propriété vérifiée et limite |
|---|---|---|
| `json_schema` | Instance, schéma local, profil/version | Forme et types ; pas vérité des valeurs métier |
| `artifact_present` / `artifact_digest` | Référence, emplacement autorisé, digest | Présence/intégrité de contenu observé ; pas qualité |
| `path_within_scope` / `diff_within_scope` | Identités de ressources, scope, diff | Conformité des ressources contrôlées ; pas sandbox OS globale |
| `git_snapshot_matches` | Repo, worktree, commit ou dirty snapshot | Identité du code auquel la preuve s'applique |
| `observed_process_exit` | Action, process identity, argv digest, exit status | Fin observée d'une commande ; aucun succès de test inventé |
| `test_observation_matches` | Observation, parseur versionné, cas attendus | Résultat parsé et snapshot associé ; ne désigne pas une commande exigée |
| `claim_evidence_links` | Claim, observations, sujet et fraîcheur | Intégrité des liens ; un pass ne vaut pas vérité de la proposition |
| `test_execution_matches` — catalogue v2 | Observation, slot littéral d'exécution attendue | Commande et résultat comparés à un attendu autoritaire scellé avant effet |
| `claim_predicate_matches` — catalogue v2 | Claim typé, prédicat/version, slot littéral attendu | Vérité observable du seul prédicat fermé ; prose qualitative inconnue |
| `integration_observed` | Repo cible, ref, commits, observation Git | Intégration constatée au snapshot ; pas absence de régression |
| `discovery_receipt_present` | Reçu, décision reuse/create avec stratégie explicite, scope | Recherche préalable tracée et justification ; pas absence absolue de doublons |

Les comparaisons reposent sur identités durables, pas seulement sur noms de fichiers ou PID. Un test ancien sur un autre commit échoue `EvidenceSnapshotMismatch`. Un log écrit par le modèle sans action collectée reste un artefact déclaré ; il n'obtient pas l'authenticité d'une observation du harness.

### Projection d'évaluation

Les paramètres `*_pointer` sont des JSON Pointers évalués sur le document `{input, output, context}` construit par le harness. `input` contient les entrées validées de la tâche et `output` la sortie candidate. `context` regroupe les valeurs que le serveur résout depuis le manifest épinglé, par exemple `source_commits` dérivés des missions sources ; ces valeurs ne viennent jamais du modèle ni du payload client. `ValidateContractDefinition` refuse toute autre racine (`InvalidRuleParameters`). Les paramètres d'attendu (`expected_snapshot_pointer`, `expected_digest_pointer`, `expected_commits_pointer`, `target_ref_pointer`) visent `/input` ou `/context` ; un pointeur d'attendu dans `/output` est refusé de même. Une valeur de contexte absente donne `unknown`.

## Contrats observables R6 et compatibilité

Les noms nouveaux appartiennent au catalogue interne `validator-catalog` version 2, catalogue livré en V1 ([ADR-07](../architecture/adr/ADR-07-contract-enforcement.md), R7) : 13 validateurs, soit les 11 historiques plus `test_execution_matches` et `claim_predicate_matches`. La version 1 (11 validateurs) reste chargeable pour un profil qui l'épingle. Compiler, evaluator et catalogue sont chargés et épinglés avant contrôle ; déclarer le numéro 2 ne prouve pas qu'un binaire les implémente. Le chargeur vérifie réellement les validateurs et leurs formats. Les anciens bundles `unleash.skill.v1` et profils `unleash.contract-profile.v1` restent valides. Leurs règles ne sont ni renommées ni augmentées rétroactivement. Une obligation nouvelle produit nouvelle version/digest de contrat, de skill et de profil ; aucun binding engagé n'est migré implicitement.

Les formats nouveaux ont leurs propres versions : [Claim V1](../schemas/contract-claim.schema.json), [ExpectedTestExecution V1](../schemas/expected-test-execution.schema.json), [ExpectedExecutionBinding V1](../schemas/expected-execution-binding.schema.json) et [TestExecutionObservation V1](../schemas/test-execution-observation.schema.json). Ils décrivent des objets persistants, pas des DTO accordant au client le choix de leur provenance. Le catalogue local résout leurs URN ; aucune résolution réseau n'est autorisée. Un changement incompatible de ces formes augmente leur version. L'extension d'allowlist des schémas skill/profil ne donne aucune permission supplémentaire.

### Sujet et projections exactes

`SubjectRef` contient `subject_id`, `subject_revision`, `scope_id`, `scope_revision` et `snapshot_ref={kind,id,sha256}`. Les sortes de snapshot sont `git_tree`, `source_snapshot` et `data_snapshot`. `git_tree` désigne un `git_snapshot` à `capture_consistency=git_tree_coherent` ; `source_snapshot` désigne toute autre entrée de `source_snapshots` (Git dirty, `file_snapshot`, `resource_snapshot`) ; `data_snapshot` désigne une `AutomationDataRevision`. Dans tous les cas, `id` est le `version_id` résolu et `sha256` le digest SHA-256 du manifeste canonique de cette version, jamais un OID Git. Ces cinq composantes et la référence complète de snapshot doivent être identiques entre attente, binding, claim et observation. Une branche, un chemin affichable, un résumé ou un hash déclaré par l'agent ne remplace pas un snapshot résolu. Le serveur vérifie le contenu/digest et sa provenance : repo/worktree/format Git ou version du jeu de données, périmètre capturé et cohérence. Une capture non cohérente, une matérialisation inconnue ou des sources modifiées pendant le test donnent `unknown` pour la correspondance requise. Le contrôle vise les sources désignées, sans promettre une isolation OS générale.

Le schéma d'observation décrit une projection collectée par un adaptateur authentifié : action, binding, référence d'attendu, sujet, commande effectivement observée et résultat. `observed_command` n'est jamais rempli par copie de l'attendu. Si une composante exigée de la commande n'est pas collectée, l'adaptateur n'émet pas de projection V1 ; il enregistre une collecte incomplète liée à l'action et au binding, et la correspondance vaut `unknown` (`EvidenceMissing`) ; aucune valeur n'est complétée depuis l'attendu. Le journal autoritaire relie l'action à son producteur, son origine invocation/replay/control_plane, son autorisation, ses séquences et sa tentative. La forme JSON seule ne confère aucune authenticité. Une lecture/collecte indisponible reste inconnue.

### Exécution de test attendue avant effet

`ExpectedTestExecution` est immuable et adressé par contenu. Son `authority_ref` désigne la version/digest d'un contrat, d'un plan d'exécution accepté ou d'un mandat de mission. Il contient le sujet exact, outil/version, digest d'argv, identité canonique du cwd, profil/version/digest d'environnement, digest d'environnement pertinent, suite/version/digest, parseur/version et politique de résultat. Le profil d'environnement nomme les composantes réellement comparées et ses exclusions. Les secrets sont représentés par des références opaques versionnées du backend, sans valeur en clair ni hash public d'un secret isolé ; une composante requise impossible à observer rend la comparaison inconnue.

Le paramètre littéral `expectation_slot` d'une règle déclare une dépendance de contexte, pas un JSON Pointer libre. Le compilateur dérive la liste des slots requis. Avant dispatch, le harness résout chaque slot depuis le plan/contrat/mandat accepté, matérialise le snapshot et scelle un `ExpectedExecutionBinding` : `binding_id`, `contract_ref`, `rule_ids`, `expectation_slot`, `expectation_ref={id,version,sha256}`, `subject`, `action_id`, `activation_ref={id,revision}` et `sealed_cursor={stream_id,sequence}`. Les règles d'un même slot partagent ce binding ; elles ne créent pas chacune un nouveau test. Réserver l'action et persister ce binding précèdent l'effet et toute observation de son résultat. Absence de source autoritaire ou de slot bloque l'admission. Un modèle peut proposer un plan dans ses grants ; il ne choisit pas l'attendu depuis sa sortie, son claim ou une observation déjà réussie.

Une tentative de remplacement après résultat est rejetée. Un retry explicitement permis conserve `expectation_ref` et crée avant son effet un nouveau binding immuable et un nouvel action_id ; la projection de l'activation désigne cette tentative courante à la révision évaluée. Modifier l'attendu exige une nouvelle version autorisée du mandat/contrat et un nouvel engagement distinct, sans transformer l'ancienne exigence en réussite. Le moteur ne recherche jamais la tentative la plus favorable. La précédente reste inspectable. À clôture, il compare l'observation à l'action et au binding courants ; une sélection de preuve ne modifie pas ceux-ci. Toute divergence de sujet, commande, suite, environnement, parseur ou référence d'attendu produit `fail` avec motif de correspondance. Référence manquante, source purgée, collecte incomplète ou observation authentifiée contradictoire du même fait produisent `unknown`, jamais choix arbitraire.

Après correspondance complète, `test_execution_matches` vaut pass exactement si le processus est `exited`, son code égale `expected_exit_code`, le parseur est `complete`, le total atteint `min_cases` et les compteurs satisfont `total=passed+failed+skipped`. Si `require_all_passed=true`, `passed=total` et `failed=skipped=0` sont requis. Une projection malformée ou des compteurs incohérents sont d'abord refusés comme preuve invalide, sans calcul de vérité. Pour une projection valide, chaque condition de résultat vaut pass/fail/unknown : `terminated` ou un code différent vaut fail ; processus inconnu ou parsing incomplet vaut unknown pour la condition concernée. Leur conjonction V1 vaut fail dès qu'une condition connue échoue, sinon unknown dès qu'une condition est inconnue, sinon pass. La suite testée n'est pas annoncée exhaustive au-delà de sa définition épinglée.

### Claims fermés et contradictions

`Claim` conserve auteur authentifié, sujet, texte explicatif, références d'observations et proposition discriminée. Le texte n'est jamais interprété comme prédicat. `kind=qualitative` reste enregistrable et sa vérité observable vaut `unknown`. La forme observable V1 est limitée à deux prédicats ; tout autre nom/version est refusé à l'enregistrement typé ou conservé explicitement comme qualitatif, sans conversion silencieuse.

| Prédicat/version | Valeur affirmée | Projection déterministe |
|---|---|---|
| `process.exit_code` / 1 | Entier signé 32 bits | Code collecté d'un processus `exited`, après correspondance à la commande liée ; sinon unknown |
| `test.expectation_satisfied` / 1 | Booléen | Résultat de la politique de test de l'attendu lié : vrai pour pass, faux pour échec de résultat connu, unknown si preuve ou correspondance insuffisante |

`claim_predicate_matches` fixe `predicate_ref` et `expectation_slot` dans la règle publiée. Le claim doit porter ce même prédicat, le même attendu et le binding sélectionné par le serveur. Le reconciler consulte les observations autorisées de l'action liée, y compris celles omises de `observation_ids` ; le modèle ne peut faire disparaître un échec en choisissant sa liste de références. Une projection connue égale à `asserted_value` produit pass ; une projection connue différente produit fail/`ClaimContradicted`, en conservant les deux objets. Un mauvais lien ou sujet est un refus de correspondance et n'est pas présenté comme preuve que l'affirmation est fausse. Ce refus vaut `fail` de la règle avec motif `EvidenceExecutionMismatch`, jamais `ClaimContradicted` ; la vérité de la proposition reste `unknown`. Une proposition qualitative, une preuve absente ou une projection non vérifiable produit unknown. Modifier seulement `statement` ne change aucun verdict.

Un claim honnête « la suite échoue » peut satisfaire le contrôle de vérité. Cela ne satisfait pas une obligation de réussite : lorsqu'elle est requise, le contrat inclut séparément `test_execution_matches` obligatoire. Lorsqu'un profil exige la vérité observable d'un claim, sa liste `required_rules` contient le digest d'une règle `claim_predicate_matches` obligatoire avec son prédicat ; `claim_evidence_links`, une règle informative ou une prose ne peuvent la remplacer. Les fixtures de publication vérifient ce raccord avant toute invocation. Aucun juge LLM n'est nécessaire à ces comparaisons.

### Parseurs et limites V1

Ces choix sont arrêtés par [ADR-07](../architecture/adr/ADR-07-contract-enforcement.md) (R7).

- JSON Schema 2020-12 est évalué par la crate `jsonschema`, sans résolution réseau : seul le registre local d'URN est consulté. La canonisation RFC 8785 utilise une implémentation existante (candidate `serde_jcs`), qualifiée sur les vecteurs de test de la RFC.
- Limites par défaut d'une instance évaluée : 1 MiB, profondeur 64 et au plus 128 règles par contrat ; au-delà, `SchemaLimitExceeded`. Un artefact plus grand est transmis par référence et digest.
- Parseurs de tests V1 : `parser.junit-xml` v1 (pytest `--junitxml`, Maven Surefire, Gradle, jest-junit, Vitest junit, cargo-nextest JUnit) et `parser.go-test-json` v1 (`go test -json`). La sortie texte de `cargo test` et le JSON instable de libtest ne forment pas une preuve complète : leur parsing est `incomplete`, donc `unknown`. Un projet Rust utilise cargo-nextest avec un profil JUnit.
- Un nouveau validateur, prédicat de claim ou parseur est du code Rust versionné, livré avec fixtures positives, négatives et inconnues, et une nouvelle version de catalogue ; il n'est jamais déduit du Markdown d'un skill. Publier une règle dont le validateur manque au catalogue épinglé échoue `UnknownValidator`.

## Recherche préalable et clôture

Un contrat peut déclarer qu'une ressource identifiable créée doit être couverte par un reçu de découverte antérieur. Les catégories sont celles de [F009](shared-knowledge.md#discovery--invariant-transversal), y compris automatisations et ressources techniques. Le reçu rapporte sources et couverture, y compris nonmergé ou index périmé. La décision de créer malgré un similaire doit avoir une justification ; le moteur en contrôle la présence et les références, pas la justesse sémantique universelle.

La vérification associe le reçu à l'opération observée par mission, scope, catégorie et séquence d'événements. Une recherche sur un autre projet ou postérieure à la création ne satisfait pas silencieusement l'exigence. La politique peut accepter une recherche tardive avec dérogation attribuée ; cela reste un résultat distinct et visible, jamais un pass normal.

Le reçu applique le vocabulaire canonique [du modèle](../architecture/skills-contract-model.md) : `decision` reuse/create porte l'identité cible ; `strategy` précise use_existing/coordinate/wait/copy/adapt/new. Un adapt sur la même ressource est reuse, une adaptation créant une ressource distincte est create. Le validateur contrôle les couples autorisés, `candidate_ids`, `target_resource_id` et l'attribution du nouvel ID ; il ne réintroduit pas `adapt` comme troisième décision.

Le [contrat discovery F009](shared-knowledge.md) fait autorité sur `origin` discriminée invocation/replay/control_plane, `source_refs` typées et `validation_basis`. Une création manuelle n'exige aucune invocation artificielle. F008 vérifie à l'effet le binding du reçu à l'opération, au payload et à la cible ; le passage à `dispatching` recontrôle sources/cible/scope/politique/ACL et éligibilité, en plus des bornes et fences. Un changement bloque. À clôture, il exige `DiscoveryUseObservation` reliant autorisation préalable, reçu, opération, cible/version/digest effectifs et ordre des événements. La fraîcheur enregistrée à `checked_at` n'est pas écrasée par la projection de fraîcheur actuelle.

Le contrôle final réconcilie cette preuve historique avec les sources, ACL, bornes et cible livrée. Seul l'effet propre exact prouvé, ou une publication identique prouvée, bénéficie de l'exception F009 ; être du même auteur ou de la même mission ne suffit pas. Une modification pertinente exige un nouveau reçu `purpose=completion_revalidation` avec `supersedes_receipt_id`, `creation_use_id` et cible finale, explicitement postérieur. Ce reçu ne répare jamais une recherche préalable absente. `posthoc_reconciliation` reste une dérogation distincte ; issue inconnue, borne expirée ou provenance manquante produit unknown/refus/revalidation selon le contrat, pas pass normal.

## Publications et autorisations

N+2 peut créer le contrat d'un skill, déléguer sa rédaction et décider la publication. Il ne peut augmenter les grants de sa propre identité. La permission effective est l'intersection des plafonds actuels ; toute révocation prend effet pour les actions futures. Les descriptions d'outil et annotations externes sont des données, pas une autorité. La frontière AI-Guard suit [ADR-10](../architecture/adr/ADR-10-ai-guard-boundary.md) (Q-04, R7) : quand AI-Guard est configuré, le GuardPort versionné évalue toute action hors `local_read` et échoue fermé ; le moteur de contrats ne présume ni nom d'endpoint ni capacité réelle avant qualification du mapping.

Une publication peut utiliser fixtures déterministes positives et négatives, analyse de compatibilité et rapport de permissions sans second juge LLM. Si une évaluation par modèle est ajoutée, elle est enregistrée comme jugement avec méthode, modèle effectif et limites. La version de prompt, schéma, outils, contrat et ACL déclarée est indivisible.

## Erreurs et conservation

Les raisons d'erreur du moteur sont `UnknownValidator`, `InvalidRuleParameters`, `UnknownClaimPredicate`, `SchemaLimitExceeded`, `UnauthorizedAction`, `ExpectedExecutionMissing`, `EvidenceExecutionMismatch`, `EvidenceProjectionInvalid`, `EvidenceAmbiguous`, `EvidenceMissing`, `EvidenceSnapshotMismatch`, `ClaimContradicted`, `DiscoveryRequired`, `StaleEvidence` et `ContractVersionConflict`. Elles contiennent règle, sujet, phase et références inspectables, avec redaction des secrets. Les classes IPC restent celles de l'enveloppe commune ; ces noms sont des raisons métier, enregistrées dans la table ci-dessous au format commun de l'[IPC](../architecture/ipc-and-apis.md) (nom de domaine, classe/reason_code, retry_class), avec une seule classe par reason_code. Dans un `ContractVerdict`, le même nom reste le motif d'un résultat de règle `fail` ou `unknown` ; il ne devient une Error que lorsqu'une commande ou une admission est refusée. `DiscoveryRequired` (ContractViolation/DISCOVERY_REQUIRED) est le refus avant effet d'une opération structurée de création sans reçu admissible ; à la clôture, l'absence de trace pour une création identifiable produit `discovery_evidence_missing` (ContractViolation/DISCOVERY_EVIDENCE_MISSING) selon [F009](../features/F009-knowledge-engine/interfaces.md). L'échec ne détruit ni claim ni observation. La conservation respecte ACL, rétention et demandes de suppression ; une preuve purgée devient indisponible avec tombstone, pas faussement valide.

| Nom de domaine | Wire (classe/reason_code) | retry_class |
| --- | --- | --- |
| UnknownValidator | InvalidArgument/UNKNOWN_VALIDATOR | never |
| InvalidRuleParameters | InvalidArgument/INVALID_RULE_PARAMETERS | never |
| UnknownClaimPredicate | InvalidArgument/UNKNOWN_CLAIM_PREDICATE | never |
| SchemaLimitExceeded | InvalidArgument/SCHEMA_LIMIT_EXCEEDED | never |
| UnauthorizedAction | CapabilityDenied/UNAUTHORIZED_ACTION | never |
| ExpectedExecutionMissing | ContractViolation/EXPECTED_EXECUTION_MISSING | after_human_decision |
| EvidenceExecutionMismatch | ContractViolation/EVIDENCE_EXECUTION_MISMATCH | never |
| EvidenceProjectionInvalid | ContractViolation/EVIDENCE_PROJECTION_INVALID | never |
| EvidenceAmbiguous | ContractViolation/EVIDENCE_AMBIGUOUS | after_human_decision |
| EvidenceMissing | NotFoundOrHidden/EVIDENCE_MISSING | never |
| EvidenceSnapshotMismatch | ContractViolation/EVIDENCE_SNAPSHOT_MISMATCH | after_refresh |
| ClaimContradicted | ContractViolation/CLAIM_CONTRADICTED | never |
| DiscoveryRequired | ContractViolation/DISCOVERY_REQUIRED | never |
| StaleEvidence | ContractViolation/STALE_EVIDENCE | after_refresh |
| ContractVersionConflict | RevisionConflict/CONTRACT_VERSION_CONFLICT | after_refresh |

`StaleEvidence` garde la ligne enregistrée par l'[IPC](../architecture/ipc-and-apis.md). `EvidenceMissing` reprend la projection NotFoundOrHidden/EVIDENCE_MISSING déjà enregistrée par [F016](../features/F016-reporting-observability/interfaces.md), car un reason_code n'a qu'une classe. `SchemaLimitExceeded` et `DiscoveryRequired` ont la même projection dans les registres [F007](../features/F007-skill-studio-registry/technical-design.md). `discovery_evidence_missing` reste enregistré par F009.

Les exigences et critères sont dans [F008](../features/F008-contract-engine/README.md). Un résultat de contrat favorable est une preuve limitée à son domaine observé, jamais une promesse d'absence de bugs.
