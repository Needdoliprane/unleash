# Architecture de l’information

Le contexte global contient profil, projet actif et connexion au daemon. La navigation principale conserve **dix espaces** : Overview, Workspaces, Agents, Conversations, Workflows, Skills Studio, Knowledge, Activity, Reports et Settings. Démarrage guidé, procédures et modèles locaux y sont intégrés ; ils n’ajoutent pas une navigation ni un backend concurrent. Les noms français affichés peuvent être harmonisés sans modifier les identifiants d’écrans du contrat.

## Objet central

Un goal ouvre ses missions, dépendances, décisions et conditions de fin. Une mission ouvre son run, ses sources/données, ses processus et son rapport ; ses agents et sa branche/worktree apparaissent lorsqu’ils existent. Un projet généraliste et une mission mécanique peuvent ne posséder ni dépôt ni agent. Un élément sélectionné conserve ce contexte dans un fil d’Ariane. Les liens vers preuves, conversations et contexte retournent à la sélection précédente.

Les catalogues de rôles/skills/workflows/automatisations affichent leur namespace : global, projet ou repository lorsqu’applicable. Un rôle du repository ne masque pas visuellement un rôle global homonyme : le résultat résolu indique la provenance et les alternatives. Une procédure affiche version, modes supportés, données requises et qualification, indépendamment de la présence d’un modèle configuré.

## Entrée guidée et répartition des nouvelles fonctions

Overview commence par le résultat attendu : décrire un objectif, choisir un template ou reprendre une procédure. Le parcours présente sources, paramètres, mode, modèle uniquement si requis, permissions et aperçu. Les valeurs par défaut sont personnalisables ; détails de rôles, workflow et contrats restent accessibles sans devenir un préalable. ApplyGuidedSetup prépare le travail localement ; StartRun l’exécute selon le mandat. Voir [l’expérience guidée](guided-automation-experience.md).

| Espace existant | Contenu ajouté ou précisé |
| --- | --- |
| Overview | Templates, drafts de démarrage, validation et aperçu ; missions avec owner user ou agent. |
| Workflows | Sous-vue Procédures : catalogue, draft, données/mapping, modes, qualification, publication, usages et versions. |
| Activity | Même Run issu d’un workflow ou d’une automation ; blocs, checkpoints, supervision, effets et ressources. |
| Reports | Résultats par ligne, preuves, coûts de préparation/supervision/réparation, incident et adaptation versionnée. |
| Settings | Profils locaux/distants, auth facultative selon profil, capacités, santé, local_only, ressources et fallback ; tests explicites. |

Workspaces, Agents, Conversations, Skills Studio et Knowledge conservent leurs opérations avancées. Leur état vide explique qu’aucun objet de ce type n’est nécessaire pour ce travail ; il n’exige pas d’en créer pour continuer. Mode guidé/expert et GUI/TUI sont deux choix indépendants de présentation.

## Recherche et filtres

La recherche globale est limitée au projet actif par défaut ; changer sa portée est explicite. Catégories : travail, procédures/templates, données, code, documents, rôles/skills, décisions et événements. Chaque résultat garde son type et sa fraîcheur. Un résultat non intégré propose ouvrir source, déclarer dépendance ou demander coordination, pas « utiliser » sans disponibilité vérifiée. Rechercher une procédure ou lire un profil ne génère aucun embedding ou test fournisseur caché, notamment sous forbidden.

Les filtres status/owner/role/worktree sont persistés par vue. Les alertes bloquantes restent accessibles même si un filtre cache leur ligne ; un compteur indique les alertes hors filtre. Aucune notification répétée par ligne de log.

## Chargement et permissions

Les pages chargent d’abord le résumé et les données visibles ; les preuves volumineuses à la demande. Une ressource masquée n’apparaît pas avec son titre puis un refus au clic. Les vues déconnectées montrent watermark et date ; elles ne laissent pas croire qu’un agent arrêté dans le cache l’est encore.

## Correspondance terminal obligatoire

Les dix espaces constituent également la navigation de la TUI, avec palette et raccourcis adaptés. Aucun espace de configuration, création ou inspection n’est réservé à la GUI. Une arborescence et des listes de relations peuvent remplacer un graphe visuel ; elles conservent identités, provenance, liens et opérations. La [matrice de parité](gui-tui-parity.md) relie les **58 groupes** au parcours terminal et à sa vérification, dont six groupes d’automatisation, un de guidage et un de modèles locaux.
