# Scénarios d'acceptation et couverture de l'annexe

Tous les scénarios sont des **tests futurs**. Les séquences A–O et les oracles transversaux sont détaillés dans [integration-tests](integration-tests.md). Chaque EV-E2E-X désigne un paquet attendu contenant état, curseur, artefacts, versions et assertions ; aucun paquet de recette produit n'est présenté comme déjà exécuté.

## A–O — Scénarios obligatoires du brief

| Scénario | Exigences principales | Critère / test | Oracles discriminants |
| --- | --- | --- | --- |
| A Multi-features | REQ-ORCH-001/003, REQ-GIT-002/006 | AC-E2E-A / T-E2E-A | Owners uniques, worktrees distincts, résultats conservés, consolidation N+2 sans micro-tâches imposées |
| B Workflow dynamique | REQ-COMP-001/003/007/009, REQ-WF-001/002 | AC-E2E-B / T-E2E-B | Draft non exécutable avant validation, digest stable, publication et run épinglé |
| C Parallélisme d'étape | REQ-ROLE-007, REQ-WF-005 | AC-E2E-C / T-E2E-C | Trois identités/scopes, même skill, join complet puis décision sur revision exacte |
| D Réutilisation | REQ-KNOW-007/008/009/010 | AC-E2E-D / T-E2E-D | Helper non mergé trouvé, localement indisponible, reçu périmé après mutation, choix justifié |
| E Création skill | REQ-SKILL-001/003/004/005 | AC-E2E-E / T-E2E-E | Conception déléguée possible, contrat fermé, publication autorisée, version consultable |
| F Trace tests | REQ-CONTRACT-004/005/007, REQ-OBS-002/003 | AC-E2E-F / T-E2E-F | Claim distincte du code de sortie, preuve liée au snapshot, résultat faux/périmé détectable |
| G Conflit Git | REQ-ORCH-010, REQ-GIT-007/009 | AC-E2E-G / T-E2E-G | Agent intégrateur sous mandat, candidat exact, conflit métier escaladé, CAS sans écrasement |
| H Crash worker | REQ-REC-001/004/005, REQ-AGENT-006 | AC-E2E-H / T-E2E-H | Même invocation, tentative/fence nouveaux, ancienne réponse refusée, unknown réconcilié |
| I Concurrence | REQ-AGENT-003/004/005/009 | AC-E2E-I / T-E2E-I | 20 N+1 logiques ; occupancy≤effective_limit≤50, défaut5 ; urgence sans slot caché |
| J Humain vers N+1 | REQ-ORCH-005/006, REQ-OBS-005 | AC-E2E-J / T-E2E-J | Message et visibilité parent atomiques ; N+2 peut rester dormant ; décision liée |
| K Version workflow | REQ-WF-001/008/010, REQ-SKILL-007 | AC-E2E-K / T-E2E-K | Pas de hot swap ; fin de toutes branches ; binding engagé stable ; slot vierge distinct |
| L Done personnalisé | REQ-WF-007, REQ-GIT-008 | AC-E2E-L / T-E2E-L | merge-only termine sans ajout d'audit/tests ; preuve de publication et autorisation requises |
| M Documents ciblés | REQ-DOC-001/003/004/005/009, REQ-MODEL-009 | AC-E2E-M / T-E2E-M | Trois connecteurs, affectation précise, révision/provenance, révocation et contexte inspectable |
| N Crash daemon | REQ-REC-002/003/004/007, REQ-PERSIST-001 | AC-E2E-N / T-E2E-N | Intent/dispatch/effet/résultat aux cutpoints, restore ancien et ancienne autorité invalidée |
| O GUI CLI TUI | REQ-CORE-003/010, REQ-UI-001/012, REQ-CLI-001/010/011/012/016 | AC-E2E-O / T-E2E-O | Même catalogue complet, résultats/permissions/preuves équivalents ; éditeurs et réglages inclus ; snapshot+watermark cohérents, resync après restore |

Chaque preuve globale EV-E2E-A à EV-E2E-O incorpore les EV des exigences principales de sa ligne et les sous-preuves complémentaires de la [matrice d'intégration](integration-tests.md) ; H et N y ajoutent les dépendances de reprise ci-dessous. Les sections d'intégration explicitent les conditions initiales et négatives. La performance d'un scénario ne peut être évaluée sans configuration de machine et charge publiées.

Les dépendances de reprise sont obligatoires : H incorpore EV-F003-001/006, EV-F018-001/004/005 et EV-F016-001/011 ; N incorpore EV-F002-001/010, EV-F018-002/003/004/007, EV-F010-009, EV-F009-005 et EV-F016-011. La [matrice d'intégration](integration-tests.md) porte les tests correspondants et la règle de clôture. Une sous-preuve manquante ou périmée reste bloquante, même si les preuves de reporting sont complètes. Le test E2E propre doit également être exécuté ; les liens ne produisent aucun `passed`.

## P — Rôle persistant et résolution

AC-E2E-P / T-E2E-P / EV-E2E-P ; REQ-ROLE-001 à008. N+1 compose Fullstack E2E Engineer depuis des versions exactes de skills, désigne entry_skill, publie dans son scope autorisé, puis un autre run le retrouve. Créer le même nom dans global/projet/repository ; vérifier priorité visible et refus d'une référence ambiguë ou interdite. Deux agents partagent rôle mais pas session/scope. Un rôle dont l'entrée ou le digest de contrat ne correspond pas au slot est refusé avant action.

## Q — Recherche préalable transversale

AC-E2E-Q / T-E2E-Q / EV-E2E-Q ; REQ-KNOW-008/009 et REQ-CONTRACT-008. Exécuter création de fonction, classe, API, test, skill, rôle et ressource. Pour chaque création identifiable, le broker exige un reçu admissible. Le shell opaque qui crée après coup reçoit observed_after_creation et ne fabrique pas une antériorité. Tester reuse/adapt sur identité existante et create/adapt sur nouvelle identité. Une justification de domaines distincts peut autoriser la création ; aucun verdict de doublon absolu n'est attendu.

## R — Starvation, quotas et baisse de capacité

AC-E2E-R / T-E2E-R / EV-E2E-R ; REQ-AGENT-003/004/005/009. Cinq N en attente d'avis de leurs leaders libèrent leurs slots après checkpoint ; le leader peut être admis. Refaire avec appels réseau non préemptibles : priorité n'est pas promesse d'arrêt distant instantané, mais deadline/contrôle déterministe restent actifs. Passer requested_limit de 50 à 5 ; admissions gelées, effective_limit demeure annoncé jusqu'au drainage. Contrôler chaque préfixe de trace, équité entre deux projets et budget fournisseur saturé.

## S — Secrets, injection et rétention

AC-E2E-S / T-E2E-S / EV-E2E-S ; REQ-SEC-001 à011, REQ-DOC-009. Document/repository/MCP tente de changer une politique et de lire un secret canari. Les instructions non fiables restent données ; l'autorité refuse l'action. Rejouer une ancienne capability après révocation et après restauration. Purger une preuve autorisée puis inspecter rapport, contexte et backup : limites visibles, aucune prétention d'effacement chez le fournisseur. Les détails sont dans [security-tests](security-tests.md).

## T — Projets multilangages

AC-E2E-T / T-E2E-T / EV-E2E-T ; INV-002, REQ-KNOW-003, REQ-TERM-001. Qualifier des fixtures Python, TypeScript, Java, Go, Rust et un monorepo mixte. Lancer leurs outils externes via le broker Rust ; rendre visibles runtime, versions, environnement et accès réseau. Vérifier recherche exacte/syntaxique/sémantique selon capacités réellement négociées ; l'absence d'analyseur est un mode dégradé explicite, pas un retrait de langue de la V1. Les analyseurs sont détectés là où l'utilisateur les a installés ; leur installation assistée n'a lieu que sur commande explicite (Q-13).

## U — Qualité du frontend et du terminal

AC-E2E-U / T-E2E-U / EV-E2E-U ; INV-054/056/057, REQ-UI-009/010/011/012 et REQ-CLI-009 à016. Sur builds réels GUI et TUI, utiliser Overview puis mission avec 50 agents, 10 worktrees et timeline volumineuse. Examiner thèmes, typographie, espacements, hiérarchie visuelle, focus, clavier, recherche et états de panne. Dans la TUI, ajouter redimensionnement, palette, Unicode/ASCII, navigation sans souris, éditeurs intégrés, restauration du terminal et reprise de session ; effectuer les parcours de la matrice de parité sans lancer la GUI. Captures et compte rendu de revue identifient défauts observables ; test de latence et accessibilité documente sa portée. Le gate visuel de [design system](../design/visual-design-system.md) doit être accepté, indépendamment du nom de framework.

## V — Intégration AI-Guard réelle

AC-E2E-V / T-E2E-V / EV-E2E-V ; REQ-SEC-012 et [ADR-10](../architecture/adr/ADR-10-ai-guard-boundary.md) (Q-04 résolue R7). Exécuter connexion, santé, politique, identité, autorisation, refus, incident, suspension et révocation contre la version réelle retenue. Débrancher le service pendant une autorisation et après dispatch ; vérifier fail-closed selon politique et issue inconnue des effets déjà engagés. Une implémentation mock permet le test du port Unleash, mais ne valide pas ce scénario externe.

## W — Procédure réutilisable, données nouvelles et dérive

AC-E2E-W / T-E2E-W / EV-E2E-W ; INV-059/060/062, REQ-AUTO-002 à015. Sur un site de fixture contrôlé et le pilote de référence qualifié de Q-18 (WebDriver `fantoccini`, Chrome for Testing), apprendre un parcours autorisé, paramétrer les identités/quantités, publier une version et importer deux jeux de données distincts. Rejouer chacun sans agent actif et sans modifier le bundle. Vérifier ID/version/digests identiques, DataRevision différentes, résultats par ligne, assertions métier et zéro dispatch modèle local/distant, compaction, embedding ou manager. L’oracle combine instrumentation du Router, descendants contrôlés et observation indépendante des destinations autorisées.

Injecter une ligne statiquement invalide : aucun effet métier ne démarre, même pour les autres lignes. Un choix explicite d’exclusion crée une nouvelle révision visible. Modifier ensuite le DOM pour que le locator vise deux cibles, puis pour qu’un clic valide ajoute le mauvais produit : le premier doit bloquer avant mutation et le second échouer sur sa postcondition. Perdre le receipt d’une mutation : aucun retry ni autre run ne répète l’effet sans réconciliation. Une réparation sous mandat produit une nouvelle version sans remplacer le run engagé. Les coûts de réparation restent visibles au niveau mission.

Les contrôles sur le site de fixture ne démontrent pas une compatibilité Carrefour. Celle-ci exige sa campagne distincte, avec compte et mandat appropriés. Le scénario n’impose ni achat ni interaction réelle avec un compte pendant les tests de base.

## X — Supervision par blocs sans pilotage de chaque clic

AC-E2E-X / T-E2E-X / EV-E2E-X ; INV-064, REQ-ORCH-012, REQ-AUTO-001/010/011/015. Le plan de fixture comprend douze actions en trois blocs, trois checkpoints et au plus trois appels LLM. Les actions sont exécutées par le driver ; le modèle reçoit trois bundles d’observations et peut choisir seulement les branches publiées. Les logs bruts supplémentaires n’augmentent pas le nombre de réveils. Mesurer tâches/checkpoints, dispatchs, coût et temps ; aucun appel supplémentaire ne peut se cacher dans la rédaction du rapport.

Un modèle de fixture retourne continue malgré un effet unknown, puis une commande hors scope, puis une décision basée sur une ancienne révision. Le harness refuse chaque effet/transition invalide sans second juge modèle. Une erreur métier anticipated produit ObservationReport puis choice ; une panne sans route publiée reste failed/blocked. Un avis sémantique médiocre mais formellement valide reste attribué au modèle : le produit ne prétend pas détecter toute erreur de jugement.

Rejouer le même parcours en mode forbidden : aucune tâche agente admissible. Une réparation préautorisée ouvre un job distinct borné ; sans autorisation il reste bloqué. Vérifier l’absence de hotpatch et les totaux mission comprenant tous les jobs. La qualification des adaptateurs modèle réels complète les doubles ; elle n’exige pas que le texte des réponses soit identique.

## Y — Modèles locaux, capacités et placement

AC-E2E-Y / T-E2E-Y / EV-E2E-Y ; INV-003/063, REQ-MODEL-013 à018. Qualifier séparément LM Studio, Ollama et llama.cpp sur la matrice de Q-19 ([modèles locaux](../specs/local-models.md)) : versions et builds figés au gel M2, modèle texte et modèle d’embedding de référence. Exercer connexion, inventaire, probe explicitement demandé, outils, sorties structurées et streaming lorsque requis par le profil. Une capacité absente ou ignorée empêche l’affectation du rôle concerné ; elle ne se transforme pas en capacité universelle parce que le dialecte s’appelle compatible.

Tester localhost menant à une destination distante, repli cloud interdit, modèle changé derrière alias, auth absente/obligatoire et endpoint distant sans transport autorisé. local_only doit refuser destination inconnue/non autorisée avant envoi du contexte. Ajouter chargement à froid, OOM, timeout et annulation dont le calcul reste actif ; les réservations et diagnostics restent explicites. Les métriques inconnues ne deviennent pas zéro. Une exécution forbidden ne démarre pas une inférence locale, un embedding ou un préchauffage attribuable au run. Voir [campagne détaillée](local-model-tests.md).

## Z — Démarrage guidé, défaut utile et parité terminal

AC-E2E-Z / T-E2E-Z / EV-E2E-Z ; INV-058/061/065, REQ-ORCH-011/012, REQ-UI-015/016, REQ-CLI-017. Partir d’un profil vierge sans agent, role utilisateur, repository ni fournisseur actif. Choisir le template recommandé par défaut `template.reconcile-csv` (Q-20), sélectionner deux exports fictifs, comprendre les entrées/sorties et démarrer un rapprochement sans configurer de modèle. Refaire depuis la TUI sans ouvrir la GUI. Le backend crée les mêmes objets et événements, avec un owner humain et une complétion mécanique explicite.

Puis choisir un objectif nécessitant une préparation/supervision modèle : la configuration guidée explique le besoin, propose un profil autorisé et montre les limites ; elle ne simule pas une capacité absente. Vérifier Save/Validate/Preview/Apply, reprise du brouillon après déconnexion, double Apply sans duplication de mission/agents, aucune action externe ni probe lourd au simple aperçu. Le mode expert donne accès aux mêmes versions, grants et réglages. Une CLI non interactive retourne un résultat structuré ou une donnée manquante explicite, jamais un prompt caché.

Le test utilisateur proposé demande à des participants non développeurs de reconnaître le résultat attendu, la source utilisée, les actions permises, la présence ou non d’un modèle et la marche à suivre en cas de blocage. Le protocole conserve leurs erreurs et les aides réellement nécessaires ; son effectif et ses seuils sont fixés par Q-20 ([parcours guidé](../design/guided-automation-experience.md)) : 5 participants en GUI et 3 en TUI depuis un profil vierge, seuils de complétion sans aide, de durée et de compréhension, sans déclarer une accessibilité universelle sur une seule réussite. Modifier le template puis exécuter un workflow custom merge-only : aucun contrôle par défaut supplémentaire n’est injecté dans ce workflow.

## Ordre de recette

Les fixtures déterministes passent d'abord. Les tests de concurrence utilisent barrières/horloge contrôlée et seeds conservées. Les tests fournisseurs et connecteurs live viennent ensuite avec comptes de test. Les pannes OS et restauration exigent des environnements isolés reproductibles. Les parcours GUI/CLI/TUI sont vérifiés sur les mêmes états backend. Aucun échec n'est masqué par une nouvelle exécution réussie sans analyse du contre-exemple initial.

