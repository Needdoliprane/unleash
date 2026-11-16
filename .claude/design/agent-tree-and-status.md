# Arbre d’agents et états

La ligne d’agent identifie niveau fonctionnel, nom, rôle versionné, mission et dernier événement pertinent. Une sous-ligne indique l’état d’invocation canonique (queued, admitted, running, checkpointing, waiting_durable, reconciling) ou « aucune invocation » ; un libellé « en cours » peut regrouper admitted/running/checkpointing sans créer de valeur ([machines à états](../architecture/state-machines.md)). Le nombre de workers historiques n’augmente pas le compteur de slots.

## Agrégation

Au niveau N+2, afficher missions et décisions attendues ; au N+1, tâches/étapes et dépendances ; au N, scope et livrable. Les enfants terminés se replient dans l’historique. Les N directs créés par N+2 apparaissent sous leur mission spécialisée, avec owner et mandat, pas dans un quatrième niveau ambigu.

Les modèles sont des badges de configuration effective, pas une couleur de statut. Une modification de rôle publiée ne remplace pas le badge versionné d’une invocation déjà engagée.

## Commandes

La sélection ouvre les capacités autorisées : message, inspection contexte, pause de mission, interruption d’invocation ou réaffectation. Avant une interruption, afficher les processus associés et les effets incertains possibles. La commande possède son reçu ; la ligne passe à « interruption demandée » avant l’état observé.

## Acceptation

Avec vingt N+1 logiques et cinq slots, le total de badges actifs doit correspondre à l’admission backend. Un N+1 qui attend ses enfants passe en waiting_durable après checkpoint sûr et n’occupe alors plus de slot ; avant ce checkpoint, le slot reste compté. Le filtre « décisions à prendre » montre les managers dormants qui ont une demande, même sans processus.

Référence : [F004](../features/F004-hierarchical-orchestration/README.md), [F003](../features/F003-agent-runtime/README.md), AC-F014-003.

