# F010 — Critères d'acceptation

Les critères sont des scénarios futurs, pas des validations exécutées. Ils portent sur effets Git observables, journal et états métier, indépendamment du texte que produit le modèle.

| ID | Given / contexte | When / action | Then / résultat observable |
|---|---|---|---|
| AC-F010-001 | Dépôt attaché avec main checkout, worktree dirty et opération Git en cours | Enregistrer le workspace | Inventaire et readiness exacts ; cible occupée annoncée avant mission ; aucune mutation de travail humain |
| AC-F010-002 | Deux missions et deux workers d'une même mission | Demander des allocations et mutations concurrentes | Missions séparées ; dans un même worktree, second écrivain non coordonné refusé ou mis en file |
| AC-F010-003 | Patch préparé avec ancien hash, scope réduit et epoch bientôt révoqué | Modifier le fichier puis appliquer le patch | Rejet explicite du contenu ou epoch périmé ; pas d'écrasement ni extension implicite |
| AC-F010-004 | Source et cible épinglées | Préparer puis déplacer les noms de branches | Le candidat reste lié aux anciens OID ; sa provenance et sa stratégie ne changent pas |
| AC-F010-005 | Git de publication encore actif après expiration de lease | Réattribuer la cible à un autre worker | Ressource en réconciliation ; aucun second effet admis avant résolution sûre |
| AC-F010-006 | Deux candidats partagent oldOID et Git externe peut avancer la cible | Publier concurremment puis sur cible checkout | Un CAS gagne au plus pour oldOID ; autres essais périmés ; cible checkout refusée sans toucher ses fichiers |
| AC-F010-007 | Deux missions modifient la même ligne de manière incompatible | Préparer intégration | État conflicted, preuves des deux côtés et mandat intégrateur spécialisé ; N+2 reçoit l'arbitrage métier |
| AC-F010-008 | Workflow A exige test ; workflow B est Dev→Merge | Intégrer un candidat textuellement propre | A requiert preuve sur son candidat ; B termine sans phase d'audit imposée et indique absence de validation comportementale |
| AC-F010-009 | Intention durable enregistrée et effet CAS prêt | Crasher avant effet, après effet et avant résultat | Chaque cas est réconcilié ; ambiguïtés restent visibles ; aucune publication répétée aveuglément |
| AC-F010-010 | Une feature consommée par un reçu est rebasée | Réconcilier Git | Anciens OID conservés comme historiques ; nouvelles générations et invalidation des reçus/dépendants |
| AC-F010-011 | Worktree dirty avec processus ou artefact encore requis | Demander nettoyage | Retrait refusé ; aucune perte ; nettoyage possible seulement après conditions et politique satisfaites |
| AC-F010-012 | Dépôts avec chemins Unicode/non UTF-8, symlink, sous-module, format qualifié et état unborn | Inspecter, capturer et rejouer les événements | Identités conservées, cas unsupported explicites, aucun découpage erroné de noms ni événement métier dupliqué |

## Parcours utilisateur décisif

Le scénario G est accepté lorsque le conflit est confié à un intégrateur distinct du manager, sa résolution est attribuée et la publication est reliée au candidat exact. Le scénario L est accepté lorsque la définition `done=merge` termine sans audit universel, tout en conservant les préconditions techniques et les preuves d'effet. Le scénario N exige de voir l'état réel après crash avant toute décision de reprise.

### AC-F010-001/004/006/008/009/011 — compléments R6

Une symref vers branche checkout/hors scope est refusée avant publication ; une substitution pendant la fenêtre CAS ne doit jamais déplacer son référent. Une ref de conservation symbolique n’autorise pas une suppression de sa cible. Commit/blob remplacés via refs/replace donnent les mêmes observations brutes que sans mapping, ou un refus de capacité explicite. Une preuve préexistante au profil inconnu est revalidée avant usage.

### AC-F010-001/006/009/011/012 — compléments R7

Politique de cible checkout (Q-10, [spécification](../../specs/git-integration.md)) : à l'adoption, une cible checkout dans un worktree quelconque, y compris en rebase ou en bisect, est annoncée `blocked_target_checkout` avant mission. La branche cible gérée non checkout est proposée par défaut ; la libération du checkout par l'utilisateur est décrite par des instructions exactes, puis réobservée. Aucune avance rapide, aucun reset, stash ou checkout forcé n'a lieu dans le checkout humain, et ses fichiers, index et travail dirty restent identiques octet pour octet. L'inventaire est relu sous lease immédiatement avant le CAS ; un checkout apparu pendant la fenêtre CAS est détecté par l'observation postérieure et classé `outcome_unknown` ou incident. Un Git système inférieur à 2.39 est refusé `GIT_VERSION_UNSUPPORTED` à l'adoption. Après crash, seule `effect_observed` publie automatiquement ; `desired_state_present` ne suffit pas à déclarer `done=merge`. Le balayage automatique ne retire un worktree propre et publié que 7 jours après l'état terminal de sa mission (Q-12) ; `RetireWorktree` n'attend pas ce délai mais refuse un worktree dirty, non publié ou utilisé par un processus ; un prune extérieur d'un worktree verrouillé met l'allocation en `needs_reconciliation`. Ces compléments ne créent aucun nouvel identifiant.
