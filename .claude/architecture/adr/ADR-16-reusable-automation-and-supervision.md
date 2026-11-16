# ADR-16 — Automatisations réutilisables et supervision par blocs

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : pilote navigateur de référence (Q-18), `project_script` de référence, templates du parcours guidé (Q-20) et absence de déclenchement calendaire (Q-21) ; voir la section « Amendement R7 ». Les besoins R3/R5 restent confirmés par Louis. Date initiale : 16 novembre 2026. Références : INV-007/016/017/018/058/059/060/061/062/064/065, H-17/18/19/21, R-26/27/28/30 ; Q-18, Q-20, Q-21 ; F004/F005/F006/F008/F011/F014/F015/F019.

## Contexte et objection centrale

Louis veut réutiliser les mécanismes de développement pour des usages courants : apprendre une procédure, l’exécuter de nouveau avec des données différentes et faire intervenir un modèle uniquement lorsque cela apporte une décision utile. Il précise qu’une supervision des logs et résultats peut améliorer la fiabilité sans produire un clic modèle par action.

Cette supervision est utile, mais ne constitue pas une preuve à elle seule. Un script peut retourner zéro, produire un log rassurant et modifier la mauvaise ressource. Le modèle doit recevoir des observations corroborées et les limites de capture ; la mécanique doit empêcher les répétitions ambiguës et les effets non autorisés indépendamment du jugement du modèle.

## Options

| Option | Avantages | Inconvénients et risques | Complexité / impact V1 |
| --- | --- | --- | --- |
| LLM décide chaque clic | S’adapte à une page nouvelle, faible préparation initiale | Coût et latence à chaque action, variabilité, preuves métier toujours nécessaires | Boucle simple en apparence, fiabilité difficile ; ne satisfait pas le replay demandé |
| Enregistrement brut de clics ou références de snapshot | Facile à enregistrer | Références périmées, données figées, aucune gestion des exceptions ni paramètres | Faible coût initial, maintenance fragile ; insuffisant comme artefact publié |
| Script généré libre avec logs | Portable, testable avec l’écosystème existant | Effets opaques, contournement de règles, logs incomplets, reprise ambiguë | Utile comme artefact de projet ; garanties à qualifier au niveau du processus |
| Procédure fermée, ports médiés, contrats et données séparées | Rejeu contrôlé, preuve de zéro appel modèle, mêmes règles de reprise | Catalogue d’opérations et qualification de drivers nécessaires | Réutilise le moteur existant ; retenue pour l’automatisation autonome |
| Procédure précédente avec checkpoints LLM publiés | Décisions sur exceptions et résultats sans raisonnement permanent | Coût de supervision, jugement faillible, boucle possible si non bornée | Même DSL et budgets ; second mode retenu, sans moteur concurrent |
| Nouvelle plateforme RPA indépendante | Fonctionnalités métier dédiées | Deux états, deux schedulers, doubles droits et reprise | Coût non justifié ; rejeté pour ce dossier |

## Décision

AutomationArtifact lie une version de workflow compilée par mode supporté, ses paramètres, contrats, opérations/driver, politiques et évaluations. AutomationDataRevision fige les données d’un lancement. AutomationRun reste la projection d’un Run créé par `CreateRun`, avec la source `automation_version` ; les commandes de lancement, pause et reprise sont communes.

Le profil [workflow v1.1](../automation-execution-profile.md) ajoute `kind=action`. Les opérations sont enregistrées et versionnées ; un modèle ne crée pas une nouvelle primitive d’autorité en lui donnant un nom. Les guards et paramètres restent un langage fermé. `kind=task` reste la frontière des invocations d’agents. Les plans publiés sont immuables.

Les modes `replay` et `supervised` répondent à deux besoins distincts. Replay impose zéro dispatch modèle, local ou distant, dans le périmètre contrôlé du Run. Supervised utilise des tâches `leader_decision` et des branchements déclarés, avec politique de fréquence, budgets et preuves requises. Un petit exemple peut faire douze actions en trois blocs et au plus trois appels ; cette valeur est une fixture de qualification, pas une règle universelle.

Les exceptions métier anticipées deviennent des ObservationReport typés puis des `choice` explicites. Aucun mécanisme caché de `catch/on_error` ne réveille librement un manager. Une exception technique non gérée bloque ou échoue. Un effet `unknown` reste inconnu même après un avis favorable ; seules observation, réconciliation ou escalade autorisées peuvent lever le blocage. Corriger la procédure exige un job de conception et une nouvelle version.

## Amendement R7

1. Les mécanismes sont adoptés : AutomationArtifact, AutomationDataRevision et AutomationRun comme projection d’un Run créé par `CreateRun` ; profils v1.1 (`kind=action`) et v1.2 (`for_each`, [ADR-18](ADR-18-bounded-data-iteration.md)) ; mode `replay` sous `model_policy=forbidden` transitif ; mode `supervised` avec tâches `leader_decision` aux checkpoints publiés ; réparation par job distinct et nouvelle version ; pool global 5/50 partagé ; deux contextes navigateur par défaut.
2. Pilote de référence du profil `mediated` (Q-18) : WebDriver W3C piloté en Rust (`fantoccini`) sur Chrome for Testing et chromedriver du même build, épinglés par version et SHA-256. Leur préparation est une étape explicite sous mandat, réalisée par le parcours de configuration existant ou une exécution F011 suivie, sans nouvelle commande ; aucun téléchargement implicite au démarrage, à l’ouverture d’un écran ou pendant un Run. Firefox est optionnel, Safari n’est pas pris en charge en `mediated` V1 et Playwright MCP n’est pas un backend V1. Détail : [exécution navigateur](../browser-execution.md).
3. Le `project_script` de référence est Playwright Test sous F011, avec lockfile ; ses garanties d’observabilité restent celles d’un processus opaque.
4. Le mode guidé instancie les six templates intégrés de Q-20 ; `template.reconcile-csv`, sans modèle, est recommandé par défaut ([parcours guidé](../../design/guided-automation-experience.md)).
5. Aucun déclenchement calendaire autonome de nouveaux Runs en V1 (Q-21). Restent la relance explicite d’une procédure publiée avec de nouvelles données, les timers internes des workflows et la CLI non interactive qu’un ordonnanceur externe choisi par l’utilisateur peut appeler.

## Autorité, supervision et autonomie

Le modèle propose `continue`, `reconcile`, `correct` ou `escalate` sur un bundle précis. Le harness vérifie schéma, référence, fraîcheur, branche permise et autorisation. Ces contrôles ne garantissent pas la qualité sémantique de son jugement. Aucun second juge modèle obligatoire n’est ajouté.

Le mode guidé instancie des templates lisibles avec paramètres et périmètre. Il crée les rôles/agents nécessaires sous les permissions en vigueur ; un parcours sans modèle n’en requiert pas. Les autorisations peuvent être préconfigurées. Les actions sensibles ne demandent une décision nouvelle que lorsque le mandat existant ne les couvre pas. La simplicité d’interface ne confère aucune autorité supplémentaire.

N+2 reste gestionnaire des objectifs, rapports et compétences ; il ne vérifie pas chaque clic. N+1 supervise les blocs de sa mission lorsque le plan le demande. Les réveils demeurent événementiels et bornés. L’exécution mécanique est du ressort du harness. Les travaux développement conservent leurs branches, worktrees et preuves Git ; ils ne deviennent pas obligatoires pour classer des documents ou comparer des fichiers.

## Frontière du navigateur et des scripts

Le profil `mediated` appelle des primitives contrôlées ; `project_script` exécute un artefact de projet lisible et versionné. Un domaine autorisé ne distingue pas une préparation de panier d’un achat sur ce même domaine. La garantie de contrôle exige une frontière d’action effective et une qualification du processus, de ses dépendances et de son confinement. À défaut, le produit indique le niveau réellement assuré et refuse d’attester une propriété non prouvée.

Les scripts générés et runtimes externes ne remettent pas en cause le cœur Rust. Playwright documente JavaScript/TypeScript, Python, Java et .NET ; le dossier ne prétend pas disposer d’un SDK Rust officiel. Le pilote de référence du profil `mediated` est WebDriver W3C piloté en Rust (amendement R7, Q-18). [Recherche navigateur](../../research/browser-automation-sources.md).

## Reprise, ressources et coûts

Les effets suivent intention durable, autorisation, dispatch, observation et réconciliation. Un clic dont l’accusé est perdu n’est pas automatiquement réessayé. Un navigateur restauré doit rétablir identité, page et préconditions ; la mémoire interne du script n’est pas une session durable.

AgentInvocation et ReplayWorker partagent le plafond global 5 par défaut, 50 maximum ; le reducer ne consomme pas une unité par transition. Les contextes navigateur et processus ont des réservations distinctes. Le défaut de deux contextes reste un paramètre à mesurer, jamais une garantie matérielle.

Learn et Repair ont leurs budgets propres. Une politique bornée peut préautoriser la réparation ; aucune confirmation humaine permanente n’est imposée. Le run initial ne change pas de politique en vol. Les totaux de mission présentent apprentissage, supervision, replay et réparation, même si le compteur du replay seul vaut zéro.

## Recette et conditions de révision

T-E2E-W vérifie deux jeux de données, absence indépendante de trafic modèle, dérive et résultat métier. T-E2E-X mesure appels par blocs, erreurs de jugement, logs trompeurs et effets inconnus. T-E2E-Z vérifie le novice sans agent, les permissions compréhensibles et la parité TUI. Les dix-huit chaînes de [F019](../../features/F019-reusable-automations/tests.md) détaillent les preuves. La conformité du JSON n’est pas une preuve d’exécution.

Réviser le choix si le catalogue fermé empêche un usage validé : préférer un nouvel adaptateur qualifié ou un script avec garanties explicites avant d’ouvrir un langage arbitraire dans le moteur. Réviser le défaut UX si les essais montrent incompréhension du résultat ou des autorisations ; ne pas résoudre ce problème en masquant les limites.

## Sources et statut de vérification

[Locators](https://playwright.dev/docs/locators), [actionability](https://playwright.dev/docs/actionability), [Tracing API](https://playwright.dev/docs/api/class-tracing), [langages](https://playwright.dev/docs/languages), [Playwright MCP](https://github.com/microsoft/playwright-mcp), consultés le 16 novembre 2026. Ils établissent des capacités et limites de composants, pas une preuve que l’intégration Unleash existe. Aucun parcours marchand réel n’a été exécuté.

