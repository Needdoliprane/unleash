# Dépendances F002

F002 dépend d'un adaptateur filesystem/secret local et d'un build SQLite qualifié. Le wrapper Rust n'efface pas la dépendance C : licence, version, options de compilation et source_id font partie du manifeste. [SQLite WAL](https://www.sqlite.org/wal.html) et la [recherche versionnée](../../research/systems-sources.md) fondent le profil, sans prouver sa qualification sur les machines supportées.

| Consommateur ou fournisseur | Relation contractuelle |
|---|---|
| [F001](../F001-core-daemon/README.md) | Fournit identité authentifiée et commandes ; reçoit receipt, état et erreurs de disponibilité. |
| [F003](../F003-agent-runtime/README.md) | Réserve slots/budgets et persiste bundle/context avant dispatch. |
| [F011](../F011-terminal-pool/README.md) | Produit observations de processus/logs et applique leases aux commandes host. |
| [F018](../F018-reliability-recovery/README.md) | Réconcilie états/pins/suppressions/backups ; importe la capsule post-snapshot et ses blocages avant activation, sans rejouer anciennes commandes ni effets. |
| Workflows/rôles/skills/Git/knowledge | Fournissent versions et provenances ; utilisent la même frontière transactionnelle. |

Le stockage ne décide pas du jugement métier N+1 ni du sens d'un résultat de test. Il conserve l'observation et sa provenance. Le contrat AI-Guard externe reste à confirmer ; les autorisations sont référencées comme décisions locales versionnées et ne supposent pas une API distante particulière.

Un adapter de migration et un testkit Rust sont des dépendances de vérification. La présence d'une sauvegarde lisible doit être démontrée avant une migration réelle ; un simple fichier portant l'extension DB n'est pas une preuve de snapshot cohérent.

Les clients F001/F014/F015 doivent persister authority_generation avec leur intention et conserver cette valeur pendant reconnexion/resync/retry. Leur file d'envoi ne peut changer lignée ou command_id pour contourner un refus. L'adaptateur filesystem doit qualifier les synchronisations et chemins d'incarnation ; aucun nom d'API OS portable n'est supposé. Ces obligations complètent la transaction SQLite, qui ne réalise pas elle-même unlink ou copie d'objets.
