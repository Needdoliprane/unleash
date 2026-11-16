# Moteur de workflow déterministe

## Responsabilités et dépendances

Le moteur Rust reçoit une définition publiée, des commandes autorisées et des événements observés. Son noyau calcule le nouvel état et les intentions d'effets. Il ne fait ni réseau, ni appel LLM, ni lecture Git pendant une réduction. La transaction locale persiste état, événement et outbox ; les adaptateurs exécutent ensuite les intentions et rapportent leurs observations. Le [DSL](../architecture/workflow-dsl.md) définit les opérations autorisées ; les [états](../architecture/state-machines.md) définissent les transitions.

Un replay logique réapplique des événements déjà observés à la même définition. Il ne répète pas les effets externes. La reproductibilité promise concerne état et intentions ; deux exécutions réelles peuvent recevoir des réponses LLM différentes. Le journal conserve le type de version du reducer afin qu'une migration de stockage ou de sémantique ne soit pas confondue avec un simple changement de workflow.

## Algorithme d'une commande

1. Authentifier l'acteur, décoder et valider taille/schéma, puis calculer le digest de commande incluant `authority_generation`. Vérifier la lignée active **avant** toute recherche de reçu ou tombstone ; une autre lignée produit `StaleFence/STALE_AUTHORITY_GENERATION`, sans réémission automatique.
2. Dans la même lignée, rechercher la clé acteur/`command_id`. Un digest identique retourne le reçu conservé, sans comparer à nouveau `expected_revision` à l'état courant ; un tombstone retourne `IDEMPOTENCY_WINDOW_EXPIRED`, un digest différent `IdempotencyConflict`. Pour une commande nouvelle seulement, vérifier autorité, révision attendue et epoch applicables.
3. Charger l'état, la définition épinglée et les bindings engagés ; vérifier l'evidence bundle référencé et ses préconditions. Revérifier la lignée et les préconditions transactionnelles dans le writer avant toute transition, selon le pipeline canonique [IPC v2](../architecture/ipc-and-apis.md) et [SQLite](../architecture/sqlite-design.md).
4. Appliquer une transition puis les microsteps purs activables dans la limite de 128 ; produire de nouvelles activations ou une continuation.
5. Écrire dans une transaction état révisionné, événements, intentions d'effets, reçu et messages de mailbox concernés. Aucun I/O lent ne se produit dans cette transaction.
6. Réveiller les dispatchers après commit. Une interruption entre commit et notification ne perd pas l'intention, car l'outbox est relue.

Les assertions de règle appartiennent au domaine ; les violations de schéma sont des erreurs d'entrée. Une exception inattendue ne valide aucune transition partielle. Le service marque l'incident, conserve le dernier état durable et refuse de poursuivre sans récupération appropriée.

## Activation, exécution et décision

Une activation obtient une identité stable et réserve ses budgets avant démarrage. Pour une feuille task, le scheduler admet une invocation lorsque sa capacité est disponible ; une feuille action des profils v1.1/v1.2 utilise un replay_worker et le même plafond global, sans session modèle. Les états et règles de slot sont ceux de [session-model](../architecture/session-model.md). Une reprise conserve `invocation_id`, crée une nouvelle `attempt_id` et un nouveau fence, sans autoriser deux tentatives actives. Le worker reçoit manifest effectif, contexte réellement envoyé, mission, step, rôle, skills, scopes, documents sélectionnés et outils autorisés. Chaque appel, action et résultat revient avec invocation, tentative, activation et epoch.

La fin du processus ne signifie pas réussite de la tâche. Le moteur de contrat valide la sortie, les artefacts et traces. Le résultat peut devenir `succeeded`, `failed` ou `waiting_evidence`. Si le workflow inclut une étape de jugement leader, celle-ci reçoit un bundle précis et produit une décision typée. Un N+1 peut ordonner correction ou escalade ; le harness refuse une correction hors borne ou hors scope.

## Retry, temps et budgets

`max_attempts` compte la première tentative. Un retry crée une nouvelle activation liée (tentative logique suivante de la clé d'activation), consomme budget et ne change pas la version du binding ; une reprise après crash conserve l'activation et crée seulement une nouvelle `attempt_id`. Un effet externe inconnu interdit le retry automatique de cette action ; une observation de réconciliation peut confirmer succès, absence d'effet ou besoin d'arbitrage. La politique de retry peut traiter une erreur fournisseur transitoire, mais elle ne suppose aucune garantie exactly-once du fournisseur.

Les deadlines sont matérialisées par événements de timer portant identité et échéance. Au replay, le reducer lit ces événements, pas l'horloge présente. Une pause autorisée enregistre l'ancienne échéance et la règle d'extension ; un crash n'accorde aucun temps gratuit. Les réservations de coût et de tokens précèdent l'appel ; le coût final observé peut être imprécis, ce qui est représenté par estimation et marge, jamais par exactitude inventée.

## Parallélisme et clôture

Les branches du DSL sont logiques ; leur nombre ne garantit pas une exécution simultanée. Le scheduler partage le plafond avec N+1/N+2. Les règles `all`, `any_success` et `quorum` déterminent le seuil logique puis l'arrêt et la réconciliation des branches restantes. La réussite globale attend la garde de done et la clôture effective des effets engagés.

Une version publiée pendant l'exécution ne remplace pas la définition. Le manifest peut recevoir une liaison explicite pour un slot tardif non engagé, validée contre les contraintes déjà publiées ; elle devient immuable. Une modification d'un slot existant, d'un guard ou d'un contrat engagé attend un nouveau run ; aucune migration en place n'existe en V1 (Q-03). L'écran affiche les versions par binding pour ne pas faire croire que tout le run a changé.

## Erreurs de domaine proposées

| Code | Wire (`code/reason_code`) | Interprétation | Réaction |
|---|---|---|---|
| `RevisionConflict` | `RevisionConflict` | État modifié depuis la décision | Recharger puis décider ; pas retry aveugle |
| `IdempotencyConflict` | `IdempotencyConflict` | Même clé avec digest de commande différent | Rejet définitif du doublon |
| `StaleEvidence` | `ContractViolation/STALE_EVIDENCE` | Preuve hors snapshot/révision | Recollecter ou expliquer l'obsolescence |
| `StaleEpoch` | `StaleFence/STALE_EPOCH` | Résultat ou commande sous epoch révoqué | Réconcilier avec l'owner actuel ; aucune réémission |
| `UnresolvedEffect` | `OutcomeUnknown/UNRESOLVED_EFFECT` | Effet engagé non réconcilié | Bloquer action/successeur concerné |
| `BudgetExhausted` | `BudgetExceeded/BUDGET_EXHAUSTED` | Réservation impossible | Arrêt contrôlé, motif visible |
| `LoopExhausted` | `ContractViolation/LOOP_EXHAUSTED` | Borne de `repeat` atteinte | Échec du nœud ; aucune itération supplémentaire |
| `ContractRejected` | `ContractViolation/CONTRACT_REJECTED` | Sortie refusée par le contrat | Correction ou échec selon le DSL |
| `BindingAlreadyEngaged` | `ContractViolation/BINDING_ALREADY_ENGAGED` | Substitution demandée sur slot lié | Préparer autre run ; conserver version |
| `TerminalImmutable` | `ContractViolation/TERMINAL_IMMUTABLE` | Mutation d'un run terminal | Rejet ; créer un successeur explicite |
| `PredecessorNotTerminal` | `ResourceBusy/PREDECESSOR_NOT_TERMINAL` (retry_class=after_refresh) | Successeur demandé avant l'état terminal du prédécesseur (Q-03) | Attendre l'état terminal |
| `SuccessorExists` | `InvalidArgument/SUCCESSOR_EXISTS` | Le prédécesseur a déjà un successeur direct (Q-03) | Rattacher au dernier Run de la lignée |
| `GuardUnknown` | Raison de blocage `GUARD_UNKNOWN` (état, pas rejet de commande) | Information nécessaire absente | Attendre preuve bornée par deadline |
| `DefinitionRevoked` | `CapabilityDenied/DEFINITION_REVOKED` | Nouveaux démarrages interdits | Refuser nouveau run ; traiter courant selon motif de révocation |

Ces raisons sont enregistrées par F005 dans l'Error commun de [l'IPC](../architecture/ipc-and-apis.md). La couverture REQ→AC→T→EV est dans [F005](../features/F005-workflow-engine/requirements.md). Le moteur ne promet pas l'absence de bugs de l'application produite ; il rend contrôlables les règles qu'il sait observer.

## Profils d’exécution R3–R5

Le profil v1 reste lisible et immuable. Le [profil v1.1](../architecture/automation-execution-profile.md) ajoute les actions d’opérations versionnées sans agent et une politique model_policy ; il conserve le même reducer, les mêmes frames et les mêmes conditions de quiescence. Une tâche de supervision est une feuille task explicite. Le choix replay/supervised est matérialisé avant admission, jamais modifié en vol.

Les champs InvocationBundle s’appliquent aux invocations agentes ; les workers mécaniques portent un ExecutionManifest de procédure, données, opérations, contrats, scope, budget et driver. Les événements et preuves relient les deux formes à run/activation/action. Un effet inconnu conserve sa barrière, même si la collecte d’ObservationReport a réussi. Les rapports déterministes et la clôture mechanical/human sont utilisables sans N+1 ni N+2.
