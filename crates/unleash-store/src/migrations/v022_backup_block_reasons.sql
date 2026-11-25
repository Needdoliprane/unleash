-- REQ-SEC-007 / REQ-REC-002 : motifs fermés de blocage avant dispatch d'une demande de sauvegarde.
--
-- La V007 a fermé `block_reason` par une contrainte CHECK. SQLite ne modifie pas une CHECK en
-- place : la table est reconstruite à l'identique (colonnes, clés, contrôles, déclencheurs et
-- lignes), dans la transaction de la migration. Deux motifs attestés s'ajoutent à l'ensemble,
-- qui reste fermé :
--   policy_changed   : politique effective courante différente de celle que la demande a engagée ;
--   preflight_failed : contrôles internes du harnais en échec avant tout marqueur d'effet.
-- Aucune table ne référence `backup_requests` par clé étrangère : la reconstruction ne touche
-- qu'à cette table.
PRAGMA defer_foreign_keys=ON;
CREATE TABLE backup_requests_v022 (
    backup_id TEXT PRIMARY KEY,
    receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
    action_id TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    authority_generation TEXT NOT NULL,
    daemon_epoch INTEGER NOT NULL CHECK (daemon_epoch >= 0),
    scope_revision INTEGER NOT NULL CHECK (scope_revision >= 0),
    policy_revision INTEGER NOT NULL CHECK (policy_revision >= 0),
    destination_path TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('reserved','claimed','executing','completed','blocked')),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    dispatch_id TEXT UNIQUE REFERENCES action_dispatches(dispatch_id),
    barrier_pin_id TEXT UNIQUE REFERENCES object_pins(pin_id),
    manifest_digest TEXT,
    manifest_json TEXT,
    created_at_ms INTEGER NOT NULL,
    block_reason TEXT CHECK (block_reason IS NULL OR block_reason IN ('guard_profile_unavailable','isolation_profile_unavailable','authorization_denied','policy_changed','preflight_failed')),
    CHECK ((state IN ('executing','completed') AND dispatch_id IS NOT NULL AND barrier_pin_id IS NOT NULL) OR state NOT IN ('executing','completed')),
    CHECK (state != 'completed' OR (manifest_digest IS NOT NULL AND manifest_json IS NOT NULL))
) STRICT;
INSERT INTO backup_requests_v022
    (backup_id, receipt_id, action_id, actor_id, principal_id, authority_generation, daemon_epoch,
     scope_revision, policy_revision, destination_path, payload_digest, state, revision,
     dispatch_id, barrier_pin_id, manifest_digest, manifest_json, created_at_ms, block_reason)
SELECT backup_id, receipt_id, action_id, actor_id, principal_id, authority_generation, daemon_epoch,
       scope_revision, policy_revision, destination_path, payload_digest, state, revision,
       dispatch_id, barrier_pin_id, manifest_digest, manifest_json, created_at_ms, block_reason
FROM backup_requests;
DROP TABLE backup_requests;
ALTER TABLE backup_requests_v022 RENAME TO backup_requests;
CREATE TRIGGER backup_requests_immutable BEFORE UPDATE ON backup_requests
WHEN OLD.backup_id IS NOT NEW.backup_id OR OLD.receipt_id IS NOT NEW.receipt_id
 OR OLD.action_id IS NOT NEW.action_id OR OLD.actor_id IS NOT NEW.actor_id
 OR OLD.principal_id IS NOT NEW.principal_id OR OLD.authority_generation IS NOT NEW.authority_generation
 OR OLD.destination_path IS NOT NEW.destination_path OR OLD.payload_digest IS NOT NEW.payload_digest
 OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR (OLD.state != 'reserved' AND (OLD.daemon_epoch IS NOT NEW.daemon_epoch OR OLD.scope_revision IS NOT NEW.scope_revision OR OLD.policy_revision IS NOT NEW.policy_revision))
 OR (OLD.dispatch_id IS NOT NULL AND OLD.dispatch_id IS NOT NEW.dispatch_id)
 OR (OLD.barrier_pin_id IS NOT NULL AND OLD.barrier_pin_id IS NOT NEW.barrier_pin_id)
 OR (OLD.manifest_digest IS NOT NULL AND OLD.manifest_digest IS NOT NEW.manifest_digest)
 OR (OLD.manifest_json IS NOT NULL AND OLD.manifest_json IS NOT NEW.manifest_json)
 OR (OLD.state != 'reserved' AND NEW.state = 'reserved')
BEGIN SELECT RAISE(ABORT,'backup request immutable identity/provenance'); END;
CREATE TRIGGER backup_requests_no_delete BEFORE DELETE ON backup_requests BEGIN SELECT RAISE(ABORT,'backup request durable identity'); END;
CREATE TRIGGER backup_block_reason_immutable BEFORE UPDATE ON backup_requests
WHEN OLD.block_reason IS NOT NULL AND OLD.block_reason IS NOT NEW.block_reason
BEGIN SELECT RAISE(ABORT,'backup block reason immutable'); END;
