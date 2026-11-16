# Seconde revue contradictoire indépendante — Expert 13

> **Addendum R2 — 16 novembre 2026.** Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). Bibliothèques existantes et HTML/CSS sont autorisés ; Kivy et Tauri sont identifiés ; la GUI et la TUI doivent avoir une parité fonctionnelle complète. Les choix actuels sont dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md). Les autres observations techniques restent à lire avec leurs limites initiales.

**Date de référence : 16 novembre 2026.** Auteur : sous-agent distinct `final_adversarial_review`, missionné après rédaction des propositions et des dix-huit dossiers de fonctionnalités. Cette revue n'est pas celle de l'auteur de la première [red team](adversarial-findings.md), ni un quatorzième processus d'expertise fictif. Les différents regards du premier groupe sont attribués dans [agents/README](../agents/README.md).

**Objet :** tenter de réfuter les garanties du dossier consolidé, particulièrement aux frontières entre équipes. **Nature :** audit de conception et de cohérence documentaire. Aucun binaire Unleash, aucune campagne de crash produit, aucune intégration AI-Guard et aucun benchmark GUI n'ont été exécutés.

## Périmètre, méthode et jugement initial

Le mandat commun et l'annexe A–M ont été lus avant le dossier. La revue a parcouru l'arborescence existante, les exigences et interfaces des dix-huit features, les spécifications transversales, schémas, exemples, plans, audits et écrans. La lecture approfondie a porté sur connaissances/discovery, sessions/scheduler, workflow/bindings, contrats, IPC/stockage, processus/reprise et contexte fournisseur. Les références citées ci-dessous désignent les sections où les contre-exemples ont été trouvés ; elles ne prétendent pas constituer une preuve d'implémentation.

Le corpus continuait à être consolidé pendant cette seconde revue. F011/F018, schémas et matrices arrivés pendant celle-ci ne sont pas comptés comme capacités absentes. Une référence documentaire temporairement manquante n'est pas assimilée à un défaut de l'architecture. Les corrections des auteurs sont consignées séparément du constat initial : les supprimer de l'historique ferait perdre la raison des décisions.

Le jugement initial est **gel des interfaces communes à refuser tant que les contradictions identifiées ne sont pas harmonisées**. Une architecture plausible ne suffit pas lorsque deux équipes peuvent en dériver des enums incompatibles ou une reprise d'autorité contradictoire. Les huit findings ci-dessous sont des problèmes de conception ou de contrat ; ils ne sont pas présentés comme huit vulnérabilités exploitées d'un produit existant.

### Classification

| ID | Sévérité initiale | Nature | Frontière affectée | État de relecture |
| --- | --- | --- | --- | --- |
| FINAL-01 | Élevée, bloquante pour les types communs | Contradiction réelle | Invocation, tentative, agent et frontière de version | Corrigé dans la spécification ; relu indépendamment |
| FINAL-02 | Élevée | Contradiction réelle | Discovery F007/F008/F009 | Corrigé ; vocabulaire et matrice de couples recontrôlés |
| FINAL-03 | Élevée, bloquante pour IPC | Contradictions réelles | Wire, exemples, stockage des événements, catalogue | Corrigé ; schémas, fixtures, identité de cause et catalogue relus |
| FINAL-04 | Élevée | Contradiction réelle | Bundle immuable, fallback et contexte par appel | Corrigé ; séparation bundle/appel et stockage relus |
| FINAL-05 | Critique pour la garantie de reprise | Protocole insuffisant, avec contraintes contradictoires | Restore ancien, fencing et process-host vivant | Corrigé ; protocole de lignée et test hostile relus |
| FINAL-06 | Élevée | Contrat insuffisamment défini | Compilation, composition des skills et binding tardif | Corrigé ; entrée unique et algorithme exact relus |
| FINAL-07 | Élevée | Incohérence avec la formulation de l'invariant | Baisse de limite 50→5 | Corrigé ; limite effective et oracle par préfixe relus |
| FINAL-08 | Élevée | Garantie insuffisamment fondée | Rapports historiques, rétention et restauration | Corrigé ; snapshot scellé, curseur et limites historiques relus |

Une décision annoncée par le coordinateur reste une **proposition de conception du dossier**. Elle ne devient pas une décision de Louis. Les invariants Rust interne, V1 complète, limites agentes, done configurable et absence de juge universel restent inchangés.

## FINAL-01 — Deux machines d'état concurrentes pour une même invocation

**Preuves initiales.** [state-machines](../architecture/state-machines.md), sections « Activation d'étape et invocation » et « Publication et adoption », déclarait ses enums normatives mais utilisait `starting`, `waiting_tool`, `stopping`, `succeeded`, `lost` pour une invocation. [session-model](../architecture/session-model.md), « Machine d'état de l'invocation », et [scheduling](../specs/scheduling.md), « Unité et admission atomique », utilisaient `admitted`, `checkpointing`, `waiting_durable`, `reconciling`, `completed`. Les états d'agent divergeaient également. Le premier document évoquait une nouvelle invocation après reprise ; les modèles de session et stockage conservaient `invocation_id` avec un nouvel `attempt_id`.

**Scénario de rupture.** L'équipe workflow écrit `succeeded` et considère `lost` terminal. L'équipe scheduler attend `completed` et garde un slot `reconciling`. Après crash, soit le slot fuit, soit une seconde admission intervient alors que le premier exécuteur reste capable d'agir. La GUI peut afficher fin de travail tandis que la reprise en crée un nouvel identifiant non relié correctement.

**Solution recommandée.** Une seule définition canonique pour les états d'invocation et le calcul du slot ; des états distincts pour worker/tentative si nécessaires. Une reprise de la même invocation conserve son identité et crée une tentative, avec budgets cumulatifs. Une nouvelle activation de tâche et une nouvelle invocation métier ne sont pas déduites de la seule mort du processus.

**Alternative.** Adopter l'autre enum partout reste possible si admission, checkpoint, réconciliation et propriété du slot sont représentés explicitement et mappés sans perte. Renommer seulement les labels GUI ne résout pas la contradiction.

**Complément de frontière.** Le scénario Publication de [concurrency-tests](../testing/concurrency-tests.md) interdisait toute « adoption v2 dans ce run », formulation trop large pour les slots tardifs. Un workflow et un binding déjà engagés restent stables ; un slot déclaré mais vierge peut recevoir explicitement une première liaison nouvelle, si cet arbitrage est retenu. Ce cas n'est pas une migration cachée du binding existant.

**Validation attendue.** Un tableau d'autorité unique ; round-trip de tous les états entre domaine, DB, IPC et clients ; crash avec reprise du même `invocation_id` et tentative différente ; aucun slot perdu ou libéré prématurément ; test séparé workflow v1, binding engagé v1 et premier binding tardif v2. Ce sont des critères futurs de qualification d'Unleash, pas des étapes ajoutées aux workflows des utilisateurs.

**Statut après relecture.** Corrigé dans la spécification. `state-machines` désigne `session-model` comme autorité Agent/Invocation, reproduit les mêmes valeurs, conserve `invocation_id` et distingue la tentative de worker. F005 distingue également tentative logique de tâche et tentative de processus. Le scénario Publication de `concurrency-tests` teste désormais séparément définition/bindings engagés stables, liaison d'un slot vierge et conservation en reprise/itération. Aucun test produit n'est annoncé exécuté.

## FINAL-02 — Le reçu de découverte ne parle pas le même langage aux registres et au knowledge

**Preuves initiales.** [skills-contract-model](../architecture/skills-contract-model.md), « Recherche avant création », [skills-and-roles](../specs/skills-and-roles.md) et [contract-enforcement](../specs/contract-enforcement.md) utilisent `reuse/adapt/create`. [shared-knowledge](../specs/shared-knowledge.md), « Discovery : invariant transversal », et [F009 interfaces](../features/F009-knowledge-engine/interfaces.md) ne définissent que `reuse/create`, avec stratégies de coopération distinctes. [F007 interfaces](../features/F007-skill-studio-registry/interfaces.md) expose `selection: adapt` sans normalisation explicite vers le reçu canonique.

**Scénario de rupture.** N+1 adapte un rôle après recherche, F007 enregistre `adapt`, F009 ne sait pas créer le reçu ou F008 rejette le choix. À l'inverse, un traducteur improvisé transforme `adapt` en réutilisation du rôle original alors qu'une nouvelle version a été créée. La provenance et la condition « rechercher avant créer » deviennent invérifiables entre composants.

**Solution recommandée.** Séparer décision d'identité de cible et stratégie. L'arbitrage du coordinateur est `decision=reuse|create` ; `reuse` accepte `use_existing|coordinate|wait|adapt`, `create` accepte `copy|adapt|new`. L'adaptation conservant l'identité logique, y compris sous nouvelle version, vaut `reuse/adapt` et référence sa cible existante ; une adaptation créant une identité distincte vaut `create/adapt` et relie la nouvelle cible à la source. La recherche, le choix et la création sont trois événements distincts ; une requête de recherche seule ne fabrique pas une décision métier.

**Alternative.** Conserver trois décisions distinctes serait acceptable avec un schéma commun qui définit exactement `adapt`, les identités sources/cibles et toutes les transitions. L'important est l'absence d'ambiguïté entre composants ; l'enum choisi n'est pas un invariant utilisateur.

**Validation attendue.** Un `DiscoveryReceipt` accepté par F007 doit se lire par F009 et se vérifier par F008 sans traduction implicite. Fixtures réutilisation, attente, copie, adaptation vers nouvelle version, création entièrement nouvelle ; refus des paires décision/stratégie incohérentes ; trace tardive distincte d'une trace préalable ; reçu invalidé après rebase ou révocation concernée. Une adaptation sémantiquement judicieuse reste un jugement N+1, pas une propriété garantie par le champ `strategy`.

**Statut après relecture.** Corrigé dans la spécification. Le modèle skills, les spécifications knowledge/contrats, F007/F009 et [discovery-receipt.schema.json](../schemas/discovery-receipt.schema.json) emploient la même règle d'identité. La matrice documentaire des douze couples décision/stratégie a été exercée : les sept couples annoncés passent, les cinq autres sont rejetés ; adaptation sans candidat et réutilisation sans cible sont rejetées. Cette validation de forme ne prouve ni l'existence de la cible ni l'antériorité réelle de la recherche, qui restent contrôles de domaine.

## FINAL-03 — Les contrats wire contredisent les exemples et leur persistance

**Preuves initiales.** [F001 interfaces](../features/F001-core-daemon/interfaces.md) donnait `protocol_version: 1`, alors que [ipc-command.schema.json](../schemas/ipc-command.schema.json) impose `unleash.ipc.v1`. Les noms et états de receipt divergeaient entre F001 et [ipc-and-apis](../architecture/ipc-and-apis.md), ainsi que `retryable/details` contre `retry_class/details_ref`. L'événement déclaré valide dans [F002 interfaces](../features/F002-persistence-events/interfaces.md) utilisait `aggregate_type`, `event_ordinal` et `causation_event_id`, absents du [schéma wire](../schemas/event.schema.json), et omettait plusieurs champs requis.

Le tableau `events` de [sqlite-design](../architecture/sqlite-design.md), « Tables d'autorité et commandes », ne précisait pas comment reconstituer l'acteur, le projet, la cause typée, les dates, le hash et la sensibilité du wire. Un `command_id` seul ne détermine pas l'acteur lorsque la clé de receipt est `(actor_id, command_id)`. Le catalogue des verbes proposait en outre `PublishWorkflow` face à `PublishWorkflowVersion`, et `ValidateWorkflow` face à `ValidateWorkflowDraft`, sans mapping explicite.

**Scénario de rupture.** Le client construit le JSON F001 ; le serveur généré depuis le schéma le rejette avant toute commande. Un événement initialement servi en mémoire ne peut pas être servi identiquement après redémarrage faute de données persistées. Deux acteurs réutilisant le même `command_id` rendent une causalité reconstruite par jointure ambiguë.

**Solution recommandée.** Les schémas wire et un catalogue de verbes publics constituent l'autorité. Aligner exemples et features ; nommer explicitement les DTO internes distincts. Persister soit toutes les colonnes nécessaires, soit une enveloppe canonique complète et versionnée, avec mapping documenté. Les IDs pédagogiques sont permis dans des fragments signalés, mais ne justifient pas des types incompatibles.

**Alternative.** Un protocole différent peut être retenu en version majeure explicitement séparée, avec adaptateur de migration et fixtures de chaque version. Deux contrats incompatibles portant le même nom `v1` ne sont pas une alternative viable.

**Validation attendue.** Validation des exemples complets contre leurs schémas, round-trip DB→wire après redémarrage, cause command/event/action/timer/external, acteur déduit de l'authentification, deux acteurs avec même ID de commande, absence de noms publics synonymes non documentés. Les erreurs spécifiques doivent être des codes/raisons définis, pas un second format par feature.

**Statut après relecture.** Corrigé dans la spécification. F001 emploie le protocole, le reçu et les erreurs canoniques ; F002 expose une enveloppe compatible avec le schéma. `receipt_id`, UUID global attribué serveur, est requis dans le schéma de reçu et unique en stockage. Pour `causation.kind=command`, l'identifiant de cause est ce reçu, résolu sans ambiguïté vers `(actor_id, command_id)` ; le `command_id` wire reste informatif. Le modèle SQLite conserve les champs nécessaires au wire et une FK différable de causalité vers le reçu. Les fixtures F001/F002, leur lien et le hash du payload ont été revalidés. Le catalogue nomme les verbes canoniques et refuse les alias silencieux ; le dernier résidu `BindRole/BindSkill` du modèle de session a été remplacé par une commande `BindRole` matérialisant les événements `SkillBound`. Le round-trip réel DB→wire et le scénario de deux acteurs partageant un `command_id` restent des critères futurs, sans prétendre qu'une validation JSON les exécute.

## FINAL-04 — Un fallback crée un nouveau bundle que l'invocation interdit de changer

**Preuves initiales.** [model-routing](../specs/model-routing.md), « Coûts, quotas, retries et fallback », dit que le changement crée un nouveau bundle. [session-model](../architecture/session-model.md), [storage-model](../architecture/storage-model.md) et [F003 design](../features/F003-agent-runtime/technical-design.md) imposent un bundle d'invocation immuable et la conservation de son identité en reprise. La même invocation peut pourtant appeler le fournisseur plusieurs fois avec des résultats d'outils nouveaux.

**Scénario de rupture.** Un appel OpenAI échoue avant un résultat utilisable ; la politique permet Anthropic. Le routeur remplace le modèle effectif dans le bundle, altérant l'histoire du premier appel, ou conserve l'ancien modèle alors que le manifeste présenté à l'utilisateur décrit le second. Après crash, une équipe reconstruit le contexte du premier fournisseur pour reprendre le second.

**Solution recommandée.** Séparer `InvocationBundle` immuable — mandat, versions, scope, politique de modèle et plafonds — et `ModelCallManifest` immuable **par action d'appel** — fournisseur/modèle effectif, profil, contexte réellement envoyé, outils, payload hash, capacités, réservation et cause du fallback. Conserver la suite ordonnée d'appels et leurs états. Le manifeste d'invocation agrège ces références sans réécrire l'appel antérieur.

**Alternative.** Terminer ou suspendre explicitement l'invocation avant le fallback puis créer une invocation liée dotée d'un nouveau bundle. Cette solution est plus lourde mais cohérente si les actions déjà engagées, coûts inconnus et causalités sont conservés.

**Validation attendue.** Deux appels dans une invocation, puis fallback inter-fournisseur, produisent des manifestes différents et immuables ; une inspection historique restitue le bon payload et modèle pour chacun ; les blocs opaques ne sont pas transposés à un autre fournisseur ; aucun outil déjà engagé n'est répété ; coûts incertains restent réservés. L'UI distingue modèle demandé et modèle de chaque appel.

**Statut après relecture.** Corrigé dans la spécification. `model-routing`, F012, `session-model`, le modèle de stockage et les tables `model_calls`/`model_call_manifests` distinguent désormais la politique de l'invocation et les profils effectifs par appel. [context-manifest.schema.json](../schemas/context-manifest.schema.json) impose les identités d'appel/action et le digest du bundle. Les manifests de requête sont immuables ; transmission/réception et résultat sont des observations distinctes. T-F003-002 exige deux appels/manifests pour le fallback avec bundle identique ; une destination hors politique impose une invocation liée. La qualification réelle reste future.

## FINAL-05 — La restauration d'un vieux snapshot peut faire reculer le domaine du fencing

**Preuves initiales.** [storage-model](../architecture/storage-model.md), « Sauvegarde et restauration », propose d'incrémenter l'epoch restauré ; [sqlite-design](../architecture/sqlite-design.md) exige des fences jamais décrémentés ; [process-supervision](../architecture/process-supervision.md), « Contrôle et fencing », exige qu'un host refuse un epoch/fence inférieur au plus haut reçu. Le protocole host initial n'inclut pas de lignée d'autorité indépendante du snapshot. Changer seulement `stream_id` traite les curseurs des clients, pas l'autorité d'un ancien exécuteur.

**Scénario de rupture.** Backup à epoch 40/fence 7 ; le daemon atteint epoch 100/fence 20 ; le snapshot ancien est restauré et l'epoch devient 41. Un host vivant doit refuser la nouvelle autorité. Le « réparer » en remettant à zéro ses compteurs pourrait réaccepter une ancienne capacité. Les deux exigences, disponibilité de reprise et refus des anciens propriétaires, ne peuvent être satisfaites par une simple incrémentation de la valeur restaurée.

**Solution recommandée.** Définir une `authority_generation` non réutilisable après restauration, des epochs monotones **à l'intérieur de cette lignée**, et une procédure de changement de lignée. L'arbitrage conservateur annoncé exige restauration hors exécution, rotation des capacités et neutralisation vérifiée des anciens workers/hosts avant admission de la nouvelle lignée. Aucun host ancien n'est adopté directement sous une nouvelle lignée ; l'absence de preuve maintient le blocage. Les effets distants demeurent à réconcilier.

**Alternative.** Maintenir un high-water d'autorité hors du snapshot restaurable, avec son propre protocole durable, authentification et collecte de l'état des exécutants. Cette voie peut autoriser des transferts plus souples mais ajoute un état critique dont la perte doit être spécifiée. Elle ne doit pas être introduite implicitement.

**Validation attendue.** Restaurer un snapshot ancien avec host vivant, ancien worker silencieux, capacité ancienne rejouée, ancien fence supérieur au nouveau et effet distant sans receipt. Aucun contrôle ancien ne modifie la nouvelle lignée ; aucun signal ne vise un PID réutilisé ; aucune réadmission si un exécuteur local non neutralisé peut encore agir. Une impossibilité de preuve produit un blocage visible, pas un reset de sécurité.

**Statut après relecture.** Corrigé dans la spécification. `failure-recovery` distingue redémarrage ordinaire et restauration offline en cinq étapes. `process-supervision` exige l'égalité de lignée avant comparaison numérique et interdit l'adoption à chaud d'un ancien host. Le stockage porte `authority_generation` dans autorité, tentatives, actions, processus, leases et fences ; ces derniers sont monotones par lignée. AC/T-F018-007 reprend le contre-exemple epoch 40/100 et les variantes host inconnu, boot différent et effet distant. La neutralisation effective sur macOS/Linux reste une preuve produit entièrement future.

## FINAL-06 — « Compatible » n'est pas encore un algorithme de contrat

**Preuves initiales.** [workflow-compiler](../specs/workflow-compiler.md), « Versions et concurrence », [role-registry](../specs/role-registry.md), « Autonomie N+1 et pinning », et [F007 design](../features/F007-skill-studio-registry/technical-design.md) promettent une compatibilité vérifiée du rôle avec le slot. Le rôle peut composer plusieurs skills ; la tâche `execute_skill` du [DSL](../architecture/workflow-dsl.md) ne nommait pas de skill principal ou d'export de rôle, alors que [model-routing](../specs/model-routing.md) suppose un skill principal pour la sélection du modèle. L'algorithme d'intersection des sorties/contrats et le profil de compatibilité des schémas n'étaient pas définis.

Les verdicts conservent les versions de validateurs, mais le contrat initial ne précisait pas leur pinning **avant** exécution. Une mise à jour de l'évaluateur pendant un run pourrait ainsi changer le sens d'un contrat pourtant inchangé dans le registre.

**Scénario de rupture.** Un rôle combine un skill dont la sortie est `{patch: ...}` et un autre dont la sortie est `{decision: ...}`. Deux équipes choisissent un skill différent, appliquent un schéma différent ou valident une union permissive. Un rôle nouvellement publié prétend satisfaire un slot d'intégration parce que sa sortie contient `integrated`, sans porter la même obligation de preuve. La phrase « contrat compatible » masque la divergence.

**Solution recommandée.** Un choix explicite de skill principal ou d'export nommé, des références exactes, un profil déterministe de compatibilité V1. Une première règle conservative peut exiger profil de contrat exact et égalité des schémas normalisés, puis union des obligations supplémentaires sans suppression d'obligation. Les capacités et permissions restent intersectées. Les versions/digests du catalogue d'évaluateurs, du profil de schéma et de normalisation appartiennent au binding effectif ; une version absente bloque ou exige une migration explicite.

**Alternative.** Un système de sous-typage fermé peut être conçu, avec règles de variance et fixtures négatives. Il doit définir les constructions permises et les cas inconnus/refusés. L'inférence de compatibilité à partir de prose ou d'une validation JSON de forme ne suffit pas.

**Validation attendue.** Deux implémentations indépendantes rendent le même verdict sur rôles compatibles/incompatibles ; champ supplémentaire, champ requis retiré, type changé, contrôle de preuve retiré, permissions élargies, deux skills principaux possibles et mise à jour d'évaluateur sont testés séparément. Le run engagé conserve ses obligations ; les nouveaux skills restent utilisables en V1 sur un slot vierge compatible. Aucun juge LLM obligatoire n'est ajouté pour compenser l'absence d'algorithme.

**Statut après relecture.** Corrigé dans la spécification. `entry_skill` est requis par [role.schema.json](../schemas/role.schema.json) et présent dans les trois fixtures de rôle comme membre exact de `skills`. Le modèle skills définit un algorithme V1 en cinq étapes : références exactes, égalité des digests de schémas, conservation de toutes les règles obligatoires, extensions conjonctives autorisées, plafonds et engagement atomique. [contract-profile.schema.json](../schemas/contract-profile.schema.json) fixe digests et versions compiler/evaluator/catalogue ; une version engagée indisponible bloque la reprise. Les règles de compatibilité et leur fixture sont vérifiables documentairement ; le compilateur et le moteur Rust ne sont pas implémentés. Composition et late binding demeurent V1.

## FINAL-07 — La limite configurée devient ambiguë pendant sa réduction

**Preuves initiales.** L'annexe F et INV-012/051 demandent de ne jamais dépasser la limite configurée ni le plafond 50. [scheduling](../specs/scheduling.md), « Reprise et modifications de limites », [F003 interfaces](../features/F003-agent-runtime/interfaces.md) et [concurrency-tests](../testing/concurrency-tests.md) autorisent C=50→5 avec cinquante exécutions conservées pendant `draining_to_limit` et une exception à l'invariant.

**Scénario de rupture.** Settings affiche « limite 5 », le pool contient encore cinquante invocations et un auditeur conclut que le plafond utilisateur est violé. Une autre équipe force immédiatement l'arrêt et libère artificiellement les quarante-cinq slots, contredisant la reprise sûre. Le mécanisme de drainage est raisonnable ; le problème est l'identité de la limite qu'il prétend appliquer.

**Solution recommandée.** Séparer `requested_limit`, `effective_limit` et état de transition. La demande de cinq gèle les admissions ; la limite effective précédente reste visible jusqu'à ce que les réservations soient réellement ≤5 ; une transaction rend alors cinq effective. Toujours `occupancy ≤ effective_limit ≤ 50`, sans exception cachée. Les coûts inconnus restent suivis indépendamment.

**Alternative.** Refuser une baisse sous l'occupation avec `ResourceBusy`, puis permettre à l'utilisateur de demander une pause/drain avant nouvelle configuration. Cela préserve l'invariant mais ajoute une étape d'usage.

**Validation attendue.** Traces par préfixe pendant réduction, crash pendant drainage, appel distant non annulé, manager urgent et limite un. Aucune libération de slot sur simple demande d'arrêt ; cible demandée et valeur effective visibles dans tous les clients. Les vingt leaders logiques dormants ne sont jamais comptés comme cinquante exécutions.

**Statut après relecture.** Corrigé dans la spécification. `scheduling`, les tables de capacité, les interfaces/tests F003 et `concurrency-tests` distinguent `requested_limit`, `effective_limit` et gel d'admission. L'oracle par préfixe ne contient plus d'exception permettant de dépasser la limite effective. La cible cinq n'est affichée effective qu'après activation transactionnelle à occupation admissible. La réserve managériale souple ou dure reste un arbitrage distinct, et les tests produit sont futurs.

> **Note R7 (16 novembre 2026) :** `draining_to_limit` est une condition dérivée (`requested_limit` < `effective_limit` et `admissions_frozen=1`), non un état persisté, distincte du mode daemon `draining` ([sqlite-design](../architecture/sqlite-design.md)). La réserve managériale est tranchée par Q-15 : réserve souple d’une unité par défaut, option dure si C ≥ 2 ([ADR-13](../architecture/adr/ADR-13-scheduler-budget.md)).

## FINAL-08 — Le rapport historique demande une reconstruction que le stockage ne garantit pas

**Preuves initiales.** [F016 interfaces](../features/F016-reporting-observability/interfaces.md), « Rapport déterministe », accepte `as_of_event_seq`; [F016 design](../features/F016-reporting-observability/technical-design.md), « Contextes et rapports », promet des agrégats sur cette frontière. [event-model](../architecture/event-model.md) et l'[audit de persistance](persistence-and-recovery.md) choisissent un état transactionnel et un journal sous rétention, pas un event sourcing intégral. Aucun protocole initial ne garantit la reconstitution des champs historiques d'un agrégat à toute séquence arbitraire. Les interfaces activité/rapport utilisent également une séquence sans toujours inclure le `stream_id` renouvelé par restauration.

**Scénario de rupture.** La mission à séquence 481 est révisée à 600 ; l'ancien contenu n'est plus dans l'état courant et le journal nécessaire a été purgé. Le rapport « au point 481 » combine état courant et anciennes preuves, tout en se présentant comme historique déterministe. Après restauration, la même séquence numérique peut appartenir à un autre flux.

**Solution recommandée.** Sceller les nouveaux rapports au watermark transactionnel courant `(stream_id, sequence)`. L'accès historique repose sur un `ReportSnapshot` conservé, ou des données historiques dont la couverture est prouvée ; sinon retourner `HistoryUnavailable` ou une lacune explicite. Les abonnements portent le curseur complet et refusent un ancien flux. La rétention des octets et le contrôle d'ACL à la lecture restent distincts.

**Alternative.** Ajouter snapshots historiques et deltas complets avec horizon de rétention, coût, migrations et tests de reconstruction. Ce choix ne nécessite pas forcément tout un event sourcing, mais son périmètre exact doit être démontré avant la promesse de requête arbitraire.

**Validation attendue.** Rapport avant/après mutation, requête sur séquence purgée, restauration avec même numéro dans nouveau flux, révocation entre création et lecture du rapport. Aucun ancien état n'est inventé ; aucune visibilité passée ne contourne une révocation actuelle ; la GUI et CLI montrent les mêmes lacunes.

**Statut après relecture.** Corrigé dans la spécification. F016 capture la frontière courante dans la même vue transactionnelle que les données, expose `as_of_cursor`, relit les rapports historiques déjà scellés et refuse une reconstruction non fondée avec `HistoryUnavailable`/lacunes. `SubscribeActivity` utilise le curseur complet. Le complément des tests T-F016-009/010 exerce mutation, historique non scellé, purge et même séquence dans deux flux. Le besoin V1 de rapports et d'historique reste intégral ; le coût d'un historique arbitraire non conservé n'est plus dissimulé.

## Points contestés sans nouveau défaut établi

### Connaissances entre worktrees et découverte obligatoire

Les propositions distinguent bien `discoverable` et `locally_available`, snapshots committés et dirty, cohérence de capture et simple conservation d'octets, index dérivé et activité durable. Le cas `normalizeEmail/cleanEmail` peut choisir coordination, attente ou copie justifiée ; aucun partage forcé n'est nécessaire. La détection des cycles d'attente est transactionnelle. Ces propriétés sont **spécifiées**, pas prouvées en exploitation.

L'absence d'un reçu antérieur pour une création par shell opaque n'est pas traitée comme une garantie magique : elle doit rester un trou de couverture ou une trace tardive. Cette limite explicite ne justifie pas de retirer le contrôle préalable des opérations structurées identifiables. Les tests futurs doivent exercer rebase, suppression, changement d'ACL et crash entre recherche et usage. FINAL-02 porte sur une contradiction exploitable entre ces composants, pas sur la promesse impossible d'absence de tout doublon.

### Frontière N+1, invocation, run et late binding

Le run terminal et quiescent comme cycle est une proposition conservatrice, **Q-03 reste ouverte**. Les slots tardifs préservent l'autonomie N+1 sans réécrire une étape engagée. Une branche restante ou un effet inconnu conserve la barrière de succession requise. La contre-proposition d'une migration à checkpoint exigerait un mapping précis ; elle n'est pas réputée acceptée par Louis. FINAL-01 et FINAL-06 demandent de rendre la proposition choisie implémentable, pas de trancher l'intention utilisateur à sa place.

### Cinq slots, plafond cinquante et managers urgents

La réserve souple donne cinq N en régime normal et une préemption coopérative pour le manager ; la réserve dure donne quatre N plus une place contrôle. Le dossier ne promet pas de latence universelle si OS, writer ou fournisseur ne répondent pas. Vieillissement, équité interprojets et limite de rafale contrôle sont prévus. Aucune sixième invocation implicite ne doit être utilisée pour débloquer les cinq premières. FINAL-07 affine la diminution de capacité ; il n'impose pas un mode de réserve contre l'autre.

### Reprise, PID, observation tardive et effets ambigus

La distinction entre identité durable, PID, boot, génération de processus et maître PTY est conservée. Un effet ambigu n'est pas rendu rejouable par la seule présence d'une outbox. Un signal envoyé n'est pas une preuve de disparition. Un résultat tardif d'un ancien worker doit perdre son autorité de commande, tout en pouvant servir de **source à authentifier par le réconciliateur actuel** ; ce dernier ne doit pas réattribuer la décision à l'ancien owner. Cette dernière distinction mérite une fixture dédiée lors de l'implémentation, sans annoncer ici un défaut produit observé.

### Code interne Rust, GUI soignée et référence utilisateur

Les projets supervisés multilangages restent V1 ; la règle Rust porte sur le code interne Unleash. La distinction dépendances natives, glue générée et code applicatif est explicite et Q-01 reste ouverte. La référence exacte « **kavi ou st** » n'est pas confirmée ; aucun autre nom ne doit lui être substitué silencieusement. Le screening et l'écran soigné comparatif ne doivent pas se transformer en choix tacite egui. Le design system et les gates d'accessibilité/IME/charge fournissent des critères futurs ; aucun rendu n'a encore démontré la qualité visuelle.

### API, scopes et AI-Guard

Les pages officielles [OpenAI GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol) et [Anthropic Claude Fable 5.2](https://platform.claude.com/docs/en/models/fable-5-2/overview) ont été rouvertes pendant cette revue, le 16 novembre 2026. Elles confirment les identifiants cités, l'exigence Responses pour les tool calls Sol et les restrictions de `tool_choice` documentées pour Fable. Cette vérification documentaire ne confirme pas l'accès d'une clé, les quotas réels ou une intégration exécutée. Les adaptateurs natifs et compatibles gardent leurs profils distincts ; FINAL-04 traite leur continuité inter-appels.

Le dossier distingue scope logique, outil médié et isolation OS réelle. Le mode strict refuse une garantie absente ; le mode standard ne prétend pas empêcher tout programme hostile du même compte. L'API AI-Guard n'a pas été fournie : les ports restent des contrats Unleash proposés, EXT-GUARD reste ouvert. Ce manque de pièce externe ne doit devenir ni une API inventée ni un retrait de F017 de V1.

## Contrôles effectivement effectués pendant cette revue

Le premier passage a validé les six schémas et neuf exemples alors présents. La relecture finale après corrections a porté sur les **dix schémas JSON présents à cet instant** : `workflow`, `skill`, `role`, `ipc-command`, `ipc-receipt`, `ipc-error`, `event`, `context-manifest`, `contract-profile` et `discovery-receipt`. Tous ont passé leur méta-schéma Draft 2020-12. Les **onze exemples JSON** de workflows, skills, rôles, commande IPC, profil contractuel et reçu de découverte ont passé leur validation structurelle, formats activés. Les trois rôles ont également une `entry_skill` appartenant exactement à leur liste `skills`.

Les quatre blocs JSON complets des interfaces F001/F002/F004 — deux commandes, un reçu et un événement — ont été validés contre leurs schémas respectifs. La causalité de l'événement F002 désigne le `receipt_id` du reçu F001, distinct du `command_id` informatif. Retirer `receipt_id` du reçu provoque bien un rejet. Le hash du payload F002 a été recalculé et correspond ; cette fixture à chaînes et entiers, sans nombres flottants, ne constitue pas un test général d'implémentation de la canonicalisation. La lecture du modèle SQLite confirme l'unicité du reçu, sa clé de déduplication par acteur et la FK de causalité différable ; aucun round-trip d'une base Unleash réelle n'a été exécuté.

La matrice de découverte a exercé les douze couples `decision`/`strategy` : les sept couples autorisés passent, les cinq autres sont rejetés. Les variantes sans candidat pour une adaptation ou sans cible pour une réutilisation ont également été refusées au contrôle précédent. Les quarante-trois liens Markdown de cet audit ont été parcourus pour vérifier l'existence des cibles locales. Les corrections de chaque finding ont fait l'objet d'une relecture des sections normatives et des scénarios de qualification associés.

**Ces résultats portent sur les fichiers et les révisions lus, pas sur le nombre final d'artefacts du dossier encore en consolidation.** Ils ne prouvent pas l'existence des références de catalogue, l'authentification, l'exécution du compilateur, la compatibilité sémantique de tous les contrats, la neutralisation OS, le reducer ou les capacités fournisseurs. Aucun code Rust de produit n'a été compilé ; il n'existe pas dans le périmètre fourni. Aucun test, audit ou juge supplémentaire n'est imposé aux workflows utilisateurs par ces vérifications du dossier.

## Jugement après corrections et conditions de clôture

**Les huit objections de cette revue sont levées au niveau de la spécification, après relecture indépendante des correctifs.** Aucune contradiction bloquante résiduelle n'a été établie sur les frontières examinées. L'avis initial opposé au gel des interfaces pour ces huit motifs est donc levé ; cela ne certifie ni l'implémentation future ni le gel global d'un dossier dont les matrices étaient encore en consolidation pendant la revue. Les inconnues d'arbitrage et les pièces externes absentes demeurent identifiées, en particulier la portée Rust/dépendances, la technologie et la qualité GUI, la référence « kavi ou st », la frontière d'adoption des versions et le contrat AI-Guard.

Chaque finding conserve son constat initial, l'option retenue par le coordinateur, les fichiers corrigés et un résultat de relecture explicite. Une contradiction documentaire peut être « corrigée dans la spécification » ; elle ne doit jamais être étiquetée « comportement produit validé ». Les questions GUI, migration et AI-Guard gardent leur autorité d'arbitrage et leur gate.

La clôture documentaire exige des contrats communs cohérents et des tests futurs capables de réfuter les garanties annoncées. Elle n'exige pas d'ajouter des audits, tests métier ou approbations à tous les workflows des utilisateurs. La V1 reste complète dans son périmètre et les inconnues externes restent visibles.
