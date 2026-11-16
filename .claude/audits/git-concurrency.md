# Audit 06 — Git, concurrence et intégration

**Date :** 16 novembre 2026. **Périmètre :** architecture candidate, sans dépôt produit ni tests exécutés. [Protocole expert](../agents/git-expert.md). Les faits Git utilisés sont regroupés dans [la recherche primaire](../research/git-knowledge-sources.md).

> **Note R7 (16 novembre 2026) :** ce rapport est l’historique des objections initiales, pas un contrat. Le contrat Git canonique est [git-integration](../specs/git-integration.md), avec [git-coordination](../architecture/git-coordination.md) et [ADR-09](../architecture/adr/ADR-09-git-worktrees.md), accepté en R7 : CAS par `git update-ref --no-deref` sur refs directes, lectures en `--no-replace-objects` ; aucune mise à jour d’une ref checkout et aucun « transfert » de cible (Q-10 : branche cible gérée non checkout, ou libération du checkout par l’utilisateur puis réobservation). La résolution de la stratégie d’intégration et le vocabulaire de réconciliation sont ceux du contrat canonique, pas ceux proposés ci-dessous.

## Diagnostic principal

Une branche par mission et un worktree par branche évitent une grande classe d'interférences de fichiers. Ils ne suffisent pas pour gérer plusieurs workers d'une même mission, l'intégration vers une cible commune, les mises à jour externes ou les conflits de comportement. Le système doit orchestrer quatre ressources différentes : répertoire mutable, référence Git, ressource de processus et dépendance métier. Les réunir sous un « lock de branche » masque des races.

## Constats vérifiés et risques déduits

Git documente des références partagées entre worktrees et des fichiers propres à chacun ; `worktree lock` ne fournit pas une exclusion générale des écrivains. Cette propriété est [confirmée par le manuel](https://git-scm.com/docs/git-worktree), consulté le 16 novembre 2026. La déduction de conception est qu'une mutation médiée doit porter un propriétaire, un mandat et un epoch, y compris quand le scope annoncé est différent du scope d'un autre agent.

| Risque | Situation déclenchante | Réponse proposée | Preuve attendue |
|---|---|---|---|
| GIT-R01 | Deux workers modifient simultanément le même fichier dans le même worktree | Mode par défaut : un écrivain actif par worktree ; parallélisme partagé seulement sur scopes contrôlés et générations distinctes, sinon worktrees enfants | Test de course de patchs : aucun écrasement silencieux |
| GIT-R02 | Deux missions intègrent sur le même ancien HEAD | Lease courte de cible + CAS exact ; seconde préparation recalculée | Ancien/nouvel OID et rejet `target_moved` |
| GIT-R03 | Un Git externe ignore la lease | CAS reste obligatoire, observation des mouvements externes | Aucun succès déduit de la seule lease |
| GIT-R04 | Crash après publication, avant journal résultat | Journal intention puis inspection exacte de ref/ancestry/reflog | État `desired_state_present` ou `outcome_unknown`, sans répétition aveugle |
| GIT-R05 | Branche cible checkout ailleurs | Refus de publication automatisée et transfert explicite | Fichiers humains inchangés ; motif inspectable |
| GIT-R06 | Ancien worker réanimé après expiration | Broker refuse son epoch ; le processus déjà parti est réconcilié avant réattribution | Aucune nouvelle commande Git admise au worker périmé |
| GIT-R07 | Rebase d'une branche déjà consommée | Politique explicite ; nouvelle préparation indépendante, provenance des anciens OID | Dépendants invalidés ou coordonnés |
| GIT-R08 | Fusion sans conflit textuel, interface devenue incompatible | Analyse d'impact et jugement selon contrat ; tests seulement si workflow prescrit | Rapport explicite « non vérifié » si aucune validation sémantique prévue |
| GIT-R09 | Hooks ou filtres effectuent des actions non prévues | Configuration Git effective inventoriée et exécution via broker | Aucun chemin de contournement caché sous « commande Git locale » |
| GIT-R10 | A attend B, B attend A | Graphe des attentes vérifié dans la même transaction que l'ajout d'arête | Cycle retourné avec ses missions et propositions d'arbitrage |

## Objection au CAS présenté comme solution complète

`git update-ref <ref> <new> <old>` vérifie la valeur attendue de la référence ; la [documentation](https://git-scm.com/docs/git-update-ref), consultée le 16 novembre 2026, ne fournit pas de transaction avec le journal Unleash. Une référence égale à `new` après redémarrage démontre l'état présent, pas nécessairement l'auteur de l'effet. Une référence revenue à `old` ne démontre pas que l'effet n'a jamais eu lieu. Un retour ABA et une réécriture externe peuvent effacer une observation intermédiaire.

Recommandation : distinguer `effect_observed`, `desired_state_present`, `target_diverged` et `outcome_unknown`. Le journal contient intention durable, oldOID, newOID préparé, action_id, epoch, version de politique et résultat observé. Le reflog apporte un élément corrélé, pas une preuve inviolable. Le journal historique n'est pas réécrit pour faire correspondre une ref extérieure à l'histoire préférée du daemon.

## Politique d'intégration recommandée

Préparer dans un worktree d'intégration détaché, à partir d'OID épinglés. Publier vers une ref cible non checkout, normalement administrée par Unleash. Dans un dépôt attaché où la cible est checkout, la mission reste `blocked_target_checkout` jusqu'au transfert de cible ou changement de cible explicite. Cette friction est assumée ; déplacer silencieusement le HEAD humain est exclu. L'inspection de l'inventaire n'est pas une serrure contre un processus externe créant ensuite un checkout : le périmètre coopératif et l'incertitude résiduelle restent visibles.

La stratégie par défaut proposée est `merge_commit_or_ff`, avec `ff_only` ou `rebase_then_ff` configurables au workflow. Pour `ff_only`, une divergence est un blocage, pas un changement de stratégie implicite. Pour `rebase_then_ff`, réécrire une branche partagée exige une politique préalablement autorisée ; les anciens snapshots restent accessibles pendant leur rétention. [Git rebase](https://git-scm.com/docs/git-rebase), consulté le 16 novembre 2026, documente le coût de réécriture pour les consommateurs.

Lors d'un conflit, N+2 crée ou assigne un agent intégrateur spécialisé ; N+2 tranche les priorités métier et N+1 valide la mission selon son contrat. Les choix d'intégration appartiennent à l'agent spécialisé, sous scope explicite. Une correction de conflit est un nouvel artefact avec provenance et validations prescrites, jamais un `ours` ou `theirs` généralisé sous prétexte de débloquer la file.

## Alternatives et objections conservées

Les verrous de fichiers généralisés simplifieraient certains conflits mais bloqueraient les refactorings et créeraient des attentes circulaires. Ils ne protégeraient toujours pas contre les programmes externes. Des clones indépendants renforceraient la séparation des métadonnées, au prix du stockage et des transferts ; ils restent possibles pour une mission sensible. Un seul worktree partagé maximise la rapidité de démarrage mais exige un arbitre de mutation et détruit une part importante du parallélisme voulu. Le choix recommandé conserve des worktrees distincts et réserve les leases aux ressources réellement communes.

## Vérifications avant livraison V1

Les essais doivent couvrir conflit textuel, conflit sémantique, ref checkout, mutation externe, branche supprimée, worktree déplacé, crash à chaque frontière d'effet, disque plein et journal indisponible. Les objets OID ne doivent pas être limités à une longueur SHA-1 codée en dur. Les chemins non UTF-8, liens symboliques, sous-modules, gros fichiers et dépôts sans premier commit exigent des états explicites. La preuve de conformité attendue est un jeu d'événements et d'états Git observés, pas la phrase « merged successfully » d'un agent.
