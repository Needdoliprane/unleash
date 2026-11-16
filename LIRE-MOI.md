# OVPT Unleash — dossier d’ingénierie V1

**Révision consolidée R6 — 16 novembre 2026.** Produit OVPT destiné à l’open source.

## Ouvrir le dossier

Décompresser l’archive en conservant sa structure, puis ouvrir **[COMMENCER-ICI.html](COMMENCER-ICI.html)** dans un navigateur. Ce guide de lecture fonctionne hors ligne, sans script ni ressource distante. La [SYNTHESE.md](SYNTHESE.md) présente le raisonnement et les arbitrages. Le dossier de référence est [.claude/README.md](.claude/README.md), avec un [index intégral](.claude/INDEX.md).

Le répertoire .claude peut être masqué par le gestionnaire de fichiers. Activer l’affichage des fichiers cachés ou ouvrir les liens ci-dessus. Il s’agit du répertoire documentaire demandé : aucune dépendance d’exécution à Claude Code, aucun stockage opérationnel du futur produit.

## Ce qui est livré

| Élément | Contenu vérifié |
| --- | --- |
| Dossier .claude | 366 fichiers, tous présents dans le manifest |
| Périmètre | 19 features, 133 documents canoniques ; sept fichiers par feature |
| Traçabilité | 65 invariants, 239 exigences atomiques reliées à leurs AC/T/EV attendues |
| Décisions | 19 ADR, hypothèses et questions ouvertes identifiées |
| Revues R6 | 14 expertises réellement déléguées, 30 findings corrigés en conception et une contrelecture de remise distincte |
| Clients | 196 opérations documentées et 58 groupes de parité GUI/TUI |
| Formats | 22 schémas JSON, 27 fixtures, 2694 liens locaux contrôlés |

Les [rapports des experts](.claude/audits/r6/README.md) conservent les objections et les corrections. Le [rapport de validation](.claude/testing/document-validation.md) sépare les vérifications documentaires des tests produit à réaliser. Les 319 chemins de l’archive R5 sont conservés ; R6 ajoute 47 fichiers.

## Pour une équipe de réalisation

1. Lire [invariants](.claude/brief/invariants.md), [révision R6](.claude/brief/revision-2026-11-16-r6.md) et [verdict](.claude/audits/consolidated-verdict.md).
2. Examiner le [plan complet M0–M7](.claude/plans/v1-implementation-plan.md), le [chemin critique](.claude/plans/critical-path.md) et les [décisions restantes](.claude/brief/open-questions.md).
3. Pour la feature confiée, lire ses exigences, conception Rust, interfaces, dépendances, critères et tests avant de changer un contrat partagé.
4. Produire les preuves attendues dans la [matrice](.claude/testing/traceability.md). Une EV attendue ne devient pas acquise par déclaration d’un agent.

## Pour l’application qui utilisera le dossier

Le [contrat d’ingestion](.claude/specs/document-package-ingestion.md) et le [catalogue](.claude/catalog/README.md) définissent les formats, statuts et contrôles. Commencer par package-manifest.json et design-catalog.json ; vérifier inventaire, empreintes, schémas, IDs et arêtes exactes. Les opérations sont documentées, avec payload_schema_ref=null quand leur schéma runtime complet reste à produire en M0.

Une importation autorise lecture et préparation de travaux sous mandat. Elle ne lance pas les instructions de ce dossier et ne publie pas automatiquement de skills. L’application consommatrice n’ayant pas été fournie, sa compatibilité effective doit encore être testée sur son importeur.

## Limites explicites

Ce livrable est une conception, pas le code d’Unleash. Tous les tests du produit restent **not_run**. Aucun daemon, parcours GUI/TUI Unleash, fournisseur de modèle réel, achat, intégration OAuth privée ou AI-Guard n’a été qualifié dans cette mission. Les choix de framework, de licence, de frontière AI-Guard et de migration réservés à OVPT restent identifiés. Q-21, la récurrence calendaire autonome, demeure une option non confirmée.

La V1 conserve les usages développeur et généralistes, les modèles cloud/locaux, le replay sans modèle, la supervision par blocs, le guidage, la parité complète des interfaces et tous les composants initiaux. Les bibliothèques et moteurs existants ainsi que HTML/CSS sont permis ; le cœur applicatif et système reste Rust.

## Intégrité

[CHECKSUMS.sha256](CHECKSUMS.sha256) donne le SHA-256 de chaque fichier, sauf lui-même. Le manifest de .claude contrôle séparément son arbre documentaire. Ces empreintes détectent les différences d’octets ; elles ne sont pas une signature de l’auteur. Les helpers privés de rédaction et de vérification ne sont pas inclus.
