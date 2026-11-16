# F011 — Pool de terminaux, processus et ports

## Objectif

Donner aux agents et à l'utilisateur une vue attribuable des commandes, tests, builds et services locaux, afin de réutiliser ce qui existe et de contrôler ce qui tourne. Un terminal est une ressource appartenant à une mission/scope, avec identité de processus, logs et lease ; il n'est pas une fenêtre sans propriétaire.

La V1 inclut PTY interactifs, commandes non interactives, services longs, discovery, réutilisation, ports, arrêt observé et reprise après panne. Les outils des projets Python, TypeScript, Java, Go, Rust et multilangages sont supportés. Le code du pool et du process-host est Rust ; les programmes externes restent des dépendances d'exécution du projet.

## Valeur de la séparation

Un process-host détient le maître PTY et le spool pendant qu'un client ou le daemon se déconnecte. Il n'appelle aucun modèle et ne décide pas de nouvelle tâche. Une invocation peut se suspendre durablement pendant un test long, libérant son slot agent ; le processus conserve un budget auxiliaire et un propriétaire. Le résultat ne peut être interprété par un agent sans nouvelle admission.

## Limites et dépendances

La reprise d'un host vivant n'est pas une restauration de PTY après perte machine. PID/PGID seuls ne suffisent pas à contrôler une génération. Un groupe Unix aide la supervision mais n'est pas une sandbox contre des descendants hostiles ou détachés. Ces limites sont centrales à [process-supervision.md](../../architecture/process-supervision.md) et [terminal-management.md](../../specs/terminal-management.md).

F001 fournit l'IPC, F002 les intentions/leases/preuves, F003 l'attribution aux invocations et F018 la reprise. Les contracts de sécurité délimitent les effets permis. Les sept documents de cette feature définissent les preuves à obtenir ; ils ne rapportent aucun terminal de produit déjà vérifié.
