# Acceptation F001

| ID | Given / When / Then observable |
|---|---|
| AC-F001-001 | Étant donné une instance prête, quand dix clients du même utilisateur démarrent avec deux profils/racines et des variables de chemins différentes (dont `XDG_RUNTIME_DIR`), alors une seule autorité et un seul ledger de slots existent ; une racine différente reçoit AuthorityAlreadyActive et ne lance aucun exécuteur. Le changement de racine reste interdit tant que des exécuteurs anciens ne sont pas neutralisés. |
| AC-F001-002 | Étant donné une mission avec effet long contrôlé, quand toutes les fenêtres sont fermées puis rouvertes, alors le daemon conserve son identité, la mission progresse et les clients récupèrent le même historique. |
| AC-F001-003 | Étant donné une révision commune, quand une mutation CLI est observée en GUI et TUI, alors statut, IDs et watermark convergent sans rafraîchissement manuel de base ni logique de réparation côté client. |
| AC-F001-004 | Étant donné un pair non autorisé ou une capacité worker limitée, quand il usurpe actor_ref ou demande une opération hors scope, alors un refus AuthenticationFailed, CapabilityDenied ou ScopeDenied (erreur interne Unauthorized) est produit sans mutation ni effet. |
| AC-F001-005 | Étant donné une commande committée dont la réponse a été perdue, quand le même actor/command_id est renvoyé, alors le receipt initial est retourné ; payload différent et révision périmée sont rejetés. |
| AC-F001-006 | Étant donné une base avec action en vol et ancien worker, quand le daemon démarre, alors aucune admission n'intervient avant fencing/classification ; le statut recovering reste inspectable. |
| AC-F001-007 | Étant donné un effet non immédiatement annulable, quand stop est demandé, alors l'intention est visible immédiatement et le résultat final distingue arrêt complet, partiel ou inconnu (`stop_outcome` complete/partial/unknown) avec preuves. |
| AC-F001-008 | Étant donné le paquet distribué et une session complète, quand les sources et écritures runtime sont inventoriées, alors le cœur est Rust, la présentation suit la politique R2, les dépendances sont déclarées et `.claude/` n'a reçu aucun état opérationnel. |
| AC-F001-009 | Étant donné des clients lents et une file saturée, quand de nouvelles requêtes arrivent, alors mémoire/file restent bornées, la saturation est visible (ResourceBusy/BACKPRESSURE) et les requêtes déjà committées restent récupérables. |
| AC-F001-010 | Étant donné un curseur avant déconnexion puis une restauration ou purge, quand le client se reconnecte, alors il reçoit ResyncRequired et un snapshot cohérent, sans manquer un changement postérieur à son watermark. |

Le critère O du mandat est principalement démontré par 003/005/010. Les essais s'exécutent sur profils macOS/Linux déclarés. Un échec au logout n'est pas masqué par le succès d'une simple fermeture de fenêtre : les deux situations sont rapportées séparément.
