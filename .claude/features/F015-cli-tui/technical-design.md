# F015 — conception technique

Statut : conception consolidée R3–R5 sur la pile adoptée par ADR-15 (R7), sans code ni test produit exécuté. La parité des **58 groupes dans dix espaces** est exigée ; les versions de bibliothèques ci-dessous sont observées dans leurs documentations au 2026-11-16 et devront être verrouillées dans le futur manifeste après qualification.

## Choix de pile et limites confirmées

**Décision adoptée ([ADR-15](../../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) : Ratatui 0.30 + Crossterm 0.29, feature `event-stream`.** Ratatui fournit le rendu terminal et les widgets ; Crossterm fournit la manipulation du terminal et les événements clavier, souris et redimensionnement. Ratatui recommande Crossterm pour le cas général [S-TUI-01, S-TUI-02, S-TUI-03]. Cette combinaison répond à l’intention « ncurses ou mieux ». C’est un choix d’adéquation au produit, pas une mesure universelle de supériorité.

| Option | Faits vérifiés | Conséquence pour Unleash |
| --- | --- | --- |
| Ratatui 0.30.2 + Crossterm 0.29.0 | Ratatui indique Rust ≥ 1.88 et Crossterm 0.29 par défaut. Le rendu est immédiat ; l’application organise état et événements. [S-TUI-01, S-TUI-02, S-TUI-05] | Pile TUI adoptée (ADR-15). Une seule version compatible de Crossterm dans le graphe ; vérifier MSRV, features, dépendances et builds macOS/Linux. |
| ncurses C + binding jeaye/ncurses-rs | Le binding est archivé depuis le 25 décembre 2024 ; 6.0.1 est sa dernière version annoncée. Son README demande ncurses installé/linkable et suggère d’autres bibliothèques Rust. [S-TUI-06] | Écarté comme choix par défaut d’un nouveau produit. L’archivage concerne ce binding, pas une affirmation d’abandon de ncurses C. |
| pancurses 0.17.0 | La documentation liste notamment ncurses et pdcurses-sys comme dépendances selon plateforme. [S-TUI-07] | Alternative curses à réexaminer seulement si une contrainte terminal l’impose ; sa couche d’abstraction ne qualifie pas toute la chaîne native. |
| Autre backend Ratatui | Ratatui documente plusieurs backends et une interface de test. [S-TUI-02] | Option de réexamen sur incident démontré, sans réduction de parité ; la matrice et la recette de plateforme restent obligatoires. |

Ratatui ne fournit pas à lui seul les éditeurs SKILL/WORKFLOW, les permissions, l’accessibilité ou la virtualisation des données backend. Ces capacités doivent être construites et prouvées. Réutiliser composants d’édition, parsers et moteurs pertinents après vérification licence/API/maintenance ; ne pas réimplémenter leurs moteurs pour une exigence artificielle de Rust intégral. Règle de choix d’ADR-15 : zone d’édition `tui-textarea` et rendu PTY `vt100` + `tui-term` si leur compatibilité avec Ratatui 0.30, leur licence et leur maintenance sont démontrées au M0 ; sinon module interne `unleash-tui::editor` fondé sur `ropey` et `unicode-segmentation`, et rendu PTY interne sur `vt100`. La logique et les adaptateurs terminal spécifiques à Unleash restent en Rust.

## Client partagé et état TUI

Un binaire unleash expose commandes scriptables et entrée tui ; aucune autre commande ne passe en raw mode ni n’attend stdin sans flag explicite. La TUI et la GUI Tauri partagent protocole, DTO, client `unleash-client` et règles backend, avec leurs propres rendus. Aucun module TUI ne dépend de la WebView. Le parseur CLI est clap 4 (API derive), version figée au M0 dans le lockfile.

Le client commun `unleash-client` traite authentification, négociation, requêtes, reçus, subscriptions, pagination et erreurs ; c’est la seule voie d’accès au daemon. Parser et actions TUI construisent les mêmes CommandEnvelope. Aucun handler ne lit ou écrit directement SQLite. Identité, scope, expected_revision, command_id et validations restent contrôlés par le backend selon [IPC](../../architecture/ipc-and-apis.md) ; l’authority_generation est épinglée dans l’intention durable du client avant le premier envoi et n’est jamais renouvelée sur retry.

L’état d’affichage contient espace actif, projet/profil, sélection par ID stable, navigation, panneau focalisé, filtres, positions de lecture, brouillons, révisions et cursors. Il distingue projection reçue, interaction locale et opérations en vol. Les dix espaces ont leurs états de présentation ; le domaine ne dépend pas des widgets. Requête, lecture d’artefact et validation longue se déroulent hors du rendu, puis rejoignent une file bornée.

Les drafts GuidedSetup et AutomationArtifact sauvegardés, leurs validations et AutomationDataRevision sont des objets backend. Le cache de formulaire garde uniquement des changements non soumis et leur base ; il ne possède pas une version autonome de mission, de workflow ou d’autorité. Les source/mode du Run et les besoins de provider sont des propriétés typées de projection, pas déduits d’un widget ou de l’existence d’une clé.

Un seul propriétaire lit les événements terminal et écrit les frames. Crossterm impose soit read/poll sur un même thread, soit EventStream, sans combinaison [S-TUI-03]. EventStream sous Tokio est adopté (ADR-15), avec une seule tâche de rendu ; l’I/O métier tourne hors du rendu sur files bornées. Éditeur, palette et PTY ne créent pas de second lecteur concurrent.

## Composants et éditions intégrées

Le shell plein écran contient navigation des dix espaces, zone de travail, inspecteur, barre d’état et palette. Tableaux, arbres repliables et formulaires possèdent recherche, focus visible, validation, raccourcis et variante compacte. Le catalogue d’actions alimente menus et palette ; chaque action suit autorité réelle, état et révision.

Les éditeurs intégrés assurent texte multiligne, sélection, copier/coller autorisé, annuler/rétablir, recherche, diagnostics et diff. SKILL sépare instructions, contrats, outils et évaluations ; rôle compose des versions précises ; WORKFLOW présente texte/AST et arbre d’étapes, branches, bornes, bindings et done. La sous-vue Procédures F019 ajoute modes, sources, scripts/locators, schémas/données et qualification. Le profil v1.1 expose des actions mécaniques sans faux binding d’agent ; les profils restent ceux du compilateur commun. Un déplacement graphique devient un déplacement clavier explicite avec diff. Validation, simulation/évaluation, publication et dépréciation passent aux services propriétaires.

Un éditeur externe reste facultatif. Son fichier temporaire privé ne contient que le document autorisé ; retour, erreur, annulation et conflit sont visibles. Le terminal est restauré avant transfert puis réinitialisé au retour. Fermer l’éditeur ne publie rien automatiquement.

## Démarrage guidé, données et suivi de procédure

Dans Overview, un formulaire séquentiel utilise ListGuidedTemplates puis CreateGuidedSetupDraft/SaveGuidedSetupDraft, ValidateGuidedSetup et les lectures de validation/preview. Goal, sources, paramètres, mode, provider conditionnel et permissions sont éditables. Les templates sont versionnés et leurs defaults personnalisables ; aucun agent précréé, dépôt, clé API ou juge qualité universel n’est requis. Sur un profil vierge, `template.reconcile-csv` est recommandé par défaut et chaque template affiche ses prérequis manquants (Q-20, [catalogue initial](../../design/guided-automation-experience.md)). Le rendu compact conserve les mêmes champs et l’accès à tous les détails.

ApplyGuidedSetup prépare idempotemment les agrégats locaux nécessaires et rend leurs identités. Le client conserve command_id et résultat, y compris après coupure ; il ne recrée pas une équipe pour compenser un reçu perdu. Aucun StartRun, apprentissage ou probe implicite. Les autorités déjà configurées sont respectées sans ajout de confirmation propre à la TUI. Le parcours reste entièrement utilisable en CLI structurée, sans dialogue stdin caché.

Le formulaire de données emploie ImportAutomationData/ValidateAutomationData et affiche lignes/colonnes, schéma, parsing, mapping, erreurs et digest de la révision immuable. Le tableau paginé ne demande pas à un modèle de corriger les entrées. L’aperçu expose version, source, données, actions futures, inconnues et coût modèle éventuel. Des données modifiées entraînent une nouvelle révision, pas un changement de run actif.

Activity suit le Run commun, qu’il provienne de workflow_version ou automation_version. Le replay forbidden n’a ni appel modèle ni invocation agente ; le worker actif compte dans le pool global partagé défaut 5/plafond 50. La supervision allowed_bounded déclare ses blocs/checkpoints/exceptions : ObservationReport, assertions, logs autorisés et tâche leader_decision. La TUI affiche fréquence, plafonds, décisions et effets inconnus ; elle n’interprète pas une sortie du leader comme une autorisation de retry. Reports expose résultats par ligne et réparation séparée avec ses propres coûts et nouvelle version.

## Contexte, previews et terminaux

L’inspecteur distingue autorisé, sélectionné, préparé, tentative de dispatch et réception confirmée ou inconnue selon le [Context Explorer](../../design/context-explorer.md). Il conserve provenance, révision/hash, appels, compactions, exclusions et rétention. Markdown devient texte structuré, tableau reste navigable, graphe devient arbre/liste de dépendances. Un binaire ou média expose métadonnées et contenu textuel disponible ; ouvrir/exporter sa source reste accessible depuis la TUI. Un protocole d’image terminal est facultatif et aucune opération métier n’en dépend.

Les vues logs neutralisent les séquences de contrôle actives. Les flux PTY utilisent un composant terminal qualifié ou un attachement explicite avec transfert contrôlé ; ils ne sont jamais recopiés sans contrôle dans la frame du shell. L’écriture exige la capacité et le lease F011 liés au process_run_uuid. Le mode attaché affiche sa touche d’échappement ; détacher libère le contrôle sans arrêter le processus.

## Configuration et authentification

Settings offre les mêmes formulaires et actions que la GUI : fournisseurs/modèles locaux ou distants, tests explicites, authentification conditionnelle, connecteurs, Explore/Pin/Scope/Watch, budgets, limites, politiques, AI-Guard et rétention. Les profils lmstudio/ollama/llama_cpp et compatibles sont distingués par dialecte. SecretRef reste facultative si le profil qualifié n’exige pas de token ; lorsqu’un secret est requis, aucune valeur dans argv, capture ou historique.

Les projections affichent santé, modèle chargé/inconnu, capacités déclarées/éprouvées/périmées, configuration de ressources et preuve de placement. local_only n’est pas déduit du nom du backend ou d’une adresse loopback ; un placement inconnu bloque la garantie stricte. SetProviderProfile garde localité/capacités/budgets dans les fallbacks. TestProviderProfile indique phase et limites : une génération ou un chargement ne se cache pas dans un rafraîchissement d’écran. La TUI ne télécharge pas de poids et ne lance pas de serveur à l’ouverture de Settings. [Spécification locale](../../specs/local-models.md).

La TUI lance et suit le service d’authentification commun. Pour les fournisseurs qualifiés, celui-ci ouvre le navigateur système et renvoie succès, refus, expiration ou annulation. Fermer le navigateur ne prouve pas le succès. Si l’ouverture automatique échoue, l’URL de continuation est fournie lorsque le flux le permet ; une session distante n’invente ni device flow ni callback supporté. Limites et mécanismes OAuth restent ceux des [connecteurs](../../specs/connectors.md). L’application GUI Unleash n’est jamais requise.

## Automatisation et erreurs

stdout JSON contient exclusivement le résultat ou le NDJSON annoncé ; diagnostics sur stderr. IDs opaques, timestamps UTC et artefacts volumineux paginés/chunkés. Projet et profil sont explicites pour les mutations scriptées. Le client peut attendre une operation_id ; son timeout n’annule pas la mission. Un command_id stable, avec l’authority_generation épinglée dans son intention durable, permet de retrouver le reçu, sans répéter un effet à l’aveugle.

Daemon inaccessible, permissions refusées, conflit, ressource occupée, source supprimée et capacité manquante disposent d’un état et d’une action applicable. Aucune base locale divergente ne sert de secours. Les brouillons hors ligne restent locaux ; après reconnexion, leur soumission vise explicitement la révision actuelle. Snapshot/cursor suivent [les événements](../../architecture/event-model.md), avec déduplication et ResyncRequired.

Un mode replay invalide pour la version, une configuration locale non qualifiée, une autorité manquante ou un effet inconnu produit le même Error/receipt que dans la GUI. doctor restitue les observations de santé disponibles ; un probe supplémentaire est une commande bornée. Il n’existe ni CreateAutomationRun ni loop de supervision dans le client : le workflow commun et le broker conservent ces décisions.

## Capacités, ressources et cycle terminal

La qualification couvre la matrice de terminaux de la [conception terminal](../../design/cli-tui-design.md#matrice-terminal-qualifiée-v1) ([ADR-14](../../architecture/adr/ADR-14-local-data-oss.md), amendement R7) : Terminal.app, iTerm2, Ghostty, terminal Ubuntu par défaut et tmux ≥ 3.4 en local et via SSH, avec les lecteurs d’écran VoiceOver et Orca. TTY, taille, encodage et capacités négociables guident le rendu, avec préférences pour couleur, souris et glyphes. Modes sans couleur, ASCII, compact et lecture linéaire gardent les actions et informations accessibles. Aucun flux non interactif n’est forcé en raw mode. La compatibilité lecteur d’écran doit être mesurée, pas attribuée automatiquement à Ratatui.

Listes/arbres sont paginés et seuls les éléments visibles sont rendus ; déplier ne charge pas tout un projet. Buffers logs, caches et files sont bornés ; contenu volumineux à la demande. L’UI regroupe les frames, préserve les événements métier et signale perte de couverture/rattrapage. La [recette](tests.md) fixe charges et budgets proposés ; aucune latence n’est déclarée atteinte.

Une seule procédure idempotente restaure raw mode, écran alternatif, curseur, souris, focus et paste. Les helpers Ratatui documentent initialisation, restauration et panic hook ; respecter l’ordre d’installation des hooks [S-TUI-04]. Unleash rétablit explicitement les modes optionnels qu’il active. Tester sortie, initialisation partielle, panique, signaux interceptables, suspension/reprise et éditeur externe. En raw mode, Ctrl+C et Ctrl+Z sont des événements clavier : la TUI restaure le terminal puis se suspend elle-même (SIGTSTP), et réinitialise raw mode et écran alternatif sur SIGCONT. SIGKILL, SIGSTOP externe, perte du terminal et arrêt système ne permettent aucune restauration préalable ; après SIGSTOP, seule la réinitialisation au SIGCONT est garantie. Cette limite ne doit pas être présentée comme un arrêt mission.

## Sources officielles et observations

Consultation : 2026-11-16. Ces liens documentent des dépendances externes, sans démontrer un comportement déjà développé dans Unleash.

| Référence | Source primaire | Confirmation |
| --- | --- | --- |
| S-TUI-01 | [Installation Ratatui](https://ratatui.rs/installation/) | 0.30.2, Rust ≥ 1.88, Crossterm 0.29 par défaut. |
| S-TUI-02 | [Backends Ratatui](https://ratatui.rs/concepts/backends/) et [comparaison](https://ratatui.rs/concepts/backends/comparison/) | Recommandation Crossterm ; versions incompatibles et état terminal distinct ; TestBackend. |
| S-TUI-03 | [Crossterm 0.29.0 — événements](https://docs.rs/crossterm/0.29.0/crossterm/event/index.html) | Clavier/souris/resize ; capture facultative ; lecture unique. |
| S-TUI-04 | [Initialisation/restauration Ratatui](https://docs.rs/ratatui/latest/ratatui/init/index.html) | Documentation affichant 0.30.2 ; helpers lifecycle, panique et ordre des hooks. |
| S-TUI-05 | [Rendu Ratatui](https://ratatui.rs/concepts/rendering/) | Rendu immédiat ; organisation de l’état et de la boucle par l’application. |
| S-TUI-06 | [Dépôt ncurses-rs](https://github.com/jeaye/ncurses-rs) | Binding archivé, dernière version annoncée 6.0.1, dépendance native ncurses. |
| S-TUI-07 | [pancurses 0.17.0](https://docs.rs/pancurses/0.17.0/pancurses/) | Dépendances ncurses/pdcurses-sys listées ; alternative à qualifier. |

