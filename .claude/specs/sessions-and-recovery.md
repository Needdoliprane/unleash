# Contrat de continuité et de reprise

Cette spécification réunit les comportements visibles de F003 et F018. Le détail de stockage est dans [session-model.md](../architecture/session-model.md), [storage-model.md](../architecture/storage-model.md) et [failure-recovery.md](../architecture/failure-recovery.md). Ces contrats de conception font autorité pour l'implémentation ; aucun test produit n'est déclaré exécuté.

## Continuité observable

Un agent conserve identité, rôle/version effectif de chaque invocation, session, décisions, workflow/run, étape, tâches, scopes, worktrees/snapshots, documents sélectionnés, actions et preuves après disparition de son worker. La continuité porte sur ces objets durables et le contexte effectivement envoyé ; elle ne promet pas de restaurer des activations du modèle ou une mémoire de processus.

L'API d'inspection retourne agent/session/invocation/attempt séparément, dernier checkpoint, état de reprise, bundle initial, manifestes de contexte et ModelCallManifests, configuration demandée et modèle/provider réellement utilisés pour chaque appel, parents, événements déclencheurs, versions des skills/rôles/contrats, décisions et liens artefacts. Un fallback autorisé crée une action/manifeste distincts sous politique figée, sans changer le bundle ; hors politique, une nouvelle invocation liée est nécessaire. Les contenus purgés sont signalés. Une compaction expose sources/omissions et le nouveau contexte des appels suivants sans réécrire le contexte initial ; aucun raisonnement privé n'est requis.

## Reprise autorisée

Le retour d'un worker échoué ne suffit pas à marquer une tâche terminée. Une sortie `claim` est d'abord rapprochée des observations du broker et du contrat de tâche. N+1 peut ensuite rendre son jugement métier suivant le workflow ; ce jugement ne remplace pas l'observation. Un retour d'une tentative dont le fence a été révoqué est stocké comme diagnostic et rejeté pour les transitions courantes.

La reprise (port interne `ResumeInvocation`, exposé aux clients par `ResumeRecoveredInvocation`) exige une invocation `waiting_durable`, ou `reconciling` dont l'ancien exécuteur est neutralisé, la révision attendue, un manifeste accessible et l'absence de tentative locale encore autorisée ; une invocation `completed`, `failed` ou `cancelled` n'est pas reprise, et une suite crée une nouvelle invocation explicitement liée. Le scheduler réserve un slot, crée une nouvelle tentative et revalide permissions/budget. Une action `outcome_unknown` qui conditionne la suite bloque cette reprise jusqu'à réconciliation, tandis qu'une mission indépendante peut continuer. Aucune opération de retry d'action ne sert de raccourci de reprise : répéter un effet passe par ReconcileAction ou par ResolveActionOutcome avec la disposition `accept_risk_retry`, et exige une preuve de non-effet ou une stratégie d'idempotence réellement qualifiée.

## Pannes distinguées

| Panne | Ce qui peut survivre | Comportement visible |
|---|---|---|
| Worker | DB, daemon, process-host, actions externes | Nouvelle tentative après fencing/checkpoint ; même session. |
| Daemon | DB committée, hosts, processus et effets distants | `recovering`, revalidation identités, slots conservateurs et outbox rattrapée. |
| Process-host | DB, logs déjà durables, descendants éventuels | PTY potentiellement perdu, état unknown, réconciliation avant relance. |
| Machine/reboot | DB/objets durables et monde distant | Nouveau boot ; aucun rattachement aux anciens PID ; effets distants encore possibles. |
| Corruption/stockage inaccessible | Éléments sauvegardés seulement | Diagnostic limité, pas de nouvelles actions ; restauration contrôlée. |

## Versionnement et messages humains

Le cycle métier est le run. Les publications n'altèrent ni sa définition workflow ni les bindings/bundles engagés. Une nouvelle définition n'est adoptée que par un nouveau Run créé par référence exacte, avec `predecessor_run_id`, après l'état terminal du précédent ; les effets inconnus du prédécesseur restent des barrières d'effet sans empêcher la création du successeur. Un slot déclaré encore vierge peut recevoir rôle/skill nouvellement publié par liaison explicite avant activation dans son plafond d'autorité ; reprise et itération conservent la liaison engagée. Une boucle interne est une itération, pas un cycle d'adoption. Cette règle est la décision Q-03 ([ADR-04](../architecture/adr/ADR-04-workflow-semantics.md)) ; les objections historiques restent dans les audits.

Un message humain à N+1 devient une entrée durable de conversation et un événement visible N+2 dans une même transaction. N+2 n'est pas nécessairement réveillé en LLM : le harness peut l'indexer et coalescer ses notifications. Si N+2 doit raisonner, son invocation attend le même pool global. Un double envoi du même command_id restitue le receipt initial ; deux messages intentionnellement distincts portent deux command_id.

## Erreurs et preuve d'acceptation

`ContextIncomplete`, `StaleFence`, `RevisionConflict`, `ActionOutcomeUnknown`, `BudgetBlocked`, `RecoveryInProgress` et `IdentityUnverified` sont des erreurs distinctes ou états de blocage. Les réponses contiennent les IDs concernés, ce qui est durable, ce qui reste incertain et les opérations permises. Une commande d'arrêt peut rendre un receipt immédiat d'intention, puis des événements d'arrêt partiel/final.

Pour H, la preuve compare les IDs et manifestes avant/après crash worker. Pour I, elle mesure les réservations, pas seulement le nombre de lignes agents. Pour J, elle démontre atomicité message/event et absence de wake obligatoire. Pour K, elle compare les hashes des bundles v1 et v2 de runs distincts. Pour N, elle coupe aux points C0–C12 et démontre que l'adaptateur externe ne reçoit aucune répétition ambiguë. Pour O, elle compare GUI/CLI/TUI au même watermark backend, y compris après reconnexion. Les paquets EV restent `planned` jusqu'à exécution réelle.
