# ADR-08 — Connaissances partagées et discovery

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : stockage des index dérivés, recherche lexicale, niveaux syntaxique et sémantique, budgets et embeddings fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

La réutilisation entre worktrees exige disponibilité, fraîcheur et droits ; une recherche sémantique ne prouve pas équivalence.

Exigences : REQ-KNOW REQ-OBS. Risques/hypothèses/questions : R-07 R-08 ; H-08.

## Options et compromis

Option recommandée : **Métadonnées centrales et index spécialisés dérivés, snapshots/provenance**.

Alternatives considérées : Index monolithique universel ; uniquement branche mergée ; base tenue par agents.

L’hybride limite duplication tout en respectant langages ; branche cible seule oublie le travail en cours ; agents seuls oublient les mises à jour.

## Décision

Code/work/evidence reliés, indexation par événements/reconciliation, niveaux d’analyse multilangages, dirty à cohérence déclarée. DiscoveryReceipt enregistre recherches et décisions de création/réutilisation.

## Amendement R7

1. Trois plans (code, travail, preuve) sur des identités et un TypedSourceRef communs. Les métadonnées d’autorité (snapshots, générations d’index, intentions, dépendances, requêtes et reçus de discovery) vivent dans la base d’[ADR-01](ADR-01-sqlite-persistence.md) et passent par le writer commun.
2. Les projections dérivées (symboles, relations, recherche) vivent dans une base SQLite dérivée par génération, sous `data/index/`, construite hors transaction d’autorité puis immuable. Publier une génération bascule le pointeur actif après comparaison de la source, de la génération, du watermark et de l’ACL ; seules la génération active et la précédente sont conservées.
3. Recherche lexicale par SQLite FTS5 : tokenizer `trigram` pour le code, `unicode61` pour la prose. Le filtrage ACL précède classement, compteurs et extraits.
4. Niveau syntaxique : tree-sitter, dépendance native inventoriée (grammaires officielles Rust, Go, Java, Python et TypeScript/TSX, versions et licences figées) ; relations marquées `syntactic`.
5. Niveau sémantique : LSP 3.17 par un client JSON-RPC Rust sur stdio, processus supervisés par F011. Les analyseurs (rust-analyzer, gopls, Eclipse JDT LS, Pyright, typescript-language-server) sont détectés là où l’utilisateur les a installés ; installation assistée seulement sur commande explicite, aucun binaire distribué ni téléchargé implicitement (Q-13). Build scripts, proc-macros, import Maven/Gradle, acquisition de types et réseau des modules Go sont désactivés jusqu’à autorisation du projet.
6. Budgets par défaut, distincts du pool 5/50 : 4 serveurs de langage actifs par utilisateur, 2 GiB de RSS par serveur, threads d’indexation = max(1, cœurs/4). Un fichier de plus de 2 MiB ou binaire est inscrit au manifeste et exclu de l’analyse ; une capture de plus de 200 000 fichiers est annoncée partielle.
7. Embeddings : adaptateur facultatif, désactivé par défaut, activable par projet ; signal de classement seulement, interdit sous `forbidden`.
8. Captures dirty conservées 7 jours après remplacement, et tant qu’un reçu, une preuve ou un pin les référence ; leur suppression laisse un tombstone.

## Conséquences, complexité et impact V1

Faux positifs et stale restent possibles ; états et limites visibles. Embeddings sont un signal, pas règle de mutualisation. Coût d’indexation borné séparément des slots LLM.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Scénario D, rebase/suppression/crash, ACL avant snippets et build dirty concurrent.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [shared-knowledge](../../specs/shared-knowledge.md) et [schéma discovery](../../schemas/discovery-receipt.schema.json) ; architecture d’indexation : [knowledge-indexing](../knowledge-indexing.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

