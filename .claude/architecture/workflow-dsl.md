# DSL de workflow — contrat accepté `unleash.workflow.v1` (ADR-04)

## Statut et frontière

Cette spécification définit le langage déclaratif V1 du daemon Rust, accepté par [ADR-04](adr/ADR-04-workflow-semantics.md). Elle ne décrit aucune bibliothèque existante. Les raisons du choix sont dans [l'audit de déterminisme](../audits/workflow-determinism.md) et les précédents externes dans [les sources](../research/workflow-skills-sources.md). Le DSL définit les chemins autorisés et leurs bornes ; un leader exerce son jugement à l'intérieur de points de décision explicites. Une instruction libre ne modifie jamais la sémantique du moteur.

Le format publié est validé par [workflow.schema.json](../schemas/workflow.schema.json), puis par une passe sémantique indispensable. Le schéma décrit la forme ; l'analyse sémantique vérifie unicité, références, scopes, domination des lectures, typage des résultats, permissions et bornes. Un draft NL peut être incomplet, mais ne peut être démarré.

## Enveloppe de définition

| Champ | Type et règle |
|---|---|
| `schema_version` | Constante `unleash.workflow.v1` |
| `workflow_id`, `version` | Identité stable et entier positif ; paire immuable après publication |
| `name`, `description` | Texte affichable ; aucune autorité d'exécution |
| `bindings` | Table locale de slots : fixe avec rôle exact `{id, version}`, ou tardif avec niveau, plafond de capacités et profil de contrat exact |
| `budget` | Durée, activations cumulées, appels LLM, tokens de sortie, coût en micro-USD et `max_agent_executions` = nombre maximal d'invocations agentes simultanément actives du run (sous-plafond de concurrence soumis au pool global) |
| `body` | Un nœud de l'AST fermé ci-dessous |
| `completion` | `success_when`, guard sur les faits disponibles lorsque `body` a terminé |

Le daemon résout les bindings fixes, contrats, schémas et outils au démarrage. Pour un slot tardif, une commande explicite `BindRole` sélectionne une version exacte avant sa première activation et vérifie contrat et plafond de capacités ; `RoleBound` et les `SkillBound` associés complètent le manifest append-only. Aucun binding engagé n'est remplacé, y compris aux retries ou itérations suivantes. Un skill nouveau peut donc servir un slot encore vierge sans réécrire le workflow ni dénaturer une étape engagée. Le manifest contient les digests et ne suit aucun alias de registre. Les bornes DSL sont des plafonds demandés ; le runtime applique le minimum avec les limites de mission, projet et utilisateur. Des limites trop élevées sont refusées ou normalisées avec un diff explicite avant publication, jamais silencieusement pendant une exécution.

La tâche utilise `entry_skill` du rôle lié. La compatibilité tardive suit l'[algorithme exact V1](skills-contract-model.md) : digests input/output égaux au profil, règles obligatoires conservées et extensions seulement restrictives. Les versions de compilateur, évaluateur et catalogue de validateurs sont épinglées dans le manifest avant évaluation, puis conservées à la reprise. Le schéma JSON seul ne vérifie pas ces propriétés.

## Nœuds et résultats

Tous les nœuds possèdent `id` et `kind`. Les identifiants sont uniques dans la définition entière. Chaque activation reçoit une clé stable `(run_id, node_id, iteration_vector, attempt)`, où `attempt` est la tentative logique d'étape (1 à `max_attempts`), distincte de l'`attempt_id` de processus ; la présence de plusieurs workers n'altère pas cette identité.

| `kind` | Champs propres | Sémantique |
|---|---|---|
| `task` | `binding`, `task_type`, `scope_key`, `inputs`, `timeout_seconds`, `max_attempts` | Exécute un skill ou sollicite une décision sous rôle/contrat épinglé ; `task_type` vaut `execute_skill`, `leader_decision` ou `integrate` |
| `sequence` | `steps` non vide | Active le premier enfant puis le suivant après réussite ; l'échec non traité se propage |
| `parallel` | `branches`, `join`, éventuellement `required_successes` | Lance des branches logiques ; les exécutions réelles respectent le scheduler ; applique la règle de jointure |
| `choice` | `cases` ordonnés `{when, then}`, `otherwise` | Sélectionne la première garde vraie ; si toutes sont fausses prend `otherwise` ; une garde inconnue avant la première vraie suspend le choix |
| `repeat` | `body`, `until`, `max_iterations` | Exécute au moins une itération ; réévalue `until` à la fin ; faux poursuit dans la borne, inconnu bloque, borne atteinte échoue avec `LOOP_EXHAUSTED` |
| `fail` | `code`, `message` | Échec local explicite et traçable ; pas d'effet externe |

`scope_key` est une clé locale d'affectation, sans autorité propre. La passe sémantique et `CreateRun` la résolvent contre les scopes exacts et révisionnés du mandat référencé par `scope_ref` (ressources, worktree, chemins). Une clé non résolue refuse le démarrage ; elle ne crée aucune permission et est distincte de `capacity_pools.scope_key`.

Dans les profils v1 et v1.1, les `inputs` d'une task sont des valeurs JSON littérales validées contre l'`input_schema` de l'`entry_skill` liée ; aucune chaîne n'est interprétée comme référence. Un identifiant de nœud tel que la valeur de `review_target` est une donnée : le harness construit l'evidence bundle de la portée lexicale correspondante, sans injection implicite de résultats. Le profil v1.2 suit la même règle ([itération de données](workflow-data-iteration.md)). Une tâche `integrate` peut porter l'entrée facultative `inputs.strategy` (`ff_only`, `merge_commit_or_ff` ou `rebase_then_ff`). Les schémas de workflow ne la typent pas : comme toute entrée de task, elle est validée par l'`input_schema` du contrat d'intégrateur lié, qui doit déclarer cette énumération pour l'admettre. Le harness résout la stratégie effective dans cet ordre : `inputs.strategy`, sinon la politique Git du repository, sinon `merge_commit_or_ff` ([stratégies et conflits](../specs/git-integration.md#stratégies-et-conflits)). Le champ `strategy` du payload de `PrepareIntegration` doit égaler cette valeur, sinon `policy_refused` ; il ne la choisit pas.

Les nœuds réussis exposent `status: "succeeded"` et un objet `outputs`. Pour une tâche, `outputs` est l'output validé du contrat ; il n'est pas créé à partir du résumé du modèle. Une séquence exporte les outputs de son dernier enfant réussi ; un choice exporte les outputs de la branche choisie. Un parallel exporte `outputs.branches`, une table identifiant → résultat complet de branche. Un repeat exporte `outputs.iterations` et `outputs.last_outputs`. Les événements conservent le choix et tous les résultats même lorsque ces projections ne les affichent pas directement. La projection de lecture d'une activation expose `input`, `results` et `facts`. `results/<node_id>` contient le dernier résultat validé visible dans cette portée. L'historique complet conserve toutes les itérations et tentatives ; la projection ne le remplace pas.

Au début de chaque itération, les résultats du sous-arbre répété sont retirés de sa projection active, ce qui empêche de lire un succès de l'itération précédente comme résultat courant. À la sortie de boucle, le résultat du bloc contient `iterations` et `last_outputs` ; les consommateurs extérieurs doivent utiliser ce résultat de bloc, pas dépendre des nœuds internes. Une branche parallèle ne lit pas les résultats en cours d'une autre branche. Après jointure, le résultat du bloc présente les résultats enfants indexés par identifiant et ordonnés pour l'export.

## Guards structurés

La forme élémentaire est `{ "op": "eq", "path": "/results/merge/outputs/integrated", "value": true }`. Les opérations autorisées sont `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in`, `exists`, `all`, `any` et `not`. Les comparaisons d'ordre portent uniquement sur des entiers dans la plage sûre JSON ; les égalités n'effectuent aucune conversion chaîne/nombre/booléen. `in` compare à une liste finie de scalaires homogènes. `in` porte `values` (1 à 64 scalaires) ; `all` et `any` portent `terms` ; `not` porte `term` ; `exists` ne porte que `path` ; la forme exacte est celle du schéma. Le chemin est un JSON Pointer restreint à la projection locale ; ce n'est ni JSONPath, ni expression shell, ni accès fichier.

Un fait absent, périmé, révoqué ou non encore observé produit `unknown` lorsqu'il est nécessaire à la comparaison. `exists` retourne faux pour une absence connue dans une projection complète ; une projection incomplète ou masquée retourne inconnu. `all` est faux si un terme est faux, vrai si tous sont vrais, inconnu sinon. `any` est vrai si un terme est vrai, faux si tous sont faux, inconnu sinon. `not` conserve l'inconnu. Aucune lecture réseau, horloge, génération aléatoire ou invocation LLM n'est autorisée dans un guard.

Une dépendance de guard doit être dominée par une tâche ou par un bloc déjà terminé sur le chemin concerné. Un guard ne peut lire la sortie d'une branche qui n'a pas été sélectionnée ; il doit d'abord tester une condition de branche ou utiliser le résultat structuré de son bloc. L'analyse des types signale le chemin, le type attendu et le producteur. Les tableaux arbitrairement grands, expressions régulières exécutables et transformations programmables ne font pas partie du profil V1.

## Jointures et annulation

`all` exige toutes les branches réussies. `any_success` exige au moins une branche réussie. `quorum` exige `required_successes` entre un et le nombre de branches. Une impossibilité mathématique de satisfaire la condition provoque l'échec du bloc. L'événement de réussite qui atteint le seuil est choisi par l'ordre durable des événements, pas par l'heure murale rapportée par les workers.

Atteindre un seuil déclenche l'annulation des branches restantes. Le bloc ne se ferme pas tant que chaque branche encore capable d'effet n'a pas acquitté l'arrêt ou été réconciliée. Un effet externe au résultat inconnu rend le bloc `blocked` avec raison `UNRESOLVED_EFFECT`. Le moteur peut enregistrer le seuil logique atteint tout en affichant la jointure non clôturée. Un worker tardif ne peut plus émettre d'effet sous un epoch révoqué ; sa preuve peut être archivée sans relancer le nœud.

## Bornes et garanties exactes

Le profil publié impose au plus 256 nœuds, profondeur 16, 50 branches par bloc parallèle, 100 itérations par boucle, profondeur de guard 8 et 64 termes par composition. Ces valeurs sont celles de la V1 (ADR-04) et ne changent que par une version de profil, pas par une clause libre. L'analyse calcule une borne supérieure d'activations en multipliant les répétitions, en additionnant les branches parallèles et en retenant le maximum d'un choice ; si elle dépasse 100 000 ou déborde l'arithmétique vérifiée, la publication échoue. Convention de borne : B(task)=B(action)=max_attempts ; B(fail)=1 ; B(sequence)=1+Σ B(steps) ; B(parallel)=1+Σ B(branches) ; B(choice)=1+max(B(then_i), B(otherwise)) ; B(repeat)=1+max_iterations×B(body) ; B(for_each) selon [l'itération de données](workflow-data-iteration.md#bornes-et-compilation). Le ledger runtime `max_step_activations` débite exactement ces activations, blocs compris. Le budget effectif peut être inférieur à la borne statique, auquel cas l'exécution échoue explicitement lorsqu'il est atteint.

Le reducer traite au plus 128 microsteps internes par transaction de dispatch, puis écrit une continuation durable. Cette limite évite de monopoliser l'écrivain ; elle ne modifie pas les résultats logiques. Les timers expirés sont des événements persistés. Un run possède une deadline globale, y compris pendant un blocage ou une attente humaine, sauf pause autorisée qui produit une nouvelle deadline dérivée de façon explicite et journalisée. Aucun budget n'est implicitement remis à zéro par retry, crash, compaction ou nouveau worker.

Les bornes garantissent la fin des calculs internes et la possibilité d'arrêter. Elles ne garantissent pas qu'un système distant réponde ni qu'un problème métier soit résolu. Un run annulé peut conserver un incident d'effet externe inconnu ; l'interface doit le montrer ; une barrière `successor_blocked` refuse, dans tout successeur, l'admission des seules actions susceptibles de répéter cet effet (`UnresolvedEffect`) jusqu'à ReconcileAction ou ResolveActionOutcome ; la création du successeur reste soumise aux seules règles Q-03 ([états](state-machines.md)).

## Fin, version et exemples

Le succès du body et la garde `completion.success_when` sont distincts. Quand le body a réussi, vrai termine `succeeded`, faux termine `failed` avec `DONE_NOT_MET`, inconnu bloque jusqu'à nouvelle preuve ou deadline. Le critère de done peut être l'observation d'une intégration Git. Il n'ajoute aucun audit ou test non déclaré ; les contrôles d'autorisation et d'intégrité restent obligatoires pour chaque effet.

La frontière est fixée par Q-03 : une version nouvelle est adoptée par un **nouveau Run**, après l'état terminal du précédent pour un successeur ; voir [les états](state-machines.md). Aucune migration en place n'existe en V1.

Exemples de conception : [workflow de feature](../examples/feature-workflow.json), [Dev → Merge](../examples/merge-only-workflow.json). Ce sont des fixtures à valider, pas des exécutions réalisées.

## Extension versionnée R3 : actions sans agent et supervision R5

Le [profil v1.1](automation-execution-profile.md) ajoute kind=action, model_policy et sous-plafond d’actions parallèles. Les définitions v1 ci-dessus restent inchangées ; leur schéma et leurs fixtures restent un profil publiable et exécutable, inchangé. Le nouveau schéma est distinct et réutilise le même reducer. Une automatisation peut avoir bindings vide et budgets modèle nuls ; elle ne simule pas un agent pour appeler un outil.

La supervision réutilise des task leader_decision aux points publiés entre blocs d’action. Des sorties d’observation typées peuvent alimenter un choice ; aucun catch/on_error arbitraire ou réparation implicite n’est ajouté. Voir [F019](../features/F019-reusable-automations/technical-design.md).

## Extension R6 : itération de données v1.2

Le [profil v1.2](workflow-data-iteration.md) ajoute for_each sur tableau immuable, curseur journalisé et after_batch publié. Il conserve le même AST/reducer et les profils précédents. Les lignes sont des activations, pas des nœuds générés dans la définition ; aucun état d’avancement caché n’est confié au driver. La projection /items est lexicale et vérifiée par F006. Le [schéma distinct](../schemas/workflow-v1.2.schema.json) n’est pas une modification rétroactive de v1.1.
