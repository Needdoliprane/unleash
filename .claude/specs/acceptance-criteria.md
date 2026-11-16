# Critères d'acceptation de la V1 complète

Le dossier spécifie des garanties à réaliser. Il ne certifie pas un logiciel déjà construit. Les critères détaillés de chaque feature font autorité sur leur comportement ; la [matrice des exigences](../testing/traceability.md) relie leurs tests et preuves.

## Acceptation à plusieurs niveaux

- **Tâche** : sortie et obligations de contrat satisfaites, résultats inconnus explicitement traités ; un worker terminé n'est pas nécessairement une tâche acceptée.
- **Étape** : join mécanique satisfait, puis décision N+1 si le workflow la demande, sur la révision et les preuves courantes.
- **Run** : prédicat de done publié satisfait et aucune branche/action restante empêchant la clôture quiescente.
- **Mission** : critères du mandat satisfaits, livrables et rapport attribués, suites ou réserves explicites.
- **Goal** : critères définis par l'utilisateur et dépendances exigées satisfaits, décision de management tracée lorsque requise.
- **Produit V1** : les 19 features, les scénarios A–Z (A–O du brief et compléments P–Z) et les contraintes de distribution sont qualifiés.

Ces niveaux ne s'additionnent pas implicitement. Un workflow merge-only n'acquiert pas une phase tests/audit parce que les tests d'Unleash sont exigeants. L'autorisation d'exécuter un merge demeure une précondition indépendante de la définition métier de terminé.

## Catalogue global

[acceptance-scenarios](../testing/acceptance-scenarios.md) relie A–O aux exigences, complète P–Z et définit les preuves E2E. [integration-tests](../testing/integration-tests.md) décrit les séquences et injections concrètes. Chaque feature fournit son acceptance.md et tests.md, avec IDs stables AC-Fxxx-nnn, T-Fxxx-nnn et EV-Fxxx-nnn.

Une preuve acceptable contient : commit/build du produit, environnement/OS/filesystem, versions dépendances et fournisseur, configuration/permissions, fixture/seed, commandes de validation, observations indépendantes, état final et artefacts expurgés. L'absence d'une preuve requise vaut non qualifié, pas réussite par défaut.

## Garanties mécaniques à démontrer

L'état engagé et ses événements sont atomiques. Les duplications de transport ne doublent pas l'application d'une commande. Les effets externes ambigus ne sont pas rejoués aveuglément. Les unités actives — invocations agentes, workers de replay et probes fournisseur — respectent effective_limit et le plafond absolu 50. Les attentes parent/enfant ne retiennent pas artificiellement les slots. Les définitions engagées ne changent pas. Les versions tardives sont liées explicitement à des slots vierges compatibles. La validité des preuves porte sur leur source exacte.

La connaissance affiche provenance, fraîcheur et disponibilité. Les contrats vérifient un langage fermé de propriétés observables ; ils ne garantissent pas le sens de toute fonction. Les scopes sont appliqués par l'outil contrôlé ; une garantie de confinement (mode `strict`, Q-05) nécessite sa frontière OS qualifiée. L'inspection expose messages et payloads conservés, sans promettre la chaîne de pensée privée.

## Qualité visuelle et expérience

AC-F014-009 à 016 et AC-F015-009 à 017 sont des gates V1 : qualité GUI/TUI sur builds réels, parité complète, éditeurs intégrés, configuration et connecteurs, navigation/accessibilité, charge, restauration et reconnexion. La revue utilise les dix espaces, thèmes et profils terminal, données réalistes, états de panne et vue mission chargée. Un screenshot vide ou la seule liste des commandes ne suffit pas. Kivy et Tauri sont les références identifiées ; la pile acceptée, Tauri 2 + React/TypeScript ([ADR-03](../architecture/adr/ADR-03-rust-gui.md)), reste qualifiée par ces critères. La [matrice GUI/TUI](../design/gui-tui-parity.md) doit être intégralement satisfaite, y compris pour les fonctions avancées.

## Dérogations et inconnues

Une EV reste attendue (`planned` ; projection catalogue `not_run`) tant qu'aucun résultat n'existe ; un test exécuté est passé, échoué, bloqué ou non applicable avec justification. Les résultats des doubles, tests live et essais de panne machine sont distincts. Une dépendance AI-Guard non fournie bloque sa qualification réelle mais n'efface pas F017. Tout retrait d'invariant nécessite une décision explicite du porteur ; les limites documentées des garanties ne sont pas une autorisation de retirer la fonctionnalité.

## Automatisations et guidage

Les scénarios W/X/Y/Z qualifient respectivement le rejeu avec données nouvelles et dérive, la supervision bornée par blocs, les trois fournisseurs locaux et le parcours novice avec parité terminal. Les templates recommandés proposent leurs contrôles ; leurs résultats et limites sont visibles. Le mode sans modèle ne doit pas en appeler un pour faire un rapport, maintenir un index ou réparer silencieusement le Run. Les jobs de conception séparés et préautorisés conservent leurs propres coûts et preuves. Ces capacités font partie de la V1 complète ; une interface démonstrative ou un mock fournisseur ne les qualifie pas.
