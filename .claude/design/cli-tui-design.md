# Design de la CLI et de la TUI

Statut : conception consolidée R3–R5 le 2026-11-16, sans implémentation. Le terminal donne accès au même produit que la GUI ([ADR-03](../architecture/adr/ADR-03-rust-gui.md)). La TUI porte les dix espaces et les **58 groupes** de leurs parcours complets ; la CLI non interactive expose les mêmes opérations métier. La [matrice de parité](gui-tui-parity.md) fait foi pour la couverture, et [F015](../features/F015-cli-tui/README.md) définit exigences, interfaces et preuves.

## Une application plein écran aboutie

La TUI doit se percevoir comme un outil de travail complet : hiérarchie visuelle stable, informations denses mais lisibles, états explicites, navigation cohérente et retour immédiat aux interactions. La pile adoptée ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) est Ratatui + Crossterm, avec clap pour la CLI ; ses versions observées, limites et options de réexamen sont justifiées dans la [conception technique](../features/F015-cli-tui/technical-design.md). Un défilement de logs ou un menu de commandes ne constitue pas cette interface.

| Zone | Contenu et interaction |
| --- | --- |
| Barre de contexte | Profil/projet, connexion, dernière synchronisation, opérations en vol et alerte persistante. |
| Navigation principale | Overview, Workspaces, Agents, Conversations, Workflows, Skills Studio, Knowledge, Activity, Reports, Settings ; raccourci et recherche d’espace. |
| Zone de travail | Tableaux triables/filtrables, arbres repliables, conversation, éditeur ou formulaire selon sélection. |
| Inspecteur | Contexte, provenance, documents, diff, preuves, terminal et détails sans perdre la sélection initiale. |
| Palette | Actions nommées et recherchables, cible courante, raccourci, disponibilité et raison de blocage. Même action que le menu. |
| Pied de page | Mode d’entrée/focus, raccourcis pertinents, pagination/position, état de chargement et feedback d’action. |

À 120×40, navigation, contenu et inspecteur peuvent être simultanés. À 80×24, l’inspecteur devient un panneau commutable ; les champs secondaires se déplient dans le détail. Ces dimensions sont les profils de recette de la matrice terminal qualifiée ci-dessous (80×24 minimum, 120×40 de référence). Plus étroit, le mode compact/linéaire expose les mêmes actions à raison d’un panneau à la fois. Aucun bouton vital n’est hors écran sans moyen de l’atteindre.

Tableaux et arbres rendent la partie visible et indiquent filtres/pagination. Les longs noms se dévoilent dans l’inspecteur ; leur troncature n’altère ni l’identité ni la cible. Toute couleur a un équivalent textuel, focus et sélection sont distincts, transitions et messages restent compréhensibles sans animation.

Adapter le [design system OVPT d’Unleash](visual-design-system.md) au terminal. Reprendre la hiérarchie, la sobriété et l’accent des actions prioritaires. Adapter les couleurs à la palette effectivement disponible et conserver la navigation, les statuts et le focus en mode sans couleur. Les dimensions ci-dessous sont données en cellules.

### Matrice terminal qualifiée V1

Cette matrice fait foi pour la TUI et la CLI (Q-11, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md), amendement R7). Terminaux : Terminal.app sur macOS 15, 26 et 27 ; iTerm2 et Ghostty sur macOS 27 ; terminal par défaut d’Ubuntu 24.04 et 26.04 ; tmux ≥ 3.4 en local et via SSH. Tailles : 80×24 minimum et 120×40 de référence. Environnement : TERM `xterm-256color` ou `tmux-256color` ; truecolor seulement s’il est annoncé ; `NO_COLOR` respecté. Lecteurs d’écran : VoiceOver sur macOS, recetté avec Terminal.app, et Orca sur Ubuntu, recetté avec le terminal par défaut ; leurs limites sont consignées. Un terminal hors de cette matrice n’est pas qualifié et ne reçoit aucune promesse de support.

## Navigation et modes d’entrée

La navigation conserve fil d’Ariane, espace, ID sélectionné, scroll, filtres et brouillon. Ouvrir une preuve puis revenir retrouve le contexte. Recherche globale et recherche dans une vue sont séparées et affichent leur portée. Une alerte bloquante reste accessible même si son objet est hors filtre. Les vues GUI et TUI masquent par défaut, de façon identique et réversible, les missions et runs terminaux depuis plus de 30 jours, les agents `retired` et les agents dormants sans invocation depuis 90 jours (Q-12, [modèle de stockage](../architecture/storage-model.md)). Ce filtre d’archives est un geste de présentation local : il ne supprime rien, ne change aucun état backend et se lève depuis la vue.

Raccourcis proposés : Tab/Shift+Tab changent de panneau, Entrée ouvre, Échap revient, / recherche, Ctrl+P ouvre la palette, ? affiche l’aide en mode navigation. Le choix final est configurable et qualifié selon terminal. La souris, si disponible, ajoute sélection et défilement ; aucune action n’en dépend.

Les modes navigation, formulaire, édition, chat et PTY sont visibles. Une lettre tapée dans un document n’est pas une commande globale. Un message multiligne possède une action d’envoi explicite ; collage ne signifie pas envoi. Le mode PTY réserve ses entrées au processus, montre sa commande de détachement et ne convertit pas une touche ordinaire en pause/interruption mission.

## Démarrage guidé dans Overview

L’accueil propose résultat attendu, template/procédure recommandée ou reprise d’un travail. Le formulaire progresse par goal, sources, paramètres/données, mode, modèle si nécessaire, permissions et aperçu. Il peut se présenter panneau par panneau en 80×24 ; aucune fonction ne disparaît. Un dépôt, des agents et une clé API ne sont pas des prérequis universels. Les détails techniques et le mode expert restent accessibles à tout moment, avec la même sélection et le même draft.

Le draft sauvegardé référence template/version/digest exacts et conserve les modifications des valeurs par défaut. CreateGuidedSetupDraft, SaveGuidedSetupDraft, ValidateGuidedSetup et les lectures de template/validation/preview suivent le [catalogue commun](../architecture/client-operation-catalog.md). ApplyGuidedSetup prépare idempotemment goal/mission et éventuels rôles/agents, puis rend les références. Il ne déclenche aucun effet externe ou appel modèle ; StartRun demeure distinct. Les autorités déjà configurées couvrent les étapes admissibles sans confirmation répétée ; une portée manquante est expliquée avec l’action concrète concernée.

Une lecture d’aperçu montre les inconnues au lieu de lancer un navigateur, un probe ou un LLM pour les combler. Le terminal ne promet pas un juge qualité général : seules les étapes et évaluations applicables à la procédure sont proposées. [Parcours détaillé](guided-automation-experience.md).

## Éditeurs, formulaires et diff

SKILL et rôle : contenu intégral, instructions, schémas, outils, contrats, capacités, version/namespace, usages, évaluations et composition restent inspectables et éditables. Les contrats déclaratifs et preuves mécaniques sont distingués. Le diff compare texte et champs structurés, y compris permissions et références.

WORKFLOW : le graphe est adapté en arbre ordonné d’étapes/branches et en vue source/AST. L’utilisateur ajoute/déplace des étapes, configure bornes, conditions, bindings, contrats et done, puis consulte diagnostics et simulation. Déplacer au clavier produit le même changement que le geste GUI. Un draft invalide est conservable ; sa publication/exécution suit la validation backend. Les runs engagés gardent leur version.

PROCÉDURE : sous-vue de Workflows pour AutomationArtifact, versions, mode, schéma de données, locators/scripts, contrats et qualification. Import CSV avec parsing/mapping explicites, tableau paginé, erreurs par ligne/colonne et AutomationDataRevision immuable. Les 11 commandes et 5 lectures propres F019 restent accessibles par menus, palette et CLI ; normalisation/validation déterministes, évaluation explicite puis publication. Un nouvel import ne remplace pas les données d’un run actif.

Les runs issus d’automatisation utilisent les mêmes contrôles dans Activity. Replay affiche forbidden, zéro appel modèle et absence d’agent requis. Supervised affiche blocs, checkpoints/exceptions déclarés, budget et décisions leader_decision ; rapports typés, assertions et logs sont inspectables séparément. Une recommandation du modèle ne remplace pas le contrôle du broker. Reports conserve résultats par ligne, effets inconnus, preuves et réparation séparée mandatée ; aucune nouvelle version n’est substituée au run en cours.

Les éditeurs intégrés possèdent saisie multiligne, sélection, recherche, annuler/rétablir et diagnostics localisés. Les formulaires indiquent label, valeur, champ requis, aide et erreur près de la saisie ; un conflit offre recharge ou comparaison sans écraser le brouillon. L’éditeur externe est un choix de confort, jamais le seul chemin terminal d’une fonction.

## Contexte, ressources et terminaux

Le Context Explorer terminal expose manifests et appels, instructions, rôle/skills, snapshots, documents, outils, compactions et exclusions. Autorisé, sélectionné et transmis ne sont pas confondus ; une réception fournisseur inconnue reste inconnue. Provenance, version/hash, fraîcheur, droits et limites de rétention restent visibles.

La preview s’adapte au médium : texte structuré pour Markdown, tableau navigable, arbre pour graphe, extrait autorisé et métadonnées pour document, description/métadonnées pour média. Le contenu original peut être ouvert/exporté depuis la TUI selon droits. Le support d’image terminal est facultatif ; tout contenu métier et toute opération restent accessibles sans protocole graphique terminal.

Knowledge donne Explore/Pin/Scope/Watch, discovery et décision reuse/create avec stratégie explicite selon le schéma [discovery-receipt](../schemas/discovery-receipt.schema.json), avec portée effective et résultats non intégrés clairement identifiés. Activity propose recherche causale, logs et inventaire des processus ; le terminal se lit en mode inspecteur et s’attache explicitement pour écrire. process_run_uuid, propriétaire/lease, worktree et droits sont visibles. Stop vise une génération exacte ; détacher ne stoppe pas.

## Settings et parcours externes

La TUI configure fournisseurs locaux/distants, modèles, authentification, connecteurs/scopes, limites, politiques, AI-Guard et rétention. LM Studio, Ollama et llama.cpp conservent dialecte, capacités et placement distincts. SecretRef est facultative si l’authentification qualifiée ne requiert pas de token ; lorsqu’un secret existe, il ne passe ni dans la palette, ni l’historique, ni argv.

Santé, modèle chargé/inconnu, capacités déclarées/vérifiées et fraîcheur sont des observations. TestProviderProfile annonce phase, éventuel chargement/génération, durée et budget. Ouvrir Settings ou actualiser une projection ne lance pas ce test. local_only exige un placement qualifié sur la machine du daemon ; aucun fallback cloud n’est induit par une panne locale. La sélection d’un provider n’est demandée dans le parcours guidé que si le mode a besoin de modèle. [Contrat local](../specs/local-models.md).

Un consentement OAuth part de Settings, ouvre si possible le navigateur système puis rend son état à la TUI. Annuler, constater une expiration et réessayer restent disponibles dans la TUI. Le flux et ses limites suivent la feature connecteur ; aucune capacité fournisseur absente n’est inventée. L’application graphique Unleash n’a pas à être lancée. Un refus ou un poste distant sans flux compatible affiche sa remédiation précise.

Notion n’emploie pas OAuth en V1 : son parcours d’intégration interne (Q-09, [connecteurs](../specs/connectors.md)) est identique dans Settings GUI et TUI (PAR-SETTINGS-02). 1) Ouvrir la page des intégrations Notion dans le navigateur système. 2) Créer une intégration interne en lecture seule. 3) Saisir le jeton par BeginSecretImport en saisie masquée ; il est conservé comme SecretRef. 4) TestConnectorAccount vérifie l’identité du bot. 5) Partager explicitement les pages avec l’intégration ; Explore n’affiche que les ressources partagées et le dit.

## CLI non interactive

Une mutation rend un reçu : identifiant, état accepted/completed/blocked/rejected, cible et commande d’inspection. JSON/NDJSON donne une structure versionnée ; diagnostics sur stderr. wait suit l’opération jusqu’à terme ou timeout client, sans annulation métier implicite. Projet/profil sont explicites pour les écritures. command_id, authority_generation d’origine et expected_revision permettent replay et contrôle de concurrence : l’intention scriptée est persistée avant le premier envoi et n’est jamais réémise avec la lignée annoncée par Hello ([IPC](../architecture/ipc-and-apis.md)).

Un script fournit fichiers/flux et flags documentés ; aucun dialogue de confirmation caché n’attend stdin. Les mandats backend restent identiques à ceux de la GUI/TUI. Une opération exigeant consentement externe retourne un état et une référence de flux exploitables, jamais un faux succès. Les interactions de présentation telles que redimensionner un panneau n’ont pas d’équivalent script requis ; leurs données et actions métier en ont un.

Les familles setup, automation et provider se raccordent aux verbes exacts des [interfaces F015](../features/F015-cli-tui/interfaces.md). CreateRun distingue workflow_version et automation_version ; il n’existe pas de commande de lifecycle AutomationRun concurrente. Un client non TTY peut préparer/valider/appliquer le même setup par document structuré puis lancer le Run autorisé ; une autorité manquante retourne un reçu/erreur inspectable au lieu d’un prompt caché. doctor lit la santé et ses limites ; tout probe distant ou génération exige sa commande explicite.

## États dégradés, accessibilité et restauration

Connexion perdue : bandeau persistant, date de dernière vue et opérations incertaines ; brouillons conservés localement sans soumission silencieuse. Reconnexion : snapshot/cursor et déduplication, resynchronisation après expiration, conflit explicite si l’objet a changé. Le cache est toujours marqué périmé et n’accorde aucun droit nouveau.

Preuve supprimée, refus d’accès, contenu incomplet, quota et ressource occupée ont libellé, cause connue et action disponible. Une ressource cachée n’est pas révélée par son titre. Le client borne mémoire, logs et files ; le chargement distant n’immobilise pas la navigation.

Sans couleur/souris ou avec glyphes limités, utiliser palette sobre, labels et bordures ASCII. Un mode de lecture linéaire sans rafraîchissement permanent facilite l’assistance ; sa compatibilité lecteur d’écran se vérifie sur la [matrice terminal qualifiée V1](#matrice-terminal-qualifiée-v1), avec VoiceOver et Orca. Le texte métier conserve ses caractères et reste exportable même si le chrome utilise ASCII.

Sortie, erreur, panique et signaux interceptables restaurent les modes terminal ; suspension et éditeur externe passent par le même cycle contrôlé. En raw mode, Ctrl+C et Ctrl+Z sont des événements clavier : la TUI restaure le terminal puis se suspend elle-même. SIGKILL, SIGSTOP externe, perte du terminal et arrêt système restent des limites explicites ; après SIGSTOP, seule la réinitialisation au SIGCONT est garantie. Quitter le client n’arrête pas daemon, mission ou processus. Les critères [AC-F015-001 à 017](../features/F015-cli-tui/acceptance.md) couvrent parcours avancés, guidage, automatisations, modèles locaux, rendu, ressources, erreurs et reprise.

