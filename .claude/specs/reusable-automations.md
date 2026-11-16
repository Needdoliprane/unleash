# Automatisations réutilisables — contrat produit proposé R3

**Date : 16 novembre 2026. Statut : spécification proposée, non implémentée.** La demande confirmée est d’utiliser le LLM pour comprendre ou préparer une procédure, puis de réexécuter cette procédure avec des données variables sans LLM tant qu’elle reste valide. Les noms de données et mécanismes ci-dessous traduisent cette demande ; ils ne décrivent pas des API déjà disponibles. Ils complètent les [workflows](workflow-engine.md), les [goals et missions](goal-management.md), les [skills](skills-and-roles.md) et le [périmètre polyvalent](../brief/general-purpose-scope.md).

## Principe et articulation avec le moteur existant

Une procédure n’est pas un prompt rejoué. Elle décrit une séquence d’actions, les entrées qu’elle accepte, les observations attendues, les effets permis et les conditions dans lesquelles elle doit s’arrêter. Elle peut être construite manuellement, importée depuis une source vérifiable ou préparée avec un LLM. Sa publication engage un artefact versionné distinct des données de chaque exécution.

`AutomationRun` est une projection enrichie d’un `Run` existant, pas un orchestrateur concurrent. Le moteur de workflows conserve états, timers, budgets, autorisation, contrats, journal et récupération. Une feuille d’action déterministe du profil de workflow proposé utilise un catalogue fermé d’opérations typées ; elle ne doit pas créer un faux agent LLM pour appeler un outil. La version du DSL et son schéma doivent explicitement prendre en charge cette feuille avant qu’une implémentation soit déclarée conforme.

Les N+2/N+1 restent responsables selon le mandat ; ils peuvent rester dormants. Si les critères de clôture sont mécaniques, le moteur les applique et conserve leur preuve. Si le contrat exige un jugement non prédéfini, le run attend ce jugement ou échoue selon sa définition. Le contrat ne doit pas déguiser ce jugement en assertion déterministe ni réveiller systématiquement un manager sur chaque réussite.

## Objets et responsabilités

| Objet | Contrat proposé |
| --- | --- |
| `AutomationArtifact` | Identité et version publiées, provenance, schémas d’entrée/sortie, références de données et de sources attendues, contrats, dépendances, plan compilé et qualification de validité. Une version publiée est immuable. |
| `AutomationDataRevision` | Révision immuable des données d’entrée, notamment CSV : contenu ou référence persistée selon rétention, digest, format, mapping validé, nombre de lignes et provenance. Un chemin de fichier seul ne suffit pas à identifier ce qui a été exécuté. |
| `AutomationRun` | Vue du run commun avec artefact exact, révision des données, paramètres résolus, sources autorisées, politique modèle, effets et résultats. L’identité d’exécution n’est pas remplacée par une nouvelle identité à chaque reconnexion client. |
| `ParameterSet` | DTO de présentation des valeurs choisies : date, quantités, destination, variantes autorisées ou autres champs du schéma. Ce n’est pas un registre autonome ni une source d’autorisation. |
| `SourceBinding` | DTO de liaison à une ressource sélectionnée : identité, scope, révision/fraîcheur, compte ou contexte applicables, référence de secret et préconditions. Les secrets eux-mêmes ne sont pas des paramètres ordinaires. |
| `EffectPreview` | DTO de prévisualisation des effets envisagés, dépendant d’un artefact, des données, du mandat et de faits observés. Il distingue ce qui est calculé, observé et encore inconnu. |
| `RunResult` | DTO de résultats, références d’artefacts, lignes traitées ou rejetées, anomalies, effets confirmés ou inconnus et preuves. Le rapport lisible ne remplace pas ces données. |

Les DTO représentent des vues des objets canoniques ; ils ne créent pas des tables obligatoires ou des commandes wire supplémentaires. La résolution des compétences conserve les namespaces et versions du registre existant. `AutomationArtifact` n’est ni un quatrième niveau d’agent, ni un synonyme de rôle, de session ou de worker.

## Politique modèle : propriété vérifiable du run

Le champ canonique proposé est **`AutomationRun.model_policy = forbidden | allowed_bounded`**. La politique effective est validée au démarrage, enregistrée et appliquée à tous les descendants causaux du run. Une politique plus permissive exige une nouvelle exécution explicitement mandatée ; une révocation peut restreindre immédiatement les actions encore possibles.

| Politique | Comportement requis |
| --- | --- |
| `forbidden` | Aucun appel à un LLM local ou distant, directement ou indirectement, depuis l’admission jusqu’à la clôture et dans les traitements descendants nécessaires à ce run. Managers, sous-runs, résumés, recherche demandant un nouvel embedding, réparation et outils auxiliaires respectent cette interdiction. |
| `allowed_bounded` | Les appels modèles sont explicitement déclarés, autorisés et soumis aux budgets de run, mission et projet. Leur usage est visible ; l’interface n’annonce pas une exécution sans LLM. |

### Deux modes : rejeu et supervision par blocs

Le mode de rejeu utilise `forbidden`. Le mode supervisé utilise `allowed_bounded` et déclare les checkpoints ou exceptions auxquels une invocation modèle est utile. Entre deux checkpoints, un bloc de script ou d’actions qualifiées s’exécute sans pilotage modèle de chaque clic. Le superviseur reçoit les résultats structurés, assertions, logs pertinents et limites de collecte ; ces contenus restent des données non fiables, pas des instructions d’autorisation.

Le superviseur peut proposer une continuation, une correction permise, une adaptation ou une escalade dans les commandes, versions, bornes et permissions engagées. Il ne peut réécrire une observation, élargir le mandat ou remplacer silencieusement une procédure publiée. Un effet incertain impose toujours une réconciliation. Une adaptation hors contrat ouvre une activité et une version distinctes.

La politique expose le moment des inspections, leurs entrées, le modèle effectif local ou distant et les budgets. Elle distingue une erreur mécanique déjà traitée par la procédure d’une exception demandant un jugement. Cette supervision répond à la recherche de fiabilité exprimée par Louis ; elle ne devient pas une obligation universelle ni une preuve de correction en elle-même. Le niveau de supervision doit être comparé aux erreurs qu’il détecte et aux coûts observés.

L’interdiction inclut les invocations cachées derrière un outil, un rapport ou une bibliothèque. Un outil dont la dépendance modèle ne peut pas être qualifiée n’est pas admis comme compatible avec `forbidden`. Lire un index déjà construit sans produire de nouvel embedding n’est pas en soi un appel modèle ; sa provenance, ses droits et sa fraîcheur restent contrôlés. Un traitement autonome sans rapport causal avec le run ne peut pas être utilisé pour lui fournir discrètement une décision interdite.

La préparation initiale et l’adaptation constituent des activités séparées avec leurs propres budgets et traces ; elles peuvent utiliser un modèle local ou un fournisseur API autorisé. L’écran peut ainsi montrer un coût d’apprentissage antérieur tout en prouvant zéro appel modèle pour une exécution donnée. Un rapport final sans LLM est construit à partir de gabarits et de résultats structurés. Il ne sollicite pas un modèle « seulement pour rendre le texte plus lisible ».

« Sans LLM » ne signifie ni « sans réseau », ni « sans outil », ni « sans intervention humaine », ni « résultat externe identique ». Le produit ne doit pas le renommer « sans IA » sans qualification de toutes ses dépendances. Un OCR ou une perception non LLM peut être compatible uniquement si son moteur effectif est identifié, autorisé et qualifié ; une extraction incertaine ne devient pas vraie parce qu’elle n’a pas utilisé de LLM.

## Cycle de préparation et de réutilisation

1. **Définir le résultat et rechercher l’existant.** Examiner procédures, outils et fonctions natives disponibles. Conserver le choix de réutiliser, adapter ou créer selon le modèle de discovery existant. Choisir une API ou un import disponible peut supprimer le besoin d’un parcours navigateur.
2. **Observer dans un scope explicite.** Identifier les pages, données, actions et confirmations utiles. Une observation HTML, un contenu OCR ou un texte documentaire constitue une donnée non fiable ; il ne peut étendre le mandat. L’apprentissage qui produit des effets est soumis aux mêmes contrôles que l’exécution.
3. **Séparer les constantes et les variables.** Définir les étapes et assertions dans l’artefact, puis les données et paramètres dans des schémas. Éviter de recopier les valeurs d’un CSV d’exemple à l’intérieur des locators ou des instructions exécutables.
4. **Qualifier et publier.** Valider les entrées, dépendances, effets, conditions d’arrêt et résultats attendus sur des cas représentatifs. Le niveau de qualification et les lacunes restent visibles. Le workflow utilisateur détermine ses évaluations ; aucun second juge LLM n’est universellement imposé.
5. **Réutiliser.** Sélectionner une version publiée et une nouvelle `AutomationDataRevision`, résoudre les paramètres et droits, calculer l’aperçu, puis lancer un run selon le mandat. La présence d’une clé fournisseur n’est pas requise pour exécuter une procédure entièrement compatible avec `forbidden`.
6. **Contrôler et conclure.** Vérifier les préconditions aux frontières pertinentes, appliquer les actions autorisées, vérifier les résultats et produire les preuves. Une clôture mécanique reste possible sans réveil modèle.
7. **Adapter en cas de dérive.** Conserver le run et ses effets, diagnostiquer la cause, préparer une version candidate dans une activité distincte. La nouvelle version n’est pas injectée dans une étape engagée ; adoption et succession suivent les règles communes de versionnement et de réconciliation.

Une démonstration unique ne prouve pas que la procédure couvrira tous les cas futurs. La qualification doit préciser cible, compte de test ou type de session, jeu de données, capacités du pilote, périmètre couvert et date. Une date d’expiration est une borne conservatrice possible ; elle ne remplace pas l’observation de la cible au moment d’agir.

## Données paramétrées et lots CSV

Le schéma d’entrée définit les champs requis, types, unités, valeurs autorisées et bornes. Le parseur choisit explicitement encodage, séparateur, en-tête et conventions numériques/date. Des types comme une quantité, un identifiant et un montant ne sont pas interchangeables par conversion silencieuse. Les cellules contenant des instructions, une commande ou une formule sont des données ; elles ne sont pas exécutées comme du code.

La prévalidation statique porte sur le CSV entier et détecte colonnes manquantes, doublons interdits, lignes vides significatives et valeurs hors domaine. **Une erreur statique empêche tout effet du run.** Le produit fournit les diagnostics pour corriger l’import ; il ne traite pas les lignes valides d’un lot invalide et n’exclut aucune ligne silencieusement. Un filtrage explicite peut préparer un nouvel import, soumis à validation et approbation applicables, qui devient une nouvelle `AutomationDataRevision` avec sa provenance ; il ne modifie pas la révision rejetée.

Un lot statiquement valide peut rencontrer une erreur pendant l’exécution, par exemple une cible devenue indisponible. Ses résultats partiels et effets déjà réalisés restent alors visibles ; ils ne sont pas assimilés à la prévalidation d’un CSV invalide ni à une réussite complète. Dans un parcours `for_each` v1.2, une ligne en échec n’est rejetée sans arrêter le lot que par une route publiée ; sinon le curseur s’arrête ([itération de données](../architecture/workflow-data-iteration.md#curseur-durable-et-effets)).

Chaque ligne porte une identité stable à l’intérieur de sa révision. Sa corrélation avec une action conserve run, étape, ligne et clé d’effet. Deux lignes identiques ne sont pas dédupliquées sans règle : elles peuvent représenter deux demandes légitimes. À l’inverse, un retry ne doit pas interpréter la même ligne comme une nouvelle commande. Une clé locale ne garantit aucune déduplication chez un service distant.

Changer une liste de courses ou la période d’un export dans le schéma autorisé crée de nouvelles données d’exécution. Changer la règle de substitution, le mapping sémantique d’un champ, les cibles d’action ou les contrats modifie la procédure ou son binding selon le contrat. Ces changements sont comparables avant lancement et ne sont pas cachés derrière la formule « seulement des paramètres ».

## Validité d’une procédure navigateur

Le profil navigateur concret de V1 décrit ce que le pilote sait réellement observer et actionner : le pilote de référence est décidé en R7 ([Q-18](../brief/open-questions.md)) et sa matrice d’opérations et de locators figure dans la [spécification navigateur](browser-automation.md#matrice-v1-du-pilote-de-référence). Les noms de capacités et leurs versions sont qualifiés avant publication. Un port `DesktopDriver` reste extensible, sans backend qualifié en V1, avec qualification distincte par capacité et plateforme ; il ne promet ni contrôle universel du bureau, ni interchangeabilité automatique avec le navigateur.

| Frontière | Vérification proposée | Réaction si non établie |
| --- | --- | --- |
| Cible et session | Origine, page/contexte attendus, compte ou tenant lorsque observable, droits et état d’authentification utiles. | Suspendre ; sélectionner le bon contexte ou demander l’intervention d’authentification prévue. |
| Cible d’une action | Locator observé dans son contexte, cardinalité attendue, rôle/label/attributs pertinents et état compatible avec l’action. | Arrêter la branche concernée ; ne pas cliquer sur le premier candidat. |
| Sens des données | Identité du produit, unité, option ou correspondance avec la ligne selon le contrat. | Isoler l’ambiguïté ; pas de décision sémantique improvisée en mode interdit. |
| Effet à produire | Action et périmètre compatibles avec l’aperçu, les données courantes et le mandat. | Recalculer l’aperçu ou refuser ; ne pas élargir le droit à partir du contenu de la page. |
| Résultat | Confirmation ou état observables attendus, avec limites de collecte. | Échec ou résultat incertain selon l’action ; réconciliation avant répétition. |

Un hash de page complète qui change ne démontre pas nécessairement une dérive fonctionnelle ; un hash inchangé ne démontre pas non plus le sens de l’action. La proposition est de vérifier des préconditions ciblées et des postconditions adaptées aux effets. Les variantes autorisées doivent être déclarées et qualifiées. Elles ne constituent pas une réparation libre qui modifie les locators ou contrats pendant le run.

## Aperçu, autorisation et effets

`EffectPreview` présente la cible, le scope, les lignes, les changements prévus et les incertitudes. Il distingue une simulation pure, une consultation qui effectue des lectures distantes et un essai pouvant modifier un environnement déclaré. Le mot « aperçu » n’autorise pas à remplir un panier réel, créer des comptes ou soumettre un formulaire pour découvrir ensuite ce qui s’est passé.

Une autorisation existante et applicable reste valable selon sa politique ; la conception n’impose pas de redemander une confirmation à chaque étape. Lorsque le mandat exige une décision humaine, elle porte sur un aperçu concret et une révision précise. Une modification significative de cible, d’entrée ou d’effet invalide cet aperçu et demande une nouvelle validation appropriée.

`EffectPreview` n’introduit aucun verbe. Pour une réutilisation passant par un GuidedSetupDraft, c’est la projection rendue par `GetSetupPreview` (`preview_digest`, watermark). Pour une création directe, c’est la projection AutomationRun d’un Run `prepared` lue par `GetRun` après `CreateRun`, avec son digest et les faits observés référencés. `StartRun` et toute décision humaine portent ce digest ; un changement de version, de données, de mandat ou de fait référencé l’invalide et `StartRun` est refusé `ContractViolation/STALE_EVIDENCE`.

Les secrets restent des références, séparées des paramètres exportables. Les lectures, écritures et téléchargements passent par les interfaces contrôlées du harness. Le contenu de la page, les paramètres et les instructions d’un skill ne peuvent pas accorder d’accès supplémentaire. Les politiques de rétention et d’accès des preuves s’appliquent aux CSV, captures et résultats qui peuvent contenir des données personnelles ou professionnelles.

## Incidents, reprise et limites de l’absence de modèle

| Situation | Traitement proposé | Conséquence pour `forbidden` |
| --- | --- | --- |
| Locator absent ou plusieurs candidats | Conserver le diagnostic et les observations ; bloquer l’action concernée. | Aucun appel de réparation implicite. |
| Site modifié mais variante déjà qualifiée applicable | Choisir cette variante selon une garde explicite, avec trace. | Reste compatible ; aucune nouvelle décision sémantique. |
| OCR absent, scan illisible ou extraction ambiguë | Signaler la capacité manquante ou la donnée incertaine ; obtenir une correction ou utiliser une capacité préalablement qualifiée. | Un OCR opaque ou fondé sur un LLM ne peut être ajouté silencieusement. |
| Nouveau produit, nouveau champ ou substitution non prévue | Demander une décision ou une adaptation distincte. | Ne pas inventer la règle manquante. |
| MFA, session expirée, consentement ou défi interactif | Mettre en attente l’intervention prévue ; vérifier le contexte après reprise. | L’intervention humaine ne rend pas le run incompatible ; aucun contournement n’est supposé. |
| Coupure après une action d’envoi | Marquer l’effet inconnu ; chercher une confirmation ou un état cible vérifiable. | Le LLM ne peut transformer le doute en succès ; aucun renvoi aveugle. |
| Permission retirée ou budget épuisé | Refuser l’action suivante et conserver le progrès. | Pas de repli sur un autre outil ou modèle pour contourner le refus. |
| Interprétation nouvelle nécessaire au rapport | Fournir les données et limites ; proposer une analyse séparée si demandée. | Le rapport déterministe reste complet sur les faits disponibles, sans narration modèle cachée. |

Pause, annulation et reprise utilisent les états communs. Arrêter un run n’annule pas un effet externe déjà produit. Une compensation n’existe que si elle est définie, autorisée et encore applicable. Les résultats par ligne conservent succès, refus, non traité et effet inconnu sans écraser le détail dans un statut global trompeur.

## Réutilisation et changement de procédure

Relancer la procédure un autre jour avec de nouvelles données appartient à la V1. Le bouton de réutilisation prépare un nouveau Run, avec version sélectionnée, nouvelle AutomationDataRevision et mandat courant. Une procédure plus récente n’est pas automatiquement adoptée si cela change effets, entrées ou permissions. Le Run engagé conserve sa version ; un successeur attend la résolution des effets inconnus pertinents.

**Décision de périmètre Q-21 (résolue R7) : récurrence calendaire exclue de la V1.** Un déclenchement calendaire autonome de nouveaux Runs n’a pas été demandé dans R3–R5 ; Louis décrit notamment le fait de relancer le script le lendemain. La V1 ne crée ni objet RunSchedule, ni opération, groupe de parité ou réglage de planification ([décision](../brief/open-questions.md#q-21--déclenchement-calendaire-de-nouveaux-runs)). Cela ne retire ni réutilisation explicite, ni scheduler V1, ni timers internes de workflow, ni réparation préautorisée. La CLI non interactive reste scriptable : un ordonnanceur externe choisi par l’utilisateur peut l’appeler sous les mêmes mandats et budgets, sans garantie calendaire d’Unleash (échéance manquée, fuseau ou chevauchement). Aucun agent permanent ne regarde l’heure.

## Recette à raccorder à F019

Ces scénarios constituent des critères de conception à intégrer aux exigences et preuves F019. Ils n’ont pas été exécutés sur un produit.

| Parcours | Résultat exigé |
| --- | --- |
| Mission sans dépôt | Créer un goal, lier un CSV et exécuter une procédure sans créer repository, branche ou worktree. |
| Deux jeux de données | Même version de procédure, deux révisions CSV, paramètres et résultats distincts ; aucune recompilation obligatoire pour une variation conforme au schéma. |
| Zéro appel transitif | Routeur modèle indisponible ou appels refusés ; run `forbidden` termine sur fixture stable, rapport compris, avec preuve d’absence d’appel et managers dormants. |
| Outil non qualifié | Dépendance susceptible d’appeler un modèle refusée avant exécution en mode interdit. |
| Prévalidation statique du CSV entier | Une erreur sur n’importe quelle ligne produit des diagnostics et aucun effet ; un filtrage explicite crée une nouvelle révision d’entrée validée, sans exclusion silencieuse. |
| Erreur en cours de lot valide | Décompte cohérent des lignes, effets confirmés ou inconnus et raisons ; résultat partiel clairement distingué d’un succès complet. |
| Dérive de page | Fixture avec locator absent, ambigu ou sens modifié ; arrêt avant l’effet concerné et diagnostic accessible GUI/TUI. |
| Effet inconnu | Coupure injectée après envoi ; reprise inspecte et réconcilie avant toute répétition. |
| Adaptation | Nouvelle version candidate distincte, diff et qualification ; ancien run et ses preuves inchangés. |
| Mandat existant / mandat insuffisant | Exécution autorisée sans confirmation redondante dans le premier cas ; refus clair dans le second. |
| Parité | Mêmes artefact, données, identité et commandes depuis GUI/TUI ; mêmes effets, reçus, erreurs et preuves. |
| Plateforme non qualifiée | Capacité indisponible déclarée ; pas de succès simulé ni de repli desktop caché. |
| Clôture mécanique | Critère observable satisfait clôturé sans appel manager ; critère de jugement non satisfait reste en attente selon le contrat. |
| Supervision par blocs | En mode `allowed_bounded`, deux blocs mécaniques et un checkpoint déclaré : une inspection bornée des résultats, aucune invocation par clic, aucune substitution des faits ou permissions. |

La [revue produit](../audits/general-purpose-product-review.md) expose les solutions alternatives et les tensions de raccordement. La validation documentaire de cette spécification ne démontre pas la disponibilité des pilotes ni la réussite de ces scénarios.
