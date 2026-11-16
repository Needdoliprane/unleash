# Sessions durables, invocations et tentatives

**Contrat canonique, retenu par [ADR-05](adr/ADR-05-durable-sessions.md) (accepté en R7).** Une session n'est ni un processus vivant, ni le cache d'un fournisseur, ni une copie de mémoire d'un modèle. Elle est l'ensemble versionné des messages utiles, décisions, références de travail et manifestes permettant de produire une nouvelle invocation explicable.

## Objets et contenu durable

L'agent logique porte son niveau N/N+1/N+2, son rattachement organisationnel et son état `dormant`, `eligible`, `active`, `blocked`, `paused` ou `retired`. Ces états ne suffisent pas à compter des slots : seul le registre d'admission fait foi. Un leader peut être dormant tout en possédant vingt missions historiques. Les workers associés sont temporaires ; leur disparition ne supprime pas l'agent.

La session contient : messages utilisateur visibles, événements de travail, décisions justifiées, contraintes, branches/worktrees et snapshots, demandes en attente, receipts d'actions, liens vers preuves, compteurs d'utilisation et chaîne de compactions. Un `context_manifest` référence les objets du payload préparé, l'ordre, les hashes, les politiques/filiations et les exclusions motivées ; ModelCallRecord distingue dispatch et réception confirmée ou inconnue. Il ne contient pas de raisonnement privé caché demandé au modèle. Un résumé est une donnée dérivée, jamais une preuve que tous les détails omis ont été conservés. La séparation entre missions d'un même agent est portée par le `context_manifest` de chaque invocation ; il n'existe pas de tête de contexte durable par mission en V1.

Chaque invocation persiste avant dispatch : `agent_id`, `session_id`, `parent_invocation_id` éventuel, `trigger_event_id`, mission/run/step/task, rôle/version, workflow/version, liste ordonnée de skills et contrats/version, configuration modèle demandée, bindings de sélection exacts avec versions/digests/scopes, politique figée de modèles/providers/capacités autorisés, scope/version, documents pins/version, manifeste de contexte initial, limites/budgets et révision. Ce bundle est immuable ; `bundle_hash` est le SHA-256 de sa sérialisation RFC 8785 (JCS) normalisée. Les appels API successifs d'un même tour sont des actions distinctes ; chaque action possède `model_call_id` et ModelCallManifest immuable avec parentmanifest/événement déclencheur, provider/modèle effectif, capacités négociées et contexte préparé. L'usage, la transmission et la sortie sont persistés dans un receipt ou manifeste de résultat append-only lié à la requête, sans réécrire celle-ci. Un retry réseau reste un dispatch distinct, historisé et comptabilisé (`action_dispatches.ordinal`) de la même action et du même manifeste ; il ne crée ni nouvel agent, ni nouvel appel, ni slot supplémentaire caché.

Un fallback admis par la politique figée crée une nouvelle action et un nouveau ModelCallManifest, avec checkpoint préalable si nécessaire ; il ne modifie pas le hash du bundle. Un modèle/provider ou une autorité hors cette politique requiert une nouvelle invocation liée, après les validations applicables. Une compaction entre deux appels crée un nouveau context manifest et un nouvel appel lié ; elle ne remplace jamais le contexte de départ contenu dans le bundle historique. Les UI montrent la configuration demandée et la succession des modèles réellement utilisés.

## Machine d'état de l'invocation

| État | Possède un slot ? | Entrée/sortie autorisées |
|---|---|---|
| `queued` | Non | Intention durable ; attente de dépendances, budget et admission. |
| `admitted` | Oui | Réservations atomiques acquises ; tentative pas encore démarrée. |
| `running` | Oui | Worker et canal broker actifs ; appel modèle ou outil ordinaire en cours. |
| `checkpointing` | Oui | Prépare un point durable ; pas de libération prématurée. |
| `waiting_durable` | Non | Contexte/action pending committés, aucun appel modèle local en vol, worker arrêté ou dépourvu de capacité de poursuivre. |
| `reconciling` | Oui tant que l'ancien exécuteur peut agir | Crash/interruption ; inventaire et fencing avant réadmission. |
| `completed` | Non | Sortie, preuves et décision terminale persistées. |
| `failed` | Non | Terminal : échec connu, diagnostic et suites permises persistés ; une suite crée une nouvelle invocation liée. |
| `cancelled` | Non | Aucune continuation active autorisée ; effets déjà accomplis/unknown restent enregistrés. |

`blocked` est une raison de file ou d'attente durable, pas une permission de continuer sans slot. De même, `waiting_owner` (N qui attend son propriétaire) et `waiting_children` (parent qui attend ses enfants) sont des valeurs de `invocations.wait_reason`, pas des états : elles conduisent à `waiting_durable` après checkpoint sûr. Une attente asynchrone de réponse modèle conserve le slot. Un outil long peut tourner après un checkpoint durable sous budget processus, mais le worker ne peut plus interroger le modèle ni traiter sa sortie sans nouvelle admission. L'action reste suivie par le broker déterministe.

Une invocation `waiting_durable`, ou `reconciling` dont l'ancien exécuteur est neutralisé, reprend avec le même `invocation_id` et une nouvelle `attempt_id` ; chaque réadmission crée une nouvelle réservation d'exécution, les réservations libérées restant historiques ([SQLite](sqlite-design.md)). Une invocation `completed`, `failed` ou `cancelled` n'est jamais reprise. Chaque tentative porte son fence et ses bornes ; au plus une tentative active de l'invocation est autorisée. Une tentative active porte l'état `admitted`, `running`, `checkpointing` ou `reconciling`, vocabulaire de l'invocation couvert par l'index unique partiel de [sqlite-design](sqlite-design.md) ; elle se termine en `ended`, `failed` ou `fenced` et ne redevient jamais active. Un retry après résultat inconnu réutilise l'action existante et son protocole de réconciliation : il ne redispatche pas automatiquement son effet. Les étapes de workflow ne sont pas déduites de l'état de la tentative.

## Checkpoint sûr et préemption

Un checkpoint est sûr lorsque les messages/references nécessaires, les décisions, les actions engagées et leurs états sont persistés ; le canal modèle est terminé ou annulé localement ; aucune instruction de continuation n'est laissée à un worker hors contrôle ; les ressources exclusives sont conservées explicitement ou rendues. La transaction qui marque `waiting_durable` libère simultanément le slot. Les processus auxiliaires persistants gardent leurs réservations et propriétaires.

La priorité d'un manager urgent déclenche une demande de checkpoint à un exécuteur `work` préemptible (N, replay_worker ou provider_probe), selon l'ordre de [scheduling.md](../specs/scheduling.md). Elle ne déplace pas arbitrairement une tâche en `waiting_durable` au milieu d'un effet. Si toutes les invocations sont dans un fragment non préemptible, le manager attend ; le contrôle déterministe de sécurité peut néanmoins révoquer/stopper sans appel LLM supplémentaire. Voir [scheduling.md](../specs/scheduling.md).

## Versions et frontières

Décision Q-03 ([ADR-04](adr/ADR-04-workflow-semantics.md)) : le run garde sa définition de workflow immuable. Une publication de workflow/rôle/skill v2 ne modifie pas une invocation matérialisée ni un binding déjà engagé, y compris en file, itération ou reprise. Le cycle métier correspond au run. Une nouvelle version n'est adoptée que par un nouveau Run créé par référence exacte, avec `predecessor_run_id`, une fois le précédent terminal ; les effets inconnus du prédécesseur restent des barrières d'effet ([workflow-dsl](workflow-dsl.md)). Une boucle interne utilise une identité d'itération, sans constituer une frontière d'adoption. Une branche encore active empêche d'affirmer que le cycle est fini.

Exception explicitement bornée de liaison tardive : un slot déclaré dans la définition, encore vierge de binding engagé et d'invocation matérialisée, peut recevoir une version de rôle/skill nouvellement publiée par une commande BindRole matérialisant les événements SkillBound explicite, avant première activation et dans son plafond d'autorité. Cela ne change ni définition de workflow ni binding engagé. Une itération ou reprise ne rend pas un slot vierge : elle conserve la liaison engagée. L'événement de binding est distinct de publication et apparaît dans le futur bundle.

Q-03 est résolue : l'invocation éligible est celle qui démarre un nouveau Run après la fin terminale du précédent, pas un hot swap à chaque réponse API ; aucune migration manuelle en place n'existe en V1. Une révocation de sécurité s'applique aux actions futures immédiatement via le broker sans réécrire les versions historiques.

## Compaction, forks et reprise

Une compaction prépare un nouveau manifeste avec sources, omissions, appel modèle producteur et raisons de sélection, puis remplace la tête de session par compare-and-swap de révision. Cette tête évolutive est distincte de l'initial_context_manifest_id du bundle immuable. Les anciens manifestes restent inspectables selon rétention. Une écriture concurrente force recalcul ou rebase du manifeste ; elle ne disparaît pas dans un résumé stale. Les données sensibles purgées portent des tombstones.

Les sorties de compaction et les forks conservent la filiation et la politique effective de toutes leurs entrées réellement utilisées, conformément au [routage](../specs/model-routing.md). La couverture d'un résumé, le retrait d'un document ou la création d'une nouvelle session n'élargit pas ses ACL, destinations, localité ou conservation. Une purge applique aussi les obligations héritées aux dérivés ; si la filiation restante ne permet plus d'autoriser leur utilisation, la continuation bloque au lieu de réinitialiser leur classification. Une déclassification autorisée produit une décision distincte reliée au contenu exact, sans réécriture des manifestes historiques.

La reconstruction V1 utilise `context_mode=explicit_unleash_history`, avec messages, résultats et blocs natifs qualifiés explicitement sérialisés. Aucun ID de réponse/conversation distant ne remplace cet historique, même s'il reste présent dans des métadonnées de corrélation. Un ancien ID devenu invalide n'exige donc pas de récupération distante. Les blocs opaques conservés ont leur propre filiation et politique ; après exclusion d'un antécédent, ils ne peuvent réintroduire les données retirées. Leur perte impose une reconstruction portable autorisée ou un blocage explicite, jamais une promesse de restituer un raisonnement privé.

Un fork de contexte est éphémère, mais sa création, ses entrées et sorties utiles sont reliées à l'invocation. Il ne transmet pas une RAM magique au prochain worker. Après crash, le nouveau worker reprend à partir du dernier checkpoint committé, des événements suivants et des observations réconciliées. Si une preuve ou un artefact essentiel manque, `CONTEXT_INCOMPLETE` bloque la continuation et expose la pièce manquante.

Les critères H/I/J/K/N reposent sur ces objets : crash worker sans perte d'identité, nombreux leaders dormants sous limite, message humain journalisé et notifié au N+2, versions stables et reprise sans répétition aveugle. Le test doit comparer le manifeste réel envoyé et les actions reçues, pas seulement le texte affiché d'une conversation.

## Run mécanique et supervision optionnelle

AutomationRun est une projection du Run commun avec référence d’AutomationArtifact, révision de données, automation_mode et model_policy ; le profil navigateur execution_mode mediated/project_script est distinct. Un replay_worker n’est ni un agent logique ni une invocation modèle : ses checkpoints conservent node/action, données, preuves et réconciliation. Il possède néanmoins un permis du plafond global d’exécutions actives.

Le mode supervised matérialise des invocations aux points de décision déclarés, avec le même InvocationBundle et la même mémoire durable. Les blocs mécaniques et les invocations ne créent pas de sessions opaques concurrentes. `forbidden` interdit ces invocations ainsi que rapports/compactions générés par modèle ; une session agente n’est pas un prérequis au rejeu. Les [règles v1.1/v1.2](automation-execution-profile.md) précisent la frontière.
