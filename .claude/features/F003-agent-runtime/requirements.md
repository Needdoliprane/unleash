# Exigences F003

| ID | Exigence atomique proposée | AC | Test | Preuve |
|---|---|---|---|---|
| REQ-AGENT-001 | Agent, session, invocation et tentative ont des identités distinctes ; session et décisions restent inspectables après mort du worker. | AC-F003-001 | T-F003-001 | EV-F003-001 |
| REQ-AGENT-002 | Toute invocation persiste son bundle immuable versions/autorités/politique modèle/contexte initial ; chaque appel/fallback possède action et ModelCallManifest distincts avec modèle/contexte effectifs, usage lié et provenance. | AC-F003-002 | T-F003-002 | EV-F003-002 |
| REQ-AGENT-003 | Toute admission N/N+1/N+2 respecte occupation≤effective_limit≤50, défaut 5 ; une baisse sous occupation reste requested_limit pending avec admissions gelées jusqu'à activation atomique. | AC-F003-003 | T-F003-003 | EV-F003-003 |
| REQ-AGENT-004 | Waiting_owner, waiting_children et attentes durables libèrent le slot uniquement après checkpoint sûr ; un appel modèle local en vol le conserve. | AC-F003-004 | T-F003-004 | EV-F003-004 |
| REQ-AGENT-005 | Le scheduler combine priorité contrôle, équité projets et vieillissement ; un manager urgent attend ou obtient un checkpoint sans dépasser C. | AC-F003-005 | T-F003-005 | EV-F003-005 |
| REQ-AGENT-006 | Une reprise crée une nouvelle tentative et un fence valide ; aucun résultat d'une ancienne tentative ne peut modifier l'état courant. | AC-F003-006 | T-F003-006 | EV-F003-006 |
| REQ-AGENT-007 | Les déclarations de worker sont distinguées des observations et jugements ; une clôture contractuelle ne fabrique pas de preuve manquante. | AC-F003-007 | T-F003-007 | EV-F003-007 |
| REQ-AGENT-008 | Définition workflow et bindings engagés restent stables ; nouvelle définition au run suivant quiescent, liaison tardive explicite permise seulement pour slot déclaré vierge avant activation dans son plafond. | AC-F003-008 | T-F003-008 | EV-F003-008 |
| REQ-AGENT-009 | L'admission et les actions modèle réservent atomiquement capacité/coût aux comptes applicables ; inconnues et quotas restent visibles. | AC-F003-009 | T-F003-009 | EV-F003-009 |
| REQ-AGENT-010 | Compactions et manifestes de contexte sont versionnés, inspectables et protégés des écritures concurrentes ; aucune chaîne de pensée privée n'est requise. | AC-F003-010 | T-F003-010 | EV-F003-010 |

Le plafond porte sur les invocations contrôlées localement. Après rupture réseau, le serveur peut encore calculer ; cette incertitude reste un état d'action et de coût, sans autoriser une répétition aveugle. Les processus auxiliaires ont des limites séparées, pas des appels modèles autonomes.
