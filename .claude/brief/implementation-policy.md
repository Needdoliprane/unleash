# Politique de langage, dépendances et interfaces — révision 2

**Statut : clarification utilisateur confirmée le 16 novembre 2026.** Cette politique remplace l'interprétation initiale « toute la pile et toute la présentation doivent être exclusivement Rust ». Elle ne change ni le périmètre V1 ni les sources de vérité du produit.

## Décisions confirmées

Le code métier et système propre à Unleash reste développé en Rust : daemon, workflows, scheduler, sessions, agents, contrats, permissions, gestion des effets, persistance, connecteurs, routage et services communs. La CLI/TUI est un client Rust du même backend. Les règles métier ne sont pas réimplémentées dans les clients.

Réutiliser une bibliothèque existante, un moteur de base de données ou une API système est autorisé, même si son implémentation utilise un autre langage. Le projet n'a pas à réécrire SQLite, un moteur graphique ou les composants du système d'exploitation pour obtenir une dépendance transitive entièrement Rust. Ces dépendances restent inventoriées, versionnées, licenciées et qualifiées.

HTML, CSS, ressources graphiques, formats déclaratifs et glue générée nécessaires à la présentation sont autorisés. Le renderer GUI peut donc utiliser une WebView avec un hôte Rust. Une couche applicative JavaScript/TypeScript limitée à la présentation peut être proposée lorsqu'elle simplifie réellement la GUI ; son adoption doit être motivée dans l'ADR et son contrat avec le cœur Rust doit rester explicite. Il ne s'agit ni d'une obligation de frontend Rust/WASM, ni d'une permission de déplacer l'orchestrateur ou les autorisations dans JavaScript. En R7, l'[ADR GUI](../architecture/adr/ADR-03-rust-gui.md) adopte cette couche : renderer React + TypeScript strict, types générés depuis les DTO Rust (ts-rs), aucune règle métier.

Les outils externes de build, de test, d'analyse et des repositories supervisés gardent leurs langages. Les nouveaux tests du cœur et utilitaires internes sont Rust par défaut ; les outils de test de présentation ou de plateforme peuvent être réutilisés lorsqu'ils évitent de reconstruire un outil existant. Toute extension de code applicatif hors Rust au-delà de la présentation exige une justification architecturale explicite, avec portée et coût de maintenance. L'outillage de développement hors produit (outils Python en bibliothèque standard de `tools/design-package/`, hooks et règles des agents de développement) n'est pas du code applicatif : il n'est ni livré ni exécuté par Unleash.

## Clarification des noms

Les références initialement transcrites « kavi ou st » étaient **Kivy et Tauri**. Q-17 est résolue. Ces références ne constituaient pas à elles seules un choix de framework ; ce choix est tranché par Q-02 en R7.

Kivy est un framework Python d'interface ; le considérer comme référence de produit soigné ne force pas une implémentation Python. Tauri associe un hôte Rust à une présentation web ; il est retenu par l'[ADR GUI](../architecture/adr/ADR-03-rust-gui.md), accepté en R7, avec un renderer React/TypeScript limité à la présentation, motivé et contractualisé dans l'ADR ; sa qualification sur les plateformes et parcours retenus reste à produire.

L'exemple utilisateur « on ne refait pas le moteur MySQL, on utilise la lib » clarifie la politique de réutilisation. Il ne remplace pas le choix privilégié SQLite et ne crée aucune dépendance MySQL pour Unleash.

## Parité GUI / terminal

La CLI interactive est une **TUI complète** : application plein écran, panneaux, navigation clavier, formulaires, recherche, palette de commandes, éditeurs, diff, configuration, conversations et inspection. La GUI et la TUI exposent le même périmètre fonctionnel et les mêmes autorisations. La CLI non interactive conserve ses commandes et sorties structurées pour l'automatisation.

Une opération ou une information disponible en GUI doit posséder un parcours équivalent en TUI. La représentation s'adapte : un graphe peut devenir une arborescence navigable, une prévisualisation riche un rendu textuel structuré avec accès à l'original. La contrainte du terminal ne permet pas de supprimer l'édition, la publication, la configuration ou l'inspection. Un navigateur système peut être lancé pour OAuth depuis les deux clients ; cela ne doit pas imposer le lancement de la GUI Unleash.

La matrice [GUI/TUI](../design/gui-tui-parity.md) définit les preuves de cette équivalence. Une fonctionnalité GUI ajoutée sans sa couverture terminal n'est pas terminée pour la V1. La qualité visuelle et interactive du terminal est un gate de produit, pas un simple exemple d'usage CLI.

## Ce qui reste à qualifier

Tauri 2, le renderer React/TypeScript et Ratatui/Crossterm sont retenus ([ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md), acceptés R7) ; versions verrouillées et qualification restent à produire. La permission de réutiliser les dépendances et HTML/CSS, ainsi que la parité fonctionnelle GUI/TUI, sont des décisions utilisateur déjà confirmées : elles ne doivent plus être présentées comme demandes d'autorisation ouvertes.

Les contrôles minimaux restent identiques : aucune clé fournisseur dans un renderer, aucun SQL direct client, contenu documentaire traité comme données non fiables, commandes backend authentifiées et révisionnées, daemon indépendant de la fermeture d'une interface. Les règles de réalisation qui les appliquent sont [Rust](../rules/rust.md) et [renderer GUI](../rules/gui-renderer.md).

## Application aux moteurs et scripts d’automatisation R3–R5

Playwright, WebDriver, LM Studio, Ollama ou llama.cpp sont des moteurs/serveurs externes réutilisés. Unleash développe en Rust ses adaptateurs, protocoles, contrats, contrôles et orchestration. Un script Playwright/Selenium généré pour une procédure utilisateur est un artefact de ce projet, versionné, inspectable et soumis aux autorisations ; son langage n’est pas celui du cœur d’Unleash. Une dépendance Node, Python ou C++ est inventoriée comme telle, sans cacher un backend interne parallèle.

Il existe une différence entre autoriser un langage d’artefact et prouver le contrôle de ses effets : un script libre avec accès à un compte n’est pas automatiquement médié action par action. Le profil d’exécution annonce ses garanties réelles et refuse celles qui ne peuvent être imposées. Aucun moteur d’inférence n’est reconstruit pour satisfaire la règle Rust.
