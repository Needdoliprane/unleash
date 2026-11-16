# Audit 01 — Chief Software Architect

> **Addendum R2 — 16 novembre 2026.** Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). Bibliothèques existantes et HTML/CSS sont autorisés ; Kivy et Tauri sont identifiés ; la GUI et la TUI doivent avoir une parité fonctionnelle complète. Les choix actuels sont dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md). Les autres observations techniques restent à lire avec leurs limites initiales.

**Nature :** audit préalable du brief et de l'architecture candidate, 16 novembre 2026. **État :** aucun code audité ; conclusions de conception, vérifications à réaliser. Ce regard a été mené dans le groupe Git/connaissance, séparément des regards 06 et 07 ; il ne prétend pas représenter trois personnes humaines ni trois implémentations indépendantes. [Mission du regard](../agents/chief-architect.md).

## Verdict et hypothèses

Un daemon utilisateur unique avec noyau déterministe, sessions durables et exécutions courtes est cohérent avec le produit demandé. Le risque principal n'est pas l'absence de parallélisme ; c'est la confusion entre plusieurs autorités : décision métier d'un leader, transition autorisée par le harness, résultat observé d'un outil et état effectif de Git. La conception doit représenter leur désaccord au lieu de laisser le dernier message LLM écraser le reste.

Hypothèses proposées : usage local mono-utilisateur à plusieurs projets, processus du daemon doté de droits utilisateur, stockage local fiable sous réserve de pannes, intégration Git coordonnée mais coexistence possible avec des commandes externes. Aucun de ces choix ne fournit une isolation contre tous les programmes du même compte.

## Constats, objections et vérifications

| Réf. | Gravité | Constat ou objection | Recommandation et vérification attendue |
|---|---|---|---|
| ARC-01 | Critique | Si N+2, N+1 et harness planifient chacun les mêmes transitions, les retries produisent des missions doublées. | Les LLM proposent des commandes métier ; un seul noyau valide versions, droits et transitions. Rejouer deux livraisons d'un événement et obtenir une seule transition durable. |
| ARC-02 | Critique | Une transaction SQLite ne rend pas atomique un effet Git, une API et le journal. | Intention durable, identifiant d'action, observation distincte et état `outcome_unknown`. Injecter un crash après effet avant journal. [Source SQLite](https://www.sqlite.org/atomiccommit.html), consultée le 16 novembre 2026. |
| ARC-03 | Critique | « Worktree isolé » peut être lu comme « sandbox ». Cette interprétation est fausse dans la conception candidate. | Distinguer allocation de répertoire, scope de mandat et autorisation technique au broker ; déclarer le périmètre OS réellement disponible. [Source Git](https://git-scm.com/docs/git-worktree), consultée le 16 novembre 2026. |
| ARC-04 | Haute | Une copie de fichiers dirty parcourus successivement peut mélanger plusieurs états. Deux scans identiques ne prouvent pas un instantané atomique. | Provenance avec `capture_consistency` ; lecture de tree Git pour la voie cohérente. Cas hostile avec écriture pendant capture et retour ABA. |
| ARC-05 | Haute | Une contrainte de cinq « agents » peut limiter par erreur les sessions dormantes et non les appels coûteux. | Compter les invocations actives N, N+1 et N+2, limiter séparément CPU/processus d'analyse, ne pas réveiller un manager pour toute ligne de log. |
| ARC-06 | Haute | Le journal et l'index ne doivent pas avoir la même stratégie de purge. | Journal durable selon politique explicite ; projections reconstructibles et atomiquement remplaçables. Un rebuild ne recrée jamais un effet externe. |
| ARC-07 | Haute | Une ref cible checkout dans un autre worktree peut être modifiée sans que ses fichiers soient synchronisés. | Ref cible gérée non checkout pour CAS ; transfert explicite avant intégration autrement. Aucun reset du travail humain. Test cible occupée. |
| ARC-08 | Haute | « Toute dépendance Rust » contredit les choix possibles SQLite, Git système, parseurs C et GUI WebView. | Distinguer code produit Rust et dépendances natives inventoriées ; arbitrage public. [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) confirme C11, consultation du 16 novembre 2026. |
| ARC-09 | Haute | Le modèle peut annoncer un test réussi sur une ancienne génération. | Attacher preuve au snapshot, invocation, commande et workflow effectif ; afficher périmètre et obsolescence. Ne pas faire de l'audit une étape universelle. |
| ARC-10 | Haute | La base SQLite système n'est pas un choix de version contrôlé. | Vérifier version chargée, provenance et correctifs ; la [documentation WAL](https://www.sqlite.org/wal.html), consultée le 16 novembre 2026, signale le WAL-reset bug corrigé en 3.51.3 et certains backports. |
| ARC-11 | Moyenne | Une crate par substantif métier crée une fragmentation coûteuse sans mieux contrôler les effets. | Séparer d'abord domaine, application et adaptateurs ; découper les modules selon équipes et stabilité réelle. Interdire les dépendances du domaine vers le runtime et la base. |
| ARC-12 | Moyenne | Une télémétrie externe peut devenir une dépendance cachée de la reprise ou une fuite de contexte. | Reprise locale fondée sur journal, export facultatif et filtré. Panne de collecteur sans perte d'état métier. |

## Carte des composants proposée à la consolidation

Les noms de crates ci-dessous sont des **propositions internes**, pas des bibliothèques existantes ni un workspace déjà créé.

| Crate candidate | Responsabilité exclusive | Dépendances interdites |
|---|---|---|
| `unleash-domain` | Identités, états, invariants, contrats et décisions de transition pures | Git, HTTP, SQLite, GUI, appels LLM |
| `unleash-application` | Commandes, orchestration déterministe des opérations et ports | Prompts concurrents inventant un second moteur |
| `unleash-store` | Transactions, versions, journal, inbox/outbox et projections | Appels réseau ou processus lents dans la transaction |
| `unleash-daemon` | Vie du service, admission des invocations et supervision | Persistance implicite en RAM comme autorité |
| `unleash-broker` | Effets contrôlés, capacités, vérifications et traces | Délégation de sécurité à une simple chaîne de texte |
| `unleash-git` | Inventaire worktrees, objets, préparation, CAS et réconciliation | Arbitre métier automatique de conflits |
| `unleash-knowledge` | Captures, index dérivés, recherche et graphe du travail | Publication Git ou remplacement du jugement N+1 |
| `unleash-language-rust` | Adaptation LSP, générations et confiance des symboles | Liaison directe aux API HIR internes présumées stables |
| `unleash-evidence` | Contrats observation/claim/judgment, rapports et export | Statut « succès » fabriqué depuis une phrase LLM |
| `unleash-api` | DTO versionnés communs GUI, CLI, TUI et agents | Commandes métier distinctes selon le client |

Le domaine peut garder connaissance, Git et preuve dans des modules jusqu'à ce qu'une séparation de crate apporte une frontière vérifiable. Le stockage demeure un port ; tous les écrivains métier passent par le writer. Les travaux CPU et processus enfants sont bornés et placés hors transactions. Le broker reçoit également les processus auxiliaires : rust-analyzer et Git peuvent déclencher d'autres effets selon configuration.

## Alternatives réellement considérées

**Processus monolithique avec workers threads :** plus simple au début, mais un plantage d'outil compromet le service et complique la révocation. **Microservices locaux :** frontières plus visibles, mais protocoles, versions et reprise multipliés sans besoin multi-hôte établi. **Daemon unique et auxiliaires supervisés :** recommandé, avec frontière d'effet et journal explicites. Ce choix reste compatible avec plusieurs équipes Rust et ne suppose ni Redis ni réseau entre machines.

L'embedding immédiat du moteur sémantique rust-analyzer est déconseillé : les [frontières internes documentées](https://rust-analyzer.github.io/book/contributing/architecture.html), consultées le 16 novembre 2026, ne garantissent pas la stabilité de HIR. LSP facilite la qualification d'une version et un redémarrage contrôlé, mais ne promet pas un graphe exhaustif du programme.

## Désaccords conservés et critères de décision

Le chef architecte préfère un graphe de dépendances de travail conservateur, mais refuse d'imposer une mutualisation de helpers : les différences métier peuvent rendre la copie justifiée. Il préfère une publication Git contrôlée, tout en signalant la friction réelle pour un dépôt dont `main` est checkout par Louis. L'interface doit montrer cette contrainte à l'adoption, pas la découvrir après une heure de travail.

Critères de sortie proposés : chaîne d'autorité sans boucle, chaque effet réconciliable, chaque source identifiée, toutes les exigences A–O reliées à des scénarios, traces discovery avant création, tests de panne définis, décisions de dépendances natives visibles, aucun « passé » attribué à un test non exécuté. L'annexe confirme les projets Python/TypeScript/Java/Go/Rust et multilangages en V1, avec outils externes ; les adaptateurs demeurent du code Rust interne. Restent à confirmer : volumes cibles, profondeur d'analyse par langue, mode d'adoption Git, politique de rétention et contrat AI-Guard. Ces questions bornent les décisions, sans bloquer la rédaction de V1.
