# Revue produit contradictoire R3 — usages polyvalents et automatisations

**Date : 16 novembre 2026. Auteur : sous-agent documentaire `generalist_product_r3`. Nature : analyse de conception du périmètre généraliste.** L’auteur a également rédigé les trois documents R3 cités ci-dessous ; cette revue n’est donc pas présentée comme une revue indépendante de ces documents. Aucun code produit, pilote navigateur, procédure marchande ou intégration comptable n’a été exécuté.

## Conclusion de conception

L’élargissement est cohérent si Unleash reste spécialisé pour le développement tout en organisant son domaine commun autour du travail, des sources, des procédures et des résultats. Il serait incohérent de conserver un dépôt ou une invocation agente comme préalable universel, puis de changer seulement les libellés de la GUI. La polyvalence n’impose pas de promettre une automatisation universelle de tout ordinateur.

La voie recommandée est hybride : préparer avec un LLM lorsque cela apporte de la valeur, publier une procédure vérifiable, l’exécuter avec des données variables et réserver la réanalyse à une activité distincte. Cette recommandation ne démontre ni sa faisabilité complète ni une économie mesurée. Les mécanismes doivent encore être raccordés, implémentés et qualifiés. F001–F018 sont conservées ; F019 est le découpage proposé pour le contrat d’automatisation réutilisable.

La clarification complémentaire ajoute une supervision possible entre blocs de script ou sur exceptions. Elle est compatible avec ce cadrage si le mode `allowed_bounded` expose les checkpoints et leurs coûts, tandis que `forbidden` reste strict. Un modèle supervise des résultats lorsque cela est prévu ; il n’est pas requis à chaque clic et ne constitue pas un juge universel. Les modèles locaux et fournisseurs API autorisés sont éligibles selon leurs capacités.

Le périmètre navigateur structuré donne une cible concrète à la qualification. Un port `DesktopDriver` permet l’extension mais ne justifie pas une promesse de contrôle général du bureau. Unleash ne devient pas, par le seul mot « polyvalent », une application comptable réglementaire, un connecteur de courses opérationnel ou un remplaçant de tous les logiciels métier.

## Base examinée et limites de cette revue

La lecture a porté sur la [vision R2 puis sa révision](../brief/vision.md), la [politique d’implémentation](../brief/implementation-policy.md), le [périmètre V1](../brief/product-scope-v1.md), les [invariants](../brief/invariants.md), la [hiérarchie](../specs/agent-hierarchy.md), les [goals](../specs/goal-management.md), les [workflows](../specs/workflow-engine.md), le [DSL](../architecture/workflow-dsl.md), les [skills](../specs/skills-and-roles.md), le [modèle de contrats](../architecture/skills-contract-model.md), l’[architecture de l’information](../design/gui-information-architecture.md), les [principes de design](../design/design-principles.md) et la [parité](../design/gui-tui-parity.md). Les README F005, F007, F010 et l’audit R2 frontend/terminal ont également été examinés.

Les constats sur les formulations initiales portent sur le point de départ R2 lu avant consolidation R3 ; ils ne prétendent pas que ces formulations subsistent après harmonisation du dossier. Les propositions de cette revue sont consignées dans [general-purpose-scope](../brief/general-purpose-scope.md), [reusable-automations](../specs/reusable-automations.md) et [guided-automation-experience](../design/guided-automation-experience.md). Les choix de design sont des raisonnements proposés, pas des faits externes ni des décisions attribuées à Louis sans confirmation.

## Objections et conditions de résolution

| Repère | Objection | Conséquence et réponse proposée | Preuve future nécessaire |
| --- | --- | --- | --- |
| R3-PROD-01 | La vision initiale définit surtout des équipes qui développent des logiciels. | Un particulier ne doit pas comprendre Git pour démarrer. Généraliser le domaine goal/mission/run et rendre le dépôt facultatif. | Parcours complet sans repository ni worktree, avec sources, résultat et historique. |
| R3-PROD-02 | Le vocabulaire et le mode normal F010 peuvent être interprétés comme « une branche par mission, quelle que soit sa nature ». | Restreindre cette règle aux missions ayant des ressources Git. Conserver l’intégralité de F010 pour le développement. | Cas Git inchangés et cas non Git sans faux objets de code. |
| R3-PROD-03 | Le profil initial du DSL active les tâches via des bindings de rôles ; un outil déterministe peut alors sembler exiger un agent. | Ajouter un chemin typé d’action déterministe dans le moteur existant. Ne pas créer un faux N ni un second orchestrateur. | Run sans invocation modèle, états/reçus et contrôles de l’orchestrateur commun. |
| R3-PROD-04 | Un owner qui « examine » chaque clôture peut être compris comme un appel N+1 obligatoire. | Rendre mécanique la clôture autorisée dont les critères sont observables. Les jugements restent distincts et explicites. | Rapport et clôture sans manager invoqué ; attente si un jugement réellement requis manque. |
| R3-PROD-05 | « Sans LLM » peut masquer résumé, embedding, sous-run ou réparation appelant un modèle. | Politique `AutomationRun.model_policy` transitive, contrôlée à l’admission et à chaque frontière d’outil ; rapport déterministe. | Refus des dépendances non qualifiées ; traces et essais négatifs de tous les chemins déclarés. |
| R3-PROD-06 | Un parcours enregistré une fois peut être confondu avec une compréhension fiable de tous les états futurs. | Publier les préconditions, variantes, postconditions et limites de qualification. Détecter la dérive et demander une adaptation distincte. | Fixtures de page ambiguë, sens de champ modifié, session différente et résultat inconnu. |
| R3-PROD-07 | « Changer le CSV » peut aussi changer le sens des actions si le mapping ou les règles sont implicites. | Séparer version de procédure, révision des données et paramètres typés. Versionner une vraie modification de règle. | Même procédure avec deux jeux conformes ; rejet ou révision pour changement sémantique hors schéma. |
| R3-PROD-08 | Un débutant peut être enfermé dans une interface simplifiée et la TUI réduite aux experts. | Séparer domaine métier, densité guidée/experte et choix d’interface. Même accès aux fonctions et preuves dans tous les cas autorisés. | Parcours guidé complet en TUI, édition et configuration incluses. |
| R3-PROD-09 | Un aperçu peut être fabriqué en produisant déjà les effets dans le compte réel. | Distinguer simulation, consultation et essai avec effets. Montrer les limites plutôt que promettre un aperçu exact inaccessible. | Zéro écriture métier cachée en aperçu ; effets de qualification annoncés et autorisés. |
| R3-PROD-10 | Relancer un lot après erreur peut répéter des commandes déjà réalisées. | Identités de lignes et effets, résultats partiels, barrières de réconciliation ; aucun exactly-once distant inventé. | Coupure après envoi ; reprise sans répétition aveugle et état incertain visible. |
| R3-PROD-11 | L’exemple comptable peut être lu comme promesse de comptabilité réglementaire ou d’écriture dans un logiciel métier. | Limiter le cas décrit au rapprochement explicite de données et à son rapport d’écarts ; qualifier toute écriture séparément. | Sorties séparant correspondances, ambiguïtés et écarts, sans validation réglementaire prétendue. |
| R3-PROD-12 | « Piloter l’ordinateur » peut être lu comme support de tous les OS, logiciels et contenus visuels. | Qualifier le navigateur concret et chaque capacité d’extension desktop. OCR, MFA et perception restent des dépendances ou interventions explicites. | Refus compréhensible d’une capacité non prise en charge ; pas de repli opaque. |
| R3-PROD-13 | Le premier parcours peut encore supposer un agent existant ou demander un workflow technique. | Proposer intention ou procédure recommandée, étapes lisibles et création/sélection des rôles et agents selon mandat. | Premier lancement depuis un espace vide sans préconfiguration d’agent ; détails avancés disponibles en GUI/TUI. |
| R3-PROD-14 | Rechercher la fiabilité maximale peut conduire à un modèle par clic ou à un juge imposé partout. | Exposer le choix rejeu strict / supervision bornée entre blocs et sur exceptions ; conserver les vérifications mécaniques et les preuves. | Checkpoints seuls responsables des appels déclarés, coûts mesurés et aucune invocation cachée en mode interdit. |

Les objections 01–05 portent sur le contrat fondamental ; un simple changement de texte marketing ne les résout pas. Les réponses sont des propositions documentées et des obligations de qualification, pas des tests déjà réussis.

## Comparaison des approches réalistes

| Approche | Intérêt | Coût ou limite | Position proposée |
| --- | --- | --- | --- |
| Agent interprétant chaque action à chaque run | Adaptation souple aux demandes nouvelles et cas peu répétitifs. | Appels modèles récurrents, résultats variables, difficulté à promettre zéro LLM ; la même compréhension peut être répétée. | Conserver pour préparation, investigation et cas où une décision nouvelle apporte de la valeur. Ne pas la déguiser en automatisation sans modèle. |
| Automatisation déterministe existante ou construite manuellement | Contrôle précis, pas de besoin intrinsèque d’un modèle pour rejouer les étapes, maintenance explicite. | Temps de préparation et maintien à la charge d’un auteur compétent ; faible aide face à un besoin encore flou. | Réutiliser dès qu’elle couvre le besoin. Unleash doit pouvoir l’héberger dans son mandat et ses preuves sans imposer un apprentissage artificiel. |
| Préparation assistée puis exécution qualifiée | Réutilise une compréhension et rend visibles les entrées, effets, versions et dérives. | Coût initial, besoin d’un contrat et d’une qualification ; aucune réparation universelle ni maintien gratuit. | Recommandée pour les usages répétés ciblés par R3, avec comparaison à la voie manuelle ou native. |

Les solutions ne s’excluent pas dans le produit : une même mission peut employer une procédure déterministe existante, puis ouvrir une activité d’analyse lorsqu’un cas nouveau survient. En revanche, un run dont la politique interdit les modèles ne peut pas basculer silencieusement vers une autre approche.

## Challenge des six exemples

| Cas | Première alternative à vérifier | Valeur additionnelle possible d’Unleash | Limite qui invalide une promesse trop large |
| --- | --- | --- | --- |
| E2E | Suite de tests et runner déjà présents. | Sélection, paramètres, orchestration, preuves et aide à l’adaptation. | Le moteur d’orchestration ne démontre pas la correction de l’application testée. |
| Courses | Liste favorite, renouvellement natif, import ou API réellement disponibles sur la cible. | Variation du CSV, contrôle des correspondances et résultats du panier. | Ambiguïtés produit, unités, disponibilité et action de paiement distincte. Aucun support Carrefour établi. |
| Rapprochement CSV | Jointure et règles dans le tableur, SQL ou logiciel métier existant. | Sources explicites, procédure réutilisable, traitement des exceptions et rapport traçable. | Déterminer une correspondance selon une règle n’est pas statuer sur son traitement comptable. |
| Classement documentaire | Règles locales ou conventions de nommage suffisantes. | Prévisualisation, collisions, lots et provenance. | Documents sans texte exploitable, OCR ou classement nécessitant un jugement nouveau. |
| Export et reporting | Export planifié natif ou tâche locale connue. | Coordination de sources, contrôle de période, génération répétée et historique des échecs. | Une source manquante ou une métrique nouvelle n’autorise pas à fabriquer le résultat. |
| Formulaire répété | Import natif ou API supportée, si disponibles. | Mapping contrôlé, aperçu, statut par ligne et reprise d’incertitude. | MFA, nouvelles contraintes et absence de preuve d’envoi empêchent une répétition aveugle. |

L’existence effective d’une fonction native, d’un import ou d’une API doit être vérifiée pour le service, le compte et la version concernés. Cette comparaison ne prétend pas que tous les services possèdent ces fonctions. Leur présence peut rendre une automatisation navigateur inutile ; leur absence ne rend pas automatiquement tout parcours navigateur fiable.

## Compromis de produit à conserver visibles

La polyvalence doit concerner le modèle de travail et l’expérience. Elle ne requiert pas que chaque domaine soit livré avec une bibliothèque complète de compétences métier. Le développement garde ses dix-huit capacités de base ; les procédures personnelles ou administratives utilisent les mêmes autorisations, sources, journaux et mécanismes de reprise.

L’accessibilité ne vient pas uniquement de la conversation. Une demande en langage naturel peut rester ambiguë. La proposition est d’en faire apparaître les conséquences dans un formulaire, un aperçu et un résultat inspectables. Le passage au mode expert doit expliquer davantage, sans modifier la décision ou faire apparaître des droits nouveaux.

La réutilisation sans LLM n’est pas synonyme d’absence de maintenance. Les indicateurs utiles seront le temps de préparation, le nombre de réutilisations valides, les arrêts explicables, le temps de réparation, les effets incertains et le coût total observé. Ils doivent être comparés à l’alternative réellement disponible pour l’utilisateur, sans chiffre de gain présumé.

## Gates avant une annonce de capacité livrée

| Gate | Critère | État de cette mission |
| --- | --- | --- |
| Domaine non Git | Sources et missions fonctionnent sans objets repository artificiels ; F010 reste complète. | Conception proposée ; implémentation et recette futures. |
| Contrat d’action | Feuille d’action, artefact, données, politique modèle et erreurs raccordés au même moteur. | Raccordement documentaire à vérifier lors de la consolidation R3. |
| Sans LLM | Politique transitive, outils qualifiés, routeur et chemins auxiliaires contrôlés ; aucune réparation cachée. | Contrat décrit ; aucune preuve runtime produite. |
| Pilote navigateur | Capacités, versions, plateformes, limites et contextes réellement qualifiés. | Aucun pilote produit exécuté ici. |
| Sécurité et reprise | Mandat actuel, secrets, prévisualisation, erreurs et effets incertains respectés. | Scénarios définis ; aucun essai de sécurité ou de crash produit réalisé. |
| Parcours guidé et TUI | Création, édition, réutilisation, adaptation, résultats et configuration accessibles avec parité. | Parcours proposés ; aucun test d’utilisabilité ni d’accessibilité réalisé. |
| Cas métier | Six scénarios bornés, alternatives évaluées et intégrations effectivement vérifiées. | Cas de conception ; aucune certification métier ni intégration de marchand acquise. |

Le contrôle réalisé par l’auteur porte sur la cohérence des quatre documents, leur vocabulaire et leurs références locales. Il ne recertifie pas les features du dossier initial. Une revue croisée après consolidation doit vérifier notamment le DSL, la clôture des missions, les schémas, le catalogue d’opérations et la matrice GUI/TUI avant de déclarer la révision documentaire entièrement harmonisée.
