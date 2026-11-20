-- Keep every V016 intention and its canonical digest unchanged. A refusal is a
-- fact about an unstarted intention; it has no native dispatch or observation.
ALTER TABLE terminal_renewal_intents
 ADD COLUMN refusal_event_id TEXT REFERENCES events(event_id);
CREATE UNIQUE INDEX terminal_renewal_refusal_event ON terminal_renewal_intents(refusal_event_id)
 WHERE refusal_event_id IS NOT NULL;

DROP INDEX terminal_renewal_one_pending;
CREATE UNIQUE INDEX terminal_renewal_one_pending ON terminal_renewal_intents(terminal_id)
 WHERE refusal_event_id IS NULL AND state IN ('prepared','handshake_resolved','outcome_unknown');

CREATE TRIGGER terminal_renewal_refusal_initial BEFORE INSERT ON terminal_renewal_intents
 WHEN NEW.refusal_event_id IS NOT NULL
 BEGIN SELECT RAISE(ABORT,'terminal refusal requires a prepared intention'); END;

DROP TRIGGER terminal_renewal_transition;
CREATE TRIGGER terminal_renewal_transition BEFORE UPDATE ON terminal_renewal_intents
 WHEN NOT(NEW.revision=OLD.revision+1 AND OLD.refusal_event_id IS NULL AND (
  (NEW.refusal_event_id IS NULL AND (
   (OLD.state='prepared' AND NEW.state IN ('handshake_resolved','outcome_unknown') AND NEW.dispatch_id IS NOT NULL)
   OR(OLD.state='handshake_resolved' AND NEW.state IN ('observed','outcome_unknown') AND NEW.dispatch_id IS OLD.dispatch_id)
   OR(OLD.state='outcome_unknown' AND NEW.state='disposed' AND NEW.dispatch_id IS OLD.dispatch_id)))
  OR(NEW.refusal_event_id IS NOT NULL AND OLD.state='prepared' AND NEW.state='prepared'
   AND NEW.dispatch_id IS NULL
   AND NOT EXISTS(SELECT 1 FROM action_dispatches WHERE action_id=OLD.action_id)
   AND NOT EXISTS(SELECT 1 FROM broker_dispatches WHERE action_id=OLD.action_id)
   AND NOT EXISTS(SELECT 1 FROM broker_phase_records WHERE action_id=OLD.action_id
                  AND (phase_key='dispatch' OR phase_key LIKE 'observation:%'))
   AND NOT EXISTS(SELECT 1 FROM actions a WHERE a.action_id=OLD.action_id
    AND (a.state NOT IN ('cancelled','denied')
     OR a.control_principal_id IS NOT OLD.principal_id
     OR a.authority_generation!=OLD.authority_generation OR a.daemon_epoch!=OLD.daemon_epoch))
   AND EXISTS(SELECT 1 FROM events e WHERE e.event_id=NEW.refusal_event_id
    AND e.event_type='TerminalControlDispatchRefused' AND e.aggregate_type='terminal_control_intent'
    AND e.aggregate_id=OLD.action_id AND e.aggregate_revision=NEW.revision
    AND e.causation_kind='command' AND e.causation_receipt_id=OLD.receipt_id
    AND e.actor_id=OLD.actor_id AND e.correlation_id=OLD.terminal_id
    AND e.project_id=json_extract(OLD.context_json,'$.project_id') AND e.payload_state='full'
    AND json_extract(e.payload_json,'$.action_id')=OLD.action_id
    AND json_extract(e.payload_json,'$.receipt_id')=OLD.receipt_id
    AND json_extract(e.payload_json,'$.command_id')=OLD.command_id
    AND json_extract(e.payload_json,'$.terminal_id')=OLD.terminal_id
    AND json_extract(e.payload_json,'$.reason') IN ('daemon_superseded','policy_changed','authorization_denied')
    AND json_extract(e.payload_json,'$.operation')=CASE WHEN json_type(OLD.context_json,'$.resize')='object'
      THEN 'resize' ELSE 'renewal' END
    AND json_extract(e.payload_json,'$.context_digest')=OLD.context_digest))))
 BEGIN SELECT RAISE(ABORT,'terminal renewal transition invalid'); END;

DROP TRIGGER terminal_control_no_pending_renewal;
CREATE TRIGGER terminal_control_no_pending_renewal BEFORE INSERT ON terminal_control_intents
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents r
  WHERE r.terminal_id=NEW.terminal_id AND r.refusal_event_id IS NULL
  AND r.state IN ('prepared','handshake_resolved','outcome_unknown')
  AND NOT EXISTS(SELECT 1 FROM terminal_renewal_transfer_links link
   WHERE link.renewal_action_id=r.action_id AND link.renewal_dispatch_id=r.dispatch_id
    AND link.renewal_context_digest=r.context_digest AND link.transfer_action_id=NEW.action_id
    AND link.transfer_receipt_id=NEW.receipt_id AND r.state='outcome_unknown'
    AND NEW.authority_generation=r.authority_generation AND NEW.daemon_epoch>r.daemon_epoch
    AND NEW.actor_id=r.actor_id AND NEW.principal_id=r.principal_id
    AND NEW.previous_lease_id=r.lease_id AND NEW.next_fence>r.resource_fence
    AND json_extract(NEW.context_json,'$.daemon_boot_identity')=json_extract(r.context_json,'$.daemon_boot_identity')
    AND json_extract(NEW.context_json,'$.process_run_uuid')=json_extract(r.context_json,'$.process_run_uuid')
    AND json_extract(NEW.context_json,'$.initial_context_digest')=json_extract(r.context_json,'$.initial_context_digest')))
 BEGIN SELECT RAISE(ABORT,'terminal renewal requires observation before transfer'); END;

DROP TRIGGER terminal_input_no_pending_renewal;
CREATE TRIGGER terminal_input_no_pending_renewal BEFORE INSERT ON terminal_input_intents
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE terminal_id=NEW.terminal_id
  AND refusal_event_id IS NULL AND state IN ('prepared','handshake_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal renewal requires observation before input'); END;

CREATE TRIGGER terminal_refusal_blocks_action_dispatch BEFORE INSERT ON action_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE action_id=NEW.action_id AND refusal_event_id IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'terminal refusal forbids a dispatch'); END;
CREATE TRIGGER terminal_refusal_blocks_dispatch_reassignment BEFORE UPDATE OF action_id ON action_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE action_id=NEW.action_id AND refusal_event_id IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'terminal refusal forbids a dispatch'); END;
CREATE TRIGGER terminal_refusal_blocks_broker_dispatch BEFORE INSERT ON broker_dispatches
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE action_id=NEW.action_id AND refusal_event_id IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'terminal refusal forbids a broker dispatch'); END;
CREATE TRIGGER terminal_refusal_blocks_effect_phase BEFORE INSERT ON broker_phase_records
 WHEN (NEW.phase_key='dispatch' OR NEW.phase_key LIKE 'observation:%')
 AND EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE action_id=NEW.action_id AND refusal_event_id IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'terminal refusal forbids a broker effect phase'); END;

-- Q-12 can remove the payload, but the event header and its digest are durable.
CREATE TRIGGER terminal_refusal_event_header BEFORE UPDATE ON events
 WHEN (NEW.event_type='TerminalControlDispatchRefused' OR OLD.event_type='TerminalControlDispatchRefused')
 AND (NEW.sequence IS NOT OLD.sequence OR NEW.event_id IS NOT OLD.event_id
  OR NEW.stream_id IS NOT OLD.stream_id OR NEW.aggregate_type IS NOT OLD.aggregate_type
  OR NEW.aggregate_id IS NOT OLD.aggregate_id OR NEW.aggregate_revision IS NOT OLD.aggregate_revision
  OR NEW.event_ordinal IS NOT OLD.event_ordinal OR NEW.event_type IS NOT OLD.event_type
  OR NEW.schema_version IS NOT OLD.schema_version OR NEW.project_id IS NOT OLD.project_id
  OR NEW.actor_ref IS NOT OLD.actor_ref OR NEW.actor_id IS NOT OLD.actor_id
  OR NEW.occurred_at_ms IS NOT OLD.occurred_at_ms OR NEW.recorded_at_ms IS NOT OLD.recorded_at_ms
  OR NEW.payload_hash IS NOT OLD.payload_hash OR NEW.sensitivity IS NOT OLD.sensitivity
  OR NEW.command_actor_id IS NOT OLD.command_actor_id OR NEW.command_id IS NOT OLD.command_id
  OR NEW.causation_kind IS NOT OLD.causation_kind OR NEW.causation_id IS NOT OLD.causation_id
  OR NEW.causation_receipt_id IS NOT OLD.causation_receipt_id OR NEW.correlation_id IS NOT OLD.correlation_id
  OR ((NEW.payload_state IS NOT OLD.payload_state OR NEW.payload_json IS NOT OLD.payload_json)
   AND NOT(OLD.payload_state='full' AND NEW.payload_state='tombstone' AND NEW.payload_json='{"tombstoned":true}')))
 BEGIN SELECT RAISE(ABORT,'terminal refusal event header is immutable'); END;
