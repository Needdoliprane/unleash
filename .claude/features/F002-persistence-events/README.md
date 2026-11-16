# F002 — Persistance, journal et événements

## Objectif

Rendre une commande reconnue durablement retrouvable après crash, avec son état, ses événements, ses réservations et les effets à poursuivre ou réconcilier. F002 fournit l'autorité transactionnelle locale. Elle n'étend pas une transaction SQLite à Git, au filesystem entier ou à une API distante et ne revendique pas exactly-once externe.

La base conserve les activités observées, décisions et versions. Git reste l'autorité du code ; les index de connaissance sont des vues dérivées. Les objets volumineux sont adressés par contenu hors base, avec protocole de préparation avant référence. Le journal est un historique transactionnel et non une promesse de reconstruction intégrale par event sourcing.

## Périmètre V1

Writer Rust propriétaire, SQLite local WAL qualifié, migrations, commandes idempotentes liées à leur lignée d'origine, tombstones de clés, événements, outbox/inbox, réservations atomiques, leases/fences, preuves, objets sous pins et collecte exclusive, sauvegardes et erreurs de stockage sont inclus. L'ensemble couvre tous les projets d'un utilisateur, sans Redis par défaut. Les données vivent dans les racines runtime, jamais dans `.claude/`.

Les [sources systèmes](../../research/systems-sources.md) identifient les versions observées et leurs limites. Le [schéma canonique](../../architecture/sqlite-design.md) définit tables et contraintes ; [storage-model.md](../../architecture/storage-model.md) définit les identités et autorités. Les fonctionnalités métiers conservent leurs propres contrats tout en utilisant cette transaction commune.

## Résultat attendu

Un client peut distinguer « commande committée », « effet en vol », « résultat inconnu », « ancienne lignée refusée » et « réponse purgée, clé conservée ». Un opérateur peut restaurer un snapshot contrôlé, inspecter les références manquantes et réconcilier le delta d'effets/coûts postérieurs sans répéter une commande client devenue ancienne. Une collecte ne détruit pas un objet entre sa préparation et la publication de sa référence. La fonctionnalité est spécifiée ici, non implémentée ; tous les paquets de preuve associés restent planifiés.
