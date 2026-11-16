# Qualification d'intégration — parcours A à O

**Statut : plan de tests V1, non exécuté.** Ce dossier est une conception ; aucun binaire produit ni test d'intégration réel n'est fourni. Les programmes de test internes seront Rust. Les outils externes des dépôts de fixture peuvent correspondre à Python, TypeScript, Java, Go ou Rust. Les contrôles de qualification d'OVPT Unleash ne doivent pas être confondus avec des phases imposées aux workflows des utilisateurs.

## Protocole commun

Chaque exécution crée des dépôts temporaires à contenu déterminé, un stockage local de fixture, des identités d'agents, des ACL et des providers simulés pour les scénarios de moteur. Elle enregistre versions, seed, configuration, OID, manifestes et séquences d'événement. Les scénarios de compatibilité provider ou connecteur réel sont séparés des simulations et identifiés comme tels.

La procédure est : préparer la fixture et son oracle indépendant ; soumettre les commandes par l'API commune ; appliquer les barrières de concurrence ou fautes prévues ; attendre une condition observable bornée ; lire Git et état métier indépendamment ; comparer ; exporter les preuves expurgées. Un timeout produit un échec ou résultat indéterminé explicite, jamais une réussite par absence d'erreur visible.

Les preuves E2E citent leurs sous-preuves feature. `EV-E2E-X` est un paquet attendu avec manifest.json, événements, états, snapshots et rapport ; il ne sera marqué réalisé qu'après exécution. Les critères locaux portent les IDs `AC-E2E-X` et tests `T-E2E-X`.

## Matrice globale

| Parcours / test | Acceptation | Sous-preuves complémentaires (en plus des EV des exigences principales d'acceptance-scenarios) | Preuve globale attendue |
|---|---|---|---|
| A / T-E2E-A | AC-E2E-A : plusieurs features avancent sans écrasement | T-F010-002/006, T-F009-007 | EV-E2E-A |
| B / T-E2E-B | AC-E2E-B : workflow NL validé, normalisé et versionné | T-F006-001/003/007/009, T-F005-001/002 | EV-E2E-B |
| C / T-E2E-C | AC-E2E-C : trois N utilisent un même skill sous scopes distincts | T-F010-003, T-F016-001 | EV-E2E-C |
| D / T-E2E-D | AC-E2E-D : helper nonmergé découvert et coopération décidée | T-F009-007/008/009/010 | EV-E2E-D |
| E / T-E2E-E | AC-E2E-E : skill créé/délégué et publié par N+2 avec contrat | T-F009-008, T-F016-004 | EV-E2E-E |
| F / T-E2E-F | AC-E2E-F : claims de tests reliées aux observations | T-F016-002/003/006 | EV-E2E-F |
| G / T-E2E-G | AC-E2E-G : conflit confié à un intégrateur spécialisé | T-F010-007/009 | EV-E2E-G |
| H / T-E2E-H | AC-E2E-H : worker crash, session durable restaurée | T-F003-001/006, T-F018-001/004/005, T-F016-001/011 | EV-E2E-H |
| I / T-E2E-I | AC-E2E-I : vingt leaders logiques, plafond d'invocations respecté | T-F003-003/004/005/009, T-F004-004 | EV-E2E-I |
| J / T-E2E-J | AC-E2E-J : message humain à N+1 visible par N+2 atomiquement | T-F016-005/010 | EV-E2E-J |
| K / T-E2E-K | AC-E2E-K : publication v2 sans altération du run v1 | T-F005-001/008/010, T-F007-007/015 | EV-E2E-K |
| L / T-E2E-L | AC-E2E-L : done=merge sans audit forcé | T-F010-008, T-F016-009 | EV-E2E-L |
| M / T-E2E-M | AC-E2E-M : documents Drive/OneDrive/Notion inspectables et scopés | T-F013-001/003/004/005/009, T-F012-009 | EV-E2E-M |
| N / T-E2E-N | AC-E2E-N : crash daemon, aucune répétition aveugle | T-F002-001/010, T-F018-002/003/004/007, T-F010-009, T-F009-005, T-F016-011 | EV-E2E-N |
| O / T-E2E-O | AC-E2E-O : GUI/CLI/TUI convergent vers même état | T-F016-010/012 | EV-E2E-O |

Pour chaque scénario A–O, les EV des exigences principales et les sous-preuves listées sont des dépendances obligatoires. Pour H et N, toutes les sous-preuves nommées dans cette matrice sont des dépendances obligatoires du paquet E2E, avec leurs EV correspondantes. Une référence absente, une preuve périmée ou un résultat autre que `passed` empêche la clôture du scénario ; un `not_applicable` ne supprime pas une dépendance obligatoire. La campagne E2E exige en plus sa propre commande, sa fixture, son environnement et son oracle indépendant : additionner des tests unitaires réussis ne remplace pas son exécution. `outcome_unknown` peut être l'état produit attendu d'un test réussi ; ce n'est pas un statut de recette permettant d'omettre la preuve. Les résultats produit restent `not_run` dans ce dossier.

## A — Features concurrentes et intégration

Étant donné trois missions issues du même commit, lancer des modifications indépendantes puis deux intégrations préparées sur la même cible. Chaque mission doit conserver branche/worktree et propriétaire. Une publication réussit sur oldOID ; l'autre observe le déplacement et prépare un nouvel essai. La preuve indépendante lit les fichiers et parents Git, montre absence d'écrasement et confirme qu'aucun verrou global de fichiers n'a sérialisé toute la réalisation. Ajouter une modification extérieure pour vérifier que la lease seule ne suffit pas.

## B — Workflow naturel vers définition effective

Soumettre une demande naturelle de workflow, obtenir un draft structuré et vérifier que seule une définition normalisée/versionnée peut démarrer. Les ambiguïtés ont un état explicite ; le texte brut ne pilote pas directement les transitions. L'export doit montrer la définition validée, son auteur, le run épinglé et les étapes effectives. Le scénario ne force pas de tests ou audit absents du draft accepté. Le moteur de workflow fournit ses propres oracles de normalisation.

## C — Trois N, même skill, scopes distincts

Créer trois workers du même skill et même version avec scopes différents. Les invocations partagent la référence de skill mais pas l'identité, les permissions, les contextes ou les propriétaires de mutation. Une tentative hors scope et un patch fondé sur un ancien hash doivent être rejetés. La preuve relie chaque sortie à son invocation, rôle, session et scope ; une réutilisation de skill n'est pas une session partagée implicite.

## D — Recherche avant helper nonmergé

A prépare `normalizeEmail` sur une branche non mergée. B veut créer `cleanEmail` et exécute discovery avant création. La recherche trouve A même sans nom exact, cite son snapshot, indique non disponibilité locale et fournit le propriétaire. Faire deux variantes : contrats compatibles entraînant réutilisation/coordination ; contrats différents entraînant copie/création justifiée. Ajouter A attend B et B attend A pour vérifier la détection de cycle. Modifier/rebaser A après la recherche : le reçu B doit devenir périmé avant consommation.

## E — Skill et rôle sans duplication aveugle

N+2 détecte un besoin, recherche les skills existants puis délègue éventuellement la rédaction technique à un N spécialisé. La publication conserve contrat, diff, versions et décision de N+2. N+1 peut composer un rôle versionné qui référence ce skill. Les reçus discovery du skill et du rôle sont distincts et respectent scopes global/projet/repository. Le scénario doit montrer une nouvelle création justifiée lorsque le domaine diffère, sans réutilisation forcée.

## F — Déclaration, observation et jugement

Faire exécuter un test sur snapshot S1, produire une claim de réussite, puis livrer un candidat S2 modifié. La preuve conserve commande, sortie, terminaison et S1. Le jugement sur S2 doit soit exiger la validation prévue, soit expliquer une réutilisation de preuve autorisée ; il ne s'appuie pas sur la seule claim. Une autre variante annonce réussite alors que le test échoue : la contradiction reste visible. Une étape non demandée est `not_requested`, jamais `passed`.

## G — Conflit Git et agent spécialisé

Deux missions modifient une même règle. Le préparateur conserve base/côtés et crée l'état conflicted. N+2 assigne un intégrateur avec scope, politique et priorités ; l'intégrateur produit un candidat attribué. N+1 décide selon contrat, puis publication CAS. La preuve vérifie que le manager n'a pas été forcé de coder le conflit et qu'aucun choix global `ours/theirs` silencieux n'a remplacé le travail spécialisé. Ajouter un conflit sémantique sans marqueur textuel pour vérifier les limites du rapport.

## H — Crash de worker

Arrêter un worker après une décision durable, puis avant et après un effet d'outil. La session logique, le rôle, les versions, le contexte effectif et les actions restent inspectables. La reprise conserve invocation_id et crée une nouvelle attempt_id avec fence valide ; elle ne dépend pas de la RAM de l'ancien fork. Les effets incertains sont réconciliés avant répétition. Les identités de processus ne reposent pas uniquement sur PID.

## I — Budget et managers urgents

Créer vingt leaders logiques dormants, puis occuper les cinq slots par défaut avec N actifs et injecter un événement urgent pour N+1 ou N+2. Le scheduler utilise sa politique déclarée de priorité, réserve, queue ou préemption coopérative ; il ne dépasse jamais la limite configurée ni le plafond dur de cinquante. Les appels managers et retries actifs sont comptés. Les serveurs de langage ont leur propre budget ; ils ne deviennent pas illimités sous prétexte de ne pas être LLM.

## J — Message humain et visibilité organisationnelle

Envoyer un message humain à N+1 via un client, interrompre la diffusion puis reconnecter N+2. Le message et l'événement visible au niveau organisationnel doivent provenir de la même transition durable. Le rejeu peut redélivrer mais ne duplique pas la décision. La visibilité ne requiert pas un réveil LLM de N+2 pour chaque message ; l'événement et son accusé applicatif suffisent à démontrer la propriété demandée.

## K — Versions en présence de branches parallèles

Lancer un run v1 avec deux branches d'exécution, publier v2 pendant que l'une termine et l'autre continue. Le run engagé garde sa définition v1 et tous ses bindings engagés. Un slot déclaré vierge peut recevoir une version exacte nouvelle explicitement compatible avant sa première activation ; ce n'est pas une migration de définition. Une nouvelle invocation dans la branche restante ne suffit pas à justifier un hot swap. L'adoption v2 n'a lieu que par un nouveau Run à référence exacte, déclarant `predecessor_run_id`, après l'état terminal de v1 (Q-03, [ADR-04](../architecture/adr/ADR-04-workflow-semantics.md)) ; aucune migration en place n'existe. L'export doit montrer cette frontière et les versions effectives de chaque invocation.

## L — Définition de done limitée au merge

Définir un workflow `Dev → Merge` sans tests ni audit. Exécuter le travail avec discovery et préconditions de sécurité, puis intégrer sur la cible exacte. La mission termine lorsque l'effet de merge observé satisfait son contrat ; aucune phase LLM d'audit ou suite de tests n'est ajoutée. Le rapport indique toutefois « validation comportementale non demandée ». Une cible checkout est signalée `blocked_target_checkout` et bloque proprement la publication, sans mise à jour de la ref (Q-10) ; préparer un candidat ailleurs ne satisfait pas faussement la définition de done.

## M — Sources documentaires explicites

Préparer une source dans chacun des connecteurs Drive, OneDrive et Notion avec sélection explicite, version et ACL. Explorer, prévisualiser, pin, scoper puis watch selon leurs capacités réelles. Le manifeste d'invocation montre les extraits effectivement envoyés et leur provenance. Modifier ou révoquer un document ; un nouveau contexte ne doit pas employer son ancien contenu comme source actuelle autorisée. Les oracles sont ceux de F013 : ressource stable, révision, ACL, politiques de pin, pagination, Retry-After et contexte excluant un accès révoqué. Les fixtures sont rejouées par protocole puis qualifiées séparément contre les trois services réels.

## N — Crash daemon aux frontières d'effet

Injecter arrêt avant intention durable, après intention, après dispatch, après CAS et avant résultat. Au redémarrage, lire les références réelles, reprendre les identités de processus et conserver les états inconnus. Si newOID est présent sans résultat, montrer `desired_state_present` avec limite causale ; si la cible est revenue à oldOID, ne pas conclure que l'effet n'a jamais eu lieu. Knowledge et receipts passent unknown/stale jusqu'à réconciliation. Aucun retry aveugle ne doit transformer la reprise en second effet.

## O — Clients cohérents et exports

Ouvrir GUI, CLI et TUI sur la même mission, déconnecter la GUI, changer un état depuis CLI puis faire dépasser la fenêtre de reprise du TUI. Le backend rend un état identique au même watermark, les clients dédupliquent et la fenêtre perdue provoque resync explicite. Exporter le rapport sous ACL courante et vérifier que les mêmes preuves et inconnues apparaissent. Une fermeture de GUI ou un échec de résumé provider ne doit pas arrêter le daemon ni empêcher le rapport déterministe.

## Critères de clôture de qualification

La qualification échoue si une preuve cite une source indéterminée, si une étape non exécutée est passée, si une création identifiée reçoit un reçu antidaté, si un scan dirty devient atomique sans mécanisme qualifié, si une ref checkout est déplacée silencieusement, si une ACL révoquée fuit dans un nouveau contexte ou si un effet incertain est rejoué sans décision. Les limitations d'environnement — par exemple absence de test de panne de courant — restent explicitement ouvertes. Les oracles de features référencés dans la matrice sont obligatoires ; leur recette réelle reste à exécuter sur le produit, sans changer ces invariants transversaux.

## Extension normative O — parité complète R2

Pour chaque groupe de la [matrice](../design/gui-tui-parity.md), charger deux états initiaux équivalents avec les mêmes droits, puis exécuter le parcours GUI et TUI ; utiliser la CLI pour lecture et automatisation équivalentes. Normaliser uniquement identifiants générés, horodatages non contractuels et préférences de vue. Comparer cible, mutation, révision, classes d’erreur, état final, reçus, preuves et causalité. Aucun filtre ne peut effacer une différence de permission ou de résultat.

Le corpus couvre particulièrement création/édition/publication de workflows, skills et rôles, configuration modèle/quotas, authentification documentaire, Explore/Pin/Scope/Watch, inspection du contexte transmis, processus, rapports et AI-Guard. Refaire sans permission, avec conflit de révision, perte de connexion après commit et restauration de profil. La TUI doit réaliser les parcours sans GUI ; un navigateur OAuth demeure une interaction fournisseur commune.

L’aide et le catalogue backend sont confrontés à la matrice : zéro opération utilisateur sans parcours GUI et TUI, zéro mutation locale directe, zéro réglage avancé caché dans une seule interface. Les essais visuels et de restauration du terminal relèvent également de T-E2E-U et des tests F015. Statut : tests à exécuter lors de l’implémentation.

## Extension R3–R5 — W à Z

Les séquences, conditions initiales, injections et oracles détaillés de W/X/Y/Z sont définis dans [acceptance-scenarios](acceptance-scenarios.md). W réutilise F005/F008/F011/F018 avec F019 et un site contrôlé. X remplace uniquement les tâches de décision par un double puis un modèle qualifié ; les branches et effets passent par le moteur réel. Y exige les trois profils locaux et leurs tests de placement/capacités. Z traverse les mêmes opérations depuis GUI, TUI et CLI, sans repository obligatoire et sans agent préexistant.

Ces campagnes doivent s’exécuter avec la persistance et le broker réels avant qualification V1. Un enchaînement de mocks ne suffit pas pour prouver reprise, absence de fuite ou contrôle d’effet. Les preuves EV-E2E-W/X/Y/Z agrègent les EV des features sans remplacer les résultats absents. Aucun de ces tests produit n’a été exécuté dans cette mission documentaire.
