# Exigences F011

| ID | Exigence atomique proposée | AC | Test | Preuve |
|---|---|---|---|---|
| REQ-TERM-001 | Toute allocation recherche d'abord les ressources compatibles, enregistre candidats/choix/raison et empêche deux créations concurrentes pour la même propriété exclusive. | AC-F011-001 | T-F011-001 | EV-F011-001 |
| REQ-TERM-002 | Le contrôle d'un processus vérifie process_run_uuid, boot, identité de démarrage, UID et PID/PGID ; un signal exige en outre une cible non réutilisable qualifiée, sinon IdentityUnverified ; des observations répétées ne suffisent pas. | AC-F011-002 | T-F011-002 | EV-F011-002 |
| REQ-TERM-003 | Un process-host Rust détient PTY/logs des sessions persistantes et permet rattachement après crash daemon seulement avec handshake/fence valides. | AC-F011-003 | T-F011-003 | EV-F011-003 |
| REQ-TERM-004 | PTY, hosts, enfants et autres ressources auxiliaires sont budgétés séparément des invocations ; aucun host ne peut initier de raisonnement modèle autonome. | AC-F011-004 | T-F011-004 | EV-F011-004 |
| REQ-TERM-005 | Toute entrée mutante passe par le broker, possède action/sequence/receipt et ne se répète pas automatiquement si son écriture au PTY est ambiguë. | AC-F011-005 | T-F011-005 | EV-F011-005 |
| REQ-TERM-006 | Stop contrôle la génération et le groupe autorisés, observe le résultat et expose descendants inconnus/arrêt partiel sans promettre une sandbox. | AC-F011-006 | T-F011-006 | EV-F011-006 |
| REQ-TERM-007 | Une réservation de port distingue lease logique et bind réel ; conflit ou propriétaire inconnu n'entraîne jamais un kill arbitraire. | AC-F011-007 | T-F011-007 | EV-F011-007 |
| REQ-TERM-008 | Les logs sont drainés et bornés, ordonnés par séquence, avec watermark durable, gaps, redaction et ACL inspectables. | AC-F011-008 | T-F011-008 | EV-F011-008 |
| REQ-TERM-009 | Programme/arguments/cwd/env/worktree sont explicitement attribués, validés et compatibles avec les outils des projets multilangages. | AC-F011-009 | T-F011-009 | EV-F011-009 |
| REQ-TERM-010 | La libération, suppression de worktree et reprise distinguent detach client, fin process, perte host et reboot, sans dupliquer implicitement un service. | AC-F011-010 | T-F011-010 | EV-F011-010 |

La compatibilité d'une ressource dépend du snapshot attendu et des permissions, pas seulement du nom du programme ou du port. La découverte d'une ressource externe n'accorde aucune propriété automatique.
