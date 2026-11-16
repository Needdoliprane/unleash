# Regard 9 — Reliability and Persistence

**Statut : audit de la proposition, aucune panne produit exécutée.** Les faits sont sourcés dans [la recherche systèmes](../research/systems-sources.md), consultée le 16/11/2026. Ce regard est produit par le sous-agent qui porte aussi les regards 2 et 12, avec critères distincts.

## Constats et risque principal

Le risque dominant n'est pas seulement de perdre une conversation : c'est de répéter une action dont le résultat n'a pas pu être journalisé. Un merge, un lancement de service, une écriture distante ou une requête facturée peuvent avoir réussi pendant que le daemon disparaît. Une transaction SQLite protège ses propres lignes ; elle ne s'étend pas à ces systèmes. Toute spécification qui appelle cette composition « exactly once » doit être rejetée.

[SQLite WAL](https://www.sqlite.org/wal.html) et les [PRAGMA de synchronisation](https://www.sqlite.org/pragma.html) motivent un stockage local, un writer propriétaire et `synchronous=FULL`. Cette proposition vise un commit reconnu durable sous les hypothèses du stockage. Elle ne promet ni réparation de matériel défectueux ni absence de corruption. Une version corrigée du WAL-reset doit être qualifiée à partir de la [release history](https://www.sqlite.org/changes.html), y compris les backports documentés ; le SQLite système ne peut pas être accepté sans contrôle.

## Risques classés

| Gravité | Scénario | État requis après découverte |
|---|---|---|
| Critique | Effet externe accompli, receipt non committé | `outcome_unknown`, preuve recherchée, interdiction de retry aveugle. |
| Critique | Worker ancien rapporte ou agit après réaffectation | Rejet `STALE_FENCE`; permissions et canal ancien invalidés. |
| Critique | PID réutilisé après crash | Processus `identity_mismatch`, aucun signal fondé sur le PID seul. |
| Élevée | Slot libéré alors qu'une requête locale vit encore | Réservation `reconciling`, admission conservatrice jusqu'au fencing et arrêt vérifié. |
| Élevée | Budget estimé dépensé deux fois par des requêtes concurrentes | Réservation atomique de la borne acceptée avant dispatch, dépenses inconnues conservées. |
| Élevée | Fichier artefact présent mais ligne absente, ou inverse | Objet préparé durable avant référence ; orphelin récupérable ; référence invalide bloquante. |
| Élevée | Copie du fichier DB sans snapshot cohérent | Sauvegarde rejetée ; utiliser procédure snapshot et manifeste des artefacts. |
| Moyenne | Horloge murale recule ou machine dort | Pas d'expiration de lease décidée par heure murale seule. |

## Objections à l'architecture candidate

Le journal ne doit pas être présenté comme une source permettant de reconstruire magiquement toute la base : ce dossier propose état courant transactionnel et historique d'événements, sans event sourcing intégral. Si une reconstruction depuis événements est exigée, il faut définir tous les upcasters et garantir la rétention correspondante. Ici une reprise normale relit l'état committé ; les index dérivés sont reconstruisibles séparément.

Un epoch valide n'arrête pas un programme déjà lancé. Le fencing doit être vérifié au moment de l'action par le broker et le process-host ; l'ancien exécuteur doit être localisé et neutralisé avant réaffectation d'une ressource exclusive. Les services externes qui n'acceptent pas ce token restent réconciliés suivant leur propre protocole. La garantie s'arrête à ce qu'Unleash contrôle.

Un lease expiré n'est pas la preuve qu'un port est libre ni qu'un merge n'est pas en cours. La reprise doit classer les ressources `suspect`, vérifier leur propriétaire et éventuellement rester bloquée. Un timeout n'autorise pas à tuer tout processus qui écoute sur le port demandé.

## Alternatives et arbitrages proposés

Le choix writer unique facilite une transaction atomique couvrant réservation, état, événement et outbox. Un broker externe ou Redis augmenterait les composants à réconcilier ; il n'est pas requis par un utilisateur local. À l'inverse, supprimer toute persistance de file économiserait des lignes mais rendrait les réveils N+2 perdables. L'outbox et l'inbox dédupliquée sont donc retenues ; leur transport reste au moins une fois.

L'annulation immédiate est un objectif d'interface, pas une garantie d'effet. La réponse `stop_requested` doit arriver vite, puis `stopped`, `partially_stopped` ou `needs_attention` seulement après observation. Les résultats inconnus doivent rester visibles et reliés à leurs coûts réservés ; les cacher pour rendre le tableau de bord vert serait une régression de sûreté.

## Vérifications attendues

Chaque coupure du [protocole de reprise](../architecture/failure-recovery.md) doit être déclenchable : avant commit, après commit avant notification, après dispatch, après effet avant receipt, pendant intégration d'un receipt, pendant sauvegarde et pendant changement d'epoch. L'oracle est l'ensemble des événements et effets observés, pas une phrase du modèle.

La sauvegarde doit utiliser un [snapshot SQLite cohérent](https://sqlite.org/backup.html), un manifeste d'objets et une restauration contrôlée sur répertoire distinct. L'opérateur doit pouvoir inspecter ce qui reste incertain avant toute remise en marche. La corruption doit conduire à une ouverture de diagnostic limitée, jamais à l'effacement du WAL pour « réparer » silencieusement.

Critères d'accord : un seul propriétaire actif par ressource, aucune admission au-delà de la limite, aucune commande acceptée perdue sans explication, aucune répétition d'effet ambigu sans décision enregistrée, sauvegarde restaurable avec références vérifiables. Critère de blocage : absence de mécanisme de preuve pour un effet à risque alors que l'interface promet une reprise automatique transparente. Dans ce cas, la fonctionnalité V1 doit inclure une réconciliation guidée, pas reporter le problème à une version ultérieure.
