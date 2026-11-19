
CREATE TABLE provider_probe_transport_contexts (
 probe_id TEXT PRIMARY KEY REFERENCES provider_probes(probe_id),
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL REFERENCES principals(principal_id),
 project_id TEXT REFERENCES projects(project_id),
 profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
 authority_generation TEXT NOT NULL,
 daemon_epoch INTEGER NOT NULL CHECK(daemon_epoch>=0),
 action_id TEXT NOT NULL UNIQUE,
 mandate_ref TEXT NOT NULL REFERENCES provider_probe_mandates(mandate_ref),
 phase TEXT NOT NULL CHECK(phase IN ('connect','inventory')),
 context_digest TEXT NOT NULL UNIQUE,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 created_at_ms INTEGER NOT NULL,
 claim_state TEXT NOT NULL DEFAULT 'reserved' CHECK(claim_state IN ('reserved','claimed')),
 claim_revision INTEGER NOT NULL DEFAULT 0 CHECK(claim_revision>=0),
 claim_id TEXT UNIQUE,
 claimed_at_ms INTEGER,
 CHECK((claim_state='reserved' AND claim_revision=0 AND claim_id IS NULL AND claimed_at_ms IS NULL)
  OR (claim_state='claimed' AND claim_revision=1 AND claim_id IS NOT NULL AND claimed_at_ms IS NOT NULL)),
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
  AND json_extract(context_json,'$.probe_id')=probe_id
  AND json_extract(context_json,'$.receipt_id')=receipt_id
  AND json_extract(context_json,'$.actor_id')=actor_id
  AND json_extract(context_json,'$.principal_id')=principal_id
  AND json_extract(context_json,'$.project_id') IS project_id
  AND json_extract(context_json,'$.profile_version_ref')=profile_version_ref
  AND json_extract(context_json,'$.authority_generation')=authority_generation
  AND json_extract(context_json,'$.daemon_epoch')=daemon_epoch
  AND json_extract(context_json,'$.action_id')=action_id
  AND json_extract(context_json,'$.phase')=phase
  AND json_extract(context_json,'$.mandate_ref')=mandate_ref,0))
) STRICT;
CREATE TRIGGER provider_probe_transport_exclusive BEFORE INSERT ON provider_probe_transport_contexts
WHEN EXISTS(SELECT 1 FROM provider_probe_contexts WHERE probe_id=NEW.probe_id)
 OR NOT EXISTS(SELECT 1 FROM provider_probes p WHERE p.probe_id=NEW.probe_id
  AND p.command_receipt_id=NEW.receipt_id AND p.principal_id=NEW.principal_id
  AND p.profile_version_ref=NEW.profile_version_ref AND p.phase=NEW.phase
  AND p.authority_generation=NEW.authority_generation AND p.daemon_epoch=NEW.daemon_epoch)
BEGIN SELECT RAISE(ABORT,'transport probe context owner mismatch'); END;
CREATE TRIGGER provider_probe_generation_exclusive BEFORE INSERT ON provider_probe_contexts
WHEN EXISTS(SELECT 1 FROM provider_probe_transport_contexts WHERE probe_id=NEW.probe_id)
BEGIN SELECT RAISE(ABORT,'generation and transport contexts are exclusive'); END;
CREATE TRIGGER provider_probe_transport_immutable BEFORE UPDATE ON provider_probe_transport_contexts
WHEN OLD.probe_id IS NOT NEW.probe_id OR OLD.receipt_id IS NOT NEW.receipt_id
 OR OLD.actor_id IS NOT NEW.actor_id OR OLD.principal_id IS NOT NEW.principal_id
 OR OLD.project_id IS NOT NEW.project_id OR OLD.profile_version_ref IS NOT NEW.profile_version_ref
 OR OLD.authority_generation IS NOT NEW.authority_generation OR OLD.daemon_epoch IS NOT NEW.daemon_epoch
 OR OLD.action_id IS NOT NEW.action_id OR OLD.mandate_ref IS NOT NEW.mandate_ref
 OR OLD.phase IS NOT NEW.phase OR OLD.context_digest IS NOT NEW.context_digest
 OR OLD.context_json IS NOT NEW.context_json OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR (OLD.claim_state='claimed' AND (OLD.claim_state IS NOT NEW.claim_state
  OR OLD.claim_revision IS NOT NEW.claim_revision OR OLD.claim_id IS NOT NEW.claim_id
  OR OLD.claimed_at_ms IS NOT NEW.claimed_at_ms))
BEGIN SELECT RAISE(ABORT,'transport probe immutable identity and claim'); END;
CREATE TRIGGER provider_probe_transport_durable BEFORE DELETE ON provider_probe_transport_contexts
BEGIN SELECT RAISE(ABORT,'transport probe context durable'); END;
CREATE TRIGGER admission_source_probe_transport_insert AFTER INSERT ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_transport_update AFTER UPDATE ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
CREATE TRIGGER admission_source_probe_transport_delete AFTER DELETE ON provider_probe_transport_contexts
BEGIN UPDATE admission_source_clock SET revision=revision+1 WHERE singleton=1; END;
