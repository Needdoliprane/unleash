# F015 — CLI et TUI Rust à parité avec la GUI

Statut : conception de la V1 complète, révisée le 2026-11-16 ; aucune implémentation du produit n’est livrée dans ce dossier.

## Objectif

Offrir le même produit en GUI et dans un terminal soigné. La TUI est une application plein écran avec les dix espaces, leurs opérations et leurs parcours complets. Elle ne se limite pas au pilotage, à la supervision ou à quelques commandes essentielles. La différence porte sur l’interaction et le rendu adaptés au terminal ; données métier, capacités, décisions, erreurs et preuves sont communes.

## Portée

La TUI comprend démarrage guidé, navigation, tableaux/arborescences, conversations, formulaires, éditeurs intégrés SKILL/WORKFLOW et rôles, procédures/données F019, comparaison de versions, contexte/provenance, documents, terminaux/logs, recherche, rapports, profils locaux/distants, connecteurs et configuration. Aucun écran ni aucune fonction de la GUI ne peut rester exclusivement graphique. Le consentement OAuth peut ouvrir le navigateur système, mais il est lancé, suivi et repris depuis la TUI ; l’application GUI n’est pas nécessaire.

La CLI non interactive conserve son rôle d’automatisation : opérations métier équivalentes, entrées par fichiers/flux, sorties humaines et JSON/NDJSON, codes de sortie, aide et complétions. Elle ne reproduit pas les gestes de mise en page. Fermer un client n’arrête pas les missions par défaut.

Un utilisateur peut partir d’un objectif ou d’un template sans dépôt, agent précréé ou jargon requis. Les defaults sont personnalisables ; données/sources, mode, provider si nécessaire, permissions et aperçu restent inspectables. ApplyGuidedSetup prépare localement et idempotemment le travail ; StartRun demeure distinct. Les droits déjà configurés sont utilisés sans confirmations systématiques. Le mode expert garde tous les éditeurs et opérations R2.

Un replay compatible avec forbidden fonctionne sans fournisseur modèle configuré ; supervised/allowed_bounded déclare ses blocs, checkpoints/exceptions, tâches leader_decision et budgets. Les profils LM Studio/Ollama/llama.cpp exposent auth conditionnelle, santé, capacités et local_only. Aucune lecture de Settings ne lance de test coûteux ou de chargement ; la configuration locale ne contourne pas forbidden. [Expérience guidée](../../design/guided-automation-experience.md), [F019](../F019-reusable-automations/README.md), [modèles locaux](../../specs/local-models.md).

Ratatui avec Crossterm est la pile terminal adoptée ([ADR-15](../../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)), avec clap pour la CLI, sous qualification sur la matrice d’[ADR-14](../../architecture/adr/ADR-14-local-data-oss.md) (amendement R7). Le client IPC commun `unleash-client` et la TUI sont en Rust ; réutiliser les bibliothèques adaptées, dont les composants d’édition et de rendu PTY choisis selon la règle d’ADR-15, sans reconstruire leurs moteurs. La GUI Tauri emploie HTML/CSS et le renderer React/TypeScript d’ADR-03. La TUI ne dépend ni de sa WebView ni de son code de rendu. Versions observées, alternatives ncurses et limites vérifiées figurent dans [technical-design](technical-design.md).

## Contrat de livraison

La [matrice GUI/TUI/CLI](../../design/gui-tui-parity.md) inventorie **58 groupes dans dix espaces** et lie leurs opérations aux preuves F015. Les [exigences](requirements.md), [critères](acceptance.md) et [tests](tests.md) portent **17 chaînes** REQ → AC → T → EV ; 001 à 008 sont conservées et renforcées, 009 à 016 décrivent finition et parcours avancés, 017 couvre guidage, automatisations et modèles locaux. Les 50 groupes R2 restent présents.

Lire ensuite le [design terminal](../../design/cli-tui-design.md), les [interfaces](interfaces.md), les [dépendances](dependencies.md) et la [spécification commune](../../specs/gui-cli-tui.md). Les doubles de contrat permettent le travail parallèle ; seule une intégration au backend commun, accompagnée de la recette terminal réelle, peut démontrer la parité. Les critères et les sources de bibliothèques ne constituent pas une preuve d’implémentation.

