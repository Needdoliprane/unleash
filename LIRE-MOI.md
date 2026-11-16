# OVPT Unleash — dossier d’ingénierie V1

**Révision consolidée R7 — 16 novembre 2026.** Produit OVPT open source sous licence Apache-2.0. Les arbitrages sont tous rendus par délégation de Louis ; la réalisation de la V1 est ouverte dès le jalon M0.

## Ouvrir le dossier

Décompresser l’archive en conservant sa structure, puis ouvrir **[COMMENCER-ICI.html](COMMENCER-ICI.html)** dans un navigateur. Ce guide de lecture fonctionne hors ligne, sans script ni ressource distante. La [SYNTHESE.md](SYNTHESE.md) présente le raisonnement et les décisions. Le dossier de référence est [.claude/README.md](.claude/README.md), avec un [index intégral](.claude/INDEX.md). La [révision R7](.claude/brief/revision-2026-11-16-r7.md) consigne les arbitrages rendus et l’outillage ajouté.

Le répertoire .claude peut être masqué par le gestionnaire de fichiers. Activer l’affichage des fichiers cachés ou ouvrir les liens ci-dessus. Il s’agit du répertoire documentaire : aucune dépendance d’exécution à Claude Code, aucun stockage opérationnel du futur produit. Il héberge aussi l’outillage optionnel des agents de développement.

## Ce qui est livré

| Élément | Contenu vérifié |
| --- | --- |
| Dossier .claude | 389 fichiers inventoriés par le manifest, plus le manifest lui-même ; sont exclus state/, worktrees/ et settings.local.json |
| Périmètre | 19 features, sept documents canoniques chacune |
| Traçabilité | 65 invariants, 239 exigences atomiques reliées à leurs AC/T/EV attendues |
| Décisions | 21 questions résolues, 19 ADR acceptés ; Q-21 (récurrence calendaire) est écartée de la V1 |
| Audit R7 | 16 revues par domaine, chacune contre-vérifiée : 238 constats, dont 218 confirmés et corrigés dans les documents canoniques et 20 réfutés |
| Clients | 204 opérations documentées (dont 8 déjà citées par les contrats, ajoutées au registre en R7) et 58 groupes de parité GUI/TUI |
| Formats | 22 schémas JSON, 3 666 liens locaux contrôlés |
| Outillage des agents | AGENTS.md, CLAUDE.md, GEMINI.md ; .claude/dev/, .claude/rules/, .claude/agents/dev/ ; skill /goal ; hook git_guard.py ; .agents/ et .codex/ |

Les [rapports R6 des experts](.claude/audits/r6/README.md) restent l’historique de la révision précédente. Le [rapport de validation](.claude/testing/document-validation.md) sépare les vérifications documentaires des tests produit à réaliser.

## Démarrer avec un agent

1. Lire [AGENTS.md](AGENTS.md) : règles communes, ordre de lecture, interdits.
2. Lancer `/goal <objectif>` dans Claude Code ou Antigravity, ou `$unleash-goal <objectif>` dans Codex. L’agent suit le [protocole de goal](.claude/dev/goal-protocol.md) ; le [démarrage M0](.claude/dev/m0-kickoff.md) décrit le premier chantier.

Une EV attendue ne devient pas acquise par déclaration d’un agent ([matrice](.claude/testing/traceability.md)).

## Pour l’application qui utilisera le dossier

Le [contrat d’ingestion](.claude/specs/document-package-ingestion.md) et le [catalogue](.claude/catalog/README.md) définissent les formats, statuts et contrôles. Commencer par package-manifest.json et design-catalog.json ; vérifier inventaire, empreintes, schémas, IDs et arêtes exactes. Les opérations sont documentées, avec payload_schema_ref=null quand leur schéma runtime complet reste à produire en M0. Le profil d’ingestion s’applique à l’export du paquet de conception, non au dépôt de réalisation qui l’héberge.

Une importation autorise lecture et préparation de travaux sous mandat. Elle ne lance pas les instructions de ce dossier et ne publie pas automatiquement de skills. L’importeur de l’application consommatrice n’a pas été fourni : sa compatibilité effective doit encore être testée.

## Limites explicites

Ce livrable est une conception et un outillage de réalisation, pas le code d’Unleash. Tous les tests du produit restent **not_run**. Aucun daemon, parcours GUI/TUI Unleash, fournisseur de modèle réel, achat, intégration OAuth ou AI-Guard n’a été qualifié. Les décisions sont rendues ; restent des preuves à produire pendant la réalisation : EXT-GUARD, EXT-MODEL (avec des modèles intelligents, sur décision de Louis), EXT-MODEL-LOCAL, EXT-BROWSER, EXT-DOC, EXT-OS, EXT-OSS et EXT-DIST.

La V1 conserve les usages développeur et généralistes, les modèles cloud/locaux, le replay sans modèle, la supervision par blocs, le guidage, la parité complète des interfaces et tous les composants initiaux. Pile retenue : Tauri 2 avec React/TypeScript, Ratatui/Crossterm, rusqlite, fantoccini et Chrome for Testing ; le cœur applicatif et système reste Rust.

## Intégrité

[CHECKSUMS.sha256](CHECKSUMS.sha256) donne le SHA-256 de chaque fichier couvert : .claude/, les fichiers racine AGENTS.md, CLAUDE.md, GEMINI.md, LIRE-MOI.md, SYNTHESE.md et COMMENCER-ICI.html, ainsi que .agents/ et .codex/. Il ne couvre ni le code produit, ni .env. Le manifest de .claude contrôle séparément son arbre documentaire. Ces empreintes détectent les différences d’octets ; elles ne sont pas une signature de l’auteur. Elles se régénèrent avec `tools/design-package/regen.py` et se contrôlent avec `validate.py`.
