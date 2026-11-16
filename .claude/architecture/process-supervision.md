# Supervision des processus et process-host

**Contrat V1, ancrage retenu par [ADR-05](adr/ADR-05-durable-sessions.md) (accepté en R7).** Les identités durables, scopes et budgets appartiennent au daemon. Le process-host Rust ne décide pas du travail à faire : il maintient un PTY ou un service déjà autorisé, observe sa sortie et applique un protocole de contrôle restreint. Il ne possède ni fournisseur modèle, ni skill, ni rôle d'orchestrateur.

## Pourquoi un détenteur séparé

Un PTY est un objet OS possédé par des descripteurs. Le registre d'un PID ne permet pas de recréer le maître après disparition de son détenteur. La bibliothèque [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) documente création et handles, pas une reprise automatique d'un maître perdu. Un process-host `unleash-process-host` tient une seule ressource PTY ou service et est le parent direct de son enfant. Il conserve ce maître lors d'un crash du daemon et permet un rattachement sous preuve d'identité. Cette propriété doit être qualifiée avec launchd et les groupes du service ; elle n'est pas supposée survivre à un reboot.

Les workers d'invocation courts utilisent un handle enfant explicite et un protocole de fin. Un `Drop` n'est pas un protocole de supervision : [std Child](https://doc.rust-lang.org/std/process/struct.Child.html) ne garantit pas l'arrêt, et les options [Tokio Command](https://docs.rs/tokio/1.54.1/tokio/process/struct.Command.html) ne rendent pas un crash brutal observable par un destructeur Rust.

## Identité et propriétaire

Le registre contient `process_id`, `process_run_uuid` aléatoire par lancement, `authority_generation`, `action_id`, propriétaire agent/mission, cwd canonique, worktree/version, exécutable/arguments hashés, UID, PID, PGID, identité de boot, identité OS de démarrage et backend d'observation. Le manifeste privé du host conserve aussi la lignée pour l'inventaire des processus vivants, indépendamment d'un vieux snapshot DB. Un secret de contrôle est référencé séparément ; il n'est jamais passé dans un argument affichable ni écrit dans les logs.

Sur macOS, l'adaptateur peut exploiter les champs de démarrage documentés dans [proc_info.h Apple](https://github.com/apple/darwin-xnu/blob/main/bsd/sys/proc_info.h) pour l'observation. Sur Linux, un [pidfd](https://www.man7.org/linux/man-pages/man2/pidfd_send_signal.2.html) permet une signalisation visant une identité stable lorsque le noyau et les opérations nécessaires ont été qualifiés. L'observation d'un PID, même répétée avec son heure de démarrage, ne supprime jamais la course entre vérification et signal. Elle ne constitue donc pas une autorité suffisante pour `kill(pid)` ou `kill(-pgid)`.

Le contrôle exige soit un mécanisme OS à cible non réutilisable, soit un host propriétaire vivant qui conserve un ancrage de cible dont la non-réutilisation jusqu'au signal est démontrée pour l'adaptateur. Une simple connexion authentifiée au host ne fournit pas cette preuve : il doit encore posséder cet ancrage. Les garanties de groupe, d'enfant détenu non encore récolté (zombie non reaped : son PID ne peut être réattribué tant que le parent n'a pas appelé waitpid) et de handle individuel sont qualifiées séparément ; un handle de leader ne confère pas automatiquement une garantie sur tous ses descendants.

Ancrage V1 retenu : le host ne signale que son enfant direct non encore récolté et le groupe dont cet enfant est leader (`setsid`), tant que l'enfant n'est pas récolté. Conditions : SIGCHLD non ignoré, pas de `SA_NOCLDWAIT`, signaux et récolte sérialisés par le seul handle propriétaire ; sous Linux, pidfd s'ajoute lorsqu'il est qualifié. Le daemon ne signale jamais un PID observé. La non-réutilisation de PID et PGID avant récolte reste à démontrer sur macOS et Linux (expérience M0 n°3, T-F011-002/006). Sans mécanisme qualifié, l'opération est refusée `IdentityUnverified`, même si l'identité observée paraît identique. Cette règle s'applique aussi après crash du host et lors de l'adoption d'un processus orphelin.

Le PGID est mémorisé avec l'identité du leader, pas considéré globalement unique dans le temps. Un signal au groupe exige l'autorisation de ressource et un mécanisme qualifié empêchant sa réutilisation pendant l'opération ; une revalidation préalable seule ne suffit pas. À défaut, contrôler individuellement les cibles détenues de manière sûre ou refuser. Les descendants qui changent de session/groupe exigent inventaire spécifique ; une supervision Unix ne constitue pas une sandbox contre un programme hostile. Les limites sont rendues visibles au composant sécurité.

## Protocole de lancement

1. Discovery : rechercher un terminal/service vivant compatible avec mission, purpose, cwd, worktree, commande, version et permissions ; proposer réutilisation avant création.
2. Transaction : créer action et génération de processus, réserver capacité auxiliaire et port/ressource, fixer fence et payload hash, committer `ProcessLaunchPrepared`.
3. Broker : autoriser l'effet avec capacité restreinte puis lancer le host Rust dans un groupe/session qualifié pour sa durée de vie.
4. Host : écrire son manifeste privé durable, ouvrir PTY/log spool, démarrer l'enfant, collecter identité et retourner handshake avec `process_run_uuid` et challenge.
5. Daemon : vérifier le handshake et enregistrer l'observation ; la commande n'est `observed_success` que lorsqu'un critère de lancement documenté est satisfait.

Un crash entre spawn et enregistrement ne déclenche pas un deuxième spawn. La reprise inspecte les manifests privés préparés, contacte le host et vérifie l'identité. Un host qui ne peut pas confirmer que l'enfant a démarré retourne `LaunchOutcomeUnknown`. Un programme qui a quitté entre deux observations reste une génération distincte et n'est pas confondu avec le nouveau détenteur du PID.

## Contrôle et fencing

Chaque commande au host porte version protocole, process_run_uuid, authority_generation, daemon_epoch, resource_fence, command_id, action_id et hash de payload. Le host exige la même lignée, puis refuse un epoch/fence inférieur au plus haut accepté dans cette lignée. Les UUID de lignée ne sont jamais comparés par ordre. Un changement de propriétaire au sein de cette lignée requiert handshake de récupération et rotation de capacité. Une restauration crée une autre lignée : aucun rattachement à chaud du host ancien n'est autorisé, même si le nouvel epoch paraît supérieur. Les commandes d'entrée PTY sont des effets : leur répétition peut exécuter deux fois une commande, donc elles ont un numéro et un receipt durable. Si le host s'arrête après écriture au PTY avant son receipt, l'entrée est `OutcomeUnknown`; ni reconnexion ni retry ne la répètent automatiquement.

Le fencing ne peut pas annuler une commande déjà reçue par le programme cible. Le host peut fermer l'entrée, interrompre ou arrêter le groupe selon politique, puis observer. Une action de kill n'est jamais déclarée succès sur la seule réussite de l'appel de signal : le statut final ou la disparition vérifiée est enregistré. En cas de divergence d'identité, aucune signalisation « pour être sûr » n'est admise.

## Logs, ports et ressources

Le host draine stdout/stderr/PTY en continu vers un spool borné, même sans GUI. Les segments portent sequence, offset, hash et statut de synchronisation. L'UI peut recevoir les derniers octets avant flush ; ils sont explicitement volatils tant que le watermark durable n'a pas avancé. Les secrets connus sont masqués avant exposition ; les octets bruts éventuellement nécessaires relèvent d'une politique d'accès/rétention distincte.

Un port possède une lease logique et, quand l'outil le permet, un socket réellement réservé ou transmis. La course « tester libre puis bind » reste possible pour un programme externe qui ne reçoit pas le descripteur : le résultat de bind est l'arbitre, avec retry de sélection borné et état visible. Un port occupé par un processus inconnu ne justifie pas son arrêt. La compatibilité de réutilisation dépend aussi du snapshot de worktree et du protocole de readiness, pas seulement du numéro de port.

## Fermeture et reprise

Le detach GUI ne ferme pas le host. Le stop de mission émet une intention, révoque entrées futures, tente arrêt coopératif puis signal de terminaison et éventuellement arrêt forcé selon politique ; chaque phase est observée. La suppression du worktree attend la fin ou le détachement sûr des processus qui l'utilisent. Une preuve manquante bloque l'opération destructive au lieu d'effacer l'enregistrement.

Après crash daemon sans rollback de données, dans la même lignée, le host peut continuer à observer mais ne reçoit aucune autonomie nouvelle ; les entrées mutantes attendent le nouveau handshake. Après crash host, le maître PTY peut être perdu : l'enfant et ses descendants sont réconciliés, les logs durables restent disponibles, et une relance nécessite une nouvelle action explicite. Après reboot, les anciens processus sont clos en `lost` (identité de boot différente, processus réputé disparu, effets externes non conclus), sans prétendre récupérer leur RAM. Restaurer un snapshot impose d'abord inventaire complet et neutralisation vérifiée de tous les anciens exécuteurs ; sinon RestorationBlocked. La nouvelle lignée démarre uniquement des nouveaux hosts avec nouveaux secrets. F011 et F018 démontrent ces cas séparément.
