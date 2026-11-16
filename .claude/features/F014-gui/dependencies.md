# F014 — dépendances

**Révision : 16 novembre 2026. Statut : pile de présentation adoptée par [ADR-03](../../architecture/adr/ADR-03-rust-gui.md) (R7) ; qualification et gel des versions au M0.**

| Feature / service | Interaction |
| --- | --- |
| F001 | Service daemon indépendant, handshake, démarrage idempotent, état et reprise ; fermeture/crash GUI sans arrêt des jobs |
| F002 | Snapshots, reçus durables, journal, abonnement ordonné et rattrapage |
| F003 | Agents logiques, invocations/tentatives et manifestes de contexte |
| F004 | Goals/missions avec owner user ou agent, conversations, arbre et GuidedSetup commun ; préparation idempotente sans agent préalable |
| F005/F006 | Éditeur WORKFLOW, compilation, validation, simulation, bindings, versions et Run commun |
| F007/F008 | Skills, rôles, contrats, évaluations et publication |
| F009/F010/F011/F016 | Connaissance, worktrees/intégration Git, terminaux contrôlés, rapports et preuves |
| F012 | Profils locaux/distants, dialectes, auth/SecretRef conditionnelles, capacités/santé/localité, tests explicites et budgets ; aucun accès direct du renderer aux serveurs |
| F013 | Ressources documentaires, connecteurs, droits et fraîcheur ; parcours disponibles aussi en terminal |
| F015 | Parité des 58 groupes dans dix espaces, modes guidé/expert, fixtures communes et recette croisée |
| F017 | Paramètres secrets/AI-Guard, broker/coffre, parcours de confiance et actions autorisées |
| F018 | Réconciliation des effets, reçus après coupure, reprise et barrières de succession ; aucun résultat inconnu masqué |
| F019 | 11 commandes/5 lectures propres pour artefacts/données, apprentissage/qualification/publication/réparation ; CreateRun et lifecycle F005 communs, replay ou supervision de blocs |

## Composants de présentation adoptés et qualification

| Composant | Position (ADR-03) | Gate |
| --- | --- | --- |
| Tauri 2 et WebView système | Adopté : shell desktop ; hôte Rust `unleash-gui`, client du daemon par `unleash-client` | Capabilities des commandes générées, CSP, scopes, packaging et mises à jour signées ([ADR-14](../../architecture/adr/ADR-14-local-data-oss.md), amendement R7), tests réels sur la matrice de plateformes |
| Renderer | Adopté : React + TypeScript strict + Vite, bundle embarqué ; types wire générés depuis les DTO Rust par ts-rs | Diff de régénération nul en CI, accessibilité/IME, charge AC-F014-011 et maintenance ; aucune règle métier dans le TypeScript |
| Bibliothèques d’éditeur, diff, tableaux, terminal et graphe | Adoptées : CodeMirror 6, @codemirror/merge, xterm.js, TanStack Virtual, React Aria Components, @xyflow/react (vue graphe toujours doublée de l’arbre navigable) | Licence, maintenance, taille, données non fiables, charge et couverture des actes ; chaque dépendance figure au SBOM |
| egui/Iced/GPUI et autres options documentées | Options de réexamen d’ADR-03, seulement sur défaut éliminatoire intrinsèque à la WebView ou à Tauri | Même design, mêmes contrats et mêmes critères de parité/durabilité, sans retrait de capacité |
| Automatisation desktop/renderer | Vitest pour la logique de présentation ; voie WebDriver Tauri qualifiée par plateforme pour les parcours ; tests de domaine en Rust | Distinguer test de renderer simulé et test du binaire réel, couvrir la matrice de plateformes, retirer les capacités de test du build livré |

Les versions documentaires observées, Kivy comme référence à runtime Python et les sources primaires datées sont dans [la comparaison](../../research/technology-comparison.md). La présence d’une bibliothèque n’implique aucune qualification produit et la dépendance à une WebView ne transforme pas le produit local en service web distant.

Les fixtures sont verrouillées avec le contrat. Les stubs permettent un développement parallèle, mais chaque intégration réelle exige sa recette. Signaler les changements de schéma aux consommateurs et migrer les versions incompatibles. Un changement de renderer ne change pas l’autorité, la persistance ou la sémantique d’une commande.

Le plan permet un premier flux utile avant que tous les écrans soient finis. La V1 complète exige cependant les 58 groupes, le guidage non développeur, les parcours avancés R2, connecteurs, frontières de sécurité et reprises requis. Les 16 chaînes F014 incluent le premier parcours et le contrôle compréhensible des modes/modèles. La recette de cycle de vie utilise F001 réel ; un mock de daemon ne prouve pas sa survie. Voir le [chemin critique](../../plans/critical-path.md).
