# F014 — GUI de supervision OVPT Unleash

**Statut : conception proposée pour la V1 complète, révisée le 16 novembre 2026 après clarification utilisateur.** Aucune implémentation ni preuve produit n’est livrée dans ce dossier.

## Objectif

Donner une interface visuellement soignée qui permette de partir d’un résultat attendu, préparer ou réutiliser une procédure, piloter les équipes lorsqu’elles sont nécessaires et retrouver décisions/preuves. Un particulier ou un comptable doit pouvoir commencer sans repository, agent précréé ou jargon workflow. Toutes les fonctions restent utilisables en CLI et en TUI, avec les mêmes données, droits, opérations et résultats.

## Portée et choix adopté

Les dix espaces Overview, Agents, Conversations, Workspaces, Workflows, Skills Studio, Activity, Reports, Knowledge et Settings sont exigés. **Tauri 2 avec renderer React/TypeScript est adopté ([ADR-03](../../architecture/adr/ADR-03-rust-gui.md), Q-02)** : hôte Rust `unleash-gui` client du daemon par `unleash-client`, renderer React + TypeScript strict construit par Vite et embarqué, types TypeScript générés depuis les DTO Rust par ts-rs avec CI en échec sur dérive. Le métier reste dans le daemon Rust commun ; le renderer ne porte aucune règle métier. La qualification de cette pile par la tranche verticale M0 reste à produire.

La clarification remplace l’interprétation « toutes les dépendances et toute la présentation doivent être Rust ». Kivy est identifié comme toolkit Python distinct de Tauri : référence UI, pas choix imposé ni réécriture du core. egui/eframe, Iced et GPUI deviennent des options de réexamen d’ADR-03, ouvertes seulement sur défaut éliminatoire mesuré ; Slint, Dioxus et GTK restent documentés dans la [comparaison](../../research/technology-comparison.md).

La GUI est un client local : fermer ou faire crasher la WebView ou son hôte ne termine pas les jobs acceptés ni le daemon. L’adaptateur Rust accède à l’IPC commun ; la WebView n’obtient pas de socket, de credential ou de secret fournisseur. Le rendu de documents et de sorties d’outils est inerte et les commandes Tauri sont explicitement bornées.

## Parité avec le terminal

La parité complète est décrite dans la [matrice GUI/TUI/CLI](../../design/gui-tui-parity.md) : 58 groupes dans les dix espaces, dont les 50 groupes avancés R2 conservés. Elle couvre les actes et les informations, sans promettre les mêmes pixels ou le même rendu de médias. Graphes, rich text et médias disposent d’une représentation accessible ou d’un accès autorisé au contenu original ; tous les actes restent disponibles sans lancer la GUI. Aucun contenu absent, comme une transcription non produite, n’est inventé.

## Guidage, automatisations et modèles — R3–R5

Overview propose des templates versionnés avec paramètres par défaut personnalisables, sources, mode, modèle uniquement si nécessaire, permissions et aperçu. Sur un profil vierge, `template.reconcile-csv` est recommandé par défaut ; chaque template affiche ses prérequis manquants (Q-20, [catalogue initial](../../design/guided-automation-experience.md)). Le contrat GuidedSetup commun prépare idempotemment les agrégats locaux ; Apply ne lance pas StartRun. Les droits déjà accordés ne provoquent pas une nouvelle confirmation à chaque écran. Aucun juge qualité LLM universel n’est imposé.

Workflows accueille les procédures F019 et leurs données ; Activity suit le même Run issu d’une workflow_version ou automation_version ; Reports expose résultats par ligne, assertions, effets et réparation séparée. Replay/forbidden reste sans modèle ; supervised/allowed_bounded affiche blocs/checkpoints/exceptions déclarés et budgets, avec conseils du leader soumis au broker. Settings configure les profils locaux ou distants, SecretRef facultative selon auth, local_only, santé et capacités, sans probe ou chargement caché. [Expérience guidée](../../design/guided-automation-experience.md), [F019](../F019-reusable-automations/README.md), [modèles locaux](../../specs/local-models.md).

## Lecture et réalisation

Commencer par la [spécification](../../specs/gui-cli-tui.md), l’[ADR-03](../../architecture/adr/ADR-03-rust-gui.md), puis [requirements](requirements.md), [technical-design](technical-design.md) et [interfaces](interfaces.md). Les [dépendances](dependencies.md) séparent contrats simulables et intégration réelle. Les [critères](acceptance.md) et [tests](tests.md) forment le contrat de sortie du chantier : **16 chaînes**. REQ-UI-001 à 011 sont conservées ; 012 à 014 couvrent parité, indépendance du daemon et WebView ; 015/016 ajoutent le guidage et le contrôle des modes/modèles.

Les équipes peuvent développer en parallèle sur des doubles de contrat versionnés. Elles doivent rejoindre les services communs avant la recette. Aucun succès sur maquette ou mock ne prouve le comportement système réel. Le code métier et ses tests restent Rust ; les composants de présentation et outils de validation sont choisis selon leur adéquation, sans réinventer une bibliothèque disponible. Les décisions externes non vérifiées restent nommées comme telles et les versions doivent être figées à la qualification.
