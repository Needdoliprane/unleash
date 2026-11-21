# F005 — Tests et preuves attendues

| Test | AC | Injection ou jeu de données | Preuve attendue |
|---|---|---|---|
| T-F005-001 | AC-F005-001 | Publication v2 entre deux appels, deux steps et un crash. | EV-F005-001 : historique de digests immuables. |
| T-F005-002 | AC-F005-002 | Rejouer corpus ordonné dans deux processus sans I/O. | EV-F005-002 : états/intents canoniques identiques et trace zéro adaptateur. |
| T-F005-003 | AC-F005-003 | Tables de vérité 3 valeurs, types et scopes de lecture. | EV-F005-003 : choix exacts, blocages et diagnostics. |
| T-F005-004 | AC-F005-004 | Bornes 1/N, dernier budget disponible, dépassement, reprise. | EV-F005-004 : ledger cumulatif et refus de réservation. |
| T-F005-005 | AC-F005-005 | Permutations de trois branches, seuil atteint, effet inconnu. | EV-F005-005 : join, stop, fencing et clôture ordonnés. |
| T-F005-006 | AC-F005-006 | Crash après effet avant observation durable. | EV-F005-006 : incident, interrogation de résultat et absence de doublon automatique. |
| T-F005-007 | AC-F005-007 | Fixture merge-only, puis permission retirée. | EV-F005-007 : plan effectif sans audit/test et refus de sécurité séparé. |
| T-F005-008 | AC-F005-008 | Branche longue, v2 candidate, demande de nouveau run ; successeur refusé PREDECESSOR_NOT_TERMINAL puis accepté après état terminal ; second successeur refusé SUCCESSOR_EXISTS. | EV-F005-008 : frontière visible, choix v2 du successeur éligible. |
| T-F005-009 | AC-F005-009 | Coupures avant/après commit et notification outbox. | EV-F005-009 : récupération sans perte logique et déduplication. |
| T-F005-010 | AC-F005-010 | Course BindRole / activation du nœud par le reducer (NodeActivated) et tentative de reliaison. | EV-F005-010 : un ordre gagnant, manifest append-only et binding figé. |

Le harness de test doit explorer les entrelacements des commandes incompatibles et produire un journal minimal de reproduction. Les tests de propriété ciblent unicité d'activation, conservation des budgets et immutabilité du run ; ils ne répliquent pas simplement le code d'implémentation. Les tests de sécurité injectent acteur forgé, preuve hors scope et changement de rôle augmentant les capacités. Tous restent planifiés ; la validation documentaire des exemples JSON est un contrôle différent, insuffisant pour déclarer cette feature implémentée.

### Persistance d'une activation de tâche

Le sous-cas Store de T-F005-001/009 matérialise une tâche depuis un run de fixture avec définition épinglée et objet retenu. Il vérifie étape, tâche, reçu et événement, le rejeu, le refus d'une source rattachée à une autre version, les bornes du nœud ainsi que l'annulation atomique avant commit. L'exécution du reducer et l'admission réelle ont leurs propres preuves.

### Cas R6 pour T-F005-003/006

Le corpus inclut for_each v1.2 aux cardinalités 0,1,100,101,300,10 000 ; contrôle des bornes, portées et curseurs à chaque cutpoint. Un effet unknown en body ne fait pas avancer la ligne ; seule une route diagnostique effectivement atteignable peut agir sous blocage. Le ledger reste bloqué durant un choix et un avis continue non étayé. Les EV doivent montrer l’ordre de commits, les admissions réduites, la résolution puis seulement la continuation normale. Les versions v1/v1.1 conservent leurs fixtures et sémantique.
