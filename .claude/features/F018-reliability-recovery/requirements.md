# Exigences F018

| ID | Exigence atomique proposée | AC | Test | Preuve |
|---|---|---|---|---|
| REQ-REC-001 | Un crash worker préserve agent/session/décisions/versions et ne reprend qu'avec tentative et fence nouveaux après contrôle des effets engagés. | AC-F018-001 | T-F018-001 | EV-F018-001 |
| REQ-REC-002 | Le redémarrage daemon conserve la lignée et applique C0–C12, dont pins/suppressions/backups C10a–d/C11a–d ; il retrouve receipts/tombstones et notifications sans réadmettre un ancien exécuteur non classifié ni libérer un pin suspect. | AC-F018-002 | T-F018-002 | EV-F018-002 |
| REQ-REC-003 | Un nouveau boot invalide les identités de processus précédentes sans supposer la disparition des effets distants ni la récupération du PTY/RAM. | AC-F018-003 | T-F018-003 | EV-F018-003 |
| REQ-REC-004 | Un effet dont résultat ou dispatch est ambigu devient outcome_unknown ; seule une preuve ou une décision autorisée enregistrée permet sa suite. | AC-F018-004 | T-F018-004 | EV-F018-004 |
| REQ-REC-005 | Toute reprise vérifie génération OS et authority_generation ; epoch/fence sont ordonnés seulement dans cette lignée, un ancien contrôle ne peut pas muter la génération courante. | AC-F018-005 | T-F018-005 | EV-F018-005 |
| REQ-REC-006 | Slots, budgets, leases et processus sont rapprochés sans libération arbitraire ; coûts inconnus et exécuteurs suspects restent réservés selon leur portée. | AC-F018-006 | T-F018-006 | EV-F018-006 |
| REQ-REC-007 | Restore neutralise les anciens exécuteurs et importe une capsule durable du delta post-snapshot avant nouvelle lignée/secrets/stream ; anciennes commandes refusées avant lookup, effets/coûts non effacés, inventaire incomplet bloquant et couverture externe inconnue gelant les scopes concernés. | AC-F018-007 | T-F018-007 | EV-F018-007 |
| REQ-REC-008 | Recul d'horloge, veille et changement de boot n'autorisent pas une réaffectation fondée sur la seule deadline murale d'une lease. | AC-F018-008 | T-F018-008 | EV-F018-008 |
| REQ-REC-009 | GUI/CLI/TUI exposent un même rapport durable de reprise avec états connus/inconnus, preuves manquantes et opérations permises. | AC-F018-009 | T-F018-009 | EV-F018-009 |
| REQ-REC-010 | Corruption, espace insuffisant ou I/O indéterminée empêchent les fausses confirmations et déclenchent mode de récupération sans supprimer silencieusement journal/WAL. | AC-F018-010 | T-F018-010 | EV-F018-010 |

Ces exigences ne promettent pas un RTO universel. Les temps de reprise sont mesurés par classe de panne, taille d'état et dépendances externes ; une inconnue réelle peut exiger une décision au lieu d'un délai arbitraire de retry.
