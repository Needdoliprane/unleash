# F019 — Design technique proposé

**Statut : conception R3/R5 complétée par les décisions R7 Q-15 et Q-18, pas implémentation.** Les noms de modules et de DTO sont des propositions Unleash. Les capacités des moteurs externes restent celles de leurs contrats qualifiés, cités dans [les sources navigateur](../../research/browser-automation-sources.md).

## Réutilisation du cœur existant

`AutomationRegistry` réutilise les objets/versionnements de F002/F007. `AutomationCompiler` construit un workflow du profil exact [v1.1](../../architecture/automation-execution-profile.md) ou [v1.2](../../architecture/workflow-data-iteration.md), obligatoire pour for_each, résout schémas, contrats et catalogue, puis calcule les bornes. `AutomationDataImporter` produit un snapshot typé à partir des fichiers autorisés. Ces composants sont Rust. Aucun d'eux n'exécute une instruction libre du draft comme commande système.

`WorkflowReducer`, `RunRepository`, `EffectDispatcher`, scheduler, broker et contrats restent les composants de F005/F008/F017. Une action reçoit une activation stable et émet une intention d'effet par la même outbox. Un worker de replay éphémère exécute les opérations autorisées ; il n'est pas une identité d'agent et ne contient pas de boucle de raisonnement. Les drivers sont des ports versionnés ; le [BrowserPort](../../architecture/browser-execution.md) est l'un d'eux, pas le centre d'un nouvel orchestrateur. Le backend navigateur de référence V1 est l'adaptateur Rust `unleash-browser` en WebDriver sur `fantoccini`, avec chromedriver et Chrome for Testing épinglés par version et SHA-256 ; chromedriver est lancé et suivi par F011, et la préparation du runtime est une étape explicite sous mandat, sans commande dédiée ([Q-18](../../brief/open-questions.md)).

`AutomationRun` enrichit la projection de `Run` avec source d'artefact, révision de données, mode, observations par ligne, profil de driver et preuves de politique modèle. La source de vérité demeure le run et son journal. Les clients ne maintiennent ni état de progression autoritaire ni liste de retries indépendante.

## De l'apprentissage à la publication

1. Rechercher les automatisations existantes ; attacher le reçu de discovery et la décision de réutilisation/création. Fixer l'objectif observable, le scope, les exemples de données et les limites d'effet autorisées.
2. Créer un draft. `LearnAutomation` peut matérialiser une invocation de conception avec rôle/skills exacts, accès aux observations autorisées et budget modèle. Une construction ou correction manuelle reste possible sans modèle.
3. Stocker séparément trace observée, paramètres proposés, claims du générateur et résultats attendus. La procédure candidate explicite étapes, locators, identités métier, données, contrats, effets et stratégies de reprise. Aucun succès de trace ne vaut autorisation d'action nouvelle.
4. Valider la forme et les références, compiler, calculer toutes les bornes, vérifier les capacités et analyser les dépendances modèle. Vérifier aussi les chemins d'exception et les sorties typées de supervision lorsqu'ils existent.
5. Évaluer le digest exact du candidat sur une cible déclarée et autorisée. La simulation sans effet, une fixture, un service de test et un site réel sont des catégories distinctes. « Dry run » ne permet pas d'affirmer qu'un clic sur un site arbitraire est sans effet. Les évaluations exécutées utilisent le même reducer et le broker sous une capacité d'évaluation bornée ; le snapshot de candidat n'est pas publié pour autant.
6. Publier atomiquement la version et ses références si les gates de contrat sont satisfaites. La publication peut suivre une politique déjà autorisée ; elle ne requiert pas universellement un clic d'approbation. Les lacunes et contradictions non résolues bloquent les gates concernées.

Une dépendance de moteur ou de runtime peut être réutilisée au lieu d'être réécrite en Rust. Le bundle enregistre version, provenance, licence, lockfile/digest et profil qualifié. Une recommandation de dépendance ne constitue pas une compatibilité testée. Les références de l'[artefact](../../architecture/automation-artifact-model.md) sont résolues avant admission et conservées à la reprise.

## Paramètres et lots

L'import crée `AutomationDataRevision` avec octets bruts, parsing déclaré, valeurs normalisées, mapping et IDs de lignes. Toute révision est validée avant son premier effet métier, notamment types, unités, doublons, tailles, nombre de lignes et expansion maximale du plan. Un mapping de paramètres est une projection typée ; il n'exécute pas de formule, d'interpolation shell ou de prompt.

Une ligne rejoint la projection d'entrée par une référence immuable. Le curseur de lot et les résultats déjà observés sont durables. L'ordre est déclaré et stable ; du parallélisme entre lignes n'est possible que si les scopes et le contrat d'effets l'autorisent. Deux lignes visant le même compte, panier ou objet muté doivent respecter un verrou/ordre explicite. Les clés de ligne n'accordent pas à elles seules l'indépendance des effets.

Corriger le CSV crée une nouvelle révision ; le run actuel ne la découvre pas automatiquement. Un nouveau run sélectionne explicitement la nouvelle révision. Le contrôle des incidents et clés métier peut bloquer des effets déjà engagés dans un autre run, y compris si le nouvel import change l'ordre ou les IDs techniques des lignes.

## Replay et supervision : graphes explicitement différents

En `automation_mode=replay`, le workflow effectif est entièrement mécanique, `model_policy=forbidden`, bindings vides et budgets modèle/invocations agentes nuls. Les contrôles d'autorité, entrées, locators, contrats, réconciliation et rapports sont déterministes. Un champ textuel « aide-moi » dans une erreur ou un CSV ne déclenche aucune invocation.

En `automation_mode=supervised`, la version sélectionnée contient des blocs d'actions et des tâches `leader_decision` explicites, avec bindings de rôle/skill exacts. Les points de supervision sont des checkpoints nommés, des lots de lignes ou des routes d'exception anticipées. Le nombre d'invocations, les appels par invocation, les tokens, le coût et les délais sont plafonnés. Le moteur ne sollicite pas un LLM à chaque clic, ni à chaque message de log ; il ne génère pas une tâche supplémentaire pour rédiger un rapport ordinaire.

Une opération d'observation peut clore sa collecte en produisant un résultat typé `observed`, `unknown` ou `expected_failure`, avec `safe_to_continue`, `effect_state` et références d'assertions. Cela signifie que la collecte est terminée, pas que l'objectif métier est atteint. Un `choice` déjà publié dirige alors vers une tâche de supervision éventuelle. Les erreurs non représentées dans ce contrat gardent leurs états failed/blocked usuels. Il n'existe pas de `catch` implicite qui invente un superviseur.

Le superviseur reçoit un evidence bundle à révision précise : logs structurés, données autorisées, sorties d'assertions, snapshots et effets en cours. Sa sortie suit un contrat fermé :

| Décision | Suite permise |
|---|---|
| `continue` | Sélectionner la prochaine branche déclarée si les assertions, préconditions et autorisations nécessaires sont satisfaites. |
| `reconcile` | Demander les observations ou opérations de réconciliation déjà autorisées ; conserver le blocage de toute répétition d'effet inconnu. |
| `correct` | Choisir une correction prépubliée, ou ouvrir un job de réparation distinct si un mandat valide le permet ; aucune édition du script ou des entrées engagées. |
| `escalate` | Créer une attente durable et une demande compréhensible à l'autorité compétente. |

Le moteur vérifie cible, contrat de sortie, fraîcheur et scope. Une décision `continue` contredisant une assertion obligatoire est refusée ; un log affirmant « tout est bon » n'est pas une preuve du résultat serveur. Un état `outcome_unknown` permet lecture/réconciliation/escalade selon mandat, jamais un retry aveugle ou une clôture fictive. Aucun jugement modèle universel n'est ajouté à un workflow dont le done mécanique suffit.

## Enforcement de la politique modèle

La politique effective est dérivée de l'origine d'exécution et de ses autorités, pas choisie par le worker. Chaque intention d'effet et descendant conserve cette origine ; une origine absente ou non vérifiable refuse le dispatch au lieu d'accorder une politique permissive. Avant dispatch, le broker, le routeur modèle et les adaptateurs vérifient `model_policy` et les capacités réelles. Sous `forbidden`, appels texte/vision, embeddings, reclassements, modèles locaux, sampling, compactions et résumés générés sont refusés, même avec un coût nul ou une clé fournie dans un fichier.

Les processus de projet n'obtiennent ni secrets modèle, ni socket administratif. Toute sortie réseau du navigateur passe par le proxy d'egress d'Unleash, qui applique les origines du manifest à chaque requête et redirection. Leur profil qualifié contrôle réseau, processus et descendants ; les routes autorisées ne se réduisent pas à une blocklist de fournisseurs connus. Un profil qui ne peut démontrer ce contrôle n'est pas attesté sans modèle et retourne `IsolationUnavailable` pour cette qualification. La frontière exacte est décrite dans F017 et BrowserPort ; un contexte navigateur isolé n'est pas une sandbox OS.

Les rapports capturent deux compteurs distincts : tentatives modèle refusées et dispatchs modèle effectivement autorisés/observés. Zéro token déclaré n'est pas une preuve de zéro appel : un appel peut échouer avant de retourner l'usage. La campagne combine instrumentation locale, providers de fixture indépendants et observation d'egress. En mode supervisé, les appels autorisés sont liés aux checkpoints, à leur manifest et à leurs décisions, jamais retranchés pour préserver un badge zéro modèle.

## Admission et ressources

La décision Q-15 ([ADR-13](../../architecture/adr/ADR-13-scheduler-budget.md)) utilise un seul pool global utilisateur de 5 unités actives par défaut, maximum 50, partagé entre invocations agentes, workers de replay et probes fournisseurs. Une action pure du reducer ne consomme pas un worker. Un `replay_worker` est admis par activation `kind=action` (`replay_workers.step_id`) ; « plusieurs actions séquentielles » désigne les actions journalisées d'une même activation, par exemple un `project_script`. Il garde son unité tant qu'il peut agir et la rend à la clôture de l'activation ou après checkpoint sûr. Les branches exécutées par des workers distincts réservent chacune leur unité.

`max_concurrent_action_executions` est un sous-plafond du run. Les activations cumulées restent sous `max_step_activations` et les retries dans leurs bornes. Les ressources supplémentaires comprennent contexte navigateur, processus, mémoire, CPU selon capacités OS, volume de données et coût d'effets non modèle. Le `browser_context_limit` vaut 2 par défaut ([ADR-16](../../architecture/adr/ADR-16-reusable-automation-and-supervision.md), amendement R7) ; ce défaut reste à mesurer en recette, n'est pas une promesse matérielle et n'est effectif que dans les limites qualifiées.

Le writer admet unité, scopes et ressources ensemble. À une attente humaine ou de supervision, le worker ne libère son unité qu'après checkpoint et neutralisation de sa continuation. Le contexte navigateur éventuellement conservé garde ses réservations mémoire/processus. Conservé entre activations, il est une réservation `capacity_reservations` du pool navigateur dont le propriétaire est le Run (`owner_type=run`, `owner_id=run_id`), liée à sa génération et à la lease du périmètre métier ; chaque activation suivante l'utilise sous un nouveau fence, réobserve page et identité et ne réutilise aucun handle éphémère. La tâche `leader_decision` acquiert ensuite son propre permis agent dans le même pool. Les parents en attente ne retiennent pas toutes les places nécessaires aux enfants.

## Effets, observations et reprise

L'intention durable précède le dispatch : activation/ligne, paramètres résolus, empreinte de cible, observation préalable, identité logique d'effet, policy revision, fence et stratégie de reprise. L'identité ne dépend pas d'une tentative de processus remplaçable. Une stratégie fournisseur idempotente n'est retenue qu'avec contrat vérifié ; une suite de clics opaque ne l'est pas par convention.

Après dispatch, un receipt ou une observation nouvelle classe l'effet. Un timeout ne prouve pas l'absence de mutation. C4/C5 de F018 imposent `outcome_unknown` puis lecture de l'état distant ou décision autorisée. La déduplication locale des commandes et celle des observations ne prétendent pas assurer exactly-once au service externe. Une réparation ou un nouveau CSV ne réinitialise pas cette obligation.

Le checkpoint enregistre frame d'AST, curseur de ligne, résultats, preuves, versions, budgets et ressources. Après crash, le nouveau worker reprend sous une nouvelle tentative/fence, inventorie l'ancien exécuteur, recrée les ressources nécessaires et réobserve la cible. Il ne réutilise pas un handle DOM, une page RAM ou une authentification supposée valide. Si l'ancien exécuteur peut encore agir, les réservations pertinentes restent en réconciliation.

## États et erreurs

Les états de run, action et ressource restent séparés. Un run peut attendre une authentification sans effet inconnu ; un effet inconnu peut persister après une intention d'annulation. Les erreurs utilisent le format wire commun de F001.

| Situation | Classe/raison proposée et comportement |
|---|---|
| Mode incompatible avec la politique ou graphe | `ContractViolation/AUTOMATION_MODE_POLICY_MISMATCH` ; aucune admission. |
| CSV ou mapping invalide | `InvalidArgument/AUTOMATION_INPUT_INVALID` ; erreurs par ligne/colonne, zéro effet métier. |
| Version/digest essentiel absent | `ContractViolation/AUTOMATION_BUNDLE_UNAVAILABLE` ; aucune substitution. |
| Appel modèle sous forbidden | `CapabilityDenied/MODEL_CALLS_FORBIDDEN` ; refus durable, aucun dispatch. |
| Budget de supervision atteint | `BudgetExceeded/SUPERVISION_BUDGET_EXHAUSTED` ; arrêt/attente explicite selon le contrat, sans repli de provider. |
| Cible absente ou non unique | `ContractViolation/AUTOMATION_TARGET_MISSING` ou `ContractViolation/AUTOMATION_TARGET_AMBIGUOUS` ; blocage avant engagement. |
| Identité métier de la cible dérivante | `ContractViolation/AUTOMATION_TARGET_DRIFT` ; blocage avant engagement lorsque la divergence est observable. |
| Postcondition métier fausse | `ContractViolation/BUSINESS_POSTCONDITION_FAILED` ; échec métier si effet connu, réconciliation si état encore ambigu. |
| Effet potentiellement accompli | `OutcomeUnknown/UNRESOLVED_EFFECT` ; aucune répétition aveugle. |
| Authentification/challenge à traiter | Run `blocked` (`NeedEvidence`, `AUTOMATION_HUMAN_STEP_REQUIRED`), étape `waiting_evidence`, unité libérée après checkpoint sûr ; une commande refusée pendant l'attente retourne `ResourceBusy/AUTOMATION_HUMAN_STEP_REQUIRED`, `retry_class=after_human_decision`. La fin d'intervention est déclarée par `AcceptStep` avec révision et evidence bundle ; elle n'accepte pas l'étape et autorise seulement une activation d'observation publiée (session, compte, mandat, préconditions) sous nouveau fence. Seule une observation conforme produit `ResolveBlock` ; sinon le run reste `blocked`. Aucune clé ou réponse inventée. |
| Confinement demandé absent | `IsolationUnavailable/AUTOMATION_PROFILE_UNAVAILABLE` ; refus de la garantie annoncée. |
| Référence de preuve devenue périmée | `ContractViolation/STALE_EVIDENCE` ; nouvelle observation ou décision requise. |

### Noms de domaine du profil navigateur et forme wire

Les noms de la [spécification navigateur](../../specs/browser-automation.md#7-erreurs-et-comportements-attendus) décrivent la raison. Sur le wire, le nom de domaine ne passe pas : l'Error commun porte la classe (`code`), le `reason_code` et le `retry_class` enregistrés ci-dessous, avec `details_ref` le cas échéant, selon la règle de l'[IPC commun](../../architecture/ipc-and-apis.md#vocabulaire-public-et-formes-internes). Cette table fait autorité pour F019 ; une raison transverse de l'IPC y garde la `retry_class` de sa ligne.

| Nom de domaine | Forme wire `code/reason_code` | `retry_class` |
|---|---|---|
| `InvalidAutomationData` | `InvalidArgument/AUTOMATION_INPUT_INVALID` | `never` |
| `BrowserRuntimeMismatch`, `BrowserCapabilityMissing` | `CapabilityDenied/CAPABILITY_UNAVAILABLE` ; capacité ou runtime manquant en `details_ref`. | `after_human_decision` |
| `AuthenticationRequired` | État Run `blocked` (`NeedEvidence`, raison `AUTOMATION_HUMAN_STEP_REQUIRED`, étape `waiting_evidence`), qui n'est pas une Error ; une commande refusée pendant l'attente retourne `ResourceBusy/AUTOMATION_HUMAN_STEP_REQUIRED`. | `after_human_decision` (commande refusée) |
| `AccountMismatch` | `ContractViolation/AUTOMATION_ACCOUNT_MISMATCH` | `after_human_decision` |
| `TargetMissing` | `ContractViolation/AUTOMATION_TARGET_MISSING` | `after_human_decision` |
| `AmbiguousTarget` | `ContractViolation/AUTOMATION_TARGET_AMBIGUOUS` ; `AUTOMATION_TARGET_DRIFT` reste réservé à une identité métier divergente. | `after_human_decision` |
| `BusinessConstraintViolated` | `ContractViolation/BUSINESS_CONSTRAINT_VIOLATED` avant effet ; `ContractViolation/BUSINESS_POSTCONDITION_FAILED` après effet. | `after_human_decision` |
| `ExternalStateChanged` | `ContractViolation/EXTERNAL_STATE_CHANGED` | `after_reconciliation` |
| `ActionabilityTimeout`, `AssertionTimeout` | `ContractViolation/ACTIONABILITY_TIMEOUT`, `ContractViolation/ASSERTION_TIMEOUT` avant dispatch ; `OutcomeUnknown/UNRESOLVED_EFFECT` si l'effet peut avoir eu lieu. | `after_backoff` avant dispatch ; `after_reconciliation` pour `UNRESOLVED_EFFECT` |
| `OutcomeUnknown` | `OutcomeUnknown/UNRESOLVED_EFFECT` | `after_reconciliation` |
| `IsolationUnavailable` | `IsolationUnavailable/AUTOMATION_PROFILE_UNAVAILABLE` | `after_human_decision` |
| `ModelUseForbidden` | `CapabilityDenied/MODEL_CALLS_FORBIDDEN` | `never` |

Les autres raisons propres de la table « États et erreurs » sont enregistrées dans le même format : `ContractViolation/AUTOMATION_MODE_POLICY_MISMATCH` → `never` ; `ContractViolation/AUTOMATION_BUNDLE_UNAVAILABLE` → `after_human_decision` ; `BudgetExceeded/SUPERVISION_BUDGET_EXHAUSTED` → `after_human_decision` ; `ContractViolation/AUTOMATION_TARGET_DRIFT` → `after_human_decision`. `ContractViolation/STALE_EVIDENCE` garde la `retry_class` transverse `after_refresh` de l'IPC.

Un appel modèle refusé sous `forbidden` porte la même raison `MODEL_CALLS_FORBIDDEN`, quel que soit le composant qui le détecte (Model Router F012, compilateur F006 ou adaptateur).

## Réparation et cycle de vie

`RepairAutomation` prend version source, incident/observations et mandat de conception. Il crée un draft lié ; une politique préautorisée peut émettre cette commande dans un périmètre et un budget précis. Le job de conception possède sa propre origine et peut utiliser un LLM ; ses coûts restent visibles dans la mission. L'existence d'un incident ne permet pas au worker interdit de créer lui-même une origine autorisée.

La version réparée suit les mêmes validations et publication. Le run ancien reste épinglé et atteint une frontière de quiescence/réconciliation. Un nouveau run sélectionne la nouvelle version ; aucune ligne n'est réessayée silencieusement. La dépréciation limite les futures sélections ; la révocation s'applique aux prochaines actions via le broker. La purge conserve les tombstones et signale l'impossibilité de reconstituer/reprendre lorsque des objets requis ont disparu.

## Admission d’un candidat à l’évaluation

EvaluateAutomationDraft fige les octets du candidat, compile un snapshot de workflow immuable et le conserve dans le stockage de versions avec visibilité qualification_only. Le Run commun référence ce workflow exact avec purpose=evaluation, le digest du candidat, les données, la cible et le mandat d’évaluation. Il n’est pas un AutomationRun de source automation_version, puisque l’artefact n’est pas encore publié. Le cycle de vie, les réservations, les actions et les preuves restent ceux du moteur commun.

CreateRun/StartRun en usage opérationnel refusent une version qualification_only ; seule l’autorité d’évaluation peut l’admettre sous le purpose prévu. Une cible live reste soumise aux grants et effets exacts annoncés. Publier l’artefact ne réécrit pas cette évaluation : la publication référence son résultat applicable au même digest. Un candidat modifié invalide l’applicabilité des anciennes preuves. Les métadonnées de purpose et de visibilité sont des champs Unleash proposés, à transporter dans le DTO commun ; aucun nouveau verbe de lancement ou moteur de simulation n’est créé.

## Parcours de données R6

Les lots génériques utilisent le [profil v1.2 for_each](../../architecture/workflow-data-iteration.md), explicitement sélectionné et épinglé. Body reçoit une ligne immuable dans sa portée /items ; after_batch reçoit un résumé/référence de résultats de lot. Le curseur et les activations appartiennent au writer/reducer commun, pas au driver. La borne statique suit workflow-data-iteration : 1 + max_items×B(body) + ceil(max_items/batch_size)×B(after_batch), arithmétique vérifiée et refus au-delà de 100 000 ; validation des données et budgets avant premier effet. Une collecte unknown ne peut faire avancer le curseur pour atteindre une supervision future : une route de diagnostic doit être présente à l’endroit réellement bloqué.

## Diagnostic pendant EffectUnknown

La [machine de Run](../../architecture/state-machines.md) conserve blocked et les barrières tout en autorisant microsteps purs et activations diagnostiques publiées à capacités réduites. Un choix ou un avis continue ne lève pas le blocage. Seule une observation/disposition autorisée résout les actions concernées avant ResolveBlock. Route absente : attente des commandes de contrôle ; forbidden : aucune tâche modèle, même pour diagnostic. L’ordre de commit empêche une fenêtre d’admission normale entre collecte et blocage.
