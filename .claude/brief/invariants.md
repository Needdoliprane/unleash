# Inventaire des invariants et ambiguïtés

Source contractuelle : brief utilisateur principal, sections 1–24, annexe A–M et clarifications utilisateur R2–R5 et R7 du 16 novembre 2026. La clarification R2 remplace la lecture absolue de Rust et précise la parité GUI/TUI ; voir [historique R2](revision-2026-11-16-r2.md). La demande confirmée R6 porte sur le livrable (DOC-ING-001 à 008, ADR-19) et n’ajoute aucun invariant produit. La clarification R7 ouvre la réalisation et l’outillage des agents de développement ; voir [révision R7](revision-2026-11-16-r7.md). « Confirmé » signifie demandé par Louis ; « proposé » n’est jamais une acceptation présumée.

## Invariants confirmés

| ID | Formulation normative | Origine | Responsable / features |
| --- | --- | --- | --- |
| INV-001 | Cœur applicatif et système Unleash en Rust ; bibliothèques existantes et HTML/CSS autorisés ; choix de présentation motivés selon la politique d’implémentation | §1.1, annexe A, clarification R2 qui précise la règle | Toutes ; [politique canonique](implementation-policy.md) |
| INV-002 | Projets cibles Python, TypeScript, Java, Go, Rust et multilangages ; outils externes permis | Annexe A | Runtime / F003 F011 F009 |
| INV-003 | Modèles appelés par API ; clés utilisateur selon le fournisseur, authentification locale adaptée ; aucun abonnement CLI | §1.2, clarification modèles locaux R4 | Router / F012 |
| INV-004 | OpenAI et Anthropic natifs, fournisseurs compatibles extensibles | §1.2 | Router / F012 |
| INV-005 | Modèles configurables sans dépendance structurelle de l’orchestration | §12 | Router / F012 |
| INV-006 | Toutes les capacités décrites font partie de la V1 | §1.3 | Produit / F001–F019 |
| INV-007 | Le harness possède les états, contrats, transitions et ressources | §2.2 | Domaine / F002 F005 F008 |
| INV-008 | N+2 décide de l’organisation, N+1 de la réalisation technique ou du domaine de mission, N exécute ; les tâches mécaniques restent au harness | §3, précisions polyvalence/supervision R3/R5 | Orchestration / F004 F019 |
| INV-009 | N+2 et N+1 sont invoqués sur événements utiles ; restent dormants sinon | §3, §10 | Orchestration / F004 |
| INV-010 | N+2 peut créer des N directs spécialisés | §3.1 | Orchestration / F004 |
| INV-011 | N+1 peut réaliser une feature complète sans silo technologique | §3.2 | Orchestration / F004 |
| INV-012 | Limite 5 invocations actives par défaut, maximum absolu 50 | §9.3, annexe F | Scheduler / F003 |
| INV-013 | Agents logiques dormants distincts des slots, non limités arbitrairement à cinq | §9.3 | Scheduler / F003 |
| INV-014 | Workflows créés en langage naturel puis validés, compilés et versionnés | §4.1 | Compiler / F006 |
| INV-015 | Étapes, ordre, boucles et terminaison configurables | §4.2 | Workflow / F005 |
| INV-016 | Aucun test, audit ou juge LLM supplémentaire universellement obligatoire | §4.4 | Workflow / F005 F008 |
| INV-017 | Définition du terminé appartient à l’utilisateur | §4.4 | Workflow / F005 |
| INV-018 | Définition engagée inchangée ; version nouvelle à frontière de fin éligible | §4.5, annexe G | Workflow / F005 |
| INV-019 | Skill, rôle, agent, session, workflow et worker sont distincts | §5.1 | Registres / F003 F007 |
| INV-020 | Rôles persistants, versionnés, composables par N+1 et réutilisables | Annexe B | Role Registry / F007 |
| INV-021 | Registres global, projet et repository avec résolution explicite | Annexe B | Registres / F007 |
| INV-022 | N+2 gère normalement skills et peut déléguer leur conception technique | §5.3, annexe E | Skill Registry / F007 F004 |
| INV-023 | Skills et rôles consultables, éditables, versionnés et audités | §5.4, annexe B | Studio / F007 F014 F015 |
| INV-024 | Recherche préalable fournie et tracée par harness pour créations identifiables | Annexe C | Discovery / F009 F007 F008 |
| INV-025 | Réutilisation ou création justifiée ; aucune promesse d’absence de doublons sémantiques | §7.4, annexe C | Knowledge / F009 |
| INV-026 | Vérité observable, déclaration modèle et jugement restent séparés | §6 | Contrats / F008 F016 |
| INV-027 | Harness maintient traces et index à partir des opérations observées | §6–7 | Knowledge / F009 F016 |
| INV-028 | Connaissances incluent branches/worktrees non mergés et fraîcheur vérifiable | §7, annexe D | Knowledge / F009 |
| INV-029 | Git vérité du code, journal vérité des activités enregistrées, index dérivés | §7.3 | F002 F009 F010 |
| INV-030 | Branches/worktrees isolent le travail ; pas de locks fichiers généralisés imposés | §8 | Git / F010 |
| INV-031 | Conflits techniques délégués à agent spécialisé, arbitrages métier au N+2 | §8.3 | F004 F010 |
| INV-032 | Local-first, macOS prioritaire, fermeture GUI/CLI sans arrêt nécessaire | §9.1 | F001 F014 F015 |
| INV-033 | SQLite privilégié ; Redis seulement sur justification concrète | §9.2 | F002 |
| INV-034 | Sessions durables indépendantes de RAM/forks éphémères | §9.4 | F003 F018 |
| INV-035 | Crash worker/daemon/machine traité, effets externes non répétés aveuglément | §9.5, scénario N | F018 |
| INV-036 | Terminal/process pool avec identité, propriété, logs et découverte | §11 | F011 |
| INV-037 | Effets de bord initiés via interface contrôlée du harness | §11 | Action Broker / F017 |
| INV-038 | Google Drive, OneDrive et Notion inclus, sélection documentaire explicite | §13 | F013 |
| INV-039 | OAuth documentaire permis, scopes et provenance contrôlés | §13.3 | F013 F017 |
| INV-040 | AI-Guard distinct, intégré et administrable ; contrat définitif ouvert | §14 | F017 |
| INV-041 | GUI, CLI et TUI utilisent mêmes opérations et états backend | §15, annexe K | F014 F015 |
| INV-042 | Les dix espaces demandés sont présents dans la GUI et accessibles avec parité fonctionnelle dans la TUI | §15.2, clarification R2 | F014 F015 |
| INV-043 | Message humain au N+1 produit événement visible N+2, sans réveil systématique | §15.3 | F004 F016 |
| INV-044 | Contexte réellement assemblé/transmis inspectable ; aucun raisonnement privé promis | §15.4, annexe J | F003 F012 F014 |
| INV-045 | Interventions humaines et liens causaux historisés | §15–16 | F016 |
| INV-046 | .claude est la référence documentaire, séparée du runtime : le produit ne le lit pas à l’exécution et ne dépend pas de Claude Code ; depuis R7, il héberge aussi l’outillage optionnel des agents de développement, dont les worktrees de développement sous .claude/worktrees/ | Annexe H ; clarification R7 | F001, gouvernance |
| INV-047 | Missions R1–R6 = audit et spécifications ; depuis R7, la réalisation de toute la V1 est ouverte à partir de M0, par des agents de développement soumis au protocole de goal et aux preuves du dossier | Annexe L ; clarification R7 | Tous les auteurs |
| INV-048 | Exigences, features, tests et preuves reliés ; seconde contradiction après rédaction | §21, annexe M | QA / F018 |
| INV-049 | Unleash est un produit OVPT destiné à l’open source | Message final initial | Gouvernance |
| INV-050 | Les quatorze regards experts produisent des conclusions attribuables | §18 | Audits |
| INV-051 | Budgets, quotas, équité et accès des managers ne dépassent jamais le plafond | Annexe F | F003 F012 |
| INV-052 | Protection des secrets, accès local, rétention et injections traités dès V1 | Annexe I | F017 |
| INV-053 | Versions skills/rôles, scope, contexte et causes attachés aux invocations | Annexe G/J | F003 F007 F012 |
| INV-054 | GUI et TUI présentent une qualité visuelle et d’interaction soignée, évaluée sur écran et terminal réels | Clarifications utilisateur, dont R2 | F014 F015 |
| INV-055 | Nom du produit : OVPT Unleash | Clarification en cours de mission | Gouvernance, F014 F015 |
| INV-056 | L’interface terminal interactive offre toutes les fonctions GUI, avec mêmes opérations backend, autorisations, états et preuves ; la navigation et le rendu s’adaptent au terminal | Clarification R2 : « cli iso à la gui » | F014 F015 |
| INV-057 | Le terminal est une TUI plein écran aboutie utilisant une bibliothèque adaptée de niveau ncurses ou équivalent ; la CLI non interactive reste disponible pour l’automatisation | Clarification R2 | F015 |
| INV-058 | Unleash conserve sa spécialisation développement et doit aussi être adaptable aux usages professionnels et personnels | Clarifications vocales R3/R5 | Produit / F004 F014 F015 F019 |
| INV-059 | Une procédure préparée avec un LLM peut être publiée puis rejouée avec de nouvelles données sans nouvel appel modèle | Clarification vocale R3 | Workflow / F005 F019 |
| INV-060 | Les changements invalidant une procédure doivent pouvoir conduire à une réanalyse et une nouvelle version, avec suivi des erreurs et du travail déjà exécuté | Clarification vocale R3 | F019 F018 |
| INV-061 | Le produit propose un parcours guidé compréhensible par un non-développeur, avec données, effets, résultats et réglages accessibles | Clarifications vocales R3/R5 | F014 F015 F019 |
| INV-062 | L’automatisation navigateur structurée et réutilisable sert à la fois les tests E2E et des procédures quotidiennes ; les moteurs existants peuvent être réutilisés | Clarification vocale R3 | F019 F011 F017 |
| INV-063 | Les modèles locaux, notamment LM Studio et Ollama, font partie du périmètre fournisseurs | Clarification vocale R4 ; « CPP » interprété comme llama.cpp, à distinguer du mot exact | F012 |
| INV-064 | Un LLM peut superviser des blocs de script via résultats et logs, sans décider chaque interaction ; le mode sans modèle reste disponible | Clarification vocale R5 en complément R3 | F004 F005 F012 F019 |
| INV-065 | L’absence d’agents configurés ou de connaissance des workflows ne doit pas empêcher de commencer ; proposer une procédure par défaut et préparer les rôles nécessaires | Clarification vocale R5 | F004 F007 F014 F015 |

La [matrice des invariants](../testing/invariant-coverage.md) relie ces IDs aux documents, critères d’acceptation, composants et limites. La [matrice des exigences](../testing/traceability.md) est plus fine : elle couvre chaque exigence de feature.

## Ambiguïtés significatives, sans correction silencieuse

| ID | Tension | Résolution proposée, non présumée acceptée |
| --- | --- | --- |
| AMB-01 | Lecture absolue de Rust contre dépendances SQLite/OS et présentation web | Résolue par R2 : réutilisation des bibliothèques et HTML/CSS autorisés ; cœur Rust, choix du moteur de présentation motivé, sans nouvelle demande de permission |
| AMB-02 | « Prochaine invocation » contre run déjà engagé | Un run épinglé jusqu’à état terminal et réconciliation ; version suivante au prochain nouveau run éligible |
| AMB-03 | Liberté du N+1 contre workflow déterministe | N+1 choisit parmi commandes et branches autorisées ; création nouvelle procédure = nouvelle version/run |
| AMB-04 | N+2 non technique contre production des skills | Gestion/publication par manager, conception déléguée avec preuve d’évaluation |
| AMB-05 | Recherche préalable universelle contre shell arbitraire | Receipts pour opérations identifiées ; détection après patch et clôture, couverture déclarée |
| AMB-06 | Scopes logiques contre processus même utilisateur | Contrôle des outils typés garanti ; confinement OS séparé et vérifié selon mode |
| AMB-07 | Reprise transparente contre effet externe incertain | État outcome_unknown et réconciliation ; pas de relance aveugle |
| AMB-08 | Cinq N occupés contre décision urgente du manager | Réserve ou restitution coopérative bornée, jamais dépassement de capacité |
| AMB-09 | Helpers visibles contre réutilisation disponible | Référence exacte et état d’intégration, dépendance ou transfert contrôlé |
| AMB-10 | Navigateur OAuth et outils externes | Clarifiée par annexes A/I et R2 : outils existants permis ; la TUI initie et suit OAuth sans exiger la GUI Unleash |
| AMB-11 | V1 AI-Guard complète contre contrat non fourni | Port proposé et gates de qualification externe, statut bloqué sur seules preuves manquantes |
| AMB-12 | Merge local contre branche cible checkout ailleurs | Protocole explicite de publication, pas de déplacement sauvage d’un ref checkout |
| AMB-13 | Stocker toute trace contre secrets/suppression | Métadonnées minimales et contenus sous politique de rétention ; tombstones de suppression |
| AMB-14 | Infinité d’agents logiques contre coût de stockage | Pas limite fixe cinq ; admission/budgets/archive et limites opérationnelles explicites |

