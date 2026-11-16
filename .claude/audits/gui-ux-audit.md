# Audit indépendant — Expert 11, supervision et UX

> **Addendum R2 — 16 novembre 2026.** Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). Bibliothèques existantes et HTML/CSS sont autorisés ; Kivy et Tauri sont identifiés ; la GUI et la TUI doivent avoir une parité fonctionnelle complète. Les choix actuels sont dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md). Les autres observations techniques restent à lire avec leurs limites initiales.

Analyse du brief, pas test utilisateur d’une application déjà construite. Les évaluations UX proposées doivent être exécutées sur le spike puis la V1.

## Constats

Les dix écrans sont justifiés si leur navigation part du projet, du goal et des missions. L’arbre d’agents seul devient illisible quand vingt N+1 dormants et de nombreux workers historiques y apparaissent. Les rapports doivent porter une décision et ses raisons vérifiables, puis permettre de descendre vers les logs.

La coexistence de plusieurs « terminé » impose des libellés précis : tâche rendue, étape en attente de validation, mission achevée, goal satisfait. Une barre de progression probabiliste ou une couleur verte sur simple réponse du modèle induirait l’utilisateur en erreur.

## Objections

UX-R1 : afficher un token stream de chaque manager pousse l’utilisateur à surveiller en continu et masque l’objectif d’un runtime événementiel. Recommandation : activité résumée, attente et prochain déclencheur visibles ; détails sur demande.

UX-R2 : « Pause » ne peut signifier à la fois suspension immédiate, attente de checkpoint et arrêt d’un service. Séparer pause de mission, interruption d’invocation et arrêt de processus, avec reçu et état transitoire.

UX-R3 : un terminal complet dans une GUI native Rust peut devenir un projet dans le projet. L’inspecteur V1 doit satisfaire logs, interaction PTY contrôlée, resize, historique et propriétaire. Le rendu émulateur et les séquences de contrôle doivent être qualifiés ; ouvrir un terminal externe est un accès complémentaire, pas la disparition de l’inspecteur.

UX-R4 : la sélection Drive/OneDrive/Notion ne doit pas promettre une arborescence exhaustive identique aux services. La notion de ressources accessibles et les résultats partiels doivent être visibles.

UX-R5 : le contexte inspectable n’est pas nécessairement le texte intégral de chaque token opaque du fournisseur. L’écran doit expliquer provenance, sélection, redaction, compaction, exclusions et données réellement envoyées, sans afficher un faux raisonnement.

## Alternatives et recommandation

Candidat principal : egui/eframe natif. Alternative sérieuse : Iced. Les deux nécessitent un spike sur navigation clavier, lecteur d’écran, saisie IME, long texte, tableaux et charge. Dioxus desktop utilise un WebView ; Leptos/WASM génère une glue JavaScript ; Slint ajoute son langage déclaratif et un choix de licence. Leur pertinence dépend de l’interprétation explicite de Rust-only, pas d’un slogan.

La recommandation native reste proposée jusqu’à preuve du spike. Elle ne justifie aucun retrait des fonctions d’édition, de lecture complète des skills, de diff ou d’accessibilité.

## Critères d’acceptation

Sur un jeu de données fixe : vingt leaders logiques et cinquante invocations historiques ou actives restent filtrables ; l’utilisateur trouve la cause d’un blocage en trois navigations au plus ; un message N+1 montre sa visibilité N+2 sans prétendre l’avoir réveillé ; après reconnexion la GUI ne duplique ni événements ni commandes.

Une édition concurrente de skill affiche le conflit de version. Un document inaccessible ne redevient pas utilisable par présence en cache. Une action critique affiche la ressource exacte, sa révision, son état et les conséquences, puis reflète le reçu backend.

Sources framework et limites : [comparaison](../research/technology-comparison.md). Design détaillé : [écrans](../design/gui-screen-specifications.md). Aucun score de maturité chiffré n’est inventé.

