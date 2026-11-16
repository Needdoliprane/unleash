# Stratégie de vérification du produit

**Statut du dossier : spécification de tests, aucun test produit exécuté.** Les vérifications documentaires éventuellement réalisées sur ces fichiers ne prouvent pas les comportements d'Unleash. Les tests du cœur et utilitaires système développés pour le produit seront Rust. Les moteurs de tests et d’automatisation UI existants peuvent être réutilisés ; leur configuration ou leur fine couche de présentation suit la [politique R2](../brief/implementation-policy.md), sans recréer leur moteur pour imposer Rust. Les outils externes de projets Python/TypeScript/Java/Go/Rust/multilangages restent dans la matrice V1.

## Principe de preuve

Chaque exigence possède un critère Given/When/Then, un test et un paquet de preuve. Un EV reste attendu (`planned` ; projection catalogue : evidence_expected et product_test_status=`not_run`) tant qu'aucun résultat n'existe ; un test exécuté donne passed, failed, blocked ou not_applicable motivé. Une preuve réelle est consignée dans `qualification/evidence/<EV-ID>.json` à la racine du dépôt, jamais dans `.claude/`. Une sortie « OK » d'un agent est un claim. Une action de test observée, reliée à une commande exacte, un snapshot et un exit status est une observation. La décision de clôture est un jugement distinct selon le workflow effectif.

Cette stratégie concerne l'ingénierie d'Unleash. Elle n'impose pas une étape tests/audit dans tous les workflows utilisateur. Une definition of done Dev→Merge doit fonctionner si elle est autorisée et valide ; le harness n'y ajoute pas un juge LLM pour satisfaire ses propres tests de produit.

## Niveaux de validation

| Niveau | Cible | Oracle et limites |
|---|---|---|
| Domaine pur | Transitions, contrats, budgets, versions, causalité | État attendu et invariants après chaque préfixe ; aucune I/O. |
| Propriétés génératives | Suites de commandes, retries, annulations et forks | Contre-exemple minimal conservé ; succès fini non preuve universelle. |
| Concurrence instrumentée | Petites primitives partagées et admission modèle | [Loom](https://docs.rs/loom/latest/loom/) sur types instrumentés ; ne couvre pas automatiquement OS/SQLite. |
| Stockage réel | Contraintes, WAL, migrations, outbox/inbox | Base SQLite qualifiée, état SQL et journal ; environnement de durabilité déclaré. |
| Intégration processus | Hosts, PTY, signaux, ports, arrêt/reprise | Fixtures Rust et inventaire OS ; macOS et Linux réellement exercés. |
| Adaptateurs | APIs modèles, Git, connecteurs et sécurité | Fixtures contractuelles puis intégration réelle versionnée ; mock ≠ compatibilité réelle. |
| Clients | GUI/CLI/TUI et reconnexion | Même backend/watermark, actions observées et erreurs visibles. |
| Pannes/charge | Cutpoints, stockage dégradé, files, équité | Effets externes comptés indépendamment, limites et états sûrs. |

[Proptest](https://proptest-rs.github.io/proptest/intro.html) est candidat pour génération/réduction ; les graines et cas de régression sont persistés. Les outils et versions réellement résolus appartiennent au manifeste de build. Une assertion testant seulement le détail privé d'une méthode n'est pas préférée à une propriété métier observable.

## Testkit proposé

`unleash-testkit` fournit horloge UTC/monotone injectée, générateur d'IDs déterministe, provider HTTP de fixture, broker espion, filesystem/failpoints qualifiés et petits binaires processus. Les fixtures peuvent : attendre une barrière avant effet, augmenter un compteur externe, perdre un receipt, produire une sortie volumineuse, occuper un port, créer un descendant, changer de groupe et s'arrêter sur signal. Elles n'utilisent pas d'API modèle réelle par défaut et ne contiennent aucun secret utilisateur.

Le provider de fixture sépare acceptation réseau, démarrage du calcul, effet/compteur, réponse et facturation. Il permet de montrer qu'une fermeture de socket n'annule pas nécessairement l'effet. Les adaptateurs de réconciliation déclarent leurs capacités au test ; unknown est un résultat attendu lorsque la preuve n'existe pas.

Les tests utilisent des barrières contrôlées plutôt que des sleeps supposés suffisants. Les temps de scheduling sont injectés pour les propriétés et mesurés sur la plateforme pour l'intégration. Une panne processus, une interruption de VM et une simulation de mauvais filesystem restent trois catégories distinctes.

## Matrice de plateformes et dépendances

Le build publie toolchain, Cargo.lock, features, target triple, OS/kernel, architecture CPU, SQLite version/source_id/options et dépendances natives. La documentation consultée n'est pas substituée à ce manifeste. Les cibles sont celles de la matrice qualifiée d'[ADR-14](../architecture/adr/ADR-14-local-data-oss.md) (Q-11) : macOS 15, 26 et 27 sur Apple Silicon (minimum 15.0), qualifiés avec fermeture GUI, login/logout, veille et restart launchd ; Ubuntu 24.04 et 26.04 LTS x86_64, qualifiés sous service utilisateur systemd et en mode foreground, en indiquant les limites du gestionnaire présent. Intel, Windows et les autres distributions sont hors matrice qualifiée.

Les profils de repositories comprennent des commandes représentatives de Python, TypeScript, Java, Go, Rust et combinaison de langages. Les contrats déclarent versions d'outils, réseau éventuel, timeout, cwd et sortie. Un outil absent produit ToolUnavailable ; le test reste bloqué pour ce profil et ne devient pas succès par omission.

## Gates de livraison proposés

Une première gate vérifie syntaxe/types, lints pertinents et domaine, ainsi que le test de couverture de parité généré depuis `catalog/operations.json`. Une deuxième vérifie stockage et commandes contre une vraie DB. La troisième couvre processus/PTY et interfaces sur OS. La quatrième exécute la campagne de pannes critique et les scénarios A–O avec leurs compléments P–Z, dont W/X/Y/Z avec persistance et broker réels. Avant publication, la distribution ajoute Gatekeeper et notarisation, vérification des signatures et des SHA-256, cycle de mise à jour signée et désinstallation sans perte de données (Q-16). Les commandes telles que `cargo test --workspace --locked` seront valides une fois le workspace implémenté ; elles ne sont pas annoncées comme exécutées ici.

La release exige une restauration démontrée, la qualification du build SQLite, aucun dépassement d'admission par défaut ou rôle, aucun retry automatique d'effet ambigu et aucune divergence d'état entre clients. Les performances ont un jeu de données et une machine de référence ; les seuils sont des objectifs avant mesure, pas des résultats anticipés. Les tests de sécurité portent sur scopes, tokens, redaction, prompt injection dans outputs et ancien fence, avec un modèle de menace explicite plutôt qu'une prétendue sandbox totale.

## Paquet de preuve et invalidation

Chaque EV rassemble ID d'exigence/AC/test, commit produit et fixture, environnement, configuration, seed/cutpoint, dates, commandes normalisées, artefacts/logs hashes, résultat attendu/observé et liens vers anomalies. Les secrets sont remplacés par données synthétiques. Une modification du contrat, du schéma ou de l'implémentation concernée invalide les preuves anciennes selon leur portée ; leur historique est conservé.

La synthèse distingue confirmé par test, supposé par conception et à confirmer. Les tests non exécutés restent visibles avec la pièce nécessaire, notamment OS absent, credentials de sandbox non disponibles ou contrat AI-Guard non fourni. L'absence de preuve n'est pas compensée par une revue LLM supplémentaire.

## Campagnes ajoutées pour R3–R5

Le catalogue A–Z préserve les scénarios initiaux et ajoute automatisations, supervision, modèles locaux et guidage. [automation-replay-tests](automation-replay-tests.md) sépare forme du plan, résultat métier, effet et absence d’inférence. [local-model-tests](local-model-tests.md) distingue HTTP de fixture, serveur réel et capacités du modèle effectivement installé. Les tests d’interface comparent les 58 groupes de parité avec les mêmes droits et receipts.

Une collecte de traces navigateur (WebDriver, captures, journaux) n’est pas suffisante : les assertions de résultat sont conservées séparément avec leur source. Le test de supervision fait mentir volontairement logs et modèle pour vérifier les contrôles que le harness prétend garantir. Les tests de compréhension du guidage évaluent des utilisateurs non développeurs ; leurs seuils sont fixés par Q-20 avant l’essai et aucun résultat n’est annoncé avant mesure. Les outils d’automatisation existants sont réutilisables sous la politique de langage sans déplacer la logique de décision métier hors du cœur Rust.
