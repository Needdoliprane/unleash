# F010 — Coordination Git d'OVPT Unleash

**Statut : spécification V1, non implémentée.** Famille `REQ-GIT`. Le dossier transforme le parallélisme de missions en opérations de code traçables : allocation de worktrees, préparation d'intégration, gestion de conflits, publication et reprise. Les commandes, événements et noms de composants sont des contrats proposés.

## Valeur et périmètre

Plusieurs missions doivent avancer sur le même produit sans partager accidentellement un répertoire mutable ni écraser la cible de livraison. Le mode normal donne une branche et un worktree à chaque mission de code qui le nécessite ; une mission généraliste sans repository ne crée pas de dépôt artificiel. Les workers d'une même mission partagent un mandat explicite ; s'ils doivent écrire réellement en parallèle, ils utilisent des allocations distinctes ou des patchs appliqués sous préconditions. La simple différence de noms de scopes ne protège pas les fichiers.

F010 prend en charge les dépôts enregistrés, les branches et worktrees de missions, un worktree d'intégration spécialisé, les stratégies de merge, les conflits, les effets Git journaux et les événements d'invalidation de connaissance. Le code interne est Rust. Git et les outils de compilation des projets sont des dépendances externes contrôlées et inventoriées ; leurs langages d'implémentation ne redéfinissent pas celui d'OVPT Unleash.

## Principes de fonctionnement

Le harness détient l'état de mission et les droits d'action ; Git fait autorité sur les objets et références observés. Une intention est conservée avant une opération à effet. La publication compare la référence cible à l'OID ayant servi à préparer le candidat. Une lease coordonne les acteurs du daemon mais ne suffit pas contre des commandes Git externes. En cas de divergence, l'ancien essai reste visible et un nouveau candidat est préparé selon le contrat.

Le conflit textuel est traité par un agent spécialisé créé ou assigné par N+2. N+2 arbitre les priorités métier ; il n'est pas obligé de coder la résolution. Les conflits sémantiques font l'objet d'un risque et des validations prévues au workflow. Un workflow `Dev → Merge` demeure valide sans audit ou tests universels ajoutés par cette feature. Le résultat expose clairement ce qui a et n'a pas été vérifié.

## Limites assumées et ergonomie

Un worktree n'est pas une sandbox OS. Un processus du même compte peut modifier des fichiers ou références hors du protocole coopératif ; ces risques relèvent aussi de la sécurité et de la réconciliation. La cible d'une publication automatisée doit être non checkout ou avoir fait l'objet d'un transfert de propriété explicite. Le produit détecte cette contrainte à l'adoption du workspace, conserve le candidat prêt et propose une action claire. Il ne déplace pas silencieusement `main` sous un répertoire humain.

Un crash peut laisser l'effet Git réalisé alors que son résultat n'a pas été écrit. Le système représente cette incertitude et observe avant d'agir ; il ne promet pas exactly-once externe. Les détails sont dans [git-integration](../../specs/git-integration.md), après [audit Git](../../audits/git-concurrency.md). Les [exigences](requirements.md), [conception](technical-design.md), [interfaces](interfaces.md), [dépendances](dependencies.md), [acceptations](acceptance.md) et [tests](tests.md) forment le contrat d'implémentation.
