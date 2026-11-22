# Protocole `/goal` : développement autonome d'Unleash

**Statut : protocole de développement adopté en R7.** Il s'applique à Claude Code, Codex et Antigravity. Il organise les agents qui construisent Unleash ; il ne fait pas partie du produit et ne remplace pas le modèle N+2/N+1/N d'Unleash.

Un *goal* est un objectif de développement donné par l'utilisateur. L'**agent root** (la session principale qui reçoit le goal) le conduit en autonomie jusqu'à un résultat vérifié : il planifie, découpe, délègue à des sous-agents avec le modèle adapté, vérifie, intègre et rend compte. Il ne s'arrête que sur une condition d'arrêt listée plus bas.

## Déclencheurs

| Outil | Déclencheur | Mécanisme |
| --- | --- | --- |
| Claude Code | `/goal <objectif> [--no-merge] [--dry-run]` | Skill [`.claude/skills/goal`](../skills/goal/SKILL.md) |
| Antigravity | `/goal <objectif>` (mode goal natif) ou `/unleash-goal <objectif>` | Règles `AGENTS.md`/`GEMINI.md` et skill `.agents/skills/unleash-goal` |
| Codex | `$unleash-goal <objectif>`, ou un message commençant par `GOAL:` | Skill `.agents/skills/unleash-goal` et agents `.codex/agents/*.toml` |

Exemples : `/goal M0 : workspace Cargo et crates unleash-domain + unleash-protocol avec DTO IPC v2 testés contre les fixtures`, `/goal Corriger les échecs de cargo test sur unleash-store`.

## Mandat accordé par un goal

Un goal autorise, sans nouvelle confirmation : lire tout le dépôt ; créer branches et worktrees sous `.claude/worktrees/` ; modifier code, tests et dossier `.claude/` dans le périmètre du goal ; exécuter build, tests, linters et outils de validation locaux ; télécharger les dépendances Cargo du `Cargo.lock` et installer au niveau utilisateur les outils de développement listés dans `AGENTS.md` (`cargo install --locked cargo-deny`) ; gérer branches, commits et fusions ; fusionner la branche du goal dans `dev` quand toutes les gates passent (sauf `--no-merge`) et pousser `dev` (`git push origin dev`, jamais forcé) ; faire des appels payants de **test** uniquement avec les modèles de `UNLEASH_LIVE_TEST_MODEL_ALLOWLIST` (`claude-haiku-5-5`, `gpt-6-luna`), dans le plafond `UNLEASH_LIVE_TEST_BUDGET_USD`, avec les clés de dev de `.env`.

Un goal n'autorise jamais : push sur `main`, push forcé, tag, release ou publication ; appel payant à un modèle hors liste autorisée (aucun modèle intelligent : sol, astra, fable, opus, sonnet), OAuth réel, achat ou envoi de message ; affichage, copie ou commit d'un secret de `.env` ; modification d'un invariant `INV-…` ; suppression de l'historique (`reset --hard` sur `main`, `push --force`, réécriture de commits fusionnés) ; installation système (`sudo`, gestionnaires de paquets globaux) ; lecture ou écriture de secrets réels. Ces actions exigent une demande explicite de l'utilisateur dans la conversation.

## Cycle de vie

### 0. Préflight

1. Lire `AGENTS.md`, ce protocole, [le routage des modèles](model-routing.md), [la définition de terminé](definition-of-done.md) et [les worktrees](worktrees.md).
2. Dépôt Git : si le dossier n'en est pas un, `git init -b main` et `git config core.fileMode false` (le dépôt est sur un partage SMB qui marque tous les fichiers exécutables), vérifier `.gitignore` et `.gitattributes`, puis commit initial `chore: baseline du dossier de conception` après `python3 tools/design-package/validate.py .` sans erreur. Si la branche `dev` n'existe pas : la créer depuis `main`, y committer la base encore non suivie (dossier `.claude/`, outillage, configuration ; jamais `.env`), puis `git push -u origin dev`. Hors de ce cas, l'arbre de travail doit être propre ; sinon, consigner l'état et demander à l'utilisateur avant d'écraser quoi que ce soit.
3. Vérifier la toolchain de `rust-toolchain.toml` dans Docker suivant les [règles Rust](../rules/rust.md#exécution-locale-dans-docker).
4. Reprise : si `.claude/state/goals/` contient un goal `in_progress` correspondant, le reprendre depuis son journal au lieu d'en créer un nouveau.

### 1. Cadrage

Reformuler le goal en résultat mesurable. Le relier au jalon (M0–M7 du [plan](../plans/v1-implementation-plan.md)), aux features, exigences `REQ-…` et tests `T-…` concernés, et aux gates du [chemin critique](../plans/critical-path.md). Fixer les critères de terminé sous forme de commandes et d'observations attendues. Lister ce qui est hors périmètre. Si le goal contredit un invariant ou une décision adoptée, s'arrêter et poser la question.

Créer `.claude/state/goals/<goal-id>/` à partir du [modèle de journal](goal-journal-template.md) : `goal.md` (objectif, critères, statut) et `journal.md` (événements horodatés). `goal-id` = `g` + numéro sur quatre chiffres + slug court (`g0001-m0-workspace`). Créer la branche `goal/<goal-id>` depuis `dev`.

### 2. Plan et découpage

Découper en tâches indépendantes et vérifiables :

- une tâche = un objectif, un ensemble de fichiers ou de crates en **écriture exclusive**, ses exigences/tests, ses dépendances, son niveau de modèle, ses commandes de vérification et son critère de terminé ;
- taille cible : un diff relisible (≈ 400 lignes hors fixtures générées) ;
- les contrats partagés (`unleash-domain`, `unleash-protocol`, schémas, migrations) passent en premier, en série ; les tâches qui en dépendent attendent leur intégration ;
- deux tâches parallèles n'écrivent jamais les mêmes fichiers.

Écrire le plan dans `goal.md`. Pour un goal large ou ambigu, faire relire le plan par un agent de niveau `frontier` avant d'exécuter.

Pour chaque lot en cours, le plan nomme l'opération qui permet de l'accepter, sa dépendance immédiate et le responsable de la prochaine action. Séparer dès le découpage les parcours livrables dont les critères et les fichiers peuvent être figés séparément. Quand un lot dispose d'un candidat testable, prioriser sa revue et son intégration avant d'étendre ce lot.

### 3. Exécution déléguée

Pour chaque tâche prête, l'agent root choisit le rôle et le modèle selon le [routage](model-routing.md). Un agent écrivain travaille dans le worktree attribué selon la [règle des worktrees](../rules/worktrees.md). Un agent en lecture seule utilise le checkout qui porte le candidat figé. Le prompt de délégation est autonome : objectif, fichiers autorisés en écriture, documents à lire (feature, exigences, tests, contrats), critère de terminé, commandes de vérification, interdits, format du compte rendu.

Le mandat tient dans un message utilisable à la reprise. Il précise :

- **Résultat attendu** — une question précise pour une revue, ou le comportement
  observable d’un lot. Citer les identifiants REQ/T applicables et les dépendances
  disponibles.
- **Périmètre** — chemin du checkout, branche et SHA de départ, fichiers
  attribués, autres writers concernés, lecture seule si applicable.
- **Preuve** — commandes exactes, résultat attendu, emplacement des preuves
  et limites de la fixture.
- **Remise ou interruption** — SHA, changements en cours, commandes exécutées
  et résultat par critère, première action encore nécessaire.

Parallélisme : au plus 4 sous-agents écrivains simultanés, plus des sous-agents en lecture seule. Le root ne code pas lui-même une tâche déléguée ; il garde le contexte du goal.

Compte rendu attendu de chaque sous-agent : branche et commit, fichiers modifiés, commandes exécutées avec leur sortie réelle, exigences couvertes, écarts et risques. « Terminé » sans sortie de commande n'est pas un compte rendu.

#### Travaux de sécurité informatique

Cette règle s’applique au root comme aux sous-agents, y compris lors d’une
reprise ou d’une revue en lecture seule. Avant de déléguer, d’écrire le
scénario ou d’exécuter le contrôle, consigner les quatre éléments suivants
dans le mandat existant. Une
demande porte sur une seule propriété de sécurité. Si un élément manque ou
reste ambigu, suspendre cette exécution jusqu’à clarification du périmètre.

- **Finalité défensive** — propriété à vérifier ou défaut de code à corriger,
  raison pour laquelle ce contrôle est nécessaire et résultat observable.
- **Périmètre autorisé** — checkout et SHA, fichiers ou suite de tests,
  environnement, données et composants réellement démarrés. Décrire les
  écritures de la fixture et sa durée de vie. Privilégier la lecture du diff et les fixtures
  synthétiques locales du dépôt. Identifier expressément tout accès réseau
  nécessaire, sa destination et son autorisation avant l’appel.
- **Commande exacte** — répertoire de travail, exécutable, arguments, filtre
  de test et effets attendus en lecture ou en écriture. Employer la commande
  la plus étroite qui vérifie la propriété. Pour une lecture par outil,
  indiquer les fichiers ou plages de lignes. Avant toute commande qui change
  de données, de cible ou d’effet, actualiser le mandat et réexaminer le
  périmètre.
- **Critère et remise** — oracle de réussite ou de refus, preuve à retenir,
  résultat réellement obtenu et limites de la vérification. Pour une lecture,
  fixer la forme de la réponse dans le mandat. Demander un verdict confirmé,
  infirmé ou non vérifié, avec les références qui l’établissent. La remise se
  limite à la propriété demandée.

Les mandats couvrent la revue de code, les corrections défensives et leurs
tests de contrat. Ils excluent le développement d’exploits, la reproduction
d’intrusions et le contournement des contrôles de la plateforme. Si celle-ci
refuse ou masque un résultat, arrêter la vérification concernée. Inscrire
son état non vérifié et conserver seulement les résultats déjà disponibles.
La demande filtrée ne doit être ni réitérée, ni reformulée pour être relancée,
ni transférée à un autre agent ou outil. Les autres lots autorisés continuent.

### 4. Vérification indépendante

Une tâche n'est intégrée qu'après vérification par un **autre** agent (rôle `verifier`, contexte neuf). Il lit le diff contre les exigences et la [définition de terminé](definition-of-done.md), examine les assertions des tests et vérifie les sorties réelles des contrôles ciblés.

Une sortie est réutilisable si le vérificateur peut examiner le résultat, l'oracle et le rattachement aux mêmes entrées (sources, dépendances, configuration, commande et environnement pertinents). Si une de ces conditions manque, il exécute le contrôle ciblé autorisé ou rend le critère non vérifié. Il consigne la preuve acceptée ou la commande réellement relancée. Un contrôle masqué suit la [règle d’arrêt ci-dessus](#travaux-de-sécurité-informatique).

Une exécution des gates complètes peut couvrir plusieurs lots revus indépendamment sur la même composition. Un conflit ou une modification invalide les preuves qu'il affecte. Les gates complètes restent requises sur la composition finale du goal et après fusion sur `dev`.

Un résultat rouge retourne au responsable avec le défaut observé. Si la revue attendue est indisponible, ROOT fige le candidat et consigne le critère ouvert, sa cause, le responsable et la condition de reprise dans le suivi du lot. Il intègre en priorité les candidats déjà acceptés dont les dépendances sont satisfaites. Les contrôles de composition et les travaux indépendants continuent en parallèle si leurs entrées sont distinctes.

Escalade pour un désaccord de fond sur un contrôle autorisé : deux échecs au même niveau de modèle → faire arbitrer au niveau supérieur. Après trois échecs au niveau `frontier`, marquer la tâche `blocked` et consigner le diagnostic.

### 5. Intégration

Le rôle `integrator` compose les tâches revues dans `goal/<goal-id>` (`git merge --no-ff`). Il exécute les gates applicables sur cette composition avant acceptation, résout les conflits triviaux et renvoie les conflits sémantiques à l'implémenteur. Un changement purement documentaire passe les contrôles du dossier. Après intégration acceptée, clôturer les worktrees selon la règle du projet et supprimer les branches de tâche devenues inutiles.

Si un contrat a changé, le rôle `doc-keeper` propage le changement dans `.claude/` selon les [règles du dossier](../rules/design-dossier.md), lance `regen.py` puis `validate.py` ; le goal n'est pas terminé tant que le validateur n'est pas à zéro erreur.

### 6. Boucle et conditions d'arrêt

Répéter planification → exécution → vérification → intégration jusqu'à ce que tous les critères soient satisfaits. S'arrêter uniquement si :

- tous les critères de terminé sont observés (succès) ;
- une action hors mandat est nécessaire (push, compte payant, secret réel, décision d'invariant) ;
- une gate externe bloque (`EXT-*` de la [readiness checklist](../plans/readiness-checklist.md)) et aucune autre tâche n'avance ;
- le budget du goal est atteint : 40 délégations ou 6 cycles complets sans progrès mesurable.

Un blocage partiel n'arrête pas le goal : continuer les tâches indépendantes.

Quand une livraison stagne après un cycle, ROOT rapproche les candidats
testables, les lots acceptés, le dernier SHA intégré et le dernier SHA publié.
Pour chaque lot encore ouvert, il donne le prochain résultat qui permet de
l’accepter, l’action et son responsable. Il traite en priorité le premier
raccord manquant sur le chemin de livraison. Si une gate externe échoue,
il suit la qualification locale acquise et la preuve distante attendue.
En cas de correction de Louis, de preuve invalidée ou de blocage répété, il
applique le [skill `retex`](../skills/retex/SKILL.md). Le journal
cite l’effet mesuré de la correction et la leçon réutilisable, lorsqu’elle
existe. La fiche du goal reste la référence pour l’état des lots.

### 7. Clôture et rapport

| Étape | Skills |
| --- | --- |
| G7 | `retro`, [`retex`](../skills/retex/SKILL.md) pour un écart observé, puis `improve-codebase-architecture` lorsque le code le justifie. |

À la clôture d’un cycle, lire la grille de `retro` et l’appliquer aux preuves
et aux entrées déjà consignées dans la [base de retours d’expérience](retours-experience.md).
Comparer l’effet attendu et l’effort des changements de méthode, de
navigation ou d’outillage. Retenir les corrections dont l’effet peut être
vérifié. Appliquer `retex` aux écarts nouveaux. Pour un écart déjà traité,
reprendre la même entrée et vérifier l’effet de la correction. Utiliser
`improve-codebase-architecture` si les observations mettent en cause la
structure du code. Les deux skills amont sont fournis par les
[skills de Matt Pocock](https://github.com/mattpocock/skills).

1. Rejouer toutes les gates sur `goal/<goal-id>` ; consigner les sorties dans le journal.
2. Si tout est vert et que `--no-merge` n'est pas demandé : `git checkout dev && git merge --no-ff goal/<goal-id>`, rejouer les gates sur `dev`, puis `git push origin dev` et vérifier la CI GitHub Actions de `dev` (`gh run list --branch dev`) jusqu'au vert. `main` ne reçoit que les releases, sur demande de l'utilisateur.
3. Enregistrer les preuves réelles dans `qualification/evidence/` (une entrée par EV réellement exécutée).
4. Passer `goal.md` à `done` (ou `blocked`) avec : résultat, commits, commandes et sorties, exigences couvertes, ce qui reste, risques, prochain goal recommandé.
5. Rendre compte à l'utilisateur en quelques lignes, avec les chemins des branches et du journal.

## Goal de programme : toute la V1

Un goal peut viser la V1 complète (`/goal V1 …`). L'agent root le conduit alors comme un **programme** :

1. Créer le goal parent `g0000-v1` (`.claude/state/goals/g0000-v1/`) avec, pour critères, les gates de clôture V1 de la [readiness checklist](../plans/readiness-checklist.md) et du [plan](../plans/v1-implementation-plan.md) : M0 à M7 sortis, 19 features acceptées sur leurs preuves réelles, scénarios de recette passés, matrice de parité GUI/TUI couverte, artefacts de distribution prêts.
2. Enchaîner un **sous-goal par jalon** dans l'ordre du chemin critique (M0, M1, … M7), chacun avec ses propres critères, plan, journal, budget et rapport de sortie (`qualification/mN/exit.md`). Les travaux indépendants d'un jalon ultérieur peuvent avancer en parallèle quand leurs contrats sont figés (par exemple les écrans sur fixtures IPC dès M1).
3. Le budget d'arrêt s'applique **par sous-goal** ; le programme continue tant qu'un jalon peut progresser. Un jalon bloqué par une gate externe est clos `blocked` sur ses seules parties externes ; le programme passe au suivant et y revient quand la pièce arrive.
4. Gates externes : AI-Guard réel (EXT-GUARD), comptes API et coûts réels (EXT-MODEL), comptes Drive/OneDrive/Notion (EXT-DOC), adhésion Apple Developer et clés de signature (EXT-DIST), raison sociale et adresse de sécurité (EXT-OSS) exigent l'utilisateur ou OVPT. L'agent root construit et teste tout le reste contre des doubles fidèles aux contrats, prépare la qualification réelle (scripts, fixtures, procédures), puis liste précisément ce qui manque. Il ne présente jamais un double comme une qualification réelle.
5. À chaque sortie de jalon : rejouer toutes les gates sur `dev`, mettre à jour `goal.md` du programme, lancer une revue d'expert adaptée au jalon (au moins `expert-12-qa` et l'expert du domaine), corriger, puis continuer sans attendre de confirmation.
6. Fin du programme : V1 construite, gates internes vertes, rapport final listant les preuves réelles acquises, les gates externes restantes avec la pièce attendue et son propriétaire, et la procédure de release prête à exécuter dès que ces pièces sont fournies.

## Suivi des travaux, journal et reprise

Le journal est la mémoire du goal entre sessions et entre outils. Chaque délégation, vérification, intégration, escalade et décision y est ajoutée (jamais réécrite). Un autre outil peut reprendre un goal en lisant `goal.md` puis `journal.md`. `.claude/state/` est versionné dans Git mais exclu du manifest du dossier de conception.

ROOT tient à jour `.claude/state/current-state.md`, la vue générée des travaux en cours et à venir. Les fiches `goal.md` portent l’état et les prochaines actions. Le journal conserve les événements, SHA et preuves, y compris après retrait d’une branche. Pour chaque chantier ouvert, la section « Actions de clôture ouvertes » donne le prochain résultat observable, la dépendance ou le blocage et son propriétaire.

Après une affectation, une reprise, un changement de statut ou de dépendance, une validation, une intégration ou un retrait, mettre à jour les fiches et le journal concernés dans le même tour. Synchroniser aussi le plan V1 si le statut d’un jalon change, puis régénérer la vue.

```sh
python3 -B tools/design-package/current_state.py .
python3 -B tools/design-package/current_state.py . --check
```

Le responsable de l’intégration vérifie la vue et applique le [cycle de vie des branches, worktrees et caches](worktrees.md) après chaque lot. Une branche ou un worktree intégré sans consommateur est à retirer dès que les preuves et contenus uniques sont préservés. Documenter tout retrait restant ouvert avec le responsable et la condition de reprise.

## Rôles

| Rôle | Mission | Écrit | Niveau par défaut |
| --- | --- | --- | --- |
| `scout` | Cartographier code et dossier, inventaires, recherches | Rien | `fast` |
| `implementer` | Implémenter une tâche Rust standard avec ses tests | Son worktree | `standard` |
| `systems-implementer` | Persistance, reprise, IPC, scheduler, processus, sécurité, concurrence | Son worktree | `strong` |
| `ui-implementer` | GUI Tauri 2 + React/TypeScript (ADR-03) et TUI Ratatui/Crossterm, à parité | Son worktree | `standard` |
| `test-engineer` | Tests de contrat, propriétés, injection de pannes, fixtures | Son worktree | `standard` |
| `verifier` | Vérification indépendante d'une tâche ou du goal | Rien (exécute les tests) | `standard` ; `strong` pour le code système et les gates de jalon |
| `integrator` | Fusions, gates, nettoyage des worktrees | Branche du goal | `standard` |
| `doc-keeper` | Propagation dans `.claude/`, régénération, validation | Dossier `.claude/` | `standard` |
| `architect` | Arbitrage de contrat, amendement d'ADR, plan d'un goal large | Dossier `.claude/` | `frontier` |
| Experts `agents/*.md` | Revue d'un domaine (14 regards) sur demande | Rien | `standard` ; `strong` pour une gate de jalon |

Les définitions exécutables sont dans `.claude/agents/dev/` (Claude Code) et `.codex/agents/` (Codex) ; Antigravity applique les mêmes rôles via ce document.
