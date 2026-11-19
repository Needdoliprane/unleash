# Interfaces F011

Le payload suivant accompagne `AcquireTerminal` dans l'envelope commun. Il est un contrat Unleash proposé, pas une API portable-pty.

```json
{
  "mission_id": "mission-demo",
  "invocation_id": "invocation-demo",
  "purpose_key": "test-runner",
  "reuse_policy": "reuse_compatible",
  "scope_id": "scope-demo",
  "scope_revision": 3,
  "worktree_id": "worktree-demo",
  "cwd": "project-relative/tests",
  "program": "cargo",
  "arguments": ["test", "--locked"],
  "environment_profile_id": "env-demo-v2",
  "pty": {"rows": 30, "cols": 100},
  "readiness": {"kind": "process_started", "timeout_ms": 10000}
}
```

`cwd` est résolu contre une racine autorisée avec contrôle de chemins et symlinks ; un programme peut aussi être python, node, java ou go suivant le projet et la permission. Aucun nom d'exécutable n'accorde à lui seul des droits supplémentaires. Les tokens sont des références injectées seulement si l'action les exige, jamais affichés comme variables dans le manifeste public.

Le résultat métier de l’acquisition est consultable par la lecture causale du reçu. Le parcours de découverte conserve `discovery_trace_id` dans son reçu. Le bail est prolongé de son TTL après une entrée confirmée par un reçu natif sous son lease_id/fence, après un redimensionnement accepté sous ce contrôle ou après une acquisition enregistrée confirmée pour le même titulaire. Le résultat de l’opération indique l’échéance courante `lease_expires_at`. Les lectures `DiscoverTerminals` et `ReadTerminalLog` et la sortie du client CLI laissent le bail suivre son échéance.

À l’échéance, le terminal perd son contrôle et le processus continue. L’ancien lease_id/fence est refusé avec `StaleFence`. Le client peut demander un nouveau contrôle par `AcquireTerminal` (variante enregistrée). Une CLI non interactive enchaîne `process acquire` et `process input` avant l’échéance. `SendTerminalInput`, `ResizeTerminal` et `ReleaseTerminal` exigent un lease_id et un fence explicites. `ReadTerminalLog` retourne des segments et `durable_log_seq`. `StopProcess` retourne une intention puis des événements d’observation.

Événements proposés : `TerminalDiscovered`, `TerminalAcquired`, `ProcessLaunchPrepared`, `ProcessObserved`, `TerminalInputOutcomeUnknown`, `ProcessStopRequested`, `ProcessExited`, `TerminalLogGap`, `TerminalUnavailable`. Erreurs internes : IdentityUnverified, ProcessGone, PortInUse, LogCursorExpired, ActionOutcomeUnknown, CapacityBlocked, LaunchOutcomeUnknown, ToolUnavailable et TerminalUnavailable ; elles sont mappées aux classes communes et reason_code, par exemple ResourceBusy/PORT_IN_USE, OutcomeUnknown/TERMINAL_INPUT_UNKNOWN ou CapabilityDenied/IDENTITY_UNVERIFIED. Projections enregistrées : LaunchOutcomeUnknown → OutcomeUnknown/LAUNCH_OUTCOME_UNKNOWN (after_reconciliation) ; ToolUnavailable → CapabilityDenied/TOOL_UNAVAILABLE (after_human_decision : installation explicite, Q-13) ; TerminalUnavailable → ResourceBusy/TERMINAL_UNAVAILABLE (after_reconciliation : terminal réobservé ou réconcilié par `ReconcileProcess`) ; `TerminalUnavailable` reste aussi le nom de l'événement d'observation. ResourceBusy/ScopeDenied/StaleFence sont déjà communes. Le format d'erreur est uniquement celui d'[ipc-and-apis.md](../../architecture/ipc-and-apis.md). Les états de terminal_sessions (preparing/ready/busy/detached/stopping/exited/suspect/unavailable, [spécification](../../specs/terminal-management.md)) ne sont pas des autorisations : chaque opération revalide sa capacité. L'état d'un processus est celui de `processes.state` ([SQLite](../../architecture/sqlite-design.md) : prepared/live/exited/lost/suspect/identity_mismatch/unknown) ; après reboot, un ancien processus est clos en `lost` (identité de boot différente, processus réputé disparu, effets externes non conclus), sans statut de sortie inventé. `reused` est un booléen distinct de ces états.

Les champs PID/PGID sont des observations, jamais l'identité demandée seule par l'API de stop. Le client vise process_id/génération ; le serveur décide du moyen sûr de l'atteindre.

## Contrôle d’un terminal enregistré et resize

`AcquireTerminal` possède deux variantes fermées et exclusives. Les deux portent un `project_id` non nul dans l’enveloppe. L’allocation figure ci-dessus. `reuse_justification` est requis avec `new_required` et absent avec `reuse_compatible`. Le serveur valide la forme et l’autorisation du critère `readiness` avant le lancement. Il en observe la satisfaction après le lancement avant de déclarer le terminal `ready`.

Le payload enregistré contient `terminal_id`, `process_run_uuid`, `scope_revision` et `control_mode="exclusive"`. `aggregate_id` désigne ce `terminal_id`. `expected_revision` compare la révision du contrôle courant du terminal, initialement 0, sans utiliser ni réécrire la révision de l’allocation de lancement. Le serveur vérifie la génération du processus, le propriétaire, la politique et la disponibilité du bail. Le reçu d’intention n’accorde aucun contrôle avant l’engagement de la preuve de transfert. Un champ de lancement ajouté à cette variante donne `InvalidArgument` et la variante enregistrée ne relance pas le programme. Le titulaire autorisé peut consulter la révision du contrôle courant sur les lectures existantes, même lorsque le bail antérieur est suspect. Cette consultation n’acquiert ni ne prolonge de bail et ne déclenche pas de transfert.

Lorsque le titulaire, le bail et l’incarnation sont inchangés, la variante enregistrée peut renouveler ce bail après observation du même host. Le reçu n’en prolonge pas immédiatement l’échéance. Le renouvellement confirmé conserve bail, fence, révision, processus et PTY. Un rejeu restitue le reçu et la preuve conservée après vérification de la lignée. La lecture de l’état ou des journaux conserve l’échéance.

Une issue de renouvellement inconnue laisse l’ancien contrôle en attente de réconciliation. Une demande distincte et explicite de transfert dans la même lignée et le même boot reste admissible au successeur autorisé sous une époque et un fence supérieurs. La rotation native qualifiée dispose la barrière du renouvellement lors de l’activation atomique du nouveau contrôle. Jusqu’à cette preuve, l’entrée et un autre renouvellement restent fermés. Une réponse perdue se traite par inspection du transfert exact. Sans preuve attribuable au dispatch, l’état reste en réconciliation. Le seul fence avancé du manifeste ne suffit pas. Le renouvellement antérieur demeure inconnu et ne se réexécute pas.

`GetStateSnapshot` retourne `terminals.items` et `terminals.truncated` sous le même curseur que les autres projections. Chaque entrée contient `terminal_id`, `process_run_uuid`, `project_id`, `control_revision`, `terminal_state`, `process_state` et `lease_state`. Le daemon filtre par principal actif et par projets autorisés avant la borne de résultat. La projection exige une identité complète déjà observée, un lancement réussi et enregistré, une réservation PTY active et un bail lié au terminal dans la même lignée et le même boot. Elle expose des faits persistés et ne contacte pas l’hôte. Le client doit obtenir la capacité par la commande de contrôle, qui vérifie séparément l’état de l’hôte et l’autorité courante.

ResizeTerminal reçoit terminal_id, process_run_uuid, lease_id, fence, rows et cols bornés strictement positifs ; l’autorité revalide génération et droit de contrôle avant appel OS. Réponse : reçu puis taille observée ou erreur explicite, sans déduire un redimensionnement réussi d’un simple envoi. ReleaseTerminal indique la disposition du lease ; détacher une vue seule n’est ni ReleaseTerminal ni StopProcess. Les noms de discussion SpawnProcess, WriteTerminal et ReadProcessLog ne sont des alias publics dans aucune version du protocole, y compris `unleash.ipc.v2` (voir [IPC](../../architecture/ipc-and-apis.md)).

## Entrée binaire du terminal

`SendTerminalInput` exige l’UUID du projet dans l’enveloppe. L’agrégat est le terminal visé et `expected_revision` vise sa révision de contrôle. Le [schéma de commande](../../schemas/ipc-command.schema.json) ferme le payload à `terminal_id`, `process_run_uuid`, `lease_id`, `fence`, `input_seq` et `payload_bytes`. Les trois identifiants sont des UUID. Le fence et la séquence sont des entiers de 1 à 2⁵³−1. `payload_bytes` contient 1 à 65 536 entiers de 0 à 255 et accepte les octets non textuels.

Pour une nouvelle entrée, le writer revalide le principal authentifié, la portée, la politique, le gel, le bail courant et son fence. Il vérifie la génération, la révision de contrôle et la séquence suivante. L’intention, les diagnostics et les preuves d’action représentent la charge par son digest et sa longueur. La sortie du PTY, y compris un écho éventuel, suit les règles de capture et de masquage des journaux du terminal. Le reçu d’intention peut être relu sans répétition de l’effet. Le renouvellement du bail exige un reçu natif corrélé et un bail encore valide au moment de la projection. Une issue inconnue conserve la dernière séquence confirmée.

## Lecture causale de l’acquisition

Le client lit le reçu de commande ordinaire avec `GetCommandReceipt` et un payload fermé `{command_id}`. Pour obtenir les faits du contrôle observé, il transmet `{command_id, result_ref}` avec l’identifiant du projet exact dans l’enveloppe. Il reprend la référence exacte du reçu d’`AcquireTerminal`. Une allocation porte `terminal-allocation:<terminal_id>` et un transfert porte `terminal-control:<action_id>`. La référence comporte de 1 à 128 octets UTF-8 sans caractère de contrôle. La requête est soumise aux restrictions d’autorité de `GetCommandReceipt`.

Le [schéma de réponse](../../schemas/ipc-query-response.schema.json) définit `terminal_acquisition_response`. Son résultat fermé porte les identifiants du reçu, de la commande, de la lignée, du daemon et du terminal, les identités de processus, `reused`, l’état `ready`, `busy` ou `detached` et les faits du bail et du contrôle. `cursor` identifie l’événement causal de contrôle observé. `watermark` donne le curseur du snapshot de lecture. `lease_expires_at` est l’échéance en millisecondes Unix. `input_seq` est la dernière séquence confirmée par un reçu natif durable. Pour un PTY neuf, la valeur zéro vient de l’observation native capturée lors du lancement.

La lecture vérifie la propriété du reçu, la portée du projet, la lignée et le bail courant, ainsi que la preuve publiée du dispatch et de son observation. Le snapshot SQLite et les preuves déjà enregistrées suffisent à cette lecture. Les champs se limitent aux faits du contrôle observé. Chaque action vérifie séparément la capacité, le gel, la politique et le bail avant d’engager un effet.

Une forme invalide retourne `InvalidArgument`. Un reçu ou une référence étrangère retourne `NotFoundOrHidden`. Un contrôle sans observation qualifiée ou un bail expiré ou remplacé retourne `StaleFence`. Une preuve indisponible renvoie `ContractViolation/EVIDENCE_UNAVAILABLE`, y compris un ancien objet de lancement dont la séquence native manque. Le reçu ordinaire reste inspectable selon son contrat.

## Préparation durable de l’allocation

Dans la variante de création par un opérateur humain, le service obtient le principal depuis la session authentifiée. Il résout la mission, la portée et la commande effective avant de passer au writer. Le port initial V010 couvre un programme absolu, un répertoire qualifié dans la racine principale du projet et une référence d’environnement versionnée et épinglée par digest. Le service reste responsable de vérifier cette référence et de résoudre chemins et symlinks. Les chaînes fournies par le client n’attestent aucune de ces vérifications.

Le writer revalide acteur, principal, révisions de mission et de projet et politique. Il engage ensemble le reçu, le bail et les réservations auxiliaires du host, de l’enfant et du PTY, distinctes du pool d’exécution. Un rejeu rend le même reçu et la même préparation. L’événement d’allocation décrit l’état `preparing`. La propriété active est vérifiée aussi contre le registre des terminaux et processus au moment du dispatch et lors de sa résolution.

La résolution du lancement vérifie le marqueur durable exact du broker, le digest figé, les propriétaires et réservations, la lignée, le boot, l’époque, le fence et le délai du bail. Elle consomme une fois la préparation avant tout lancement. Une issue inconnue passe par la réconciliation. Un état `launch_resolved` ne suffit pas à constater qu’un processus est parti ou qu’un PTY est prêt. Ces constats attendent le producteur OS et le handshake du host.

Le writer engage la preuve du lancement et les lignes processus/terminal avec la phase d’observation du broker. L’instant de capture de l’identité peut précéder celui du reçu. La preuve doit appartenir au dispatch exact et satisfaire l’ordre temporel du [contrat SQLite](../../architecture/sqlite-design.md#actions-processus-et-preuves). Une identité observée puis une sortie confirmée par le même host peuvent former une observation valable même pour un processus bref.

Le retrait de la préparation conserve son historique. Le writer exige une observation interne attribuable à la même incarnation et liée causalement au marqueur ou à l’absence vérifiée de dispatch. Il vérifie toute ligne de terminal ou de processus déjà inscrite. Une ressource inconnue, un bail expiré, un redémarrage ou une restauration ne libèrent pas la propriété à eux seuls. L’opération de disposition reste interne. L’API publique ne permet pas au client de produire cette preuve. Voir le [contrat SQLite](../../architecture/sqlite-design.md#actions-processus-et-preuves).

## Configuration du terminal

`SetSettings` accepte `patch.terminal_runtime` sur le singleton `user/default`. La demande porte `project_id=null` et utilise les mêmes contrôles d’authentification et de révision que les autres paramètres utilisateur. L’omission du champ conserve la configuration existante. Une valeur explicite `null` est invalide. La version 1 configure séparément les plafonds de hosts, de processus et de PTY entre 1 et 50 et la durée de bail entre 1 000 et 300 000 ms.

Chaque configuration porte entre un et seize profils. Un profil contient `profile_id`, `version` et une table non vide `variables`. Le profil est qualifié par le serveur et son contenu est épinglé. Un couple identifiant/version reste lié au même digest et la version effective d’un profil ne recule pas. Le writer contrôle l’unicité des identifiants dans la liste. La version 1 accepte uniquement les valeurs de locale, de terminal et de fuseau énumérées dans le [schéma de commande](../../schemas/ipc-command.schema.json). Les secrets suivent le coffre F017 et n’entrent ni dans cette table ni dans le manifeste de lancement.

Le service résout `environment_profile_id` à partir de cette configuration avant la préparation durable. Un profil absent, une configuration invalide ou un contrôle d’autorité insuffisant ferme l’acquisition avant effet. Lorsque `terminal_runtime` est présent, le schéma refuse les champs inconnus dans `payload`, `patch` et les profils. Les champs de patch déjà admis restent utilisables avec lui. Les tests de la voie produit doivent aussi vérifier la persistance, la configuration effective des pools et l’observation du PTY.
