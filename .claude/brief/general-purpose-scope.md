# Unleash polyvalent — périmètre produit R3

**Date : 16 novembre 2026. Statut : clarification utilisateur et conception acceptée en R7 ([ADR-16](../architecture/adr/ADR-16-reusable-automation-and-supervision.md)), sans implémentation produit.** La clarification R3 et ses compléments ouvrent Unleash aux usages professionnels et personnels. Unleash reste spécialisé pour le développement, avec une architecture et des parcours suffisamment polyvalents pour ces autres usages ; il ne se repositionne pas comme automatisation universelle de tout logiciel. Le présent document complète la [vision](vision.md), le [périmètre V1](product-scope-v1.md) et la [politique d’implémentation](implementation-policy.md) ; les mécanismes acceptés sont distingués des demandes confirmées.

## Ce qui est confirmé, accepté et à qualifier

| Statut | Contenu |
| --- | --- |
| Confirmé par Louis en R3 | Unleash est polyvalent : développeur, comptable et particulier doivent pouvoir y réaliser un travail utile. Un LLM peut comprendre un parcours navigateur et préparer une procédure réutilisable avec des données variables, par exemple un CSV. Les exécutions suivantes évitent le LLM tant que la procédure reste valide ; un changement du site peut nécessiter une réanalyse. |
| Confirmé et maintenu | Cœur métier et système Rust, bibliothèques existantes réutilisables, présentation HTML/CSS autorisée ; GUI accessible et soignée, TUI complète avec les mêmes fonctions et une navigation adaptée ; conservation des capacités de développement ; mission documentaire jusqu’à R6, réalisation ouverte au jalon M0 par la [révision R7](revision-2026-11-16-r7.md). |
| Architecture acceptée (R7) | Un domaine commun goal → mission → run, des sources sans dépôt obligatoire, un artefact d’automatisation versionné, des paramètres distincts de la procédure et des modes guidé/expert indépendants des permissions, acceptés par [ADR-16](../architecture/adr/ADR-16-reusable-automation-and-supervision.md). Accepté ne signifie pas confirmé : ces choix font autorité pour la réalisation, restent révisables par un ADR de supersession et ne deviennent pas des invariants. F019 regroupe l’automatisation réutilisable ; F001–F018 restent intégralement dans la V1. Ce découpage en features organise la réalisation selon le [plan](../plans/v1-implementation-plan.md). |
| À qualifier | Pilote navigateur décidé R7 (Q-18), qualification EXT-BROWSER ; matrice de plateformes et distribution décidées R7 (Q-11, Q-16, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md)), qualification EXT-OS et EXT-DIST ; modes réellement supportés, contrats des outils, formats documentaires, seuils de validité et coût de maintenance des procédures. Un port `DesktopDriver` permet d’envisager des capacités supplémentaires ; il ne prouve aucun support général des applications de bureau. |
| Non établi | Une automatisation opérationnelle de Carrefour ou d’un autre marchand, une intégration comptable réglementaire, des connecteurs d’écriture universels, une procédure déjà capable de fonctionner sur tous les sites. Aucun de ces résultats n’a été réalisé dans cette mission. |

La consigne « ne pas refaire Dots » borne la démarche : on précise la valeur et les frontières d’Unleash à partir de cette clarification. Ce document ne prétend pas décrire l’architecture ou les fonctions de Dots, qui n’ont pas été fournies ici.

## Valeur centrale

Unleash doit permettre de transformer un travail compris une fois en travail réexécutable, observable et adaptable. L’utilisateur exprime son résultat, choisit les données et autorise les effets nécessaires. Il peut partir d’une procédure existante, en préparer une avec un LLM, ou la décrire explicitement. Il retrouve ensuite une procédure, ses paramètres, ses résultats et les raisons de ses éventuels blocages.

Un besoin ponctuel ne justifie pas automatiquement de créer une automatisation durable. La comparaison avec une action manuelle, un export déjà disponible ou un script existant fait partie du cadrage. L’apprentissage a une valeur lorsqu’il évite de répéter une compréhension et une maintenance raisonnablement mutualisables. Aucun nombre d’agents ou de procédures créées ne mesure à lui seul cette valeur.

Le produit conserve sa capacité d’orchestration N+2/N+1/N. Ces responsabilités ne doivent pas imposer une invocation modèle à chaque ligne CSV ni à chaque réussite mécanique. Les responsables peuvent rester dormants pendant un run déterministe ; les événements, contrats et preuves sont conservés par le moteur commun. La [spécification des automatisations](../specs/reusable-automations.md) précise cette séparation.

La clarification complémentaire autorise aussi une supervision par LLM entre des blocs de script : lecture des résultats et logs pertinents, puis décision bornée aux checkpoints ou exceptions. Deux modes sont donc explicites : rejeu sans aucun LLM et exécution avec supervision autorisée et bornée. Un clic ne demande pas nécessairement un appel modèle ; la supervision n’est ni un juge universel obligatoire ni une garantie absolue de fiabilité. Préparation, adaptation et supervision peuvent utiliser un modèle local ou un fournisseur API autorisé.

## Domaine commun, spécialisations explicites

| Objet | Sens commun (ADR-16) | Spécialisation développement |
| --- | --- | --- |
| Projet / espace de travail | Périmètre durable de données, objectifs, procédures, autorisations et historique. Il peut ne contenir aucun dépôt. | Un ou plusieurs repositories, outils de build et contextes de code. |
| Goal | Résultat attendu avec critères de fin et exclusions. | Livrer une fonctionnalité, corriger une régression, préparer une release. |
| Mission | Travail borné, owner, scope, budget, livrables et procédure sélectionnée. | Feature de bout en bout, lot d’intégration ou investigation. |
| Run | Exécution particulière, données et versions engagées, effets et état observé. | Tests, compilation, modifications et intégration selon le workflow choisi. |
| Skill et rôle | Compétence réutilisable et responsabilités selon les [registres existants](../specs/skills-and-roles.md). | Analyseur, développeur, testeur, intégrateur ou autre composition choisie. |
| Source | Fichier, document sélectionné, export, cible navigateur ou autre ressource autorisée, avec provenance et fraîcheur. | Snapshot de code, branche, worktree, logs ou configuration de build. |
| Procédure | Présentation utilisateur d’un `AutomationArtifact` versionné et du workflow auquel il se rattache. | Suite E2E, génération, maintenance de dépôt ou procédure de release. |
| Résultat | Artefacts, observations, anomalies, décisions et effets réellement constatés. | Commit, diff, résultat de test, intégration ou rapport. |

Le repository devient donc une ressource possible du projet ; il n’est pas la condition d’existence d’un goal. Git continue de faire autorité sur les objets de code pour lesquels il est utilisé. Les fichiers et données métier conservent leurs propres références de source et de révision. Il n’est pas nécessaire de créer un faux dépôt, une branche ou un commit pour importer une liste de courses.

Les portées global/projet/repository du registre restent utilisables : la résolution ignore simplement la portée repository lorsqu’aucun dépôt n’est applicable. Un profil métier ne crée ni droits implicites ni accès à toutes les ressources de l’utilisateur. La donnée fournie par un document ou une page ne devient pas une instruction d’autorisation.

## Six cas transversaux et leur alternative raisonnable

Les scénarios suivants sont des cas de conception et de recette future. Ils ne constituent pas une liste de connecteurs livrés. Chacun possède une solution plus simple possible ; Unleash doit pouvoir la réutiliser ou reconnaître que l’automatisation n’apporte pas assez de valeur.

### GEN-01 — Rejouer un parcours E2E de développement

**Intention :** vérifier un parcours de création de compte sur une application de test. Les entrées sont l’URL autorisée, des comptes de fixture et un jeu de données. Le LLM peut aider à comprendre la page et à proposer les étapes ; la procédure publiée décrit ensuite les actions et assertions observables.

L’aperçu indique l’environnement, les comptes utilisés et les données créées. Le résultat distingue parcours réussi, assertion échouée, précondition absente et effet incertain. Une exécution sans LLM est pertinente lorsque les locators, données, assertions et conditions de nettoyage sont explicitement définis. Un changement d’interface ne doit pas être « réparé » en cliquant sur un autre élément au seul motif que son texte ressemble.

**Alternative à évaluer :** une suite E2E existante et son runner peuvent déjà couvrir le besoin. Unleash devrait alors en piloter l’exécution et conserver les preuves, sans recréer un moteur de test. **Limite :** réussir un scénario ne démontre pas la qualité entière de l’application.

### GEN-02 — Préparer un panier à partir d’une liste de courses

**Intention :** utiliser un CSV contenant références ou libellés, quantités, unités et règles de remplacement explicitement choisies. Une première préparation peut examiner le parcours navigateur. La réutilisation change les données sans réapprendre les gestes pour chaque produit.

L’aperçu présente les lignes résolues, les ambiguïtés, les quantités et les effets prévus sur le panier. Les prix et disponibilités observés gardent leur date ; ils ne sont pas promis constants. La préparation du panier et la validation d’un achat sont des effets distincts, chacun soumis au mandat applicable. Aucun paiement n’est déduit de la simple demande de construire une procédure.

**Alternative à évaluer :** listes favorites, fonctions natives de commande répétée, import ou API officiellement disponible pour le compte concerné. Le navigateur n’est choisi qu’après cette vérification. **Limite :** produit introuvable, conditionnement différent ou substitution non prévue créent une décision, pas une correspondance inventée. Carrefour peut servir de cible de qualification ultérieure ; aucun support Carrefour n’est déclaré acquis.

### GEN-03 — Rapprocher deux exports CSV

**Intention :** comparer un export d’opérations et un export de pièces selon des clés, montants, devises, dates et tolérances définies par l’utilisateur. Les données d’entrée et règles de correspondance sont versionnées séparément.

L’aperçu montre le mapping des colonnes et quelques lignes représentatives ; le résultat distingue correspondances exactes, candidats multiples, écarts et lignes non appariées. Les doublons et montants ambigus ne sont pas résolus silencieusement. Le calcul peut rester sans LLM si les règles sont entièrement déterminées et les conversions contrôlées.

**Alternative à évaluer :** requête dans un tableur, jointure SQL ou fonction du logiciel métier déjà utilisé. **Limite :** le rapport de rapprochement n’est ni une écriture comptable, ni une validation réglementaire, ni un jugement sur le traitement comptable adéquat. L’exemple ne promet aucune intégration bancaire ou comptable existante.

### GEN-04 — Classer des documents avec une règle réutilisable

**Intention :** renommer et ranger des documents selon date, fournisseur ou référence déjà présente dans des métadonnées ou du texte accessible. La source peut être un dossier explicitement choisi ; la destination et la politique de collision sont des paramètres contrôlés.

L’aperçu affiche ancienne et nouvelle destination, collisions, doublons et documents non reconnus. Le résultat conserve les correspondances et les déplacements observés. Une annulation utilise seulement les inverses dont les préconditions restent vérifiées ; elle n’efface pas une modification ultérieure de l’utilisateur.

**Alternative à évaluer :** règles de classement déjà disponibles, conventions de nommage ou traitement par lot local. **Limite :** OCR, mauvaise qualité de scan et compréhension d’un document nouveau nécessitent des capacités et validations supplémentaires. Un document illisible reste une exception. Aucun OCR ni classement sémantique n’est présumé disponible ou compatible avec le mode sans LLM.

### GEN-05 — Réutiliser un export et un rapport périodiques

**Intention :** récupérer chaque période un export autorisé, appliquer des transformations connues et produire un tableau de suivi. Les paramètres sont la période, les sources, les colonnes, les règles d’agrégation et la destination de l’artefact.

L’aperçu montre la période choisie, le périmètre, la destination et la version à relancer. Le résultat indique sources réellement récupérées, lignes traitées ou rejetées, calculs et fraîcheur. Le rapport sans LLM emploie un gabarit et des calculs explicites ; il ne génère pas une nouvelle explication causale des chiffres. Une période non traitée ou une source indisponible reste visible. Le lancement est explicite ; la récurrence calendaire est exclue de la V1 (Q-21) : un ordonnanceur externe choisi par l’utilisateur peut appeler la CLI non interactive, sans garantie calendaire d’Unleash.

**Alternative à évaluer :** export planifié du service, outil de reporting existant, tâche locale déterministe. **Limite :** changer de période peut exposer des données manquantes ou une nouvelle colonne ; ce n’est pas une permission d’élargir l’accès ni d’inventer les valeurs. La remise à un tiers ou la publication constitue un effet séparé de la création du fichier.

### GEN-06 — Préremplir un formulaire répété

**Intention :** reporter des lignes validées dans un formulaire interne, par exemple une demande de matériel. Chaque champ, option, format et clé de ligne sont définis. Le résultat peut être un brouillon ou un envoi selon le mandat.

L’aperçu montre la destination, le nombre de formulaires, les champs et les erreurs avant traitement. Le run conserve un résultat par ligne et les références de confirmation disponibles. Il ne renvoie pas toute la série après une coupure : les lignes au résultat inconnu attendent une réconciliation.

**Alternative à évaluer :** import natif du service, API déclarée ou fonctionnalité de traitement par lot existante. **Limite :** nouveau champ obligatoire, MFA, changement de session ou doute sur l’envoi déclenchent un arrêt explicable. Une interaction humaine peut être nécessaire sans que le run appelle un LLM ; « sans LLM » ne signifie pas « sans intervention ».

## Profils et interfaces

Deux axes restent indépendants. Le premier décrit le travail : développement, administratif, personnel ou autre domaine. Le second décrit la présentation : guidée ou experte. Aucun de ces axes ne détermine si l’utilisateur peut choisir la TUI, accéder aux fonctions avancées ou disposer d’un droit.

Le mode guidé met d’abord en avant objectif, sources, paramètres, aperçu et résultats. Le mode expert donne plus de place aux versions, contrats, bindings, locators, journaux et politiques. Les mêmes objets restent inspectables depuis les deux modes. Les dix espaces de navigation R2 sont conservés ; les raccourcis guidés constituent des entrées vers ces espaces, pas une application parallèle.

Le premier parcours ne suppose aucun agent déjà créé. « Décrire le résultat » ou « Utiliser une procédure recommandée » suffit pour commencer ; le produit prépare les objets et, lorsque nécessaire, crée ou sélectionne rôles et agents dans le mandat. Il présente un modèle de procédure avec des étapes lisibles avant exécution, sans demander d’abord une configuration technique de workflow.

La [conception du parcours guidé](../design/guided-automation-experience.md) détaille les adaptations GUI/TUI. Les fonctions Git peuvent être absentes du résumé d’un projet sans dépôt ; elles restent accessibles et complètes lorsqu’un dépôt est enregistré. Cela ne réduit pas F010 et n’impose pas un parcours Git aux autres usages.

## Frontières du périmètre et critères de réussite

F001–F018 continuent de fournir daemon, persistance, agents, workflows, registres, contrats, connaissance, Git, processus, modèles, documents, interfaces, rapports, sécurité et reprise. F019 définit le contrat spécifique d’une automatisation paramétrée, sa préparation, son exécution sans modèle et son invalidation. Il réutilise ces fondations ; il ne crée pas un second moteur d’orchestration.

Le navigateur structuré constitue le parcours concret à qualifier en V1. Le port d’extension `DesktopDriver` doit décrire ses capacités, préconditions et plateformes effectivement testées avant tout usage. Le nom d’un port n’accorde aucun pouvoir sur le bureau, et un pilote navigateur n’établit pas la capacité de manipuler n’importe quelle application native.

La réussite du cadrage se vérifie par des parcours complets : créer une mission sans dépôt ; réutiliser une procédure avec un nouveau CSV sans modification de sa version ; inspecter la prévisualisation et les résultats dans les deux interfaces ; constater zéro appel modèle pour un run interdit aux modèles ; expliquer une dérive sans répéter un effet incertain ; conserver les parcours de développement existants. Les mesures de temps gagné, coût et taux de reprise devront être relevées sur des usages réels. Aucun résultat chiffré n’est inventé ici.

Les limites et choix alternatifs font l’objet de la [revue produit R3](../audits/general-purpose-product-review.md). Les vérifications décrites sont des critères futurs, pas des tests du produit déjà passés.
