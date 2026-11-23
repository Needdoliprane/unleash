---
paths:
  - "**/*.rs"
  - "**/tests/**"
  - "**/benches/**"
  - "**/fixtures/**"
---

# Règles de test et de preuve

Ces règles appliquent la [stratégie de vérification](../testing/test-strategy.md) et la [traçabilité](../testing/traceability.md) au code.

- **Tests d'abord pour un contrat.** Avant d'implémenter une exigence `REQ-…`, écrire le test `T-F…` correspondant à partir de `features/<F>/tests.md` et `acceptance.md`, le voir échouer, puis implémenter.
- **Nommer la preuve.** Un test qui réalise un test du dossier porte son identifiant dans le nom ou dans un commentaire de tête : `// T-F002-004 / AC-F002-004 / REQ-PERSIST-004`.
- **Cas négatifs obligatoires.** Chaque exigence a au moins un cas positif et ses rejets significatifs : entrée invalide, état concurrent, panne au point d'injection, issue inconnue.
- **Pannes déterministes.** La reprise se teste avec les points d'injection de `unleash-testkit` (avant/après commit, après dispatch, avant observation) ; jamais avec des `sleep` ou des kills aléatoires non rejouables. Graine affichée en cas d'échec.
- **Isolation.** Répertoire temporaire par test, sur disque local ; ports éphémères ; aucune dépendance à l'ordre des tests ; aucun réseau sauf tests explicitement marqués `#[ignore]` et documentés comme tests d'intégration réels.
- **Pas d'effet réel par défaut.** Aucun appel facturé à un modèle, OAuth réel, achat, envoi de courriel ou push Git dans la suite par défaut. Les adaptateurs réels sont qualifiés dans des campagnes séparées.
- **Tests live payants pendant le développement.** Marqués `#[ignore]` et exécutés seulement avec `UNLEASH_LIVE_TESTS=1` (`cargo test -- --ignored live_`). Le testkit charge `.env` (`dotenvy`), lit les clés `UNLEASH_DEV_ANTHROPIC_API_KEY` et `UNLEASH_DEV_OPENAI_API_KEY`, refuse tout modèle absent de `UNLEASH_LIVE_TEST_MODEL_ALLOWLIST` (`claude-haiku-5-5`, `gpt-6-luna`) et stoppe la campagne au-delà de `UNLEASH_LIVE_TEST_BUDGET_USD`. Aucun modèle intelligent (sol, astra, fable, opus, sonnet) n'est appelé en test ; jamais de test live en CI ; jamais de clé dans un log, une fixture ou un commit.
- **Fixtures du dossier.** Les DTO et workflows sont testés contre `.claude/examples/*.json` et `.claude/schemas/*.json` ; un écart est corrigé dans le code ou, s'il révèle une erreur de contrat, dans le dossier (voir [règles du dossier](design-dossier.md)).
- **Preuves acquises hors du dossier.** Les fichiers du dossier gardent `product_test_status = not_run`. Une exécution réelle produit un enregistrement dans `qualification/evidence/<EV-ID>.json` à la racine du dépôt : commit, commande exacte, environnement, résultat, date, artefacts. Les rapports d'expériences M0 vont dans `qualification/m0/`. Une EV n'est jamais déclarée acquise sans cet enregistrement et sa sortie réelle.
- **Pas de faux vert.** Un test désactivé, ignoré ou affaibli pour faire passer une tâche est un échec de la tâche. Le rapporter avec la raison.
- **Révision exécutée.** Dans un banc Docker Cargo réutilisé, extraire le candidat avec `tar -m -xf -` pour actualiser les dates. Vérifier les empreintes des sources pertinentes et observer leur recompilation ou l’identité du binaire déjà qualifié. Le résultat cite la révision exécutée, le filtre et le statut de sortie. Voir [RETEX-002](../dev/retours-experience.md#retex-002--rejouer-le-bon-binaire).
- **Reprise d’un banc Docker.** Retrouver la session et le conteneur utilisés. Borner chaque opération et vérifier dans l’ordre la réponse de l’API, la sortie d’une commande simple dans le conteneur, puis la création et le retrait d’un fichier dans les chemins du compilateur et du target Cargo. Reprendre le transfert ou Cargo au premier point interrompu, après vérification de ses entrées. En cas d’expiration, conserver les résultats terminés et noter le premier prérequis en défaut. Réconcilier les effets du transfert avant une nouvelle tentative. Voir [RETEX-004](../dev/retours-experience.md#retex-004--vérifier-le-banc-avant-de-recompiler).
  Pour un banc alternatif sur macOS, exécuter d’abord un prévol borné. Si le binaire expire avant sa première sortie, corréler son PID et l’heure de lancement aux journaux macOS avant de changer de volume ou de préparer l’invité. Traiter une demande d’autorisation par le circuit normal de macOS.
- **Disponibilité d’un banc lancé par le test.** Sonder l’opération utile avec un délai court et une échéance de démarrage distincte. Tant que le processus reste vivant dans cette échéance, reprendre la sonde sur le même processus. Après un arrêt observé, relire les effets déjà engagés avant de renouveler une mutation. Voir [la reprise de la fixture Ollama](../state/goals/g0003-m2/journal.md#reprise-de-la-fixture-ollama).
- **Workflow GitHub Actions.** Passer `actionlint` sur chaque workflow modifié avant intégration. Consigner la révision et le résultat de ce contrôle. Voir [RETEX-005](../dev/retours-experience.md#retex-005--valider-les-expressions-du-workflow-avant-intégration).
