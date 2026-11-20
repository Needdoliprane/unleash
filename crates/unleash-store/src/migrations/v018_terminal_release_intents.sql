-- A control retirement never disposes the physical host, child or PTY holds.
CREATE TABLE terminal_release_intents (
 action_id TEXT PRIMARY KEY,
 terminal_id TEXT NOT NULL REFERENCES terminal_sessions(terminal_id),
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 command_id TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 authority_generation TEXT NOT NULL,
 daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
 lease_id TEXT NOT NULL REFERENCES resource_leases(lease_id),
 resource_fence INTEGER NOT NULL CHECK(resource_fence BETWEEN 1 AND 9007199254740991),
 control_revision INTEGER NOT NULL CHECK(control_revision BETWEEN 0 AND 9007199254740990),
 source_expires_at_ms INTEGER NOT NULL,
 context_digest TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 state TEXT NOT NULL CHECK(state IN ('prepared','retire_resolved','observed','outcome_unknown','not_dispatched','disposed')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
 observation_object_hash TEXT REFERENCES object_records(object_hash),
 refusal_event_id TEXT UNIQUE REFERENCES events(event_id),
 disposition_event_id TEXT UNIQUE REFERENCES events(event_id),
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
  AND json_extract(context_json,'$.action_id')=action_id
  AND json_extract(context_json,'$.initial_action_id')!=action_id
  AND json_extract(context_json,'$.terminal_id')=terminal_id
  AND json_extract(context_json,'$.receipt_id')=receipt_id
  AND json_extract(context_json,'$.command_id')=command_id
  AND json_extract(context_json,'$.actor_id')=actor_id
  AND json_extract(context_json,'$.principal_id')=principal_id
  AND json_extract(context_json,'$.authority_generation')=authority_generation
  AND json_extract(context_json,'$.daemon_epoch')=daemon_epoch
  AND json_extract(context_json,'$.lease_id')=lease_id
  AND json_extract(context_json,'$.resource_fence')=resource_fence
  AND json_extract(context_json,'$.control_revision')=control_revision
  AND json_extract(context_json,'$.source_expires_at_ms')=source_expires_at_ms,0)),
 CHECK((state='prepared' AND dispatch_id IS NULL AND observation_object_hash IS NULL AND refusal_event_id IS NULL AND disposition_event_id IS NULL)
  OR(state IN ('retire_resolved','outcome_unknown') AND dispatch_id IS NOT NULL AND observation_object_hash IS NULL AND refusal_event_id IS NULL AND disposition_event_id IS NULL)
  OR(state='observed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NOT NULL AND refusal_event_id IS NULL AND disposition_event_id IS NULL)
  OR(state='not_dispatched' AND dispatch_id IS NULL AND observation_object_hash IS NULL AND refusal_event_id IS NOT NULL AND disposition_event_id IS NULL)
  OR(state='disposed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NOT NULL AND refusal_event_id IS NULL AND disposition_event_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX terminal_release_command ON terminal_release_intents(actor_id,command_id,authority_generation);
CREATE UNIQUE INDEX terminal_release_one_pending ON terminal_release_intents(terminal_id)
 WHERE state IN ('prepared','retire_resolved','outcome_unknown');
CREATE TRIGGER terminal_release_initial BEFORE INSERT ON terminal_release_intents
 WHEN NEW.state!='prepared' OR NEW.revision!=1
 BEGIN SELECT RAISE(ABORT,'terminal release must start prepared'); END;
CREATE TRIGGER terminal_release_immutable BEFORE UPDATE ON terminal_release_intents
 WHEN NEW.action_id IS NOT OLD.action_id OR NEW.terminal_id IS NOT OLD.terminal_id
 OR NEW.receipt_id IS NOT OLD.receipt_id OR NEW.command_id IS NOT OLD.command_id
 OR NEW.actor_id IS NOT OLD.actor_id OR NEW.principal_id IS NOT OLD.principal_id
 OR NEW.authority_generation IS NOT OLD.authority_generation OR NEW.daemon_epoch IS NOT OLD.daemon_epoch
 OR NEW.lease_id IS NOT OLD.lease_id OR NEW.resource_fence IS NOT OLD.resource_fence
 OR NEW.control_revision IS NOT OLD.control_revision OR NEW.source_expires_at_ms IS NOT OLD.source_expires_at_ms
 OR NEW.context_digest IS NOT OLD.context_digest OR NEW.context_json IS NOT OLD.context_json
 BEGIN SELECT RAISE(ABORT,'terminal release intention immutable'); END;
CREATE TRIGGER terminal_release_transition BEFORE UPDATE ON terminal_release_intents
 WHEN NOT(NEW.revision=OLD.revision+1 AND (
  (OLD.state='prepared' AND NEW.state IN ('retire_resolved','outcome_unknown') AND NEW.dispatch_id IS NOT NULL)
  OR(OLD.state='retire_resolved' AND NEW.state IN ('observed','outcome_unknown') AND NEW.dispatch_id IS OLD.dispatch_id)
  OR(OLD.state='outcome_unknown' AND NEW.state='disposed' AND NEW.dispatch_id IS OLD.dispatch_id
   AND EXISTS(SELECT 1 FROM actions a WHERE a.action_id=OLD.action_id AND a.state='outcome_unknown')
   AND EXISTS(SELECT 1 FROM object_records o JOIN object_references f ON f.object_hash=o.object_hash
    WHERE o.object_hash=NEW.observation_object_hash AND o.state='live' AND f.state='live'
     AND f.owner_kind='terminal_release_observation' AND f.owner_id=OLD.action_id AND f.source_field=OLD.dispatch_id)
   AND EXISTS(SELECT 1 FROM events e WHERE e.event_id=NEW.disposition_event_id
    AND e.event_type='TerminalReleaseHistoricalAttested' AND e.aggregate_type='terminal_control_intent'
    AND e.aggregate_id=OLD.action_id AND e.aggregate_revision=NEW.revision AND e.causation_kind='command'
    AND e.causation_receipt_id=OLD.receipt_id AND e.actor_id=OLD.actor_id
    AND e.project_id=json_extract(OLD.context_json,'$.project_id') AND e.correlation_id=OLD.terminal_id
    AND e.payload_state='full' AND json_extract(e.payload_json,'$.context_digest')=OLD.context_digest
    AND json_extract(e.payload_json,'$.dispatch_id')=OLD.dispatch_id
    AND json_extract(e.payload_json,'$.observation_object_hash')=NEW.observation_object_hash))
  OR(OLD.state='prepared' AND NEW.state='not_dispatched' AND NEW.dispatch_id IS NULL
   AND NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=OLD.action_id)
   AND NOT EXISTS(SELECT 1 FROM broker_dispatches WHERE action_id=OLD.action_id)
   AND NOT EXISTS(SELECT 1 FROM broker_phase_records WHERE action_id=OLD.action_id AND (phase_key='dispatch' OR phase_key LIKE 'observation:%'))
   AND NOT EXISTS(SELECT 1 FROM actions WHERE action_id=OLD.action_id AND state NOT IN ('prepared','authorizing','authorized','denied','cancelled'))
   AND EXISTS(SELECT 1 FROM events e WHERE e.event_id=NEW.refusal_event_id AND e.event_type='TerminalControlDispatchRefused'
    AND e.aggregate_type='terminal_control_intent' AND e.aggregate_id=OLD.action_id AND e.aggregate_revision=NEW.revision
    AND e.causation_kind='command' AND e.causation_receipt_id=OLD.receipt_id AND e.actor_id=OLD.actor_id
    AND e.project_id=json_extract(OLD.context_json,'$.project_id') AND e.correlation_id=OLD.terminal_id
    AND e.payload_state='full' AND json_extract(e.payload_json,'$.context_digest')=OLD.context_digest
    AND json_extract(e.payload_json,'$.operation')='release'))))
 BEGIN SELECT RAISE(ABORT,'terminal release transition invalid'); END;
CREATE TRIGGER terminal_release_durable BEFORE DELETE ON terminal_release_intents
 BEGIN SELECT RAISE(ABORT,'terminal release intention durable'); END;
CREATE TRIGGER terminal_release_exclusive BEFORE INSERT ON terminal_release_intents
 WHEN EXISTS(SELECT 1 FROM terminal_input_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','write_resolved','outcome_unknown'))
 OR EXISTS(SELECT 1 FROM terminal_control_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','handshake_resolved','outcome_unknown'))
 OR EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE terminal_id=NEW.terminal_id AND refusal_event_id IS NULL AND state IN ('prepared','handshake_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal effect requires observation before release'); END;
CREATE TRIGGER terminal_input_no_pending_release BEFORE INSERT ON terminal_input_intents
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','retire_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal release requires observation before input'); END;
CREATE TRIGGER terminal_control_no_pending_release BEFORE INSERT ON terminal_control_intents
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','retire_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal release requires observation before control'); END;
CREATE TRIGGER terminal_renewal_no_pending_release BEFORE INSERT ON terminal_renewal_intents
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','retire_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal release requires observation before renewal'); END;
CREATE TRIGGER terminal_release_refusal_blocks_dispatch BEFORE INSERT ON action_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE action_id=NEW.action_id AND state='not_dispatched')
 BEGIN SELECT RAISE(ABORT,'terminal release refusal forbids a dispatch'); END;
CREATE TRIGGER terminal_release_refusal_blocks_dispatch_move BEFORE UPDATE OF action_id ON action_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE action_id=NEW.action_id AND state='not_dispatched')
 BEGIN SELECT RAISE(ABORT,'terminal release refusal forbids a dispatch'); END;
CREATE TRIGGER terminal_release_refusal_blocks_broker_dispatch BEFORE INSERT ON broker_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_release_intents WHERE action_id=NEW.action_id AND state='not_dispatched')
 BEGIN SELECT RAISE(ABORT,'terminal release refusal forbids a broker dispatch'); END;
CREATE TRIGGER terminal_release_refusal_blocks_effect_phase BEFORE INSERT ON broker_phase_records
 WHEN (NEW.phase_key='dispatch' OR NEW.phase_key LIKE 'observation:%')
 AND EXISTS(SELECT 1 FROM terminal_release_intents WHERE action_id=NEW.action_id AND state='not_dispatched')
 BEGIN SELECT RAISE(ABORT,'terminal release refusal forbids an effect phase'); END;
