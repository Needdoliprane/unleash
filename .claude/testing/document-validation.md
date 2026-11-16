# Rapport de validation documentaire

**Date : 16 novembre 2026. Révision consolidée R7. Périmètre : artefacts de conception et outillage du dossier uniquement.** Les contrôles de la section R7 ont été exécutés par `python3 tools/design-package/validate.py .` ; les campagnes de mutation et les chiffres des sections suivantes sont historiques R6, non rejoués en R7. Tous les tests du futur produit restent **not_run** ; aucune fiabilité de runtime, qualité visuelle de la future application, inférence réelle ou compatibilité avec un importeur non fourni n’est acquise par ces résultats.

## Contrôles R7

Le validateur `tools/design-package/validate.py` contrôle : inventaire, tailles, SHA-256 et digest du manifest ; `CHECKSUMS.sha256` ; JSON sans clé dupliquée et TOML ; validité Draft 2020-12 des 22 schémas et validation des fixtures et projections contre leurs schémas en registre local fermé ; liens et ancres locaux ; concordance du catalogue avec les sources Markdown (exigences, questions, ADR, invariants, risques, hypothèses, opérations, parité) ; identifiants croisés tous définis ; cohérence de révision. Les outils `sync.py` (projections), `index.py`, `regen.py` (manifest et checksums) et `validate.py` sont dans `tools/design-package/`.

| Mesure R7 | Valeur |
| --- | --- |
| Fichiers inventoriés par le manifest | 389, plus le manifest lui-même ; exclus : state/, worktrees/, settings.local.json |
| Features | 19, sept documents chacune |
| Exigences, invariants | 239 REQ, 65 INV |
| Opérations documentées | 204, dont 8 déjà citées par les contrats et ajoutées au registre en R7 |
| Groupes de parité GUI/TUI | 58 |
| Schémas JSON | 22 |
| Liens locaux contrôlés | 3 666 |
| Audit R7 | 16 revues par domaine, chacune contre-vérifiée : 238 constats, 218 confirmés et corrigés dans les documents canoniques, 20 réfutés |

`CHECKSUMS.sha256` couvre `.claude/`, les fichiers racine `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `LIRE-MOI.md`, `SYNTHESE.md` et `COMMENCER-ICI.html`, ainsi que `.agents/` et `.codex/`. Il ne couvre ni le code produit, ni `.env`. Le résultat du contrôle final de remise R7 est donné par le coordinateur après régénération du manifest et des checksums ; ce rapport n'en anticipe pas la valeur.

Ces contrôles portent sur des fichiers. Tous les tests du produit restent `not_run`.

## Résultat global du dossier stabilisé (état R6, historique)

| Contrôle | Résultat observé |
| --- | --- |
| Arborescence minimale demandée | Présente et enrichie ; aucun document vide ni marqueur de travail inachevé détecté |
| Features F001–F019 | 19 dossiers, 133 documents canoniques : sept fichiers par feature |
| Exigences et arêtes | 239 REQ uniques, comparées aux sources dans les deux sens ; chaînes REQ→AC→T→EV exactes |
| Invariants | 65 INV avec couverture et statut, composant, feature, acceptation et limites |
| Décisions | 19 ADR ; choix proposés distincts des contraintes confirmées |
| Expertises R6 | 14 instances spécialisées réellement déléguées, rapports distincts ; 30 findings corrigés en conception |
| Contrelecture de remise | R6-REL-01 corrigé et relu par un autre agent ; portée documentaire |
| Opérations communes | 196 opérations documentées (R6 ; 204 en R7), noms/propriétaires/références/parité vérifiés ; payloads runtime marqués documented |
| Parité GUI/TUI | 58 groupes PAR distincts avec relations aller/retour vers les opérations |
| Schémas JSON | 22 schémas contrôlés contre Draft 2020-12, références enregistrées localement |
| Fixtures JSON | 27 exemples validés ; types, références et formats déclarés contrôlés selon le profil ci-dessous |
| Blocs de documentation | 18 blocs JSON parsés, quatre blocs wire F001/F002/F004 validés ; un TOML parsé |
| Causalité d’exemple | Receipt correctement relié ; hash de payload F002 recalculé conforme |
| Liens locaux | 2694 références contrôlées, aucune cible locale manquante |
| Sources externes | 171 URL distinctes présentes ; pas de promesse de crawl final exhaustif |
| Inventaire | 366 fichiers dans .claude (R6 ; 389 en R7) : 311 Markdown, 54 JSON et un TOML |
| Manifest | Inventaire exact, tailles/SHA-256 et digest de l’arbre concordants ; le manifest exclut seulement lui-même |

Le [catalogue](../catalog/README.md), la [matrice lisible](traceability.md), sa [projection JSON](traceability.json) et les documents de features expriment les mêmes exigences. Les comparaisons ne se limitent plus à retrouver un identifiant quelque part : une chaîne permutée est refusée même si tous ses éléments existent. La présence d’un texte ou d’un lien n’établit pas que le comportement produit est déjà réalisé.

## Contre-épreuves réellement exécutées

### Paquet d’ingénierie R6 (historique, non rejoué en R7)

La campagne finale comporte **22 cas : un paquet exact accepté et 21 altérations rejetées**, avec résultat conforme pour chaque oracle et zéro anomalie dans le paquet réel. Les mutations de relations sont réempreintées avant validation : leur rejet démontre le contrôle des sources et des arêtes, pas seulement celui des hash.

- Retrait d’une exigence dans les deux projections, duplication, permutation des chaînes existantes et modification du texte normatif.
- Statut produit passed sans preuve, opération inventée, documentation promue en autorité d’exécution, clé JSON dupliquée.
- Octets modifiés, fichier surnuméraire non inventorié, document de feature manquant, chemin sortant et collision de casse.
- Références de schéma HTTPS, fichier, chemin relatif extérieur ou URN inconnue : refus avant résolution ; aucune tentative réseau observée par la sentinelle de cette campagne.
- Archives synthétiques portant chemin parent, entrée dupliquée, lien symbolique ou collision de casse : refus sans extraction.

La [contre-expertise E13](../audits/r6/expert-13-adversarial.md) a également exercé six mutations de références, dont formes transitives et dynamicRef, après correction du registre fermé. Elle n’a observé ni appel réseau ni lecture externe. Ces cas recouvrent en partie la campagne finale et ne sont pas additionnés comme six nouvelles preuves produit.

### Schémas de workflows et supervision conservés

La campagne R5 rejouée conserve **19 cas : quatre formes valides et quinze contre-exemples**, zéro divergence, plus quatre correspondances de fixtures. Elle vérifie notamment le modèle interdit dans les branches récursives, les budgets, les opérations versionnées et les contradictions d’observation. Les fixtures v1 et v1.1 restent valides ; le profil v1.2 est distinct.

Les [revues workflow](../audits/r6/expert-03-workflow.md), [knowledge](../audits/r6/expert-07-knowledge.md), [contrats](../audits/r6/expert-08-skills.md) et [QA](../audits/r6/expert-12-qa.md) conservent leurs mesures ciblées et leurs limites. E12 a vérifié en mémoire vingt mutations des projections, deux refus du générateur sans écriture et des dates positives/négatives. Ces essais de validation sont attribués aux audits correspondants ; ils ne prouvent ni scheduling, ni compilation sémantique Rust, ni résultat métier réel.

### Formats et portée du validateur

Le contrôle global refuse les clés JSON dupliquées et une référence de schéma hors registre local. Les formats présents dans les schémas sont date, date-time et UUID. Le profil temporel documentaire explicite est UTC : calendrier grégorien valide, forme YYYY-MM-DDTHH:MM:SS[.fraction]Z, secondes 00–59 et fraction optionnelle de 1–9 chiffres. Il n’affirme pas couvrir toutes les variantes RFC 3339 ; offsets et secondes intercalaires sont hors de ce profil de fixtures. L’activation du seul FormatChecker par défaut ne suffisait pas dans cet environnement : le contrôle temporel explicite a été ajouté puis exercé.

Une anomalie des sources, des projections ou des formats produit un statut d’échec non nul. Le générateur de matrice refuse les sources incohérentes avant écriture. Les instruments de rédaction/vérification utilisés pour ce dossier sont hors produit et ne sont pas embarqués comme une implémentation Unleash.

## Couverture, corrections et conservation

Les [65 invariants](invariant-coverage.md), les [sections du brief et de l’annexe](brief-coverage.md) et les [scénarios A–Z](acceptance-scenarios.md) sont conservés. Les révisions antérieures gardent leur historique ; R6 ajoute les corrections et le contrat d’ingestion sans réduire la V1. La comparaison avec l’archive R5 conserve ses **319 chemins documentaires** : aucun supprimé, **47 fichiers ajoutés**, pour 366 fichiers au total. Cette conservation ne suffit pas à prouver le sens des textes : les expertises et la [revue produit E14](../audits/r6/expert-14-product.md) vérifient le périmètre et les raccords.

Les [trente findings R6](../audits/r6/README.md) disposent d’un auteur, d’une correction et d’une référence canonique. Leurs références de tests existent dans les features et ont été recoupées avec leurs sections R6. La [contrelecture finale](../audits/r6/release-readthrough.md) corrige l’ordre lignée/idempotence d’un résumé du moteur et confirme sa concordance avec IPC v2. Corrigé en conception ne signifie pas test produit passé.

## Qualification encore nécessaire

La mission n’a construit ni compilé le workspace produit. Aucun daemon Unleash, crash de sa machine, benchmark, sandbox, PTY du produit, serveur LM Studio/Ollama/llama.cpp, compte API facturé, OAuth privé ou instance AI-Guard n’a été utilisé pour qualifier la V1. Les exemples d’automatisation sont synthétiques ; aucun achat ou parcours Carrefour n’a été exécuté. Le guide HTML de l’archive est statique ; sa structure, ses liens et l’absence de ressources distantes ont été contrôlés. L’inspection visuelle par navigateur headless n’a pas abouti : le lancement Chromium a été refusé par les restrictions de processus/sockets de cet environnement. Aucun contrôle visuel réussi n’est donc revendiqué, ni pour ce guide, ni pour le frontend Unleash.

L’importeur de l’application consommatrice réelle n’a pas été fourni. Le [contrat d’ingestion](../specs/document-package-ingestion.md) et les schémas permettent de construire et tester son adaptateur ; les validations locales du paquet ne sont pas une importation réussie dans cette application. Les payloads des opérations runtime restent documented et nécessitent les DTO complets de M0. L’import ne lance aucun agent, ne publie aucun skill et ne transforme aucune EV attendue en preuve acquise.

Les gates GUI, OS/distribution, navigateur, modèles locaux/distants, OAuth, AI-Guard et licence restent ceux de la [readiness checklist](../plans/readiness-checklist.md). Les décisions sont rendues : frontière de migration (Q-03) fixée au Run, Q-21 écartée de la V1. Restent des preuves à produire : EXT-GUARD, EXT-MODEL (avec des modèles intelligents, sur décision de Louis), EXT-MODEL-LOCAL, EXT-BROWSER, EXT-DOC, EXT-OS, EXT-OSS et EXT-DIST. Les essais runtime de M0–M7 produisent les preuves attendues.

## Contrôle de livraison

Le [manifest](../catalog/package-manifest.json) est régénéré par `regen.py` après stabilisation des textes, de l’INDEX et de ce rapport. Le paquet d’ingestion comprend le dossier .claude, AGENTS.md, CLAUDE.md, GEMINI.md, un guide HTML statique, un LIRE-MOI, une synthèse, `.agents/`, `.codex/` et des checksums ; le reste du dépôt est hors paquet. Sa recette compare l’ensemble des entrées et leurs octets aux sources, vérifie les CRC, les chemins et les checksums, puis revalide le dossier extrait en mémoire. Les fichiers privés de travail et helpers ne sont pas embarqués. Le résultat de ces contrôles d’empaquetage est annoncé seulement après leur exécution.
