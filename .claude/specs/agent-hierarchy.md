# Hiérarchie agente et décisions — spécification acceptée (ADR-12)

## Modèle d'autorité

N+2 porte les goals, l'organisation, l'arbitrage intermissions et la responsabilité du registre de skills. Il crée N+1 et peut créer N directement pour une mission spécialisée. N+1 porte une mission de bout en bout : compréhension, plan, rôles, skills, scopes, exécution, jugement technique, correction et escalade. N réalise une tâche sous rôle et contrat. Aucun quatrième niveau n'est autorisé ; un N qui rencontre une compétence manquante propose une demande à son owner.

L'autonomie porte sur des choix que le modèle formule en commandes typées. Le harness valide autorisation, révision, budget et preuves, puis exécute les mutations. Il ne possède pas un prompt qui déciderait à nouveau du même plan. Les transitions pures et les politiques de scheduling sont du code Rust ; les décisions techniques restent attribuées au leader qui les émet.

## Identités et relations durables

`Agent` contient `agent_id`, `level`, `created_by_agent_id`, `organization_parent_id`, `state` et `session_id`. `MissionAssignment` contient mission, rôle exact, scope, owner, epoch, date d'effet et raison. L'arbre affiche le parent organisationnel ; la mission affiche son owner. Ces relations peuvent être différentes pour un N d'intégration créé par N+2 mais affecté à une mission N+1. Il n'y a qu'un owner effectif d'une mission à une révision donnée.

Créer directement un N requiert `mission_id`, `owner_agent_id`, `scope_ref`, `role_binding_ref` et un objectif borné. Le propriétaire peut être N+2 si la mission spécialisée est pilotée directement. Le harness refuse une création qui dépasserait la profondeur, créerait un cycle ou laisserait un N sans mission. Une création n'occupe aucun slot tant qu'aucune invocation n'est démarrée.

## Commandes et evidence bundle

L'enveloppe commune est celle des [machines à états](../architecture/state-machines.md). Les commandes de ce domaine comprennent `AssignMission`, `ReviseMission`, `CreateAgent`, `BindRole`, `AssignTask`, `AcceptStep`, `RequestCorrection`, `RequestAgentReassignment`, `RecordMissionDecision` et `RecordGoalDecision`. Elles ne sont pas des API externes. Le payload d'une décision contient `decision_id`, `evidence_bundle_id`, `reviewed_revision`, `reason` et les références de sujets concernés.

Le bundle contient les observations pertinentes, claims et contradictions avec leur fraîcheur. Pour AcceptStep/RequestCorrection, le leader conclut `accept`, `correct` ou `escalate` ; pour RecordMissionDecision/RecordGoalDecision, le verdict est `accept`, `rework`, `escalate` ou `fail` selon [F004](../features/F004-hierarchical-orchestration/interfaces.md) ; il ne peut pas changer l'output observé d'un outil. `AcceptStep` ne réussit que si les règles contractuelles obligatoires passent ; les règles de jugement sont attribuées au décideur. Exception : sur une étape `waiting_evidence` de raison `AUTOMATION_HUMAN_STEP_REQUIRED`, `AcceptStep` déclare seulement la fin d'intervention et autorise une activation d'observation sous nouveau fence ; il n'accepte pas l'étape ([machines à états](../architecture/state-machines.md)). Une révision changée ou un bundle devenu inapplicable produit `RevisionConflict` ou `StaleEvidence`, avec identifiant de l'état courant et sans application partielle.

## Invocations événementielles

Un leader dormant reçoit des événements dans une mailbox durable. La politique de réveil sélectionne les événements utiles : nouveau mandat, blocage, demande explicite, correction disponible, conflit de priorité ou changement affectant une décision. Les logs ordinaires peuvent être regroupés sans appel LLM. Le regroupement conserve la liste des événements et une borne de délai ; il ne supprime pas une demande urgente.

Une invocation possède budget de temps, appels, tokens, commandes et actions. Le nombre de décisions successives sans nouvelle preuve ni mutation utile est borné. Après son lot de commandes, le leader rend la main ; un nouvel événement peut le réveiller. Le résultat tardif d'une invocation révoquée est archivé, puis refusé pour les mutations. Les appels N+1/N+2 occupent les mêmes slots agents que N ; les processus auxiliaires relèvent de budgets distincts.

## Message humain et visibilité N+2

`SendHumanMessage` écrit dans une même transaction le message destiné à N+1, un événement `HumanMessageReceived` et une entrée mailbox pour N+2. Cette visibilité ne dépend pas du succès d'un appel fournisseur. Le contenu exposé suit l'ACL : un N+2 organisationnel peut recevoir une notification sans le corps d'un document qui lui est interdit. Le client voit le reçu et la révision ; après reconnexion il peut constater exactement ce qui a été persisté.

Le réveil N+2 n'est pas systématique. Une politique explicite distingue « visible », « à traiter », « traité » et « ignoré avec motif ». Répondre à l'utilisateur n'efface pas l'événement. Les conversations restent des interfaces vers le même service de mission que GUI, CLI et TUI, avec les mêmes validations.

## Concurrence et réaffectation

Une réaffectation incrémente l'epoch d'ownership, ferme les leases d'action de l'ancien owner, transfère les références de session autorisées et publie un événement. Les actions en vol sont arrêtées coopérativement ou réconciliées. Un ancien worker ne peut plus muter sous l'epoch précédent. Un scope n'est pas une isolation OS ; il est contrôlé par l'autorisation des actions et vérifié sur les artefacts.

Si cinq N sont actifs et N+2 doit traiter une urgence, aucune sixième invocation n'est lancée. Le scheduler applique priorité et équité : appliquer la réserve de contrôle Q-15 ([scheduling](scheduling.md)), demander une interruption coopérative, ou attendre le premier slot libéré. Une action non interrompable n'est pas tuée simplement pour accélérer un leader. Les décisions de scheduling, attentes et raisons de non-préemption sont inspectables.

Critères vérifiables : [F004](../features/F004-hierarchical-orchestration/README.md). L'objection à un pinning intégral trop précoce et la réponse par slots tardifs sont détaillées dans [le modèle](../architecture/skills-contract-model.md).

## Généralisation R3–R5 et supervision par blocs

La hiérarchie N+2/N+1/N demeure celle des agents, pas celle de toute exécution mécanique. Le N+1 possède l’expertise du domaine de sa mission : technique pour développement, procédure documentaire ou administrative pour un autre usage. Le harness peut exécuter directement un AutomationArtifact sans agent actif. Une mission sous OwnerRef user n’introduit aucun niveau d’agent supplémentaire.

Les managers restent responsables et consultables sans être systématiquement réveillés par la réussite d’une procédure. Sous replay forbidden, leurs mailboxes peuvent recevoir l’événement mais aucune invocation modèle n’est admise sur cette chaîne. Sous supervised allowed_bounded, les points publiés reçoivent un paquet de preuves/résultats/logs et rendent une décision typée dans le workflow. Le modèle ne pilote pas chaque clic et ne devient pas propriétaire des états ni des retries.

Le parcours par défaut crée les identités logiques et bindings de versions nécessaires de façon idempotente, selon le mandat utilisateur. Il ne demande pas au novice de choisir les niveaux N ou d’inventer un skill. Les effets, grants, coûts et choix de modèle restent inspectables dans les deux interfaces.

## Borne persistante des décisions sans progression — R6

`DecisionProgressPolicy` est versionnée. Défaut accepté : `max_no_progress_turns=8`, configurable de 1 à 100 dans les budgets supérieurs. Une `decision_chain_id` est attribuée par le harness au premier problème de décision et attachée à un sujet durable (mission ou goal et catégorie de blocage). Tous les messages, escalades, retours et invocations dérivés conservent cette chaîne, même entre N+1 et N+2. Un nouveau command_id, une reformulation, un retry ou un nouvel événement ne crée pas une chaîne indépendante pour le même problème encore ouvert. Un agent ne peut fournir un ID arbitraire pour réinitialiser son compteur.

Avant admission d’une invocation de décision, le writer vérifie compteur consécutif + réservations de tours non encore classés < seuil, puis réserve un tour dans le compteur durable de chaîne ; crash/reprise de la même invocation ne réserve pas deux tours. Chaque tour terminé sans progression admissible incrémente le nombre consécutif ; une tentative perdue reste débitée sauf preuve d’absence totale de dispatch. Au seuil, aucune neuvième invocation sans progrès n’est admise : état DecisionStalled, événement NoProgressLimit et notification déterministe à l’autorité humaine. Cette notification n’est pas une nouvelle escalade LLM. Les coûts et tours cumulés restent comptés même après réarmement.

Progression admissible = transition de step contractualisée avec preuve nouvellement validée, résultat observable requis auparavant absent, résolution d’un effet/incertitude attestée ou révision du mandat par l’autorité humaine. Un message, une paraphrase, renommage, publication de rapport, création d’agent ou simple log ne suffisent pas. Le catalogue versionné de classes d’événement admises et le watermark de preuve sont persistés ; le reducer déduplique leur application. Cette règle structurelle ne prétend pas mesurer le mérite sémantique d’une décision. Une nouvelle preuve admissible remet le compteur consécutif à zéro et conserve l’historique ; un réarmement humain sans nouvelle preuve passe par ReviseMission/ReviseGoal avec directive explicite, motif et nouvelle allocation bornée, jamais par texte libre ignoré du contrat.

Le compteur `(decision_chain_id, policy_version, consecutive_turns, cumulative_turns, last_progress_event_id, state)` et les réservations d’invocation sont transactionnels. Une chaîne close ne peut être réouverte par un ancien événement. Les sous-problèmes réellement indépendants reçoivent une chaîne fille sous budget cumulé de mission ; le harness contrôle leur sujet et ne permet pas une délégation circulaire de recréer un budget. La politique forbidden continue d’interdire toute admission modèle, y compris pour sortir d’un blocage.
