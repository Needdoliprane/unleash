# Acceptation F003

| ID | Given / When / Then |
|---|---|
| AC-F003-001 | Étant donné un agent ayant décisions et actions, quand son worker meurt, alors agent/session/invocation restent inspectables et une reprise crée seulement une nouvelle tentative. |
| AC-F003-002 | Étant donné trois N même skill/scopes distincts et un fallback autorisé, quand ils exécutent leur contrat, alors bundle_hash reste stable, chaque appel/fallback possède action et manifeste effectif distincts, et les accès hors scope sont refusés. |
| AC-F003-003 | Étant donné vingt leaders et des workers multi-projets, quand C=5 puis 50 sont utilisés et qu'une baisse 50→5 arrive avec 50 occupés, alors occupation≤effective_limit≤50 toujours ; requested_limit=5 reste pending et admissions gelées jusqu'à occupation≤5, puis activation atomique ; 51 est refusé. |
| AC-F003-004 | Étant donné cinq N en attente de jugement ou des N+1 attendant leurs enfants, quand l'attente est durable, alors leurs checkpoints rendent les slots ; un appel modèle encore localement en vol ne les rend pas. |
| AC-F003-005 | Étant donné cinq N actifs et un manager urgent, quand une préemption est demandée, alors le manager attend un slot réellement rendu ; sous fixtures bornées il démarre dans la borne annoncée et les workers vieillissants progressent. |
| AC-F003-006 | Étant donné une reprise avec nouveau fence, quand l'ancienne tentative renvoie un résultat, alors StaleFence empêche une transition et le résultat reste uniquement diagnostic. |
| AC-F003-007 | Étant donné un claim « tests OK » sans exécution observée, quand la clôture est demandée, alors l'absence de preuve est visible et la décision respecte le contrat effectif sans inventer de test. |
| AC-F003-008 | Étant donné un run v1 avec branche active et slot déclaré vierge, quand workflow/rôle/skill v2 sont publiés, alors (1) définition workflow et bindings engagés restent v1 ; (2) slot vierge peut recevoir explicitement rôle/skill v2 dans son plafond avant activation ; (3) reprise/itération garde le binding engagé et seule nouvelle définition workflow attend le run suivant quiescent. |
| AC-F003-009 | Étant donné budgets presque épuisés et coût inconnu, quand plusieurs appels sont admissibles, alors réservations atomiques et quota empêchent la surallocation ; l'inconnue garde sa réserve. |
| AC-F003-010 | Étant donné une compaction concurrente avec un message humain, quand la nouvelle tête est proposée, alors le message n'est pas perdu, les sources/omissions restent inspectables et aucun raisonnement privé n'est requis. |

H/I sont couverts par 001/003/004/005/006, C par 002 et K par 008. La borne de démarrage du manager est conditionnelle et mesurée sur fixtures à deadlines contrôlées ; elle n'est pas présentée comme garantie face à un OS bloqué ou à une panne fournisseur.
