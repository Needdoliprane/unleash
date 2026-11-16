# Workflow v1.2 — itération bornée sur données

**Contrat accepté ([ADR-18](adr/ADR-18-bounded-data-iteration.md)), issu de R6-E03-01.** La même procédure doit parcourir 2, 300 ou 10 000 lignes sans générer autant de nœuds, coder une boucle opaque dans un driver ou demander un modèle à chaque ligne. Le [schéma v1.2](../schemas/workflow-v1.2.schema.json) ajoute un seul bloc `for_each` au reducer commun de F005. Les profils [v1](workflow-dsl.md) et [v1.1](automation-execution-profile.md) restent immuables. Le runtime doit négocier ce profil ; une version inconnue est refusée, jamais réinterprétée.

## Contrat fermé

| Champ | Règle |
| --- | --- |
| id, kind | Identifiant unique de définition ; kind=for_each. |
| items | `{source:path, path:...}` vers un tableau immuable de `/input` ou d’un élément d’itération englobante `/items`. Pas de lecture de résultats évolutifs, réseau ou code. |
| max_items | Entier 1..10 000 ; borne de publication. Un tableau vide est permis si son schéma d’entrée le permet. |
| batch_size | Entier 1..1 000 ; nombre de lignes entre deux after_batch, dernier lot éventuellement incomplet. |
| body | Nœud du même AST, activé pour chaque élément dans l’ordre du tableau. |
| after_batch | Nœud optionnel exécuté après la clôture des éléments du lot, avant toute ligne du lot suivant. Peut contenir une décision publiée en mode supervisé. |

Le profil de référence traite les éléments séquentiellement. Des branches parallèles explicites restent permises dans body, sous scopes et scheduler communs ; des partitions indépendantes peuvent être des branches de workflow distinctes. Aucun parallélisme caché ni nouvelle pool ne vient du curseur. La concurrence entre éléments d’un même for_each est exclue de la V1.

## Entrées, portée et résultat

Avant admission, les données entières sont parsées, typées, bornées et figées dans AutomationDataRevision. Pour chaque activation for_each, le writer épingle référence de tableau, digest, nombre et ordre. Dans body, `/items/<node_id>` expose `index` (base zéro), `value`, `item_digest` et `data_revision_ref`. L’identité technique d’élément est `(run_id, activation_id, index)` ; elle ne remplace jamais la clé métier d’effet ni les barrières entre runs. Une itération imbriquée ajoute son propre node_id dans cette portée lexicale.

À chaque ligne, les résultats du sous-arbre body sont retirés de la projection active, selon la même règle de fraîcheur que repeat. À after_batch, `value/index` ne sont plus visibles ; seule `/items/<node_id>/batch` expose `start_index`, `end_exclusive`, `completed_count`, `is_last` et `item_results_ref` immuable. Ce dernier désigne un artefact paginable de résultats/provenances ; aucune injection de toutes les lignes dans un prompt n’est implicite. Le compilateur vérifie la portée et refuse une lecture de value à after_batch ou hors du bloc. Les résultats du sous-arbre after_batch sont retirés au début du lot suivant.

Les `inputs` d’une task restent des littéraux, comme en v1/v1.1 ([DSL](workflow-dsl.md#nœuds-et-résultats)). Les données de ligne (`/items/<node_id>/value`) et de lot (`/items/<node_id>/batch`) n’atteignent une task que par l’evidence bundle que le harness construit depuis la portée lexicale vérifiée par F006 ; aucune autre donnée n’y est injectée. Une feuille action les lit par `action_input` de source `path`.

La sortie du bloc est `{item_count, completed_count, batch_count, all_items_completed, item_results_ref}`. all_items_completed signifie contrats d’élément achevés, pas réussite sémantique universelle : la garde done peut encore exiger des assertions métier. Le tableau vide donne counts=0 et all_items_completed=true, n’active ni body ni after_batch ; un contrat de métier peut refuser les entrées vides. Le contenu originel du tableau n’est jamais modifié pendant un run.

## Curseur durable et effets

Le frame durable contient activation, tableau/digest, `next_index`, `batch_start`, état body/after_batch et références de résultats. Une transaction prépare l’activation de ligne avant le premier effet. Les actions gardent leurs identités et protocoles F008/F018. Après réussite contractuelle du body et réconciliation de ses effets, une transaction écrit ItemCompleted, le résultat et next_index=i+1. Un crash avant ce commit reprend la même activation de ligne ; il ne réémet pas un effet déjà engagé sans réconciliation. Une ligne failed ne fait pas avancer le curseur. Un choix prépublié peut traiter une erreur attendue et produire un résultat contractuel distinct ; aucune option implicite « sauter les erreurs ».

À la frontière de lot, le curseur n’admet pas la ligne suivante avant clôture d’after_batch et commit BatchCompleted. Un superviseur en after_batch peut décider à partir des logs et assertions du lot ; il ne reprend pas chaque clic. Si une action devient unknown au milieu d’une ligne, le curseur n’avance pas jusqu’à after_batch. Une route de diagnostic souhaitée dans ce cas doit être publiée **dans body**, après la collecte typée concernée ; sinon la réconciliation est extérieure, selon [state-machines](state-machines.md). Cela évite de prétendre qu’un checkpoint encore inatteignable résoudra son propre blocage.

## Bornes et compilation

Pour n=max_items, b=batch_size et B(x) la borne supérieure d’activations selon la [convention du DSL](workflow-dsl.md#bornes-et-garanties-exactes), `B(for_each)=1+n×B(body)+ceil(n/b)×B(after_batch)` (dernier terme zéro si absent). Les bornes imbriquées se multiplient, les parallèles s’additionnent, l’arithmétique est vérifiée ; au-delà de 100 000 activations, refus à la publication. Les essais ne confondent pas 10 000 lignes possibles avec 10 000 nœuds dans la définition, qui reste bornée à 256/profondeur16. À CreateRun, longueur réelle ≤max_items et budget effectif suffisant sont contrôlés avant le premier effet métier. Les budgets d’appels et de décisions tiennent aussi compte du nombre maximal de lots. Un dépassement non prévisible pendant une reprise entraîne l’état de budget prévu, jamais une boucle non bornée.

Sous forbidden, body et after_batch sont récursivement machine-only et tous les autres contrôles du profil v1.1 s’appliquent. Le schéma valide la forme, pas les scopes lexicaux, types, digests, bornes calculées ou autorités. Le compilateur Rust réalise ces passes ; aucun moteur exécuté n’est fourni dans ce dossier.

## Options et acceptation

Option retenue : un nœud borné dans le DSL commun, explicitement versionné. Avantage : mêmes états, projections, preuves et reprises pour toutes les opérations. Coût : analyse de portée et borne supplémentaire, visible dans F005/F006. Alternative : opération bulk spécialisée à curseur interne. Elle peut rester utile pour une API distante atomique, mais exige son propre contrat de progression et convient moins à des points de supervision génériques ; elle ne devient pas le mécanisme implicite de tous les CSV.

Fixture : [automation-rows-workflow.json](../examples/automation-rows-workflow.json), opérations de test proposées. T-F019-003/005/012 et T-F005-003/006 couvrent 0,1,100,101,300,10 000 lignes, budget insuffisant, changement de fichier, interruption avant/après effet/curseur/lot, lecture hors portée, tâche cachée sous forbidden et preuve d’absence de saut/double effet. Les résultats de produit sont not_run ; la validation documentaire vérifie seulement schéma et calculs de fixture.
