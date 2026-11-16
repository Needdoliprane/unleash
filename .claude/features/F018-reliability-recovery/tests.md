# Tests F018

**Statut : not_run.** Les tests de panne utiliseront des données synthétiques et des processus Rust isolés. Les opérations destructives ne ciblent jamais des projets réels par défaut.

| Mapping | Injection et preuve |
|---|---|
| T-F018-001 → AC-F018-001 → EV-F018-001 | Arrêt worker avant/après checkpoint ; manifestes, décisions, ancienne/nouvelle tentative et statut de l'action. |
| T-F018-002 → AC-F018-002 → EV-F018-002 | Campagne C0–C12 dont C10a–d/C11a–d : pins, deleting/unlink/tombstone et backup interrompu ; DB restaurée, lignée conservée au restart, receipt/tombstone client, octets référencés, compteur externe et chronologie admission. |
| T-F018-003 → AC-F018-003 → EV-F018-003 | Reboot VM ou modèle de boot explicitement séparé ; identités rejetées et effets distants conservés. |
| T-F018-004 → AC-F018-004 → EV-F018-004 | Fixture effet compteur puis perte receipt ; zéro retry automatique, décision autorisée et observation indépendante. |
| T-F018-005 → AC-F018-005 → EV-F018-005 | Ancienne capacité après nouveau fence, PID réutilisé ; aucune mutation/signaux de mauvaise génération. |
| T-F018-006 → AC-F018-006 → EV-F018-006 | Réservations divergentes et providers inconnus ; AccountingMismatch, coûts retenus et libération seulement après neutralisation locale. |
| T-F018-007 → AC-F018-007 → EV-F018-007 | Backup40 puis host100, ancien token, StartRun(K) à réponse perdue et effets/coûts postérieurs ; capsule durable avant activation, ancien K refusé sur connexion actuelle malgré révision restaurée égale, zéro second effet ; variantes action terminale, prepared au snapshot puis exécutée, capsule partielle/perdue et second crash après import. |
| T-F018-008 → AC-F018-008 → EV-F018-008 | Horloges injectées plus veille OS réelle ; absence de double propriétaire et évènements de revalidation. |
| T-F018-009 → AC-F018-009 → EV-F018-009 | Rapport durable partiel, clients déconnectés, reconnexion ; projections normalisées identiques. |
| T-F018-010 → AC-F018-010 → EV-F018-010 | Filesystem limité, failpoints I/O et copie corrompue ; mode diagnostic, absence de fausse confirmation et conservation WAL. |

Les paquets EV consignent modèle de panne, OS/filesystem, versions SQLite et compile options, seed/cutpoint, préconditions, captures du service de fixture, résultats SQL et rapport final. Un test uniquement simulé n'est pas présenté comme coupure électrique réelle ; une campagne VM est identifiée comme telle.

La procédure inclut un second crash pendant recovery, un résultat ancien livré après reprise et un provider jamais capable de réconcilier. Dans ce dernier cas, le résultat attendu est unknown inspectable avec suite guidée, pas réussite automatique. Les délais de reprise sont mesurés et les exceptions documentées ; les refus sûrs et blocages font partie de l'acceptation lorsqu'ils correspondent au contrat.

Les variantes R6 restent **not_run**. Pour 007, perdre aussi la réponse de RestoreBackup puis retrouver restore_id via inspection après bascule ; aucun client ne reconstruit la commande avec la nouvelle lignée. Vérifier la capsule après crash, les coûts par action source, le gel ciblé si la couverture partielle est localisable et le gel global si son étendue est inconnue. Pour 002, retarder l'ancien collecteur après sélection et après unlink, recréer les mêmes octets dans une nouvelle incarnation, puis vérifier qu'aucune suppression tardive ne touche cette dernière. Examiner les pins embarqués dans le snapshot et les rapprocher des jobs réels avant de les libérer.

Les variantes R7 restent **not_run** et ne créent aucun identifiant. Pour 007 : restauration bloquée avant capsule, avec `restore_requests` en blocked et aucune ligne `restore_imports` ; événement pending dans l'outbox du snapshot puis action postérieure dans la capsule, avec scope gelé et zéro nouvelle action pour le même effet après bascule ; passage du daemon en `read_only_recovery` pendant les étapes 1 à 5. Pour 001 : reprise de la même invocation après waiting_durable avec nouvelle réservation et nouvelle tentative ; une invocation `failed` n'est pas reprise. Pour 004 : classification `unknown` du recovery item, action laissée `outcome_unknown` après ResolveActionOutcome, et EffectOutcomeResolved écrit sur le Run bloqué dans la même transaction.

Voir [crash-recovery-tests.md](../../testing/crash-recovery-tests.md) pour la matrice commune et [test-strategy.md](../../testing/test-strategy.md) pour les règles de preuve.
