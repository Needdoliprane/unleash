# Principes d’interface

Exigences : REQ-UI-001 à REQ-UI-016, REQ-CLI-017 et contrats F019. Le produit s’appelle OVPT Unleash. La qualité visuelle et l’accès pour un non-développeur font partie de l’usage attendu ; la finition avancée R2 reste applicable.

1. Partir du résultat recherché. Proposer objectif, template/procédure et sources sans exiger dépôt, agent précréé ou vocabulaire workflow ; montrer état, prochain point de décision, blocage et responsable pertinent.
2. Donner une synthèse qui mène à ses preuves. Un particulier ou un comptable retrouve lignes, données et résultats ; le développeur peut descendre vers commande, snapshot et logs. Un avis de leader reste distinct d’un fait observé.
3. Distinguer état logique, activité physique et certitude. Dormant, en attente, arrêté, incertain et terminé ne partagent pas le même badge.
4. Faire comprendre la portée d’une intervention. Pause de mission, interruption d’invocation, arrêt de processus et révocation sont des verbes séparés.
5. Afficher la source et la fraîcheur de ce qui est montré. Un index stale ou un client déconnecté ne paraît pas actuel.
6. Garantir une navigation clavier et une densité réglable. Les informations restent lisibles avec vingt leaders logiques.
7. Les permissions viennent du backend ; un bouton absent ou grisé ne suffit pas comme contrôle.
8. Réduire l’activité visuelle inutile. Les streams se regroupent, l’interface conserve son calme pendant le travail autonome.
9. Présenter des valeurs par défaut personnalisables. Le mode guidé facilite les choix ; il n’ajoute pas un juge qualité LLM universel ni des étapes invisibles. Le mode expert conserve les mêmes contrats et droits.
10. Séparer préparation, application locale du setup et exécution. Montrer template/version, paramètres, sources, autorisations et aperçu ; ApplyGuidedSetup prépare, StartRun démarre. Réutiliser les mandats valides sans confirmations répétées.
11. Montrer le besoin réel de modèle. Replay forbidden reste utilisable sans fournisseur ; supervised affiche blocs, checkpoints, exceptions et budgets. Logs et assertions aident le leader sans lui permettre de contourner le broker.
12. Expliquer les profils locaux avec les mêmes exigences de preuve. Local_only, placement, capacités et santé sont distincts ; SecretRef dépend de l’authentification. Lire Settings ne lance ni chargement, ni génération, ni test coûteux.

## Revue de chaque écran

Vérifier l’action principale, le contexte visible, les états vide/chargement/erreur/déconnexion, les droits insuffisants, le texte long, le focus et les données périmées. L’écran doit rester compréhensible sans connaître les noms de crates ou détails SQLite. Les identifiants techniques sont disponibles dans l’inspecteur, pas répétés partout.

Le [design system](visual-design-system.md) fixe la cohérence ; les [écrans](gui-screen-specifications.md) décrivent les comportements. Une belle capture nominale ne suffit pas : la revue couvre les erreurs et les tailles de fenêtre.

## Parité complète et présentation adaptée — clarification R2

GUI et TUI exposent toutes les mêmes fonctions, avec catalogue, validations et états du backend commun. Les 58 groupes restent dans dix espaces, avec modes guidé et expert accessibles dans les deux clients. Le terminal possède panneaux, palette, recherche, formulaires, éditeurs et inspecteurs intégrés ; la CLI non interactive expose les mêmes opérations par données structurées. HTML/CSS et bibliothèques existantes sont permis. [Politique](../brief/implementation-policy.md), [matrice des parcours](gui-tui-parity.md), [design terminal](cli-tui-design.md), [expérience guidée](guided-automation-experience.md).

La beauté se vérifie sur contenus réels : hiérarchie, alignements, densité, focus visible, lisibilité des erreurs, stabilité du rendu et confort clavier. Un thème coloré ne dispense pas d’un mode sans couleur ni d’étiquettes de statut ; une belle capture ne démontre pas l’accès aux fonctions avancées.
