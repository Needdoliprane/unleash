# F017 — exigences

Statut : règles proposées pour implémenter les invariants du brief et les clarifications. Chaque ID a une correspondance directe avec acceptation, test et preuve.

| Exigence | Règle | Critère |
| --- | --- | --- |
| REQ-SEC-001 | Calculer les droits par intersection et interdire l’auto-élévation des skills/rôles. | AC-F017-001 |
| REQ-SEC-002 | Faire passer les effets initiés par Unleash via ActionBroker. | AC-F017-002 |
| REQ-SEC-003 | Utiliser coffre OS et SecretRef sans mettre clés générales dans contexte ou processus projet. | AC-F017-003 |
| REQ-SEC-004 | Distinguer client administratif et capability worker sur API locale. | AC-F017-004 |
| REQ-SEC-005 | Faire appliquer scopes outils et isolation réellement requise. | AC-F017-005 |
| REQ-SEC-006 | Empêcher source externe de devenir autorité de contrôle. | AC-F017-006 |
| REQ-SEC-007 | Revalider mandat, politique et fence juste avant engagement. | AC-F017-007 |
| REQ-SEC-008 | Définir broker_only/optional/required sans repli implicite. | AC-F017-008 |
| REQ-SEC-009 | Connecter, afficher santé/capacités/politiques/audit et gérer suspension/révocation selon protocole réel. | AC-F017-009 |
| REQ-SEC-010 | Lier chaque décision de sécurité à action et effet observé. | AC-F017-010 |
| REQ-SEC-011 | Appliquer suppression/redaction selon politique avec limites explicites. | AC-F017-011 |
| REQ-SEC-012 | Ne clôturer intégration AI-Guard qu’avec preuves réelles des frontières retenues. | AC-F017-012 |

Pour REQ-SEC-009, la suspension d’un sujet (SuspendSubject, levée par ResumeSubject) est locale, réversible, toujours disponible et distincte de la révocation (RevokeSubject) ; seule sa propagation vers AI-Guard dépend du protocole réel négocié par GuardPort ([ai-guard](../../specs/ai-guard.md)).

Les limites décrites dans les audits sont des frontières à exposer, pas des exceptions silencieuses aux critères. Une action acceptée, une action accomplie et une preuve disponible sont des états différents.

