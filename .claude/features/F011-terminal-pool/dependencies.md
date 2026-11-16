# Dépendances F011

| Dépendance | Utilisation et limite |
|---|---|
| [F001](../F001-core-daemon/README.md) | Commandes autorisées et clients reconnectables ; GUI fermée ne ferme pas le host. |
| [F002](../F002-persistence-events/README.md) | Intention avant spawn, identités, leases, budgets et receipts ; aucun effet sans journal préparé. |
| [F003](../F003-agent-runtime/README.md) | Propriétaire invocation ; checkpoint permettant attente longue sans raisonnement hors quota. |
| [F018](../F018-reliability-recovery/README.md) | Inventaire hosts/processus et classification d'entrées/spawns inconnus. |
| Gestion Git/worktrees | Identité repository/snapshot, interdiction de suppression tant qu'une ressource dépendante n'est pas traitée. |
| Sécurité/broker | Scopes, secrets, shell permission, contrôle de sortie non fiable, policy revision et révocation. |

Les sources primaires sur portable-pty, Tokio, Apple proc_info et Linux pidfd figurent dans [systems-sources.md](../../research/systems-sources.md). Un adaptateur OS Rust doit démontrer les opérations concrètes ; une structure de header Apple ne prouve pas la compatibilité d'un binding ou les permissions de chaque OS supporté.

Le gestionnaire de services peut terminer des descendants au restart/logout. La survivance du host exige donc qualification de packaging et de groupes, séparément du fonctionnement PTY en foreground. Aucun comportement de launchd/systemd n'est inventé pour combler ce risque.

Les outils de projets sont externes : versions, disponibilité, codes de sortie et parsing sont rattachés au contrat projet. Leur absence retourne ToolUnavailable, sans limiter silencieusement la V1 aux seuls projets Rust.
