# ADR-09 — Git, worktrees et intégration

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : politique de cible checkout (Q-10), Git système minimal et emplacement des worktrees fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Plusieurs features doivent avancer sans locks fichiers généralisés ni modifications sauvages de la branche humaine.

Exigences : REQ-GIT REQ-ORCH. Risques/hypothèses/questions : R-09 R-10 ; H-01 ; Q-10.

## Options et compromis

Option recommandée : **Worktree mission, intentions visibles, intégration dédiée et CAS cible**.

Alternatives considérées : Worktree par worker ; un seul worktree verrouillé ; forge comme autorité optionnelle.

Par mission limite stockage mais impose scopes/coordination internes ; par worker isole plus et coûte plus d’intégrations ; verrou global sérialise abusivement.

## Décision

Leases ciblés sur intégration/ressources, oldOID/newOID exacts, publication sur cible gérée non checkout ou après libération du checkout par l’utilisateur (Q-10), receipts et réconciliation. Agent spécialisé traite conflits ; N+2 arbitre métier.

## Amendement R7

1. La décision est adoptée : une branche et un worktree par mission qui modifie un dépôt ; worktrees enfants ou lots de patchs sous préconditions pour les mutations réellement parallèles ; un worktree d’intégration détaché par candidat ; leases ciblées sur intégration et ressources ; oldOID/newOID exacts publiés par `update-ref --no-deref` ; refs gérées directes ; lectures en `--no-replace-objects`.
2. Cible checkout (Q-10) : Unleash ne met jamais à jour une branche checkout dans un worktree quelconque, y compris pendant un rebase ou un bisect. Sous la lease de cible, l’inventaire des checkouts est relu juste avant le CAS et la cible est réobservée après publication ; une divergence devient `outcome_unknown` ou incident, jamais une correction automatique.
3. À l’adoption, une cible checkout est signalée `blocked_target_checkout` avant toute mission. Remèdes : une branche cible gérée non checkout, proposée par défaut ; ou la libération du checkout par l’utilisateur, avec instructions exactes, puis réobservation par Unleash. Aucune nouvelle opération ; ni avance rapide, reset, stash automatique, checkout forcé ni déplacement d’une ref checkout. Changer de cible change la définition de done, de façon visible. La branche gérée devient la cible des Runs créés ensuite ; un Run déjà engagé garde sa cible ([Q-03](ADR-04-workflow-semantics.md)) : seule la libération du checkout le débloque, sinon un Run successeur (`predecessor_run_id`), créé après son état terminal, adopte la nouvelle cible.
4. Git système en ligne de commande, version minimale 2.39, avec vecteurs d’arguments, sorties NUL et environnement contrôlé ; aucune bibliothèque Git Rust pour les mutations en V1. Une version inférieure donne `CapabilityDenied/GIT_VERSION_UNSUPPORTED`.
5. Les worktrees du produit sont créés par défaut sous `<données Unleash>/worktrees/<repo_id>/<allocation_id>`, emplacement modifiable par projet, toujours hors du dépôt utilisateur et hors `.claude/`. Les worktrees de développement du dépôt Unleash lui-même relèvent de l’[outillage de développement](../../dev/worktrees.md), pas de cette décision.

## Conséquences, complexité et impact V1

Git partagé conserve refs/config et effets de hooks. Les conflits fonctionnels ne sont pas garantis absents par merge textuel. Aucun test obligatoire supplémentaire n’est imposé au done utilisateur.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Scénarios A/G/L, mutation Git externe et double dispatch CAS.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/git-integration.md) ; architecture : [coordination Git](../git-coordination.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

