# F017 — tests et erreurs

Les outils de test spécifiques au produit sont développés en Rust. Les tests de domaine emploient fixtures déterministes ; les tests de plateforme complètent les doubles sans se substituer aux assertions mécaniques. Aucune preuve produit n’est revendiquée dans ce dossier de conception.

| Test | Critère et exigence | Cas limite / injection | Evidence attendue |
| --- | --- | --- | --- |
| T-F017-001 | AC-F017-001 ; REQ-SEC-001 | Skill malveillant et rôle composé contradictoire. | EV-F017-001 : assertions, journal causal, versions et artefacts |
| T-F017-002 | AC-F017-002 ; REQ-SEC-002 | Chemin secondaire d’adaptateur et appel direct d’outil. | EV-F017-002 : assertions, journal causal, versions et artefacts |
| T-F017-003 | AC-F017-003 ; REQ-SEC-003 | Coffre verrouillé et client non autorisé. | EV-F017-003 : assertions, journal causal, versions et artefacts |
| T-F017-004 | AC-F017-004 ; REQ-SEC-004 | Autre UID, faux socket, token expiré et même UID déclaré hors garantie forte. | EV-F017-004 : assertions, journal causal, versions et artefacts |
| T-F017-005 | AC-F017-005 ; REQ-SEC-005 | TOCTOU, parent traversal et backend incomplet ; mode strict sans backend → IsolationUnavailable avant lancement ; mode standard affiché non confiné. | EV-F017-005 : assertions, journal causal, versions et artefacts |
| T-F017-006 | AC-F017-006 ; REQ-SEC-006 | README, outil MCP, résultat test et skill importé. | EV-F017-006 : assertions, journal causal, versions et artefacts |
| T-F017-007 | AC-F017-007 ; REQ-SEC-007 | Course multi-thread et décision mise en cache. | EV-F017-007 : assertions, journal causal, versions et artefacts |
| T-F017-008 | AC-F017-008 ; REQ-SEC-008 | Timeout, réponse malformée et version incompatible ; guard_optional + Deny ou obligation non imposable → refus ; RequireApproval approuvée, refusée et non prise en charge ; classe d’effet inconnue refusée avant GuardPort. | EV-F017-008 : assertions, journal causal, versions et artefacts |
| T-F017-009 | AC-F017-009 ; REQ-SEC-009 | Permission admin absente et panne pendant révocation ; suspension puis reprise d’un sujet, distinctes de la révocation. | EV-F017-009 : assertions, journal causal, versions et artefacts |
| T-F017-010 | AC-F017-010 ; REQ-SEC-010 | Journal distant indisponible et receipt dupliqué. | EV-F017-010 : assertions, journal causal, versions et artefacts |
| T-F017-011 | AC-F017-011 ; REQ-SEC-011 | WAL, cache, export, sauvegarde et rétention fournisseur. | EV-F017-011 : assertions, journal causal, versions et artefacts |
| T-F017-012 | AC-F017-012 ; REQ-SEC-012 | Tests allow/deny/require_approval/timeout/suspend/revoke/isolation sur vrai système. | EV-F017-012 : assertions, journal causal, versions et artefacts |

## Procédure de validation

Préparer une configuration reproduisible et une graine pour les scénarios concurrents. Vérifier l’état initial ; déclencher l’action et la panne au point nommé ; observer reçu, effets et état durable ; redémarrer si nécessaire ; vérifier absence de duplication et présence de limites explicites. Une réussite doit être liée au commit réellement testé.

Pour les interfaces, enregistrer les parcours clavier et observations visuelles avec captures sans secrets. Pour la sécurité, utiliser canaris et environnements de test dédiés. Pour les systèmes externes, conserver la version de contrat et les capacités réellement négociées. Ne jamais déclarer le gate externe passé sur la seule base d’une réponse de mock.

### R6 — T-F017-005/008/010 et T-SEC-10

Double GuardPort : Allow avec obligation inconnue, connue mais non imposable, contradictoire, entièrement applicable, puis perte de capacité entre évaluation et engagement. Espions indépendants du mapper vérifient zéro dispatch et zéro résolution de secret pour les refus, et contrôle effectif pendant le cas accepté. Sous le profil initial `obligations=none_accepted`, le cas entièrement applicable est lui aussi refusé ; il ne devient un cas accepté qu’avec un `guard_mapping_profile` qualifié. Tester replay/probe sans identité agente factice. La qualification du mapping externe reste séparée ; les EV du double ne prouvent pas l’intégration réelle AI-Guard.
