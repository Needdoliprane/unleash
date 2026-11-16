# Dépendances F003

| Dépendance | Donnée ou garantie attendue | En cas d'absence |
|---|---|---|
| [F002](../F002-persistence-events/README.md) | Sessions, manifests, réservations, fences et transactions | Aucune admission ; StorageUnavailable. |
| [F001](../F001-core-daemon/README.md) | Identité et commandes uniformes, flux durable | Worker/client se reconnecte, pas de deuxième autorité. |
| Router modèle | ID/capacités effectifs, prix, deadlines, observabilité des requêtes | Invocation bloquée avec modèle/capacité manquants ; aucun ID inventé. |
| Registres rôles/skills/contrats | Versions publiées et contenus hashés | Bundle non matérialisable ; ContractUnavailable. |
| Moteur workflow | Run, étapes, dépendances et frontière terminale | Aucune adoption v2 implicite. |
| Broker sécurité/actions | Autorisation, dispatch, observation et réconciliation | Effets refusés ou unknown explicites. |
| [F011](../F011-terminal-pool/README.md) | Outils/processus persistants sans raisonnement autonome | Attente durable ou échec de ressource, sans slot caché. |
| [F018](../F018-reliability-recovery/README.md) | Récupération d'anciennes tentatives | Admission conservatrice et diagnostic. |

Le runtime ne dépend pas d'un abonnement Codex/Claude ni d'un CLI fournisseur ; seuls les adaptateurs API à clés utilisateur sont admis par le brief. Il ne dépend pas du langage du repository. Un worker peut coordonner un test Python ou un build Java via le broker tout en restant un exécutable Rust.

La frontière Q-03 et la réserve Q-15 sont fixées par [ADR-04](../../architecture/adr/ADR-04-workflow-semantics.md) et [ADR-13](../../architecture/adr/ADR-13-scheduler-budget.md) et exposées dans les paramètres. La disponibilité de slots n'implique pas disponibilité de tokens/quota chez le fournisseur ; les blocages restent séparés dans les vues.
