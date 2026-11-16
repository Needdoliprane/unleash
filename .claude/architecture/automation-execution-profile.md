# Profil d’exécution des automatismes — workflow v1.1

**Statut : proposition R3, motivée par la demande utilisateur du 16 novembre 2026.** Ce profil étend le DSL fermé existant ; il n’introduit ni un second orchestrateur ni un langage de scripts dans les guards. Références : [DSL v1](workflow-dsl.md), [artefacts d’automatisation](automation-artifact-model.md), [navigateur](browser-execution.md), [F019](../features/F019-reusable-automations/README.md).

## Pourquoi une extension explicite

Le profil initial `unleash.workflow.v1` impose un binding de rôle à chaque tâche et des budgets modèle strictement positifs. Un budget mis à zéro après coup ne suffit donc pas à représenter une procédure sans agent. R3 ajoute `unleash.workflow.v1.1`, conservé dans un [schéma séparé](../schemas/workflow-v1.1.schema.json). Les fixtures v1 et les définitions engagées restent inchangées. Le runtime négocie la capacité de lire le nouveau profil et refuse proprement une version inconnue.

Le même WorkflowReducer traite sequence, choice, parallel, repeat, fail et leurs règles de jointure, de done, de quiescence et de reprise. La nouvelle feuille `kind=action` invoque un adaptateur d’opération déterministe enregistré, sous broker, avec contrat de preuve. Elle ne construit pas de session LLM et ne requiert pas de rôle d’agent. Les nœuds `kind=task` existants restent utilisés pour les décisions ou travaux d’agents.

## Champs ajoutés et règles

| Élément | Contrat |
| --- | --- |
| `schema_version` | `unleash.workflow.v1.1`, version de format distincte de l’ID/version métier du workflow |
| `model_policy` | `forbidden` ou `allowed_bounded` ; le Run et les autorités supérieures peuvent seulement restreindre la valeur |
| `bindings` | Table vide admise pour un plan entièrement mécanique ; références exactes toujours requises pour les tâches agentes |
| Budgets modèle | `max_llm_calls`, `max_output_tokens`, `max_cost_microusd`, `max_agent_executions` peuvent valoir zéro ; sous forbidden les budgets d’appels/tokens/coût modèle/invocations agentes sont zéro ; `max_agent_executions` limite les invocations agentes simultanément actives du run (sous-plafond de concurrence soumis au pool global), leur cumul étant borné par `max_step_activations` et `max_llm_calls` |
| `max_concurrent_action_executions` | Sous-plafond de workers d’action actifs, 0..50 ; distinct du total d’actions du run et soumis au plafond global d’admission |
| `max_step_activations` | Borne cumulée existante, au plus 100 000 ; ne pas limiter un CSV de 100 lignes à 50 actions séquentielles par confusion avec la concurrence |
| `kind=action` | `operation`, `contract_profile`, `scope_key`, `inputs`, `timeout_seconds`, `max_attempts`, `retry_policy` |

`scope_key` suit la règle de [workflow-dsl](workflow-dsl.md#nœuds-et-résultats) : cette clé locale d’affectation n’accorde aucune permission et reste distincte de `capacity_pools.scope_key`.

OperationRef et ContractProfileRef contiennent ID et version exacts ; leurs digests sont résolus et épinglés avant admission. Le catalogue d’opérations est installé/qualifié par le produit et ne peut être étendu par un texte modèle. Une opération peut réutiliser une bibliothèque ou un moteur externe ; elle décrit ses effets, permissions, ressources, entrées/sorties et protocole de réconciliation. Le champ retry_policy peut seulement restreindre ce catalogue : `never`, `read_only`, `idempotent` ou `reconcile_before_retry`. Un script ne devient pas une lecture parce que son auteur choisit ce mot.

## Paramètres et données

Les entrées d’une action sont une table nom → valeur d’entrée. Une valeur est soit `{source: literal, value: ...}`, soit `{source: path, path: ...}`. Les chemins sont des JSON Pointers dans `/input`, `/results` ou `/facts` ; le profil v1.2 ajoute la portée `/items` de [for_each](workflow-data-iteration.md). Résolution et typage sont déterministes ; aucune interpolation shell, évaluation JavaScript, requête réseau ou recherche sémantique n’a lieu dans ce mécanisme. Une valeur absente, masquée ou périmée bloque ou échoue selon le contrat, sans demande implicite au modèle. Dans ce profil, les `inputs` d’une `task` ne sont pas des références : ce sont des valeurs JSON littérales validées contre l’`input_schema` de l’`entry_skill` liée ([workflow-dsl](workflow-dsl.md#nœuds-et-résultats)) ; seules les feuilles `action` emploient `{source: path}`.

La passe sémantique vérifie producteur dominant, schéma de l’opération, contrat, droits, tailles et bornes. Les coûts d’achats ou d’autres effets externes appartiennent au mandat d’action ; `max_cost_microusd` reste ici le budget d’appels modèles, pas le montant du panier. CSV et paramètres sont enregistrés en AutomationDataRevision avant exécution ; la source ne change pas sous les pieds du run.

## Garantie sans appel modèle

Pour `model_policy=forbidden`, la publication refuse les feuilles agentes `task` et les opérations du catalogue qui appellent un modèle ou peuvent déléguer librement. Le schéma utilise un sous-ensemble récursif machine-only ; le validateur sémantique contrôle en plus les dépendances réelles des adaptateurs. À l’exécution, le Model Router refuse toute demande attribuable à ce Run ou à un descendant, même si un bug, un skill, une compaction, un rapport, un embedding ou un manager demande un appel.

La garantie concerne Unleash, ses drivers/scripts et descendants contrôlés. Elle ne prétend pas révéler les traitements internes du site distant. Un script arbitraire disposant d’un réseau libre ne reçoit pas l’attestation de zéro modèle : le profil de médiation et le confinement doivent rendre cette propriété vérifiable, ou le produit refuse ce niveau de garantie.

Les résultats usuels produisent un rapport déterministe et des notifications sans réveil LLM. Un blocage peut attendre une décision humaine sans modèle. `LearnAutomation` ou `RepairAutomation` est une opération de conception distincte, explicitement autorisée, avec `allowed_bounded` et ses propres budgets. Elle conserve le lien causal avec l’incident mais n’est pas une continuation dissimulée du run annoncé sans modèle. Le run initial garde son état, son compteur zéro et ses références exactes.

## Admission et reprise

Un replay_worker actif consomme une unité du même pool global que les invocations agentes et les `provider_probe` : cinq par défaut, cinquante maximum au total ([ADR-13](adr/ADR-13-scheduler-budget.md)). Il ne compte pas comme invocation LLM. Les sous-compteurs expliquent cette distinction ; les budgets navigateur/processus/mémoire s’ajoutent, ils ne créent pas un autre pool de cinquante exécutions. Le reducer purement mécanique ne consomme pas un worker par transition.

Une attente humaine ou externe durable libère son unité uniquement après checkpoint et neutralisation de la continuation. Un contexte navigateur conservé reste compté dans ses réservations mémoire/processus. `browser_context_limit=2` est le défaut accepté ([ADR-16](adr/ADR-16-reusable-automation-and-supervision.md), amendement R7), à qualifier en recette, pas une promesse de capacité matérielle. Les mêmes priorités et équité s’appliquent ; aucune invocation managériale n’est créée pour contourner forbidden.

Avant une action, le journal persiste intention, autorité, paramètres résolus, cible et préconditions. Après dispatch, l’observation attribuable détermine l’issue. Un timeout de clic, un crash de navigateur ou un accusé perdu peut laisser l’effet inconnu. Le retry ne répète pas l’effet sans observation ou disposition autorisée. Le checkpoint conserve l’étape et les preuves, pas une pile JavaScript ni la mémoire du navigateur. La reprise peut exiger réauthentification et vérification de la page avant de continuer.

## Deux modes d’artefact et frontière de sécurité

Le profil de référence `mediated` exécute les opérations bornées via BrowserPort et les autres ports contrôlés. Un export Playwright/Selenium est un artefact de projet utilisateur, versionné et lisible. Le profil `project_script` peut exécuter cet artefact avec un runtime externe connu, mais son niveau de garantie dépend de la médiation et du confinement effectivement démontrés.

Un filtre de domaine réseau n’empêche pas à lui seul un script de passer une commande plutôt que de seulement préparer un panier sur le même site. Les contrôles sensibles exigent une frontière d’action effective ; à défaut la publication autonome sous cette garantie est refusée ou l’exécution est explicitement accompagnée dans un autre profil. On ne présente pas un commentaire dans le script comme un contrôle d’autorisation.

## Validation documentaire et tests produit

La [fixture de rapprochement](../examples/automation-reconciliation-workflow.json) illustre un workflow sans binding ni budget modèle. Elle est une définition de test, sans comptes ni données réelles ; les noms d’opérations de démonstration sont des propositions Unleash, pas des adaptateurs existants. La compilation, le driver et les assertions métier restent à implémenter.

Le schéma vérifie forme, zéro budgets sous forbidden et absence récursive de tâches agentes. Des contre-exemples doivent refuser task caché dans une branche, budget positif sous forbidden, opération sans version et path illégal. Les tests produit F019 exercent séparément zéro trafic modèle, zéro réveil manager, équité avec agents, résultat métier, dérive, effets inconnus et réparation versionnée. La validité JSON ne prouve aucune de ces propriétés d’exécution.

## Supervision bornée par blocs — clarification R5

`automation_mode=supervised` emploie `model_policy=allowed_bounded`. Le plan publié alterne blocs d’actions et task de type leader_decision à des checkpoints annoncés. Le modèle reçoit résultats typés, assertions, provenance, logs utiles et limites de capture ; il propose une décision autorisée, puis le reducer applique la branche et le broker contrôle les effets. La fréquence, le nombre d’appels, le temps et le coût sont bornés par la section supervision de l’entrée `supervised` de la version d’AutomationArtifact (checkpoints nommés, taille de lot, routes `on_exception` compilées en `choice` sur ObservationReport par F006, plafonds d’invocations, d’appels, de tokens, de coût et de durée), reportée dans les budgets du workflow compilé, et par les budgets supérieurs. Cette section, dite SupervisionPolicy, n’est ni un registre ni un champ du DSL. Une procédure de cinquante clics ne requiert donc pas cinquante appels ; le nombre réel dépend de points de revue explicitement configurés.

Le DSL ne possède pas de catch/on_error implicite. Les exceptions métier anticipées d’un bloc produisent un ObservationReport contractuel avec safe_to_continue, effect_state, reason et assertion_refs ; un choice explicite peut router ce résultat vers un leader. Achever la collecte d’observation ne signifie pas réussir le métier. Si effect_state=unknown, le journal conserve l’effet non résolu et le broker interdit répétition et opérations conflictuelles ; seules observations/réconciliation/escalade autorisées peuvent avancer. La quiescence finale reste impossible tant que les effets requis ne sont pas traités. Les pannes non traitées par une route publiée restent failed/blocked.

Le passage d’un run replay forbidden à un run supervisé n’est pas un changement de drapeau en vol. Il exige clôture/checkpoint/réconciliation et un nouveau mandat/run lié sous politique autorisée. Une réparation peut être préautorisée par une règle versionnée de scope, budgets et évaluations : elle forme alors un job distinct, sans validation humaine permanente imposée, et son coût apparaît au niveau mission. Le badge zéro modèle du run n’affirme pas que toute la phase d’apprentissage ou de maintenance n’a jamais utilisé de modèle.

## Contrat commun des observations

Le schéma [ObservationReport v1](../schemas/automation-observation.schema.json) définit outcome observed/expected_failure/unknown, safe_to_continue, effect_state none/known/unknown, run/activation/révision et références d’actions/assertions/logs. none signifie aucune mutation externe dans le bloc ; known signifie que les issues sont connues, y compris des échecs ; unknown conserve au moins une issue ambiguë. Ce résumé ne remplace pas les états détaillés du journal d’actions. Une observation expected_failure ou unknown ne permet pas continue ; le workflow peut choisir une autre branche autorisée.

Un rapport avec effet unknown doit référencer les actions concernées et ne peut déclarer safe_to_continue=true. Le validateur sémantique vérifie que les références existent, sont lisibles, fraîches, liées au run et couvrent réellement les effets. Un script qui ment dans ces champs ne reçoit pas de garantie par sa conformité JSON. La fixture [supervisée](../examples/automation-supervised-workflow.json) illustre une seule revue, avec rôle/skill cohérents.

## Profil de lots R6

v1.1 reste adapté aux blocs sans itération générique. Les automatisations parcourant des données choisissent explicitement [v1.2 for_each](workflow-data-iteration.md), avec même moteur, budgets et politique modèle. Le choix de profil est épinglé dans la version d’automatisation, son manifest et le run. Une définition v1.1 ne reçoit pas silencieusement cette sémantique.
