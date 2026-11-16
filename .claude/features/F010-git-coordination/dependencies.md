# F010 — Dépendances et intégration

| Dépendance | Contrat nécessaire | Frontière de responsabilité |
|---|---|---|
| Noyau domaine et workflow | Mission, run épinglé, stratégie, scope, définition de done | Git ne modifie pas le workflow pour rendre le candidat acceptable |
| Persistance et reprise | Journal intention/résultat, révisions, leases et inbox/outbox | SQLite ne prétend pas inclure Git dans sa transaction |
| Broker sécurité | Autorisation d'action, chemins, environnement, processus et epoch | Git adapter ne lance pas de sous-processus caché |
| Terminal/process registry | Identité durable, groupes de processus et issue observée | PID seul insuffisant après crash ou réutilisation |
| Orchestration N+2/N+1 | Mandat intégrateur, arbitrage des dépendances et validation | N+2 arbitre métier ; agent spécialisé résout le conflit technique |
| [F009 knowledge](../F009-knowledge-engine/README.md) | Snapshots, relations non mergées, reçus et invalidation | Connaissance ne publie pas une branche |
| [F016 reporting](../F016-reporting-observability/README.md) | Preuves d'action, résultats et rapports de livraison | Une claim ne vaut pas observation de merge |
| Clients communs | État prêt/bloqué/conflit et actions de reprise | GUI/CLI/TUI ne contournent pas les préconditions |

Git système est la voie proposée, avec une plage de versions à qualifier sur macOS et Linux. Les fonctionnalités réellement utilisées sont sondées ; une option récente non disponible ne provoque pas de fallback silencieux à une opération plus dangereuse. Les primitives retenues et leurs limites sont documentées dans [les sources](../../research/git-knowledge-sources.md).

Les dépôts peuvent contenir hooks, filtres, helpers, sous-modules, Git LFS ou configurations externes. Leur support V1 est explicite : inventorier, autoriser les effets nécessaires et déclarer les capacités indisponibles. Un pointeur LFS n'est pas le contenu binaire ; un gitlink n'est pas tout le sous-module. Les téléchargements et accès réseau suivent les mêmes règles que les autres effets.

La politique d'adoption est importante pour l'UX : un dépôt dont `main` est checkout doit annoncer la disponibilité de merge avant de lancer les missions. Le transfert de cible conserve le travail utilisateur et les autorisations antérieures ; le produit ne demande pas une permission déjà donnée pour une action identique, mais ne déduit pas d'un simple attachement l'autorisation de déplacer ou jeter du travail.
