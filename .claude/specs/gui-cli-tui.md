# GUI, CLI et TUI : même produit et parité fonctionnelle complète

**Consolidation R2–R5, 16 novembre 2026 ; conception non implémentée.** Invariants INV-041 à 045, INV-054, INV-056/057 et contrats de F019. Les clients utilisent les mêmes commandes, requêtes, abonnements, permissions et états décrits dans [IPC](../architecture/ipc-and-apis.md). La [politique de langage](../brief/implementation-policy.md) autorise dépendances et présentation HTML/CSS autour du cœur Rust.

## Trois formes d'accès, un périmètre commun

La GUI propose une présentation graphique. La CLI interactive est une TUI plein écran, soignée et utilisable sans lancer la GUI. La CLI non interactive permet scripts, automatisation et sorties structurées. Le terme « CLI » employé par l'utilisateur inclut cette expérience terminal interactive complète.

Toutes les fonctions utilisateur de la GUI ont un parcours terminal : démarrage guidé, goals/missions, staffing si nécessaire, conversations, interventions, workflows, procédures réutilisables et données, skills/rôles, éditeurs/diff/publication, sources documentaires, knowledge, Git/worktrees quand applicables, processus, rapports, contexte, modèles locaux/distants, connecteurs, sécurité et réglages. Il n’existe pas de catégorie « fonctionnalités avancées réservées à la GUI ».

La [matrice de parité](../design/gui-tui-parity.md) est normative : **58 groupes dans les mêmes dix espaces**, dont les 50 R2 conservés et huit ajouts. Les affordances graphiques sont traduites en navigation terminal : graphe en arbre/liste structurée, souris en raccourci/action, panneau riche en vue structurée avec accès aux ressources. Les informations de provenance, d’autorisation et d’incertitude restent présentes.

Un éditeur externe peut être proposé comme confort ; les éditions nécessaires ne doivent pas dépendre de son installation. Les éditeurs intégrés de texte, skills, rôles, configuration et workflow structuré, avec diagnostics et diff, font partie de la TUI. Le navigateur système requis par un flux OAuth peut être ouvert depuis la TUI ; l'état, l'annulation, l'erreur et la reprise restent pilotables au terminal.

## Démarrer par le résultat, personnaliser les détails

Overview propose décrire le résultat, choisir un template ou réutiliser une procédure. Le parcours commun présente goal, sources, paramètres/données, mode, fournisseur seulement si nécessaire, permissions et aperçu. Il ne requiert ni agent précréé, ni dépôt, ni connaissance du vocabulaire workflow. Les valeurs par défaut sont visibles et personnalisables ; les rôles/agents nécessaires sont préparés par le backend. Aucun juge qualité LLM systématique n’est ajouté à tous les travaux.

CreateGuidedSetupDraft/SaveGuidedSetupDraft conservent une révision de template et la configuration ; ValidateGuidedSetup produit un rapport déterministe ; GetSetupPreview expose les créations, réutilisations et effets attendus avec leurs incertitudes. ApplyGuidedSetup prépare idempotemment les agrégats locaux, sans exécuter le travail. StartRun est distinct. Un mandat préconfiguré est réutilisé ; les interfaces ne demandent pas une nouvelle confirmation à chaque écran lorsqu’aucune autorité supplémentaire n’est nécessaire. Les contrats complets sont ceux du [catalogue](../architecture/client-operation-catalog.md) et de [l’expérience guidée](../design/guided-automation-experience.md).

Le mode guidé et le mode expert sont deux densités de présentation accessibles en GUI comme en TUI. Changer de densité conserve brouillon, paramètres et droits. Les versions, contrats, scripts/locators, preuves et options avancées restent accessibles dans les deux cas.

## Automatisations, supervision et fournisseurs locaux

F019 ajoute 11 commandes et 5 lectures propres au même backend. Une automatisation publiée et sa révision de données utilisent CreateRun avec source automation_version ; un workflow utilise source workflow_version. Il n’y a ni second DSL, ni nouvelle machine de Run. Draft, validation/normalisation, évaluation, publication, données et réparation sont distingués ; une nouvelle version n’est pas adoptée en cours d’effet.

En replay, forbidden interdit tout appel modèle, y compris local, manager, résumé ou réparation transitive ; aucun fournisseur n’est requis pour une procédure compatible. En supervised, allowed_bounded expose les blocs mécaniques, checkpoints/exceptions déclarés, tâches leader_decision et plafonds. Les rapports typés et assertions restent distincts des logs et des avis ; le broker refuse une continuation conflictuelle ou un retry sur effet inconnu. Le coût de préparation et d’une réparation séparée reste visible à côté de celui du run.

Settings propose les profils qualifiés LM Studio, Ollama, llama.cpp et compatible générique avec dialecte, endpoint, capacités, santé/fraîcheur, localité et fallback. SecretRef est facultative si le profil qualifié n’exige pas d’authentification. local_only contrôle le placement réel sur la machine du daemon ; loopback ou LAN seuls ne le prouvent pas. Lire les métadonnées ne lance ni téléchargement, ni chargement, ni génération ; TestProviderProfile nomme phase et bornes explicitement. Les limites et mécanismes suivent [local-models](local-models.md).

## Qualité d'interaction

La TUI dispose d'une navigation cohérente, d'une palette de commandes, d'un focus visible, d'aide contextuelle, de formulaires, de recherche/filtrage et de vues volumineuses virtualisées. Les modes terminaux moins capables disposent d'un rendu adapté et d'un mode linéaire exploitable ; les contenus techniques ne reposent pas sur la couleur seule.

Les interfaces distinguent chargement, traitement d'une commande, attente de décision et succès engagé. Elles ne demandent pas à l'utilisateur de reconstruire une action en copiant des IDs entre sorties brutes lorsqu'un parcours interactif est disponible.

Les comportements complets et la pile terminal adoptée ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) : Ratatui, Crossterm, clap) sont dans [cli-tui-design](../design/cli-tui-design.md) et [F015](../features/F015-cli-tui/README.md). Le choix GUI adopté (Tauri 2, renderer React/TypeScript) est consigné dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md). Les trois clients accèdent au daemon par le seul client commun `unleash-client`. La logique métier reste backend, indépendamment du renderer.

## État affiché et synchronisation

Un client maintient une projection locale issue d'un snapshot et d'un curseur stream_id/sequence. Sur déconnexion, la dernière vue porte un indicateur périmé ; les écritures sont désactivées ou conservées comme brouillons non soumis. Reconnexion et commandes rejouées utilisent le protocole idempotent commun, avec l'enveloppe d'origine (command_id et authority_generation) conservée ; une commande d'une lignée antérieure reçoit StaleFence et n'est jamais réémise avec la lignée courante.

États distincts : daemon arrêté, connexion perdue, synchronisation, agent dormant, worker mécanique actif, invocation en file/active/en attente, checkpoint de supervision, erreur fournisseur, mission bloquée, pause demandée/atteinte, réconciliation, issue inconnue et terminé selon contrat. Les trois clients exposent ces nuances ; une vue terminal plus compacte ne doit pas les fusionner en un seul statut. Les compteurs séparent activité mécanique et appels modèle, dans le pool global partagé défaut 5/borne dure 50, sans exemption, avec réserve de contrôle ([scheduling](scheduling.md), Q-15).

Fermer ou faire crasher la GUI ou la TUI ne termine pas les missions autorisées à continuer. La TUI restaure le mode du terminal lors des sorties maîtrisées et documente la récupération après terminaison non interceptable.

## Conversations et interventions

Un message humain adressé à N+1 persiste avec sa visibilité N+2 dans la même transaction. L'interface distingue « visible par le manager » de « analysé par un LLM ». Les décisions référencent leurs causes.

Réviser une mission, réaffecter, pause/reprise, interruption, arrêt de processus et révocation sont des actions distinctes, avec cible exacte, diff et expected_revision. Les deux clients exposent préconditions, refus, effets en vol et résultats inconnus. Les gestes graphiques et raccourcis terminal ne modifient pas l'autorité d'une commande.

## Contexte et documents

L'inspecteur affiche instructions, rôle/skills versions, sources, extraits, résultats d'outils, compactions, exclusions, payloads et provenance retenus sous politique. Il distingue le contexte initial des manifestes par appel API, ainsi que préparé, transmis et réception inconnue. Aucun raisonnement privé n'est promis.

Les trois connecteurs sont navigables depuis les deux clients, avec Explore/Pin/Scope/Watch, édition d'affectation et état des droits. Un média que le terminal ne peut rendre possède une représentation informative et une action d'ouverture/export contrôlée ; la décision utilisateur reste pilotable depuis la TUI.

## Recette de parité

Tout changement fonctionnel GUI/terminal cite sa ligne de matrice et ses critères F014/F015. Sur mêmes droits et état initial, les deux parcours doivent produire commandes équivalentes, mêmes effets backend, mêmes preuves et mêmes refus ; leurs événements conservent des identités de client distinctes. Une différence de mise en page ne justifie aucune perte de capacité.

Les critères complets sont [F014, 001 à 016](../features/F014-gui/acceptance.md), [F015, 001 à 017](../features/F015-cli-tui/acceptance.md), [F019](../features/F019-reusable-automations/acceptance.md) et le scénario O étendu. La V1 n’est pas complète si une configuration, un parcours guidé ou un éditeur reste accessible uniquement dans la GUI. Ces critères sont planifiés ; aucun test produit n’a été exécuté par cette consolidation.

