# Tests et preuves F001

**Statut de tous les tests : not_run.** Les commandes Cargo évoquées sont des commandes futures de l'implémentation ; ce dossier n'affirme pas qu'un package compilable existe.

| Test → acceptation → preuve | Méthode et contenu du paquet EV |
|---|---|
| T-F001-001 → AC-F001-001 → EV-F001-001 | Barrière de démarrage dix clients, deux profils/data_root et surcharges de chemins ; un seul verrou utilisateur, un seul ledger/epoch, refus AuthorityAlreadyActive. Tester changement de racine avec host vivant puis neutralisé ; aucune seconde admission avant la barrière. |
| T-F001-002 → AC-F001-002 → EV-F001-002 | Test UI/daemon réel ; identités avant/après fermeture, événements de mission et traces du gestionnaire OS. |
| T-F001-003 → AC-F001-003 → EV-F001-003 | Même mutation par chaque client ; snapshots normalisés au même watermark et diff sémantique vide. |
| T-F001-004 → AC-F001-004 → EV-F001-004 | Credentials invalides, capacité expirée et actor_ref forgé ; refus et absence de dispatch dans adaptateur espion. |
| T-F001-005 → AC-F001-005 → EV-F001-005 | Couper socket après commit ; retries identiques/divergents ; nombre de mutations et receipt stable. |
| T-F001-006 → AC-F001-006 → EV-F001-006 | Injecter ancien worker/lease ; chronologie recovery/admission démontrant la barrière. |
| T-F001-007 → AC-F001-007 → EV-F001-007 | Fixture Rust bloquée avant effet ; stop à plusieurs cutpoints ; rapport final et événements non contradictoires. |
| T-F001-008 → AC-F001-008 → EV-F001-008 | Nomenclature cœur Rust, présentation et dépendances réutilisées ; vérification de la frontière sans logique métier dans le renderer ; capture des chemins écrits et dépôts projets multilangages. |
| T-F001-009 → AC-F001-009 → EV-F001-009 | Saturation de queues et clients non lecteurs ; courbes de profondeur/mémoire, erreurs et receipts conservés. |
| T-F001-010 → AC-F001-010 → EV-F001-010 | Déconnexion, rétention puis restauration ; flux snapshot/watermark et vérification d'absence de trou. |

Chaque EV contient commit du produit, versions toolchain/SQLite/OS, configuration, logs redigés, graine si nécessaire et statut passed/failed/blocked. Les tests de charge fixent leurs budgets avant exécution ; ils ne prouvent pas des performances sur toute machine. Les tests de sécurité utilisent comptes/fixtures isolés et ne tentent pas une élévation réelle.

La validation cible passe par `cargo test --workspace --locked` et des suites d'intégration OS dédiées une fois le workspace créé. Le build sans interface graphique ne valide pas F001-002/003 ; leur absence reste `blocked` ou `not_run`, jamais un succès par substitution. Voir [test-strategy.md](../../testing/test-strategy.md).
