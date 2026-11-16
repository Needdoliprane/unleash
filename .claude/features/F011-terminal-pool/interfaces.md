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

La réponse contient terminal_id, process_id, process_run_uuid, reused, lease_id, fence, lease_expires_at, state, discovery_trace_id et cursor. Aucune opération dédiée ne renouvelle un lease : il est prolongé de son ttl par chaque `SendTerminalInput` ou `ResizeTerminal` accepté sous ce lease_id/fence, ou par une nouvelle `AcquireTerminal` (variante enregistrée) du même titulaire, et chaque réponse rend le lease_expires_at courant ; une query (`DiscoverTerminals`, `ReadTerminalLog`) ne le prolonge pas, et la fin d'un processus CLI ne le libère ni ne le prolonge. Un lease non renouvelé expire à lease_expires_at : le contrôle est retiré sans arrêter le processus, une opération présentée ensuite sous l'ancien lease_id/fence est refusée en StaleFence sans effet, et le reprendre exige une nouvelle `AcquireTerminal` (variante enregistrée). Une CLI non interactive enchaîne donc `process acquire` et `process input` avant lease_expires_at. Aucune opération n'acquiert ni ne libère un lease implicitement ; `SendTerminalInput`, `ResizeTerminal` et `ReleaseTerminal` exigent lease_id et fence explicites, y compris depuis la CLI non interactive. `SendTerminalInput` ajoute input_seq et un payload binaire encodé selon schéma borné ; `ReadTerminalLog` retourne segments et durable_log_seq ; `StopProcess` retourne une intention puis des événements d'observation.

Événements proposés : `TerminalDiscovered`, `TerminalAcquired`, `ProcessLaunchPrepared`, `ProcessObserved`, `TerminalInputOutcomeUnknown`, `ProcessStopRequested`, `ProcessExited`, `TerminalLogGap`, `TerminalUnavailable`. Erreurs internes : IdentityUnverified, ProcessGone, PortInUse, LogCursorExpired, ActionOutcomeUnknown, CapacityBlocked, LaunchOutcomeUnknown, ToolUnavailable et TerminalUnavailable ; elles sont mappées aux classes communes et reason_code, par exemple ResourceBusy/PORT_IN_USE, OutcomeUnknown/TERMINAL_INPUT_UNKNOWN ou CapabilityDenied/IDENTITY_UNVERIFIED. Projections enregistrées : LaunchOutcomeUnknown → OutcomeUnknown/LAUNCH_OUTCOME_UNKNOWN (after_reconciliation) ; ToolUnavailable → CapabilityDenied/TOOL_UNAVAILABLE (after_human_decision : installation explicite, Q-13) ; TerminalUnavailable → ResourceBusy/TERMINAL_UNAVAILABLE (after_reconciliation : terminal réobservé ou réconcilié par `ReconcileProcess`) ; `TerminalUnavailable` reste aussi le nom de l'événement d'observation. ResourceBusy/ScopeDenied/StaleFence sont déjà communes. Le format d'erreur est uniquement celui d'[ipc-and-apis.md](../../architecture/ipc-and-apis.md). Les états de terminal_sessions (preparing/ready/busy/detached/stopping/exited/suspect/unavailable, [spécification](../../specs/terminal-management.md)) ne sont pas des autorisations : chaque opération revalide sa capacité. L'état d'un processus est celui de `processes.state` ([SQLite](../../architecture/sqlite-design.md) : prepared/live/exited/lost/suspect/identity_mismatch/unknown) ; après reboot, un ancien processus est clos en `lost` (identité de boot différente, processus réputé disparu, effets externes non conclus), sans statut de sortie inventé. `reused` est un booléen distinct de ces états.

Les champs PID/PGID sont des observations, jamais l'identité demandée seule par l'API de stop. Le client vise process_id/génération ; le serveur décide du moyen sûr de l'atteindre.

## Contrôle d’un terminal enregistré et resize

AcquireTerminal possède deux variantes de payload exclusives : demande d’allocation, illustrée ci-dessus, ou acquisition d’un terminal enregistré avec terminal_id, process_run_uuid, scope_revision et mode de contrôle demandé. Une requête contenant à la fois une cible enregistrée et une nouvelle commande de lancement est InvalidArgument. La variante enregistrée ne relance jamais le programme ; elle valide propriétaire, politique et lease disponible.

ResizeTerminal reçoit terminal_id, process_run_uuid, lease_id, fence, rows et cols bornés strictement positifs ; l’autorité revalide génération et droit de contrôle avant appel OS. Réponse : reçu puis taille observée ou erreur explicite, sans déduire un redimensionnement réussi d’un simple envoi. ReleaseTerminal indique la disposition du lease ; détacher une vue seule n’est ni ReleaseTerminal ni StopProcess. Les noms de discussion SpawnProcess, WriteTerminal et ReadProcessLog ne sont des alias publics dans aucune version du protocole, y compris `unleash.ipc.v2` (voir [IPC](../../architecture/ipc-and-apis.md)).
