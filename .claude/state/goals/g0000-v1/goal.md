# g0000-v1 — Programme V1 complète d'OVPT Unleash

- Statut : in_progress
- Demandé : 2026-11-17T00:32:48+01:00 par l'utilisateur ; outil : Antigravity
- Branche : dev ; base : main@a8a4379a027b31aed4ff755b4d2175096bc8ede7
- Options : merge=oui, dry-run=non

## Objectif reformulé
Réaliser et qualifier la V1 complète distribuable et open source (Apache-2.0) d'OVPT Unleash sur la branche `dev` (poussée sur `origin` sans toucher `main`), en enchaînant les jalons M0 à M7 dans l'ordre du chemin critique : noyau durable Rust, 19 features (F001–F019), 239 exigences reliées à leurs tests `T-…` et preuves réelles `qualification/evidence/EV-….json`, parité GUI (Tauri 2 + React/TS) / TUI (Ratatui/Crossterm) / CLI sur les 58 groupes `PAR-…`, campagnes de pannes, sécurité et préparation de distribution/packaging.

## Rattachement
Jalons : M0 → M7 — Features : F001–F019 — Exigences : 239 REQ — Invariants : INV-001..INV-065 — Gates : GATE-ARCH, GATE-RUST, GATE-UI, GATE-AUTH, GATE-AUTOMATION, GATE-LOCAL, GATE-GUIDED, EXT-*

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

## Actions attendues de l'utilisateur
1. **GitHub CLI (`gh`)** : exécuter `gh auth login -h github.com` localement (le token `gh` actuel pour `Needdoliprane` est invalide, empêchant `gh run list --branch dev` de contrôler automatiquement les runs GitHub Actions).
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
| g0004-m3 | M3 | Workflows, contrats, compétences, hiérarchie & automatisations : F005, F008, F006, F007, F004, F019 | `qualification/m3/exit.md` | pending |
| g0005-m4 | M4 | Travail réel dans les repositories : F010, F009, enrichissement F016 | `qualification/m4/exit.md` | pending |
| g0006-m5 | M5 | Expérience complète & connecteurs : F013, F014 (10 espaces), F015 (10 espaces), F016, parcours guidé Q-20 | `qualification/m5/exit.md` | pending |
| g0007-m6 | M6 | Sécurité, isolation `strict` Q-05 et adaptateur AI-Guard F017 | `qualification/m6/exit.md` | pending |
| g0008-m7 | M7 | Recette A–Z, F018 pannes/restauration, `unleash-update`, packaging macOS/Ubuntu, SBOM, OSS | `qualification/m7/exit.md` | pending |

## Résultat
M0 est clos. M1 est publié sur `origin/dev@8598866` avec 369 tests Rust, 7 tests renderer et les contrôles locaux verts. La CI distante et le résidu physique d’un checkout restent ouverts. M2 avance sur `goal/g0003-m2`. Le socle `32773bf` rassemble l’admission, la préparation SQLite des sondes et du terminal, les transports OpenAI et Anthropic testés sur TLS local, les versions durables des profils fournisseurs et le correctif d’arrêt. Les six contrôles passent avec 523 tests sur 126 suites. Les raccords au daemon, le parcours terminal après crash et la matrice Q-19 restent ouverts.
