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
- [ ] `crates/unleash-action-broker` implémente `GuardPort`, profil `none_accepted`, modes `broker_only`/`guard_optional`/`guard_required`, révocation/suspension et secrets opaques sans fuite
- [ ] Client disparu après acceptation ne crée pas 2 commandes ; daemon tué redémarre avec état engagé ; restore invalide ancienne lignée (`STALE_AUTHORITY_GENERATION`) ; acteur non autorisé refusé et historisé sans secret
- [ ] Preuves `qualification/evidence/EV-F017-*.json` (limites `EXT-GUARD` explicites) & `EV-F018-*.json` et `qualification/m1/exit.md` validés par `qa-expert`, `reliability-expert`, `security-expert`
- [ ] Toutes gates vertes sur `dev` et `git push origin dev` réussi

## Hors périmètre
- Contrat/serveur réel AI-Guard (`EXT-GUARD`, prévu en M6)
- Model Router & scheduler 5/50 (M2)

## Plan
| Tâche | Objectif | Écriture exclusive | Dépend de | Rôle | Niveau / modèle | Vérification | Statut |
| --- | --- | --- | --- | --- | --- | --- | --- |
| t01a | `crates/unleash-action-broker` + double `GuardPort` dans `unleash-testkit` (parallèle) | `Cargo.*`, `crates/unleash-action-broker/**`, `crates/unleash-testkit/**` | — | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | in_progress |
| t01b | `crates/unleash-store` M1 + backup/restore/capsule F018 + Q-12 (parallèle) | `crates/unleash-store/**` | — | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | in_progress |
| t02 | Câblage `ActionBroker` et opérations M1 dans `daemon`/`client`/`cli` | `crates/unleash-{daemon,client,cli}/** et transport Hello ciblé dans unleash-platform/protocol` | APIs t01b stables pour démarrer, candidats t01a/t01b vérifiés avant intégration | systems-implementer | strong / pro | fmt/clippy/test/doc/deny | in_progress |
| t03 | Preuves `EV-F017/F018-*`, `qualification/m1/exit.md`, revues experts, fusion `dev` & push | `qualification/**`, `.claude/state/goals/**` | t02 | root + experts | strong / pro | Toutes gates sur `dev` | pending |

## Résultat
En cours d'exécution.
