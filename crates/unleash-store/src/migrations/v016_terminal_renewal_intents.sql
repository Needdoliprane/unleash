-- An observed renewal extends only an existing, still-held controller lease.
-- It never allocates a successor lease, fence, process, host or PTY.
CREATE TABLE terminal_renewal_intents (
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
 control_revision INTEGER NOT NULL CHECK(control_revision BETWEEN 0 AND 9007199254740991),
 previous_expires_at_ms INTEGER NOT NULL,
 proposed_expires_at_ms INTEGER NOT NULL CHECK(proposed_expires_at_ms>=previous_expires_at_ms),
 context_digest TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 state TEXT NOT NULL CHECK(state IN ('prepared','handshake_resolved','observed','outcome_unknown','disposed')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
 observation_object_hash TEXT REFERENCES object_records(object_hash),
 disposition_object_hash TEXT REFERENCES object_records(object_hash),
 disposition_receipt_id TEXT REFERENCES command_receipts(receipt_id),
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
  AND json_extract(context_json,'$.previous_expires_at_ms')=previous_expires_at_ms
  AND json_extract(context_json,'$.proposed_expires_at_ms')=proposed_expires_at_ms,0)),
 CHECK((state='prepared' AND dispatch_id IS NULL AND observation_object_hash IS NULL AND disposition_object_hash IS NULL AND disposition_receipt_id IS NULL)
  OR(state IN ('handshake_resolved','outcome_unknown') AND dispatch_id IS NOT NULL AND observation_object_hash IS NULL AND disposition_object_hash IS NULL AND disposition_receipt_id IS NULL)
  OR(state='observed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NOT NULL AND disposition_object_hash IS NULL AND disposition_receipt_id IS NULL)
  OR(state='disposed' AND dispatch_id IS NOT NULL AND observation_object_hash IS NULL AND disposition_object_hash IS NOT NULL AND disposition_receipt_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX terminal_renewal_one_pending ON terminal_renewal_intents(terminal_id)
 WHERE state IN ('prepared','handshake_resolved','outcome_unknown');
CREATE UNIQUE INDEX terminal_renewal_command ON terminal_renewal_intents(actor_id,command_id,authority_generation);
CREATE TRIGGER terminal_renewal_initial BEFORE INSERT ON terminal_renewal_intents
 WHEN NEW.state!='prepared' OR NEW.revision!=1
 BEGIN SELECT RAISE(ABORT,'terminal renewal must start prepared'); END;
CREATE TRIGGER terminal_renewal_immutable BEFORE UPDATE ON terminal_renewal_intents
 WHEN NEW.action_id IS NOT OLD.action_id OR NEW.terminal_id IS NOT OLD.terminal_id
 OR NEW.receipt_id IS NOT OLD.receipt_id OR NEW.command_id IS NOT OLD.command_id
 OR NEW.actor_id IS NOT OLD.actor_id OR NEW.principal_id IS NOT OLD.principal_id
 OR NEW.authority_generation IS NOT OLD.authority_generation OR NEW.daemon_epoch IS NOT OLD.daemon_epoch
 OR NEW.lease_id IS NOT OLD.lease_id OR NEW.resource_fence IS NOT OLD.resource_fence
 OR NEW.control_revision IS NOT OLD.control_revision
 OR NEW.previous_expires_at_ms IS NOT OLD.previous_expires_at_ms
 OR NEW.proposed_expires_at_ms IS NOT OLD.proposed_expires_at_ms
 OR NEW.context_digest IS NOT OLD.context_digest OR NEW.context_json IS NOT OLD.context_json
 BEGIN SELECT RAISE(ABORT,'terminal renewal intention immutable'); END;
CREATE TRIGGER terminal_renewal_transition BEFORE UPDATE ON terminal_renewal_intents
 WHEN NOT(NEW.revision=OLD.revision+1 AND (
  (OLD.state='prepared' AND NEW.state IN ('handshake_resolved','outcome_unknown') AND NEW.dispatch_id IS NOT NULL)
  OR(OLD.state='handshake_resolved' AND NEW.state IN ('observed','outcome_unknown') AND NEW.dispatch_id IS OLD.dispatch_id)
  OR(OLD.state='outcome_unknown' AND NEW.state='disposed' AND NEW.dispatch_id IS OLD.dispatch_id)))
 BEGIN SELECT RAISE(ABORT,'terminal renewal transition invalid'); END;
CREATE TRIGGER terminal_renewal_durable BEFORE DELETE ON terminal_renewal_intents
 BEGIN SELECT RAISE(ABORT,'terminal renewal intention durable'); END;

CREATE TABLE terminal_renewal_transfer_links (
 renewal_action_id TEXT PRIMARY KEY REFERENCES terminal_renewal_intents(action_id),
 renewal_dispatch_id TEXT NOT NULL UNIQUE REFERENCES action_dispatches(dispatch_id),
 renewal_context_digest TEXT NOT NULL,
 transfer_action_id TEXT NOT NULL UNIQUE REFERENCES terminal_control_intents(action_id) DEFERRABLE INITIALLY DEFERRED,
 transfer_receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE TRIGGER terminal_renewal_transfer_link_initial BEFORE INSERT ON terminal_renewal_transfer_links
 WHEN NOT EXISTS(SELECT 1 FROM terminal_renewal_intents r JOIN actions a ON a.action_id=r.action_id
  WHERE r.action_id=NEW.renewal_action_id AND r.state='outcome_unknown'
   AND r.dispatch_id=NEW.renewal_dispatch_id AND r.context_digest=NEW.renewal_context_digest
   AND a.state='outcome_unknown')
 BEGIN SELECT RAISE(ABORT,'renewal transfer requires quarantined exact dispatch'); END;
CREATE TRIGGER terminal_renewal_transfer_link_immutable BEFORE UPDATE ON terminal_renewal_transfer_links
 BEGIN SELECT RAISE(ABORT,'renewal transfer link immutable'); END;
CREATE TRIGGER terminal_renewal_transfer_link_durable BEFORE DELETE ON terminal_renewal_transfer_links
 BEGIN SELECT RAISE(ABORT,'renewal transfer link durable'); END;

-- Renewal is serialized against both other effect paths. Existing input-unknown
-- to successor-transfer recovery remains valid; no cross-guard is added there.
CREATE TRIGGER terminal_renewal_exclusive BEFORE INSERT ON terminal_renewal_intents
 WHEN EXISTS(SELECT 1 FROM terminal_control_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','handshake_resolved','outcome_unknown'))
 OR EXISTS(SELECT 1 FROM terminal_input_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','write_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal effect requires observation before renewal'); END;
CREATE TRIGGER terminal_control_no_pending_renewal BEFORE INSERT ON terminal_control_intents
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents r WHERE r.terminal_id=NEW.terminal_id AND r.state IN ('prepared','handshake_resolved','outcome_unknown')
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
CREATE TRIGGER terminal_input_no_pending_renewal BEFORE INSERT ON terminal_input_intents
 WHEN EXISTS(SELECT 1 FROM terminal_renewal_intents WHERE terminal_id=NEW.terminal_id AND state IN ('prepared','handshake_resolved','outcome_unknown'))
 BEGIN SELECT RAISE(ABORT,'terminal renewal requires observation before input'); END;
