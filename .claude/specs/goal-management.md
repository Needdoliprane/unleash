# Goals, missions, done et clôture

## Objets distincts

Un goal exprime un résultat utilisateur et ses critères observables. Une mission donne un engagement à un owner avec scope, budget et livrables. Une tâche est un travail confié ; une étape est son placement dans un workflow. Une validation leader est un jugement attribué, pas le nom générique de toutes les transitions. Une permission autorise un effet ; elle ne prouve pas que cet effet atteint le goal.

Les états canoniques sont dans [state-machines.md](../architecture/state-machines.md). Le goal peut rester `active` alors qu'une mission a échoué et qu'une autre solution est préparée. Une mission peut réussir sans que tout le goal soit atteint. L'UI expose séparément ces niveaux pour éviter qu'un compteur d'étapes terminées soit interprété comme résultat final.

## Contrat proposé de goal

Un `GoalDefinition` versionné contient identité, auteur, texte d'intention, critères de succès, exclusions, scope organisationnel, budget, politique de priorité et liste de missions liées, owner (OwnerRef user ou agent N+2), `owner_epoch` incrémenté à chaque réaffectation et `completion_policy` mechanical/human/owner_agent ; sous forbidden, owner_agent est refusé. Un critère comporte type `observable` ou `judgment`, références attendues et autorité de décision. Pour un critère observable, le catalogue des validateurs précise ce qui est mesuré. Pour un jugement, le décideur fournit justification et preuve utile sans promesse d'objectivité absolue.

Un `MissionDefinition` contient goal éventuel, owner (OwnerRef user ou agent) avec `owner_epoch` incrémenté à chaque réaffectation et vérifié par RecordMissionDecision, scope, livrables, done, dépendances, workflow sélectionné et règles de succession des runs. Les dépendances forment un graphe acyclique à la publication d'un plan ; une nouvelle dépendance est une commande révisionnée qui refuse les cycles. Un goal n'autorise pas implicitement chaque action d'une mission : permissions et budgets restent contrôlés à l'action.

## Formation et révision d'un plan

Le N+2 peut proposer plusieurs missions parallèles avec scopes partiellement distincts. Le N+1 affine sa mission, effectue les découvertes nécessaires, choisit ou crée des rôles et sélectionne des skills. Un plan comprend les raisons de séparation, interfaces entre missions, dépendances et stratégies d'intégration. Le harness enregistre le plan et contrôle ses propriétés structurelles ; il ne remplace pas le jugement technique sur le découpage.

Quand une compétence manque, N+1 peut demander au responsable du registre un skill, réutiliser un autre rôle ou justifier une création. N+2 peut déléguer la rédaction technique à un N. La publication et la liaison demeurent distinctes : un skill nouveau devient candidat pour un slot tardif encore vierge, sans changer un binding engagé.

Un changement majeur de scope ou de done est versionné et expliqué. Il ne réécrit pas rétroactivement un run clos. Si l'utilisateur change la finalité pendant un run, le système enregistre l'intention, identifie les travaux encore utiles et prépare une transition contrôlée ; il ne marque pas artificiellement l'ancien done comme satisfait.

## Done et sécurité

Un utilisateur peut définir « développement puis merge ». Le moteur vérifie les contrats et autorisations nécessaires puis accepte l'observation du merge comme done, sans insérer automatiquement tests ou audit. Un autre workflow peut demander tests, audit, revue leader ou validation humaine. Ces choix sont explicites dans la définition effective et les sorties attendues.

Le fait qu'un audit soit facultatif ne rend pas l'autorisation facultative. Une opération interdite reste refusée, y compris si le done demande cette opération. Le système affiche `blocked` avec raison de permission et autorité compétente. Il ne réinterprète pas le refus comme un résultat métier favorable.

## Succession, échec et annulation

Un run suit une définition immuable. La fin terminale déclenche la politique de clôture publiée de la mission : `mechanical` évalue critères observables et quiescence sans LLM ; `human` attend une décision de l’utilisateur ; `owner_agent` sollicite l’owner sous budget lorsque le mode l’autorise. Le profil développement initial peut conserver owner_agent ; une automatisation sans modèle utilise mechanical ou human. Une terminalité seule ne prouve pas le done. Un nouveau run possède nouvel identifiant, lien `predecessor_run_id`, version sélectionnée et raison. Le prédécesseur doit être terminal ; un Run a au plus un successeur direct (Q-03). Les effets incertains du précédent imposent des barrières de succession ciblées. Les budgets de mission ne sont pas remis à zéro par cette création.

Un goal annulé révoque les nouvelles actions de ses missions selon politique, sans prétendre annuler magiquement les effets déjà réalisés. Les artefacts, branches et preuves restent associés à leur propriétaire jusqu'à la politique de rétention. L'annulation est un événement attribué avec motif et horizon d'effet ; un processus externe encore vivant devient un incident visible.

## Vérifications

Le scénario A doit montrer plusieurs features parallèles reliées à un goal, avec owners, scopes et dépendances observables. Le scénario L doit démontrer done=merge sans étape de test ajoutée. Le scénario K doit montrer que publication v2 n'altère pas les runs v1, et préciser si un slot tardif a été lié sans substitution. Les scénarios d'échec vérifient qu'un goal ne passe pas `achieved` uniquement parce que toutes les missions sont terminales.

Cette spécification est une proposition de conception ; les tests de produit sont planifiés dans [F004](../features/F004-hierarchical-orchestration/tests.md) et [F005](../features/F005-workflow-engine/tests.md).

## Missions généralistes et démarrage guidé

Le projet peut contenir des SourceBinding de fichiers, CSV, documents, comptes navigateur ou API sans repository. Les references Git sont optionnelles hors mission de code. MissionDefinition précise OwnerRef de type user ou agent, completion_policy, automation_mode (`replay` ou `supervised`) lorsque applicable et références de sources ; l’utilisateur propriétaire n’est pas un quatrième niveau d’agent. Un run de procédure publiée peut rester sous responsabilité humaine sans aucune invocation N+1/N+2.

Le démarrage guidé matérialise les mêmes objets versionnés que le mode expert. Un template peut créer N+2/N+1/N logiques nécessaires selon les permissions et seulement à bon escient ; aucun agent ne consomme de slot par sa simple création. Les paramètres par défaut sont exposés en langage courant et modifiables. Une procédure recommandée n’ajoute pas un audit à un workflow personnalisé existant.

La supervision par blocs suit des points de décision publiés, compare observations et assertions métier, puis émet des commandes autorisées. Une ligne de log annonçant une réussite reste une déclaration sans observation suffisante. Sous model_policy=forbidden, clôture, résumé, embedding et traitement d’erreur ne déclenchent pas de modèle.

## Contrat de clôture explicite R6

La soumission de jugement utilise [RecordMissionDecision et RecordGoalDecision](../features/F004-hierarchical-orchestration/interfaces.md), avec critères, owner/epoch, révision et evidence bundle exacts. AcceptStep demeure limité à une étape. La politique mechanical appelle le même domaine de clôture sans modèle ; human/owner_agent fournissent un verdict sous leur autorité. Une décision négative distingue rework/escalate/fail et ne force jamais un succès. Réaffectation ou critères modifiés rendent le jugement précédent inapplicable ; les preuves historiques restent consultables.
