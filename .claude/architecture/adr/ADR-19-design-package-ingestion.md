# ADR-19 — Paquet de conception contrôlable par une application

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : cycle de statut des décisions, révision R7, périmètre d’export du paquet, outillage du dossier et emplacement des preuves fixés ; voir la section « Amendement R7 ». Proposé en R6 ; exécution par les responsables documentation, produit et QA. Demande confirmée : dossier `.claude/` exploitable par une application pour organiser la réalisation ; aucune application consommatrice existante n’a été fournie. Invariants INV-046/047/048/050 ; hypothèses H-02/H-07 ; risques R-22 (principal), R-06 et R-16 selon leurs périmètres.

## Contexte et options

Des fichiers Markdown seuls sont lisibles mais imposent à chaque consumer de redécouvrir IDs, statuts et relations. Les convertir directement en commandes runtime donnerait une autorité injustifiée à du texte de conception. Une base documentaire dédiée ou un serveur d’indexation ajouterait une dépendance inutile au livrable local.

Option retenue : conserver les sources canoniques Markdown, produire des projections JSON validées et un manifeste de fichiers/empreintes. Alternative : un seul document structuré canonique avec vues Markdown générées, au prix d’un changement complet du workflow rédactionnel. Alternative minimale : parser seulement la matrice d’exigences, insuffisant pour décisions/risques/parité.

## Décision et frontières

Adopter [le contrat d’ingestion](../../specs/document-package-ingestion.md) et les [catalogues](../../catalog/README.md). Statuts proposés/acceptés/confirmés/ouverts explicites ; preuves de produit not_run/attendues ; rapports de validation documentaire séparés. IDs et arêtes sont vérifiés bidirectionnellement, hashes sur octets et rejet des chemins dangereux. Un désaccord entre projection et source empêche l’import qualifié.

Le registre d’opérations permet d’inventorier et de contrôler les noms/propriétaires/parité. Il annonce honnêtement les payloads encore documentés plutôt que complètement schématisés pour génération de clients. Aucun import n’exécute exemples, instructions d’experts, commandes, outils ou publication de skills. Le produit demeure libre de prendre ce package comme source dans un workflow autorisé.

## Amendement R7

1. Les sources Markdown restent canoniques. `catalog/design-catalog.json`, `catalog/operations.json` et `testing/traceability.json` sont des projections régénérées et comparées dans les deux sens ; une divergence bloque l’import qualifié.
2. Le statut d’une décision suit `proposed → accepted → superseded`. « Accepté » signifie arrêté par l’autorité déléguée, avec date et référence ; ce n’est jamais « confirmé », statut réservé aux invariants. L’en-tête d’un ADR accepté porte « Statut : accepté … » et le catalogue `accepted` ; une question tranchée porte `resolved` ; les hypothèses restent `proposed`, donc réfutables.
3. Les projections et leurs schémas passent à `revision = "R7"`.
4. Le code vit dans ce même dépôt, à la racine, à côté de `.claude/`. Le profil d’ingestion s’applique à l’export du paquet de conception : `.claude/` et les fichiers racine de présentation et d’amorçage déclarés par le [contrat d’ingestion](../../specs/document-package-ingestion.md), dont `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.agents/` et `.codex/`. Le reste du dépôt de réalisation n’est ni importé ni motif de refus.
5. L’outillage de développement hébergé dans `.claude/` (`agents/dev/`, `dev/`, `hooks/`, `rules/`, `skills/` et `settings.json`, selon le [contrat d’ingestion](../../specs/document-package-ingestion.md)) est inventorié et haché, non normatif, et n’est jamais exécuté par un import (INV-046). Le manifest déclare `excluded_prefixes` (`state/`, `worktrees/`) et exclut `settings.local.json`.
6. Outils du dossier : `tools/design-package/regen.py`, `validate.py` et `sync.py`, en Python standard, outillage de développement hors produit ; pas de `xtask` pour le dossier. Le manifest et `CHECKSUMS.sha256` ne s’éditent jamais à la main.
7. Les preuves réelles vont dans `qualification/evidence/<EV-ID>.json` à la racine du dépôt, jamais dans `.claude/` ; `product_test_status` reste `not_run` dans le dossier.

## Conséquences et recette

Coût limité à la génération/validation de projections et à la gestion des versions. Aucune nouvelle feature runtime, aucun service distribué, aucune dépendance à Claude Code. La recette documentaire couvre paquet exact et mutations adverses ; une intégration dans l’application réelle exige son protocole et ses tests propres. Un hash interne ne signe pas l’identité de l’auteur. La publication du ZIP et sa somme globale permettent de vérifier le transfert, sans prétendre au fonctionnement d’un binaire inexistant.
