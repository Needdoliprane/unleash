# F005 — Design technique proposé

## Noyau et persistance

`WorkflowReducer` est une fonction de domaine qui reçoit état, définition, commande et faits autorisés ; elle retourne nouvel état, événements et intentions. `RunRepository` écrit la transaction locale. `EffectDispatcher` consomme l'outbox, et les adaptateurs renvoient des observations. `ContinuationDispatcher` reprend les microsteps lorsque la limite par transaction est atteinte. Ces modules sont Rust et leurs noms sont proposés.

Le stockage contient `workflow_run`, `node_activation`, `run_binding`, `budget_ledger`, `timer`, `effect_intent`, `effect_observation` et reçus de commandes ; ce sont des modules logiques projetés sur les tables de [sqlite-design](../../architecture/sqlite-design.md#tables-r6-workflow-et-hiérarchie) (section « Tables R6 workflow et hiérarchie », par exemple `run_binding` → `registry_bindings`, `timer` → `workflow_timers`). La clé d'activation combine run, nœud, vecteur d'itérations et tentative logique d'étape. Cette tentative logique est distincte de l'`attempt_id` de processus : après crash, une invocation peut reprendre sous la même `invocation_id` avec nouvelle tentative de worker sans réactiver la tâche ni réinitialiser ses budgets. Le journal garde les raisons et versions de reducer. Un manifest lie définition, versions de rôles/skills engagés, contrats, scopes et contexte sans enfermer la session dans la RAM d'un processus.

## Contrôle et données

L'AST structuré est interprété avec une pile de frames persistables. Chaque frame contient position de séquence, état des branches, compteur de boucle et projection locale. Le moteur refuse une lecture d'une branche concurrente ou d'une ancienne itération qui n'est pas valide dans cette portée. Les guards lisent des projections de faits déjà observés, avec valeurs typées et état de fraîcheur. Ils ne font aucun appel fournisseur.

Le reducer traite un événement puis ses conséquences pures, jusqu'à stabilité ou à 128 microsteps. Dans le second cas il persiste une continuation avant de céder l'écrivain. Les décisions de choix suivent l'ordre déclaré ; un résultat inconnu qui pourrait changer le choix bloque. Les branches exécutables sont mises en file logique, le scheduler décidant de leur admission effective.

## Jointures et clôture

Une jointure all attend toutes les réussites ; any_success et quorum enregistrent le premier seuil atteint selon l'ordre du journal. Les branches devenues inutiles reçoivent une annulation et une révocation de leurs futures actions. Le bloc attend la réconciliation des effets en vol. Un effet inconnu reste visible avec une intention terminale distincte ; un écran ne doit pas afficher réussite finale alors qu'une branche peut encore modifier le dépôt sous l'ancienne autorité.

La réussite du body déclenche l'évaluation du done. Vrai mène à succeeded, faux à failed avec raison, inconnu à blocked sous deadline. Une annulation clôturée peut conserver un incident résiduel et une barrière de succession ciblée. Un nouveau run ne réessaie pas l'effet incertain simplement parce que l'ancien run est marqué cancelled.

## Versions et autonomie

Le workflow et son digest sont fixés à CreateRun. Un slot fixe résout ses dépendances à ce moment. Un slot tardif déclare contrat minimal et plafond de capacités ; BindRole peut le compléter avant première activation. Le manifest reçoit des événements append-only. Le slot devient ensuite immuable pour les tentatives et les itérations du run. N+1 peut ainsi utiliser une compétence nouvellement créée sur travail encore non engagé sans hot swap invisible.

Publier v2 émet un événement de registre et change éventuellement un alias pour futurs runs. Cela ne déclenche aucune mutation de l'AST courant. La migration à checkpoint est écartée de la V1 (Q-03) ; adopter v2 se fait par CancelRun puis successeur explicite, ou à la fin terminale. La reprise après crash exige la même définition et les mêmes bindings existants ; si un bundle manque, le système bloque avec erreur d'intégrité.

## Erreurs et risques

Les erreurs de parsing appartiennent au compilateur ; les erreurs runtime sont celles du [tableau de domaine](../../specs/workflow-engine.md#erreurs-de-domaine-proposées), avec leur forme wire. Les erreurs sont durables et attribuées à l'activation. Une panne fournisseur ne devient pas automatiquement échec métier ; sa politique de retry s'applique dans la borne et uniquement lorsque l'effet est réconciliable. Le moteur n'effectue aucune I/O lente dans la transaction et ne promet pas de finir un appel distant qui ne répond pas.

## Actions sans agent et supervision R3–R5

Une activation action résout l’opération et ses entrées dans le manifest épinglé, puis admet un replay_worker pour cette activation sous le pool partagé. La frame et le journal d’effet sont communs au moteur. Une activation task matérialise l’invocation agente habituelle. Un simple branchement ou guard reste une transition pure ; il ne crée ni worker ni appel modèle. Le Run expose source.kind workflow_version/automation_version et automation_mode sans seconde state machine.

La sortie d’une collecte d’observation peut être complète tandis que l’effet demeure unknown. Le contrat ObservationReport distingue les deux ; une route publiée vers supervision n’efface pas la barrière d’effet. Les états de done, droits et réconciliation restent ceux du moteur commun. Le manifest de modèle n’existe que pour les appels réels ; le replay possède son manifest d’exécution et un compteur modèle nul vérifié indépendamment.

## Parcours de données R6

Les lots génériques utilisent le [profil v1.2 for_each](../../architecture/workflow-data-iteration.md), explicitement sélectionné et épinglé. Body reçoit une ligne immuable dans sa portée /items ; after_batch reçoit un résumé/référence de résultats de lot. Le curseur et les activations appartiennent au writer/reducer commun, pas au driver. La borne statique suit workflow-data-iteration : 1 + max_items×B(body) + ceil(max_items/batch_size)×B(after_batch), arithmétique vérifiée et refus au-delà de 100 000 ; validation des données et budgets avant premier effet. Une collecte unknown ne peut faire avancer le curseur pour atteindre une supervision future : une route de diagnostic doit être présente à l’endroit réellement bloqué.

## Diagnostic pendant EffectUnknown

La [machine de Run](../../architecture/state-machines.md) conserve blocked et les barrières tout en autorisant microsteps purs et activations diagnostiques publiées à capacités réduites. Un choix ou un avis continue ne lève pas le blocage. Seule une observation/disposition autorisée résout les actions concernées avant ResolveBlock. Route absente : attente des commandes de contrôle ; forbidden : aucune tâche modèle, même pour diagnostic. L’ordre de commit empêche une fenêtre d’admission normale entre collecte et blocage.
