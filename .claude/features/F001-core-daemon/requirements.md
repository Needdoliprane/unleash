# Exigences F001

Toutes les exigences sont proposées pour la V1 et portent sur un comportement observable. L'ID de preuve désigne une preuve future, pas un résultat acquis.

| ID | Exigence atomique | AC | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-CORE-001 | Une identité utilisateur OS possède une seule autorité daemon et un seul ledger global, indépendamment du profil et de la racine de données ; un second démarrage rejoint l'instance ou échoue explicitement. | AC-F001-001 | T-F001-001 | EV-F001-001 |
| REQ-CORE-002 | Fermer tous les clients GUI laisse les missions et services autorisés continuer ; rouvrir reconstitue l'état backend. | AC-F001-002 | T-F001-002 | EV-F001-002 |
| REQ-CORE-003 | GUI, CLI et TUI utilisent les mêmes commandes, révisions et projections, sans logique métier divergente côté client. | AC-F001-003 | T-F001-003 | EV-F001-003 |
| REQ-CORE-004 | Le socket vérifie identité locale et capacités avant toute opération ; une identité privilégiée déclarée dans JSON n'accorde aucun droit. | AC-F001-004 | T-F001-004 | EV-F001-004 |
| REQ-CORE-005 | Une commande est dédupliquée par acteur authentifié et command_id ; une révision périmée ou un payload différent produit un conflit sans mutation. | AC-F001-005 | T-F001-005 | EV-F001-005 |
| REQ-CORE-006 | Le démarrage interdit l'admission avant verrou, vérification du schéma et reprise des ressources nécessaires. | AC-F001-006 | T-F001-006 | EV-F001-006 |
| REQ-CORE-007 | Pause et stop exposent intention, progression et résultat réel, y compris arrêt partiel et effet inconnu. | AC-F001-007 | T-F001-007 | EV-F001-007 |
| REQ-CORE-008 | Le cœur applicatif et système reste Rust, inventorie dépendances et présentation selon la politique R2, et n'écrit aucun état runtime dans le dossier `.claude/` documentaire. | AC-F001-008 | T-F001-008 | EV-F001-008 |
| REQ-CORE-009 | Files IPC/writer et tailles de messages sont bornées ; saturation retourne Backpressure sans empêcher le diagnostic local. | AC-F001-009 | T-F001-009 | EV-F001-009 |
| REQ-CORE-010 | Un client reconnecté reprend par stream_id/sequence ou recharge un snapshot et son watermark cohérents si le curseur est invalide. | AC-F001-010 | T-F001-010 | EV-F001-010 |

Les paramètres de taille et latence doivent être fixés par configuration versionnée et mesurés. Aucune exigence ne présume un daemon root, une disponibilité après logout ou un OS capable d'arrêter immédiatement toute opération.
