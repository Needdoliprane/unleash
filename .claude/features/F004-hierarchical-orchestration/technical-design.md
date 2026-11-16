# F004 — Design technique proposé

## Composants Rust

`OrganizationService` gère l'arbre et les owners ; `MissionService` gère engagements et dépendances ; `LeaderDispatcher` transforme les événements pertinents en demandes d'invocation ; `DecisionReducer` contrôle les commandes de leaders. Ces noms sont des modules internes proposés, pas des crates externes ; leurs objets sont des modules logiques projetés sur les tables de [sqlite-design](../../architecture/sqlite-design.md#tables-r6-workflow-et-hiérarchie). Ils utilisent le domaine pur, le stockage transactionnel et le scheduler communs. Les clients n'implémentent aucune copie de ces règles.

`agent` stocke identité, niveau et session. `mission` stocke scope, owner et révision. `assignment` associe agent, rôle lié, mission, scope et epoch. `mailbox_item` référence événement, destinataire, visibilité, état de traitement et règle de réveil. `leader_decision` référence commande, bundle de preuves et révision examinée. La section « Tables R6 workflow et hiérarchie » de sqlite-design fixe leur correspondance (`agents`, `missions`, `mission_assignments`, `mailbox_items`) ; `leader_decision` y est un événement de décision lié à son receipt. Les relations conservent l'historique ; changer owner ne réécrit pas le créateur original.

## Déroulement d'une décision

Le dispatcher sélectionne un ensemble borné d'événements non traités selon priorité, ancienneté et politique de coalescence. Il construit un manifest de contexte avec sources autorisées et lacunes. Le scheduler admet une invocation lorsque la capacité existe. Le leader propose un lot borné de commandes ; chaque commande doit référencer les agrégats et bundles examinés. Le lot dépendant est appliqué atomiquement ou refusé dans son ensemble. Les opérations indépendantes peuvent être proposées séparément.

Le reducer valide autorité, profondeur, absence de cycle, révision, grants et budget. Le nouveau state, les événements, les messages et le reçu sont écrits ensemble. Aucune requête LLM ne se produit dans cette transaction. Une réponse obsolète reste consultable comme proposition rejetée, sans effet. Cette séparation permet de comprendre pourquoi le leader a demandé une correction même si une autre décision a gagné la course.

## Création directe et réaffectation

`CreateAgent` de niveau N issu de N+2 exige une mission existante ou une création de mission dans le même lot. L'owner est N+2 ou N+1 autorisé ; la profondeur maximale est vérifiée sur les relations organisationnelles, pas sur une chaîne de prompts. Le N ne reçoit pas de droit de création de sous-agents. Une affectation de tâche utilise un rôle et un skill exacts avec scope propre.

Une réaffectation fait avancer l'epoch, révoque les nouvelles actions de l'ancien owner et enregistre les travaux en vol à réconcilier. Le nouveau owner voit les preuves autorisées et l'état de récupération. L'ancien processus peut terminer un calcul local, mais son résultat ne peut entraîner une mutation hors epoch. Les actions externes déjà engagées gardent leur identité et ne sont pas répétées sans observation de résultat.

## Scheduling et managers urgents

La création de vingt leaders ne crée pas vingt boucles tournantes. Ils restent dormants en l'absence d'événements. Avec cinq N actifs, une urgence N+2 est mise en file prioritaire ; le scheduler applique la réserve de contrôle Q-15 ([scheduling](../../specs/scheduling.md)), demande une interruption coopérative à un worker admissible ou attend la prochaine libération. Il ne crée pas un sixième slot. Les travaux non interrompables rendent l'attente visible. Vieillissement et quotas empêchent une priorité manager permanente d'affamer les réalisateurs.

## Messages, erreurs et observabilité

La transaction `SendHumanMessage` contient message, événement et notification N+2. L'ACL détermine si le contenu ou seulement un pointeur de visibilité est exposé. Le réveil est ensuite évalué séparément. Les erreurs proposées incluent `InvalidHierarchy`, `MissingOwner`, `ScopeDenied`, `RevisionConflict`, `StaleEvidence`, `StaleEpoch`, `InvocationBudgetExceeded` et `NoProgressLimit`.

Les métriques suivent temps d'attente, invocations par niveau, rejets sur révision, messages non traités et cycles sans progression. Les logs expurgés portent identités d'agrégat et corrélation ; ils ne contiennent ni secrets ni raisonnement privé. Un compteur de tâches n'est pas présenté comme mesure automatique de qualité métier.

## Mission sans dépôt et préparation guidée R3–R5

OwnerRef distingue user et agent ; MissionDefinition enregistre completion_policy mechanical/human/owner_agent. La propriété humaine n’ajoute aucun quatrième niveau d’agent. Un plan mécanique sous forbidden peut créer et clore la mission sans invocation. Un plan supervisé invoque les rôles liés aux checkpoints déclarés et ne reçoit pas un manager permanent.

GuidedSetup est un brouillon de configuration du même backend. Son Apply vérifie versions, droits, révision et aperçu, puis crée de manière atomique/idempotente les objets nécessaires dans son mandat. Aucune saisie novice ne peut élargir les grants du rôle composé. Les références de sources peuvent viser fichiers, documents ou comptes sans repository ; les références Git sont obligatoires seulement pour les opérations qui en dépendent. Les contrats d’interface et leur liste de commandes sont dans [interfaces](interfaces.md).
