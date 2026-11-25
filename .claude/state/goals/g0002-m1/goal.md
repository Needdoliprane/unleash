# g0002-m1 — Jalon M1 — Cœur durable & autorité locale : F001, F002 complet, ActionBroker F017, squelette F018

- Statut : in_progress
- Demandé : 2026-11-17T21:08:42Z par programme `g0000-v1` ; outil : Antigravity
- Branche : goal/g0002-m1 ; base : dev@ab2b7ba
- Options : merge=oui, dry-run=non

## Objectif reformulé
Jalon M1 — Cœur durable & autorité locale : F001, F002 complet, ActionBroker F017, squelette F018

## Rattachement
Jalon : M1 — Features : F001, F002, F017 (broker interne), F018 (squelette reprise/backup/restore) — Exigences : REQ-CORE-001..010, REQ-PERSIST-001..010, REQ-SEC-001..011, REQ-REC-002/004/005/007/009/010 — Tests : T-F001-*, T-F002-*, T-F017-001..011, T-F018-002/004/005/007/009/010 — Gates : Gate M1 + revues qa-expert, reliability-expert, security-expert

## Critères de terminé
- [x] `crates/unleash-action-broker` implémente `GuardPort`, profil `none_accepted`, modes `broker_only`/`guard_optional`/`guard_required`, révocation/suspension et secrets opaques sans fuite
- [x] Client disparu après acceptation ne crée pas 2 commandes ; daemon tué redémarre avec état engagé ; restore invalide ancienne lignée (`STALE_AUTHORITY_GENERATION`) ; acteur non autorisé refusé et historisé sans secret
- [x] Preuves `qualification/evidence/EV-F017-*.json` (limites `EXT-GUARD` explicites) & `EV-F018-*.json` et `qualification/m1/exit.md` validés par `qa-expert`, `reliability-expert`, `security-expert`
- [x] Gates Rust et dossier vertes sur `dev@8598866`, push réussi et SHA distant vérifié
- [ ] CI GitHub Actions observée sur le SHA publié `b7c73bb`. Actions et Checks renvoient HTTP 404.

## Hors périmètre
- Contrat/serveur réel AI-Guard (`EXT-GUARD`, prévu en M6)
- Model Router & scheduler 5/50 (M2)

## Plan
| Tâche | Objectif | Écriture exclusive | Dépend de | Rôle | Niveau / modèle | Vérification | Statut |
| --- | --- | --- | --- | --- | --- | --- | --- |
| t01a | `crates/unleash-action-broker` + double `GuardPort` dans `unleash-testkit` (parallèle) | `Cargo.*`, `crates/unleash-action-broker/**`, `crates/unleash-testkit/**` | — | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | done |
| t01a-journal | Stabiliser la reprise de l’évaluation d’autorisation du broker | `crates/unleash-action-broker/**`, doubles ciblés | 08e62c0 intégré, 109b937 pour l’ID partagé | systems-implementer | strong / Sol ultra | reprise exacte, backoff, concurrence, refus de redispatch | done |
| t01b | `crates/unleash-store` M1 + backup/restore/capsule F018 + Q-12 (parallèle) | `crates/unleash-store/**` | — | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | intégré sur `8598866` |
| t02 | Câblage `ActionBroker` et opérations M1 dans `daemon`/`client`/`cli` | `crates/unleash-{daemon,client,cli}/** et transport Hello ciblé dans unleash-platform/protocol` | APIs t01b stables pour démarrer, candidats t01a/t01b vérifiés avant intégration | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | intégré sur `8598866`, retrait physique et Git vérifié le 23 novembre |
| t03a | E2E restauration et lignée sur daemon/CLI réels | `crates/unleash-daemon/tests/e2e_m1_restore_lineage.rs` | APIs t01b et t02 du candidat privé | test-engineer | strong / Sol ultra | Rejet de l’ancienne lignée et consultation du reçu historique | done |
| t03 | Preuves `EV-F017/F018-*`, `qualification/m1/exit.md`, revues experts, fusion `dev` & push | `qualification/**`, `.claude/state/goals/**` | t02 | root + experts | strong / pro | Toutes gates sur `dev` | publié sur `8598866`, lecture CI encore ouverte |

### Actions de clôture ouvertes

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| CI M1 | Identifiant d’exécution et résultat des jobs GitHub Actions pour le SHA publié sur `dev`. | ROOT. Le workflow corrigé passe `actionlint` et est publié sur `dev` (`b7c73bb`, confirmé par Git). Le 25 novembre, l’accès GitHub est rétabli (`gh` connecté, dépôt `Needdoliprane/unleash`) : le dépôt compte zéro exécution d’Actions, le push de `b7c73bb` n’a jamais déclenché de workflow ; Actions est activé depuis. La preuve viendra du prochain push sur `dev` (après intégration de M2 et gates vertes) : relever l’identifiant d’exécution et les conclusions des jobs `ubuntu-latest` et `macos-latest` pour le SHA publié. |

## Résultat
M1 a été accepté localement et publié à `85988665786b1b14d1ca5d74a4fbaba9e5cee2b0`. Le rejeu `dev` donne 369 tests réussis et un dossier valide. [Rapport M1](../../../../qualification/m1/exit.md). Le seul changement publié en `b7c73bbc41e95cfcd5ec0102ad1f4be0d6021f31` corrige le workflow CI. Les checkouts t01b et t02 sont absents physiquement et dans Git. La clôture de M1 attend le résultat de la CI GitHub sur le SHA publié.
