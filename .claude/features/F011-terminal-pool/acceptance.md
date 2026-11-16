# Acceptation F011

| ID | Given / When / Then |
|---|---|
| AC-F011-001 | Étant donné un serveur compatible existant, quand deux agents demandent simultanément la même ressource, alors discovery et justification sont tracées et un seul service compatible est contrôlé, sans double spawn. |
| AC-F011-002 | Étant donné un PID réutilisé avec autre start_identity/run_uuid, quand stop ou reattach est demandé, alors IdentityUnverified ou identity_mismatch bloque le contrôle et le nouveau processus reste intact. Une réutilisation injectée entre dernière observation et signal ne peut viser le témoin ; sans mécanisme stable qualifié, aucun signal n’est émis. |
| AC-F011-003 | Étant donné un PTY détenu par host vivant, quand le daemon est tué puis relancé sans rollback dans la même lignée, alors handshake/fence permet contrôle autorisé ; ancienne capacité et tentative d'adoption depuis autre lignée sont refusées. |
| AC-F011-004 | Étant donné un test long et des limites auxiliaires atteintes, quand l'agent attend durablement, alors son slot est libre mais les réservations processus restent ; un host tentant un appel modèle est refusé. |
| AC-F011-005 | Étant donné une entrée PTY à effet, quand le host tombe après écriture avant receipt, alors l'action devient unknown et le retry de transport ne réécrit pas les octets. |
| AC-F011-006 | Étant donné un groupe avec descendant ordinaire et descendant détaché, quand stop est demandé, alors l'arrêt observé distingue les deux et n'affirme pas un nettoyage total non prouvé. |
| AC-F011-007 | Étant donné une course de bind avec processus inconnu, quand un service démarre, alors PortInUse est observable, le propriétaire inconnu n'est pas tué et toute nouvelle sélection est tracée. |
| AC-F011-008 | Étant donné sortie volumineuse et client lent, quand le quota log est atteint, alors drainage/mémoire restent bornés et les segments, watermarks, gaps et masquages sont cohérents. |
| AC-F011-009 | Étant donné projets de six profils linguistiques et arguments contenant métacaractères, quand leurs outils sont lancés, alors cwd/env/argv restent attribués et aucun shell implicite n'interprète les arguments. |
| AC-F011-010 | Étant donné ressource dépendante d'un worktree, quand detach, delete, crash host puis reboot sont exercés séparément, alors chaque état est distinct et aucune relance implicite ne prétend continuer l'ancien PTY. |

Les preuves de survie host doivent indiquer mode foreground/service utilisateur et OS. Un succès Linux ne remplace pas le cas macOS ; un succès crash daemon ne remplace pas reboot.
