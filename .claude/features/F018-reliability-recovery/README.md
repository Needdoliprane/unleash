# F018 — Fiabilité, reprise et réconciliation

## Objectif

Après une panne, retrouver les commandes reconnues, déterminer qui peut encore agir et reprendre sans répéter aveuglément un effet ambigu. La fonctionnalité couvre crash worker, crash daemon, perte process-host, reboot, stockage indisponible, corruption et restauration. Elle conserve les décisions, versions, sessions, preuves et références Git ; elle ne restaure pas la mémoire d'un fork.

La V1 comporte la réconciliation guidée des situations qui ne peuvent pas être résolues automatiquement. Un état inconnu est un résultat utile lorsqu'aucune preuve ne permet de conclure. L'interface doit permettre d'inspecter la situation et de choisir une suite autorisée, sans afficher un succès inventé pour rendre le tableau de bord propre. Après restauration, les anciennes commandes des clients restent liées à leur lignée d'origine et sont refusées ; une capsule durable conserve les effets/receipts/coûts postérieurs au snapshot et les blocages dus à une couverture incomplète.

## Garanties et limites

La transaction locale rend état/journal/outbox atomiques ; elle n'assure pas exactly-once à l'extérieur. Le fencing protège les interfaces contrôlées ; il ne retire pas une commande déjà reçue par un service distant. Un PID n'est pas une identité durable et une expiration de lease n'est pas la preuve qu'un port ou un processus a disparu. De même, un pin d'objet ou de backup ne disparaît pas au timeout : reprise ou abandon réconcilié précède sa libération. Une suppression reste deleting jusqu'à son issue vérifiée, sans publication concurrente vers les octets visés.

La capacité globale compte les invocations encore contrôlables localement. Après arrêt du canal local, un serveur peut continuer son calcul : cette action et sa réserve financière restent inconnues jusqu'à preuve ou décision documentée. Les dépendances matérielles de durabilité sont exposées et qualifiées, jamais présentées comme infaillibles.

## Références

[failure-recovery.md](../../architecture/failure-recovery.md) est le protocole canonique, avec cutpoints C0–C12. [sessions-and-recovery.md](../../specs/sessions-and-recovery.md) décrit le comportement utilisateur. F001/F002/F003/F011 apportent respectivement autorité, persistance, sessions/admission et processus. Les tests de ce dossier sont des obligations futures, non des résultats déjà obtenus.
