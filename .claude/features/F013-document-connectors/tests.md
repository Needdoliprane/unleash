# F013 — stratégie de validation

Harness de tests développé en Rust. Les scénarios utilisent horloge contrôlée, IDs stables de fixture, doubles réseau/processus et injection de fautes. Les tests live sont séparés, avec compte dédié, plafond de coût et secrets hors rapport. Aucune suite produit n’a été exécutée pendant la rédaction.

| Test | Critère / exigence | Injection ou cas limite | Preuve attendue |
| --- | --- | --- | --- |
| T-F013-001 | AC-F013-001 / REQ-DOC-001 | Fixtures REST puis recette comptes réels, sans substituer deux services à trois. | EV-F013-001 : reçu, journal causal, assertions et versions du test |
| T-F013-002 | AC-F013-002 / REQ-DOC-002 | Notion titres seulement et Drive scope restreint. | EV-F013-002 : reçu, journal causal, assertions et versions du test |
| T-F013-003 | AC-F013-003 / REQ-DOC-003 | Renommage du document et URL d’affichage modifiée. | EV-F013-003 : reçu, journal causal, assertions et versions du test |
| T-F013-004 | AC-F013-004 / REQ-DOC-004 | Cycles de liens, très grand dossier et raccourcis externes. | EV-F013-004 : reçu, journal causal, assertions et versions du test |
| T-F013-005 | AC-F013-005 / REQ-DOC-005 | CursorExpired, notifications doublées et laptop suspendu. | EV-F013-005 : reçu, journal causal, assertions et versions du test |
| T-F013-006 | AC-F013-006 / REQ-DOC-006 | Édition au milieu de la pagination. | EV-F013-006 : reçu, journal causal, assertions et versions du test |
| T-F013-007 | AC-F013-007 / REQ-DOC-007 | Rejeu callback, port loopback occupé, tentative expirée après 10 min (`expired`), nonce/aud/iss invalides, issuer Microsoft validé par tid, profil officiel absent → `bring_your_own`, rotation des refresh tokens, coffre verrouillé. | EV-F013-007 : reçu, journal causal, assertions et versions du test |
| T-F013-008 | AC-F013-008 / REQ-DOC-008 | Vérifier artefacts de build et settings exportés : le scanner n’admet que les `public_client_credential` du profil de distribution, aucun jeton Notion ni secret client confidentiel. | EV-F013-008 : reçu, journal causal, assertions et versions du test |
| T-F013-009 | AC-F013-009 / REQ-DOC-009 | Cache chaud, snippets et embeddings croisés ; 403 de raison inconnue : injections suspendues sans tombstone ni purge. | EV-F013-009 : reçu, journal causal, assertions et versions du test |
| T-F013-010 | AC-F013-010 / REQ-DOC-010 | Taille limite, URL de fichier expirée, encodage invalide. | EV-F013-010 : reçu, journal causal, assertions et versions du test |
| T-F013-011 | AC-F013-011 / REQ-DOC-011 | Horloge virtuelle et retry_after supérieur à une minute ; Drive 403 `userRateLimitExceeded` : aucun AccessRevoked, backpressure commune. | EV-F013-011 : reçu, journal causal, assertions et versions du test |
| T-F013-012 | AC-F013-012 / REQ-DOC-012 | SSRF, token passthrough et changement schema. | EV-F013-012 : reçu, journal causal, assertions et versions du test |

## Oracle et gestion des erreurs

Une assertion porte sur le résultat engagé, les effets réellement captés et l’absence d’effet interdit. Les logs seuls ne sont pas l’oracle. Comparer l’état avant/après, les appels au double et les liens de preuve. Une panne injectée après dispatch exige un état de reprise explicable ; relancer le test jusqu’à obtenir un succès ne valide pas l’idempotence.

Chaque preuve doit indiquer commit du produit, version de fixture, configuration effective, commandes Rust de test exécutées, code de sortie et artefacts. Les données sensibles sont remplacées par canaris. Les échecs attendus doivent aussi être persistés pour démontrer la frontière des garanties.

### R6 — T-F013-007/009

Reconnexion même principal A, puis B dans même tenant, B dans autre tenant et rotation de token d’intégration. EV-F013-007/009 vérifie l’identité qualifiée avant activation, auth_generation, aucun Watch/contexte sous compte mal attribué, absence de secret dans incident, caches/affectations conservant leur autorité. Les doubles de fournisseur ne remplacent pas la qualification OAuth réelle.
