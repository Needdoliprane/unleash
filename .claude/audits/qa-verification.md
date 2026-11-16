# Regard 12 — QA and Verification

**Périmètre : vérifiabilité de la conception.** Ce document ne rapporte aucun test du produit. Il s'agit du troisième regard identifiable porté par le sous-agent systèmes, avec séparation explicite des critères et objections. Sources consultées le 16/11/2026 : [registre systèmes](../research/systems-sources.md).

## Conclusion

Les exigences sont testables si les identités, frontières de commit, états inconnus et compteurs sont des données publiques du protocole. Elles ne le sont pas si « l'agent se souvient », « le manager juge bon » ou « le terminal est repris » constituent les seuls oracles. La V1 doit exposer les preuves opérationnelles utiles, sans exposer ni promettre une chaîne de pensée privée.

Une validation métier décidée par N+1 et une validation technique du produit Unleash sont deux choses distinctes. Le workflow de l'utilisateur peut se terminer au merge sans étape universelle d'audit. En revanche, l'équipe qui développe Unleash doit vérifier le moteur qui respecte cette configuration. Aucune deuxième évaluation LLM obligatoire ne doit être introduite sous couvert de QA.

## Constats et hypothèses

Le brief fournit des scénarios A–O mais aucun code, fixture ni contrat AI-Guard existant. Les critères F001/F002/F003/F011/F018 sont donc des obligations futures. Les tests OS exigent une vraie cible macOS, même si une simulation Linux passe. Les mocks des API modèles servent à piloter retards et pannes ; ils ne démontrent pas la compatibilité réelle avec un fournisseur.

[Loom](https://docs.rs/loom/latest/loom/) et [Proptest](https://proptest-rs.github.io/proptest/intro.html) offrent des techniques pertinentes pour entrelacements et propriétés. Leur périmètre doit rester explicite : Loom n'exécute pas SQLite sous tous les ordonnancements possibles, et un test génératif fini ne prouve pas toutes les entrées. Les campagnes de pannes inspirées du [travail SQLite](https://www.sqlite.org/testing.html) doivent porter sur le protocole applicatif propre à Unleash.

## Objections et risques de faux positifs

| Faux oracle | Pourquoi il trompe | Oracle requis |
|---|---|---|
| Compteur mémoire retombé à zéro | Un ancien worker ou appel local peut continuer. | Réservations DB + inventaire processus + journal de fencing. |
| Sortie modèle « tests OK » | Elle peut être une déclaration sans exécution. | Action de test, commande exacte, commit/snapshot, exit status et logs reliés. |
| Événement unique visible GUI | L'effet peut avoir eu lieu deux fois hors UI. | Compteur de l'adaptateur externe et identité de chaque dispatch. |
| `SIGKILL` daemon équivaut à panne électrique | Le noyau et le cache disque survivent au premier cas. | Deux suites distinctes, panne processus et perte machine simulée/VM. |
| Nombre de tests élevé | Ils peuvent reproduire le même chemin heureux. | Couverture des invariants et des cutpoints, contre-exemples conservés. |
| Même rôle/skill sur trois agents | Ne prouve pas la séparation des scopes. | Trois contrats distincts, actions autorisées et refus hors scope observables. |

## Recommandations de stratégie

Le domaine doit être testé à horloge et identifiants injectés. Les transitions sont vérifiées comme une machine à états, avec invariants sur chaque préfixe d'historique. Le writer est testé contre une vraie base temporaire et les erreurs SQLite pertinentes. Les intégrations processus utilisent de petits exécutables Rust de fixture : descendant, port occupé, sortie volumineuse, pause avant effet, receipt perdu. Aucun utilitaire Python/JS n'est ajouté au produit pour faciliter ces tests.

Les scénarios de concurrence doivent imposer des barrières, pas attendre arbitrairement quelques secondes. Ils doivent réordonner deux commandes portant la même révision, un retry avec même clé, un résultat ancien après réaffectation et une réduction de limite pendant cinquante invocations. La baisse de limite bloque les nouvelles admissions sans prétendre faire disparaître instantanément les exécutions en cours.

L'équipe doit distinguer preuve d'exécution et résultat attendu dans ses artefacts. Une entrée EV décrit initialement un paquet requis avec statut `planned`. Elle devient `passed` seulement après un run signé par l'outil de test, avec environnement, versions et résultats. Un scénario non exécuté reste explicitement `not_run`, même si sa procédure semble solide.

## Critères de sortie

La couverture minimale exige tous les IDs d'exigences reliés à AC, T et EV, tous les scénarios A–O couverts quelque part, les chemins critiques de crash sur macOS et Linux, et un protocole API enregistré réellement compatible. Les liens et schémas doivent être contrôlés indépendamment de la prose.

Le rapport de livraison doit séparer les tests passés, échoués, ignorés et impossibles à exécuter, avec motif. Les SLO proposés ne deviennent pas des résultats avant mesure. Les [tests de concurrence](../testing/concurrency-tests.md), [tests de reprise](../testing/crash-recovery-tests.md) et la [stratégie](../testing/test-strategy.md) précisent ces obligations. Je bloque une déclaration « fiable » sans campagne reproductible, mais pas la rédaction de cette spécification : documenter précisément ce qui reste à démontrer est le résultat attendu à ce stade.
