# Machines à états et frontières d'exécution

**Contrats de référence, fondés sur les ADR acceptés ([ADR-04](adr/ADR-04-workflow-semantics.md), [ADR-12](adr/ADR-12-hierarchy.md)).** Ce document définit les états de workflow, mission et goal. Pour Agent et Invocation, [session-model.md](session-model.md) est l'autorité canonique et les valeurs ci-dessous en sont une reprise exacte ; [scheduling.md](../specs/scheduling.md) détermine la possession des slots. Une implémentation devra générer des types Rust exhaustifs et des codes de raison distincts. Un label de GUI n'est jamais une nouvelle valeur d'état. Sources conceptuelles et limites : [recherche](../research/workflow-skills-sources.md).

## Vocabulaire temporel

| Objet | Définition et frontière |
|---|---|
| Appel API | Une requête fournisseur logique (`model_call_id`, une action, un ModelCallManifest `request`) ; un retry de transport est un dispatch distinct, historisé et comptabilisé (`action_dispatches.ordinal`) de la même action ; seuls un nouvel appel du tour ou un fallback créent une nouvelle action ; les états de cette action suivent la [machine d'état d'action](failure-recovery.md#machine-détat-daction) |
| Invocation | Travail borné d'un agent logique, lié à une session et à un manifest ; peut comporter plusieurs appels API et actions autorisées |
| Étape | Activation d'un nœud DSL pour une tentative logique d'étape `(run_id, node_id, iteration_vector, attempt)` ; peut contenir plusieurs invocations ; un retry `max_attempts` crée une nouvelle activation liée, alors qu'une reprise après crash conserve l'activation et crée seulement une nouvelle `attempt_id` |
| Run | Exécution durable d'une définition immuable ; manifest de bindings append-only, chaque binding engagé étant immuable |
| Cycle | **Décision Q-03 :** durée logique d'un run jusqu'à son état terminal ; aucune itération, étape ou invocation n'est une frontière d'adoption |
| Mission | Engagement métier avec owner, scope, critères de done ; peut nécessiter plusieurs runs liés |
| Goal | Résultat organisationnel qui agrège ou dépend de missions, sans être leur compteur de complétion |

Le changement de processus, la compaction, la fin d'un appel API et la fin d'une étape ne terminent pas un cycle. Une branche parallèle encore active empêche de considérer son run comme clôturé. Un run terminal n'est jamais rouvert ; une poursuite crée un autre identifiant avec lien au précédent.

## Run

| État source | Commande ou événement | Conditions | État cible |
|---|---|---|---|
| `prepared` | `StartRun` | Définition/bindings résolus, budget réservé, scopes autorisés | `running` |
| `running` | `PauseRun` | Auteur autorisé ; aucune nouvelle activation après barrière | `paused` une fois activités réconciliées ; sinon `blocked` avec intention pause |
| `running` | `NeedEvidence`, `NeedAuthorization`, `EffectUnknown` | Fait indispensable indisponible | `blocked` |
| `blocked` | `ResolveBlock` | Preuve ou autorisation nouvelle, même définition | `running` ou `paused` selon intention durable |
| `paused` | `ResumeRun` | Autorisation actuelle, deadline et budget explicités | `running` |
| `prepared/running/paused/blocked` | `CancelRun` | Auteur autorisé ; fencing et arrêt demandés | `cancelling` |
| `cancelling` | `CancellationReconciled` | Aucune activité encore autorisée à produire effet ; incidents résiduels explicités | `cancelled` |
| `running` | `BodyClosed` | Body réussi ; done vrai, branches réconciliées | `succeeded` |
| `running` | `BodyClosed` | Body réussi ; done faux | `failed` (`DONE_NOT_MET`) |
| `running` | `BodyClosed` | Body réussi ; done inconnu (fait requis absent, périmé ou masqué) | `blocked` (`GUARD_UNKNOWN`) avec intention de clôture ; aucune réactivation du body |
| `blocked` | `ResolveBlock` d'un blocage de done | Nouvelle preuve ; réévaluation de `success_when` seule | `succeeded` ou `failed` (`DONE_NOT_MET`) |
| `running/blocked` | `DeadlineExceeded`, `BudgetExhausted`, échec non traité | Arrêt/fencing puis réconciliation | `failed` après fermeture, sinon `blocked` avec intention terminale |
| terminal | Nouvelle commande de mutation | Jamais réouvert | Rejet `TERMINAL_IMMUTABLE` |

`BodyClosed` n'est émis que pour un body réussi ; un body en échec non traité suit la ligne « échec non traité ».

Une authentification expirée, un CAPTCHA ou une validation humaine prévue est un `NeedEvidence` de raison `AUTOMATION_HUMAN_STEP_REQUIRED` : l'étape passe `waiting_evidence` et l'unité est libérée après checkpoint sûr. La fin d'intervention est déclarée par `AcceptStep` sur cette étape, avec révision et evidence bundle. Cette décision n'accepte pas l'étape : elle autorise seulement une activation d'observation publiée (session, compte, mandat, préconditions) sous nouveau fence. Seule une observation conforme produit `ResolveBlock` ; sinon le run reste `blocked`.

Les états terminaux sont `succeeded`, `failed`, `cancelled`. L'intention terminale, la raison et la liste d'effets à réconcilier sont des champs séparés. Une deadline ne permet pas de mentir sur un effet externe : elle révoque les actions futures, crée l'incident et conduit à une fermeture explicitement qualifiée. Une fermeture avec effet inconnu impose une barrière `successor_blocked` pour les actions concernées ; le statut terminal n'est pas une autorisation de les répéter.

## Activation d'étape et invocation

| Objet | États | Terminalité |
|---|---|---|
| Étape | `ready`, `queued`, `running`, `waiting_evidence`, `awaiting_decision`, `blocked`, `succeeded`, `failed`, `cancelled` | Réussite après contrat ; décision leader seulement si le DSL la demande |
| Invocation | `queued`, `admitted`, `running`, `checkpointing`, `waiting_durable`, `reconciling`, `completed`, `failed`, `cancelled` | Selon session-model ; `completed` signifie sortie et preuves persistées, pas réussite automatique de l'étape |
| Agent logique | `dormant`, `eligible`, `active`, `blocked`, `paused`, `retired` | Selon session-model ; seul le registre d'admission compte les slots |
| Mission | `proposed`, `ready`, `active`, `paused`, `blocked`, `succeeded`, `failed`, `cancelled` | Clôture selon politique mechanical/human/owner_agent ; si agent, jugement N+1/N+2 via commande validée, jamais LLM forcé sous forbidden |
| Goal | `draft`, `active`, `blocked`, `achieved`, `cancelled` | `achieved` exige ses critères explicites ; toutes missions terminées ne suffit pas |

Une étape `running` peut avoir une invocation `reconciling` après disparition de son worker et passer à `waiting_evidence` pour réconcilier une action. La perte du worker est un événement de supervision, pas un état `lost` de l'invocation. Les budgets de tentatives et d'appels sont cumulatifs. Après suspension ou crash, la même `invocation_id` reprend avec une nouvelle `attempt_id` et un nouveau fence, sous une seule tentative active autorisée ; elle ne crée pas automatiquement une nouvelle invocation. Une décision nouvelle déclenchée ultérieurement par un autre événement peut créer une nouvelle invocation explicitement liée. L'étape et la session restent les mêmes pour la reprise, sans effacer les observations antérieures.

Le scheduler compte les réservations d'admission, y compris leaders. `admitted`, `running` et `checkpointing` possèdent un slot ; `reconciling` le conserve tant que l'ancien exécuteur peut agir. `waiting_durable` ne libère le slot qu'après checkpoint durable, canal modèle terminé ou annulé localement et worker arrêté ou privé de continuation. Une attente réseau ordinaire reste `running` avec slot. Les agents dormants ne consomment aucun slot, et les processus auxiliaires gardent leurs réservations propres.

## Révision, ordre et idempotence

Toute commande mutante porte `protocol_version`, `request_id`, `command_id`, `authority_generation`, `actor_ref`, `project_id`, `aggregate_id`, `expected_revision`, `command_type` et `payload`. `actor_ref` est résolu par le serveur, jamais accepté sur la seule déclaration du client. Le couple acteur/`command_id` est la clé d'idempotence. Le reducer valide acteur, epoch, révision, références de preuve et préconditions, puis écrit état, événement et outbox dans la transaction locale. Les événements entrants sont dédupliqués par leur identité. Après contrôle de `authority_generation` (autre lignée : `StaleFence/STALE_AUTHORITY_GENERATION` avant toute recherche de reçu), une même clé avec le même digest canonique de commande — lignée, `command_type`/`protocol_version`, projet/agrégat, révision attendue et payload canonique, selon [IPC v2](ipc-and-apis.md) — rend le reçu conservé, ou `ContractViolation/IDEMPOTENCY_WINDOW_EXPIRED` sur tombstone après purge ; un digest différent échoue `IdempotencyConflict`. L'ordre durable du journal tranche les courses, non les horodatages des processus.

Un message humain N+1 produit atomiquement le message, l'événement et l'entrée mailbox visible au N+2. Le réveil du N+2 est une décision de scheduling séparée. La payload visible respecte l'ACL ; à défaut, un pointeur signalant l'existence de l'événement est exposé sans révéler son contenu.

## Publication et adoption

Un workflow, skill ou rôle passe par `draft` → `validated` → `published` ; un échec de validation retourne un rapport et conserve le draft. Les variantes `deprecated` et `revoked` sont des statuts de registre et ne réécrivent pas le contenu publié. Une révocation de permission s'applique aux actions futures même si le run conserve ses bindings.

**Frontière d'adoption (Q-03).** Publication workflow v2 pendant run v1 : v1 conserve sa définition jusqu'à sa fin. Un binding de rôle/skill déjà engagé conserve également versions, contrats et schémas ; les invocations suivantes qui l'utilisent ne résolvent pas `latest`. Un slot tardif déclaré mais jamais engagé peut recevoir une nouvelle version par commande explicite, sous les contraintes de la définition. Ce n'est pas une migration du binding existant. Après clôture de toutes les branches et traitement des barrières de succession, `CreateRun` peut résoudre v2 par référence exacte (alias éventuellement résolu avant création), avec `predecessor_run_id` pour un successeur unique, refusé `PREDECESSOR_NOT_TERMINAL` tant que le prédécesseur n'est pas terminal ; un second successeur est refusé `SUCCESSOR_EXISTS`. L'écran doit distinguer version publiée, version effective par binding, slot non lié, version candidate suivante et raison d'inéligibilité.

**Alternative écartée en V1 : migration de checkpoint en place.** Une demande future exige un nouvel ADR, un profil de migration et ses preuves. Aucun changement de version à chaque appel LLM n'est acceptable. Pour adopter v2 sans attendre la fin naturelle, l'auteur autorisé annule v1 (`CancelRun`), puis crée le successeur après `cancelled`.

## Jugement de mission et de goal

RecordMissionDecision/RecordGoalDecision appliquent le contrat [F004](../features/F004-hierarchical-orchestration/interfaces.md). `accept` valide les critères effectifs et la quiescence avant succeeded/achieved ; `rework` prépare une poursuite sans rouvrir un run clos ; `escalate` crée blocked ; `fail` conduit mission à failed après réconciliation, goal à blocked. Les transitions ne peuvent être obtenues par affectation directe du statut. Le compteur de chaîne de décision défini dans [la hiérarchie](../specs/agent-hierarchy.md) survit aux réveils et aux crashes ; son épuisement suspend les admissions de décision et génère une notification sans LLM.

## Progression de diagnostic sous blocage d’effet — R6

`EffectUnknown` et la sortie typée d’une collecte sont appliqués dans une même transaction (ou le blocage est persisté avant la sortie, jamais après une admission normale). Le Run reste `blocked` avec son ensemble de barrières d’actions. L’admission métier normale est interdite. Le reducer peut néanmoins consommer les résultats déjà validés, exécuter ses microsteps purs et sélectionner un choice publié ; cela ne clôt pas le run et ne lève aucune barrière.

Une activation sélectionnée dans ce chemin peut être admise comme **diagnostic** seulement si son profil de contrat/catalogue épinglé la qualifie pour lecture, observation, réconciliation ou escalade, et si ses capacités effectives excluent toute mutation métier, retry d’effet ou création libre de travaux. Une tâche leader_decision est admissible dans cette voie si elle est déjà publiée, autorisée par model_policy/budget et restreinte au même mandat diagnostique. Le leader peut proposer une opération de réconciliation autorisée ; il ne déclare pas l’effet accompli sur la seule base de son avis. Les opérations de réconciliation qui modifient l’état distant ne sont pas de simples observations et exigent un mandat spécifique vérifié ; elles restent interdites si elles peuvent répéter l’effet inconnu.

Chaque activation diagnostique conserve `blocked_by_action_ids` et le digest du profil qui prouve son éligibilité. Cette qualification provient du catalogue installé et des contrats exacts, pas d’un booléen fourni par le worker. Un profil mêlant accès normal et diagnostic ne peut bénéficier de cette admission restreinte sans capacité réduite vérifiable. Une branche choisie qui commence par une action normale reste prête mais non admissible ; continue n’est jamais une levée de blocage.

Une observation autorisée enregistrée via ReconcileAction ou via la [disposition factuelle F011](../features/F011-terminal-pool/interfaces.md#disposition-du-contrôle-dont-le-retrait-est-attesté), ou une disposition via ResolveActionOutcome dans les limites F018, émet EffectOutcomeResolved sur chaque Run bloqué par l’action correspondante. Le reducer réévalue les barrières puis applique ResolveBlock uniquement quand toutes celles nécessaires à la continuation sont levées et que budgets/deadline/autorité le permettent. Il reprend running, ou paused si l’intention pause demeure. Un résultat tardif sous ancien fence est archivé mais ne lève rien. Si aucune route diagnostique n’est publiée, le run attend ces commandes de contrôle externes ; le harness n’invente aucune tâche LLM. Sous forbidden, toute variante diagnostique utilisant un modèle est refusée.

| État/événement | Progression autorisée | État conservé |
| --- | --- | --- |
| blocked + ObservationRecorded | Microsteps purs et choix publié, aucune admission normale | blocked |
| blocked + DiagnosticAdmitted | Mandat réduit exact, même pool global ; lectures/réconciliation bornées | blocked |
| blocked + avis continue seul | Aucune preuve d’effet nouvelle ; action normale refusée | blocked |
| blocked + EffectOutcomeResolved | Réévaluer toutes barrières et intention puis ResolveBlock | running/paused si admissible, sinon blocked |

T-F005-003/006 et T-F019-001/011 injectent receipt perdu avant collecte, route présente/absente, replay/supervised, résultat trompeur et crash entre chaque commit. L’oracle exige zéro répétition, zéro mutation conflictuelle pendant blocage et aucune clôture fictive. Cette extension d’admission n’ajoute aucun état Run concurrent.
