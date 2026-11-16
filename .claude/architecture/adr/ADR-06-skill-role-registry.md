# ADR-06 — Skills et rôles persistants

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : registres distincts sur stockage versionné commun, résolution, autorités, évaluation par défaut et reason_code fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Les skills ne sont pas des rôles, et N+1 doit composer puis réutiliser des rôles au-delà d’un seul prompt.

Exigences : REQ-SKILL REQ-ROLE. Risques/hypothèses/questions : R-06 R-19 ; H-07 H-12.

## Options et compromis

Option recommandée : **Registres conceptuellement séparés, stockage versionné commun possible**.

Alternatives considérées : Fichiers de prompts libres ; registre distant obligatoire ; rôle éphémère.

Un stockage commun limite composants, identités séparées gardent sens ; prompts seuls ne portent pas état de publication/contrats ; externe obligatoire contredit autonomie locale.

## Décision

Namespace repo/projet/global, versions immuables, discovery avant création, N+2 gère skills et délègue technique ; N+1 compose rôles sous mandat. Édition humaine tracée.

## Amendement R7

1. Skill Registry et Role Registry restent distincts dans le domaine, les identifiants, les commandes, les événements et les écrans ; ils partagent un stockage versionné adressé par contenu, dont les tables canoniques sont dans [sqlite-design](../sqlite-design.md). Aucune clé d’unicité ne contient de NULL : `scope_owner_key` vaut `global` pour la portée globale.
2. Une version publiée est un entier ≥ 1 attribué par le writer, au contenu immuable et au `content_hash` SHA-256 du JSON canonique RFC 8785 ; états de publication `published`, `deprecated`, `revoked`.
3. Les reçus de découverte restent la propriété de F009 ; F007 ne crée ni table de reçu ni table `reuse_decision`. Toute version publiée porte un `discovery_receipt_id` ; si la découverte est indisponible, le reçu déclare sa couverture limitée et sa justification.
4. Résolution non qualifiée : repository > project > global, tous les candidats autorisés restant visibles ; une égalité donne `AMBIGUOUS_REFERENCE`. Une référence qualifiée ne retombe jamais sur une autre portée.
5. N+2 publie les skills et peut déléguer leur rédaction à un N, sans auto-publication ; N+1 compose et publie des rôles dans ses grants ; l’utilisateur agit dans ses propres grants. Aucun auteur ne s’accorde de capacité.
6. Évaluation par défaut d’un nouveau skill : `required_checks` = schema, references, permissions, fixtures ; `judge_policy = none`.
7. Un binding engagé n’est jamais migré ; une nouvelle version ne sert qu’un slot vierge ou un nouveau Run ([ADR-04](ADR-04-workflow-semantics.md), Q-03).
8. Import Agent Skills : contenu non fiable converti en draft `unleash.skill.v1` ; scripts inertes jusqu’à classification d’outil ; `allowed-tools` sans effet sur les grants ; licence et source conservées.
9. reason_code canoniques des deux registres : `DISCOVERY_REQUIRED`, `AMBIGUOUS_REFERENCE`, `MISSING_DEPENDENCY`, `SCHEMA_LIMIT_EXCEEDED`, `PUBLICATION_CONFLICT`, `PERMISSION_EXPANSION_DENIED`, `INCOMPATIBLE_ROLE`, `CAPABILITY_MISMATCH`, `BINDING_ALREADY_ENGAGED`, `BUNDLE_UNAVAILABLE`.

## Conséquences, complexité et impact V1

Évaluations à maintenir, collisions et dépréciations explicites. Aucun auteur ne s’accorde des droits. Pas de dépendance Claude Code malgré imports possibles.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Scénario E, création rôle réutilisée et publication concurrente.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../skills-contract-model.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

