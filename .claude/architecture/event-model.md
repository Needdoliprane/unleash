# Événements, dispatch et réveils

Décision [ADR-02](adr/ADR-02-event-architecture.md), acceptée en R7 : état transactionnel + journal append-only + outbox durable dans la base d'autorité, sans broker ; pas event sourcing complet. Le journal métier n'a pas d'échéance automatique (politique Q-12 de [storage-model](storage-model.md)). Seule une purge explicite PurgeHistory le réduit : elle ne supprime jamais physiquement une ligne d'événement encore référencée (outbox, inbox, invocations, appels modèle, entrées de session) ; après acquittement de toutes ses entrées outbox et expiration des garanties consommateurs, jamais moins de 7 jours après son enregistrement (fenêtre de reprise d'[ADR-02](adr/ADR-02-event-architecture.md)), elle remplace seulement son payload par un tombstone (colonne interne `payload_state` passée à tombstone), jamais la ligne, et conserve enveloppe, payload_hash et causalité ([sqlite-design](sqlite-design.md#tables-dautorité-et-commandes)). Les événements décrivent des faits engagés, les commandes demandent des changements. Le transport peut livrer plusieurs fois ; un consommateur applique une fois par clé inbox conservée.

## Enveloppe d’événement

event_id, stream_id, sequence, event_type, schema_version, occurred_at, recorded_at, project_id, aggregate_id, aggregate_revision, actor_ref, correlation_id, causation, command_id, payload, payload_hash et sensitivity. sequence ordonne le journal local ; les timestamps ne départagent pas les races. Sur le wire, causation est un objet typé {kind, id}. Pour kind=command, id est le receipt_id global unique attribué côté serveur et résolu vers (actor_id authentifié, command_id) ; le command_id brut n'est jamais utilisé comme identité globale de cause. Pour event/action, id est l'identifiant global de cet objet ; timer/external référencent leur observation durable. correlation_id regroupe le goal ou incident. Le stockage peut normaliser cet objet en colonnes causation_kind et causation_id. Le modèle physique exact suit [sqlite-design](sqlite-design.md) ; le schéma d’enveloppe est [event.schema.json](../schemas/event.schema.json).

payload est un objet borné porté inline ; un contenu volumineux y figure comme référence adressée par contenu, jamais comme champ payload_ref de l'enveloppe. Une observation d’outil et une déclaration modèle ont des types distincts. Un événement externe est d’abord reçu, authentifié et dédupliqué, puis normalisé en observation ; son nom ne lui donne pas l’autorité d’une transition interne. `schema_version` versionne l'enveloppe (valeur 1). La version d'un payload est portée par son `event_type` : un changement incompatible de payload crée un nouveau type d'événement, jamais une réinterprétation d'un événement engagé.

## Catalogue et dispatch

| Famille / exemples | État persistant | Réaction mécanique | Décision éventuelle |
| --- | --- | --- | --- |
| GoalCreated, GoalRevised | Version goal, message source | Préparer template/procédure ou demande de planification dédupliquée selon contrat | N+2 seulement si décision agente nécessaire et autorisée |
| MissionAssigned, MissionRevised | Mandat, procédure éventuelle et dépendances | Préparer run mécanique ou décision initiale prévue, invalider travaux concernés selon contrat | N+1/N+2 aux points déclarés ; aucun agent imposé à un replay |
| InvocationAdmitted, InvocationStarted, InvocationCheckpointed ; admission/checkpoint de worker d'action | Unité globale, tentative, bundle et ressources | Sous-compteurs agents/replay et reprise | Aucune sur événement normal |
| WorkerFailed, ActionOutcomeUnknown, EffectUnknown, EffectOutcomeResolved | Incident et preuves | Réconciliation/attente, aucune répétition aveugle | Task de supervision si route publiée sous allowed_bounded ; sous forbidden, décision humaine ou mécanique autorisée |
| TaskCompleted, TaskContractRejected | Résultat + contract report | Join/conditions de l’étape | N+1 quand point de décision déclaré, permis par la politique et utile |
| StepNeedsReview, StepApproved, StepRejected | État/révision et décision | Transition autorisée ou correction | N+1 à point défini |
| RunCompleted, MissionCompleted | Résultats/rapport | Vérifier dépendances et done goal | N+2 si consolidation requise |
| IntegrationConflict | OIDs, conflit, scope | Suspendre publication | N+2 staffing, N spécialisé résolution |
| SkillPublished, RolePublished | Nouvelle version | Indexer et notifier abonnés ; pas hot swap | N+2 si demande en attente |
| AutomationVersionPublished, AutomationDataValidated | Artefact/données et digests exacts, qualification | Mettre à jour catalogue et éligibilité des futurs runs ; aucun changement d'un run engagé | Aucune décision modèle implicite |
| Observation de checkpoint/ligne d'automatisation | Outputs typés, safe_to_continue, effect_state, assertion_refs | Évaluer le choice publié, le done et les barrières d'effets | leader_decision seulement au checkpoint/exception déclaré en mode supervised |
| AutomationRepairRequested | Mandat de conception, source/incident, nouveau draft et budget distincts | Matérialiser un job autorisé et sa provenance | Modèle permis seulement par ce nouveau mandat, jamais hérité d'un échec forbidden |
| DiscoveryRecorded, SnapshotIndexed | Reçu/génération | Invalidation et disponibilité | Pas réveil par résultat ordinaire |
| HumanMessageReceived | Message + destinataire + visibilité parent | Mailbox N+1 et référence N+2 atomiques | Selon urgence/impact déclaré |
| DocumentChanged, AccessRevoked | Révision/droits | Invalider contexte futur/index | N+1 si source nécessaire à décision |
| BudgetThresholdReached, ProviderUnavailable | Budget/état fournisseur | Backpressure et attentes durables | N+1 ou N+2 selon périmètre |
| Changement de capacité/localité d'un serveur local, OOM ou annulation incertaine | Profil/tuple de qualification, état de génération et réservations | Invalider snapshot de capacités ou retenir capacité inconnue ; appliquer fallback autorisé | Aucun cloud fallback implicite ni réveil modèle sous forbidden |
| GuardHealthChanged, SubjectRevoked | Politique/incident | Bloquer actions concernées | Humain pour nouvelle autorité |
| GoalCompleted | Done utilisateur + preuve/rapport | Clôture affichée | Aucune célébration qui remplace preuve |

Les noms de détails des fonctionnalités sont spécialisés sous ces familles. Un fait qui touche plusieurs agrégats écrit un événement par agrégat dans la même transaction : ActionOutcomeUnknown (action) s'accompagne d'EffectUnknown sur chaque Run concerné, et ActionReconciled ou RecoveryDecisionRecorded (action) d'EffectOutcomeResolved sur chaque Run bloqué par cette action ([state-machines](state-machines.md)). L'installation et la mise à jour signées de l'application (Q-16) sont exécutées par l'installeur ou l'updater hors protocole daemon ([ADR-14](adr/ADR-14-local-data-oss.md)) ; elles n'ajoutent aucune famille d'événements métier. Aucun événement de logs ou de tokens ne réveille à lui seul un manager. La fin d'une collecte d'observations peut produire unknown/expected_failure sans affirmer le succès métier : seuls les outputs typés et les contrats permettent la suite. Un log « succès » n'efface ni une assertion contraire ni le ledger d'effets inconnus.

## Politique de coalescence

Une mailbox logique contient des références d’événements non consommés, une cause d’invocation et une clé de décision. Regrouper rapports d’une même étape/mission, conserver les messages humains individuels, éviter un réveil par N terminé lorsque le join attend encore. La décision référence les événements qu’elle a réellement vus et la révision d’état ; des événements nouveaux n’entrent pas rétroactivement dans son bundle.

Un message informatif N+1 est visible N+2 immédiatement dans sa boîte d’activité. Une modification de goal, un conflit inter-missions, une demande de budget ou un incident majeur peut créer une invocation managériale selon politique et points de décision autorisés ; ce n'est pas un effet obligatoire de la notification. L’auteur peut marquer urgence, mais ne crée pas une dérogation au plafond global partagé ou à `forbidden`. Les workflows de supervision coalescent les observations par checkpoint/lot dans leurs limites d'invocations et de coût ; ils ne transforment pas chaque clic en demande modèle.

## Origine d'exécution et jobs de conception distincts

Le stockage relie chaque action, worker et traitement déclenché à une origine d'exécution vérifiable. La référence d'origine et la politique effective appartiennent aux manifests/payloads typés versionnés ; elles ne sont pas acceptées sur la seule déclaration d'une page, d'un script ou d'un événement externe. `forbidden` s'applique aux descendants : rapports, embeddings, compaction, sampling et notifications ne peuvent blanchir leur filiation pour accéder au routeur modèle. Une origine absente ou non vérifiable refuse le dispatch concerné.

Une demande utilisateur ou une politique préautorisée peut ouvrir un job `learn`/`repair` distinct, avec mandat, budget, gates et modèle propres. Son lien causal à l'incident est conservé ; il ne change pas la politique du run source. Le contrôle déterministe vérifie le mandat avant matérialisation. Les rapports de mission additionnent ses coûts au lieu de les masquer derrière le badge zéro modèle du replay. Une nouvelle version n'est adoptée que par un nouveau run éligible après les barrières d'effets applicables.

Les événements locaux de modèle séparent configuration, santé passive, inventaire et génération de probe. Une consultation Settings ou un run forbidden ne doit pas déclencher un chargement de poids. Les décisions de routing relisent les autorités actuelles, dont `local_only`, sans déduire la localité de l'URL observée.

## Garanties et tests

État et événement sont engagés ensemble. Le dispatcher peut être arrêté après commit sans perdre la demande ; l’outbox est reprise. L’inbox déduplique les wakeups, la commande de décision est idempotente, et expected_revision rejette un jugement périmé. Un replay de journal pour audit ne redispatche pas d’actions externes.

Tester crash avant/après commit, redelivery x10, arrivée d’événement pendant revue, client lent, restore stream différent et boucle de rapports N+1/N+2. Ajouter replay terminé avec zéro réveil modèle, supervision par lots, exception sans route publiée, réparation préautorisée distincte, OOM/localité inconnue et origine de descendant falsifiée. La politique de réveil garde une borne d’invocations par cause et une escalade autorisée en cas de thrashing. Voir [qualification des automatisations](../testing/automation-replay-tests.md) et [modèles locaux](../testing/local-model-tests.md) ; aucune campagne produit n'a été exécutée dans ce dossier.

## Paramètres de dispatch (ADR-02, R7)

Ces valeurs sont celles d'[ADR-02](adr/ADR-02-event-architecture.md) et font autorité pour l'implémentation.

- Le dispatcher est interne au daemon (tâches Tokio). Il est réveillé après chaque commit et par un balayage de sûreté chaque seconde. `outbox.destination` est l'identifiant `inbox.consumer_id`.
- Livraison au moins une fois, dans l'ordre de `sequence` par destination. Une seule livraison est en vol par couple (destination, agrégat) ; le parallélisme entre agrégats est borné à 32. Les consommateurs restent idempotents par inbox et révision attendue et tolèrent un réordonnancement entre agrégats.
- Une entrée `leased` porte `lease_epoch = daemon_epoch` ; au démarrage, une lease d'un epoch antérieur redevient `pending`.
- Une erreur transitoire (ResourceBusy, dont STORAGE_BUSY et BACKPRESSURE) est rejouée en backoff exponentiel de 200 ms × 2, plafonné à 60 s, avec une gigue de ±20 %, tant qu'elle reste transitoire. Une erreur non transitoire d'un handler passe en `dead_letter` après 8 tentatives.
- Un `dead_letter` arrête les retries du couple (événement, destination) et apparaît dans GetHealth et GetDaemonStatus (consumer_id, event_id, erreur, tentatives). Si l'événement conditionne une transition (join, réveil, invalidation), l'agrégat concerné passe `blocked` avec la raison DELIVERY_DEAD_LETTER. Il est réarmé une seule fois, au démarrage d'un binaire dont `consumer_version` diffère ; aucune opération publique n'est ajoutée.
- Une ligne inbox n'est purgeable que si l'entrée outbox correspondante est `acked` depuis au moins 7 jours ; jamais pour `pending`, `leased` ou `dead_letter`.
- SubscribeEvents garantit la reprise d'un curseur de moins de 7 jours dans le même `stream_id` ; PurgeHistory n'atteint jamais un événement de moins de 7 jours. Un curseur plus ancien, antérieur au plus ancien payload conservé ou émis avant une restauration reçoit ResyncRequired ([IPC](ipc-and-apis.md#diffusion-et-reconnexion)).
- Les réveils managériaux sont coalescés sur une fenêtre de 2 s par clé (agent, mission ou step, decision_key). Un message humain n'est jamais fusionné ; il peut seulement partager un réveil.

## Décisions de clôture et absence de progrès R6

MissionDecisionRecorded et GoalDecisionRecorded portent sujet/révision, critères, owner/epoch, verdict et evidence bundle ; ils sont atomiques avec la transition réellement autorisée et le reçu. Une clôture human ou mechanical ne réveille aucun LLM par nécessité. DecisionStalled/NoProgressLimit sont des observations persistantes du compteur de chaîne de décision ; elles produisent une notification déterministe et suspendent les réveils de cette chaîne jusqu’au réarmement autorisé. Leur redelivery ne remet pas le compteur à zéro. La sémantique et le seuil par défaut (`max_no_progress_turns=8`, configurable de 1 à 100, [ADR-12](adr/ADR-12-hierarchy.md)) sont dans [agent-hierarchy](../specs/agent-hierarchy.md).

## Discovery R6

DiscoveryDecisionRecorded conserve le reçu immuable ; DiscoveryUseAuthorized/DiscoveryUseObserved conservent les préconditions et l’opération exacte avant/au dispatch ; DiscoveryCompletionRevalidated lie une nouvelle découverte à l’usage antérieur et à la cible livrée. Les noms sont des événements internes proposés, pas de nouveaux verbes publics. Une notification de source modifiée affecte current_freshness ; elle n’efface pas l’antériorité historique. Chaque événement porte origine/autorité, versions de source/politique et curseur, sous ACL commune.
