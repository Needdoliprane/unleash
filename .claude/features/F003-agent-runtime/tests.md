# Tests F003

**État : not_run pour tous les cas.** Le testkit sera Rust et utilisera horloge, provider et effets contrôlables. Les preuves contiennent le maximum d'occupation mesuré sur chaque préfixe d'historique, pas uniquement le compteur final.

| Mapping | Procédure et oracle |
|---|---|
| T-F003-001 → AC-F003-001 → EV-F003-001 | Tuer worker après checkpoint puis pendant action ; comparer identités/manifests et nouvelle tentative. |
| T-F003-002 → AC-F003-002 → EV-F003-002 | Trois scopes même skill ; actions croisées refusées ; fallback autorisé donne deux action_id/model_call_id/manifests avec bundle_hash identique, usage lié et modèle effectif vérifiable ; hors politique, nouvelle invocation requise. |
| T-F003-003 → AC-F003-003 → EV-F003-003 | Barrières 20 leaders/N+2/N ; C=1/5/50/51, baisse 50→5 occupée, retries/outils modèles ; trace requested/effective/frozen et occupation≤effective≤50 sur tous préfixes. |
| T-F003-004 → AC-F003-004 → EV-F003-004 | Construire attente circulaire parent/enfant/jugement ; observer checkpoints et progression ; maintenir un stream modèle ouvert comme contre-test. |
| T-F003-005 → AC-F003-005 → EV-F003-005 | Cinq N occupés, manager urgent, soft puis hard reserve ; deadlines contrôlées, aging, quota contrôle et maximum de slots. |
| T-F003-006 → AC-F003-006 → EV-F003-006 | Retarder résultat ancien, reprendre puis livrer ; refus fence et absence de double clôture. |
| T-F003-007 → AC-F003-007 → EV-F003-007 | Injecter claims sans receipt, receipt d'un autre snapshot puis preuve correcte ; décisions et raisons contractuelles. |
| T-F003-008 → AC-F003-008 → EV-F003-008 | Trois assertions indépendantes : définition/bindings engagés immuables ; binding explicite v2 du slot déclaré vierge sous ceiling avant activation ; reprise/itération garde sa liaison. Nouveau workflow seulement au run suivant quiescent. |
| T-F003-009 → AC-F003-009 → EV-F003-009 | Réservation concurrente multi-comptes, quota 429 simulé, timeout coût inconnu ; ledger et absence de retry non autorisé. |
| T-F003-010 → AC-F003-010 → EV-F003-010 | Barrière entre lecture session et commit compaction ; message intercalé, révision et manifeste final complet. |

La campagne distingue requested_limit pending et effective_limit pendant la condition dérivée `draining_to_limit` (requested_limit < effective_limit, admissions_frozen=1), sans nouvelle admission ; le passage à la nouvelle effective_limit et le retrait du gel sont atomiques après drainage. Les tests d'équité fixent poids et seuils avant exécution ; les résultats incluent attente par projet et progrès minimal sous arrivée continue de contrôle. Un test peut prouver la politique de scheduling sur son modèle sans prouver un délai universel fournisseur.

Le sous-cas lecture de T-F003-001 vérifie l’identité et la session durable via `GetAgent` sur le socket du daemon. Il contrôle le watermark, le refus sans preuve administrative, la portée exacte, le masquage d’un parent ou créateur hors scope et le rejet d’un payload non fermé. La création authentifiée, l’assignation et le crash du worker ont leurs propres preuves.

Le sous-cas `GetInvocation` part d’un historique de fixture persisté. Il lit une ancienne tentative clôturée par fence puis une tentative terminée, avec les mêmes identités agent, session et invocation. La suite vérifie l’ordre des pages, la fin de pagination, les bornes, la portée et le watermark transactionnel.

Les tests end-to-end doivent comparer le contexte envoyé et les actions observées, avec données synthétiques et prix fixes de fixture. Les intégrations réelles fournisseur sont séparées, facultatives par credentials de test mais obligatoires pour revendiquer cette compatibilité. Voir [concurrency-tests.md](../../testing/concurrency-tests.md).
