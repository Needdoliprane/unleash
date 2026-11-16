# F014 — acceptation

**Révision : 16 novembre 2026.** Critères spécifiés, non exécutés sur le produit. Identifiants 001 à 011 conservés, ajouts 012 à 014 (R2) puis 015/016 (R3–R5). Chaque preuve doit identifier build, versions de dépendances, environnement et configuration.

## AC-F014-001 — Écrans

Exigence : REQ-UI-001.

- Étant donné : Un projet possède agents, workflows, documents, logs et rapports.
- Lorsque : Parcourir tous les espaces depuis une mission.
- Alors : Aucune capacité V1 omise ; liens de causalité et retour au contexte conservés.
- Preuve attendue : EV-F014-001, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-002 — État commun

Exigence : REQ-UI-002.

- Étant donné : CLI crée une mission ; GUI et TUI sont connectées.
- Lorsque : Modifier puis intervenir depuis un autre client avec la même identité.
- Alors : Même révision et mêmes événements visibles, sans duplication de logique.
- Preuve attendue : EV-F014-002, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-003 — Supervision

Exigence : REQ-UI-003.

- Étant donné : 20 leaders logiques et cinq slots configurés.
- Lorsque : Filtrer et examiner l’arbre.
- Alors : Capacité et causalité compréhensibles sans assimiler 20 leaders à 20 appels.
- Preuve attendue : EV-F014-003, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-004 — Conversations

Exigence : REQ-UI-004.

- Étant donné : Manager dormant et message humain au leader.
- Lorsque : Envoyer puis ouvrir les deux historiques.
- Alors : Message unique, événement lié, état d’analyse distinct.
- Preuve attendue : EV-F014-004, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-005 — Contexte

Exigence : REQ-UI-005.

- Étant donné : Bundle avec skills, docs, compaction et exclusions.
- Lorsque : Ouvrir le Context Explorer d’un worker disparu.
- Alors : Contenu conservé inspectable ; sections supprimées ou opaques signalées.
- Preuve attendue : EV-F014-005, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-006 — Interventions

Exigence : REQ-UI-006.

- Étant donné : Une action externe est en vol.
- Lorsque : Demander pause puis inspection.
- Alors : États transitoires exacts ; aucune réussite fictive d’annulation.
- Preuve attendue : EV-F014-006, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-007 — Éditeurs

Exigence : REQ-UI-007.

- Étant donné : Deux éditeurs partent d’une même version.
- Lorsque : Publier deux drafts concurrents.
- Alors : Le second reçoit conflit et diff ; version engagée intacte.
- Preuve attendue : EV-F014-007, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-008 — Reconnexion

Exigence : REQ-UI-008.

- Étant donné : Client perd sa connexion puis le daemon change de stream.
- Lorsque : Reconnecter.
- Alors : Snapshot cohérent puis nouveaux événements ; aucune commande rejouée aveuglément.
- Preuve attendue : EV-F014-008, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-009 — Design

Exigence : REQ-UI-009.

- Étant donné : Même écran de mission et données sur la pile adoptée par ADR-03, build identifié.
- Lorsque : Évaluer hiérarchie, cohérence, densité, gestes, composants réutilisés et maintenance.
- Alors : Grille de revue renseignée, écrans soignés, écarts corrigés ou réexamen d’ADR-03 déclenché selon ses critères ; revue esthétique explicite du porteur ; limites explicites ; nom OVPT Unleash.
- Preuve attendue : EV-F014-009, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-010 — Accessibilité

Exigence : REQ-UI-010.

- Étant donné : Utilisateur sans souris et saisie IME.
- Lorsque : Réaliser parcours goal→mission→preuve et éditer un skill.
- Alors : Toutes les actions du parcours sont accessibles sans souris ni perte de saisie, avec focus visible et labels ; la recette de parité complète est couverte par AC-F014-012.
- Preuve attendue : EV-F014-010, artefacts et résultats du test, avec limites de portée explicites.

## AC-F014-011 — Charge

Exigence : REQ-UI-011.

- Étant donné : 1000 agents logiques, 100000 événements, logs soutenus.
- Lorsque : Filtrer, scroller, ouvrir un diff et recevoir événements.
- Alors : Objectifs M0 mesurés, UI répond, flux retardé visible.
- Preuve attendue : EV-F014-011, artefacts et résultats du test, avec limites de portée explicites.


## AC-F014-012 — Parité fonctionnelle complète

Exigence : REQ-UI-012.

- Étant donné : Catalogue versionné et matrice GUI/CLI/TUI couvrant les dix espaces, toutes lectures/mutations, le même état initial et la même identité.
- Lorsque : Exécuter chaque opération dans chacun des clients, y compris édition/validation/publication, workflows, connecteurs, paramètres, contexte, preuves, rapports et interventions ; rejouer permissions refusées, conflits et contenus riches.
- Alors : Mêmes données métier autorisées, effets, erreurs, états, règles de révision et sémantique de reçu ; aucun acte exclusivement GUI. Les variantes de rendu disposent d’une alternative accessible et d’un accès autorisé au contenu, sans inventer transcription ni promettre mêmes pixels. Une ligne manquante bloque la parité complète.
- Preuve attendue : EV-F014-012, matrice sans omission, parcours GUI/CLI/TUI et traces comparées après normalisation limitée aux identifiants/horodatages propres aux exécutions.

## AC-F014-013 — Daemon indépendant du frontend

Exigence : REQ-UI-013.

- Étant donné : Daemon installé comme service utilisateur et jobs acceptés, dont une commande engagée dont la réponse est encore inconnue du client.
- Lorsque : Fermer les fenêtres, quitter la GUI puis faire crasher WebView et hôte dans des essais distincts ; continuer l’observation via CLI/TUI et rouvrir la GUI.
- Alors : Le daemon et les hosts des jobs restent actifs selon leur contrat ; aucune pause/annulation/commande d’arrêt implicite. La GUI retrouve état et reçus par snapshot/cursor/command_id, sans effet ni message supplémentaire. L’arrêt explicite demeure une opération distincte.
- Preuve attendue : EV-F014-013, logs de service et événements corrélés, processus identifiés, progression des jobs durant l’absence GUI et absence de duplication après reconnexion sur chaque plateforme V1.

## AC-F014-014 — Frontière WebView, secrets et contenu non fiable

Exigence : REQ-UI-014.

- Étant donné : Build Tauri de production à commandes/capabilities/scopes recensés, CSP effective, contenu d’outil hostile et secrets canaris dédiés au test.
- Lorsque : Tenter un accès depuis une fenêtre non autorisée, une commande inconnue, un scope/acteur forgé ou un payload excessif ; afficher documents/logs/Markdown hostiles et effectuer un parcours de configuration de secret.
- Alors : Rejets explicites sans effet hors autorité ; validation du daemon maintenue ; aucun accès WebView au socket/credential IPC, au shell/SQL brut ou aux secrets. Contenu inerte, aucune exécution ni ressource distante non autorisée. La saisie de confiance laisse seulement référence/statut dans la WebView ; les outils de test ne sont pas exposés dans le build livré.
- Preuve attendue : EV-F014-014, manifeste effectif de commandes et permissions, CSP observée, résultats des injections, canaris absents du DOM/stockage/événements/diagnostics et erreurs/reçus corrélés. Les limites de la WebView et des handlers Rust sont consignées ; aucun succès sur mock ne vaut cette preuve.

## AC-F014-015 — Premier parcours non développeur

Exigence : REQ-UI-015.

- Étant donné : Un profil neuf ne contient aucun agent et l’utilisateur ne connaît ni skills ni workflows.
- Lorsque : Choisir un cas, ses données et une procédure recommandée ; reprendre après fermeture au milieu de la préparation.
- Alors : Le setup conserve le draft, explique effets/coûts, crée seulement les objets nécessaires et n’impose aucun écran de configuration d’agents ; le mode expert reste accessible.
- Preuve attendue : EV-F014-015, environnement, versions, observations et limites. Statut : non exécuté.

## AC-F014-016 — Contrôle compréhensible des modes

Exigence : REQ-UI-016.

- Étant donné : La même procédure possède une version replay et une version supervisée ; un profil local et un distant sont configurés.
- Lorsque : Inspecter, choisir les données, lancer chaque mode, provoquer dérive et refus local_only.
- Alors : Le badge sans modèle ne masque ni réparation ni appel local ; les logs sont distincts des assertions métier, aucun fallback cloud n’est invisible, et les trois interfaces rendent les mêmes opérations.
- Preuve attendue : EV-F014-016, environnement, versions, observations et limites. Statut : non exécuté.
