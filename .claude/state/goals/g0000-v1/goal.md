# g0000-v1 — Programme V1 complète d'OVPT Unleash

- Statut : in_progress
- Demandé : 2026-11-17T00:32:48+01:00 par l'utilisateur ; outil : Antigravity
- Branche : dev ; base : main@a8a4379a027b31aed4ff755b4d2175096bc8ede7
- Options : merge=oui, dry-run=non

## Objectif reformulé
Réaliser et qualifier la V1 complète distribuable et open source (Apache-2.0) d'OVPT Unleash sur la branche `dev` (poussée sur `origin` sans toucher `main`), en enchaînant les jalons M0 à M7 dans l'ordre du chemin critique : noyau durable Rust, 19 features (F001–F019), 239 exigences reliées à leurs tests `T-…` et preuves réelles `qualification/evidence/EV-….json`, parité GUI (Tauri 2 + React/TS) / TUI (Ratatui/Crossterm) / CLI sur les 58 groupes `PAR-…`, campagnes de pannes, sécurité et préparation de distribution/packaging.

## Rattachement
Jalons : M0 → M7 — Features : F001–F019 — Exigences : 239 REQ — Invariants : INV-001..INV-065 — Gates : GATE-ARCH, GATE-RUST, GATE-UI, GATE-AUTH, GATE-AUTOMATION, GATE-LOCAL, GATE-GUIDED, EXT-*

### Exécution Rust sur le poste

Exécuter Cargo, rustc, rustfmt, Clippy, rustdoc et les binaires Rust de test exclusivement dans Docker pour les travaux locaux du programme. Confier les validations natives Darwin à un runner macOS dédié. Chaque résultat indique la plateforme réellement utilisée.

### Conduite vers la V1 commercialisable

Enchaîner les jalons du plan et qualifier chaque rapport de sortie avant de déclarer le jalon clos. Poursuivre les travaux indépendants lorsqu’une gate externe reste ouverte. La préparation à la distribution comprend les paquets macOS et Ubuntu, l’installation, la mise à jour, la restauration et les contrôles de sécurité du plan de recette. Suivre les pièces externes de signature et de release jusqu’à leur qualification.

Appliquer les [skills de Matt Pocock](https://github.com/mattpocock/skills) à chaque incrément concerné. Utiliser `tdd` pour vérifier le comportement par l’interface d’acceptation avec un cycle rouge puis vert. Consulter `codebase-design` pour les décisions d’interface. Utiliser `code-review` pour obtenir une revue indépendante de la spécification et des standards sur le candidat figé. Consigner le résultat des tests et le SHA examiné dans le journal du jalon.

À [G7](../../../dev/goal-protocol.md#7-clôture-et-rapport), combiner `retro` et `retex` en utilisant les preuves du jalon et leurs entrées de retour d’expérience.

## Critères de terminé
- [ ] Jalons M0 à M7 sortis avec rapports `qualification/m0/exit.md` .. `qualification/m7/exit.md`
- [ ] 19 features (F001–F019) implémentées (tests `T-F…` écrits avant implémentation) et acceptées sur preuves réelles dans `qualification/evidence/`
- [ ] Scénarios de recette A–O, T, U, W, X, Y, Z passés sur doubles/runtime local (limites externes tracées sans dissimulation)
- [ ] Matrice de parité GUI/TUI/CLI (58 groupes `PAR-…`) vérifiée à 100 % y compris par test CI généré depuis `.claude/catalog/operations.json`
- [ ] Packaging macOS/Ubuntu, `unleash-update`, SBOM et conformité Apache-2.0 (`LICENSE`, `NOTICE`, `cargo deny check`) prêts
- [ ] Gates `cargo fmt`, `cargo clippy -D warnings`, `cargo test`, `cargo doc -D warnings`, `cargo deny check`, `gui-renderer` et `validate.py` (`0 errors`) vertes sur `dev` poussée sur `origin`
- [ ] Inventaire exact des pièces externes attendues (`EXT-GUARD`, `EXT-MODEL`, `EXT-MODEL-LOCAL`, `EXT-BROWSER`, `EXT-DOC`, `EXT-OS`, `EXT-OSS`, `EXT-DIST`, `gh auth`) consigné avec procédure de release prête

## Hors périmètre (sans secrets / pièces externes non fournies)
- Signature et notarisation Apple réelle (`EXT-DIST`), enregistrements OAuth Google/Microsoft officiels (`EXT-DOC`), comptes Drive/OneDrive/Notion réels (`EXT-DOC`), service AI-Guard réel (`EXT-GUARD`), qualification des profils modèles intelligents (`gpt-6.1-sol`, `claude-fable-5-2`) et push/tag/release sur `main`.

## Décision de Louis du 24 novembre 2026 : la partie OS est pour « demain »

La mise en open source se fait en laissant chacun construire ; la qualification propre à chaque OS (runner macOS dédié, essais natifs, matrice `EXT-OS`, paquets qualifiés, signature) est reportée. Les critères de terminé ci-dessus qui en dépendent restent dus et sont reportés, jamais comptés comme remplis. Cibles finales annoncées : macOS Apple Silicon, Linux, Windows ; macOS Intel exclu. Windows n'est pas dans la matrice R7 (Q-11) : à amender quand le travail sur les OS reprend. ROOT peut déployer plus de sous-agents selon les besoins. Détail et interprétation dans le [journal](journal.md#décision-de-louis-du-24-novembre-22-h-heure-locale--la-partie-os-est-pour-demain).

## Actions attendues de l'utilisateur
1. **GitHub Actions**. Fournir un accès en lecture aux exécutions de `dev@b7c73bb` ou le résultat primaire de l’exécution avec ID, SHA et conclusions des jobs. L’accès courant à `gh` renvoie HTTP 401 sur le compte et HTTP 404 sur Actions.
2. **EXT-GUARD** : fournir contrat/endpoint réel AI-Guard et identifiants de test.
3. **EXT-MODEL** : fournir clés OVPT de qualification finale pour `gpt-6.1-sol` et `claude-fable-5-2` (hors campagne dev `claude-haiku-5-5` / `gpt-6-luna`).
4. **EXT-MODEL-LOCAL** : confirmer disponibilité des serveurs/poids locaux LM Studio, Ollama et llama.cpp sur machine de référence pour Q-19 (Qwen3-8B instruct Q4_K_M + embedding).
5. **EXT-BROWSER** : confirmer mise en place du couple Chrome for Testing + chromedriver épinglé (Q-18) si téléchargement réseau souhaité pour recette réelle.
6. **EXT-DOC** : fournir comptes de test et client IDs OAuth Google Drive / OneDrive + jeton Notion de test.
7. **EXT-DIST / EXT-OSS** : fournir adhésion/certificat Developer ID OVPT, clés de signature updater, raison sociale, adresse email de sécurité et antériorité de marque avant publication publique / release sur `main`.

## Plan du programme (Sous-goals)
| Sous-goal | Jalon | Objectif | Rapport de sortie | Statut |
| --- | --- | --- | --- | --- |
| g0001-m0 | M0 | Workspace Cargo, contrats partagés (domain, protocol, testkit) & 5 spikes (SQLite, UI, Runtime, Contrats/Connaissance, Navigateur) | `qualification/m0/exit.md` | done |
| g0002-m1 | M1 | Cœur durable & autorité locale : F001, F002 complet, F017 broker interne, squelette F018 | `qualification/m1/exit.md` | in_progress |
| g0003-m2 | M2 | Exécutions, router modèles API/locaux & processus : F003, F011, F012 | `qualification/m2/exit.md` | in_progress |
| g0004-m3 | M3 | Workflows, contrats, compétences, hiérarchie & automatisations : F005, F008, F006, F007, F004, F019 | `qualification/m3/exit.md` | in_progress |
| g0005-m4 | M4 | Travail réel dans les repositories : F010, F009, enrichissement F016 | `qualification/m4/exit.md` | in_progress |
| g0006-m5 | M5 | Expérience complète & connecteurs : F013, F014 (10 espaces), F015 (10 espaces), F016, parcours guidé Q-20 | `qualification/m5/exit.md` | pending |
| g0007-m6 | M6 | Sécurité, isolation `strict` Q-05 et adaptateur AI-Guard F017 | `qualification/m6/exit.md` | pending |
| g0008-m7 | M7 | Recette A–Z, F018 pannes/restauration, `unleash-update`, packaging macOS/Ubuntu, SBOM, OSS | `qualification/m7/exit.md` | pending |

## Résultat
M0 est clos. M1 a été publié sur `origin/dev@8598866` avec 369 tests Rust, 7 tests renderer et les contrôles locaux verts. Le retrait physique et Git de t02 est vérifié le 23 novembre. La CI distante reste ouverte sur le correctif publié en `b7c73bb`. M2 compte 13 lots intégrés sur 16 sur `goal/g0003-m2`. V014 prépare l’entrée durable et `5f251ad` intègre son exécution IPC. Le vrai PTY est repris après perte de l’ACK lors du test C5, et le daemon nominal expose la séquence confirmée depuis le reçu natif. `a6a3968` ajoute l’envoi binaire par la CLI, vérifié sur les vrais binaires et après reprise. La campagne locale passe 637 tests Rust et trois essais CLI distincts sans échec. Les autres opérations du terminal, les sondes fournisseur, la preuve runtime de charge mixte et la matrice Q-19 restent ouvertes.

Le 20 novembre, la branche M2 a intégré le passage de l'admission au worker Rust (`23b71c0`), le rattachement de la preuve historique de retrait (`64ea50c`), la commande CLI `process release` (`11956c7`) et la lecture versionnée `provider show` (`de49a2b`). La campagne de 53 tests vérifie notamment le raccord ordonnanceur → worker. La préparation de la génération Ollama et son corps exact sont testés localement et attendent la liaison durable d'autorisation et la revue. Les [actions de clôture de M2](../g0003-m2/goal.md#actions-de-clôture-ouvertes) donnent les dépendances qui maintiennent les lots ouverts.

État de livraison le 20 novembre : 1 jalon sur 8 formellement clos. La qualification locale de M1 est disponible dans [son rapport](../../../../qualification/m1/exit.md) et le SHA publié connu est `8598866`. La vérification de la CI de ce SHA reste ouverte après HTTP 404. Sur M2, les changements locaux précités sont intégrés dans la branche du jalon, toujours à 13 lots sur 16. Les travaux F003, F011 et F012 suivent chacun leur prochaine preuve d'acceptation.

Le 22 novembre, Git confirme `origin/dev@b7c73bb`. Le commit corrige le workflow de validation du socle M1. Les mêmes entrées de produit conservent leurs preuves locales, et la CI GitHub reste à observer après les lectures HTTP 404 de ce commit. [Journal M1](../g0002-m1/journal.md#report-du-correctif-du-workflow-de-validation-sur-dev).

Le 21 novembre, `244e4a4` intègre `provider set` et les corrections Linux sur la branche M2. Le chemin CLI → daemon configure et révise un profil local de fixture. Sur le même arbre, 808 tests Rust passent sous Docker au parallélisme par défaut et 26 restent ignorés. M2 compte 13 lots intégrés sur 16. Les validations natives macOS restent à qualifier sur un runner dédié.

Le 23 novembre, M3 intègre localement `87f47ec`, une première tranche indépendante qui affiche la version engagée et l’état durable d’un run par `GetRun` dans Workflows. Les résultats renderer sur fixture sont acceptés. Les preuves Tauri avec daemon, TUI et moteur restent ouvertes. M2 conserve les priorités F003, F011, F012 et Q-19. La V1 compte un jalon clos sur huit. [Journal M3](../g0004-m3/journal.md)

M3 intègre ensuite le contrat de lecture `ListRuns` en `6f33247`. Sept tests documentaires passent et le dossier exact est approuvé par les deux revues. Le raccord Store/daemon et l’exercice dans les clients restent ouverts. [Journal M3](../g0004-m3/journal.md#23-novembre-2026--f005-contrat-de-découverte-des-runs)

Le 24 novembre, le contrôle documentaire R7 rend zéro erreur et zéro avertissement. M3 dispose des candidats locaux `f5c87b9` pour le premier essai IPC et `face239` pour la régression du renderer composé. M2 conserve le candidat F012 `b103442` séparé du reste des travaux. Les nouveaux essais restent à exécuter. La clôture des deux anciens worktrees Antigravity et de leurs branches est vérifiée après conservation des deux préimages F018. Docker Desktop reste bloqué sur `stopping` et GitHub Actions n’expose pas la CI M1 avec les accès courants. Un jalon sur huit est clos. [Suivi courant](../../current-state.md)

M3 intègre le contrat F006 de brouillon et validation en `966fa7d`. M4 démarre par le contrat F010 de lecture des worktrees en `fd72e00`, sur la base M3. Les deux intégrations locales couvrent les schémas et les fixtures. Leurs essais Docker restent `not_run`. [Fiche M3](../g0004-m3/goal.md) · [Fiche M4](../g0005-m4/goal.md)

Le 24 novembre au soir, le banc Docker est rétabli et reproductible (image, conteneurs isolés, scripts). Deux références y sont mesurées : sur `goal/g0005-m4@37d4431`, renderer et rustdoc verts, `fmt` vert, Clippy rouge (16 erreurs dans un module de test du daemon), tests rouge (847 réussis, 7 échecs, 28 ignorés) ; sur l'instantané du WIP, `fmt`, Clippy et construction des tests rouges. Le WIP non commité est découpé en dix commits par lot sur `task/g0003-m2/green-line@1e0d396` et trois rédacteurs le corrigent en parallèle (lot noyau, lot F012, avis npm du renderer : 2 avis critiques et 3 modérés sur la chaîne de test). L'écart mesuré est de 71 tests du dossier rattachés à du code sur 239 et de 37 preuves enregistrées. Le premier audit de sécurité (profil quick, ciblé) est clos : aucune vulnérabilité confirmée, 4 pistes à valider, 3 candidats réfutés, 19 unités de couverture différées. La CI distante reste le premier prérequis manquant : `gh` répond 401 et 404. Le programme repasse `in_progress` ; un jalon sur huit est clos. [Journal](journal.md#24-novembre-2026--reprise-root-zcode-banc-docker-rétabli-premiers-résultats-réels)
