# Modèles de fichiers d'un goal

Copier ces deux modèles dans `.claude/state/goals/<goal-id>/` au cadrage. `goal.md` est mis à jour ; `journal.md` est en ajout seul.

## `goal.md`

```markdown
# <goal-id> — <titre court>

- Statut : planned | in_progress | blocked | done
- Demandé : <date UTC> par l'utilisateur ; outil : <Claude Code | Codex | Antigravity>
- Branche : goal/<goal-id> ; base : dev@<commit>
- Options : merge=<oui|non>, dry-run=<oui|non>

## Objectif reformulé
<résultat mesurable>

## Rattachement
Jalon : M? — Features : F??? — Exigences : REQ-… — Tests : T-… — Gates : …

## Critères de terminé
- [ ] <commande> → <observation attendue>

## Hors périmètre
- …

## Plan
| Tâche | Objectif | Écriture exclusive | Dépend de | Rôle | Niveau / modèle | Vérification | Statut |
| --- | --- | --- | --- | --- | --- | --- | --- |
| t01 | … | crates/… | — | implementer | standard / sonnet | cargo test -p … | pending |

### Actions de clôture ouvertes

| Périmètre | Prochain résultat observable | Dépendance immédiate et responsable |
| --- | --- | --- |
| t01 | … | … |

## Résultat
<rempli à la clôture : commits, sorties, exigences couvertes, reste à faire, risques, prochain goal>
```

## `journal.md`

```markdown
# Journal <goal-id>

- <horodatage UTC> | cadrage | <résumé>
- <horodatage UTC> | délégation | t01 → implementer, sonnet, worktree .claude/worktrees/<goal-id>-t01
- <horodatage UTC> | compte rendu | t01 commit <sha> ; cargo test -p unleash-domain : 42 passed
- <horodatage UTC> | vérification | t01 verifier opus : accepté | refusé (<constats>)
- <horodatage UTC> | escalade | t03 sonnet → opus après 2 échecs
- <horodatage UTC> | intégration | t01 fusionnée dans goal/<goal-id> ; gates vertes
- <horodatage UTC> | clôture | done ; fusion dev <sha>
```

## Preuve réelle `qualification/evidence/<EV-ID>.json`

```json
{
  "evidence_id": "EV-F002-001",
  "test_id": "T-F002-001",
  "requirement_id": "REQ-PERSIST-001",
  "commit": "<sha>",
  "command": "cargo test -p unleash-store --test crash_points",
  "environment": {"os": "macOS 26.2 arm64", "rustc": "<version>"},
  "result": "passed",
  "observed_at": "2026-11-16T22:00:00Z",
  "artifacts": [],
  "limits": "Fixture locale ; ne qualifie pas une matrice OS complète."
}
```
